use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

// Minimum dimensions for the reset dialog.
const MIN_W: u16 = 36;
const MIN_H: u16 = 10;

// Dialog dimensions.
const DIALOG_W: u16 = 38;
const DIALOG_H: u16 = 12;

// ── ResetDialogRects ──────────────────────────────────────────────────────────

pub struct ResetDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect for the "Also clear the map" checkbox.
    pub checkbox: Rect,
    /// Hit-rect for the "Delete saved progress" checkbox.
    pub checkbox_data: Rect,
    pub reset: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_reset_dialog ─────────────────────────────────────────────────────────

/// Draw the reset-confirmation dialog centered over `area`.
///
/// Returns `None` when `state.reset_dialog` is false or the area is too small.
/// Returns `ResetDialogRects` with hit-rects for close, checkbox, and buttons.
pub fn draw_reset_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<ResetDialogRects> {
    if !state.reset_dialog {
        return None;
    }

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Reset, label: "Reset" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    // Focus ring: 0 = "clear map" checkbox, 1 = "delete data" checkbox,
    // 2 = Reset button, 3 = Cancel button. Only buttons highlight via DialogSpec;
    // a focused checkbox is highlighted inline below.
    let button_focus = state.dialog_focus.checked_sub(2);
    let spec = DialogSpec {
        title: "Reset game?",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Reset),
        focus: button_focus,
        field: None,
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Draw content into the content area.
    // Row 0: body line
    if content.height >= 1 {
        let body_style = state.colors.dialog;
        crate::render::draw_str_clipped(
            buf,
            content.x,
            content.y,
            "Restart the story from the beginning.",
            body_style,
            content,
        );
    }

    // Row 1: blank (skip)

    // Helper: draw one checkbox row, highlighting it when focused. Returns its
    // hit-rect (a zero-size rect when the row is off the content area).
    let draw_checkbox = |buf: &mut Buffer, y: u16, checked: bool, focused: bool, text: &str| -> Rect {
        if y >= content.bottom() {
            return Rect::new(content.x, content.y, 0, 0);
        }
        let check = if checked { "[x]" } else { "[ ]" };
        let label = format!("{} {}", check, text);
        let style = if focused { state.colors.dialog_button_active } else { state.colors.dialog };
        crate::render::draw_str_clipped(buf, content.x, y, &label, style, content);
        let rect_w = (label.chars().count() as u16).min(content.width);
        Rect::new(content.x, y, rect_w, 1)
    };

    // Row 2: "Also clear the map" checkbox (focus 0).
    let checkbox_rect = draw_checkbox(
        buf,
        content.y + 2,
        state.reset_clear_map,
        state.dialog_focus == 0,
        "Also clear the map",
    );

    // Row 3: "Delete saved progress" checkbox (focus 1).
    let checkbox_data_rect = draw_checkbox(
        buf,
        content.y + 3,
        state.reset_delete_data,
        state.dialog_focus == 1,
        "Delete saved progress",
    );

    // Map button rects from draw_dialog output.
    let reset_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Reset).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(ResetDialogRects {
        area: rects.area,
        close: rects.close,
        checkbox: checkbox_rect,
        checkbox_data: checkbox_data_rect,
        reset: reset_rect,
        cancel: cancel_rect,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_dialog_renders_title_checkbox_and_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.reset_dialog = true;
        state.reset_clear_map = false;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal.draw(|f| { rects = draw_reset_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
        let r = rects.expect("dialog should render when reset_dialog is set");
        assert!(r.close.is_some() && r.reset.is_some() && r.cancel.is_some());
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars()).collect();
        assert!(all.contains("Reset game?"), "title present");
        assert!(all.contains("Also clear the map"), "clear-map checkbox label present");
        assert!(all.contains("Delete saved progress"), "delete-data checkbox label present");
        assert!(all.contains("[ ]"), "unchecked box shown when reset flags are false");
    }
}
