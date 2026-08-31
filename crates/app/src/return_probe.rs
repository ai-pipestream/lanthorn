//! After a move, find the way BACK — in a silent shadow of the game (SQ-0785).
//!
//! ```text
//! > enter window
//! Kitchen
//! ```
//!
//! …and the map now knows that `east` returns you to Behind House, without
//! anybody having typed it, and without claiming that `west` works from Behind
//! House. Two different facts, and the second one is a lie.
//!
//! # The gap this closes, and the one it must not invent
//!
//! An automap built from a player's moves only ever learns one direction of a
//! passage at a time. Half the rooms on a map you have walked through once are
//! joined by a single arrow, and the layout, the routing and the click-to-route
//! all reason about a graph that is thinner than the world it describes.
//!
//! The obvious fix is to assume passages reciprocate. They do not: these games
//! are full of one-way drops, doors that only open from one side, and mazes
//! whose whole design is that the way back is not the way you came. Guessing
//! wrong writes an edge that does not exist, and a wrong edge is worse than the
//! missing one it replaced — it is the map asserting something false, and the
//! player has no way to tell which arrows were observed and which were assumed.
//!
//! So the way back is **discovered**, in a copy of the game that costs nothing:
//! [`crate::probe`]'s shadow is restored to exactly where the player is standing
//! and asked to walk one direction. If it comes out in a room the map already
//! holds, that passage is real and goes on the map. If it comes out anywhere
//! else, nothing at all is recorded.
//!
//! # Success is room identity, not a room
//!
//! **Landing somewhere is not landing back.** The test is `step.location`: the
//! mapper's own location detection, the same `snap.number`
//! [`crate::session::apply_turn`] keys rooms by. The search ENDS when that number
//! is the room the player came from; a landing anywhere else is a different
//! question's answer and leaves the search running.
//!
//! **A v4+ story tells the interpreter where it is through the STATUS LINE**, and
//! Quetzal archives no screen — so a shadow restored into the player's moment
//! used to inherit the previous probe's status line, and a story that repaints
//! only as many columns as its new room name needs left the tail of the longer
//! one behind. Zork I's shadow read `Forest Pathse`, which matches no object; the
//! ladder fell off `PlayerParent` onto the text rung and `resolve_room_object`
//! prefix-matched object 1 — the scenery object named `forest` — so a real return
//! path was discarded as a landing in the wrong room. `restore_state` now blanks
//! the upper window, because memory restored without a screen must not be read
//! against another moment's screen (SQ-0785).
//!
//! That is also why this consumer needs none of the probe seam's
//! [`crate::probe::Refusals`] machinery. A vocabulary offer has to read the
//! story's prose to find out whether anything happened, because "did this verb
//! do something" is only answerable in words. "Am I back where I started" is
//! answerable in a room number.
//!
//! **A probe that lands in a room the map does NOT hold records the ATTEMPT and
//! nothing else.** Not room C, not the edge to it, not its existence. The map is
//! a record of what the PLAYER has seen, and keeping C "known but hidden" would
//! leak straight back out through the layout, the pathfinder and click-to-route.
//! Total failure likewise says nothing about the map: it proves only that these
//! directions did not work from here, this time. A door may need opening, and a
//! one-way passage is a real and beloved part of these games.
//!
//! **But a room the map ALREADY HOLDS is a room the player has stood in**, and a
//! passage between two such rooms reveals nothing unseen — so it is recorded even
//! though it is not the answer the search was after, and the search carries on
//! looking for the one that is. That is not a bonus, it is the fix for a defect
//! the narrower rule caused (SQ-0785): [`mapper::graph::Room::probed`] says "this
//! direction was walked from here", but the answer it stood for was "…and it did
//! not reach THAT origin". Reused against a different origin it SUPPRESSED the
//! right answer. Zork I's South of House was probed westward while the search was
//! asking about Behind House, reached West of House, and threw that away; on a
//! later visit — with the player now arriving FROM West of House — `W` was
//! already on the probed record, so the first surviving candidate was the
//! diagonal `NW`, and the map recorded a diagonal where a cardinal was known.
//! Recording every landing on a known room closes the gap on the first visit, so
//! the second one has no gap left to ask about.
//!
//! # Two records, and why they must never merge
//!
//! [`mapper::graph::Room::tried`] is what the PLAYER has typed here, and
//! `untried()` turns it into the exits the map still offers as unexplored.
//! [`mapper::graph::Room::probed`] is what the SEARCH has walked. Marking a probe
//! as tried would take a genuine unexplored exit off the map and quietly steer
//! the player away from content they have never seen.
//!
//! Nothing in this module reads either list directly:
//! [`mapper::graph::MapGraph::probe_candidates`] is the single accessor that
//! consults both and applies the priority order, so the rule about which record
//! means what lives in one place rather than being re-derived by every caller.
//!
//! # The order it tries, and why it starts wide
//!
//! The way back is overwhelmingly the way you came, so the search leads with
//! `opposite(D)`, then the two perpendiculars, then the two diagonals beside the
//! opposite, then everything else — all twelve real passages. Starting wide is
//! deliberate: narrowing the list is a measurement decision and there was no
//! measurement yet. Every attempt that is answered is recorded permanently, so
//! the cost of a wide list is paid once per room in the life of a map, not once
//! per visit.
//!
//! # On the worker, and why staleness does not apply
//!
//! The search runs one attempt at a time through [`crate::probe::ShadowProbe`],
//! which lives on its own thread and holds one question at a time. Two
//! consequences of SQ-1124's threading deliberately do NOT carry over:
//!
//! **A return-path result is never stale.** A vocabulary suggestion is worthless
//! once the player has typed again, so SQ-1124 drops any answer whose
//! `turn_epoch` has moved. *"South from the Kitchen returns to Behind House"* is
//! true wherever the player has wandered since — it is a fact about the map, not
//! about this turn — so an answer arriving three moves later is recorded exactly
//! as one arriving immediately.
//!
//! **A new MOVE aborts the search**, though, because the move may itself be the
//! walk back, which records the true edge for free and makes the search moot. A
//! turn that does not move the player (`look`, `take lamp`, a refused direction)
//! leaves the search running.
//!
//! Aborting is cheap because progress is durable: every answered attempt marks
//! the probed record before anything else happens, so the next visit resumes
//! where this one stopped instead of starting over. The single attempt that was
//! IN FLIGHT when the abort came is the one thing not carried — its answer was
//! never read, so nothing was learned about it, and it is offered again next
//! time rather than being written off.
//!
//! # Sharing one shadow with the vocabulary offer
//!
//! [`crate::probe::ShadowProbe`] holds one question at a time, and
//! [`crate::vocab`] asks it too. When the shadow is busy the search simply does
//! not ask this pass and tries again on the next one. It cannot starve on a slow
//! game the way a vocabulary offer can, for the reason above: it is not tied to a
//! turn, so waiting costs it nothing but time.
//!
//! # The caution the search can also afford to say (SQ-1043)
//!
//! A search that walks its whole list and never reaches the room the player left
//! has established something the player might want to know: *the passage they
//! just walked appears to be one-way*. That is the whole of SQ-1043, and it is
//! **free** — the twelve commands were typed anyway, for the map. So the caution
//! rides this search rather than forking a second one, which is what the quest
//! asked for in as many words ("one probe, not two"; whichever lands first owns
//! the seam, and this one landed first).
//!
//! **It fires AFTER the crossing, and that is a decision rather than a
//! shortcut.** Warning before it would mean holding the player's command while a
//! worker walks thirteen turns of their game — a stall on the main thread's
//! behalf, which is the very thing SQ-1124 took out of this seam — or scouting
//! every exit of every room speculatively, twelve searches deep, for exits the
//! player will mostly never take. What it costs instead is nothing at all, and
//! the caution still arrives while the player is standing in the room, because
//! [`arm_return_search`] abandons the search the moment they move: either the
//! evidence is in before their next command or it is never spoken.
//!
//! **The threshold is deliberately narrow, because a caution that cries wolf is
//! worse than none.** Three things must all hold, and the third is the one that
//! matters:
//!
//! 1. the search was armed at all — a real crossing, on a passage the map holds,
//!    with no way back already known;
//! 2. it ran its whole candidate list without landing on the origin;
//! 3. **and the shadow got OUT of the room at least once.** Without that,
//!    "nothing led back" is indistinguishable from "nothing led anywhere" — a
//!    dark room, a locked cell, a cutscene, a story that ended, an engine whose
//!    location we cannot read. Every one of those would otherwise fire the
//!    caution on evidence it does not have.
//!
//! What survives that threshold is still evidence and not proof, so the line
//! claims only what was tested: twelve directions, none of which returned. A way
//! back that needs a door opened, a key, a turn to pass, or two moves instead of
//! one is a way back this cannot see — which is why the wording says *looks*
//! one-way and names the room rather than saying anything at all about whether
//! the game can still be won.
//!
//! It cannot nag, either, and that falls out of the mechanism rather than being
//! arranged: exhausting the list marks all twelve on
//! [`mapper::graph::Room::probed`], so the next arrival has an empty candidate
//! list and arms no search. The caution is said once per room, ever.
//!
//! # The three switches the caution answers to, in this order
//!
//! Three, and none of them collapses into another — each answers a different
//! question, and a player can hold any combination of the three (SQ-1043's
//! follow-up).
//!
//! 1. **[`Config::one_way_caution`]** — the player's wish about THIS line, and
//!    first because it is the only switch that is about the line itself. Off, the
//!    map work below carries on exactly as before and only the sentence goes.
//!    Checked in [`one_way_caution`], where the sentence is made.
//!    Default **on**: turning `return_probe` on is already opting into this class
//!    of help, so the key exists to decline the line, not to have to ask for it.
//! 2. **[`Config::guidance`]** — the line is an assist, in the Guiding Light's
//!    register and its gutter, so the Light's own switch silences it with
//!    everything else the Light says. Enforced once for every assist in
//!    [`AppState::push_assist`] rather than here: a feature that has to remember
//!    to ask is a feature the player cannot turn off.
//! 3. **[`Config::return_probe`]** — last because it is not really a preference
//!    about the caution at all: mechanically the caution is a READING of this
//!    search, and with the probe off there is no search to read. Enforced in
//!    [`arm_return_search`], which is where the search would have started.
//!
//! # …and the suppression that is not a switch: undo
//!
//! **With undo switched off, the caution says nothing at all**, whatever those
//! three hold. That is the user's own verdict on the shipped feature turned into
//! a rule: a warning is worth saying only if the player can act on it, and the
//! act this one exists to prompt is `undo`. With `undo_levels = 0` there is
//! nothing to do about a one-way passage but read that it was one.
//!
//! The value read is the LIVE one — [`Engine::undo_levels`], asked of the running
//! session at the moment the search arms — and not `config.undo_levels`. The two
//! genuinely differ: `undo_levels` is one of the three settings-screen rows that
//! can only land at boot, so after a Save the config says one thing and the
//! machine the player is typing at is still capped at another. What the player
//! can actually do is the machine's answer.

use mapper::direction::{long_label, Direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

use crate::assist::Assist;
use crate::config::Config;
use crate::engine::Engine;
use crate::state::AppState;

/// One direction out with the worker: what was asked, and the token it will
/// answer under.
#[derive(Debug, Clone, Copy)]
struct Attempt {
    token: u64,
    dir: Direction,
}

/// A search for the way back from one room to another, in progress.
///
/// Session state and never persisted — what IS persisted is everything it
/// learns, in the graph's own two records. A restore begins with no search
/// running and picks up wherever the probed record left off.
#[derive(Debug)]
pub struct ReturnSearch {
    /// The room the player came FROM: the room a probe has to land in to succeed.
    origin: RoomId,
    /// The room the player is standing in, and the room every attempt starts from.
    /// The search ends the moment this stops being where they are.
    here: RoomId,
    /// The directions still to try, best first — [`MapGraph::probe_candidates`]'s
    /// order, taken once when the search is armed.
    ///
    /// [`MapGraph::probe_candidates`]: mapper::graph::MapGraph::probe_candidates
    queue: Vec<Direction>,
    /// The attempt out with the worker, if any.
    attempt: Option<Attempt>,
    /// The moment every attempt is asked from: the live game as it stood the
    /// instant the player arrived here.
    ///
    /// **Taken once, for the whole search.** Attempts go out one at a time so
    /// each answer is durable, and [`crate::probe::ShadowProbe::ask`] would
    /// otherwise charge the player's thread for a host snapshot per attempt —
    /// 102 ms each on Counterfeit Monkey in a debug build, twelve times over.
    /// One snapshot is 102 ms once, and the answer is about the map rather than
    /// about this instant. See [`crate::probe::ShadowProbe::snapshot`].
    from: crate::probe::ProbeSnapshot,
    /// Whether any attempt got the shadow OUT of [`here`](Self::here) — into any
    /// room at all, known or not, the origin or not.
    ///
    /// The whole of SQ-1043's threshold hangs on this. A search that exhausts its
    /// list having never left the room has not shown that the passage is one-way;
    /// it has shown that the shadow could not move, which is what a dark room, a
    /// locked cell, a cutscene and an unreadable location all look like from
    /// here. Only a room the shadow could walk OUT of, but not BACK from, is
    /// evidence of a one-way passage.
    left: bool,
    /// Whether the running story could take this move back — [`Engine::undo_levels`]
    /// asked of the live session when the search armed, `Some(0)` being undo off.
    ///
    /// False silences the caution and nothing else: the search still runs and
    /// still closes the map's gap. See the module docs — a warning is worth
    /// saying only if the player can act on it.
    ///
    /// Recorded HERE, at arm time, because that is the one moment in this
    /// module's life that holds the session (`pump_return_search` runs off the
    /// event loop with only the state and the map in hand). Nothing can change it
    /// under a live search either: the cap is written at boot and a search is
    /// abandoned the moment the player moves.
    undoable: bool,
}

impl ReturnSearch {
    /// The room the search runs from, for tests and diagnostics.
    pub fn here(&self) -> RoomId {
        self.here
    }

    /// The room it is trying to reach.
    pub fn origin(&self) -> RoomId {
        self.origin
    }

    /// How many directions it has left to try, the one in flight excluded.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }

    /// Whether the shadow has managed to leave [`here`](Self::here) at all — the
    /// third leg of SQ-1043's threshold, for tests and diagnostics.
    pub fn left_the_room(&self) -> bool {
        self.left
    }

    /// Whether the story this search armed on can take a move back, for tests and
    /// diagnostics. See [`undoable`](Self::undoable).
    pub fn is_undoable(&self) -> bool {
        self.undoable
    }
}

/// Start a search for the way back, if this turn earned one (SQ-0785).
///
/// Called once per turn, after `apply_turn` has settled where the player is.
/// `room_before` is what `mapper.graph.current()` said BEFORE that call — the
/// only moment it is knowable.
///
/// It is also where a running search is ABORTED: a turn that moved the player
/// ends whatever was in flight, because the move may be the walk back and the
/// search from the old room is about a room nobody is standing in any more.
///
/// The gate, in order:
///
/// * the feature is on for this game;
/// * the probe seam is armed (a session that never kept the story bytes has no
///   shadow to fork);
/// * the player is in a room, came from a different room, and a passage joins
///   them the way they went — so a death, a teleport and a refused move all
///   arm nothing, having crossed nothing;
/// * and **the map does not already know a way back**. That is the whole point:
///   with a return path recorded there is no gap to close, and the cheapest
///   probe is the one not run.
pub fn arm_return_search(
    state: &mut AppState,
    mapper: &Mapper,
    live: &dyn Engine,
    cmd: &str,
    room_before: Option<RoomId>,
) {
    let here = mapper.graph.current();
    // A move ends any search that was running: the room it was asking about is
    // behind us, and the move itself may have been the answer.
    if state.return_search.as_ref().is_some_and(|s| Some(s.here) != here) {
        state.return_search = None;
    }
    if !state.config.return_probe || !state.probe.is_armed() {
        return;
    }
    let (Some(here), Some(origin)) = (here, room_before) else { return };
    if here == origin {
        return; // no crossing this turn
    }
    // Only a passage the map actually holds is worth asking about the reverse of.
    // A relocation (death, teleport) mints no edge and must arm nothing.
    if !mapper.graph.connections().iter().any(|c| c.origin == origin && c.dest == here) {
        return;
    }
    // THE GATE. A known return path means there is no gap, and nothing to do.
    if mapper.graph.connections().iter().any(|c| c.origin == here && c.dest == origin) {
        return;
    }
    let mut queue = mapper.graph.probe_candidates(here, mapper::direction::parse_direction(cmd));
    if queue.is_empty() {
        return;
    }
    // The one snapshot the whole search runs from, and the one thing here the
    // player's thread pays for. Taken now rather than per attempt.
    let Some(from) = state.probe.snapshot(live) else { return };
    // Asked of the LIVE session, here, because this is the only place in the
    // module that has one — and `Some(0)` is undo switched off, the documented
    // value of `undo_levels = 0`. `None` is an engine with no host-settable cap
    // (Glulx keeps its own, Scott has none) and must not read as "undo is off".
    let undoable = live.undo_levels() != Some(0);
    queue.reverse(); // popped from the back, so the best candidate goes last
    state.return_search =
        Some(ReturnSearch { origin, here, queue, attempt: None, from, left: false, undoable });
}

/// What a search that has run out of candidates has to say to the player, if
/// anything (SQ-1043).
///
/// `None` unless the shadow actually left the room — see [`ReturnSearch::left`]
/// and the module docs. The line states the test rather than its implication:
/// twelve directions were walked from here and none of them reached the room the
/// player came from. It does not say the game cannot be won, does not say there
/// is no way back at all (a longer route, an opened door and a key are all
/// invisible to a one-step search), and does not say what to do about it.
///
/// The origin is named because a room the player has stood in is a room they can
/// picture, and because the alternative — the story's own "you" — is the voice
/// [`crate::assist`] exists to keep lanthorn out of.
///
/// Two of the caution's three switches are decided elsewhere and only the first
/// is decided here; see the module docs for the order and why each sits where it
/// does. `guidance` is [`AppState::push_assist`]'s, for every assist at once, and
/// `return_probe` is [`arm_return_search`]'s, because with the probe off there is
/// no search to read. The undo suppression is the search's own
/// [`undoable`](ReturnSearch::undoable), recorded off the live session when it
/// armed.
fn one_way_caution(cfg: &Config, search: &ReturnSearch, mapper: &Mapper) -> Option<Assist> {
    if !cfg.one_way_caution {
        return None;
    }
    // Undo off ⇒ silence: the act this line exists to prompt is unavailable, and
    // a warning nobody can act on is noise. The map work above has already
    // happened and is untouched by this.
    if !search.undoable {
        return None;
    }
    if !search.left {
        return None;
    }
    let origin = mapper.graph.room(search.origin).map(|r| r.name.trim()).filter(|n| !n.is_empty());
    Some(Assist::caution(match origin {
        Some(name) => format!("looks one-way — no direction leads back to {name}"),
        None => "looks one-way — no direction leads back".to_string(),
    }))
}

/// Hand the next candidate to the worker, if there is one and the shadow is
/// free. Called every pass of the event loop; returns true when something was
/// asked (nothing to redraw, but the caller may want to know).
///
/// The shadow is shared with the vocabulary offer and holds one question at a
/// time, so "busy" is an ordinary outcome and simply means try again next pass.
///
/// It is also where the search says its one line to the player, on the way out:
/// a list walked to the end without reaching the origin is SQ-1043's caution, if
/// the run cleared the threshold in [`one_way_caution`].
pub fn pump_return_search(state: &mut AppState, mapper: &Mapper) -> bool {
    let Some(search) = &state.return_search else { return false };
    if search.attempt.is_some() {
        return false; // one out already
    }
    let Some(&dir) = search.queue.last() else {
        // Nothing left to try. Total failure records nothing about the MAP: a
        // door may need opening, and a one-way passage is a real answer. But it
        // is exactly the observation SQ-1043 wanted, so it is said out loud —
        // once, here, while the player is still standing in the room.
        let caution = one_way_caution(&state.config, search, mapper);
        state.return_search = None;
        if let Some(caution) = caution {
            state.push_assist(&caution);
        }
        return false;
    };
    let Some(token) = state.probe.ask_from(&search.from, &[long_label(dir).to_string()]) else {
        return false; // busy, unarmed, or mid-save — ask again next pass
    };
    if let Some(search) = &mut state.return_search {
        search.queue.pop();
        search.attempt = Some(Attempt { token, dir });
    }
    true
}

/// True when `token` answers a question this search asked.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.return_search.as_ref().and_then(|s| s.attempt).is_some_and(|a| a.token == token)
}

/// Read one answer back, and record what it found (SQ-0785).
///
/// Returns true when the map changed, which is what tells the event loop to
/// bump the graph generation and redraw.
///
/// Four outcomes, in the order they are decided:
///
/// 1. **The attempt is recorded as probed, whatever it found.** First, and
///    unconditionally, so an abort a moment later still leaves the search one
///    step further along than it was.
/// 2. **It came out in a room the map already holds** — the passage is real, and
///    goes on the map through the same call a walked crossing makes.
///    [`Mapper::record_probed_passage`] is what enforces the no-leak rule: it
///    refuses a room the map does not have, so an unvisited room cannot arrive
///    this way however the probe lands.
/// 3. **…and if that room is the one the player LEFT, the search is over.**
///    Otherwise it keeps going: the gap it was opened to close is still open, and
///    what it just recorded is a different question's answer (SQ-0785).
/// 4. **Anything else** — an unknown room, nowhere at all, a death, a story that
///    ended, an engine that cannot say where it is — and nothing is recorded but
///    the attempt. The search moves on to the next direction.
pub fn deliver(
    state: &mut AppState,
    mapper: &mut Mapper,
    answer: &crate::probe::Answer,
) -> Option<ProbedPassage> {
    let search = state.return_search.as_mut()?;
    let attempt = search.attempt.filter(|a| a.token == answer.token)?;
    search.attempt = None;
    let (here, origin) = (search.here, search.origin);

    // (1) The attempt is durable before anything is judged.
    mapper.graph.mark_probed(here, attempt.dir);

    // (2) WHERE did it come out? Room identity and nothing else — a step that
    // ended the story or reached for a file answers nothing about the map,
    // whatever `location` happens to hold.
    let landed = answer.run.as_ref().and_then(|run| {
        run.steps.first().filter(|s| !s.quit && !s.escaped).and_then(|s| s.location)
    });
    let Some(landed) = landed else {
        return None; // no room. Nothing about it is recorded, not even that it exists.
    };
    // The shadow got out. Not a fact about the map — the room it reached may be
    // one nobody has seen and stays unrecorded — but the fact SQ-1043's caution
    // needs, which is that the exits from here work at all.
    if landed != here {
        search.left = true;
    }

    // (3) A room the map already holds is a room the PLAYER has stood in, so the
    // passage to it can be drawn without revealing anything unseen — whether or
    // not it is the room this search was asking about. `record_probed_passage`
    // refuses an unknown room itself, so the no-leak rule lives in one place
    // rather than being restated here. It also refuses `from == to` (a refused
    // move, where the player never left) and a direction already leaving `from`
    // (the player walked it back while this was in flight, and a real traversal
    // is the better authority on its own passage).
    let passage = ProbedPassage { from: here, dir: attempt.dir, to: landed };
    let recorded = mapper.record_probed_passage(passage);

    // (4) …but the SEARCH ends only on the room it was opened to find, and it
    // ends whether or not the graph took the edge — with the player back there
    // by their own move there is no gap left to close either way. A landing
    // anywhere else leaves it running: what was just recorded is a different
    // question's answer, and this question is still open.
    if landed == origin {
        state.return_search = None;
    }
    recorded.then_some(passage)
}

/// Run a search to its end, waiting for each answer instead of collecting one
/// that has already arrived — the whole search in one call.
///
/// **Not for the event loop.** It is what a test harness and a measurement
/// harness need: the answer without racing the thread, and the shadow's own
/// `probes`/`spent` counters left holding the cost of the whole search.
///
/// Returns the passage back to the ORIGIN if one was found. Edges to other rooms
/// the map already holds are recorded as they turn up and do not end the search,
/// so a caller wanting every change the run made should read the graph rather
/// than this value. Bounded by the candidate list, so it terminates whatever the
/// story does; a shadow that will not answer at all ends it by breaking the seam,
/// which [`crate::probe::ShadowProbe::settle`] reports as `None`.
pub fn settle_return_search(state: &mut AppState, mapper: &mut Mapper) -> Option<ProbedPassage> {
    while state.return_search.is_some() {
        if !pump_return_search(state, mapper) {
            // Nothing was asked: either the search just ended, or the shadow is
            // busy with somebody else's question — and in a harness there is
            // nobody else, so this is the seam refusing and the search is over.
            if state.return_search.as_ref().is_some_and(|s| s.attempt.is_none()) {
                state.return_search = None;
            }
            if state.return_search.is_none() {
                break;
            }
        }
        let Some(answer) = state.probe.settle() else { break };
        if !owns(state, answer.token) {
            continue; // somebody else's, and nobody is here to want it
        }
        let passage = deliver(state, mapper, &answer);
        // `deliver` clears the search only on the room it was asking about, so an
        // empty `return_search` here IS the end — and the passage has to be
        // carried out of the loop rather than left to the `while`, which would
        // drop it. A landing on some OTHER known room records its edge and leaves
        // the search running, which is the whole point (SQ-0785).
        if state.return_search.is_none() {
            return passage;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real engine, because arming takes a snapshot of one. `tiny_cave.dat` is
    /// the smallest story in the repo and is freely redistributable, so these
    /// cases never skip.
    fn blind() -> crate::scott_session::ScottSession {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        crate::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads")
    }

    fn armed_state() -> AppState {
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(crate::probe::ShadowRecipe {
            story_bytes: std::sync::Arc::new(
                include_bytes!("../../scott/tests/tiny_cave.dat").to_vec(),
            ),
            ..Default::default()
        });
        state
    }

    fn walked(m: &mut Mapper) {
        m.observe(1, "Behind House", None);
        m.observe(2, "Kitchen", Some(Direction::In));
    }

    /// The gate: a crossing with no way back arms a search, and the same crossing
    /// with a way back already on the map arms nothing at all.
    #[test]
    fn a_known_return_path_means_no_search() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1));
        let s = state.return_search.as_ref().expect("a gap to close");
        assert_eq!((s.here(), s.origin()), (2, 1));
        assert_eq!(s.remaining(), 12, "all twelve, best first");

        // Now the player walks back themselves, and the gap is gone.
        m.observe(1, "Behind House", Some(Direction::E));
        m.observe(2, "Kitchen", Some(Direction::In));
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1));
        assert!(state.return_search.is_none(), "no gap, no probe");
    }

    /// Nothing arms without a crossing: a turn that did not move the player, and
    /// a relocation that minted no passage (a death, a teleport), both of which
    /// leave `current` changed but no edge behind them.
    #[test]
    fn only_a_real_crossing_arms_a_search() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();

        arm_return_search(&mut state, &m, &blind(), "look", Some(2));
        assert!(state.return_search.is_none(), "the player did not cross anything");

        m.observe_relocation(3, "Forest");
        arm_return_search(&mut state, &m, &blind(), "north", Some(2));
        assert!(state.return_search.is_none(), "a relocation walked no passage");
    }

    /// Off is off, and an unarmed seam probes nothing — the default state of
    /// every test-built `AppState`, and of any session with no story bytes kept.
    #[test]
    fn the_switch_and_the_seam_both_have_to_be_on() {
        let mut m = Mapper::default();
        walked(&mut m);

        let mut off = armed_state();
        off.config.return_probe = false;
        arm_return_search(&mut off, &m, &blind(), "enter window", Some(1));
        assert!(off.return_search.is_none());

        let mut unarmed = AppState::default();
        unarmed.config.return_probe = true;
        assert!(!unarmed.probe.is_armed());
        arm_return_search(&mut unarmed, &m, &blind(), "enter window", Some(1));
        assert!(unarmed.return_search.is_none());
    }

    /// A MOVE ends the search; a turn that does not move the player leaves it
    /// running, because the room it is asking about is still the room they are in.
    #[test]
    fn a_move_aborts_the_search_and_a_still_turn_does_not() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1));
        assert!(state.return_search.is_some());

        arm_return_search(&mut state, &m, &blind(), "take lamp", Some(2));
        assert!(state.return_search.is_some(), "a still turn leaves it alone");

        m.observe(3, "Attic", Some(Direction::Up));
        arm_return_search(&mut state, &m, &blind(), "up", Some(2));
        let s = state.return_search.as_ref().expect("a fresh search from the new room");
        assert_eq!((s.here(), s.origin()), (3, 2), "the old one is gone, not resumed");
    }

    /// A search that has run out of candidates, with the shadow's own record of
    /// whether it ever got out of the room. The one shape [`one_way_caution`]
    /// judges, built directly because no story reliably produces it on demand
    /// and [`crate::probe::Answer`] cannot be forged.
    fn exhausted(state: &mut AppState, left: bool) {
        exhausted_with_undo(state, left, true);
    }

    /// The same shape, with the undo the caution's suppression turns on stated
    /// rather than assumed.
    fn exhausted_with_undo(state: &mut AppState, left: bool, undoable: bool) {
        let from = state.probe.snapshot(&blind()).expect("the seam is armed");
        state.return_search = Some(ReturnSearch {
            origin: 1,
            here: 2,
            queue: Vec::new(),
            attempt: None,
            from,
            left,
            undoable,
        });
    }

    /// Everything on a test-built state's transcript, which is only ever what
    /// this module put there: the once-per-session introduction and the caution.
    /// Deliberately not filtered by transcript KIND — spelling that variant
    /// anywhere under `src/` outside the three files that own it is what
    /// `assist_voice`'s `push_assist_is_the_only_producer_of_an_assist_line`
    /// exists to catch, and a test helper is not a reason to relax it.
    fn said(state: &AppState) -> Vec<String> {
        state.transcript.clone()
    }

    /// **The caution, and the whole of what it claims** (SQ-1043). Twelve
    /// directions walked from a room the shadow could get OUT of, none of which
    /// reached the room the player left — so the passage looks one-way, and the
    /// line says that and names the room, and nothing else.
    #[test]
    fn an_exhausted_search_that_could_leave_the_room_says_so_once() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        state.config.guidance = true;
        exhausted(&mut state, true);

        assert!(!pump_return_search(&mut state, &m), "nothing left to ask");
        assert!(state.return_search.is_none(), "and the search is over");

        let lines = said(&state);
        let line = lines.last().expect("the caution");
        assert_eq!(line, "looks one-way — no direction leads back to Behind House");
        // The register's own rules, on the one line this module writes.
        assert!(!line.starts_with('['), "the parser's brackets are never ours: {line:?}");
        assert!(!line.contains("you"), "the story owns \"you\": {line:?}");
        assert!(!line.contains("win") && !line.contains("stuck"), "no claim it cannot support");

        // Said once. A second pass has no search left to exhaust.
        let before = lines.len();
        assert!(!pump_return_search(&mut state, &m));
        assert_eq!(said(&state).len(), before, "nothing to repeat");
    }

    /// **The threshold's third leg.** A search that exhausted its list without
    /// the shadow ever leaving the room has not shown the passage is one-way —
    /// it has shown the shadow could not move, which is what a dark room, a
    /// locked cell and an unreadable location all look like. Silence.
    #[test]
    fn a_shadow_that_never_got_out_of_the_room_says_nothing() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        state.config.guidance = true;
        exhausted(&mut state, false);

        pump_return_search(&mut state, &m);
        assert!(state.return_search.is_none(), "the search still ends");
        assert!(said(&state).is_empty(), "but it has established nothing to say");
    }

    /// The player's switch is the same switch, because there is one door: with
    /// Lanthorn's Guiding Light off, `push_assist` drops the line and the search
    /// goes on closing map gaps in silence.
    #[test]
    fn the_guidance_switch_silences_the_caution_and_not_the_search() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        state.config.guidance = false;
        exhausted(&mut state, true);

        pump_return_search(&mut state, &m);
        assert!(said(&state).is_empty(), "the light is off");
        assert!(state.config.return_probe, "and the map work is untouched");
    }

    /// **With undo off, there is nothing to say.** The act this line exists to
    /// prompt is `undo`, so with the running machine capped at zero the caution
    /// is noise — and the search that produced it still ends, having done the map
    /// work it was armed for. Same state, same evidence, the one bit flipped.
    #[test]
    fn undo_switched_off_silences_the_caution_and_not_the_search() {
        let mut m = Mapper::default();
        walked(&mut m);

        let mut off = armed_state();
        off.config.guidance = true;
        exhausted_with_undo(&mut off, true, false);
        pump_return_search(&mut off, &m);
        assert!(off.return_search.is_none(), "the search still ends");
        assert!(said(&off).is_empty(), "nothing the player could act on: {:?}", said(&off));

        let mut on = armed_state();
        on.config.guidance = true;
        exhausted_with_undo(&mut on, true, true);
        pump_return_search(&mut on, &m);
        assert_eq!(
            said(&on).last().map(String::as_str),
            Some("looks one-way — no direction leads back to Behind House"),
            "and with undo available the very same search speaks"
        );
    }

    /// The caution's OWN switch (SQ-1043's follow-up): off is silence, and the
    /// probe it reads is untouched — the map still closes its gaps.
    #[test]
    fn the_cautions_own_switch_silences_the_line_and_not_the_probe() {
        let mut m = Mapper::default();
        walked(&mut m);
        let mut state = armed_state();
        state.config.guidance = true;
        assert!(state.config.one_way_caution, "on out of the box");
        state.config.one_way_caution = false;
        exhausted(&mut state, true);

        pump_return_search(&mut state, &m);
        assert!(said(&state).is_empty(), "the player declined the line");
        assert!(state.config.return_probe, "and kept the search that feeds it");
    }

    /// The live session is what is asked, and `Some(0)` — the documented off
    /// value of `undo_levels` — is what silences it. An engine that answers
    /// `None` (no host-settable cap at all) is not "undo off".
    #[test]
    fn the_undo_bit_is_read_off_the_running_session_when_the_search_arms() {
        let mut m = Mapper::default();
        walked(&mut m);

        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1));
        assert!(
            state.return_search.as_ref().expect("armed").is_undoable(),
            "Scott answers None — no host cap — which must not read as undo off"
        );

        let bytes = include_bytes!("../../zvm/tests/fixtures/minizork.z3").to_vec();
        let mut z = crate::session::GameSession::new(bytes, true, false, None)
            .expect("minizork boots");
        z.machine.undo_cap = 0;
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &z, "enter window", Some(1));
        assert!(
            !state.return_search.as_ref().expect("armed").is_undoable(),
            "a machine capped at 0 has undo switched off"
        );

        z.machine.undo_cap = 16;
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &z, "enter window", Some(1));
        assert!(
            state.return_search.as_ref().expect("armed").is_undoable(),
            "and the same machine with a cap does not"
        );
    }

    /// A room the map cannot name still gets a caution — the fact it states does
    /// not depend on the name, and a blank one must not become `back to `.
    #[test]
    fn an_unnamed_origin_drops_the_clause_rather_than_trailing_it() {
        let mut m = Mapper::default();
        walked(&mut m);
        m.graph.upsert_room(1, String::new());
        let mut state = armed_state();
        state.config.guidance = true;
        exhausted(&mut state, true);

        pump_return_search(&mut state, &m);
        assert_eq!(said(&state).last().unwrap(), "looks one-way — no direction leads back");
    }

    /// Every answered attempt is marked before anything is judged, so a search
    /// that is aborted mid-way resumes from where it stopped rather than
    /// re-walking ground it has covered.
    #[test]
    fn an_answered_attempt_is_durable_and_the_next_search_resumes() {
        let mut m = Mapper::default();
        walked(&mut m);
        m.graph.mark_probed(2, Direction::Out); // an earlier search got this far
        m.graph.mark_probed(2, Direction::N);
        let mut state = armed_state();
        arm_return_search(&mut state, &m, &blind(), "enter window", Some(1));
        let s = state.return_search.as_ref().expect("still worth asking");
        assert_eq!(s.remaining(), 10, "the two already walked are not offered again");
    }
}
