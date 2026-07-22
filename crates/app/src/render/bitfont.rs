//! Embedded CC0 8×8 ASCII bitmap font (`font8x8`), rasterized into an RGBA
//! canvas for the v6 pixel composite (Phase 1c). Glyphs are scaled
//! nearest-neighbour to fill a `cw × ch` device-pixel cell so text stays
//! legible at terminal cell sizes (~9×19). A taller/native font is SQ-0450.

use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};

/// Blit one glyph into `canvas`, top-left at device pixel `(px, py)`, scaled to
/// `cw × ch` device px. Set bits paint `fg`; clear bits paint `bg` when `Some`
/// (skipped when `None`, leaving the canvas — transparent text over graphics).
/// Unprintable / out-of-font chars paint only `bg` (a blank cell). Blits are
/// clipped to the canvas bounds.
pub fn blit_glyph(
    canvas: &mut RgbaImage,
    glyph: char,
    px: u32,
    py: u32,
    cw: u32,
    ch: u32,
    fg: Rgba<u8>,
    bg: Option<Rgba<u8>>,
) {
    let bits = font8x8::BASIC_FONTS.get(glyph); // Option<[u8; 8]>
    let (cwidth, cheight) = (canvas.width(), canvas.height());
    for dy in 0..ch {
        let oy = py + dy;
        if oy >= cheight {
            break;
        }
        let row = (dy * 8 / ch) as usize; // nearest source row
        for dx in 0..cw {
            let ox = px + dx;
            if ox >= cwidth {
                break;
            }
            let col = (dx * 8 / cw) as u32; // nearest source col
            // font8x8 packs each row LSB = leftmost column.
            let on = bits.map_or(false, |g| g[row] & (1 << col) != 0);
            if on {
                canvas.put_pixel(ox, oy, fg);
            } else if let Some(b) = bg {
                canvas.put_pixel(ox, oy, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_paints_only_bg() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]));
        blit_glyph(&mut c, ' ', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), Some(Rgba([9, 9, 9, 255])));
        // No set bits → every pixel is the bg fill, none is fg.
        assert!(c.pixels().all(|p| *p == Rgba([9, 9, 9, 255])));
    }

    #[test]
    fn glyph_sets_some_fg_pixels() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'A', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None);
        // 'A' has set bits → at least one fg pixel, and transparent bg elsewhere.
        assert!(c.pixels().any(|p| *p == Rgba([255, 0, 0, 255])), "A has fg pixels");
        assert!(c.pixels().any(|p| p[3] == 0), "unset bits stay transparent (bg=None)");
    }

    #[test]
    fn transparent_bg_leaves_canvas_on_clear_bits() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        blit_glyph(&mut c, '.', 0, 0, 8, 8, Rgba([255, 255, 255, 255]), None);
        // A '.' is mostly clear; those cells keep the original canvas colour.
        assert!(c.pixels().any(|p| *p == Rgba([1, 2, 3, 255])), "clear bits keep canvas");
    }

    #[test]
    fn out_of_range_char_is_blank() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, '\u{2588}', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None);
        assert!(c.pixels().all(|p| p[3] == 0), "unknown glyph paints nothing with bg=None");
    }

    #[test]
    fn scales_up_to_fill_cell() {
        // 8×8 glyph blitted into a 16×16 cell must touch the lower-right quadrant.
        let mut c = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None);
        assert!(
            (8..16).any(|y| (0..16).any(|x| c.get_pixel(x, y)[3] == 255)),
            "scaled glyph reaches the lower half of the cell"
        );
    }
}
