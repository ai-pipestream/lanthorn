//! Cover-art decoding + terminal-protocol caching for the story picker.
//!
//! `load_cover` pulls a blorb's `Fspc` frontispiece image and decodes it;
//! `CoverState` holds the decoded image for the currently-selected story and
//! lazily builds (and caches) a `ratatui-image` protocol scaled to the panel's
//! cover region. Every failure resolves to `None` — the picker simply shows no
//! cover.

use std::path::{Path, PathBuf};

use ratatui::layout::{Rect, Size};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

/// Decode PNG/JPEG bytes into a `DynamicImage`. `None` on any decode failure.
pub fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    image::load_from_memory(bytes).ok()
}

/// Read `path`; if it is a blorb declaring an `Fspc` frontispiece, fetch and
/// decode that Pict. `None` when the file isn't a blorb, has no frontispiece,
/// the referenced Pict is missing, or the image doesn't decode.
pub fn load_cover(path: &Path) -> Option<image::DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return None;
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    let n = b.frontispiece()?;
    let (_ty, data) = b.resource(b"Pict", n)?;
    decode(data)
}

/// Selection-scoped cover state: the decoded image for the current story (if it
/// has a frontispiece) plus a protocol cached by `(path, cols, rows)` so it is
/// rebuilt only when the story or the cover region's size changes.
#[derive(Default)]
pub struct CoverState {
    decoded: Option<(PathBuf, image::DynamicImage)>,
    proto: Option<(PathBuf, u16, u16, Protocol)>,
}

impl CoverState {
    /// True when `path`'s cover image is already decoded (skip re-read/decode).
    pub fn has(&self, path: &Path) -> bool {
        self.decoded.as_ref().is_some_and(|(p, _)| p == path)
    }

    /// Store the decoded cover for `path` (or clear it when `img` is `None`).
    /// Invalidates any previously-built protocol.
    pub fn set(&mut self, path: &Path, img: Option<image::DynamicImage>) {
        self.decoded = img.map(|i| (path.to_path_buf(), i));
        self.proto = None;
    }

    /// Build-or-reuse a protocol for `path`'s cover, fitted (aspect-preserved)
    /// into `area`. `None` when `path` has no decoded cover or the build fails.
    pub fn protocol(&mut self, picker: &Picker, path: &Path, area: Rect) -> Option<&Protocol> {
        let img = self
            .decoded
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, i)| i)?;
        let fresh = matches!(
            &self.proto,
            Some((p, w, h, _)) if p == path && *w == area.width && *h == area.height
        );
        if !fresh {
            let built = picker
                .new_protocol(img.clone(), Size::new(area.width, area.height), Resize::Fit(None))
                .ok()?;
            self.proto = Some((path.to_path_buf(), area.width, area.height, built));
        }
        self.proto.as_ref().map(|(_, _, _, p)| p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decode_accepts_png_rejects_garbage() {
        assert!(decode(&png_bytes()).is_some());
        assert!(decode(b"not an image").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn cover_state_caches_by_path_and_builds_protocol() {
        let mut st = CoverState::default();
        let path = Path::new("game.gblorb");
        assert!(!st.has(path));

        st.set(path, decode(&png_bytes()));
        assert!(st.has(path));

        // halfblocks() needs no terminal query — deterministic in tests.
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = ratatui::layout::Rect::new(0, 0, 10, 5);
        assert!(st.protocol(&picker, path, area).is_some());

        // A different path has no cover until set.
        let other = Path::new("other.gblorb");
        assert!(!st.has(other));
        assert!(st.protocol(&picker, other, area).is_none());
    }

    #[test]
    fn cover_state_clears_on_none() {
        let mut st = CoverState::default();
        let path = Path::new("g.gblorb");
        st.set(path, decode(&png_bytes()));
        assert!(st.has(path));
        st.set(path, None); // e.g. re-selected a game with no frontispiece
        assert!(!st.has(path));
    }
}
