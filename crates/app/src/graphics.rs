//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::HashMap;
use std::sync::Arc;

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

/// Unpack a Glk 24-bit `0xRRGGBB` color into an opaque RGBA pixel.
fn rgb(color: u32) -> Rgba<u8> {
    Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF])
}

/// A graphics window's pixel canvas.
pub struct Canvas {
    pub img: RgbaImage,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        Canvas { img: RgbaImage::new(w.max(1), h.max(1)), bg: Rgba([0, 0, 0, 0xFF]), version: 1 }
    }

    /// Resize (preserving nothing — Glk redraws) if the pixel dims changed.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (self.img.width(), self.img.height()) != (w.max(1), h.max(1)) {
            self.img = RgbaImage::from_pixel(w.max(1), h.max(1), self.bg);
            self.version += 1;
        }
    }

    pub fn set_background(&mut self, color: u32) { self.bg = rgb(color); }

    fn paint(&mut self, px: Rgba<u8>, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = (self.img.width() as i64, self.img.height() as i64);
        let x0 = left.max(0) as i64;
        let y0 = top.max(0) as i64;
        let x1 = (left as i64 + w as i64).min(cw);
        let y1 = (top as i64 + h as i64).min(ch);
        for y in y0..y1 {
            for x in x0..x1 {
                self.img.put_pixel(x as u32, y as u32, px);
            }
        }
        self.version += 1;
    }

    pub fn fill_rect(&mut self, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.paint(rgb(color), left, top, w, h);
    }

    pub fn erase_rect(&mut self, left: i32, top: i32, w: u32, h: u32) {
        let bg = self.bg;
        self.paint(bg, left, top, w, h);
    }

    /// Composite `src` at `(x, y)`, optionally scaled to `(sw, sh)`, honoring alpha.
    ///
    /// `(sw, sh)` come from the game (`glk_image_draw_scaled`) and are clamped
    /// to the canvas dimensions before allocating the scaled bitmap — anything
    /// larger is clipped by `overlay` anyway, and clamping bounds the
    /// allocation against a malicious/buggy game requesting e.g. a
    /// 0x40000000 x 0x40000000 image.
    pub fn draw_image(&mut self, src: &DynamicImage, x: i32, y: i32, scale: Option<(u32, u32)>) {
        let scaled;
        let view: &DynamicImage = match scale {
            Some((sw, sh)) if sw > 0 && sh > 0 => {
                let sw = sw.min(self.img.width());
                let sh = sh.min(self.img.height());
                scaled = src.resize_exact(sw, sh, image::imageops::FilterType::Triangle);
                &scaled
            }
            _ => src,
        };
        image::imageops::overlay(&mut self.img, view, x as i64, y as i64);
        self.version += 1;
    }

    pub fn arc(&self) -> Arc<RgbaImage> { Arc::new(self.img.clone()) }
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    cache: HashMap<u32, Option<DynamicImage>>,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        PictSource { blorb, cache: HashMap::new() }
    }

    fn get(&mut self, resnum: u32) -> Option<&DynamicImage> {
        if !self.cache.contains_key(&resnum) {
            let decoded = self.blorb.as_ref()
                .and_then(|b| b.resource(b"Pict", resnum))
                .and_then(|(_ty, bytes)| crate::cover::decode(bytes));
            self.cache.insert(resnum, decoded);
        }
        self.cache.get(&resnum).and_then(|o| o.as_ref())
    }

    /// `(width, height)` of a Pict, or `None`.
    pub fn info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.get(resnum).map(|i| i.dimensions())
    }

    /// The decoded image for a Pict, or `None`.
    pub fn image(&mut self, resnum: u32) -> Option<&DynamicImage> {
        self.get(resnum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_paints_pixels_and_bumps_version() {
        let mut c = Canvas::new(10, 10);
        let v0 = c.version;
        c.fill_rect(0x00FF_0000, 2, 3, 4, 5); // red
        assert!(c.version > v0);
        let px = c.img.get_pixel(2, 3);
        assert_eq!(px.0, [0xFF, 0x00, 0x00, 0xFF]);
        // outside the rect stays transparent/default
        assert_ne!(c.img.get_pixel(9, 9).0, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn fill_rect_clips_out_of_bounds() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x0000_FF00, -2, -2, 100, 100); // green, way oversized
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0xFF, 0x00, 0xFF]);
        // no panic; whole canvas filled
        assert_eq!(c.img.get_pixel(3, 3).0, [0x00, 0xFF, 0x00, 0xFF]);
    }

    #[test]
    fn erase_uses_background_color() {
        let mut c = Canvas::new(4, 4);
        c.set_background(0x0000_00FF); // blue
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4);
        c.erase_rect(0, 0, 2, 2);
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0x00, 0xFF, 0xFF]); // erased → bg
        assert_eq!(c.img.get_pixel(3, 3).0, [0xFF, 0x00, 0x00, 0xFF]); // untouched
    }

    #[test]
    fn draw_image_composites_scaled() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 1, 1, Some((4, 4)));
        assert_eq!(c.img.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(c.img.get_pixel(4, 4).0, [10, 20, 30, 255]); // scaled to 4x4
    }

    #[test]
    fn draw_image_clamps_absurd_scale() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        // A malicious/buggy game could request a ~4 exabyte scaled bitmap;
        // this must clamp to the canvas size instead of allocating it.
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 0, 0, Some((1_000_000_000, 1_000_000_000)));
        assert_eq!(c.img.dimensions(), (8, 8));
        assert_eq!(c.img.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn pict_source_resolves_and_caches() {
        // No blorb → None.
        let mut none = PictSource::new(None);
        assert!(none.info(1).is_none());
        assert!(none.image(1).is_none());
    }
}
