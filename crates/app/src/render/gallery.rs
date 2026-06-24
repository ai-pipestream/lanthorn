//! Full-screen symbol gallery modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::{AppState, GalleryState, GALLERY_CATEGORY_NAMES};
use crate::symbols::{Arrows, BoxStyle, PathGlyphs, PortalGlyphs};

/// Draw the full-screen symbol gallery modal.
pub fn draw_gallery(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(gallery) = &state.gallery else { return };

    // Fill background.
    let bg = Style::new().fg(Color::White).bg(Color::Black);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(bg);
            }
        }
    }

    // Title.
    let title = "Symbol Gallery  Esc/Enter: close  \u{2190}\u{2192}: category  \u{2191}\u{2193}: preset";
    let title_style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    crate::render::draw_str_clipped(buf, area.x + 1, area.y, title, title_style, area);

    if area.height < 3 {
        return;
    }

    // Footer hint: the "Output all settings" export button.
    let footer = "o: Output all settings";
    let footer_style = Style::new().fg(Color::Yellow);
    crate::render::draw_str_clipped(buf, area.x + 1, area.bottom() - 1, footer, footer_style, area);

    let content_area = Rect { y: area.y + 1, height: area.height.saturating_sub(2), ..area };

    // Left pane: 20 cols for categories.
    let left_w = 20u16.min(area.width / 3);
    let left_area = Rect { x: content_area.x, y: content_area.y, width: left_w, height: content_area.height };
    let right_area = Rect {
        x: content_area.x + left_w,
        y: content_area.y,
        width: content_area.width.saturating_sub(left_w),
        height: content_area.height,
    };

    draw_category_pane(gallery, left_area, buf);
    draw_preset_pane(state, gallery, right_area, buf);
}

fn draw_category_pane(gallery: &GalleryState, area: Rect, buf: &mut Buffer) {
    let normal = Style::new().fg(Color::White);
    let active = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    for (i, name) in GALLERY_CATEGORY_NAMES.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.bottom() {
            break;
        }
        let style = if i == gallery.category_idx { active } else { normal };
        let marker = if i == gallery.category_idx { ">" } else { " " };
        let line = format!("{} {}", marker, name);
        crate::render::draw_str_clipped(buf, area.x, y, &line, style, area);
    }
}

fn draw_preset_pane(state: &AppState, gallery: &GalleryState, area: Rect, buf: &mut Buffer) {
    let cat = gallery.category_idx;
    let preset_names: &[&str] = match cat {
        0 => BoxStyle::preset_names(),
        1 => Arrows::preset_names(),
        2 => PortalGlyphs::preset_names(),
        _ => PathGlyphs::preset_names(),
    };

    let selected_idx = gallery.selections[cat];
    let normal = Style::new().fg(Color::White);
    let selected = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD);

    // Category header.
    crate::render::draw_str_clipped(
        buf, area.x, area.y, GALLERY_CATEGORY_NAMES[cat],
        Style::new().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
        area,
    );

    // Preset list.
    for (i, name) in preset_names.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y >= area.bottom() {
            break;
        }
        let style = if i == selected_idx { selected } else { normal };
        let marker = if i == selected_idx { "[x]" } else { "[ ]" };
        let line = format!("{} {}", marker, name);
        crate::render::draw_str_clipped(buf, area.x, y, &line, style, area);
    }

    // Preview: draw a small synthetic map below the preset list.
    let preview_y = area.y + 1 + preset_names.len() as u16 + 1;
    if preview_y + 2 < area.bottom() && area.width > 10 {
        let preview_area = Rect {
            x: area.x,
            y: preview_y,
            width: area.width,
            height: area.bottom() - preview_y,
        };
        draw_preview(state, gallery, preview_area, buf);
    }
}

fn draw_preview(state: &AppState, gallery: &GalleryState, area: Rect, buf: &mut Buffer) {
    use crate::symbols::SymbolSet;
    let _ = state; // not needed for v1 preview, kept for signature consistency

    crate::render::draw_str_clipped(buf, area.x, area.y, "Preview:", Style::new().fg(Color::DarkGray), area);
    if area.height < 2 {
        return;
    }

    // Build preview symbol set from current gallery selections.
    let cfg = gallery.symbol_config();
    let preview_symbols = SymbolSet::resolve(&cfg);
    let bs = &preview_symbols.room_normal;

    // Draw a tiny room box: 7 wide x 3 tall.
    let bw = 7u16.min(area.width.saturating_sub(2));
    let bh = 3u16;
    if area.height < 2 + bh {
        return;
    }

    let bx = area.x + 1;
    let by = area.y + 1;
    let style = Style::new().fg(Color::White);

    // Top row.
    crate::render::draw_char_clipped(buf, bx, by, bs.tl, style, area);
    for x in (bx + 1)..(bx + bw - 1) {
        crate::render::draw_char_clipped(buf, x, by, bs.h, style, area);
    }
    crate::render::draw_char_clipped(buf, bx + bw - 1, by, bs.tr, style, area);

    // Middle rows.
    for row in 1..(bh - 1) {
        crate::render::draw_char_clipped(buf, bx, by + row, bs.v, style, area);
        crate::render::draw_char_clipped(buf, bx + bw - 1, by + row, bs.v, style, area);
    }

    // Bottom row.
    crate::render::draw_char_clipped(buf, bx, by + bh - 1, bs.bl, style, area);
    for x in (bx + 1)..(bx + bw - 1) {
        crate::render::draw_char_clipped(buf, x, by + bh - 1, bs.h, style, area);
    }
    crate::render::draw_char_clipped(buf, bx + bw - 1, by + bh - 1, bs.br, style, area);

    // Show path glyph to the right of the box.
    let arrow_x = bx + bw;
    if arrow_x < area.right() {
        crate::render::draw_char_clipped(buf, arrow_x, by + 1, preview_symbols.path.ew, Style::new().fg(Color::Cyan), area);
    }
    // Show portal marker inside box.
    if bw > 2 {
        crate::render::draw_char_clipped(buf, bx + 1, by + 1, preview_symbols.portal.marker, Style::new().fg(Color::Cyan), area);
    }
    // Show arrow.
    let arrow_x2 = bx + bw + 1;
    if arrow_x2 < area.right() {
        crate::render::draw_char_clipped(buf, arrow_x2, by + 1, preview_symbols.arrows.east, Style::new().fg(Color::Cyan), area);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::AppState;

    fn make_state_with_gallery() -> AppState {
        let mut s = AppState::default();
        s.gallery = Some(crate::state::GalleryState {
            category_idx: 0,
            selections: [0, 0, 0, 0],
        });
        s
    }

    #[test]
    fn gallery_renders_category_names() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_gallery();
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Box style"), "should show category names");
        assert!(content.contains("Arrows"), "should show Arrows category");
    }

    #[test]
    fn gallery_shows_output_all_settings_footer() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_gallery();
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("Output all settings"), "should show the export footer hint");
    }

    #[test]
    fn gallery_shows_active_selection_marker() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::default();
        // Select ascii box style (index 3 in rounded,thick,double,ascii,borderless).
        state.gallery = Some(crate::state::GalleryState {
            category_idx: 0,
            selections: [3, 0, 0, 0],
        });
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("ascii"), "should show ascii preset");
        // ascii preset preview should show + corners.
        assert!(content.contains('+'), "ascii box style preview should show + corners");
    }
}
