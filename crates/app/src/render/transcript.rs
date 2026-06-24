//! GAME pane rendering: status line (top), scrolling transcript (middle), input line (bottom).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use zvm::cpu::exec::Machine;
use zvm::screen::{StatusLine, StatusRight};

use crate::state::{AppState, Focus};
use super::draw_str_clipped;

// ── Styles ─────────────────────────────────────────────────────────────────────
//
// Status, normal text, and suggestion styles are read from `state.colors` at
// render time.  The CURSOR style remains a local constant as it is structural
// (REVERSED only, no color content mapped by ColorScheme).

const CURSOR_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED);

// ── Pure helpers (testable without Machine) ────────────────────────────────────

/// Format a `StatusLine` into a left-part (location) and right-part (score/turns or time).
pub(crate) fn format_status(sl: &StatusLine) -> (String, String) {
    let left = sl.location.clone();
    let right = match &sl.right {
        StatusRight::ScoreTurns { score, turns } => {
            format!("Score: {}  Moves: {}", score, turns)
        }
        StatusRight::Time { hours, minutes } => {
            format!("{:02}:{:02}", hours, minutes)
        }
    };
    (left, right)
}

/// Return the slice of transcript lines visible in `rows` rows, honouring
/// `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned slice always has ≤ `rows` entries and is ordered oldest-first
/// so the caller can draw them top-to-bottom.
///
/// Note: the renderer now uses `visible_wrapped_lines` which handles word-wrap.
/// This function is retained for unit testing the slice logic in isolation.
#[cfg(test)]
pub(crate) fn visible_lines(
    transcript: &[String],
    rows: usize,
    scroll: u16,
) -> &[String] {
    if rows == 0 || transcript.is_empty() {
        return &[];
    }
    // Total lines available.
    let n = transcript.len();
    // `scroll` offsets the window upward from the bottom.
    let scroll = scroll as usize;
    // The window ends (exclusive) at n - scroll, clamped to [0, n].
    let end = n.saturating_sub(scroll);
    // The window starts at end - rows, clamped to 0.
    let start = end.saturating_sub(rows);
    &transcript[start..end]
}

/// Truncate `line` to at most `width` characters (not bytes).
pub(crate) fn truncate_line(line: &str, width: usize) -> &str {
    // Find the byte position after `width` chars.
    let byte_pos = line
        .char_indices()
        .nth(width)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    &line[..byte_pos]
}

/// Word-wrap a single logical line into display rows of at most `width` columns.
///
/// - Tries to break at spaces (word-wrap): the line is split at the last space
///   that allows a row of ≤ `width` chars.
/// - Falls back to hard char-break for words longer than `width`.
/// - An empty line produces a single empty string (preserves blank lines).
/// - Zero width returns the line unsplit.
pub(crate) fn wrap_line(line: &str, width: u16) -> Vec<String> {
    let width = width as usize;
    if width == 0 {
        return vec![line.to_string()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut rows: Vec<String> = Vec::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        let char_count: usize = remaining.chars().count();
        if char_count <= width {
            rows.push(remaining.to_string());
            break;
        }

        // Collect byte offsets for chars 0..=width (inclusive so we see the boundary char).
        // We want the last space at char index ≤ width to break at.
        let mut last_space_before: Option<usize> = None; // byte offset of last space in 0..width
        let mut byte_at_width: usize = remaining.len();  // byte offset of char #width (the first that doesn't fit)
        for (i, (byte_i, ch)) in remaining.char_indices().enumerate() {
            if i == width {
                byte_at_width = byte_i;
                // If the char right at the boundary is a space, break here
                // (the row is exactly `width` non-space chars).
                if ch == ' ' {
                    last_space_before = Some(byte_i);
                }
                break;
            }
            if ch == ' ' {
                last_space_before = Some(byte_i);
            }
        }

        if let Some(sp) = last_space_before {
            // Break at the space: take everything before it, skip the space.
            rows.push(remaining[..sp].to_string());
            // Advance past the space (sp is a byte offset of ' ', so sp+1 is safe for ASCII ' ').
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
        } else {
            // No space found: hard-break at `width` chars.
            rows.push(remaining[..byte_at_width].to_string());
            remaining = &remaining[byte_at_width..];
        }
    }

    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Expand a slice of logical transcript lines into wrapped display rows.
pub(crate) fn wrap_lines(transcript: &[String], width: u16) -> Vec<String> {
    transcript
        .iter()
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

/// Return the slice of **wrapped** display rows visible in `rows` rows,
/// honouring `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned vec is ordered oldest-first so the caller can draw top-to-bottom.
pub(crate) fn visible_wrapped_lines(
    transcript: &[String],
    rows: usize,
    scroll: u16,
    width: u16,
) -> Vec<String> {
    if rows == 0 || transcript.is_empty() {
        return Vec::new();
    }
    let display_rows = wrap_lines(transcript, width);
    let n = display_rows.len();
    let scroll = scroll as usize;
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    display_rows[start..end].to_vec()
}

/// Format the input prompt line: `"> " + input`.
pub(crate) fn format_input_line(input: &str) -> String {
    format!("> {}", input)
}

/// Format the autocomplete suggestion bar from a list of candidates and the
/// currently-highlighted index.  Returns an empty string when `suggestions` is
/// empty.  The highlighted entry is wrapped in `[brackets]`; others are plain.
///
/// Example: `north  [northeast]  northwest`
pub(crate) fn format_suggestion_line(suggestions: &[String], highlight_idx: usize) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let idx = highlight_idx % suggestions.len();
    suggestions
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == idx {
                format!("[{}]", w)
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

// ── Main render function ───────────────────────────────────────────────────────

/// Render the GAME pane into `buf` within `area`:
///
/// - Top row: v3 status line (location left, score/turns or time right), reversed style.
/// - Middle rows: scrolling transcript from `state.transcript` (newest at bottom).
/// - Bottom row: `"> " + state.input`; cursor indicator `_` when `state.focus == Focus::Game`.
pub fn render_transcript(machine: &Machine, state: &AppState, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let w = area.width as usize;
    let status_style = state.colors.status_bar;
    let normal_style = state.colors.transcript;

    // ── Top row: status line ─────────────────────────────────────────────────

    let status_y = area.y;
    {
        // Fill entire top row with the status style first (background fill).
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, status_y)) {
                cell.set_symbol(" ").set_style(status_style);
            }
        }

        let sl = machine.status_line();
        let (left, right) = format_status(&sl);

        // Draw left (location).
        let left_trunc = truncate_line(&left, w);
        draw_str_clipped(buf, area.x, status_y, left_trunc, status_style, area);

        // Draw right (score/time), right-aligned if it fits without overlapping left.
        if right.len() < w {
            let right_x = area.x + (w - right.len()) as u16;
            draw_str_clipped(buf, right_x, status_y, &right, status_style, area);
        }
    }

    if area.height < 2 {
        return;
    }

    // ── Bottom row: input line ────────────────────────────────────────────────

    let input_y = area.bottom() - 1;
    {
        let prompt = format_input_line(&state.input);
        let prompt_trunc = truncate_line(&prompt, w);
        draw_str_clipped(buf, area.x, input_y, prompt_trunc, normal_style, area);

        // Cursor indicator when focused on Game pane.
        if state.focus == Focus::Game {
            let cursor_x = area.x + prompt_trunc.chars().count() as u16;
            if cursor_x < area.right() {
                if let Some(cell) = buf.cell_mut((cursor_x, input_y)) {
                    cell.set_symbol("_").set_style(CURSOR_STYLE);
                }
            }
        }
    }

    // ── Suggestion line: one row above input (game focus only) ───────────────

    // Reserve the row above input for suggestions when they are available.
    // The transcript area shrinks accordingly so suggestions never overlap text.
    let has_suggestions = state.focus == Focus::Game && !state.suggestions.is_empty();
    let suggestion_y = input_y.saturating_sub(1);
    if has_suggestions && area.height >= 3 && suggestion_y > area.y {
        let sug_line = format_suggestion_line(&state.suggestions, state.suggestion_idx);
        let sug_trunc = truncate_line(&sug_line, w);
        let sug_style = state.colors.suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, sug_trunc, sug_style, area);
    }

    // ── Middle rows: transcript ───────────────────────────────────────────────

    if area.height < 3 {
        return;
    }

    // Middle rows: from status_y + 1 to input_y - 1 (or input_y - 2 when the
    // suggestion line is visible, so transcript text is never overdrawn).
    let transcript_top = area.y + 1;
    let transcript_bottom = if has_suggestions && suggestion_y > area.y {
        suggestion_y // exclusive: transcript stops before the suggestion row
    } else {
        input_y // exclusive: transcript stops before the input row
    };
    let transcript_rows = (transcript_bottom - transcript_top) as usize;

    let lines = visible_wrapped_lines(
        &state.transcript,
        transcript_rows,
        state.transcript_scroll,
        area.width,
    );
    for (i, line) in lines.iter().enumerate() {
        let row_y = transcript_top + i as u16;
        if row_y >= transcript_bottom {
            break;
        }
        // Lines are already wrapped to width, just draw them.
        draw_str_clipped(buf, area.x, row_y, line, normal_style, area);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    // ── Pure helper tests (no Machine required) ──────────────────────────────

    #[test]
    fn format_status_score_turns() {
        let sl = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 10, turns: 5 },
        };
        let (left, right) = format_status(&sl);
        assert_eq!(left, "West of House");
        assert_eq!(right, "Score: 10  Moves: 5");
    }

    #[test]
    fn format_status_time() {
        let sl = StatusLine {
            location: "Hall".into(),
            right: StatusRight::Time { hours: 9, minutes: 3 },
        };
        let (left, right) = format_status(&sl);
        assert_eq!(left, "Hall");
        assert_eq!(right, "09:03");
    }

    #[test]
    fn visible_lines_newest_at_bottom() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 0 → last 5 lines: line5..line9
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[4], "line 9");
        assert_eq!(vis[0], "line 5");
    }

    #[test]
    fn visible_lines_scroll_up() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 2 → lines 3..7 (end = 10-2=8, start = 8-5=3)
        let vis = visible_lines(&transcript, 5, 2);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[0], "line 3");
        assert_eq!(vis[4], "line 7");
    }

    #[test]
    fn visible_lines_fewer_than_rows() {
        let transcript = vec!["only one".to_string()];
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0], "only one");
    }

    #[test]
    fn truncate_line_clips_at_width() {
        assert_eq!(truncate_line("hello world", 5), "hello");
        assert_eq!(truncate_line("hi", 10), "hi");
        assert_eq!(truncate_line("abc", 3), "abc");
    }

    #[test]
    fn wrap_line_basic_word_wrap() {
        // "the quick brown fox" at width 9: "the quick" + "brown fox"
        let result = wrap_line("the quick brown fox", 9);
        assert_eq!(result, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_line_hard_break_long_word() {
        // "abcdefghij" at width 4: "abcd" + "efgh" + "ij"
        let result = wrap_line("abcdefghij", 4);
        assert_eq!(result, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_line_fits_in_one_row() {
        let result = wrap_line("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_line_empty_string() {
        let result = wrap_line("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn wrap_line_exact_width() {
        // "abc" at width 3: exactly fits
        let result = wrap_line("abc", 3);
        assert_eq!(result, vec!["abc"]);
    }

    #[test]
    fn wrap_lines_expands_multiple_logical_lines() {
        let lines = vec![
            "hello world test".to_string(),
            "short".to_string(),
        ];
        // width 5: "hello" + "world" + "test" + "short"
        let result = wrap_lines(&lines, 5);
        assert_eq!(result, vec!["hello", "world", "test", "short"]);
    }

    #[test]
    fn visible_wrapped_lines_newest_at_bottom() {
        // 3 logical lines at width 5 = 3 display rows; scroll=0, rows=3
        let transcript = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
        let vis = visible_wrapped_lines(&transcript, 3, 0, 10);
        assert_eq!(vis.len(), 3);
        assert_eq!(vis[2], "ghi");
    }

    #[test]
    fn visible_wrapped_lines_scroll_offset() {
        // "hello world" wraps to ["hello", "world"] at width 5
        let transcript = vec!["hello world".to_string()];
        // scroll=1: end = 2-1=1, start = 1-1=0 → ["hello"]
        let vis = visible_wrapped_lines(&transcript, 1, 1, 5);
        assert_eq!(vis, vec!["hello"]);
        // scroll=0: end=2, start=1 → ["world"]
        let vis2 = visible_wrapped_lines(&transcript, 1, 0, 5);
        assert_eq!(vis2, vec!["world"]);
    }

    #[test]
    fn format_input_line_prefix() {
        assert_eq!(format_input_line("open mailbox"), "> open mailbox");
        assert_eq!(format_input_line(""), "> ");
    }

    // ── Render tests: transcript + input rows (no Machine) ───────────────────
    //
    // We still need a Machine for render_transcript. We build a minimal one from
    // zvm's sample_story (v3) to avoid needing a real fixture file.

    fn minimal_machine() -> Machine {
        use zvm::memory::Memory;
        // Use the same sample_story helper that zvm's own tests use.
        // It's in zvm::header::tests_support but that's cfg(test)-only.
        // Instead we build a minimal valid v3 story buffer ourselves.
        //
        // Minimum valid v3 story file:
        //   byte 0x00 = version (3)
        //   bytes 0x04-0x05 = high memory base (e.g. 0x0040)
        //   bytes 0x06-0x07 = initial PC (e.g. 0x0040)
        //   bytes 0x0A-0x0B = dictionary base (0x0080)
        //   bytes 0x0C-0x0D = object table base (0x0100)
        //   bytes 0x0E-0x0F = global var table base (0x0300)
        //   bytes 0x08-0x09 = static mem base (0x0400)
        //   bytes 0x02-0x03 = (release number, ignored)
        //   Total: 0x500 bytes should be enough.
        //
        // We use the same layout as zvm/src/header.rs tests_support::sample_story(3).
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;                       // version = 3
        // high_mem_base = 0x0040
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        // initial_pc = 0x0040 (will contain a QUIT/quit opcode: 0x00 = rtrue? use 0xba = quit)
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dict base = 0x0080
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        // object table = 0x0100
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        // global var table = 0x0300
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        // static mem base = 0x0400 (dynamic = 0x0000..0x03FF)
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        // abbreviation table = 0x0060 (word at 0x18-0x19)
        buf[0x18] = 0x00; buf[0x19] = 0x60;

        // Place a valid dictionary at 0x0080: word-separators count=0, entry_size=4, entry_count=0.
        buf[0x0080] = 0; // 0 word-separators
        buf[0x0081] = 4; // entry size = 4 bytes
        buf[0x0082] = 0; buf[0x0083] = 0; // entry count = 0

        // Object table at 0x0100: 31 prop-default words (62 bytes), then no objects.
        // Property defaults: all zero (62 bytes, already 0).

        // Put a QUIT opcode at 0x0040 so stepping won't panic.
        buf[0x0040] = 0xba; // opcode for 'quit' in v3 (0OP:0x0a → encoded as 0xba).

        let mem = Memory::new(buf).expect("minimal v3 story should be valid");
        Machine::new(mem)
    }

    #[test]
    fn render_transcript_input_and_transcript_lines() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = vec![
            "You are in a hall.".to_string(),
            "It is dark.".to_string(),
        ];
        state.input = "open mailbox".to_string();
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Bottom row (y=9) should contain "> open mailbox".
        let bottom_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("> open mailbox"),
            "bottom row should contain '> open mailbox'; got: {:?}",
            bottom_row
        );

        // A middle row should contain one of the transcript lines.
        let found_transcript = (1u16..9u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("You are in a hall.") || row.contains("It is dark.")
        });
        assert!(found_transcript, "a middle row should contain a transcript line");
    }

    #[test]
    fn render_transcript_cursor_shown_when_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "hi".to_string();
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Cursor at position 4 ("> hi" = 4 chars → cursor at x=4).
        let cursor_cell = buf.cell((4, 4)).expect("cursor cell should exist");
        assert_eq!(cursor_cell.symbol(), "_", "cursor should be '_' at end of input");
        assert!(
            cursor_cell.modifier.contains(Modifier::REVERSED),
            "cursor cell should have REVERSED modifier"
        );
    }

    #[test]
    fn render_transcript_no_cursor_when_not_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "hi".to_string();
        state.focus = Focus::Map; // not focused on game

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Position x=4 should not have '_'.
        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert_ne!(cell.symbol(), "_", "no cursor when focus is Map");
    }

    #[test]
    fn render_transcript_status_line_reversed() {
        let machine = minimal_machine();
        let state = AppState::default();

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Top row (y=0) should all have REVERSED modifier (status line background).
        let top_cell = buf.cell((0, 0)).expect("top-left cell should exist");
        assert!(
            top_cell.modifier.contains(Modifier::REVERSED),
            "top row should have REVERSED modifier for status line"
        );
    }

    #[test]
    fn render_transcript_scroll_offset() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // 10 lines, scroll=5 should show lines 0..4 (end=10-5=5, start=5-4=1 for 4-row middle)
        state.transcript = (0..10).map(|i| format!("L{}", i)).collect();
        state.transcript_scroll = 5;

        // 7-row area: 1 status + 5 transcript + 1 input
        let area = Rect::new(0, 0, 40, 7);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Middle rows y=1..5: should NOT show L9 (newest) but should show L4 or earlier.
        let found_l9 = (1u16..6u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("L9")
        });
        assert!(!found_l9, "L9 (newest) should not be visible when scrolled back 5");
    }

    // ── Status line test with czech.z5 fixture (skipped if absent) ───────────

    // ── format_suggestion_line tests ─────────────────────────────────────────

    #[test]
    fn format_suggestion_line_empty() {
        assert_eq!(format_suggestion_line(&[], 0), "");
    }

    #[test]
    fn format_suggestion_line_single_highlighted() {
        let sug = vec!["north".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert_eq!(line, "[north]");
    }

    #[test]
    fn format_suggestion_line_highlight_first() {
        let sug = vec!["north".to_string(), "northeast".to_string(), "northwest".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert!(line.starts_with("[north]"), "first entry should be highlighted: {}", line);
        assert!(line.contains("northeast") && !line.contains("[northeast]"));
    }

    #[test]
    fn format_suggestion_line_highlight_second() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        let line = format_suggestion_line(&sug, 1);
        assert!(line.contains("[northeast]"), "second entry highlighted: {}", line);
        assert!(!line.contains("[north]"), "first not highlighted: {}", line);
    }

    #[test]
    fn format_suggestion_line_idx_wraps() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        // idx=2 wraps to 0
        let line = format_suggestion_line(&sug, 2);
        assert!(line.starts_with("[north]"), "idx wraps: {}", line);
    }

    #[test]
    fn render_transcript_shows_suggestion_line_above_input() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input = "nor".to_string();
        state.suggestions = vec!["north".to_string()];
        state.suggestion_idx = 0;

        // 10-row area: row 0=status, rows 1..7=transcript, row 8=suggestion, row 9=input
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Row 9 (bottom) must contain the input.
        let input_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(input_row.contains("> nor"), "input row: {:?}", input_row);

        // Row 8 must contain the suggestion.
        let sug_row: String = (0..40u16)
            .map(|x| buf.cell((x, 8)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(sug_row.contains("north"), "suggestion row: {:?}", sug_row);
    }

    #[test]
    fn render_transcript_status_line_nonblank_with_fixture() {
        let fixture = std::path::Path::new(
            "/Volumes/Videos/Source/babelmap/crates/zvm/tests/fixtures/czech.z5",
        );
        if !fixture.exists() {
            eprintln!("SKIP: czech.z5 fixture not found");
            return;
        }

        let data = std::fs::read(fixture).expect("read czech.z5");
        let mem = zvm::memory::Memory::new(data).expect("parse czech.z5");
        let machine = Machine::new(mem);

        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Top row should have at least one non-space character (status line text or reversed bg).
        // We verify the REVERSED modifier is present on the whole row.
        let top_has_reversed = (0..80u16).all(|x| {
            buf.cell((x, 0))
                .map(|c| c.modifier.contains(Modifier::REVERSED))
                .unwrap_or(false)
        });
        assert!(
            top_has_reversed,
            "status line row should be fully reversed-video for czech.z5"
        );
    }
}
