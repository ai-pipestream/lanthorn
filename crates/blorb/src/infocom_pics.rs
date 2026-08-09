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

/// Size of one Amiga/Mac directory record.
const ENTRY_SIZE: usize = 14;
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
}

impl PicEntry {
    /// Whether this entry carries compressed pixels. When false the entry is a
    /// size-only placeholder that the game fills with a solid rectangle.
    pub fn has_pixels(&self) -> bool {
        self.data != 0 && self.width != 0 && self.height != 0
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
    pub fn rgba(&self) -> Vec<u8> {
        let pal = self.palette.unwrap_or(DEFAULT_PALETTE);
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
    huff: usize,
    part: u8,
}

impl InfocomPics {
    /// Parse an Amiga/Mac `Pic.data`.
    ///
    /// The format carries no signature, so this validates structurally: the
    /// record size must be the Amiga/Mac 14, the directory and the Huffman
    /// tree must fit, and every non-empty entry must point inside the file.
    pub fn parse(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        if data.len() < 16 {
            return Err(PicError::Truncated);
        }
        // Amiga/Mac headers are big-endian throughout; the PC ones are not, and
        // put a different record size at offset 8.
        if usize::from(data[8]) != ENTRY_SIZE {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(be16(&data, 4));
        // `gh.huffOff`, stored as a word that addresses words.
        let huff = usize::from(be16(&data, 2)) * 2;
        let dir_end = 16 + count * ENTRY_SIZE;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        if huff < dir_end || huff >= data.len() {
            return Err(PicError::UnsupportedContainer);
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let o = 16 + i * ENTRY_SIZE;
            let e = PicEntry {
                id: be16(&data, o),
                width: be16(&data, o + 2),
                height: be16(&data, o + 4),
                flags: be16(&data, o + 6),
                data: be24(&data, o + 8),
                palette: be24(&data, o + 11),
            };
            if e.data >= data.len() || e.palette >= data.len() {
                return Err(PicError::Truncated);
            }
            entries.push(e);
        }
        let part = data[0];
        Ok(InfocomPics {
            data,
            entries,
            huff,
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

        let mut indices = unhuff_unrle(self.huff_tree(), payload, mid_size, w * h)?;
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

    fn huff_tree(&self) -> &[u8] {
        let end = (self.huff + HUFF_LEN).min(self.data.len());
        &self.data[self.huff..end]
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

        // Picture 1 is the 320x200 title screen, on a 14-colour palette whose
        // first entry lands at colour index 2.
        let p = pics.decode(1).unwrap();
        assert_eq!((p.width, p.height), (320, 200));
        let pal = p.palette.expect("picture 1 carries a palette");
        assert_eq!(pal[2], [0xcc, 0xcc, 0xcc]);
        assert_eq!(pal[15], [0xee, 0xee, 0xee]);
        assert_eq!(p.rgba().len(), 320 * 200 * 4);
    }
}
