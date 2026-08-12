//! Group binary: The diagnostic dump commands — `cell dump`, `window dump` on every engine,
//! and the death/turn bookkeeping they read.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 6. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/cell_dump_command.rs"]
mod cell_dump_command;
#[path = "suites/death_turn_tried.rs"]
mod death_turn_tried;
#[path = "suites/window_dump_bound_key.rs"]
mod window_dump_bound_key;
#[path = "suites/window_dump_engines.rs"]
mod window_dump_engines;
#[path = "suites/window_dump_last_game_frame.rs"]
mod window_dump_last_game_frame;
#[path = "suites/window_dump_non_v6.rs"]
mod window_dump_non_v6;
