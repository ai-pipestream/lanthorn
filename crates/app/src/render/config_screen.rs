//! Config-screen modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::config::BackgroundTidy;
use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

/// Row definitions: (display name, type tag).
pub(crate) const CONFIG_ROWS: &[(&str, ConfigRowKind)] = &[
    ("user_dir",             ConfigRowKind::Path),
    ("use_default_map",      ConfigRowKind::Bool),
    ("auto_load",            ConfigRowKind::Bool),
    ("auto_save",            ConfigRowKind::Bool),
    ("record_history",       ConfigRowKind::Bool),
    ("show_room_numbers",    ConfigRowKind::Bool),
    ("background_tidy",      ConfigRowKind::Enum),
    ("colors.scheme",        ConfigRowKind::Choice),
    ("symbols.box_style",    ConfigRowKind::Enum),
    ("symbols.arrow_set",    ConfigRowKind::Enum),
    ("symbols.portal_icons", ConfigRowKind::Enum),
    ("symbols.path_style",   ConfigRowKind::Enum),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigRowKind {
    Path,
    Bool,
    Enum,
    Choice,
}

/// Draw the config-screen modal centered over `area`.
/// Does nothing when `state.config_screen` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` otherwise.
pub fn draw_config_screen(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let Some(cs) = &state.config_screen else { return None };

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    // +4: title row (inside border) + header + button row + border overhead
    let modal_h = (CONFIG_ROWS.len() as u16 + 6).min(area.height.saturating_sub(2));
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // Build DialogStyle from state colors
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
        DialogButton { id: ButtonId::Save,   label: "Save"   },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];

    let spec = DialogSpec {
        title: "Settings",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // Draw column headers inside content
    let hdr_style = Style::new()
        .fg(Color::White)
        .add_modifier(Modifier::UNDERLINED)
        .patch(state.colors.dialog);
    let name_col_w = 22usize;
    let hdr = format!("{:<width$}  Value", "Setting", width = name_col_w);
    if content.height > 0 {
        crate::render::draw_str_clipped(buf, content.x, content.y, &hdr, hdr_style, content);
    }

    // Row styles
    let normal = state.colors.dialog;
    let selected_style = Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // Content rows start after the header line
    let rows_area = if content.height > 1 {
        Rect::new(content.x, content.y + 1, content.width, content.height - 1)
    } else {
        Rect::new(content.x, content.y, content.width, 0)
    };

    for (i, (name, _kind)) in CONFIG_ROWS.iter().enumerate() {
        let row_y = rows_area.y + i as u16;
        if row_y >= rows_area.bottom() {
            break;
        }

        let is_selected = i == cs.selected;
        let row_style = if is_selected { selected_style } else { normal };

        // Fill row background.
        for col in content.x..content.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(row_style);
            }
        }

        // Build value string.
        let value = config_row_value(&cs.working, i);

        let marker = if is_selected { ">" } else { " " };
        let name_trunc: String = name.chars().take(name_col_w).collect();
        let line = format!("{} {:<width$}  {}", marker, name_trunc, value, width = name_col_w);
        crate::render::draw_str_clipped(buf, content.x, row_y, &line, row_style, rows_area);
    }

    Some(rects)
}

/// Build the display value string for row `i` from the working config.
fn config_row_value(cfg: &crate::config::Config, i: usize) -> String {
    match i {
        0 => cfg.user_dir.to_string_lossy().to_string(),
        1 => bool_str(cfg.use_default_map),
        2 => bool_str(cfg.auto_load),
        3 => bool_str(cfg.auto_save),
        4 => bool_str(cfg.record_history),
        5 => bool_str(cfg.show_room_numbers),
        6 => match cfg.background_tidy {
            BackgroundTidy::Off => "off".to_string(),
            BackgroundTidy::EveryRoom => "every_room".to_string(),
            BackgroundTidy::OnOverlap => "on_overlap".to_string(),
            BackgroundTidy::Debounced => "debounced".to_string(),
        },
        7 => cfg.colors.scheme.clone().unwrap_or_else(|| "(none)".to_string()),
        8 => cfg.symbols.box_style.clone().unwrap_or_else(crate::config::default_box_style),
        9 => cfg.symbols.arrow_set.clone().unwrap_or_else(crate::config::default_arrow_set),
        10 => cfg.symbols.portal_icons.clone().unwrap_or_else(crate::config::default_portal_icons),
        11 => cfg.symbols.path_style.clone().unwrap_or_else(crate::config::default_path_style),
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

    #[test]
    fn draw_config_screen_shows_chrome() {
        // Render test: config screen shows a border + title + [Save]/[Cancel] + [X]
        // with colors from state.colors.dialog_*
        use crate::render::paneframe::BorderStyle;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = state_with_config_screen();
        // Use Single border so the title renders on the border row (not overlapping content)
        state.colors.dialog_box_style = BorderStyle::Single;
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_config_screen(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();

        // Title should appear
        assert!(content.contains("Settings"), "title 'Settings' should be present");
        // Save and Cancel buttons
        assert!(content.contains("Save"), "[Save] button should be visible");
        assert!(content.contains("Cancel"), "[Cancel] button should be visible");
        // Close button (✕)
        assert!(content.contains('✕'), "[X] close button should be visible");

        // DialogRects should be returned
        let rects = rects_out.expect("draw_config_screen should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 2, "should have 2 buttons");

        // Verify button ids
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Save));
        assert!(ids.contains(&ButtonId::Cancel));
    }
}
