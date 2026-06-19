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
    fn as_any(&self) -> &dyn Any;
}

/// Simple accumulating sink for tests and headless use.
pub struct BufferOutput {
    pub buf: String,
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
}
