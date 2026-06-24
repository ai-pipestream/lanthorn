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

// ── InsetSegment ──────────────────────────────────────────────────────────────

pub struct InsetSegment<'a> {
    pub text: &'a str,
    pub active: bool,
}

// ── draw_top_inset ────────────────────────────────────────────────────────────

pub fn draw_top_inset(
    buf: &mut Buffer,
    top_inset: Rect,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
) -> Vec<Rect> {
    if segments.is_empty() || top_inset.width == 0 || top_inset.height == 0 {
        return segments.iter().map(|_| Rect::default()).collect();
    }

    // Build the full bracketed string to measure total width.
    // Format: ┫ seg0 ┃ seg1 ┃ seg2 ┣
    // We need to know:
    // - total chars to check if it fits
    // - which segment is active (for overflow logic)
    let active_idx = segments.iter().position(|s| s.active);

    let full_width = compute_full_width(segments);

    let avail = top_inset.width as usize;

    if full_width <= avail {
        // It fits: render centered
        let leading = (avail - full_width) / 2;
        let start_x = top_inset.x + leading as u16;
        render_segments(buf, top_inset.y, start_x, segments, base, active)
    } else {
        // Overflow: show active segment ± neighbors with ‹…› markers
        render_overflow(buf, top_inset, segments, base, active, active_idx)
    }
}

fn compute_full_width(segments: &[InsetSegment]) -> usize {
    if segments.is_empty() {
        return 0;
    }
    // ┫ + space + text + space + (┃ + space + text + space)* + ┣
    let n = segments.len();
    let text_len: usize = segments.iter().map(|s| s.text.chars().count()).sum();
    // 1 (┫) + 1 (space) + text_len + 1 (space) * n + (n-1) (┃) + 1 (┣)
    // = 1 + n*2 + text_len + (n-1) + 1
    // = 1 + 2n + text_len + n - 1 + 1 = 1 + 3n + text_len - 1 + 1
    1 + n * 2 + text_len + (n - 1) + 1
}

/// Render all segments into the buffer starting at start_x, returning hit-rects.
fn render_segments(
    buf: &mut Buffer,
    row: u16,
    start_x: u16,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); segments.len()];
    let mut cx = start_x;

    // ┫ bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol("┫").set_style(base);
    }
    cx += 1;

    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            // separator ┃
            if let Some(c) = buf.cell_mut((cx, row)) {
                c.set_symbol("┃").set_style(base);
            }
            cx += 1;
        }

        let seg_style = if seg.active { active } else { base };
        let seg_start_x = cx;

        // space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        // text chars
        for ch in seg.text.chars() {
            if let Some(c) = buf.cell_mut((cx, row)) {
                let s = ch.to_string();
                c.set_symbol(&s).set_style(seg_style);
            }
            cx += 1;
        }

        // trailing space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        let seg_end_x = cx;
        rects[i] = Rect::new(seg_start_x, row, seg_end_x - seg_start_x, 1);
    }

    // ┣ bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol("┣").set_style(base);
    }

    rects
}

/// Overflow rendering: show active ± neighbors, with ‹…› markers.
fn render_overflow(
    buf: &mut Buffer,
    top_inset: Rect,
    segments: &[InsetSegment],
    base: Style,
    active: Style,
    active_idx: Option<usize>,
) -> Vec<Rect> {
    let n = segments.len();
    let avail = top_inset.width as usize;
    let row = top_inset.y;
    let x = top_inset.x;

    // Find the active index, default to 0
    let ai = active_idx.unwrap_or(0);

    // Determine window of segments to show
    // Start with just the active segment; expand neighbors while space allows.
    // Markers: ‹…› is 3 chars each side (when needed).
    let marker = "‹…›"; // 3 chars
    let marker_width = 3usize;

    // Try to fit active + expanding neighbors
    let mut lo = ai;
    let mut hi = ai;

    loop {
        let needs_left_marker = lo > 0;
        let needs_right_marker = hi < n - 1;
        let left_overhead = if needs_left_marker { marker_width + 1 } else { 0 }; // marker + space
        let right_overhead = if needs_right_marker { marker_width + 1 } else { 0 };

        // Width for current window [lo..=hi]
        let window_segs = &segments[lo..=hi];
        let window_w = compute_full_width(window_segs);
        let total = left_overhead + window_w + right_overhead;

        if total > avail {
            // Can't fit even the active alone; just show active truncated
            break;
        }

        // Try expanding
        let can_expand_left = lo > 0;
        let can_expand_right = hi < n - 1;

        if !can_expand_left && !can_expand_right {
            break; // no more to expand
        }

        // Try expanding left first
        let mut expanded = false;
        if can_expand_left {
            let new_lo = lo - 1;
            let new_needs_left_marker = new_lo > 0;
            let new_left_overhead = if new_needs_left_marker { marker_width + 1 } else { 0 };
            let new_window_segs = &segments[new_lo..=hi];
            let new_window_w = compute_full_width(new_window_segs);
            let new_total = new_left_overhead + new_window_w + right_overhead;
            if new_total <= avail {
                lo = new_lo;
                expanded = true;
            }
        }

        if can_expand_right {
            let new_hi = hi + 1;
            let new_needs_right_marker = new_hi < n - 1;
            let new_right_overhead = if new_needs_right_marker { marker_width + 1 } else { 0 };
            let new_window_segs = &segments[lo..=new_hi];
            let new_window_w = compute_full_width(new_window_segs);
            let cur_left_overhead = if lo > 0 { marker_width + 1 } else { 0 };
            let new_total = cur_left_overhead + new_window_w + new_right_overhead;
            if new_total <= avail {
                hi = new_hi;
                expanded = true;
            }
        }

        if !expanded {
            break;
        }
    }

    let needs_left_marker = lo > 0;
    let needs_right_marker = hi < n - 1;

    // Calculate total width for centering
    let left_overhead = if needs_left_marker { marker_width + 1 } else { 0 };
    let right_overhead = if needs_right_marker { marker_width + 1 } else { 0 };
    let window_w = compute_full_width(&segments[lo..=hi]);
    let total_w = left_overhead + window_w + right_overhead;

    let leading = if total_w < avail { (avail - total_w) / 2 } else { 0 };
    let mut cx = x + leading as u16;

    let mut rects = vec![Rect::default(); n];

    // Left marker
    if needs_left_marker {
        for ch in marker.chars() {
            if let Some(c) = buf.cell_mut((cx, row)) {
                let s = ch.to_string();
                c.set_symbol(&s).set_style(base);
            }
            cx += 1;
        }
        // space after marker
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(base);
        }
        cx += 1;
    }

    // ┫ bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol("┫").set_style(base);
    }
    cx += 1;

    // Render visible segments
    for (wi, si) in (lo..=hi).enumerate() {
        let seg = &segments[si];
        if wi > 0 {
            // separator ┃
            if let Some(c) = buf.cell_mut((cx, row)) {
                c.set_symbol("┃").set_style(base);
            }
            cx += 1;
        }

        let seg_style = if seg.active { active } else { base };
        let seg_start_x = cx;

        // space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        // text chars
        for ch in seg.text.chars() {
            if let Some(c) = buf.cell_mut((cx, row)) {
                let s = ch.to_string();
                c.set_symbol(&s).set_style(seg_style);
            }
            cx += 1;
        }

        // trailing space
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(seg_style);
        }
        cx += 1;

        let seg_end_x = cx;
        rects[si] = Rect::new(seg_start_x, row, seg_end_x - seg_start_x, 1);
    }

    // ┣ bracket
    if let Some(c) = buf.cell_mut((cx, row)) {
        c.set_symbol("┣").set_style(base);
    }
    cx += 1;

    // Right marker
    if needs_right_marker {
        // space before marker
        if let Some(c) = buf.cell_mut((cx, row)) {
            c.set_symbol(" ").set_style(base);
        }
        cx += 1;
        for ch in marker.chars() {
            if let Some(c) = buf.cell_mut((cx, row)) {
                let s = ch.to_string();
                c.set_symbol(&s).set_style(base);
            }
            cx += 1;
        }
    }

    let _ = cx; // suppress unused warning
    rects
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

    #[test]
    fn top_inset_centers_single_title() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let strip = Rect::new(0,0,20,1);
        let mut buf = Buffer::empty(Rect::new(0,0,20,1));
        let rects = draw_top_inset(&mut buf, strip, &[InsetSegment{text:"ZORK I", active:false}], Style::default(), Style::default());
        let row: String = (0..20).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
        assert!(row.contains("ZORK I"));
        // centered: leading filler before the bracket
        assert!(row.find("ZORK I").unwrap() > 3);
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn top_inset_overflow_keeps_active_with_marker() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let strip = Rect::new(0,0,9,1);
        let mut buf = Buffer::empty(Rect::new(0,0,9,1));
        let segs = [InsetSegment{text:"0",active:false},InsetSegment{text:"1",active:true},InsetSegment{text:"2",active:false},InsetSegment{text:"3",active:false}];
        let _ = draw_top_inset(&mut buf, strip, &segs, Style::default(), Style::default());
        let row: String = (0..9).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
        assert!(row.contains("1"));      // active shown
        assert!(row.contains("‹") || row.contains("…")); // overflow marker present
    }
}
