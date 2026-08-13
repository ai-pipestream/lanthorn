//! Read a Macintosh disk image — a DiskCopy 4.2 `.image` or a bare HFS volume —
//! well enough to pull a story file and its native Infocom picture archive
//! straight off the original release media, with no extraction step.
//!
//! This is the Macintosh sibling of [`crate::adf`], and it exists for the same
//! reason: Infocom shipped these games on floppies, and a floppy is a container
//! babelmap can open rather than something a person has to unpack first.
//! Verified against `Zork Zero Disk.image` — Zork Zero, **version 6, release
//! 296, serial 881019**, a build 97 releases earlier than the r393/890714 that
//! every PC medium in the corpus carries.
//!
//! macOS itself is no help here: `hdiutil attach` refuses an HFS-standard image
//! on anything past 10.14, so the whole chain has to be ours. It is three layers
//! deep and every one of them is hand-rolled, exactly as `blorb`'s zero
//! dependencies require.
//!
//! # Layer 1 — the DiskCopy 4.2 wrapper
//!
//! An 84-byte header, then the volume, then the tags:
//!
//! | offset | field |
//! |---|---|
//! | `0x00` | 64-byte Pascal disk name |
//! | `0x40` | data size, big-endian u32 — the volume |
//! | `0x44` | tag size, big-endian u32 |
//! | `0x50` | encoding |
//! | `0x51` | format |
//! | `0x52` | `0x0100`, the format's magic |
//!
//! **The tag bytes are not part of the volume.** Apple's 800K floppies carried
//! 12 bytes of tag per 512-byte sector, so a 1600-sector disk drags 19 200 bytes
//! along behind its 819 200, and a reader that folds them into the filesystem
//! shifts every offset past the end of the data. Take the volume as exactly the
//! `data size` bytes that follow the header, and ignore the rest.
//!
//! A bare volume with no wrapper is read just the same — see
//! [`Hfs::looks_like_hfs`], which tries both placements and lets the volume's own
//! signature decide.
//!
//! # Layer 2 — HFS, which is not MFS
//!
//! The signature two logical blocks in (volume offset 1024) is `0x4244`, `BD`:
//! HFS, with a B*-tree catalog. Flat MFS would say `0xD2D7`, and nothing here
//! would read it. The Master Directory Block that carries that signature also
//! carries the geometry every offset below is computed from — the allocation
//! block size, where the allocation blocks start, and the extents of the two
//! special files:
//!
//! * the **catalog file**, whose leaf records name every file and folder;
//! * the **extents overflow file**, which holds the extents of any fork too
//!   fragmented for the three the catalog record has room for.
//!
//! Both are B*-trees of 512-byte nodes: a header node whose first record names
//! the first leaf, then a linked chain of leaves. Reading them means walking
//! that chain — no key comparisons, no tree descent, because we want *every*
//! record rather than one.
//!
//! Structure comes from Apple's *Inside Macintosh: Files*, chapter 2 ("Data
//! Organization on Volumes"), which lays out the MDB, the B*-tree node and the
//! catalog and extents records field by field.
//!
//! # Layer 3 — choosing what to run
//!
//! Files are enumerated flat: the catalog names a parent for every record, and
//! this reader simply does not care which folder something is in. That is the
//! same choice [`crate::adf`] makes and for the same reason — the story is
//! identified by CONTENT ([`crate::adf::looks_like_story`], and
//! [`crate::infocom_pics::InfocomPics::parse`] for the artwork), so where it
//! sits is not a question anyone has to answer. Infocom's `Story.data` /
//! `Pic.data` names are a tiebreak, never a test.
//!
//! # What is on the Macintosh Zork Zero disk
//!
//! Measured, and worth writing down because it is the whole reason to look:
//!
//! | file | type/creator | data fork | what it is |
//! |---|---|---|---|
//! | `Story.data` | `INdf`/`IN0Z` | 295 936 | the story — v6, r296, s881019 |
//! | `CPic.data` | `INdf`/`IN0Z` | 218 624 | **colour** artwork, 483 records, 14-byte |
//! | `Pic.data` | `INdf`/`IN0Z` | 239 104 | **monochrome** artwork, 483 records, 12-byte |
//! | `Zork Zero` | `APPL`/`IN0Z` | 0 (38 833 rsrc) | Infocom's own 68k interpreter |
//! | `Desktop` | `FNDR`/`ERIK` | 0 (1 665 rsrc) | the Finder's desktop database |
//!
//! So the Macintosh release ships **two** picture archives, one per screen the
//! machine had. The colour one is an ordinary big-endian Infocom archive and
//! [`InfocomPics`] reads all 386 of its pictures today. The monochrome one
//! declares 12-byte directory records — no palette to store, on a screen with
//! two colours — which the Amiga/Mac reader rejects as a container it does not
//! know. That is the same file bocfel identifies by its header flags reading
//! `0x0e`, and supporting it is a separate piece of work from opening the disk.
//! [`Hfs::pictures`] therefore lands on the colour archive here, by parse rather
//! than by preference.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;

/// A logical block: the unit the MDB and the B*-tree nodes are measured in.
pub const BLOCK: usize = 512;

/// Bytes of DiskCopy 4.2 header ahead of the volume.
const DISKCOPY_HEADER: usize = 84;
/// `dataSize` — the volume's length in bytes.
const DISKCOPY_DATA_SIZE: usize = 0x40;
/// `tagSize` — the sector tags, which FOLLOW the volume and are not part of it.
const DISKCOPY_TAG_SIZE: usize = 0x44;
/// `private`, the format's magic number.
const DISKCOPY_MAGIC_OFF: usize = 0x52;
const DISKCOPY_MAGIC: u16 = 0x0100;

/// The Master Directory Block sits two logical blocks into the volume.
const MDB_OFFSET: usize = 2 * BLOCK;
/// `drSigWord` for HFS. (MFS, which this reader does not handle, is `0xD2D7`.)
const HFS_SIGNATURE: u16 = 0x4244;

// MDB fields, from *Inside Macintosh: Files*, "Master Directory Blocks".
const MDB_NM_AL_BLKS: usize = 18; // drNmAlBlks — allocation blocks on the volume
const MDB_AL_BLK_SIZ: usize = 20; // drAlBlkSiz — bytes per allocation block
const MDB_AL_BL_ST: usize = 28; // drAlBlSt — first allocation block, in 512-byte blocks
const MDB_VN: usize = 36; // drVN — volume name, Pascal, ≤27 bytes
const MDB_XT_FL_SIZE: usize = 130; // drXTFlSize — extents overflow file size
const MDB_XT_EXT_REC: usize = 134; // drXTExtRec — its first three extents
const MDB_CT_FL_SIZE: usize = 146; // drCTFlSize — catalog file size
const MDB_CT_EXT_REC: usize = 150; // drCTExtRec — its first three extents
/// Bytes of MDB this reader reads.
const MDB_LEN: usize = 162;

/// Every B*-tree node on an HFS volume is one logical block.
const NODE_SIZE: usize = BLOCK;
/// `ndFLink`, `ndBLink`, `ndType`, `ndNHeight`, `ndNRecs`, `ndResv2`.
const NODE_HEADER: usize = 14;
/// `ndNRecs`, the record count, within that header.
const ND_N_RECS: usize = 10;
/// `bthFNode` — the first leaf node — within a header node's first record.
const BTH_F_NODE: usize = 10;

/// `cdrFilRec`: the catalog record for a plain file.
const CDR_FILE: u8 = 2;
/// The catalog file's own CNID, which is how its overflow extents are keyed.
const CATALOG_CNID: u32 = 4;

// Catalog file-record fields, offset from `cdrType`.
const FIL_TYPE: usize = 4; // filUsrWds.fdType
const FIL_CREATOR: usize = 8; // filUsrWds.fdCreator
const FIL_FL_NUM: usize = 20; // filFlNum — the file's CNID
const FIL_LG_LEN: usize = 26; // filLgLen — data fork logical length
const FIL_R_LG_LEN: usize = 36; // filRLgLen — resource fork logical length
const FIL_EXT_REC: usize = 74; // filExtRec — first three data-fork extents
const FILE_REC_LEN: usize = 102;

/// `xkrFkType` for a data fork. (A resource fork is `0xFF`; this reader reads
/// only data forks, so an overflow record for one is skipped.)
const FORK_DATA: u8 = 0x00;
/// Bytes of extents-overflow key ahead of the record's own data.
const XKR_LEN: usize = 7;

/// One extent: a starting allocation block and a count.
type Extent = (u16, u16);
/// The three extents a catalog record or an overflow record carries.
type ExtentRecord = [Extent; 3];

/// Infocom's conventional names on a release disk. Never a test — only a
/// tiebreak when content identification finds more than one candidate. The Mac
/// ships both `Pic.data` and `CPic.data`, so both count as conventional.
const CONVENTIONAL_STORY: &str = "story.data";
const CONVENTIONAL_PICTURES: [&str; 2] = ["pic.data", "cpic.data"];

/// Errors that can arise while mounting a Macintosh disk image.
#[derive(Debug, PartialEq, Eq)]
pub enum HfsError {
    /// The bytes are not an HFS volume, wrapped or bare: no `BD` signature
    /// where one has to be, or a Master Directory Block whose geometry does not
    /// describe the image it sits in.
    NotHfs,
}

/// One file found on the volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfsEntry {
    /// The Macintosh filename, as stored. See [`mac_name`] on the upper half.
    pub name: String,
    /// Data-fork size in bytes, from the catalog record.
    pub size: usize,
    /// Resource-fork size in bytes. Reported so a listing can say what a file
    /// is — a Macintosh application is *all* resource fork — but not read: an
    /// Infocom release keeps its story and its artwork in data forks.
    pub resource_size: usize,
    /// The Finder type, e.g. `INdf` for Infocom's data files, `APPL` for an
    /// application.
    pub file_type: [u8; 4],
    /// The Finder creator, e.g. `IN0Z` for Zork Zero.
    pub creator: [u8; 4],
    /// The catalog node id, unique on the volume, and the key its overflow
    /// extents are stored under.
    pub id: u32,
    /// The first three data-fork extents; any more live in the overflow file.
    extents: ExtentRecord,
}

/// A mounted Macintosh volume.
#[derive(Debug)]
pub struct Hfs {
    image: Vec<u8>,
    /// Where the volume starts in `image`: 0 bare, 84 inside a DiskCopy wrapper.
    volume: usize,
    /// Bytes per allocation block.
    alloc_size: usize,
    /// First allocation block, in 512-byte logical blocks from the volume start.
    alloc_start: usize,
    /// Allocation blocks on the volume.
    alloc_count: usize,
    name: String,
    files: Vec<HfsEntry>,
    /// Extents overflow records for data forks, as `(cnid, first block, extents)`.
    overflow: Vec<(u32, u16, ExtentRecord)>,
}

impl Hfs {
    /// Cheap sniff: does this look like a Macintosh disk image?
    ///
    /// By CONTENT — the `.image` extension the fixture happens to carry means
    /// nothing in particular, and this corpus has taught four separate times
    /// that a filename is not a format. The volume's own signature decides, and
    /// its geometry has to describe the image it sits in, so the two-block
    /// header a DiskCopy wrapper adds cannot be mistaken for a bare volume or
    /// the other way round. A Z-machine, Glulx, Blorb, Scott or AmigaDOS image
    /// can never collide: none of them has `BD` a kilobyte in.
    pub fn looks_like_hfs(bytes: &[u8]) -> bool {
        volume_offset(bytes).is_some()
    }

    /// Mount an image and enumerate its files.
    pub fn mount(image: Vec<u8>) -> Result<Hfs, HfsError> {
        let volume = volume_offset(&image).ok_or(HfsError::NotHfs)?;
        let mdb = &image[volume + MDB_OFFSET..volume + MDB_OFFSET + MDB_LEN];
        let mut hfs = Hfs {
            alloc_size: be32(mdb, MDB_AL_BLK_SIZ) as usize,
            alloc_start: usize::from(be16(mdb, MDB_AL_BL_ST)),
            alloc_count: usize::from(be16(mdb, MDB_NM_AL_BLKS)),
            name: mac_name(&mdb[MDB_VN..]).unwrap_or_default(),
            volume,
            image,
            files: Vec::new(),
            overflow: Vec::new(),
        };

        // The extents overflow file can only be described by the MDB — it is
        // where every OTHER file's extra extents live, so it cannot have any of
        // its own. Read it first; the catalog may need it.
        let mdb = hfs.mdb().to_vec();
        let xt_size = be32(&mdb, MDB_XT_FL_SIZE) as usize;
        let xt = hfs.read_extents(&extent_record(&mdb, MDB_XT_EXT_REC), xt_size);
        hfs.overflow = overflow_records(&xt);
        let ct = extent_record(&mdb, MDB_CT_EXT_REC);
        let catalog = hfs
            .read_fork(CATALOG_CNID, ct, be32(&mdb, MDB_CT_FL_SIZE) as usize)
            .ok_or(HfsError::NotHfs)?;
        hfs.files = catalog_files(&catalog);
        Ok(hfs)
    }

    /// The volume's name, as the Finder showed it.
    pub fn volume_name(&self) -> &str {
        &self.name
    }

    /// Every file on the volume, in catalog order. Folders are not listed: the
    /// scan is flat, so a file in one is found without recursing into it.
    pub fn files(&self) -> &[HfsEntry] {
        &self.files
    }

    fn mdb(&self) -> &[u8] {
        &self.image[self.volume + MDB_OFFSET..self.volume + MDB_OFFSET + MDB_LEN]
    }

    /// One allocation block, or `None` if it is off the end of the volume.
    fn alloc_block(&self, n: usize) -> Option<&[u8]> {
        if n >= self.alloc_count {
            return None;
        }
        let at = self.volume + self.alloc_start * BLOCK + n * self.alloc_size;
        self.image.get(at..at + self.alloc_size)
    }

    /// Concatenate `extents`, stopping once `limit` bytes are in hand. A run
    /// that leaves the volume ends the read where it is; the caller checks the
    /// length it got.
    fn read_extents(&self, extents: &ExtentRecord, limit: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for (start, count) in extents {
            for i in 0..usize::from(*count) {
                if out.len() >= limit {
                    return out;
                }
                let Some(b) = self.alloc_block(usize::from(*start) + i) else {
                    return out;
                };
                out.extend_from_slice(b);
            }
        }
        out
    }

    /// Read one data fork whole: its first three extents, then as many overflow
    /// records as it takes. `None` when the chain runs short of `size`.
    fn read_fork(&self, cnid: u32, first: ExtentRecord, size: usize) -> Option<Vec<u8>> {
        let mut out = self.read_extents(&first, size);
        // Each pass consumes one extent record, and a fork cannot need more of
        // them than the overflow file holds in total.
        for _ in 0..=self.overflow.len() {
            if out.len() >= size {
                out.truncate(size);
                return Some(out);
            }
            // Overflow records are keyed by the fork-relative allocation block
            // the record starts at, which is exactly what has been read so far.
            let next = u16::try_from(out.len() / self.alloc_size).ok()?;
            let (_, _, extents) =
                self.overflow.iter().find(|(id, abn, _)| *id == cnid && *abn == next)?;
            let more = self.read_extents(extents, size - out.len());
            if more.is_empty() {
                return None;
            }
            out.extend_from_slice(&more);
        }
        None
    }

    /// Read a file's data fork. `None` if its extents are broken or run short of
    /// the size the catalog declares.
    pub fn read(&self, entry: &HfsEntry) -> Option<Vec<u8>> {
        self.read_fork(entry.id, entry.extents, entry.size)
    }

    /// Read a file by name (case-insensitive), for callers that already know
    /// what they want. Prefer [`Hfs::story`] / [`Hfs::pictures`], which identify
    /// by content.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.files.iter().find(|e| e.name.eq_ignore_ascii_case(name))?;
        self.read(e)
    }

    /// The story image on this volume, with the name it was stored under.
    ///
    /// Every file is tested with [`looks_like_story`]; a volume with none yields
    /// `None`. When more than one passes, Infocom's `Story.data` convention
    /// breaks the tie, then the largest candidate, so the choice is
    /// deterministic rather than catalog-order luck.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.name.clone(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .collect();
        cands.sort_by_key(|(name, bytes)| {
            (!name.eq_ignore_ascii_case(CONVENTIONAL_STORY), std::cmp::Reverse(bytes.len()))
        });
        cands.into_iter().next()
    }

    /// The native Infocom picture archive on this volume, with its stored name.
    ///
    /// Identified by parsing, exactly as [`crate::adf::Adf::pictures`] does.
    /// The Macintosh ships two archives — a colour one and a monochrome one —
    /// and only the colour one is a container [`InfocomPics`] reads, so on the
    /// media in hand this choice is settled before the tiebreak is reached. The
    /// tiebreak still names both conventional filenames, then picture count, so
    /// that a disk offering two readable archives has a deterministic answer.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.name.clone(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(name, b)| InfocomPics::parse(b).ok().map(|p| (name, p)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(name, pics)| {
            let lower = name.to_ascii_lowercase();
            let conventional = CONVENTIONAL_PICTURES.contains(&lower.as_str());
            (!conventional, std::cmp::Reverse(pics.entries().len()))
        });
        cands.into_iter().next()
    }
}

/// Where the HFS volume starts inside `bytes` — 0 for a bare volume, 84 inside a
/// DiskCopy 4.2 wrapper — or `None` when neither placement holds one.
fn volume_offset(bytes: &[u8]) -> Option<usize> {
    if let Some(len) = diskcopy_volume_len(bytes) {
        if volume_is_sane(&bytes[DISKCOPY_HEADER..DISKCOPY_HEADER + len]) {
            return Some(DISKCOPY_HEADER);
        }
    }
    volume_is_sane(bytes).then_some(0)
}

/// The volume length a DiskCopy 4.2 header declares, when `bytes` carries one.
fn diskcopy_volume_len(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < DISKCOPY_HEADER || be16(bytes, DISKCOPY_MAGIC_OFF) != DISKCOPY_MAGIC {
        return None;
    }
    let data = be32(bytes, DISKCOPY_DATA_SIZE) as usize;
    let tag = be32(bytes, DISKCOPY_TAG_SIZE) as usize;
    // The tags follow the volume; both have to fit, and a volume is a whole
    // number of logical blocks.
    let total = DISKCOPY_HEADER.checked_add(data)?.checked_add(tag)?;
    (data >= 3 * BLOCK && data.is_multiple_of(BLOCK) && total <= bytes.len()).then_some(data)
}

/// Does `volume` open with a Master Directory Block that describes it?
///
/// The signature alone is two bytes and would fire on noise; the geometry is
/// what makes this safe. An allocation block is a whole number of logical
/// blocks, there is at least one of them, and the last one has to be inside the
/// volume — which a wrapper's 84-byte offset breaks, so the two placements
/// cannot both pass.
fn volume_is_sane(volume: &[u8]) -> bool {
    if volume.len() < MDB_OFFSET + MDB_LEN || be16(volume, MDB_OFFSET) != HFS_SIGNATURE {
        return false;
    }
    let mdb = &volume[MDB_OFFSET..MDB_OFFSET + MDB_LEN];
    let alloc_size = be32(mdb, MDB_AL_BLK_SIZ) as usize;
    let count = usize::from(be16(mdb, MDB_NM_AL_BLKS));
    let start = usize::from(be16(mdb, MDB_AL_BL_ST));
    if alloc_size == 0 || !alloc_size.is_multiple_of(BLOCK) || count == 0 || start < 3 {
        return false;
    }
    start * BLOCK + count * alloc_size <= volume.len()
}

/// The three extents at `off`.
fn extent_record(b: &[u8], off: usize) -> ExtentRecord {
    [
        (be16(b, off), be16(b, off + 2)),
        (be16(b, off + 4), be16(b, off + 6)),
        (be16(b, off + 8), be16(b, off + 10)),
    ]
}

/// Every data-fork record in the extents overflow file, as
/// `(cnid, first fork-relative allocation block, extents)`.
fn overflow_records(tree: &[u8]) -> Vec<(u32, u16, ExtentRecord)> {
    let mut out = Vec::new();
    for rec in leaf_records(tree) {
        if usize::from(rec[0]) < XKR_LEN || rec.len() < XKR_LEN + 1 + 12 {
            continue;
        }
        if rec[1] != FORK_DATA {
            continue;
        }
        // The key is `xkrKeyLen` plus that many bytes; the data follows, and
        // both halves are word-aligned by construction (7 + 1 is even).
        out.push((be32(rec, 2), be16(rec, 6), extent_record(rec, XKR_LEN + 1)));
    }
    out
}

/// Every file the catalog names, wherever it lives on the volume.
fn catalog_files(tree: &[u8]) -> Vec<HfsEntry> {
    let mut out = Vec::new();
    for rec in leaf_records(tree) {
        // Key: length byte, reserved byte, parent id, then a Pascal name. The
        // record's data starts at the next even offset past it.
        let key_len = usize::from(rec[0]);
        if key_len < 6 {
            continue;
        }
        let Some(name) = mac_name(&rec[6..]) else { continue };
        let data = (1 + key_len).next_multiple_of(2);
        let Some(d) = rec.get(data..data + FILE_REC_LEN) else { continue };
        if d[0] != CDR_FILE {
            continue;
        }
        out.push(HfsEntry {
            name,
            size: be32(d, FIL_LG_LEN) as usize,
            resource_size: be32(d, FIL_R_LG_LEN) as usize,
            file_type: [d[FIL_TYPE], d[FIL_TYPE + 1], d[FIL_TYPE + 2], d[FIL_TYPE + 3]],
            creator: [d[FIL_CREATOR], d[FIL_CREATOR + 1], d[FIL_CREATOR + 2], d[FIL_CREATOR + 3]],
            id: be32(d, FIL_FL_NUM),
            extents: extent_record(d, FIL_EXT_REC),
        });
    }
    out
}

/// Every record in every leaf of a B*-tree, in order.
///
/// Node 0 is the header node; the first record in it is the header record, and
/// `bthFNode` names the first leaf. From there the leaves are a linked list
/// through `ndFLink`, which is what makes reading all of them a walk rather than
/// a tree descent. The walk is bounded by the number of nodes in the file, so a
/// corrupt image that links a leaf to itself terminates.
fn leaf_records(tree: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let Some(header) = tree.get(..NODE_SIZE) else { return out };
    let Some(first) = node_records(header).first().map(|r| be32(r, BTH_F_NODE)) else {
        return out;
    };
    let mut n = first as usize;
    for _ in 0..tree.len() / NODE_SIZE {
        if n == 0 {
            break;
        }
        let Some(node) = tree.get(n * NODE_SIZE..(n + 1) * NODE_SIZE) else { break };
        out.extend(node_records(node));
        n = be32(node, 0) as usize;
    }
    out
}

/// The records in one node.
///
/// A node's records grow from the front and their offsets grow *backwards* from
/// the end, one 16-bit offset per record plus one more marking the free space —
/// so record `i` runs from `offsets[i]` to `offsets[i + 1]`. Anything that does
/// not describe a forward-running record inside the node ends the node early
/// rather than panicking.
fn node_records(node: &[u8]) -> Vec<&[u8]> {
    let count = usize::from(be16(node, ND_N_RECS));
    let mut out = Vec::new();
    if count == 0 || 2 * (count + 1) > NODE_SIZE - NODE_HEADER {
        return out;
    }
    let at = |i: usize| usize::from(be16(node, NODE_SIZE - 2 * (i + 1)));
    for i in 0..count {
        let (start, end) = (at(i), at(i + 1));
        if start < NODE_HEADER || end > NODE_SIZE || end <= start {
            break;
        }
        out.push(&node[start..end]);
    }
    out
}

/// A Pascal string, or `None` when it is empty or over-runs its buffer.
///
/// Macintosh filenames are MacRoman. Its lower half is ASCII, and this reader
/// does **not** transliterate the upper half: the 128 characters above it are a
/// table that would have to be verified against Apple's own, and nothing here
/// selects a file by name — the story and the artwork are identified by content,
/// and the conventional names are ASCII. Bytes outside printable ASCII become
/// U+FFFD, so a name is safe to display and never silently wrong.
fn mac_name(pascal: &[u8]) -> Option<String> {
    let len = usize::from(*pascal.first()?);
    if len == 0 {
        return None;
    }
    let raw = pascal.get(1..1 + len)?;
    let printable = |c: &u8| if (0x20..0x7f).contains(c) { char::from(*c) } else { '\u{fffd}' };
    Some(raw.iter().map(printable).collect())
}

/// Big-endian word at `off`.
fn be16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}

/// Big-endian longword at `off`.
fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 800K floppy's worth of volume: 1600 logical blocks.
    const VOLUME_BLOCKS: usize = 1600;
    /// Where this builder puts its allocation blocks, matching a real disk.
    const ALLOC_START: usize = 4;

    /// Builder for a synthetic HFS volume, so the tests need no fixture.
    ///
    /// One allocation block per logical block (which is what an 800K disk uses),
    /// a catalog laid out as a header node plus one leaf, and files written
    /// contiguously — with a hook to fragment one, because the extents overflow
    /// file is the part a single-extent fixture would never exercise.
    struct VolumeBuilder {
        volume: Vec<u8>,
        /// Next free allocation block for file data.
        next: usize,
        /// Catalog leaf records, in the order they were added.
        catalog: Vec<Vec<u8>>,
        /// Extents overflow records: `(cnid, first block, extents)`.
        overflow: Vec<(u32, u16, ExtentRecord)>,
        next_cnid: u32,
    }

    impl VolumeBuilder {
        fn new() -> VolumeBuilder {
            VolumeBuilder {
                volume: vec![0u8; VOLUME_BLOCKS * BLOCK],
                // The two B*-trees take the first six allocation blocks.
                next: 6,
                catalog: Vec::new(),
                overflow: Vec::new(),
                next_cnid: 16,
            }
        }

        fn put16(&mut self, at: usize, v: u16) {
            self.volume[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }

        fn put32(&mut self, at: usize, v: u32) {
            self.volume[at..at + 4].copy_from_slice(&v.to_be_bytes());
        }

        /// Write `data` as a file, split into `pieces` extents. More than three
        /// pieces spills into the extents overflow file, exactly as HFS does.
        fn add_file(&mut self, name: &str, ftype: &[u8; 4], data: &[u8], pieces: usize) {
            let cnid = self.next_cnid;
            self.next_cnid += 1;
            let blocks = data.len().div_ceil(BLOCK);
            let per = blocks.div_ceil(pieces.max(1));
            let mut extents: Vec<Extent> = Vec::new();
            let mut written = 0usize;
            while written < blocks {
                let run = per.min(blocks - written);
                let start = self.next;
                self.next += run;
                // A gap between runs, so a reader that assumed contiguity fails.
                self.next += 1;
                for i in 0..run {
                    let src = (written + i) * BLOCK;
                    let end = data.len().min(src + BLOCK);
                    let at = (ALLOC_START + start + i) * BLOCK;
                    self.volume[at..at + (end - src)].copy_from_slice(&data[src..end]);
                }
                extents.push((start as u16, run as u16));
                written += run;
            }
            let first = pad_extents(&extents[..extents.len().min(3)]);
            for (i, chunk) in extents[extents.len().min(3)..].chunks(3).enumerate() {
                // Keyed by the fork-relative allocation block the record starts
                // at: three extents of `per` blocks each precede it.
                self.overflow.push((cnid, ((3 + i * 3) * per) as u16, pad_extents(chunk)));
            }

            // The catalog record: key (length, reserved, parent, name) then a
            // file record.
            let mut rec = vec![0u8; 1 + 1 + 4 + 1 + name.len()];
            rec[0] = (rec.len() - 1) as u8;
            rec[2..6].copy_from_slice(&2u32.to_be_bytes()); // parent: the root
            rec[6] = name.len() as u8;
            rec[7..].copy_from_slice(name.as_bytes());
            while !rec.len().is_multiple_of(2) {
                rec.push(0);
            }
            let mut d = vec![0u8; FILE_REC_LEN];
            d[0] = CDR_FILE;
            d[FIL_TYPE..FIL_TYPE + 4].copy_from_slice(ftype);
            d[FIL_CREATOR..FIL_CREATOR + 4].copy_from_slice(b"IN0Z");
            d[FIL_FL_NUM..FIL_FL_NUM + 4].copy_from_slice(&cnid.to_be_bytes());
            d[FIL_LG_LEN..FIL_LG_LEN + 4].copy_from_slice(&(data.len() as u32).to_be_bytes());
            for (i, (start, count)) in first.iter().enumerate() {
                let at = FIL_EXT_REC + i * 4;
                d[at..at + 2].copy_from_slice(&start.to_be_bytes());
                d[at + 2..at + 4].copy_from_slice(&count.to_be_bytes());
            }
            rec.extend_from_slice(&d);
            self.catalog.push(rec);
        }

        /// Lay the MDB and both B*-trees down and hand back the volume.
        fn finish(mut self) -> Vec<u8> {
            let catalog = btree(&self.catalog);
            let overflow: Vec<Vec<u8>> = self
                .overflow
                .iter()
                .map(|(cnid, abn, exts)| {
                    let mut r = vec![0u8; XKR_LEN + 1 + 12];
                    r[0] = XKR_LEN as u8;
                    r[1] = FORK_DATA;
                    r[2..6].copy_from_slice(&cnid.to_be_bytes());
                    r[6..8].copy_from_slice(&abn.to_be_bytes());
                    for (i, (start, count)) in exts.iter().enumerate() {
                        let at = XKR_LEN + 1 + i * 4;
                        r[at..at + 2].copy_from_slice(&start.to_be_bytes());
                        r[at + 2..at + 4].copy_from_slice(&count.to_be_bytes());
                    }
                    r
                })
                .collect();
            let extents = btree(&overflow);

            // Trees first: allocation blocks 0..3 are the extents file, 3..6 the
            // catalog. (`VolumeBuilder::new` reserves exactly those.)
            for (base, tree) in [(0usize, &extents), (3usize, &catalog)] {
                let at = (ALLOC_START + base) * BLOCK;
                self.volume[at..at + tree.len()].copy_from_slice(tree);
            }

            let mdb = MDB_OFFSET;
            self.put16(mdb, HFS_SIGNATURE);
            self.put16(mdb + MDB_NM_AL_BLKS, (VOLUME_BLOCKS - ALLOC_START) as u16);
            self.put32(mdb + MDB_AL_BLK_SIZ, BLOCK as u32);
            self.put16(mdb + MDB_AL_BL_ST, ALLOC_START as u16);
            let name = b"Test Disk";
            self.volume[mdb + MDB_VN] = name.len() as u8;
            self.volume[mdb + MDB_VN + 1..mdb + MDB_VN + 1 + name.len()].copy_from_slice(name);
            self.put32(mdb + MDB_XT_FL_SIZE, (3 * BLOCK) as u32);
            self.put16(mdb + MDB_XT_EXT_REC, 0);
            self.put16(mdb + MDB_XT_EXT_REC + 2, 3);
            self.put32(mdb + MDB_CT_FL_SIZE, (3 * BLOCK) as u32);
            self.put16(mdb + MDB_CT_EXT_REC, 3);
            self.put16(mdb + MDB_CT_EXT_REC + 2, 3);
            self.volume
        }
    }

    fn pad_extents(exts: &[Extent]) -> ExtentRecord {
        let mut out: ExtentRecord = [(0, 0); 3];
        for (i, e) in exts.iter().take(3).enumerate() {
            out[i] = *e;
        }
        out
    }

    /// A header node naming one leaf, then that leaf. Three blocks, which is
    /// what [`VolumeBuilder`] reserves for each tree.
    fn btree(records: &[Vec<u8>]) -> Vec<u8> {
        let mut tree = vec![0u8; 3 * NODE_SIZE];
        // Header node: one record, whose `bthFNode` is node 1.
        tree[8] = 1; // ndType: header
        tree[ND_N_RECS..ND_N_RECS + 2].copy_from_slice(&1u16.to_be_bytes());
        tree[NODE_SIZE - 2..NODE_SIZE].copy_from_slice(&(NODE_HEADER as u16).to_be_bytes());
        let header_end = NODE_HEADER as u16 + 106;
        tree[NODE_SIZE - 4..NODE_SIZE - 2].copy_from_slice(&header_end.to_be_bytes());
        tree[NODE_HEADER + BTH_F_NODE..NODE_HEADER + BTH_F_NODE + 4]
            .copy_from_slice(&1u32.to_be_bytes());

        // Leaf node 1: the records, front to back, with their offsets from the
        // back. `ndType` is -1 and `ndFLink` 0 — one leaf, no chain.
        let leaf = NODE_SIZE;
        tree[leaf + 8] = 0xFF;
        tree[leaf + ND_N_RECS..leaf + ND_N_RECS + 2]
            .copy_from_slice(&(records.len() as u16).to_be_bytes());
        let mut at = NODE_HEADER;
        for (i, r) in records.iter().enumerate() {
            tree[leaf + NODE_SIZE - 2 * (i + 1)..leaf + NODE_SIZE - 2 * i]
                .copy_from_slice(&(at as u16).to_be_bytes());
            tree[leaf + at..leaf + at + r.len()].copy_from_slice(r);
            at += r.len();
        }
        let n = records.len();
        tree[leaf + NODE_SIZE - 2 * (n + 1)..leaf + NODE_SIZE - 2 * n]
            .copy_from_slice(&(at as u16).to_be_bytes());
        tree
    }

    /// Wrap a volume the way DiskCopy 4.2 does, tags and all.
    fn diskcopy(volume: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; DISKCOPY_HEADER];
        let name = b"Test Disk";
        out[0] = name.len() as u8;
        out[1..1 + name.len()].copy_from_slice(name);
        let tags = 12 * (volume.len() / BLOCK);
        out[DISKCOPY_DATA_SIZE..DISKCOPY_DATA_SIZE + 4]
            .copy_from_slice(&(volume.len() as u32).to_be_bytes());
        out[DISKCOPY_TAG_SIZE..DISKCOPY_TAG_SIZE + 4].copy_from_slice(&(tags as u32).to_be_bytes());
        out[0x50] = 0x01;
        out[0x51] = 0x22;
        let magic = DISKCOPY_MAGIC.to_be_bytes();
        out[DISKCOPY_MAGIC_OFF..DISKCOPY_MAGIC_OFF + 2].copy_from_slice(&magic);
        out.extend_from_slice(volume);
        out.extend(std::iter::repeat_n(0xA5u8, tags));
        out
    }

    /// A minimal but structurally valid v6 story header, padded to `len`.
    fn fake_story(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 8) as u16); // file length, v6 unit
        b[0x12..0x18].copy_from_slice(b"881019");
        b
    }

    #[test]
    fn rejects_images_that_are_not_macintosh_volumes() {
        assert!(!Hfs::looks_like_hfs(b"not a disk"));
        assert!(!Hfs::looks_like_hfs(&vec![0u8; VOLUME_BLOCKS * BLOCK]), "no BD signature");
        // An MFS volume — the flat filesystem HFS replaced — says so at the same
        // offset, and nothing here would read its directory.
        let mut mfs = vec![0u8; VOLUME_BLOCKS * BLOCK];
        mfs[MDB_OFFSET..MDB_OFFSET + 2].copy_from_slice(&0xD2D7u16.to_be_bytes());
        assert!(!Hfs::looks_like_hfs(&mfs));
        assert_eq!(Hfs::mount(vec![0u8; 16]).unwrap_err(), HfsError::NotHfs);
    }

    /// The tags are 12 bytes per sector and they follow the volume. A reader
    /// that treats the whole file as the volume, or that misplaces the 84-byte
    /// header, gets a volume whose geometry does not fit — which is exactly what
    /// [`volume_is_sane`] tests, so both mistakes are caught here.
    #[test]
    fn reads_a_volume_bare_and_inside_a_diskcopy_wrapper() {
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        let volume = b.finish();
        let wrapped = diskcopy(&volume);
        assert!(wrapped.len() > volume.len() + DISKCOPY_HEADER, "the tags are there too");

        for (what, image) in [("bare", volume.clone()), ("wrapped", wrapped)] {
            assert!(Hfs::looks_like_hfs(&image), "{what}");
            let hfs = Hfs::mount(image).unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(hfs.volume_name(), "Test Disk", "{what}");
            assert_eq!(hfs.files().len(), 1, "{what}");
            assert_eq!(hfs.story().expect("a story").0, "Story.data", "{what}");
        }
    }

    /// A fork with more than three extents keeps the rest in the extents
    /// overflow file. Nothing on the Zork Zero disk needs it — every file there
    /// is one extent — so this is the case only a synthetic volume can pin.
    #[test]
    fn reads_a_fork_that_spills_into_the_extents_overflow_file() {
        let mut b = VolumeBuilder::new();
        let contiguous: Vec<u8> = (0..40 * BLOCK).map(|i| (i % 251) as u8).collect();
        let fragmented: Vec<u8> = (0..60 * BLOCK).map(|i| (i % 241) as u8).collect();
        b.add_file("Whole", b"INdf", &contiguous, 1);
        b.add_file("Shattered", b"INdf", &fragmented, 6);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.read_named("whole").as_deref(), Some(&contiguous[..]));
        assert_eq!(
            hfs.read_named("Shattered").as_deref(),
            Some(&fragmented[..]),
            "six extents: three in the catalog record, three in the overflow file"
        );
        assert_eq!(hfs.read_named("absent"), None);
    }

    #[test]
    fn finds_a_story_by_content_not_by_name() {
        let mut b = VolumeBuilder::new();
        b.add_file("Zork Zero", b"APPL", b"Joy!peffpwpc application", 1);
        b.add_file("Bootleg", b"INdf", &fake_story(4096), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        let (name, bytes) = hfs.story().expect("the story is found under any name");
        assert_eq!(name, "Bootleg");
        assert_eq!(bytes.len(), 4096);
    }

    #[test]
    fn a_disk_with_no_story_says_so() {
        let mut b = VolumeBuilder::new();
        b.add_file("Desktop", b"FNDR", &vec![0x11u8; 1665], 1);
        b.add_file("System", b"ZSYS", &vec![0x22u8; 8000], 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story(), None);
        assert!(hfs.pictures().is_none());
    }

    #[test]
    fn the_conventional_name_only_breaks_a_tie() {
        let mut b = VolumeBuilder::new();
        b.add_file("Backup.data", b"INdf", &fake_story(8192), 1);
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story().expect("a story").0, "Story.data", "convention beats size");

        let mut b = VolumeBuilder::new();
        b.add_file("Alpha", b"INdf", &fake_story(4096), 1);
        b.add_file("Beta", b"INdf", &fake_story(8192), 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        assert_eq!(hfs.story().expect("a story").0, "Beta", "no convention → the largest");
    }

    /// The catalog names types and sizes, which is what lets a listing say what
    /// a Macintosh file *is* — an application has no data fork at all.
    #[test]
    fn a_listing_reports_type_creator_and_both_forks() {
        let mut b = VolumeBuilder::new();
        b.add_file("Story.data", b"INdf", &fake_story(4096), 1);
        b.add_file("Zork Zero", b"APPL", b"", 1);
        let hfs = Hfs::mount(b.finish()).expect("mounts");
        let story = &hfs.files()[0];
        assert_eq!(&story.file_type, b"INdf");
        assert_eq!(&story.creator, b"IN0Z");
        assert_eq!(story.size, 4096);
        assert_eq!(hfs.files()[1].size, 0, "an application is all resource fork");
    }

    /// Real media: the user's Macintosh Zork Zero, if they have it. It lives
    /// outside the repo, so this skips vacuously.
    #[test]
    fn real_macintosh_zork_zero_disk() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork Zero Disk.image");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: original Macintosh media absent at {}", path.display());
            return;
        };
        assert!(Hfs::looks_like_hfs(&bytes));
        let hfs = Hfs::mount(bytes).expect("the disk mounts");
        assert_eq!(hfs.volume_name(), "Zork Zero Disk");

        // The whole catalog, which is what says whether the disk carries art.
        let listing: Vec<(String, usize, usize)> = hfs
            .files()
            .iter()
            .map(|e| (e.name.clone(), e.size, e.resource_size))
            .collect();
        assert_eq!(
            listing,
            vec![
                ("CPic.data".to_string(), 218_624, 0),
                ("Desktop".to_string(), 0, 1_665),
                ("Pic.data".to_string(), 239_104, 0),
                ("Story.data".to_string(), 295_936, 0),
                ("Zork Zero".to_string(), 0, 38_833),
            ],
            "five files, and two of them are picture archives"
        );

        let (name, story) = hfs.story().expect("Story.data is found");
        assert_eq!(name, "Story.data");
        assert_eq!(story.len(), 295_936);
        assert_eq!(story[0], 6, "Zork Zero is v6");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 296, "release 296, not the PC's 393");
        assert_eq!(&story[0x12..0x18], b"881019");

        // The colour archive reads today; the monochrome one declares 12-byte
        // records and does not.
        let (pname, pics) = hfs.pictures().expect("the colour archive is found");
        assert_eq!(pname, "CPic.data");
        assert_eq!(pics.entries().len(), 483);
        assert!(pics.decode(1).is_ok(), "picture 1 decodes straight off the disk");
        let mono = hfs.read_named("Pic.data").expect("the monochrome archive is there");
        assert_eq!(mono[1], 0x0e, "its header flags are bocfel's monochrome-Macintosh 0x0e");
        assert_eq!(mono[8], 12, "…and its directory records are 12 bytes, not 14");
    }
}
