//! The relocation probe (SQ-1771): a crossing made with a command that names no direction --
//! `climb tree`, `jump in river` -- is given the direction the game itself accepts for it, found
//! in the silent shadow from the state before the command.
//!
//! Real-game cases skip vacuously without `stories/` (gitignored), the CI-safe pattern; the
//! timed-relocation case is held at the probe-logic level in `relocation_probe.rs`'s own unit
//! tests (a scripted answer), because no convenient real story carries a clock that moves the
//! player whatever they type.

use std::path::PathBuf;
use std::sync::Arc;

use app::engine::Engine;
use app::probe::ShadowRecipe;
use app::state::AppState;

use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../stories")
        .join(name);
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

/// A real story driven the way `host::turn::finish_command_turn` drives it: submit, `apply_turn`,
/// arm the searches, let them finish, and keep the end-of-turn snapshot for the next turn.
struct Play {
    state: AppState,
    mapper: Mapper,
    session: Box<dyn Engine>,
    death: app::session::DeathWatch,
}

impl Play {
    fn new(session: Box<dyn Engine>, bytes: &[u8]) -> Play {
        let mut state = AppState::default();
        state.config.return_probe = true;
        state.probe.arm(recipe(bytes));
        let mut p = Play {
            state,
            mapper: Mapper::default(),
            session,
            death: Default::default(),
        };
        p.turn("look");
        p
    }

    fn zork1() -> Option<Play> {
        Play::zork1_file("zork1-r88-s840726.z3")
    }

    fn zork1_file(name: &str) -> Option<Play> {
        let bytes = story(name)?;
        let mut s = app::session::GameSession::new_with_trace(
            bytes.clone(),
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
        Some(Play::new(Box::new(s), &bytes))
    }

    fn adventureland() -> Option<Play> {
        let bytes = story("adv01.dat")?;
        let s =
            app::scott_session::ScottSession::new(bytes.clone(), None).expect("adv01.dat loads");
        Some(Play::new(Box::new(s), &bytes))
    }

    fn turn(&mut self, cmd: &str) {
        let r = self.session.submit(cmd);
        let room_before = self.mapper.graph.current();
        app::session::apply_turn(&mut self.mapper, cmd, &r, &mut self.death);
        let mut save = app::engine::TurnSave::default();
        app::return_probe::arm_return_search(
            &mut self.state,
            &self.mapper,
            &*self.session,
            cmd,
            room_before,
            &mut save,
        );
        app::relocation_probe::arm_relocation_search(
            &mut self.state,
            &self.mapper,
            &*self.session,
            cmd,
            room_before,
        );
        app::relocation_probe::settle_relocation_search(&mut self.state, &mut self.mapper);
        let _ = app::return_probe::settle_return_search(&mut self.state, &mut self.mapper);
        self.state.random_exit_pre_move_save = Some((
            self.mapper.graph.current().unwrap_or(0),
            save.get(&*self.session),
        ));
    }

    fn here(&self) -> RoomId {
        self.mapper.graph.current().expect("a current room")
    }

    fn name(&self, id: RoomId) -> String {
        self.mapper
            .graph
            .room(id)
            .map(|r| r.label().to_string())
            .unwrap_or_default()
    }

    fn edge(&self, from: RoomId, dir: Direction) -> Option<RoomId> {
        self.mapper
            .graph
            .connections()
            .iter()
            .find(|c| c.origin == from && c.dir == dir)
            .map(|c| c.dest)
    }

    fn unlabelled(&self) -> usize {
        self.mapper
            .graph
            .connections()
            .iter()
            .filter(|c| c.dir == Direction::Unknown)
            .count()
    }

    fn dump(&self) -> String {
        self.mapper
            .graph
            .connections()
            .iter()
            .map(|c| {
                format!(
                    "{}({}) -{:?}-> {}({})",
                    self.name(c.origin),
                    c.origin,
                    c.dir,
                    self.name(c.dest),
                    c.dest
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Forest Path, then `climb tree`: Up a Tree is placed as UP of the path, and `climb down` from
/// there places DOWN -- neither command names a direction the mapper understands.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`, `north`, `climb tree`, `climb down`.)
#[test]
fn zork1_climbing_the_tree_is_up_and_climbing_down_is_down() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("north");
    p.turn("north");
    let path = p.here();
    assert_eq!(
        p.name(path),
        "Forest Path",
        "the walk reached Forest Path: {}",
        p.dump()
    );

    p.turn("climb tree");
    let tree = p.here();
    assert_eq!(p.name(tree), "Up a Tree");
    assert_eq!(p.edge(path, Direction::Up), Some(tree), "up: {}", p.dump());
    assert_eq!(p.unlabelled(), 0, "the ? stub was replaced: {}", p.dump());
    let (a, b) = (
        p.mapper.graph.room(path).unwrap().pos.unwrap(),
        p.mapper.graph.room(tree).unwrap().pos.unwrap(),
    );
    assert_eq!(b, (a.0, a.1 - 1), "and the room is laid out above the path");

    p.turn("climb down");
    assert_eq!(p.here(), path);
    assert_eq!(
        p.edge(tree, Direction::Down),
        Some(path),
        "down: {}",
        p.dump()
    );
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}

/// The same on the Version 5 release (r52), whose shadow `look` reports no location: the control
/// must not read an unknown location as a landing in the destination.
///
/// (Fixture: `zork1-invclues-r52-s871125.z5`; `look`, `north`, `north`, `climb tree`.)
#[test]
fn zork1_r52_climbing_the_tree_is_still_placed_up() {
    let Some(mut p) = Play::zork1_file("zork1-invclues-r52-s871125.z5") else { return };
    p.turn("north");
    p.turn("north");
    let path = p.here();
    assert_eq!(p.name(path), "Forest Path", "{}", p.dump());
    p.turn("climb tree");
    let tree = p.here();
    assert_eq!(p.name(tree), "Up a Tree");
    assert_eq!(p.edge(path, Direction::Up), Some(tree), "up: {}", p.dump());
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}

/// A room change no direction reproduces stays exactly as it was: unplaced, a `?`.
///
/// Adventureland (`adv01.dat`): `climb tree` in the forest reaches the top of an oak, but the
/// game does not accept `up` there (its trees are `go tree` travel, not a vertical exit) -- so
/// every direction fails in the shadow and nothing is placed. The search ran (it is finished,
/// not merely unarmed) and left the map alone.
#[test]
fn adventureland_climbing_the_tree_is_not_a_direction_and_stays_unplaced() {
    let Some(mut p) = Play::adventureland() else {
        return;
    };
    let forest = p.here();
    p.turn("climb tree");
    assert_ne!(p.here(), forest, "the command moved the player");
    assert_eq!(p.unlabelled(), 1, "still the ? it always was: {}", p.dump());
    assert!(
        p.mapper
            .graph
            .connections()
            .iter()
            .all(|c| c.dir == Direction::Unknown),
        "{}",
        p.dump()
    );
    assert!(
        p.state.relocation_search.is_none(),
        "and the search ran to its end"
    );
}

/// In/Out are portals, which the layout cuts the map at; a compass word that walks between the
/// same two rooms lets it draw them flat. After `enter window` (placed as In by the main search)
/// the compass alias is recorded ALONGSIDE it, and a typed `out` gets its own.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, `north`, `east`, `open window`, `enter window`,
/// `out`.)
#[test]
fn zork1_the_window_gains_a_compass_alias_beside_in_and_out() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("north");
    p.turn("east");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House", "{}", p.dump());
    p.turn("open window");
    p.turn("enter window");
    let kitchen = p.here();
    assert_eq!(p.name(kitchen), "Kitchen", "{}", p.dump());
    assert_eq!(
        p.edge(behind, Direction::In),
        Some(kitchen),
        "the observed in: {}",
        p.dump()
    );
    assert_eq!(
        p.edge(behind, Direction::W),
        Some(kitchen),
        "and the compass alias: {}",
        p.dump()
    );
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());

    p.turn("out");
    assert_eq!(p.here(), behind);
    assert_eq!(
        p.edge(kitchen, Direction::Out),
        Some(behind),
        "{}",
        p.dump()
    );
    assert_eq!(p.edge(kitchen, Direction::E), Some(behind), "{}", p.dump());
}
