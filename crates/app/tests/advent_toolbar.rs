//! advent.blb's clickable graphical toolbar — a detailed graphics window at the
//! top of the screen, whose buttons the game hit-tests itself in canvas pixels.
//!
//! - SQ-0520: the toolbar must reach the image protocol. At common pane widths it
//!   lands 2 cells tall, and the thin-strip rule heuristic (SQ-0332) used to claim
//!   it and shred it into colour-averaged ─ glyphs instead of drawing the image.
//! - SQ-0562: its noun-taking verb buttons prime the input line via Glk's
//!   pre-filled line input rather than running a command.
//! - SQ-0563: the compass rose's W/E buttons are unreachable at cell granularity.
//!
//! Boots the real gitignored story; skips cleanly when absent.

use app::engine::{Engine, WinNode};
use app::glulx_session::GlulxSession;

/// Boot the real gitignored story at the user-report geometry: a 138×51 pane at
/// 8×18 char cells → the toolbar window comes out 138×2 cells with a fully
/// painted 1104×36 canvas. `None` when the story is absent.
fn boot_advent() -> Option<GlulxSession> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/advent.blb");
    let raw = std::fs::read(&path).ok()?;
    let blorb = blorb::Blorb::parse(raw.clone()).expect("valid blorb");
    let bytes = match app::hints::extract_story(raw).expect("extract") {
        app::hints::LoadedStory::Glulx(b) => b,
        _ => panic!("expected Glulx"),
    };
    Some(
        GlulxSession::new(bytes, 138, 51, true, true, false, (8, 18), Some(blorb), &[])
            .expect("session"),
    )
}

#[test]
fn advent_toolbar_reaches_the_image_protocol() {
    let Some(mut sess) = boot_advent() else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let _ = sess.take_transcript();

    fn find_graphics(node: &WinNode) -> Option<&app::engine::GraphicsWindow> {
        match node {
            WinNode::Graphics(gw) => Some(gw),
            WinNode::Pair { first, second, .. } => {
                find_graphics(first).or_else(|| find_graphics(second))
            }
            _ => None,
        }
    }
    let model = sess.screen();
    let gw = find_graphics(&model.root).expect("advent opens a graphics toolbar window");

    // The game painted the whole toolbar: every canvas pixel opaque.
    assert!(gw.canvas.pixels().all(|p| p.0[3] != 0), "toolbar canvas fully painted");

    // The 2-cell-tall toolbar must NOT be claimed by the thin-rule cells path —
    // it falls through to the image protocol (SQ-0520).
    let area = ratatui::layout::Rect::new(0, 0, 138, 2);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    assert!(
        !app::render::graphics::render_graphics_as_cells(gw, area, &mut buf, false),
        "detailed toolbar must not be averaged into rule glyphs"
    );
}

/// SQ-0562 regression: the toolbar's noun-taking verbs (Examine, Take, Drop, Open,
/// Close, Read) don't run a command — they re-request line input with the verb
/// ALREADY in the game's buffer (Glk §4.2 `initlen`). The app must take that
/// prefill and start its input line with it; ignoring it left the prompt empty, so
/// Enter submitted a blank line and the game answered with a bare newline.
///
/// The buttons live at fixed canvas pixels (from the boot draw trace) and the press
/// is animated: the click arms a 50ms timer and the command only lands a few ticks
/// later, so drive the timer until the request appears.
#[test]
fn advent_toolbar_verb_buttons_prefill_the_input_line() {
    if boot_advent().is_none() {
        eprintln!("SKIP: no advent.blb");
        return;
    }
    // (button canvas pixel, the verb it primes) — six buttons, 32px apart. A fresh
    // boot per button so one press can't colour the next.
    for (px, want) in [
        (295u32, "Examine "),
        (327, "Take "),
        (359, "Drop "),
        (391, "Open "),
        (423, "Close "),
        (455, "Read "),
    ] {
        let mut sess = boot_advent().expect("story present");
        let _ = sess.take_transcript();
        let _ = sess.take_line_prefill();
        let windows = sess.mouse_windows();
        let (win, _, _) = *windows.first().expect("the toolbar watches for clicks");
        sess.deliver_mouse(win, px, 8);
        let mut got = None;
        for _ in 0..8 {
            sess.deliver_timer();
            if let Some(p) = sess.take_line_prefill() {
                got = Some(p);
                break;
            }
        }
        assert_eq!(got.as_deref(), Some(want), "button at canvas x={px} primes {want:?}");
    }
}

/// The compass rose's W and E buttons sit in a canvas band that cell-granular
/// clicks cannot reach: the toolbar is 2 cells of 18px, so a cell-centre click
/// only ever reports canvas y 9 or 27, while W/E occupy y 12..24. Pinned as the
/// measured geometry behind the report — the buttons are unreachable by
/// construction, not by a mapping mistake. (SQ-0563)
#[test]
fn advent_compass_w_e_fall_between_the_two_cell_rows() {
    let Some(mut sess) = boot_advent() else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let _ = sess.take_transcript();
    let windows = sess.mouse_windows();
    let (_, _, rect) = *windows.first().expect("the toolbar watches for clicks");
    let char_px = sess.char_pixels();
    // Every canvas y a click can name, for a window this tall.
    let reachable: Vec<u32> =
        (0..rect.height).map(|r| r * char_px.1 + char_px.1 / 2).collect();
    assert_eq!(reachable, vec![9, 27], "two cell rows → two addressable canvas rows");
    // The middle compass row (drawn at canvas y=12, ~12px tall) contains neither.
    let w_e_band = 12..24;
    assert!(
        !reachable.iter().any(|y| w_e_band.contains(y)),
        "W/E band {w_e_band:?} is unreachable from {reachable:?}"
    );
}
