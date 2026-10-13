//! Which ways lead OUT of this room? Ask the game, in the shadow, the first time you are in it
//! (SQ-1773).
//!
//! ```text
//! West of House
//! > (nothing typed)
//! ```
//!
//! …and the map already knows that `north`, `south` and `west` work from here, that `east` is
//! boarded up, and that nothing else does -- without anybody having typed any of them, and
//! without drawing, naming or even recording the rooms beyond. The matrix and the room card then
//! mark the three real exits `»` instead of listing every direction nobody has typed as a maybe
//! (`·`). Rooms nobody has scanned (the setting off, the scan not finished, an engine with no
//! shadow) keep the old reading: [`mapper::graph::MapGraph::untried`] is every compass direction
//! the player has not typed.
//!
//! # What a scan asks
//!
//! From the state the player is standing in at the end of the turn that put them in the room
//! ([`crate::state::AppState::random_exit_pre_move_save`]), each of `n s e w ne nw se sw u d in
//! out` plus the ship words the story's own dictionary holds
//! ([`crate::relocation_probe::nautical_words`]), every one from a freshly restored state
//! ([`crate::probe::ShadowProbe::ask_from`]). Only the room change is read -- **never the
//! refusal text**: "The windows are all boarded" and "You can't go that way" are both just a
//! step that moved nobody.
//!
//! # The guards, and the failures they exist for
//!
//! **The `look` control.** The first command of the first question is `look`, from the same
//! state. If it does not leave the shadow in the room being scanned, the moment is unstable (a
//! timed event is carrying the player off whatever they type, as in [`crate::relocation_probe`]),
//! every direction would be "answered" by something other than the direction. **Nothing is
//! recorded for the scan**, not even a wall, so the room stays unscanned and is simply tried
//! again. Only a `look` that PROVABLY lands in another room (or quits, escapes, dies) rejects:
//! an unknown location is accepted, because the shadow restores with a blank status line
//! (SQ-0785) and a v4+ story that repaints only changed fields reports no room after `look`.
//!
//! **The confirm replay.** Every word that landed somewhere is replayed once more, alone, from
//! the same state, and must land in the same room. A random exit that fires on one restore would
//! otherwise be filed as a fixed passage on a coin flip. A disagreement is not an exit; the
//! direction is kept as a wall, so a later visit asks again.
//!
//! **A self-loop is an exit, not a wall.** Some exits lead back into the room they leave -- every
//! way out of a maze room, three of the four exits of Adventureland's start forest -- and the
//! shadow cannot tell one from a refusal by the room alone. So when a direction leaves the shadow
//! in the SAME room, the story's own declared exit ([`crate::engine::Engine::declared_exit`],
//! read from the live engine when the scan is queued) is consulted: `Room(this room)` makes it a
//! real exit, anything else keeps it a wall. Refusal text is never read. It is recorded as a scan
//! exit only -- no passage, and nothing that says it loops -- so the map shows `»` like any other
//! unwalked exit and a maze stays a puzzle.
//!
//! **A death is not a landing** ([`crate::probe::ProbeStep::landing`], SQ-1506): the troll that
//! kills the shadow wakes it in the Forest, a room the map holds, and that is not where `north`
//! leads.
//!
//! # What a scan records, and the no-reveal rule
//!
//! * **A landing in a room the map already holds** is recorded exactly as
//!   [`crate::return_probe`] records one: a verified passage
//!   ([`mapper::mapper::Mapper::record_probed_passage`]) and the direction marked `probed`. It
//!   is **not** marked tried -- the player did not walk it ([`mapper::graph::Room::tried`] is
//!   theirs and must not be written by a shadow; see the return probe's "Two records" section).
//! * **A landing in a room the map does NOT hold** records only that this direction is an exit
//!   ([`mapper::graph::Room::scan_exits`]). The destination's id, name and existence stay out of
//!   the map: it is a record of what the player has seen, and an exit stub is the whole of what
//!   a scan may say about the far side. Walking it later is an ordinary first walk. The landing
//!   id is kept HIDDEN beside the stub ([`mapper::graph::Room::scan_landings`], SQ-1775): never
//!   shown, named, or used to create the far room, only compared when a room first enters the
//!   map -- by a walk, a relocation or a chain replay -- at which point every stub that landed
//!   there becomes a verified passage, as above, so reaching the room by another way does not
//!   leave `»` standing beside it. (A self-loop stub has no landing.)
//! * **No room change** is not an exit at present; the direction goes on
//!   [`mapper::graph::Room::scan_walls`].
//!
//! The scan's results are a third record beside `tried` (the player's) and `probed` (the return
//! search's), and persist with the rooms.
//!
//! # Revisits
//!
//! On a later arrival only the directions that FAILED are asked again (a door opened, a puzzle
//! was solved), from the new moment. A direction that was an exit stays one. Directions that
//! already carry an edge are never asked.
//!
//! # Priority, and staleness
//!
//! A scan is the lowest priority user of the shadow. It is asked only when no chain replay,
//! relocation search or return search is pending ([`pump_exit_scan`]), and in small questions
//! ([`CHUNK`] words) so the shadow is free again quickly when one of those arrives. Scans queue
//! (at most [`MAX_QUEUED`], newest kept) rather than racing. An answer is applied only if its
//! room is still on the map -- a rewind that took the room away drops it -- and the `look`
//! control additionally proves the snapshot was of that room.
//!
//! Gated by [`crate::config::Config::scan_exits`] and by the shadow being armed; independent of
//! `return_probe`.

use mapper::direction::{Direction, long_label};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

use crate::engine::Engine;
use crate::state::AppState;

/// Words asked per question. Small, so a more urgent search never waits long behind a scan.
pub const CHUNK: usize = 4;

/// Scans waiting their turn; the oldest is dropped past this.
pub const MAX_QUEUED: usize = 6;

/// Where one word took the shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// A room the map holds: a passage can be recorded.
    Known(RoomId),
    /// A room the map does not hold. The id exists only to compare the confirm replay against;
    /// it is session state and never reaches the graph.
    Unknown(RoomId),
    /// The shadow stayed in this very room AND the story's own exit table declares this direction
    /// a fixed exit back into it: a self-loop (a maze room), a real exit that changes no room.
    SelfLoop,
}

impl Landing {
    fn room(self) -> RoomId {
        match self {
            Landing::Known(r) | Landing::Unknown(r) => r,
            Landing::SelfLoop => RoomId::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// `look`, then every word.
    Wide,
    /// Each word that landed, once more.
    Confirm,
}

/// A scan of one room, in progress. Session state, never persisted; what it learns lives on the
/// room.
#[derive(Debug)]
pub struct ExitScan {
    room: RoomId,
    /// The game as it stood when the player was in the room.
    from: crate::probe::ProbeSnapshot,
    /// Slots the story's own exit table declares a fixed exit back into this room (read from the
    /// live engine when the scan was queued: the shadow cannot be asked, and the table is static).
    loops: Vec<Direction>,
    /// Every direction slot this scan decides (a slot may have several words).
    slots: Vec<Direction>,
    /// Words still to ask in the wide stage.
    todo: Vec<(&'static str, Direction)>,
    /// What the wide stage found.
    landed: Vec<(&'static str, Direction, Landing)>,
    /// Words still to replay in the confirm stage.
    confirm_todo: Vec<(&'static str, Direction, Landing)>,
    /// What the confirm stage verified, one per word.
    confirmed: Vec<(Direction, Landing)>,
    stage: Stage,
    /// Whether the `look` control has been answered.
    controlled: bool,
    /// The words out with the worker, with the landing the confirm stage expects of each.
    in_flight: Vec<(&'static str, Direction, Option<Landing>)>,
    token: Option<u64>,
}

impl ExitScan {
    /// The room being scanned.
    pub fn room(&self) -> RoomId {
        self.room
    }

    /// The direction slots this scan decides, for tests and diagnostics.
    pub fn slots(&self) -> &[Direction] {
        &self.slots
    }

    /// Words not yet asked in the wide stage, for tests and diagnostics.
    pub fn remaining(&self) -> usize {
        self.todo.len()
    }

    /// The commands the next question types.
    fn commands(&self) -> Vec<String> {
        match self.stage {
            Stage::Wide => {
                let take = if self.controlled { CHUNK } else { CHUNK - 1 };
                let mut c: Vec<String> = Vec::new();
                if !self.controlled {
                    c.push("look".to_string());
                }
                c.extend(self.todo.iter().take(take).map(|(w, _)| w.to_string()));
                c
            }
            Stage::Confirm => self.confirm_todo.iter().take(CHUNK).map(|(w, ..)| w.to_string()).collect(),
        }
    }
}

/// Queue a scan for the room the player has just arrived in (or is standing in unscanned),
/// if this turn earned one (SQ-1773).
///
/// Called at the end of a finished turn, once `random_exit_pre_move_save` holds the state the
/// player is standing in. See the module docs for what is asked.
pub fn arm_exit_scan(state: &mut AppState, mapper: &Mapper, live: &dyn Engine, room_before: Option<RoomId>) {
    if !state.config.scan_exits || !state.probe.is_armed() {
        return;
    }
    let Some(here) = mapper.graph.current() else { return };
    if mapper.graph.room(here).is_none() {
        return;
    }
    let arrived = room_before != Some(here);
    let scanned = mapper.graph.is_scanned(here);
    // What the map shows as blocked here: the scan's walls plus anything typed and refused
    // (SQ-1783). A scanned room re-asks all of them, never exits or edges.
    let mut walls = mapper.graph.scan_walls(here);
    for d in mapper::matrix::MATRIX_DIRS {
        if !walls.contains(&d) && mapper::matrix::classify(&mapper.graph, here, d) == mapper::matrix::MatrixCell::Probed {
            walls.push(d);
        }
    }
    let wanted = if arrived {
        !scanned || !walls.is_empty()
    } else if scanned {
        // Standing still in a scanned room: a turn spent here may have opened a door, so look
        // again at what is blocked -- unless a look at this room is already queued or in flight.
        !walls.is_empty()
            && state.scan_search.as_ref().is_none_or(|s| s.room != here)
            && !state.scan_queue.iter().any(|s| s.room == here)
    } else {
        // Standing still, never scanned: only a room nobody has tried this session (the room the
        // game began in, a map restored from before scans existed, the setting just switched on).
        !state.scan_attempted.contains(&here)
    };
    if !wanted {
        return;
    }
    // The shadow is asked a typed command, so the story must be waiting for a line.
    if live.is_saveload_pending() || live.pending_input() != crate::session::InputKind::Line {
        return;
    }
    let Some((saved_room, save)) = &state.random_exit_pre_move_save else { return };
    if *saved_room != here {
        return;
    }
    let from = crate::probe::ProbeSnapshot::from_save(live, std::sync::Arc::clone(save));
    let edges: Vec<Direction> = mapper
        .graph
        .connections()
        .iter()
        .filter(|c| c.origin == here)
        .map(|c| c.dir)
        .collect();
    let slots: Vec<Direction> = mapper::matrix::MATRIX_DIRS
        .iter()
        .copied()
        .filter(|d| !edges.contains(d) && (!scanned || walls.contains(d)))
        .collect();
    if slots.is_empty() {
        return;
    }
    let nautical = if scanned { Vec::new() } else { crate::relocation_probe::nautical_words(state, live) };
    if !scanned {
        state.scan_attempted.push(here);
    }
    let loops: Vec<Direction> = slots
        .iter()
        .copied()
        .filter(|&d| live.declared_exit(here, d) == crate::engine::DeclaredExit::Room(here))
        .collect();
    queue_scan(state, here, from, slots, nautical, loops);
}

/// Queue a scan of `room` over `slots`, from `from`. The words are each slot's compass word,
/// then the story's ship words that fill that slot.
pub(crate) fn queue_scan(
    state: &mut AppState,
    room: RoomId,
    from: crate::probe::ProbeSnapshot,
    slots: Vec<Direction>,
    nautical: Vec<(&'static str, Direction)>,
    loops: Vec<Direction>,
) {
    let mut todo: Vec<(&'static str, Direction)> = slots.iter().map(|&d| (long_label(d), d)).collect();
    todo.extend(nautical.into_iter().filter(|(_, d)| slots.contains(d)));
    // A newer look at the same room replaces an older one that has not started.
    state.scan_queue.retain(|s| s.room != room);
    state.scan_queue.push(ExitScan {
        room,
        from,
        loops,
        slots,
        todo,
        landed: Vec::new(),
        confirm_todo: Vec::new(),
        confirmed: Vec::new(),
        stage: Stage::Wide,
        controlled: false,
        in_flight: Vec::new(),
        token: None,
    });
    while state.scan_queue.len() > MAX_QUEUED {
        state.scan_queue.remove(0);
    }
}

/// True when something more urgent than a scan holds the shadow or is about to.
fn others_pending(state: &AppState) -> bool {
    state.chain_replay.is_some()
        || state.relocation_search.is_some()
        || !state.relocation_queue.is_empty()
        || state.return_search.is_some()
        || state.random_exit_search.is_some()
}

/// Hand the scan's next question to the worker if nothing else wants the shadow. Called every
/// pass of the event loop AFTER the other searches' pumps. Returns true when something was asked.
pub fn pump_exit_scan(state: &mut AppState, mapper: &Mapper) -> bool {
    if !state.probe.is_armed() || !state.config.scan_exits {
        state.scan_search = None;
        state.scan_queue.clear();
        return false;
    }
    if state.scan_search.as_ref().is_some_and(|s| s.token.is_some()) {
        return false;
    }
    if others_pending(state) {
        return false; // the player's own moves come first
    }
    // Drop scans of rooms a rewind took away before spending a question on them.
    state.scan_queue.retain(|s| mapper.graph.room(s.room).is_some());
    if state.scan_search.is_none() {
        if state.scan_queue.is_empty() {
            return false;
        }
        state.scan_search = Some(state.scan_queue.remove(0));
    }
    let Some(scan) = &state.scan_search else { return false };
    if mapper.graph.room(scan.room).is_none() {
        state.scan_search = None;
        return false;
    }
    let Some(token) = state.probe.ask_from(&scan.from, &scan.commands()) else {
        return false; // busy: ask again next pass
    };
    if let Some(scan) = &mut state.scan_search {
        scan.token = Some(token);
        let words: Vec<_> = match scan.stage {
            Stage::Wide => scan
                .todo
                .iter()
                .take(if scan.controlled { CHUNK } else { CHUNK - 1 })
                .map(|&(w, d)| (w, d, None))
                .collect(),
            Stage::Confirm => scan
                .confirm_todo
                .iter()
                .take(CHUNK)
                .map(|&(w, d, l)| (w, d, Some(l)))
                .collect(),
        };
        scan.in_flight = words;
    }
    true
}

/// True when `token` answers the scan's question.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.scan_search.as_ref().and_then(|s| s.token) == Some(token)
}

/// Read one answer back (SQ-1773). Returns true when the map changed.
///
/// * **Wide.** The first question's first step is the `look` control; unless it leaves the shadow
///   in the scanned room the scan is dropped and records nothing. Each word's landing is noted:
///   a room other than this one is a candidate, and whether the map holds it decides what a
///   confirmed landing records.
/// * **Confirm.** A word must land in the same room again.
/// * When the last confirm chunk is read (or nothing landed), the results are applied: see
///   [`finish`].
pub fn deliver(state: &mut AppState, mapper: &mut Mapper, answer: &crate::probe::Answer) -> bool {
    if !owns(state, answer.token) {
        return false;
    }
    let Some(mut scan) = state.scan_search.take() else { return false };
    scan.token = None;
    let in_flight = std::mem::take(&mut scan.in_flight);
    let Some(run) = &answer.run else { return false }; // a broken shadow: no evidence either way
    let controls = usize::from(scan.stage == Stage::Wide && !scan.controlled);
    if run.steps.len() != controls + in_flight.len() {
        return false; // a torn run
    }
    if mapper.graph.room(scan.room).is_none() {
        return false; // the room is gone: the map was rewound
    }
    if controls == 1 {
        // The control is rejected only when `look` provably lands elsewhere (or the run ended).
        // An unknown location is stable: the shadow restores with a blank status line and some
        // v4+ stories repaint only the fields that changed, so `look` reports no room at all.
        let stayed = run.steps[0].location.is_none_or(|l| l == scan.room)
            && !run.steps[0].quit
            && !run.steps[0].escaped
            && !run.steps[0].died;
        if !stayed {
            return false;
        }
        scan.controlled = true;
    }
    match scan.stage {
        Stage::Wide => {
            for (i, &(word, dir, _)) in in_flight.iter().enumerate() {
                scan.todo.remove(0);
                match run.steps[controls + i].landing() {
                    Some(r) if r != scan.room => {
                        let l = if mapper.graph.room(r).is_some() { Landing::Known(r) } else { Landing::Unknown(r) };
                        scan.landed.push((word, dir, l));
                    }
                    // No room change is a wall -- unless the story's own table says this
                    // direction is a fixed exit back into the room. Never the refusal text.
                    Some(_) if scan.loops.contains(&dir) => scan.landed.push((word, dir, Landing::SelfLoop)),
                    _ => {}
                }
            }
            if scan.todo.is_empty() {
                scan.confirm_todo = std::mem::take(&mut scan.landed);
                scan.stage = Stage::Confirm;
            }
        }
        Stage::Confirm => {
            for (i, &(_, dir, expect)) in in_flight.iter().enumerate() {
                scan.confirm_todo.remove(0);
                let again = run.steps[i].landing();
                if let (Some(expect), Some(r)) = (expect, again) {
                    let same = match expect {
                        Landing::SelfLoop => r == scan.room,
                        other => r == other.room(),
                    };
                    if same {
                        scan.confirmed.push((dir, expect));
                    }
                }
            }
        }
    }
    if scan.stage == Stage::Confirm && scan.confirm_todo.is_empty() {
        return finish(mapper, &scan);
    }
    state.scan_search = Some(scan);
    false
}

/// Apply a finished scan to the map: verified passages for landings on known rooms, exit stubs
/// for the rest, walls for every other direction asked. See the module docs.
fn finish(mapper: &mut Mapper, scan: &ExitScan) -> bool {
    let mut changed = false;
    let mut exits: Vec<Direction> = Vec::new();
    for &slot in &scan.slots {
        let Some(&(_, landing)) = scan.confirmed.iter().find(|(d, _)| *d == slot) else { continue };
        match landing {
            Landing::Known(to) => {
                mapper.graph.mark_probed(scan.room, slot);
                changed |= mapper.record_probed_passage(ProbedPassage { from: scan.room, dir: slot, to });
            }
            // Both are exits with nothing known beyond them. A self-loop is deliberately drawn
            // like any other unwalked exit (`»`), so a maze stays a puzzle; walking it is what
            // turns the cell into `↩`.
            Landing::Unknown(_) | Landing::SelfLoop => exits.push(slot),
        }
    }
    changed |= mapper.graph.record_scan(scan.room, &scan.slots, &exits);
    // The landing id is kept, hidden, so the stub becomes a passage the moment the map holds
    // that room by any route (SQ-1775). A self-loop has no landing to keep.
    for &slot in &exits {
        if let Some(&(_, Landing::Unknown(r))) = scan.confirmed.iter().find(|(d, _)| *d == slot) {
            mapper.graph.note_scan_landing(scan.room, slot, r);
        }
    }
    changed
}

/// Run every scan to its end, waiting for each answer -- for a test or measurement harness,
/// never the event loop. Returns whether the map changed.
pub fn settle_exit_scan(state: &mut AppState, mapper: &mut Mapper) -> bool {
    let mut changed = false;
    while state.scan_search.is_some() || !state.scan_queue.is_empty() {
        if !pump_exit_scan(state, mapper)
            && state.scan_search.as_ref().is_none_or(|s| s.token.is_none())
        {
            break; // nothing was asked: refused, or the scan was dropped
        }
        let Some(answer) = state.probe.settle() else { break };
        changed |= deliver(state, mapper, &answer);
    }
    changed
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::probe::{ProbeRun, ProbeStep, WorldPrint, test_answer};

    fn step(command: &str, location: Option<RoomId>, died: bool) -> ProbeStep {
        ProbeStep {
            command: command.into(),
            reply: String::new(),
            location,
            world: WorldPrint::default(),
            quit: false,
            escaped: false,
            died,
            room_name: None,
            after: None,
        }
    }

    fn run(steps: Vec<ProbeStep>) -> Option<ProbeRun> {
        Some(ProbeRun { baseline: WorldPrint::default(), steps })
    }

    fn live() -> crate::scott_session::ScottSession {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        crate::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads")
    }

    fn snapshot() -> crate::probe::ProbeSnapshot {
        let live = live();
        crate::probe::ProbeSnapshot::from_save(&live, std::sync::Arc::new(live.save_state()))
    }

    /// A map holding room 1 (the room being scanned), plus room 2 when `known` says so.
    fn map(known: bool) -> Mapper {
        let mut m = Mapper::default();
        // `upsert_room`, not `observe`: two observed rooms in a row are joined by a walked edge.
        m.graph.upsert_room(1, "Here".into());
        if known {
            m.graph.upsert_room(2, "Elsewhere".into());
        }
        m
    }

    /// A scan of room 1 over north and up, its first question out under token 7:
    /// `look`, `north`, `up`.
    fn state_scanning() -> AppState {
        let mut state = AppState::default();
        queue_scan(&mut state, 1, snapshot(), vec![Direction::N, Direction::Up], Vec::new(), Vec::new());
        let mut scan = state.scan_queue.remove(0);
        scan.token = Some(7);
        scan.in_flight = vec![("north", Direction::N, None), ("up", Direction::Up, None)];
        state.scan_search = Some(scan);
        state
    }

    /// `look` stays, `north` goes nowhere, `up` lands in `up_to`.
    fn wide(look_at: Option<RoomId>, up_to: RoomId, died: bool) -> Option<ProbeRun> {
        run(vec![step("look", look_at, false), step("north", Some(1), false), step("up", Some(up_to), died)])
    }

    fn ask_confirm(state: &mut AppState) {
        let scan = state.scan_search.as_mut().expect("still scanning");
        assert_eq!(scan.stage, Stage::Confirm);
        scan.token = Some(7);
        scan.in_flight = scan.confirm_todo.iter().map(|&(w, d, l)| (w, d, Some(l))).collect();
    }

    /// The timed event: `look` alone carries the shadow elsewhere, so no direction's answer is
    /// about the direction. Nothing is recorded -- not even a wall -- and the scan ends.
    #[test]
    fn the_look_control_rejects_a_moment_that_is_not_stable() {
        let mut state = state_scanning();
        let mut m = map(true);
        assert!(!deliver(&mut state, &mut m, &test_answer(7, wide(Some(2), 2, false))));
        assert!(state.scan_search.is_none(), "the scan is over");
        assert!(!m.graph.is_scanned(1), "and the room is not marked scanned");
        assert!(m.graph.scan_walls(1).is_empty(), "no walls either");
    }

    /// An unknown location (blank status line, SQ-0785) is not a provable move: the scan goes on.
    #[test]
    fn a_look_that_reports_no_location_is_accepted_as_stable() {
        let mut state = state_scanning();
        let mut m = map(false);
        assert!(!deliver(&mut state, &mut m, &test_answer(7, wide(None, 2, false))));
        let scan = state.scan_search.as_ref().expect("still scanning");
        assert_eq!(scan.commands(), vec!["up".to_string()]);
    }

    /// Falsifier for the test above: the same answer with a `look` that stays goes on to the
    /// confirm stage with `up` as the only candidate.
    #[test]
    fn a_stable_moment_goes_on_to_confirm_the_landing() {
        let mut state = state_scanning();
        let mut m = map(false);
        assert!(!deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false))));
        let scan = state.scan_search.as_ref().expect("still scanning");
        assert_eq!(scan.commands(), vec!["up".to_string()]);
        assert!(!m.graph.is_scanned(1), "nothing is recorded until the replay agrees");
    }

    /// An exit into a room the map does not hold: the direction is recorded, the room is not.
    #[test]
    fn a_confirmed_landing_in_an_unseen_room_records_the_exit_and_nothing_else() {
        let mut state = state_scanning();
        let mut m = map(false);
        deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false)));
        ask_confirm(&mut state);
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(vec![step("up", Some(2), false)]))));
        assert!(state.scan_search.is_none());
        assert!(m.graph.is_scanned(1));
        assert_eq!(m.graph.room(1).unwrap().scan_exits, vec![Direction::Up]);
        assert_eq!(m.graph.scan_walls(1), vec![Direction::N], "north moved nobody");
        assert!(m.graph.room(2).is_none(), "the far room is not on the map");
        assert_eq!(m.graph.rooms().count(), 1);
        assert!(m.graph.connections().is_empty(), "and no edge was minted");
        assert!(m.graph.room(1).unwrap().tried.is_empty(), "the player tried nothing");
        assert_eq!(m.graph.untried(1), vec![Direction::Up], "the real exit is what is unexplored");
        assert_eq!(m.graph.room(1).unwrap().scan_landings, vec![(Direction::Up, 2)], "kept hidden");
    }

    /// SQ-1775: reaching the stub's far room another way turns the stub into a passage.
    #[test]
    fn mapping_the_landing_room_later_upgrades_the_stub_to_a_passage() {
        let mut state = state_scanning();
        let mut m = map(false);
        deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false)));
        ask_confirm(&mut state);
        deliver(&mut state, &mut m, &test_answer(7, run(vec![step("up", Some(2), false)])));
        m.observe(2, "Elsewhere", Some(Direction::E));
        let c = m.graph.connections();
        assert!(c.iter().any(|c| (c.origin, c.dir, c.dest) == (1, Direction::Up, 2)), "{c:?}");
        assert!(m.graph.is_probed(1, Direction::Up));
        assert!(!m.graph.room(1).unwrap().tried.contains(&Direction::Up));
        assert!(m.graph.room(1).unwrap().scan_exits.is_empty());
    }

    /// A landing in a room the map holds is a verified passage, never marked tried.
    #[test]
    fn a_confirmed_landing_in_a_known_room_records_the_passage() {
        let mut state = state_scanning();
        let mut m = map(true);
        deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false)));
        ask_confirm(&mut state);
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(vec![step("up", Some(2), false)]))));
        let c = m.graph.connections();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].origin, c[0].dir, c[0].dest), (1, Direction::Up, 2));
        assert!(m.graph.room(1).unwrap().tried.is_empty(), "the shadow's walk is not the player's");
        assert!(m.graph.is_probed(1, Direction::Up));
        assert!(m.graph.room(1).unwrap().scan_exits.is_empty(), "an edge, not a stub");
    }

    /// The confirm replay: landing elsewhere the second time means the first was luck.
    #[test]
    fn a_confirmation_that_disagrees_is_not_an_exit() {
        let mut state = state_scanning();
        let mut m = map(false);
        deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false)));
        ask_confirm(&mut state);
        deliver(&mut state, &mut m, &test_answer(7, run(vec![step("up", Some(1), false)])));
        assert!(m.graph.is_scanned(1));
        assert!(m.graph.room(1).unwrap().scan_exits.is_empty());
        assert_eq!(m.graph.scan_walls(1), vec![Direction::N, Direction::Up], "kept to be asked again");
    }

    /// A shadow that died woke up somewhere: that is a resurrection, not an arrival.
    #[test]
    fn a_death_is_not_a_landing() {
        let mut state = state_scanning();
        let mut m = map(true);
        deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, true)));
        assert!(state.scan_search.is_none(), "nothing landed, so nothing to confirm");
        assert!(m.graph.connections().is_empty());
        assert!(m.graph.room(1).unwrap().scan_exits.is_empty());
        assert_eq!(m.graph.scan_walls(1), vec![Direction::N, Direction::Up]);
    }

    /// A direction that leaves the shadow in the SAME room is a wall -- unless the story's own exit
    /// table declares it a fixed exit back into the room (a self-loop): then it is an exit, with
    /// no passage and no hint that it loops, so a maze stays a puzzle.
    #[test]
    fn a_declared_self_loop_is_an_exit_and_an_undeclared_stay_is_a_wall() {
        for declared in [true, false] {
            let mut state = AppState::default();
            let loops = if declared { vec![Direction::N] } else { Vec::new() };
            queue_scan(&mut state, 1, snapshot(), vec![Direction::N, Direction::Up], Vec::new(), loops);
            let mut scan = state.scan_queue.remove(0);
            scan.token = Some(7);
            scan.in_flight = vec![("north", Direction::N, None), ("up", Direction::Up, None)];
            state.scan_search = Some(scan);
            let mut m = map(false);
            // look stays; north stays; up stays: nothing moved anywhere.
            deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 1, false)));
            if declared {
                ask_confirm(&mut state);
                deliver(&mut state, &mut m, &test_answer(7, run(vec![step("north", Some(1), false)])));
            }
            assert!(state.scan_search.is_none());
            let want = if declared { vec![Direction::N] } else { Vec::new() };
            assert_eq!(m.graph.room(1).unwrap().scan_exits, want, "declared={declared}");
            assert!(m.graph.connections().is_empty(), "never a passage: it must not hint at the loop");
            assert!(!m.graph.scan_walls(1).contains(&Direction::N) || !declared);
            assert!(m.graph.scan_walls(1).contains(&Direction::Up), "up is declared nowhere: a wall");
        }
    }

    /// A rewind took the room away between asking and answering: nothing to record.
    #[test]
    fn a_stale_answer_for_a_room_the_map_lost_is_dropped() {
        let mut state = state_scanning();
        let mut m = Mapper::default();
        m.observe(5, "Somewhere else", None);
        assert!(!deliver(&mut state, &mut m, &test_answer(7, wide(Some(1), 2, false))));
        assert!(state.scan_search.is_none());
        assert!(!m.graph.is_scanned(5));
    }

    /// Someone else's answer is not ours to read.
    #[test]
    fn an_answer_for_another_question_is_ignored() {
        let mut state = state_scanning();
        let mut m = map(false);
        assert!(!deliver(&mut state, &mut m, &test_answer(8, wide(Some(1), 2, false))));
        assert!(state.scan_search.is_some());
    }

    /// The lowest priority: a relocation search pending means the scan waits; with none it asks.
    #[test]
    fn a_scan_waits_behind_the_searches_the_players_moves_need() {
        let mut state = AppState::default();
        state.probe.arm(crate::probe::ShadowRecipe {
            catalogue: None,
            story_bytes: std::sync::Arc::new(include_bytes!("../../scott/tests/tiny_cave.dat").to_vec()),
            store: std::path::PathBuf::new(),
            vfs_bytes: std::sync::Arc::new(Vec::new()),
            honor_game_colours: true,
            interpreter_number: None,
            random_seed: None,
            acceleration: true,
            screen: (80, 24),
        });
        let m = map(false);
        queue_scan(&mut state, 1, snapshot(), vec![Direction::N], Vec::new(), Vec::new());
        crate::relocation_probe::queue_search(&mut state, 1, 2, snapshot(), Vec::new(), false);
        assert!(!pump_exit_scan(&mut state, &m), "a relocation search is queued first");
        assert!(state.scan_search.is_none() && state.scan_queue.len() == 1, "and the scan has not started");
        state.relocation_queue.clear();
        assert!(pump_exit_scan(&mut state, &m), "with nothing else pending it asks");
        assert!(state.scan_search.as_ref().is_some_and(|s| s.token.is_some()));
        let _ = state.probe.settle();
    }

    /// Scans queue instead of racing, the oldest dropped past the cap, and a newer scan of the
    /// same room replaces an older one that has not started.
    #[test]
    fn the_queue_is_bounded_and_keeps_the_newest_look_at_a_room() {
        let mut state = AppState::default();
        for room in 0..(MAX_QUEUED as RoomId + 3) {
            queue_scan(&mut state, room, snapshot(), vec![Direction::N], Vec::new(), Vec::new());
        }
        assert_eq!(state.scan_queue.len(), MAX_QUEUED);
        assert_eq!(state.scan_queue[0].room, 3, "the oldest went first");
        queue_scan(&mut state, 5, snapshot(), vec![Direction::S], Vec::new(), Vec::new());
        assert_eq!(state.scan_queue.iter().filter(|s| s.room == 5).count(), 1);
        assert_eq!(state.scan_queue.last().unwrap().slots(), &[Direction::S]);
    }
}
