use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the quit dialog.
const MIN_W: u16 = 38;
const MIN_H: u16 = 9;

// Dialog dimensions.
const DIALOG_W: u16 = 42;
const DIALOG_H: u16 = 10;

// ── QuitDialogRects ───────────────────────────────────────────────────────────

pub struct QuitDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub save: Option<Rect>,
    pub quit: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_quit_dialog ──────────────────────────────────────────────────────────

/// Draw the quit-confirmation dialog centered over `area`.
///
/// Returns `None` when `state.quit_dialog` is false or the area is too small.
/// Returns `QuitDialogRects` with hit-rects for close and buttons.
pub fn draw_quit_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<QuitDialogRects> {
    if !state.quit_dialog {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

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
        DialogButton { id: ButtonId::Save, label: "Save & quit" },
        DialogButton { id: ButtonId::Ok, label: "Quit" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    let spec = DialogSpec {
        title: "Save before quitting?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: None,
        focus: None,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // Draw body line into the content area.
    if content.height >= 1 {
        let body_style = state.colors.dialog;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y,
            "You have unsaved progress.",
            body_style,
            content,
        );
    }

    // Map button rects from draw_dialog output.
    let save_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).map(|(_, r)| *r);
    let quit_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(QuitDialogRects {
        area: rects.area,
        close: rects.close,
        save: save_rect,
        quit: quit_rect,
        cancel: cancel_rect,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_dialog_renders_title_body_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.quit_dialog = true;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_quit_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when quit_dialog is set");
        assert!(r.save.is_some(), "save button rect must be present");
        assert!(r.quit.is_some(), "quit button rect must be present");
        assert!(r.cancel.is_some(), "cancel button rect must be present");
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Save before quitting?"), "title must be present");
        assert!(all.contains("unsaved progress"), "body line must be present");
        assert!(all.contains("Save & quit"), "save button label must be present");
        assert!(all.contains("Quit"), "quit button label must be present");
        assert!(all.contains("Cancel"), "cancel button label must be present");
    }

    #[test]
    fn quit_dialog_returns_none_when_flag_false() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = crate::state::AppState::default(); // quit_dialog = false
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_quit_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when quit_dialog is false");
    }

    #[test]
    fn quit_dialog_returns_none_when_area_too_small() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.quit_dialog = true;
        // Use an area smaller than MIN_W x MIN_H
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_quit_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when area is too small");
    }
}
