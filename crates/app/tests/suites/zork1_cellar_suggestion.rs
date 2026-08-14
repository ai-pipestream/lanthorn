//! SQ-0853: Zork I's cellar has to offer itself a layer while the player is still down there.
//!
//! The reported symptom, from a player verifying SQ-0439: "in zork-1 i'm exploring the cellar and
//! have never gotten prompted to peel, even after 7 rooms". The detector was wired correctly and
//! the four-room floor was cleared long before; what silenced it was the timing rule. The
//! structural trigger only ever fired on a RETURN crossing — a portal into a room the map already
//! knew — and Zork's trapdoor crashes shut and is barred behind you, so that crossing never
//! happens. The first prompt would have arrived climbing out of the chimney, hours later.
//!
//! This drives the real game down the real trapdoor and pins both halves of the fix: the prompt
//! arrives on the fourth underground room, and the rooms it offers are the ones BELOW the seam,
//! never the five surface rooms the player came from.
//!
//! The story is gitignored, so this skips vacuously when absent — CI has no `stories/` at all.

use std::path::PathBuf;

use app::engine::Engine;
use app::session::{apply_turn, DeathWatch, GameSession, InputKind};
use mapper::direction::Direction;
use mapper::graph::RoomId;
use mapper::mapper::Mapper;
use mapper::suggest::{LayerSuggestion, Trigger};

fn story_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork1-r88-s840726.z3")
}

/// Boot Zork I release 88 / serial 840726 to its first line prompt, in West of House.
fn boot_zork1() -> Option<GameSession> {
    let path = story_path();
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let session = GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some((25, 80)),
    )
    .expect("zork1-r88-s840726.z3 should load and boot without a ZError");
    assert_eq!(session.pending_input(), InputKind::Line, "a v3 game opens straight at a prompt");
    Some(session)
}

/// One turn: submit it, feed it to the map, and hand back whatever the map made of it.
fn turn(
    session: &mut GameSession,
    map: &mut Mapper,
    death: &mut DeathWatch,
    cmd: &str,
) -> (String, Option<LayerSuggestion>) {
    let r = session.submit(cmd);
    assert!(!r.quit && r.fault.is_none(), "{cmd:?} faulted or quit: {:?}", r.fault);
    apply_turn(map, cmd, &r, death);
    (r.transcript.clone(), map.take_suggestion())
}

fn here(map: &Mapper) -> (RoomId, String) {
    let id = map.graph.current().expect("the map must know where the player is");
    (id, map.graph.room(id).expect("a current room is a real room").name.clone())
}

/// The whole reported walk: through the house to the Living Room, down the barred trapdoor, and
/// four rooms into the underground. The map must speak on the fourth, and what it offers must be
/// the underground.
#[test]
fn exploring_zork1s_cellar_offers_the_underground_a_layer_of_its_own() {
    let Some(mut session) = boot_zork1() else { return };
    let mut map = Mapper::default();
    let mut death = DeathWatch::default();

    // ── The surface. Five rooms, all reached by compass passages, and not one word from the map:
    // the starting region predates everything, so nothing can claim it as an appendage.
    let surface = [
        ("look", "West of House"),
        ("north", "North of House"),
        ("east", "Behind House"),
        ("open window", "Behind House"),
        ("west", "Kitchen"),
        ("west", "Living Room"),
    ];
    let mut above: Vec<RoomId> = Vec::new();
    for (cmd, want) in surface {
        let (text, suggestion) = turn(&mut session, &mut map, &mut death, cmd);
        let (id, name) = here(&map);
        assert_eq!(name, want, "{cmd:?} should reach {want}: {text:?}");
        if !above.contains(&id) {
            above.push(id);
        }
        assert!(
            suggestion.is_none(),
            "the surface must never be offered as a region: {suggestion:?}"
        );
    }
    assert_eq!(above.len(), 5, "five distinct surface rooms: {above:?}");
    let living_room = *above.last().expect("the Living Room is where the trapdoor is");

    // Housekeeping the descent needs: a lit lamp, the rug moved, the trapdoor open. None of it
    // moves the player, so none of it can produce a suggestion either.
    for cmd in ["take lamp", "turn on lamp", "move rug", "open trap door"] {
        let (_, suggestion) = turn(&mut session, &mut map, &mut death, cmd);
        assert!(suggestion.is_none(), "{cmd:?} moved nobody and must say nothing");
    }

    // ── Down. The trapdoor is barred behind the player on this very turn, which is exactly why
    // waiting for a return crossing meant waiting forever.
    let (text, suggestion) = turn(&mut session, &mut map, &mut death, "down");
    assert!(
        text.contains("trap door crashes shut") && text.contains("barring it"),
        "the fixture must be the release whose trapdoor bars itself: {text:?}"
    );
    assert_eq!(here(&map).1, "Cellar", "down reaches the Cellar: {text:?}");
    assert!(suggestion.is_none(), "one room behind a trapdoor is a doorway, not a floor plan");

    // ── Three more rooms, and the map speaks on the last of them.
    let mut below: Vec<RoomId> = vec![here(&map).0];
    let mut spoke: Option<LayerSuggestion> = None;
    for (cmd, want) in [("south", "East of Chasm"), ("east", "Gallery"), ("north", "Studio")] {
        let (text, suggestion) = turn(&mut session, &mut map, &mut death, cmd);
        let (id, name) = here(&map);
        assert_eq!(name, want, "{cmd:?} should reach {want}: {text:?}");
        below.push(id);
        if let Some(s) = suggestion {
            assert!(spoke.is_none(), "it must say its piece once, not once per room");
            spoke = Some(s);
        }
    }

    let s = spoke.expect("four rooms under a barred trapdoor is when the map has something to say");
    assert_eq!(s.trigger, Trigger::Structural, "a cellar is not a maze");
    assert_eq!(
        s.seam,
        mapper::suggest::SeamKey { from: living_room, dir: Direction::Down },
        "remembered under the trapdoor the region hangs off"
    );

    // The heart of it: the rooms offered are the ones BEYOND the seam.
    assert_eq!(
        s.region.rooms.iter().copied().collect::<Vec<_>>(),
        {
            let mut v = below.clone();
            v.sort_unstable();
            v
        },
        "the four underground rooms, and only those"
    );
    for id in &above {
        assert!(
            !s.region.rooms.contains(id),
            "the surface must never be what is offered: {:?} in {:?}",
            map.graph.room(*id).map(|r| r.name.clone()),
            s.region.rooms
        );
    }
    assert!(!s.destinations.is_empty(), "a suggestion with nowhere to go is never made");
    assert_eq!(map.graph.layers().len(), 1, "and nothing has moved: it only suggests");
}
