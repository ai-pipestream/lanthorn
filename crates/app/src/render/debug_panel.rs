//! Debug-inspector renderer (tiled pane in the map slot). Paints the
//! DebugPanelState snapshot: three tabbed windows (left full height; right
//! split top/bottom), each with its tab strip embedded in its top border row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::debug_panel::{self, DebugPanelState, HoverTip, Section, WINDOW_TABS};
use crate::render::draw_str_clipped;
use crate::render::paneframe::draw_pane_frame;
use crate::state::AppState;

/// Redraw the char ranges `clickable_spans` reports within `line` with the
/// UNDERLINED modifier added to `style` (color unchanged), at `x_base + range.start`.
fn underline_clickables(buf: &mut Buffer, x_base: u16, y: u16, line: &str, style: Style, section: Section, area: Rect) {
    for (range, _target) in debug_panel::clickable_spans(section, line) {
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
    // Bound once: the loop below reads these per row.
    let pc_style = state.colors.theme.get("debug.pc").style;
    let exec_style = state.colors.theme.get("debug.disasm_executed").style;
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
            draw_str_clipped(buf, content.x + 1, y, &text, pc_style, content);
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
        draw_str_clipped(buf, content.x, y, marker, exec_style, content);
        draw_str_clipped(buf, content.x + 1, y, line, body, text_rect);
        underline_clickables(buf, content.x + 1, y, line, body, Section::Disasm, text_rect);
    }
}

/// Draw one window: frame, tab strip, and the active section's content.
fn draw_window(buf: &mut Buffer, area: Rect, window: usize, panel: &DebugPanelState, state: &AppState) {
    if area.width < 2 || area.height < 2 { return; }
    let focused = panel.focus == window;
    // Frame colour mirrors the story/map panes: transparent cyan when idle,
    // cyan+bold when focused — never the chrome surface (which bakes a black
    // background into the border cells, clashing with the pane behind them).
    let border = if focused {
        state.colors.theme.get("panel.border:active").style
    } else {
        state.colors.theme.get("panel.border").style
    };
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
    let tab_active = state.colors.theme.get("panel.tab:active").style;
    let tab = state.colors.theme.get("panel.tab").style;
    for (i, (section, rect)) in sections.iter().zip(tab_rects.iter()).enumerate() {
        if rect.width == 0 { continue; }
        let label = format!(" {} ", section.label());
        let style = if i == panel.tab[window] { tab_active } else { tab };
        draw_str_clipped(buf, rect.x, rect.y, &label, style, *rect);
    }

    // Active section content, clipped to the frame's content rect.
    let section = panel.active_section(window);
    let lines = panel.snapshot.section(section);
    let content = frame.content;
    // Body text is the plain text role (foreground only), so the pane's own
    // background shows through instead of a chrome-black block behind every glyph.
    let body = state.colors.theme.get("text").style;

    if section == Section::Disasm {
        draw_disasm(buf, content, panel, state, body);
        return;
    }

    if section == Section::Objects {
        draw_objects(buf, content, window, panel, body);
        return;
    }

    if section == Section::CallStack {
        draw_callstack(buf, content, window, panel, body);
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
    }
}

/// Draw the Objects section: `objects_rows` interleaves each tree line with
/// its expanded detail lines (if any), so render and the click hit-test
/// (`objects_click_at`) always agree on which screen row is which object row.
/// Tree rows that carry an object id get a ▶/▼ disclosure triangle (clickable
/// to toggle); detail rows are drawn plain and indented.
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
                // Disclosure triangle marks a toggleable object row: ▼ when
                // expanded, ▶ when collapsed; a marker-less row (no id) keeps a
                // two-space pad so columns stay aligned.
                let marker = match obj {
                    Some(n) if panel.expanded_objects.contains(n) => "▼ ",
                    Some(_) => "▶ ",
                    None => "  ",
                };
                let text = format!("{marker}{line}");
                draw_str_clipped(buf, content.x, y, &text, body, content);
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

/// Draw the Call Stack section: `stack_rows` interleaves each frame line with
/// its expanded locals detail lines (if any), so render and the two hit-tests
/// (`clickable_at`, `stack_click_at`) always agree on which screen row is which
/// frame line. Frame rows get a ▶/▼ disclosure triangle (a 2-column marker, so
/// the frame text — with its clickable `fn@` address — draws at `content.x + 2`);
/// detail rows are drawn plain and indented.
fn draw_callstack(buf: &mut Buffer, content: Rect, window: usize, panel: &DebugPanelState, body: Style) {
    let rows = debug_panel::stack_rows(
        &panel.snapshot.stack, &panel.expanded_frames, &panel.snapshot.frame_details,
        panel.scroll[window], content.height as usize,
    );
    for (r, row_entry) in rows.iter().enumerate() {
        let y = content.y + r as u16;
        match row_entry {
            debug_panel::StackRow::Frame { line_idx, frame } => {
                let Some(line) = panel.snapshot.stack.get(*line_idx) else { continue };
                let marker = match frame {
                    Some(n) if panel.expanded_frames.contains(n) => "▼ ",
                    Some(_) => "▶ ",
                    None => "  ", // e.g. the "(no frames)" line
                };
                draw_str_clipped(buf, content.x, y, marker, body, content);
                draw_str_clipped(buf, content.x + 2, y, line, body, content);
                underline_clickables(buf, content.x + 2, y, line, body, Section::CallStack, content);
            }
            debug_panel::StackRow::Detail { frame, di } => {
                let Some(det) = panel.snapshot.frame_details.get(frame) else { continue };
                let Some(line) = det.get(*di) else { continue };
                let indented = format!("    {line}");
                draw_str_clipped(buf, content.x, y, &indented, body, content);
            }
        }
    }
}

/// Draw the Memory section: an address line always occupies the top content
/// row so the jump affordance is discoverable — it shows the current address
/// (with a `press : to jump` hint) when idle, and becomes the edit field with
/// a `_` cursor while `mem_input` is editing. The hex dump fills the rows
/// below it. Memory is pre-windowed by its addr, so it never applies a scroll
/// offset.
fn draw_memory(buf: &mut Buffer, content: Rect, panel: &DebugPanelState, state: &AppState, body: Style) {
    let (line, style) = match &panel.mem_input {
        Some(input) => (format!("jump: {input}_"), state.colors.theme.get("panel.border:active").style),
        None => (format!("addr: 0x{:06x}  (: jump — hex, gNN, localN, sp)", panel.mem_addr), body),
    };
    draw_str_clipped(buf, content.x, content.y, &line, style, content);
    let height = content.height.saturating_sub(1);
    for (row, hex) in panel.snapshot.memory.iter().take(height as usize).enumerate() {
        draw_str_clipped(buf, content.x, content.y + 1 + row as u16, hex, body, content);
    }
}

/// Draw the floating variable-value tooltip on top of the windows. Preferred
/// position is just below the hovered token; the box is clamped inside `area`
/// (shifted left / flipped above as needed) and simply skipped if `area` is too
/// small to hold it (never panics).
fn draw_tooltip(buf: &mut Buffer, area: Rect, tip: &HoverTip, state: &AppState) {
    let inner = tip.lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let h = tip.lines.len() as u16;
    let w = inner + 2; // one space of horizontal padding on each side
    if h == 0 || area.width < w || area.height < h { return; }

    // Preferred: just below the token; clamp into `area` (flip above if needed).
    let mut x = tip.col;
    let mut y = tip.row + 1;
    if x + w > area.right() { x = area.right().saturating_sub(w); }
    if y + h > area.bottom() { y = tip.row.saturating_sub(h); }
    x = x.max(area.x);
    y = y.max(area.y);

    let box_rect = Rect::new(x, y, w, h);
    let style = state.colors.theme.get("debug.tooltip").style;
    // Reset every cell the box covers before drawing: draw_char_clipped PATCHES
    // cell styles, so a modifier already on the disasm underneath (e.g. the
    // UNDERLINED on a clickable operand) would otherwise bleed through the
    // tooltip. A clean reset makes the borderless box fully opaque.
    for yy in box_rect.y..box_rect.bottom() {
        for xx in box_rect.x..box_rect.right() {
            if let Some(cell) = buf.cell_mut((xx, yy)) { cell.reset(); }
        }
    }
    // Borderless: fill each row with the tooltip background, then the padded text.
    let pad: String = " ".repeat(w as usize);
    for (i, line) in tip.lines.iter().enumerate() {
        let ry = y + i as u16;
        draw_str_clipped(buf, x, ry, &pad, style, box_rect);
        draw_str_clipped(buf, x + 1, ry, line, style, box_rect);
    }
}

pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(panel) = &state.debug else { return };
    // Interior fill: the standard panel surface (§2a). Transparent by default so
    // the terminal background shows through; a themed `panel.background` bg paints
    // the whole pane as a solid surface, with borders/text composing on top (their
    // transparent-bg styles patch the cell, preserving this fill).
    let surface = state.colors.theme.get("panel.background").style;
    for yy in area.top()..area.bottom() {
        for xx in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((xx, yy)) {
                c.set_symbol(" ").set_style(surface);
            }
        }
    }
    let windows = debug_panel::window_rects(area);
    for (i, w) in windows.iter().enumerate() {
        draw_window(buf, *w, i, panel, state);
    }
    // Mouse selection (SQ-0420): reverse-video the selected cells and publish the
    // copy text read back from the drawn buffer, so a mouse-release can emit it via
    // OSC 52 — mirrors the story pane's highlight+extract. The selection lives in
    // the window's content-relative coordinates.
    if let Some((win, sel)) = panel.sel {
        if !sel.is_empty() {
            if let Some(content) = debug_panel::window_content(area, win) {
                let mut out: Vec<String> = Vec::new();
                for r in 0..content.height {
                    if let Some((c0, c1)) = crate::clipboard::row_span(content.width, sel, r as usize) {
                        let mut line = String::new();
                        for c in c0..=c1 {
                            if let Some(cell) = buf.cell_mut((content.x + c, content.y + r)) {
                                let s = cell.style();
                                cell.set_style(s.add_modifier(ratatui::style::Modifier::REVERSED));
                                line.push_str(cell.symbol());
                            }
                        }
                        out.push(line.trim_end().to_string());
                    }
                }
                *panel.selection_text.borrow_mut() = Some(out.join("\n"));
            }
        }
    }
    // Tooltip paints on top of the windows.
    if let Some(tip) = &panel.hover { draw_tooltip(buf, area, tip, state); }
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
    fn selection_highlights_cells_and_publishes_copy_text() {
        // SQ-0420: a selection over the Locals list (window 1, no synthetic rows)
        // reverse-videos the selected cells and publishes the logical lines as the
        // copy text read back from the drawn buffer.
        use ratatui::style::Modifier;
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.snapshot.locals = vec!["local0 = 0001".into(), "local1 = 0002".into()];
        // Window 1 (right-top) shows Locals by default; select its first two content
        // rows in full (cols 0..=12 covers "local0 = 0001").
        panel.sel = Some((1, crate::clipboard::Selection {
            anchor: crate::clipboard::Point { row: 0, col: 0 },
            head: crate::clipboard::Point { row: 1, col: 12 },
        }));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let copied = state.debug.as_ref().unwrap().selection_text.borrow().clone();
        assert_eq!(copied.as_deref(), Some("local0 = 0001\nlocal1 = 0002"));
        // Window 1 content starts at (41,1); the first selected cell is reverse-video.
        let cell = buf.cell((41, 1)).expect("cell in window 1 content");
        assert!(cell.modifier.contains(Modifier::REVERSED), "selected cell must be reversed");
        // A cell outside the selection (window 0) is not reversed.
        let other = buf.cell((2, 1)).expect("cell in window 0");
        assert!(!other.modifier.contains(Modifier::REVERSED), "unselected cell not reversed");
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
        assert_eq!(divider_modifier, state.colors.theme.get("debug.pc").style.add_modifier);

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

    #[test]
    fn objects_show_disclosure_triangles_and_no_underline() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.tab[1] = 1; // Objects tab (window 1)
        panel.snapshot.objects = vec!["[1] lamp".into(), "[2] rock".into()];
        panel.expanded_objects = std::collections::HashSet::from([1u16]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("▼"), "expanded object gets a ▼ marker");
        assert!(text.contains("▶"), "collapsed object gets a ▶ marker");

        // The tree line is no longer underlined: the first glyph of "[1] lamp"
        // (drawn just after the "▼ " marker) carries no UNDERLINED modifier.
        let [_, top, _] = crate::debug_panel::window_rects(area);
        let content = Rect::new(top.x + 1, top.y + 1, top.width.saturating_sub(2), top.height.saturating_sub(2));
        let cell = buf.cell((content.x + 2, content.y)).unwrap(); // past "▼ "
        assert_eq!(cell.symbol(), "[");
        assert!(!cell.style().add_modifier.contains(Modifier::UNDERLINED), "tree line must not be underlined");
    }

    #[test]
    fn callstack_shows_disclosure_triangles_and_indents_expanded_locals() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        // Window 2 tab 0 = Call Stack (the default).
        panel.snapshot.stack = vec![
            "#0  fn@004a00  ret=004a35  args=2".into(),
            "#1  fn@005000  ret=005035  args=0".into(),
        ];
        panel.expanded_frames = std::collections::HashSet::from([0usize]);
        panel.snapshot.frame_details = std::collections::HashMap::from([
            (0usize, vec!["local0 = 0x0001  (1)".to_string()]),
        ]);
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("▼"), "expanded frame gets a ▼ marker");
        assert!(text.contains("▶"), "collapsed frame gets a ▶ marker");
        assert!(text.contains("local0 = 0x0001  (1)"), "expanded frame's locals appear");

        // The frame text draws at content.x + 2 (past the "▼ " marker); its first
        // glyph is '#'. The detail row below (row 1) is indented by four spaces.
        let [_, _, bot] = crate::debug_panel::window_rects(area);
        let content = Rect::new(bot.x + 1, bot.y + 1, bot.width.saturating_sub(2), bot.height.saturating_sub(2));
        assert_eq!(buf.cell((content.x + 2, content.y)).unwrap().symbol(), "#");
        assert_eq!(buf.cell((content.x, content.y)).unwrap().symbol(), "▼");
        // Detail row (row 1) indented: first four cells blank, then the local.
        assert_eq!(buf.cell((content.x + 4, content.y + 1)).unwrap().symbol(), "l");
    }

    #[test]
    fn draws_the_hover_tooltip_value_text() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  loadw g0f -> sp".into()];
        panel.hover = Some(crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 5, 5));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("g0f = 0x1234"), "tooltip value text is painted");
    }

    #[test]
    fn hover_tooltip_is_opaque_and_clears_underline_underneath() {
        let mut state = crate::state::AppState::default();
        state.colors.dialog_box_style = crate::render::paneframe::BorderStyle::Single;
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.pc = 0x1000;
        panel.snapshot.disasm = vec!["001000  loadw g0f -> sp".into()];
        panel.hover = Some(crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 5, 5));
        state.debug = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        // Pre-underline everything, as clickable-span underlines would leave the
        // disasm cells beneath the tooltip.
        for cell in buf.content.iter_mut() {
            cell.set_style(Style::new().add_modifier(Modifier::UNDERLINED));
        }
        draw_debug_panel(&state, area, &mut buf);

        // The tooltip anchors at (5,5) → box at (5,6); its first text cell (6,6)
        // is the 'g' of "g0f = …" and must not carry a bled-through underline.
        let cell = buf.cell((6, 6)).expect("cell in bounds");
        assert_eq!(cell.symbol(), "g", "tooltip text painted here");
        assert!(!cell.style().add_modifier.contains(Modifier::UNDERLINED),
            "underline underneath must not bleed through the tooltip");
    }

    #[test]
    fn hover_tooltip_clamps_at_the_bottom_right_corner_without_panicking() {
        let state = crate::state::AppState::default();
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        // Anchor at the very bottom-right corner: the box must flip/shift inside
        // `area` rather than draw out of bounds and panic.
        let tip = crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), area.right() - 1, area.bottom() - 1);
        draw_tooltip(&mut buf, area, &tip, &state);
        assert!(buf_text(&buf).contains("g0f = 0x1234"), "clamped tooltip still paints");
    }

    #[test]
    fn hover_tooltip_skips_a_too_small_area_without_panicking() {
        let state = crate::state::AppState::default();
        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        let tip = crate::debug_panel::HoverTip::for_var(0x1f, Some(0x1234), 0, 0);
        draw_tooltip(&mut buf, area, &tip, &state); // must not panic
    }
}
