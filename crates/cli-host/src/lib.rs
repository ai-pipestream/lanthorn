//! Host plumbing shared by `zvm-cli`, `gvm-cli`, and `scott-cli`.
//!
//! The three CLIs drive three unrelated VMs and each owns its own renderer —
//! Z-machine windows, Glk windows, and Scott's prose are genuinely different
//! display models, and nothing useful is shared between them. But *around* the
//! renderer every one of them does the same four things: decide whether the
//! terminal can take escapes, read input (and stop cleanly at EOF), put the
//! terminal back on the way out, and answer `--help` / `--version`. That is what
//! lives here.
//!
//! The motivating evidence is drift, not tidiness. Five escape helpers were
//! byte-identical in `zvm-cli` and `gvm-cli`; more tellingly, the stdin-EOF bug —
//! treating a 0-byte read as a blank line and looping forever on it — was fixed
//! in `zvm-cli` long ago and still shipped in `gvm-cli` months later (SQ-0604),
//! and `zvm-cli`'s *char* path still exited without restoring the terminal. One
//! defect, three chances to get it wrong. Now there is one implementation.
//!
//! Deliberately NOT here: anything that draws. See the module docs for the
//! boundary each piece sits on.

pub mod args;
pub mod flags;
pub mod input;
pub mod line;
pub mod mode;
pub mod pager;
pub mod score;
pub mod storage;
pub mod term;

pub use args::{Matches, Opt, scan};
pub use flags::{EXIT_USAGE, handled_common_flags, looks_like_flag, usage_error};
pub use line::LineHold;
pub use input::{read_byte_or_eof, read_byte_stdin, read_line_or_eof, read_line_stdin};
pub use mode::{HostMode, PLAIN_FLAGS, no_color, plain_requested};
pub use pager::{Pager, wait_for_keypress};
pub use score::{ScoreWatch, score_in_status};
pub use storage::{game_dir, resolve_save_input, story_key};
pub use term::{
    TerminalGuard, cursor_reset, cursor_steady_block, end_raw_mode, osc_reset_bg, osc_set_bg,
    page_bg_escape, restore_and_exit, restore_terminal, rgb24,
};
