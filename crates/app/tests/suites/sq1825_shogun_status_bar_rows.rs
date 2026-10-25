//! SQ-1825: a multi-line chrome bar gets one terminal row per game text row, and a field the
//! scale lays on top of its neighbour is not dropped.
//!
//! Shogun r322's in-game status bar is window 1, two 16px game rows: y=0 "Erasmus: ...
//! SHOGUN ... Score: 0" and y=17 "Bridge ... Moves: 0". The band above the story is as many
//! terminal rows as the scaled native height rounds to, so in portrait, where a terminal
//! cell is much taller than a scaled game row (39x30 cells of 27x60 device px: the bar is
//! 52 px, less than one 60 px cell), it got ONE row: the second line vanished (SQ-1817's row
//! oracle had no room to push it into) and the score's "0", five columns after "Score:" in
//! the game but landing on top of it, was dropped as a stray glyph. The viewport now gives
//! up the rows the chrome text above it needs, and a field the scale lays on the field
//! before it is moved to one blank cell past it.
//!
//! Specimen: `shogun-r322-s890706.z6` (release 322, serial 890706), IBM PC press, TWO keys
//! past the splash: Space (the title card and boot menu), then Enter (START). The first
//! gameplay frame: window 1 holds the bar, the transcript opens on the bridge. Hybrid on
//! kitty, `honor_game_colours` on.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::render::screen::{hybrid_chrome_layout, render_story_pane, V6HybridChromeLayout};
use app::render::v6_layout as v6;
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn first_gameplay_frame() -> Option<GameSession> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/shogun-r322-s890706.z6");
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, picture_dims, picts.std_window(), None, None).ok()?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    let _ = s.submit_char(b' ');
    let r = s.submit_char(13);
    assert!(r.transcript.contains("Bridge"), "premise: the first gameplay frame");
    Some(s)
}

fn state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.set_game_picker(Some(app::render::graphics::kitty_picker(8, 16)));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st
}

fn host_layout(s: &GameSession, cols: u16, rows: u16, cell_px: (u16, u16)) -> V6HybridChromeLayout {
    let st = state();
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    hybrid_chrome_layout(&layout, native, Rect::new(0, 0, cols, rows), cell_px, &st).expect("a hybrid chrome layout")
}

/// The four device sizes the embedding host reported: two portrait ones (which failed) and
/// two landscape ones (which did not).
const HOST_SIZES: [(u16, u16, u16, u16); 4] = [(39, 30, 27, 60), (42, 35, 20, 46), (71, 16, 27, 60), (88, 36, 10, 23)];

/// Both lines of the bar are on distinct terminal rows, "Score: 0" and "Moves: 0" read whole
/// (the value a blank past its label, on the label's own row), and the story viewport starts
/// below the bar. Landscape sizes keep their layout.
///
/// FALSIFY by removing the viewport push in `build_hybrid_frame_with` (the `SQ-1825` block
/// before `content_ring_bands`): the portrait sizes lose the second line. Or by zeroing the
/// `nudge` in `row_spans`: the portrait "0"s resolve `None`.
#[test]
fn both_status_lines_get_rows_at_every_host_size() {
    let Some(s) = first_gameplay_frame() else { return };
    for (cols, rows, cw, ch) in HOST_SIZES {
        let who = format!("{cols}x{rows}@{cw}x{ch}");
        let hyb = host_layout(&s, cols, rows, (cw, ch));
        let find = |text: &str, y: u16| {
            hyb.runs
                .iter()
                .find(|r| r.run.text == text && r.run.y == y)
                .unwrap_or_else(|| panic!("{who}: no chrome run {text:?} at native y {y}"))
        };
        let (erasmus, bridge) = (find("Erasmus:", 1), find("Bridge", 17));
        assert_eq!(bridge.row, erasmus.row + 1, "{who}: the two game rows are on consecutive terminal rows");
        assert!(i32::from(hyb.viewport.y) > bridge.row, "{who}: the story viewport {:?} starts below the bar", hyb.viewport);
        for (label, y) in [("Score:", 1u16), ("Moves:", 17)] {
            let l = find(label, y);
            let value = hyb
                .runs
                .iter()
                .find(|r| r.run.text == "0" && r.run.y == y)
                .unwrap_or_else(|| panic!("{who}: no value run at native y {y}"));
            assert_eq!(value.row, l.row, "{who}: {label} and its value share a row");
            let (_, label_end) = l.resolved.unwrap_or_else(|| panic!("{who}: {label} is dropped"));
            let (value_at, _) = value.resolved.unwrap_or_else(|| panic!("{who}: the value after {label} is dropped"));
            assert!(value_at > label_end, "{who}: {label} {label_end} is followed by a blank before its value at {value_at}");
        }
    }
    // Landscape: nothing collided, so nothing moved (the bar already had its two rows).
    for (cols, rows, cw, ch) in &HOST_SIZES[2..] {
        let hyb = host_layout(&s, *cols, *rows, (*cw, *ch));
        assert_eq!(hyb.viewport.y, 2, "{cols}x{rows}: the landscape viewport still starts under two rows");
    }
}

fn render(s: &GameSession, cols: u16, rows: u16) -> Buffer {
    let st = state();
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16, width: u16) -> String {
    // Image placements (the flank art beside the bar) are not text.
    (0..width)
        .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().filter(|c| !c.is_control() && *c < '\u{E000}').unwrap_or(' '))
        .collect()
}

/// The terminal ring at a portrait-like 39 columns by 60 rows, and every width from 25 to 100
/// at three heights: the bar is two rows, whole, with its values.
///
/// FALSIFY as above.
#[test]
fn the_terminal_bar_is_two_whole_rows_at_every_width() {
    let Some(s) = first_gameplay_frame() else { return };
    let mut sizes: Vec<(u16, u16)> = vec![(39, 60)];
    for cols in 25..=100u16 {
        for rows in [25u16, 40, 60] {
            sizes.push((cols, rows));
        }
    }
    for (cols, rows) in sizes {
        let buf = render(&s, cols, rows);
        let (top, second) = (row_text(&buf, 0, cols), row_text(&buf, 1, cols));
        let words = |line: &str| line.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let (top_words, second_words) = (words(&top), words(&second));
        // SQ-1826: the bar stays between the side art (three columns each side at the
        // narrowest) and is truncated on the right where it cannot fit: below 30 columns
        // the first line loses its tail, never its start.
        if cols < 30 {
            assert!(top_words.first().is_some_and(|w| w == "Erasmus:"), "{cols}x{rows}: row 0 still starts the first line: {top:?}");
            assert!(second_words.first().is_some_and(|w| w == "Bridge"), "{cols}x{rows}: row 1 still starts the second line: {second:?}");
            continue;
        }
        assert!(
            top_words.first().is_some_and(|w| w == "Erasmus:") && top_words.ends_with(&["Score:".into(), "0".into()]),
            "{cols}x{rows}: row 0 is the first line, the score a blank past its label: {top:?}"
        );
        assert!(
            second_words.first().is_some_and(|w| w == "Bridge") && second_words.ends_with(&["Moves:".into(), "0".into()]),
            "{cols}x{rows}: row 1 is the second line, the count a blank past its label: {second:?}"
        );
    }
}
