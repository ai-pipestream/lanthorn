//! Basic DOS-style screen model for zvm-cli: pure formatting/SGR/terminal
//! helpers (this module) plus the stateful `ScreenView` (Task 3).

// Helpers are wired into the run loop in the final integration task; until then
// they are only reachable from unit tests, which would dead-code-warn in a
// binary crate. Removed once `main.rs` consumes them.
#![allow(dead_code)]

use zvm::screen::{StatusLine, StatusRight, UpperWindow};

pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// SGR set-codes for a Z-machine text-style bitmask (no leading/trailing reset).
/// 1=reverse, 2=bold, 4=italic; 8 (fixed-pitch) has no terminal equivalent here.
pub fn sgr_set(style: u8) -> String {
    let mut s = String::new();
    if style & 0x01 != 0 {
        s.push_str("\x1b[7m");
    }
    if style & 0x02 != 0 {
        s.push_str("\x1b[1m");
    }
    if style & 0x04 != 0 {
        s.push_str("\x1b[3m");
    }
    s
}

/// Wrap lower-window text in SGR when on a TTY and a style is set; else plain.
pub fn style_wrap(s: &str, style: u8, is_tty: bool) -> String {
    if !is_tty || style == 0 {
        return s.to_string();
    }
    format!("{}{}\x1b[0m", sgr_set(style), s)
}

/// Terminal BEL per bleep, TTY-gated.
pub fn bleep_bytes(count: usize, is_tty: bool) -> String {
    if is_tty {
        "\x07".repeat(count)
    } else {
        String::new()
    }
}

/// Raw single-key `read_char` only makes sense on a TTY stdin.
pub fn wants_raw_char(stdin_is_tty: bool) -> bool {
    stdin_is_tty
}

/// Parse `stty size` output ("rows cols").
pub fn parse_stty_size(out: &str) -> Option<(u16, u16)> {
    let mut it = out.split_whitespace();
    let rows = it.next()?.parse().ok()?;
    let cols = it.next()?.parse().ok()?;
    Some((rows, cols))
}

/// Resolve the terminal row count: stty size, then env LINES, then default.
pub fn term_rows(stty_out: Option<&str>, env_lines: Option<&str>) -> u16 {
    if let Some((rows, _)) = stty_out.and_then(parse_stty_size) {
        if rows > 0 {
            return rows;
        }
    }
    if let Some(n) = env_lines.and_then(|s| s.trim().parse::<u16>().ok()) {
        if n > 0 {
            return n;
        }
    }
    DEFAULT_ROWS
}

fn right_field(right: &StatusRight) -> String {
    match right {
        StatusRight::ScoreTurns { score, turns } => format!("Score: {score}  Moves: {turns}"),
        StatusRight::Time { hours, minutes } => format!("Time: {hours:02}:{minutes:02}"),
    }
}

/// Plain v3 status row: " `location` ... `right` ", padded to exactly `cols`.
pub fn status_text(st: &StatusLine, cols: u16) -> String {
    let cols = cols as usize;
    if cols < 2 {
        return " ".repeat(cols);
    }
    let inner = cols - 2; // one border space each side
    let right = right_field(&st.right);
    let right_w = right.chars().count().min(inner);
    let right: String = right.chars().take(right_w).collect();
    let left_max = inner - right_w;
    let left: String = st.location.chars().take(left_max).collect();
    let fill = inner - left.chars().count() - right_w;
    format!(" {}{}{} ", left, " ".repeat(fill), right)
}

/// Plain text of one upper-window row, trailing blanks trimmed.
pub fn upper_row_text(upper: &UpperWindow, row: u16) -> String {
    let mut s = String::new();
    for c in 1..=upper.cols {
        s.push(upper.cell(row, c).ch);
    }
    s.trim_end().to_string()
}

/// One upper-window row with per-cell SGR runs (for the pinned TTY region).
pub fn upper_row_ansi(upper: &UpperWindow, row: u16) -> String {
    // Last column with non-blank content (a blank cell is ' ' at style 0);
    // trailing blanks are dropped so the row closes with a reset, matching the
    // line-clear (`ESC[2K`) done before each row is written in the TTY region.
    let last = (1..=upper.cols)
        .rev()
        .find(|&c| {
            let cell = upper.cell(row, c);
            cell.ch != ' ' || cell.style != 0
        })
        .unwrap_or(0);
    let mut out = String::new();
    let mut cur = 0u8;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        if cell.style != cur {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_set(cell.style));
            cur = cell.style;
        }
        out.push(cell.ch);
    }
    if cur != 0 {
        out.push_str("\x1b[0m");
    }
    out
}

/// Set the scroll region below the pinned rows and park the cursor at the
/// bottom of the lower region.
pub fn enter_region(top_rows: u16, term_rows: u16) -> String {
    format!(
        "\x1b[{};{}r\x1b[{};1H",
        top_rows + 1,
        term_rows,
        term_rows
    )
}

/// Reset the scroll region to the full screen.
pub fn leave_region() -> String {
    "\x1b[r".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight, UpperWindow};

    #[test]
    fn sgr_set_maps_bits() {
        assert_eq!(sgr_set(0), "");
        assert_eq!(sgr_set(1), "\x1b[7m"); // reverse
        assert_eq!(sgr_set(2), "\x1b[1m"); // bold
        assert_eq!(sgr_set(4), "\x1b[3m"); // italic
        assert_eq!(sgr_set(8), ""); // fixed-pitch ignored
        assert_eq!(sgr_set(1 | 2), "\x1b[7m\x1b[1m");
    }

    #[test]
    fn style_wrap_only_when_tty_and_styled() {
        assert_eq!(style_wrap("hi", 0, true), "hi");
        assert_eq!(style_wrap("hi", 2, false), "hi");
        assert_eq!(style_wrap("hi", 2, true), "\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn bleep_bytes_tty_gated() {
        assert_eq!(bleep_bytes(3, true), "\x07\x07\x07");
        assert_eq!(bleep_bytes(3, false), "");
        assert_eq!(bleep_bytes(0, true), "");
    }

    #[test]
    fn parse_and_resolve_term_rows() {
        assert_eq!(parse_stty_size("24 80\n"), Some((24, 80)));
        assert_eq!(parse_stty_size("garbage"), None);
        assert_eq!(term_rows(Some("40 100"), None), 40); // stty wins
        assert_eq!(term_rows(None, Some("50")), 50); // env fallback
        assert_eq!(term_rows(None, None), DEFAULT_ROWS); // default
        assert_eq!(term_rows(Some("bad"), Some("x")), DEFAULT_ROWS);
    }

    #[test]
    fn status_text_pads_and_right_aligns() {
        let st = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 0, turns: 1 },
        };
        let row = status_text(&st, 40);
        assert_eq!(row.chars().count(), 40, "padded to width");
        assert!(row.starts_with(" West of House"), "location left: {row:?}");
        assert!(row.trim_end().ends_with("Moves: 1"), "right field: {row:?}");
    }

    #[test]
    fn status_text_truncates_long_location() {
        let st = StatusLine {
            location: "x".repeat(100),
            right: StatusRight::Time { hours: 9, minutes: 5 },
        };
        let row = status_text(&st, 20);
        assert_eq!(row.chars().count(), 20);
        assert!(row.contains("09:05"));
    }

    #[test]
    fn upper_row_text_and_ansi() {
        let mut u = UpperWindow::default();
        u.resize(1, 5);
        u.put(1, 1, 'H', 0);
        u.put(1, 2, 'i', 2); // bold
        let text = upper_row_text(&u, 1);
        assert_eq!(text, "Hi"); // trailing blanks trimmed
        let ansi = upper_row_ansi(&u, 1);
        assert!(
            ansi.contains("\x1b[1m") && ansi.ends_with("\x1b[0m"),
            "ansi: {ansi:?}"
        );
    }

    #[test]
    fn region_strings() {
        assert_eq!(leave_region(), "\x1b[r");
        assert!(enter_region(1, 24).starts_with("\x1b[2;24r"));
    }
}
