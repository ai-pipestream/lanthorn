//! `gvm` — a zero-dependency Glulx virtual machine (Phase 2a: foundation +
//! headless runner). Structured like `zvm`: a [`memory::Memory`] over the loaded
//! image and a pluggable [`io::Output`] sink. The `Machine` (execution engine)
//! is added in later tasks.
//!
//! All opcode numbers, addressing modes, and the header/call-frame layout are
//! transcribed from the Glulx specification into `GLULX_NOTES.md`, and the code
//! is implemented against that file.

#[cfg(test)]
mod asm;
pub mod error;
pub mod exec;
pub mod header;
pub mod io;
pub mod memory;

pub use error::GError;
pub use exec::{Machine, StepResult};
pub use io::{BufferOutput, Output};
pub use memory::{Memory, WriteFault};
