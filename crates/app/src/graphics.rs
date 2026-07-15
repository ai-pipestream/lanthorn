//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::HashMap;
use std::sync::Arc;

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

/// Unpack a Glk 24-bit `0xRRGGBB` color into an opaque RGBA pixel.
fn rgb(color: u32) -> Rgba<u8> {
    Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF])
}

/// A graphics window's pixel canvas.
///
/// `img` is an `Arc` so [`arc`](Canvas::arc) — called for every graphics window
/// on every screen refresh (once per timer tick during an animation) — is a
/// cheap reference-count bump, not a full-bitmap deep copy. Mutations go through
/// `Arc::make_mut`, which copies-on-write only when a previously-handed-out clone
/// is still alive, so a static canvas is never copied. (SQ-0343)
pub struct Canvas {
    pub img: Arc<RgbaImage>,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        // Default background is TRANSPARENT, not opaque black: a graphics window's
        // pixels that the game hasn't painted (a fresh canvas, or one just cleared
        // by a resize before the game's Arrange redraw lands) must show the pane
        // underneath, never a solid black block. Games that want an opaque
        // background set it via glk_window_set_background_color. (SQ-0332)
        Canvas { img: Arc::new(RgbaImage::new(w.max(1), h.max(1))), bg: Rgba([0, 0, 0, 0x00]), version: 1 }
    }

    /// Resize (preserving nothing — Glk redraws) if the pixel dims changed. Cleared
    /// to `bg` (transparent unless the game set one), so an un-redrawn window shows
    /// the pane, not a black block.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (self.img.width(), self.img.height()) != (w.max(1), h.max(1)) {
            self.img = Arc::new(RgbaImage::from_pixel(w.max(1), h.max(1), self.bg));
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
        let img = Arc::make_mut(&mut self.img);
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x as u32, y as u32, px);
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
        image::imageops::overlay(Arc::make_mut(&mut self.img), view, x as i64, y as i64);
        self.version += 1;
    }

    /// A cheap clone of the canvas bitmap (an `Arc` ref-count bump — see the type
    /// docs), handed to the renderer each frame.
    pub fn arc(&self) -> Arc<RgbaImage> { Arc::clone(&self.img) }
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    cache: HashMap<u32, Option<Arc<DynamicImage>>>,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        PictSource { blorb, cache: HashMap::new() }
    }

    fn get(&mut self, resnum: u32) -> Option<&Arc<DynamicImage>> {
        if !self.cache.contains_key(&resnum) {
            let decoded = self.blorb.as_ref()
                .and_then(|b| b.resource(b"Pict", resnum))
                .and_then(|(_ty, bytes)| crate::cover::decode(bytes))
                .map(Arc::new);
            self.cache.insert(resnum, decoded);
        }
        self.cache.get(&resnum).and_then(|o| o.as_ref())
    }

    /// `(width, height)` of a Pict, or `None`.
    pub fn info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.get(resnum).map(|i| i.dimensions())
    }

    /// The decoded image for a Pict, or `None`. Returns a cheap `Arc` clone
    /// of the cached decode rather than deep-copying the `DynamicImage`.
    pub fn image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        self.get(resnum).cloned()
    }

    /// The bytes + text-flag of Blorb `Data` resource `resnum`, for
    /// `glk_stream_open_resource`. `is_text` is true for a `TEXT` chunk, false
    /// for `BINA`/`FORM` (binary). `None` when there is no Blorb or no such Data
    /// resource. (The `PictSource` is AppGlk's sole Blorb holder, so Data lookup
    /// lives here alongside `Pict` lookup.)
    pub fn data_resource(&self, resnum: u32) -> Option<(Vec<u8>, bool)> {
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Data", resnum)?;
        Some((bytes.to_vec(), ty == b"TEXT"))
    }
}

/// Test-only: build a minimal Blorb containing one `Pict` resource whose raw
/// bytes are `data`, at resource number `resnum` — for tests that need a
/// resolvable image without a full story file.
#[cfg(test)]
pub(crate) fn test_blorb_with_pict(resnum: u32, data: &[u8]) -> blorb::Blorb {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    let ridx_data_len = 4 + 12; // count + one 12-byte entry
    let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
    let pict_chunk = chunk(b"PNG ", data);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Pict");
    ridx.extend_from_slice(&resnum.to_be_bytes());
    ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
    let ridx_chunk = chunk(b"RIdx", &ridx);
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&ridx_chunk);
    inner.extend_from_slice(&pict_chunk);
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    blorb::Blorb::parse(file).expect("valid test blorb")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_shares_while_unchanged_and_copies_on_write() {
        // arc() must be a cheap Arc share when the canvas hasn't changed (the
        // per-tick deep-clone this removes), and a later draw must NOT mutate a
        // frame already handed to the renderer (copy-on-write isolation). (SQ-0343)
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4); // red
        let snap = c.arc(); // renderer's frame this tick
        assert!(Arc::ptr_eq(&snap, &c.img), "arc() shares the bitmap, no deep copy");
        c.fill_rect(0x0000_00FF, 0, 0, 4, 4); // game draws blue next tick
        assert_eq!(snap.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF], "handed-out frame stays red");
        assert_eq!(c.img.get_pixel(0, 0).0, [0, 0, 0xFF, 0xFF], "live canvas is now blue");
        assert!(!Arc::ptr_eq(&snap, &c.img), "make_mut copied-on-write for the new draw");
    }

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

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// Build a minimal Blorb carrying one `Data` resource at `resnum` with the
    /// given chunk type (`b"TEXT"` / `b"BINA"`) and raw bytes.
    fn test_blorb_with_data(resnum: u32, chunk_ty: &[u8; 4], data: &[u8]) -> blorb::Blorb {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12;
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let data_chunk = chunk(chunk_ty, data);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Data");
        ridx.extend_from_slice(&resnum.to_be_bytes());
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&data_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid test blorb")
    }

    #[test]
    fn data_resource_reads_text_and_binary_chunks() {
        // A TEXT chunk reports is_text=true; a BINA chunk false; a missing
        // number and a Blorb-less source both yield None.
        let src = PictSource::new(Some(test_blorb_with_data(3, b"TEXT", b"hello")));
        assert_eq!(src.data_resource(3), Some((b"hello".to_vec(), true)));
        assert_eq!(src.data_resource(4), None, "no such Data resource");

        let bin = PictSource::new(Some(test_blorb_with_data(1, b"BINA", &[1, 2, 3])));
        assert_eq!(bin.data_resource(1), Some((vec![1, 2, 3], false)));

        assert_eq!(PictSource::new(None).data_resource(1), None, "no blorb → None");
    }

    #[test]
    fn image_hands_out_cheap_arc_clones_of_one_decode() {
        // SQ-0175 part B: `PictSource::image` must not deep-clone the decoded
        // `DynamicImage` on every draw — repeated calls for the same resnum
        // should return `Arc` clones pointing at the same allocation.
        let blorb = test_blorb_with_pict(1, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        let a = src.image(1).expect("resolves");
        let b = src.image(1).expect("resolves");
        assert!(Arc::ptr_eq(&a, &b), "both calls must share one cached decode");
        assert_eq!(a.dimensions(), (2, 2));
    }
}
