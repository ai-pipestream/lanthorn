//! Terminal escapes, and putting the terminal back.
//!
//! The escape helpers were byte-identical in `zvm-cli/src/screen.rs` and
//! `gvm-cli/src/glk_term.rs`; they are here now, once. What is genuinely new is
//! [`TerminalGuard`]: a restore that happens even when nobody remembers to ask
//! for it.
//!
//! Restoring the terminal is the easiest thing in a CLI to get *almost* right.
//! Before this crate, `zvm-cli` restored on quit, on fault, and on Ctrl-C — but
//! not when a piped char read hit EOF, so `echo cmds | zvm-cli story.z5` in a
//! real terminal left the shell wearing the game's background and a block
//! cursor. Nor did any of them restore on a panic. Each of those is a separate
//! `if` somebody has to write on a path they were not thinking about. The guard
//! inverts that: restoring is the default, and the interesting exit paths just
//! say when.

use std::io::{IsTerminal, Write};

use crossterm::terminal;

use crate::mode::HostMode;

/// Split a 24-bit `0xRRGGBB` colour into `(r, g, b)`.
pub fn rgb24(v: u32) -> (u8, u8, u8) {
    (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
}

/// OSC 11: set the terminal's default background to `#rrggbb`.
pub fn osc_set_bg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x07")
}

/// OSC 111: reset the terminal's default background to the user's default.
pub fn osc_reset_bg() -> &'static str {
    "\x1b]111\x07"
}

/// DECSCUSR: force a steady block cursor (SQ-0281 — a box rather than the
/// terminal's default underline).
pub fn cursor_steady_block() -> &'static str {
    "\x1b[2 q"
}

/// DECSCUSR: restore the terminal's default cursor shape.
pub fn cursor_reset() -> &'static str {
    "\x1b[0 q"
}

/// The escape to emit for a page-bg transition from `prev` to `cur` (both are
/// already honor-resolved: `None` = no game bg / default). `None` return = no
/// change.
pub fn page_bg_escape(cur: Option<(u8, u8, u8)>, prev: Option<(u8, u8, u8)>) -> Option<String> {
    if cur == prev {
        return None;
    }
    Some(match cur {
        Some(rgb) => osc_set_bg(rgb),
        None => osc_reset_bg().to_string(),
    })
}

/// Leave raw mode. Harmless when raw mode was never entered, or when stdin is
/// not a terminal at all.
pub fn end_raw_mode() {
    let _ = terminal::disable_raw_mode();
}

/// The bytes that put a rich terminal back the way we found it: styling
/// cleared, page background released, default cursor shape.
///
/// The page-background reset is deliberately *not* gated on whether a game ever
/// set one. OSC 111 restores the terminal's own configured default, so sending
/// it when nothing was set is a no-op — and the alternative is asking every exit
/// path to remember whether colour was honoured, which is precisely the question
/// the Ctrl-C paths already gave up on answering.
pub fn restore_bytes() -> String {
    format!("\x1b[0m{}{}", osc_reset_bg(), cursor_reset())
}

/// Leave raw mode and put the terminal back.
///
/// `prefix` is any renderer-specific teardown to emit first — `zvm-cli` passes
/// its scroll-region reset and bottom-row park; pass `""` when there is none.
/// Silent on a non-rich terminal: there is nothing to undo on a pipe.
pub fn restore_terminal(prefix: &str) {
    end_raw_mode();
    if !HostMode::current().rich() {
        return;
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "{prefix}{}", restore_bytes());
    let _ = out.flush();
}

/// Restore the terminal, then exit — for the paths that cannot unwind to the
/// guard.
///
/// Two of them exist and both are deep inside a read helper: Ctrl-C / Ctrl-D in
/// raw mode (where the terminal delivers a keypress rather than a signal, so
/// nothing else will stop the process) and true EOF on piped stdin (where there
/// is no input left to answer the game's request with, and synthesizing one
/// spins forever — SQ-0604). Neither has a path back to `main`.
pub fn restore_and_exit(prefix: &str, code: i32) -> ! {
    restore_terminal(prefix);
    std::process::exit(code)
}

/// Restores the terminal when dropped, unless it already has been.
///
/// Hold one for the lifetime of `main`. Call [`TerminalGuard::restore`] at a
/// normal exit (passing any renderer teardown as the prefix) and the drop
/// becomes a no-op; forget to, or panic, and the drop covers it. It does *not*
/// cover [`std::process::exit`], which runs no destructors — use
/// [`restore_and_exit`] there.
///
/// Two flavours, because a guard should undo what its CLI actually did.
/// [`TerminalGuard::new`] is for hosts that dress the terminal up — page
/// background, block cursor — and must put all of it back. [`raw_only`] is for
/// `scott-cli`, which emits no escape sequences at all and whose only terminal
/// state is raw mode: sending it a cursor reset at exit would put escapes into
/// the one transcript that had none, which is exactly the property that makes it
/// worth keeping (SQ-0608).
///
/// [`raw_only`]: TerminalGuard::raw_only
pub struct TerminalGuard {
    done: bool,
    escapes: bool,
}

impl TerminalGuard {
    /// Arm a full guard: raw mode, styling, page background, cursor shape.
    ///
    /// Emits nothing on construction — installing the cursor shape and other
    /// entry-side setup stays with the CLI that wants it.
    pub fn new() -> Self {
        TerminalGuard { done: false, escapes: true }
    }

    /// Arm a guard that only leaves raw mode, emitting no escape sequences.
    pub fn raw_only() -> Self {
        TerminalGuard { done: false, escapes: false }
    }

    /// Restore now, emitting `prefix` (renderer teardown) first. Idempotent —
    /// later calls, and the eventual drop, do nothing.
    pub fn restore(&mut self, prefix: &str) {
        if self.done {
            return;
        }
        self.done = true;
        if self.escapes {
            restore_terminal(prefix);
        } else {
            end_raw_mode();
        }
    }
}

impl Default for TerminalGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore("");
    }
}

/// True when stdout is a terminal *right now*, ignoring any installed mode.
/// Only for setup code that runs before a [`HostMode`] exists.
pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_set_bg_formats_hex() {
        assert_eq!(osc_set_bg((0x12, 0x34, 0x56)), "\x1b]11;#123456\x07");
        assert_eq!(osc_set_bg((0, 0, 0)), "\x1b]11;#000000\x07");
        assert_eq!(osc_reset_bg(), "\x1b]111\x07");
    }

    #[test]
    fn page_bg_escape_emits_only_on_change() {
        assert_eq!(page_bg_escape(None, None), None);
        assert_eq!(page_bg_escape(Some((1, 2, 3)), None), Some("\x1b]11;#010203\x07".into()));
        assert_eq!(page_bg_escape(Some((1, 2, 3)), Some((1, 2, 3))), None);
        assert_eq!(
            page_bg_escape(Some((9, 9, 9)), Some((1, 2, 3))),
            Some("\x1b]11;#090909\x07".into()),
            "one colour to another still emits"
        );
        assert_eq!(page_bg_escape(None, Some((1, 2, 3))), Some("\x1b]111\x07".into()));
    }

    #[test]
    fn cursor_escapes_are_decscusr() {
        assert_eq!(cursor_steady_block(), "\x1b[2 q");
        assert_eq!(cursor_reset(), "\x1b[0 q");
    }

    #[test]
    fn rgb24_unpacks_big_endian() {
        assert_eq!(rgb24(0x123456), (0x12, 0x34, 0x56));
        assert_eq!(rgb24(0xFFFFFF), (255, 255, 255));
    }

    #[test]
    fn restore_bytes_clears_style_background_and_cursor() {
        let b = restore_bytes();
        assert!(b.starts_with("\x1b[0m"), "styling cleared first: {b:?}");
        assert!(b.contains(osc_reset_bg()), "page background released: {b:?}");
        assert!(b.ends_with(cursor_reset()), "cursor shape restored last: {b:?}");
    }

    #[test]
    fn guard_restore_is_idempotent() {
        // The observable effect is process-global (it writes to the real stdout,
        // which under `cargo test` is not a terminal, so `restore_terminal`
        // emits nothing). What is testable here is the latch itself: a second
        // restore, and the drop, must not repeat the work.
        let mut g = TerminalGuard::new();
        assert!(!g.done);
        g.restore("");
        assert!(g.done, "first restore latches");
        g.restore("");
        assert!(g.done, "second restore is a no-op");
    }

    #[test]
    fn raw_only_guard_promises_no_escapes() {
        // scott-cli's transcripts are escape-free and must stay that way; the
        // flavour is what decides, so it is worth pinning.
        assert!(!TerminalGuard::raw_only().escapes);
        assert!(TerminalGuard::new().escapes);
    }
}
