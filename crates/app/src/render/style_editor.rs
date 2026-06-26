//! Style-editor preview board overlay.
//!
//! Draws a full-screen modal showing all styleable selectors as labeled
//! samples. Each sample is styled from `ed.preview` so live edits render
//! immediately.  The active row is highlighted. Returns hit-rects for every
//! sample (used by the mouse handler to set `ed.active`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

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
pub struct StyleEditorRects {
    pub samples: Vec<(usize, Rect)>,
    pub attr_chips: Vec<(AttrKind, Rect)>,
    pub dialog: DialogRects,
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

    let modal_w = 72u16.min(area.width.saturating_sub(4));
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
    // Property pane is 24 cols wide with a 1-col gap; board gets the rest.
    const PROP_W: u16 = 24;
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

    // Walk groups and draw samples in the board area.
    let mut samples: Vec<(usize, Rect)> = Vec::new();
    let mut global_idx: usize = 0;
    let mut row_y = board_area.y;

    for (group_label, selectors) in SELECTOR_GROUPS {
        if row_y >= board_area.bottom() {
            break;
        }

        // Group header line.
        let hdr = format!(" {}", group_label);
        crate::render::draw_str_clipped(buf, board_area.x, row_y, &hdr, header_style, board_area);
        row_y += 1;

        for sel in *selectors {
            if row_y >= board_area.bottom() {
                break;
            }

            let is_active = global_idx == ed.active;
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
            samples.push((global_idx, row_rect));

            global_idx += 1;
            row_y += 1;
        }
    }

    // ── Property pane ─────────────────────────────────────────────────────────

    let mut attr_chips: Vec<(AttrKind, Rect)> = Vec::new();

    if let Some(prop) = prop_area {
        // Clear the property pane background.
        for py in prop.y..prop.bottom() {
            for px in prop.x..prop.right() {
                if let Some(cell) = buf.cell_mut((px, py)) {
                    cell.set_symbol(" ").set_style(normal_style);
                }
            }
        }

        let prop_focused = ed.focus == StyleFocus::Attrs;
        let label_style = if prop_focused { active_style } else { normal_style };

        // "Property" header.
        let sel_name = ed.selectors[ed.active];
        let trunc: String = sel_name.chars().take(PROP_W as usize).collect();
        let hdr_text = format!(" {}", trunc);
        crate::render::draw_str_clipped(buf, prop.x, prop.y, &hdr_text, header_style, prop);

        // Look up the active Decl (may be absent if user hasn't edited this selector).
        let active_decl = ed.doc.colors.selectors.get(sel_name);

        // fg / bg rows.
        let fg_str = active_decl.and_then(|d| d.fg.as_deref()).unwrap_or("default");
        let bg_str = active_decl.and_then(|d| d.bg.as_deref()).unwrap_or("default");
        if prop.height > 1 {
            let fg_line = format!(" fg:  {}", fg_str);
            crate::render::draw_str_clipped(buf, prop.x, prop.y + 1, &fg_line, normal_style, prop);
        }
        if prop.height > 2 {
            let bg_line = format!(" bg:  {}", bg_str);
            crate::render::draw_str_clipped(buf, prop.x, prop.y + 2, &bg_line, normal_style, prop);
        }

        // Attribute chips row (row 4 within the prop pane).
        if prop.height > 4 {
            let chip_y = prop.y + 4;
            let mut chip_x = prop.x + 1;

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
                    // Attribute is ON: use active (highlighted) style.
                    active_style
                } else if is_chip_cursor {
                    // Cursor on this chip but not active: use label style (slightly highlighted).
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

    Some(StyleEditorRects { samples, attr_chips, dialog: dialog_rects })
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
    fn style_editor_noop_when_closed() {
        let s = AppState::default(); // style_editor = None
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        let result = draw_style_editor(&s, area, &mut buf);
        assert!(result.is_none());
    }
}
