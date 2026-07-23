//! The value type for an image that flows inline with text-buffer output
//! (Glk `glk_image_draw` into a text-buffer window), plus its cell geometry.
//! Rendered as a full-width block; the raw `align` is retained for a future
//! margin-float renderer.

use std::sync::Arc;

/// Glk `imagealign_*` argument for a buffer-window `glk_image_draw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAlign {
    InlineUp,
    InlineDown,
    InlineCenter,
    MarginLeft,
    MarginRight,
}

impl ImageAlign {
    /// Decode a Glk `imagealign` constant. Unknown values default to `InlineUp`.
    pub fn from_glk(v: u32) -> ImageAlign {
        match v {
            1 => ImageAlign::InlineUp,
            2 => ImageAlign::InlineDown,
            3 => ImageAlign::InlineCenter,
            4 => ImageAlign::MarginLeft,
            5 => ImageAlign::MarginRight,
            _ => ImageAlign::InlineUp,
        }
    }
}

/// Where a transcript-anchored [`InlineImage`] came from — decides which render
/// modes may draw it inline. (SQ-0461 decision 3)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ImageSource {
    /// A window-0 story picture (Zork Zero's drop-caps / room icons). Every mode
    /// floats it in the transcript — it *is* the story content.
    #[default]
    Story,
    /// Large CONTENT art drawn into a graphics window (windows 1–7), e.g.
    /// Shogun's title splash. Only `frameless` renders this inline; `hybrid`
    /// and `raster` draw the graphics window itself, so they must SKIP it to
    /// avoid double-rendering.
    ContentSplash,
}

/// An image drawn into a text-buffer window, carrying its pixels (shared, like
/// `GraphicsWindow.canvas`), its alignment, and an optional scaled target size.
#[derive(Clone, Debug)]
pub struct InlineImage {
    pub pixels: Arc<image::RgbaImage>,
    pub align: ImageAlign,
    pub scaled: Option<(u32, u32)>,
    /// For a margin float: the pixel x where text should start beside the image
    /// (the v6 game's own `set_margins` value when it followed the draw). `None`
    /// = derive from the image width. Ignored for inline (non-margin) aligns.
    pub margin_px: Option<u32>,
    /// Provenance — gates which render modes draw this image inline (SQ-0461).
    pub source: ImageSource,
}

impl InlineImage {
    /// The `(cols, rows)` this image occupies at the given band `width` and
    /// terminal cell pixel size, aspect-preserved and capped to `width`.
    /// Both dimensions floor at 1.
    pub fn fitted_cells(&self, width: u16, char_px: (u16, u16)) -> (u16, u16) {
        let (cell_w, cell_h) = (char_px.0.max(1) as u32, char_px.1.max(1) as u32);
        let (pw, ph) = self.scaled.unwrap_or_else(|| {
            let d = &self.pixels;
            (d.width().max(1), d.height().max(1))
        });
        let (pw, ph) = (pw.max(1), ph.max(1));
        let max_px_w = width.max(1) as u32 * cell_w;
        let (dw, dh) = if pw <= max_px_w {
            (pw, ph)
        } else {
            // Scale down to fit width, preserving aspect ratio.
            let dh = ((ph as u64 * max_px_w as u64) / pw as u64) as u32;
            (max_px_w, dh.max(1))
        };
        let cols = dw.div_ceil(cell_w).clamp(1, width.max(1) as u32) as u16;
        let rows = dh.div_ceil(cell_h).max(1) as u16;
        (cols, rows)
    }

    /// The frameless-mode target `(w, h)` in device pixels for this image, or
    /// `None` to keep native size — the value to store in `scaled` before render
    /// (SQ-0461 decision 2). Native 320×200-era v6 art is tiny on a modern
    /// display, so frameless upsizes it, crisply:
    ///
    /// - A **drop-cap float** (`MarginLeft`) scales toward **3.5 text rows** tall
    ///   (kept ~3–4 rows) via a crisp integer/half-integer/reciprocal factor —
    ///   large caps shrink, tiny caps grow.
    /// - Any **other (band) image** upsizes by an **integer 2×/3×** for pixel-art
    ///   crispness, capped at ~**60% of the viewport width**, never below 1×.
    ///
    /// `viewport_cols` is the transcript body width in cells; `char_px` the
    /// picker's cell pixel size. Only the frameless render path calls this;
    /// hybrid/raster leave `scaled` untouched (their own letterbox factor drives
    /// sizing), so their output stays byte-identical.
    pub fn frameless_scaled(&self, char_px: (u16, u16), viewport_cols: u16) -> Option<(u32, u32)> {
        let (cw, ch) = (char_px.0.max(1) as f32, char_px.1.max(1) as f32);
        let nw = self.pixels.width().max(1);
        let nh = self.pixels.height().max(1);
        match self.align {
            ImageAlign::MarginLeft => {
                let s = dropcap_scale(nh as f32, ch);
                let w = (nw as f32 * s).round().max(1.0) as u32;
                let h = (nh as f32 * s).round().max(1.0) as u32;
                Some((w, h))
            }
            _ => {
                let cap_px = 0.6 * viewport_cols.max(1) as f32 * cw;
                let s = band_upscale(nw as f32, cap_px);
                Some((nw * s, nh * s))
            }
        }
    }
}

/// Crisp scale factor for a drop-cap float: the candidate factor whose scaled
/// height lands closest to 3.5 text rows (keeping the picture ~3–4 rows tall).
/// Candidates are simple reciprocals (shrink oversized caps) and
/// integer/half-integer steps (grow small ones) so upscales stay pixel-crisp.
fn dropcap_scale(native_h: f32, cell_h: f32) -> f32 {
    let target = 3.5 * cell_h;
    const CANDS: &[f32] = &[
        0.25, 1.0 / 3.0, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 7.0, 8.0,
    ];
    let mut best = 1.0f32;
    let mut best_err = f32::MAX;
    for &s in CANDS {
        let err = (s * native_h - target).abs();
        if err < best_err {
            best_err = err;
            best = s;
        }
    }
    best
}

/// Integer upscale factor for a band image: 3× or 2× native for crisp pixel art,
/// but never wider than `cap_px`, and never below 1×.
fn band_upscale(native_w: f32, cap_px: f32) -> u32 {
    if native_w * 3.0 <= cap_px {
        3
    } else if native_w * 2.0 <= cap_px {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn img(w: u32, h: u32) -> InlineImage {
        InlineImage { pixels: Arc::new(image::RgbaImage::new(w, h)), align: ImageAlign::InlineUp, scaled: None , margin_px: None, source: ImageSource::Story }
    }

    fn img_aligned(w: u32, h: u32, align: ImageAlign) -> InlineImage {
        InlineImage { pixels: Arc::new(image::RgbaImage::new(w, h)), align, scaled: None, margin_px: None, source: ImageSource::Story }
    }

    #[test]
    fn align_decodes_all_glk_constants() {
        assert_eq!(ImageAlign::from_glk(1), ImageAlign::InlineUp);
        assert_eq!(ImageAlign::from_glk(2), ImageAlign::InlineDown);
        assert_eq!(ImageAlign::from_glk(3), ImageAlign::InlineCenter);
        assert_eq!(ImageAlign::from_glk(4), ImageAlign::MarginLeft);
        assert_eq!(ImageAlign::from_glk(5), ImageAlign::MarginRight);
        assert_eq!(ImageAlign::from_glk(999), ImageAlign::InlineUp); // unknown → default
    }

    #[test]
    fn fitted_cells_native_when_it_fits() {
        // 16x16 px, cell 8x8 → 2x2 cells; width 40 leaves it native.
        let (cols, rows) = img(16, 16).fitted_cells(40, (8, 8));
        assert_eq!((cols, rows), (2, 2));
    }

    #[test]
    fn fitted_cells_scales_down_to_width_preserving_aspect() {
        // 800x400 px, cell 8x8 → native 100x50 cells; width 40 → scale to 40 cols,
        // height scales by 40/100 → 20 cells.
        let (cols, rows) = img(800, 400).fitted_cells(40, (8, 8));
        assert_eq!(cols, 40);
        assert_eq!(rows, 20);
    }

    #[test]
    fn fitted_cells_uses_scaled_dims_when_present() {
        let mut i = img(16, 16);
        i.scaled = Some((80, 40)); // 80x40 px scaled request overrides native 16x16
        // 80x40 px, cell 8x8 → 10x5 cells; width 40 fits.
        assert_eq!(i.fitted_cells(40, (8, 8)), (10, 5));
    }

    #[test]
    fn fitted_cells_floor_is_one() {
        // Tiny image never disappears to 0 cells.
        assert_eq!(img(1, 1).fitted_cells(40, (8, 8)), (1, 1));
    }

    // ── frameless scaling policy (SQ-0461 decision 2) ──────────────────────

    /// Height (in whole cell rows) a drop-cap occupies after the frameless policy.
    fn dropcap_rows(native_h: u32, cell_h: u16) -> u32 {
        let im = img_aligned(native_h, native_h, ImageAlign::MarginLeft); // square, so h == native_h
        let (_, h) = im.frameless_scaled((cell_h, cell_h), 80).unwrap();
        h.div_ceil(cell_h as u32)
    }

    #[test]
    fn dropcap_targets_three_to_four_rows() {
        // A range of native cap heights all land in the 3–4 row band with a
        // 16px cell (target 3.5 rows = 56px).
        for native in [8u32, 12, 16, 20, 24, 32, 40, 48, 56] {
            let rows = dropcap_rows(native, 16);
            assert!((3..=4).contains(&rows), "native {native}px → {rows} rows (want 3–4)");
        }
    }

    #[test]
    fn dropcap_tiny_image_upscales() {
        // An 8px cap with a 16px cell → target 56px; the crisp factor is 7×,
        // so it grows rather than staying a single row.
        let im = img_aligned(8, 8, ImageAlign::MarginLeft);
        let (_, h) = im.frameless_scaled((16, 16), 80).unwrap();
        assert!(h >= 3 * 16, "tiny drop-cap must upscale toward 3–4 rows, got {h}px");
    }

    #[test]
    fn dropcap_large_image_downscales() {
        // A 120px cap with a 16px cell → target 56px; a reciprocal factor shrinks
        // it toward the band rather than leaving it 8 rows tall.
        let im = img_aligned(120, 120, ImageAlign::MarginLeft);
        let (_, h) = im.frameless_scaled((16, 16), 80).unwrap();
        assert!(h < 120, "oversized drop-cap must downscale, got {h}px");
    }

    #[test]
    fn band_image_upscales_to_integer_factor_capped_at_60pct() {
        // 100px-wide art, cell 8px, viewport 80 cols → 640px, 60% cap = 384px.
        // 3× = 300px ≤ 384 → 3×.
        let im = img_aligned(100, 60, ImageAlign::InlineUp);
        let (w, h) = im.frameless_scaled((8, 8), 80).unwrap();
        assert_eq!((w, h), (300, 180), "100px art upscales 3× under the 60% cap");
    }

    #[test]
    fn band_image_caps_upscale_to_viewport() {
        // 160px-wide art, cell 8px, viewport 80 cols → 640px, cap 384px.
        // 3× = 480 > 384; 2× = 320 ≤ 384 → 2×.
        let im = img_aligned(160, 100, ImageAlign::InlineUp);
        let (w, _) = im.frameless_scaled((8, 8), 80).unwrap();
        assert_eq!(w, 320, "art too wide for 3× falls back to 2×");
    }

    #[test]
    fn band_image_never_below_native() {
        // 320px art in a 40-col × 8px viewport (320px) → cap 192px; even 2× and 3×
        // overflow, so it stays native 1× (fitted_cells clamps it for display).
        let im = img_aligned(320, 200, ImageAlign::InlineUp);
        let (w, h) = im.frameless_scaled((8, 8), 40).unwrap();
        assert_eq!((w, h), (320, 200), "over-wide art is never shrunk below native");
    }

    #[test]
    fn fitted_cells_pins_known_geometry() {
        // 100x60 px image, char_px 8x16, band width 40.
        // max_px_w = 40 * 8 = 320; native width 100 <= 320, so no downscale:
        // dw=100, dh=60. cols = ceil(100/8) = 13 (clamped to width 40 → 13).
        // rows = ceil(60/16) = 4.
        let (cols, rows) = img(100, 60).fitted_cells(40, (8, 16));
        assert_eq!((cols, rows), (13, 4));
    }
}
