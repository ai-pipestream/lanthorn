//! v6 automapper location smoke (SQ-0468).
//!
//! v6 (graphical) Z-machine games never populate the v4+ status-line grid —
//! their status text is PAINT in the v6 window model — so `detect_location`
//! returned `None` for them and the automapper never saw a room. The engine now
//! sources v6 candidates from the paint runs and feeds the SAME validation
//! ladder. These headless smokes assert the mapper actually receives rooms:
//!   - Zork Zero: PlayerParent-validated room at boot (Banquet Hall) that
//!     CHANGES on a movement command (→ Scullery via "ne").
//!   - Shogun: a validated location (Bridge) once past the boot menu, with the
//!     "Erasmus" ship label and "SHOGUN" banner NEVER becoming the room.
//!   - Arthur (SQ-0530): a validated room (Churchyard) that changes on a move,
//!     from a status bar hung TWELVE rows down the screen rather than at the top.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None)
            .expect("v6 story should load and boot without a ZError");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

#[test]
fn zork0_v6_location_detected_and_changes_on_move() {
    let Some(mut session) = boot("zork0-r393-s890714.z6") else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };

    // At boot Zork0's status band already names the opening room. The avatar's
    // object-tree parent IS that room, so detection is the strongest form.
    let start = session.current_location().expect("Zork0 opening room must be detected");
    assert!(
        start.name.contains("Banquet Hall"),
        "expected the opening room Banquet Hall, got {:?}",
        start.name
    );

    // "ne" leaves the opening room (the narration points northeast). The mapper
    // must now see a DIFFERENT, sensibly-named room.
    assert_eq!(session.pending_input(), InputKind::Line, "expected a line prompt before the move");
    let result = session.submit("ne");
    assert!(!result.quit && result.fault.is_none(), "the \"ne\" move faulted/quit: {:?}", result.fault);

    let after = session.current_location().expect("a room must still be detected after the move");
    assert_ne!(
        after.number, start.number,
        "the room id must change across the \"ne\" move (was #{} {:?}, still #{} {:?})",
        start.number, start.name, after.number, after.name
    );
    assert!(
        after.name.contains("Scullery"),
        "expected Scullery after \"ne\", got {:?}",
        after.name
    );
}

#[test]
fn shogun_v6_location_validated_no_banner_no_ship() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };

    // Drive past the boot menu into gameplay (Enter selects START), then run a
    // few turns so the status band paints. Collect the detected room names.
    let mut names = Vec::new();
    let mut line_cmds = ["look", "look", "n", "look"].into_iter();
    for _ in 0..8 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit(line_cmds.next().unwrap_or("look")),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit && result.fault.is_none(), "Shogun faulted/quit: {:?}", result.fault);
        let _ = session.take_transcript();
        if let Some(loc) = session.current_location() {
            names.push(loc.name);
        }
    }

    assert!(
        names.iter().any(|n| n.contains("Bridge")),
        "Shogun's room Bridge must be detected during gameplay, got {names:?}"
    );
    // The ship label and the title banner must NEVER be mistaken for a room.
    assert!(
        !names.iter().any(|n| n.contains("Erasmus") || n.contains("SHOGUN")),
        "a banner/ship label leaked in as a room: {names:?}"
    );
}

/// Boot Arthur past the sword-in-the-stone intro into ordinary gameplay, where
/// the status bar (location + date) is painted. Arthur is booted with the game's
/// colours HONOURED — the app's shipped default, and the mode a real player runs.
fn boot_arthur() -> Option<GameSession> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None)
            .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap through the intro; answer 'n' to the "restore a saved position?" prompt.
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(session)
}

/// (SQ-0530) Arthur is the title that breaks "the status bar lives at the top of
/// the screen": it reserves the first twelve native rows for a graphics panel and
/// hangs its reverse-video bar at y=193, immediately above a story window opening
/// at y=209. Room detection therefore has to find the band by looking just above
/// the STORY window, not by scanning the first few rows of the screen. It must
/// also reassemble the location from single-glyph paint runs — Arthur emits the
/// whole bar one letter per run — and keep the "St Anne's Day, Compline" date on
/// the right out of the room name.
#[test]
fn arthur_v6_location_detected_below_the_graphics_panel_and_changes_on_move() {
    let Some(mut session) = boot_arthur() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };

    let start = session.current_location().expect("Arthur's opening room must be detected");
    assert!(
        start.name.to_lowercase().contains("churchyard"),
        "expected the opening room Churchyard, got {:?}",
        start.name
    );
    // The room is object-backed, so the avatar's own ancestor chain validated it —
    // this is not a name plucked off a banner.
    assert_ne!(start.number, 0, "the detected room must be a real object, got {start:?}");

    // "in" walks up the steps into the church: a DIFFERENT room.
    assert_eq!(session.pending_input(), InputKind::Line, "expected a line prompt before the move");
    let result = session.submit("in");
    assert!(!result.quit && result.fault.is_none(), "the \"in\" move faulted/quit: {:?}", result.fault);

    let after = session.current_location().expect("a room must still be detected after the move");
    assert_ne!(
        after.number, start.number,
        "the room id must change across the move (was #{} {:?}, still #{} {:?})",
        start.number, start.name, after.number, after.name
    );
    assert!(
        after.name.to_lowercase().contains("church"),
        "expected the church after \"in\", got {:?}",
        after.name
    );

    // The right-hand date field must never be mistaken for a room.
    let _ = session.submit("out");
    let back = session.current_location().expect("back in the churchyard");
    assert!(
        !back.name.contains("Anne") && !back.name.contains("Compline"),
        "the date field leaked in as a room: {:?}",
        back.name
    );
    assert_eq!(back.number, start.number, "\"out\" returns to the opening room");
}
