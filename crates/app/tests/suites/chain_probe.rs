//! A command line that chains several commands (`north then east`) is replayed a piece at a
//! time in the shadow, and the map gets each step that really happened instead of one passage
//! under the first direction (SQ-1772). See `app::chain_probe` for the design and for the table of
//! what each game accepts as a separator.
//!
//! Real-game cases skip vacuously without `stories/` (gitignored), the CI-safe pattern.

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
        let bytes = story("zork1-r88-s840726.z3")?;
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
        app::chain_probe::arm_chain_replay(
            &mut self.state,
            &self.mapper,
            &*self.session,
            cmd,
            room_before,
        );
        app::chain_probe::settle_chain_replay(&mut self.state, &mut self.mapper);
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

/// The bug as reported: from West of House, `go north then east` filed one passage north that
/// goes nowhere near Behind House. The map now has the two passages the line walked, through a
/// room the player never stood in.
///
/// (Fixture: `zork1-r88-s840726.z3`; `look`, then `go north then east`.)
#[test]
fn zork1_north_then_east_maps_each_step_not_one_passage() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    assert_eq!(p.name(west), "West of House");
    p.turn("go north then east");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House", "{}", p.dump());
    let north = p.edge(west, Direction::N).unwrap_or_else(|| panic!("{}", p.dump()));
    assert_eq!(p.name(north), "North of House", "{}", p.dump());
    assert_eq!(p.edge(north, Direction::E), Some(behind), "{}", p.dump());
    assert_ne!(
        p.edge(west, Direction::N),
        Some(behind),
        "no passage north straight to Behind House: {}",
        p.dump()
    );
    assert_eq!(p.unlabelled(), 0, "the provisional ? was replaced: {}", p.dump());
}

/// A period separates commands in Zork I too (`n. e`).
#[test]
fn zork1_a_period_separates_commands() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    p.turn("n. e");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House", "{}", p.dump());
    let north = p.edge(west, Direction::N).unwrap_or_else(|| panic!("{}", p.dump()));
    assert_eq!(p.edge(north, Direction::E), Some(behind), "{}", p.dump());
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}

/// A chain that stops part-way: `north then up` -- Zork walks north and refuses `up`. Only the
/// step that happened is recorded.
#[test]
fn zork1_a_refused_step_records_only_the_real_one() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    p.turn("north then up");
    let north = p.here();
    assert_eq!(p.name(north), "North of House", "{}", p.dump());
    assert_eq!(p.edge(west, Direction::N), Some(north), "{}", p.dump());
    assert!(
        p.mapper.graph.connections().iter().all(|c| c.dir != Direction::Up),
        "the refused up is not a passage: {}",
        p.dump()
    );
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}

/// A line that walks out and back still walked: `north then west` ends where it began, and both
/// passages (and the room between) are on the map.
#[test]
fn zork1_a_line_that_walks_out_and_back_records_both_passages() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    p.turn("north then west");
    assert_eq!(p.here(), west, "{}", p.dump());
    let north = p.edge(west, Direction::N).unwrap_or_else(|| panic!("{}", p.dump()));
    assert_eq!(p.name(north), "North of House");
    assert_eq!(p.edge(north, Direction::W), Some(west), "{}", p.dump());
}

/// A piece that changes room without naming a direction goes through the SQ-1771 search from the
/// state just before it: `open window then enter window` from Behind House is In plus the West
/// alias, exactly as typing the two commands one at a time.
#[test]
fn zork1_a_directionless_piece_in_a_chain_is_named_by_the_relocation_search() {
    let Some(mut p) = Play::zork1() else { return };
    p.turn("north");
    p.turn("east");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House", "{}", p.dump());
    p.turn("open window then enter window");
    let kitchen = p.here();
    assert_eq!(p.name(kitchen), "Kitchen", "{}", p.dump());
    assert_eq!(p.edge(behind, Direction::In), Some(kitchen), "{}", p.dump());
    assert_eq!(p.edge(behind, Direction::W), Some(kitchen), "{}", p.dump());
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}

/// A comma that addresses an actor is not a walk: `mailbox, north` is refused by Zork ("You can't
/// talk to the small mailbox!"), nobody moves, and the map stays empty. (A comma between two
/// commands, `north, east`, walks both in Zork I -- the replay settles which a line is.)
#[test]
fn zork1_a_comma_that_addresses_an_actor_maps_nothing() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    p.turn("mailbox, north");
    assert_eq!(p.here(), west);
    assert!(p.mapper.graph.connections().is_empty(), "{}", p.dump());
}

/// ...and a comma between commands is a chain in Zork I, recorded step by step.
#[test]
fn zork1_a_comma_between_commands_is_a_chain() {
    let Some(mut p) = Play::zork1() else { return };
    let west = p.here();
    p.turn("north, east");
    let behind = p.here();
    assert_eq!(p.name(behind), "Behind House", "{}", p.dump());
    let north = p.edge(west, Direction::N).unwrap_or_else(|| panic!("{}", p.dump()));
    assert_eq!(p.edge(north, Direction::E), Some(behind), "{}", p.dump());
}

/// Scott Adams reads the first command and ignores the rest of the line. `east then west` walks
/// east and stops, and the map agrees: one passage east (the return search may add the way back, as after any walked move).
///
/// (Fixture: `adv01.dat`, Adventureland, starting in the forest.)
#[test]
fn adventureland_a_chained_line_is_mapped_as_the_one_command_the_game_ran() {
    let Some(mut p) = Play::adventureland() else { return };
    let forest = p.here();
    p.turn("east then west");
    let meadow = p.here();
    assert_ne!(meadow, forest, "the first command ran: {}", p.dump());
    assert_eq!(p.edge(forest, Direction::E), Some(meadow), "{}", p.dump());
    assert!(
        p.mapper.graph.connections().iter().all(|c| c.dir != Direction::Unknown),
        "{}",
        p.dump()
    );
    assert_eq!(p.unlabelled(), 0, "{}", p.dump());
}
