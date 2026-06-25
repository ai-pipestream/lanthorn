use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the launch dialog.
const MIN_W: u16 = 38;
const MIN_H: u16 = 9;

// Dialog dimensions.
const DIALOG_W: u16 = 44;
const DIALOG_H: u16 = 10;

// ── LaunchDialogRects ─────────────────────────────────────────────────────────

pub struct LaunchDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub resume: Option<Rect>,
    pub new_game: Option<Rect>,
}

// ── draw_launch_dialog ────────────────────────────────────────────────────────

/// Draw the launch "Resume saved game?" dialog centered over `area`.
///
/// Returns `None` when `state.launch_dialog` is false or the area is too small.
/// Returns `LaunchDialogRects` with hit-rects for close and buttons.
pub fn draw_launch_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<LaunchDialogRects> {
    if !state.launch_dialog {
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
        DialogButton { id: ButtonId::Resume, label: "Resume" },
        DialogButton { id: ButtonId::NewGame, label: "New game" },
    ];
    let spec = DialogSpec {
        title: "Resume saved game?",
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
            "A save was found for this story.",
            body_style,
            content,
        );
    }

    // Map button rects from draw_dialog output.
    let resume_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Resume).map(|(_, r)| *r);
    let new_game_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::NewGame).map(|(_, r)| *r);

    Some(LaunchDialogRects {
        area: rects.area,
        close: rects.close,
        resume: resume_rect,
        new_game: new_game_rect,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_dialog_renders_title_body_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.launch_dialog = true;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when launch_dialog is set");
        assert!(r.resume.is_some(), "resume button rect must be present");
        assert!(r.new_game.is_some(), "new_game button rect must be present");
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Resume saved game?"), "title must be present");
        assert!(all.contains("save was found"), "body line must be present");
        assert!(all.contains("Resume"), "resume button label must be present");
        assert!(all.contains("New game"), "new_game button label must be present");
    }

    #[test]
    fn launch_dialog_returns_none_when_flag_false() {
        use ratatui::{backend::TestBackend, Terminal};
        let state = crate::state::AppState::default(); // launch_dialog = false
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when launch_dialog is false");
    }

    #[test]
    fn launch_dialog_returns_none_when_area_too_small() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.launch_dialog = true;
        // Use an area smaller than MIN_W x MIN_H
        let backend = TestBackend::new(10, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_launch_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        assert!(rects.is_none(), "dialog must not render when area is too small");
    }
}
