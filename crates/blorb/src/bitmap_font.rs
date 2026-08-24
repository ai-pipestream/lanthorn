//! The shape a bitmap font comes back in, whichever machine's release it came off.
//!
//! Two producers, one type: [`crate::amiga_font`] reads an AmigaDOS disk font off a
//! floppy, [`crate::mac_font`] reads a Macintosh `FONT` resource out of a resource
//! fork. They agree on everything a renderer cares about, and differ only in how
//! they are stored, so the caller does not learn which machine it is drawing.
//!
//! Rows are **MSB-leftmost**, as both formats store them, so a row can be read
//! against a hex dump without mental arithmetic. For a glyph whose row is exactly
//! one byte — every glyph up to 8px wide, bearing included, which was every font
//! this module could read before SQ-1038 — a consumer wanting the opposite
//! convention flips with [`u8::reverse_bits`]; a wider row is [`Glyph::row_bytes`]
//! bytes and needs the byte order reversed too, so reach for [`row_bit`] instead
//! of hand-rolling that.

/// The widest a glyph row ([`Glyph::rows`]) can be — bearing plus ink, in pixels —
/// before a parser refuses the font rather than representing it (SQ-1038).
///
/// Comfortably past the widest glyph measured on real media: Geneva 24's glyphs
/// reach 26px (bearing included) off `FONT` 408 on a mounted System 6.0.8 startup
/// disk, and the widest Amiga face measured (`fonts/garnet/16`) reaches 20px. This
/// stays a bound rather than growing unboundedly so a corrupt header cannot demand
/// an arbitrarily large per-row allocation.
pub const MAX_ROW_WIDTH: usize = 64;

/// One glyph: its advance width in pixels, and its scanlines.
///
/// [`BitmapFont::height`] rows, each [`Glyph::row_bytes`] bytes — one for a glyph
/// up to 8px wide (bearing included), more for a wider one, up to [`MAX_ROW_WIDTH`].
/// A parser refuses a font needing more rather than silently truncating a glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    /// How far the pen advances. On a proportional font this is the glyph's own
    /// width and not [`BitmapFont::width`].
    pub width: u8,
    /// [`BitmapFont::height`] rows, flat, [`Glyph::row_bytes`] bytes each. Within a
    /// row, column `c` is bit `7 - c % 8` of byte `c / 8` — MSB = leftmost column,
    /// continuing into the next byte past column 7 exactly as a hex dump would read
    /// left to right.
    pub rows: Vec<u8>,
}

impl Glyph {
    /// Bytes per scanline of [`Glyph::rows`], given the font's `height` — 1 for a
    /// glyph up to 8px wide (bearing included), more for a wider one. Every row of
    /// a glyph is the same width, so this is exact division; `height` comes from
    /// [`BitmapFont::height`], since a `Glyph` does not carry its own row count.
    pub fn row_bytes(&self, height: u8) -> usize {
        self.rows.len().checked_div(usize::from(height)).unwrap_or(0)
    }

    /// Whether column `col` of row `row` is ink — see [`row_bit`], which this
    /// forwards to. `row_bytes` is [`Glyph::row_bytes`]'s result, passed in rather
    /// than recomputed so a caller reading many pixels of one glyph pays the
    /// division once.
    pub fn bit(&self, row_bytes: usize, row: usize, col: usize) -> bool {
        row_bit(&self.rows, row_bytes, row, col)
    }
}

/// Read one pixel out of a flat, MSB-leftmost, `row_bytes`-per-row buffer: column
/// `col` of `row` is bit `7 - col % 8` of byte `row * row_bytes + col / 8`.
///
/// Free-standing rather than a [`Glyph`] method alone, because a caller that has
/// already transformed a glyph's rows — [`crate::amiga_font`]'s bearing shift, or a
/// renderer's synthesized bold/italic pass — is reading a buffer that is no longer
/// one `Glyph`'s, and this is the one place that reads MSB-leftmost so a
/// hand-rolled shift elsewhere can't quietly get the direction backwards
/// (SQ-1038's "two bit orders are live in one function" trap). Out-of-range
/// `row`/`col` reads as unset rather than panicking, so a caller need not bound
/// `col` against the glyph's own ink width to stay safe.
pub fn row_bit(rows: &[u8], row_bytes: usize, row: usize, col: usize) -> bool {
    if row_bytes == 0 {
        return false;
    }
    rows.get(row * row_bytes + col / 8).is_some_and(|byte| byte & (0x80 >> (col % 8)) != 0)
}

/// A parsed bitmap font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapFont {
    /// The nominal cell width. On a proportional font this is the widest glyph,
    /// not every glyph's width — see [`Glyph::width`].
    pub width: u8,
    /// Rows per glyph — pass this to [`Glyph::row_bytes`] to read a glyph's rows,
    /// since a `Glyph` does not carry its own row count.
    pub height: u8,
    /// Rows from the top of the cell down to the baseline.
    pub baseline: u8,
    /// How far a glyph is smeared right to embolden it — AmigaDOS `tf_BoldSmear`
    /// (SQ-1009).
    ///
    /// **The advance grows by the same amount**, which is the half that matters
    /// and the half a synthesised bold forgets. The Amiga draws a bold glyph as
    /// itself OR'd with itself shifted right by this many pixels, and moves the pen
    /// that much further so the extra column has somewhere to live. Emboldening
    /// without widening eats the inter-character gap instead: at a fixed 8-wide
    /// cell there is slack to absorb that, and at a real 3-to-8 px proportional
    /// advance there is none, so bold words run together.
    ///
    /// `0` for a format that has no such field, which is every Macintosh `FONT`
    /// resource — there the synthesised smear is all there is.
    pub bold_smear: u8,
    /// Whether the advance width actually varies between glyphs.
    ///
    /// **Measured, not taken from a flag**, because the flag and the truth disagree
    /// in both directions: a Macintosh `FONT` records a font type whose fixed-width
    /// bit is unreliable, and the question a renderer is really asking is whether
    /// laying this font out in a fixed cell will look wrong. It will if this is
    /// true — see SQ-0916, where Arthur's proportional Amiga font read visibly worse
    /// centred in a fixed cell than the public-domain font it would have replaced.
    pub proportional: bool,
    /// The code the first glyph stands for.
    pub lo: u8,
    /// One entry per code from [`BitmapFont::lo`] upward.
    pub glyphs: Vec<Glyph>,
}

impl BitmapFont {
    /// The glyph for `code`, or `None` when the font does not cover it.
    pub fn glyph(&self, code: u8) -> Option<&Glyph> {
        self.glyphs.get(usize::from(code).checked_sub(usize::from(self.lo))?)
    }

    /// Whether the advance width varies across the glyphs that are actually drawn.
    ///
    /// Blank glyphs are excluded: an undefined character carries a zero advance in
    /// both formats, and counting those would call every font proportional.
    pub(crate) fn measure_proportional(glyphs: &[Glyph]) -> bool {
        let mut seen: Option<u8> = None;
        for g in glyphs.iter().filter(|g| g.rows.iter().any(|&r| r != 0)) {
            match seen {
                None => seen = Some(g.width),
                Some(w) if w != g.width => return true,
                Some(_) => {}
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-byte-per-row glyph — everything this module could hold before
    /// SQ-1038 — still reads the same way through the new helpers: `row_bytes` is
    /// 1 and `bit` agrees with the raw `0x80 >> col` a caller used to write by hand.
    #[test]
    fn narrow_glyph_is_one_byte_per_row() {
        // A 3x2 'L'-ish shape: row0 = #.., row1 = ###.
        let g = Glyph { width: 4, rows: vec![0b1000_0000, 0b1110_0000] };
        assert_eq!(g.row_bytes(2), 1);
        assert!(g.bit(1, 0, 0));
        assert!(!g.bit(1, 0, 1));
        assert!(g.bit(1, 1, 0) && g.bit(1, 1, 1) && g.bit(1, 1, 2));
        assert!(!g.bit(1, 1, 3));
    }

    /// A glyph wider than 8px spans multiple bytes per row, MSB-leftmost
    /// continuing across the byte boundary: column 8 is bit 7 of the SECOND byte,
    /// not bit 7 of the first reused.
    #[test]
    fn wide_glyph_spans_multiple_bytes_msb_leftmost_across_the_boundary() {
        // One row, 12px wide: a run of ink from column 6 through column 9 —
        // straddling the byte boundary at column 8 — bytes 0b0000_0011, 0b1100_0000.
        let rows = vec![0b0000_0011u8, 0b1100_0000u8];
        let g = Glyph { width: 13, rows: rows.clone() };
        assert_eq!(g.row_bytes(1), 2, "one row, 2 bytes wide");
        for col in [0u32, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15] {
            assert!(!row_bit(&rows, 2, 0, col as usize), "column {col} should be unset");
        }
        for col in 6u32..=9 {
            assert!(row_bit(&rows, 2, 0, col as usize), "column {col} should be ink");
        }
    }

    /// [`row_bit`] on an empty buffer, or past the glyph's own width, answers
    /// unset rather than panicking — a caller reading a proportional glyph's
    /// advance-wide cell routinely asks about columns past its own ink.
    #[test]
    fn out_of_range_reads_as_unset() {
        assert!(!row_bit(&[], 0, 0, 0));
        let rows = vec![0xFFu8];
        assert!(!row_bit(&rows, 1, 5, 0), "row past the buffer");
        assert!(!row_bit(&rows, 1, 0, 100), "column absurdly out of range");
    }
}
