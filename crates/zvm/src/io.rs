// Output sink abstraction — ZMSD §7.
//
// `Output` is the pluggable interface for all text the Z-machine emits.
// `BufferOutput` is a test/headless sink that accumulates output into a String.
//
// `Output` requires `as_any` so callers can downcast to concrete types (e.g.,
// to read `BufferOutput::buf` in tests).

use std::any::Any;

use crate::screen::ZColour;

/// Text attributes for one styled run (logical colour, pre-reverse-swap).
#[derive(Debug, Clone, Copy, Default)]
pub struct TextAttrs {
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}

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
    /// Print `s` carrying full text attributes (style bitmask + logical
    /// colour). The default delegates to `print_styled`, so sinks that do not
    /// render colour are unaffected.
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.print_styled(s, attrs.style);
    }
    /// Notify the sink that the Z-machine `buffer_mode` opcode changed the
    /// buffering flag (ZMSD §7.2.1: buffering is on at the start of a game).
    /// When `on` is `false` the interpreter must NOT word-wrap output — text
    /// breaks after the last character that fits (explicit `\n` and paging still
    /// apply).
    ///
    /// The default is a no-op, which is right only for sinks that never wrap
    /// (e.g. `BufferOutput`, which just accumulates a `String`). Any sink that
    /// lays text out in columns MUST override it — `zvm-cli`'s `StdoutOutput`
    /// stops soft-wrapping, and the `app` crate's `CaptureSink` flags the runs it
    /// captures so the transcript char-breaks them.
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

    #[test]
    fn default_print_attr_delegates_to_print_styled() {
        use crate::screen::ZColour;
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print_styled("hi", 0x02);
        b.print_attr("hi", TextAttrs { style: 0x02, fg: ZColour::Standard(3), bg: ZColour::Default });
        assert_eq!(a.buf, b.buf, "default print_attr falls back to print_styled");
    }
}
