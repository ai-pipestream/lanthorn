//! The **Atari 8-bit** S.A.G.A. picture sides (SQ-1483, SQ-1484).
//!
//! Every US S.A.G.A. release for the Atari 8-bit is two single-density disks:
//! side A holds the §12 binary database ([`crate::saga_us`] reads it) and side
//! B holds nothing but artwork. Side B has **no filesystem** — sectors 361-368,
//! where an Atari DOS 2 directory would be, hold picture data like every other
//! sector — so §8.3 is right that "there is no filesystem walk" and §12.10 is
//! right that the association "cannot be recovered from the database".
//!
//! # What the seven sides actually hold, measured
//!
//! **Two formats, not one**, and which one a title uses is not stated anywhere
//! in the specification:
//!
//! | title | side B | this module |
//! |---|---|---|
//! | #1 Adventureland | line-art token stream (`crate::saga_atari_lineart`) | [`read_line_art_table`] |
//! | #2 Pirate Adventure | line-art token stream (`crate::saga_atari_lineart`) | [`read_line_art_table`] |
//! | #3 Mission Impossible | line-art token stream (`crate::saga_atari_lineart`) | [`read_line_art_table`] |
//! | #6 Strange Odyssey | line-art token stream (`crate::saga_atari_lineart`) | [`read_line_art_table`] |
//! | #4 Voodoo Castle | family-C bitmaps, [`FamilyCScheme::NoLiteral`] | **yes** |
//! | #5 The Count | family-C bitmaps, [`FamilyCScheme::NoLiteral`] | **yes** |
//! | #13 Claymorgue Castle | family-C bitmaps, [`FamilyCScheme::Standard`] | **yes** |
//!
//! The split is exactly the split §7.4's string test makes on the **Apple II**
//! releases of the same seven titles: the four whose Apple side A is an
//! ordinary DOS 3.3 disk draw with line tokens on both machines, and the three
//! "scrambled" ones ship bitmaps on both. **The line-art side is not the same
//! grammar the Apple side is**, though the two open with byte-identical bytes
//! at file offset `0x1000` — SQ-1524 measured that those bytes are the MIDDLE
//! of the darkness card's own record, not a picture in their own right, and
//! SQ-1525 read the real grammar off the release's own renderer: fixed
//! three-byte tokens on a 160x96 Atari `GRAPHICS 7` canvas, not the Apple's
//! mixed-width tokens on 280x192. See `crate::saga_atari_lineart` for that
//! grammar and [`read_line_art_table`] for how a record is found at all — the
//! table lives on SIDE B here, unlike the bitmap titles' side-A one below.
//!
//! # The record, as measured
//!
//! §8.3 describes the Commodore 64 record and says the Atari's is the same
//! thing reached differently — "a picture's record starts **two bytes before**
//! the listed offset, and its length is the little-endian word at the listed
//! offset **plus two**". Measured on all three specimens, the Atari record is
//! **ten bytes of header and no tail**:
//!
//! | offset | field |
//! |---|---|
//! | 0-1 | little-endian **record size**, the whole record, this header included |
//! | 2 | left edge in 8-pixel columns **plus 3** |
//! | 3 | top row |
//! | 4 | right edge in columns **plus 3** |
//! | 5 | bottom row |
//! | 6-9 | four colour bytes |
//! | 10.. | compressed data, to the end of the record |
//!
//! So there is **no two-byte load address** — §8.3's "two bytes before the
//! listed offset" are the previous record's last data bytes, and they read as
//! `$FFFF`, `$3F3F` and `$C3C3` as often as anything else — and **no two-byte
//! tail**. The consistency check below is what settles that rather than the
//! prose: read with a twelve-byte header the data is two bytes short and no
//! record's region ever fills.
//!
//! # Finding the records
//!
//! Records are laid end to end from file offset `0x297` — after a boot loader
//! that is byte-identical on all seven sides, and after three bytes holding
//! the release's own Adventure International number — with **nought to six
//! bytes of filler between them**. The filler is why a reader cannot simply
//! add sizes, and [`scan_picture_side`] therefore re-finds each record rather
//! than trusting the previous one's arithmetic.
//!
//! What makes that safe is that a family-C record is **self-proving**. Its
//! header says how many byte pairs it must hold — `cols * pairs`, from
//! [`StripLayout`] — and decoding it either produces exactly that many and
//! stops within a byte of the declared size, or it does not. Measured over all
//! three sides, 241 records satisfy that and every one of them decodes to a
//! coherent picture; the declared size exceeds the decoded length by 0 on 198
//! of them and by exactly 1 on the other 43, and never by more.
//!
//! # Which picture index a record answers to (SQ-1496)
//!
//! It is not in the disk's own order — *The Count*'s twenty-five full-canvas
//! records include its darkness card (§8.6's reserved index 0) and its room 1
//! next to each other and in that order, but the record *before* them is room
//! 2's picture — and it is not in the database (§12.10 is right that "cannot
//! be recovered from the database" is about the database). **It is on side A
//! all the same**, in a table the game program reads rather than the database
//! parser does: [`read_picture_table`] walks 190 two-byte entries at
//! [`PICTURE_TABLE_OFFSET`], validating each against a record this module's
//! own scan (or [`decode_record_with_bad_sector_fallback`]'s SQ-1498
//! fallback) can actually decode, plus one more fixed entry at
//! [`INVENTORY_BACKDROP_ENTRY`] for the picture §12.11's inventory command
//! draws behind the carried items. See that section for the table's layout
//! and for the investigation note's per-title tables in the quest itself
//! (SQ-1496).
//!
//! # SQ-1498: two damaged records on *The Count*
//!
//! Two of the 241 records this module's scan locates are not really
//! unreadable — they are ordinary well-formed records each missing exactly
//! one 128-byte sector on the one specimen this crate has (a stale duplicate
//! sector for room 6, an unwritten all-zero one for room 16). The strict scan
//! still refuses both, correctly — a damaged record is not a well-formed one
//! — but [`decode_record_with_bad_sector_fallback`] gives
//! [`decode_table_picture`] a documented, specimen-specific second path that
//! recovers a recognisable (if slightly marred) picture for the two records
//! the picture table names there. See that function's own doc for the
//! recipe.
//!
//! # The volume table of contents
//!
//! §7.3's splice is real and this module applies it: the 128 bytes at file
//! offsets `0xB390`-`0xB40F` are sector 360, they are not picture data (on
//! *The Count*'s side B they are ten `0xFF` bytes, a long run of zeros and
//! eleven more `0xFF`, between two stretches of run-length data that continue
//! across them), and a record spanning them must have them excised.
//!
//! # The line-art side's own table (SQ-1524) and grammar (SQ-1525)
//!
//! Both are settled now. The four `AtariPictureFormat::LineArt` titles carry
//! their own (usage, index) table on **side B itself** — not side A, where
//! the three bitmap titles keep theirs — behind a three-byte adventure-number
//! marker at [`LINE_ART_TABLE_MARKER`], 190 two-byte entries at
//! [`LINE_ART_TABLE_OFFSET`] in the same room-then-object shape the bitmap
//! table uses, but a different arithmetic ([`line_art_entry_file_offset`]):
//! a line-art record's tokens are three bytes each, where a bitmap record's
//! no-literal units are seven. There is no inventory flag bit and no separate
//! fixed inventory-backdrop pointer either — an object picture here serves
//! both [`PictureUsage::ObjectInRoom`] and [`PictureUsage::ObjectInInventory`]
//! (exactly as the same four titles' Apple II `B<aa><nnn>` files, one per
//! object with no R/I suffix, already do), and index 98 is an ordinary table
//! entry rather than [`AtariPictureTable`]'s fixed pointer. [`read_line_art_table`]
//! reads it, validating every entry against a record
//! [`draw_line_art_record`] can actually play.
//!
//! The grammar those records play under is `crate::saga_atari_lineart`'s, read
//! off the release's own renderer (SQ-1525) rather than assumed from the
//! Apple II's — see that module's docs for the tokens, the two-bitmap screen,
//! and the fill and line algorithms. **State carries between records** (the
//! canvas, the fill colours, the pen), which is how an object picture draws
//! over its room and how six named room records that never clear the screen
//! draw over whatever came before them — [`draw_line_art_record`] plays a
//! record onto a caller-supplied [`crate::saga_atari_lineart::LineArtCanvas`] in
//! place for exactly that reason, never onto a fresh one internally.

use crate::saga_atari_lineart::{CommittedLine, LineArtCanvas, LineArtError, LineArtPicture};
use crate::saga_pictures::{
    atari_colour, paint_strips, paint_strips_from, resolve_palette, FamilyCScheme, Painted, Picture,
    PictureError, StripLayout, CANVAS_HEIGHT, CANVAS_WIDTH,
};
use crate::saga_us::PictureUsage;

/// File offset of sector 360, the volume table of contents (§7.3).
///
/// `16 + 359 * 128`, which is the arithmetic §7.3 gives to confirm both the
/// sixteen-byte header and the 128-byte sector stride.
pub const VTOC_OFFSET: usize = 0xB390;

/// How long the volume table of contents is: one sector.
pub const VTOC_LEN: usize = 128;

/// The byte length of a single-density `.atr` image, header included (§7.3).
pub const SIDE_LEN: usize = 92_176;

/// Where [`scan_picture_side`] starts looking.
///
/// The boot loader in front of it is byte-identical on all seven sides — it
/// prints `HIT RETURN … OTHERWISE INSERT OTHER DISK` — and the first record's
/// header sits at `0x297` on all three bitmap sides, right after three bytes
/// holding the release's Adventure International number (`04 04 04` on *Voodoo
/// Castle*, `05 05 05` on *The Count*, `0D 0D 0D` on *Claymorgue Castle*) and
/// four zeros. The scan begins a little in front of that rather than at it, so
/// that a differently-mastered disk is not refused for the sake of one
/// constant.
pub const FIRST_RECORD: usize = 0x290;

/// One family-C record located on a companion picture side.
///
/// Byte offsets are **into the spliced side** ([`splice_vtoc`]), not into the
/// `.atr` file, because a record may span the volume table of contents and a
/// file offset cannot describe that. [`Self::file_offset`] converts back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtariRecord {
    /// Offset of the record's first header byte, into the spliced side.
    pub(crate) offset: usize,
    /// The record's declared size — its own header's first word, covering the
    /// whole record including that word.
    pub(crate) size: usize,
    /// How many bytes decoding actually read. Either `size` or `size - 1`;
    /// see the module docs.
    pub(crate) decoded_len: usize,
    /// Where the record's strips land, already resolved under the release's
    /// scheme.
    pub(crate) layout: StripLayout,
    /// The four stored colour bytes, in file order.
    pub(crate) colour_bytes: [u8; 4],
}

impl AtariRecord {
    /// Offset of the record's first header byte, into the spliced side.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The record's declared size — its own header's first word, covering the
    /// whole record including that word.
    pub fn size(&self) -> usize {
        self.size
    }

    /// How many bytes decoding actually read. Either [`Self::size`] or
    /// `size - 1`; see the module docs.
    pub fn decoded_len(&self) -> usize {
        self.decoded_len
    }

    /// Where the record's strips land, already resolved under the release's
    /// scheme.
    pub fn layout(&self) -> StripLayout {
        self.layout
    }

    /// The four stored colour bytes, in file order.
    pub fn colour_bytes(&self) -> [u8; 4] {
        self.colour_bytes
    }

    /// The record's offset in the original `.atr` file.
    ///
    /// Equal to [`Self::offset`] in front of the volume table of contents and
    /// [`VTOC_LEN`] more behind it. A record that *spans* the table has a file
    /// offset in front of it and a length that does not describe its extent on
    /// disk, which is exactly why this type carries spliced offsets.
    pub fn file_offset(&self) -> usize {
        if self.offset < VTOC_OFFSET {
            self.offset
        } else {
            self.offset + VTOC_LEN
        }
    }
}

/// Remove sector 360, the volume table of contents, from a companion side
/// (§7.3).
///
/// Returns `side` unchanged when it is too short to contain the table, so that
/// a caller holding a fragment gets a fragment rather than a panic.
pub fn splice_vtoc(side: &[u8]) -> Vec<u8> {
    if side.len() <= VTOC_OFFSET + VTOC_LEN {
        return side.to_vec();
    }
    let mut out = Vec::with_capacity(side.len() - VTOC_LEN);
    out.extend_from_slice(&side[..VTOC_OFFSET]);
    out.extend_from_slice(&side[VTOC_OFFSET + VTOC_LEN..]);
    out
}

/// Read the record whose header sits at `offset` of a spliced side, or `None`
/// when there is not one there.
///
/// The test is the self-proving one the module docs describe, and it is worth
/// naming the parts because every one of them is load-bearing:
///
/// - the declared size is between the ten-byte header plus one unit and a
///   little over a full-canvas record, and lies inside the side;
/// - the four edge bytes describe a region on a 280 x 160 canvas;
/// - the region wants at most a full canvas' worth of byte pairs;
/// - decoding **fills that region exactly** — not one pair fewer, and not one
///   more than the last run overshoots by;
/// - and the bytes that took land within one of the declared size.
///
/// The fourth is the one that cannot be satisfied by accident. A wrong offset
/// gets a plausible header often enough (a 92 KB side offers 92,000 of them),
/// but its region and its data then disagree, and over three whole sides the
/// only offsets that pass are the 242 that are real records.
pub fn record_at(spliced: &[u8], offset: usize, scheme: FamilyCScheme) -> Option<AtariRecord> {
    let head = spliced.get(offset..offset + 10)?;
    let size = usize::from(u16::from_le_bytes([head[0], head[1]]));
    if !(13..=MAX_RECORD).contains(&size) || offset + size > spliced.len() {
        return None;
    }
    // The edges are bounded as STORED bytes rather than as resolved pixels,
    // because the stored form is where the slack is: full-canvas records
    // routinely declare a left edge of column -1 and a right edge one column
    // past the canvas — two of the *Hulk*'s Commodore 64 records do the same —
    // and clipping in `paint_strips` already keeps those off the canvas.
    if head[2] > MAX_COLUMN || head[4] > MAX_COLUMN || head[5] > MAX_ROW {
        return None;
    }
    let layout = StripLayout::resolve(
        i32::from(head[2]),
        i32::from(head[3]),
        i32::from(head[4]),
        i32::from(head[5]),
        scheme,
    )
    .ok()?;
    if layout.pair_count() > MAX_PAIRS {
        return None;
    }
    // A region wholly off the canvas paints nothing, so whatever it is it is
    // not a picture. Cheap, and principled where a minimum size would not be:
    // *Claymorgue Castle*'s smallest real record is fifteen byte pairs.
    if layout.left + layout.cols * 8 <= 0 || layout.left >= CANVAS_WIDTH as i32 {
        return None;
    }
    let painted = paint_strips(&spliced[offset + 10..offset + size], &layout, scheme);
    if painted.emitted != layout.pair_count() {
        return None;
    }
    let decoded_len = 10 + painted.consumed;
    if !(size == decoded_len || size == decoded_len + 1) {
        return None;
    }
    Some(AtariRecord {
        offset,
        size,
        decoded_len,
        layout,
        colour_bytes: [head[6], head[7], head[8], head[9]],
    })
}

/// The largest record any of the three sides holds, rounded up. *The Count*'s
/// biggest is 4,868 bytes and *Claymorgue Castle*'s 5,199.
const MAX_RECORD: usize = 9_000;

/// How many byte pairs a full canvas is, with room to spare: 36 columns x 80
/// pairs is 2,880, and the widest record measured wants exactly that.
const MAX_PAIRS: usize = 4_200;

/// The largest stored edge column byte. [`CANVAS_WIDTH`] is 35 columns and the
/// stored form adds 3, so 38 is the canvas' right edge; a few columns of slack
/// past it costs nothing, because a wider region simply cannot be filled by
/// the record's own data.
const MAX_COLUMN: u8 = 43;

/// The largest stored bottom row. [`CANVAS_HEIGHT`] is 160 and the tallest
/// record measured declares 158.
const MAX_ROW: u8 = 191;

/// Every family-C record on a companion picture side, in disk order.
///
/// `side` is the whole `.atr` file; the volume-table splice is applied here so
/// that a caller does not have to know about it. `scheme` is the release's,
/// from [`crate::SagaUs::picture_scheme`] — **never sniffed**, for the reason
/// [`FamilyCScheme`] gives.
///
/// The walk is greedy and re-syncing: at each offset it asks [`record_at`],
/// steps over a record it finds, and otherwise advances one byte. That copes
/// with the nought-to-six bytes of filler between records without needing to
/// know how much there is, and it recovers by itself if one record in a side
/// is unreadable.
///
/// Note what this does **not** answer: which picture index each record is. See
/// the module docs.
pub fn scan_picture_side(side: &[u8], scheme: FamilyCScheme) -> Vec<AtariRecord> {
    let spliced = splice_vtoc(side);
    let mut out = Vec::new();
    let mut at = FIRST_RECORD.min(spliced.len());
    while at + 10 <= spliced.len() {
        match record_at(&spliced, at, scheme) {
            Some(rec) => {
                at += rec.size;
                out.push(rec);
            }
            None => at += 1,
        }
    }
    out
}

/// Decode one located record to a picture.
///
/// `spliced` must be the same buffer [`scan_picture_side`] walked — the record
/// carries a spliced offset, not a file one. Reach for [`splice_vtoc`] if you
/// have only the `.atr`.
///
/// # Errors
///
/// [`PictureError::TooShort`] when the record runs past the end of `spliced`.
/// A located record cannot fail any other way: [`record_at`] has already
/// decoded it once.
pub fn decode_record(
    spliced: &[u8],
    record: &AtariRecord,
    scheme: FamilyCScheme,
) -> Result<Picture, PictureError> {
    let data = spliced
        .get(record.offset + 10..record.offset + record.size)
        .ok_or(PictureError::TooShort { len: spliced.len().saturating_sub(record.offset) })?;
    let strips = paint_strips(data, &record.layout, scheme);
    let (palette, unrecognised_colours) =
        resolve_palette(record.colour_bytes, |stored| Some(atari_colour(stored)));
    Ok(Picture {
        width: CANVAS_WIDTH,
        height: CANVAS_HEIGHT,
        pixels: strips.pixels,
        palette,
        colour_bytes: record.colour_bytes,
        unrecognised_colours,
        // Measured from the writes, so a record that paints only part of its
        // own region reports what it really covered (SQ-1487's rectangle).
        painted: strips.bounds,
    })
}

// ── The (usage, index) table on side A (SQ-1496) ───────────────────────────
//
// §12.10 is right that the DATABASE has no (usage, index) association for a
// picture record — but the program area of the same side does. Measured
// identically on all three bitmap titles (module docs' investigation note):
// side A file offset 0x9593 holds 190 two-byte entries, preceded at 0x9590 by
// three copies of the release's own Adventure International number. Entries
// [0..99] are ROOM-usage picture indices, [100..189] are OBJECT picture
// indices (entry `i` names item `i - 100`), and an all-zero entry means "no
// picture". One more entry, at 0x984F, is not part of the table: the
// inventory backdrop, reached by a fixed pointer rather than by index 98
// (which is unused in the table itself).

/// Side A file offset of the marker in front of the picture table: three
/// copies of the release's own Adventure International number (`04` Voodoo
/// Castle, `05` The Count, `0D` Claymorgue Castle) — read and checked rather
/// than trusted, so a differently-mastered disk is refused instead of read
/// through a table that is not really there.
pub const PICTURE_TABLE_MARKER: usize = 0x9590;

/// Side A file offset of the picture table itself, immediately after
/// [`PICTURE_TABLE_MARKER`]'s three bytes.
pub const PICTURE_TABLE_OFFSET: usize = 0x9593;

/// How many two-byte entries the table holds — see the section docs above.
pub const PICTURE_TABLE_ENTRIES: usize = 190;

/// Side A file offset of the one entry outside the table proper: the
/// inventory backdrop, the same three-colour-bar card on all three titles.
pub const INVENTORY_BACKDROP_ENTRY: usize = 0x984F;

/// Where a two-byte table entry `[a, s]` points, as a FILE offset into side B
/// (measured identical on all three bitmap titles).
///
/// `s` and the low two bits of `a` are a 1-based, 128-byte Atari sector
/// number; bits 3-7 of `a`, times 7, are the byte within that sector. Every
/// one of the 241 records this module's scan locates starts at a multiple of
/// seven bytes into its sector — that is what the nought-to-six bytes of
/// filler between records are for — which is what settles the `* 7`. Bit 2 of
/// `a` is a flag, set only on the object entries drawn on the inventory
/// screen rather than in a room.
pub fn table_entry_file_offset(a: u8, s: u8) -> usize {
    let sector = (usize::from(a & 3) << 8) | usize::from(s);
    16 + sector.saturating_sub(1) * 128 + usize::from(a >> 3) * 7
}

/// A side-B FILE offset in the SPLICED coordinates [`record_at`] and
/// [`scan_picture_side`] use — see [`splice_vtoc`].
pub fn spliced_of(file_offset: usize) -> usize {
    if file_offset < VTOC_OFFSET {
        file_offset
    } else {
        file_offset - VTOC_LEN
    }
}

/// One resolved (usage, index) → record association, read off side A's
/// picture table (SQ-1496).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PictureTableEntry {
    /// What the named record is for.
    pub usage: PictureUsage,
    /// The picture index — a room number for [`PictureUsage::Room`], an item
    /// number otherwise (§8.6's three reserved values apply here exactly as
    /// they do to a named picture file on another platform).
    pub index: u16,
    /// The record's offset into the whole `.atr` FILE, not the spliced side —
    /// pass through [`spliced_of`] before calling [`record_at`] directly, or
    /// hand it straight to [`decode_table_picture`], which does that already.
    pub file_offset: usize,
}

/// Side A's whole picture table (SQ-1496): every (usage, index) association
/// [`read_picture_table`] could verify against a record [`scan_picture_side`]
/// (or [`decode_record_with_bad_sector_fallback`]'s SQ-1498 fallback) can
/// actually locate, plus the fixed inventory-backdrop pointer.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AtariPictureTable {
    entries: Vec<PictureTableEntry>,
    inventory_backdrop_file_offset: Option<usize>,
}

impl AtariPictureTable {
    /// Every resolved (usage, index) association, in table order.
    pub fn entries(&self) -> &[PictureTableEntry] {
        &self.entries
    }

    /// The record a picture with this `usage` and `index` names, as a FILE
    /// offset — pass to [`decode_table_picture`] — or `None` when the table
    /// carries no such entry.
    pub fn find(&self, usage: PictureUsage, index: u16) -> Option<usize> {
        self.entries
            .iter()
            .find(|e| e.usage == usage && e.index == index)
            .map(|e| e.file_offset)
    }

    /// The inventory backdrop's record, as a FILE offset (§12.11's index 98
    /// on every other platform; the Atari reaches it through
    /// [`INVENTORY_BACKDROP_ENTRY`] instead — see the section docs). `None`
    /// when the entry is zero or does not resolve to a record this crate can
    /// decode.
    pub fn inventory_backdrop_file_offset(&self) -> Option<usize> {
        self.inventory_backdrop_file_offset
    }
}

/// Read side A's picture table (SQ-1496).
///
/// `side_a` is the whole database side, raw. `side_b_spliced` is the
/// companion picture side with the volume table of contents already excised
/// ([`splice_vtoc`]) — every entry is validated against a record
/// [`decode_table_picture`] can actually decode before it is trusted, so a
/// table entry pointing at garbage (measured on *Voodoo Castle*, whose table
/// runs short and reads leftover picture data past its real extent) is
/// refused rather than kept. `adventure` is the release's own AI series
/// number ([`crate::saga_us::SagaUs::adventure`]) — checked against
/// [`PICTURE_TABLE_MARKER`] before the table is trusted at all.
///
/// `None` when the marker does not match (a differently-mastered disk, or a
/// side A this table's offset does not describe) or when `side_a` is too
/// short to hold the table.
pub fn read_picture_table(
    side_a: &[u8],
    side_b_spliced: &[u8],
    scheme: FamilyCScheme,
    adventure: u16,
) -> Option<AtariPictureTable> {
    let marker = side_a.get(PICTURE_TABLE_MARKER..PICTURE_TABLE_MARKER + 3)?;
    let want = adventure as u8;
    if marker != [want, want, want] {
        return None;
    }
    let mut entries = Vec::new();
    for i in 0..PICTURE_TABLE_ENTRIES {
        let at = PICTURE_TABLE_OFFSET + 2 * i;
        let pair = side_a.get(at..at + 2)?;
        let (lo, hi) = (pair[0], pair[1]);
        if lo == 0 && hi == 0 {
            continue;
        }
        let file_offset = table_entry_file_offset(lo, hi);
        if decode_table_picture(side_b_spliced, file_offset, scheme).is_none() {
            // Refused rather than drawn — see the doc above.
            continue;
        }
        let (usage, index) = if i < 100 {
            (PictureUsage::Room, i as u16)
        } else if lo & 4 != 0 {
            (PictureUsage::ObjectInInventory, (i - 100) as u16)
        } else {
            (PictureUsage::ObjectInRoom, (i - 100) as u16)
        };
        entries.push(PictureTableEntry { usage, index, file_offset });
    }
    let inv = side_a.get(INVENTORY_BACKDROP_ENTRY..INVENTORY_BACKDROP_ENTRY + 2)?;
    let inventory_backdrop_file_offset = if inv == [0, 0] {
        None
    } else {
        let file_offset = table_entry_file_offset(inv[0], inv[1]);
        decode_table_picture(side_b_spliced, file_offset, scheme).map(|_| file_offset)
    };
    Some(AtariPictureTable { entries, inventory_backdrop_file_offset })
}

/// Decode the record a `file_offset` (from [`AtariPictureTable`]) names,
/// trying the ordinary [`record_at`]/[`decode_record`] path first and falling
/// back to [`decode_record_with_bad_sector_fallback`] only when that refuses
/// it — SQ-1498's two named exceptions on *The Count*, and nothing else on
/// any other specimen this crate has measured.
pub fn decode_table_picture(
    spliced_side_b: &[u8],
    file_offset: usize,
    scheme: FamilyCScheme,
) -> Option<Picture> {
    let at = spliced_of(file_offset);
    if let Some(rec) = record_at(spliced_side_b, at, scheme) {
        return decode_record(spliced_side_b, &rec, scheme).ok();
    }
    decode_record_with_bad_sector_fallback(spliced_side_b, at, scheme)
}

// ── SQ-1498: the two damaged Count records ──────────────────────────────────

/// Atari sector boundaries are 128-byte aligned starting at file offset 16
/// (§7.3: `file_offset = 16 + (sector - 1) * 128`). Because [`splice_vtoc`]
/// removes exactly one whole sector-aligned 128-byte span ([`VTOC_LEN`]), this
/// congruence holds in SPLICED coordinates too — every offset behind the
/// splice point shifts by exactly one sector, which does not move it off the
/// grid — so the functions below use spliced offsets throughout with no
/// special case for the splice.
const SECTOR_BASE: usize = 16;
const SECTOR_LEN: usize = 128;

/// The first sector-aligned offset at or after `off`.
fn sector_boundary_at_or_after(off: usize) -> usize {
    if off <= SECTOR_BASE {
        return SECTOR_BASE;
    }
    let rem = (off - SECTOR_BASE) % SECTOR_LEN;
    if rem == 0 {
        off
    } else {
        off + (SECTOR_LEN - rem)
    }
}

/// Find the first 128-byte-aligned sector inside `[start, end)` that is
/// either entirely zero or a byte-for-byte duplicate of an EARLIER
/// 128-byte-aligned sector inside `[start, end)` — the two damage shapes
/// SQ-1498 measured on *The Count*'s side B (a stale duplicate sector for
/// room 6, an unwritten all-zero one for room 16). Returns the sector's own
/// `[start, end)` span, in the same (spliced) coordinates as its arguments.
fn find_bad_sector(spliced: &[u8], start: usize, end: usize) -> Option<(usize, usize)> {
    let mut boundaries = Vec::new();
    let mut b = sector_boundary_at_or_after(start);
    while b + SECTOR_LEN <= end {
        boundaries.push(b);
        b += SECTOR_LEN;
    }
    for (n, &b) in boundaries.iter().enumerate() {
        let span = spliced.get(b..b + SECTOR_LEN)?;
        if span.iter().all(|&x| x == 0) {
            return Some((b, b + SECTOR_LEN));
        }
        if boundaries[..n].iter().any(|&a| spliced.get(a..a + SECTOR_LEN) == Some(span)) {
            return Some((b, b + SECTOR_LEN));
        }
    }
    None
}

/// A best-effort decode of a table-named record [`record_at`] could not
/// verify because exactly one 128-byte sector of its data is damaged
/// (SQ-1498) — reached only from [`decode_table_picture`], for a
/// `record_offset` the picture table names but the ordinary scan refuses.
/// **Not a relaxation of [`record_at`]'s own check**, which is untouched and
/// stays exactly as strict for every other record on every other disk; this
/// is a second, explicitly-named path taken only after the first one fails.
///
/// The recipe (measured on the one specimen this crate has, *The Count*'s
/// side B, SQ-1498's own investigation note): parse the header exactly as
/// [`record_at`] does; find the first bad sector in the record's data
/// ([`find_bad_sector`]); decode the bytes BEFORE it forward from pair 0,
/// which is the intact head painted at its own correct position; decode the
/// bytes AFTER it forward too, but ANCHORED so its own pairs land at the END
/// of the region ([`paint_strips_from`] with a `start_pair` computed from how
/// many pairs the tail itself holds) — the run-length stream's control-byte
/// phase resumes cleanly right after the missing sector on this specimen, so
/// the tail decodes correctly once it is placed at the position it would
/// have painted had the sector not been lost, not the position it happens to
/// start at. The two decodes are composited onto one canvas the same way
/// [`crate::graphics`]'s own object-overlay compositor works: the tail's own
/// painted rectangle is copied over the head's canvas, nothing more. The gap
/// between them — the lost sector's own pairs, which cannot be recovered —
/// renders as a black stripe a few columns wide, the honest artifact of one
/// missing sector rather than a defect in the reading.
pub fn decode_record_with_bad_sector_fallback(
    spliced: &[u8],
    offset: usize,
    scheme: FamilyCScheme,
) -> Option<Picture> {
    let head = spliced.get(offset..offset + 10)?;
    let size = usize::from(u16::from_le_bytes([head[0], head[1]]));
    if !(13..=MAX_RECORD).contains(&size) || offset + size > spliced.len() {
        return None;
    }
    if head[2] > MAX_COLUMN || head[4] > MAX_COLUMN || head[5] > MAX_ROW {
        return None;
    }
    let layout = StripLayout::resolve(
        i32::from(head[2]),
        i32::from(head[3]),
        i32::from(head[4]),
        i32::from(head[5]),
        scheme,
    )
    .ok()?;
    if layout.pair_count() > MAX_PAIRS {
        return None;
    }
    if layout.left + layout.cols * 8 <= 0 || layout.left >= CANVAS_WIDTH as i32 {
        return None;
    }

    let data_start = offset + 10;
    let data_end = offset + size;
    let (bad_start, bad_end) = find_bad_sector(spliced, data_start, data_end)?;
    let prefix = &spliced[data_start..bad_start];
    let suffix = &spliced[bad_end..data_end];

    let prefix_paint = paint_strips(prefix, &layout, scheme);

    // Measure the tail's own pair count with a probe layout that never caps
    // or wraps (one column, an effectively unbounded pair count), then decode
    // it again, for real, anchored so its own pairs land at the end of the
    // region rather than at the start.
    let probe = StripLayout { left: 0, top: 0, cols: 1, pairs: i32::MAX };
    let tail_pairs = paint_strips(suffix, &probe, scheme).emitted;
    let need = layout.pair_count();
    let anchor = need.saturating_sub(tail_pairs);
    let suffix_paint = paint_strips_from(suffix, &layout, scheme, anchor);

    let mut pixels = prefix_paint.pixels;
    if let Some(area) = suffix_paint.bounds {
        for y in area.top()..=area.bottom() {
            for x in area.left()..=area.right() {
                pixels[y * CANVAS_WIDTH + x] = suffix_paint.pixels[y * CANVAS_WIDTH + x];
            }
        }
    }
    let painted = match (prefix_paint.bounds, suffix_paint.bounds) {
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
        (Some(a), Some(b)) => Some(Painted {
            left: a.left().min(b.left()),
            top: a.top().min(b.top()),
            right: a.right().max(b.right()),
            bottom: a.bottom().max(b.bottom()),
        }),
    };
    let colour_bytes = [head[6], head[7], head[8], head[9]];
    let (palette, unrecognised_colours) =
        resolve_palette(colour_bytes, |stored| Some(atari_colour(stored)));
    Some(Picture { width: CANVAS_WIDTH, height: CANVAS_HEIGHT, pixels, palette, colour_bytes, unrecognised_colours, painted })
}

// ── The line-art sides' own (usage, index) table (SQ-1524) ─────────────────
//
// Measured identically on all four `AtariPictureFormat::LineArt` titles: side
// B file offset 0x290 holds the same three-copies-of-the-adventure-number
// marker the bitmap sides carry (there, on side A), and 0x293 holds 190
// two-byte entries in the same room-then-object shape — but on a THREE-byte
// grid rather than the bitmap table's seven, because a line-art record's own
// tokens are three bytes each. See the module docs for the full account.

/// Side B file offset of the marker in front of the line-art table — three
/// copies of the release's own Adventure International number (`01`
/// Adventureland, `02` Pirate Adventure, `03` Mission Impossible, `06`
/// Strange Odyssey).
pub const LINE_ART_TABLE_MARKER: usize = 0x290;

/// Side B file offset of the line-art table itself, immediately after
/// [`LINE_ART_TABLE_MARKER`]'s three bytes.
pub const LINE_ART_TABLE_OFFSET: usize = 0x293;

/// How many two-byte entries the line-art table holds: 100 room slots, 90
/// object slots.
pub const LINE_ART_TABLE_ENTRIES: usize = 190;

/// Where a line-art table entry `[a, s]` points, as a FILE offset into side B
/// (SQ-1524): `s` and the low two bits of `a` are a 1-based, 128-byte Atari
/// sector number, and the top six bits of `a`, times **three**, are the byte
/// within it — a line-art record's tokens are three bytes each, where
/// [`table_entry_file_offset`]'s bitmap units are seven.
pub fn line_art_entry_file_offset(a: u8, s: u8) -> usize {
    let sector = (usize::from(a & 3) << 8) | usize::from(s);
    16 + sector.saturating_sub(1) * 128 + usize::from(a >> 2) * 3
}

/// One resolved (usage, index) → record association, read off a line-art
/// side's own table (SQ-1524).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LineArtTableEntry {
    /// What the named record is for. Every object slot is stored as
    /// [`PictureUsage::ObjectInRoom`] — see [`LineArtPictureTable::find`] for
    /// why an [`PictureUsage::ObjectInInventory`] lookup still finds it.
    pub usage: PictureUsage,
    /// The picture index — a room number for [`PictureUsage::Room`], an item
    /// number otherwise.
    pub index: u16,
    /// The record's offset into the whole `.atr` FILE, not the spliced side —
    /// pass through [`spliced_of`] before calling [`record_at`]-style code
    /// directly, or hand it straight to [`draw_line_art_record`], which does
    /// that already.
    pub file_offset: usize,
}

/// A line-art side's whole picture table (SQ-1524): every (usage, index)
/// association [`read_line_art_table`] could verify against a record
/// [`draw_line_art_record`] can actually play.
///
/// Unlike the bitmap titles' [`AtariPictureTable`], there is no separate
/// fixed inventory-backdrop pointer and no inventory flag bit: index 98 is an
/// ordinary room-usage entry, and an object picture here serves both
/// [`PictureUsage::ObjectInRoom`] and [`PictureUsage::ObjectInInventory`] —
/// exactly as the same four titles' Apple II `B<aa><nnn>` files, one per
/// object with no R/I suffix, already do.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LineArtPictureTable {
    entries: Vec<LineArtTableEntry>,
}

impl LineArtPictureTable {
    /// Every resolved (usage, index) association, in table order.
    pub fn entries(&self) -> &[LineArtTableEntry] {
        &self.entries
    }

    /// The record a picture with this `usage` and `index` names, as a FILE
    /// offset — pass to [`draw_line_art_record`] — or `None` when the table
    /// carries no such entry.
    ///
    /// A [`PictureUsage::ObjectInRoom`] and a [`PictureUsage::ObjectInInventory`]
    /// lookup at the same `index` answer the same record — see the struct
    /// doc for why — so only [`PictureUsage::Room`] is checked exactly;
    /// anything else is "an object slot", regardless of which of the two
    /// object usages was asked for.
    pub fn find(&self, usage: PictureUsage, index: u16) -> Option<usize> {
        let want_room = matches!(usage, PictureUsage::Room);
        self.entries
            .iter()
            .find(|e| e.index == index && matches!(e.usage, PictureUsage::Room) == want_room)
            .map(|e| e.file_offset)
    }
}

/// Read a line-art side's own picture table (SQ-1524).
///
/// `side_b_spliced` is the companion side with the volume table of contents
/// already excised ([`splice_vtoc`]) — unlike the bitmap titles' table, this
/// one lives on side B itself, so there is no separate side-A argument.
/// `adventure` is the release's own AI series number
/// ([`crate::saga_us::SagaUs::adventure`]) — checked against
/// [`LINE_ART_TABLE_MARKER`] before the table is trusted at all. Every entry
/// is validated by actually playing it, on a throwaway canvas, through
/// [`draw_line_art_record`] before it is kept — refused rather than drawn,
/// the same rule [`read_picture_table`] holds its own entries to.
///
/// `None` when the marker does not match (a differently-mastered disk) or
/// when `side_b_spliced` is too short to hold the table.
pub fn read_line_art_table(side_b_spliced: &[u8], adventure: u16) -> Option<LineArtPictureTable> {
    let marker = side_b_spliced.get(LINE_ART_TABLE_MARKER..LINE_ART_TABLE_MARKER + 3)?;
    let want = adventure as u8;
    if marker != [want, want, want] {
        return None;
    }
    let mut entries = Vec::new();
    for i in 0..LINE_ART_TABLE_ENTRIES {
        let at = LINE_ART_TABLE_OFFSET + 2 * i;
        let pair = side_b_spliced.get(at..at + 2)?;
        let (a, s) = (pair[0], pair[1]);
        if a == 0 && s == 0 {
            continue;
        }
        let file_offset = line_art_entry_file_offset(a, s);
        let mut probe = LineArtCanvas::new();
        if draw_line_art_record(&mut probe, side_b_spliced, file_offset).is_err() {
            // Refused rather than drawn — see the doc above.
            continue;
        }
        let (usage, index) = if i < 100 {
            (PictureUsage::Room, i as u16)
        } else {
            (PictureUsage::ObjectInRoom, (i - 100) as u16)
        };
        entries.push(LineArtTableEntry { usage, index, file_offset });
    }
    Some(LineArtPictureTable { entries })
}

/// Play the record at `file_offset` (from [`LineArtPictureTable`]) onto
/// `canvas`, **in place** — not a fresh decode.
///
/// The state [`LineArtCanvas`] carries between plays (its two bitmaps, the
/// fill colours, the pen) is exactly how an object picture draws over its own
/// room on the real machine, and how the six room records SQ-1525's own note
/// names never clear the screen: they draw over whatever `canvas` already
/// held. A caller that hands this a fresh [`LineArtCanvas`] every time gets a
/// black canvas under those six instead of the previous picture — see the
/// module docs.
///
/// # Errors
///
/// [`LineArtError`] from the play itself (a malformed record — refused by
/// [`read_line_art_table`] already, for every entry it kept, but a caller
/// reaching this directly from a raw file offset has no such guarantee), or a
/// [`LineArtError::Truncated`] when `file_offset` (through [`spliced_of`])
/// does not name a byte inside `side_b_spliced` at all.
pub fn draw_line_art_record(
    canvas: &mut LineArtCanvas,
    side_b_spliced: &[u8],
    file_offset: usize,
) -> Result<LineArtPicture, LineArtError> {
    let at = spliced_of(file_offset);
    let stream = side_b_spliced
        .get(at..)
        .ok_or(LineArtError::Truncated { len: side_b_spliced.len().saturating_sub(at) })?;
    canvas.draw(stream)
}

/// The picture a host should SHOW for the shared darkness card (line-art
/// [`PictureUsage::Room`] index 0, the same record on all four titles) —
/// SQ-1525's own note: the record is an animation — `IT'S TOO DARK!`
/// lettering and a pair of eyes across three pauses — that ends with a plain
/// clear to black, so the frame worth showing a player is the LAST one the
/// animation paused on, not the true final (black) frame.
///
/// Plays the record onto `canvas` **in place**, exactly as
/// [`draw_line_art_record`] does — so its own true end state (genuinely
/// black) is what the next record draws over, matching the real machine —
/// but returns the earlier snapshot instead of the final one. A record with
/// no pause at all (not the case for any specimen measured, but not assumed
/// impossible either) answers with its one and only frame.
///
/// # Errors
///
/// As [`draw_line_art_record`].
pub fn draw_darkness_card(
    canvas: &mut LineArtCanvas,
    side_b_spliced: &[u8],
    file_offset: usize,
) -> Result<LineArtPicture, LineArtError> {
    let at = spliced_of(file_offset);
    let stream = side_b_spliced
        .get(at..)
        .ok_or(LineArtError::Truncated { len: side_b_spliced.len().saturating_sub(at) })?;
    let mut frames = canvas.draw_frames(stream)?;
    let shown = if frames.len() >= 2 { frames.len() - 2 } else { frames.len() - 1 };
    Ok(frames.swap_remove(shown))
}

/// [`draw_line_art_record`], also appending every line the record actually
/// committed to `lines` (SQ-1526) — [`LineArtCanvas::draw_tracking`] in
/// place of [`LineArtCanvas::draw`]. A caller compositing a room record and
/// its object overlays onto one canvas for a SINGLE scale-aware render
/// (`app::graphics::PictSource::scott_line_art_composite`) hands the SAME
/// `lines` accumulator to every call and then supersamples once, through
/// [`LineArtCanvas::picture_from_lines_at`], so the room's and every
/// object's own strokes all inform it — see [`LineArtCanvas::draw_tracking`]'s
/// own doc for why this is a caller-supplied accumulator rather than state
/// the canvas keeps itself.
///
/// # Errors
///
/// As [`draw_line_art_record`].
pub fn draw_line_art_record_tracking(
    canvas: &mut LineArtCanvas,
    side_b_spliced: &[u8],
    file_offset: usize,
    lines: &mut Vec<CommittedLine>,
) -> Result<LineArtPicture, LineArtError> {
    let at = spliced_of(file_offset);
    let stream = side_b_spliced
        .get(at..)
        .ok_or(LineArtError::Truncated { len: side_b_spliced.len().saturating_sub(at) })?;
    canvas.draw_tracking(stream, lines)
}

/// [`draw_darkness_card`] at `scale` device pixels per native pixel
/// (SQ-1526) — [`LineArtCanvas::draw_frames_at`] in place of
/// [`LineArtCanvas::draw_frames`], so the paused frame it hands back is
/// supersampled from exactly the lines that frame's own snapshot shows
/// (`LineArtCanvas::draw_frames_at`'s own doc), not a naive re-walk of the
/// whole record.
///
/// # Errors
///
/// As [`draw_line_art_record`].
pub fn draw_darkness_card_at(
    canvas: &mut LineArtCanvas,
    side_b_spliced: &[u8],
    file_offset: usize,
    scale: u32,
) -> Result<LineArtPicture, LineArtError> {
    let at = spliced_of(file_offset);
    let stream = side_b_spliced
        .get(at..)
        .ok_or(LineArtError::Truncated { len: side_b_spliced.len().saturating_sub(at) })?;
    let mut frames = canvas.draw_frames_at(stream, scale)?;
    let shown = if frames.len() >= 2 { frames.len() - 2 } else { frames.len() - 1 };
    Ok(frames.swap_remove(shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one Atari record: the ten-byte header, then no-literal units.
    fn record(size_fudge: i32, edges: [u8; 4], colours: [u8; 4], units: &[(u8, u8, u8)]) -> Vec<u8> {
        let len = 10 + units.len() * 3;
        let declared = (len as i32 + size_fudge) as u16;
        let mut out = declared.to_le_bytes().to_vec();
        out.extend_from_slice(&edges);
        out.extend_from_slice(&colours);
        for (count, hi, lo) in units {
            out.extend_from_slice(&[*count, *hi, *lo]);
        }
        out
    }

    /// A side with `lead` filler bytes, then the records, then trailing zeros,
    /// padded out past the volume table so the splice has something to do.
    fn side(lead: usize, records: &[Vec<u8>], gaps: &[usize]) -> Vec<u8> {
        let mut out = vec![0u8; FIRST_RECORD + lead];
        for (n, rec) in records.iter().enumerate() {
            out.extend_from_slice(rec);
            out.extend(std::iter::repeat_n(0u8, gaps.get(n).copied().unwrap_or(0)));
        }
        out.resize(SIDE_LEN, 0);
        out
    }

    // A hand-built side, with the record shape this module measured and the
    // pixels worked out from §8.3's rules. Two columns of four pairs at
    // x 0..16, y 0..8 (the exclusive reading of edges 3, 0, 5, 8), so eight
    // pairs and two units.
    #[test]
    fn the_scan_finds_a_hand_built_record_and_decodes_its_pixels() {
        let rec = record(
            0,
            [3, 0, 5, 8],
            [0x36, 0x87, 0x0E, 0x00],
            &[(4, 0xFF, 0xFF), (4, 0x00, 0x55)],
        );
        assert_eq!(rec.len(), 16, "ten header bytes and two three-byte units");
        assert_eq!(usize::from(u16::from_le_bytes([rec[0], rec[1]])), 16, "size covers the header");
        let disk = side(7, std::slice::from_ref(&rec), &[]);

        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1, "exactly one record on the side");
        let r = &found[0];
        assert_eq!(r.offset, FIRST_RECORD + 7);
        assert_eq!(r.file_offset(), FIRST_RECORD + 7, "in front of the volume table");
        assert_eq!(r.size, 16);
        assert_eq!(r.decoded_len, 16, "the data ends where the size says");
        assert_eq!((r.layout.cols, r.layout.pairs), (2, 4));
        assert_eq!(r.colour_bytes, [0x36, 0x87, 0x0E, 0x00]);

        let spliced = splice_vtoc(&disk);
        let pic = decode_record(&spliced, r, FamilyCScheme::NoLiteral).expect("decodes");
        let px = |x: usize, y: usize| pic.pixels[y * CANVAS_WIDTH + x];
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(px(x, y), 3, "column one is all value 3 at ({x}, {y})");
            }
        }
        // (0x00, 0x55): even rows all value 0, odd rows 0b01 four times.
        for y in [0usize, 2, 4, 6] {
            for x in 8..16 {
                assert_eq!(px(x, y), 0, "column two, even row {y}");
                assert_eq!(px(x, y + 1), 1, "column two, odd row {}", y + 1);
            }
        }
        assert_eq!(px(16, 0), 0, "nothing right of the record");
        // The palette resolves through the hardware table and nothing is left
        // unnamed, which is the Atari half of §8.3's colour rule.
        assert_eq!(pic.palette[0], (0, 0, 0), "entry 0 is black however the record reads");
        assert_eq!(pic.palette[3], (0xE0, 0xE0, 0xE0), "0x0E is the grey row's luminance 14");
        assert!(pic.unrecognised_colours.is_empty(), "every Atari byte has a colour");
    }

    // Records are laid end to end with nought to six bytes of filler between
    // them; the scan re-finds each one rather than adding sizes.
    #[test]
    fn the_scan_steps_over_the_filler_between_records() {
        let a = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let b = record(0, [5, 0, 8, 4], [0x36, 0x87, 0x0E, 0], &[(6, 0x55, 0x55)]);
        let c = record(1, [3, 10, 6, 16], [0x94, 0x0E, 0, 0], &[(9, 0xAA, 0xAA)]);
        let disk = side(0, &[a, b, c], &[3, 6, 0]);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 3, "all three, filler notwithstanding");
        assert_eq!(found[0].offset, FIRST_RECORD);
        assert_eq!(found[1].offset, FIRST_RECORD + 16 + 3, "three bytes of filler");
        assert_eq!(found[2].offset, FIRST_RECORD + 16 + 3 + 13 + 6, "and six more");
        assert_eq!((found[1].layout.cols, found[1].layout.pairs), (3, 2));
        assert_eq!((found[2].layout.cols, found[2].layout.pairs), (3, 3));
        // The last one declares one byte more than it needs, which is what 44
        // of the 242 real records do.
        assert_eq!(found[2].size, found[2].decoded_len + 1);
    }

    // The consistency check is what makes the scan safe, so each of its limbs
    // needs a case that fails it.
    #[test]
    fn a_region_the_data_does_not_fill_is_not_a_record() {
        // One unit short of the region.
        let short = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&short), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
        // The right pairs, but a size two bytes past where they end — the
        // twelve-byte-header reading of §8.3, which is what this refutes.
        let fat = record(2, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&fat), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
        // An edge far off the canvas.
        let wide = record(0, [3, 0, 40, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF)]);
        let spliced = splice_vtoc(&side(0, std::slice::from_ref(&wide), &[]));
        assert_eq!(record_at(&spliced, FIRST_RECORD, FamilyCScheme::NoLiteral), None);
    }

    // Same bytes, other scheme: the reading is the release's and the data does
    // not tell you which it is (which is why nothing sniffs).
    #[test]
    fn the_scheme_is_the_releases_and_the_wrong_one_finds_nothing() {
        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let disk = side(0, std::slice::from_ref(&rec), &[]);
        assert_eq!(scan_picture_side(&disk, FamilyCScheme::NoLiteral).len(), 1);
        assert!(scan_picture_side(&disk, FamilyCScheme::Standard).is_empty());
    }

    // §7.3's splice: the volume table is excised, and a record behind it keeps
    // a file offset that names where it really is on the disk.
    #[test]
    fn the_volume_table_is_excised_and_file_offsets_survive_it() {
        let mut disk = vec![0u8; SIDE_LEN];
        for (n, b) in disk.iter_mut().enumerate().take(VTOC_OFFSET + VTOC_LEN) {
            if n >= VTOC_OFFSET {
                *b = 0xFF;
            }
        }
        let spliced = splice_vtoc(&disk);
        assert_eq!(spliced.len(), SIDE_LEN - VTOC_LEN);
        assert!(spliced.iter().all(|&b| b == 0), "the 0xFF sector is gone");

        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        // Put it a little way behind the volume table, in FILE coordinates.
        let mut disk = vec![0u8; SIDE_LEN];
        let file_at = VTOC_OFFSET + VTOC_LEN + 32;
        disk[file_at..file_at + rec.len()].copy_from_slice(&rec);
        disk[VTOC_OFFSET..VTOC_OFFSET + VTOC_LEN].fill(0xEE);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].offset, file_at - VTOC_LEN, "spliced coordinates");
        assert_eq!(found[0].file_offset(), file_at, "and back to the file's");
    }

    // A record that spans the volume table decodes across it, which is the
    // whole point of working in spliced coordinates.
    #[test]
    fn a_record_spanning_the_volume_table_decodes_across_it() {
        let rec = record(0, [3, 0, 5, 8], [0x36, 0x87, 0x0E, 0], &[(4, 0xFF, 0xFF), (4, 0, 0)]);
        let mut disk = vec![0u8; SIDE_LEN];
        // Straddle it: four bytes of header in front, the rest behind.
        let file_at = VTOC_OFFSET - 4;
        disk[file_at..file_at + 4].copy_from_slice(&rec[..4]);
        disk[VTOC_OFFSET..VTOC_OFFSET + VTOC_LEN].fill(0xEE);
        let behind = VTOC_OFFSET + VTOC_LEN;
        disk[behind..behind + rec.len() - 4].copy_from_slice(&rec[4..]);
        let found = scan_picture_side(&disk, FamilyCScheme::NoLiteral);
        assert_eq!(found.len(), 1, "one record, read straight through the splice");
        assert_eq!(found[0].file_offset(), file_at);
        let spliced = splice_vtoc(&disk);
        let pic = decode_record(&spliced, &found[0], FamilyCScheme::NoLiteral).expect("decodes");
        assert_eq!(pic.pixels[0], 3, "and its first stored pixel is value 3");
    }
}
