use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

// ── BorderStyle ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Single,
    Double,
    Thick,
    PictureFrame,
}

pub fn parse_border_style(s: &str) -> BorderStyle {
    match s {
        "none" => BorderStyle::None,
        "single" => BorderStyle::Single,
        "double" => BorderStyle::Double,
        "thick" => BorderStyle::Thick,
        "picture-frame" => BorderStyle::PictureFrame,
        _ => BorderStyle::Single,
    }
}

// ── PaneFrame ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct PaneFrame {
    pub area: Rect,
    pub content: Rect,
    pub top_inset: Rect,
}

// ── Glyph sets ────────────────────────────────────────────────────────────────

struct Glyphs {
    tl: &'static str,
    top: &'static str,
    tr: &'static str,
    side: &'static str,
    bl: &'static str,
    br: &'static str,
}

const SINGLE: Glyphs = Glyphs { tl: "┌", top: "─", tr: "┐", side: "│", bl: "└", br: "┘" };
const DOUBLE: Glyphs = Glyphs { tl: "╔", top: "═", tr: "╗", side: "║", bl: "╚", br: "╝" };
const THICK: Glyphs  = Glyphs { tl: "┏", top: "━", tr: "┓", side: "┃", bl: "┗", br: "┛" };

// ── draw_picture_frame ────────────────────────────────────────────────────────

fn draw_picture_frame(buf: &mut Buffer, area: Rect, color: Style) -> PaneFrame {
    let x = area.x;
    let y = area.y;
    let w = area.width;
    let h = area.height;
    let right = x + w - 1;   // col w-1
    let bottom = y + h - 1;  // row h-1

    // ── Outer heavy perimeter (THICK) ──────────────────────────────────────────
    // Top row
    if let Some(c) = buf.cell_mut((x, y))         { c.set_symbol("┏").set_style(color); }
    if let Some(c) = buf.cell_mut((right, y))      { c.set_symbol("┓").set_style(color); }
    for cx in (x + 1)..right {
        if let Some(c) = buf.cell_mut((cx, y))     { c.set_symbol("━").set_style(color); }
    }
    // Bottom row
    if let Some(c) = buf.cell_mut((x, bottom))     { c.set_symbol("┗").set_style(color); }
    if let Some(c) = buf.cell_mut((right, bottom)) { c.set_symbol("┛").set_style(color); }
    for cx in (x + 1)..right {
        if let Some(c) = buf.cell_mut((cx, bottom)) { c.set_symbol("━").set_style(color); }
    }
    // Left and right sides (rows 1..h-2, i.e. y+1..bottom)
    for cy in (y + 1)..bottom {
        if let Some(c) = buf.cell_mut((x, cy))     { c.set_symbol("┃").set_style(color); }
        if let Some(c) = buf.cell_mut((right, cy)) { c.set_symbol("┃").set_style(color); }
    }

    // ── Inner top run: row 1, cols 2..=w-3 ────────────────────────────────────
    // (spaces at col 1 and col w-2 are already blank from buffer init)
    let inner_top_y = y + 1;
    let inner_bot_y = y + h - 2; // row h-2
    let inner_l = x + 2;         // col 2
    let inner_r = x + w - 3;     // col w-3
    let side_l  = x + 1;         // col 1
    let side_r  = x + w - 2;     // col w-2

    // Spaces at the inset gap cells (col1 row1, col w-2 row1, col1 row h-2, col w-2 row h-2)
    for &(cx, cy) in &[(side_l, inner_top_y), (side_r, inner_top_y),
                       (side_l, inner_bot_y), (side_r, inner_bot_y)] {
        if let Some(c) = buf.cell_mut((cx, cy))    { c.set_symbol(" ").set_style(color); }
    }

    // Horizontal runs (─) at inner_top_y and inner_bot_y, cols inner_l..=inner_r
    for cx in inner_l..=inner_r {
        if let Some(c) = buf.cell_mut((cx, inner_top_y)) { c.set_symbol("─").set_style(color); }
        if let Some(c) = buf.cell_mut((cx, inner_bot_y)) { c.set_symbol("─").set_style(color); }
    }

    // ── Inner side runs: cols 1 and w-2, rows 2..=h-3 ─────────────────────────
    let notch_top_y = y + 2;        // row 2
    let notch_bot_y = y + h - 3;    // row h-3
    for cy in notch_top_y..=notch_bot_y {
        if let Some(c) = buf.cell_mut((side_l, cy)) { c.set_symbol("│").set_style(color); }
        if let Some(c) = buf.cell_mut((side_r, cy)) { c.set_symbol("│").set_style(color); }
    }

    // ── Corner notches ─────────────────────────────────────────────────────────
    // Top-left notch: row1 col2 = ┌, row2 col1 = ┌, row2 col2 = ┘
    if let Some(c) = buf.cell_mut((inner_l,   inner_top_y)) { c.set_symbol("┌").set_style(color); }
    if let Some(c) = buf.cell_mut((side_l,    notch_top_y)) { c.set_symbol("┌").set_style(color); }
    if let Some(c) = buf.cell_mut((inner_l,   notch_top_y)) { c.set_symbol("┘").set_style(color); }

    // Top-right notch: row1 col w-3 = ┐, row2 col w-3 = └, row2 col w-2 = ┐
    if let Some(c) = buf.cell_mut((inner_r,   inner_top_y)) { c.set_symbol("┐").set_style(color); }
    if let Some(c) = buf.cell_mut((inner_r,   notch_top_y)) { c.set_symbol("└").set_style(color); }
    if let Some(c) = buf.cell_mut((side_r,    notch_top_y)) { c.set_symbol("┐").set_style(color); }

    // Bottom-left notch: row h-3 col1 = └, row h-3 col2 = ┐, row h-2 col2 = └
    if let Some(c) = buf.cell_mut((side_l,    notch_bot_y)) { c.set_symbol("└").set_style(color); }
    if let Some(c) = buf.cell_mut((inner_l,   notch_bot_y)) { c.set_symbol("┐").set_style(color); }
    if let Some(c) = buf.cell_mut((inner_l,   inner_bot_y)) { c.set_symbol("└").set_style(color); }

    // Bottom-right notch: row h-3 col w-3 = ┌, row h-3 col w-2 = ┘, row h-2 col w-3 = ┘
    if let Some(c) = buf.cell_mut((inner_r,   notch_bot_y)) { c.set_symbol("┌").set_style(color); }
    if let Some(c) = buf.cell_mut((side_r,    notch_bot_y)) { c.set_symbol("┘").set_style(color); }
    if let Some(c) = buf.cell_mut((inner_r,   inner_bot_y)) { c.set_symbol("┘").set_style(color); }

    // ── Content and top_inset ──────────────────────────────────────────────────
    // Content = cols 2..=w-3, rows 2..=h-3
    let content = Rect::new(x + 2, y + 2, w - 4, h - 4);

    // top_inset = inner top horizontal run (the drawable span between notch corners)
    // This is the ─ run at row 1, cols 3..=w-4 (between the two ┌┐ corner glyphs)
    // But per the spec, top_inset is the top border row between the outer corners.
    // For picture-frame, it's the inner top row (row 1) between the ┌ and ┐ notch glyphs,
    // i.e. cols 3..=w-4 — the actual ─ cells that can be overwritten for a title.
    let inset_x = x + 3;
    let inset_w = if w >= 6 { w - 6 } else { 0 };
    let top_inset = Rect::new(inset_x, y + 1, inset_w, 1);

    PaneFrame { area, content, top_inset }
}

// ── draw_pane_frame ────────────────────────────────────────────────────────────

pub fn draw_pane_frame(buf: &mut Buffer, area: Rect, style: BorderStyle, color: Style) -> PaneFrame {
    let effective = match style {
        BorderStyle::None => {
            // No border drawn; content == area; top_inset is the top row
            let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
            return PaneFrame { area, content: area, top_inset };
        }
        BorderStyle::PictureFrame => {
            // Degrade to Single for tiny panes
            if area.width < 7 || area.height < 7 {
                BorderStyle::Single
            } else {
                return draw_picture_frame(buf, area, color);
            }
        }
        other => other,
    };

    if area.width < 2 || area.height < 2 {
        // Too small to draw a border; degrade to None
        let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        return PaneFrame { area, content: area, top_inset };
    }

    let glyphs = match effective {
        BorderStyle::Single => &SINGLE,
        BorderStyle::Double => &DOUBLE,
        BorderStyle::Thick  => &THICK,
        _ => unreachable!(),
    };

    let x = area.x;
    let y = area.y;
    let w = area.width;
    let h = area.height;
    let right = x + w - 1;
    let bottom = y + h - 1;

    // Draw top row
    if let Some(cell) = buf.cell_mut((x, y)) { cell.set_symbol(glyphs.tl).set_style(color); }
    if let Some(cell) = buf.cell_mut((right, y)) { cell.set_symbol(glyphs.tr).set_style(color); }
    for cx in (x + 1)..right {
        if let Some(cell) = buf.cell_mut((cx, y)) { cell.set_symbol(glyphs.top).set_style(color); }
    }

    // Draw bottom row
    if let Some(cell) = buf.cell_mut((x, bottom)) { cell.set_symbol(glyphs.bl).set_style(color); }
    if let Some(cell) = buf.cell_mut((right, bottom)) { cell.set_symbol(glyphs.br).set_style(color); }
    for cx in (x + 1)..right {
        if let Some(cell) = buf.cell_mut((cx, bottom)) { cell.set_symbol(glyphs.top).set_style(color); }
    }

    // Draw left and right sides
    for cy in (y + 1)..bottom {
        if let Some(cell) = buf.cell_mut((x, cy)) { cell.set_symbol(glyphs.side).set_style(color); }
        if let Some(cell) = buf.cell_mut((right, cy)) { cell.set_symbol(glyphs.side).set_style(color); }
    }

    // content = area inset by 1 on each side
    let content = Rect::new(x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2));

    // top_inset = top border row between corners (x+1 .. right-1) at row y
    let inset_x = x + 1;
    let inset_w = right.saturating_sub(inset_x);
    let top_inset = Rect::new(inset_x, y, inset_w, 1);

    PaneFrame { area, content, top_inset }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_border_perimeter_and_content() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::Single, Style::default());
        assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┌");
        assert_eq!(buf.cell((5,0)).unwrap().symbol(), "┐");
        assert_eq!(buf.cell((0,3)).unwrap().symbol(), "└");
        assert_eq!(buf.cell((5,3)).unwrap().symbol(), "┘");
        assert_eq!(f.content, Rect::new(1,1,4,2));
    }

    #[test]
    fn none_border_content_is_full_area() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0,0,6,4);
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::None, Style::default());
        assert_eq!(f.content, area);
    }

    #[test]
    fn parse_border_style_known_and_unknown() {
        assert!(matches!(parse_border_style("double"), BorderStyle::Double));
        assert!(matches!(parse_border_style("picture-frame"), BorderStyle::PictureFrame));
        assert!(matches!(parse_border_style("bogus"), BorderStyle::Single));
    }

    #[test]
    fn picture_frame_exact_glyphs_and_content() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0,0,9,8); // w=9,h=8
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::PictureFrame, Style::default());
        // outer corners
        assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┏");
        assert_eq!(buf.cell((8,0)).unwrap().symbol(), "┓");
        assert_eq!(buf.cell((0,7)).unwrap().symbol(), "┗");
        assert_eq!(buf.cell((8,7)).unwrap().symbol(), "┛");
        // inner top inset by 1 from corners (row 1: space at col1, ┌ at col2)
        assert_eq!(buf.cell((1,1)).unwrap().symbol(), " ");
        assert_eq!(buf.cell((2,1)).unwrap().symbol(), "┌");
        assert_eq!(buf.cell((6,1)).unwrap().symbol(), "┐");
        // corner notch row 2: col1 ┌, col2 ┘
        assert_eq!(buf.cell((1,2)).unwrap().symbol(), "┌");
        assert_eq!(buf.cell((2,2)).unwrap().symbol(), "┘");
        // inner side flush at col1 mid-rows
        assert_eq!(buf.cell((1,3)).unwrap().symbol(), "│");
        assert_eq!(f.content, Rect::new(2,2,5,4)); // cols 2..=6, rows 2..=5
    }

    #[test]
    fn picture_frame_tiny_pane_degrades_to_single() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0,0,5,5);
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::PictureFrame, Style::default());
        assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┌"); // single, not ┏
        assert_eq!(f.content, Rect::new(1,1,3,3));
    }
}
