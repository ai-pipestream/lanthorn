//! `north then east` on one line: replay it a piece at a time, in the shadow (SQ-1772).
//!
//! ```text
//! > go north then east
//! North of House
//! Behind House
//! ```
//!
//! The mapper reads a command's direction off its first word
//! ([`mapper::direction::parse_direction`]), so this used to file `West of House --N--> Behind
//! House`: one passage north that does not exist, because the line walked two rooms and the
//! first-named direction was glued to the LAST room. The map then disagreed with the game
//! forever -- walk `north` from West of House and you arrive somewhere else.
//!
//! # What it does
//!
//! The live line still goes to the game exactly as typed. What changed is what the map makes of
//! it:
//!
//! 1. **A chained line is not a move in any one direction.** [`mapper::direction::command_pieces`]
//!    cuts a line at `then`, `and`, `.` and `,` (outside double quotes), and
//!    [`mapper::direction::parse_direction`] answers `None` for anything it cuts into two or more
//!    pieces. The turn lands on the map as an unlabelled `?` passage, the way a directionless
//!    `climb tree` always has -- provisional, and honest about not knowing.
//! 2. **The shadow replays the line a piece at a time** ([`crate::probe::ShadowProbe::
//!    ask_sequence_from`]): restore the state from BEFORE the turn, type the first piece, read
//!    the room, type the second from where the first left the shadow, and so on. Room identity is
//!    the mapper's own location number, and a death is not a landing
//!    ([`crate::probe::ProbeStep::landing`]) -- the judgement [`crate::return_probe`] and
//!    [`crate::relocation_probe`] already make.
//! 3. **Every room change the replay saw is recorded as a walked move from the room before that
//!    piece.** A piece that is a plain direction goes in under it. A piece that changes room
//!    without naming one (`enter window`) goes in as a `?` passage and is handed to the SQ-1771
//!    machinery ([`crate::relocation_probe`]: the `look` control, the compass search, the confirm
//!    replay), started from the shadow's state just before THAT piece -- so `open window then
//!    enter window` files `Behind House --In--> Kitchen` plus the `West` alias, same as typing the
//!    two commands separately. Rooms the player passed through were visited for real, and go on the
//!    map ([`mapper::mapper::Mapper::replace_unlabelled_with_walk`]).
//!
//! # The safeguard, and why it is enough
//!
//! **Nothing is recorded unless the shadow's final room is the room the live game ended in.** That
//! one check is what makes it safe to cut a line in places the game may not (see the table
//! below): a wrong cut types the wrong thing, ends somewhere else, and records nothing. A chain
//! that stops part-way -- Zork refuses `north then up` after the `up` and stays in North of House
//! -- replays identically and passes the check, recording only the step that happened.
//!
//! **And a second hypothesis, for a game that reads only the first command.** Scott Adams
//! interpreters take a verb and a noun and ignore the rest of the line, so `go east then go west`
//! walks east and stops. If the piece-by-piece replay does not end where the live game did, the
//! FIRST piece alone is replayed (one more question) and, if THAT ends there, the line is mapped as
//! that single move. A game that is simply ignoring the rest is thus mapped as the one command it
//! really ran.
//!
//! # Black-box findings: which separators the games accept (2026-10-11)
//!
//! Typed into the real stories through a headless session (`GameSession` / `GlulxSession` /
//! `ScottSession`), from a room with an exit, noting whether BOTH moves ran. No interpreter source
//! was read for any of this.
//!
//! | game (engine, release) | `then` | `.` | `,` | `and` |
//! |---|---|---|---|---|
//! | Zork I (ZIL, z3 r88 s840726) | both | both | both | both |
//! | Curses (Inform 6, z5) | both | both | both | refused (`only understood you as far as wanting to e`) |
//! | Bronze (Inform 7, z-machine, zblorb) | both | both | both | refused |
//! | Anchorhead (Inform 6, Glulx gblorb) | both | both | both | refused |
//! | And Then You Come to a House (Inform 7, Glulx) | both | both | both | refused |
//! | Adventureland (Scott Adams, adv01.dat) | first only | first only | first only | first only |
//!
//! Notes that shaped the code:
//!
//! * `and then` is two separators in a row; Zork reads `north and then east` as `north`, `then
//!   east` and answers *There was no verb in that sentence!* after walking north. The empty piece
//!   between two separators is dropped; the replay settles what actually happened.
//! * A trailing separator (`north then`) is harmless and a leading one (`then north`) is not a
//!   move in Zork (*What do you want to tell?*); the pieces are the non-empty runs between
//!   separators, and a line of one piece is not a chain.
//! * **A comma is ambiguous, and is cut anyway.** `mailbox, north` in Zork I answers *You can't
//!   talk to the small mailbox!* -- there a comma addresses an actor -- yet `north, east` walks
//!   both. The text cannot say which, so the line is cut and the replay (which types the pieces
//!   with the comma removed) decides: an address that moved nobody has no room change to record,
//!   and one that did cannot reach the live final room by accident without recording a path the
//!   shadow genuinely walked.
//! * Scott's parser keeps the first word, and `go` plus the next. `e then w` is *You use word(s)
//!   I don't know!* (a one-letter first word is no motion there) and moves nobody.
//!
//! # Provisional, and staleness
//!
//! Until the replay answers, the map holds the `?` passage and nothing else, so a game the shadow
//! cannot answer (the seam off, `return_probe` off, no pre-turn snapshot, a replay that disagrees
//! with the live game) simply keeps the honest `?`. A late answer is applied only if the `?`
//! passage it replaces is still there ([`mapper::mapper::Mapper::replace_unlabelled_with_walk`]
//! stands down otherwise: a rewind took the room away, or the map moved on). While a replay is out,
//! the return search waits ([`crate::return_probe::pump_return_search`]) -- what the crossing even
//! was is not known yet.
//!
//! Off with the rest of the probes: gated by `config.return_probe`, like its siblings.

use mapper::direction::{Direction, command_pieces, parse_direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, WalkedStep};

use crate::engine::Engine;
use crate::probe::{Answer, ProbeRun, ProbeSnapshot};
use crate::state::AppState;

/// What the replay is waiting to hear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Every piece, in order, from one restore.
    All,
    /// The first piece alone: a game that reads only the first command.
    First,
}

/// A replay of one chained command line, in progress. Session state, never persisted.
#[derive(Debug)]
pub struct ChainReplay {
    /// The room the player was in when they typed the line.
    origin: RoomId,
    /// The room the live game ended the line in: the room the replay has to end in.
    dest: RoomId,
    /// The game as it stood before the line was typed.
    from: ProbeSnapshot,
    pieces: Vec<String>,
    stage: Stage,
    /// The question out with the worker, if any.
    token: Option<u64>,
}

/// One room change the replay saw: piece `piece` took the shadow from `from` to `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hop {
    piece: usize,
    from: RoomId,
    to: RoomId,
    to_name: String,
}

/// Start a replay for this finished turn, if it earned one (SQ-1772).
///
/// Called once per finished turn, after `apply_turn`, before the pre-turn snapshot is replaced.
/// The gate: the probe seam is on and armed; the line chains two or more commands; none of them is
/// `undo`/`restart`/`restore` (the room they come out in is where the game put you); a room
/// changed or at least one piece names a direction (a line that walked out and back is still a
/// walk); no replay is already out with the worker; and the snapshot of the moment before the
/// turn belongs to the room that was left.
pub fn arm_chain_replay(
    state: &mut AppState,
    mapper: &Mapper,
    live: &dyn Engine,
    cmd: &str,
    room_before: Option<RoomId>,
) {
    if !state.config.return_probe || !state.probe.is_armed() {
        return;
    }
    let pieces = command_pieces(cmd);
    if pieces.len() < 2 || pieces.len() > crate::probe::MAX_PROBES {
        return;
    }
    let (Some(origin), Some(dest)) = (room_before, mapper.graph.current()) else {
        return;
    };
    if pieces.iter().any(|p| {
        let first = p.split_whitespace().next().unwrap_or("").to_lowercase();
        crate::relocation_probe::NOT_A_PASSAGE.contains(&first.as_str())
    }) {
        return;
    }
    if origin == dest && !pieces.iter().any(|p| parse_direction(p).is_some()) {
        return;
    }
    if state.chain_replay.as_ref().is_some_and(|c| c.token.is_some()) {
        return; // one is out with the worker; its answer is not ours to abandon
    }
    let Some((saved_room, save)) = &state.random_exit_pre_move_save else {
        return;
    };
    if *saved_room != origin {
        return; // a snapshot of some other moment: a restore or a reset came between
    }
    let from = ProbeSnapshot::from_save(live, std::sync::Arc::clone(save));
    state.chain_replay = Some(ChainReplay { origin, dest, from, pieces, stage: Stage::All, token: None });
}

/// Hand the replay's next question to the worker, if the shadow is free. Called every pass of the
/// event loop. Returns true when something was asked.
pub fn pump_chain_replay(state: &mut AppState) -> bool {
    if !state.probe.is_armed() {
        state.chain_replay = None;
        return false;
    }
    let Some(chain) = &state.chain_replay else { return false };
    if chain.token.is_some() {
        return false;
    }
    let token = match chain.stage {
        Stage::All => state.probe.ask_sequence_from(&chain.from, &chain.pieces),
        Stage::First => state.probe.ask_from(&chain.from, &chain.pieces[..1]),
    };
    let Some(token) = token else {
        return false; // busy: ask again next pass
    };
    if let Some(chain) = &mut state.chain_replay {
        chain.token = Some(token);
    }
    true
}

/// True when `token` answers the question this replay asked.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.chain_replay.as_ref().and_then(|c| c.token) == Some(token)
}

/// The room changes a replay saw, and the room it ended in. `None` when the run cannot be read:
/// it is torn (fewer steps than pieces), or a step ended the story, escaped the shadow, died, or
/// could not name its room -- none of which is a landing.
fn trace(origin: RoomId, run: &ProbeRun, pieces: usize) -> Option<(Vec<Hop>, RoomId)> {
    if run.steps.len() != pieces {
        return None;
    }
    let mut here = origin;
    let mut hops = Vec::new();
    for (piece, step) in run.steps.iter().enumerate() {
        let to = step.landing()?;
        if to != here {
            hops.push(Hop { piece, from: here, to, to_name: step.room_name.clone()? });
            here = to;
        }
    }
    Some((hops, here))
}

/// Read one answer back (SQ-1772). Returns true when the map changed.
///
/// * **All pieces.** The replay must end in the room the live game did; then its hops are
///   recorded. Otherwise, if the game may simply have read only the first command, the first piece
///   is asked alone.
/// * **First piece.** The same check against the one hop; it ends the replay either way.
pub fn deliver(state: &mut AppState, mapper: &mut Mapper, answer: &Answer) -> bool {
    if !owns(state, answer.token) {
        return false;
    }
    let Some(mut chain) = state.chain_replay.take() else { return false };
    chain.token = None;
    let Some(run) = &answer.run else { return false }; // a broken shadow: no evidence either way
    let asked = match chain.stage {
        Stage::All => chain.pieces.len(),
        Stage::First => 1,
    };
    let walked = trace(chain.origin, run, asked).filter(|(_, end)| *end == chain.dest);
    let Some((hops, _)) = walked else {
        if chain.stage == Stage::All {
            chain.stage = Stage::First;
            state.chain_replay = Some(chain);
        }
        return false;
    };
    let walk: Vec<WalkedStep> = hops
        .iter()
        .map(|h| WalkedStep {
            from: h.from,
            dir: parse_direction(&chain.pieces[h.piece]).unwrap_or(Direction::Unknown),
            to: h.to,
            to_name: h.to_name.clone(),
        })
        .collect();
    if !mapper.replace_unlabelled_with_walk(chain.origin, chain.dest, &walk) {
        return false; // stale: the `?` it would replace is gone
    }
    // What the walk left to name, from the shadow's state just before the piece that made it.
    for (hop, step) in hops.iter().zip(&walk) {
        let before = if hop.piece == 0 {
            Some(chain.from.clone())
        } else {
            run.steps[hop.piece - 1].after.as_ref().map(|s| chain.from.with_save(std::sync::Arc::clone(s)))
        };
        let Some(before) = before else { continue };
        match step.dir {
            Direction::Unknown => {
                crate::relocation_probe::queue_search(state, hop.from, hop.to, before, false);
            }
            Direction::In | Direction::Out => {
                let key = (hop.from, step.dir, hop.to);
                if !state.alias_searched.contains(&key) {
                    state.alias_searched.push(key);
                    crate::relocation_probe::queue_search(state, hop.from, hop.to, before, true);
                }
            }
            _ => {}
        }
    }
    // The return search armed for the one-step `?` crossing asks about a pair of rooms that is
    // no longer a passage: re-aim it for a single named step, drop it for a longer walk.
    match walk.as_slice() {
        [only] if only.dir != Direction::Unknown => {
            crate::return_probe::reseed_after_upgrade(
                state,
                mapper,
                chain.origin,
                chain.dest,
                mapper::direction::long_label(only.dir),
            );
        }
        _ => {
            if state
                .return_search
                .as_ref()
                .is_some_and(|s| s.origin() == chain.origin && s.here() == chain.dest)
            {
                state.return_search = None;
            }
        }
    }
    true
}

/// Run a replay to its end, waiting for each answer -- for a test or measurement harness, never
/// the event loop. Returns whether the map changed.
pub fn settle_chain_replay(state: &mut AppState, mapper: &mut Mapper) -> bool {
    let mut changed = false;
    while state.chain_replay.is_some() {
        if !pump_chain_replay(state) && state.chain_replay.as_ref().is_some_and(|c| c.token.is_none()) {
            break; // the shadow refused: nobody else is asking in a harness
        }
        let Some(answer) = state.probe.settle() else { break };
        changed |= deliver(state, mapper, &answer);
    }
    changed
}

#[cfg(all(test, feature = "t-session"))]
mod tests {
    use super::*;
    use crate::probe::{ProbeStep, WorldPrint, test_answer};

    fn step(command: &str, location: RoomId, name: &str) -> ProbeStep {
        ProbeStep {
            command: command.into(),
            reply: String::new(),
            location: Some(location),
            world: WorldPrint::default(),
            quit: false,
            escaped: false,
            died: false,
            room_name: Some(name.into()),
            after: None,
        }
    }

    fn run(steps: Vec<ProbeStep>) -> Option<ProbeRun> {
        Some(ProbeRun { baseline: WorldPrint::default(), steps })
    }

    fn replay(pieces: &[&str], stage: Stage) -> ChainReplay {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        let live = crate::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads");
        ChainReplay {
            origin: 1,
            dest: 3,
            from: ProbeSnapshot::from_save(&live, std::sync::Arc::new(live.save_state())),
            pieces: pieces.iter().map(|p| p.to_string()).collect(),
            stage,
            token: Some(7),
        }
    }

    /// West of House (1), the line `north then east` left a `?` stub to Behind House (3).
    fn crossed() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(3, "Behind House", None);
        m
    }

    fn edges(m: &Mapper) -> Vec<(RoomId, Direction, RoomId)> {
        m.graph.connections().iter().map(|c| (c.origin, c.dir, c.dest)).collect()
    }

    fn state_with(chain: ChainReplay) -> AppState {
        let mut state = AppState::default();
        state.chain_replay = Some(chain);
        state
    }

    #[test]
    fn a_replay_that_ends_where_the_live_game_did_is_recorded_step_by_step() {
        let mut state = state_with(replay(&["go north", "east"], Stage::All));
        let mut m = crossed();
        let steps = vec![step("go north", 2, "North of House"), step("east", 3, "Behind House")];
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::N, 2), (2, Direction::E, 3)]);
        assert!(state.chain_replay.is_none());
    }

    /// The safeguard: a replay that ends elsewhere records nothing, and asks for the first piece
    /// alone before giving up.
    #[test]
    fn a_replay_that_ends_elsewhere_records_nothing_and_tries_the_first_piece_alone() {
        let mut state = state_with(replay(&["north", "east"], Stage::All));
        let mut m = crossed();
        let steps = vec![step("north", 2, "North of House"), step("east", 9, "Elsewhere")];
        assert!(!deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::Unknown, 3)], "still a ?");
        let c = state.chain_replay.as_ref().expect("the first-piece question is next");
        assert_eq!(c.stage, Stage::First);
        assert!(c.token.is_none());
    }

    /// A game that reads only the first command (Scott): the first piece alone lands in the live
    /// room, and the line is mapped as that one move.
    #[test]
    fn a_game_reading_only_the_first_command_is_mapped_as_that_one_move() {
        let mut state = state_with(replay(&["go east", "go west"], Stage::First));
        let mut m = crossed();
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(vec![step("go east", 3, "Behind House")]))));
        assert_eq!(edges(&m), [(1, Direction::E, 3)]);
    }

    #[test]
    fn the_first_piece_failing_ends_the_replay_with_nothing_recorded() {
        let mut state = state_with(replay(&["go east", "go west"], Stage::First));
        let mut m = crossed();
        assert!(!deliver(&mut state, &mut m, &test_answer(7, run(vec![step("go east", 1, "West of House")]))));
        assert!(state.chain_replay.is_none());
        assert_eq!(edges(&m), [(1, Direction::Unknown, 3)]);
    }

    /// A step that fails part-way (`north then up`): only the step that moved is recorded.
    #[test]
    fn a_refused_step_records_only_the_real_one() {
        let mut state = state_with(replay(&["north", "up"], Stage::All));
        state.chain_replay.as_mut().unwrap().dest = 2;
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", None);
        let steps = vec![step("north", 2, "North of House"), step("up", 2, "North of House")];
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::N, 2)]);
    }

    /// A step that killed the shadow is not a landing: nothing is recorded.
    #[test]
    fn a_death_in_the_replay_records_nothing() {
        let mut state = state_with(replay(&["north", "east"], Stage::All));
        let mut m = crossed();
        let steps = vec![step("north", 2, "North of House"), ProbeStep { died: true, ..step("east", 3, "Behind House") }];
        assert!(!deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::Unknown, 3)]);
    }

    /// A late answer for a `?` the map no longer holds is dropped.
    #[test]
    fn a_stale_answer_changes_nothing() {
        let mut state = state_with(replay(&["north", "east"], Stage::All));
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(3, "Behind House", Some(Direction::S)); // the player walked on, a real edge
        let steps = vec![step("north", 2, "North of House"), step("east", 3, "Behind House")];
        assert!(!deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::S, 3)]);
    }

    /// A piece that changes room without naming a direction is filed `?` and queued for the
    /// SQ-1771 search, which starts from the shadow's state before THAT piece.
    #[test]
    fn a_directionless_piece_is_queued_for_the_relocation_search() {
        let mut state = state_with(replay(&["open window", "climb tree"], Stage::All));
        state.chain_replay.as_mut().unwrap().origin = 1;
        state.chain_replay.as_mut().unwrap().dest = 3;
        let mut m = crossed();
        let mut mid = step("open window", 1, "Behind House");
        let live = crate::scott_session::ScottSession::new(include_bytes!("../../scott/tests/tiny_cave.dat").to_vec(), None).unwrap();
        mid.after = Some(std::sync::Arc::new(live.save_state()));
        let steps = vec![mid, step("climb tree", 3, "Up a Tree")];
        assert!(deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert_eq!(edges(&m), [(1, Direction::Unknown, 3)]);
        assert_eq!(state.relocation_queue.len(), 1);
        assert_eq!((state.relocation_queue[0].origin(), state.relocation_queue[0].dest()), (1, 3));
    }

    #[test]
    fn an_answer_with_another_token_is_ignored() {
        let mut state = state_with(replay(&["north", "east"], Stage::All));
        let mut m = crossed();
        assert!(!deliver(&mut state, &mut m, &test_answer(8, None)));
        assert!(state.chain_replay.is_some());
    }
}
