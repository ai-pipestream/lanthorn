//! Hand-laid-out lower-window text in a pane narrower than the width the game was
//! told (SQ-1810).
//!
//! A Version 4+ story that caches the screen width is told 80 columns whatever the
//! pane is (SQ-1809), and some of those stories lay lower-window text out for 80 by
//! hand, with leading spaces: AMFV's opening quotation is indented 22, 23 and 39
//! columns so it sits as a centred block. lanthorn reflows window 0 to the pane, so
//! in a 45-column pane the long line wraps and its attribution lands on a row of
//! its own.
//!
//! The rule, applied when the transcript is wrapped for a pane `W` columns wide by a
//! story told `T`:
//!
//! * a **candidate** line is one hard-broken line (one transcript line, never a soft
//!   wrap) that starts with at least `MIN_HAND_LAID_INDENT` (4) spaces, has text
//!   after it, and whose whole
//!   length (indent plus text, trailing spaces ignored) is at most `T`. A prose
//!   paragraph longer than `T` is never a candidate, and neither is a paragraph
//!   indented 1-3 spaces (LGOP r4 uses 3, Nord and Bert r19 uses 2): such a line
//!   keeps its indent and breaks a run like any other text line;
//! * a **block** is a maximal run of consecutive candidates. Empty lines inside the
//!   run do not break it; any other line does;
//! * a block whose furthest right edge (`indent + length`, maximised) passes `W` is
//!   shifted left, every line by the same `min(overflow, smallest indent)`. A block
//!   that fits is left alone, and a pane wider than `T` changes nothing (no
//!   re-centring). Lines still wider than `W` after the shift wrap as before.
//!
//! This is a **render-time transform**, recomputed for every width and never
//! written back: the stored transcript, saves and the archive keep the game's own
//! text, so resizing back to 80 restores the original indents. An embedding host
//! that reflows text itself calls [`block_shifts`] with the same inputs and gets the
//! same answer as the TUI, which calls it too.

use crate::state::StyleRun;
use crate::textwidth::str_cells;

/// The smallest indent that marks a line as hand-laid. Short prose paragraphs are
/// indented by the games' own paragraph convention, not laid out as a block: Leather
/// Goddesses of Phobos r4 indents them 3 spaces, Nord and Bert r19 indents them 2.
/// Shifting those in a narrow pane would strip a first-line indent that is the
/// paragraph's only marker, so lines indented fewer than this are plain text.
const MIN_HAND_LAID_INDENT: usize = 4;

/// One line's layout facts: `Some((indent, edge))` for a candidate, `None` for
/// anything else. A candidate's `edge` is `indent + length`, in cells.
fn candidate(line: &str, told: u16) -> Option<(usize, usize)> {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    if indent < MIN_HAND_LAID_INDENT {
        return None;
    }
    let body = line.trim_end_matches(' ');
    if body.len() <= indent {
        return None; // all spaces: an empty line, not a candidate
    }
    let edge = indent + str_cells(&body[indent..]);
    (edge <= told as usize).then_some((indent, edge))
}

/// Is `line` blank (empty or only spaces)? Blank lines sit inside a block without
/// breaking it.
fn is_blank(line: &str) -> bool {
    line.trim_end_matches(' ').is_empty()
}

/// Is `line` a candidate for the block shift in a story told `told` columns?
pub fn is_hand_laid(line: &str, told: u16) -> bool {
    candidate(line, told).is_some()
}

/// How many columns to take off the left of each of `lines` (hard-broken lower-window
/// lines, oldest first) for a pane `width` columns wide, in a story told `told`
/// columns. One entry per line; 0 for every line outside a block that overflows.
///
/// The shift never exceeds a line's own indent, so removing that many leading
/// spaces is always lossless of text.
pub fn block_shifts<S: AsRef<str>>(lines: &[S], told: u16, width: u16) -> Vec<u16> {
    let mut out = vec![0u16; lines.len()];
    if width == 0 || width >= told {
        // Nothing can overflow a pane at least as wide as the told width: every
        // candidate ends within `told`.
        return out;
    }
    let mut i = 0;
    while i < lines.len() {
        if candidate(lines[i].as_ref(), told).is_none() {
            i += 1;
            continue;
        }
        // Grow the block over candidates and the blank lines between them.
        let mut members: Vec<usize> = Vec::new();
        let mut j = i;
        while j < lines.len() {
            let l = lines[j].as_ref();
            if candidate(l, told).is_some() {
                members.push(j);
            } else if !is_blank(l) {
                break;
            }
            j += 1;
        }
        let (mut min_indent, mut edge) = (usize::MAX, 0usize);
        for &m in &members {
            let (indent, e) = candidate(lines[m].as_ref(), told).expect("members are candidates");
            min_indent = min_indent.min(indent);
            edge = edge.max(e);
        }
        let overflow = edge.saturating_sub(width as usize);
        let shift = overflow.min(min_indent) as u16;
        for &m in &members {
            out[m] = shift;
        }
        i = j;
    }
    out
}

/// Drop the first `shift` columns (all leading spaces — see [`block_shifts`]) from
/// `line`, and move `runs` left with the text. A run that lay wholly inside the
/// dropped spaces disappears; one that straddled the edge is clipped to it.
pub fn shift_line_left(line: &str, runs: &[StyleRun], shift: u16) -> (String, Vec<StyleRun>) {
    let s = shift as usize;
    if s == 0 {
        return (line.to_string(), runs.to_vec());
    }
    let text: String = line.chars().skip(s).collect();
    let runs = runs
        .iter()
        .filter(|r| r.end > s)
        .map(|r| StyleRun { start: r.start.saturating_sub(s), end: r.end - s, ..*r })
        .collect();
    (text, runs)
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;

    fn amfv_quote() -> Vec<String> {
        vec![
            format!("{}\"Tomorrow never yet", " ".repeat(22)),
            format!("{}On any human being rose or set.", " ".repeat(23)),
            format!("{}-- William Marsden", " ".repeat(39)),
        ]
    }

    fn indents(lines: &[String], shifts: &[u16]) -> Vec<usize> {
        lines
            .iter()
            .zip(shifts)
            .map(|(l, s)| shift_line_left(l, &[], *s).0.chars().take_while(|c| *c == ' ').count())
            .collect()
    }

    #[test]
    fn the_amfv_quotation_shifts_to_10_11_27_at_45_columns() {
        let q = amfv_quote();
        let shifts = block_shifts(&q, 80, 45);
        assert_eq!(shifts, vec![12, 12, 12]);
        assert_eq!(indents(&q, &shifts), vec![10, 11, 27]);
    }

    #[test]
    fn a_block_that_fits_or_a_wider_pane_changes_nothing() {
        let q = amfv_quote();
        assert_eq!(block_shifts(&q, 80, 80), vec![0, 0, 0], "pane == told");
        assert_eq!(block_shifts(&q, 80, 120), vec![0, 0, 0], "wider pane: no re-centring");
        assert_eq!(block_shifts(&q, 80, 57), vec![0, 0, 0], "the block's edge is exactly the pane");
        assert_eq!(block_shifts(&q, 80, 56), vec![1, 1, 1]);
    }

    #[test]
    fn the_smallest_indent_caps_the_shift() {
        let lines = vec![format!("{}short", " ".repeat(4)), format!("{}{}", " ".repeat(30), "x".repeat(40))];
        // edge 70, pane 40: overflow 30, but the first line only has 4 spaces.
        assert_eq!(block_shifts(&lines, 80, 40), vec![4, 4]);
    }

    #[test]
    fn a_blank_line_inside_a_block_does_not_break_it() {
        let lines = vec![
            format!("{}AAAA", " ".repeat(30)),
            String::new(),
            format!("{}BBBBBBBBBBBBBBBBBBBB", " ".repeat(40)),
            "   ".to_string(),
            format!("{}CC", " ".repeat(31)),
        ];
        // edge 60, pane 40: overflow 20, min indent 30 -> 20 for all three.
        assert_eq!(block_shifts(&lines, 80, 40), vec![20, 0, 20, 0, 20]);
    }

    #[test]
    fn a_prose_paragraph_longer_than_the_told_width_is_never_a_candidate() {
        let prose = format!("  {}", "word ".repeat(30)); // > 80 cells
        assert!(prose.trim_end().chars().count() > 80);
        assert_eq!(block_shifts(&[prose], 80, 40), vec![0]);
    }

    #[test]
    fn a_text_line_between_two_blocks_splits_them() {
        let lines = vec![
            format!("{}{}", " ".repeat(20), "a".repeat(30)),
            "plain text, no indent".to_string(),
            format!("{}{}", " ".repeat(4), "b".repeat(10)),
        ];
        // First block alone overflows (edge 50 vs 40): shift 10. The second
        // block fits and is untouched, though joined it would have shifted by 4.
        assert_eq!(block_shifts(&lines, 80, 40), vec![10, 0, 0]);
    }

    #[test]
    fn a_three_space_short_line_wider_than_the_pane_is_not_shifted() {
        let lines = vec![format!("   {}", "p".repeat(50))];
        assert!(!is_hand_laid(&lines[0], 80));
        assert_eq!(block_shifts(&lines, 80, 40), vec![0]);
    }

    #[test]
    fn a_two_space_line_between_two_wide_indent_blocks_splits_them() {
        let lines = vec![
            format!("{}{}", " ".repeat(20), "a".repeat(30)),
            format!("  {}", "b".repeat(10)),
            format!("{}{}", " ".repeat(8), "c".repeat(40)),
        ];
        // Joined, the 2-space line would cap the shift at 2 for all three. Split:
        // the first moves 10, the last 8, the middle none.
        assert_eq!(block_shifts(&lines, 80, 40), vec![10, 0, 8]);
    }

    #[test]
    fn a_four_space_line_is_still_shifted() {
        let lines = vec![format!("    {}", "p".repeat(50))];
        assert!(is_hand_laid(&lines[0], 80));
        assert_eq!(block_shifts(&lines, 80, 40), vec![4]);
    }

    #[test]
    fn runs_move_with_their_text_and_clip_at_the_edge() {
        let line = format!("{}hello", " ".repeat(10));
        let runs = vec![
            StyleRun { start: 0, end: 12, bits: 1, ..Default::default() }, // straddles the edge
            StyleRun { start: 12, end: 15, bits: 2, ..Default::default() },
            StyleRun { start: 2, end: 5, bits: 4, ..Default::default() }, // inside the dropped spaces
        ];
        let (text, out) = shift_line_left(&line, &runs, 6);
        assert_eq!(text, format!("{}hello", " ".repeat(4)));
        assert_eq!(out.len(), 2, "the run wholly in the dropped spaces is gone");
        assert_eq!((out[0].start, out[0].end, out[0].bits), (0, 6, 1));
        assert_eq!((out[1].start, out[1].end, out[1].bits), (6, 9, 2));
    }
}
