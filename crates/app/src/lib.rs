// Test fixtures build structs by defaulting then setting a few fields, which is
// clearer than a full struct literal here. Silence the pedantic lint in tests only.
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

pub mod anim;
pub mod archive;
pub mod aux_store;
pub mod clipboard;
pub mod export;
pub mod history;
pub mod hints;
pub mod slash;
pub mod colors;
pub mod complete;
pub mod config;
pub mod cover;
pub mod engine;
pub mod glk_backend;
pub mod glulx_session;
pub mod graphics;
pub mod inventory;
pub mod export_dot;
pub mod export_svg;
pub mod ifid;
pub mod input;
pub mod keymap;
pub mod list_scroll;
pub mod map_dump;
pub mod persist_files;
pub mod picker;
pub mod reload;
pub mod render;
pub mod roomid;
pub mod session;
pub mod state;
pub mod style;
pub mod style_mru;
pub mod styles;
pub mod symbols;
pub mod watch;
