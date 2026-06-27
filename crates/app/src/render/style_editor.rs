//! Style-editor preview board overlay.
//!
//! Draws a full-screen modal showing all styleable selectors as labeled
//! samples. Each sample is styled from `ed.preview` so live edits render
//! immediately.  The active row is highlighted. Returns hit-rects for every
//! sample (used by the mouse handler to set `ed.active`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::input::AttrKind;
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::{AppState, StyleFocus};
use crate::style::{SELECTOR_GROUPS, style_for_selector};

/// The five attribute chips in display order.
const ATTR_KINDS: [(AttrKind, &str); 5] = [
    (AttrKind::Bold,      "[B]  "),
    (AttrKind::Italic,    "[I]  "),
    (AttrKind::Underline, "[U]  "),
    (AttrKind::Dim,       "[dim]"),
    (AttrKind::Reversed,  "[rev]"),
];

/// Hit-rects returned from `draw_style_editor`.
///
/// `samples` maps each drawn sample to `(global_selector_index, Rect)`.
/// `attr_chips` maps each attribute chip to its `(AttrKind, Rect)`.
/// `fg_swatches`/`bg_swatches`: 17 rects each (indices 0-15 = ANSI, 16 = default).
/// `mru_rects`: one rect per drawn MRU cell (index == `ed.mru` position).
/// `custom_rect`: the custom hex-entry field.
pub struct StyleEditorRects {
    pub samples: Vec<(usize, Rect)>,
    pub attr_chips: Vec<(AttrKind, Rect)>,
    pub dialog: DialogRects,
    pub fg_swatches: Vec<Rect>,
    pub bg_swatches: Vec<Rect>,
    pub mru_rects: Vec<Rect>,
    pub custom_rect: Option<Rect>,
}

/// Draw the style-editor full-screen overlay onto `buf`.
///
/// Returns `Some(StyleEditorRects)` when drawn, `None` when
/// `state.style_editor` is `None` or the area is too small.
pub fn draw_style_editor(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<StyleEditorRects> {
    let Some(ed) = &state.style_editor else { return None };

    // Total count of selectors across all groups (used for wrapping nav).
    let total_selectors: usize = SELECTOR_GROUPS.iter().map(|(_, s)| s.len()).sum();
    if total_selectors == 0 {
        return None;
    }

    // Wide enough for board (≥30 cols) + gap + property pane (40 cols).
    let modal_w = 86u16.min(area.width.saturating_sub(4));
    // rows = all selectors + one header per group + 2 padding lines.
    let n_groups = SELECTOR_GROUPS.len() as u16;
    let n_rows = total_selectors as u16 + n_groups + 2;
    let modal_h = (n_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 6 {
        return None;
    }

    // Build DialogStyle from state colors (same pattern as config_screen).
    let ds = DialogStyle {
        frame: state.colors.dialog,
        box_style: state.colors.dialog_box_style,
        title: state.colors.dialog_title,
        button: state.colors.dialog_button,
        button_active: state.colors.dialog_button_active,
        shadow: state.colors.dialog_shadow,
        shadow_on: state.colors.dialog_shadow_on,
    };

    let buttons = &[
        DialogButton { id: ButtonId::Save,   label: "Save"   },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];

    let spec = DialogSpec {
        title: "Style Editor",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Save),
        focus: Some(state.dialog_focus),
    };

    let dialog_rects = draw_dialog(buf, &spec, &ds);
    let content = dialog_rects.content;

    // Split content into board (left) and property pane (right) if wide enough.
    // Property pane is 40 cols wide (fits 16×2 ANSI swatches + labels); board gets the rest.
    const PROP_W: u16 = 40;
    const GAP: u16 = 1;
    let (board_area, prop_area) = if content.width >= PROP_W + GAP + 20 {
        let board_w = content.width.saturating_sub(PROP_W + GAP);
        let board = Rect::new(content.x, content.y, board_w, content.height);
        let prop = Rect::new(content.x + board_w + GAP, content.y, PROP_W, content.height);
        (board, Some(prop))
    } else {
        (content, None)
    };

    // Styles for group headers and row highlight.
    let header_style = state.colors.dialog_title
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let normal_style = state.colors.dialog;

    let active_style = state.colors.dialog_button_active
        .add_modifier(Modifier::BOLD);

    // Build an ordered list of visual lines: None = group header, Some(idx) = selector row.
    // Tag each with a display string (&str from static data).
    // Simultaneously record which visual-line index holds the active selector.
    let mut visual_lines: Vec<(Option<usize>, &str)> = Vec::new();
    let mut active_line_idx: usize = 0;
    let mut g: usize = 0;
    for (group_label, selectors) in SELECTOR_GROUPS {
        visual_lines.push((None, group_label));
        for sel in *selectors {
            if g == ed.active {
                active_line_idx = visual_lines.len();
            }
            visual_lines.push((Some(g), sel));
            g += 1;
        }
    }

    // Compute stateless auto-follow scroll so the active line is always visible.
    let total_lines = visual_lines.len();
    let visible_rows = board_area.height as usize;
    let max_scroll = total_lines.saturating_sub(visible_rows);
    // Put active line at the bottom of the visible window if it would be off-screen.
    let scroll = active_line_idx
        .saturating_sub(visible_rows.saturating_sub(1))
        .min(max_scroll);

    // Render only the visible slice; record hit-rects for rendered selector rows.
    let mut samples: Vec<(usize, Rect)> = Vec::new();
    let end = (scroll + visible_rows).min(total_lines);
    for (offset, line) in visual_lines[scroll..end].iter().enumerate() {
        let row_y = board_area.y + offset as u16;
        if row_y >= board_area.bottom() {
            break;
        }
        match line {
            (None, group_label) => {
                // Group header line.
                let hdr = format!(" {}", group_label);
                crate::render::draw_str_clipped(buf, board_area.x, row_y, &hdr, header_style, board_area);
            }
            (Some(idx), sel) => {
                let is_active = *idx == ed.active;
                let label_style = if is_active { active_style } else { normal_style };

                // Fill row background.
                for col in board_area.x..board_area.right() {
                    if let Some(cell) = buf.cell_mut((col, row_y)) {
                        cell.set_symbol(" ").set_style(label_style);
                    }
                }

                // Name column: up to 28 chars.
                let name_w = 28usize;
                let marker = if is_active { ">" } else { " " };
                let name_trunc: String = sel.chars().take(name_w).collect();
                let label = format!("{} {:<width$}", marker, name_trunc, width = name_w);
                crate::render::draw_str_clipped(buf, board_area.x, row_y, &label, label_style, board_area);

                // Sample swatch: render a short styled text after the name.
                let swatch_x = board_area.x + label.chars().count() as u16 + 1;
                let sample_style = style_for_selector(&ed.preview, sel);
                let swatch_text = " Sample ";
                if swatch_x < board_area.right() {
                    let swatch_area = Rect::new(swatch_x, row_y, board_area.right().saturating_sub(swatch_x), 1);
                    crate::render::draw_str_clipped(buf, swatch_x, row_y, swatch_text, sample_style, swatch_area);
                }

                // Record the full row rect as the hit-rect for this selector.
                let row_rect = Rect::new(board_area.x, row_y, board_area.width, 1);
                samples.push((*idx, row_rect));
            }
        }
    }

    // ── Property pane ─────────────────────────────────────────────────────────
    //
    // Layout (rows within the prop pane, relative to prop.y):
    //   0: selector name header
    //   1: "fg: <current_value>"
    //   2: 16 ANSI swatch cells (2 chars each) + default cell
    //   3: gap
    //   4: "bg: <current_value>"
    //   5: 16 ANSI swatch cells + default cell
    //   6: gap
    //   7: MRU row (shared hex-color history, up to 16 cells × 2 chars)
    //   8: custom hex entry "# <buf>"
    //   9: gap
    //  10: attribute chips [B] [I] [U] [dim] [rev]

    let mut attr_chips: Vec<(AttrKind, Rect)> = Vec::new();
    let mut fg_swatches: Vec<Rect> = Vec::new();
    let mut bg_swatches: Vec<Rect> = Vec::new();
    let mut mru_rects: Vec<Rect> = Vec::new();
    let mut custom_rect: Option<Rect> = None;

    if let Some(prop) = prop_area {
        // Clear the property pane background.
        for py in prop.y..prop.bottom() {
            for px in prop.x..prop.right() {
                if let Some(cell) = buf.cell_mut((px, py)) {
                    cell.set_symbol(" ").set_style(normal_style);
                }
            }
        }

        // Row 0: selector name header.
        let sel_name = ed.selectors[ed.active];
        let trunc: String = sel_name.chars().take(PROP_W as usize).collect();
        crate::render::draw_str_clipped(buf, prop.x, prop.y, &format!(" {}", trunc), header_style, prop);

        // Look up the active Decl (may be absent if user hasn't edited this selector).
        let active_decl = ed.doc.colors.selectors.get(sel_name);
        let fg_val = active_decl.and_then(|d| d.fg.as_deref()).unwrap_or("default");
        let bg_val = active_decl.and_then(|d| d.bg.as_deref()).unwrap_or("default");

        // Row 1: fg label.
        if prop.height > 1 {
            let fg_focused = ed.focus == StyleFocus::Fg;
            let fg_lbl_style = if fg_focused { active_style } else { normal_style };
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 1,
                &format!(" fg: {}", fg_val), fg_lbl_style, prop,
            );
        }

        // Row 2: fg swatch row (16 ANSI + default).
        if prop.height > 2 {
            let show_fg_cursor = ed.focus == StyleFocus::Fg;
            draw_swatch_row(buf, prop, prop.y + 2, fg_val, &mut fg_swatches, normal_style, active_style, show_fg_cursor, ed.swatch_cursor);
        }

        // Row 4: bg label.
        if prop.height > 4 {
            let bg_focused = ed.focus == StyleFocus::Bg;
            let bg_lbl_style = if bg_focused { active_style } else { normal_style };
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 4,
                &format!(" bg: {}", bg_val), bg_lbl_style, prop,
            );
        }

        // Row 5: bg swatch row.
        if prop.height > 5 {
            let show_bg_cursor = ed.focus == StyleFocus::Bg;
            draw_swatch_row(buf, prop, prop.y + 5, bg_val, &mut bg_swatches, normal_style, active_style, show_bg_cursor, ed.swatch_cursor);
        }

        // Row 7: MRU row (shared across fg/bg).
        if prop.height > 7 && !ed.mru.is_empty() {
            let mru_y = prop.y + 7;
            let mut mru_x = prop.x + 1;
            for hex in &ed.mru {
                if mru_x + 2 > prop.right() {
                    break;
                }
                let color = crate::colors::parse_hex_color(hex)
                    .map(|c| Style::new().bg(c))
                    .unwrap_or(normal_style);
                for dx in 0..2u16 {
                    if let Some(cell) = buf.cell_mut((mru_x + dx, mru_y)) {
                        cell.set_symbol(" ").set_style(color);
                    }
                }
                mru_rects.push(Rect::new(mru_x, mru_y, 2, 1));
                mru_x += 2;
            }
        }

        // Row 8: custom hex entry.
        if prop.height > 8 {
            let custom_y = prop.y + 8;
            let custom_focused = ed.focus == StyleFocus::Custom;
            let cstyle = if custom_focused { active_style } else { normal_style };
            let prefix = " # ";
            let prefix_w = prefix.len() as u16;
            let max_buf_w = prop.right().saturating_sub(prop.x + prefix_w) as usize;
            let buf_display: String = ed.custom_buf.chars().take(max_buf_w).collect();
            let custom_text = format!("{}{}", prefix, buf_display);
            crate::render::draw_str_clipped(buf, prop.x, custom_y, &custom_text, cstyle, prop);
            // Record the rect of the editable portion.
            let field_w = (buf_display.chars().count() as u16).max(1);
            custom_rect = Some(Rect::new(prop.x + prefix_w, custom_y, field_w, 1));
        }

        // Row 10: attribute chips.
        if prop.height > 10 {
            let chip_y = prop.y + 10;
            let mut chip_x = prop.x + 1;
            let prop_focused = ed.focus == StyleFocus::Attrs;
            let label_style = if prop_focused { active_style } else { normal_style };

            for (ci, (kind, label)) in ATTR_KINDS.iter().enumerate() {
                let flag_on = active_decl
                    .and_then(|d| match kind {
                        AttrKind::Bold      => d.bold,
                        AttrKind::Italic    => d.italic,
                        AttrKind::Underline => d.underline,
                        AttrKind::Dim       => d.dim,
                        AttrKind::Reversed  => d.reversed,
                    })
                    .unwrap_or(false);

                let is_chip_cursor = prop_focused && ci == ed.attr_cursor;

                let chip_style = if flag_on {
                    active_style
                } else if is_chip_cursor {
                    label_style
                } else {
                    normal_style
                };

                let chip_text = label.trim_end();
                let chip_w = chip_text.chars().count() as u16;

                if chip_x + chip_w <= prop.right() {
                    let chip_rect = Rect::new(chip_x, chip_y, chip_w, 1);
                    attr_chips.push((*kind, chip_rect));
                    crate::render::draw_str_clipped(buf, chip_x, chip_y, chip_text, chip_style, prop);
                    chip_x += chip_w + 1;
                }
            }
        }
    }

    Some(StyleEditorRects { samples, attr_chips, dialog: dialog_rects, fg_swatches, bg_swatches, mru_rects, custom_rect })
}

// ── draw_swatch_row ───────────────────────────────────────────────────────────

/// Draw a row of 16 ANSI color swatches (2 chars each) + a 1-char "default" cell.
///
/// Each ANSI cell is filled with the ANSI color as background; the cell matching
/// `current_val` is highlighted with a `▸` marker.  The "d" default cell uses
/// `active_style` when selected.  Always pushes exactly 17 rects into `rects`
/// (indices 0–15 = ANSI colors, 16 = default); out-of-bounds cells get a
/// zero-width rect so Task 6 mouse hit-testing skips them cleanly.
///
/// When `show_cursor` is true, the cell at `swatch_cursor` gets an underline
/// to indicate keyboard-navigation position.
fn draw_swatch_row(
    buf: &mut Buffer,
    prop: Rect,
    row_y: u16,
    current_val: &str,
    rects: &mut Vec<Rect>,
    normal_style: Style,
    active_style: Style,
    show_cursor: bool,
    swatch_cursor: usize,
) {
    let mut x = prop.x + 1;

    for (idx, name) in crate::style_mru::ANSI_NAMES.iter().enumerate() {
        if x + 2 <= prop.right() {
            let is_selected = current_val == *name;
            let is_cursor = show_cursor && swatch_cursor == idx;
            let color = crate::colors::parse_named_color(name).unwrap_or(Color::Reset);
            let mut cell_style = if is_selected {
                Style::new().bg(color).fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::new().bg(color)
            };
            if is_cursor {
                cell_style = cell_style.add_modifier(Modifier::UNDERLINED);
            }
            let sym0 = if is_selected { "▸" } else { " " };
            if let Some(cell) = buf.cell_mut((x, row_y)) { cell.set_symbol(sym0).set_style(cell_style); }
            if let Some(cell) = buf.cell_mut((x + 1, row_y)) { cell.set_symbol(" ").set_style(cell_style); }
            rects.push(Rect::new(x, row_y, 2, 1));
            x += 2;
        } else {
            rects.push(Rect::new(prop.right(), row_y, 0, 1));
        }
    }

    // Default cell (1 char); index == ANSI_NAMES.len() == 16.
    if x + 1 <= prop.right() {
        let is_selected = current_val == "default";
        let is_cursor = show_cursor && swatch_cursor == crate::style_mru::ANSI_NAMES.len();
        let mut dflt_style = if is_selected { active_style } else { normal_style };
        if is_cursor {
            dflt_style = dflt_style.add_modifier(Modifier::UNDERLINED);
        }
        if let Some(cell) = buf.cell_mut((x, row_y)) { cell.set_symbol("d").set_style(dflt_style); }
        rects.push(Rect::new(x, row_y, 1, 1));
    } else {
        rects.push(Rect::new(prop.right(), row_y, 0, 1));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::buffer::Buffer;

    #[test]
    fn style_editor_board_renders_samples_and_highlights_active() {
        let mut s = AppState::default();
        crate::input::open_style_editor(&mut s);
        // Use a large area so all selectors fit and get drawn.
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        let rects = draw_style_editor(&s, area, &mut buf).expect("drawn");
        assert!(!rects.samples.is_empty(), "samples have hit-rects");
        // The active selector's sample rect maps to index 0.
        assert!(rects.samples.iter().any(|(i, _)| *i == 0));

        // Board order must match ed.selectors order: every selector has exactly
        // one sample at its own index (proves board order == ed.selectors).
        let ed = s.style_editor.as_ref().unwrap();
        let mut idxs: Vec<usize> = rects.samples.iter().map(|(i, _)| *i).collect();
        idxs.sort_unstable();
        assert_eq!(
            idxs,
            (0..ed.selectors.len()).collect::<Vec<_>>(),
            "every selector has exactly one sample at its own index (board order == ed.selectors)"
        );
    }

    #[test]
    fn board_scrolls_to_keep_active_visible() {
        let mut s = AppState::default();
        crate::input::open_style_editor(&mut s);
        let n = s.style_editor.as_ref().unwrap().selectors.len();
        s.style_editor.as_mut().unwrap().active = n - 1; // last selector
        // Small area that cannot show all selectors at once:
        let area = Rect::new(0, 0, 90, 18);
        let mut buf = Buffer::empty(area);
        let rects = draw_style_editor(&s, area, &mut buf).expect("drawn");
        assert!(rects.samples.iter().any(|(i, _)| *i == n - 1),
            "the active (last) selector must be rendered with a hit-rect even on a short board");
    }

    #[test]
    fn style_editor_noop_when_closed() {
        let s = AppState::default(); // style_editor = None
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        let result = draw_style_editor(&s, area, &mut buf);
        assert!(result.is_none());
    }

    #[test]
    fn style_editor_swatch_rects_populated() {
        let mut s = AppState::default();
        // Use a non-existent user_dir so load_mru returns empty regardless of disk state.
        s.config.user_dir = std::path::PathBuf::from("/tmp/babelmap-test-empty-mru-dir");
        crate::input::open_style_editor(&mut s);
        // Wide enough to display the property pane (needs >= 61 content cols).
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        let rects = draw_style_editor(&s, area, &mut buf).expect("drawn");

        // Both fg and bg swatch rows must have exactly 17 rects (16 ANSI + default).
        assert_eq!(rects.fg_swatches.len(), 17,
            "fg_swatches: expected 17 rects (16 ANSI + default)");
        assert_eq!(rects.bg_swatches.len(), 17,
            "bg_swatches: expected 17 rects (16 ANSI + default)");

        // Custom rect must be Some (custom field is always rendered when prop visible).
        assert!(rects.custom_rect.is_some(), "custom_rect should be Some");

        // MRU is empty initially, so no MRU rects.
        assert!(rects.mru_rects.is_empty(), "no MRU entries on fresh open");
    }
}
