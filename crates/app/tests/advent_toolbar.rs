//! SQ-0520 regression: advent.blb's clickable graphical toolbar (a detailed
//! graphics window at the top of the screen) must reach the image protocol —
//! at common pane widths it lands 2 cells tall, and the thin-strip rule
//! heuristic (SQ-0332) used to claim it and shred it into colour-averaged
//! ─ glyphs ("two thin strips of pixels") instead of drawing the image.
//!
//! Boots the real gitignored story; skips cleanly when absent.

use app::engine::{Engine, WinNode};
use app::glulx_session::GlulxSession;

#[test]
fn advent_toolbar_reaches_the_image_protocol() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories/advent.blb");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: no advent.blb");
        return;
    };
    let blorb = blorb::Blorb::parse(raw.clone()).expect("valid blorb");
    let bytes = match app::hints::extract_story(raw).expect("extract") {
        app::hints::LoadedStory::Glulx(b) => b,
        _ => panic!("expected Glulx"),
    };
    // The user-report geometry: 138×51 pane at 8×18 char cells → the toolbar
    // window comes out 138×2 cells with a fully-painted 1104×36 canvas.
    let mut sess = GlulxSession::new(bytes, 138, 51, true, true, false, (8, 18), Some(blorb), &[])
        .expect("session");
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
