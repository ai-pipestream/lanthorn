//! SQ-1822: Shogun's boot menu items stay whole in the story box on a narrow pane.
//!
//! Shogun r322 (`shogun-r322-s890706.z6`, serial 890706, IBM PC press) prints its boot
//! menu, "START the game" / "RESTORE a saved game" / "QUIT the game", INSIDE window 0's
//! story box (the in-box pass of `render_node`, not a chrome strip), one terminal cell
//! per native text cell from the box's left edge, with the selection bar a reversed
//! run. The items are laid at game column 29 (cell 24 of the 548px box), so a box
//! narrower than 44 cells clipped them on the right while blank cells sat to their left
//! ("START the " at 40 columns). `story_row_cols` now slides a LONE run that overruns
//! the box left until it ends at the box's right edge (SQ-1820's rule, shared through
//! `lone_glyph_run` / `slide_left_to_fit`), and the click map follows
//! (`story_slid_click_runs`).
//!
//! Specimen frame: one space past the splash, "You may choose to:" in the transcript
//! and the selection bar on START. Hybrid on kitty at 8x16, `honor_game_colours` on.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::render::screen::{hybrid_chrome_layout, render_story_pane};
use app::render::v6_layout as v6;
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const ITEMS: [&str; 3] = ["START the game", "RESTORE a saved game", "QUIT the game"];

/// The cell an item is laid at, from the box's left edge: game column 29 less the box's
/// own 5 (46 px / 8).
const LAID_AT: i32 = 24;

fn boot_menu() -> Option<GameSession> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/shogun-r322-s890706.z6");
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, picture_dims, picts.std_window(), None, None).ok()?;
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    let r = s.submit_char(b' ');
    assert!(r.transcript.contains("You may choose to:"), "premise: the boot menu");
    Some(s)
}

fn state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.set_game_picker(Some(app::render::graphics::kitty_picker(8, 16)));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st
}

fn overlay(s: &GameSession, cols: u16) -> app::render::screen::V6HybridChromeLayout {
    let st = state();
    let area = Rect::new(0, 0, cols, 30);
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let native = v6::native_extent(items, &st.v6_text);
    let layout = v6::classify_windows(items, st.v6_text.cell());
    hybrid_chrome_layout(&layout, native, area, (8, 16), &st).expect("a hybrid chrome layout")
}

fn row_text(buf: &Buffer, y: u16, width: u16) -> String {
    (0..width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

fn render(s: &GameSession, cols: u16) -> Buffer {
    let st = state();
    let area = Rect::new(0, 0, cols, 30);
    let mut buf = Buffer::empty(area);
    let _ = render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
    buf
}

/// The three items slide TOGETHER (SQ-1823): they are consecutive rows laid at one
/// column, so a block shares the largest overrun, RESTORE's, and their left edges stay
/// aligned. Whole wherever the box holds RESTORE (20 cells); the longest ends at the
/// box's right edge once the block slides; a pane wide enough for all three to sit where
/// the game laid them is untouched. The host layout agrees with the terminal.
///
/// FALSIFY by sliding each row by its own overrun (`story_block_slides` returning each
/// row's own `slide_left_to_fit`): START, RESTORE and QUIT start at three different
/// columns below 44 cells of box.
#[test]
fn boot_menu_items_slide_together_and_stay_whole() {
    let Some(s) = boot_menu() else { return };
    let (mut slid, mut narrowest_whole) = (0, u16::MAX);
    for cols in (20u16..=56).chain([60, 72, 80, 100]) {
        let buf = render(&s, cols);
        let hyb = overlay(&s, cols);
        let vp = hyb.viewport;
        let longest = ITEMS.iter().map(|i| i.len()).max().unwrap() as i32;
        let laid = i32::from(vp.x) + LAID_AT;
        let want = if laid + longest <= i32::from(vp.right()) {
            laid
        } else {
            (i32::from(vp.right()) - longest).max(i32::from(vp.x))
        };
        let mut whole = true;
        for item in ITEMS {
            let run = hyb
                .story_overlay
                .iter()
                .find(|r| r.run.text.trim() == item)
                .unwrap_or_else(|| panic!("{cols}: no overlay run for {item:?}"));
            let (a, b) = run.spans[0];
            let text = row_text(&buf, run.row as u16, cols);
            whole &= text.contains(item);
            if text.contains(item) {
                let at = text.find(item).map(|byte| text[..byte].chars().count() as i32);
                assert_eq!(at, Some(a), "{cols} cols: host and terminal agree on {item:?}");
            }
            assert_eq!(a, want, "{cols} cols: {item:?} starts at the block's shared column");
            assert!(b > a);
        }
        if want != laid {
            slid += 1;
            let r = hyb.story_overlay.iter().find(|r| r.run.text.trim() == "RESTORE a saved game").unwrap();
            // The longest ends at the box's right edge, unless the box is narrower than it.
            if want != i32::from(vp.x) {
                assert_eq!(r.spans[0].1, i32::from(vp.right()), "{cols} cols: RESTORE ends at the right edge");
            }
        }
        if whole {
            narrowest_whole = narrowest_whole.min(cols);
        }
        if i32::from(vp.right()) - i32::from(vp.x) >= longest {
            assert!(whole, "{cols} cols: the box holds RESTORE, so every item is whole");
        }
    }
    eprintln!("narrowest width with all three items whole: {narrowest_whole}");
    assert!(slid > 10, "non-vacuity: only {slid} slid widths");
}

/// A click on any drawn cell of a slid item resolves to that item's own native span,
/// and the game takes it: the selection bar moves to the clicked item.
///
/// FALSIFY by emptying `story_slid_click_runs`: the cells of a slid item resolve a whole
/// slide to the right of where it is drawn, and the bar lands on the wrong item.
#[test]
fn clicks_on_slid_items_select_them() {
    for cols in [36u16, 40, 42, 44, 46, 51] {
        for pick in ITEMS {
            let Some(mut s) = boot_menu() else { return };
            let hyb = overlay(&s, cols);
            let run = hyb.story_overlay.iter().find(|r| r.run.text.trim() == pick).unwrap();
            let (a, _) = run.spans[0];
            let x = i32::from(run.run.x.max(1));
            let (x0, x1) = (x - 4, x + pick.len() as i32 * 8 + 4);
            for c in a..a + pick.len() as i32 {
                let (gx, _) = hyb
                    .click_map
                    .map_click(c as u16, run.row as u16)
                    .unwrap_or_else(|| panic!("{cols}: {pick:?} cell {c} maps to None"));
                assert!(
                    (x0..=x1).contains(&i32::from(gx)),
                    "{cols} cols: {pick:?} cell {c} clicks to native x {gx}, outside {x0}..{x1}"
                );
            }
            let mid = (a + pick.len() as i32 / 2) as u16;
            let (gx, gy) = hyb.click_map.map_click(mid, run.row as u16).unwrap();
            s.set_mouse(gy, gx);
            let _ = s.submit_char(254);
            let after = overlay(&s, cols);
            let bar: Vec<&str> =
                after.story_overlay.iter().filter(|r| r.run.style & 1 != 0).map(|r| r.run.text.trim()).collect();
            assert_eq!(bar, [pick], "{cols} cols: clicking {pick:?} selects it");
        }
    }
}
