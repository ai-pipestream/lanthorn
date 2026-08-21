//! Read a DOS or an Atari ST release floppy — **one FAT12 reader, two
//! machines** (SQ-0833, SQ-0835).
//!
//! Infocom pressed its PC releases onto 360 KB and 720 KB DOS floppies and its
//! ST releases onto 820 KB and 902 KB GEMDOS ones, and the two filesystems are
//! the same filesystem. GEMDOS puts its BIOS Parameter Block at **exactly** the
//! DOS offsets — `0x0B` bytes/sector, `0x0D` sectors/cluster, `0x0E` reserved,
//! `0x10` FATs, `0x11` root entries, `0x13` total sectors, `0x16` sectors/FAT —
//! so a plain DOS parser reads an Atari disk with no Atari-specific code in it
//! at all. That is why these are one module and not two: the FILESYSTEM is one
//! thing, and the MACHINE is a separate question asked of the boot sector.
//!
//! Everything below is measured against the **twenty-four** images in the
//! user's corpus — fifteen DOS and nine Atari ST: `floppy1-5.ima` (*The Lost
//! Treasures of Infocom* I), `disk1-4.img` (Vol II), the *Zork Zero* 360 KB
//! three-disk and 720 KB two-disk releases, the standalone 360 KB
//! *Hitchhiker's*, and `Infocom Compilation 1-9 (19xx)(-).st`. Nothing else in
//! that directory is claimed: not the `.adf`s, not the HFS `.image`, not a bare
//! `.z6`.
//!
//! # Which machine pressed it
//!
//! DOS's own load protocol requires the boot sector to begin with an **x86 jump
//! over the BPB** — `EB xx 90` (short jump + `NOP`) or `E9 xx xx` (near jump) —
//! because the BIOS loads the sector and executes it from offset 0, and the BPB
//! that follows is data. GEMDOS has no such requirement: TOS executes an ST boot
//! sector only when its 256 big-endian words sum to `0x1234`, and a non-bootable
//! ST disk leaves whatever the formatter wrote. So an x86 jump is a thing only a
//! DOS formatter has a reason to write, and [`looks_like_dos`] is exactly that
//! test; every other volume with a sane BPB is [`Machine::AtariSt`].
//!
//! The measurement backs it with no exceptions: all fifteen DOS images open
//! `EB xx 90` with four different OEM strings (`IBM  2.0`, `IBM  3.3`,
//! `IBM PNCI`, `MSDOS5.0`), and all nine ST images open `00 00 4E` or
//! `00 00 00` with the OEM field either `NNNNNN` or **all zeros**.
//!
//! **What this deliberately does not key on**, and why:
//!
//! * **The extension.** `.ima` and `.img` are the same DOS format in this
//!   corpus, and the ST images are `.st`. `blorb` recognises media by content
//!   everywhere else and this is no exception.
//! * **The OEM string.** Four different ones on the DOS side and zeros on the
//!   ST side; it identifies a formatter, not a machine.
//! * **The `55 AA` signature at offset 510.** It agrees with the jump test on
//!   all twenty-four images — every DOS one has it, no ST one does — but on the ST
//!   those same two bytes are the boot **checksum** word rather than a
//!   signature, so a bootable ST disk is one coincidence away from carrying
//!   `55 AA` honestly. Two tests that can disagree need a tie-break nothing in
//!   the corpus establishes, so there is one test. (The agreement is pinned as a
//!   test anyway, so a counterexample arriving in the corpus is caught.)
//!
//! # Choosing what to run
//!
//! By CONTENT, like every other reader here, and this format needs it most:
//! **every story on an Atari ST compilation is called `STORY.DAT`**, so the
//! filename identifies nothing and the *directory* names the game. Files are
//! therefore reported by their [`Fat12Entry::path`] — `HITCHHIK/STORY.DAT`
//! rather than a fourth indistinguishable `STORY.DAT` — which is the disk's own
//! spelling and disambiguates without inventing a name.
//!
//! Subdirectories are real and must be walked: the ST compilations 8 and 9 put
//! each game in its own folder, and the standalone DOS *Hitchhiker's* disk keeps
//! a `DEMO` folder beside the game. A root-only enumeration would miss four
//! games out of four on `Infocom Compilation 9`.
//!
//! Nothing but a story passes [`looks_like_story`]: the ST disks carry someone's
//! 1996 saved games (`BILL1.SAV`, `STEVE1.SAV`, `LURK1.SAV`), and the DOS disks
//! carry `ZIP1.COM`, `INFOCOM.EXE`, `ANSI3.SYS`, `SCREEN.DAT`, `FONT3.DAT` and
//! `INSTALL.DAT`. None of them is offered as a game.
//!
//! # One image at a time
//!
//! A mount is one disk. That is a real limit rather than an oversight, and the
//! corpus is where it bites: *Zork Zero*'s story is on Lost Treasures floppy**5**
//! with its EGA art while its CGA art is on floppy**4**, and the standalone
//! 360 KB release splits installer+CGA, story, and EGA across three disks. So
//! mounting one image of a multi-disk release yields whatever that image holds —
//! the story, the art, or neither — and joining several into one release is a
//! **set** model this module does not have.

use crate::adf::looks_like_story;
use crate::infocom_pics::InfocomPics;

// ── BPB (BIOS Parameter Block) — identical on DOS and GEMDOS ─────────────────

/// Bytes per sector, `u16` little-endian.
const BPB_BYTES_PER_SECTOR: usize = 0x0b;
/// Sectors per cluster, `u8`.
const BPB_SECTORS_PER_CLUSTER: usize = 0x0d;
/// Reserved sectors before the first FAT, `u16` little-endian.
const BPB_RESERVED: usize = 0x0e;
/// Number of FAT copies, `u8`.
const BPB_FATS: usize = 0x10;
/// Root-directory entries, `u16` little-endian.
const BPB_ROOT_ENTRIES: usize = 0x11;
/// Total sectors, `u16` little-endian. Zero means "see the 32-bit field", which
/// no FAT12 floppy uses and this reader declines.
const BPB_TOTAL_SECTORS: usize = 0x13;
/// Media descriptor, `u8` — `F0`…`FF`. DOS repeats it in the FAT's own first
/// byte; GEMDOS does not (see [`Bpb::parse`]).
const BPB_MEDIA: usize = 0x15;
/// Sectors per FAT, `u16` little-endian.
const BPB_SECTORS_PER_FAT: usize = 0x16;

// ── Directory entries ────────────────────────────────────────────────────────

/// Bytes in one directory entry.
const DIR_ENTRY: usize = 32;
/// Attribute byte.
const DIR_ATTR: usize = 11;
/// First cluster, `u16` little-endian.
const DIR_FIRST_CLUSTER: usize = 26;
/// File size in bytes, `u32` little-endian.
const DIR_SIZE: usize = 28;
/// A first byte of zero: this entry and every one after it is free.
const DIR_FREE: u8 = 0x00;
/// A first byte of `E5`: deleted.
const DIR_DELETED: u8 = 0xe5;
/// A first byte of `05` stands in for a real leading `E5` (KANJI).
const DIR_E5_ESCAPE: u8 = 0x05;
/// Volume-label attribute — the volume's name, not a file.
const ATTR_VOLUME_LABEL: u8 = 0x08;
/// Subdirectory attribute.
const ATTR_DIRECTORY: u8 = 0x10;
/// A VFAT long-name fragment masquerading as an entry; skipped whole.
const ATTR_LONG_NAME: u8 = 0x0f;

// ── FAT ──────────────────────────────────────────────────────────────────────

/// The first cluster number that can hold data; 0 and 1 are reserved.
const FIRST_CLUSTER: usize = 2;
/// A 12-bit FAT value at or above this ends the chain (`FF8`…`FFF`).
const FAT12_END_OF_CHAIN: u16 = 0xff8;
/// The cluster count at which a volume stops being FAT12 (Microsoft's own
/// boundary). Above it the FAT entries are 16 bits and this reader would
/// silently misread them, so a bigger volume is declined rather than mangled.
const MAX_FAT12_CLUSTERS: usize = 4085;
/// How deep the directory walk will recurse. The corpus never exceeds one
/// level; this only stops a malformed image from running away.
const MAX_DEPTH: usize = 8;

/// Infocom's conventional story name on an Atari ST compilation. Never a test —
/// only a tiebreak when content identification finds more than one candidate.
const CONVENTIONAL_STORY: &str = "story.dat";

/// The machine a FAT12 volume was pressed for. See the module docs for the
/// discriminator and why it is the one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    /// An IBM PC / MS-DOS floppy: the boot sector opens with an x86 jump.
    Dos,
    /// An Atari ST GEMDOS floppy: the same BPB, and no x86 jump in front of it.
    AtariSt,
}

/// Errors that can arise while mounting a FAT12 image.
#[derive(Debug, PartialEq, Eq)]
pub enum Fat12Error {
    /// The bytes are not a FAT12 volume this reader recognises — no sane BPB,
    /// or a volume too large to be FAT12.
    NotFat12,
}

/// One file found on the image. Directories are not listed; the walk descends
/// into them and reports what is inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fat12Entry {
    /// The 8.3 filename, exactly as stored (`STORY.DAT`, `ZORK0.ZIP`).
    pub name: String,
    /// The directory it lives in, or `None` at the root. **This is what names
    /// an ST game**: four files called `STORY.DAT` are told apart only by
    /// `HITCHHIK`, `BUREAUCR.ACY`, `CUTHROAT` and `LEATHER.GOD`.
    pub dir: Option<String>,
    /// Size in bytes, from the directory entry.
    pub size: usize,
    /// First cluster of the file's chain.
    first_cluster: u16,
}

impl Fat12Entry {
    /// How this file is named to the outside world: `DIR/NAME` in a
    /// subdirectory, the bare name at the root.
    pub fn path(&self) -> String {
        match &self.dir {
            Some(d) => format!("{d}/{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A mounted FAT12 volume — a DOS or an Atari ST release floppy.
#[derive(Debug)]
pub struct Fat12 {
    image: Vec<u8>,
    machine: Machine,
    bpb: Bpb,
    label: Option<String>,
    files: Vec<Fat12Entry>,
}

/// The geometry a BPB describes, in bytes rather than sectors, once it has been
/// checked for sanity.
#[derive(Debug, Clone, Copy)]
struct Bpb {
    bytes_per_cluster: usize,
    /// Byte offset of the first FAT.
    fat_start: usize,
    /// Bytes in one FAT — the bound a 12-bit entry lookup must respect.
    fat_len: usize,
    /// Byte offset of the root directory.
    root_start: usize,
    /// Bytes in the root directory.
    root_len: usize,
    /// Byte offset of cluster 2.
    data_start: usize,
    /// Data clusters on the volume, so a chain cannot walk off the end.
    clusters: usize,
}

/// Is `raw` a **DOS** floppy image? See the module docs: the test is the x86
/// jump over the BPB, which DOS's load protocol requires and GEMDOS does not.
pub fn looks_like_dos(raw: &[u8]) -> bool {
    machine_of(raw) == Some(Machine::Dos)
}

/// Is `raw` an **Atari ST** floppy image? A sane FAT12 BPB with no x86 jump in
/// front of it.
pub fn looks_like_atari_st(raw: &[u8]) -> bool {
    machine_of(raw) == Some(Machine::AtariSt)
}

/// The machine `raw` was pressed for, or `None` when it is not a FAT12 volume
/// at all. The single place the two questions — *is this FAT12?* and *whose?* —
/// are joined.
fn machine_of(raw: &[u8]) -> Option<Machine> {
    Bpb::parse(raw)?;
    Some(if has_x86_jump(raw) { Machine::Dos } else { Machine::AtariSt })
}

/// Does the boot sector open with an x86 jump over the BPB — `EB xx 90` or
/// `E9 xx xx`? Every DOS image in the corpus does; no ST image does.
fn has_x86_jump(raw: &[u8]) -> bool {
    matches!(raw, [0xeb, _, 0x90, ..] | [0xe9, _, _, ..])
}

impl Fat12 {
    /// Cheap sniff: is this a FAT12 volume of EITHER machine?
    ///
    /// Callers that care which machine want [`looks_like_dos`] or
    /// [`looks_like_atari_st`]; those are what `blorb::medium`'s table holds,
    /// one row each, because they imply different machines even though they
    /// mount through this one reader.
    pub fn looks_like_fat12(raw: &[u8]) -> bool {
        machine_of(raw).is_some()
    }

    /// Mount an image and enumerate it, subdirectories and all.
    pub fn mount(image: Vec<u8>) -> Result<Fat12, Fat12Error> {
        let bpb = Bpb::parse(&image).ok_or(Fat12Error::NotFat12)?;
        let machine = if has_x86_jump(&image) { Machine::Dos } else { Machine::AtariSt };
        let mut fs = Fat12 { image, machine, bpb, label: None, files: Vec::new() };
        let root = fs.bpb.root_start..fs.bpb.root_start + fs.bpb.root_len;
        let root = fs.image[root].to_vec();
        fs.label = volume_label(&root);
        fs.files = fs.walk(&root, None, 0);
        Ok(fs)
    }

    /// Which machine pressed this disk.
    pub fn machine(&self) -> Machine {
        self.machine
    }

    /// The volume label from the root directory, when the disk carries one.
    /// The Lost Treasures disks do (`Tresure 1`…`Tresure 5`, `DISK 1`…`DISK 4`,
    /// `ZORK0 1`…`ZORK0 3`); no ST compilation does.
    pub fn volume_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Every file on the image, in directory order, subdirectory contents
    /// included. Directories themselves are not listed.
    pub fn files(&self) -> &[Fat12Entry] {
        &self.files
    }

    /// Walk one directory's raw bytes, recursing into subdirectories.
    fn walk(&self, dir: &[u8], parent: Option<&str>, depth: usize) -> Vec<Fat12Entry> {
        let mut out = Vec::new();
        for e in dir.as_chunks::<DIR_ENTRY>().0 {
            match e[0] {
                DIR_FREE => break,
                DIR_DELETED => continue,
                _ => {}
            }
            let attr = e[DIR_ATTR];
            if attr & ATTR_LONG_NAME == ATTR_LONG_NAME || attr & ATTR_VOLUME_LABEL != 0 {
                continue;
            }
            let Some(name) = short_name(e) else { continue };
            if name == "." || name == ".." {
                continue;
            }
            let first_cluster = le16(e, DIR_FIRST_CLUSTER);
            if attr & ATTR_DIRECTORY != 0 {
                if depth >= MAX_DEPTH {
                    continue;
                }
                // A directory's size field is zero, so its chain is what bounds
                // it: read every cluster of it and walk what is there.
                let sub = self.read_chain(first_cluster, usize::MAX);
                out.extend(self.walk(&sub, Some(&name), depth + 1));
                continue;
            }
            out.push(Fat12Entry {
                name,
                dir: parent.map(str::to_string),
                size: le32(e, DIR_SIZE) as usize,
                first_cluster,
            });
        }
        out
    }

    /// Read a file's bytes, or `None` when its chain runs short of the size the
    /// directory declares.
    pub fn read(&self, entry: &Fat12Entry) -> Option<Vec<u8>> {
        let bytes = self.read_chain(entry.first_cluster, entry.size);
        (bytes.len() >= entry.size).then(|| {
            let mut bytes = bytes;
            // The last cluster is padded; the directory's byte size decides.
            bytes.truncate(entry.size);
            bytes
        })
    }

    /// Follow a cluster chain, stopping once `want` bytes are in hand.
    ///
    /// A damaged FAT can chain a cycle, so the walk is bounded twice over: by
    /// the number of clusters on the volume, and by a visited set.
    fn read_chain(&self, first: u16, want: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut seen = vec![false; self.bpb.clusters + FIRST_CLUSTER];
        let mut cluster = usize::from(first);
        while (FIRST_CLUSTER..self.bpb.clusters + FIRST_CLUSTER).contains(&cluster) {
            if seen[cluster] {
                break;
            }
            seen[cluster] = true;
            let at = self.bpb.data_start + (cluster - FIRST_CLUSTER) * self.bpb.bytes_per_cluster;
            let Some(data) = self.image.get(at..at + self.bpb.bytes_per_cluster) else { break };
            out.extend_from_slice(data);
            if out.len() >= want {
                break;
            }
            cluster = usize::from(self.fat_entry(cluster));
        }
        out
    }

    /// The 12-bit FAT entry for `cluster`: two bytes little-endian, then the
    /// high or the low nibble-triple depending on the cluster's parity.
    fn fat_entry(&self, cluster: usize) -> u16 {
        let at = self.bpb.fat_start + cluster + cluster / 2;
        if at + 2 > self.bpb.fat_start + self.bpb.fat_len {
            return FAT12_END_OF_CHAIN;
        }
        let Some(pair) = self.image.get(at..at + 2) else { return FAT12_END_OF_CHAIN };
        let raw = u16::from(pair[0]) | (u16::from(pair[1]) << 8);
        if cluster.is_multiple_of(2) { raw & 0x0fff } else { raw >> 4 }
    }

    /// Read a file by path or by bare name, case-insensitively.
    ///
    /// `read_named("STORY.DAT")` on an ST compilation matches the first of four;
    /// `read_named("HITCHHIK/STORY.DAT")` names the one you meant. Prefer
    /// [`Fat12::story`] / [`Fat12::pictures`], which identify by content.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.files.iter().find(|e| {
            e.path().eq_ignore_ascii_case(name) || e.name.eq_ignore_ascii_case(name)
        })?;
        self.read(e)
    }

    /// The story image on this disk, with the path it was stored under.
    ///
    /// Every file is tested with [`looks_like_story`], so the saved games and
    /// the interpreters sitting beside it are never offered. When more than one
    /// passes — which is most of this corpus, since seven of nine ST
    /// compilations and every Lost Treasures disk carry several — the ST's
    /// `STORY.DAT` convention breaks the tie, then the largest candidate, so the
    /// answer is deterministic rather than directory-order luck. A compilation
    /// wants [`Fat12::files`] and a chooser, not this.
    pub fn story(&self) -> Option<(String, Vec<u8>)> {
        let mut cands: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .collect();
        cands.sort_by_key(|(path, bytes)| {
            let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
            (name != CONVENTIONAL_STORY, std::cmp::Reverse(bytes.len()))
        });
        cands.into_iter().next()
    }

    /// The native Infocom picture archive on this disk, with its stored path.
    ///
    /// Identified by parsing, exactly as [`crate::adf::Adf::pictures`] does.
    /// Files already claimed as stories are excluded, so a story can never be
    /// mistaken for art.
    ///
    /// **No video card is preferred.** A PC release's `.MG1` (MCGA), `.EG1`
    /// (EGA) and `.CG1` (CGA) are three renditions of one machine's artwork, and
    /// choosing between them is `app`'s `PictureOverride` question, not the
    /// disk's. No image in the corpus carries two anyway — Lost Treasures puts
    /// CGA on floppy4 and EGA on floppy5 — so the ordering below (most pictures,
    /// then name) exists to be deterministic, not to express a preference.
    pub fn pictures(&self) -> Option<(String, InfocomPics)> {
        let mut cands: Vec<(String, InfocomPics)> = self
            .files
            .iter()
            .filter_map(|e| self.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| !looks_like_story(b))
            .filter_map(|(path, b)| InfocomPics::parse(b).ok().map(|p| (path, p)))
            .filter(|(_, p)| p.entries().iter().any(|e| e.has_pixels()))
            .collect();
        cands.sort_by_key(|(path, pics)| (std::cmp::Reverse(pics.entries().len()), path.clone()));
        cands.into_iter().next()
    }
}

impl Bpb {
    /// Parse and sanity-check a BIOS Parameter Block.
    ///
    /// Every field is checked against what a FAT12 floppy can actually be, and
    /// the derived geometry has to fit inside the image — which is what makes
    /// this safe to run over arbitrary bytes ahead of story loading. A Z-machine
    /// story, a Blorb, a Glulx image, an AmigaDOS floppy and an HFS volume all
    /// fail it (an AmigaDOS boot block reads `bytes_per_sector = 0` at `0x0B`,
    /// for instance).
    fn parse(raw: &[u8]) -> Option<Bpb> {
        if raw.len() < BPB_SECTORS_PER_FAT + 2 {
            return None;
        }
        let bytes_per_sector = usize::from(le16(raw, BPB_BYTES_PER_SECTOR));
        let sectors_per_cluster = usize::from(raw[BPB_SECTORS_PER_CLUSTER]);
        let reserved = usize::from(le16(raw, BPB_RESERVED));
        let fats = usize::from(raw[BPB_FATS]);
        let root_entries = usize::from(le16(raw, BPB_ROOT_ENTRIES));
        let total_sectors = usize::from(le16(raw, BPB_TOTAL_SECTORS));
        let media = raw[BPB_MEDIA];
        let sectors_per_fat = usize::from(le16(raw, BPB_SECTORS_PER_FAT));

        // Sector and cluster sizes are powers of two in the ranges the format
        // allows; every floppy in the corpus is 512 bytes and 2 sectors.
        if !(128..=4096).contains(&bytes_per_sector) || !bytes_per_sector.is_power_of_two() {
            return None;
        }
        if !(1..=128).contains(&sectors_per_cluster) || !sectors_per_cluster.is_power_of_two() {
            return None;
        }
        if reserved == 0 || !(1..=2).contains(&fats) || sectors_per_fat == 0 {
            return None;
        }
        // A FAT12/16 root directory is a whole number of sectors and is never
        // empty; `0x13` is the only total-sector field a floppy uses.
        if root_entries == 0 || !(root_entries * DIR_ENTRY).is_multiple_of(bytes_per_sector) {
            return None;
        }
        if total_sectors == 0 || !(0xf0..=0xff).contains(&media) {
            return None;
        }

        let root_sectors = root_entries * DIR_ENTRY / bytes_per_sector;
        let system = reserved + fats * sectors_per_fat + root_sectors;
        if system >= total_sectors {
            return None;
        }
        // The volume must actually be present in the bytes we were handed, and
        // a disk image is a whole number of sectors (every one in the corpus is
        // exactly `total_sectors` long).
        let end = total_sectors.checked_mul(bytes_per_sector)?;
        if end > raw.len() || !raw.len().is_multiple_of(bytes_per_sector) {
            return None;
        }
        // FAT12 stops here; a bigger volume has 16-bit FAT entries this reader
        // would misread, so decline it rather than mangle it.
        let clusters = (total_sectors - system) / sectors_per_cluster;
        if clusters == 0 || clusters >= MAX_FAT12_CLUSTERS {
            return None;
        }
        // NOT checked: that the FAT repeats the media descriptor in its own
        // first byte. DOS writes `F9 FF FF` there and it is the classic extra
        // sanity check — but **GEMDOS does not**: all nine Atari compilations
        // declare media `F9` at `0x15` and open their FAT with `00 00 00`, so
        // requiring it would have rejected every ST disk in the corpus. It is
        // a DOS convention, not a FAT12 one, and this reader serves both.
        let fat_start = reserved * bytes_per_sector;

        let root_start = fat_start + fats * sectors_per_fat * bytes_per_sector;
        Some(Bpb {
            bytes_per_cluster: sectors_per_cluster * bytes_per_sector,
            fat_start,
            fat_len: sectors_per_fat * bytes_per_sector,
            root_start,
            root_len: root_sectors * bytes_per_sector,
            data_start: root_start + root_sectors * bytes_per_sector,
            clusters,
        })
    }
}

/// The volume label from a root directory's entries, trimmed of the padding
/// spaces and NULs real disks leave behind (`Tresure 1\0\0`).
fn volume_label(root: &[u8]) -> Option<String> {
    for e in root.as_chunks::<DIR_ENTRY>().0 {
        if e[0] == DIR_FREE {
            break;
        }
        if e[0] == DIR_DELETED || e[DIR_ATTR] & ATTR_LONG_NAME == ATTR_LONG_NAME {
            continue;
        }
        if e[DIR_ATTR] & ATTR_VOLUME_LABEL == 0 {
            continue;
        }
        let raw: String = e[..DIR_ATTR].iter().map(|c| char::from(*c)).collect();
        let name = raw.trim_end_matches([' ', '\0']).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// The 8.3 name in a directory entry, or `None` when it is empty or carries
/// control bytes (which a directory full of garbage would).
fn short_name(e: &[u8]) -> Option<String> {
    let mut raw = [0u8; DIR_ATTR];
    raw.copy_from_slice(&e[..DIR_ATTR]);
    // `05` in the first byte stands in for a real leading `E5`.
    if raw[0] == DIR_E5_ESCAPE {
        raw[0] = DIR_DELETED;
    }
    if raw.iter().any(|c| *c < 0x20 || *c == 0x7f) {
        return None;
    }
    let stem = str_of(&raw[..8]);
    let ext = str_of(&raw[8..]);
    if stem.is_empty() {
        return None;
    }
    Some(if ext.is_empty() { stem } else { format!("{stem}.{ext}") })
}

/// One space-padded 8.3 field as a string. The bytes are OEM code page 437 on
/// DOS and Atari's own set on the ST; both are ASCII over the range Infocom
/// used, so they map to `char` verbatim.
fn str_of(field: &[u8]) -> String {
    field.iter().map(|c| char::from(*c)).collect::<String>().trim_end().to_string()
}

/// Little-endian word at `off`.
fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// Little-endian longword at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 720 KB DOS floppy's geometry, which is also the shape of most of the
    /// corpus: 1440 sectors of 512, 2 sectors per cluster, 3 sectors per FAT.
    const SECTORS: usize = 1440;
    const SECTOR: usize = 512;
    const SECTORS_PER_CLUSTER: usize = 2;
    const SECTORS_PER_FAT: usize = 3;
    const ROOT_ENTRIES: usize = 112;
    const MEDIA: u8 = 0xf9;

    /// Builder for a synthetic FAT12 image, so the reader's tests need no
    /// fixture. Writes one FAT, real cluster chains, and real subdirectories.
    pub(crate) struct DiskBuilder {
        image: Vec<u8>,
        /// Next free cluster.
        next: usize,
        root_used: usize,
    }

    impl DiskBuilder {
        pub(crate) fn new(machine: Machine) -> DiskBuilder {
            let mut image = vec![0u8; SECTORS * SECTOR];
            match machine {
                // `EB 3C 90` — the jump every DOS formatter writes, then an OEM
                // string, exactly as `disk1.img` opens.
                Machine::Dos => {
                    image[0..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
                    image[3..11].copy_from_slice(b"MSDOS5.0");
                    image[510..512].copy_from_slice(&[0x55, 0xaa]);
                }
                // What `Infocom Compilation 8` actually opens with: nothing.
                Machine::AtariSt => {}
            }
            image[BPB_BYTES_PER_SECTOR..BPB_BYTES_PER_SECTOR + 2]
                .copy_from_slice(&(SECTOR as u16).to_le_bytes());
            image[BPB_SECTORS_PER_CLUSTER] = SECTORS_PER_CLUSTER as u8;
            image[BPB_RESERVED..BPB_RESERVED + 2].copy_from_slice(&1u16.to_le_bytes());
            image[BPB_FATS] = 2;
            image[BPB_ROOT_ENTRIES..BPB_ROOT_ENTRIES + 2]
                .copy_from_slice(&(ROOT_ENTRIES as u16).to_le_bytes());
            image[BPB_TOTAL_SECTORS..BPB_TOTAL_SECTORS + 2]
                .copy_from_slice(&(SECTORS as u16).to_le_bytes());
            image[BPB_MEDIA] = MEDIA;
            image[BPB_SECTORS_PER_FAT..BPB_SECTORS_PER_FAT + 2]
                .copy_from_slice(&(SECTORS_PER_FAT as u16).to_le_bytes());
            // FAT[0] is the media descriptor, FAT[1] the end-of-chain marker.
            let fat = SECTOR;
            image[fat] = MEDIA;
            image[fat + 1] = 0xff;
            image[fat + 2] = 0xff;
            DiskBuilder { image, next: FIRST_CLUSTER, root_used: 0 }
        }

        fn fat_start(&self) -> usize {
            SECTOR
        }

        fn root_start(&self) -> usize {
            SECTOR + 2 * SECTORS_PER_FAT * SECTOR
        }

        fn data_start(&self) -> usize {
            self.root_start() + ROOT_ENTRIES * DIR_ENTRY
        }

        fn set_fat(&mut self, cluster: usize, value: u16) {
            let at = self.fat_start() + cluster + cluster / 2;
            let raw = u16::from(self.image[at]) | (u16::from(self.image[at + 1]) << 8);
            let new = if cluster.is_multiple_of(2) {
                (raw & 0xf000) | (value & 0x0fff)
            } else {
                (raw & 0x000f) | (value << 4)
            };
            self.image[at..at + 2].copy_from_slice(&new.to_le_bytes());
        }

        /// Write `data` into a fresh cluster chain and return its first cluster.
        fn write_chain(&mut self, data: &[u8]) -> u16 {
            let size = SECTORS_PER_CLUSTER * SECTOR;
            let chunks: Vec<&[u8]> = if data.is_empty() { vec![&[]] } else { data.chunks(size).collect() };
            let first = self.next;
            for (i, chunk) in chunks.iter().enumerate() {
                let c = self.next;
                self.next += 1;
                let at = self.data_start() + (c - FIRST_CLUSTER) * size;
                self.image[at..at + chunk.len()].copy_from_slice(chunk);
                if i + 1 < chunks.len() {
                    self.set_fat(c, (c + 1) as u16);
                } else {
                    self.set_fat(c, 0xfff);
                }
            }
            first as u16
        }

        fn dir_entry(name: &str, attr: u8, cluster: u16, size: u32) -> [u8; DIR_ENTRY] {
            let mut e = [0u8; DIR_ENTRY];
            e[..DIR_ATTR].fill(b' ');
            let (stem, ext) = match name.split_once('.') {
                Some((s, x)) => (s, x),
                None => (name, ""),
            };
            e[..stem.len()].copy_from_slice(stem.as_bytes());
            e[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
            e[DIR_ATTR] = attr;
            e[DIR_FIRST_CLUSTER..DIR_FIRST_CLUSTER + 2].copy_from_slice(&cluster.to_le_bytes());
            e[DIR_SIZE..DIR_SIZE + 4].copy_from_slice(&size.to_le_bytes());
            e
        }

        fn push_root(&mut self, e: [u8; DIR_ENTRY]) {
            let at = self.root_start() + self.root_used * DIR_ENTRY;
            self.image[at..at + DIR_ENTRY].copy_from_slice(&e);
            self.root_used += 1;
        }

        /// A file in the root directory.
        pub(crate) fn add_file(&mut self, name: &str, data: &[u8]) {
            let cluster = self.write_chain(data);
            self.push_root(Self::dir_entry(name, 0, cluster, data.len() as u32));
        }

        /// A subdirectory holding `files` — the shape of `Infocom Compilation 9`.
        pub(crate) fn add_dir(&mut self, dir: &str, files: &[(&str, &[u8])]) {
            // Lay the children down first so their clusters are known, then the
            // directory's own cluster carries the entries that point at them.
            let children: Vec<[u8; DIR_ENTRY]> = files
                .iter()
                .map(|(name, data)| {
                    let c = self.write_chain(data);
                    Self::dir_entry(name, 0, c, data.len() as u32)
                })
                .collect();
            let mut table = Vec::new();
            // Real subdirectories open with `.` and `..`; the walk must skip them.
            table.extend_from_slice(&Self::dir_entry(".", ATTR_DIRECTORY, 0, 0));
            table.extend_from_slice(&Self::dir_entry("..", ATTR_DIRECTORY, 0, 0));
            for c in &children {
                table.extend_from_slice(c);
            }
            let cluster = self.write_chain(&table);
            self.push_root(Self::dir_entry(dir, ATTR_DIRECTORY, cluster, 0));
        }

        /// A volume label, the way the Lost Treasures disks carry one.
        pub(crate) fn add_volume_label(&mut self, label: &str) {
            self.push_root(Self::dir_entry(label, ATTR_VOLUME_LABEL, 0, 0));
        }

        pub(crate) fn finish(self) -> Vec<u8> {
            self.image
        }
    }

    /// One synthetic floppy of `machine` carrying `files` in its root, for the
    /// mount-seam tests in [`crate::medium`]. They need a real volume of every
    /// format and cannot reach a builder that is private to this module.
    pub(crate) fn sample_disk(files: &[(&str, &[u8])], machine: Machine) -> Vec<u8> {
        let mut d = DiskBuilder::new(machine);
        for (name, data) in files {
            d.add_file(name, data);
        }
        d.finish()
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
        b[0x12..0x18].copy_from_slice(b"890323");
        b
    }

    #[test]
    fn rejects_everything_that_is_not_a_fat12_volume() {
        assert!(!Fat12::looks_like_fat12(b"not a disk"));
        assert!(!Fat12::looks_like_fat12(&[]));
        assert!(!Fat12::looks_like_fat12(&vec![0u8; SECTORS * SECTOR]), "no BPB at all");
        // An AmigaDOS boot block: `DOS` then a flag byte, and nothing at 0x0B.
        let mut adf = vec![0u8; 1760 * SECTOR];
        adf[0..3].copy_from_slice(b"DOS");
        assert!(!Fat12::looks_like_fat12(&adf), "an Amiga floppy is not FAT12");
        assert_eq!(Fat12::mount(vec![0u8; 16]).unwrap_err(), Fat12Error::NotFat12);
    }

    /// A BPB whose geometry does not fit the bytes in hand is not a volume, and
    /// this is what keeps the sniff safe to run over an arbitrary story file.
    #[test]
    fn a_bpb_that_does_not_fit_its_image_is_declined() {
        let full = sample_disk(&[("A", b"x")], Machine::Dos);
        assert!(Fat12::looks_like_fat12(&full));
        assert!(!Fat12::looks_like_fat12(&full[..full.len() / 2]), "half a volume is not one");

        // The media descriptor is `F0`…`FF` and nothing else.
        let mut bent = full.clone();
        bent[BPB_MEDIA] = 0x00;
        assert!(!Fat12::looks_like_fat12(&bent), "no media descriptor");

        // …but the FAT's own first byte is NOT required to repeat it: DOS
        // writes `F9 FF FF` there and GEMDOS writes zeros, and both are real.
        let mut atari_fat = full.clone();
        atari_fat[SECTOR..SECTOR + 3].copy_from_slice(&[0, 0, 0]);
        assert!(Fat12::looks_like_fat12(&atari_fat), "every ST disk in the corpus looks like this");

        // Too many clusters to be FAT12.
        let mut big = full.clone();
        big[BPB_SECTORS_PER_CLUSTER] = 1;
        big[BPB_TOTAL_SECTORS..BPB_TOTAL_SECTORS + 2].copy_from_slice(&9000u16.to_le_bytes());
        assert!(!Fat12::looks_like_fat12(&big), "9000 single-sector clusters is FAT16");
    }

    /// **The discriminator.** Same filesystem, same BPB, two machines — and the
    /// x86 jump is the whole of the difference. See the module docs.
    #[test]
    fn the_x86_jump_is_what_tells_dos_from_the_atari_st() {
        let dos = sample_disk(&[("STORY.DAT", &fake_story(4096))], Machine::Dos);
        let st = sample_disk(&[("STORY.DAT", &fake_story(4096))], Machine::AtariSt);
        assert!(looks_like_dos(&dos) && !looks_like_atari_st(&dos));
        assert!(looks_like_atari_st(&st) && !looks_like_dos(&st));
        assert_eq!(Fat12::mount(dos.clone()).unwrap().machine(), Machine::Dos);
        assert_eq!(Fat12::mount(st.clone()).unwrap().machine(), Machine::AtariSt);

        // The BPBs are byte-identical from 0x0B on; only the boot code differs.
        assert_eq!(dos[0x0b..0x18], st[0x0b..0x18], "GEMDOS puts its BPB at the DOS offsets");

        // `E9 xx xx` is the other DOS jump form and must be read as one.
        let mut near = st.clone();
        near[0..3].copy_from_slice(&[0xe9, 0x11, 0x22]);
        assert!(looks_like_dos(&near), "E9 is a near jump, and just as much a DOS one");

        // A 68000 `BRA.S` is not an x86 jump: a BOOTABLE ST disk stays an ST one.
        let mut bootable = st.clone();
        bootable[0..2].copy_from_slice(&[0x60, 0x38]);
        assert!(looks_like_atari_st(&bootable), "BRA.S is 68000 code, not an x86 jump");
    }

    #[test]
    fn round_trips_files_of_every_length() {
        for machine in [Machine::Dos, Machine::AtariSt] {
            let mut d = DiskBuilder::new(machine);
            let small = b"hello from 1987".to_vec();
            // Several clusters, ending mid-cluster so the truncation matters.
            let big: Vec<u8> = (0..40 * SECTOR + 7).map(|i| (i % 251) as u8).collect();
            d.add_file("SMALL.TXT", &small);
            d.add_file("BIG.DAT", &big);
            d.add_file("EMPTY.DAT", b"");
            let fs = Fat12::mount(d.finish()).expect("mounts");

            assert_eq!(fs.machine(), machine);
            let names: Vec<String> = fs.files().iter().map(Fat12Entry::path).collect();
            assert_eq!(names, ["SMALL.TXT", "BIG.DAT", "EMPTY.DAT"], "{machine:?}");
            assert_eq!(fs.read_named("small.txt").as_deref(), Some(&small[..]), "{machine:?}");
            assert_eq!(fs.read_named("BIG.DAT").as_deref(), Some(&big[..]), "{machine:?}");
            assert_eq!(fs.read_named("EMPTY.DAT").as_deref(), Some(&[][..]), "{machine:?}");
            assert_eq!(fs.read_named("ABSENT.DAT"), None);
        }
    }

    /// The shape of `Infocom Compilation 9`: four games, four folders, four
    /// files called `STORY.DAT`. Root-only enumeration finds none of them, and
    /// the filename tells them apart not at all.
    #[test]
    fn subdirectories_are_walked_and_the_directory_names_the_game() {
        let mut d = DiskBuilder::new(Machine::AtariSt);
        d.add_dir("HITCHHIK", &[("STORY.DAT", &fake_story(4096)), ("HITCHHIK.PRG", b"\x60\x1a")]);
        d.add_dir("CUTHROAT", &[("STORY.DAT", &fake_story(8192))]);
        d.add_file("DESKTOP.INF", b"#a000000\r\n");
        let fs = Fat12::mount(d.finish()).expect("mounts");

        let paths: Vec<String> = fs.files().iter().map(Fat12Entry::path).collect();
        assert_eq!(
            paths,
            ["HITCHHIK/STORY.DAT", "HITCHHIK/HITCHHIK.PRG", "CUTHROAT/STORY.DAT", "DESKTOP.INF"],
            "the `.` and `..` entries are not files, and the folder names the game"
        );
        assert_eq!(fs.read_named("CUTHROAT/STORY.DAT").map(|b| b.len()), Some(8192));
    }

    #[test]
    fn finds_a_story_by_content_not_by_name() {
        let mut d = DiskBuilder::new(Machine::Dos);
        // `.ZIP` is Infocom's DOS extension for a bare story file, not PKZIP.
        d.add_file("ZORK0.ZIP", &fake_story(4096));
        d.add_file("ZORKZERO.EXE", b"MZ\x90\x00\x03\x00\x00\x00");
        let fs = Fat12::mount(d.finish()).expect("mounts");
        let (path, bytes) = fs.story().expect("the story is found under any extension");
        assert_eq!(path, "ZORK0.ZIP");
        assert_eq!(bytes.len(), 4096);
    }

    /// The negative SQ-0835 asked for outright: `Infocom Compilation 9` carries
    /// someone's 1996 saved games, and a content scan that offered one as a game
    /// would be worse than no scan.
    #[test]
    fn saved_games_and_interpreters_are_not_stories() {
        let mut d = DiskBuilder::new(Machine::AtariSt);
        d.add_dir(
            "BUREAUCR.ACY",
            &[
                ("STORY.DAT", &fake_story(4096)),
                ("BILL1.SAV", &vec![0x03u8; 19456]),
                ("BILL2.SAV", &{
                    let mut s = vec![0u8; 19456];
                    s[0] = 0x03;
                    s[1] = 0xaa;
                    s
                }),
                ("BUREAUCR.PRG", &vec![0x60u8; 21706]),
            ],
        );
        let fs = Fat12::mount(d.finish()).expect("mounts");
        let stories: Vec<String> = fs
            .files()
            .iter()
            .filter(|e| fs.read(e).is_some_and(|b| looks_like_story(&b)))
            .map(Fat12Entry::path)
            .collect();
        assert_eq!(stories, ["BUREAUCR.ACY/STORY.DAT"], "only the game is a game");
    }

    /// Seven of nine ST compilations and every Lost Treasures disk hold several
    /// games, so the single-story tiebreak has to be deterministic.
    #[test]
    fn the_conventional_name_only_breaks_a_tie() {
        let mut d = DiskBuilder::new(Machine::AtariSt);
        d.add_dir("TRINITY", &[("STORY.DAT", &fake_story(4096))]);
        d.add_file("BIGGER.T", &fake_story(8192));
        let fs = Fat12::mount(d.finish()).expect("mounts");
        assert_eq!(fs.story().expect("a story").0, "TRINITY/STORY.DAT", "convention beats size");

        let mut d = DiskBuilder::new(Machine::Dos);
        d.add_file("ZORK1.DAT", &fake_story(4096));
        d.add_file("ZORK3.DAT", &fake_story(8192));
        let fs = Fat12::mount(d.finish()).expect("mounts");
        assert_eq!(fs.story().expect("a story").0, "ZORK3.DAT", "no convention → the largest");
    }

    /// The DOS disks label their volumes and the ST disks do not; both answers
    /// have to read cleanly, and the padding a real label carries has to go.
    #[test]
    fn a_volume_label_is_read_and_trimmed() {
        let mut d = DiskBuilder::new(Machine::Dos);
        d.add_volume_label("Tresure 1");
        d.add_file("ZORK1.DAT", &fake_story(4096));
        let fs = Fat12::mount(d.finish()).expect("mounts");
        assert_eq!(fs.volume_label(), Some("Tresure 1"), "trailing spaces and NULs are padding");
        assert_eq!(fs.files().len(), 1, "the label is not a file");

        let unlabelled = Fat12::mount(sample_disk(&[("A.DAT", b"x")], Machine::AtariSt)).unwrap();
        assert_eq!(unlabelled.volume_label(), None);
    }

    /// An installer disk carries files and no game — Lost Treasures floppy4
    /// holds *Zork Zero*'s CGA artwork and no *Zork Zero*, and the 360 KB
    /// release's disk 1 is an installer. Saying so is what lets a caller ask
    /// "is this the right disk?" instead of reporting a corrupt story.
    #[test]
    fn a_disk_with_no_game_mounts_and_offers_nothing() {
        let mut d = DiskBuilder::new(Machine::Dos);
        d.add_file("INSTALL.EXE", b"MZ\x90\x00");
        d.add_file("ANSI3.SYS", &vec![0x11u8; 1678]);
        let fs = Fat12::mount(d.finish()).expect("mounts");
        assert_eq!(fs.files().len(), 2);
        assert_eq!(fs.story(), None);
        assert!(fs.pictures().is_none());
    }

    /// A cyclic FAT is a damaged disk, not a hang.
    #[test]
    fn a_cyclic_cluster_chain_terminates() {
        let mut d = DiskBuilder::new(Machine::Dos);
        d.add_file("LOOP.DAT", &vec![0x5au8; 8 * SECTOR]);
        let first = 2usize;
        d.set_fat(first + 3, first as u16);
        let fs = Fat12::mount(d.finish()).expect("mounts");
        // Whatever it reads, it stops reading: the file is short, so it is
        // reported unreadable rather than looping.
        let _ = fs.read(&fs.files()[0]);
    }

    // ── Real media ───────────────────────────────────────────────────────────

    /// The user's `stories/` directory, which is gitignored — every test over it
    /// skips vacuously.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn read_fixture(name: &str) -> Option<Vec<u8>> {
        let path = stories_dir().join(name);
        match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(_) => {
                eprintln!("SKIP: gitignored medium missing at {}", path.display());
                None
            }
        }
    }

    /// `The Lost Treasures of Infocom` I, floppy5 — the disk *Zork Zero*'s story
    /// is actually on, with its EGA artwork beside it. r393 s890714, and the
    /// `.ZIP` is a bare story file rather than a PKZIP archive.
    #[test]
    fn real_dos_release_floppy() {
        let Some(bytes) = read_fixture("floppy5.ima") else { return };
        assert!(looks_like_dos(&bytes), "an x86 jump and a FAT12 BPB");
        let fs = Fat12::mount(bytes).expect("floppy5 mounts");
        assert_eq!(fs.machine(), Machine::Dos);
        assert_eq!(fs.volume_label(), Some("Tresure 5"));
        let paths: Vec<String> = fs.files().iter().map(Fat12Entry::path).collect();
        assert_eq!(paths, ["ZORK0.EG1", "ZORK0.ZIP"]);

        let (name, story) = fs.story().expect("Zork Zero is on this disk");
        assert_eq!(name, "ZORK0.ZIP");
        assert_eq!(story.len(), 300032);
        assert_eq!(story[0], 6, "Zork Zero is v6");
        assert_eq!(u16::from_be_bytes([story[2], story[3]]), 393);
        assert_eq!(&story[0x12..0x18], b"890714");

        let (pname, pics) = fs.pictures().expect("…and its EGA artwork");
        assert_eq!(pname, "ZORK0.EG1");
        assert_eq!(pics.flavour(), crate::infocom_pics::Flavour::Pc);
        assert!(pics.entries().len() > 100, "{} pictures", pics.entries().len());
    }

    /// The standalone 360 KB *Hitchhiker's* disk: a different geometry (720
    /// sectors), a `DEMO` subdirectory, and a v3 r58 release that is neither of
    /// the other two Hitchhiker's in this corpus.
    #[test]
    fn real_dos_disk_with_a_subdirectory() {
        let Some(bytes) = read_fixture(
            "Hitchhiker's Guide to the Galaxy, The (1987) (r58, Serial 851002) \
             (Infocom, Inc.) (360K) [!].ima",
        ) else {
            return;
        };
        assert!(looks_like_dos(&bytes));
        let fs = Fat12::mount(bytes).expect("mounts");
        assert_eq!(fs.files().len(), 19, "12 in the root and 7 in DEMO");
        assert!(
            fs.files().iter().any(|e| e.path() == "DEMO/DPRES.EXE"),
            "a root-only walk would miss the DEMO folder"
        );
        let (name, story) = fs.story().expect("the game");
        assert_eq!(name, "HITCHHIK.DAT");
        assert_eq!((story[0], u16::from_be_bytes([story[2], story[3]])), (3, 58));
        assert_eq!(&story[0x12..0x18], b"851002");
    }

    /// `Infocom Compilation 9` — the ST disk SQ-0835 surveyed: four games in
    /// four folders, all four called `STORY.DAT`, with saved games beside them.
    #[test]
    fn real_atari_st_compilation() {
        let Some(bytes) = read_fixture("Infocom Compilation 9 (19xx)(-).st") else { return };
        assert!(looks_like_atari_st(&bytes), "a FAT12 BPB with no x86 jump in front of it");
        let fs = Fat12::mount(bytes).expect("the ST disk mounts");
        assert_eq!(fs.machine(), Machine::AtariSt);
        assert_eq!(fs.volume_label(), None, "no ST compilation labels its volume");

        let stories: Vec<(String, u8, u16, String)> = fs
            .files()
            .iter()
            .filter_map(|e| fs.read(e).map(|b| (e.path(), b)))
            .filter(|(_, b)| looks_like_story(b))
            .map(|(p, b)| {
                (p, b[0], u16::from_be_bytes([b[2], b[3]]), String::from_utf8_lossy(&b[0x12..0x18]).into_owned())
            })
            .collect();
        assert_eq!(
            stories,
            [
                ("HITCHHIK/STORY.DAT".to_string(), 3, 56, "841221".to_string()),
                ("BUREAUCR.ACY/STORY.DAT".to_string(), 4, 86, "870212".to_string()),
                ("CUTHROAT/STORY.DAT".to_string(), 3, 23, "840809".to_string()),
                ("LEATHER.GOD/STORY.DAT".to_string(), 3, 59, "860730".to_string()),
            ],
            "the directory names the game; the filename never does"
        );
        assert!(
            fs.files().iter().any(|e| e.name == "BILL1.SAV"),
            "the saved games are on the disk…"
        );
        assert!(
            !stories.iter().any(|(p, ..)| p.ends_with(".SAV")),
            "…and none of them is offered as a game"
        );
    }

    /// **The corroboration the discriminator deliberately does not use.** Every
    /// DOS image in the corpus carries `55 AA` at 510 and every ST image does
    /// not, which is why the jump test is safe — and pinning it here means a
    /// counterexample arriving in `stories/` is caught rather than assumed away.
    #[test]
    fn every_image_in_the_corpus_agrees_on_which_machine_pressed_it() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut seen = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let Ok(raw) = std::fs::read(&path) else { continue };
            let Some(machine) = machine_of(&raw) else { continue };
            seen += 1;
            let signature = raw[510..512] == [0x55, 0xaa];
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            match machine {
                Machine::Dos => assert!(signature, "{name}: a DOS image without a 55 AA signature"),
                Machine::AtariSt => {
                    assert!(!signature, "{name}: an ST image carrying a 55 AA signature")
                }
            }
            // And whatever we claim, we can open.
            let fs = Fat12::mount(raw).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(!fs.files().is_empty(), "{name}: mounted but empty");
        }
        if seen == 0 {
            eprintln!("SKIP: no FAT12 media present");
        }
    }
}
