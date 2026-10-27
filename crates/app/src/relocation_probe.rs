//! After `climb tree`, ask the game which DIRECTION that was (SQ-1771).
//!
//! ```text
//! > climb tree
//! Up a Tree
//! ```
//!
//! …and the map files `Forest Path --?--> Up a Tree`: a room dropped on the nearest free cell
//! with a question mark for an edge, because the only moves the mapper understands are direction
//! words ([`mapper::direction::WalkedDir`]: one token after an optional `go`). `push statue`,
//! `jump in river`, `crawl through hole` and `enter window` are all passages, and all of them
//! come out like that. The game knows perfectly well that this one is `up`; the map just never
//! typed it.
//!
//! # The idea: ask the game, in the shadow
//!
//! No verb table, no idiom list. When a turn changes room and no plain direction explains it, the
//! shadow ([`crate::probe`]) is restored to the moment BEFORE that turn -- the room the player
//! left -- and asked the same question [`crate::return_probe`] asks, pointed forward: which
//! direction word, typed there, lands in the room the player is now in? If exactly the game's
//! own answer is `up`, the `?` becomes `Up`, the room moves to the cell above, and the map is
//! what it would have been had the player typed `up` (see
//! [`mapper::mapper::Mapper::upgrade_unlabelled_passage`]). It runs on the shadow's worker thread
//! like every other probe; the answer arrives a beat later and the room snaps into place.
//!
//! The pre-turn moment is `AppState::random_exit_pre_move_save`: the snapshot the host keeps from
//! the END of every turn, which is the state the next command starts from. It is guarded by the
//! room it was taken in, so a snapshot that does not belong to the room being left is not used.
//!
//! # When it arms
//!
//! Only for the crossing a no-direction command leaves behind, recognised structurally: the room
//! changed, the command names no direction, and the map now holds a `?` passage from the old room
//! to the new one with no directional passage beside it. That excludes, without restating any of
//! them, every turn [`crate::session::apply_turn`] already classifies as a relocation -- a death
//! and its resurrection, `go to`, a random-exit landing, a suspicion left for another probe -- which
//! mint no edge. `undo`, `restart` and `restore` are named and skipped: the room they come out in
//! is where the game put you, and the way back to it is not a passage.
//!
//! # The two guards, and the failures they exist for
//!
//! **The `look` control.** Some relocations do not depend on what the player typed: a timed event
//! carries them off whatever they type (`wait`, `z`, even `look`). If every direction were tried
//! from a state where the event was due, one would "reach" the room the event sends everybody to,
//! and the map would draw a doorway where there is a clock. So the first command of the first
//! question is `look`, restored from the same pre-turn state as the rest: if the shadow ends in
//! the destination after `look`, the relocation was independent of the player's command and
//! nothing is recorded -- the crossing stays as it is today.
//!
//! **The confirm replay.** The winning direction is replayed once more, alone, from the same
//! pre-turn state, and must land in the same room. A random relocation that happens to fire in
//! one shadow attempt (an exit that sometimes sends you elsewhere) would otherwise be recorded
//! as a fixed passage on a coin flip. If the replay disagrees, nothing is placed.
//!
//! Beyond those: a shadow that DIED is not a landing ([`crate::probe::ProbeStep::landing`] --
//! Zork I's troll wakes the shadow in the Forest, a room the map holds, which is not where any
//! direction leads); and **no match places nothing** -- the crossing stays unplaced, exactly as
//! it was before this module, because a magic word or `pray` is a relocation no direction
//! reproduces.
//!
//! # The order it asks
//!
//! `n s e w ne nw se sw`, then `u d`, then `in out`, each from a freshly restored pre-turn state
//! (the shadow does this per command, see [`crate::probe::ShadowProbe::ask_from`]); the first
//! landing in the destination wins. The ship words (`fore`, `aft`, `port`, `starboard` and the
//! quarter words) are asked in a second question, and only for a story whose own dictionary
//! holds them -- a story that has never heard of `starboard` has no business being asked. That
//! is three questions at most: wide (the control plus the twelve compass words), nautical, confirm.
//!
//! # The compass alias of an in/out passage
//!
//! `In` and `Out` are portals: the layout has no cell offset for them and cuts the map into
//! regions there, so Zork I's `enter window` puts the whole house interior on a layer of its own
//! although `west` walks between the same two rooms. So when a crossing is recorded as `In` or
//! `Out` -- a typed `in`/`out`/`enter`/`exit`, or a directionless move this module just placed
//! as one -- the same machinery asks the **eight compass words only** (no `u`/`d`, also portals;
//! no ship words), from the same pre-turn state, with the same `look` control and confirm replay.
//! A confirmed compass passage is recorded **alongside** the portal, never in place of it: both
//! are true, and `in` stays the player's word. Each portal is searched once, the first time the
//! map sees it (`AppState::alias_searched`).
//!
//! # Staleness
//!
//! Like a return-path answer, the result is a fact about the map, not about this turn, so an
//! answer arriving three moves later is applied exactly like one arriving at once. What guards
//! it is the thing it upgrades: the `?` passage must still be there
//! ([`mapper::mapper::Mapper::upgrade_unlabelled_passage`] stands down if the player walked
//! `up` themselves meanwhile, or a rewind took the room away). While the search runs it holds the
//! shadow: [`crate::return_probe::pump_return_search`] waits for it, and when it succeeds the
//! return search armed for the same crossing is re-aimed as if the player had typed the word
//! ([`crate::return_probe::reseed_after_upgrade`]).

use mapper::direction::{Direction, NAUTICAL_PROBE_WORDS, long_label, parse_direction};
use mapper::graph::RoomId;
use mapper::mapper::{Mapper, ProbedPassage};

use crate::engine::Engine;
use crate::state::AppState;

/// The compass words a search asks, in the order it asks them.
const WIDE_ORDER: [Direction; 12] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::NE,
    Direction::NW,
    Direction::SE,
    Direction::SW,
    Direction::Up,
    Direction::Down,
    Direction::In,
    Direction::Out,
];

/// The eight compass points: what an alias search asks, and what counts as a compass passage.
const COMPASS: [Direction; 8] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::NE,
    Direction::NW,
    Direction::SE,
    Direction::SW,
];

/// The words a wide question asks: all twelve, or only the compass eight for an alias search.
fn wide_for(alias: bool) -> Vec<(&'static str, Direction)> {
    let dirs: &[Direction] = if alias { &COMPASS } else { &WIDE_ORDER };
    dirs.iter().map(|&d| (long_label(d), d)).collect()
}

/// Commands whose room change is the game's doing, not a passage.
pub(crate) const NOT_A_PASSAGE: [&str; 3] = ["undo", "restart", "restore"];

/// What the search is waiting to hear.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    /// `look`, then the compass words.
    Wide,
    /// The ship words the story knows.
    Nautical,
    /// The winning word, once more.
    Confirm,
}

/// A search for the direction behind one crossing, in progress.
///
/// Session state and never persisted; what it learns is a passage in the graph.
#[derive(Debug)]
pub struct RelocationSearch {
    /// The room the player left: where every attempt starts.
    origin: RoomId,
    /// The room the player arrived in: the room an attempt has to land in.
    dest: RoomId,
    /// The game as it stood before the command was typed.
    from: crate::probe::ProbeSnapshot,
    stage: Stage,
    /// The compass words, in the order asked.
    wide: Vec<(&'static str, Direction)>,
    /// The ship words this story knows, one per slot.
    nautical: Vec<(&'static str, Direction)>,
    /// The word that landed in `dest`, awaiting its confirmation.
    found: Option<(&'static str, Direction)>,
    /// The question out with the worker, if any.
    token: Option<u64>,
    /// A compass alias search for a crossing already recorded as `In`/`Out`: the winner is
    /// recorded ALONGSIDE that passage rather than in place of a `?` stub.
    alias: bool,
}

impl RelocationSearch {
    /// The room the search runs from, for tests and diagnostics.
    pub fn origin(&self) -> RoomId {
        self.origin
    }

    /// The room it is trying to reach.
    pub fn dest(&self) -> RoomId {
        self.dest
    }

    /// The ship words this search will ask, for tests and diagnostics.
    pub fn nautical_words(&self) -> &[(&'static str, Direction)] {
        &self.nautical
    }

    /// The commands the next question types, in order.
    fn commands(&self) -> Vec<String> {
        match self.stage {
            Stage::Wide => std::iter::once("look".to_string())
                .chain(self.wide.iter().map(|(w, _)| w.to_string()))
                .collect(),
            Stage::Nautical => self.nautical.iter().map(|(w, _)| w.to_string()).collect(),
            Stage::Confirm => self.found.iter().map(|(w, _)| w.to_string()).collect(),
        }
    }
}

/// The ship words this story's own dictionary holds: the first known spelling per slot.
pub(crate) fn nautical_words(state: &mut AppState, live: &dyn Engine) -> Vec<(&'static str, Direction)> {
    let mut out: Vec<(&'static str, Direction)> = Vec::new();
    for (word, dir) in NAUTICAL_PROBE_WORDS {
        if out.iter().any(|&(_, d)| d == dir) {
            continue;
        }
        let known = live
            .knows_word(word)
            .unwrap_or_else(|| state.vocab.get(live).is_some_and(|v| v.knows(word)));
        if known {
            out.push((word, dir));
        }
    }
    out
}

/// Start a search for the direction behind this turn's crossing, if it earned one (SQ-1771).
///
/// Called once per finished turn, after `apply_turn`, before the pre-turn snapshot is replaced.
/// See the module docs for the gate; in short, a changed room, no direction named, a `?` passage
/// minted and nothing directional beside it, and a snapshot of the room that was left.
pub fn arm_relocation_search(
    state: &mut AppState,
    mapper: &Mapper,
    live: &dyn Engine,
    cmd: &str,
    room_before: Option<RoomId>,
) {
    if !state.config.return_probe || !state.probe.is_armed() {
        return;
    }
    let (Some(origin), Some(dest)) = (room_before, mapper.graph.current()) else {
        return;
    };
    if mapper::direction::is_chained(cmd) {
        return; // SQ-1772: a chained line is replayed piece by piece, see `crate::chain_probe`
    }
    let typed = parse_direction(cmd);
    let portal = matches!(typed, Some(Direction::In | Direction::Out));
    if origin == dest || (typed.is_some() && !portal) {
        return;
    }
    let first = cmd.split_whitespace().next().unwrap_or("").to_lowercase();
    if NOT_A_PASSAGE.contains(&first.as_str()) {
        return;
    }
    let between = || {
        mapper
            .graph
            .connections()
            .iter()
            .filter(|c| c.origin == origin && c.dest == dest)
    };
    let alias = match typed {
        Some(portal_dir) => {
            // A plain `in`/`out`: the passage is already on the map as a portal. Look for the
            // compass word beside it, once per passage.
            if !between().any(|c| c.dir == portal_dir)
                || between().any(|c| COMPASS.contains(&c.dir))
                || state.alias_searched.contains(&(origin, portal_dir, dest))
            {
                return;
            }
            state.alias_searched.push((origin, portal_dir, dest));
            true
        }
        None => {
            if !between().any(|c| c.dir == Direction::Unknown)
                || between().any(|c| c.dir != Direction::Unknown)
            {
                return;
            }
            false
        }
    };
    if state
        .relocation_search
        .as_ref()
        .is_some_and(|s| s.token.is_some())
    {
        return; // one is out with the worker; its answer is not ours to abandon
    }
    let Some((saved_room, save)) = &state.random_exit_pre_move_save else {
        return;
    };
    if *saved_room != origin {
        return; // a snapshot of some other moment: a restore or a reset came between
    }
    let from = crate::probe::ProbeSnapshot::from_save(live, std::sync::Arc::clone(save));
    let nautical = if alias {
        Vec::new()
    } else {
        nautical_words(state, live)
    };
    state.relocation_search = Some(RelocationSearch {
        origin,
        dest,
        from,
        stage: Stage::Wide,
        wide: wide_for(alias),
        nautical,
        found: None,
        token: None,
        alias,
    });
}

/// Queue a search for the direction behind one crossing of a replayed chained line (SQ-1772).
///
/// `from` is the shadow's state just BEFORE the piece that made the crossing, which is why it
/// takes a snapshot rather than reading the pre-turn save. `nautical` is the ship words the
/// story's dictionary holds ([`nautical_words`]), computed when the chain was armed -- the live
/// engine is not on hand when an answer is read, but the dictionary is static -- so a chain-started
/// search runs the same Wide, Nautical, Confirm stages as a typed one. `alias` asks for the compass word beside an In/Out portal
/// instead, exactly as for a typed `in`/`out`.
pub(crate) fn queue_search(
    state: &mut AppState,
    origin: RoomId,
    dest: RoomId,
    from: crate::probe::ProbeSnapshot,
    nautical: Vec<(&'static str, Direction)>,
    alias: bool,
) {
    state.relocation_queue.push(RelocationSearch {
        origin,
        dest,
        from,
        stage: Stage::Wide,
        wide: wide_for(alias),
        nautical: if alias { Vec::new() } else { nautical },
        found: None,
        token: None,
        alias,
    });
}

/// Hand the search's next question to the worker, if the shadow is free. Called every pass of the
/// event loop, before the return search's pump. Returns true when something was asked.
pub fn pump_relocation_search(state: &mut AppState) -> bool {
    if !state.probe.is_armed() {
        state.relocation_search = None;
        state.relocation_queue.clear();
        return false;
    }
    if state.relocation_search.is_none() && !state.relocation_queue.is_empty() {
        state.relocation_search = Some(state.relocation_queue.remove(0));
    }
    let Some(search) = &state.relocation_search else {
        return false;
    };
    if search.token.is_some() {
        return false;
    }
    let Some(token) = state.probe.ask_from(&search.from, &search.commands()) else {
        return false; // busy: ask again next pass
    };
    if let Some(search) = &mut state.relocation_search {
        search.token = Some(token);
    }
    true
}

/// True when `token` answers the question this search asked.
pub fn owns(state: &AppState, token: u64) -> bool {
    state.relocation_search.as_ref().and_then(|s| s.token) == Some(token)
}

/// Read one answer back (SQ-1771). Returns true when the map changed.
///
/// * **Wide.** `look` lands in the destination: a timed event, nothing recorded. Otherwise the
///   first compass word that lands there is the candidate and goes to confirmation; none, and the
///   ship words are asked if the story has any; none of those, and the crossing stays as it is.
/// * **Nautical.** The same scan, without a control (the wide question already ran it).
/// * **Confirm.** The word must land in the same room again; then, and only then, the passage is
///   recorded.
///
/// A landing is [`crate::probe::ProbeStep::landing`]: a shadow that died, quit or reached for a
/// file landed nowhere.
pub fn deliver(state: &mut AppState, mapper: &mut Mapper, answer: &crate::probe::Answer) -> bool {
    if !owns(state, answer.token) {
        return false;
    }
    let Some(mut search) = state.relocation_search.take() else {
        return false;
    };
    search.token = None;
    let Some(run) = &answer.run else { return false }; // a broken shadow: no evidence either way
    let lands = |i: usize| run.steps.get(i).and_then(|s| s.landing()) == Some(search.dest);
    match search.stage {
        Stage::Wide | Stage::Nautical => {
            let (skip, words) = match search.stage {
                Stage::Wide => (1, &search.wide),
                _ => (0, &search.nautical),
            };
            if skip == 1 && (run.steps.len() != 1 + words.len() || lands(0)) {
                return false; // the control: it happens whatever is typed (or the run is torn)
            }
            let winner = words
                .iter()
                .enumerate()
                .find(|(i, _)| lands(i + skip))
                .map(|(_, &w)| w);
            match winner {
                Some(w) => {
                    search.found = Some(w);
                    search.stage = Stage::Confirm;
                }
                None if search.stage == Stage::Wide && !search.nautical.is_empty() => {
                    search.stage = Stage::Nautical;
                }
                None => return false,
            }
            state.relocation_search = Some(search);
            false
        }
        Stage::Confirm => {
            let (Some((word, dir)), true) = (search.found, lands(0)) else {
                return false;
            };
            let passage = ProbedPassage {
                from: search.origin,
                dir,
                to: search.dest,
            };
            if search.alias {
                // Alongside the observed In/Out passage, never in place of it: both are true.
                return mapper.record_probed_passage(passage);
            }
            let done = mapper.upgrade_unlabelled_passage(passage);
            if done {
                crate::return_probe::reseed_after_upgrade(
                    state,
                    mapper,
                    search.origin,
                    search.dest,
                    word,
                );
                // A directionless move that turned out to be In/Out is a portal like a typed
                // one: look for the compass word beside it too, from the same moment.
                let key = (search.origin, dir, search.dest);
                if matches!(dir, Direction::In | Direction::Out)
                    && !state.alias_searched.contains(&key)
                {
                    state.alias_searched.push(key);
                    state.relocation_search = Some(RelocationSearch {
                        stage: Stage::Wide,
                        wide: wide_for(true),
                        nautical: Vec::new(),
                        found: None,
                        token: None,
                        alias: true,
                        ..search
                    });
                }
            }
            done
        }
    }
}

/// Run a search to its end, waiting for each answer -- for a test or measurement harness, never
/// the event loop. Returns whether the map changed.
pub fn settle_relocation_search(state: &mut AppState, mapper: &mut Mapper) -> bool {
    let mut changed = false;
    while state.relocation_search.is_some() || !state.relocation_queue.is_empty() {
        if !pump_relocation_search(state)
            && state
                .relocation_search
                .as_ref()
                .is_some_and(|s| s.token.is_none())
        {
            break; // the shadow refused: nobody else is asking in a harness
        }
        let Some(answer) = state.probe.settle() else {
            break;
        };
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
        Some(ProbeRun {
            baseline: WorldPrint::default(),
            steps,
        })
    }

    /// A map with the `?` stub `climb tree` leaves: Path (1) --?--> Tree (2), player in Tree.
    fn crossed() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "Path", None);
        m.observe(2, "Tree", None);
        m
    }

    fn search(stage: Stage) -> RelocationSearch {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        let live =
            crate::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat loads");
        RelocationSearch {
            origin: 1,
            dest: 2,
            from: crate::probe::ProbeSnapshot::from_save(
                &live,
                std::sync::Arc::new(live.save_state()),
            ),
            stage,
            wide: WIDE_ORDER.iter().map(|&d| (long_label(d), d)).collect(),
            nautical: vec![("fore", Direction::N)],
            found: None,
            token: Some(7),
            alias: false,
        }
    }

    /// A wide answer where `look` leaves the shadow at `look_at` and `up` (index 9) lands in 2.
    fn wide_answer(look_at: RoomId) -> Option<ProbeRun> {
        let mut steps = vec![step("look", Some(look_at), false)];
        for d in WIDE_ORDER {
            let to = if d == Direction::Up { 2 } else { 1 };
            steps.push(step(long_label(d), Some(to), false));
        }
        run(steps)
    }

    fn state_with(search: RelocationSearch) -> AppState {
        let mut state = AppState::default();
        state.relocation_search = Some(search);
        state
    }

    /// The timed event: `look` alone carries the shadow to the destination, so the passage that
    /// "up" appears to be is the clock's doing. Nothing may be recorded and the search ends.
    #[test]
    fn the_look_control_rejects_a_relocation_no_command_causes() {
        let mut state = state_with(search(Stage::Wide));
        let mut m = crossed();
        let changed = deliver(&mut state, &mut m, &test_answer(7, wide_answer(2)));
        assert!(!changed);
        assert!(state.relocation_search.is_none(), "the search is over");
        assert!(
            m.graph
                .connections()
                .iter()
                .all(|c| c.dir == Direction::Unknown),
            "still a ?"
        );
    }

    /// Falsifier for the test above: the same answer with a `look` that stays put goes on to
    /// confirmation, with `up` as the candidate.
    #[test]
    fn without_the_control_firing_the_first_landing_goes_to_confirmation() {
        let mut state = state_with(search(Stage::Wide));
        let mut m = crossed();
        assert!(!deliver(
            &mut state,
            &mut m,
            &test_answer(7, wide_answer(1))
        ));
        let s = state.relocation_search.as_ref().expect("still searching");
        assert_eq!(s.stage, Stage::Confirm);
        assert_eq!(s.found, Some(("up", Direction::Up)));
        assert_eq!(s.commands(), vec!["up".to_string()]);
    }

    /// The confirm replay: a landing elsewhere the second time means the first was luck.
    #[test]
    fn a_confirmation_that_disagrees_places_nothing() {
        let mut s = search(Stage::Confirm);
        s.found = Some(("up", Direction::Up));
        let mut state = state_with(s);
        let mut m = crossed();
        assert!(!deliver(
            &mut state,
            &mut m,
            &test_answer(7, run(vec![step("up", Some(1), false)]))
        ));
        assert!(
            m.graph
                .connections()
                .iter()
                .all(|c| c.dir == Direction::Unknown)
        );
        assert!(state.relocation_search.is_none());
    }

    #[test]
    fn a_confirmation_that_agrees_records_the_direction() {
        let mut s = search(Stage::Confirm);
        s.found = Some(("up", Direction::Up));
        let mut state = state_with(s);
        let mut m = crossed();
        assert!(deliver(
            &mut state,
            &mut m,
            &test_answer(7, run(vec![step("up", Some(2), false)]))
        ));
        let c = m.graph.connections();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].origin, c[0].dir, c[0].dest), (1, Direction::Up, 2));
    }

    /// A shadow that died in the destination is a resurrection, not an arrival.
    #[test]
    fn a_death_is_not_a_landing() {
        let mut s = search(Stage::Confirm);
        s.found = Some(("up", Direction::Up));
        let mut state = state_with(s);
        let mut m = crossed();
        assert!(!deliver(
            &mut state,
            &mut m,
            &test_answer(7, run(vec![step("up", Some(2), true)]))
        ));
        assert!(
            m.graph
                .connections()
                .iter()
                .all(|c| c.dir == Direction::Unknown)
        );
    }

    /// No compass word lands: the ship words go next when the story has them, and nothing is
    /// recorded when it has none.
    #[test]
    fn no_compass_match_falls_to_the_ship_words_then_gives_up() {
        let mut steps = vec![step("look", Some(1), false)];
        steps.extend(
            WIDE_ORDER
                .iter()
                .map(|&d| step(long_label(d), Some(1), false)),
        );
        let mut state = state_with(search(Stage::Wide));
        let mut m = crossed();
        deliver(&mut state, &mut m, &test_answer(7, run(steps.clone())));
        assert_eq!(
            state.relocation_search.as_ref().map(|s| s.stage.clone()),
            Some(Stage::Nautical)
        );

        let mut none = search(Stage::Wide);
        none.nautical.clear();
        let mut state = state_with(none);
        deliver(&mut state, &mut m, &test_answer(7, run(steps)));
        assert!(state.relocation_search.is_none());
        assert!(
            m.graph
                .connections()
                .iter()
                .all(|c| c.dir == Direction::Unknown)
        );
    }

    /// An alias search whose `look` control stays put and whose compass word `west` lands: the
    /// confirmed compass passage is recorded ALONGSIDE the observed `In`, never in place of it.
    #[test]
    fn a_compass_alias_is_recorded_beside_the_portal() {
        let mut m = Mapper::default();
        m.observe(1, "Behind House", None);
        m.observe(2, "Kitchen", Some(Direction::In));
        let mut s = search(Stage::Confirm);
        s.alias = true;
        s.found = Some(("west", Direction::W));
        let mut state = state_with(s);
        assert!(deliver(
            &mut state,
            &mut m,
            &test_answer(7, run(vec![step("west", Some(2), false)]))
        ));
        let c: Vec<_> = m
            .graph
            .connections()
            .iter()
            .map(|c| (c.origin, c.dir, c.dest))
            .collect();
        assert!(
            c.contains(&(1, Direction::In, 2)) && c.contains(&(1, Direction::W, 2)),
            "{c:?}"
        );
    }

    /// An in/out move no compass word reproduces records only the portal, and an alias search
    /// asks the compass eight only (not up/down/in/out).
    #[test]
    fn an_alias_with_no_compass_match_records_only_the_portal() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Barrow", Some(Direction::In));
        let mut s = search(Stage::Wide);
        s.alias = true;
        s.wide = wide_for(true);
        s.nautical.clear();
        assert_eq!(
            s.commands().len(),
            9,
            "look and eight compass words: {:?}",
            s.commands()
        );
        let mut steps = vec![step("look", Some(1), false)];
        steps.extend(s.wide.iter().map(|(w, _)| step(w, Some(1), false)));
        let mut state = state_with(s);
        assert!(!deliver(&mut state, &mut m, &test_answer(7, run(steps))));
        assert!(state.relocation_search.is_none());
        assert_eq!(m.graph.connections().len(), 1);
    }

    /// SQ-1772: a search queued from a chain carries the story's ship words, so a crossing only a
    /// ship word reproduces is found (Wide misses, Nautical lands, Confirm records). With an empty
    /// list it gives up after Wide.
    #[test]
    fn a_queued_search_with_ship_words_finds_a_crossing_only_they_reproduce() {
        let bytes = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        let live = crate::scott_session::ScottSession::new(bytes, None).unwrap();
        let snap = crate::probe::ProbeSnapshot::from_save(&live, std::sync::Arc::new(live.save_state()));
        for (words, finds) in [(vec![("fore", Direction::N)], true), (Vec::new(), false)] {
            let mut state = AppState::default();
            let mut m = crossed();
            queue_search(&mut state, 1, 2, snap.clone(), words, false);
            let mut s = state.relocation_queue.remove(0);
            s.token = Some(7);
            state.relocation_search = Some(s);
            let mut steps = vec![step("look", Some(1), false)];
            steps.extend(WIDE_ORDER.iter().map(|&d| step(long_label(d), Some(1), false)));
            deliver(&mut state, &mut m, &test_answer(7, run(steps)));
            if !finds {
                assert!(state.relocation_search.is_none(), "no ship words: gives up");
                continue;
            }
            state.relocation_search.as_mut().unwrap().token = Some(8);
            assert_eq!(state.relocation_search.as_ref().unwrap().commands(), vec!["fore".to_string()]);
            deliver(&mut state, &mut m, &test_answer(8, run(vec![step("fore", Some(2), false)])));
            state.relocation_search.as_mut().unwrap().token = Some(9);
            assert!(deliver(&mut state, &mut m, &test_answer(9, run(vec![step("fore", Some(2), false)]))));
            let c = m.graph.connections();
            assert_eq!((c[0].origin, c[0].dir, c[0].dest), (1, Direction::N, 2));
        }
    }

    /// An answer nobody asked for is left alone.
    #[test]
    fn an_answer_with_another_token_is_ignored() {
        let mut state = state_with(search(Stage::Wide));
        let mut m = crossed();
        assert!(!deliver(
            &mut state,
            &mut m,
            &test_answer(8, wide_answer(1))
        ));
        assert!(state.relocation_search.is_some());
    }
}
