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
    let _ = state; // kept for signature consistency

    crate::render::draw_str_clipped(buf, area.x, area.y, "Preview:", Style::new().fg(Color::DarkGray), area);
    if area.height < 2 {
        return;
    }

    // Build preview symbol set from current gallery selections.
    let cfg = gallery.symbol_config();
    let sym = SymbolSet::resolve(&cfg);
    let bs = &sym.room_normal;

    // ── Room box: 9 wide x 5 tall ─────────────────────────────────────────────
    // Layout: corners at (bx,by), (bx+bw-1,by), (bx,by+bh-1), (bx+bw-1,by+bh-1).
    // Cardinal arrows: N on top-center, S on bottom-center, E on right-center, W on left-center.
    // Corner arrows: NW/NE/SW/SE at the four corners (overwrite box corners).
    let bw: u16 = 9u16.min(area.width.saturating_sub(2));
    let bh: u16 = 5u16;
    // Minimum area needed: 1 (label) + bh (box) + 3 (path) + 1 (portals) = 10 rows.
    // We draw what fits; each block checks before drawing.
    let bx = area.x + 1;
    let by = area.y + 1;

    let box_style = Style::new().fg(Color::White);
    let arrow_style = Style::new().fg(Color::Cyan);
    let path_style = Style::new().fg(Color::Yellow);
    let portal_style = Style::new().fg(Color::Magenta);

    if bw >= 2 && area.height >= 2 + bh {
        // Top row.
        crate::render::draw_char_clipped(buf, bx, by, bs.tl, box_style, area);
        for x in (bx + 1)..(bx + bw - 1) {
            crate::render::draw_char_clipped(buf, x, by, bs.h, box_style, area);
        }
        crate::render::draw_char_clipped(buf, bx + bw - 1, by, bs.tr, box_style, area);

        // Middle rows.
        for row in 1..(bh - 1) {
            crate::render::draw_char_clipped(buf, bx, by + row, bs.v, box_style, area);
            crate::render::draw_char_clipped(buf, bx + bw - 1, by + row, bs.v, box_style, area);
        }

        // Bottom row.
        crate::render::draw_char_clipped(buf, bx, by + bh - 1, bs.bl, box_style, area);
        for x in (bx + 1)..(bx + bw - 1) {
            crate::render::draw_char_clipped(buf, x, by + bh - 1, bs.h, box_style, area);
        }
        crate::render::draw_char_clipped(buf, bx + bw - 1, by + bh - 1, bs.br, box_style, area);

        // Cardinal arrows on the box sides (overwrite border chars at mid-points).
        let mid_x = bx + bw / 2;
        let mid_y = by + bh / 2;
        crate::render::draw_char_clipped(buf, mid_x, by, sym.arrows.north, arrow_style, area);
        crate::render::draw_char_clipped(buf, mid_x, by + bh - 1, sym.arrows.south, arrow_style, area);
        crate::render::draw_char_clipped(buf, bx, mid_y, sym.arrows.west, arrow_style, area);
        crate::render::draw_char_clipped(buf, bx + bw - 1, mid_y, sym.arrows.east, arrow_style, area);

        // Corner arrows ON the box corners (overwrite the corner glyphs), the same way
        // the cardinals sit on the edge mid-points — matching how a diagonal exit sits on
        // a room's corner in the map.
        if bw >= 4 && bh >= 4 {
            crate::render::draw_char_clipped(buf, bx, by, sym.arrows.nw, arrow_style, area);
            crate::render::draw_char_clipped(buf, bx + bw - 1, by, sym.arrows.ne, arrow_style, area);
            crate::render::draw_char_clipped(buf, bx, by + bh - 1, sym.arrows.sw, arrow_style, area);
            crate::render::draw_char_clipped(buf, bx + bw - 1, by + bh - 1, sym.arrows.se, arrow_style, area);
        }
    }

    // ── Multi-segment path ────────────────────────────────────────────────────
    // Three-row path demonstrating >=2 straights, >=2 corners, >=1 junction.
    // Layout (col offsets from bx):
    //   Row 0: se ew ew ew ew ew sw
    //   Row 1: ns                ns
    //   Row 2: ne ew ew nesw ew ew nw
    // That gives: 4 straights (ew x4 + ew x2 on row2), 4 corners, 1 junction (nesw).
    let path_y = by + bh + 1; // one blank row gap after box
    let pw = bw; // same width as box
    if path_y + 2 < area.bottom() && pw >= 4 {
        let px = bx;
        let p = &sym.path;

        // Row 0: se + ew straights + sw
        crate::render::draw_char_clipped(buf, px, path_y, p.se, path_style, area);
        for x in (px + 1)..(px + pw - 1) {
            crate::render::draw_char_clipped(buf, x, path_y, p.ew, path_style, area);
        }
        crate::render::draw_char_clipped(buf, px + pw - 1, path_y, p.sw, path_style, area);

        // Row 1: ns on left and right
        crate::render::draw_char_clipped(buf, px, path_y + 1, p.ns, path_style, area);
        crate::render::draw_char_clipped(buf, px + pw - 1, path_y + 1, p.ns, path_style, area);

        // Row 2: ne + ew + nesw (junction) + ew + nw
        crate::render::draw_char_clipped(buf, px, path_y + 2, p.ne, path_style, area);
        let mid_pw = pw / 2;
        for x in (px + 1)..(px + mid_pw) {
            crate::render::draw_char_clipped(buf, x, path_y + 2, p.ew, path_style, area);
        }
        crate::render::draw_char_clipped(buf, px + mid_pw, path_y + 2, p.nesw, path_style, area);
        for x in (px + mid_pw + 1)..(px + pw - 1) {
            crate::render::draw_char_clipped(buf, x, path_y + 2, p.ew, path_style, area);
        }
        crate::render::draw_char_clipped(buf, px + pw - 1, path_y + 2, p.nw, path_style, area);
    }

    // ── All 4 portal icons ────────────────────────────────────────────────────
    // Draw up/down/in/out in a horizontal row with a space gap between each.
    // Layout: up ' ' down ' ' in_ ' ' out
    let portal_y = path_y + 3;
    if portal_y < area.bottom() {
        let portals = [sym.portal.up, sym.portal.down, sym.portal.in_, sym.portal.out];
        let mut px = bx;
        for ch in portals {
            if px < area.right() {
                crate::render::draw_char_clipped(buf, px, portal_y, ch, portal_style, area);
                px += 2; // glyph + space gap
            }
        }
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
    use crate::state::GalleryState;

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
        // Select the ascii box style (look up its index so preset reordering can't break this).
        let ascii_idx = crate::symbols::BoxStyle::preset_names()
            .iter().position(|&n| n == "ascii").expect("ascii preset exists");
        state.gallery = Some(crate::state::GalleryState {
            category_idx: 0,
            selections: [ascii_idx, 0, 0, 0],
        });
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("ascii"), "should show ascii preset");
        // ascii preset preview shows '|'/'-' walls (the box corners are overwritten by
        // the diagonal corner arrows, matching how a diagonal exit sits on a room corner).
        assert!(content.contains('|'), "ascii box style preview should show | walls");
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

    #[test]
    fn preview_shows_box_corner_path_arrows_and_portals() {
        // Build a GalleryState selecting:
        //   box=thick (index 1), arrows=nf-box (index 4),
        //   portal=nerdfont-stairs (index 2), path=heavy (index 1)
        // BoxStyle preset_names: ["rounded","thick","double","ascii","borderless"]  => thick=1
        // Arrows preset_names:   ["filled","line","nerdfont","nf-bold","nf-box",..] => nf-box=4
        // PortalGlyphs preset_names: ["ascii","nerdfont","nerdfont-stairs"]          => nerdfont-stairs=2
        // PathGlyphs preset_names:   ["light","heavy","dotted"]                      => heavy=1
        let mut state = AppState::default();
        state.gallery = Some(GalleryState {
            category_idx: 0,
            selections: [
                1, // box = thick
                4, // arrows = nf-box
                2, // portal = nerdfont-stairs
                1, // path = heavy
            ],
        });

        // Use a wide terminal so the dialog opens and the preview pane has room.
        // The dialog is up to 70 wide x 24 tall; the preview is below the preset list.
        // We use 80x40 to give ample space.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            draw_gallery(&state, f.area(), f.buffer_mut());
        }).unwrap();

        // Collect every character in the buffer as a flat string.
        let all_chars: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars())
            .collect();

        // Thick box corner must be present (tl = '┏').
        assert!(
            all_chars.contains('┏'),
            "preview must contain thick box top-left corner '┏'"
        );

        // At least one corner arrow from nf-box: nw=F1968, ne=F196A, sw=F1964, se=F1966.
        let corner_arrows = ['\u{F1968}', '\u{F196A}', '\u{F1964}', '\u{F1966}'];
        assert!(
            corner_arrows.iter().any(|&ch| all_chars.contains(ch)),
            "preview must contain at least one nf-box corner arrow glyph"
        );

        // At least 2 distinct heavy path glyphs: ew='━', ns='┃', se='┏', sw='┓', ne='┗', nw='┛',
        // nse='┣', nsw='┫', ews='┳', ewn='┻', nesw='╋'.
        let heavy_path_glyphs = ['━', '┃', '┗', '┛', '┣', '┫', '┳', '┻', '╋'];
        let found_path: Vec<char> = heavy_path_glyphs.iter().copied()
            .filter(|&ch| all_chars.contains(ch))
            .collect();
        assert!(
            found_path.len() >= 2,
            "preview must contain >=2 distinct heavy path glyphs; found: {:?}",
            found_path
        );

        // All 4 nerdfont-stairs portal glyphs must appear:
        //   up=F12BD (stairs-up), down=F12BE (stairs-down),
        //   in_=F0FC4 (location-enter), out=F0A48 (exit-run).
        let portal_glyphs = [
            ('\u{F12BD}', "up/stairs-up"),
            ('\u{F12BE}', "down/stairs-down"),
            ('\u{F0FC4}', "in/location-enter"),
            ('\u{F0A48}', "out/exit-run"),
        ];
        for (ch, label) in portal_glyphs {
            assert!(
                all_chars.contains(ch),
                "preview must contain portal glyph {} ({:?})",
                label, ch
            );
        }
    }
}
