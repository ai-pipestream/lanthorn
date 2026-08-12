//! Group binary: The v6 render path itself, game-agnostic — frame borders at every medium,
//! the game-colour regression matrix, prose freezing and raster text loss.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 4. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/v6_frame_border_medium.rs"]
mod v6_frame_border_medium;
#[path = "suites/v6_game_colour_regression.rs"]
mod v6_game_colour_regression;
#[path = "suites/v6_prose_freeze.rs"]
mod v6_prose_freeze;
#[path = "suites/v6_raster_text_loss.rs"]
mod v6_raster_text_loss;
