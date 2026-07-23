//! Zork0 v6 gameplay smoke (SQ-0186, Lane S of
//! `docs/superpowers/plans/2026-07-22-v6-completion.md`).
//!
//! Drives `stories/zork0-r393-s890714.z6` headlessly past boot and asserts:
//! no fault; the banner **compass overlay** — Zork0 draws 8 direction-
//! indicator tiles (picture numbers in `9..=24`) into window 1 at the
//! Rect-derived banner centre `x=139, y=1` — fires correctly for the initial
//! room; a single safe movement command doesn't fault and leaves window 0's
//! box unchanged. Mirrors the skip-if-missing / Pict-source setup pattern in
//! `zork0_v6_windows.rs`.
//!
//! IMPORTANT FINDING (see the doc comment on the second test below): a second
//! command submitted after a room-changing move (any direction) deterministically
//! faults, so this file does **not** drive a second turn after a move — doing so
//! would make the test fail. The fault is reported, not asserted as a pass/fail
//! condition, per the "don't fix, don't fail" rule for smoke tests that expose bugs.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Compass overlay tile picture numbers (banner direction indicators).
const COMPASS_PICS: std::ops::RangeInclusive<u16> = 9..=24;

/// A `@draw_picture(...)` trace line is a compass-overlay redraw iff its
/// picture number falls in `COMPASS_PICS` AND it targets window 1 at the
/// banner centre (y=1, x=139).
fn is_compass_draw(line: &str) -> bool {
    if !line.starts_with("@draw_picture(") {
        return false;
    }
    if !(line.contains("window=1") && line.contains("x=139") && line.contains("y=1")) {
        return false;
    }
    let Some(rest) = line.strip_prefix("@draw_picture(number=") else { return false };
    let Some(comma) = rest.find(',') else { return false };
    rest[..comma].parse::<u16>().map(|n| COMPASS_PICS.contains(&n)).unwrap_or(false)
}

#[test]
fn zork0_v6_gameplay_smoke_boot_compass_and_one_safe_move() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();

    // trace_from_boot = true: capture screen-op tracing from the very first
    // boot instruction (via `take_screen_trace`, `Engine` trait) so the
    // boot-time compass draws are visible.
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, true, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");

    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot: {:?}", session.machine.fault_trace);

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // Snapshot window 0's box (the main text/status window) before any moves,
    // to assert the layered model stays sane (no drift/underflow) across the
    // one move driven below.
    let window0_before = {
        let v6 = session.machine.screen.v6.as_ref().expect("v6 story must populate ScreenState.v6");
        let w = &v6.windows[0];
        (w.x_coord, w.y_coord, w.x_size, w.y_size)
    };

    // (a) Compass overlay fires correctly for the initial room: Zork0 draws 8
    // direction-indicator tiles into window 1 at the banner centre during boot
    // (the game's first "room entry" event). This is the real, verified
    // compass-overlay behavior; see the module doc comment for why a *second*,
    // movement-triggered re-fire could not be safely exercised here.
    let boot_trace = session.take_screen_trace();
    let boot_compass_draws: Vec<&String> = boot_trace.iter().filter(|l| is_compass_draw(l)).collect();
    eprintln!("boot screen trace: {} lines total", boot_trace.len());
    for l in &boot_compass_draws {
        eprintln!("  boot compass draw: {l}");
    }
    assert_eq!(
        boot_compass_draws.len(),
        8,
        "expected 8 compass-overlay draw_picture events (number in 9..=24, window=1, y=1, x=139) at boot, got: {boot_trace:?}"
    );
    for l in &boot_compass_draws {
        // Every compass tile's picture number must be in the direction-tile range.
        assert!(is_compass_draw(l), "not a compass draw: {l}");
    }

    // Clear the boot transcript so the move's own output is isolated.
    let _ = session.take_transcript();

    // (b) Drive exactly one safe move ("ne" — Zork Zero's opening narration
    // literally points the player northeast: "An insistent finger points
    // northeast."). Assert no fault, no quit.
    assert_eq!(
        session.pending_input(),
        app::session::InputKind::Line,
        "expected a line-input prompt before submitting the move"
    );
    let result = session.submit("ne");
    assert!(!result.quit, "Zork0 quit on the \"ne\" move");
    assert!(result.fault.is_none(), "Zork0 faulted on the \"ne\" move: {:?}", result.fault);

    // Document (not assert) the screen ops this move actually emits: in the
    // observed trace it's `@split_window(...)` (a v6 legacy no-op, per the
    // comment in `exec.rs`'s 0x0A arm) and `@erase_line(0)` — no draw_picture,
    // i.e. no compass redraw is emitted within the SAME turn as the move
    // itself. See the module doc comment for what happens on the *next* turn.
    let move_trace = session.take_screen_trace();
    eprintln!("\"ne\" move screen trace ({} lines): {move_trace:?}", move_trace.len());

    // (c) The layered v6 window model stays sane after the move: window 0's
    // box is unchanged (only banner/graphics content would change, not the
    // text window's own geometry).
    let v6 = session.machine.screen.v6.as_ref().expect("v6 story must still populate ScreenState.v6");
    let w = &v6.windows[0];
    assert_eq!(
        (w.x_coord, w.y_coord, w.x_size, w.y_size),
        window0_before,
        "window 0 box drifted after the \"ne\" move"
    );

    let final_screen = session.screen();
    assert!(
        matches!(final_screen.root, app::engine::WinNode::Layered(_)),
        "v6 story's screen() root must stay WinNode::Layered after the move, got {:?}",
        final_screen.root
    );
}

/// Documents a real, reproducible bug found while building the gameplay smoke
/// above: submitting a **second** command after a room-changing move (any
/// direction — "ne", "n", "s", "w" were all tried) deterministically faults
/// the VM, regardless of what that second command's text is (tried: "n", "s",
/// "look", "north" — all fault identically). The fault:
///
/// ```text
/// memory fault: read8 @0x000a9438
/// PC=0x01296e  op=op:Two/0x0a   (2OP:10 = test_attr)
/// ```
///
/// i.e. a `test_attr` on a garbage/out-of-range object number, most likely a
/// per-turn daemon (e.g. the "insistent finger" servant-escort NPC introduced
/// in the opening narration) reading a bad object after the move sets up its
/// state. By contrast, if the FIRST command is a non-committal "look" (no room
/// change), a second command does NOT fault — it instead surfaces Infocom's
/// "Software Function Key definition" screen text (an interpreter-level
/// menu overlay baked into the story, unrelated to normal parser play).
///
/// This test asserts the fault happens exactly as observed (a regression pin
/// for a KNOWN bug, not a requirement) so a future fix is visible as a test
/// change here rather than silently drifting. Per Lane S's rules, this is
/// reported, not fixed, in this lane.
#[test]
fn zork0_v6_gameplay_second_turn_after_move_faults_known_bug() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let first = session.submit("ne");
    assert!(!first.quit && first.fault.is_none(), "the first move itself should not fault");

    let second = session.submit("look");
    eprintln!("known-bug fault trace: {:?}", second.fault);
    assert!(
        second.quit,
        "expected the known bug to force a quit on the second turn after a move \
         (if this now passes, the bug may be fixed — replace this pin with a real assertion)"
    );
    assert!(
        second.fault.as_ref().is_some_and(|f| f.iter().any(|l| l.contains("test_attr") || l.contains("op:Two/0x0a"))),
        "expected the known test_attr memory-fault signature, got: {:?}",
        second.fault
    );
}
