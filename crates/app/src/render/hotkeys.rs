//! Hotkey dialog overlay — draw_hotkey_dialog.
//!
//! Renders a centered bordered box over the terminal area listing the hotkey
//! groups from state.hotkeys. Each group has a title and a list of
//! "KEY  label" rows. A footer shows how to close the dialog.
//! Only drawn when state.hotkey_dialog == true.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::keymap::Command;
use crate::render::{draw_str_clipped, put_str};
use crate::state::AppState;

// ── draw_hotkey_dialog ────────────────────────────────────────────────────────

/// Render the hotkey dialog overlay onto `buf` using the full terminal `area`.
///
/// The overlay is a centered bordered panel. Groups from state.hotkeys.groups
/// are listed with their title and "KEY  label" rows.
/// Footer shows "Ctrl+K / q: close".
pub fn draw_hotkey_dialog(state: &AppState, area: Rect, buf: &mut Buffer) {
    if area.height < 6 || area.width < 30 {
        return;
    }

    // Center a panel that is at most 60 wide.
    let panel_w = area.width.min(60);
    let panel_x = area.x + (area.width - panel_w) / 2;

    // Build rows to determine height.
    let rows = build_rows(state);
    // +4: top border + title + separator line + bottom border
    let panel_h = (rows.len() as u16 + 4).min(area.height);
    let panel_y = area.y + (area.height - panel_h) / 2;

    let panel = Rect::new(panel_x, panel_y, panel_w, panel_h);

    // Background fill.
    let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    for y in panel.y..panel.bottom() {
        for x in panel.x..panel.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(bg_style);
            }
        }
    }

    // Border.
    let border_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    draw_border(buf, panel, border_style);

    // Title.
    let prefix_label = state.hotkeys.prefix.label();
    let title = format!(" Commands ({prefix_label} / q: close) ");
    let title_x = panel.x + 1 + (panel.width.saturating_sub(2).saturating_sub(title.len() as u16)) / 2;
    draw_str_clipped(buf, title_x, panel.y, &title, border_style, panel);

    // Content area.
    let content_area = Rect::new(
        panel.x + 1,
        panel.y + 1,
        panel.width.saturating_sub(2),
        panel.height.saturating_sub(2),
    );
    let heading_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let key_style = Style::default().fg(Color::Cyan);
    let label_style = Style::default().fg(Color::White);

    let mut y = content_area.y;

    for row in &rows {
        if y >= content_area.bottom() {
            break;
        }
        if let Some(stripped) = row.strip_prefix("##") {
            draw_str_clipped(buf, content_area.x, y, stripped.trim(), heading_style, content_area);
        } else if row == "---" {
            // blank separator — skip
        } else if let Some((key_part, label_part)) = row.split_once("  ") {
            let kw = key_part.len() as u16;
            put_str(buf, content_area.x as i32, y as i32, key_part, key_style, content_area);
            let label_x = content_area.x + kw + 2;
            if label_x < content_area.right() {
                draw_str_clipped(buf, label_x, y, label_part, label_style, content_area);
            }
        } else {
            draw_str_clipped(buf, content_area.x, y, row, label_style, content_area);
        }
        y += 1;
    }
}

/// Build text rows for the dialog panel.
/// Section headings are prefixed with "##"; separators are "---".
fn build_rows(state: &AppState) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();

    for (title, cmds) in &state.hotkeys.groups {
        rows.push(format!("## {title}"));
        for cmd in cmds {
            let key_label = primary_key_label(state, *cmd);
            rows.push(format!("{:<12}  {}", key_label, cmd.label()));
        }
        rows.push("---".into());
    }

    rows
}

/// Find the primary key label for a command in the state's keymap.
fn primary_key_label(state: &AppState, cmd: Command) -> String {
    state.keymap.primary_key(cmd)
        .map(|k| k.label())
        .unwrap_or_else(|| "?".to_string())
}

fn draw_border(buf: &mut Buffer, area: Rect, style: Style) {
    if area.height < 2 || area.width < 2 {
        return;
    }
    let (x0, y0) = (area.x, area.y);
    let (x1, y1) = (area.right() - 1, area.bottom() - 1);

    draw_char(buf, x0, y0, '\u{256d}', style);
    draw_char(buf, x1, y0, '\u{256e}', style);
    draw_char(buf, x0, y1, '\u{2570}', style);
    draw_char(buf, x1, y1, '\u{256f}', style);

    for x in (x0 + 1)..x1 {
        draw_char(buf, x, y0, '\u{2500}', style);
        draw_char(buf, x, y1, '\u{2500}', style);
    }
    for y in (y0 + 1)..y1 {
        draw_char(buf, x0, y, '\u{2502}', style);
        draw_char(buf, x1, y, '\u{2502}', style);
    }
}

fn draw_char(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        let mut s = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::draw_hotkey_dialog;
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
        // The retidy command's label appears in that group
        assert!(text.contains("retidy"), "expected 'retidy' label in dialog");
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
}
