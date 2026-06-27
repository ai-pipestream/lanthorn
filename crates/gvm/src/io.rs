// Output sink abstraction — mirrors `zvm::io::Output`.
//
// `Output` is the pluggable interface for all text the Glulx machine emits
// (under the Glk I/O system). `BufferOutput` is a test/headless sink that
// accumulates output into a String. `as_any`/`as_any_mut` let callers downcast
// to concrete types (e.g. to read `BufferOutput::buf` in tests).

use std::any::Any;

/// Trait for Glulx text output sinks.
pub trait Output: Any {
    /// Emit a string fragment to the sink.
    fn print(&mut self, s: &str);
    /// Immutable downcast support.
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcast support — used to drain sink state in tests.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Simple accumulating sink for tests and headless use.
pub struct BufferOutput {
    /// Everything printed so far.
    pub buf: String,
}

impl Default for BufferOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferOutput {
    /// A fresh empty buffer.
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
