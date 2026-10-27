//! The facts core code reads about the terminal's image backend, as a plain
//! value (SQ-1804).
//!
//! Almost every reader of `AppState`'s picker wants exactly three things: are
//! game images on at all, how many device pixels is one terminal cell, and
//! which protocol will paint them. None of that needs `ratatui_image`'s
//! `Picker`, which is the object that builds and encodes terminal graphics
//! protocols. A non-terminal embedding host (a GUI building for a target where
//! terminal graphics make no sense) builds this library without
//! `ratatui-image`, so core code must be able to ask these questions without
//! naming it. This module therefore imports nothing from `ratatui_image`; the
//! one conversion from a `Picker` lives beside the picker code
//! (`render::graphics`), and `AppState::set_game_picker` /
//! `AppState::set_game_imaging` are the only writers, so the picker and the
//! facts cannot drift.

/// Which terminal graphics protocol paints the game's images. Mirrors
/// `ratatui_image::picker::ProtocolType` variant for variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    Halfblocks,
    Sixel,
    Kitty,
    Iterm2,
}

/// What the image backend tells core code: the terminal cell size in device
/// pixels and the protocol in use. Its presence (`Option<GameImaging>`) is
/// "game images are enabled".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameImaging {
    /// One terminal cell, `(width, height)` in device pixels.
    pub cell_px: (u16, u16),
    pub protocol: ImageProtocol,
}
