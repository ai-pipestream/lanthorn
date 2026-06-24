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

// ── draw_pane_frame ────────────────────────────────────────────────────────────

pub fn draw_pane_frame(buf: &mut Buffer, area: Rect, style: BorderStyle, color: Style) -> PaneFrame {
    // PictureFrame falls through to Single for now (Task 2 implements it)
    let effective = match style {
        BorderStyle::None => {
            // No border drawn; content == area; top_inset is the top row
            let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
            return PaneFrame { area, content: area, top_inset };
        }
        BorderStyle::PictureFrame => BorderStyle::Single,
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
}
