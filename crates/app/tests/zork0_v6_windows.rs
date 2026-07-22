//! Zork0 v6 window-geometry headless smoke — the Phase 1a acceptance gate
//! (SQ-0186, `docs/superpowers/plans/2026-07-21-v6-phase1a-engine.md` Task 10).
//!
//! Drives the real `stories/zork0-r393-s890714.z6` (v6/graphical) headlessly
//! through boot and a couple of turns, and asserts that Tasks 1–9's v6 window
//! model + graphics boundary produce sane, non-underflowed geometry and real
//! injected picture dimensions/draw events — everything Plan 1b needs to
//! render Zork0.
//!
//! Zork0's release layout is a bare `.z6` executable beside a resources-only
//! sidecar Blorb (`Zork0.blb`, no `Exec` chunk of its own) — discovered during
//! Task 9. The story loaded here is the bare `.z6`; `blorb::resolve_resource_blorb`
//! finds `Zork0.blb` as the picture sidecar via its dir-scan stem-prefix match,
//! exactly as `startup.rs`'s `ZCode` arm does.
//!
//! The story asset is gitignored (large, local-only), so this test **skips
//! cleanly** when absent — CI and fresh clones stay green, dev worktrees get
//! the real coverage. Mirrors the skip-if-absent pattern in
//! `crates/gvm/tests/kerkerkruip_boots.rs` / `crates/app/tests/wizard_sniffer.rs`.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::GameSession;

/// The repo-root `stories/` directory (a gitignored symlink in dev worktrees,
/// absent in CI), resolved relative to this crate's manifest.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The Phase-0 layout-underflow signature: a `u16` that wrapped negative
/// (e.g. `0 - 16` in pixel-rect math) lands in the top of the `u16` range.
/// Any v6 window whose `x_size`/`y_size` falls in here is a bug, not geometry.
const UNDERFLOW_RANGE: std::ops::RangeInclusive<u16> = 0xFFF0..=0xFFFF;

#[test]
fn zork0_v6_windows_smoke() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    // Build the v6 Pict dimension table the same way `startup.rs`'s ZCode arm
    // does: resolve the story's resource Blorb (Zork0's actual release layout
    // is a dir-scan stem-prefix match onto `Zork0.blb`, a resources-only Blorb
    // with no `Exec` of its own), header-sniff every Pict's size, and inject
    // the table BEFORE constructing the session — `picture_data` is called
    // during boot, which happens inside `GameSession::new_with_trace` itself
    // (Phase 0 lesson).
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    eprintln!("resolved {} Pict dimension entries from the sidecar", picture_dims.len());

    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims)
            .expect("Zork0 (v6) should load and boot without a ZError");

    // (a) No fault/panic during boot.
    assert!(!session.quit, "Zork0 quit during boot (fault or premature quit), before reaching the first prompt");
    assert!(
        session.machine.fault_trace.is_none(),
        "Zork0 faulted during boot: {:?}",
        session.machine.fault_trace.as_ref().map(|t| t.to_lines())
    );

    // (b) The sidecar dims were injected into the machine.
    assert!(
        !session.machine.picture_dims.is_empty(),
        "machine.picture_dims should be non-empty after Task 9's sidecar injection"
    );

    let v6 = session
        .machine
        .screen
        .v6
        .as_ref()
        .expect("a v6 story must populate ScreenState.v6 (Task 2)");
    eprintln!("--- v6 window table at first prompt (current={}) ---", v6.current);
    for (i, w) in v6.windows.iter().enumerate() {
        eprintln!(
            "  window {i}: y={} x={} y_size={} x_size={} attrs={:#06x}",
            w.y_coord, w.x_coord, w.y_size, w.x_size, w.attributes
        );
    }
    eprintln!("pending_pictures at first prompt: {:?}", session.machine.pending_pictures);

    // (c) At least one window has nonzero, non-underflowed size — locked to the
    // real geometry observed from a `--nocapture` run: window 0 (the status/
    // banner grid, 630x184px) and window 7 (Zork0's graphics window, 640x192px)
    // are both real, sane rects, nowhere near the Phase-0 underflow range.
    for (i, w) in v6.windows.iter().enumerate() {
        assert!(
            !UNDERFLOW_RANGE.contains(&w.x_size) && !UNDERFLOW_RANGE.contains(&w.y_size),
            "window {i} shows the Phase-0 underflow signature: x_size={} y_size={}",
            w.x_size,
            w.y_size
        );
    }
    assert_eq!((v6.windows[0].x_size, v6.windows[0].y_size), (630, 184), "window 0 geometry");
    assert_eq!((v6.windows[7].x_size, v6.windows[7].y_size), (640, 192), "window 7 (graphics) geometry");

    // Drive a couple of turns past the first prompt.
    let _ = session.take_transcript();
    for cmd in ["look", "north"] {
        let result = session.submit(cmd);
        eprintln!(
            "turn {cmd:?}: quit={} fault={:?} transcript_chars={}",
            result.quit,
            result.fault,
            result.transcript.chars().count()
        );
        assert!(!result.quit, "Zork0 quit on command {cmd:?}");
        assert!(result.fault.is_none(), "Zork0 faulted on command {cmd:?}: {:?}", result.fault);
    }
    eprintln!("pending_pictures after two turns: {:?}", session.machine.pending_pictures);

    // (d) draw_picture accumulated at least one draw event, ideally targeting
    // window 7 (the v6 convention for the banner/graphics window Zork0 draws
    // its opening image into).
    assert!(
        !session.machine.pending_pictures.is_empty(),
        "machine.pending_pictures should have accumulated at least one draw_picture event"
    );
    let saw_window_7 = session.machine.pending_pictures.iter().any(|e| e.window == 7);
    assert!(
        saw_window_7,
        "expected at least one draw_picture event targeting window 7 (Zork0's graphics window); got {:?}",
        session.machine.pending_pictures
    );
}
