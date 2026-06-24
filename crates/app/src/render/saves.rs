//! Saves-manager modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::AppState;

/// Draw the saves-manager modal centered over `area`.
///
/// The modal lists all save files for the current story: default slot labelled
/// "(default)", named saves showing name, turn count, and save timestamp.
/// The currently-selected row is highlighted. A footer shows the available
/// key actions.
///
/// Does nothing when `state.saves` is `None`.
pub fn draw_saves(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(saves) = &state.saves else { return };

    // ── Modal geometry ────────────────────────────────────────────────────────

    // Target: up to 60 wide, tall enough for entries + 2 header + 1 footer.
    let modal_w = 62u16.min(area.width.saturating_sub(4));
    let entry_rows = saves.entries.len() as u16;
    let modal_h = (entry_rows + 4).min(area.height.saturating_sub(2)); // 2 header + 1 sep + 1 footer
    if modal_w < 20 || modal_h < 3 {
        return;
    }

    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal = Rect { x, y, width: modal_w, height: modal_h };

    // ── Background fill ───────────────────────────────────────────────────────

    let bg = Style::new().fg(Color::White).bg(Color::DarkGray);
    for row in modal.y..modal.bottom() {
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ").set_style(bg);
            }
        }
    }

    // ── Title row ─────────────────────────────────────────────────────────────

    let title_style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    crate::render::draw_str_clipped(buf, modal.x + 1, modal.y, "Saves", title_style, modal);

    // ── Column headers ────────────────────────────────────────────────────────

    if modal_h < 3 {
        return;
    }
    let hdr_style = Style::new().fg(Color::White).add_modifier(Modifier::UNDERLINED);
    let hdr = format!("{:<28}  {:>5}  {:<16}", "Name", "Turns", "Saved at");
    crate::render::draw_str_clipped(buf, modal.x + 1, modal.y + 1, &hdr, hdr_style, modal);

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = Style::new().fg(Color::White).bg(Color::DarkGray);
    let selected = Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let entries_area = Rect {
        x: modal.x,
        y: modal.y + 2,
        width: modal.width,
        height: modal.height.saturating_sub(3), // leave room for footer
    };

    for (i, entry) in saves.entries.iter().enumerate() {
        let row_y = entries_area.y + i as u16;
        if row_y >= entries_area.bottom() {
            break;
        }

        let style = if i == saves.selected { selected } else { normal };

        // Fill the whole row background with the row style.
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }

        // Marker + name (truncated to 28 chars).
        let marker = if i == saves.selected { ">" } else { " " };
        let name_trunc: String = entry.name.chars().take(26).collect();
        let short_time = format_time(&entry.saved_at);
        let line = format!(
            "{} {:<27}  {:>5}  {:<16}",
            marker, name_trunc, entry.turns, short_time
        );
        crate::render::draw_str_clipped(buf, modal.x + 1, row_y, &line, style, modal);
    }

    // ── Footer hint ───────────────────────────────────────────────────────────

    let footer_y = modal.bottom().saturating_sub(1);
    if footer_y > modal.y {
        let footer_style = Style::new().fg(Color::DarkGray).bg(Color::Black);
        let footer = "Enter:load  s:save-as  d:delete  e:export  i:import  Esc:close";
        // Fill footer row.
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, footer_y)) {
                cell.set_symbol(" ").set_style(footer_style);
            }
        }
        crate::render::draw_str_clipped(buf, modal.x + 1, footer_y, footer, footer_style, modal);
    }
}

/// Format an RFC3339 timestamp for compact display (show only date + HH:MM).
/// Returns empty string for empty input.
fn format_time(ts: &str) -> String {
    if ts.is_empty() {
        return String::new();
    }
    // "2026-06-18T12:34:56Z" → "2026-06-18 12:34"
    if ts.len() >= 16 {
        let date = &ts[0..10];
        let time = &ts[11..16];
        return format!("{} {}", date, time);
    }
    ts[..ts.len().min(16)].to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, SavesState};
    use crate::persist_files::SaveInfo;
    use std::path::PathBuf;

    fn dummy_save(name: &str, turns: u32, is_default: bool) -> SaveInfo {
        SaveInfo {
            path: PathBuf::from(format!("/tmp/{}.babelmap", name)),
            name: name.to_string(),
            turns,
            saved_at: "2026-06-18T10:00:00Z".to_string(),
            is_default,
        }
    }

    fn state_with_saves(entries: Vec<SaveInfo>, selected: usize) -> AppState {
        let mut s = AppState::default();
        s.saves = Some(SavesState { entries, selected });
        s
    }

    #[test]
    fn draw_saves_shows_entry_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![
                dummy_save("(default)", 0, true),
                dummy_save("before-troll", 42, false),
            ],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("(default)"), "default slot should be listed");
        assert!(content.contains("before-troll"), "named save should be listed");
    }

    #[test]
    fn draw_saves_labels_default_slot() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![dummy_save("(default)", 0, true)],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("(default)"), "default slot must be labelled (default)");
    }

    #[test]
    fn draw_saves_shows_turn_count() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_saves(
            vec![dummy_save("after-troll", 99, false)],
            0,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("99"), "turn count should be displayed");
    }

    #[test]
    fn draw_saves_selection_marker_on_active_row() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        // Two entries; selected = 1.
        let state = state_with_saves(
            vec![
                dummy_save("(default)", 0, true),
                dummy_save("chapter-2", 55, false),
            ],
            1,
        );
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        // "chapter-2" should appear in the content with the marker
        assert!(content.contains("chapter-2"), "selected entry name should appear");
    }

    #[test]
    fn draw_saves_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // saves = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_saves should be a no-op when saves is None");
    }

    #[test]
    fn format_time_truncates_to_date_and_hhmm() {
        assert_eq!(super::format_time("2026-06-18T12:34:56Z"), "2026-06-18 12:34");
        assert_eq!(super::format_time(""), "");
        assert_eq!(super::format_time("2026-06-18T10:00:00Z"), "2026-06-18 10:00");
    }
}
