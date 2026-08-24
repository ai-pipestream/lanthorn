//! Amiga bitmap fonts, as Infocom shipped them on the release floppies (SQ-0916).
//!
//! Four releases carry one. It is an ordinary AmigaDOS **disk font**, which looks
//! like an executable and is not: the file opens `$000003F3` (`HUNK_HEADER`) and its
//! single CODE hunk begins `70 00 4E 75` — `moveq #0,d0 / rts`, the stub that makes
//! running one as a program a harmless no-op. The font proper starts behind it.
//!
//! | release | file | geometry |
//! |---|---|---|
//! | *Arthur* | `char.data` | 10×10 nominal, **proportional** (widths 2–8), chars 32–127 |
//! | *Journey* | `Char.data` | 8×8 fixed, chars 32–255 — Z-machine font 3, not letters |
//! | *Beyond Zork* | `Graphic.Data` | byte-identical to Journey's, under another name |
//! | *Shogun*, *Zork Zero* | — | none; they take the system topaz |
//!
//! # Layout
//!
//! From `<diskfont/diskfont.h>` and `<graphics/text.h>`: the four-byte stub, then
//! `DiskFontHeader` — a 14-byte `Node`, `dfh_FileID` (`0x0F80`, the signature this
//! parser checks), `dfh_Revision`, `dfh_Segment`, and a 32-byte name — then
//! `TextFont`, whose own 20-byte `Message` precedes `tf_YSize`. Glyph rows are one
//! long bitmap per row, `tf_Modulo` bytes wide, all glyphs side by side;
//! `tf_CharLoc` is an array of `(bit offset, bit width)` pairs, one per code.
//!
//! # The bitmap width is NOT the advance (SQ-1009)
//!
//! `tf_CharLoc` says how many bits of the strike belong to a glyph. What the PEN
//! does is two other arrays: `tf_CharKern` is added BEFORE the glyph is drawn (the
//! left side bearing) and `tf_CharSpace` after it, so one character advances by
//! `kern + space` and the ink sits `kern` in from the pen. Reading the strike width
//! as the advance is wrong for every glyph in a proportional face and catastrophic
//! for the SPACE, whose strike is **zero bits wide** — draw with it and the words
//! run together, which is exactly what happened the first time this face was
//! rasterised.
//!
//! Arthur's own table settles a question three notes on SQ-1009 could not: summed
//! over `THE CHURCHYARD` it gives **80 px**, against the 83-px highlight box
//! measured on `machine-screenshots/amiga-arthur-hint.png`, and 4.70 px/char on
//! prose against the 4.5 measured in `machine-screenshots/info.txt`. Both agree at
//! **1:1** and are off by a factor of two at 2:1 — so those captures are native
//! 320x200 frames, independently of the palette argument.
//!
//! A fixed-pitch face carries neither array (both pointers are zero on Journey's
//! and Beyond Zork's font-3 sets), and there the advance is `tf_XSize`.
//!
//! `dfh_Name` is EMPTY on all of these, which is why they are files beside the game
//! rather than entries in `FONTS:` — the interpreter loads the segment directly
//! instead of asking `diskfont.library` for a font by name.
//!
//! # Bit order
//!
//! Rows come back **MSB-leftmost**, exactly as the disk stores them, so a row can be
//! read against a hex dump without mental arithmetic — one byte per row for a glyph
//! up to 8px wide (bearing included), more for a wider one; see
//! [`crate::bitmap_font::Glyph::row_bytes`] and [`crate::bitmap_font::row_bit`]
//! (SQ-1038). `app`'s renderer flips a DIFFERENT, unrelated font (`font8x8`, which
//! numbers columns from the low bit) with [`u8::reverse_bits`] — that trick only
//! applies to a single byte, so it is not the right tool for a row of these.

use crate::bitmap_font::{BitmapFont, Glyph};

/// `dfh_FileID`, the signature that separates a disk font from any other hunk file.
const DFH_ID: u16 = 0x0F80;
/// `FPF_PROPORTIONAL` in `tf_Flags`.
const FPF_PROPORTIONAL: u8 = 0x20;
/// Offset of `TextFont` from the start of the CODE hunk: the 4-byte stub, then
/// `DiskFontHeader` up to and including its 32-byte `dfh_Name`.
const TF: usize = 0x3A;

/// Parse a disk font, or `None` when the bytes are not one.
    ///
    /// Every field is bounds-checked against the buffer and the geometry has to
    /// agree with itself, so this can be pointed at any file on a volume — which is
    /// how it is used, since the name varies per release and `Graphic.Data` sounds
    /// like artwork.
pub fn parse(raw: &[u8]) -> Option<BitmapFont> {
    let be16 = |o: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?]))
    };
    let be32 = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes([
            *raw.get(o)?,
            *raw.get(o + 1)?,
            *raw.get(o + 2)?,
            *raw.get(o + 3)?,
        ]))
    };
    // HUNK_HEADER, then an EMPTY resident-library list — a font has none, and
    // requiring that is what keeps this off ordinary executables cheaply.
    if be32(0)? != 0x0000_03F3 || be32(4)? != 0 {
        return None;
    }
    // table size, first, last, one length per hunk, then HUNK_CODE and its size.
    let hunks = usize::try_from(be32(16)?.checked_sub(be32(12)?)?.checked_add(1)?).ok()?;
    let body = 20usize.checked_add(hunks.checked_mul(4)?)?.checked_add(8)?;
    let at = |o: usize| body.checked_add(o);

    if be16(at(0x12)?)? != DFH_ID {
        return None;
    }
    let height = be16(at(TF + 20)?)?;
    let flags = *raw.get(at(TF + 23)?)?;
    let width = be16(at(TF + 24)?)?;
    let baseline = be16(at(TF + 26)?)?;
    // `tf_BoldSmear`, between the baseline and `tf_Accessors` — how far the face
    // smears (and how much wider it advances) when a run asks for bold (SQ-1009).
    let bold_smear = be16(at(TF + 28)?)?;
    let (lo, hi) = (*raw.get(at(TF + 32)?)?, *raw.get(at(TF + 33)?)?);
    let chardata = usize::try_from(be32(at(TF + 34)?)?).ok()?;
    let modulo = usize::from(be16(at(TF + 38)?)?);
    let charloc = usize::try_from(be32(at(TF + 40)?)?).ok()?;
    // Zero means "this font has no such array", which is how a fixed-pitch face is
    // stored — not an offset of zero.
    let charspace = usize::try_from(be32(at(TF + 44)?)?).ok().filter(|&p| p != 0);
    let charkern = usize::try_from(be32(at(TF + 48)?)?).ok().filter(|&p| p != 0);
    // Bounds that only a decoding error could exceed. NOTE the nominal width is
    // NOT capped at 8: Arthur's font is 10 wide nominally while every glyph in
    // it is 8 or narrower, and capping here rejected it outright. The width that
    // has to fit a byte is the per-GLYPH one, checked below.
    if hi < lo || height == 0 || height > 32 || width == 0 || width > 32 || modulo == 0 {
        return None;
    }

    let mut glyphs = Vec::with_capacity(usize::from(hi - lo) + 1);
    for i in 0..=usize::from(hi - lo) {
        let entry = charloc.checked_add(i.checked_mul(4)?)?;
        let off = usize::from(be16(at(entry)?)?);
        let gw = usize::from(be16(at(entry + 2)?)?);
        // A signed word each, and both default to the fixed-pitch behaviour when
        // the font omits them: no bearing, and one nominal cell of advance.
        let word = |p: usize| -> Option<i16> {
            Some(be16(at(p.checked_add(i.checked_mul(2)?)?)?)? as i16)
        };
        let kern = match charkern {
            Some(p) => word(p)?,
            None => 0,
        };
        let space = match charspace {
            Some(p) => word(p)?,
            None => i16::try_from(width).ok()?,
        };
        // The bearing is carried in the BITMAP, the way `mac_font` carries the
        // Macintosh's, so a consumer needs only the advance and the rows. A
        // negative bearing cannot be represented that way and is dropped rather
        // than clipping the glyph's left edge; none of the shipped faces has one.
        let bearing = usize::try_from(kern.max(0)).ok()?;
        // The row has to hold bearing + ink, not just ink (SQ-1038) — this used to
        // cap at 8 and silently DROP the bearing past it rather than decline, which
        // flushed a glyph left instead of refusing it; now it refuses, the same
        // discipline `mac_font` applies.
        let span = bearing.checked_add(gw)?;
        if span > crate::bitmap_font::MAX_ROW_WIDTH {
            return None;
        }
        let row_bytes = span.div_ceil(8).max(1);
        let mut rows = Vec::with_capacity(usize::from(height) * row_bytes);
        for y in 0..usize::from(height) {
            let base = chardata.checked_add(y.checked_mul(modulo)?)?;
            let mut row = vec![0u8; row_bytes];
            for x in 0..gw {
                let bit = off.checked_add(x)?;
                if *raw.get(at(base.checked_add(bit / 8)?)?)? & (0x80 >> (bit % 8)) != 0 {
                    let col = x + bearing;
                    row[col / 8] |= 0x80 >> (col % 8);
                }
            }
            rows.extend_from_slice(&row);
        }
        glyphs.push(Glyph { width: u8::try_from(kern.saturating_add(space).max(0)).ok()?, rows });
    }
Some(BitmapFont {
        width: u8::try_from(width).ok()?,
        height: u8::try_from(height).ok()?,
        baseline: u8::try_from(baseline).ok()?,
        bold_smear: u8::try_from(bold_smear).unwrap_or(0),
        proportional: flags & FPF_PROPORTIONAL != 0
            || BitmapFont::measure_proportional(&glyphs),
        lo,
        glyphs,
    })
}

/// The font a mounted volume carries, if it carries one.
///
/// `files` is whatever the medium reported, as `(path, bytes)` — the same door
/// [`crate::infocom_sound::from_volume`] takes. **Nothing is matched on the
/// filename**: the two names in use are `char.data` and `Char.data`, one release
/// calls the identical file `Graphic.Data`, and case varies by volume, so every file
/// is offered to [`AmigaFont::parse`] and the signature decides. A volume with no
/// font costs one failed signature check per file.
///
/// When more than one parses, the widest-covering wins, so a text font outranks the
/// 95-glyph font-3 set on a hypothetical disk carrying both.
pub fn from_volume<'a, I>(files: I) -> Option<BitmapFont>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    files
        .into_iter()
        .filter_map(|(_, bytes)| parse(bytes))
        .max_by_key(|f| f.glyphs.len())
}

/// Every font a volume carries, each with the file it was found in.
///
/// [`from_volume`]'s companion: that one CHOOSES, this one REPORTS, for a
/// surface showing a person what a medium holds. The name is kept because it is
/// the only handle a reader has on an Amiga face — nothing is matched on it (see
/// above), so it is a label rather than a key.
pub fn faces_in_volume<'a, I>(files: I) -> Vec<(String, BitmapFont)>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    files.into_iter().filter_map(|(name, bytes)| parse(bytes).map(|f| (name.to_string(), f))).collect()
}
