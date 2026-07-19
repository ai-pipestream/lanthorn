//! Full-screen debug-inspector renderer. Paints the DebugPanelState snapshot.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::debug_panel::{DebugPane, DebugView};
use crate::render::draw_str_clipped;
use crate::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment};
use crate::state::AppState;

/// One titled pane: border in the focused/unfocused style, snapshot lines inside.
fn draw_pane(
    buf: &mut Buffer, area: Rect, title: &str, lines: &[String], scroll: usize,
    focused: bool, state: &AppState,
) {
    if area.width < 2 || area.height < 2 { return; }
    let border = if focused { state.colors.debug_pane_focused } else { state.colors.debug_pane };
    let pane = draw_pane_frame(buf, area, state.colors.dialog_box_style, &state.colors.dialog_glyphs, border);
    draw_top_inset(buf, pane.top_inset, &[InsetSegment { text: title, active: focused }],
        state.colors.debug_title, state.colors.debug_title);
    let content = pane.content;
    let body = state.colors.debug_pane;
    for (row, line) in lines.iter().skip(scroll).take(content.height as usize).enumerate() {
        draw_str_clipped(buf, content.x, content.y + row as u16, line, body, content);
    }
}

pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(panel) = &state.overlays.debug_panel else { return };
    let view = panel.focus.view();

    // Left column full height; right column split into two stacked panes.
    let left_w = area.width / 2;
    let right_x = area.x + left_w;
    let right_w = area.width - left_w;
    let top_h = area.height / 2;
    let left = Rect::new(area.x, area.y, left_w, area.height);
    let r_top = Rect::new(right_x, area.y, right_w, top_h);
    let r_bot = Rect::new(right_x, area.y + top_h, right_w, area.height - top_h);

    let s = &panel.snapshot;
    let f = panel.focus;
    let ls = panel.list_scroll;
    // Which pane is focused decides where list_scroll applies; address panes
    // (disasm/memory) scroll via their addr, so pass 0 for their offset.
    match view {
        DebugView::Execution => {
            draw_pane(buf, left, " Disassembly ", &s.disasm, 0, f == DebugPane::Disasm, state);
            draw_pane(buf, r_top, " Locals ", &s.locals, if f == DebugPane::Locals { ls } else { 0 }, f == DebugPane::Locals, state);
            draw_pane(buf, r_bot, " Stack ", &s.stack, if f == DebugPane::Stack { ls } else { 0 }, f == DebugPane::Stack, state);
        }
        DebugView::WorldState => {
            draw_pane(buf, left, " Globals ", &s.globals, if f == DebugPane::Globals { ls } else { 0 }, f == DebugPane::Globals, state);
            // Right-top shows Objects; when Dict/Memory focused it takes the top slot.
            let (top_title, top_lines, top_pane) = match f {
                DebugPane::Dict => (" Dictionary ", &s.dict, DebugPane::Dict),
                DebugPane::Memory => (" Memory ", &s.memory, DebugPane::Memory),
                _ => (" Objects ", &s.objects, DebugPane::Objects),
            };
            let off = if f == top_pane && top_pane != DebugPane::Memory { ls } else { 0 };
            draw_pane(buf, r_top, top_title, top_lines, off, f == top_pane, state);
            // Right-bottom shows the other of Objects/Dictionary for context.
            let bot = if top_pane == DebugPane::Objects { (" Dictionary ", &s.dict) } else { (" Objects ", &s.objects) };
            draw_pane(buf, r_bot, bot.0, bot.1, 0, false, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_text(buf: &Buffer) -> String {
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn draws_execution_view_panes() {
        let mut state = crate::state::AppState::default();
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.snapshot.disasm = vec!["1000  add".into()];
        panel.snapshot.locals = vec!["local0 = 0001".into()];
        panel.snapshot.stack = vec!["#0 main".into()];
        state.overlays.debug_panel = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("add"));
        assert!(text.contains("main"));
    }
}
