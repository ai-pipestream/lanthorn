//! Full-screen help overlay — lists all key bindings grouped by context.
//!
//! `draw_help` renders a centered bordered box over the terminal area showing
//! every binding from the `KeyMap`, grouped under "Global", "Map", and
//! "Tidy-anim" headings. Each row shows all keys bound to a command and the
//! command's human label. Toggled by `Action::ToggleHelp` (F1 / `?`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::keymap::{Command, Context, KeyMap, ALL_COMMANDS};
use crate::render::{draw_str_clipped, put_str};

// ── draw_help ─────────────────────────────────────────────────────────────────

/// Render the help overlay onto `buf` using the full terminal `area`.
///
/// The overlay is a centered bordered panel. Bindings are grouped by context
/// (Global, Map, Tidy-anim). Each row: "  KEYS   label".
pub fn draw_help(keymap: &KeyMap, area: Rect, buf: &mut Buffer) {
    if area.height < 6 || area.width < 40 {
        return;
    }

    // Center a panel that is at most 60 wide and as tall as needed.
    let panel_w = area.width.min(66);
    let panel_x = area.x + (area.width - panel_w) / 2;

    // Collect rows per context.
    let rows: Vec<String> = build_rows(keymap);
    // +4: top border + title + separator + bottom border
    let panel_h = (rows.len() as u16 + 7).min(area.height);
    let panel_y = area.y + (area.height - panel_h) / 2;

    let panel = Rect::new(panel_x, panel_y, panel_w, panel_h);

    // Background fill
    let bg_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    for y in panel.y..panel.bottom() {
        for x in panel.x..panel.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(bg_style);
            }
        }
    }

    // Border
    let border_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    draw_border(buf, panel, border_style);

    // Title
    let title = " Key Bindings (Esc / F1 to close) ";
    let title_x = panel.x + 1 + (panel.inner_width().saturating_sub(title.len() as u16)) / 2;
    draw_str_clipped(buf, title_x, panel.y, title, border_style, panel);

    // Content
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
            // Section heading
            draw_str_clipped(buf, content_area.x, y, stripped.trim(), heading_style, content_area);
        } else if row == "---" {
            // Blank separator
        } else {
            // Binding row: "KEYS   label"
            if let Some((keys_part, label_part)) = row.split_once("  ") {
                let kw = keys_part.len() as u16;
                put_str(buf, content_area.x as i32, y as i32, keys_part, key_style, content_area);
                let label_x = content_area.x + kw + 2;
                if label_x < content_area.right() {
                    draw_str_clipped(buf, label_x, y, label_part, label_style, content_area);
                }
            } else {
                draw_str_clipped(buf, content_area.x, y, row, label_style, content_area);
            }
        }
        y += 1;
    }
}

/// Build the text rows for the help panel.
/// Section headings are prefixed with "##"; separators are "---".
fn build_rows(keymap: &KeyMap) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for (ctx, heading) in &[
        (Context::Global, "Global"),
        (Context::Map, "Map"),
        (Context::Anim, "Tidy-anim"),
    ] {
        rows.push(format!("## {heading}"));
        // Collect unique commands for this context, in ALL_COMMANDS order.
        let mut seen: Vec<Command> = Vec::new();
        for cmd in ALL_COMMANDS {
            if cmd.context() == *ctx && !seen.contains(cmd) {
                // Collect all keys for this command in this context.
                let keys: Vec<String> = keymap.bindings.iter()
                    .filter(|(_, c, cx)| *c == *cmd && *cx == *ctx)
                    .map(|(s, _, _)| s.label())
                    .collect();
                if !keys.is_empty() {
                    let keys_str = keys.join("/");
                    rows.push(format!("{:<18}  {}", keys_str, cmd.label()));
                    seen.push(*cmd);
                }
            }
        }
        rows.push("---".into());
    }
    rows
}

fn draw_border(buf: &mut Buffer, area: Rect, style: Style) {
    if area.height < 2 || area.width < 2 {
        return;
    }
    let (x0, y0) = (area.x, area.y);
    let (x1, y1) = (area.right() - 1, area.bottom() - 1);

    // Corners
    draw_char(buf, x0, y0, '╭', style);
    draw_char(buf, x1, y0, '╮', style);
    draw_char(buf, x0, y1, '╰', style);
    draw_char(buf, x1, y1, '╯', style);

    // Horizontal lines
    for x in (x0 + 1)..x1 {
        draw_char(buf, x, y0, '─', style);
        draw_char(buf, x, y1, '─', style);
    }
    // Vertical lines
    for y in (y0 + 1)..y1 {
        draw_char(buf, x0, y, '│', style);
        draw_char(buf, x1, y, '│', style);
    }
}

fn draw_char(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        let mut s = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
    }
}

// Needed for centered title calculation
trait RectExt {
    fn inner_width(&self) -> u16;
}
impl RectExt for Rect {
    fn inner_width(&self) -> u16 {
        self.width.saturating_sub(2)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    use super::draw_help;
    use crate::keymap::KeyMap;

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
    fn draw_help_writes_global_heading_and_save_game() {
        let km = KeyMap::default();
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_help(&km, area, &mut buf);
        let text = buf_text(&buf);
        assert!(text.contains("Global"), "expected 'Global' heading in help overlay");
        assert!(text.contains("save game"), "expected 'save game' label in help overlay");
        assert!(text.contains("Ctrl+S"), "expected 'Ctrl+S' key in help overlay");
    }

    #[test]
    fn draw_help_writes_map_heading() {
        let km = KeyMap::default();
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_help(&km, area, &mut buf);
        let text = buf_text(&buf);
        assert!(text.contains("Map"), "expected 'Map' heading");
        assert!(text.contains("zoom in"), "expected 'zoom in' label");
    }

    #[test]
    fn draw_help_reflects_rebinding() {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.overrides.insert("zoom_in".into(), "z".into());
        let (km, _) = KeyMap::resolve(&cfg);
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        draw_help(&km, area, &mut buf);
        let text = buf_text(&buf);
        // After rebinding, Z should appear next to zoom in
        assert!(text.contains('Z'), "expected 'Z' key after rebinding zoom_in");
    }
}
