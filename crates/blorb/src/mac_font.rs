//! Macintosh `FONT` / `NFNT` bitmap fonts, as the v6 releases carry them (SQ-0911).
//!
//! Every Macintosh v6 title — *Arthur*, *Journey*, *Shogun*, *Zork Zero* — keeps two
//! of these in its resource fork, and *Beyond Zork* keeps one. MEASURED off
//! `/MAC/ARTHUR FOLDER/ARTHUR`:
//!
//! | resource | cell | advance widths |
//! |---|---|---|
//! | `FONT` 524 | 7×15 | 6, 7 |
//! | `FONT` 1033 | 7×12 | **6 only** |
//!
//! A `FONT` id encodes family × 128 + point size, so 524 is family 4 at 12pt and
//! 1033 is family 8 at 9pt; id ≡ 0 (mod 128) is the family-name record and carries
//! no bitmap, which is why [`parse`] refuses a zero-length resource rather than
//! treating it as a broken font.
//!
//! **These are the fonts worth drawing with.** The Amiga's are the other half of the
//! corpus and the wrong shape for us: Arthur's is proportional, and SQ-0916 has a
//! rendered comparison showing that centring a proportional font in babelmap's fixed
//! cell reads worse than the public-domain font it would replace. `FONT` 1033 gives
//! every drawn character the same advance, so it drops into the cell model cleanly.
//!
//! # Format
//!
//! A 26-byte header, then the **strike**: one wide bitmap holding every glyph side
//! by side, `rowWords` words per row, `fRectHeight` rows. Behind it a location table
//! of `lastChar - firstChar + 3` words — glyph *n* occupies strike columns
//! `loc[n]..loc[n+1]`, so a glyph's width is the difference and a zero difference
//! means "not in this font". Behind that the offset/width table, a byte pair per
//! character, `0xFFFF` for a character the font does not define. The offset byte is
//! the left side bearing (added to `kernMax`), and [`parse`] applies it, so a glyph's
//! rows are already positioned inside its advance rather than flush left.
//!
//! The last two entries are the "missing character" glyph and the terminator, which
//! is why the tables carry two more entries than the font has characters.

use crate::bitmap_font::{BitmapFont, Glyph};

/// Header size, and so the offset of the strike.
const HEADER: usize = 26;

/// Parse a `FONT` or `NFNT` resource, or `None` when the bytes are not one.
///
/// The glyph rows come back MSB-leftmost and at most 8 columns wide. A font whose
/// glyphs are wider is refused rather than truncated; every one measured here is 7.
pub fn parse(raw: &[u8]) -> Option<BitmapFont> {
    let be16 = |o: usize| -> Option<u16> { Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?])) };
    let first = usize::from(be16(2)?);
    let last = usize::from(be16(4)?);
    let rect_w = be16(12)?;
    let rect_h = usize::from(be16(14)?);
    let kern_max = i16::from_be_bytes([*raw.get(8)?, *raw.get(9)?]);
    let ow_loc = usize::from(be16(16)?);
    let ascent = be16(18)?;
    let row_words = usize::from(be16(24)?);

    // Bounds that only a decoding error, or a family-name record with no bitmap,
    // could fail. `first` past `last` and a zero-height cell are the giveaways.
    if last < first || last > 255 || rect_h == 0 || rect_h > 32 || row_words == 0 || rect_w == 0 || rect_w > 8 {
        return None;
    }
    let strike = row_words.checked_mul(2)?.checked_mul(rect_h)?;
    if HEADER.checked_add(strike)? > raw.len() {
        return None;
    }
    // `owTLoc` counts WORDS from its own field, which sits at offset 16.
    let ow_table = 16usize.checked_add(ow_loc.checked_mul(2)?)?;
    let n = last.checked_sub(first)?.checked_add(3)?; // +2 sentinels, +1 inclusive
    let loc_table = ow_table.checked_sub(n.checked_mul(2)?)?;

    let mut glyphs = Vec::with_capacity(n);
    for i in 0..n - 1 {
        let (lo, hi) = (be16(loc_table + i * 2)?, be16(loc_table + i * 2 + 2)?);
        let img_w = usize::from(hi.checked_sub(lo)?);
        // The advance is the offset/width table's width byte; `0xFF` marks a
        // character the font does not define, which draws and advances as nothing.
        let obyte = *raw.get(ow_table + i * 2)?;
        let wbyte = *raw.get(ow_table + i * 2 + 1)?;
        let advance = if wbyte == 0xFF { 0 } else { wbyte };
        // The LEFT SIDE BEARING, which is `kernMax + owOffset` and is what places a
        // narrow glyph inside its advance. Dropping it flushes every glyph left and
        // makes evenly-advanced text look raggedly letter-spaced — that mistake is
        // what made the first SQ-0916 preview of this font read far worse than it is.
        let bearing = if obyte == 0xFF {
            0
        } else {
            usize::try_from(i32::from(kern_max) + i32::from(obyte)).unwrap_or(0)
        };
        if img_w.checked_add(bearing)? > 8 {
            return None;
        }
        let mut rows = Vec::with_capacity(rect_h);
        for y in 0..rect_h {
            let base = HEADER + y * row_words * 2;
            let mut bits = 0u8;
            for x in 0..img_w {
                let bit = usize::from(lo) + x;
                if *raw.get(base + bit / 8)? & (0x80 >> (bit % 8)) != 0 {
                    bits |= 0x80 >> (x + bearing);
                }
            }
            rows.push(bits);
        }
        glyphs.push(Glyph { width: advance, rows });
    }
    // The final two entries are the missing-character glyph and the terminator;
    // neither stands for a code, so neither belongs in the table.
    glyphs.truncate(last - first + 1);
    Some(BitmapFont {
        width: u8::try_from(rect_w).ok()?,
        height: u8::try_from(rect_h).ok()?,
        baseline: u8::try_from(ascent).unwrap_or(0),
        proportional: BitmapFont::measure_proportional(&glyphs),
        lo: u8::try_from(first).ok()?,
        glyphs,
    })
}

/// The best font in a resource fork, or `None` when it holds none.
///
/// "Best" is the tallest cell, because that is the one designed for reading: the two
/// a v6 release carries are a 7×15 and a 7×12, and the taller is the body face.
/// `NFNT` is checked as well as `FONT` — they are the same payload under two type
/// codes, and a release could use either.
pub fn from_fork(fork: &crate::resource_fork::ResourceFork) -> Option<BitmapFont> {
    fork.of_type(b"FONT")
        .iter()
        .chain(fork.of_type(b"NFNT"))
        .filter_map(|r| parse(&r.data))
        .max_by_key(|f| f.height)
}
