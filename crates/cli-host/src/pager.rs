//! `[MORE]` paging: stopping a long turn before it scrolls off the top.
//!
//! A turn that prints more than a screenful is common — an opening, a long room
//! description, a game dumping its credits — and without a pause the beginning
//! of it is gone before the player has read it. The original interpreters
//! stopped at the bottom of each page and waited for a key, and so does
//! babelmap's TUI.
//!
//! Only `zvm-cli` did. `gvm-cli` and `scott-cli` had no paging code at all
//! (SQ-0617), which for Glulx also put `gvm-cli` behind an ordinary Glk library:
//! Gargoyle and glkterm both pause a text-buffer window this way.
//!
//! The rule is engine-neutral — count lines, stop at the bottom, wait, erase the
//! prompt, start counting again — so it lives here and each host feeds it the
//! newlines it writes. What stays with the host is *which* writes count: the
//! Z-machine's lower window, Glk's streaming buffer window, Scott's prose.
//!
//! ## When paging is wrong
//!
//! It needs both ends to be a terminal: pausing for a key that a pipe will never
//! send is a hang, which is why the headless harnesses never see it. And it is
//! off in screen-reader mode by choice — a blocking prompt that hides the rest
//! of the output behind a keypress is the shape a reader copes with worst.

use std::io::Write;

use crossterm::event::{self, Event};
use crossterm::terminal;

/// Reverse-video `[MORE]`, matching the TUI's bar and the original interpreters.
pub const MORE_PROMPT: &str = "\x1b[7m[MORE]\x1b[0m";

/// Return to column 0 and clear the line, erasing the prompt.
pub const MORE_ERASE: &str = "\r\x1b[2K";

/// Counts emitted lines and says when the page is full.
#[derive(Debug)]
pub struct Pager {
    enabled: bool,
    page_height: u16,
    lines: u16,
}

impl Pager {
    /// `page_height` is the usable rows; see [`Pager::height_for`].
    pub fn new(enabled: bool, page_height: u16) -> Self {
        Pager { enabled, page_height, lines: 0 }
    }

    /// The page height to use for a terminal `rows` tall.
    ///
    /// Two rows are held back: one for the `[MORE]` prompt itself and one so the
    /// line the player was reading does not sit flush against it. The floor of 2
    /// keeps a degenerate terminal from paging on every single line.
    pub fn height_for(rows: u16) -> u16 {
        rows.saturating_sub(2).max(2)
    }

    /// Keep up with a terminal that was resized.
    pub fn set_page_height(&mut self, page_height: u16) {
        self.page_height = page_height;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Start the count again — at an input prompt, or after the screen is
    /// cleared. The player has caught up either way, and a count that carried
    /// across would pause partway through the next turn for no reason.
    pub fn reset(&mut self) {
        self.lines = 0;
    }

    /// Record one emitted line; `true` when the page is now full.
    ///
    /// The threshold is `page_height - 1` rather than `page_height`: the pause
    /// happens *before* the line that would scroll the top away, not after.
    pub fn line(&mut self) -> bool {
        // Saturating: nothing resets the count while a game prints without
        // pausing — piped output, `--no-more`, a print loop — so it climbs
        // without bound. It used to be `+=` on a u16 and panicked with "attempt
        // to add with overflow" at exactly 65_536 newlines; Zork Zero reached
        // that in under twenty seconds (SQ-0601). Nothing reads the count past
        // the threshold, so pinning it at the ceiling changes no behaviour.
        self.lines = self.lines.saturating_add(1);
        self.enabled && self.page_height >= 2 && self.lines >= self.page_height - 1
    }

    /// Show the prompt, wait for a key, erase it, and start the count again.
    ///
    /// `interactive` is the caller's "can I actually read a key?" — false makes
    /// this a no-op rather than a hang.
    pub fn pause<W: Write>(&mut self, out: &mut W, interactive: bool) {
        self.reset();
        if !interactive {
            return;
        }
        let _ = out.write_all(MORE_PROMPT.as_bytes());
        let _ = out.flush();
        wait_for_keypress();
        let _ = out.write_all(MORE_ERASE.as_bytes());
        let _ = out.flush();
    }
}

/// Block until any key is pressed, in raw mode so it takes a single keystroke
/// rather than a line.
///
/// Resize events and mouse reports are skipped: they are not the player saying
/// "go on", and treating them as one would make a window drag dismiss the page.
pub fn wait_for_keypress() {
    let _ = terminal::enable_raw_mode();
    loop {
        match event::read() {
            Ok(Event::Key(_)) => break,
            Ok(_) => {}
            Err(_) => break, // no terminal to read from; do not spin
        }
    }
    let _ = terminal::disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that records bytes, standing in for stdout.
    fn paged(height: u16) -> Pager {
        Pager::new(true, height)
    }

    #[test]
    fn the_pause_comes_before_the_line_that_would_scroll_the_top_away() {
        // height 24 → pause on the 23rd line, leaving the prompt a row and the
        // last line of text still visible above it.
        let mut p = paged(24);
        for n in 1..23 {
            assert!(!p.line(), "line {n} of a 24-row page should not pause yet");
        }
        assert!(p.line(), "the 23rd fills the page");
    }

    #[test]
    fn resetting_is_what_stops_a_pause_landing_mid_turn() {
        let mut p = paged(4);
        assert!(!p.line());
        assert!(!p.line());
        assert!(p.line());
        // An input prompt means the player has caught up.
        p.reset();
        assert!(!p.line(), "the count starts again");
    }

    #[test]
    fn disabled_never_pauses_however_much_is_printed() {
        let mut p = Pager::new(false, 24);
        for _ in 0..1000 {
            assert!(!p.line());
        }
        assert!(!p.is_enabled());
    }

    #[test]
    fn a_degenerate_page_height_does_not_pause_on_every_line() {
        for height in [0, 1] {
            let mut p = paged(height);
            for _ in 0..50 {
                assert!(!p.line(), "height {height} must not page");
            }
        }
    }

    #[test]
    fn the_line_counter_saturates_instead_of_overflowing() {
        // SQ-0601: this was `+=` on a u16 and panicked at exactly 65_536
        // newlines — Zork Zero got there in under twenty seconds.
        let mut p = Pager::new(false, 24);
        for _ in 0..10 {
            p.line();
        }
        p.lines = u16::MAX - 2;
        for _ in 0..10 {
            p.line();
        }
        assert_eq!(p.lines, u16::MAX, "pinned at the ceiling, never wrapped");
    }

    #[test]
    fn height_for_holds_back_two_rows_with_a_floor() {
        assert_eq!(Pager::height_for(24), 22);
        assert_eq!(Pager::height_for(10), 8);
        assert_eq!(Pager::height_for(3), 2, "floored");
        assert_eq!(Pager::height_for(0), 2, "and never underflows");
    }

    #[test]
    fn pausing_without_a_terminal_is_a_no_op_not_a_hang() {
        // The headless harnesses run with a pipe on both ends; a prompt there
        // would wait for a key that is never coming.
        let mut p = paged(4);
        let mut out: Vec<u8> = Vec::new();
        p.line();
        p.pause(&mut out, false);
        assert!(out.is_empty(), "nothing written when there is nobody to ask");
    }

    #[test]
    fn the_prompt_is_reverse_video_and_erases_itself() {
        assert!(MORE_PROMPT.contains("[MORE]"));
        assert!(MORE_PROMPT.starts_with("\x1b[7m") && MORE_PROMPT.ends_with("\x1b[0m"));
        assert_eq!(MORE_ERASE, "\r\x1b[2K", "column 0, clear line");
    }
}
