//! Command-palette popup overlay (SQ-0419).
//!
//! A fuzzy-searchable list of every registry command, drawn with the shared
//! dialog chrome. The top content row is the palette's own input line (query +
//! args); below it is the ranked candidate list, matched characters highlighted,
//! the selected row emphasised. Works anywhere — even where no story prompt
//! exists (modal / debug views).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::complete::palette_candidates;
use crate::render::dialog::{
    draw_dialog, DialogField, DialogRects, DialogSpec, DialogStyle, Placement,
};
use crate::render::{draw_str_clipped, put_char};
use crate::state::AppState;

/// Draw the command palette centered over `area`.
///
/// `vp_out` receives the candidate-list viewport height (rows) so nav actions can
/// window/animate the scroll. `hits` is filled with `(cmd_index, row_rect)` for
/// every drawn candidate row so a click can execute that command.
///
/// Does nothing when `state.overlays.palette` is `None`. Returns `Some(DialogRects)`
/// when drawn (for `[X]` / outside-click hit-testing), `None` otherwise.
pub fn draw_palette(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
    hits: &mut Vec<(usize, Rect)>,
) -> Option<DialogRects> {
    let Some(palette) = &state.overlays.palette else { return None };

    let cands = palette_candidates(palette.query());

    // ── Modal geometry ────────────────────────────────────────────────────────
    let modal_w = 72u16.min(area.width.saturating_sub(4));
    // 1 field row + candidate rows + 1 footer + border(2). Cap the list height so
    // a huge registry never overflows the screen.
    let list_rows = (cands.len() as u16).clamp(1, 14);
    let modal_h = (list_rows + 4).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 5 {
        return None;
    }

    // ── Dialog chrome + input field ───────────────────────────────────────────
    let st = DialogStyle::from_colors(&state.colors);
    let theme = &state.colors.theme;
    let query_style = theme.get("palette_query").style;
    let caret_style = query_style.add_modifier(Modifier::REVERSED);

    let field = DialogField {
        label: "/",
        value: &palette.input.value,
        cursor: palette.input.cursor,
        show_caret: true,
        dim: false,
        text_style: query_style,
        dim_style: query_style.add_modifier(Modifier::DIM),
        caret_style,
    };

    let spec = DialogSpec {
        title: "Command Palette  (/ commands)",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
        field: Some(field),
    };

    let rects = draw_dialog(buf, area, &spec, &st);
    let content = rects.content;
    if content.height == 0 {
        return Some(rects);
    }

    // Content layout: row 0 = the input field (drawn by draw_dialog); the last
    // row = footer hint; the candidate list fills the middle.
    let footer_y = content.bottom().saturating_sub(1);
    let list_top = content.y + 1;
    let list_h = footer_y.saturating_sub(list_top);
    let list_area = Rect::new(content.x, list_top, content.width, list_h);
    *vp_out = list_area.height as usize;

    // ── Candidate rows ────────────────────────────────────────────────────────
    let name_style = theme.get("palette_name").style;
    let match_style = theme.get("palette_match").style;
    let desc_style = theme.get("palette_desc").style;
    let sel_style = theme.get("palette_selected").style;

    if cands.is_empty() {
        draw_str_clipped(buf, list_area.x, list_area.y, "(no matching commands)", desc_style, list_area);
    } else {
        let total = cands.len();
        let viewport = list_area.height as usize;
        let scrollbar_visible =
            crate::render::scroll::needs_scrollbar(total, viewport) && list_area.width >= 2;
        let row_w = if scrollbar_visible { list_area.width.saturating_sub(1) } else { list_area.width };
        let offset = palette.scroll.display_offset();
        let selected = palette.scroll.selected;

        for row in 0..viewport {
            let i = offset + row;
            if i >= total {
                break;
            }
            let cand = &cands[i];
            let spec = &crate::slash::COMMANDS[cand.cmd_index];
            let row_y = list_area.y + row as u16;
            let is_sel = i == selected;
            let base = if is_sel { sel_style } else { name_style };

            // Fill the row so the selection highlight spans its full width.
            for col in list_area.x..list_area.x + row_w {
                if let Some(cell) = buf.cell_mut((col, row_y)) {
                    cell.set_symbol(" ").set_style(base);
                }
            }
            hits.push((cand.cmd_index, Rect::new(list_area.x, row_y, row_w, 1)));

            // Command name, matched chars highlighted.
            let name_x = list_area.x + 1;
            let name_right = name_x + spec.name.chars().count() as u16;
            for (ci, ch) in spec.name.chars().enumerate() {
                let x = name_x + ci as u16;
                let matched = cand.positions.contains(&ci);
                let style = match (is_sel, matched) {
                    (true, true) => sel_style.add_modifier(Modifier::BOLD),
                    (true, false) => sel_style,
                    (false, true) => match_style,
                    (false, false) => name_style,
                };
                put_char(buf, x as i32, row_y as i32, ch, style, list_area);
            }

            // One-line help, right of the name column (aligned to col 26).
            let desc_x = name_x.max(name_right + 1).max(list_area.x + 26);
            if desc_x < list_area.x + row_w {
                let d_style = if is_sel { sel_style } else { desc_style };
                let avail = (list_area.x + row_w - desc_x) as usize;
                let desc: String = spec.description.chars().take(avail).collect();
                let clip = Rect::new(desc_x, row_y, list_area.x + row_w - desc_x, 1);
                draw_str_clipped(buf, desc_x, row_y, &desc, d_style, clip);
            }
        }

        if scrollbar_visible {
            let sb = Rect::new(list_area.right().saturating_sub(1), list_area.y, 1, list_area.height);
            crate::render::scroll::draw_scrollbar(
                buf,
                sb,
                total,
                viewport,
                palette.scroll.target_offset(),
                crate::render::scroll::ScrollbarLook::from_theme(theme),
            );
        }
    }

    // ── Footer hint ───────────────────────────────────────────────────────────
    if footer_y > content.y {
        let footer = "\u{2191}\u{2193} move  Tab complete  Enter run  Esc close";
        draw_str_clipped(buf, content.x, footer_y, footer, desc_style, content);
    }

    Some(rects)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, PaletteState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buf_text(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect()
    }

    fn state_with_palette(query: &str) -> AppState {
        let mut s = AppState::default();
        let mut p = PaletteState::new(false);
        p.input = crate::text_field::TextField::new(query.to_string());
        s.overlays.palette = Some(p);
        s
    }

    #[test]
    fn draw_palette_noop_when_closed() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default();
        let before: Vec<_> = terminal.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect();
        terminal.draw(|f| { draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut Vec::new()); }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect();
        assert_eq!(before, after, "draw_palette must be a no-op when the palette is closed");
    }

    #[test]
    fn draw_palette_lists_matching_commands_and_chrome() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette("zoom");
        let mut rects = None;
        let mut hits = Vec::new();
        terminal.draw(|f| { rects = draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut hits); }).unwrap();
        let text = buf_text(terminal.backend().buffer());
        assert!(text.contains("Command Palette"), "title should be present");
        assert!(text.contains("zoom-map"), "zoom-map should be listed for query 'zoom'");
        assert!(text.contains('\u{2715}'), "[X] close button should be visible");
        let rects = rects.expect("palette open should return rects");
        assert!(rects.close.is_some());
        // Row hits recorded, and zoom-map is among them.
        assert!(hits.iter().any(|(i, _)| crate::slash::COMMANDS[*i].name == "zoom-map"));
    }

    #[test]
    fn draw_palette_shows_no_match_message() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = state_with_palette("zzzznotacommand");
        terminal.draw(|f| { draw_palette(&state, f.area(), f.buffer_mut(), &mut 0, &mut Vec::new()); }).unwrap();
        let text = buf_text(terminal.backend().buffer());
        assert!(text.contains("no matching"), "expected a no-match message");
    }
}
