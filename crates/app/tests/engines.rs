//! Group binary: Engine and medium coverage — Glulx, Scott Adams, the headless driver,
//! interpreter profiles, disk images and the release pins each medium carries.
//!
//! Each member below used to be its own test binary. The suites now live in
//! `tests/suites/`, which cargo does not auto-build, and are pulled in here as
//! modules — one link instead of 16. `cargo nextest run <old_file_name>` still
//! selects a single suite, because the module path carries the old filename.

#![allow(dead_code, unused_imports)]

#[path = "suites/adf_disk_image.rs"]
mod adf_disk_image;
#[path = "suites/glulx_banner_rooms.rs"]
mod glulx_banner_rooms;
#[path = "suites/glulx_boot_room_id.rs"]
mod glulx_boot_room_id;
#[path = "suites/glulx_game_colours.rs"]
mod glulx_game_colours;
#[path = "suites/glulx_ingame_save_host_restore.rs"]
mod glulx_ingame_save_host_restore;
#[path = "suites/glulx_maze_identity.rs"]
mod glulx_maze_identity;
#[path = "suites/glulx_pending_io_host_restore.rs"]
mod glulx_pending_io_host_restore;
#[path = "suites/glulx_resume_location.rs"]
mod glulx_resume_location;
#[path = "suites/glulx_room_detection.rs"]
mod glulx_room_detection;
#[path = "suites/headless.rs"]
mod headless;
#[path = "suites/interpreter_profile.rs"]
mod interpreter_profile;
#[path = "suites/picture_override.rs"]
mod picture_override;
#[path = "suites/real_media_releases.rs"]
mod real_media_releases;
#[path = "suites/restart_reboots_in_place.rs"]
mod restart_reboots_in_place;
#[path = "suites/scott_mapper.rs"]
mod scott_mapper;
#[path = "suites/shogun_dict_words.rs"]
mod shogun_dict_words;
#[path = "suites/wizard_sniffer.rs"]
mod wizard_sniffer;
