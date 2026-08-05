//! A fatal move must not leave the direction on the room's `tried` record (SQ-0671).
//!
//! Walking north into a grue mints no edge, so the matrix drew the direction as `_` — "tried, and
//! there is no path that way". That is a claim the turn never made: dying tells you nothing about
//! whether the passage is open. The record is rolled back and the cell stays `·`, on the
//! exploration frontier where it belongs.
//!
//! The timing is the interesting part. Most games print `*** You have died ***` on the turn that
//! kills you, but Adventure asks whether to reincarnate you first — so the banner (if it ever
//! comes) arrives on the turn that ANSWERS, and the rollback still has to undo the move that
//! killed the player, not whatever they typed at the prompt.

use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;

use app::session::{
    apply_turn, rollback_tried_on_death, tried_record_for, turn_reports_death, TurnResult,
};

fn turn(num: u16, name: &str, transcript: &str) -> TurnResult {
    TurnResult {
        transcript: transcript.into(),
        transcript_runs: Vec::new(),
        location: Some(zvm::ObjectSnapshot { number: num, parent: 0, name: name.into() }),
        quit: false,
        erase_lower: false,
        info: None,
        sounds: Vec::new(),
        glulx_sound_ops: Vec::new(),
        diagnostics: Vec::new(),
        fault: None,
        location_method: None,
        pending_io: None,
        timed_out: false,
        pictures: Vec::new(),
        transcript_elems: Vec::new(),
    }
}

/// One turn of the run loop's mapping, exactly as `finish_command_turn` runs it: capture what the
/// turn is about to record, apply it, then roll back if the turn reported a death.
fn play(m: &mut Mapper, pending: &mut Option<(RoomId, Direction)>, cmd: &str, r: &TurnResult) {
    let attempted = tried_record_for(m, cmd);
    apply_turn(m, cmd, r);
    rollback_tried_on_death(m, pending, attempted, turn_reports_death(&r.transcript));
}

const DEATH_BANNER: &str = "Oh, no! A lurking grue slithered into the room and devoured you!\n \n   ****  You have died  **** \n\nForest\n";

// ── The turn that kills you says so ───────────────────────────────────────────

#[test]
fn a_fatal_move_leaves_the_direction_untried_while_a_wall_still_records() {
    let mut m = Mapper::default();
    let mut pending = None;

    play(&mut m, &mut pending, "", &turn(1, "Living Room", "Living Room\n"));
    play(&mut m, &mut pending, "down", &turn(2, "Cellar", "Cellar\n"));

    // A move that bounces off a wall: the location does not change and no heading is reprinted.
    // That IS knowledge — the direction was tried and there is no way through — and must survive.
    play(&mut m, &mut pending, "east", &turn(2, "Cellar", "You can't go that way.\n"));
    assert!(m.graph.is_tried(2, Direction::E), "a refused move is still a tried direction");

    // The fatal move: a grue eats the player and the game resurrects them in the Forest.
    play(&mut m, &mut pending, "north", &turn(3, "Forest", DEATH_BANNER));

    assert!(
        !m.graph.is_tried(2, Direction::N),
        "the direction that killed the player is not on the record: {:?}",
        m.graph.room(2).unwrap().tried,
    );
    assert!(
        m.graph.untried(2).contains(&Direction::N),
        "…so it is still on the exploration frontier"
    );
    assert!(m.graph.is_tried(2, Direction::E), "and the wall to the east is untouched by that");
    assert!(
        !m.graph.connections().iter().any(|c| c.origin == 2 && c.dest == 3),
        "no passage was minted either (SQ-0259)"
    );
}

/// The rollback drops the TYPED record only. A direction that had already proved itself keeps its
/// passage, and stays tried on the strength of it.
#[test]
fn a_direction_that_already_led_somewhere_keeps_its_passage() {
    let mut m = Mapper::default();
    let mut pending = None;
    play(&mut m, &mut pending, "", &turn(1, "Hall", "Hall\n"));
    play(&mut m, &mut pending, "north", &turn(2, "Study", "Study\n"));
    play(&mut m, &mut pending, "south", &turn(1, "Hall", "Hall\n"));
    // North again — and this time something in the Study kills the player.
    play(&mut m, &mut pending, "north", &turn(3, "Forest", DEATH_BANNER));

    assert!(m.graph.is_tried(1, Direction::N), "the Hall's north passage is still known");
    assert!(
        m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N && c.dest == 2),
        "and the edge that proves it was never touched"
    );
}

// ── The turn that kills you asks a question first ─────────────────────────────

/// Adventure's shape: the fatal turn offers to reincarnate you, and the banner only arrives if
/// you decline — several turns later, because "Please answer yes or no." is a turn of its own.
#[test]
fn a_death_admitted_turns_later_still_rolls_back_the_move_that_caused_it() {
    let mut m = Mapper::default();
    let mut pending = None;
    play(&mut m, &mut pending, "", &turn(1, "In Cobble Crawl", "In Cobble Crawl\n"));
    play(&mut m, &mut pending, "west", &turn(2, "Darkness", "Darkness\n"));

    // The fatal move. Adventure prints no banner here — just the offer.
    play(
        &mut m,
        &mut pending,
        "up",
        &turn(
            2,
            "Darkness",
            "You fell into a pit and broke every bone in your body!\n\nOh dear, you seem to have \
             gotten yourself killed. I might be able to help you out, but I've never really done \
             this before. Do you want me to try to reincarnate you?\n",
        ),
    );
    assert!(
        !m.graph.is_tried(2, Direction::Up),
        "the offer is an admission of death: roll the move back on the spot"
    );

    // Now the same shape for a game that only admits it on the answer. Replay from a clean slate
    // with the offer text stripped of its death words, so the fatal turn is NOT recognised.
    let mut m = Mapper::default();
    let mut pending = None;
    play(&mut m, &mut pending, "", &turn(1, "In Cobble Crawl", "In Cobble Crawl\n"));
    play(&mut m, &mut pending, "west", &turn(2, "Darkness", "Darkness\n"));
    play(&mut m, &mut pending, "up", &turn(2, "Darkness", "You fall a long way.\n"));
    assert!(m.graph.is_tried(2, Direction::Up), "nothing said death yet, so it reads as a probe");
    assert_eq!(pending, Some((2, Direction::Up)), "…but the move is held, in case it was fatal");

    // Two turns of the game insisting on an answer, then the banner.
    play(&mut m, &mut pending, "look", &turn(2, "Darkness", "Please answer yes or no.\n"));
    play(&mut m, &mut pending, "no", &turn(2, "Darkness", DEATH_BANNER));
    assert!(
        !m.graph.is_tried(2, Direction::Up),
        "the rollback belongs to the turn that CONTAINED the fatal move, not the answer"
    );
}

/// The held record is not held forever: once the player has walked out of the room they typed it
/// in, whatever it recorded is settled and a later death must not reach back for it.
#[test]
fn a_death_after_the_player_has_moved_on_does_not_reach_back() {
    let mut m = Mapper::default();
    let mut pending = None;
    play(&mut m, &mut pending, "", &turn(1, "Hall", "Hall\n"));
    play(&mut m, &mut pending, "east", &turn(1, "Hall", "You can't go that way.\n"));
    assert_eq!(pending, Some((1, Direction::E)));

    play(&mut m, &mut pending, "north", &turn(2, "Study", "Study\n"));
    play(&mut m, &mut pending, "wait", &turn(2, "Study", "Time passes.\n"));
    assert_eq!(pending, None, "the player left the Hall: its east wall is settled knowledge");

    play(&mut m, &mut pending, "wait", &turn(3, "Forest", DEATH_BANNER));
    assert!(m.graph.is_tried(1, Direction::E), "the Hall's east wall survived a later death");
}

// ── The real thing ────────────────────────────────────────────────────────────

/// Adventure, driven for real: walk into the dark below the grate and fall into the pit. The
/// killing move is `west` out of a room whose west has never been tried, so the rollback has
/// something to undo and the assertion cannot pass by accident.
///
/// The route is deterministic — no dwarves, no lamp, and the pit takes the second move made in
/// the dark. Skips vacuously without the gitignored story file.
#[test]
fn adventures_pit_leaves_the_direction_untried() {
    use app::engine::Engine;
    use app::glulx_session::GlulxSession;

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/advent.blb");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let blorb = blorb::Blorb::parse(bytes).expect("advent.blb parses as a Blorb");
    let (_k, exec) = blorb.executable().expect("advent.blb carries an executable chunk");
    let mut s = GlulxSession::new(exec.to_vec(), 80, 24, true, false, false, (1, 1), None, &[])
        .expect("Adventure (Glulx) boots");

    let mut m = Mapper::default();
    let mut pending = None;
    let approach = [
        "in", "get lamp", "get keys", "out", "south", "south", "south",
        "unlock grate with keys", "open grate", "down", "west", "west", "up",
    ];
    for cmd in approach {
        let r = Engine::submit(&mut s, cmd);
        play(&mut m, &mut pending, cmd, &r);
    }
    // Standing in the dark, one move from the pit, with west untried from here.
    let here = m.graph.current().expect("the mapper is following the player");
    assert!(!m.graph.is_tried(here, Direction::W), "west is fresh in this room");
    let elsewhere: Vec<_> = m
        .graph
        .rooms()
        .filter(|r| r.id != here && !r.tried.is_empty())
        .map(|r| (r.id, r.tried.clone()))
        .collect();
    assert!(!elsewhere.is_empty(), "ordinary moves are being recorded as tried");

    let fatal = Engine::submit(&mut s, "west");
    assert!(
        fatal.transcript.contains("broke every bone"),
        "expected the pit death, got: {}",
        fatal.transcript
    );
    assert!(turn_reports_death(&fatal.transcript), "Adventure's death must be recognised as one");
    play(&mut m, &mut pending, "west", &fatal);

    assert!(
        !m.graph.is_tried(here, Direction::W),
        "the move that killed the player is not recorded as tried: {:?}",
        m.graph.room(here).unwrap().tried,
    );
    assert!(
        !m.graph.connections().iter().any(|c| c.origin == here && c.dir == Direction::W),
        "and no passage was minted to wherever the pit dropped them"
    );
    // Every other room's record is exactly as it was: the rollback is one direction in one room.
    let after: Vec<_> = m
        .graph
        .rooms()
        .filter(|r| r.id != here && !r.tried.is_empty())
        .map(|r| (r.id, r.tried.clone()))
        .collect();
    assert_eq!(after, elsewhere, "normal moves elsewhere still record their directions");
}
