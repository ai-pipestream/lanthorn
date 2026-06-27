// Terminal Glk backend for gvm-cli.
//
// Phase 3a-1 ships a temporary streaming shim: text-buffer output goes straight
// to stdout. The full screen model (status TextGrid pinned via an ANSI
// scroll-region, styles → SGR, non-TTY plain mode) lands in Task 5.

use std::any::Any;
use std::io::{self, Write};

use gvm::glk::{GlkBackend, GlkStyle};

/// A minimal terminal backend that streams text-buffer output to stdout.
pub struct TerminalBackend;

impl TerminalBackend {
    /// Construct the backend.
    pub fn new() -> Self {
        TerminalBackend
    }
}

impl GlkBackend for TerminalBackend {
    fn put_text(&mut self, _win: u32, _style: GlkStyle, s: &str) {
        print!("{s}");
        let _ = io::stdout().flush();
    }
    fn grid_put(&mut self, _win: u32, _x: u32, _y: u32, _style: GlkStyle, s: &str) {
        print!("{s}");
        let _ = io::stdout().flush();
    }
    fn flush(&mut self) {
        let _ = io::stdout().flush();
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
