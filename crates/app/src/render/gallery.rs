//! Symbol gallery modal overlay — centered dialog chrome via draw_dialog.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::state::{AppState, GalleryState, GALLERY_CATEGORY_NAMES};
use crate::symbols::{Arrows, BoxStyle, PathGlyphs, PortalGlyphs};

// Minimum dialog dimensions: must fit the two-pane picker with chrome overhead.
// Category pane: 20 cols; preview pane: 30 cols; gutter: 1; border: 2 → w=53.
// Rows: header 1 + categories + preview + footer ≈ 20; border overhead 2 + button row 1 → h=23.
const GALLERY_MIN_W: u16 = 53;
const GALLERY_MIN_H: u16 = 18;

/// Draw the symbol gallery modal centered over `area`.
///
/// Uses `draw_dialog` for consistent chrome (border, title, [X], [Done]).
/// The two-pane picker (category list + preset/preview) is drawn into the
/// `content` rect returned by the dialog.
///
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` when
/// the gallery is closed or the area is too small.
pub fn draw_gallery(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let Some(gallery) = &state.gallery else { return None };

    // Compute dialog size: as wide as the area allows (up to 70), tall enough
    // for the content. Bail if the available area is too small.
    let modal_w = 70u16.min(area.width.saturating_sub(4));
    let modal_h = 24u16.min(area.height.saturating_sub(2));

    if modal_w < GALLERY_MIN_W || modal_h < GALLERY_MIN_H {
        // Terminal too small — bail without drawing to avoid layout corruption.
        return None;
    }

    // Build DialogStyle from state colors.
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
        title: "Symbol Gallery",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // ── Two-pane picker drawn into content ───────────────────────────────────

    // Footer hint row at bottom of content.
    if content.height > 1 {
        let footer_y = content.bottom() - 1;
        let footer = "o: Output all settings";
        let footer_style = Style::new().fg(Color::Yellow).patch(state.colors.dialog);
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    // Pane area: content minus the footer row.
    let pane_h = if content.height > 1 { content.height - 1 } else { content.height };
    let pane_area = Rect::new(content.x, content.y, content.width, pane_h);

    if pane_area.height == 0 || pane_area.width == 0 {
        return Some(rects);
    }

    // Left pane: 20 cols for categories.
    let left_w = 20u16.min(pane_area.width / 3);
    let left_area = Rect { x: pane_area.x, y: pane_area.y, width: left_w, height: pane_area.height };
    let right_area = Rect {
        x: pane_area.x + left_w,
        y: pane_area.y,
        width: pane_area.width.saturating_sub(left_w),
        height: pane_area.height,
    };

    draw_category_pane(gallery, left_area, buf, state);
    draw_preset_pane(state, gallery, right_area, buf);

    Some(rects)
}

fn draw_category_pane(gallery: &GalleryState, area: Rect, buf: &mut Buffer, state: &AppState) {
    let normal = Style::new().fg(Color::White).patch(state.colors.dialog);
    let active = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD).patch(state.colors.dialog);
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
    let normal = Style::new().fg(Color::White).patch(state.colors.dialog);
    let selected = Style::new().fg(Color::Green).add_modifier(Modifier::BOLD).patch(state.colors.dialog);

    // Category header.
    crate::render::draw_str_clipped(
        buf, area.x, area.y, GALLERY_CATEGORY_NAMES[cat],
        Style::new().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED).patch(state.colors.dialog),
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
    use crate::render::dialog::ButtonId;
    use crate::render::paneframe::BorderStyle;
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
    fn gallery_is_centered_bordered_dialog_not_fullscreen() {
        // The gallery must render as a CENTERED bordered dialog.
        // The top-left of the dialog must NOT be at screen column 0 / row 0.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_state_with_gallery();
        state.colors.dialog_box_style = BorderStyle::Single;

        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let rects = rects_out.expect("draw_gallery should return DialogRects when gallery is open");

        // Centered: top-left must not be at (0,0).
        assert!(
            rects.area.x > 0 || rects.area.y > 0,
            "gallery dialog must be centered, not full-screen (top-left was {:?})",
            (rects.area.x, rects.area.y)
        );

        // Must have [X] close button and [Done] button.
        assert!(rects.close.is_some(), "gallery dialog must have [X] close button");
        assert_eq!(rects.buttons.len(), 1, "gallery dialog must have one button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done), "gallery dialog must have [Done] button");

        // The top-left cell must be a border corner, not a space.
        let buf = terminal.backend().buffer();
        let top_left_sym = buf.cell((rects.area.x, rects.area.y))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default();
        // Single border: top-left corner is '┌'
        assert!(
            top_left_sym == "┌" || top_left_sym == "╔" || top_left_sym == "+" || top_left_sym == "┏",
            "top-left cell of gallery dialog must be a border corner, got {:?}", top_left_sym
        );

        // The content must be non-empty.
        assert!(rects.content.width > 0 && rects.content.height > 0, "content rect must be non-empty");
    }

    #[test]
    fn gallery_shows_dialog_chrome_title_and_buttons() {
        // The gallery must show "Symbol Gallery" title, [X], and [Done].
        // Use SingleBorder so the dialog has a proper frame with inset title area.
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_state_with_gallery();
        state.colors.dialog_box_style = BorderStyle::Single;

        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();

        // Collect all symbols (including multi-byte Unicode chars from borders).
        let all_syms: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars())
            .collect();
        assert!(all_syms.contains("Symbol Gallery"), "title 'Symbol Gallery' should be present");
        assert!(all_syms.contains("Done"), "[Done] button should be visible");
        assert!(all_syms.contains('✕'), "[X] close button should be visible");
    }

    #[test]
    fn gallery_renders_category_names() {
        let backend = TestBackend::new(80, 30);
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
        let backend = TestBackend::new(80, 30);
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
        let backend = TestBackend::new(80, 30);
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

    #[test]
    fn gallery_noop_when_closed() {
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = AppState::default(); // gallery = None
        let before: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let after: Vec<_> = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().to_string())
            .collect();
        assert_eq!(before, after, "draw_gallery should be a no-op when gallery is None");
    }

    #[test]
    fn gallery_returns_none_on_small_terminal() {
        // On a very small terminal, draw_gallery should bail and return None.
        let backend = TestBackend::new(30, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_gallery();
        let mut rects_out: Option<DialogRects> = None;
        terminal.draw(|f| {
            rects_out = draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        assert!(rects_out.is_none(), "draw_gallery should return None when terminal is too small");
    }
}
