//! SQ-1824: a slid story-box menu item never lands on text already on its cell row.
//!
//! Shogun r322 (`shogun-r322-s890706.z6`, IBM PC press) prints its boot prompt, "You may
//! choose to:", in window 0's transcript on the SAME cell row as START, the first item of
//! the menu window laid over the story box (SQ-1822). The slide that keeps a menu item
//! whole on a narrow pane (SQ-1822/1823) had no idea the prose was there: from 20 to 45
//! columns START slid onto it, and at 39 the row read "You may choose" with START over
//! "to:". A slide now stops one blank cell after the prose already drawn to its left on its
//! row, a block of items stops at its most constrained member so the left edges stay
//! aligned, and whatever then still overruns the box clips on the right. A second defect
//! hid behind it: the menu window's erase panel was mapped through the ring scale, not
//! counted from the box like the text it carries, so it blanked the prose before the menu
//! at widths where the scale and the box disagree (a colon missing at 60 columns).
//!
//! Specimen: one space past the splash, the prompt pushed into the transcript as the app
//! does. Hybrid on kitty at 8x16, `honor_game_colours` on.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::render::screen::{hybrid_chrome_layout, render_story_pane, V6HybridChromeLayout};
use app::render::v6_layout as v6;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const PROMPT: &str = "You may choose to:";
const ITEMS: [&str; 3] = ["START the game", "RESTORE a saved game", "QUIT the game"];

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
    assert!(r.transcript.ends_with(PROMPT), "premise: the boot prompt is the transcript's last line");
    Some(s)
}

fn state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.set_game_picker(Some(app::render::graphics::kitty_picker(8, 16)));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.push_transcript_kind(PROMPT, TranscriptKind::Story);
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

fn render(s: &GameSession, cols: u16) -> Buffer {
    let st = state();
    let area = Rect::new(0, 0, cols, 30);
    let mut buf = Buffer::empty(area);
    let _ = render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
    buf
}

fn row_chars(buf: &Buffer, y: u16, width: u16) -> Vec<char> {
    (0..width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// From 20 to 100 columns the prompt is whole on its row wherever the box holds it, one
/// blank cell separates it from the item that follows, and the host layout agrees with the
/// terminal about where each item is drawn. Reports the narrowest width with all three
/// items whole: with the prompt on START's row they cannot all fit before the box holds
/// prompt, a blank and RESTORE.
///
/// FALSIFY by making `story_block_slides` ignore the floor (`c.3 = i32::MIN` after the
/// `min_col` loop): START slides over "to:" (30..38 columns) and the prompt is gone.
#[test]
fn slid_menu_never_touches_the_prompt() {
    let Some(s) = boot_menu() else { return };
    let (mut narrowest_whole, mut slid, mut hidden) = (u16::MAX, 0, 0);
    for cols in 20u16..=100 {
        let buf = render(&s, cols);
        let hyb = host_layout(&s, cols, 30, (8, 16));
        let vp = hyb.viewport;
        // A box too narrow to leave START any cell past the prompt hides it altogether.
        let Some(start) = hyb.story_overlay.iter().find(|r| r.run.text.trim() == ITEMS[0]) else {
            hidden += 1;
            continue;
        };
        let row = start.row as u16;
        let text = row_chars(&buf, row, cols);
        let line: String = text.iter().collect();
        if vp.width >= 19 {
            assert!(line.contains(PROMPT), "{cols} cols: the prompt is whole and untouched on START's row: {line:?}");
        }
        let prompt_at = line.find(PROMPT).map(|b| line[..b].chars().count() as i32);
        let mut whole = true;
        for item in ITEMS {
            let Some(run) = hyb.story_overlay.iter().find(|r| r.run.text.trim() == item) else {
                whole = false;
                continue;
            };
            let (a, _) = run.spans[0];
            let shown: String = (a..i32::from(vp.right())).map(|c| text_at(&buf, c, run.row)).collect();
            whole &= shown.starts_with(item);
            assert!(item.starts_with(shown.trim_end()) || shown.starts_with(item), "{cols} cols: {item:?} drawn as {shown:?}");
            if let Some(p) = prompt_at {
                assert!(a > p + PROMPT.len() as i32, "{cols} cols: {item:?} at {a} is not past the prompt ending {}", p + 18);
            }
        }
        if whole {
            narrowest_whole = narrowest_whole.min(cols);
        }
        if a_of(&hyb, ITEMS[0]) < i32::from(vp.x) + 24 {
            slid += 1;
        }
        if i32::from(vp.right()) - i32::from(vp.x) >= PROMPT.len() as i32 + 1 + ITEMS[1].len() as i32 {
            assert!(whole, "{cols} cols: the box holds prompt, a blank and RESTORE, so every item is whole");
        }
    }
    eprintln!("narrowest width with all three items whole beside the prompt: {narrowest_whole}; START hidden at {hidden} widths");
    assert!(slid > 5, "non-vacuity: only {slid} widths slid an item up against the prompt");
}

fn a_of(hyb: &V6HybridChromeLayout, item: &str) -> i32 {
    hyb.story_overlay.iter().find(|r| r.run.text.trim() == item).unwrap().spans[0].0
}

fn text_at(buf: &Buffer, col: i32, row: i32) -> char {
    buf.cell((col as u16, row as u16)).unwrap().symbol().chars().next().unwrap_or(' ')
}

/// The host path at the Retroid's portrait cells (39x30 of 27x60, 42x35 of 20x46): the
/// items sit past the prompt, which fills the first 18 cells of the box.
///
/// FALSIFY as above.
#[test]
fn host_portrait_items_sit_past_the_prompt() {
    let Some(s) = boot_menu() else { return };
    for (cols, rows, cell) in [(39u16, 30u16, (27u16, 60u16)), (42, 35, (20, 46))] {
        let hyb = host_layout(&s, cols, rows, cell);
        let vp = hyb.viewport;
        for item in ITEMS {
            let a = a_of(&hyb, item);
            assert!(a > i32::from(vp.x) + PROMPT.len() as i32, "{cols}x{rows}@{cell:?}: {item:?} at {a} overlaps the prompt (box starts {})", vp.x);
        }
    }
}

/// A click on any visible cell of a slid item still selects it, with the prompt on the
/// row.
///
/// FALSIFY by emptying `story_slid_click_runs`: the cells of a slid item resolve a whole
/// slide to the right of where they are drawn, and the bar lands on the wrong item.
#[test]
fn clicks_on_items_beside_the_prompt_select_them() {
    for cols in [30u16, 36, 39, 42, 46, 51, 60] {
        for pick in ITEMS {
            let Some(mut s) = boot_menu() else { return };
            let hyb = host_layout(&s, cols, 30, (8, 16));
            let run = hyb.story_overlay.iter().find(|r| r.run.text.trim() == pick).unwrap();
            let (a, _) = run.spans[0];
            let x = i32::from(run.run.x.max(1));
            let (x0, x1) = (x - 4, x + pick.len() as i32 * 8 + 4);
            for c in a..(a + pick.len() as i32).min(i32::from(hyb.viewport.right())) {
                let (gx, _) = hyb
                    .click_map
                    .map_click(c as u16, run.row as u16)
                    .unwrap_or_else(|| panic!("{cols}: {pick:?} cell {c} maps to None"));
                assert!(
                    (x0..=x1).contains(&i32::from(gx)),
                    "{cols} cols: {pick:?} cell {c} clicks to native x {gx}, outside {x0}..{x1}"
                );
            }
            let mid = (a + (pick.len() as i32 / 2).min(i32::from(hyb.viewport.right()) - a - 1)) as u16;
            let (gx, gy) = hyb.click_map.map_click(mid, run.row as u16).unwrap();
            s.set_mouse(gy, gx);
            let _ = s.submit_char(254);
            let after = host_layout(&s, cols, 30, (8, 16));
            let bar: Vec<&str> =
                after.story_overlay.iter().filter(|r| r.run.style & 1 != 0).map(|r| r.run.text.trim()).collect();
            assert_eq!(bar, [pick], "{cols} cols: clicking {pick:?} selects it");
        }
    }
}
