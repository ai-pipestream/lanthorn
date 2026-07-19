//! Debug-inspector renderer (tiled pane in the map slot). Paints the
//! DebugPanelState snapshot: three tabbed windows (left full height; right
//! split top/bottom), each with its tab strip embedded in its top border row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::debug_panel::{self, DebugPanelState, Section, WINDOW_TABS};
use crate::render::draw_str_clipped;
use crate::render::paneframe::draw_pane_frame;
use crate::state::AppState;

/// Draw one window: frame, tab strip, and the active section's content.
fn draw_window(buf: &mut Buffer, area: Rect, window: usize, panel: &DebugPanelState, state: &AppState) {
    if area.width < 2 || area.height < 2 { return; }
    let focused = panel.focus == window;
    let border = if focused { state.colors.debug_pane_focused } else { state.colors.debug_pane };
    // Tabs are drawn on the top border row; guarantee a border row exists even
    // when the dialog box style resolves to None, or content row 0 would overwrite
    // (hide) the tabs — which are now the primary navigation affordance.
    let box_style = if matches!(state.colors.dialog_box_style, crate::render::paneframe::BorderStyle::None) {
        crate::render::paneframe::BorderStyle::Single
    } else {
        state.colors.dialog_box_style
    };
    let frame = draw_pane_frame(buf, area, box_style, &state.colors.dialog_glyphs, border);

    // Tab strip: embedded in the window's top border row. `tab_hit_rects` is
    // the SAME geometry the mouse click handler uses (crate::debug_panel::tab_at),
    // so a click always lands on the tab actually drawn here.
    let sections = WINDOW_TABS[window];
    let tab_rects = debug_panel::tab_hit_rects(area, sections);
    for (i, (section, rect)) in sections.iter().zip(tab_rects.iter()).enumerate() {
        if rect.width == 0 { continue; }
        let label = format!(" {} ", section.label());
        let style = if i == panel.tab[window] { state.colors.debug_tab_active } else { state.colors.debug_tab };
        draw_str_clipped(buf, rect.x, rect.y, &label, style, *rect);
    }

    // Active section content, clipped to the frame's content rect.
    let section = panel.active_section(window);
    let lines = panel.snapshot.section(section);
    let content = frame.content;
    let body = state.colors.debug_pane;
    // Disasm/Memory are pre-windowed by their addr (offset 0); list sections
    // apply their per-window scroll offset.
    let scroll = match section {
        Section::Disasm | Section::Memory => 0,
        _ => panel.scroll[window],
    };
    let pc_prefix = format!("{:06x}", panel.pc);
    for (row, line) in lines.iter().skip(scroll).take(content.height as usize).enumerate() {
        let y = content.y + row as u16;
        let style = if section == Section::Disasm && line.starts_with(&pc_prefix) {
            state.colors.debug_disasm_pc
        } else {
            body
        };
        draw_str_clipped(buf, content.x, y, line, style, content);
    }
}

pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(panel) = &state.debug else { return };
    let windows = debug_panel::window_rects(area);
    for (i, w) in windows.iter().enumerate() {
        draw_window(buf, *w, i, panel, state);
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
    fn draws_all_three_windows_default_tabs() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.snapshot.disasm = vec!["001000  add".into()];
        panel.snapshot.locals = vec!["local0 = 0001".into()];
        panel.snapshot.stack = vec!["#0 main".into()];
        panel.pc = 0x1000;
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("add"));
        assert!(text.contains("main"));
        // Tab labels for the default (first) tab of each window.
        assert!(text.contains("Disassembly"));
        assert!(text.contains("Locals"));
        assert!(text.contains("Stack"));
    }

    #[test]
    fn highlights_the_pc_line_in_disassembly() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  add".into(), "001004  sub".into()];
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        let content_y = left.y + 1; // first content row under the top border
        let pc_line_modifier = buf.cell((left.x + 1, content_y)).unwrap().style().add_modifier;
        // Compare modifiers only: `Cell::style()` always reports concrete
        // Reset colors for unset fg/bg, so it never equals a Style::default()
        // built with `.add_modifier(...)` alone.
        assert_eq!(pc_line_modifier, state.colors.debug_disasm_pc.add_modifier);
    }

    #[test]
    fn shows_a_non_default_active_tab_and_hides_the_others_content() {
        let mut state = crate::state::AppState::default();
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[0] = 1; // Globals instead of Disassembly
        panel.snapshot.disasm = vec!["001000  should-not-show".into()];
        panel.snapshot.globals = vec!["g00=0012".into()];
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("g00=0012"));
        assert!(!text.contains("should-not-show"));
    }
}
