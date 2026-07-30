//! Zork0 v6 mouse-input acceptance (SQ-0186, Lane M of
//! `docs/superpowers/plans/2026-07-22-v6-completion.md`).
//!
//! Boots `stories/zork0-r393-s890714.z6` headlessly (skip-if-missing), reaches
//! the first input wait, and exercises the click→game-pixel→VM path end to end:
//!
//!  1. Build a [`V6ClickMap`] with a plausible letterbox geometry and call
//!     `map_click` directly on a cell over the banner compass's NE spoke. Zork0's
//!     compass occupies native game pixels x∈139..184, y∈1..40; the mapped pixel
//!     must land in that region.
//!  2. Feed the mapped pixel to `GameSession::set_mouse` (the `Engine` hook) and
//!     read the header extension table back out of `machine.mem` to confirm the
//!     click coordinates were written (word 1 = X, word 2 = Y, ZMSD §11).
//!  3. Submit the single-click ZSCII (254, ZMSD §3.8) and assert the VM does not
//!     fault or quit. (Full COMPASS-CLICK movement acceptance may hit the game's
//!     function-key screen — known SQ-0452 — so this asserts no-fault, not a
//!     room change, per the plan.)

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::render::graphics::V6ClickMap;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork Zero to its first input wait, with pictures wired up. `None` when the
/// gitignored story is absent.
fn boot_zork0() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut session = GameSession::new_with_trace(
        story_bytes, false, false, None, true, picture_dims, std_window, None,
    )
    .expect("Zork0 (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

// Compass banner region in NATIVE game pixels (from the Rect-derived overlay:
// the 8 direction spokes occupy x 139..184, y 1..40).
const COMPASS_X: std::ops::RangeInclusive<u16> = 139..=184;
const COMPASS_Y: std::ops::RangeInclusive<u16> = 1..=40;

#[test]
fn zork0_v6_compass_click_maps_writes_header_and_does_not_fault() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, true, picture_dims, picts.std_window(), None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // Reach the first input wait (Zork0 boots to a line prompt).
    assert!(
        matches!(
            session.pending_input(),
            app::session::InputKind::Line | app::session::InputKind::Char
        ),
        "expected a line/char input wait after boot"
    );

    // (1) A plausible letterbox: native 320×200 drawn 1:1 (scale 1.0, no offset)
    // into a pane at the terminal origin with 8×16-pixel cells. The click map is
    // engine-neutral geometry, so this needs no live renderer/picker. Cell
    // (21, 0) sits over the compass's NE spoke.
    let click_map = V6ClickMap {
        pane_x: 0,
        pane_y: 0,
        cell_w: 8,
        cell_h: 16,
        img_x: 0.0,
        img_y: 0.0,
        img_w: 320.0,
        img_h: 200.0,
        text_rows: None,
        native_w: 320,
        native_h: 200,
    };
    let (gx, gy) = click_map
        .map_click(21, 0)
        .expect("a cell over the compass must map into the game image");
    assert!(
        COMPASS_X.contains(&gx) && COMPASS_Y.contains(&gy),
        "mapped click ({gx}, {gy}) is not in the compass NE region x{COMPASS_X:?} y{COMPASS_Y:?}"
    );

    // A click outside the drawn image (letterbox / off-pane) must map to None.
    assert!(
        click_map.map_click(60, 0).is_none(),
        "a click beyond the image's right edge must not map to a game pixel"
    );

    // (2) Report the click to the VM and read the header extension table back.
    // ZMSD §11: word 0 = count, word 1 = mouse X, word 2 = mouse Y.
    let ext = session.machine.mem.read_word(0x36) as u32;
    assert_ne!(ext, 0, "Zork0 must have a header extension table for mouse coords");
    let count = session.machine.mem.read_word(ext);
    assert!(count >= 2, "Zork0's extension table must hold the mouse X/Y words (count={count})");

    session.set_mouse(gy, gx); // Engine hook: (y_px, x_px)
    assert_eq!(session.machine.mem.read_word(ext + 2), gx, "ext word 1 = mouse X");
    assert_eq!(session.machine.mem.read_word(ext + 4), gy, "ext word 2 = mouse Y");

    // (3) Deliver the single-click ZSCII; the VM must stay alive (no fault/quit).
    let result = session.submit_char(254);
    assert!(!result.quit, "Zork0 quit on the mouse click");
    assert!(
        result.fault.is_none(),
        "Zork0 faulted on the mouse click: {:?}",
        result.fault
    );
}

/// SQ-0566: a compass click during ORDINARY PLAY. Zork Zero spends the game at a
/// line prompt, and its terminating-characters table is `[255]` — the "any function
/// key" wildcard, which covers the single-click code 254 (ZMSD §10.7 / §3.8). So a
/// click must end that read with whatever is typed plus the terminator, whereupon
/// the game reads the coordinates and acts. Delivering clicks only during
/// `read_char` meant the border compass did nothing except while a menu was up.
///
/// The rose was mapped by clicking the real story pixel by pixel: it spans native
/// x 282..372, y 6..80 of the 640×400 screen, laid out as pie slices around the
/// centre. The cells below were chosen so a cell CENTRE lands deep inside its
/// slice, not near a boundary.
#[test]
fn zork0_compass_click_ends_the_line_read_with_the_clicked_direction() {
    let Some(probe) = boot_zork0() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    // The gate: this story wants a mouse AND accepts a click as a line terminator.
    assert!(probe.wants_mouse(), "Zork0 sets Flags2 bit 5");
    assert_eq!(
        probe.mouse_click_terminator(),
        Some(254),
        "the [255] wildcard covers the single-click code, so a click ends a line read"
    );
    assert!(
        matches!(probe.pending_input(), app::session::InputKind::Line),
        "Zork0 boots to a LINE prompt — the case that used to ignore clicks"
    );

    // The screen is 640×400 drawn 1:1 at the pane origin with 8×16-pixel cells.
    let click_map = V6ClickMap {
        pane_x: 0,
        pane_y: 0,
        cell_w: 8,
        cell_h: 16,
        img_x: 0.0,
        img_y: 0.0,
        img_w: 640.0,
        img_h: 400.0,
        text_rows: None,
        native_w: 640,
        native_h: 400,
    };

    // (terminal cell, the direction that slice of the rose means)
    for (col, row, want) in [(39u16, 0u16, "north"), (44, 2, "east"), (39, 4, "south"), (36, 2, "west")] {
        let mut session = boot_zork0().expect("story present");
        let _ = session.take_transcript();
        let (gx, gy) = click_map
            .map_click(col, row)
            .expect("a cell over the compass maps into the game image");
        assert!(
            (282..=372).contains(&gx) && (6..=80).contains(&gy),
            "cell ({col},{row}) → ({gx},{gy}) is outside the compass rose"
        );

        // Exactly what the run loop does for a click at a line prompt: record the
        // coordinates, then end the read with the typed text (nothing here) and the
        // click as terminator.
        session.set_mouse(gy, gx);
        let result = session.submit_line_with_terminator("", 254);
        assert!(result.fault.is_none(), "faulted on a compass click: {:?}", result.fault);
        assert!(!result.quit, "quit on a compass click");
        let echoed = result.transcript.trim().lines().next().unwrap_or("").trim().to_string();
        assert_eq!(echoed, want, "cell ({col},{row}) → ({gx},{gy}) must mean {want:?}");
    }
}

/// The gate has to EXCLUDE stories that don't accept a click at a line prompt, or a
/// stray click would submit a partial command. Journey wants a mouse but lists no
/// terminating characters at all — its menus are driven by `read_char` — so a click
/// there stays with the app.
#[test]
fn journey_does_not_accept_a_click_as_a_line_terminator() {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let session = GameSession::new(story_bytes, false, false, None).expect("Journey should boot");
    assert!(session.wants_mouse(), "Journey does set Flags2 bit 5");
    assert_eq!(
        session.mouse_click_terminator(),
        None,
        "but with an empty terminating-characters table a click must not end its line read"
    );
}

/// SQ-0576: a compass click must reach the MAPPER as the direction travelled.
///
/// A click types nothing — it terminates the line read with ZSCII 254 — so the
/// mapper used to see an empty command and mint no directional edge. The game
/// itself echoes the command it synthesized ("north", alone on the first output
/// line); `echoed_direction_command` adopts that echo, and `apply_turn` then
/// maps the move exactly like a typed "north".
///
/// End to end on the real game: drive to the Great Hall, click the compass's
/// north spoke (native 640x400: the rose occupies x 278..368, y 2..80), and
/// assert the echo parses and the mapper records Great Hall --N--> Entrance
/// Hall.
#[test]
fn zork0_compass_click_maps_a_directional_edge() {
    use app::session::{apply_turn, echoed_direction_command, InputKind};

    let Some(mut session) = boot_zork0() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    // Drive to the Great Hall (same prelude as the hybrid suite).
    let _ = session.take_transcript();
    let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
    for _ in 0..16 {
        let _ = match session.pending_input() {
            InputKind::Line => session.submit(lines.next().unwrap_or("wait")),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
    }
    let mut mapper = mapper::mapper::Mapper::default();
    let mut seeded = false;
    for _ in 0..6 {
        let r = match session.pending_input() {
            InputKind::Char => session.submit_char(13),
            _ => session.submit("look"),
        };
        if r.transcript.contains("Great Hall") {
            apply_turn(&mut mapper, "look", &r);
            seeded = true;
            break;
        }
    }
    assert!(seeded, "reached the Great Hall");
    let here = mapper.graph.current().expect("Great Hall observed");

    // Click the compass's north spoke, exactly as main.rs delivers it: the
    // click terminates the pending LINE read (ZSCII 254) with nothing typed.
    let term = session.mouse_click_terminator().expect("Zork0 accepts click terminators");
    session.set_mouse(12, 322); // engine stores (y, x); N spoke at ~(322, 12)
    let result = session.submit_line_with_terminator("", term);

    let echoed = echoed_direction_command(&result.transcript);
    assert_eq!(echoed, Some("north"), "the game echoes the synthesized command: {:?}", result.transcript);
    apply_turn(&mut mapper, echoed.unwrap(), &result);

    let there = mapper.graph.current().expect("moved somewhere");
    assert_ne!(here, there, "the click moved the player");
    let conns = mapper.graph.connections();
    assert_eq!(conns.len(), 1, "the click-driven move minted exactly one edge");
    assert_eq!(
        (conns[0].origin, conns[0].dir, conns[0].dest),
        (here, mapper::direction::Direction::N, there),
        "Great Hall --N--> Entrance Hall"
    );
}
