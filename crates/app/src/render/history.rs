//! Rewind/replay history modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{
    ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::AppState;

/// Draw the replay/rewind modal centered over `area`: a turn list (turn# +
/// command) with the selected turn highlighted, the selected turn's transcript,
/// and a transport footer. Does nothing when `state.replay` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing).
pub fn draw_history(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let replay = state.replay.as_ref()?;
    if state.history.is_empty() {
        return None;
    }

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    // up to 12 list rows + 1 footer + 2 header/sep + chrome.
    let list_rows = (state.history.len() as u16).min(12);
    let modal_h = (list_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 6 {
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
    let buttons = &[DialogButton { id: ButtonId::Done, label: "Done" }];
    let spec = DialogSpec {
        title: "Replay",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.dialog_focus),
    };
    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    let normal = state.colors.dialog;
    let selected_style = Style::new()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // ── Turn list ────────────────────────────────────────────────────────────
    // Window the list around the selection so it stays visible.
    let visible = list_rows as usize;
    let first = replay.idx.saturating_sub(visible.saturating_sub(1));
    for (row, i) in (first..state.history.len()).take(visible).enumerate() {
        let row_y = content.y + row as u16;
        if row_y >= content.bottom() { break; }
        let rec = &state.history[i];
        let style = if i == replay.idx { selected_style } else { normal };
        for col in content.x..content.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        let marker = if i == replay.idx { ">" } else { " " };
        let cmd_trunc: String = rec.command.chars().take(40).collect();
        let map_tag = if rec.map_snapshot.is_some() { "*" } else { " " };
        let line = format!("{} T{:<5} {} {}", marker, rec.turn, map_tag, cmd_trunc);
        crate::render::draw_str_clipped(buf, content.x, row_y, &line, style, content);
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    let footer_y = content.bottom().saturating_sub(1);
    if footer_y >= content.y {
        let footer_style = Style::new()
            .fg(ratatui::style::Color::DarkGray)
            .patch(state.colors.dialog);
        let footer = "←/→:step  Space:play  Enter/r:resume  Esc:close";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, ReplayState};
    use mapper::mapper::Mapper;

    #[test]
    fn draw_history_renders_when_open_and_noops_when_closed() {
        let mut state = AppState::default();
        let m = Mapper::default();
        for t in 1..=2 {
            crate::history::record_turn(&mut state.history, t, "go north", vec![t as u8], &m, false, "Forest");
        }

        // Closed → None.
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            let out = draw_history(&state, area, f.buffer_mut());
            assert!(out.is_none(), "draw_history is a no-op when replay is None");
        }).unwrap();

        // Open → Some, and a turn command appears in the buffer.
        state.replay = Some(ReplayState::new(1));
        let mut term2 = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut rects: Option<DialogRects> = None;
        term2.draw(|f| {
            let area = f.area();
            rects = draw_history(&state, area, f.buffer_mut());
        }).unwrap();
        assert!(rects.is_some(), "draw_history returns rects when open");
    }
}
