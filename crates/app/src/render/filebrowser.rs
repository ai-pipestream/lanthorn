//! File-browser modal overlay for import/export of standard Quetzal saves.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::AppState;

/// Draw the file-browser modal centered over `area`.
///
/// Shows the current directory, a list of entries (`.." parent, dirs, then
/// matching files in PickFile mode), and a footer with key hints.
///
/// Does nothing when `state.file_browser` is `None`.
pub fn draw_file_browser(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(fb) = &state.file_browser else { return };

    // ── Modal geometry ────────────────────────────────────────────────────────

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    let entry_rows = fb.entries.len() as u16;
    // 1 title + 1 cwd + entry rows + 1 footer = entry_rows + 3
    let modal_h = (entry_rows + 3).max(5).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 3 {
        return;
    }

    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal = Rect { x, y, width: modal_w, height: modal_h };

    // ── Background fill (opaque) ──────────────────────────────────────────────

    let bg = Style::reset().bg(Color::DarkGray).fg(Color::White);
    for row in modal.y..modal.bottom() {
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ").set_style(bg);
            }
        }
    }

    // ── Title row ─────────────────────────────────────────────────────────────

    use crate::state::FbMode;
    let title = match fb.mode {
        FbMode::PickFile => "Import Save (.qzl/.sav)",
        FbMode::PickDir => "Export Save — choose directory",
    };
    let title_style = Style::reset().fg(Color::Cyan).add_modifier(Modifier::BOLD).bg(Color::DarkGray);
    crate::render::draw_str_clipped(buf, modal.x + 1, modal.y, title, title_style, modal);

    // ── CWD row ───────────────────────────────────────────────────────────────

    let cwd_str = format!("  {}", fb.cwd.display());
    let cwd_style = Style::reset().fg(Color::Yellow).bg(Color::DarkGray);
    crate::render::draw_str_clipped(buf, modal.x, modal.y + 1, &cwd_str, cwd_style, modal);

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = Style::reset().fg(Color::White).bg(Color::DarkGray);
    let selected_style = Style::reset()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dir_style = Style::reset().fg(Color::Cyan).bg(Color::DarkGray);
    let dir_selected_style = Style::reset()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let entries_start_y = modal.y + 2;
    // Reserve 1 row for footer.
    let entries_max_y = modal.bottom().saturating_sub(1);

    for (i, entry) in fb.entries.iter().enumerate() {
        let row_y = entries_start_y + i as u16;
        if row_y >= entries_max_y {
            break;
        }

        let is_sel = i == fb.selected;

        // Fill row background.
        let row_bg = if is_sel { selected_style } else { normal };
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(row_bg);
            }
        }

        // Choose text style.
        let text_style = if is_sel {
            if entry.is_dir { dir_selected_style } else { selected_style }
        } else if entry.is_dir {
            dir_style
        } else {
            normal
        };

        let marker = if is_sel { ">" } else { " " };
        let suffix = if entry.is_dir && entry.name != ".." { "/" } else { "" };
        let label = format!("{} {}{}", marker, entry.name, suffix);
        crate::render::draw_str_clipped(buf, modal.x + 1, row_y, &label, text_style, modal);
    }

    // ── Footer hint ───────────────────────────────────────────────────────────

    let footer_y = modal.bottom().saturating_sub(1);
    if footer_y > modal.y {
        let footer_style = Style::reset().fg(Color::DarkGray).bg(Color::Black);
        let footer = match fb.mode {
            FbMode::PickFile => "Up/Dn:move  Enter:open/import  Esc/q:cancel",
            FbMode::PickDir => "Up/Dn:move  Enter:open  s:export here  Esc/q:cancel",
        };
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, footer_y)) {
                cell.set_symbol(" ").set_style(footer_style);
            }
        }
        crate::render::draw_str_clipped(buf, modal.x + 1, footer_y, footer, footer_style, modal);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, FbMode, FileBrowserState};
    use std::path::PathBuf;

    fn state_with_browser(cwd: PathBuf, mode: FbMode) -> AppState {
        let mut s = AppState::default();
        s.file_browser = Some(FileBrowserState::build(cwd, mode, "ZCODE-1-TEST-0.qzl".to_string()));
        s
    }

    #[test]
    fn draw_file_browser_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // file_browser = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_file_browser should be a no-op when file_browser is None");
    }

    #[test]
    fn draw_file_browser_shows_in_pickfile_mode() {
        let tmp = std::env::temp_dir();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_browser(tmp, FbMode::PickFile);
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Import"), "should show Import title in PickFile mode");
    }

    #[test]
    fn draw_file_browser_shows_in_pickdir_mode() {
        let tmp = std::env::temp_dir();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_browser(tmp, FbMode::PickDir);
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Export"), "should show Export title in PickDir mode");
    }
}
