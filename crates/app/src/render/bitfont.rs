//! Embedded 8×8 bitmap font, rasterized into an RGBA canvas for the v6 pixel
//! composite (Phase 1c). Glyphs are scaled to fill a `cw × ch` device-pixel
//! cell so text stays legible at terminal cell sizes (~9×19). A taller/native
//! font is future work; this pass (SQ-0450) is a coverage + quality revision
//! of the existing 8×8 set plus optional edge smoothing when blit at exact 2×.
//!
//! ## Provenance
//! The bulk of the glyph data comes from the `font8x8` crate (`BASIC_FONTS`,
//! `LATIN_FONTS`, `BOX_FONTS`, `BLOCK_FONTS`) — a CC0/public-domain 8×8 font
//! ported from `https://github.com/dhepper/font8x8`, itself extracted from a
//! public-domain `.asm` source. No copyrighted ROM font is used anywhere in
//! this file.
//!
//! `font8x8` doesn't cover every codepoint v6 titles actually print, though:
//! BeyondZork's "font 3" (see `zvm::cpu::exec::font3_translate`) emits cursor
//! arrows, an APL quad, and ~26 decorative "atmosphere" runic codepoints, and
//! the ZSCII default Unicode table (155–223, ZMSD §3.8.5) includes `œ`/`Œ`
//! which fall outside `font8x8`'s Latin-1-only extended-Latin set.
//! `EXTRA_GLYPHS` below supplies ORIGINAL 8×8 bitmaps for exactly those gaps,
//! hand-drawn for this pass — not sourced from any font, ROM, or existing
//! artwork. The runic entries are stylised angular stem-and-branch
//! placeholders (a full-height vertical stave plus a distinct combination of
//! diagonal/bar strokes per codepoint): each reads as "a rune" and is
//! visually distinct from the others, but this is NOT a claim of scholarly
//! Elder Futhark letterform accuracy — treat them as decorative placeholders,
//! matching their in-game use as unreadable "atmosphere" text.

use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};

/// Original 8×8 bitmaps for codepoints `font8x8` doesn't ship (see module
/// docs for provenance and design notes). Checked only after `font8x8`'s
/// built-in sets in [`glyph_bits`], since this is a short, hand-authored
/// fallback list, not a general-purpose font.
static EXTRA_GLYPHS: &[(char, [u8; 8])] = &[
    ('\u{2190}', [0x00, 0x08, 0x0C, 0x7E, 0x0C, 0x08, 0x00, 0x00]), // ← cursor left (BeyondZork font 3)
    ('\u{2192}', [0x00, 0x10, 0x30, 0x7E, 0x30, 0x10, 0x00, 0x00]), // → cursor right
    ('\u{2191}', [0x08, 0x1C, 0x3E, 0x08, 0x08, 0x08, 0x08, 0x00]), // ↑ cursor up
    ('\u{2193}', [0x00, 0x08, 0x08, 0x08, 0x08, 0x3E, 0x1C, 0x08]), // ↓ cursor down
    ('\u{2195}', [0x08, 0x1C, 0x08, 0x08, 0x08, 0x08, 0x1C, 0x08]), // ↕ cursor up/down
    ('\u{2395}', [0x00, 0x7E, 0x42, 0x42, 0x42, 0x42, 0x7E, 0x00]), // ⎕ APL quad (font 3 code 95)
    ('\u{FFFD}', [0xFF, 0x81, 0xBD, 0xA5, 0xA5, 0xBD, 0x81, 0xFF]), // unknown-glyph placeholder box
    ('\u{0153}', [0x00, 0x00, 0x36, 0x49, 0x79, 0x09, 0x76, 0x00]), // œ (ZSCII default table)
    ('\u{0152}', [0x00, 0x76, 0x09, 0x39, 0x09, 0x09, 0x76, 0x00]), // Œ (ZSCII default table)
    // BeyondZork font-3 "atmosphere" runic placeholders (codes 97–122).
    ('\u{16AA}', [0x14, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16D2}', [0x05, 0x06, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16C7}', [0x14, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16DE}', [0x05, 0x06, 0x04, 0x04, 0x04, 0x04, 0x06, 0x05]),
    ('\u{16D6}', [0x04, 0x04, 0x04, 0x3C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16A0}', [0x04, 0x04, 0x04, 0x07, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16B7}', [0x14, 0x0C, 0x04, 0x1C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16BB}', [0x04, 0x04, 0x0C, 0x14, 0x0C, 0x04, 0x04, 0x04]),
    ('\u{16C1}', [0x04, 0x04, 0x06, 0x05, 0x06, 0x04, 0x04, 0x04]),
    ('\u{16C4}', [0x04, 0x0C, 0x04, 0x14, 0x04, 0x04, 0x0C, 0x04]),
    ('\u{16E6}', [0x04, 0x06, 0x04, 0x05, 0x04, 0x04, 0x06, 0x04]),
    ('\u{16DA}', [0x1C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1C]),
    ('\u{16D7}', [0x07, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x07]),
    ('\u{16BE}', [0x04, 0x04, 0x1C, 0x04, 0x04, 0x1C, 0x04, 0x04]),
    ('\u{16A9}', [0x04, 0x0C, 0x14, 0x04, 0x14, 0x0C, 0x04, 0x04]),
    ('\u{15BE}', [0x04, 0x0C, 0x14, 0x0C, 0x14, 0x0C, 0x04, 0x04]),
    ('\u{16B3}', [0x04, 0x04, 0x04, 0x7C, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16B1}', [0x14, 0x0C, 0x04, 0x14, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16CB}', [0x04, 0x04, 0x0E, 0x14, 0x0E, 0x04, 0x04, 0x04]),
    ('\u{16CF}', [0x15, 0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('\u{16A2}', [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E, 0x15]),
    ('\u{16E0}', [0x14, 0x0C, 0x04, 0x1C, 0x04, 0x04, 0x0C, 0x14]),
    ('\u{16B9}', [0x04, 0x14, 0x04, 0x14, 0x04, 0x14, 0x04, 0x04]),
    ('\u{16C9}', [0x14, 0x04, 0x14, 0x04, 0x14, 0x04, 0x14, 0x04]),
    ('\u{16A5}', [0x04, 0x06, 0x04, 0x06, 0x04, 0x06, 0x04, 0x04]),
    ('\u{16DF}', [0x14, 0x0C, 0x04, 0x3C, 0x04, 0x04, 0x04, 0x04]),
];

/// Look up the 8×8 bitmap for `glyph`. Tries `font8x8`'s basic-Latin,
/// Latin-1-supplement, box-drawing, and block-element sets (all CC0,
/// covering ASCII, the ZSCII default accented-character table, and
/// BeyondZork's font-3 box/block glyphs) in order, then falls back to
/// [`EXTRA_GLYPHS`] for the handful of codepoints those sets don't cover.
fn glyph_bits(glyph: char) -> Option<[u8; 8]> {
    font8x8::BASIC_FONTS
        .get(glyph)
        .or_else(|| font8x8::LATIN_FONTS.get(glyph))
        .or_else(|| font8x8::BOX_FONTS.get(glyph))
        .or_else(|| font8x8::BLOCK_FONTS.get(glyph))
        .or_else(|| EXTRA_GLYPHS.iter().find(|&&(c, _)| c == glyph).map(|&(_, bits)| bits))
}

/// Read one bit from an 8×8 bitmap, clamping out-of-range `row`/`col` to the
/// nearest edge pixel (so [`scale2x`]'s neighbour rule degrades to plain
/// replication at the glyph's border instead of treating off-canvas space as
/// background).
fn bit_at(bits: &[u8; 8], row: i32, col: i32) -> bool {
    let r = row.clamp(0, 7) as usize;
    let c = col.clamp(0, 7) as usize;
    bits[r] & (1 << c) != 0
}

/// Upscale an 8×8 monochrome bitmap to 16×16 using the Scale2x/AdvMAME2x edge
/// rule: each destination corner takes its diagonal source neighbour's value
/// only when that neighbour agrees with one adjacent side and disagrees with
/// the other. That rounds the re-entrant corners nearest-neighbour leaves
/// stair-stepped on diagonal strokes (box-drawing diagonals, letter serifs),
/// while flat/orthogonal regions fall back to the source pixel unchanged.
/// Pixel-deterministic — no blending, no alpha, same input always yields the
/// same output.
fn scale2x(bits: &[u8; 8]) -> [[bool; 16]; 16] {
    let mut out = [[false; 16]; 16];
    for row in 0..8i32 {
        for col in 0..8i32 {
            let p = bit_at(bits, row, col);
            let a = bit_at(bits, row - 1, col); // north
            let b = bit_at(bits, row, col + 1); // east
            let c = bit_at(bits, row, col - 1); // west
            let d = bit_at(bits, row + 1, col); // south
            let e0 = if c == a && c != d && a != b { a } else { p };
            let e1 = if a == b && a != c && b != d { b } else { p };
            let e2 = if d == c && c != a && d != b { c } else { p };
            let e3 = if d == b && b != a && d != c { b } else { p };
            let (r0, c0) = (row as usize * 2, col as usize * 2);
            out[r0][c0] = e0;
            out[r0][c0 + 1] = e1;
            out[r0 + 1][c0] = e2;
            out[r0 + 1][c0 + 1] = e3;
        }
    }
    out
}

/// Blit one glyph into `canvas`, top-left at device pixel `(px, py)`, scaled to
/// `cw × ch` device px. Set bits paint `fg`; clear bits paint `bg` when `Some`
/// (skipped when `None`, leaving the canvas — transparent text over graphics).
/// Unprintable / out-of-font chars paint only `bg` (a blank cell). Blits are
/// clipped to the canvas bounds.
///
/// When `cw`/`ch` request an exact 2× cell (16×16), edges are smoothed via
/// [`scale2x`] instead of plain nearest-neighbour doubling. Any other size
/// (including the native 8×8 used by the current v6 pixel-canvas call sites)
/// uses nearest-neighbour, unchanged from before.
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
    let bits = glyph_bits(glyph); // Option<[u8; 8]>
    let smoothed = (cw == 16 && ch == 16).then(|| bits.map(|b| scale2x(&b))).flatten();
    let (cwidth, cheight) = (canvas.width(), canvas.height());
    for dy in 0..ch {
        let oy = py + dy;
        if oy >= cheight {
            break;
        }
        let row = (dy * 8 / ch) as usize; // nearest source row (non-smoothed path)
        for dx in 0..cw {
            let ox = px + dx;
            if ox >= cwidth {
                break;
            }
            let on = if let Some(grid) = &smoothed {
                grid[dy as usize][dx as usize]
            } else {
                let col = dx * 8 / cw; // nearest source col
                // font8x8 packs each row LSB = leftmost column.
                bits.is_some_and(|g| g[row] & (1 << col) != 0)
            };
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

    fn assert_has_glyph(c: char) {
        let bits = glyph_bits(c);
        assert!(bits.is_some(), "{c:?} (U+{:04X}) has no glyph", c as u32);
        if c != ' ' {
            assert_ne!(bits.unwrap(), [0u8; 8], "{c:?} (U+{:04X}) resolves to a blank glyph", c as u32);
        }
    }

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
        // U+2588 (a block glyph) used to be the "out of range" probe here,
        // back when only BASIC_FONTS was consulted; it's covered now (via
        // BLOCK_FONTS, per the coverage audit), so this uses a CJK ideograph
        // instead — genuinely outside every set `glyph_bits` checks.
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, '\u{4E2D}', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None);
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

    #[test]
    fn glyph_coverage_ascii_printable() {
        for b in 32u8..=126 {
            assert_has_glyph(b as char);
        }
    }

    #[test]
    fn glyph_coverage_zscii_default_unicode_table() {
        // ZSCII 155-223 (ZMSD §3.8.5) is the full set of accented characters
        // v6 titles can print via the default Unicode translation table.
        for zscii in 155u16..=223 {
            assert_has_glyph(zvm::text::decode::zscii_to_char(zscii));
        }
    }

    #[test]
    fn glyph_coverage_font3_box_and_cursor_symbols() {
        // BeyondZork's font-3 box-drawing/block/cursor codepoints
        // (zvm::cpu::exec::font3_translate, codes 32-96 minus the unassigned
        // 71-74 gap which maps to U+FFFD).
        let symbols = [
            '\u{2190}', '\u{2191}', '\u{2192}', '\u{2193}', '\u{2195}',
            '\u{2500}', '\u{2502}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2524}',
            '\u{2514}', '\u{250C}', '\u{2510}', '\u{2518}', '\u{253C}',
            '\u{2571}', '\u{2572}', '\u{2573}',
            '\u{2588}', '\u{2580}', '\u{2584}', '\u{258C}', '\u{2590}',
            '\u{259D}', '\u{2597}', '\u{2596}', '\u{2598}',
            '\u{2594}', '\u{2581}', '\u{258F}', '\u{2595}',
            '\u{258E}', '\u{258D}', '\u{258B}', '\u{258A}', '\u{2589}',
            '\u{2395}', '\u{FFFD}',
        ];
        for c in symbols {
            assert_has_glyph(c);
        }
    }

    #[test]
    fn glyph_coverage_font3_runic_placeholders() {
        // The 26 BeyondZork "atmosphere" runic codepoints (font-3 codes 97-122).
        for &(c, _) in EXTRA_GLYPHS {
            assert_has_glyph(c);
        }
    }

    #[test]
    fn extra_glyphs_are_pairwise_distinct() {
        // Sanity check that the hand-drawn runic placeholders don't collapse
        // onto duplicate bitmaps (which would make two different in-game
        // codepoints look identical).
        for (i, (ci, gi)) in EXTRA_GLYPHS.iter().enumerate() {
            for (cj, gj) in EXTRA_GLYPHS.iter().skip(i + 1) {
                assert_ne!(gi, gj, "{:?} and {:?} render identically", ci, cj);
            }
        }
    }

    #[test]
    fn scale2x_path_is_deterministic() {
        let mut c1 = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        let mut c2 = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c1, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None);
        blit_glyph(&mut c2, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None);
        assert_eq!(c1, c2, "same glyph/size blitted twice must produce identical pixels");
    }

    #[test]
    fn scale2x_reshapes_a_diagonal_versus_naive_doubling() {
        // '╱' is a pure diagonal stroke — the textbook case scale2x smooths.
        let bits = glyph_bits('\u{2571}').expect("box diagonal glyph must exist");
        let mut smoothed = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut smoothed, '\u{2571}', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None);

        // Naive nearest-neighbour doubling: the pre-smoothing behaviour,
        // each source pixel expands into an exact 2×2 block.
        let mut naive = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        for row in 0..8u32 {
            for col in 0..8u32 {
                if bits[row as usize] & (1 << col) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            naive.put_pixel(col * 2 + dx, row * 2 + dy, Rgba([255, 0, 0, 255]));
                        }
                    }
                }
            }
        }
        assert_ne!(smoothed, naive, "scale2x should reshape corners vs naive doubling on a diagonal");
    }

    #[test]
    fn bitfont_sample_png_renders_1x_2x_3x() {
        // Composite pangram + digits + box/arrow glyphs at 1x/2x/3x scale, so
        // the controller can eyeball the CC0 base font, the coverage fixes,
        // and the scale2x smoothing side-by-side. Written to target/ (not
        // committed) — a visual oracle, not a pass/fail assertion.
        let text_rows = [
            "THE QUICK BROWN FOX JUMPS",
            "the quick brown fox jumps",
            "0123456789 !?.,;:'\"-+=/\\",
            "\u{2190}\u{2191}\u{2192}\u{2193}\u{2195} \u{2395} \u{FFFD}",
            "\u{250C}\u{2500}\u{2500}\u{2510} \u{2588}\u{2580}\u{2584}\u{258C}\u{2590}",
            "\u{2514}\u{2500}\u{2500}\u{2518} \u{16A0}\u{16A2}\u{16B1}\u{16C9}\u{16DF}",
            "\u{00E4}\u{00F6}\u{00FC}\u{00DF} \u{0153}\u{0152} \u{00E9}\u{00E8}\u{00E7}\u{00F1}",
        ];
        let scales: [u32; 3] = [1, 2, 3];
        let cols = text_rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u32;
        let rows = text_rows.len() as u32;

        // Lay the three scales out left-to-right with a gutter between them.
        let gutter = 8u32;
        let mut x_offset = 0u32;
        let mut panel_w = [0u32; 3];
        for (i, &s) in scales.iter().enumerate() {
            panel_w[i] = cols * 8 * s;
        }
        let total_w: u32 = panel_w.iter().sum::<u32>() + gutter * (scales.len() as u32 - 1);
        let total_h: u32 = rows * 8 * scales.iter().max().copied().unwrap_or(1);

        let mut canvas = RgbaImage::from_pixel(total_w.max(1), total_h.max(1), Rgba([16, 16, 20, 255]));
        let fg = Rgba([230, 230, 200, 255]);

        for (i, &s) in scales.iter().enumerate() {
            let cell = 8 * s;
            // blit_glyph only smooths at exactly 16x16 (s == 2); other
            // scales fall back to nearest, which is expected/documented.
            for (row, line) in text_rows.iter().enumerate() {
                for (col, ch) in line.chars().enumerate() {
                    let px = x_offset + col as u32 * cell;
                    let py = row as u32 * cell;
                    blit_glyph(&mut canvas, ch, px, py, cell, cell, fg, None);
                }
            }
            x_offset += panel_w[i] + gutter;
        }

        let out_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bitfont_sample.png");
        if let Some(dir) = out_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        canvas.save(&out_path).expect("write bitfont sample PNG");
    }
}
