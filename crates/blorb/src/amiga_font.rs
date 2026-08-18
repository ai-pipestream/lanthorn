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
//! `dfh_Name` is EMPTY on all of these, which is why they are files beside the game
//! rather than entries in `FONTS:` — the interpreter loads the segment directly
//! instead of asking `diskfont.library` for a font by name.
//!
//! # Bit order
//!
//! Rows come back **MSB-leftmost**, exactly as the disk stores them, so a row can be
//! read against a hex dump without mental arithmetic. A consumer that wants the
//! opposite convention flips with [`u8::reverse_bits`]; `app`'s renderer does, since
//! `font8x8` numbers columns from the low bit.

/// One glyph: its advance width in pixels, and one byte per row.
///
/// Only widths up to 8 are representable, which is every glyph in every font
/// measured here — [`AmigaFont::parse`] refuses a font that needs more rather than
/// silently truncating one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    /// How far the pen advances, which on a proportional font is the inked width
    /// and not [`AmigaFont::width`].
    pub width: u8,
    /// One byte per row, `height` of them, MSB = leftmost column.
    pub rows: Vec<u8>,
}

/// A parsed Amiga disk font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmigaFont {
    /// `tf_XSize`: the nominal cell width. On a proportional font this is the
    /// widest glyph, not every glyph's width — see [`Glyph::width`].
    pub width: u8,
    /// `tf_YSize`: rows per glyph, and the length of every [`Glyph::rows`].
    pub height: u8,
    /// `tf_Baseline`: rows from the top of the cell down to the baseline.
    pub baseline: u8,
    /// `tf_Flags & FPF_PROPORTIONAL`. *Arthur*'s font is; the font-3 one is not.
    pub proportional: bool,
    /// `tf_LoChar`: the code the first glyph stands for.
    pub lo: u8,
    /// One entry per code from [`AmigaFont::lo`] upward.
    pub glyphs: Vec<Glyph>,
}

/// `dfh_FileID`, the signature that separates a disk font from any other hunk file.
const DFH_ID: u16 = 0x0F80;
/// `FPF_PROPORTIONAL` in `tf_Flags`.
const FPF_PROPORTIONAL: u8 = 0x20;
/// Offset of `TextFont` from the start of the CODE hunk: the 4-byte stub, then
/// `DiskFontHeader` up to and including its 32-byte `dfh_Name`.
const TF: usize = 0x3A;

impl AmigaFont {
    /// The glyph for `code`, or `None` when the font does not cover it.
    pub fn glyph(&self, code: u8) -> Option<&Glyph> {
        self.glyphs.get(usize::from(code).checked_sub(usize::from(self.lo))?)
    }

    /// Parse a disk font, or `None` when the bytes are not one.
    ///
    /// Every field is bounds-checked against the buffer and the geometry has to
    /// agree with itself, so this can be pointed at any file on a volume — which is
    /// how it is used, since the name varies per release and `Graphic.Data` sounds
    /// like artwork.
    pub fn parse(raw: &[u8]) -> Option<AmigaFont> {
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
        let (lo, hi) = (*raw.get(at(TF + 32)?)?, *raw.get(at(TF + 33)?)?);
        let chardata = usize::try_from(be32(at(TF + 34)?)?).ok()?;
        let modulo = usize::from(be16(at(TF + 38)?)?);
        let charloc = usize::try_from(be32(at(TF + 40)?)?).ok()?;
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
            if gw > 8 {
                return None;
            }
            let mut rows = Vec::with_capacity(usize::from(height));
            for y in 0..usize::from(height) {
                let base = chardata.checked_add(y.checked_mul(modulo)?)?;
                let mut bits = 0u8;
                for x in 0..gw {
                    let bit = off.checked_add(x)?;
                    if *raw.get(at(base.checked_add(bit / 8)?)?)? & (0x80 >> (bit % 8)) != 0 {
                        bits |= 0x80 >> x;
                    }
                }
                rows.push(bits);
            }
            glyphs.push(Glyph { width: u8::try_from(gw).ok()?, rows });
        }
        Some(AmigaFont {
            width: u8::try_from(width).ok()?,
            height: u8::try_from(height).ok()?,
            baseline: u8::try_from(baseline).ok()?,
            proportional: flags & FPF_PROPORTIONAL != 0,
            lo,
            glyphs,
        })
    }
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
pub fn from_volume<'a, I>(files: I) -> Option<AmigaFont>
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    files
        .into_iter()
        .filter_map(|(_, bytes)| AmigaFont::parse(bytes))
        .max_by_key(|f| f.glyphs.len())
}
