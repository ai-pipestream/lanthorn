//! The exit scan (SQ-1773): the first time the player is in a room, a silent copy of the game is
//! asked every direction from there, and the map learns which ones are real exits -- without
//! creating, naming or revealing the rooms beyond.
//!
//! Real-game cases drive `zork1-r88-s840726.z3` the way `host::turn::finish_command_turn` does
//! (submit, `apply_turn`, keep the end-of-turn snapshot, arm the scan, let it finish) and skip
//! vacuously without `stories/` (gitignored). The truth they compare against is NOT the scan's
//! own machinery: `oracle_exits` boots a fresh game per word and types it for real.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::AppState;

use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;
use mapper::matrix::{MATRIX_DIRS, MatrixCell, classify};

const ZORK1: &str = "zork1-r88-s840726.z3";

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

fn recipe(bytes: &[u8]) -> ShadowRecipe {
    ShadowRecipe {
        catalogue: None,
        story_bytes: Arc::new(bytes.to_vec()),
        store: PathBuf::new(),
        vfs_bytes: Arc::new(Vec::new()),
        honor_game_colours: true,
        interpreter_number: None,
        random_seed: None,
        acceleration: true,
        screen: (80, 24),
    }
}

fn zork_session(bytes: &[u8]) -> app::session::GameSession {
    let mut s = app::session::GameSession::new_with_trace(
        bytes.to_vec(),
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some((25, 80)),
    )
    .expect("zork1-r88-s840726.z3 boots without a ZError");
    s.set_strip_prompt(false);
    s
}

/// A real story driven the way `finish_command_turn` drives it, with the exit scan armed.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: Box<dyn Engine>,
    death: app::session::DeathWatch,
}

impl Play {
    fn zork1(scan: bool) -> Option<Play> {
        let bytes = story(ZORK1)?;
        let mut state = AppState::default();
        state.config.scan_exits = scan;
        state.probe.arm(recipe(&bytes));
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session: Box::new(zork_session(&bytes)),
            death: Default::default(),
        };
        p.turn("look");
        Some(p)
    }

    fn adventureland() -> Option<Play> {
        let bytes = story("adv01.dat")?;
        let session = app::scott_session::ScottSession::new(bytes.clone(), None).expect("adv01.dat loads");
        let mut state = AppState::default();
        state.probe.arm(recipe(&bytes));
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session: Box::new(session),
            death: Default::default(),
        };
        p.turn("look");
        Some(p)
    }

    /// One finished turn, leaving the scan it armed queued (not yet asked).
    fn turn_unsettled(&mut self, cmd: &str) {
        let mut r = self.session.submit(cmd);
        let room_before = self.mapper.graph.current();
        // As `finish_command_turn` does: what the room being left declares for this move.
        r.declared_exit = app::random_exit_probe::declared_exit_for_command(cmd, room_before, |o, d| {
            self.session.declared_exit(o, d)
        });
        app::session::apply_turn(&mut self.mapper, cmd, &r, &mut self.death);
        let mut save = app::engine::TurnSave::default();
        self.state.random_exit_pre_move_save =
            Some((self.mapper.graph.current().unwrap_or(0), save.get(&*self.session)));
        app::scan_probe::arm_exit_scan(&mut self.state, &self.mapper, &*self.session, room_before);
    }

    /// One finished turn, with every scan it armed run to its end.
    fn turn(&mut self, cmd: &str) {
        self.turn_unsettled(cmd);
        app::scan_probe::settle_exit_scan(&mut self.state, &mut self.mapper);
    }

    fn here(&self) -> RoomId {
        self.mapper.graph.current().expect("a current room")
    }

    fn name(&self, id: RoomId) -> String {
        self.mapper.graph.room(id).map(|r| r.label().to_string()).unwrap_or_default()
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }

    /// The directions the scan found to be exits into rooms the map does not hold, in column order.
    fn stubs(&self, room: RoomId) -> Vec<Direction> {
        MATRIX_DIRS.iter().copied().filter(|d| self.mapper.graph.is_scan_exit(room, *d)).collect()
    }
}

/// The truth, independent of the scan: boot a FRESH game for every compass word, play `setup`,
/// type the word for real, and see whether the player's room changed. Death is not a move.
fn oracle_exits(bytes: &[u8], setup: &[&str]) -> Vec<Direction> {
    let mut out = Vec::new();
    for dir in MATRIX_DIRS {
        let mut s = zork_session(bytes);
        for cmd in setup {
            s.submit(cmd);
        }
        let start = s.current_location().expect("a location").number;
        let r = s.submit(mapper::direction::long_label(dir));
        let died = app::session::turn_reports_death(&r.transcript);
        let after = s.current_location().expect("a location").number;
        if after != start && !died {
            out.push(dir);
        }
    }
    out
}

/// West of House, scanned on arrival: the unexplored exits are exactly the ones the game really
/// allows from there -- checked against a fresh game typing each word -- and the boarded front
/// door (`east`) is not one of them.
///
/// (Fixture: `zork1-r88-s840726.z3`; the opening `look` only.)
#[test]
fn zork1_west_of_house_exits_are_what_the_game_allows() {
    let Some(bytes) = story(ZORK1) else { return };
    let Some(p) = Play::zork1(true) else { return };
    let here = p.here();
    assert_eq!(p.name(here), "West of House");
    assert!(p.mapper.graph.is_scanned(here), "the scan finished");

    let truth = oracle_exits(&bytes, &[]);
    assert!(!truth.is_empty(), "the oracle found exits at all");
    assert_eq!(p.stubs(here), truth, "scan vs. a fresh game typing each word");
    assert_eq!(
        p.mapper.graph.untried(here),
        truth,
        "and those, nothing more, are what the map offers as unexplored"
    );
    assert!(!p.mapper.graph.untried(here).contains(&Direction::E), "the boarded door is no exit");
    for d in [Direction::N, Direction::S, Direction::W] {
        assert!(truth.contains(&d), "{d:?} is a real way out of West of House");
        assert_eq!(classify(&p.mapper.graph, here, d), MatrixCell::Exit);
    }
    assert_eq!(classify(&p.mapper.graph, here, Direction::E), MatrixCell::Probed);
    eprintln!("West of House exits: {truth:?}");
}

/// The no-reveal rule: an exit into a room the map has not seen records only the direction. No
/// room, no name, no edge -- the map holds exactly the one room the player stands in.
#[test]
fn zork1_a_scanned_exit_into_an_unvisited_room_reveals_nothing() {
    let Some(p) = Play::zork1(true) else { return };
    let here = p.here();
    assert!(!p.stubs(here).is_empty(), "there are exits to hide the far side of");
    assert_eq!(p.mapper.graph.rooms().count(), 1, "only West of House exists");
    assert!(p.mapper.graph.connections().is_empty(), "no edge: {:?}", p.mapper.graph.connections());
    assert!(p.mapper.graph.room(here).unwrap().tried.is_empty(), "nothing is the player's");
    let names: Vec<String> = p.mapper.graph.rooms().map(|r| r.label().to_string()).collect();
    assert_eq!(names, vec!["West of House".to_string()], "no other name leaked");
}

/// A scanned landing in a room the map already holds is a verified passage, and not the player's
/// try. Zork's North of House opens west onto West of House, which the player has stood in.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`.)
#[test]
fn zork1_a_landing_on_a_known_room_records_the_passage_untried() {
    let Some(mut p) = Play::zork1(true) else { return };
    let west = p.here();
    p.turn("north");
    let north = p.here();
    assert_eq!(p.name(north), "North of House");
    let back: Vec<Direction> = MATRIX_DIRS
        .iter()
        .copied()
        .filter(|&d| p.edge(north, d) == Some(west))
        .collect();
    assert!(!back.is_empty(), "North of House reaches West of House, found by the scan");
    for d in back {
        assert!(p.mapper.graph.is_probed(north, d), "{d:?} is on the search's record");
        assert!(!p.mapper.graph.room(north).unwrap().tried.contains(&d), "{d:?} is not the player's");
        assert!(!p.mapper.graph.is_scan_exit(north, d), "an edge, not a stub");
    }
}

/// SQ-1775: the opening scan found West of House's south exit as a stub (South of House was not
/// on the map). Walking round to South of House by another way must turn that stub into a
/// passage, not leave `»` beside it for ever.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`, `east`, `south`.)
#[test]
fn zork1_a_stub_becomes_a_passage_once_its_room_is_reached_another_way() {
    let Some(mut p) = Play::zork1(true) else { return };
    let west = p.here();
    assert!(p.stubs(west).contains(&Direction::S), "the opening scan left a south stub");
    for cmd in ["north", "east", "south"] {
        p.turn(cmd);
    }
    let south = p.here();
    assert_eq!(p.name(south), "South of House");
    assert_eq!(p.edge(west, Direction::S), Some(south), "West of House -S-> South of House");
    assert_eq!(p.edge(south, Direction::W), Some(west), "South of House -W-> West of House");
    assert!(!p.mapper.graph.is_scan_exit(west, Direction::S), "no longer a stub");
    assert!(p.mapper.graph.is_probed(west, Direction::S));
    assert!(!p.mapper.graph.room(west).unwrap().tried.contains(&Direction::S), "not the player's");
}

/// A revisit asks only the directions that failed: Behind House has no west/in while the window
/// is shut. Once it is open and the player comes back, the rescan finds them.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`, `east`, `open window`, `north`, `east`.)
#[test]
fn zork1_a_door_that_opens_is_found_on_the_next_visit() {
    let Some(mut p) = Play::zork1(true) else { return };
    p.turn("north");
    p.turn("east");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House");
    for d in [Direction::W, Direction::In] {
        assert!(!p.mapper.graph.is_scan_exit(behind, d), "{d:?} is shut: {:?}", p.stubs(behind));
        assert!(p.mapper.graph.scan_walls(behind).contains(&d), "and is kept to be asked again");
    }
    let stubs_before = p.stubs(behind);

    p.turn("open window");
    p.turn("north");
    p.turn_unsettled("east");
    assert_eq!(p.here(), behind);
    let queued = p.state.scan_queue.first().expect("a rescan is queued");
    let mut expect = p.mapper.graph.scan_walls(behind);
    expect.retain(|d| p.edge(behind, *d).is_none());
    let mut got = queued.slots().to_vec();
    got.sort_by_key(|d| MATRIX_DIRS.iter().position(|x| x == d));
    expect.sort_by_key(|d| MATRIX_DIRS.iter().position(|x| x == d));
    assert_eq!(got, expect, "only the directions that failed are asked");
    assert!(stubs_before.iter().all(|d| !got.contains(d)), "and never one already found");

    app::scan_probe::settle_exit_scan(&mut p.state, &mut p.mapper);
    for d in [Direction::W, Direction::In] {
        assert!(p.mapper.graph.is_scan_exit(behind, d), "{d:?} is open now: {:?}", p.stubs(behind));
        assert!(!p.mapper.graph.scan_walls(behind).contains(&d));
    }
}

/// An exit the room description never mentions is shown: the Living Room's trap door is under the
/// rug, so `down` is a wall until the rug is moved and the door opened; the rescan on return
/// finds it and the room's unexplored exits include it.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`, `east`, `open window`, `enter window`,
/// `west`, `move rug`, `open trap door`, `east`, `west`.)
#[test]
fn zork1_a_hidden_exit_is_shown_once_it_exists() {
    let Some(mut p) = Play::zork1(true) else { return };
    for cmd in ["north", "east", "open window", "enter window", "west"] {
        p.turn(cmd);
    }
    let living = p.here();
    assert_eq!(p.name(living), "Living Room");
    assert!(!p.mapper.graph.is_scan_exit(living, Direction::Down), "under the rug, no way down yet");
    assert!(!p.mapper.graph.untried(living).contains(&Direction::Down));
    p.turn("move rug");
    p.turn("open trap door");
    p.turn("east");
    p.turn("west");
    assert_eq!(p.here(), living);
    assert!(p.mapper.graph.is_scan_exit(living, Direction::Down), "found on the revisit");
    assert!(p.mapper.graph.untried(living).contains(&Direction::Down));
    assert_eq!(classify(&p.mapper.graph, living, Direction::Down), MatrixCell::Exit);
    assert!(
        !p.mapper.graph.room(living).unwrap().tried.contains(&Direction::Down),
        "the player never typed it"
    );
}

/// With the setting off nothing is scanned: the room keeps today's reading, every compass
/// direction nobody typed.
#[test]
fn zork1_with_the_setting_off_the_map_behaves_as_before() {
    let Some(mut p) = Play::zork1(false) else { return };
    let here = p.here();
    assert!(p.state.scan_queue.is_empty() && p.state.scan_search.is_none());
    assert!(!p.mapper.graph.is_scanned(here));
    assert_eq!(p.mapper.graph.untried(here).len(), 10, "all ten maybes");
    assert_eq!(classify(&p.mapper.graph, here, Direction::N), MatrixCell::Untried);
    p.turn("north");
    assert!(!p.mapper.graph.is_scanned(p.here()));
    assert!(p.mapper.graph.rooms().all(|r| r.scan_exits.is_empty() && r.scan_walls.is_empty()));
}

/// The lowest priority, for real: while a return search for the player's own move is pending the
/// scan does not ask; once the return search is done it does.
#[test]
fn zork1_a_scan_waits_for_the_return_search() {
    let Some(mut p) = Play::zork1(true) else { return };
    let west = p.here();
    let r = p.session.submit("north");
    app::session::apply_turn(&mut p.mapper, "north", &r, &mut p.death);
    let mut save = app::engine::TurnSave::default();
    app::return_probe::arm_return_search(&mut p.state, &p.mapper, &*p.session, "north", Some(west), &mut save);
    p.state.random_exit_pre_move_save = Some((p.here(), save.get(&*p.session)));
    app::scan_probe::arm_exit_scan(&mut p.state, &p.mapper, &*p.session, Some(west));
    assert!(p.state.return_search.is_some(), "the way back to West of House is still to find");
    assert_eq!(p.state.scan_queue.len(), 1, "and the scan of North of House is queued");

    assert!(!app::scan_probe::pump_exit_scan(&mut p.state, &p.mapper), "the return search goes first");
    assert!(p.state.scan_search.is_none(), "the scan has not started");

    let _ = app::return_probe::settle_return_search(&mut p.state, &mut p.mapper);
    assert!(p.state.return_search.is_none());
    app::scan_probe::settle_exit_scan(&mut p.state, &mut p.mapper);
    assert!(p.mapper.graph.is_scanned(p.here()), "then it ran");
}

/// What a scan costs on Zork I, on the worker: wall time per room. Printed, not asserted beyond
/// a generous bound (run with `--no-capture` to read it).
#[test]
fn zork1_scan_cost() {
    let Some(mut p) = Play::zork1(true) else { return };
    let mut total = std::time::Duration::ZERO;
    let mut n = 0;
    for cmd in ["north", "east", "south", "west", "south", "east"] {
        p.turn_unsettled(cmd);
        let t = std::time::Instant::now();
        app::scan_probe::settle_exit_scan(&mut p.state, &mut p.mapper);
        let dt = t.elapsed();
        eprintln!("scan after `{cmd}` in {}: {dt:?}", p.name(p.here()));
        total += dt;
        n += 1;
    }
    eprintln!("zork1 mean scan: {:?} over {n}", total / n);
    assert!(total.as_secs() < 120, "a scan of a Z-machine room is a fraction of a second");
}

// ── How a scanned exit shows (SQ-1773) ────────────────────────────────────────────────────────

mod display {
    use super::*;
    use app::render::map::render_map_layered;
    use mapper::layer::MapView;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    const AREA: Rect = Rect { x: 0, y: 0, width: 110, height: 20 };

    /// Two walked rooms, the first scanned: `up` is a real exit into an unseen room, `in` too.
    fn scanned_map() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", None);
        m.graph.record_scan(1, &[Direction::Up, Direction::In, Direction::Down], &[Direction::Up, Direction::In]);
        m
    }

    fn drawn(m: &mut Mapper, honor: bool) -> (Buffer, AppState) {
        let m2 = m;
        m2.graph.set_layer_view(0, Some(MapView::Matrix));
        let mut st = AppState::default();
        st.config.honor_game_colours = honor;
        st.set_viewed_layer(Some(0));
        assert!(st.map_shows_matrix(&m2.graph), "the pane must be drawing the table");
        let rm = mapper::render::render_layer(&m2.graph, st.active_layer(&m2.graph));
        let mut buf = Buffer::empty(AREA);
        render_map_layered(&rm, &m2.graph, &st, AREA, &mut buf);
        (buf, st)
    }

    fn cell_rect(g: &mapper::graph::MapGraph, room: RoomId, dir: Direction) -> Rect {
        let ml = app::render::matrix::layout(g, 0, AREA.width);
        let row = ml.matrix.index_of(room).expect("a row");
        let col = MATRIX_DIRS.iter().position(|&d| d == dir).unwrap();
        Rect::new(AREA.x + ml.label_w + ml.cell_w * col as u16, AREA.y + 2 + row as u16, ml.cell_w, 1)
    }

    fn glyphs(buf: &Buffer, r: Rect) -> (String, Vec<ratatui::style::Style>) {
        let cells: Vec<_> = (r.x..r.right()).filter_map(|x| buf.cell((x, r.y))).filter(|c| c.symbol() != " ").collect();
        (cells.iter().map(|c| c.symbol()).collect(), cells.iter().map(|c| c.style()).collect())
    }

    /// A scanned exit reads `»` in the exit style, a refused direction `×`, and an unscanned
    /// room's maybes stay `·` -- in both colour modes.
    #[test]
    fn a_scanned_exit_is_a_visibly_different_mark_in_both_colour_modes() {
        let mut m = scanned_map();
        for honor in [true, false] {
            let (buf, st) = drawn(&mut m, honor);
            let exit = st.colors.theme.get("map.matrix.cell:exit").style;
            let frontier = st.colors.theme.get("map.matrix.cell:frontier").style;
            assert_ne!(
                (exit.fg, exit.add_modifier),
                (frontier.fg, frontier.add_modifier),
                "the yes does not look like the maybe (honor={honor})"
            );

            let (sym, styles) = glyphs(&buf, cell_rect(&m.graph, 1, Direction::Up));
            assert_eq!(sym, "»", "an exit (honor={honor})");
            assert!(styles.iter().all(|s| s.fg == exit.fg), "in the exit style (honor={honor})");
            assert_eq!(glyphs(&buf, cell_rect(&m.graph, 1, Direction::In)).0, "»");
            assert_eq!(glyphs(&buf, cell_rect(&m.graph, 1, Direction::Down)).0, "×", "refused: a wall");
            assert_eq!(glyphs(&buf, cell_rect(&m.graph, 1, Direction::S)).0, "×", "scanned: no maybes");
            assert_eq!(glyphs(&buf, cell_rect(&m.graph, 2, Direction::Up)).0, "·", "unscanned: still a maybe");
        }
    }

    /// The selector is in the registry with its documented default, so `style.toml` can restyle it.
    #[test]
    fn the_exit_selector_resolves_and_is_listed() {
        let st = AppState::default();
        let exit = st.colors.theme.get("map.matrix.cell:exit").style;
        assert!(exit.fg.is_some(), "it resolves to a real colour (the `accent` role)");
        let accent = st.colors.theme.get("accent").style;
        assert_eq!(exit.fg, accent.fg, "by default it reads as the accent");
    }
}

// ── Scott Adams, and self-loops ───────────────────────────────────────────────────────────────

/// What the game's own database declares from the start room, in column order: the truth the
/// scan is held to. NOT "the room changed": Adventureland's forest lists
/// `Obvious exits: North. South. East. West.` and three of those lead back into the same forest.
fn scott_declared(bytes: &[u8], room: RoomId) -> Vec<Direction> {
    let s = app::scott_session::ScottSession::new(bytes.to_vec(), None).unwrap();
    MATRIX_DIRS
        .iter()
        .copied()
        .filter(|&d| s.declared_exit(room, d) != app::engine::DeclaredExit::Unknown)
        .collect()
}

/// The start forest: N, S, E and W are all real exits (the game says so), though only E changes
/// room. The scan finds all four, shows all four as `»` -- a self-loop is not `↩` until walked, so
/// the maze stays a puzzle -- records no passage, and nothing else is an exit.
///
/// (Fixture: `adv01.dat`; the opening `look` only.)
#[test]
fn adventureland_forest_self_loops_are_exits_not_walls() {
    let Some(bytes) = story("adv01.dat") else { return };
    let Some(p) = Play::adventureland() else { return };
    let here = p.here();
    let want = vec![Direction::N, Direction::S, Direction::E, Direction::W];
    assert_eq!(scott_declared(&bytes, here), want, "the database's own exit table");
    assert!(p.mapper.graph.is_scanned(here), "the scan finished on Scott");
    assert_eq!(p.stubs(here), want);
    assert_eq!(p.mapper.graph.untried(here), want);
    for d in &want {
        assert_eq!(classify(&p.mapper.graph, here, *d), MatrixCell::Exit, "{d:?}");
    }
    assert!(p.mapper.graph.connections().is_empty(), "no passage, no hint that any of it loops");
    assert_eq!(p.mapper.graph.rooms().count(), 1);
    assert_eq!(classify(&p.mapper.graph, here, Direction::Up), MatrixCell::Probed);
}

/// Walking a self-loop reads `↩`, not `×`: the game's own exit table says the way is there, so a
/// wall would be a lie, and the scanned `»` is gone (it has been walked). Scott prints no heading
/// on a loop, so the declared exit is the only evidence. A refused direction still reads `×`, and
/// nothing is marked random.
#[test]
fn adventureland_a_walked_self_loop_reads_as_one_and_a_refusal_still_reads_as_a_wall() {
    let Some(mut p) = Play::adventureland() else { return };
    let here = p.here();
    assert_eq!(classify(&p.mapper.graph, here, Direction::N), MatrixCell::Exit, "scanned: a yes");
    p.turn("north");
    assert_eq!(p.here(), here, "north leads back into the forest");
    assert_eq!(classify(&p.mapper.graph, here, Direction::N), MatrixCell::SelfLoop);
    assert!(!p.mapper.graph.untried(here).contains(&Direction::N), "walked: no longer unexplored");
    assert!(!p.mapper.graph.is_random_exit(here, Direction::N));
    assert_eq!(p.mapper.graph.self_loops(here), vec![Direction::N], "recorded once");
    assert_eq!(classify(&p.mapper.graph, here, Direction::S), MatrixCell::Exit, "unwalked: still a yes");

    p.turn("up");
    assert_eq!(classify(&p.mapper.graph, here, Direction::Up), MatrixCell::Probed, "refused: a wall");
    assert!(p.mapper.graph.self_loops(here).iter().all(|d| *d != Direction::Up));

    p.turn("east");
    assert_ne!(p.here(), here);
    assert!(p.mapper.graph.rooms().all(|r| r.random_exits.is_empty()), "no random marks");
}

/// `declared_exit` is only useful to its consumers if it never contradicts a real walk: from every
/// room reachable by plain direction words, every direction that lands somewhere else lands in
/// the room the table declares. (A mismatch would read as a suspicion of a random exit.)
#[test]
fn adventureland_the_exit_table_agrees_with_every_reachable_plain_walk() {
    let Some(bytes) = story("adv01.dat") else { return };
    let mut s = app::scott_session::ScottSession::new(bytes, None).unwrap();
    let start = s.save_state();
    let mut seen: Vec<RoomId> = Vec::new();
    let mut todo: Vec<(RoomId, app::engine::EngineSave)> =
        vec![(s.current_location().unwrap().number, start)];
    let mut checked = 0;
    while let Some((room, save)) = todo.pop() {
        if seen.contains(&room) {
            continue;
        }
        seen.push(room);
        for dir in [Direction::N, Direction::S, Direction::E, Direction::W, Direction::Up, Direction::Down] {
            s.restore_state(&save).unwrap();
            let r = s.submit(mapper::direction::long_label(dir));
            if app::session::turn_reports_death(&r.transcript) {
                continue;
            }
            let landed = s.current_location().unwrap().number;
            let declared = s.declared_exit(room, dir);
            if landed != room {
                assert_eq!(
                    declared,
                    app::engine::DeclaredExit::Room(landed),
                    "room {room} {dir:?}: the table disagrees with the walk"
                );
                checked += 1;
                todo.push((landed, s.save_state()));
            }
        }
    }
    assert!(seen.len() > 5 && checked > 5, "a real stretch of the map was walked ({} rooms)", seen.len());
}
