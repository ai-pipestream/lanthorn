//! Group binary: The card-table v6 games — Journey's FMV poker hybrid and every Scopa
//! suite (buttons, decks, painted cards, declined colours, screen extent).
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 8. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

// Shared fixture-path resolution, declared ONCE per group binary: the suites
// below are modules of this one crate, so a `#[path]` module in each of them is
// the same file loaded several times over (clippy::duplicate_mod).
#[path = "suites/fixture_paths.rs"]
mod fixture_paths;

#[path = "suites/v6_ega_transparency.rs"]
mod v6_ega_transparency;
#[path = "suites/v6_fmvpoker_hybrid.rs"]
mod v6_fmvpoker_hybrid;
#[path = "suites/v6_halfblocks_colour_depth.rs"]
mod v6_halfblocks_colour_depth;
#[path = "suites/v6_halfblocks_upscale.rs"]
mod v6_halfblocks_upscale;
#[path = "suites/v6_scopa_button_labels.rs"]
mod v6_scopa_button_labels;
#[path = "suites/v6_scopa_declined_colours.rs"]
mod v6_scopa_declined_colours;
#[path = "suites/v6_scopa_hybrid_no_story.rs"]
mod v6_scopa_hybrid_no_story;
#[path = "suites/v6_scopa_painted_cards.rs"]
mod v6_scopa_painted_cards;
#[path = "suites/v6_scopa_picture_decks.rs"]
mod v6_scopa_picture_decks;
#[path = "suites/v6_scopa_selection_repaint.rs"]
mod v6_scopa_selection_repaint;
#[path = "suites/v6_scopa_screen_extent.rs"]
mod v6_scopa_screen_extent;
