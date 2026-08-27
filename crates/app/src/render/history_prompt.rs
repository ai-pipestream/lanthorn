//! The "turn history isn't being recorded — switch it on?" prompt (SQ-1091).
//!
//! Raised by `open-history` when there is nothing to replay AND the capture that
//! would have filled it is off. That combination used to be a silent no-op: the
//! command did nothing, said nothing, and gave the player no way to find out that
//! a setting governed it. A menu row that cannot work was removed for the same
//! reason (SQ-1090's sibling change); this is what the command does instead.
//!
//! Deliberately thin. The chrome, the focus ring, the button hit-rects and the
//! keyboard ladder all come from [`crate::render::dialog::draw_dialog`] and the
//! `Overlay` trait, exactly as the aux-storage prompt does — the only thing here
//! that is not shared is the wording and two button labels.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::AppState;

const MIN_W: u16 = 44;
const MIN_H: u16 = 9;
const DIALOG_W: u16 = 62;
const DIALOG_H: u16 = 10;

pub struct HistoryPromptRects {
    pub area: Rect,
    pub close: Option<Rect>,
    pub enable: Option<Rect>,
    pub cancel: Option<Rect>,
}

/// Draw the prompt centred over `area`, or `None` when it is closed or the pane
/// is too small to hold it.
pub fn draw_history_prompt(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<HistoryPromptRects> {
    if !state.overlays.history_prompt {
        return None;
    }
    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));
    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    let st = DialogStyle::from_colors(&state.colors);
    let buttons = &[
        DialogButton { id: ButtonId::Ok, label: "Record from now on" },
        DialogButton { id: ButtonId::Cancel, label: "Not now" },
    ];
    let spec = DialogSpec {
        title: "Rewind",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Ok),
        focus: Some(state.overlays.dialog_focus),
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;

    // Say what is true now, what switching it on does, and what it costs — the
    // last of those is why the setting is off by default, so a prompt that hid it
    // would be talking the player into something.
    let body = [
        "Turn history is not being recorded, so there is",
        "nothing to rewind through yet.",
        "",
        "Recording keeps a snapshot of every turn in this",
        "game's archive. It grows the file.",
    ];
    let style = state.colors.theme.get("dialog.background").style;
    for (i, line) in body.iter().enumerate() {
        let y = content.y + i as u16;
        if y < content.bottom() {
            crate::render::draw_str_clipped(buf, content.x, y, line, style, content);
        }
    }

    Some(HistoryPromptRects {
        area: rects.area,
        close: rects.close,
        enable: rects.buttons.iter().find(|(id, _)| *id == ButtonId::Ok).map(|(_, r)| *r),
        cancel: rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).map(|(_, r)| *r),
    })
}
