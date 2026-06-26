//! Text selection + clipboard copy for the story pane (no dependencies).
//!
//! Selection is a linear (browser-style) range from an anchor to a head, but
//! every row is clamped to the story pane's column band — so a wrapped/middle
//! row never pulls in text from a side-by-side map pane. Copy is delivered via
//! the OSC 52 terminal escape, which works over SSH and needs no clipboard lib.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// A text selection in absolute screen coordinates (x, y).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (u16, u16),
    pub head: (u16, u16),
}

impl Selection {
    pub fn new(at: (u16, u16)) -> Self {
        Selection { anchor: at, head: at }
    }
    /// True when the selection covers more than a single cell.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Order two points in reading order (top-to-bottom, then left-to-right).
fn ordered(a: (u16, u16), b: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) }
}

/// The inclusive column span selected on row `y`, clamped to `area`'s columns.
/// Returns `None` if `y` is outside the selection or the area.
fn row_span(area: Rect, a: (u16, u16), b: (u16, u16), y: u16) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if y < area.y || y >= area.bottom() {
        return None;
    }
    let ((sx, sy), (ex, ey)) = ordered(a, b);
    if y < sy || y > ey {
        return None;
    }
    let right = area.right() - 1;
    let c0 = if y == sy { sx.max(area.x) } else { area.x };
    let c1 = if y == ey { ex.min(right) } else { right };
    if c0 > c1 || c0 > right || c1 < area.x {
        return None;
    }
    Some((c0.max(area.x), c1.min(right)))
}

/// True if cell `(x, y)` falls within the selection, clamped to `area`.
pub fn contains(area: Rect, sel: Selection, x: u16, y: u16) -> bool {
    match row_span(area, sel.anchor, sel.head, y) {
        Some((c0, c1)) => x >= c0 && x <= c1,
        None => false,
    }
}

/// Extract the selected text from `buf`, clamped to `area`'s columns. Rows are
/// joined with `\n`; trailing whitespace on each row is trimmed.
pub fn extract(buf: &Buffer, area: Rect, sel: Selection) -> String {
    let ((_, sy), (_, ey)) = ordered(sel.anchor, sel.head);
    let y0 = sy.max(area.y);
    let y1 = ey.min(area.bottom().saturating_sub(1));
    let mut rows: Vec<String> = Vec::new();
    let mut y = y0;
    while y <= y1 {
        if let Some((c0, c1)) = row_span(area, sel.anchor, sel.head, y) {
            let mut line = String::new();
            let mut x = c0;
            while x <= c1 {
                if let Some(cell) = buf.cell((x, y)) {
                    line.push_str(cell.symbol());
                }
                x += 1;
            }
            rows.push(line.trim_end().to_string());
        }
        if y == u16::MAX {
            break;
        }
        y += 1;
    }
    rows.join("\n")
}

/// Highlight the selection in `buf` (reversed video) and return the selected
/// text, clamped to `area`. Returns `None` for an empty (single-cell)
/// selection. Used during render so the copy reads THIS frame's buffer (the
/// terminal's back-buffer is reset after draw and can't be read afterwards).
pub fn highlight_and_extract(buf: &mut Buffer, area: Rect, sel: Selection) -> Option<String> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if contains(area, sel, x, y) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    let s = cell.style();
                    cell.set_style(s.add_modifier(Modifier::REVERSED));
                }
            }
        }
    }
    if sel.is_empty() {
        None
    } else {
        Some(extract(buf, area, sel))
    }
}

/// Standard base64 (RFC 4648) with padding.
pub fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Build the OSC 52 "set clipboard" escape sequence for `text`.
pub fn osc52_copy_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn osc52_wraps_base64_in_escape() {
        assert_eq!(osc52_copy_sequence("Man"), "\x1b]52;c;TWFu\x07");
    }

    fn buf_with(rows: &[&str], area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                let x = area.x + c as u16;
                let y = area.y + r as u16;
                if x < area.right() && y < area.bottom() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(&ch.to_string());
                    }
                }
            }
        }
        buf
    }

    #[test]
    fn extract_single_row_substring() {
        // Story pane occupies columns 10..20.
        let area = Rect::new(10, 0, 10, 3);
        let buf = buf_with(&["HELLOWORLD"], area);
        // Select columns 12..=15 on row 0 → "LLOW".
        let sel = Selection { anchor: (12, 0), head: (15, 0) };
        assert_eq!(extract(&buf, area, sel), "LLOW");
    }

    #[test]
    fn extract_multi_row_is_clamped_to_story_columns() {
        // Story pane is columns 10..20; a map would live in columns 0..10.
        let area = Rect::new(10, 0, 10, 3);
        let buf = buf_with(&["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"], area);
        // Selection from (13,0) to (14,2) — middle row spans the full story band,
        // never columns < 10 (the map).
        let sel = Selection { anchor: (13, 0), head: (14, 2) };
        let out = extract(&buf, area, sel);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "aaaaaaa", "first row from col 13 to story right edge");
        assert_eq!(lines[1], "bbbbbbbbbb", "middle row is full story width");
        assert_eq!(lines[2], "ccccc", "last row from story left to col 14");
    }

    #[test]
    fn highlight_and_extract_returns_text_and_marks_cells() {
        let area = Rect::new(10, 0, 10, 2);
        let mut buf = buf_with(&["HELLOWORLD", "secondrowX"], area);
        let sel = Selection { anchor: (12, 0), head: (15, 0) }; // "LLOW"
        let text = highlight_and_extract(&mut buf, area, sel);
        assert_eq!(text.as_deref(), Some("LLOW"));
        // Selected cells are reversed; an in-area but unselected cell is not.
        assert!(buf.cell((12, 0)).unwrap().modifier.contains(Modifier::REVERSED));
        assert!(!buf.cell((16, 0)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn highlight_and_extract_empty_selection_is_none() {
        let area = Rect::new(10, 0, 10, 2);
        let mut buf = buf_with(&["HELLOWORLD"], area);
        let sel = Selection::new((12, 0)); // anchor == head
        assert_eq!(highlight_and_extract(&mut buf, area, sel), None);
    }

    #[test]
    fn contains_excludes_columns_outside_story() {
        let area = Rect::new(10, 0, 10, 3);
        let sel = Selection { anchor: (13, 0), head: (14, 2) };
        // A middle-row cell at column 5 (in the map band) is never contained.
        assert!(!contains(area, sel, 5, 1));
        // A middle-row cell inside the story band is contained.
        assert!(contains(area, sel, 12, 1));
        // Last row (==ey): columns up to the head (14) are contained.
        assert!(contains(area, sel, 12, 2));
        // A row beyond the selection is not contained.
        assert!(!contains(area, sel, 12, 3));
    }
}
