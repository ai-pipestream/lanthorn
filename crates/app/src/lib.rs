// Test fixtures build structs by defaulting then setting a few fields, which is
// clearer than a full struct literal here. Silence the pedantic lint in tests only.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

/// The ONE lock the integration suites take around `zvm::screen::set_palette`.
///
/// The palette is process-global. Every suite that boots a story on a machine profile
/// sets it, and every suite that asserts a resolved colour depends on it, so they must
/// exclude one another — and for a long time each suite declared its **own**
/// `static PALETTE`, twenty-three of them. Within a suite that reads as the rule its
/// doc claims ("no two cases here may boot at once"); across suites it excludes
/// nothing, because `crates/app/tests/suites/` are MODULES pulled into ~14 group
/// binaries and every suite in a group shares one process.
///
/// Invisible to the local gate and fatal on CI, which is the whole point of putting it
/// here: `cargo nextest run` gives every test its own PROCESS, so no suite can observe
/// another's palette and twenty-three locks are indistinguishable from one. `cargo
/// test`, which CI runs, gives a binary's tests one process and many threads. MEASURED
/// on main: `arthurs_notices_are_the_machines_white_on_the_machines_dark_grey` read a
/// page whose r channel was 90 — `#5A5A5A`, §8.3.1's standard grey — where the Amiga's
/// is 68, because another suite in `v6_render` held the standard palette at the moment
/// it looked (SQ-0904).
///
/// A suite takes it with `static PALETTE: &Mutex<()> = &app::V6_PALETTE_LOCK;` so its
/// own call sites are unchanged. It lives in the library rather than in a test module
/// because one static per PROCESS is what correctness needs, and every group binary
/// links this crate.
pub static V6_PALETTE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub mod anim;
pub mod archive;
pub mod assets;
pub mod cell_dump;
pub mod aux_store;
pub mod browser;
pub mod clipboard;
pub mod export;
pub mod history;
pub mod hints;
pub mod hint_download;
pub mod slash;
pub mod colors;
pub mod complete;
pub mod config;
pub mod config_template;
pub mod cover;
pub mod debug_panel;
/// Which files are volumes of one multi-disk release (SQ-0844).
///
/// **Re-exported, not declared** (SQ-0874): the rule moved to `cli-host` the day
/// `zvm-cli` needed it, because a CLI cannot depend on `app` and a second copy of
/// "which files form a release" is how two front-ends end up disagreeing about
/// what is on a shelf. One implementation, and every `app::disk_set::…` spelling
/// still reaches it.
pub use cli_host::disk_set;
pub mod engine;
pub mod garglk_ini;
pub mod glk_backend;
pub mod glulx_debug;
pub mod glulx_roomlock;
pub mod glulx_session;
pub mod graphics;
pub mod inline_image;
pub mod inventory;
pub mod export_dot;
pub mod export_svg;
pub mod fetch_worker;
pub mod cover_gallery;
pub mod ifdb;
pub mod ifdb_search;
pub mod ifdb_search_modal;
pub mod ifiction;
pub mod ifid;
pub mod input;
pub mod interpreter;
pub mod keymap;
pub mod launch_options;
pub mod native_sound;
pub mod layout;
pub mod list_scroll;
pub mod map_dump;
pub mod notify;
pub mod pager;
pub mod pane_drag;
pub mod pcset_store;
pub mod period;
pub mod pixel_mouse;
pub mod persist_files;
pub mod picker;
pub mod query_sweep;
pub mod reload;
pub mod render;
pub mod roomid;
pub mod scott_debug;
pub mod scott_session;
pub mod session;
pub mod state;
pub mod stderr_redirect;
pub mod storage;
pub mod style;
pub mod story_info;
pub mod styles;
pub mod symbols;
pub mod term_colors;
pub mod text_field;
pub mod textwidth;
pub mod theme;
pub mod tidy;
pub mod trace;
pub mod vfs_store;
pub mod watch;
