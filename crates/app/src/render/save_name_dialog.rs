use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::render::dialog::{
    ButtonId, DialogButton, DialogField, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::AppState;

// Minimum dimensions for the save-name dialog.
const MIN_W: u16 = 34;
const MIN_H: u16 = 7;

// Dialog dimensions.
const DIALOG_W: u16 = 44;
const DIALOG_H: u16 = 7;

// ── SaveNameDialogRects ───────────────────────────────────────────────────────

pub struct SaveNameDialogRects {
    pub area: Rect,
    pub close: Option<Rect>,
    /// Hit-rect for the text field row.
    pub field: Option<Rect>,
    pub save: Option<Rect>,
    pub cancel: Option<Rect>,
}

// ── draw_save_name_dialog ─────────────────────────────────────────────────────

/// Draw the save-name dialog (a common-dialog with a caret text field) centered
/// over `area` (the graphics-free dialog region). Returns `None` when the dialog
/// is closed or the area is too small.
///
/// Focus ring: 0 = text field, 1 = Save, 2 = Cancel. Only the buttons highlight
/// via `DialogSpec.focus`; the field's caret is shown when it is focused and active.
pub fn draw_save_name_dialog(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Option<SaveNameDialogRects> {
    let dlg = state.save_name_dialog.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Save, label: "Save" },
        DialogButton { id: ButtonId::Cancel, label: "Cancel" },
    ];
    // Buttons occupy focus slots 1 and 2; slot 0 is the field.
    let button_focus = state.dialog_focus.checked_sub(1);

    // The caret shows only while the field is focused and being edited; the
    // placeholder (default) is dimmed until adopted.
    let field_focused = state.dialog_focus == 0;
    let field = DialogField {
        label: "Name: ",
        value: &dlg.field.value,
        cursor: dlg.field.cursor,
        show_caret: field_focused && dlg.active,
        dim: !dlg.active,
        text_style: state.colors.dialog,
        dim_style: state.colors.dialog.add_modifier(Modifier::DIM),
        caret_style: state.colors.dialog.add_modifier(Modifier::REVERSED),
    };

    let spec = DialogSpec {
        title: if dlg.ingame { "Save game as" } else { "Save State as" },
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Save),
        focus: button_focus,
        field: Some(field),
    };

    let rects = draw_dialog(buf, area, &spec, &st);

    let save_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).map(|(_, r)| *r);
    let cancel_rect = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r);

    Some(SaveNameDialogRects {
        area: rects.area,
        close: rects.close,
        field: rects.field,
        save: save_rect,
        cancel: cancel_rect,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_name_dialog_renders_field_and_buttons_in_area() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut state = crate::state::AppState::default();
        state.save_name_dialog =
            Some(crate::state::SaveNameDialog::new("2026-07-13 1432".to_string(), false));
        state.dialog_focus = 0;
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects = None;
        terminal
            .draw(|f| rects = draw_save_name_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let r = rects.expect("dialog renders when save_name_dialog is set");
        assert!(r.close.is_some() && r.save.is_some() && r.cancel.is_some());
        assert!(r.field.is_some(), "field rect recorded for hit-testing");
        // The dialog is confined to the passed area (graphics-safe): its frame sits
        // inside the terminal, not painted over a story/graphics pane out of bounds.
        assert!(r.area.right() <= 60 && r.area.bottom() <= 20);
        let all: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all.contains("Save State as"), "title present");
        assert!(all.contains("Name:"), "field label present");
        assert!(all.contains("2026-07-13 1432"), "default name prefilled");
    }
}
