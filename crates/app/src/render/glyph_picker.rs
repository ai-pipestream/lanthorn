//! Glyph-picker modal overlay for the style editor.
//!
//! Rendered over the style editor when `AppState.glyph_picker` is `Some`.
//! Returns `GlyphPickerRects` for mouse hit-testing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::input::{GLYPH_BLOCKS, GLYPH_GRID_COLS, picker_block_range};
use crate::render::dialog::{draw_dialog, DialogPlacement, DialogSpec, DialogStyle, Placement};
use crate::render::paneframe::{BorderStyle, PaneGlyphs};
use crate::state::AppState;

// Modal dimensions.
const MODAL_W: u16 = 62;
const MODAL_H: u16 = 22;
const MIN_W: u16 = 40;
const MIN_H: u16 = 14;

// ── GlyphPickerRects ──────────────────────────────────────────────────────────

/// Hit-rects produced by `draw_glyph_picker` for mouse handling.
pub struct GlyphPickerRects {
    /// Full modal area (clicks outside here are ignored while modal is open).
    pub area: Rect,
    /// The ✕ close button.
    pub close: Option<Rect>,
    /// Each valid glyph cell in the curated-block grid: (glyph_str, rect).
    pub glyphs: Vec<(String, Rect)>,
    /// Each MRU glyph cell: (glyph_str, rect).
    pub mru: Vec<(String, Rect)>,
    /// ◀ previous-block button.
    pub blocks_prev: Option<Rect>,
    /// ▶ next-block button.
    pub blocks_next: Option<Rect>,
    /// `[Clear]` button.
    pub clear: Option<Rect>,
    /// Custom-range entry area (click to activate custom entry).
    pub custom: Option<Rect>,
}

// ── draw_glyph_picker ─────────────────────────────────────────────────────────

/// Draw the glyph-picker modal centered over `area`.
///
/// Returns `None` when `state.overlays.glyph_picker` is `None` or the area is too small.
pub fn draw_glyph_picker(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<GlyphPickerRects> {
    let picker = state.overlays.glyph_picker.as_ref()?;

    let modal_w = MODAL_W.min(area.width.saturating_sub(4));
    let modal_h = MODAL_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    // Style tokens from the live color scheme (same selectors as before).
    let frame_style = state.colors.dialog;
    let title_style = state.colors.dialog_title;
    let button_style = state.colors.dialog_button;
    let cursor_style = state.colors.dialog_button_active;

    // Frame, opaque fill, centered bracketed title, and the ✕ close button all
    // come from the shared dialog chrome now. The picker keeps its fixed
    // single-line, centered, shadowless geometry (its tight glyph grid depends on
    // the content rect being inset by exactly one cell), so it builds an explicit
    // DialogStyle rather than honoring the dialog box/glyph/shadow/placement theme
    // selectors it never used.
    let st = DialogStyle {
        frame: frame_style,
        box_style: BorderStyle::Single,
        glyphs: PaneGlyphs::default(),
        title: title_style,
        button: button_style,
        button_active: cursor_style,
        shadow: state.colors.dialog_shadow,
        shadow_on: false,
        placement: DialogPlacement::Center,
        margin: 0,
    };
    let spec = DialogSpec {
        title: "Glyph Picker",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
        field: None,
    };
    let dialog_rects = draw_dialog(buf, area, &spec, &st);

    let modal = dialog_rects.area;
    let content = dialog_rects.content;
    let cw = content.width;
    let close_rect = dialog_rects.close;

    // ── Row 0: block name header with ◀ ▶ ────────────────────────────────────
    let mut prev_rect: Option<Rect> = None;
    let mut next_rect: Option<Rect> = None;
    if content.height >= 1 {
        let header_y = content.y;
        let block_name = if picker.custom_start.is_some() {
            "Custom Range"
        } else {
            GLYPH_BLOCKS[picker.block.min(GLYPH_BLOCKS.len() - 1)].0
        };

        // ◀ at content.x
        if content.width >= 1 {
            let bx = content.x;
            if let Some(cell) = buf.cell_mut((bx, header_y)) {
                cell.set_symbol("◀").set_style(button_style);
            }
            prev_rect = Some(Rect::new(bx, header_y, 1, 1));
        }

        // Block name centered.
        let label = format!(" {} ", block_name);
        let label_len = label.chars().count() as u16;
        let label_x = content.x + 1 + (cw.saturating_sub(2).saturating_sub(label_len)) / 2;
        crate::render::draw_str_clipped(buf, label_x, header_y, &label, title_style, content);

        // ▶ at content.right()-1
        if content.width >= 2 {
            let bx = content.right().saturating_sub(1);
            if let Some(cell) = buf.cell_mut((bx, header_y)) {
                cell.set_symbol("▶").set_style(button_style);
            }
            next_rect = Some(Rect::new(bx, header_y, 1, 1));
        }
    }

    // ── Separator after header ────────────────────────────────────────────────
    if content.height >= 2 {
        let sep_y = content.y + 1;
        for col in content.x..content.right() {
            if let Some(cell) = buf.cell_mut((col, sep_y)) {
                cell.set_symbol("─").set_style(frame_style);
            }
        }
    }

    // ── Glyph grid ────────────────────────────────────────────────────────────
    let grid_start_y = content.y + 2; // after header + separator
    let mut glyph_rects: Vec<(String, Rect)> = Vec::new();

    if content.height > 2 {
        let (lo, hi) = picker_block_range(picker);
        let pending_glyph = picker.pending.as_deref();

        let mut grid_idx = 0usize;
        'outer: for cp in lo..=hi {
            if let Some(c) = char::from_u32(cp) {
                let s = c.to_string();
                if crate::style_mru::is_valid_glyph(&s) {
                    let col = grid_idx % GLYPH_GRID_COLS;
                    let row = grid_idx / GLYPH_GRID_COLS;
                    let gx = content.x + col as u16 * 2;
                    let gy = grid_start_y + row as u16;

                    // Stop if we've run out of vertical space (leave room for MRU/clear rows).
                    if gy + 3 >= content.bottom() {
                        break 'outer;
                    }
                    if gx + 1 >= content.right() {
                        grid_idx += 1;
                        continue;
                    }

                    // Highlight cursor position OR pending match.
                    let is_cursor = pending_glyph.map_or(grid_idx == picker.cursor, |p| p == s);
                    let cell_style = if is_cursor { cursor_style } else { frame_style };

                    if let Some(cell) = buf.cell_mut((gx, gy)) {
                        cell.set_symbol(&s).set_style(cell_style);
                    }
                    // Space after glyph.
                    if let Some(cell) = buf.cell_mut((gx + 1, gy)) {
                        cell.set_symbol(" ").set_style(cell_style);
                    }

                    glyph_rects.push((s, Rect::new(gx, gy, 2, 1)));
                    grid_idx += 1;
                }
            }
        }
    }

    // ── MRU row ───────────────────────────────────────────────────────────────
    let mru_y = content.bottom().saturating_sub(3);
    let mut mru_rects: Vec<(String, Rect)> = Vec::new();

    if content.height >= 4 && !picker.mru.is_empty() {
        // Label.
        crate::render::draw_str_clipped(buf, content.x, mru_y, "MRU:", frame_style, content);
        let mru_start_x = content.x + 5;
        let mut mx_pos = mru_start_x;
        for g in &picker.mru {
            if mx_pos + 1 >= content.right() {
                break;
            }
            let is_match = picker.pending.as_deref() == Some(g.as_str());
            let cell_style = if is_match { cursor_style } else { button_style };
            if let Some(cell) = buf.cell_mut((mx_pos, mru_y)) {
                cell.set_symbol(g).set_style(cell_style);
            }
            if let Some(cell) = buf.cell_mut((mx_pos + 1, mru_y)) {
                cell.set_symbol(" ").set_style(frame_style);
            }
            mru_rects.push((g.clone(), Rect::new(mx_pos, mru_y, 2, 1)));
            mx_pos += 2;
        }
    }

    // ── Custom range entry ────────────────────────────────────────────────────
    let custom_y = content.bottom().saturating_sub(2);
    let mut custom_rect: Option<Rect> = None;
    if content.height >= 3 {
        let label = if picker.custom_focus {
            // Show typed hex digits padded to at least 4 underscores.
            let padded = format!("{:_<4}", &picker.custom_buf);
            format!("custom: U+{}", padded)
        } else if let Some(start) = picker.custom_start {
            format!("custom: U+{:04X}", start)
        } else {
            "custom: U+____".to_string()
        };
        let custom_style = if picker.custom_focus { cursor_style } else { frame_style };
        crate::render::draw_str_clipped(buf, content.x, custom_y, &label, custom_style, content);
        let llen = label.chars().count() as u16;
        custom_rect = Some(Rect::new(content.x, custom_y, llen.min(cw), 1));
    }

    // ── Clear button ──────────────────────────────────────────────────────────
    let clear_y = content.bottom().saturating_sub(1);
    let mut clear_rect: Option<Rect> = None;
    if content.height >= 2 {
        let label = "[ Clear ]";
        let llen = label.chars().count() as u16;
        let clear_x = content.right().saturating_sub(llen);
        crate::render::draw_str_clipped(buf, clear_x, clear_y, label, button_style, content);
        clear_rect = Some(Rect::new(clear_x, clear_y, llen.min(cw), 1));
    }

    // ── Invalid-pending warning hint ──────────────────────────────────────────
    if let Some(p) = picker.pending.as_deref() {
        if !crate::style_mru::is_valid_glyph(p) {
            let hint = format!("'{p}' invalid \u{2014} single-width only");
            let warn_style = state.colors.transcript_warning;
            // Render on the row just above MRU; the grid's safety margin leaves
            // mru_y clear, so only mru_y-1 is a potential overlap (acceptable
            // for a warning, which is drawn last and takes visual priority).
            let warn_y = mru_y.saturating_sub(1);
            crate::render::draw_str_clipped(buf, content.x, warn_y, &hint, warn_style, content);
        }
    }

    Some(GlyphPickerRects {
        area: modal,
        close: close_rect,
        glyphs: glyph_rects,
        mru: mru_rects,
        blocks_prev: prev_rect,
        blocks_next: next_rect,
        clear: clear_rect,
        custom: custom_rect,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_pending_shows_warning_hint() {
        use ratatui::{backend::TestBackend, Terminal};
        use crate::input::open_style_editor_hermetic;
        use crate::input::Action;
        use crate::input::apply_action;
        use mapper::mapper::Mapper;

        // Confirm "漢" is indeed invalid (double-width) so the test is meaningful.
        assert!(
            !crate::style_mru::is_valid_glyph("漢"),
            "漢 must be an invalid glyph (double-width) for this test to be meaningful",
        );

        let mut state = crate::state::AppState::default();
        open_style_editor_hermetic(&mut state);
        apply_action(
            Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top),
            &mut state,
            &mut Mapper::default(),
        );
        assert!(state.overlays.glyph_picker.is_some());
        // Inject an invalid (double-width) pending glyph directly.
        state.overlays.glyph_picker.as_mut().unwrap().pending = Some("漢".into());

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let _ = draw_glyph_picker(&state, f.area(), f.buffer_mut());
            })
            .unwrap();

        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("single-width"), "invalid pending must show a single-width-only hint");
    }

    #[test]
    fn glyph_picker_title_uses_shared_dialog_chrome() {
        // Adopting `draw_dialog` means the title is now the shared bracketed strip
        // "┫ Glyph Picker ┣" (matching every other modal), drawn by draw_top_inset,
        // rather than the old plain centered title. The row is contiguous, so the
        // bracketed run survives row-major concatenation of the buffer.
        use ratatui::{backend::TestBackend, Terminal};
        use crate::input::open_style_editor_hermetic;
        use crate::input::{apply_action, Action};
        use mapper::mapper::Mapper;

        let mut state = crate::state::AppState::default();
        open_style_editor_hermetic(&mut state);
        apply_action(
            Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top),
            &mut state,
            &mut Mapper::default(),
        );

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let _ = draw_glyph_picker(&state, f.area(), f.buffer_mut());
            })
            .unwrap();
        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("┫ Glyph Picker ┣"), "shared bracketed dialog title strip");
    }

    #[test]
    fn glyph_picker_renders_title_and_header() {
        use ratatui::{backend::TestBackend, Terminal};
        use crate::input::open_style_editor_hermetic;
        use crate::input::Action;
        use crate::input::apply_action;
        use mapper::mapper::Mapper;

        let mut state = crate::state::AppState::default();
        open_style_editor_hermetic(&mut state);
        apply_action(
            Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top),
            &mut state,
            &mut Mapper::default(),
        );
        assert!(state.overlays.glyph_picker.is_some());

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<GlyphPickerRects> = None;
        terminal
            .draw(|f| {
                rects = draw_glyph_picker(&state, f.area(), f.buffer_mut());
            })
            .unwrap();

        let r = rects.expect("modal should render");
        assert!(r.close.is_some(), "close button present");
        assert!(r.blocks_prev.is_some(), "◀ present");
        assert!(r.blocks_next.is_some(), "▶ present");
        assert!(!r.glyphs.is_empty(), "glyph grid non-empty for Box Drawing block");

        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("Glyph Picker"), "title present");
        assert!(all.contains("Box Drawing"), "block name present");
    }
}
