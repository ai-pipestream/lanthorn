//! What kind of terminal are we talking to, and how much may we assume?
//!
//! Every CLI opens by asking the same two questions of stdin and stdout, then
//! spends the rest of its life branching on the answers. Two decisions fall out
//! of them, and they are not the same decision:
//!
//! - **rich** — may we emit escape sequences (colour, cursor shape, scroll
//!   regions, OSC page background)? Today this is exactly "stdout is a TTY".
//! - **raw input** — may we take over line editing in raw mode, echoing
//!   characters ourselves? Today this is exactly "stdin is a TTY".
//!
//! They are separate because a plain-text mode (SQ-0606) turns *both* off while
//! leaving the underlying TTY facts unchanged: a screen reader wants a real
//! terminal with no escapes in it and the kernel's own cooked line editing.
//! [`HostMode::detect_with`] takes that `force_plain` decision as a parameter so
//! the flag can be added without touching this type; until then every caller
//! passes `false` and the answers are byte-for-byte what the CLIs computed
//! inline before.
//!
//! `both_tty` stays a plain TTY fact — it gates terminal-size polling and
//! `[MORE]` paging, which are questions about the device, not about how much
//! decoration the reader wants.

use std::io::{self, IsTerminal};
use std::sync::OnceLock;

/// The process-wide mode, published by [`HostMode::install`].
///
/// A global because terminal state *is* process-global: the exit paths that need
/// it most ([`crate::term::restore_and_exit`], reached from inside a read helper
/// on Ctrl-C or EOF) are several frames below anything that could have been
/// handed a copy, and the alternative is threading the mode through every input
/// signature or — as the CLIs did before — guessing at the bottom.
static MODE: OnceLock<HostMode> = OnceLock::new();

/// How much this run may assume about its terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostMode {
    stdin_tty: bool,
    stdout_tty: bool,
    plain: bool,
}

impl HostMode {
    /// Detect from the real stdin/stdout, with no plain-text override.
    pub fn detect() -> Self {
        Self::detect_with(false)
    }

    /// Detect from the real stdin/stdout. `force_plain` suppresses both escapes
    /// and raw-mode line editing regardless of what the terminal can do.
    pub fn detect_with(force_plain: bool) -> Self {
        HostMode {
            stdin_tty: io::stdin().is_terminal(),
            stdout_tty: io::stdout().is_terminal(),
            plain: force_plain,
        }
    }

    /// Construct explicitly (tests, and hosts that already know their streams).
    pub fn new(stdin_tty: bool, stdout_tty: bool, plain: bool) -> Self {
        HostMode { stdin_tty, stdout_tty, plain }
    }

    /// Publish this mode as the process-wide one, and return it unchanged.
    ///
    /// Only the first call wins, so a stray second call cannot retune the exit
    /// paths mid-run.
    pub fn install(self) -> Self {
        let _ = MODE.set(self);
        self
    }

    /// The process-wide mode. Falls back to detection when nothing was
    /// installed, so a helper called from an untouched code path still behaves.
    pub fn current() -> Self {
        *MODE.get_or_init(Self::detect)
    }

    /// stdin is a terminal (the device fact).
    pub fn stdin_tty(self) -> bool {
        self.stdin_tty
    }

    /// stdout is a terminal (the device fact).
    pub fn stdout_tty(self) -> bool {
        self.stdout_tty
    }

    /// Both streams are terminals — interactive in the full sense. Gates
    /// terminal-size polling and `[MORE]` paging, which would block a headless
    /// harness.
    pub fn both_tty(self) -> bool {
        self.stdin_tty && self.stdout_tty
    }

    /// May we emit escape sequences?
    pub fn rich(self) -> bool {
        self.stdout_tty && !self.plain
    }

    /// May we take over line editing in raw mode?
    pub fn raw_input(self) -> bool {
        self.stdin_tty && !self.plain
    }

    /// Plain-text output was requested explicitly.
    pub fn plain(self) -> bool {
        self.plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_facts_and_decorations_are_separate_questions() {
        let full = HostMode::new(true, true, false);
        assert!(full.rich() && full.raw_input() && full.both_tty());

        // Piped stdin, terminal stdout: escapes are still fine, raw editing is not.
        let piped_in = HostMode::new(false, true, false);
        assert!(piped_in.rich(), "stdout is still a terminal");
        assert!(!piped_in.raw_input(), "nothing to edit on a pipe");
        assert!(!piped_in.both_tty());

        // Piped stdout: no escapes, whatever stdin is.
        assert!(!HostMode::new(true, false, false).rich());
    }

    #[test]
    fn plain_suppresses_both_without_lying_about_the_device() {
        let plain = HostMode::new(true, true, true);
        assert!(!plain.rich(), "plain mode emits no escapes");
        assert!(!plain.raw_input(), "plain mode leaves line editing to the kernel");
        // The device facts are untouched — resize polling still knows it has a
        // terminal to ask.
        assert!(plain.stdin_tty() && plain.stdout_tty() && plain.both_tty());
    }
}
