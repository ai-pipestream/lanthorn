//! SQ-1826: v6 chrome text and its fills stay BETWEEN the side art and clip on the right.
//!
//! The owner's rule (2026-10-24): "anything that doesn't fit on the line (or between
//! side-art) should be truncated on the right side." Text never moves over side art and
//! never reflows. On Shogun r322's title card the embedding host (39x30 cells of 27x60 px,
//! scale 1.6453; and 42x35 of 20x46) reported the opposite: the story box is x=3 w=33 with
//! ornament columns 0..3 and 36..39, yet "Copyright (c) 1988 by Infocom" ran to col 39,
//! the trademark line to cols 1..39, rows 6..8 were slid to col 0 (SQ-1820), and the ground
//! fills (0,3,39,1) / (0,5,39,5) wiped the ornament on those rows.
//!
//! Cause: `ChromeRowOracle::blocked` judged a row by where its glyphs would be drawn
//! UNCLIPPED, so a line longer than the box "reached" the flank columns, the flank yielded
//! the row, and the text strip took the full pane width. The oracle now asks where the text
//! stands once clipped at the art's inner edge, so the flank keeps the row, the strip is the
//! art-free span, and the plan (SQ-1814), rules (SQ-1818), slide (SQ-1820/1823) and fills
//! all use that span as their bounds.
//!
//! The same cause made Shogun's in-game score bar flip between spanning the art and sitting
//! between it: "Erasmus:" is laid 3 native px past the ornament, which the scale ROUNDS into
//! the ornament at some widths (not at others), and the flank yielded the row exactly when it
//! did. A word the scale lays left of the strip is now drawn at the strip's edge, so the bar
//! is between the art at every width.
//!
//! Specimen: `shogun-r322-s890706.z6` (release 322, serial 890706), IBM PC press. Title card:
//! ONE Enter past the splash picture. Gameplay: Space then Enter (the first gameplay frame).
//! Hybrid, `honor_game_colours` on.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::screen::{hybrid_chrome_layout, render_story_pane, V6HybridChromeLayout};
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const HOST_SIZES: [(u16, u16, u16, u16); 4] = [(39, 30, 27, 60), (42, 35, 20, 46), (71, 16, 27, 60), (88, 36, 10, 23)];

fn boot_shogun() -> Option<GameSession> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/shogun-r322-s890706.z6");
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let profile = InterpreterProfile::IbmPc;
    let screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        screen_px,
        profile.default_colours(),
        None,
    )
    .ok()?;
    s.machine.set_palette(profile.palette());
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// The title card: one Enter past the splash.
fn title_card() -> Option<GameSession> {
    let mut s = boot_shogun()?;
    let _ = match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    Some(s)
}

/// The first gameplay frame: Space (title card and boot menu), then Enter (START).
fn gameplay() -> Option<GameSession> {
    let mut s = boot_shogun()?;
    let _ = s.submit_char(b' ');
    let r = s.submit_char(13);
    assert!(r.transcript.contains("Bridge"), "premise: the first gameplay frame");
    Some(s)
}

fn state(picker: ratatui_image::picker::Picker) -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.set_game_picker(Some(picker));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = true;
    st
}

fn host(s: &GameSession, (cols, rows, cw, ch): (u16, u16, u16, u16)) -> V6HybridChromeLayout {
    let st = state(app::render::graphics::kitty_picker(cw, ch));
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    hybrid_chrome_layout(&layout, native, Rect::new(0, 0, cols, rows), (cw, ch), &st).expect("a hybrid chrome layout")
}

fn tui(s: &GameSession, cols: u16, rows: u16) -> Buffer {
    let st = state(ratatui_image::picker::Picker::halfblocks());
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
    buf
}

/// The art-free span the terminal ring uses at this pane: the host layout at the picker's own
/// font size (the one source the terminal draw and the host share).
fn tui_span(s: &GameSession, cols: u16, rows: u16) -> Rect {
    let fs = ratatui_image::picker::Picker::halfblocks().font_size();
    host(s, (cols, rows, fs.width, fs.height)).viewport
}

fn glyph(buf: &Buffer, x: u16, y: u16) -> char {
    buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')
}

/// The chrome text runs above the viewport (the card, or the bar).
fn chrome_text(ring: &V6HybridChromeLayout) -> Vec<&app::render::screen::V6HybridChromeRun> {
    ring.runs.iter().filter(|r| r.row < i32::from(ring.viewport.y) && !r.run.text.trim().is_empty()).collect()
}

/// Every chrome run above the viewport lies within `[vp.x, vp.right())` once resolved, and no
/// ground fill above the viewport leaves it.
fn assert_between_the_art(ring: &V6HybridChromeLayout, who: &str) {
    let vp = ring.viewport;
    let (lo, hi) = (i32::from(vp.x), i32::from(vp.right()));
    for r in chrome_text(ring) {
        let (a, b) = r.resolved.unwrap_or_else(|| panic!("{who}: {:?} is dropped", r.run.text));
        assert!(a >= lo && b <= hi, "{who}: {:?} is drawn at {a}..{b}, outside the art-free span {lo}..{hi}", r.run.text);
    }
    for g in ring.ground.iter().filter(|g| g.rect.y < vp.y) {
        let r = g.rect;
        assert!(i32::from(r.x) >= lo && i32::from(r.right()) <= hi, "{who}: ground fill {r:?} covers side-art columns (span {lo}..{hi})");
    }
}

/// The title card at the host's portrait and landscape sizes: every run and every fill is
/// inside the art-free span, the lines that overrun are clipped on the right, and the card
/// keeps its nine lines.
///
/// FALSIFY by making `ChromeRowOracle::run_cols` return the unclipped `(c, c + n)`: the fills
/// at 39x30 are (0,3,39,1) and (0,5,39,5) again.
#[test]
fn title_card_stays_between_the_side_art_on_the_host_path() {
    let Some(s) = title_card() else { return };
    for size in HOST_SIZES {
        let ring = host(&s, size);
        let who = format!("{}x{}@{}x{}", size.0, size.1, size.2, size.3);
        assert_between_the_art(&ring, &who);
        assert_eq!(chrome_text(&ring).len(), 9, "{who}: the card's nine lines");
        let lit = ring.runs.iter().find(|r| r.run.text.starts_with("Original Literary")).expect("literary line");
        assert_eq!(lit.run.text.chars().count(), 54, "{who}: the run keeps its whole text (clipped, never reflowed)");
        let (a, b) = lit.resolved.unwrap();
        assert!(b - a <= i32::from(ring.viewport.width), "{who}: the long line is clipped to the span ({a}..{b})");
    }
}

/// The reported 39x30 / 42x35 numbers, pinned: the box is the art-free span, every fill is
/// exactly that wide, and the card's long lines start at its left edge.
#[test]
fn reported_portrait_sizes_are_pinned() {
    let Some(s) = title_card() else { return };
    for (size, vp_x, vp_w) in [(HOST_SIZES[0], 3u16, 33u16), (HOST_SIZES[1], 4, 34)] {
        let ring = host(&s, size);
        assert_eq!((ring.viewport.x, ring.viewport.width), (vp_x, vp_w), "{size:?}: the box");
        let fills: Vec<_> = ring.ground.iter().filter(|g| g.rect.y < ring.viewport.y).collect();
        assert!(!fills.is_empty(), "{size:?}: non-vacuity: the card has a ground fill");
        for fill in fills {
            assert_eq!((fill.rect.x, fill.rect.width), (vp_x, vp_w), "{size:?}: a fill leaves the box: {:?}", fill.rect);
        }
        for text in ["Original Literary", "Licensed by", "Release 322"] {
            let r = ring.runs.iter().find(|r| r.run.text.starts_with(text)).expect(text);
            assert_eq!(r.resolved.map(|(a, _)| a), Some(i32::from(vp_x)), "{size:?}: {text} at the box's left edge");
        }
    }
}

/// The terminal ring, across the pane widths a narrow terminal uses: the cells left and
/// right of the art-free span on every card row hold the ornament (a drawn, non-blank,
/// non-letter cell). This is also the landscape question: at 71x16 the old full-width fill
/// (7,6,57,4) wiped the ornament on rows 6..9 and made the gap in the side art; the game
/// paints art there (the flank draws it from the native frame), so the gap was our fill.
///
/// FALSIFY as above: the ornament cells on the card rows become card text or blanks.
#[test]
fn title_card_rows_show_art_beside_the_text_in_the_terminal() {
    let Some(s) = title_card() else { return };
    let mut checked = 0;
    for cols in (40u16..=100).step_by(4) {
        let span = tui_span(&s, cols, 25);
        let buf = tui(&s, cols, 25);
        for y in 0..10u16 {
            for x in (0..span.x).chain(span.right()..cols) {
                let c = glyph(&buf, x, y);
                assert!(!c.is_ascii_alphanumeric(), "{cols} cols: card text {c:?} over the art at ({x},{y})");
            }
            // The ornament's outer margin is bare in the game's own frame, so ask for ART on
            // each side of the row, not in every cell.
            let art = |r: std::ops::Range<u16>| r.filter(|&x| glyph(&buf, x, y) != ' ').count();
            assert!(art(0..span.x) >= 2, "{cols} cols: no ornament left of the span on row {y} (span {span:?})");
            assert!(art(span.right()..cols) >= 2, "{cols} cols: no ornament right of the span on row {y} (span {span:?})");
            checked += 1;
        }
    }
    assert!(checked >= 150, "non-vacuity: {checked}");
}

/// The in-game score bar is between the art at EVERY width, on the host path (30..=100
/// columns at 25/40/70 rows, and the four host sizes) and in the terminal: one behaviour,
/// not one that flips with the rounding of the art's edge. Both lines read whole.
///
/// FALSIFY by deleting the `is_word_run` clamps in `chrome_run_origin` and `run_cols`: the
/// bar spans the art at 70 and 71 columns (and at 84..87 and 98..100 on the taller panes).
#[test]
fn the_score_bar_is_between_the_art_at_every_width() {
    let Some(s) = gameplay() else { return };
    let mut sizes: Vec<(u16, u16, u16, u16)> = HOST_SIZES.to_vec();
    for rows in [25u16, 40, 70] {
        for cols in 30..=100u16 {
            sizes.push((cols, rows, 8, 16));
        }
    }
    for size in sizes {
        let ring = host(&s, size);
        let who = format!("{}x{}@{}x{}", size.0, size.1, size.2, size.3);
        assert_between_the_art(&ring, &who);
        let bar = chrome_text(&ring);
        assert_eq!(bar.len(), 7, "{who}: the bar's seven fields: {:?}", bar.iter().map(|r| &r.run.text).collect::<Vec<_>>());
        for r in bar {
            let (a, b) = r.resolved.unwrap();
            assert_eq!(b - a, r.run.text.chars().count() as i32, "{who}: {:?} whole", r.run.text);
        }
    }
    for rows in [25u16, 40, 70] {
        for cols in (30..=100u16).step_by(5) {
            let span = tui_span(&s, cols, rows);
            let buf = tui(&s, cols, rows);
            for y in 0..2u16 {
                for x in (0..span.x).chain(span.right()..cols) {
                    let c = glyph(&buf, x, y);
                    assert!(!c.is_ascii_alphanumeric(), "{cols}x{rows}: bar text {c:?} over the art at ({x},{y})");
                }
            }
        }
    }
}
