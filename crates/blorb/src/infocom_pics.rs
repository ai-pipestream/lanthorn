//! Reader for Infocom's *native* picture archive — the `Pic.data` file shipped
//! on the Amiga (and Macintosh) releases of the graphical version-6 games.
//!
//! Infocom's own interpreter sources describe the codec (`mac/gfx.c`,
//! `mac/gfx.p`, `amiga/gfx.c`, published at
//! <https://github.com/erkyrath/infocom-zcode-terps>). Quoting `mac/gfx.c`:
//!
//! ```text
//! Steps in picture compression (undo in reverse order):
//!
//! 1.  Each line of the picture is exclusive-or'ed with the previous line.
//!
//! 2.  A run-length encoding is applied, as follows:  byte values 0
//! through 15 represent colors; byte values 16 through 127 are repetition
//! counts (16 will never actually appear)  Thus: 3 occurrences of byte
//! value 2 will turn into 2 17 (subtract 15 from the 17 to find that the
//! two should be repeated 2 MORE times).
//!
//! 3.  Optionally, the whole thing is Huffman-coded, using an encoding
//! specified in the header file.
//! ```
//!
//! The PC-format archives (`.MG1`/`.EG1`/`.CG1`) are a *different*, unrelated
//! container: little-endian, and LZW-compressed rather than Huffman+RLE. This
//! module deliberately reads only the Amiga/Mac flavour and reports
//! [`PicError::UnsupportedContainer`] for the rest.
//!
//! There is no magic number, so a caller must decide by other means that a
//! given file belongs to the story it is about to draw — see the
//! `<story-stem>.pic` naming convention.

/// An 8-bit RGB triple.
pub type Rgb = [u8; 3];

/// Errors that can arise while reading a native Infocom picture archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PicError {
    /// A length or offset ran past the end of the data.
    Truncated,
    /// The header does not describe an Amiga/Mac picture archive (the only
    /// flavour this module decodes).
    UnsupportedContainer,
    /// No picture with that id is in the directory.
    NoSuchPicture,
    /// The entry is a size-only placeholder: dimensions but no pixel data.
    /// Blorb conversions render these as `Rect` resources.
    NoPixelData,
    /// The entry uses a compression variant this module does not decode
    /// (raw un-Huffman'd RLE, 1-bit mono, or embedded IFF).
    UnsupportedCompression(u16),
    /// The compressed stream did not expand to `width * height` pixels.
    BadPixelData,
}

/// Entry flag bits, from `amiga/gfx.c`.
const EF_TRANS: u16 = 1; // picture uses colour 0 as transparent
const EF_PHUFF: u16 = 2; // picture is Huffman-coded
const EF_XOR2: u16 = 4; // picture was XORed on alternate lines
const EF_MONO: u16 = 8; // two-colour picture
const EF_IFF: u16 = 32; // picture is IFFed

/// Header flag bits, from `amiga/gfx.c`. Byte 1 of the file is `gh.flags`.
const HF_EHUFF: u8 = 2; // directory records carry a Huffman-tree pointer
const HF_GHUFF: u8 = 4; // the file header carries one global Huffman-tree pointer

/// Size of one Amiga/Mac directory record without a per-entry Huffman pointer:
/// `picID`, `picX`, `picY`, `eFlags` (2 bytes each) then `dataOff` and `palOff`
/// (3 bytes each). `ReadGFXEntry` reads exactly these, in this order.
const ENTRY_SIZE: usize = 14;
/// A per-entry Huffman-tree pointer adds a word to the record — the 16-byte
/// flavour. See [`InfocomPics::parse`].
const HUFF_PTR_SIZE: usize = 2;
/// Bytes of length prefix in front of each picture's compressed data
/// (`LBYTES` in `amiga/gfx.c`), twice over: `minSize` then `midSize`.
const LBYTES: usize = 3;
/// The flat Huffman tree is a 256-byte array of node/leaf bytes.
const HUFF_LEN: usize = 256;

/// The 16-colour default palette. A picture whose directory entry names no
/// palette inherits whatever palette the interpreter last loaded; when there is
/// nothing to inherit this is the fallback, and it is also what the Blorb
/// conversions of Zork Zero baked into such pictures.
pub const DEFAULT_PALETTE: [Rgb; 16] = [
    [0, 0, 0],
    [0, 0, 170],
    [0, 170, 0],
    [0, 170, 170],
    [170, 0, 0],
    [170, 0, 170],
    [170, 170, 0],
    [170, 170, 170],
    [85, 85, 85],
    [85, 85, 255],
    [85, 255, 85],
    [85, 255, 255],
    [255, 85, 85],
    [255, 85, 255],
    [255, 255, 85],
    [255, 255, 255],
];

/// Colour index at which a picture's own palette starts. Indices 0 and 1 are
/// reserved (0 is the transparent colour when `EF_TRANS` is set); a 14-entry
/// palette therefore fills 2..=15.
const PALETTE_BASE: usize = 2;

/// One directory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PicEntry {
    /// Picture number, as the story's `@draw_picture` refers to it.
    pub id: u16,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Entry flags (`EF_*`).
    pub flags: u16,
    data: usize,
    palette: usize,
    /// Byte offset of the 256-byte Huffman tree this picture decodes through —
    /// the file's one global tree, or the record's own, per the header flags.
    huff: usize,
}

impl PicEntry {
    /// Whether this entry carries compressed pixels. When false the entry is a
    /// size-only placeholder that the game fills with a solid rectangle.
    pub fn has_pixels(&self) -> bool {
        self.data != 0 && self.width != 0 && self.height != 0
    }

    /// Whether this entry names a palette of its own.
    ///
    /// A zero palette offset is the native format's way of saying *adaptive*:
    /// the picture has no colours of its own and must be drawn through whatever
    /// palette the interpreter last loaded. It is exactly the condition Blorb
    /// spells out with a top-level `APal` chunk — for Zork Zero the 172 entries
    /// here that have pixels and no palette are, id for id, the 172 numbers
    /// `Zork0.blb` lists in `APal`.
    pub fn has_own_palette(&self) -> bool {
        self.palette != 0
    }
}

/// A decoded picture: palette indices plus the palette to read them through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// One palette index per pixel, row-major, `width * height` long.
    pub indices: Vec<u8>,
    /// The picture's own palette, if it carries one. `None` means it inherits
    /// the interpreter's current palette.
    pub palette: Option<[Rgb; 16]>,
    /// The colour index drawn as transparent, if any.
    pub transparent: Option<u8>,
}

impl Picture {
    /// Straight RGBA8 expansion, row-major. Falls back to [`DEFAULT_PALETTE`]
    /// when the picture carries no palette of its own; the transparent index,
    /// if any, gets alpha 0.
    ///
    /// A picture with no palette is *adaptive* (see [`PicEntry::has_own_palette`])
    /// and the fallback is only what to show when nothing has been loaded yet —
    /// a caller that has a current palette should pass it to [`rgba_with`].
    ///
    /// [`rgba_with`]: Picture::rgba_with
    pub fn rgba(&self) -> Vec<u8> {
        self.rgba_with(&self.palette.unwrap_or(DEFAULT_PALETTE))
    }

    /// RGBA8 expansion through a caller-supplied colour table — the adaptive
    /// path, where the palette comes from the interpreter rather than the
    /// picture.
    pub fn rgba_with(&self, pal: &[Rgb; 16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 4);
        for &i in &self.indices {
            let c = pal[usize::from(i) & 15];
            let a = if Some(i) == self.transparent { 0 } else { 255 };
            out.extend_from_slice(&[c[0], c[1], c[2], a]);
        }
        out
    }
}

/// A parsed native picture archive: owns the file bytes and the directory.
#[derive(Debug)]
pub struct InfocomPics {
    data: Vec<u8>,
    entries: Vec<PicEntry>,
    part: u8,
}

impl InfocomPics {
    /// Parse an Amiga/Mac `Pic.data`.
    ///
    /// # Header
    ///
    /// `InitGFX` in `amiga/gfx.c` reads the 16-byte header field by field, and
    /// that is where every constant below comes from:
    ///
    /// | offset | field | note |
    /// |---|---|---|
    /// | 0 | `gh.fileID` | byte; "not used on Amiga" |
    /// | 1 | `gh.flags` | byte; the `HF_*` set |
    /// | 2..4 | `gh.huffOff` | word, **doubled** to a byte offset |
    /// | 4..6 | `gh.nPics` | records in this file |
    /// | 6..8 | `gh.ngPics` | records in the global file, if any |
    /// | 8 | `gh.dirEntryLen` | byte; directory record size |
    ///
    /// # Two record sizes, and which one a file uses
    ///
    /// Byte 1 is a **flag set**, not a version number, and it decides the record
    /// size. `ReadGFXEntry` copies 8 bytes of `picID`/`picX`/`picY`/`eFlags`,
    /// then 3-byte `dataOff` and `palOff`, and then:
    ///
    /// ```text
    /// if (BAND (gh.flags, HF_EHUFF+HF_GHUFF) == HF_EHUFF)
    ///     { ge.huffOff = 0; doCopy (pEnt, (UBYTE *)(&ge.huffOff) + 2, 2); ge.huffOff *= 2; }
    /// else    ge.huffOff = gh.huffOff;
    /// ```
    ///
    /// So a file that declares `HF_EHUFF` *without* `HF_GHUFF` gives every
    /// picture its own Huffman tree, named by a further word in the record —
    /// 16 bytes rather than 14. Zork Zero, Journey and Arthur declare `6`
    /// (`HF_EHUFF | HF_GHUFF`) and share one global tree in 14-byte records;
    /// Shogun declares `2` (`HF_EHUFF` alone) and carries 48 trees in 16-byte
    /// records. Both are read here; the record size the header declares must be
    /// the one its flags imply.
    ///
    /// # Validation
    ///
    /// The format carries no signature, so this validates structurally: the
    /// declared record size must match the flags, the directory must fit, ids
    /// must ascend (`ReadGFXEntry` binary-searches the directory, so they have
    /// to), and every offset a record names must land inside the file.
    pub fn parse(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        if data.len() < 16 {
            return Err(PicError::Truncated);
        }
        // Amiga/Mac headers are big-endian throughout; the PC ones are not, and
        // put a record size at offset 8 that no flag combination here implies.
        let per_entry_huff = data[1] & (HF_EHUFF | HF_GHUFF) == HF_EHUFF;
        let entry_size = ENTRY_SIZE + if per_entry_huff { HUFF_PTR_SIZE } else { 0 };
        if usize::from(data[8]) != entry_size {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(be16(&data, 4));
        // `gh.huffOff`, stored as a word that addresses words.
        let global_huff = usize::from(be16(&data, 2)) * 2;
        let dir_end = 16 + count * entry_size;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        // A tree must sit past the directory and inside the file. With one
        // global tree that is a single check; with per-entry trees the header
        // word is unused (zero in Shogun) and each record answers for itself.
        let in_range = |off: usize| (dir_end..data.len()).contains(&off);
        if !per_entry_huff && !in_range(global_huff) {
            return Err(PicError::UnsupportedContainer);
        }

        let mut entries: Vec<PicEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let o = 16 + i * entry_size;
            let e = PicEntry {
                id: be16(&data, o),
                width: be16(&data, o + 2),
                height: be16(&data, o + 4),
                flags: be16(&data, o + 6),
                data: be24(&data, o + 8),
                palette: be24(&data, o + 11),
                huff: if per_entry_huff {
                    usize::from(be16(&data, o + 14)) * 2
                } else {
                    global_huff
                },
            };
            if e.data >= data.len() || e.palette >= data.len() {
                return Err(PicError::Truncated);
            }
            if e.has_pixels() && e.flags & EF_PHUFF != 0 && !in_range(e.huff) {
                return Err(PicError::UnsupportedContainer);
            }
            if entries.last().is_some_and(|p| p.id >= e.id) {
                return Err(PicError::UnsupportedContainer);
            }
            entries.push(e);
        }
        let part = data[0];
        Ok(InfocomPics {
            data,
            entries,
            part,
        })
    }

    /// The archive's part number. Multi-part sets number their files 1, 2, …
    pub fn part(&self) -> u8 {
        self.part
    }

    /// Every directory record, in file order.
    pub fn entries(&self) -> &[PicEntry] {
        &self.entries
    }

    /// The directory record for a picture number.
    pub fn entry(&self, id: u16) -> Option<&PicEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Every picture that must be drawn through the interpreter's current
    /// palette rather than one of its own — this format's `APal` (see
    /// [`PicEntry::has_own_palette`]). Placeholders are excluded: an entry with
    /// no pixels has nothing to colour.
    pub fn adaptive_pictures(&self) -> Vec<u16> {
        self.entries
            .iter()
            .filter(|e| e.has_pixels() && !e.has_own_palette())
            .map(|e| e.id)
            .collect()
    }

    /// A picture's own 16-entry colour table, without decoding its pixels —
    /// what a non-adaptive draw establishes as the current palette. `None` for
    /// an unknown id or an adaptive picture, which has no table of its own.
    pub fn palette_of(&self, id: u16) -> Option<[Rgb; 16]> {
        self.palette(self.entry(id)?)
    }

    /// Decode one picture to palette indices plus its palette.
    pub fn decode(&self, id: u16) -> Result<Picture, PicError> {
        let e = *self.entry(id).ok_or(PicError::NoSuchPicture)?;
        if !e.has_pixels() {
            return Err(PicError::NoPixelData);
        }
        if e.flags & (EF_MONO | EF_IFF) != 0 || e.flags & EF_PHUFF == 0 {
            return Err(PicError::UnsupportedCompression(e.flags));
        }

        let w = usize::from(e.width);
        let h = usize::from(e.height);
        let hdr = e.data + 2 * LBYTES;
        if hdr > self.data.len() {
            return Err(PicError::Truncated);
        }
        let min_size = be24(&self.data, e.data);
        let mid_size = be24(&self.data, e.data + LBYTES);
        let payload = self
            .data
            .get(hdr..hdr + min_size)
            .ok_or(PicError::Truncated)?;

        let mut indices = unhuff_unrle(self.huff_tree(e.huff), payload, mid_size, w * h)?;
        // Undo step 1: each line was XORed with the line above it. Infocom's own
        // decoder zero-fills a virtual row -1, so row 0 comes through unchanged.
        // `EF_XOR2` claims alternate lines, but every known decoder — Infocom's
        // included — XORs every line.
        let _ = EF_XOR2;
        for i in w..w * h {
            indices[i] ^= indices[i - w];
        }

        Ok(Picture {
            width: e.width,
            height: e.height,
            indices,
            palette: self.palette(&e),
            transparent: (e.flags & EF_TRANS != 0).then_some(0),
        })
    }

    /// The 256-byte Huffman tree at `off` — the file's global one, or the one a
    /// 16-byte record names for itself.
    fn huff_tree(&self, off: usize) -> &[u8] {
        let end = (off + HUFF_LEN).min(self.data.len());
        &self.data[off..end]
    }

    /// Build the 16-entry colour table for an entry. The stored RGB triples
    /// occupy indices [`PALETTE_BASE`]`..`; anything they do not cover keeps the
    /// default. The stipple-id table that follows the triples (`gp.stips`, a
    /// pattern per colour for 1-bit screens) is not used for colour rendering.
    fn palette(&self, e: &PicEntry) -> Option<[Rgb; 16]> {
        if e.palette == 0 {
            return None;
        }
        let count = usize::from(*self.data.get(e.palette)?);
        let rgb = self.data.get(e.palette + 1..e.palette + 1 + count * 3)?;
        let mut pal = DEFAULT_PALETTE;
        for (i, c) in rgb.chunks_exact(3).enumerate() {
            match pal.get_mut(PALETTE_BASE + i) {
                Some(slot) => *slot = [c[0], c[1], c[2]],
                None => break,
            }
        }
        Some(pal)
    }
}

/// Undo steps 3 and 2: walk the flat Huffman tree for `symbols` symbols,
/// expanding run-length symbols as they come out.
///
/// The tree is a byte array of node pairs. At node `n` (always even) the child
/// for a clear bit is `tree[n]` and for a set bit `tree[n + 1]`. A value below
/// 128 is an internal node id — double it for the array index. A value of 128
/// or more is a leaf carrying symbol `value - 128`. Symbols 0..=15 are colour
/// indices; 16..=127 repeat the last colour `symbol - 15` further times. Bits
/// are read most-significant first.
fn unhuff_unrle(
    tree: &[u8],
    payload: &[u8],
    symbols: usize,
    expect: usize,
) -> Result<Vec<u8>, PicError> {
    let mut out = Vec::with_capacity(expect);
    let mut node = 0usize;
    let mut last = 0u8;
    let mut left = symbols;
    let mut bits = payload.iter().flat_map(|b| (0..8).map(move |i| b >> (7 - i) & 1));
    while left > 0 {
        let bit = bits.next().ok_or(PicError::BadPixelData)?;
        let child = *tree
            .get(node + usize::from(bit))
            .ok_or(PicError::BadPixelData)?;
        if child < 128 {
            node = usize::from(child) * 2;
            continue;
        }
        let sym = child - 128;
        if sym < 16 {
            out.push(sym);
            last = sym;
        } else {
            if out.is_empty() {
                return Err(PicError::BadPixelData);
            }
            out.extend(std::iter::repeat_n(last, usize::from(sym) - 15));
        }
        if out.len() > expect {
            return Err(PicError::BadPixelData);
        }
        left -= 1;
        node = 0;
    }
    if out.len() != expect {
        return Err(PicError::BadPixelData);
    }
    Ok(out)
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from(b[o]) << 8 | u16::from(b[o + 1])
}

fn be24(b: &[u8], o: usize) -> usize {
    usize::from(b[o]) << 16 | usize::from(b[o + 1]) << 8 | usize::from(b[o + 2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A hand-built one-picture archive: 4x2, rows `2222` then `3333`.
    ///
    /// Pre-XOR the rows are `2222` and `1111`, which run-length encodes to the
    /// four symbols `[2, 18, 1, 18]` (18 = repeat 3 more). Huffman codes:
    /// `0` -> 2, `10` -> 1, `11` -> 18, so the bit stream is `0 11 10 11`,
    /// padded to `0b0111_0110`.
    fn synthetic() -> Vec<u8> {
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[2] = 0x00; // huffOff, in words: 0x0F * 2 = 30 = end of directory
        f[3] = 0x0F;
        f[5] = 1; // one picture
        f[8] = ENTRY_SIZE as u8;
        let data_off = 16 + ENTRY_SIZE + HUFF_LEN;
        f.extend_from_slice(&[
            0, 7, // id 7
            0, 4, // width
            0, 2, // height
            0, (EF_TRANS | EF_PHUFF) as u8,
            (data_off >> 16) as u8,
            (data_off >> 8) as u8,
            data_off as u8,
            0,
            0,
            0, // no palette
        ]);
        let mut tree = vec![0u8; HUFF_LEN];
        tree[0] = 128 + 2; // bit 0 -> leaf, symbol 2
        tree[1] = 1; // bit 1 -> internal node 1, i.e. index 2
        tree[2] = 128 + 1; // leaf, symbol 1
        tree[3] = 128 + 18; // leaf, symbol 18 (repeat 3 more)
        f.extend_from_slice(&tree);
        f.extend_from_slice(&[0, 0, 1]); // minSize
        f.extend_from_slice(&[0, 0, 4]); // midSize: four symbols
        f.push(0b0111_0110);
        f
    }

    /// A hand-built TWO-picture archive in the 16-byte flavour: header flags
    /// `HF_EHUFF` alone, so every record names its own Huffman tree and the
    /// header's global tree word is unused (zero, as it is in Shogun).
    ///
    /// The two trees deliberately decode the *same* bit patterns to different
    /// symbols, so reading either picture through the other's tree produces
    /// different pixels. Picture 7 is `synthetic`'s: tree A codes `0` -> 2,
    /// `10` -> 1, `11` -> 18, bit stream `0b0111_0110`. Picture 9 uses tree B —
    /// `1` -> 4, `00` -> 6, `01` -> 18 — over `[4, 18, 6, 18]`, bit stream
    /// `1 01 00 01` padded to `0b1010_0010`, which un-XORs to rows `4444`,
    /// `2222`.
    fn synthetic_per_entry_huff() -> Vec<u8> {
        const E: usize = ENTRY_SIZE + HUFF_PTR_SIZE;
        let mut f = vec![0u8; 16];
        f[0] = 1;
        f[1] = HF_EHUFF; // no HF_GHUFF: the trees live in the records
        f[5] = 2; // two pictures
        f[8] = E as u8;
        let dir_end = 16 + 2 * E;
        let (tree_a, tree_b) = (dir_end, dir_end + HUFF_LEN);
        let data_a = tree_b + HUFF_LEN;
        let data_b = data_a + 2 * LBYTES + 1;
        let mut record = |id: u16, data: usize, huff: usize| {
            f.extend_from_slice(&id.to_be_bytes());
            f.extend_from_slice(&[0, 4, 0, 2, 0, (EF_TRANS | EF_PHUFF) as u8]);
            f.extend_from_slice(&[(data >> 16) as u8, (data >> 8) as u8, data as u8]);
            f.extend_from_slice(&[0, 0, 0]); // no palette
            f.extend_from_slice(&u16::try_from(huff / 2).unwrap().to_be_bytes());
        };
        record(7, data_a, tree_a);
        record(9, data_b, tree_b);

        let mut a = vec![0u8; HUFF_LEN];
        a[0] = 128 + 2; // `0`  -> symbol 2
        a[1] = 1; // `1`  -> node 1
        a[2] = 128 + 1; // `10` -> symbol 1
        a[3] = 128 + 18; // `11` -> repeat 3 more
        let mut b = vec![0u8; HUFF_LEN];
        b[0] = 1; // `0`  -> node 1
        b[1] = 128 + 4; // `1`  -> symbol 4
        b[2] = 128 + 6; // `00` -> symbol 6
        b[3] = 128 + 18; // `01` -> repeat 3 more
        f.extend_from_slice(&a);
        f.extend_from_slice(&b);
        f.extend_from_slice(&[0, 0, 1, 0, 0, 4, 0b0111_0110]);
        f.extend_from_slice(&[0, 0, 1, 0, 0, 4, 0b1010_0010]);
        f
    }

    /// SQ-0744. Shogun's Amiga archive declares `HF_EHUFF` without `HF_GHUFF`,
    /// which by `ReadGFXEntry` gives every picture its own Huffman tree in a
    /// 16-byte record. Both record sizes must read, and each picture must go
    /// through *its own* tree — decoding picture 9 with picture 7's would give
    /// `[2, 2, 2, 2, 3, 3, 3, 3]` here.
    #[test]
    fn decodes_a_sixteen_byte_archive_through_per_entry_huffman_trees() {
        let pics = InfocomPics::parse(synthetic_per_entry_huff()).unwrap();
        assert_eq!(pics.entries().len(), 2);

        let p = pics.decode(7).unwrap();
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        let q = pics.decode(9).unwrap();
        assert_eq!(q.indices, vec![4, 4, 4, 4, 2, 2, 2, 2]);
    }

    /// The record size a file declares must be the one its header flags imply:
    /// 16 bytes exactly when `HF_EHUFF` stands alone, 14 otherwise. Neither
    /// size is accepted on its own say-so, which is what keeps a PC `.MG1` (a
    /// little-endian LZW container with its own byte at offset 8) out.
    #[test]
    fn the_record_size_must_match_the_header_flags() {
        let mut f = synthetic_per_entry_huff();
        f[8] = ENTRY_SIZE as u8;
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));

        let mut f = synthetic();
        f[8] = (ENTRY_SIZE + HUFF_PTR_SIZE) as u8;
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));

        // `HF_EHUFF | HF_GHUFF` — Zork Zero's 6 — is the global-tree flavour and
        // stays 14 bytes, so a header that sets both must not grow its records.
        let mut f = synthetic();
        f[1] = HF_EHUFF | HF_GHUFF;
        assert!(InfocomPics::parse(f).is_ok());
    }

    /// `ReadGFXEntry` binary-searches the directory, so a real archive's ids
    /// ascend. A container whose "ids" wander is not one of these files.
    #[test]
    fn rejects_a_directory_whose_ids_do_not_ascend() {
        let mut f = synthetic_per_entry_huff();
        f[16] = 0;
        f[17] = 9; // both records now claim id 9
        assert_eq!(InfocomPics::parse(f).err(), Some(PicError::UnsupportedContainer));
    }

    #[test]
    fn decodes_a_synthetic_picture() {
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 1);
        let p = pics.decode(7).unwrap();
        assert_eq!((p.width, p.height), (4, 2));
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        assert_eq!(p.transparent, Some(0));
        assert!(p.palette.is_none());
        // No palette of its own, so `rgba` reads through the default table.
        let c = DEFAULT_PALETTE[2];
        assert_eq!(&p.rgba()[..4], &[c[0], c[1], c[2], 255]);
    }

    #[test]
    fn a_zero_palette_offset_marks_a_picture_adaptive() {
        // The synthetic archive's one picture names no palette, so it is
        // adaptive: it has no colours of its own and must be drawn through
        // whatever the caller supplies.
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert_eq!(pics.adaptive_pictures(), vec![7]);
        assert_eq!(pics.palette_of(7), None, "an adaptive picture has no palette to hand out");

        let p = pics.decode(7).unwrap();
        let mut current = DEFAULT_PALETTE;
        current[2] = [1, 2, 3];
        assert_eq!(&p.rgba_with(&current)[..4], &[1, 2, 3, 255], "drawn through the supplied palette");
        assert_eq!(&p.rgba()[..4], &[0, 170, 0, 255], "and through the fallback without one");

        // A picture that DOES name a palette is not adaptive, and answers with
        // it — the per-picture distinction the format makes.
        let mut f = synthetic();
        let pal_off = f.len();
        f[16 + 11] = (pal_off >> 16) as u8;
        f[16 + 12] = (pal_off >> 8) as u8;
        f[16 + 13] = pal_off as u8;
        f.extend_from_slice(&[1, 9, 8, 7]); // one colour, landing at index 2
        let pics = InfocomPics::parse(f).unwrap();
        assert!(pics.entry(7).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());
        assert_eq!(pics.palette_of(7).unwrap()[2], [9, 8, 7]);
    }

    #[test]
    fn a_placeholder_is_not_adaptive() {
        // No pixels means nothing to colour: a size-only entry is a rectangle
        // the game fills, not an adaptive picture.
        let mut f = synthetic();
        f[16 + 8] = 0;
        f[16 + 9] = 0;
        f[16 + 10] = 0;
        let pics = InfocomPics::parse(f).unwrap();
        assert!(!pics.entry(7).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());
    }

    #[test]
    fn rejects_non_amiga_containers() {
        let mut f = synthetic();
        f[8] = 8; // a record size this module does not read
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );
        assert_eq!(
            InfocomPics::parse(vec![0; 4]).err(),
            Some(PicError::Truncated)
        );
    }

    #[test]
    fn reports_missing_and_placeholder_pictures() {
        let pics = InfocomPics::parse(synthetic()).unwrap();
        assert_eq!(pics.decode(99), Err(PicError::NoSuchPicture));

        let mut f = synthetic();
        f[16 + 8] = 0; // clear the data offset: a size-only placeholder
        f[16 + 9] = 0;
        f[16 + 10] = 0;
        let pics = InfocomPics::parse(f).unwrap();
        assert!(!pics.entry(7).unwrap().has_pixels());
        assert_eq!(pics.decode(7), Err(PicError::NoPixelData));
    }

    fn zork0_pic() -> Option<Vec<u8>> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork0.pic");
        std::fs::read(p).ok()
    }

    /// Real-media smoke: the Amiga Zork Zero archive. `stories/` is gitignored,
    /// so this skips vacuously when the fixture is absent.
    #[test]
    fn reads_amiga_zork_zero() {
        let Some(bytes) = zork0_pic() else { return };
        let pics = InfocomPics::parse(bytes).unwrap();
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 495);

        let with_pixels: Vec<_> = pics
            .entries()
            .iter()
            .filter(|e| e.has_pixels())
            .copied()
            .collect();
        assert_eq!(with_pixels.len(), 388);
        // Everything else is a size-only placeholder, matching the 107 `Rect`
        // resources in the Blorb conversion exactly.
        assert_eq!(pics.entries().len() - with_pixels.len(), 107);

        // Every picture with pixels must decode to exactly its stated size, and
        // to exactly the bytes the decoder was validated against: 383 of these
        // 388 were compared pixel-for-pixel with the PNGs in the Blorb
        // conversion `Zork0.blb` and matched byte-for-byte. (The other five —
        // ids 5, 6, 7, 8 and 33 — are the ones whose two sources disagree on
        // dimensions; the Blorb holds a cropped or substituted version.) This
        // FNV-1a hash over the decoded indices of all 388, in file order, pins
        // that result without needing a PNG decoder here.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for e in &with_pixels {
            let p = pics.decode(e.id).unwrap();
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            assert!(p.indices.iter().all(|&i| i < 16));
            for &i in &p.indices {
                hash = (hash ^ u64::from(i)).wrapping_mul(0x100_0000_01b3);
            }
        }
        assert_eq!(hash, 0xde67_de1e_0b55_f2fb);

        // 172 of those 388 name no palette at all and are therefore adaptive —
        // among them the 16 compass overlays, ids 9..=24. That set is, id for
        // id, the 172 numbers `Zork0.blb` lists in its `APal` chunk.
        let adaptive = pics.adaptive_pictures();
        assert_eq!(adaptive.len(), 172);
        assert!((9..=24).all(|id| adaptive.contains(&id)), "the compass overlays are adaptive");
        assert_eq!(pics.palette_of(10), None, "an adaptive picture has no palette of its own");

        // Picture 1 is the 320x200 title screen, on a 14-colour palette whose
        // first entry lands at colour index 2.
        let p = pics.decode(1).unwrap();
        assert_eq!((p.width, p.height), (320, 200));
        let pal = p.palette.expect("picture 1 carries a palette");
        assert_eq!(pal[2], [0xcc, 0xcc, 0xcc]);
        assert_eq!(pal[15], [0xee, 0xee, 0xee]);
        assert_eq!(p.rgba().len(), 320 * 200 * 4);
    }

    /// The picture archive off an Amiga release floppy, or `None` (with a SKIP
    /// note) when the gitignored disk image is not there.
    fn adf_pictures(image: &str) -> Option<InfocomPics> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(image);
        let Ok(bytes) = std::fs::read(&p) else {
            eprintln!("SKIP: gitignored disk image missing at {}", p.display());
            return None;
        };
        let adf = crate::adf::Adf::mount(bytes).expect("an Amiga release floppy mounts");
        Some(adf.pictures().expect("the floppy carries a picture archive").1)
    }

    /// FNV-1a over every decoded picture's indices *and* its resolved palette,
    /// in directory order — one number that moves if any pixel or any colour of
    /// an archive changes.
    fn fingerprint(pics: &InfocomPics) -> (usize, usize, u64) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut n = 0;
        let mut feed = |h: &mut u64, b: u8| *h = (*h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        for e in pics.entries() {
            if !e.has_pixels() {
                continue;
            }
            n += 1;
            let p = pics.decode(e.id).expect("a picture with pixels decodes");
            assert_eq!(
                p.indices.len(),
                usize::from(e.width) * usize::from(e.height),
                "picture {} decoded to the wrong size",
                e.id
            );
            for &i in &p.indices {
                feed(&mut h, i);
            }
            for c in p.palette.unwrap_or(DEFAULT_PALETTE) {
                for b in c {
                    feed(&mut h, b);
                }
            }
        }
        (pics.entries().len(), n, h)
    }

    /// SQ-0744, real media: Shogun's Amiga floppy, the 16-byte flavour.
    ///
    /// Before this reader learned the flavour, `parse` rejected the whole file
    /// on its record size and Shogun booted with no artwork at all — no title
    /// screen. `stories/` is gitignored, so this skips vacuously.
    #[test]
    fn reads_amiga_shogun() {
        let Some(pics) = adf_pictures("James Clavell's Shogun.adf") else { return };
        assert_eq!(pics.entries().len(), 48);

        // 42 records carry pixels; the other 6 are size-only placeholders, and
        // they are id for id the 6 `Rect` resources in `Shogun.blb` — the same
        // 1:1 correspondence SQ-0713 measured for Zork Zero's 107.
        let placeholders: Vec<u16> =
            pics.entries().iter().filter(|e| !e.has_pixels()).map(|e| e.id).collect();
        assert_eq!(placeholders, vec![2, 45, 46, 47, 48, 49]);

        // Every one decodes, and to the bytes the Blorb oracle validated:
        // 34 of the 39 pictures `Shogun.blb` also holds are byte-exact
        // (`crates/app/tests/v6_shogun_native_archive.rs` runs that comparison).
        assert_eq!(fingerprint(&pics), (48, 42, 0x85ce_26b1_aa1f_20db));

        // SQ-0743's correspondence, checked on this flavour: every record with
        // pixels names a palette of its own, so nothing here is adaptive — and
        // `Shogun.blb` carries no `APal` chunk at all. The two agree, as they do
        // for Zork Zero's 172.
        assert!(pics.adaptive_pictures().is_empty());

        // Picture 1 is the title screen the report is about.
        let p = pics.decode(1).unwrap();
        assert_eq!((p.width, p.height), (320, 200));
        assert_eq!(p.palette.expect("the title screen carries a palette")[2], [0x00, 0x00, 0xff]);
    }

    /// The 14-byte flavour must not move. Zork Zero, Journey and Arthur declare
    /// `HF_EHUFF | HF_GHUFF` and share one global Huffman tree; these
    /// fingerprints were taken before this quest touched `parse` and are
    /// byte-identical after it.
    #[test]
    fn the_global_huffman_tree_archives_are_unmoved() {
        for (image, want) in [
            ("Zork Zero - The Revenge of Megaboz.adf", (495, 388, 0x6c01_a84b_8143_80bfu64)),
            ("Journey - The Quest Begins.adf", (134, 134, 0x3c7e_9a34_6ab8_41f2)),
            ("Arthur - The Quest for Excalibur.adf", (169, 135, 0x4f95_fb95_3640_f18f)),
        ] {
            let Some(pics) = adf_pictures(image) else { continue };
            assert_eq!(fingerprint(&pics), want, "{image} decoded differently");
        }
    }
}
