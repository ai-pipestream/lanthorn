//! Hotkey dialog overlay — draw_hotkey_dialog.
//!
//! Renders a centered bordered box over the terminal area listing the hotkey
//! groups from state.hotkeys. Each group has a title and a list of
//! "KEY  label" rows. A footer shows how to close the dialog.
//! Only drawn when state.hotkey_dialog == true.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::render::{draw_str_clipped, put_str};
use crate::state::AppState;

// ── draw_hotkey_dialog ────────────────────────────────────────────────────────

/// Render the hotkey dialog overlay onto `buf` using the full terminal `area`.
///
/// The overlay is a centered bordered panel rendered via draw_dialog (opaque
/// background fixes command-panel bleed, #17). Groups from state.hotkeys.groups
/// are listed with their title and "KEY  label" rows.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_hotkey_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    if area.height < 6 || area.width < 30 {
        return None;
    }

    // Build rows to determine height.
    let rows = build_rows(state);

    // Target: at most 60 wide; tall enough for rows + chrome overhead.
    let panel_w = area.width.min(60);
    // +2: border top/bottom; button row adds 1 more (draw_dialog subtracts it from content).
    let panel_h = ((rows.len() as u16).saturating_add(2)).min(area.height);
    if panel_w < 20 || panel_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let prefix_label = state.hotkeys.prefix.label();
    let title = format!("Commands ({prefix_label}: close)");

    let spec = DialogSpec {
        title: title.as_str(),
        placement: Placement::Centered { w: panel_w, h: panel_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // ── Render rows ───────────────────────────────────────────────────────────

    let heading_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .patch(state.colors.dialog_title);
    let key_style = state.colors.hotkey_key;
    let label_style = state.colors.dialog;

    for (y, row) in (content.y..).zip(rows.iter()) {
        if y >= content.bottom() {
            break;
        }
        if let Some(stripped) = row.strip_prefix("##") {
            draw_str_clipped(buf, content.x, y, stripped.trim(), heading_style, content);
        } else if row == "---" {
            // blank separator — skip
        } else if let Some((key_part, label_part)) = row.split_once("  ") {
            let kw = key_part.len() as u16;
            put_str(buf, content.x as i32, y as i32, key_part, key_style, content);
            let label_x = content.x + kw + 2;
            if label_x < content.right() {
                draw_str_clipped(buf, label_x, y, label_part, label_style, content);
            }
        } else {
            draw_str_clipped(buf, content.x, y, row, label_style, content);
        }
    }

    Some(rects)
}

/// Build text rows for the dialog panel.
/// Section headings are prefixed with "##"; separators are "---".
fn build_rows(state: &AppState) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();

    for (title, cmds) in &state.hotkeys.groups {
        rows.push(format!("## {title}"));
        for (letter, cmd_str) in cmds {
            let key_label = letter.to_string();
            let first = cmd_str.split_whitespace().next().unwrap_or(cmd_str.as_str());
            let label = crate::slash::find_command(first)
                .map(|c| c.description)
                .unwrap_or(cmd_str.as_str());
            rows.push(format!("{:<3} {}", key_label, label));
        }
        rows.push("---".into());
    }

    rows
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier};

    use super::draw_hotkey_dialog;
    use crate::render::dialog::ButtonId;
    use crate::state::AppState;

    fn buf_text(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in buf.area.y..buf.area.bottom() {
            for x in buf.area.x..buf.area.right() {
                if let Some(cell) = buf.cell((x, y)) {
                    s.push_str(cell.symbol());
                }
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn draw_hotkey_dialog_shows_group_title_and_command() {
        let mut state = AppState::default();
        state.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);
        // The default layout has a "Layout" group
        assert!(text.contains("Layout"), "expected 'Layout' group heading in dialog");
        // The tidy-map command's label (registry description) appears in that group
        assert!(text.contains("tidy"), "expected 'tidy' label in dialog");
    }

    #[test]
    fn draw_hotkey_dialog_shows_authored_letter() {
        let mut state = AppState::default();
        state.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let rects = draw_hotkey_dialog(&state, area, &mut buf).expect("dialog should draw");
        let content = rects.content;

        // The tidy-map row is authored with leader letter 't'; find its row by
        // locating the row whose content-area text contains tidy-map's unique
        // registry description, then check the row's key column is 't'.
        let tidy_desc = crate::slash::find_command("tidy-map")
            .expect("tidy-map command should be registered")
            .description;
        let mut found_tidy_row = false;
        for y in content.y..content.bottom() {
            let mut line = String::new();
            for x in content.x..content.right() {
                if let Some(cell) = buf.cell((x, y)) {
                    line.push_str(cell.symbol());
                }
            }
            if line.contains(tidy_desc) {
                found_tidy_row = true;
                let key_cell = buf.cell((content.x, y)).expect("key cell");
                assert_eq!(key_cell.symbol(), "t", "expected authored letter 't' in the key column of the tidy row");
            }
        }
        assert!(found_tidy_row, "expected a row containing tidy-map's description in the dialog content");

        // The old global-keymap chord label must not appear anywhere.
        let text = buf_text(&buf);
        assert!(!text.contains("^T"), "old chord label '^T' should not appear in the dialog");

        // The Layers group's cycle-layer entry should show its real registry
        // description, not the raw "cycle-layer next" command string.
        let cycle_layer_desc = crate::slash::find_command("cycle-layer")
            .expect("cycle-layer command should be registered")
            .description;
        assert!(text.contains(cycle_layer_desc), "expected cycle-layer's registry description in dialog");
        assert!(!text.contains("cycle-layer next"), "raw command string should not be shown");
    }

    #[test]
    fn draw_hotkey_dialog_shows_close_hint() {
        let mut state = AppState::default();
        state.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);
        // Footer or title should show close hint
        assert!(text.contains("close") || text.contains("q"), "expected close hint");
    }

    #[test]
    fn draw_hotkey_dialog_shows_dialog_chrome() {
        // Render test: dialog shows [X] close button and [Done] button.
        let mut state = AppState::default();
        state.hotkey_dialog = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let rects_opt = draw_hotkey_dialog(&state, area, &mut buf);
        let text = buf_text(&buf);

        assert!(text.contains('✕'), "[X] close button should be visible");
        assert!(text.contains("Done"), "[Done] button should be visible");

        let rects = rects_opt.expect("draw_hotkey_dialog should return DialogRects");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1);
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }

    #[test]
    fn draw_hotkey_dialog_bg_opaque_over_map_cell() {
        // Render test: the hotkey dialog background is OPAQUE over a pre-filled
        // map cell (no bleed — the underlying cell's bg/REVERSED is replaced).
        // This verifies fix for issue #17 (command-panel current-room color bleed).
        let mut state = AppState::default();
        state.hotkey_dialog = true;

        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);

        // Pre-fill a REVERSED cell with Red bg in the center where the dialog will sit.
        // The dialog is centered at ~col 10..70, row 0..40 (panel_w=60).
        // Place the sentinel cell near center.
        let sentinel_col = 40u16;
        let sentinel_row = 20u16;
        if let Some(cell) = buf.cell_mut((sentinel_col, sentinel_row)) {
            cell.set_symbol("M")
                .set_style(ratatui::style::Style::new()
                    .bg(Color::Red)
                    .add_modifier(Modifier::REVERSED));
        }

        draw_hotkey_dialog(&state, area, &mut buf);

        // After drawing, the dialog's opaque fill should have replaced the cell.
        let cell = buf.cell((sentinel_col, sentinel_row)).unwrap();
        assert!(
            !cell.modifier.contains(Modifier::REVERSED),
            "dialog opaque bg must clear REVERSED modifier (no bleed)"
        );
        assert_ne!(
            cell.bg,
            Color::Red,
            "dialog opaque bg must replace the Red map background (no bleed)"
        );
        // And [X] should appear somewhere in the buffer.
        let text = buf_text(&buf);
        assert!(text.contains('✕'), "[X] must be present in the dialog");
    }
}
