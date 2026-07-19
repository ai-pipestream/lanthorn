//! Debug-inspector renderer (tiled pane in the map slot). Paints the
//! DebugPanelState snapshot: three tabbed windows (left full height; right
//! split top/bottom), each with its tab strip embedded in its top border row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::debug_panel::{self, DebugPanelState, Section, WINDOW_TABS};
use crate::render::draw_str_clipped;
use crate::render::paneframe::draw_pane_frame;
use crate::state::AppState;

/// Redraw the char ranges `clickable_spans` reports within `line` with the
/// UNDERLINED modifier added to `style` (color unchanged), at `x_base + range.start`.
fn underline_clickables(buf: &mut Buffer, x_base: u16, y: u16, line: &str, style: Style, section: Section, area: Rect) {
    for (range, _addr) in debug_panel::clickable_spans(section, line) {
        let Some(sub) = line.get(range.clone()) else { continue };
        let x = x_base + range.start as u16;
        draw_str_clipped(buf, x, y, sub, style.add_modifier(Modifier::UNDERLINED), area);
    }
}

/// Draw the Disassembly section: `disasm_rows` inserts a PC-divider row
/// directly above the instruction at `pc`, so render and the click hit-test
/// (`clickable_at`) always agree on which screen row is which disasm line.
fn draw_disasm(buf: &mut Buffer, content: Rect, panel: &DebugPanelState, state: &AppState, body: Style) {
    let disasm = &panel.snapshot.disasm;
    let rows = debug_panel::disasm_rows(disasm, panel.pc, content.height as usize);
    let text_rect = Rect::new(content.x + 1, content.y, content.width.saturating_sub(1), content.height);
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        if row_entry.divider {
            let width = content.width.saturating_sub(1) as usize;
            let core = "▼── PC ──▼";
            let text: String = if core.chars().count() >= width {
                core.chars().take(width).collect()
            } else {
                let mut s = core.to_string();
                s.push_str(&"─".repeat(width - core.chars().count()));
                s
            };
            draw_str_clipped(buf, content.x + 1, y, &text, state.colors.debug_disasm_pc, content);
            continue;
        }
        let Some(line) = disasm.get(row_entry.line_idx) else { continue };
        // 1-column execution-coverage gutter: `|` when this line's leading
        // address ran during the last command turn, else blank. Text is
        // drawn one column in so the gutter never overlaps it.
        let marked = line.get(0..6)
            .and_then(|a| u32::from_str_radix(a, 16).ok())
            .is_some_and(|addr| panel.snapshot.executed.contains(&addr));
        let marker = if marked { "|" } else { " " };
        draw_str_clipped(buf, content.x, y, marker, state.colors.debug_exec_mark, content);
        draw_str_clipped(buf, content.x + 1, y, line, body, text_rect);
        underline_clickables(buf, content.x + 1, y, line, body, Section::Disasm, text_rect);
    }
}

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

    if section == Section::Disasm {
        draw_disasm(buf, content, panel, state, body);
        return;
    }

    if section == Section::Objects {
        draw_objects(buf, content, window, panel, body);
        return;
    }

    if section == Section::Memory {
        draw_memory(buf, content, panel, state, body);
        return;
    }

    // List sections apply their per-window scroll offset.
    let scroll = panel.scroll[window];
    for (row, line) in lines.iter().skip(scroll).take(content.height as usize).enumerate() {
        let y = content.y + row as u16;
        draw_str_clipped(buf, content.x, y, line, body, content);
        if section == Section::CallStack {
            underline_clickables(buf, content.x, y, line, body, section, content);
        }
    }
}

/// Draw the Objects section: `objects_rows` interleaves each tree line with
/// its expanded detail lines (if any), so render and the click hit-test
/// (`objects_click_at`) always agree on which screen row is which object row.
/// Tree rows that carry an object id are underlined (clickable to toggle);
/// detail rows are drawn plain and indented.
fn draw_objects(buf: &mut Buffer, content: Rect, window: usize, panel: &DebugPanelState, body: Style) {
    let rows = debug_panel::objects_rows(
        &panel.snapshot.objects, &panel.expanded_objects, &panel.snapshot.object_details,
        panel.scroll[window], content.height as usize,
    );
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        match row_entry {
            debug_panel::ObjRow::Tree { line_idx, obj } => {
                let Some(line) = panel.snapshot.objects.get(*line_idx) else { continue };
                draw_str_clipped(buf, content.x, y, line, body, content);
                if obj.is_some() {
                    draw_str_clipped(buf, content.x, y, line, body.add_modifier(Modifier::UNDERLINED), content);
                }
            }
            debug_panel::ObjRow::Detail { obj, di } => {
                let Some(det) = panel.snapshot.object_details.get(obj) else { continue };
                let Some(line) = det.get(*di) else { continue };
                let indented = format!("    {line}");
                draw_str_clipped(buf, content.x, y, &indented, body, content);
            }
        }
    }
}

/// Draw the Memory section: when `mem_input` is editing, an address-input
/// line occupies the top content row (with a `_` cursor) and the hex dump is
/// drawn below it (content height − 1); otherwise the hex dump fills the
/// whole content rect. Memory is pre-windowed by its addr, so it never
/// applies a scroll offset.
fn draw_memory(buf: &mut Buffer, content: Rect, panel: &DebugPanelState, state: &AppState, body: Style) {
    let mut y = content.y;
    let mut height = content.height;
    if let Some(input) = &panel.mem_input {
        let line = format!("addr: 0x{input}_");
        draw_str_clipped(buf, content.x, y, &line, state.colors.debug_pane_focused, content);
        y += 1;
        height = height.saturating_sub(1);
    }
    for (row, line) in panel.snapshot.memory.iter().take(height as usize).enumerate() {
        draw_str_clipped(buf, content.x, y + row as u16, line, body, content);
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
    fn draws_a_pc_divider_row_above_the_pc_instruction() {
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
        // content.x = left.x + 1 (border inset); divider text is drawn at
        // content.x + 1 = left.x + 2, same column as regular disasm text.
        // Row 0 is the divider (drawn ABOVE the PC line); row 1 is the actual
        // "001000  add" instruction line, shifted down by the divider.
        let divider_row: String = (left.x + 2..left.x + 2 + 10)
            .map(|x| buf.cell((x, content_y)).unwrap().symbol().to_string())
            .collect();
        assert!(divider_row.starts_with("▼── PC ──▼"), "got {divider_row:?}");
        let divider_modifier = buf.cell((left.x + 2, content_y)).unwrap().style().add_modifier;
        // Compare modifiers only: `Cell::style()` always reports concrete
        // Reset colors for unset fg/bg, so it never equals a Style::default()
        // built with `.add_modifier(...)` alone.
        assert_eq!(divider_modifier, state.colors.debug_disasm_pc.add_modifier);

        let text = buf_text(&buf);
        assert!(text.contains("add"), "PC line still rendered below the divider");
    }

    #[test]
    fn marks_executed_disasm_lines_with_a_gutter_bar() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  add".into(), "001004  sub".into()];
        panel.snapshot.executed = std::collections::HashSet::from([0x1000]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let [left, ..] = crate::debug_panel::window_rects(area);
        // Row 0 is the PC divider; row 1 is "001000 add" (executed); row 2 is
        // "001004 sub" (not executed).
        let content_y = left.y + 1 + 1;
        // left.x + 1 is the execution-marker gutter column (content.x); line
        // text starts one column further in.
        // Executed line (0x1000): gutter column shows the marker.
        assert_eq!(buf.cell((left.x + 1, content_y)).unwrap().symbol(), "|");
        // Not-executed line (0x1004): gutter column is blank.
        assert_eq!(buf.cell((left.x + 1, content_y + 1)).unwrap().symbol(), " ");
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
