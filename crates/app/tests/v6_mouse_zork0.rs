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
        GameSession::new_with_trace(story_bytes, false, false, None, true, picture_dims, picts.std_window())
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
