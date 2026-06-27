//! Glyph-picker modal overlay for the style editor.
//!
//! Rendered over the style editor when `AppState.glyph_picker` is `Some`.
//! Returns `GlyphPickerRects` for mouse hit-testing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::input::{GLYPH_BLOCKS, GLYPH_GRID_COLS, picker_block_range};
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
/// Returns `None` when `state.glyph_picker` is `None` or the area is too small.
pub fn draw_glyph_picker(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<GlyphPickerRects> {
    let picker = state.glyph_picker.as_ref()?;

    let modal_w = MODAL_W.min(area.width.saturating_sub(4));
    let modal_h = MODAL_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    // Center the modal.
    let mx = area.x + (area.width - modal_w) / 2;
    let my = area.y + (area.height - modal_h) / 2;
    let modal = Rect::new(mx, my, modal_w, modal_h);

    // Style tokens from the live color scheme.
    let frame_style = state.colors.dialog;
    let title_style = state.colors.dialog_title;
    let button_style = state.colors.dialog_button;
    let cursor_style = state.colors.dialog_button_active;

    // Fill modal area opaque.
    let fill = Style::reset().patch(frame_style);
    for row in modal.y..modal.bottom() {
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ").set_style(fill);
            }
        }
    }

    // Draw a simple single-line border manually (avoids pulling in paneframe extras).
    draw_border(buf, modal, frame_style);

    // Content area = inside the border.
    if modal_w < 2 || modal_h < 2 {
        return None;
    }
    let cx = modal.x + 1;
    let cy = modal.y + 1;
    let cw = modal_w.saturating_sub(2);
    let ch = modal_h.saturating_sub(2);
    let content = Rect::new(cx, cy, cw, ch);

    // ── Close button [✕] ──────────────────────────────────────────────────────
    let close_rect = if modal.width >= 3 {
        let bx = modal.right().saturating_sub(2);
        let by = modal.y;
        if let Some(cell) = buf.cell_mut((bx, by)) {
            cell.set_symbol("✕").set_style(frame_style);
        }
        Some(Rect::new(bx, by, 1, 1))
    } else {
        None
    };

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

    // ── Title ─────────────────────────────────────────────────────────────────
    let title = "Glyph Picker";
    let title_x = modal.x + (modal_w.saturating_sub(title.len() as u16 + 2)) / 2;
    crate::render::draw_str_clipped(buf, title_x, modal.y, title, title_style, modal);

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

// ── Border helper ─────────────────────────────────────────────────────────────

fn draw_border(buf: &mut Buffer, r: Rect, style: Style) {
    if r.width < 2 || r.height < 2 {
        return;
    }
    let x0 = r.x;
    let x1 = r.right() - 1;
    let y0 = r.y;
    let y1 = r.bottom() - 1;

    // Corners.
    set_sym(buf, x0, y0, "┌", style);
    set_sym(buf, x1, y0, "┐", style);
    set_sym(buf, x0, y1, "└", style);
    set_sym(buf, x1, y1, "┘", style);

    // Top/bottom edges.
    for x in (x0 + 1)..x1 {
        set_sym(buf, x, y0, "─", style);
        set_sym(buf, x, y1, "─", style);
    }

    // Left/right edges.
    for y in (y0 + 1)..y1 {
        set_sym(buf, x0, y, "│", style);
        set_sym(buf, x1, y, "│", style);
    }
}

fn set_sym(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(s).set_style(style);
    }
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
        assert!(state.glyph_picker.is_some());
        // Inject an invalid (double-width) pending glyph directly.
        state.glyph_picker.as_mut().unwrap().pending = Some("漢".into());

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
        assert!(state.glyph_picker.is_some());

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
