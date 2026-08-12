//! Group binary: The card-table v6 games — Journey's FMV poker hybrid and every Scopa
//! suite (buttons, decks, painted cards, declined colours, screen extent).
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 7. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/v6_fmvpoker_hybrid.rs"]
mod v6_fmvpoker_hybrid;
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
#[path = "suites/v6_scopa_screen_extent.rs"]
mod v6_scopa_screen_extent;
