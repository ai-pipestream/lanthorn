//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::{HashMap, HashSet};
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
/// Process-global draw sequence: every v6 picture draw stamps the target
/// canvas with the next value, so the renderer can z-order overlapping v6
/// windows by DRAW ORDER (later draw = on top) instead of window number — the
/// order the game actually painted them (e.g. Zork0 draws its banner, then the
/// compass overlays, then the room illustration on top). (SQ-0186)
static DRAW_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The next global draw-sequence stamp.
pub fn next_draw_seq() -> u64 {
    DRAW_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Canvas {
    pub img: Arc<RgbaImage>,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
    /// Global draw-order stamp of this canvas's most recent picture draw
    /// (0 = never drawn). The v6 compositor sorts overlapping windows by this,
    /// so later-drawn windows paint on top. Set only on the v6 picture path.
    pub z_seq: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        // Default background is TRANSPARENT, not opaque black: a graphics window's
        // pixels that the game hasn't painted (a fresh canvas, or one just cleared
        // by a resize before the game's Arrange redraw lands) must show the pane
        // underneath, never a solid black block. Games that want an opaque
        // background set it via glk_window_set_background_color. (SQ-0332)
        Canvas { img: Arc::new(RgbaImage::new(w.max(1), h.max(1))), bg: Rgba([0, 0, 0, 0x00]), version: 1, z_seq: 0 }
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

    /// Grow the canvas to at least `w × h`, PRESERVING existing content (a v6
    /// window can receive several stacked pictures, and a picture may extend past
    /// the window's nominal pixel size — e.g. Zork0's 45×40 compass into a 320×5
    /// banner). Never shrinks; a no-op when already big enough. Unlike `resize`,
    /// this keeps what was already drawn. (SQ-0186)
    pub fn grow_to(&mut self, w: u32, h: u32) {
        let (cw, ch) = (self.img.width(), self.img.height());
        let (nw, nh) = (cw.max(w.max(1)), ch.max(h.max(1)));
        if (nw, nh) == (cw, ch) {
            return;
        }
        let mut grown = RgbaImage::from_pixel(nw, nh, self.bg);
        image::imageops::replace(&mut grown, &*self.img, 0, 0);
        self.img = Arc::new(grown);
        self.version += 1;
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

    /// Like [`Canvas::draw_image`] (unscaled), but clipped to the `clip`
    /// pixel box `(w, h)` anchored at the canvas origin — ZMSD §8's "all
    /// plotting is always clipped to the current window" for a canvas that may
    /// be larger than the window's current box (a window that shrank keeps its
    /// old pixels; only new plotting is bounded by the new size).
    pub fn draw_image_clipped(&mut self, src: &DynamicImage, x: i32, y: i32, clip: (u32, u32)) {
        if x < 0 || y < 0 {
            // v6 draw coords are 1-based-positive by the time they reach the
            // canvas; anything else is clamped upstream.
            return;
        }
        let (cx, cy) = (x as u32, y as u32);
        let (cw, ch) = clip;
        if cx >= cw || cy >= ch {
            return;
        }
        let allow_w = (cw - cx).min(src.width());
        let allow_h = (ch - cy).min(src.height());
        if allow_w == 0 || allow_h == 0 {
            return;
        }
        if allow_w < src.width() || allow_h < src.height() {
            let cropped = src.crop_imm(0, 0, allow_w, allow_h);
            image::imageops::overlay(Arc::make_mut(&mut self.img), &cropped, x as i64, y as i64);
        } else {
            image::imageops::overlay(Arc::make_mut(&mut self.img), src, x as i64, y as i64);
        }
        self.version += 1;
    }

    /// A cheap clone of the canvas bitmap (an `Arc` ref-count bump — see the type
    /// docs), handed to the renderer each frame.
    pub fn arc(&self) -> Arc<RgbaImage> { Arc::clone(&self.img) }
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
///
/// Adaptive palettes (Blorb spec §11.3): pictures listed in the container's
/// `APal` chunk carry a PLACEHOLDER palette. When one is drawn it must be
/// plotted with the "Current Palette" — the palette (PLTE) of the most recently
/// drawn NON-adaptive picture — not its own. We track the current palette as raw
/// PLTE bytes and, when decoding an adaptive picture, splice those bytes into a
/// copy of its PNG's PLTE chunk (fixing the CRC) before handing it to the
/// decoder. Only the PLTE is substituted: the spec derives the Current Palette
/// from "PLTE, gAMA, cHRM and sRGB/iCCP" but NOT tRNS, and Infocom's adaptive
/// overlays rely on their OWN tRNS for the transparent index, so tRNS is left
/// intact. (Since the decoder reads palette RGB verbatim, copying PLTE alone
/// reproduces exactly the colours the base picture renders with.)
#[derive(Debug)]
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    cache: HashMap<u32, Option<Arc<DynamicImage>>>,
    /// Pict numbers declared adaptive by the Blorb `APal` chunk (§11.3). Empty
    /// for the overwhelmingly common no-`APal` case, where `image` takes the
    /// original palette-agnostic fast path.
    adaptive: HashSet<u32>,
    /// Raw PLTE bytes (RGB triples) of the most recently drawn non-adaptive
    /// indexed picture — the "Current Palette". `None` until one is drawn; per
    /// §11.3 an adaptive picture drawn before any non-adaptive one is undefined,
    /// and we fall back to its own placeholder palette.
    current_plte: Option<Vec<u8>>,
    /// Bumped whenever `current_plte` actually changes. Adaptive decodes are
    /// cached per `(resnum, palette_gen)` so a palette change re-decodes them
    /// (the same overlay is legally drawn under different base palettes over a
    /// game's life).
    palette_gen: u64,
    /// Adaptive decodes keyed by `(resnum, palette_gen)`.
    adaptive_cache: HashMap<(u32, u64), Option<Arc<DynamicImage>>>,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        let adaptive = blorb
            .as_ref()
            .map(|b| b.adaptive_pictures().iter().copied().collect())
            .unwrap_or_default();
        PictSource {
            blorb,
            cache: HashMap::new(),
            adaptive,
            current_plte: None,
            palette_gen: 0,
            adaptive_cache: HashMap::new(),
        }
    }

    /// The Blorb `Reso` standard window `(width, height)` in pixels — the
    /// resolution the pictures were authored for. A v6 story advertises this
    /// as its screen size so its hardcoded pixel art lines up (SQ-0186).
    /// `None` when there's no Blorb or no `Reso` chunk.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        self.blorb.as_ref().and_then(|b| b.std_window())
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

    /// The decoded image for a Pict about to be DRAWN, or `None`. Returns a
    /// cheap `Arc` clone rather than deep-copying the `DynamicImage`.
    ///
    /// This is the adaptive-palette establishment point (Blorb §11.3): drawing a
    /// NON-adaptive picture updates the Current Palette from its PLTE; drawing an
    /// ADAPTIVE picture decodes it with that Current Palette spliced in. Size
    /// queries (`info`/`dims`) deliberately do NOT go through here, so querying a
    /// picture's dimensions never counts as "drawing" for palette purposes.
    pub fn image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        // No APal chunk → no adaptive pictures: keep the original fast path
        // (and never touch palette state) for every non-v6 / non-adaptive blorb.
        if self.adaptive.is_empty() {
            return self.get(resnum).cloned();
        }
        if self.adaptive.contains(&resnum) {
            return self.adaptive_image(resnum);
        }
        // A non-adaptive draw establishes the Current Palette for later adaptive
        // draws, then resolves normally.
        let arc = self.get(resnum).cloned();
        if arc.is_some() {
            self.set_current_palette_from(resnum);
        }
        arc
    }

    /// Remember Pict `resnum`'s PLTE as the Current Palette (§11.3). No-op for a
    /// non-indexed picture (no PLTE); bumps `palette_gen` only on a real change.
    fn set_current_palette_from(&mut self, resnum: u32) {
        let Some(plte) = self
            .blorb
            .as_ref()
            .and_then(|b| b.resource(b"Pict", resnum))
            .and_then(|(_ty, bytes)| png_plte(bytes))
        else {
            return;
        };
        if self.current_plte.as_deref() != Some(plte.as_slice()) {
            self.current_plte = Some(plte);
            self.palette_gen += 1;
        }
    }

    /// Decode an adaptive picture with the Current Palette spliced into its PLTE
    /// (§11.3), caching per `(resnum, palette_gen)`. With no base picture drawn
    /// yet the palette is undefined per spec; we fall back to the placeholder.
    fn adaptive_image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        let key = (resnum, self.palette_gen);
        if !self.adaptive_cache.contains_key(&key) {
            // Clone the raw PNG bytes so the immutable blorb borrow ends before
            // we mutate the cache.
            let raw = self
                .blorb
                .as_ref()
                .and_then(|b| b.resource(b"Pict", resnum))
                .map(|(_ty, bytes)| bytes.to_vec());
            let decoded = raw.and_then(|raw| {
                let spliced = self
                    .current_plte
                    .as_ref()
                    .and_then(|plte| splice_plte(&raw, plte));
                crate::cover::decode(spliced.as_deref().unwrap_or(&raw))
            });
            self.adaptive_cache.insert(key, decoded.map(Arc::new));
        }
        self.adaptive_cache.get(&key).and_then(|o| o.clone())
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

    /// `(width, height)` of Pict `resnum`, sniffed from the image header only —
    /// no full decode. Used by the v6 Z-machine `picture_data` dimension table
    /// (Plan 1a), where only the size is needed at boot, not the pixels.
    ///
    /// A `Rect` chunk (Blorb §Rect: 8 bytes, width then height, big-endian) is a
    /// dimension-only placeholder with no pixels — Infocom v6 games (Zork Zero,
    /// Shogun, Arthur) query these via `picture_data` as invisible *placement*
    /// pictures whose (height, width) encode screen (y, x) layout coordinates.
    pub fn dims(&mut self, resnum: u32) -> Option<(u32, u32)> {
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Pict", resnum)?;
        if ty == b"Rect" {
            let b: &[u8] = bytes;
            if b.len() < 8 {
                return None;
            }
            let w = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            let h = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
            return Some((w, h));
        }
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    }

    /// `(number, width, height)` for every `Pict` resource in the Blorb, header-
    /// sniffed via [`PictSource::dims`]. Feeds the v6 `Machine::set_picture_dims`
    /// injection at session construction (Plan 1a). Empty when there is no Blorb.
    pub fn all_pict_dims(&mut self) -> Vec<(u16, u16, u16)> {
        let numbers: Vec<u32> = match &self.blorb {
            Some(b) => b.resources().iter().filter(|r| &r.usage == b"Pict").map(|r| r.number).collect(),
            None => Vec::new(),
        };
        numbers
            .into_iter()
            .filter_map(|n| self.dims(n).map(|(w, h)| (n as u16, w as u16, h as u16)))
            .collect()
    }
}

/// PNG 8-byte signature.
const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The raw PLTE chunk data (palette RGB triples) of a PNG byte stream, or `None`
/// when the bytes aren't a PNG or carry no PLTE (e.g. a truecolor picture, which
/// therefore never becomes the Current Palette — Blorb §11.3 tracks PLTE).
fn png_plte(png: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let mut q = 8;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            break;
        }
        if ty == b"PLTE" {
            return Some(png[ds..ds + len].to_vec());
        }
        q = ds + len + 4; // data + 4-byte CRC
    }
    None
}

/// The bit depth from a PNG's IHDR (always the first chunk), used to cap the
/// spliced palette to the `2^bitdepth`-entry PLTE maximum. `None` if `png` isn't
/// a PNG opening with an IHDR chunk.
fn png_bit_depth(png: &[u8]) -> Option<u8> {
    // [sig 8][len 4][IHDR 4][width 4][height 4][bit_depth 1]… → offset 24.
    if png.len() <= 24 || &png[0..8] != PNG_SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    Some(png[24])
}

/// A copy of PNG `png` with its PLTE chunk data replaced by the Current Palette
/// `new_plte` (CRC recomputed), or `None` if `png` isn't an indexed PNG carrying
/// a PLTE. Entry-count differences (Blorb §11.3): the replacement is capped to
/// the picture's bit-depth maximum (`2^bitdepth` entries); when the Current
/// Palette is SHORTER than the placeholder it replaces, the placeholder's
/// trailing entries are kept so no pixel index is left without a colour. Only
/// PLTE is touched — every other chunk (crucially tRNS, which carries the
/// overlay's transparent index) is copied verbatim.
fn splice_plte(png: &[u8], new_plte: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let max_bytes = (1usize << png_bit_depth(png)?).saturating_mul(3);
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&png[0..8]);
    let mut q = 8;
    let mut replaced = false;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            return None; // truncated chunk → don't hand a corrupt stream on
        }
        if ty == b"PLTE" {
            let orig = &png[ds..ds + len];
            let mut pal = new_plte[..new_plte.len().min(max_bytes)].to_vec();
            if pal.len() < orig.len() {
                pal.extend_from_slice(&orig[pal.len()..]); // keep trailing indices in range
            }
            pal.truncate(max_bytes);
            pal.truncate(pal.len() - pal.len() % 3); // whole RGB triples only
            out.extend_from_slice(&(pal.len() as u32).to_be_bytes());
            out.extend_from_slice(b"PLTE");
            out.extend_from_slice(&pal);
            out.extend_from_slice(&png_crc(b"PLTE", &pal).to_be_bytes());
            replaced = true;
        } else {
            out.extend_from_slice(&png[q..ds + len + 4]);
        }
        q = ds + len + 4;
    }
    replaced.then_some(out)
}

/// CRC-32 (PNG/zlib polynomial `0xEDB88320`) over a chunk's type bytes followed
/// by its data — the value PNG stores after each chunk's payload.
fn png_crc(ty: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in ty.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
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
    fn dims_and_all_pict_dims_header_sniff_without_full_decode() {
        // A tiny in-memory Blorb with one Pict (a 2x2 PNG) at resource number 5.
        let blorb = test_blorb_with_pict(5, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        assert_eq!(src.dims(5), Some((2, 2)));
        assert_eq!(src.dims(99), None, "no such resource");
        assert_eq!(src.all_pict_dims(), vec![(5u16, 2u16, 2u16)]);

        assert_eq!(PictSource::new(None).all_pict_dims(), Vec::<(u16, u16, u16)>::new());
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

    // ── Adaptive palettes (Blorb spec §11.3, SQ-0485) ───────────────────────

    /// zlib "stored" (uncompressed) wrapper so we can hand-build indexed PNGs
    /// without a compressor: header, one final stored block, adler32 trailer.
    fn zlib_store(raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(ty);
        v.extend_from_slice(data);
        v.extend_from_slice(&super::png_crc(ty, data).to_be_bytes());
        v
    }

    /// A `w`×`h` 4-bit indexed PNG. `rows[y][x]` = palette index; `palette` is
    /// RGB triples; `trns` optional per-index alpha. Filter-none scanlines.
    fn indexed_png(w: u32, h: u32, palette: &[u8], trns: Option<&[u8]>, rows: &[&[u8]]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[4, 3, 0, 0, 0]); // bitdepth=4, indexed, comp/filter/interlace=0
        let mut raw = Vec::new();
        for row in rows {
            raw.push(0); // filter: none
            let mut x = 0usize;
            while x < w as usize {
                let hi = row[x] & 0xf;
                let lo = if x + 1 < w as usize { row[x + 1] & 0xf } else { 0 };
                raw.push((hi << 4) | lo);
                x += 2;
            }
        }
        let mut png = super::PNG_SIG.to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
        png.extend_from_slice(&png_chunk(b"PLTE", palette));
        if let Some(t) = trns {
            png.extend_from_slice(&png_chunk(b"tRNS", t));
        }
        png.extend_from_slice(&png_chunk(b"IDAT", &zlib_store(&raw)));
        png.extend_from_slice(&png_chunk(b"IEND", b""));
        png
    }

    /// The stored trailing CRC of the first chunk of type `ty` in a PNG.
    fn stored_crc(png: &[u8], ty: &[u8; 4]) -> Option<u32> {
        let mut q = 8;
        while q + 12 <= png.len() {
            let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
            let t = &png[q + 4..q + 8];
            let ds = q + 8;
            if ds + len + 4 > png.len() {
                break;
            }
            if t == ty {
                let c = &png[ds + len..ds + len + 4];
                return Some(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
            }
            q = ds + len + 4;
        }
        None
    }

    /// Build a Blorb with the given `(number, png_bytes)` Pict resources and an
    /// `APal` chunk listing `apal` as adaptive.
    fn blorb_apal(picts: &[(u32, &[u8])], apal: &[u32]) -> blorb::Blorb {
        fn iff(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12 * picts.len();
        let mut apal_bytes = Vec::new();
        for n in apal {
            apal_bytes.extend_from_slice(&n.to_be_bytes());
        }
        let apal_chunk = iff(b"APal", &apal_bytes);
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2) + apal_chunk.len();
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_n, data) in picts {
            offsets.push(cursor as u32);
            let c = iff(b"PNG ", data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&(picts.len() as u32).to_be_bytes());
        for (i, (n, _d)) in picts.iter().enumerate() {
            ridx.extend_from_slice(b"Pict");
            ridx.extend_from_slice(&n.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff(b"RIdx", &ridx));
        inner.extend_from_slice(&apal_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid apal test blorb")
    }

    fn top_left(img: &DynamicImage) -> [u8; 4] {
        img.to_rgba8().get_pixel(0, 0).0
    }

    #[test]
    fn splice_plte_substitutes_palette_fixes_crc_and_decodes() {
        // 2×1, both pixels index 1. Placeholder idx1 = magenta.
        let png = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        assert_eq!(top_left(&crate::cover::decode(&png).unwrap()), [170, 0, 170, 255], "placeholder is magenta");
        // Current Palette: idx1 = green.
        let current = [0u8, 0, 0, 0, 170, 0];
        let spliced = super::splice_plte(&png, &current).expect("indexed PNG splices");
        assert_eq!(super::png_plte(&spliced).as_deref(), Some(&current[..]), "PLTE now the current palette");
        // CRC was recomputed over the new PLTE (not left stale).
        assert_eq!(stored_crc(&spliced, b"PLTE"), Some(super::png_crc(b"PLTE", &current)));
        // Decodes cleanly (CRC valid) to the substituted colour.
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [0, 170, 0, 255], "now green");
    }

    #[test]
    fn splice_plte_keeps_trailing_entries_when_current_is_shorter() {
        // Placeholder has 4 entries; the pixel uses index 3.
        let placeholder = [0, 0, 0, 10, 10, 10, 20, 20, 20, 200, 100, 50];
        let png = indexed_png(2, 1, &placeholder, None, &[&[3, 3]]);
        // Current Palette shorter (2 entries): index 3 would otherwise dangle.
        let spliced = super::splice_plte(&png, &[1, 2, 3, 4, 5, 6]).unwrap();
        let pal = super::png_plte(&spliced).unwrap();
        assert_eq!(pal.len(), placeholder.len(), "length kept so index 3 stays in range");
        assert_eq!(&pal[0..6], &[1, 2, 3, 4, 5, 6], "leading entries from the current palette");
        assert_eq!(&pal[9..12], &[200, 100, 50], "trailing placeholder entry retained");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [200, 100, 50, 255], "index 3 still resolves");
    }

    #[test]
    fn splice_plte_caps_current_palette_to_bit_depth_max() {
        let png = indexed_png(2, 1, &[0, 0, 0, 9, 9, 9], None, &[&[1, 1]]);
        // A 20-entry current palette exceeds the 16-entry (2^4) PLTE cap.
        let current: Vec<u8> = (0..20u8).flat_map(|i| [i, i, i]).collect();
        let spliced = super::splice_plte(&png, &current).unwrap();
        assert_eq!(super::png_plte(&spliced).unwrap().len(), 16 * 3, "capped to 16 entries");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [1, 1, 1, 255], "idx1 → current[1]");
    }

    #[test]
    fn splice_and_plte_reject_non_indexed_png() {
        // A truecolor PNG has no PLTE; §11.3 derives the palette from PLTE, so
        // there is nothing to substitute and the adaptive path decodes it as-is.
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 8, 7]));
        let mut rgb = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut rgb), image::ImageFormat::Png)
            .unwrap();
        assert!(super::png_plte(&rgb).is_none(), "truecolor PNG has no PLTE");
        assert!(super::splice_plte(&rgb, &[1, 2, 3]).is_none(), "nothing to splice");
    }

    #[test]
    fn adaptive_picture_uses_current_palette_and_reacts_to_palette_change() {
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]); // placeholder magenta
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        assert!(src.adaptive.contains(&2) && !src.adaptive.contains(&1), "APal set parsed");

        // (a) Adaptive drawn before any base is undefined per §11.3 → placeholder.
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "no base yet → own placeholder");

        // (b) Draw the green base, then the adaptive: it takes the green palette.
        src.image(1).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [0, 170, 0, 255], "plotted with current (green) palette");

        // (c) A different base re-establishes the palette; the SAME adaptive
        //     picture re-decodes (cache keyed by palette generation).
        src.image(3).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [200, 0, 0, 255], "palette change re-decodes adaptive");
    }

    #[test]
    fn size_queries_do_not_establish_the_palette() {
        // `info`/`dims` must not count as "drawing": querying a base picture's
        // size must NOT set the Current Palette that later adaptive draws use.
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        src.info(1); // size query on the base — not a draw
        src.dims(1);
        assert!(src.current_plte.is_none(), "a size query must not establish the palette");
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "adaptive still on its placeholder");
    }
}
