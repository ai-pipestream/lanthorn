//! Where a CLI pins the game's fixed rows, and why that decides whether the
//! player gets scrollback (SQ-0909).
//!
//! Both `zvm-cli` and `gvm-cli` keep some rows fixed while the story scrolls under
//! them — a v3 status bar, a Glk grid window, BeyondZork's compass, an InvisiClues
//! menu. Both do it with DECSTBM, and both had their own copy of these three lines
//! until this module.
//!
//! # The measurement that matters
//!
//! A terminal pushes a line into history when it scrolls off the **top of the
//! screen**, and it judges that by the scroll region's TOP MARGIN. Measured against
//! Ghostty's core in this module's tests, feeding 30 lines to a 10-row screen:
//!
//! | region | rows reaching history |
//! |---|---|
//! | none | 21 |
//! | rows 2..10 — pinned at the top | **0** |
//! | rows 1..9 — pinned at the bottom | **22** |
//!
//! So it is not pinning that costs the history, it is pinning **at the top**: with a
//! top margin of 2, the line leaving row 2 has not left the screen, and the terminal
//! drops it. Move the same rows to the bottom and the region starts at row 1 again,
//! so everything scrolling past is archived exactly as it would be with no region —
//! the terminal's own history, with its own wheel, selection and search.
//!
//! **Nothing is buffered on our side either way.** The scrollback a player wants is
//! the one their terminal already keeps; the only question is whether we prevent it.

use std::fmt;

/// Where the game's pinned rows sit on the physical terminal.
///
/// [`Top`](Pin::Top) is the default because it is where Infocom put the status line
/// and where players expect it. [`Bottom`](Pin::Bottom) trades that placement for
/// scrollback — see the module header for the measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pin {
    /// Pinned rows at the top, story below. No scrollback.
    #[default]
    Top,
    /// Pinned rows at the bottom, story above and scrolling into history.
    Bottom,
}

impl Pin {
    /// Parse the `--pin` value, or `None` when it is neither.
    pub fn parse(value: &str) -> Option<Pin> {
        match value.trim().to_ascii_lowercase().as_str() {
            "top" => Some(Pin::Top),
            "bottom" => Some(Pin::Bottom),
            _ => None,
        }
    }

    /// The other one.
    pub fn flipped(self) -> Pin {
        match self {
            Pin::Top => Pin::Bottom,
            Pin::Bottom => Pin::Top,
        }
    }

    /// What to tell the player when it changes.
    pub fn note(self) -> &'static str {
        match self {
            Pin::Top => "[pinned at the top — no terminal scrollback while a window is pinned there]",
            Pin::Bottom => "[pinned at the bottom — the story scrolls into the terminal's own history]",
        }
    }
}

impl fmt::Display for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Pin::Top => "top",
            Pin::Bottom => "bottom",
        })
    }
}

/// The host command that moves the pin at runtime.
pub const PIN_COMMAND: &str = "/pin";

/// Read a `/pin` line: `Some(Some(p))` to move there, `Some(None)` for a bad
/// argument, `None` when this is not the command at all.
///
/// A bare `/pin` swaps, because the reason to move it changes within one session —
/// top while you play, bottom when you want to read back over what just happened —
/// and swapping is what a player reaches for.
pub fn pin_request(line: &str, current: Pin) -> Option<Option<Pin>> {
    let t = line.trim();
    let rest = if t.eq_ignore_ascii_case(PIN_COMMAND) {
        ""
    } else {
        let (head, tail) = t.split_at_checked(PIN_COMMAND.len())?;
        if !head.eq_ignore_ascii_case(PIN_COMMAND) || !tail.starts_with(' ') {
            return None;
        }
        tail
    };
    Some(match rest.trim() {
        "" => Some(current.flipped()),
        other => Pin::parse(other),
    })
}

/// Confine the screen so `rows` of chrome stay put, wherever [`Pin`] puts them.
///
/// `\x1b7`/`\x1b8` (DECSC/DECRC) wrap the region change because DECSTBM homes the
/// cursor as a side effect, which would disconnect input from wherever the game left
/// its prompt.
pub fn enter_region(rows: u16, term_rows: u16, pin: Pin) -> String {
    match pin {
        Pin::Top => format!("\x1b7\x1b[{};{}r\x1b8", rows + 1, term_rows),
        // From row 1 — that is the whole point; see the module header.
        Pin::Bottom => format!("\x1b7\x1b[1;{}r\x1b8", term_rows.saturating_sub(rows).max(1)),
    }
}

/// Reset the scroll region to the full screen. A no-op when none is set, which is
/// why every teardown can send it unconditionally.
pub fn leave_region() -> String {
    "\x1b[r".to_string()
}

/// The first physical row the pinned rows occupy, 1-based.
pub fn pinned_origin(pin: Pin, rows: u16, term_rows: u16) -> u16 {
    match pin {
        Pin::Top => 1,
        Pin::Bottom => term_rows.saturating_sub(rows) + 1,
    }
}

/// Un-pin at exit and leave the cursor BELOW everything the game drew, on a line of
/// its own.
///
/// Both placements need the same treatment and for the same reason: the bottom row
/// is occupied — by the last line of story under a top pin, by the chrome itself
/// under a bottom pin — so parking there hands the shell a row it would overwrite.
/// Scrolling once past it costs one line, and the display goes into history with the
/// rest of the session.
///
/// **Every** way out has to use this, including the ones that never reach `main`:
/// Ctrl-C and Ctrl-D in raw mode are keypresses rather than signals, so nothing else
/// will stop the process, and they used to reset the region without moving the
/// cursor — which left the shell prompt sitting in the middle of the story text
/// (SQ-0913).
pub fn leave_and_park(term_rows: u16) -> String {
    format!("{}\x1b[{term_rows};1H\r\n", leave_region())
}

/// [`leave_and_park`] for a caller that does not track the terminal height.
///
/// `gvm-cli` re-polls the size per input rather than keeping it, and its exit paths
/// have none in scope. Falls back to a plain reset plus a newline when the size is
/// unknown — a pipe, where there is no cursor to park anyway.
pub fn leave_and_park_now() -> String {
    match crossterm::terminal::size() {
        Ok((_, rows)) if rows > 0 => leave_and_park(rows),
        _ => format!("{}\r\n", leave_region()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qwertty_term_vt::stream::{Stream, TerminalHandler};
    use qwertty_term_vt::terminal::{Options, Terminal};

    const COLS: u16 = 40;
    const ROWS: u16 = 10;

    /// Feed `bytes` to a fresh terminal and report how many rows reached history.
    ///
    /// The witness is `qwertty-term-vt`, Ghostty's core ported to Rust — the same
    /// second opinion `app` uses for image placement (SQ-0764). Checking our own
    /// renderer against our own decoder would only prove it agrees with itself.
    fn scrollback_after(bytes: &str) -> usize {
        let t = Terminal::new(Options {
            cols: COLS,
            rows: ROWS,
            max_scrollback: 10_000,
            ..Default::default()
        });
        let mut stream = Stream::new(TerminalHandler::new(t));
        stream.feed(bytes.as_bytes());
        stream.handler.terminal.snapshot_window(0).scrollback_len
    }

    /// Enough lines to push the screen well past its height.
    fn narrative(n: usize) -> String {
        (0..n).map(|i| format!("line {i}\r\n")).collect()
    }

    /// Unpinned, the terminal keeps the history itself.
    #[test]
    fn a_full_screen_scroll_fills_the_terminals_own_history() {
        let got = scrollback_after(&narrative(usize::from(ROWS) * 3));
        assert!(got >= usize::from(ROWS), "expected rows in history, got {got}");
    }

    /// **Pinned at the TOP, the same lines are thrown away.** This is the cost, and
    /// the reason `--pin bottom` exists at all.
    #[test]
    fn pinning_at_the_top_throws_the_scrolled_lines_away() {
        let s = format!("{}{}", enter_region(1, ROWS, Pin::Top), narrative(usize::from(ROWS) * 3));
        assert_eq!(
            scrollback_after(&s),
            0,
            "lines scrolled out of a top-margin-2 region should reach no history at all",
        );
    }

    /// **Pinned at the BOTTOM, they are kept** — same number of pinned rows, same
    /// narrative, and the region now starts at row 1.
    #[test]
    fn pinning_at_the_bottom_keeps_them() {
        let s = format!("{}{}", enter_region(1, ROWS, Pin::Bottom), narrative(usize::from(ROWS) * 3));
        assert!(
            scrollback_after(&s) >= usize::from(ROWS),
            "a top margin of 1 archives what scrolls past it",
        );
    }

    /// Dropping the region lets history resume, which is what makes `/pin` usable
    /// mid-session rather than a launch-time decision.
    #[test]
    fn dropping_the_region_lets_history_resume() {
        let mut s = format!("{}{}", enter_region(1, ROWS, Pin::Top), narrative(usize::from(ROWS) * 2));
        s.push_str(&leave_region());
        s.push_str(&narrative(usize::from(ROWS) * 2));
        assert!(scrollback_after(&s) >= usize::from(ROWS), "history resumes once unpinned");
    }

    #[test]
    fn the_pinned_rows_land_where_the_placement_says() {
        assert_eq!(pinned_origin(Pin::Top, 6, 24), 1);
        assert_eq!(pinned_origin(Pin::Bottom, 6, 24), 19, "six rows occupy 19..=24");
        assert!(enter_region(6, 24, Pin::Top).contains("\x1b[7;24r"));
        assert!(enter_region(6, 24, Pin::Bottom).contains("\x1b[1;18r"));
    }

    #[test]
    fn the_pin_command_parses() {
        assert_eq!(pin_request("/pin", Pin::Top), Some(Some(Pin::Bottom)), "bare swaps");
        assert_eq!(pin_request("/PIN bottom", Pin::Top), Some(Some(Pin::Bottom)));
        assert_eq!(pin_request("  /pin top  ", Pin::Bottom), Some(Some(Pin::Top)));
        assert_eq!(pin_request("/pin sideways", Pin::Top), Some(None), "bad argument");
        assert_eq!(pin_request("pin", Pin::Top), None, "not the command");
        assert_eq!(pin_request("/pinch the guard", Pin::Top), None, "not a prefix match");
    }

    /// The exit teardown un-pins AND moves, which is the half that was missing.
    #[test]
    fn the_exit_teardown_unpins_and_parks_below() {
        let out = leave_and_park(24);
        assert!(out.starts_with("\x1b[r"), "region dropped first: {out:?}");
        assert!(out.contains("\x1b[24;1H"), "cursor to the last row: {out:?}");
        assert!(out.ends_with("\r\n"), "and past it, so the shell gets a clean line: {out:?}");
    }
}
