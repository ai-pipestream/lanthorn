//! Reader for Infocom's *native* picture archives — the `Pic.data` file shipped
//! on the Amiga and Macintosh releases of the graphical version-6 games, and the
//! `.MG1`/`.EG1`/`.EG2`/`.CG1` files shipped on the PC releases of the same
//! games.
//!
//! # One container, two codecs
//!
//! Every flavour shares a 16-byte header and a directory of fixed-size records;
//! Infocom's own interpreter sources spell that layout out field by field
//! (`amiga/gfx.c`, `mac/gfx.p` and `apple/yzip/rel.15/zip.equ`, published at
//! <https://github.com/erkyrath/infocom-zcode-terps>). What differs is byte
//! order — each platform wrote its own — and how the pixels are packed.
//!
//! **Amiga and Macintosh** are big-endian and Huffman-coded. `mac/gfx.c` opens
//! with a prose specification of that codec:
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
//! **PC** archives are little-endian and carry no Huffman tree at all. Each
//! picture's data offset points at a bare GIF-style LZW stream — minimum code
//! size 8, so clear is 256 and end-of-stream 257, codes packed least-significant
//! bit first and growing from 9 to 12 bits — that expands straight to finished
//! pixels. There is **no** run-length stage and **no** per-line XOR: the LZW
//! output *is* the picture. See [`InfocomPics::parse`] for how the two are told
//! apart, and [`Flavour`] for what the published sources do and do not settle.
//!
//! There is no magic number, so a caller must decide by other means that a
//! given file belongs to the story it is about to draw — see the
//! `<story-stem>.pic` naming convention.

/// An 8-bit RGB triple.
pub type Rgb = [u8; 3];

/// Which platform wrote an archive, and therefore how to read it.
///
/// # What the published sources settle, and what they do not
///
/// `github.com/erkyrath/infocom-zcode-terps` carries graphics code for the
/// Amiga (`amiga/gfx.c`), the Macintosh (`mac/gfx.c`, `mac/gfx.p`) and the
/// Apple II (`apple/yzip/rel.15/zip.equ`, `pic.asm`) — and **no PC/DOS
/// interpreter at all**. So the *container* below is authoritative: those three
/// agree field for field on the 16-byte header and on the directory record, and
/// the PC files match it exactly once read little-endian.
///
/// The PC *pixel* codec is not in that repository. Two other things stand in
/// for it, and they agree.
///
/// The first is a second implementation: Stefan Jokisch's `src/dos/bcpic.c` in
/// Frotz, the DOS front end that draws these very files, whose comments spell
/// the codec out. Every LZW constant here is quoted from it below.
///
/// The second is an oracle, because a second implementation is still not the
/// format. Zork Zero's MCGA archive and its Amiga `Pic.data` hold the same
/// artwork on the same 12-bit palettes, and for all 383 pictures whose two
/// directories agree on dimensions the LZW output is **byte-for-byte identical**
/// to what the (source-verified) Huffman path produces. Two unrelated codecs
/// cannot agree on 383 images by accident, and that is also what proves there is
/// no XOR stage on the PC side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// Amiga and Macintosh `Pic.data`: big-endian, Huffman + RLE + per-line XOR.
    AmigaMac,
    /// The PC archives — `.MG1` (MCGA), `.EG1`/`.EG2` (EGA), `.CG1` (CGA):
    /// little-endian, GIF-style LZW, no XOR.
    Pc,
}

/// Errors that can arise while reading a native Infocom picture archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PicError {
    /// A length or offset ran past the end of the data.
    Truncated,
    /// The header does not describe a picture archive this module reads.
    UnsupportedContainer,
    /// No picture with that id is in the directory.
    NoSuchPicture,
    /// The entry is a size-only placeholder: dimensions but no pixel data.
    /// Blorb conversions render these as `Rect` resources.
    NoPixelData,
    /// The entry uses a compression variant this module does not decode: on the
    /// Amiga/Mac side raw un-Huffman'd RLE or 1-bit mono, and on either side an
    /// embedded IFF (`EF_IFF`).
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

/// The PC record with no palette pointer: `picID`, `picX`, `picY`, `eFlags`,
/// `dataOff` (3 bytes) and one pad byte. `mac/gfx.p`'s `ReadGFXEntry` is where
/// that pad byte is written down — a record with no palette offset
/// "skip[s] over /single/ pad byte" instead. EGA and CGA archives use it: the
/// hardware fixes their colours, so there is nothing to store.
const PC_ENTRY_SIZE: usize = 12;
/// The PC record with a palette pointer, laid out exactly like the Amiga's 14.
/// MCGA archives use it — MCGA could pick its 16 colours freely, so it had to
/// carry them.
const PC_ENTRY_SIZE_PAL: usize = ENTRY_SIZE;

/// GIF-style LZW, as the PC archives use it. Minimum code size 8, so the two
/// reserved codes land at 256 and 257 and the first assignable code at 258;
/// codes are packed least-significant bit first and widen from 9 bits to 12.
const LZW_MIN_CODE_WIDTH: u32 = 9;
const LZW_CLEAR: u16 = 256;
const LZW_END: u16 = 257;
const LZW_FIRST_CODE: u16 = 258;
const LZW_MAX_CODE_WIDTH: u32 = 12;
const LZW_MAX_CODES: usize = 1 << LZW_MAX_CODE_WIDTH;
/// Bytes of length prefix in front of each picture's compressed data
/// (`LBYTES` in `amiga/gfx.c`), twice over: `minSize` then `midSize`.
const LBYTES: usize = 3;
/// The flat Huffman tree is a 256-byte array of node/leaf bytes.
const HUFF_LEN: usize = 256;

/// The 16-colour default palette. A picture whose directory entry names no
/// palette inherits whatever palette the interpreter last loaded; when there is
/// nothing to inherit this is the fallback, and it is also what the Blorb
/// conversions of Zork Zero baked into such pictures.
///
/// It is *not* claimed to be the IBM EGA or CGA hardware table, and a PC EGA or
/// CGA archive — which stores no palettes of its own — should be drawn through
/// one the caller supplies rather than through this.
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
    ///
    /// A PC **EGA or CGA** archive stores no palettes at all — its records are
    /// 12 bytes with no room for one, because those adapters fix their colours
    /// in hardware. Every picture in such a file therefore reads as adaptive,
    /// and a caller must supply the hardware table. Only MCGA, which could
    /// choose its sixteen colours freely, carries palettes on the PC side.
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
    flavour: Flavour,
}

impl InfocomPics {
    /// Parse a native picture archive of either flavour.
    ///
    /// # Telling the two apart
    ///
    /// Neither flavour carries a signature, and the two headers occupy the same
    /// sixteen bytes, so the discriminator has to come from the format's own
    /// semantics. It does: **a Huffman-coded archive must name a Huffman tree**,
    /// and `ReadGFXEntry` in `amiga/gfx.c` says where:
    ///
    /// ```text
    /// if (BAND (gh.flags, HF_EHUFF+HF_GHUFF) == HF_EHUFF)
    ///     { ge.huffOff = 0; doCopy (pEnt, (UBYTE *)(&ge.huffOff) + 2, 2); ge.huffOff *= 2; }
    /// else    ge.huffOff = gh.huffOff;
    /// ```
    ///
    /// Either `HF_EHUFF` stands alone and every record names its own tree, or
    /// the header's word at offset 2 names the file's one global tree. An
    /// archive that does neither cannot be running the Huffman codec — and that
    /// is what every PC archive looks like: `HF_EHUFF` clear and a tree word of
    /// zero, which reads zero in either byte order, so the test does not have to
    /// know the byte order before it can pick one.
    ///
    /// Two header fields that look like discriminators are **not** used, because
    /// measurement says they do not separate the flavours:
    ///
    /// * the record size at offset 8 — MCGA archives use 14, the same as the
    ///   Amiga, and only EGA/CGA use 12;
    /// * the flag byte at offset 1. It is the picture-space width, not a
    ///   platform: Frotz's `ux_pic.c` reads it as
    ///   `x_scale = (flags & 0x08) ? 640 : 320`, which is why CGA and EGA
    ///   archives read `0x38` and MCGA ones `0x30`.
    ///
    /// Frotz never has to make this decision at all — `bcpic.c` builds the
    /// filename from the display mode (`extension[1] = "cmem"[display - 2]`) and
    /// so always knows what it opened — and it labels the tree word at offset 2
    /// `unused1`, which it is for a file that has no tree. Infocom's own sources
    /// are what give that word a meaning, and having one is exactly what being
    /// the other flavour consists of.
    ///
    /// # Header
    ///
    /// `InitGFX` in `amiga/gfx.c` reads the 16-byte header field by field, and
    /// that is where every constant below comes from. `mac/gfx.p`'s
    /// `gfx_Header` record and the Apple II's `zip.equ` (`PHFID`, `PHFLG`,
    /// `PHHUFF`, `PHNLD`, `PHNGD`, `PHDSIZE`, `PHSIZE = 16`) name the same
    /// fields at the same offsets, which is what makes the layout common to
    /// every flavour rather than an Amiga peculiarity:
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
    /// # Record sizes
    ///
    /// `mac/gfx.p` calls `entryLen` "length of each entry (12-14-16)" and
    /// `ReadGFXEntry` shows how each length is spent. All three start with the
    /// same 8 bytes of `picID`/`picX`/`picY`/`eFlags` and a 3-byte `dataOff`.
    /// Then:
    ///
    /// * **12** — no palette pointer, just a pad byte ("skip over /single/ pad
    ///   byte"). The EGA and CGA archives, whose colours the hardware fixes.
    /// * **14** — a 3-byte `palOff`. The Amiga/Mac archives, and the MCGA ones.
    /// * **16** — `palOff` plus a per-entry Huffman-tree word, which
    ///   `ReadGFXEntry` reads only when the header declares `HF_EHUFF` without
    ///   `HF_GHUFF`:
    ///
    /// ```text
    /// if (BAND (gh.flags, HF_EHUFF+HF_GHUFF) == HF_EHUFF)
    ///     { ge.huffOff = 0; doCopy (pEnt, (UBYTE *)(&ge.huffOff) + 2, 2); ge.huffOff *= 2; }
    /// else    ge.huffOff = gh.huffOff;
    /// ```
    ///
    /// Zork Zero, Journey and Arthur declare `6` (`HF_EHUFF | HF_GHUFF`) on the
    /// Amiga and share one global tree in 14-byte records; Shogun declares `2`
    /// (`HF_EHUFF` alone) and carries 48 trees in 16-byte records. On the
    /// Amiga/Mac side the record size the header declares must therefore be the
    /// one its flags imply; on the PC side, where no tree exists, 12 and 14 are
    /// both accepted and the size alone says whether palettes are stored.
    ///
    /// # Validation
    ///
    /// The format carries no signature, so this validates structurally: the
    /// declared record size must be one the flavour allows, the directory must
    /// fit, ids must ascend (`ReadGFXEntry` binary-searches the directory, so
    /// they have to), and every offset a record names must land inside the file
    /// and past the directory.
    pub fn parse(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        if data.len() < 16 {
            return Err(PicError::Truncated);
        }
        let per_entry_huff = data[1] & (HF_EHUFF | HF_GHUFF) == HF_EHUFF;
        // No tree named anywhere means the Huffman codec cannot be in play, and
        // a zero word reads zero whichever end it is written from — so this one
        // test picks the flavour without first having to know the byte order.
        if !per_entry_huff && be16(&data, 2) == 0 {
            return Self::parse_pc(data);
        }
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
            flavour: Flavour::AmigaMac,
        })
    }

    /// Parse a PC archive: `.MG1`, `.EG1`, `.EG2` or `.CG1`.
    ///
    /// Same header, same directory, read the other way round — with one twist
    /// that is not a matter of taste. The 16-bit fields are little-endian, as
    /// x86 wrote them, but the **3-byte offsets stay most-significant byte
    /// first**. The Apple II is the source that settles that, because it is the
    /// other little-endian platform Infocom shipped this format on, and
    /// `pic.asm` reads its pointer high byte first:
    ///
    /// ```text
    /// lda   PICINFO+PLDPTR     ; MSB of offset
    /// sta   PFSEEK+SM_FPOS+2   ; MSB of seek
    /// lda   PICINFO+PLDPTR+1   ; Middle
    /// ```
    ///
    /// It is also self-evident in the files: every archive's first data or
    /// palette offset, read that way, lands exactly on the byte after the
    /// directory.
    fn parse_pc(data: Vec<u8>) -> Result<InfocomPics, PicError> {
        let entry_size = usize::from(data[8]);
        if entry_size != PC_ENTRY_SIZE && entry_size != PC_ENTRY_SIZE_PAL {
            return Err(PicError::UnsupportedContainer);
        }
        let count = usize::from(le16(&data, 4));
        let dir_end = 16 + count * entry_size;
        if count == 0 || dir_end > data.len() {
            return Err(PicError::UnsupportedContainer);
        }
        let in_range = |off: usize| off == 0 || (dir_end..data.len()).contains(&off);

        let mut entries: Vec<PicEntry> = Vec::with_capacity(count);
        for i in 0..count {
            let o = 16 + i * entry_size;
            let e = PicEntry {
                id: le16(&data, o),
                width: le16(&data, o + 2),
                height: le16(&data, o + 4),
                flags: le16(&data, o + 6),
                data: be24(&data, o + 8),
                // A 12-byte record spends its last byte on padding, not on a
                // palette pointer.
                palette: if entry_size == PC_ENTRY_SIZE_PAL {
                    be24(&data, o + 11)
                } else {
                    0
                },
                huff: 0,
            };
            if !in_range(e.data) || !in_range(e.palette) {
                return Err(PicError::UnsupportedContainer);
            }
            // Nothing here can be Huffman-coded — the file named no tree, which
            // is how it got routed to this parser in the first place.
            if e.flags & EF_PHUFF != 0 {
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
            flavour: Flavour::Pc,
        })
    }

    /// The archive's part number. Multi-part sets number their files 1, 2, …
    ///
    /// Arthur and Journey split their EGA artwork in two: `.EG1` reads 1 and
    /// `.EG2` reads 2, and a complete EGA set for either game means both.
    pub fn part(&self) -> u8 {
        self.part
    }

    /// Which platform wrote this archive, and therefore which codec read it.
    pub fn flavour(&self) -> Flavour {
        self.flavour
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
        let indices = match self.flavour {
            Flavour::AmigaMac => self.decode_huffed(&e)?,
            Flavour::Pc => self.decode_lzw(&e)?,
        };
        Ok(Picture {
            width: e.width,
            height: e.height,
            indices,
            palette: self.palette(&e),
            // `EF_TRANS` says a colour drops out; which colour is the flag
            // word's top nibble. Both witnesses agree. The Apple II's `pic.asm`
            // uses `$FF` for "none" and otherwise shifts the flag right four
            // bits into `TRANSCLR`; Frotz's `bcpic.c` does the same thing in C —
            // "Bit 0 of "flags" indicates that the picture uses a transparent
            // colour, the top four bits tell us which colour it is", i.e.
            // `transparent = pic_flags >> 12`. Amiga/Mac archives leave that
            // nibble zero throughout, which is the same thing `amiga/gfx.c`
            // describes in prose ("picture uses color 0"); the PC EGA archives
            // really do use it, naming colours 1, 2, 3, 7, 8 and 9.
            transparent: (e.flags & EF_TRANS != 0).then_some((e.flags >> 12) as u8),
        })
    }

    /// Amiga/Mac: Huffman, then run-length, then undo the per-line XOR.
    fn decode_huffed(&self, e: &PicEntry) -> Result<Vec<u8>, PicError> {
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
        Ok(indices)
    }

    /// PC: one bare LZW stream per picture, expanding straight to pixels.
    ///
    /// The stream is self-delimiting — it ends on the end-of-stream code — so
    /// the byte count it yields says which of the format's two pixel packings a
    /// picture uses, and the two are never the same size for a picture wider
    /// than one pixel:
    ///
    /// * `width * height` bytes: one palette index per byte. Every MCGA and EGA
    ///   picture, and Zork Zero's whole CGA archive.
    /// * `ceil(width / 8) * height` bytes: one **bit** per pixel, rows padded
    ///   out to whole bytes — `EF_MONO`, "two-color picture". Arthur, Journey
    ///   and Shogun each keep their large CGA artwork this way, 228 pictures
    ///   between them. Frotz's `bcpic.c` calls it out in the same words:
    ///
    ///   ```text
    ///   (There is a special case: CGA pictures
    ///   with no transparent colour are stored as bit patterns, i.e.
    ///   every byte holds the pattern for eight pixels. A pixel must
    ///   be white if the corresponding bit is set, otherwise it must
    ///   be black.)
    ///   ```
    ///
    ///   `bcpic.c` reaches that case by knowing it opened a `.CG1` — it picks
    ///   the extension from the display mode, so it never has to ask a file what
    ///   it is. A reader handed only bytes has no display mode, which is why the
    ///   packing is decided here by `EF_MONO` plus the length the stream itself
    ///   declares. On the 228 packed pictures in the corpus the two rules pick
    ///   the same ones: every packed picture sets `EF_MONO` and clears
    ///   `EF_TRANS`, and no EGA or MCGA picture sets `EF_MONO` at all.
    ///
    /// The high bit of each byte is the leftmost pixel. `bcpic.c` writes the
    /// decoded byte straight into the CGA framebuffer, where bit 7 is the
    /// left-hand pixel of its eight, and unpacking that way also leaves
    /// consistently fewer horizontal colour changes across all three CGA
    /// archives than the other order does.
    fn decode_lzw(&self, e: &PicEntry) -> Result<Vec<u8>, PicError> {
        if e.flags & EF_IFF != 0 {
            return Err(PicError::UnsupportedCompression(e.flags));
        }
        let w = usize::from(e.width);
        let h = usize::from(e.height);
        let flat = w * h;
        let raw = lzw_expand(&self.data[e.data..], flat)?;
        if raw.len() == flat {
            return Ok(raw);
        }
        let stride = w.div_ceil(8);
        if e.flags & EF_MONO != 0 && raw.len() == stride * h {
            let mut out = vec![0u8; flat];
            for y in 0..h {
                for x in 0..w {
                    out[y * w + x] = raw[y * stride + (x >> 3)] >> (7 - (x & 7)) & 1;
                }
            }
            return Ok(out);
        }
        Err(PicError::BadPixelData)
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
    ///
    /// Both flavours lay the palette out the same way, which is worth saying
    /// because it was found twice independently. `bcpic.c`, on the PC side:
    /// "The first colour to be defined is colour 2. Every map defines up to 14
    /// colours (colour 2 to 15)."
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

/// Expand one PC picture's LZW stream, stopping at the end-of-stream code.
///
/// This is GIF's variable-width LZW with the minimum code size fixed at 8: 256
/// clears the table, 257 ends the stream, assignable codes start at 258, and the
/// code width runs 9 to 12 bits, widening as soon as the table fills the current
/// width. Codes are packed least-significant bit first, so a code straddling a
/// byte boundary takes its low bits from the earlier byte. Every one of those
/// constants is Frotz's `src/dos/bcpic.c`, in its own words:
///
/// ```text
/// Note that low bits always come first.
///
/// There are two codes with a special meaning. The first one
/// is 256 which clears the table and sets the number of bits
/// per code to 9. ... The
/// second one is 257 which marks the end of the picture.
///
/// At the start of decompression 9 bits make one code; during
/// the process this can rise to 12 bits per code. 9 bits are
/// sufficient to address both 256 literal values and 256 table
/// entries; 12 bits are sufficient to address both 256 literal
/// values and all 3840 table entries.
/// ```
///
/// 3840 table entries above 256 literals is 4096 codes, which is this module's
/// [`LZW_MAX_CODES`]; `bcpic.c` biases every code down by 256 for speed, so its
/// `next_entry` and its `raise_bits` thresholds of 256/768/1792 are 258 and
/// 512/1024/2048 read the way they are here. The `next_entry == prev_code` case
/// it warns about ("a code may legally refer to the table entry which is
/// currently being set") is the placeholder below.
///
/// `limit` caps the output: a picture is `width * height` pixels at the very
/// most, and a stream claiming more than that is malformed rather than large.
fn lzw_expand(stream: &[u8], limit: usize) -> Result<Vec<u8>, PicError> {
    let mut prefix = [0u16; LZW_MAX_CODES];
    let mut suffix = [0u8; LZW_MAX_CODES];
    let mut out: Vec<u8> = Vec::with_capacity(limit);
    // Codes decode back to front, so a string is stacked and then unwound.
    let mut stack: Vec<u8> = Vec::with_capacity(LZW_MAX_CODES);
    let mut next = LZW_FIRST_CODE;
    let mut width = LZW_MIN_CODE_WIDTH;
    let mut prev: Option<u16> = None;
    let mut bit = 0usize;
    let bits = stream.len() * 8;

    loop {
        if bit + width as usize > bits {
            return Err(PicError::BadPixelData);
        }
        let mut code = 0u16;
        for i in 0..width as usize {
            let at = bit + i;
            code |= u16::from(stream[at >> 3] >> (at & 7) & 1) << i;
        }
        bit += width as usize;

        if code == LZW_CLEAR {
            next = LZW_FIRST_CODE;
            width = LZW_MIN_CODE_WIDTH;
            prev = None;
            continue;
        }
        if code == LZW_END {
            return Ok(out);
        }

        stack.clear();
        // The one code that is legal before it has been assigned: it can only be
        // the string just emitted followed by that string's own first byte.
        let mut walk = if code < next {
            code
        } else if code == next {
            stack.push(0); // placeholder for that first byte, patched below
            prev.ok_or(PicError::BadPixelData)?
        } else {
            return Err(PicError::BadPixelData);
        };
        while walk >= LZW_CLEAR {
            stack.push(suffix[usize::from(walk)]);
            walk = prefix[usize::from(walk)];
        }
        let first = walk as u8;
        stack.push(first);
        if code == next {
            stack[0] = first;
        }

        if out.len() + stack.len() > limit {
            return Err(PicError::BadPixelData);
        }
        out.extend(stack.iter().rev());

        if let Some(p) = prev {
            if usize::from(next) < LZW_MAX_CODES {
                prefix[usize::from(next)] = p;
                suffix[usize::from(next)] = first;
                next += 1;
                if usize::from(next) == 1 << width && width < LZW_MAX_CODE_WIDTH {
                    width += 1;
                }
            }
        }
        prev = Some(code);
    }
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from(b[o]) << 8 | u16::from(b[o + 1])
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from(b[o]) | u16::from(b[o + 1]) << 8
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

    /// An Amiga/Mac record size must be the one its header flags imply: 16
    /// bytes exactly when `HF_EHUFF` stands alone, 14 otherwise. Neither size is
    /// accepted on its own say-so.
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
    fn rejects_containers_it_cannot_read() {
        let mut f = synthetic();
        f[8] = 8; // the Apple II record size, which this module does not read
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

    /// One hand-built PC picture: what to put in the directory, and the LZW
    /// codes whose expansion is its pixels.
    struct PcPic {
        id: u16,
        w: u16,
        h: u16,
        flags: u16,
        codes: Vec<u16>,
        palette: Vec<u8>,
    }

    /// Pack LZW codes least-significant bit first at the starting width of 9.
    ///
    /// Deliberately *not* the decoder's bit reader run backwards: this is an
    /// independent encoder, and it stays at 9 bits, which every stream here is
    /// short enough to earn (the table would have to reach 512 to widen).
    fn pc_pack(codes: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut acc = 0u32;
        let mut held = 0u32;
        for &c in codes {
            acc |= u32::from(c) << held;
            held += 9;
            while held >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                held -= 8;
            }
        }
        if held > 0 {
            out.push(acc as u8);
        }
        out
    }

    /// Assemble a PC archive around those pictures. Records are 12 bytes (the
    /// EGA/CGA shape, a pad byte where the palette pointer would be) unless some
    /// picture carries a palette, in which case they are 14.
    fn pc_archive(pics: &[PcPic]) -> Vec<u8> {
        let paletted = pics.iter().any(|p| !p.palette.is_empty());
        let entry_size = if paletted {
            PC_ENTRY_SIZE_PAL
        } else {
            PC_ENTRY_SIZE
        };
        let dir_end = 16 + pics.len() * entry_size;
        let mut dir = Vec::new();
        let mut body: Vec<u8> = Vec::new();
        let off3 = |o: usize| [(o >> 16) as u8, (o >> 8) as u8, o as u8];
        for p in pics {
            let pal_off = if p.palette.is_empty() {
                0
            } else {
                let o = dir_end + body.len();
                body.extend_from_slice(&p.palette);
                o
            };
            let data_off = dir_end + body.len();
            body.extend_from_slice(&pc_pack(&p.codes));
            dir.extend_from_slice(&p.id.to_le_bytes());
            dir.extend_from_slice(&p.w.to_le_bytes());
            dir.extend_from_slice(&p.h.to_le_bytes());
            dir.extend_from_slice(&p.flags.to_le_bytes());
            dir.extend_from_slice(&off3(data_off));
            if paletted {
                dir.extend_from_slice(&off3(pal_off));
            } else {
                dir.push(0); // `mac/gfx.p`'s "single pad byte"
            }
        }
        let mut f = vec![0u8; 16];
        f[0] = 1; // part
        f[1] = 0x30; // neither HF_EHUFF nor HF_GHUFF: no Huffman tree anywhere
        f[4..6].copy_from_slice(&u16::try_from(pics.len()).unwrap().to_le_bytes());
        f[8] = entry_size as u8;
        f[12] = 1; // version
        f.extend_from_slice(&dir);
        f.extend_from_slice(&body);
        f
    }

    /// SQ-0735. The PC flavour, end to end on a file with no Infocom media in
    /// it: little-endian header and directory, MSB-first 3-byte offsets, and one
    /// bare LZW stream per picture that expands straight to pixels — no
    /// run-length stage and no XOR.
    #[test]
    fn decodes_a_synthetic_pc_picture() {
        let f = pc_archive(&[PcPic {
            id: 7,
            w: 4,
            h: 2,
            flags: EF_TRANS,
            // clear, then eight literals, then end-of-stream.
            codes: vec![LZW_CLEAR, 2, 2, 2, 2, 3, 3, 3, 3, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.flavour(), Flavour::Pc);
        assert_eq!(pics.part(), 1);
        assert_eq!(pics.entries().len(), 1);

        let p = pics.decode(7).unwrap();
        assert_eq!((p.width, p.height), (4, 2));
        assert_eq!(p.indices, vec![2, 2, 2, 2, 3, 3, 3, 3]);
        assert_eq!(p.transparent, Some(0));
        // 12-byte records have nowhere to keep a palette, so every picture in an
        // EGA/CGA archive is adaptive.
        assert!(p.palette.is_none());
        assert_eq!(pics.adaptive_pictures(), vec![7]);
    }

    /// The LZW table itself: a code that stands for a string the stream taught
    /// the decoder, and the self-referential code that is legal one step before
    /// it is defined. Literals alone would leave both untested.
    #[test]
    fn expands_learned_and_self_referential_lzw_codes() {
        // 258 is assigned "1 2" by the pair before it, then used.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 4,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, LZW_FIRST_CODE, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1).unwrap().indices, vec![1, 2, 1, 2]);

        // 258 used in the very code that defines it: it can only mean the last
        // string plus that string's own first byte.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 3,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 5, LZW_FIRST_CODE, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1).unwrap().indices, vec![5, 5, 5]);

        // A code that is neither known nor the next one to be assigned is not a
        // picture at all.
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 3,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 5, LZW_FIRST_CODE + 1, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1), Err(PicError::BadPixelData));
    }

    /// `EF_MONO` — "two-color picture" — is one BIT per pixel with rows padded
    /// out to whole bytes, and the high bit of each byte is the leftmost pixel.
    /// Twelve pixels wide exercises the padding: two bytes a row, four bits of
    /// each second byte thrown away.
    #[test]
    fn expands_a_one_bit_pc_picture() {
        let f = pc_archive(&[PcPic {
            id: 3,
            w: 12,
            h: 2,
            flags: EF_MONO,
            codes: vec![LZW_CLEAR, 0xA0, 0xF0, 0x0F, 0x00, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        let p = pics.decode(3).unwrap();
        assert_eq!(p.indices.len(), 24);
        #[rustfmt::skip]
        assert_eq!(
            p.indices,
            vec![
                1, 0, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1,
                0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0,
            ]
        );
        assert_eq!(p.transparent, None, "EF_MONO alone claims no transparent colour");
    }

    /// A 14-byte PC record keeps a palette pointer where the 12-byte one keeps a
    /// pad byte, and the palette is laid out exactly as the Amiga's: a count,
    /// that many RGB triples starting at colour index 2, then the stipple table.
    /// MCGA archives are the ones that use it.
    #[test]
    fn a_pc_archive_can_carry_palettes() {
        let f = pc_archive(&[PcPic {
            id: 4,
            w: 2,
            h: 1,
            flags: EF_TRANS,
            codes: vec![LZW_CLEAR, 2, 3, LZW_END],
            palette: vec![2, 9, 8, 7, 6, 5, 4, 0],
        }]);
        assert_eq!(usize::from(f[8]), PC_ENTRY_SIZE_PAL);
        let pics = InfocomPics::parse(f).unwrap();
        assert!(pics.entry(4).unwrap().has_own_palette());
        assert!(pics.adaptive_pictures().is_empty());

        let p = pics.decode(4).unwrap();
        let pal = p.palette.expect("a 14-byte record names a palette");
        assert_eq!(pal[2], [9, 8, 7]);
        assert_eq!(pal[3], [6, 5, 4]);
        assert_eq!(pal[4], DEFAULT_PALETTE[4], "the count stops at two colours");
        assert_eq!(&p.rgba()[..8], &[9, 8, 7, 255, 6, 5, 4, 255]);
    }

    /// A stream that keeps going past `width * height` is malformed, not big.
    #[test]
    fn rejects_a_pc_stream_that_overruns_its_picture() {
        let f = pc_archive(&[PcPic {
            id: 1,
            w: 2,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, 3, 4, LZW_END],
            palette: vec![],
        }]);
        let pics = InfocomPics::parse(f).unwrap();
        assert_eq!(pics.decode(1), Err(PicError::BadPixelData));
    }

    /// Structural gates on the PC directory: ids ascend (the interpreter binary
    /// -searches it), and nothing may claim a Huffman tree in a file that names
    /// none.
    #[test]
    fn rejects_malformed_pc_directories() {
        let two = || {
            [1u16, 2].map(|id| PcPic {
                id,
                w: 2,
                h: 1,
                flags: 0,
                codes: vec![LZW_CLEAR, 1, 2, LZW_END],
                palette: vec![],
            })
        };
        assert!(InfocomPics::parse(pc_archive(&two())).is_ok());

        let mut f = pc_archive(&two());
        f[16 + PC_ENTRY_SIZE] = 1; // the second record now claims id 1 as well
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );

        let mut f = pc_archive(&two());
        f[16 + 6] = EF_PHUFF as u8; // Huffman-coded, with no tree in the file
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );

        let mut f = pc_archive(&two());
        f[8] = 16; // a record size no PC archive uses
        assert_eq!(
            InfocomPics::parse(f).err(),
            Some(PicError::UnsupportedContainer)
        );
    }

    /// The flavour is decided by whether the file names a Huffman tree, not by
    /// its record size — MCGA archives use the Amiga's 14 — and not by the flag
    /// byte, which reads `0x30` on four MCGA archives and `0x38` on a fifth.
    #[test]
    fn the_flavour_is_decided_by_whether_a_huffman_tree_is_named() {
        let mut f = pc_archive(&[PcPic {
            id: 1,
            w: 2,
            h: 1,
            flags: 0,
            codes: vec![LZW_CLEAR, 1, 2, LZW_END],
            palette: vec![1, 9, 8, 7, 0], // forces the 14-byte record
        }]);
        assert_eq!(f[8], PC_ENTRY_SIZE_PAL as u8, "the Amiga's record size");
        assert_eq!(InfocomPics::parse(f.clone()).unwrap().flavour(), Flavour::Pc);

        f[1] = 0x38;
        assert_eq!(
            InfocomPics::parse(f).unwrap().flavour(),
            Flavour::Pc,
            "the flag byte does not decide it — real MCGA archives read both"
        );

        // The Amiga's own archives, which do name a tree, keep reading as the
        // Amiga's.
        assert_eq!(
            InfocomPics::parse(synthetic()).unwrap().flavour(),
            Flavour::AmigaMac
        );
        assert_eq!(
            InfocomPics::parse(synthetic_per_entry_huff()).unwrap().flavour(),
            Flavour::AmigaMac
        );
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

    /// A file out of the gitignored `stories/`, or `None` with a SKIP note.
    fn fixture(name: &str) -> Option<Vec<u8>> {
        let p: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories")
            .join(name);
        match std::fs::read(&p) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored fixture missing at {}", p.display());
                None
            }
        }
    }

    /// FNV-1a over every decoded picture's indices, in directory order.
    fn pixel_fingerprint(pics: &InfocomPics) -> (usize, usize, u64) {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut n = 0;
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
                h = (h ^ u64::from(i)).wrapping_mul(0x100_0000_01b3);
            }
        }
        (pics.entries().len(), n, h)
    }

    /// SQ-0735, real media: every PC archive on hand.
    ///
    /// All but the last row is original DOS media for four Infocom titles.
    /// Two files on hand are deliberately absent: `FMVPOKER.EG1` and
    /// `zorkzero.mg1` are byte-identical to `zork0.eg1` and `zork0.mg1` (the
    /// first because fmvpoker's readme tells players to rename the Zork Zero
    /// file), so they are copies rather than specimens.
    ///
    /// `beyondzo.mg1` is **not** Infocom media: it is the Atari ST release's
    /// title picture converted into the MCGA container by Stefan Jokisch, whose
    /// `bcpic.c` is one of this decoder's two witnesses. Beyond Zork's own DOS
    /// release shipped no artwork. It is kept as a fixture precisely because it
    /// came out of a different encoder, and it earns its keep: it decodes on the
    /// same rules as the authentic files. Nothing about the format is inferred
    /// from it — its header even carries the 640-wide flag byte (`0x38`) on a
    /// 320-wide archive, which no authentic `.MG1` does.
    ///
    /// The fingerprints are regression pins, not the proof of correctness; that
    /// is [`mcga_zork_zero_decodes_to_the_amiga_pixels`]. Each was computed by a
    /// separately written decoder before this one existed and matched it.
    ///
    /// [`mcga_zork_zero_decodes_to_the_amiga_pixels`]: mcga_zork_zero_decodes_to_the_amiga_pixels
    #[test]
    fn reads_the_pc_archives() {
        #[rustfmt::skip]
        let corpus: [(&str, u8, usize, usize, usize, u64); 15] = [
            // file             part  records  pixels  colours  fingerprint
            ("zork0.mg1",          1,     503,    396,      16, 0x2c3c_51eb_c815_51c8),
            ("zork0.eg1",          1,     503,    396,      16, 0x32d3_69e3_066a_58d6),
            ("zork0.cg1",          1,     503,    396,       4, 0x8de5_413b_5636_0cf8),
            ("arthur.mg1",         1,     171,    137,      16, 0x1e5f_de1a_8541_fe0e),
            ("arthur.eg1",         1,     125,     97,      16, 0xc5c5_4b8a_fbb7_1762),
            ("arthur.eg2",         2,     101,     77,      16, 0xdea4_ab1d_39c8_d0a1),
            ("arthur.cg1",         1,     170,    136,       4, 0x1c9a_843e_e318_15dc),
            ("journey.mg1",        1,     134,    134,      16, 0x17d0_5716_509d_75e0),
            ("journey.eg1",        1,      80,     80,      16, 0xd287_b2f3_5e8f_0f7a),
            ("journey.eg2",        2,      67,     67,      16, 0xa3cb_ba73_0ba7_92a5),
            ("journey.cg1",        1,     134,    134,       4, 0x2769_2a5a_147c_588e),
            ("shogun.mg1",         1,      48,     42,      16, 0x04f3_726e_9f03_3aaa),
            ("shogun.eg1",         1,      50,     44,      16, 0x9cf4_195e_0590_bbe7),
            ("shogun.cg1",         1,      50,     44,       4, 0x0e1b_af9a_9e34_93cb),
            // A third-party conversion, segregated on purpose. See above.
            ("beyondzo.mg1",       1,       1,      1,      16, 0x3b66_df38_f6d2_1e66),
        ];
        for (name, part, records, pixels, colours, want) in corpus {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(pics.flavour(), Flavour::Pc, "{name}");
            assert_eq!(pics.part(), part, "{name} part number");
            assert_eq!(
                pixel_fingerprint(&pics),
                (records, pixels, want),
                "{name} decoded differently"
            );
            // Nothing may index past the palette it will be drawn through, and
            // the depth each rendition was drawn for shows in the indices: CGA
            // never exceeds 3, EGA and MCGA never exceed 15.
            for e in pics.entries().iter().filter(|e| e.has_pixels()) {
                let p = pics.decode(e.id).unwrap();
                assert!(
                    p.indices.iter().all(|&i| usize::from(i) < colours),
                    "{name} picture {} indexes past {colours} colours",
                    e.id
                );
            }
        }
    }

    /// SQ-0735's oracle, and the reason the LZW path can be trusted at all:
    /// **two unrelated codecs agree, picture for picture**.
    ///
    /// `zork0.mg1` (DOS, MCGA) and `zork0.pic` (the Amiga `Pic.data`) hold the
    /// same artwork at the same 320x200 on the same 12-bit palettes. The Amiga
    /// side is Huffman + run-length + per-line XOR and was verified against
    /// Infocom's own sources and against `Zork0.blb`'s PNGs; the PC side is LZW
    /// and shares not one line of code with it. For every id whose two
    /// directories agree on dimensions, the decoded pixels are identical.
    ///
    /// That is also what rules out a hidden stage on the PC side: an
    /// un-run-length or un-XOR step would have to be the identity on 383
    /// different pictures.
    #[test]
    fn mcga_zork_zero_decodes_to_the_amiga_pixels() {
        let (Some(pc), Some(amiga)) = (fixture("zork0.mg1"), fixture("zork0.pic")) else {
            return;
        };
        let pc = InfocomPics::parse(pc).unwrap();
        let amiga = InfocomPics::parse(amiga).unwrap();
        assert_eq!(pc.flavour(), Flavour::Pc);
        assert_eq!(amiga.flavour(), Flavour::AmigaMac);

        let (mut same, mut differ, mut sized_differently) = (0, 0, Vec::new());
        for e in pc.entries().iter().filter(|e| e.has_pixels()) {
            let Some(a) = amiga.entry(e.id).filter(|a| a.has_pixels()) else {
                continue;
            };
            if (a.width, a.height) != (e.width, e.height) {
                sized_differently.push(e.id);
                continue;
            }
            let (p, q) = (pc.decode(e.id).unwrap(), amiga.decode(e.id).unwrap());
            if p.indices == q.indices {
                same += 1;
            } else {
                differ += 1;
                assert!(differ < 4, "picture {} differs between the two codecs", e.id);
            }
        }
        assert_eq!((same, differ), (383, 0));
        // The only ids the two directories size differently are the five
        // SQ-0713 already catalogued, where the Amiga release holds a full
        // decorative frame and the other sources a cropped band.
        assert_eq!(sized_differently, vec![5, 6, 7, 8, 33]);

        // Same palettes, too — the MCGA archive stores the Amiga's 12-bit
        // values, which is why the pixels can be identical in the first place.
        // Pinned absolutely as well as relatively, so that the shared claim
        // "the stored triples start at colour 2" is answerable by real media on
        // this side and not only by a hand-built archive.
        assert_eq!(pc.palette_of(1), amiga.palette_of(1));
        let pal = pc.palette_of(1).expect("the MCGA title screen carries a palette");
        assert_eq!(pal[2], [0xcc, 0xcc, 0xcc]);
        assert_eq!(pal[15], [0xee, 0xee, 0xee]);
    }

    /// Zork Zero shipped 503 pictures in every PC rendition, and the three
    /// directories are the same directory: same ids, and the same 396 of them
    /// carry pixels while the same 107 are size-only placeholders. That holds
    /// without decoding anything, so it is an independent check on the
    /// little-endian directory reading.
    #[test]
    fn the_pc_renditions_of_zork_zero_share_a_directory() {
        let renditions: Vec<InfocomPics> = ["zork0.mg1", "zork0.eg1", "zork0.cg1"]
            .iter()
            .filter_map(|n| fixture(n))
            .map(|b| InfocomPics::parse(b).unwrap())
            .collect();
        if renditions.len() < 3 {
            return;
        }
        let shape = |p: &InfocomPics| -> Vec<(u16, bool)> {
            p.entries().iter().map(|e| (e.id, e.has_pixels())).collect()
        };
        assert_eq!(shape(&renditions[0]).len(), 503);
        assert_eq!(shape(&renditions[1]), shape(&renditions[0]));
        assert_eq!(shape(&renditions[2]), shape(&renditions[0]));

        // MCGA is the only PC rendition with palettes of its own; EGA and CGA
        // records are 12 bytes with nowhere to put one.
        assert!(renditions[0].adaptive_pictures().len() < 396);
        assert_eq!(renditions[1].adaptive_pictures().len(), 396);
        assert_eq!(renditions[2].adaptive_pictures().len(), 396);
    }

    /// Arthur's, Journey's and Shogun's CGA archives are the only ones that
    /// carry `EF_MONO` artwork — one bit per pixel, rows padded to whole bytes.
    /// Zork Zero's CGA archive carries none, so a suite that only looked at
    /// Zork Zero would never exercise the unpacking at all.
    #[test]
    fn the_cga_archives_carry_bit_packed_artwork() {
        for (name, packed) in [
            ("arthur.cg1", 92),
            ("journey.cg1", 111),
            ("shogun.cg1", 25),
            ("zork0.cg1", 0),
        ] {
            let Some(bytes) = fixture(name) else { continue };
            let pics = InfocomPics::parse(bytes).unwrap();
            let mono: Vec<&PicEntry> = pics
                .entries()
                .iter()
                .filter(|e| e.has_pixels() && e.flags & EF_MONO != 0 && e.flags & EF_TRANS == 0)
                .collect();
            assert_eq!(mono.len(), packed, "{name}");
            for e in mono {
                let p = pics.decode(e.id).unwrap();
                assert!(
                    p.indices.iter().all(|&i| i < 2),
                    "{name} picture {} is not two-colour after unpacking",
                    e.id
                );
            }
        }
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
        let feed = |h: &mut u64, b: u8| *h = (*h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
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
