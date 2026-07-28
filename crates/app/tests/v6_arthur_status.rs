//! Arthur (Infocom v6) top status bar — SQ-0500 / SQ-0499 / SQ-0504.
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
//! The bar reads solid across the FULL pane width (SQ-0504) — the game paints its
//! runs only from col ~4 to ~75, but a pure reverse-video row fills edge to edge.
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
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window(), None)
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

/// (SQ-0500 + SQ-0499 + SQ-0504) HYBRID: the status row renders as terminal CELLS
/// — the "St Anne's Day, Compline" date is real buffer text — and its reverse bar
/// spans the FULL pane width (no unreversed gap anywhere on the row). The graphics
/// panel above the status row stays the pixel ring (half-block image cells).
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

    // The reverse bar spans the FULL pane width (SQ-0504): the game paints its
    // status runs only from col ~4 to ~75, but the bar reads edge to edge — every
    // cell on the row is reverse-video, including the leading/trailing cells the
    // game left bare and the lone gap before the date (old SQ-0499 hole).
    let holes: Vec<u16> = (0..area.width)
        .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the reverse status bar spans the full pane [0,{}) with no unreversed gap; holes at {holes:?}\nrow: {:?}",
        area.width,
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

/// (SQ-0509) At a pane size whose letterbox scale is not 1.0 (100×30 → scale 1.2),
/// Arthur's status text — emitted as single-glyph runs at fixed 8-px pixel starts —
/// must still read "Churchyard" as ONE unbroken word: the strip merges the abutting
/// fragments before mapping, instead of rounding each independently into stray cell
/// gaps ("Chu rch yard"). The right-hand date field stays in the right portion of
/// the bar (its runs are separated from the location by a real pixel gap, so they
/// do NOT merge into the location text).
#[test]
fn arthur_hybrid_status_churchyard_is_one_word_date_right() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 100×30 → scale 1.2: at 1.0 the fixed 8-px runs already lined up; only a
    // non-unit scale exposed the per-run rounding that split the word.
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let status_y = (0..area.height).find(|&y| row_text(y).contains("Anne")).expect("status row present");
    let row = row_text(status_y);
    // The location reads as one unbroken word — no stray gap inside it.
    assert!(row.contains("Churchyard"), "location must read 'Churchyard' as one word; got: {row:?}");
    // The date field is intact and sits in the RIGHT portion of the bar (well past
    // the location on the left), never merged into the location run.
    let date_at = row.find("St Anne's Day, Compline").expect("date field intact and contiguous");
    let loc_at = row.find("Churchyard").unwrap();
    assert!(date_at > loc_at + "Churchyard".len(), "the date stays to the right of the location");
    assert!(date_at * 2 > area.width as usize, "the date sits in the right half of the {}-col bar (at {date_at})", area.width);
}

/// (SQ-0505 dynamic hybrid layout) At a TALL pane (90×40 — taller than the 8:5
/// native aspect, so there is vertical letterbox dead space), Arthur has NO bottom
/// chrome: header art + status bar on top, side borders, nothing below the story.
/// The ring is top-anchored (no vertical centering) and the story text viewport
/// extends all the way to the pane BOTTOM at its constant inset width. Where the
/// side art ends, the flanks below it are the theme backdrop — no stretched art.
#[test]
fn arthur_hybrid_tall_pane_extends_story_to_bottom() {
    use ratatui::style::Color;
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 90×40 → scale 1.40625 (width-limited), ~238 px of vertical letterbox slack.
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    // The story viewport reaches the pane bottom (dead space reclaimed).
    assert_eq!(vp.bottom(), area.bottom(), "story viewport extends to the pane bottom; got {vp:?}");
    // It keeps its side insets — the story column did NOT reflow to full width.
    assert!(vp.x > 0 && vp.right() < area.right(), "story keeps its constant inset width beside the side borders; got {vp:?}");
    // The top chrome ring (header art + status bar) is still imaged above the story.
    let is_image = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    let top_ring = (0..vp.y).flat_map(|y| (0..area.width).map(move |x| (x, y))).filter(|&(x, y)| is_image(x, y)).count();
    assert!(top_ring > 0, "the header/status ring stays imaged above the top-anchored story ({top_ring} cells)");
    // A flank cell in a LEFT-band row above the story-native bottom carries side
    // art; a flank cell deep below it is the theme backdrop (Reset), proving the
    // art is not stretched or tiled into the reclaimed space.
    let flank_art = (vp.y..vp.y + 6).any(|y| buf.cell((vp.x - 1, y)).unwrap().bg != Color::Reset);
    assert!(flank_art, "the side border art shows in the flank beside the top of the story");
    let deep = buf.cell((vp.x - 1, area.height - 2)).unwrap();
    assert_eq!(deep.bg, Color::Reset, "the flank below the side art is the theme backdrop, not stretched art: {deep:?}");
}
