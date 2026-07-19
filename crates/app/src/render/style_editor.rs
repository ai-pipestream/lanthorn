//! Style-editor preview board overlay.
//!
//! Draws a full-screen modal showing all styleable selectors as labeled
//! samples. Each sample is styled from `ed.preview` so live edits render
//! immediately.  The active row is highlighted. Returns hit-rects for every
//! sample (used by the mouse handler to set `ed.active`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::input::{AttrKind, is_bordered_selector};
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::{AppState, BorderZone, StyleFocus};
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
    /// 8 border zone rects (BorderZone, Rect) for mouse hit-testing.
    /// Only populated when the active selector is a bordered selector.
    pub border_zones: Vec<(BorderZone, Rect)>,
    pub border_type_prev: Option<Rect>,
    pub border_type_next: Option<Rect>,
    pub border_header: Option<Rect>,
    pub border_shadow: Option<Rect>,
}

/// Draw the style-editor full-screen overlay onto `buf`.
///
/// Returns `Some(StyleEditorRects)` when drawn, `None` when
/// `state.overlays.style_editor` is `None` or the area is too small.
pub fn draw_style_editor(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<StyleEditorRects> {
    let Some(ed) = &state.overlays.style_editor else { return None };

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
    let ds = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Save,     label: "Save Global Style" },
        DialogButton { id: ButtonId::SaveGame, label: "Save Game Style"   },
        DialogButton { id: ButtonId::Cancel,   label: "Cancel"            },
    ];

    let spec = DialogSpec {
        title: "Style Editor",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Save),
        // Highlight a footer button only once focus has Tabbed onto the button row;
        // otherwise just the default (Save) is marked, and the body widgets own focus.
        focus: if ed.focus == StyleFocus::Buttons { Some(state.overlays.dialog_focus) } else { None },
        field: None,
    };

    let dialog_rects = draw_dialog(buf, area, &spec, &ds);
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

    // When the list overflows, reserve the board's rightmost column as a scrollbar gutter.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total_lines, visible_rows) && board_area.width >= 2;
    let draw_area = if scrollbar_visible {
        Rect::new(board_area.x, board_area.y, board_area.width.saturating_sub(1), board_area.height)
    } else {
        board_area
    };

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
                crate::render::draw_str_clipped(buf, board_area.x, row_y, &hdr, header_style, draw_area);
            }
            (Some(idx), sel) => {
                let is_active = *idx == ed.active;
                let label_style = if is_active { active_style } else { normal_style };

                // Fill row background (stop before the scrollbar gutter when visible).
                for col in board_area.x..draw_area.right() {
                    if let Some(cell) = buf.cell_mut((col, row_y)) {
                        cell.set_symbol(" ").set_style(label_style);
                    }
                }

                // Name column: up to 28 chars.
                let name_w = 28usize;
                let marker = if is_active { ">" } else { " " };
                let name_trunc: String = sel.chars().take(name_w).collect();
                let label = format!("{} {:<width$}", marker, name_trunc, width = name_w);
                crate::render::draw_str_clipped(buf, board_area.x, row_y, &label, label_style, draw_area);

                // Sample swatch: render a short styled text after the name.
                let swatch_x = board_area.x + label.chars().count() as u16 + 1;
                let sample_style = style_for_selector(&ed.preview, sel);
                let swatch_text = " Sample ";
                if swatch_x < draw_area.right() {
                    let swatch_area = Rect::new(swatch_x, row_y, draw_area.right().saturating_sub(swatch_x), 1);
                    crate::render::draw_str_clipped(buf, swatch_x, row_y, swatch_text, sample_style, swatch_area);
                }

                // Record the full row rect as the hit-rect for this selector.
                let row_rect = Rect::new(board_area.x, row_y, board_area.width, 1);
                samples.push((*idx, row_rect));
            }
        }
    }

    // ── Scrollbar (only when the selector list overflows the board height) ────
    if scrollbar_visible {
        let sb_area = Rect::new(board_area.right().saturating_sub(1), board_area.y, 1, board_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total_lines,
            visible_rows,
            scroll,
            state.colors.scrollbar,
        );
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
    let mut border_zones: Vec<(BorderZone, Rect)> = Vec::new();
    let mut border_type_prev: Option<Rect> = None;
    let mut border_type_next: Option<Rect> = None;
    let mut border_header: Option<Rect> = None;
    let mut border_shadow: Option<Rect> = None;

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
            let fg_mark = if !ed.color_target { "\u{25b8}" } else { " " }; // ▸ marks active target
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 1,
                &format!("{}fg: {}", fg_mark, fg_val), fg_lbl_style, prop,
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
            let bg_mark = if ed.color_target { "\u{25b8}" } else { " " }; // ▸ marks active target
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 4,
                &format!("{}bg: {}", bg_mark, bg_val), bg_lbl_style, prop,
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
            let prefix = if ed.color_target { " hex \u{2192}bg " } else { " hex \u{2192}fg " };
            let prefix_w = prefix.chars().count() as u16;
            let max_buf_w = prop.right().saturating_sub(prop.x + prefix_w + 4) as usize; // 4 = "[ " + " ]"
            let buf_display: String = ed.custom_buf.chars().take(max_buf_w).collect();
            let cursor = if custom_focused { "\u{258f}" } else { "" }; // ▏
            let field = format!("[ {}{} ]", buf_display, cursor);
            let custom_text = format!("{}{}", prefix, field);
            crate::render::draw_str_clipped(buf, prop.x, custom_y, &custom_text, cstyle, prop);
            // Hit-rect covers the bracketed field (interior + brackets).
            let field_w = field.chars().count() as u16;
            custom_rect = Some(Rect::new(prop.x + prefix_w, custom_y, field_w, 1));
        }

        // Row 10: attribute chips.
        if prop.height > 10 {
            let chip_y = prop.y + 10;
            let mut chip_x = prop.x + 1;
            let prop_focused = ed.focus == StyleFocus::Attrs;

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

                // flag_on drives the on/off background; is_chip_cursor adds UNDERLINED
                // so the cursor position is visible even on an off (flag=false) chip.
                let mut chip_style = if flag_on { active_style } else { normal_style };
                if is_chip_cursor {
                    chip_style = chip_style.add_modifier(Modifier::UNDERLINED);
                }

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

        // ── Border sub-editor (rows 12–19) ───────────────────────────────────
        // Only shown for the six bordered selectors.
        let active_sel = ed.selectors[ed.active];
        if is_bordered_selector(active_sel) && prop.height > 13 {
            let border_focused = ed.focus == StyleFocus::Border;
            let section_style = if border_focused { active_style } else { normal_style };

            // Determine the current border style name from the Decl.
            let style_name = active_decl
                .and_then(|d| d.style.as_deref())
                .unwrap_or("single");

            // Row 12: type cycle row  ` type: ◀ <name> ▶`
            if prop.height > 12 {
                let type_y = prop.y + 12;
                let prefix = " type: ";
                let prefix_w = prefix.chars().count() as u16;
                let arrow_l = "◀";
                let arrow_r = "▶";
                let name_display = style_name;
                let full_text = format!("{}{} {} {}", prefix, arrow_l, name_display, arrow_r);
                crate::render::draw_str_clipped(buf, prop.x, type_y, &full_text, section_style, prop);

                // Hit-rects for the arrows.
                let prev_x = prop.x + prefix_w;
                border_type_prev = Some(Rect::new(prev_x, type_y, 1, 1));
                let next_x = prev_x + 1 + 1 + name_display.chars().count() as u16 + 1;
                border_type_next = Some(Rect::new(next_x, type_y, 1, 1));
            }

            // Rows 14-16: the 8-zone border box (3 rows).
            // Zone cell layout (x offsets from prop.x):
            //   col0 = prop.x+1 (width 3): TL / Left / BL
            //   col1 = prop.x+5 (width 3): Top / (empty) / Bottom
            //   col2 = prop.x+9 (width 3): TR / Right / BR
            let col0 = prop.x + 1;
            let col1 = prop.x + 5;
            let col2 = prop.x + 9;

            // Helper: get the display glyph for a zone.
            let zone_glyph = |zone: BorderZone| -> String {
                // Override from decl takes priority.
                let override_g: Option<&str> = active_decl.and_then(|d| match zone {
                    BorderZone::Top    => d.glyph_top.as_deref(),
                    BorderZone::Bottom => d.glyph_bottom.as_deref(),
                    BorderZone::Left   => d.glyph_left.as_deref(),
                    BorderZone::Right  => d.glyph_right.as_deref(),
                    BorderZone::Tl     => d.glyph_tl.as_deref(),
                    BorderZone::Tr     => d.glyph_tr.as_deref(),
                    BorderZone::Bl     => d.glyph_bl.as_deref(),
                    BorderZone::Br     => d.glyph_br.as_deref(),
                });
                if let Some(g) = override_g {
                    g.to_string()
                } else {
                    // Default glyph based on style.
                    let default: &'static str = match style_name {
                        "double" => match zone {
                            BorderZone::Top | BorderZone::Bottom => "═",
                            BorderZone::Left | BorderZone::Right => "║",
                            BorderZone::Tl => "╔", BorderZone::Tr => "╗",
                            BorderZone::Bl => "╚", BorderZone::Br => "╝",
                        },
                        "thick" => match zone {
                            BorderZone::Top | BorderZone::Bottom => "━",
                            BorderZone::Left | BorderZone::Right => "┃",
                            BorderZone::Tl => "┏", BorderZone::Tr => "┓",
                            BorderZone::Bl => "┗", BorderZone::Br => "┛",
                        },
                        "rounded" => match zone {
                            BorderZone::Top | BorderZone::Bottom => "─",
                            BorderZone::Left | BorderZone::Right => "│",
                            BorderZone::Tl => "╭", BorderZone::Tr => "╮",
                            BorderZone::Bl => "╰", BorderZone::Br => "╯",
                        },
                        "none" => " ",
                        _ => match zone { // single, unknown
                            BorderZone::Top | BorderZone::Bottom => "─",
                            BorderZone::Left | BorderZone::Right => "│",
                            BorderZone::Tl => "┌", BorderZone::Tr => "┐",
                            BorderZone::Bl => "└", BorderZone::Br => "┘",
                        },
                    };
                    default.to_string()
                }
            };

            // Render all 8 zone cells across 3 rows.
            let zone_rows: &[(u16, &[(BorderZone, u16)])] = &[
                (14, &[(BorderZone::Tl, col0), (BorderZone::Top, col1), (BorderZone::Tr, col2)]),
                (15, &[(BorderZone::Left, col0), (BorderZone::Right, col2)]),
                (16, &[(BorderZone::Bl, col0), (BorderZone::Bottom, col1), (BorderZone::Br, col2)]),
            ];

            for (row_offset, cells) in zone_rows {
                let zy = prop.y + row_offset;
                if zy >= prop.bottom() { break; }
                for (zone, zx) in *cells {
                    let zone = *zone;
                    let zx = *zx;
                    // Determine zone index for cursor comparison.
                    let zone_idx = match zone {
                        BorderZone::Tl => 0, BorderZone::Top => 1, BorderZone::Tr => 2,
                        BorderZone::Left => 3, BorderZone::Right => 4,
                        BorderZone::Bl => 5, BorderZone::Bottom => 6, BorderZone::Br => 7,
                    };
                    let is_cursor = border_focused && ed.border_zone == zone_idx;
                    let has_override = active_decl.is_some_and(|d| match zone {
                        BorderZone::Top    => d.glyph_top.is_some(),
                        BorderZone::Bottom => d.glyph_bottom.is_some(),
                        BorderZone::Left   => d.glyph_left.is_some(),
                        BorderZone::Right  => d.glyph_right.is_some(),
                        BorderZone::Tl     => d.glyph_tl.is_some(),
                        BorderZone::Tr     => d.glyph_tr.is_some(),
                        BorderZone::Bl     => d.glyph_bl.is_some(),
                        BorderZone::Br     => d.glyph_br.is_some(),
                    });

                    let base_style = if is_cursor {
                        active_style
                    } else if has_override {
                        normal_style.add_modifier(Modifier::BOLD)
                    } else {
                        normal_style
                    };

                    let glyph = zone_glyph(zone);
                    // Draw 3-cell zone: space + glyph + space
                    let cell_rect = Rect::new(zx, zy, 3, 1);
                    if zx + 3 <= prop.right() {
                        let cell_text = format!(" {} ", glyph);
                        crate::render::draw_str_clipped(buf, zx, zy, &cell_text, base_style, prop);
                        border_zones.push((zone, cell_rect));
                    }
                }
            }

            // Row 18-19: header/shadow toggle chips.
            // header applies to pane selectors (all bordered except dialog);
            // shadow applies to dialog only.
            let is_dialog = active_sel == "dialog";
            let show_header = is_bordered_selector(active_sel) && !is_dialog;
            let show_shadow = is_dialog;

            if prop.height > 18 {
                let toggle_y = prop.y + 18;
                let hdr_on = active_decl.and_then(|d| d.header).unwrap_or(false);
                let shd_on = active_decl.and_then(|d| d.shadow).unwrap_or(false);

                let hdr_text = "[header]";
                let shd_text = "[shadow]";
                let hdr_w = hdr_text.chars().count() as u16;
                let shd_w = shd_text.chars().count() as u16;

                let hdr_x = prop.x + 1;
                let shd_x = hdr_x + hdr_w + 1;

                let hdr_style = if hdr_on { active_style } else { normal_style };
                let shd_style = if shd_on { active_style } else { normal_style };

                if show_header && hdr_x + hdr_w <= prop.right() {
                    crate::render::draw_str_clipped(buf, hdr_x, toggle_y, hdr_text, hdr_style, prop);
                    border_header = Some(Rect::new(hdr_x, toggle_y, hdr_w, 1));
                }
                if show_shadow && shd_x + shd_w <= prop.right() {
                    crate::render::draw_str_clipped(buf, shd_x, toggle_y, shd_text, shd_style, prop);
                    border_shadow = Some(Rect::new(shd_x, toggle_y, shd_w, 1));
                }
            }
        }
    }

    Some(StyleEditorRects { samples, attr_chips, dialog: dialog_rects, fg_swatches, bg_swatches, mru_rects, custom_rect, border_zones, border_type_prev, border_type_next, border_header, border_shadow })
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
    if x < prop.right() {
        let is_selected = current_val == "default" || current_val == "reset";
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
        crate::input::open_style_editor_hermetic(&mut s);
        // Use a large area so all selectors fit and get drawn (must stay
        // ahead of SELECTOR_GROUPS growth — bumped for SQ-0348's 5 new
        // story-list selectors).
        let area = Rect::new(0, 0, 120, 90);
        let mut buf = Buffer::empty(area);
        let rects = draw_style_editor(&s, area, &mut buf).expect("drawn");
        assert!(!rects.samples.is_empty(), "samples have hit-rects");
        // The active selector's sample rect maps to index 0.
        assert!(rects.samples.iter().any(|(i, _)| *i == 0));

        // Board order must match ed.selectors order: every selector has exactly
        // one sample at its own index (proves board order == ed.selectors).
        let ed = s.overlays.style_editor.as_ref().unwrap();
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
        crate::input::open_style_editor_hermetic(&mut s);
        let n = s.overlays.style_editor.as_ref().unwrap().selectors.len();
        s.overlays.style_editor.as_mut().unwrap().active = n - 1; // last selector
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
        crate::input::open_style_editor_hermetic(&mut s);
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

    #[test]
    fn header_shadow_chips_are_selector_appropriate() {
        let area = Rect::new(0, 0, 120, 40);

        // PANE selector (map_border): header chip present, shadow chip absent.
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        {
            let ed = s.overlays.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|&sel| sel == "map_border")
                .expect("map_border exists");
        }
        let mut buf = Buffer::empty(area);
        let rects = draw_style_editor(&s, area, &mut buf).expect("drawn");
        assert!(rects.border_header.is_some(), "pane selector shows header chip");
        assert!(rects.border_shadow.is_none(), "pane selector hides shadow chip");

        // dialog selector: shadow chip present, header chip absent.
        let mut s2 = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s2);
        {
            let ed = s2.overlays.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|&sel| sel == "dialog")
                .expect("dialog exists");
        }
        let mut buf2 = Buffer::empty(area);
        let rects2 = draw_style_editor(&s2, area, &mut buf2).expect("drawn");
        assert!(rects2.border_shadow.is_some(), "dialog shows shadow chip");
        assert!(rects2.border_header.is_none(), "dialog hides header chip");
    }

    #[test]
    fn selector_list_draws_scrollbar_when_overflowing() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        // A short area forces the selector list (~39+ visual lines) to overflow.
        let area = Rect::new(0, 0, 120, 12);
        let mut buf = Buffer::empty(area);
        let _ = draw_style_editor(&s, area, &mut buf);
        // The scrollbar thumb glyph (█) must appear somewhere; it is unambiguous
        // because border lines use │/─ and selector text never contains █.
        let mut drew = false;
        for y in 0..area.height {
            for x in 0..area.width {
                if buf[(x, y)].symbol() == "█" {
                    drew = true;
                }
            }
        }
        assert!(drew, "selector list must draw a scrollbar thumb when the list overflows");
    }

    #[test]
    fn selector_list_no_scrollbar_when_not_overflowing() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        // A very tall area fits all selectors; should not panic and should not draw a thumb.
        let area = Rect::new(0, 0, 120, 100);
        let mut buf = Buffer::empty(area);
        let result = draw_style_editor(&s, area, &mut buf);
        assert!(result.is_some(), "draw_style_editor must succeed on a large area");
        let mut drew = false;
        for y in 0..area.height {
            for x in 0..area.width {
                if buf[(x, y)].symbol() == "█" {
                    drew = true;
                }
            }
        }
        assert!(!drew, "no scrollbar thumb expected when all selectors fit in the board area");
    }

    #[test]
    fn property_pane_shows_fg_bg_target_indicator() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        let area = Rect::new(0, 0, 120, 60);

        // Default target is fg.
        let mut buf = Buffer::empty(area);
        let _ = draw_style_editor(&s, area, &mut buf);
        let fg_text: String = buf.content().iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(fg_text.contains("\u{2192}fg"), "custom row tags the fg target by default");

        // Switch target to bg.
        s.overlays.style_editor.as_mut().unwrap().color_target = true;
        let mut buf2 = Buffer::empty(area);
        let _ = draw_style_editor(&s, area, &mut buf2);
        let bg_text: String = buf2.content().iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(bg_text.contains("\u{2192}bg"), "custom row tags the bg target when color_target is bg");
    }

    #[test]
    fn custom_hex_renders_bracketed_box_with_cursor_when_focused() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        {
            let ed = s.overlays.style_editor.as_mut().unwrap();
            ed.focus = crate::state::StyleFocus::Custom;
            ed.custom_buf = "#ab12cd".to_string();
        }
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        let _ = draw_style_editor(&s, area, &mut buf);
        let text: String = buf.content().iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains("[ #ab12cd"), "hex field is drawn as a bracketed box");
        assert!(text.contains("\u{258f}"), "a cursor glyph shows when the custom field is focused");
    }

    #[test]
    fn mini_box_renders_actual_override_glyph() {
        let area = Rect::new(0, 0, 120, 40);
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        {
            let ed = s.overlays.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|&sel| sel == "map_border")
                .expect("map_border exists");
            let decl = ed.doc.colors.selectors.entry("map_border".to_string()).or_default();
            decl.glyph_top = Some("═".into());
        }
        let mut buf = Buffer::empty(area);
        let _ = draw_style_editor(&s, area, &mut buf);
        let mut found = false;
        for y in 0..area.height {
            for x in 0..area.width {
                if buf[(x, y)].symbol() == "═" { found = true; }
            }
        }
        assert!(found, "mini border-box must render the actual override glyph, not a placeholder");
    }
}
