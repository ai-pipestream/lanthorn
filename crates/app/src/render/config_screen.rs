//! Config-screen modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::config::BackgroundTidy;
use crate::state::AppState;

/// Row definitions: (display name, type tag).
const CONFIG_ROWS: &[(&str, ConfigRowKind)] = &[
    ("user_dir",             ConfigRowKind::Path),
    ("use_default_map",      ConfigRowKind::Bool),
    ("auto_load",            ConfigRowKind::Bool),
    ("auto_save",            ConfigRowKind::Bool),
    ("record_history",       ConfigRowKind::Bool),
    ("background_tidy",      ConfigRowKind::Enum),
    ("colors.scheme",        ConfigRowKind::Choice),
    ("symbols.box_style",    ConfigRowKind::Enum),
    ("symbols.arrow_set",    ConfigRowKind::Enum),
    ("symbols.portal_icons", ConfigRowKind::Enum),
    ("symbols.path_style",   ConfigRowKind::Enum),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigRowKind {
    Path,
    Bool,
    Enum,
    Choice,
}

/// Draw the config-screen modal centered over `area`.
/// Does nothing when `state.config_screen` is `None`.
pub fn draw_config_screen(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(cs) = &state.config_screen else { return };

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    let modal_h = (CONFIG_ROWS.len() as u16 + 4).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return;
    }

    let x = area.x + area.width.saturating_sub(modal_w) / 2;
    let y = area.y + area.height.saturating_sub(modal_h) / 2;
    let modal = Rect { x, y, width: modal_w, height: modal_h };

    // Opaque background.
    let bg = Style::reset().bg(Color::DarkGray);
    for row in modal.y..modal.bottom() {
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row)) {
                cell.set_symbol(" ").set_style(bg);
            }
        }
    }

    // Title.
    let title_style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD).bg(Color::DarkGray);
    crate::render::draw_str_clipped(buf, modal.x + 1, modal.y, "Settings", title_style, modal);

    // Column headers.
    let hdr_style = Style::new().fg(Color::White).add_modifier(Modifier::UNDERLINED).bg(Color::DarkGray);
    let name_col_w = 22usize;
    let hdr = format!("{:<width$}  Value", "Setting", width = name_col_w);
    crate::render::draw_str_clipped(buf, modal.x + 1, modal.y + 1, &hdr, hdr_style, modal);

    // Row styles.
    let normal = Style::new().fg(Color::White).bg(Color::DarkGray);
    let selected_style = Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD);

    for (i, (name, _kind)) in CONFIG_ROWS.iter().enumerate() {
        let row_y = modal.y + 2 + i as u16;
        if row_y >= modal.bottom().saturating_sub(1) {
            break;
        }

        let is_selected = i == cs.selected;
        let row_style = if is_selected { selected_style } else { normal };

        // Fill row background.
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(row_style);
            }
        }

        // Build value string.
        let value = config_row_value(&cs.working, i);

        let marker = if is_selected { ">" } else { " " };
        let name_trunc: String = name.chars().take(name_col_w).collect();
        let line = format!("{} {:<width$}  {}", marker, name_trunc, value, width = name_col_w);
        crate::render::draw_str_clipped(buf, modal.x + 1, row_y, &line, row_style, modal);
    }

    // Footer.
    let footer_y = modal.bottom().saturating_sub(1);
    if footer_y > modal.y + 2 {
        let footer_style = Style::new().fg(Color::DarkGray).bg(Color::Black);
        let footer = "\u{2191}\u{2193} move  \u{2190}\u{2192}/Space change  s save  Esc cancel";
        for col in modal.x..modal.right() {
            if let Some(cell) = buf.cell_mut((col, footer_y)) {
                cell.set_symbol(" ").set_style(footer_style);
            }
        }
        crate::render::draw_str_clipped(buf, modal.x + 1, footer_y, footer, footer_style, modal);
    }
}

/// Build the display value string for row `i` from the working config.
fn config_row_value(cfg: &crate::config::Config, i: usize) -> String {
    match i {
        0 => cfg.user_dir.to_string_lossy().to_string(),
        1 => bool_str(cfg.use_default_map),
        2 => bool_str(cfg.auto_load),
        3 => bool_str(cfg.auto_save),
        4 => bool_str(cfg.record_history),
        5 => match cfg.background_tidy {
            BackgroundTidy::Off => "off".to_string(),
            BackgroundTidy::EveryRoom => "every_room".to_string(),
            BackgroundTidy::OnOverlap => "on_overlap".to_string(),
            BackgroundTidy::Debounced => "debounced".to_string(),
        },
        6 => cfg.colors.scheme.clone().unwrap_or_else(|| "(none)".to_string()),
        7 => cfg.symbols.box_style.clone(),
        8 => cfg.symbols.arrow_set.clone(),
        9 => cfg.symbols.portal_icons.clone(),
        10 => cfg.symbols.path_style.clone(),
        _ => String::new(),
    }
}

fn bool_str(b: bool) -> String {
    if b { "true".to_string() } else { "false".to_string() }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, ConfigScreenState};

    fn state_with_config_screen() -> AppState {
        let mut s = AppState::default();
        let working = crate::input::clone_config(&s.config);
        s.config_screen = Some(ConfigScreenState { working, selected: 0 });
        s
    }

    #[test]
    fn draw_config_screen_shows_settings() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_config_screen();
        terminal.draw(|f| {
            draw_config_screen(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("auto_save"), "auto_save row should be visible");
        assert!(content.contains("background_tidy"), "background_tidy row should be visible");
    }

    #[test]
    fn draw_config_screen_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // config_screen = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_config_screen(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_config_screen should be no-op when closed");
    }
}
