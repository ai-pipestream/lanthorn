//! The shape a bitmap font comes back in, whichever machine's release it came off.
//!
//! Two producers, one type: [`crate::amiga_font`] reads an AmigaDOS disk font off a
//! floppy, [`crate::mac_font`] reads a Macintosh `FONT` resource out of a resource
//! fork. They agree on everything a renderer cares about, and differ only in how
//! they are stored, so the caller does not learn which machine it is drawing.
//!
//! Rows are **MSB-leftmost**, as both formats store them, so a row can be read
//! against a hex dump without mental arithmetic. A consumer wanting the opposite
//! convention flips with [`u8::reverse_bits`].

/// One glyph: its advance width in pixels, and one byte per row.
///
/// Only widths up to 8 are representable, which covers every font measured here —
/// the Amiga's are at most 8 and the Macintosh's are 7. A parser refuses a font
/// needing more rather than silently truncating a glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    /// How far the pen advances. On a proportional font this is the glyph's own
    /// width and not [`BitmapFont::width`].
    pub width: u8,
    /// One byte per row, [`BitmapFont::height`] of them, MSB = leftmost column.
    pub rows: Vec<u8>,
}

/// A parsed bitmap font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapFont {
    /// The nominal cell width. On a proportional font this is the widest glyph,
    /// not every glyph's width — see [`Glyph::width`].
    pub width: u8,
    /// Rows per glyph, and the length of every [`Glyph::rows`].
    pub height: u8,
    /// Rows from the top of the cell down to the baseline.
    pub baseline: u8,
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
