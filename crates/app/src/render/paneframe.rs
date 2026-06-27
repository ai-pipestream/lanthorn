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

/// Return the canonical style-file name for a [`BorderStyle`].
pub fn border_style_name(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::None => "none",
        BorderStyle::Single => "single",
        BorderStyle::Double => "double",
        BorderStyle::Thick => "thick",
        BorderStyle::PictureFrame => "picture-frame",
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

// Picture-frame outer-border block glyphs.
// `RAMP` is the lower one-eighth block series, level 1 (thin) .. 8 (full). These
// fill from the bottom of the cell, so the painted mass hugs the inner frame
// below — the top border ramps up to crest at the centred layer-tab strip.
const RAMP: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
// Thin one-eighth blocks for the sides and bottom, each positioned to sit flush
// against the inner frame: ▕ (right eighth) on the left, ▏ (left eighth) on the
// right, ▔ (upper eighth) along the bottom.
const PF_SIDE_L: &str = "▕";
const PF_SIDE_R: &str = "▏";
const PF_BOTTOM: &str = "▔";

// ── draw_picture_frame ────────────────────────────────────────────────────────

fn draw_picture_frame(buf: &mut Buffer, area: Rect, color: Style) -> PaneFrame {
    let x = area.x;
    let y = area.y;
    let w = area.width;
    let h = area.height;
    let right = x + w - 1;   // outer right col
    let bottom = y + h - 1;  // outer bottom row

    // ── Top outer border: stepped lower-block ramp ─────────────────────────────
    // Each cell rises one level per two cells from the nearest corner, capping at
    // a full block. The ramp crests at the centre, where the layer-tab strip is
    // later overlaid via `draw_top_inset` (the strip overwrites the peak cells).
    for cx in x..=right {
        let from_corner = (cx - x).min(right - cx) as usize;
        let level = (1 + from_corner / 2).min(8);
        if let Some(c) = buf.cell_mut((cx, y)) { c.set_symbol(RAMP[level - 1]).set_style(color); }
    }

    // ── Thin side blocks (hug the inner frame) ─────────────────────────────────
    for cy in (y + 1)..bottom {
        if let Some(c) = buf.cell_mut((x, cy))     { c.set_symbol(PF_SIDE_L).set_style(color); }
        if let Some(c) = buf.cell_mut((right, cy))  { c.set_symbol(PF_SIDE_R).set_style(color); }
    }

    // ── Thin bottom block ──────────────────────────────────────────────────────
    for cx in x..=right {
        if let Some(c) = buf.cell_mut((cx, bottom)) { c.set_symbol(PF_BOTTOM).set_style(color); }
    }

    // ── Inner single-line frame ────────────────────────────────────────────────
    let il = x + 1;        // inner left col
    let ir = right - 1;    // inner right col
    let it = y + 1;        // inner top row
    let ib = bottom - 1;   // inner bottom row
    if let Some(c) = buf.cell_mut((il, it)) { c.set_symbol("┌").set_style(color); }
    if let Some(c) = buf.cell_mut((ir, it)) { c.set_symbol("┐").set_style(color); }
    if let Some(c) = buf.cell_mut((il, ib)) { c.set_symbol("└").set_style(color); }
    if let Some(c) = buf.cell_mut((ir, ib)) { c.set_symbol("┘").set_style(color); }
    for cx in (il + 1)..ir {
        if let Some(c) = buf.cell_mut((cx, it)) { c.set_symbol("─").set_style(color); }
        if let Some(c) = buf.cell_mut((cx, ib)) { c.set_symbol("─").set_style(color); }
    }
    for cy in (it + 1)..ib {
        if let Some(c) = buf.cell_mut((il, cy)) { c.set_symbol("│").set_style(color); }
        if let Some(c) = buf.cell_mut((ir, cy)) { c.set_symbol("│").set_style(color); }
    }

    // ── Content and top_inset ──────────────────────────────────────────────────
    // Content lives inside the inner single-line frame.
    let content = Rect::new(x + 2, y + 2, w - 4, h - 4);
    // The layer-tab strip crests at the top outer row, centred over the inner span.
    let top_inset = Rect::new(x + 1, y, w.saturating_sub(2), 1);

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

// ── PaneGlyphs + PaneSides + per-side frame drawing ───────────────────────────

/// Per-side/corner glyph overrides for a pane border.
///
/// Each field is `Some(glyph)` when the user has explicitly set an override for
/// that position, or `None` to fall back to the border-style default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneGlyphs {
    pub top: Option<String>,
    pub bottom: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub tl: Option<String>,
    pub tr: Option<String>,
    pub bl: Option<String>,
    pub br: Option<String>,
}

/// Per-side border styles for one pane. A side of `None` is omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSides {
    pub top: BorderStyle,
    pub bottom: BorderStyle,
    pub left: BorderStyle,
    pub right: BorderStyle,
}

impl PaneSides {
    /// All four sides set to one style.
    pub fn all(style: BorderStyle) -> PaneSides {
        PaneSides { top: style, bottom: style, left: style, right: style }
    }

    /// True if any side draws a border (i.e. not all `None`). Render gates that
    /// decide whether to box a pane MUST use this, not the base `*_style`, or a
    /// `style="none"` + per-side config renders nothing.
    pub fn any_on(self) -> bool {
        self.top != BorderStyle::None
            || self.bottom != BorderStyle::None
            || self.left != BorderStyle::None
            || self.right != BorderStyle::None
    }
}

/// "Weight" used to pick a corner glyph when two adjacent sides differ:
/// thick > double > single > none.
fn border_weight(s: BorderStyle) -> u8 {
    match s {
        BorderStyle::Thick => 3,
        BorderStyle::Double => 2,
        BorderStyle::Single => 1,
        _ => 0, // None / PictureFrame never reach the per-side corner path
    }
}

fn glyphs_for(style: BorderStyle) -> &'static Glyphs {
    match style {
        BorderStyle::Double => &DOUBLE,
        BorderStyle::Thick => &THICK,
        _ => &SINGLE,
    }
}

/// Which corner of the frame, for `corner_glyph`.
#[derive(Clone, Copy)]
enum Corner { Tl, Tr, Bl, Br }

/// The glyph for one corner, given its horizontal (top/bottom) and vertical
/// (left/right) adjacent side styles. Both present → corner glyph of the heavier
/// style; only horizontal → that horizontal glyph; only vertical → that vertical
/// glyph; neither → a space.
fn corner_glyph(h: BorderStyle, v: BorderStyle, which: Corner) -> &'static str {
    let h_on = h != BorderStyle::None;
    let v_on = v != BorderStyle::None;
    match (h_on, v_on) {
        (true, true) => {
            let g = if border_weight(h) >= border_weight(v) { glyphs_for(h) } else { glyphs_for(v) };
            match which { Corner::Tl => g.tl, Corner::Tr => g.tr, Corner::Bl => g.bl, Corner::Br => g.br }
        }
        (true, false) => glyphs_for(h).top,
        (false, true) => glyphs_for(v).side,
        (false, false) => " ",
    }
}

/// Draw a pane frame with independent per-side styles. Each present side draws
/// its straight glyph; corners resolve via `corner_glyph`; `content` is inset by
/// 1 only on sides that have a border; `top_inset` is the top row (between the
/// left/right insets) only when the top side is present, else zero-height.
pub fn draw_pane_frame_sides(buf: &mut Buffer, area: Rect, sides: PaneSides, color: Style) -> PaneFrame {
    if area.width < 2 || area.height < 2 {
        let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        return PaneFrame { area, content: area, top_inset };
    }
    let x = area.x;
    let y = area.y;
    let right = x + area.width - 1;
    let bottom = y + area.height - 1;
    let on = |s: BorderStyle| s != BorderStyle::None;

    // Horizontal runs (top/bottom): straight glyph between the corners.
    if on(sides.top) {
        let g = glyphs_for(sides.top);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, y)) { c.set_symbol(g.top).set_style(color); }
        }
    }
    if on(sides.bottom) {
        let g = glyphs_for(sides.bottom);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, bottom)) { c.set_symbol(g.top).set_style(color); }
        }
    }
    // Vertical runs (left/right).
    if on(sides.left) {
        let g = glyphs_for(sides.left);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((x, cy)) { c.set_symbol(g.side).set_style(color); }
        }
    }
    if on(sides.right) {
        let g = glyphs_for(sides.right);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((right, cy)) { c.set_symbol(g.side).set_style(color); }
        }
    }
    // Corners.
    let set = |buf: &mut Buffer, px: u16, py: u16, sym: &str| {
        if sym != " " {
            if let Some(c) = buf.cell_mut((px, py)) { c.set_symbol(sym).set_style(color); }
        }
    };
    set(buf, x, y, corner_glyph(sides.top, sides.left, Corner::Tl));
    set(buf, right, y, corner_glyph(sides.top, sides.right, Corner::Tr));
    set(buf, x, bottom, corner_glyph(sides.bottom, sides.left, Corner::Bl));
    set(buf, right, bottom, corner_glyph(sides.bottom, sides.right, Corner::Br));

    // Content: inset 1 on each bordered side only.
    let l = if on(sides.left) { 1 } else { 0 };
    let r = if on(sides.right) { 1 } else { 0 };
    let t = if on(sides.top) { 1 } else { 0 };
    let b = if on(sides.bottom) { 1 } else { 0 };
    let content = Rect::new(
        x + l,
        y + t,
        area.width.saturating_sub(l + r),
        area.height.saturating_sub(t + b),
    );

    // top_inset only valid when the top side is present.
    let top_inset = if on(sides.top) {
        let inset_x = x + 1;
        let inset_w = right.saturating_sub(inset_x);
        Rect::new(inset_x, y, inset_w, 1)
    } else {
        Rect::new(x, y, 0, 0)
    };

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

// ── draw_header_plain + draw_framed ─────────────────────────────────────────────

/// Draw header segments centered on a single plain row (no border brackets),
/// returning per-segment hit-rects. Used for a borderless header (top side off).
/// Segments are joined with two spaces; the active one uses `active` style.
pub fn draw_header_plain(buf: &mut Buffer, row: Rect, segments: &[InsetSegment], base: Style, active: Style) -> Vec<Rect> {
    if segments.is_empty() || row.width == 0 || row.height == 0 {
        return segments.iter().map(|_| Rect::default()).collect();
    }
    let widths: Vec<usize> = segments.iter().map(|s| s.text.chars().count()).collect();
    let sep = 2usize;
    let total: usize = widths.iter().sum::<usize>() + sep * segments.len().saturating_sub(1);
    let avail = row.width as usize;
    let leading = if total <= avail { (avail - total) / 2 } else { 0 };
    let mut cx = row.x + leading as u16;
    let mut rects = vec![Rect::default(); segments.len()];
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            for _ in 0..sep {
                if let Some(c) = buf.cell_mut((cx, row.y)) { c.set_symbol(" ").set_style(base); }
                cx += 1;
            }
        }
        let style = if seg.active { active } else { base };
        let start = cx;
        for ch in seg.text.chars() {
            if let Some(c) = buf.cell_mut((cx, row.y)) { c.set_symbol(&ch.to_string()).set_style(style); }
            cx += 1;
        }
        rects[i] = Rect::new(start, row.y, cx - start, 1);
    }
    rects
}

/// The result of drawing a pane frame: where content goes, and (optionally) where
/// the header strip should be drawn and whether that row is a border row.
#[derive(Debug, Clone, Copy)]
pub struct FramedPane {
    pub content: Rect,
    /// Where to draw the header strip, or `None` when no header is shown.
    pub header: Option<Rect>,
    /// True when `header` is a top border row (use `draw_top_inset`); false when it
    /// is a reclaimed plain content row (use `draw_header_plain`).
    pub header_bordered: bool,
}

/// Draw a pane frame choosing the composited path for `picture-frame` or the
/// per-side path otherwise, and resolve header placement from `header_on`.
pub fn draw_framed(buf: &mut Buffer, area: Rect, base: BorderStyle, sides: PaneSides, color: Style, header_on: bool) -> FramedPane {
    if base == BorderStyle::PictureFrame {
        let frame = draw_pane_frame(buf, area, BorderStyle::PictureFrame, color);
        return FramedPane {
            content: frame.content,
            header: if header_on { Some(frame.top_inset) } else { None },
            header_bordered: true,
        };
    }
    let frame = draw_pane_frame_sides(buf, area, sides, color);
    let top_present = sides.top != BorderStyle::None;
    if !header_on {
        FramedPane { content: frame.content, header: None, header_bordered: false }
    } else if top_present {
        FramedPane { content: frame.content, header: Some(frame.top_inset), header_bordered: true }
    } else {
        // Reclaim the first content row for a plain header; content drops one row.
        let c = frame.content;
        if c.height == 0 {
            FramedPane { content: c, header: None, header_bordered: false }
        } else {
            let header = Rect::new(c.x, c.y, c.width, 1);
            let content = Rect::new(c.x, c.y + 1, c.width, c.height - 1);
            FramedPane { content, header: Some(header), header_bordered: false }
        }
    }
}

// ── build_layer_segments ──────────────────────────────────────────────────────

/// An owned layer-tab segment, produced by `build_layer_segments`.
/// Unlike `InsetSegment<'a>`, this owns its text so it can be returned from a
/// function without a lifetime tying it to a caller-supplied string buffer.
/// Convert to `InsetSegment` via `as_inset()` when passing to `draw_top_inset`.
pub struct LayerSegment {
    pub text: String,
    pub active: bool,
}

impl LayerSegment {
    pub fn as_inset(&self) -> InsetSegment<'_> {
        InsetSegment { text: &self.text, active: self.active }
    }
}

/// Build one `LayerSegment` per entry in `layers`, marking the one matching
/// `active_layer` as active.  The `text` field is the layer id as a decimal string.
pub fn build_layer_segments(
    layers: &[mapper::layer::LayerId],
    active_layer: mapper::layer::LayerId,
) -> Vec<LayerSegment> {
    layers
        .iter()
        .map(|&id| LayerSegment { text: id.to_string(), active: id == active_layer })
        .collect()
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
        let area = Rect::new(0,0,9,8); // w=9,h=8,right=8,bottom=7
        let mut buf = Buffer::empty(area);
        let f = draw_pane_frame(&mut buf, area, BorderStyle::PictureFrame, Style::default());
        // Top outer border: lower-block ramp, thin at the corners rising to centre.
        assert_eq!(buf.cell((0,0)).unwrap().symbol(), "▁"); // corner, level 1
        assert_eq!(buf.cell((8,0)).unwrap().symbol(), "▁"); // corner, level 1
        assert_eq!(buf.cell((4,0)).unwrap().symbol(), "▃"); // centre, from_corner=4 -> level 3
        // Thin side blocks hug the inner frame.
        assert_eq!(buf.cell((0,3)).unwrap().symbol(), "▕"); // left side
        assert_eq!(buf.cell((8,3)).unwrap().symbol(), "▏"); // right side
        // Thin bottom block.
        assert_eq!(buf.cell((4,7)).unwrap().symbol(), "▔");
        // Inner single-line frame: corners at (1,1),(7,1),(1,6),(7,6).
        assert_eq!(buf.cell((1,1)).unwrap().symbol(), "┌");
        assert_eq!(buf.cell((7,1)).unwrap().symbol(), "┐");
        assert_eq!(buf.cell((1,6)).unwrap().symbol(), "└");
        assert_eq!(buf.cell((7,6)).unwrap().symbol(), "┘");
        assert_eq!(buf.cell((1,3)).unwrap().symbol(), "│"); // inner side
        assert_eq!(f.content, Rect::new(2,2,5,4)); // inside the inner frame
        // top_inset is the top outer row (row 0), inner span.
        assert_eq!(f.top_inset, Rect::new(1,0,7,1));
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
    fn layer_tab_segments_mark_active() {
        // build_layer_segments(layers, active) -> Vec<LayerSegment>
        let segs = build_layer_segments(&[0,1,2], 1);
        assert_eq!(segs.len(), 3);
        assert!(segs[1].active);
        assert!(!segs[0].active && !segs[2].active);
        assert_eq!(segs[0].text, "0");
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

    #[test]
    fn pane_sides_all_and_left_right_only() {
        let s = PaneSides::all(BorderStyle::Single);
        assert_eq!(s.top, BorderStyle::Single);
        assert_eq!(s.bottom, BorderStyle::Single);

        // left+right only: vertical bars, no corners, content keeps full height.
        let sides = PaneSides { top: BorderStyle::None, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::Single };
        let area = Rect::new(0, 0, 10, 4);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame_sides(&mut buf, area, sides, Style::default());
        // sides present on every row; corners are NOT drawn (no ┌ at 0,0).
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "│");
        assert_eq!(buf.cell((9, 0)).unwrap().symbol(), "│");
        assert_eq!(buf.cell((0, 3)).unwrap().symbol(), "│");
        // content inset 1 left + 1 right, full height (top/bottom open).
        assert_eq!(frame.content, Rect::new(1, 0, 8, 4));
        // no top border → top_inset has zero height.
        assert_eq!(frame.top_inset.height, 0);
    }

    #[test]
    fn draw_sides_corner_when_two_meet_and_heavier_wins() {
        // top thick + left single → top-left corner uses the thick corner (heavier wins).
        let sides = PaneSides { top: BorderStyle::Thick, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::None };
        let area = Rect::new(0, 0, 6, 4);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame_sides(&mut buf, area, sides, Style::default());
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┏"); // thick tl corner
        // top row interior is the thick horizontal; left side is the single vertical.
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "━");
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "│");
        // content inset 1 top + 1 left only.
        assert_eq!(frame.content, Rect::new(1, 1, 5, 3));
        // top present → top_inset spans the top row between the left inset and the right edge.
        assert_eq!(frame.top_inset.y, 0);
        assert_eq!(frame.top_inset.height, 1);
    }

    #[test]
    fn draw_header_plain_centers_without_brackets() {
        let row = Rect::new(0, 0, 11, 1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 11, 1));
        let rects = draw_header_plain(&mut buf, row, &[InsetSegment { text: "AB", active: false }], Style::default(), Style::default());
        let line: String = (0..11u16).map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
        // "AB" centered, no ┫/┣/┃ brackets anywhere.
        assert!(line.contains("AB"), "got {:?}", line);
        assert!(!line.contains('┫') && !line.contains('┣') && !line.contains('┃'), "no brackets: {:?}", line);
        assert_eq!(rects.len(), 1);
    }

    #[test]
    fn draw_framed_header_placement_matrix() {
        let area = Rect::new(0, 0, 12, 6);

        // header on + top present → header is the top border row (bordered).
        let mut b1 = Buffer::empty(area);
        let f1 = draw_framed(&mut b1, area, BorderStyle::Single, PaneSides::all(BorderStyle::Single), Style::default(), true);
        assert!(f1.header_bordered);
        assert_eq!(f1.header.unwrap().y, 0);
        assert_eq!(f1.content, Rect::new(1, 1, 10, 4));

        // header on + top none → header on reclaimed first content row (plain); content drops a row.
        let sides_no_top = PaneSides { top: BorderStyle::None, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
        let mut b2 = Buffer::empty(area);
        let f2 = draw_framed(&mut b2, area, BorderStyle::None, sides_no_top, Style::default(), true);
        assert!(!f2.header_bordered);
        let h2 = f2.header.unwrap();
        assert_eq!(h2.height, 1);
        // content starts one row below the header row.
        assert_eq!(f2.content.y, h2.y + 1);

        // header off → no header; content uses the inner area.
        let mut b3 = Buffer::empty(area);
        let f3 = draw_framed(&mut b3, area, BorderStyle::Single, PaneSides::all(BorderStyle::Single), Style::default(), false);
        assert!(f3.header.is_none());
        assert_eq!(f3.content, Rect::new(1, 1, 10, 4));
    }

}
