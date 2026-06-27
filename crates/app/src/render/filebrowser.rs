//! File-browser modal overlay for import/export of standard Quetzal saves.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Draw the file-browser modal centered over `area`.
///
/// Shows the current directory, a list of entries (".." parent, dirs, then
/// matching files in PickFile mode), and a footer with key hints.
///
/// Does nothing when `state.file_browser` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_file_browser(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let Some(fb) = &state.file_browser else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    let entry_rows = fb.entries.len() as u16;
    // 1 cwd row + entry rows + 1 footer + border overhead (2) + button row (1) = entry_rows + 5
    let modal_h = (entry_rows + 5).max(7).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    use crate::state::FbMode;
    let title = match fb.mode {
        FbMode::PickFile => "Import Save (.qzl/.sav)",
        FbMode::PickDir => "Export Save — choose directory",
    };

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let spec = DialogSpec {
        title,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // ── CWD row ───────────────────────────────────────────────────────────────

    let cwd_str = format!("  {}", fb.cwd.display());
    let cwd_style = Style::default().fg(Color::Yellow).patch(state.colors.dialog);
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &cwd_str, cwd_style, content);
    }

    // ── Entry rows ────────────────────────────────────────────────────────────

    let normal = state.colors.dialog;
    let selected_style = Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dir_style = Style::default().fg(Color::Cyan).patch(state.colors.dialog);
    let dir_selected_style = Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let entries_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    // Reserve last row of entries_area for footer.
    let entries_max_y = entries_area.bottom().saturating_sub(1);

    for (i, entry) in fb.entries.iter().enumerate() {
        let row_y = entries_area.y + i as u16;
        if row_y >= entries_max_y {
            break;
        }

        let is_sel = i == fb.selected;

        // Fill row background.
        let row_bg = if is_sel { selected_style } else { normal };
        for col in content.x..content.right() {
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
        crate::render::draw_str_clipped(buf, content.x, row_y, &label, text_style, content);
    }

    // ── Footer hint ───────────────────────────────────────────────────────────

    let footer_y = entries_max_y;
    if footer_y < entries_area.bottom() {
        let footer_style = Style::default().fg(Color::DarkGray).patch(state.colors.dialog);
        let footer = match fb.mode {
            FbMode::PickFile => "Up/Dn:move  Enter:open/import  Esc:cancel",
            FbMode::PickDir => "Up/Dn:move  Enter:open  s:export here  Esc:cancel",
        };
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::render::dialog::ButtonId;
    use crate::render::paneframe::BorderStyle;
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
        // Use a taller terminal (30 rows) so the dialog chrome renders with Single border
        // and the title appears in the border row distinct from the content body.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_browser(tmp, FbMode::PickFile);
        // Use Single border so title and content do not overlap.
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
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
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_browser(tmp, FbMode::PickDir);
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        terminal.draw(|f| {
            draw_file_browser(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Export"), "should show Export title in PickDir mode");
    }

    #[test]
    fn draw_file_browser_shows_dialog_chrome() {
        // Render test: file browser shows bordered titled chrome with [X] + [Done] and dialog_* colors.
        let tmp = std::env::temp_dir();
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_browser(tmp, FbMode::PickFile);
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_file_browser(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        assert!(content.contains("Import"), "title should be present");
        assert!(content.contains("Done"), "[Done] button should be visible");
        assert!(content.contains('✕'), "[X] close button should be visible");

        let rects = rects_out.expect("draw_file_browser should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1, "should have 1 button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }
}
