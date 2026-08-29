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
//! and asked to walk one direction. If it comes out in the room the player just
//! left, that passage is real and goes on the map. If it comes out anywhere
//! else, nothing at all is recorded.
//!
//! # Success is room identity, not a room
//!
//! **Landing somewhere is not landing back.** A probe that walks into some third
//! room C and is counted as a success draws an edge that does not exist — which
//! is the exact failure the whole design is arranged around. So the test is
//! `step.location == Some(origin)`: the mapper's own location detection, the same
//! `snap.number` [`crate::session::apply_turn`] keys rooms by, compared against
//! the room the player came from.
//!
//! That is also why this consumer needs none of the probe seam's
//! [`crate::probe::Refusals`] machinery. A vocabulary offer has to read the
//! story's prose to find out whether anything happened, because "did this verb
//! do something" is only answerable in words. "Am I back where I started" is
//! answerable in a room number.
//!
//! **And a probe that lands in the wrong room records the ATTEMPT and nothing
//! else.** Not room C, not the edge to it, not its existence. The map is a record
//! of what the PLAYER has seen, and keeping C "known but hidden" would leak
//! straight back out through the layout, the pathfinder and click-to-route.
//! Total failure likewise says nothing about the map: it proves only that these
//! directions did not work from here, this time. A door may need opening, and a
//! one-way passage is a real and beloved part of these games.
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

use mapper::direction::{long_label, Direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

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
    queue.reverse(); // popped from the back, so the best candidate goes last
    state.return_search = Some(ReturnSearch { origin, here, queue, attempt: None, from });
}

/// Hand the next candidate to the worker, if there is one and the shadow is
/// free. Called every pass of the event loop; returns true when something was
/// asked (nothing to redraw, but the caller may want to know).
///
/// The shadow is shared with the vocabulary offer and holds one question at a
/// time, so "busy" is an ordinary outcome and simply means try again next pass.
pub fn pump_return_search(state: &mut AppState) -> bool {
    let Some(search) = &state.return_search else { return false };
    if search.attempt.is_some() {
        return false; // one out already
    }
    let Some(&dir) = search.queue.last() else {
        // Nothing left to try. Total failure records nothing about the map: a
        // door may need opening, and a one-way passage is a real answer.
        state.return_search = None;
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
/// Three outcomes, in the order they are decided:
///
/// 1. **The attempt is recorded as probed, whatever it found.** First, and
///    unconditionally, so an abort a moment later still leaves the search one
///    step further along than it was.
/// 2. **It came out in the room the player left** — the passage is real, and
///    goes on the map through the same call a walked crossing makes.
/// 3. **Anything else** — a different room, nowhere at all, a death, a story
///    that ended, an engine that cannot say where it is — and nothing is
///    recorded but the attempt. The search moves on to the next direction.
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

    // (2) Did it land back where the player came from? Room identity, and
    // nothing else — a step that ended the story or reached for a file answers
    // nothing about the map, whatever `location` happens to hold.
    let landed_home = answer.run.as_ref().is_some_and(|run| {
        run.steps
            .first()
            .is_some_and(|s| !s.quit && !s.escaped && s.location == Some(origin))
    });
    if !landed_home {
        // (3) Wrong room, or no room. Nothing about C is recorded: not the room,
        // not the edge, not that it exists.
        return None;
    }

    let passage = ProbedPassage { from: here, dir: attempt.dir, to: origin };
    // The search is over whether or not the graph takes the edge — a passage the
    // player walked back themselves while this was in flight is the better
    // authority, and there is no gap left to close either way.
    state.return_search = None;
    mapper.record_probed_passage(passage).then_some(passage)
}

/// Run a search to its end, waiting for each answer instead of collecting one
/// that has already arrived — the whole search in one call.
///
/// **Not for the event loop.** It is what a test harness and a measurement
/// harness need: the answer without racing the thread, and the shadow's own
/// `probes`/`spent` counters left holding the cost of the whole search.
///
/// Returns the passage if one was found. Bounded by the candidate list, so it
/// terminates whatever the story does; a shadow that will not answer at all ends
/// it by breaking the seam, which [`crate::probe::ShadowProbe::settle`] reports
/// as `None`.
pub fn settle_return_search(state: &mut AppState, mapper: &mut Mapper) -> Option<ProbedPassage> {
    while state.return_search.is_some() {
        if !pump_return_search(state) {
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
        if let Some(passage) = deliver(state, mapper, &answer) {
            return Some(passage);
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
