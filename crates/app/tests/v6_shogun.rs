//! Group binary: Shogun (v6) — gameplay walkthrough, the declared status columns, the title
//! header and the native archive round-trip.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 5. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/v6_shogun_declared_columns.rs"]
mod v6_shogun_declared_columns;
#[path = "suites/v6_shogun_gameplay.rs"]
mod v6_shogun_gameplay;
#[path = "suites/v6_shogun_menu_ground.rs"]
mod v6_shogun_menu_ground;
#[path = "suites/v6_shogun_native_archive.rs"]
mod v6_shogun_native_archive;
#[path = "suites/v6_shogun_prompt_style.rs"]
mod v6_shogun_prompt_style;
#[path = "suites/v6_shogun_status_alignment.rs"]
mod v6_shogun_status_alignment;
#[path = "suites/v6_shogun_title_header.rs"]
mod v6_shogun_title_header;

#[path = "suites/v6_screen_palette.rs"]
mod v6_screen_palette;
