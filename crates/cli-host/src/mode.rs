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
//! They are separate because plain-text mode turns *both* off while leaving the
//! underlying TTY facts unchanged: a screen reader wants a real terminal with no
//! escapes in it and the kernel's own cooked line editing (so the reader can
//! hook the terminal's echo to announce typed characters, and the user keeps
//! their familiar line editing).
//!
//! `both_tty` stays a plain TTY fact — it gates terminal-size polling, which is
//! a question about the device, not about how much decoration the reader wants.

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
    ///
    /// On Windows this is also where escape output is made to work at all.
    /// These hosts write their escapes with raw `print!`, and the console only
    /// interprets those once `ENABLE_VIRTUAL_TERMINAL_PROCESSING` is set on the
    /// *output* handle. crossterm 0.29 sets it lazily, but only on its own
    /// Command-API paths (`queue!`/`execute!` → `Command::is_ansi_code_supported`
    /// → `ansi_support::supports_ansi`, src/command.rs); `enable_raw_mode`
    /// touches only the *input* handle (src/terminal/sys/windows.rs). Nothing
    /// these CLIs call would ever trigger it, so run the one-shot enable here —
    /// the single point every CLI passes before its first escape.
    pub fn install(self) -> Self {
        #[cfg(windows)]
        if self.rich() {
            let _ = crossterm::ansi_support::supports_ansi();
        }
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

/// The flags that ask for plain-text output.
///
/// `--screen-reader` is first, and is the canonical spelling: it says what the
/// mode is *for*, where `--plain` describes only the mechanism. `--plain`
/// remains as an alias for anyone who wants escape-free output without being a
/// screen-reader user — piping to a log, say — which is a real use even though
/// it is not the one that motivated the mode.
pub const PLAIN_FLAGS: [&str; 2] = ["--screen-reader", "--plain"];

/// Was plain-text output asked for — by flag, or by `TERM=dumb`?
pub fn plain_requested(argv: &[String]) -> bool {
    plain_from(argv, std::env::var("TERM").ok().as_deref())
}

/// The decision behind [`plain_requested`], with the environment passed in.
///
/// `TERM=dumb` counts because it is the terminal saying it has no capabilities —
/// emitting cursor addressing at it would be nonsense whatever the user asked
/// for. `NO_COLOR` deliberately does **not** count here; see [`no_color`].
/// Stops at `--`, agreeing with `args::scan`: past it, `--plain` is a story
/// path.
pub fn plain_from(argv: &[String], term: Option<&str>) -> bool {
    crate::flags::before_double_dash(argv).any(|a| PLAIN_FLAGS.contains(&a.as_str()))
        || term == Some("dumb")
}

/// Is `NO_COLOR` set to a non-empty value?
///
/// This suppresses **colour**, not layout. The convention (no-color.org) is
/// specifically that software "prevents the addition of ANSI color" — it says
/// nothing about cursor addressing, and someone who sets `NO_COLOR` has not
/// asked to lose their pinned status line. So it maps onto the CLIs' existing
/// `--no-game-colours`, not onto plain mode. `TERM=dumb` is the variable that
/// means the whole vocabulary is unavailable.
pub fn no_color() -> bool {
    no_color_from(std::env::var("NO_COLOR").ok().as_deref())
}

/// The decision behind [`no_color`], with the environment passed in.
pub fn no_color_from(no_color: Option<&str>) -> bool {
    no_color.is_some_and(|v| !v.is_empty())
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

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_comes_from_either_flag_or_a_dumb_terminal() {
        assert!(plain_from(&argv(&["prog", "--plain"]), Some("xterm-256color")));
        assert!(plain_from(&argv(&["prog", "--screen-reader"]), Some("xterm-256color")));
        assert!(plain_from(&argv(&["prog", "story.z5"]), Some("dumb")));
        assert!(!plain_from(&argv(&["prog", "story.z5"]), Some("xterm-256color")));
        assert!(!plain_from(&argv(&["prog", "story.z5"]), None));
        // Not a prefix match — a story file named like the flag is not the flag.
        assert!(!plain_from(&argv(&["prog", "--plaintext"]), None));
    }

    /// SQ-0636. Past `--` the spelling is a story path, matching `args::scan`.
    #[test]
    fn double_dash_ends_the_plain_pre_scan() {
        assert!(!plain_from(&argv(&["prog", "--", "--plain"]), None));
        assert!(!plain_from(&argv(&["prog", "--", "--screen-reader"]), None));
        assert!(plain_from(&argv(&["prog", "--plain", "--", "x"]), None), "before -- still counts");
        // TERM=dumb is the terminal's own statement, not an argument — `--`
        // cannot un-say it.
        assert!(plain_from(&argv(&["prog", "--", "x"]), Some("dumb")));
    }

    #[test]
    fn no_color_is_presence_and_non_empty_not_truthiness() {
        // no-color.org: set and non-empty, "regardless of its value".
        assert!(no_color_from(Some("1")));
        assert!(no_color_from(Some("0")), "even \"0\" means set");
        assert!(no_color_from(Some("anything")));
        assert!(!no_color_from(Some("")), "empty means not set");
        assert!(!no_color_from(None));
    }

    #[test]
    fn no_color_does_not_imply_plain() {
        // NO_COLOR suppresses colour, not cursor addressing: someone who sets it
        // has not asked to lose their pinned status line. Only TERM=dumb says
        // the terminal has no vocabulary at all.
        assert!(!plain_from(&argv(&["prog"]), Some("xterm-256color")));
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
