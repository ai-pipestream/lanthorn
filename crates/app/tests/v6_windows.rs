//! Group binary: v6 window management — the secondary prose window, window-zero splits,
//! the band placement lifecycle, shared-screen erases, kitty graphics, and the echo of
//! a key the game reads through a window of its own.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 7. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/v6_band_placement_lifecycle.rs"]
mod v6_band_placement_lifecycle;
#[path = "suites/v6_kitty_graphics.rs"]
mod v6_kitty_graphics;
#[path = "suites/v6_secondary_prose_window.rs"]
mod v6_secondary_prose_window;
#[path = "suites/v6_shared_screen_erase.rs"]
mod v6_shared_screen_erase;
#[path = "suites/v6_sunburst_char_echo.rs"]
mod v6_sunburst_char_echo;
#[path = "suites/v6_split_tiles_window_zero.rs"]
mod v6_split_tiles_window_zero;
#[path = "suites/v6_sunburst_window_model.rs"]
mod v6_sunburst_window_model;
#[path = "suites/v6_win0_backdrop.rs"]
mod v6_win0_backdrop;

#[path = "suites/v6_mysterious01_boot.rs"]
mod v6_mysterious01_boot;
