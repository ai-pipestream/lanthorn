// Output sink abstraction — ZMSD §7.
//
// `Output` is the pluggable interface for all text the Z-machine emits.
// `BufferOutput` is a test/headless sink that accumulates output into a String.
//
// `Output` requires `as_any` so callers can downcast to concrete types (e.g.,
// to read `BufferOutput::buf` in tests).

use std::any::Any;

/// Trait for Z-machine text output sinks.
pub trait Output: Any {
    fn print(&mut self, s: &str);
    /// Print `s` carrying the current Z-machine text-style bitmask
    /// (ZMSD §8.7.1: 1=reverse, 2=bold, 4=italic, 8=fixed-pitch). The default
    /// ignores the style and delegates to `print`, so existing sinks are
    /// unaffected until they override this.
    fn print_styled(&mut self, s: &str, _style: u8) {
        self.print(s);
    }
    /// Notify the sink that the Z-machine `buffer_mode` opcode changed the
    /// buffering flag. When `on` is `false` the interpreter must NOT soft-wrap
    /// output at the terminal column limit (though explicit `\n` and paging
    /// still apply). The default is a no-op so that `BufferOutput` and the
    /// `app` crate's sink compile unchanged.
    fn set_buffer_mode(&mut self, _on: bool) {}
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcast support — required to drain sink state (e.g. `CaptureSink::take_text`).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Simple accumulating sink for tests and headless use.
pub struct BufferOutput {
    pub buf: String,
}

impl Default for BufferOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferOutput {
    pub fn new() -> Self {
        BufferOutput { buf: String::new() }
    }
}

impl Output for BufferOutput {
    fn print(&mut self, s: &str) {
        self.buf.push_str(s);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod print_styled_tests {
    use super::*;

    #[test]
    fn default_print_styled_delegates_to_print() {
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print("hello");
        b.print_styled("hello", 0x02); // style ignored by default impl
        assert_eq!(a.buf, b.buf, "default print_styled must equal print");
    }
}
