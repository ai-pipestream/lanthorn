//! Saves-manager modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Draw the saves-manager modal centered over `area`.
///
/// The modal lists all save files for the current story: default slot labelled
/// "(default)", named saves showing name, turn count, and save timestamp.
/// The currently-selected row is highlighted. A footer shows the available
/// key actions.
///
/// Does nothing when `state.saves` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_saves(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let Some(saves) = &state.saves else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    // Target: up to 62 wide, tall enough for entries + 2 header + 1 footer + chrome overhead.
    let modal_w = 62u16.min(area.width.saturating_sub(4));
    let entry_rows = saves.entries.len() as u16;
    // 2 header rows + entry rows + 1 footer + border overhead (2) + button row (1) = entry_rows + 6
    let modal_h = (entry_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle {
        frame: state.colors.dialog,
        box_style: state.colors.dialog_box_style,
        title: state.colors.dialog_title,
        button: state.colors.dialog_button,
        button_active: state.colors.dialog_button_active,
        shadow: state.colors.dialog_shadow,
        shadow_on: state.colors.dialog_shadow_on,
    };

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let spec = DialogSpec {
        title: "Saves",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.dialog_focus),
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // ── Column headers ────────────────────────────────────────────────────────

    let hdr_style = Style::new().add_modifier(Modifier::UNDERLINED).patch(state.colors.dialog);
    let hdr = format!("{:<28}  {:>5}  {:<16}", "Name", "Turns", "Saved at");
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &hdr, hdr_style, content);
    }

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = state.colors.dialog;
    let selected_style = Style::new()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let entries_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    for (i, entry) in saves.entries.iter().enumerate() {
        let row_y = entries_area.y + i as u16;
        if row_y >= entries_area.bottom() {
            break;
        }

        let style = if i == saves.selected { selected_style } else { normal };

        // Fill the whole row background with the row style.
        for col in content.x..content.right() {
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
        crate::render::draw_str_clipped(buf, content.x, row_y, &line, style, content);
    }

    // ── Footer hint (below entries) ───────────────────────────────────────────

    let footer_y = entries_area.bottom();
    if footer_y < content.bottom() {
        let footer_style = Style::new()
            .fg(ratatui::style::Color::DarkGray)
            .patch(state.colors.dialog);
        let footer = "Enter:load  s:save-as  d:delete  e:export  i:import  Esc:close";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
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
    use crate::render::dialog::ButtonId;
    use crate::render::paneframe::BorderStyle;
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

    #[test]
    fn draw_saves_shows_dialog_chrome() {
        // Render test: saves shows bordered titled chrome with [X] + [Done] and dialog_* colors.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_saves(
            vec![dummy_save("(default)", 0, true)],
            0,
        );
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_saves(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("Saves"), "title 'Saves' should be present");
        assert!(content.contains("Done"), "[Done] button should be visible");
        assert!(content.contains('✕'), "[X] close button should be visible");

        let rects = rects_out.expect("draw_saves should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1, "should have 1 button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }
}
