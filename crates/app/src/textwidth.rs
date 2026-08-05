//! Display-width ↔ char-index conversions (SQ-0655).
//!
//! Terminal cells are measured in DISPLAY COLUMNS, not in Unicode scalars: a CJK
//! ideograph or an emoji occupies two cells, a combining mark occupies none. Any
//! code that turns a screen column into a string offset — a mouse click, a
//! selection span, a clip-to-width — has to convert between the two, or it reads
//! the wrong character out of the string it just drew.
//!
//! These are the conversion helpers, kept in one place so the selection, caret and
//! clipping paths agree on the same measurement.
//!
//! Control characters count as ONE cell here: the renderers
//! (`render::draw_char_clipped` and friends) blank a control to a space rather
//! than dropping it, so it does occupy a cell on screen even though
//! `unicode-width` reports no width for it.

use unicode_width::UnicodeWidthChar;

/// Cells `ch` occupies on screen: 2 for wide (CJK/emoji), 0 for combining marks
/// and other zero-width scalars, 1 for everything else — including control
/// characters, which the renderers draw as a blank cell.
pub fn char_cells(ch: char) -> usize {
    if ch.is_control() {
        return 1;
    }
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Total cells `s` occupies on screen.
pub fn str_cells(s: &str) -> usize {
    s.chars().map(char_cells).sum()
}

/// Cells occupied by the first `idx` CHARS of `s` — i.e. the display column at
/// which char `idx` starts. Saturates at the string's full width.
pub fn cols_of_chars(s: &str, idx: usize) -> usize {
    s.chars().take(idx).map(char_cells).sum()
}

/// Char index of the glyph drawn at display column `col`.
///
/// Both cells of a double-width glyph map to that glyph's own index (so a click
/// anywhere on it puts the caret before it), and a column past the end of `s`
/// maps to the char count (the caret goes after the last char).
pub fn col_to_char_idx(s: &str, col: usize) -> usize {
    let mut c = 0usize;
    for (i, ch) in s.chars().enumerate() {
        let w = char_cells(ch);
        // A zero-width scalar starts and ends at the same column; it belongs to
        // the glyph before it and never owns a column of its own.
        if w > 0 && col < c + w {
            return i;
        }
        c += w;
    }
    s.chars().count()
}

/// The substring of `s` covering the INCLUSIVE display-column range `[c0, c1]`.
///
/// A glyph is taken when any of its cells falls inside the range, so a selection
/// that clips a double-width glyph copies the whole glyph rather than half of it.
/// Zero-width scalars (combining marks, variation selectors, ZWJ) follow the
/// glyph they modify, so a selected emoji comes out whole.
pub fn slice_cols(s: &str, c0: usize, c1: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    let mut took_prev = false;
    for ch in s.chars() {
        let w = char_cells(ch);
        let take = if w == 0 {
            took_prev
        } else {
            col <= c1 && col + w > c0
        };
        if take {
            out.push(ch);
        }
        took_prev = take;
        col += w;
        // Past the range with nothing left to attach to: no later glyph can join.
        if col > c1 && !took_prev {
            break;
        }
    }
    out
}

/// The longest prefix of `s` that fits in `max` display columns. A double-width
/// glyph that would straddle the limit is dropped whole (the last column is left
/// blank rather than half-painted).
pub fn truncate_to_cols(s: &str, max: usize) -> &str {
    let mut col = 0usize;
    for (b, ch) in s.char_indices() {
        let w = char_cells(ch);
        if col + w > max {
            return &s[..b];
        }
        col += w;
    }
    s
}

/// Clip `s` to at most `max` display columns, marking a shortened string with a
/// trailing `…` (which costs one of those columns).
pub fn clip_to_cols_ellipsis(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if str_cells(s) <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }
    let mut out = truncate_to_cols(s, max - 1).to_string();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_counts_wide_and_zero_width() {
        assert_eq!(char_cells('a'), 1);
        assert_eq!(char_cells('日'), 2);
        assert_eq!(char_cells('\u{0301}'), 0, "combining acute is zero-width");
        assert_eq!(char_cells('\u{200d}'), 0, "ZWJ is zero-width");
        assert_eq!(char_cells('\u{7}'), 1, "control chars render as one blank cell");
        assert_eq!(str_cells("日本語"), 6);
        assert_eq!(str_cells("ab日"), 4);
    }

    #[test]
    fn col_to_char_idx_maps_both_cells_of_a_wide_glyph() {
        // "a日b" → columns: a=0, 日=1..2, b=3.
        let s = "a日b";
        assert_eq!(col_to_char_idx(s, 0), 0);
        assert_eq!(col_to_char_idx(s, 1), 1);
        assert_eq!(col_to_char_idx(s, 2), 1, "right cell of 日 still selects 日");
        assert_eq!(col_to_char_idx(s, 3), 2);
        assert_eq!(col_to_char_idx(s, 4), 3, "past the end → char count");
        assert_eq!(col_to_char_idx(s, 99), 3);
        assert_eq!(col_to_char_idx("", 0), 0);
    }

    #[test]
    fn cols_of_chars_is_the_inverse() {
        let s = "a日b";
        assert_eq!(cols_of_chars(s, 0), 0);
        assert_eq!(cols_of_chars(s, 1), 1);
        assert_eq!(cols_of_chars(s, 2), 3);
        assert_eq!(cols_of_chars(s, 3), 4);
        assert_eq!(cols_of_chars(s, 99), 4);
    }

    #[test]
    fn slice_cols_is_ascii_identical() {
        assert_eq!(slice_cols("HELLOWORLD", 2, 5), "LLOW");
        assert_eq!(slice_cols("hi", 0, 9), "hi");
        assert_eq!(slice_cols("hello", 9, 12), "");
    }

    #[test]
    fn slice_cols_takes_whole_wide_glyphs() {
        // "日本語" occupies columns 0..5.
        assert_eq!(slice_cols("日本語", 0, 1), "日");
        assert_eq!(slice_cols("日本語", 2, 3), "本");
        assert_eq!(slice_cols("日本語", 0, 3), "日本");
        // A range that clips a glyph in half still yields the whole glyph.
        assert_eq!(slice_cols("日本語", 1, 2), "日本");
        assert_eq!(slice_cols("日本語", 3, 5), "本語");
    }

    #[test]
    fn slice_cols_keeps_combining_marks_with_their_base() {
        let s = "e\u{0301}x"; // "é" as e + combining acute, then x
        assert_eq!(slice_cols(s, 0, 0), "e\u{0301}");
        assert_eq!(slice_cols(s, 1, 1), "x");
    }

    #[test]
    fn truncate_and_clip_respect_width() {
        assert_eq!(truncate_to_cols("日本語", 5), "日本", "a wide glyph never straddles the limit");
        assert_eq!(truncate_to_cols("日本語", 6), "日本語");
        assert_eq!(truncate_to_cols("abc", 2), "ab");
        assert_eq!(clip_to_cols_ellipsis("abcdef", 4), "abc…");
        assert_eq!(clip_to_cols_ellipsis("abc", 4), "abc");
        assert_eq!(clip_to_cols_ellipsis("日本語", 4), "日…");
        assert!(str_cells(&clip_to_cols_ellipsis("日本語です", 5)) <= 5);
        assert_eq!(clip_to_cols_ellipsis("abc", 1), "…");
        assert_eq!(clip_to_cols_ellipsis("abc", 0), "");
    }
}
