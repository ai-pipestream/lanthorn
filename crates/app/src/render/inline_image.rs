//! Blits inline-image bands (one terminal-row strip per call) via ratatui-image,
//! mirroring `render/graphics.rs`. Each band row renders the corresponding
//! horizontal strip of the fitted image, so partial-scroll degrades cleanly.
//!
//! The built per-row protocol is cached, keyed by
//! `(Arc::as_ptr(&band.image.pixels) as usize, band.cols, band.rows, band.row)`,
//! so a stable band (unchanged image/geometry/row) reuses the resized strip
//! across frames instead of rebuilding it every time.

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::render::transcript::ImageBand;
use crate::state::AppState;

/// If `wr` is an inline-image band row, blit it and return true (caller does
/// `continue`). A band row is consumed (no text drawn over it) even when no
/// game picker is present — matches the prior duplicated behavior at both call
/// sites (transcript draw loop and non-primary buffer windows).
pub(crate) fn try_blit_band_row(
    state: &AppState,
    wr: &super::transcript::WrappedRow,
    area_x: u16,
    area_width: u16,
    row_y: u16,
    buf: &mut Buffer,
) -> bool {
    if let Some(band) = &wr.band {
        if let Some(picker) = state.game_picker.as_ref() {
            blit_band(&state.inline_image_render, picker, band, area_x, area_width, row_y, state.colors.theme.get("inline_image").style, buf);
        }
        return true;
    }
    false
}

/// Compute the clamped 1-row `dest` for an image band within a body area and
/// blit its strip. Shared by the transcript draw loop (Task 8) and non-primary
/// buffer windows (Task 9): both offset by `band.x_off`, clamp the width to the
/// drawable body, and render the strip via `InlineImageRender::render_row`, so a
/// game-supplied band can never exceed the area.
pub(crate) fn blit_band(
    render: &std::cell::RefCell<InlineImageRender>,
    picker: &Picker,
    band: &ImageBand,
    area_x: u16,
    area_width: u16,
    row_y: u16,
    letterbox: Style,
    buf: &mut Buffer,
) {
    let dest = Rect::new(
        area_x + band.x_off.min(area_width),
        row_y,
        band.cols.min(area_width.saturating_sub(band.x_off)),
        1,
    );
    render.borrow_mut().render_row(picker, band, dest, letterbox, buf);
}

/// Cache key for one band row's built protocol: the image's pixel-buffer
/// identity plus the band geometry that determines the resized strip.
type BandCacheKey = (usize, u16, u16, u16);

#[derive(Default)]
pub struct InlineImageRender {
    /// Value pins the source `Arc` alongside the built protocol: holding the
    /// `Arc` keeps its pixel-buffer address reserved while cached, so the
    /// pointer-based key can never collide with a later image that reuses a
    /// freed address (the ABA bug). Same shared allocation the live image holds.
    cache: std::collections::HashMap<BandCacheKey, (std::sync::Arc<image::RgbaImage>, Protocol)>,
}

impl std::fmt::Debug for InlineImageRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineImageRender").field("cached", &self.cache.len()).finish()
    }
}

impl InlineImageRender {
    /// Blit the strip for `band.row` (of `band.rows`) into the 1-row `dest`.
    pub(crate) fn render_row(&mut self, picker: &Picker, band: &ImageBand, dest: Rect, letterbox: Style, buf: &mut Buffer) {
        if dest.width == 0 || dest.height == 0 {
            return;
        }
        // Letterbox the destination first (padding when the image is narrower).
        for y in dest.top()..dest.bottom() {
            for x in dest.left()..dest.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(letterbox);
                }
            }
        }
        let key: BandCacheKey = (std::sync::Arc::as_ptr(&band.image.pixels) as usize, band.cols, band.rows, band.row);
        if let std::collections::hash_map::Entry::Vacant(e) = self.cache.entry(key) {
            // Fit the whole image to the band's cell box in pixels, then crop
            // the strip for this row. Cell pixel size comes from the picker font.
            let fs = picker.font_size();
            let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
            let box_w = band.cols as u32 * fw;
            let box_h = band.rows as u32 * fh;
            if box_w == 0 || box_h == 0 {
                return;
            }
            let full = image::DynamicImage::ImageRgba8((*band.image.pixels).clone())
                .resize_exact(box_w, box_h, image::imageops::FilterType::Triangle);
            let strip_y = band.row as u32 * fh;
            if strip_y >= box_h {
                return;
            }
            let strip_h = fh.min(box_h - strip_y);
            let strip = full.crop_imm(0, strip_y, box_w, strip_h);
            if let Ok(proto) = picker.new_protocol(strip, Size::new(band.cols, 1), Resize::Fit(None)) {
                e.insert((band.image.pixels.clone(), proto));
            }
        }
        if let Some((_, proto)) = self.cache.get(&key) {
            Image::new(proto).render(dest, buf);
        }
    }

    /// Drop cache entries for bands no longer live, keyed by source Arc-ptr
    /// (`live` holds the currently-visible bands' pointers). Bounds growth and,
    /// with the pinned Arc in the value, releases addresses only once truly gone.
    pub fn retain_live(&mut self, live: &std::collections::HashSet<usize>) {
        self.cache.retain(|key, _| live.contains(&key.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui_image::picker::Picker;

    #[test]
    fn renders_band_row_without_panic() {
        let mut px = image::RgbaImage::new(16, 16);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 0, 0, 255]);
        }
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None ,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        // No panic == pass; the halfblock protocol writes into (0,0)..(2,1).
    }

    #[test]
    fn render_row_caches_built_protocol() {
        let px = image::RgbaImage::new(16, 16);
        let img = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None ,
        };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        assert_eq!(r.cache.len(), 0);
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        assert_eq!(r.cache.len(), 1);
        // A second render of the same band/row reuses the cached protocol
        // rather than inserting a new entry.
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        assert_eq!(r.cache.len(), 1);
        // A different row of the same band gets its own cache entry.
        let band_row1 = crate::render::transcript::ImageBand { row: 1, ..band };
        r.render_row(&picker, &band_row1, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        assert_eq!(r.cache.len(), 2);
    }

    fn band_for(pixels: std::sync::Arc<image::RgbaImage>) -> crate::render::transcript::ImageBand {
        let img = crate::inline_image::InlineImage {
            pixels,
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None ,
        };
        crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 }
    }

    #[test]
    fn cache_pins_source_arc_blocking_aba() {
        // Building a protocol pins the source Arc in the cache value, so the
        // image's pixel-buffer address cannot be freed and reused while cached.
        // A NEW image therefore always gets a distinct pointer key — the stale
        // protocol can never be served for the wrong picture (the ABA bug).
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();

        let arc_a = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr_a = std::sync::Arc::as_ptr(&arc_a) as usize;
        let band_a = band_for(arc_a.clone());
        r.render_row(&picker, &band_a, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        assert_eq!(r.cache.len(), 1);
        // Drop every strong reference to A that this test holds; only the cache
        // still pins it. Its address stays reserved and un-reusable.
        drop(band_a);
        drop(arc_a);

        let arc_b = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr_b = std::sync::Arc::as_ptr(&arc_b) as usize;
        // The pin guarantees B cannot land on A's still-reserved address.
        assert_ne!(ptr_b, ptr_a, "cached Arc must keep A's address reserved");
        let band_b = band_for(arc_b);
        r.render_row(&picker, &band_b, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        // B is a fresh, distinct entry — it never reuses A's cached protocol.
        assert_eq!(r.cache.len(), 2);
    }

    #[test]
    fn retain_live_evicts_absent_bands_keeps_present() {
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();

        let arc1 = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let arc2 = std::sync::Arc::new(image::RgbaImage::new(16, 16));
        let ptr1 = std::sync::Arc::as_ptr(&arc1) as usize;
        let ptr2 = std::sync::Arc::as_ptr(&arc2) as usize;
        r.render_row(&picker, &band_for(arc1.clone()), Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        r.render_row(&picker, &band_for(arc2.clone()), Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        assert_eq!(r.cache.len(), 2);

        // Only band 1 is still live: band 2's entry is evicted, band 1's kept.
        r.retain_live(&std::collections::HashSet::from([ptr1]));
        assert_eq!(r.cache.len(), 1);
        assert!(r.cache.keys().any(|k| k.0 == ptr1));
        assert!(!r.cache.keys().any(|k| k.0 == ptr2));
    }
}
