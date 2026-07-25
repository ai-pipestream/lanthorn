//! Arthur (Infocom v6) top status bar — SQ-0500 / SQ-0499.
//!
//! Arthur paints a reverse-video status bar (location + "St Anne's Day, Compline"
//! date) as pixel-positioned runs at native row 12, ABOVE the story buffer (which
//! starts at native row 13) and BELOW a graphics panel (native rows 0–11). The
//! panel's frame is a full-screen graphics window, but its INTERIOR — where the
//! status text sits — is transparent, so the status row is a pure-TEXT strip
//! sandwiched between art strips.
//!
//! HYBRID mode (SQ-0500): the status row decomposes into its own terminal-CELL
//! strip (crisp reverse bar) while the graphics panel above stays the pixel ring.
//! The bar reads SOLID across its painted extent — no lone unreversed cell where
//! the game left a gap between its runs (SQ-0499 cell path).
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Arthur past the sword-in-the-stone intro to ordinary gameplay, where the
/// top status bar (location + date) is painted and the story buffer is streaming.
fn arthur_at_status() -> Option<GameSession> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
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

/// The status runs reach the model as reverse-video grid runs at native row 12,
/// carrying the "St Anne's Day" date text — a sanity gate for the harness.
#[test]
fn arthur_status_reaches_the_model() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let mut date = false;
    let mut reversed = false;
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if (t.y.max(1) - 1) / 16 == 12 {
                    reversed |= t.style & 1 != 0;
                    if t.text.contains('A') || t.text.contains('n') {
                        date = true;
                    }
                }
            }
        }
    }
    assert!(date, "status runs present at native row 12");
    assert!(reversed, "status bar is reverse-video");
}

/// (SQ-0500 + SQ-0499) HYBRID: the status row renders as terminal CELLS — the
/// "St Anne's Day, Compline" date is real buffer text — and its reverse bar is
/// SOLID across the painted extent (no lone unreversed cell). The graphics panel
/// above the status row stays the pixel ring (half-block image cells).
#[test]
fn arthur_hybrid_status_row_is_solid_terminal_bar() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // Locate the terminal row carrying the status date as real cell text.
    let status_y = (0..area.height)
        .find(|&y| row_text(y).contains("Anne"))
        .expect("status date renders as terminal cells");

    // The reverse bar is solid: from the first to the last reversed cell on that
    // row, EVERY cell is reversed — the old lone unreversed gap before the date is
    // gone (SQ-0499 cell path).
    let reversed: Vec<u16> = (0..area.width)
        .filter(|&x| buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(!reversed.is_empty(), "status row {status_y} is a reverse bar");
    let (first, last) = (reversed[0], *reversed.last().unwrap());
    let holes: Vec<u16> = (first..=last)
        .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the reverse status bar is solid across [{first},{last}] with no unreversed gap; holes at {holes:?}\nrow: {:?}",
        row_text(status_y)
    );

    // The graphics panel above the status row stays the pixel ring.
    let is_image_cell = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != ratatui::style::Color::Reset
    };
    let panel_ink = (0..status_y.saturating_sub(1))
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| is_image_cell(x, y))
        .count();
    assert!(panel_ink > 0, "the graphics panel above the status stays imaged in the ring (got {panel_ink} image cells)");
}
