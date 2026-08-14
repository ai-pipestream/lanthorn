//! The one mount path for a release disk image: what it is, what it holds, and
//! the machine it implies (SQ-0839, SQ-0840).
//!
//! The user's rule, twice over: *"in general when reading a specific disk format
//! we should default the interpreter number to match (even on the cli), but
//! still allow an override"*, and then *"please keep the functionality
//! consistent for all disk image formats — we should have a standard api in our
//! blorb crate for disk images."*
//!
//! So this module is the standard API, and it is the **only** place in the
//! workspace that names a disk format. Everything a mounted disk is asked for
//! — is it an image ([`DiskImage::detect`]), open it ([`MountedDisk::mount`]),
//! what stories are on it ([`MountedDisk::stories`]), which one to play
//! ([`MountedDisk::story`]), what artwork ([`MountedDisk::pictures`]), what it
//! calls itself ([`MountedDisk::label`]), what machine it implies
//! ([`MountedDisk::interpreter_number`]) — is answered here, in one vocabulary,
//! for every format alike. No front-end matches on a format, and none can.
//!
//! ## Why that matters, in this codebase's own history
//!
//! Before SQ-0840 the policy was an `if looks_like_adf … else if looks_like_hfs`
//! chain, written out three times: once in the TUI's picture resolution, once in
//! its story loading, and once — with the HFS arm simply missing — in `zvm-cli`.
//! The CLI therefore mounted an Amiga floppy and refused a Macintosh one, months
//! after `blorb` had learned to read that disk. Neither lane was wrong inside its
//! own scope; the chain guaranteed that nobody owned the join. Adding a format is
//! now a row in [`FORMATS`] and an `impl Volume` beside it, both in this file,
//! and every front-end gains the format in the same commit.
//!
//! ## Two invariants worth stating outright
//!
//! **Detect and mount cannot disagree.** Both walk [`FORMATS`], so a format this
//! crate can recognise is a format it can open — the property `zvm-cli`'s old
//! `looks_like_image` had to guard against by hand, and no longer does.
//!
//! **Recognition is by CONTENT, never by extension.** A disk image under any
//! name is recognised, and a mis-named ordinary story file is not — the same
//! rule the readers apply file-by-file inside a volume, where it is not
//! negotiable: every Atari ST story is called `STORY.DAT`, five of nine Amiga
//! floppies call theirs `Story.data`, `.ima` and `.img` are one format, and the
//! ProDOS `2IMG` length field reads zero on every image in the corpus.
//!
//! A row does carry [`Format::extensions`], and that is not a crack in the rule:
//! it is the census a front-end scanning a DIRECTORY needs to decide which files
//! are worth OPENING, and what a file turns out to be is still
//! [`DiskImage::detect`]'s answer over its bytes. See [`DiskImage::extensions`]
//! for why it lives here and nowhere else — it lived in the TUI once, and went
//! stale the moment a format arrived (SQ-0849).

use crate::adf::{Adf, looks_like_story};
use crate::fat12::Fat12;
use crate::hfs::Hfs;
use crate::infocom_pics::InfocomPics;
use crate::prodos::ProDos;

/// Amiga, from the ZMSD §11.1.3 interpreter-number table (1 DECSystem-20,
/// 2 Apple IIe, 3 Macintosh, **4 Amiga**, 5 Atari ST, 6 IBM PC, 7 Commodore 128,
/// 8 Commodore 64, 9 Apple IIc, 10 Apple IIgs, 11 Tandy Color).
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

/// Macintosh, from the same ZMSD §11.1.3 table, read from the standard rather
/// than recalled: *"Infocom used the interpreter numbers: 1 DECSystem-20,
/// 2 Apple IIe, **3 Macintosh**, 4 Amiga, 5 Atari ST, 6 IBM PC, …"* — and the
/// same section is explicit that this matters here in particular: *"In Version
/// 6, the decision is more serious, as existing Infocom story files depend on
/// interpreter number in many ways"*.
pub const MACINTOSH_INTERPRETER_NUMBER: u8 = 3;

/// Atari ST, from the same §11.1.3 table — *"… 4 Amiga, **5 Atari ST**,
/// 6 IBM PC …"* — and, unusually, corroborated by the machine's own
/// interpreters rather than by the standard alone (SQ-0835).
///
/// Infocom's Atari ST sources write this byte themselves, and say so. Both the
/// XZIP (Version 5) and the EZIP (Version 4) builds carry the identical pair of
/// lines, at `PINTWD EQU 30` — decimal 30 is `$1E`:
///
/// ```text
///   st/stx1.s:384   PINTWD  EQU     30      * INTERPRETER ID/VERSION
///   st/stx1.s:422   INTWRD  DC.B    5       * MACHINE ID FOR ATARI ST
///   st/stx1.s:731           MOVE.W  INTWRD,PINTWD(A2) * SET INTERPRETER ID/VERSION WORD
/// ```
///
/// **It is a flat constant, not a version-dependent rule** — which is the
/// question worth asking here, because the IBM PC's honest number *is*
/// version-dependent and that is exactly why [`DiskImage::Fat12Dos`] answers
/// `None`. The ST has no such rule. The one conditional in `st/stzip.s` is the
/// Version 3 assembly, and it declines the byte rather than claiming a different
/// machine:
///
/// ```text
///   st/stzip.s:339      IFEQ EZIP
///   st/stzip.s:340  INTWRD  DC.B    5       * MACHINE ID FOR ATARI ST
///   st/stzip.s:344      IFEQ CZIP
///   st/stzip.s:345  INTWRD  DC.B    0       * (UNUSED)
/// ```
///
/// Byte `$1E` carries no meaning before Version 4, so the Version 3 build leaves
/// it zero and comments it "(UNUSED)". Advertising 5 to a Version 3 story is
/// therefore harmless rather than merely tolerable, and it is *measured* so:
/// all thirty-two v3 stories in the nine-compilation ST corpus produce a
/// byte-identical trace under 1 and under 5.
pub const ATARI_ST_INTERPRETER_NUMBER: u8 = 5;

/// Which release medium a story was mounted out of, when it was one at all.
///
/// The variant is the mount's own answer — every one of them is decided by the
/// image's own filesystem rather than by its filename. Callers use it to NAME
/// the container (the picker's TYPE column) and, via
/// [`DiskImage::interpreter_number`], to imply the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskImage {
    /// An Amiga AmigaDOS release floppy — conventionally `.adf` (SQ-0719).
    Adf,
    /// A Macintosh HFS volume, bare or inside a DiskCopy 4.2 wrapper —
    /// conventionally `.image` (SQ-0837).
    Hfs,
    /// An IBM PC / MS-DOS FAT12 release floppy — conventionally `.ima` or
    /// `.img`, and the two are the same thing (SQ-0833).
    Fat12Dos,
    /// An Atari ST GEMDOS floppy — conventionally `.st` (SQ-0835). The **same**
    /// FAT12 filesystem as [`DiskImage::Fat12Dos`], down to the byte offsets of
    /// the BPB, and a different machine; see [`crate::fat12`] for the
    /// discriminator.
    Fat12AtariSt,
    /// An Apple II ProDOS volume, bare or inside a `2IMG` wrapper —
    /// conventionally `.2mg` (SQ-0836).
    ///
    /// **The one format here that names a FAMILY rather than a machine.**
    /// ProDOS is the Apple II's filesystem from the IIe on, and ZMSD §11.1.3
    /// gives that family three interpreter numbers — 2 Apple IIe, 9 Apple IIc,
    /// 10 Apple IIgs. The corpus contains both kinds of press, so the ambiguity
    /// is live rather than theoretical; see this variant's row in [`FORMATS`],
    /// where the consequence for [`DiskImage::interpreter_number`] is argued.
    ProDos,
}

impl DiskImage {
    /// Which release medium `raw` is, or `None` when it is not one.
    ///
    /// The sniffs are disjoint by construction — AmigaDOS is identified by its
    /// `DOS` boot block and HFS by a volume signature at a fixed offset (bare, or
    /// past a DiskCopy 4.2 header) — so the order of [`FORMATS`] is a formality
    /// rather than a precedence.
    ///
    /// Whatever this recognises, [`MountedDisk::mount`] opens: both walk the same
    /// table, so the two cannot drift apart.
    pub fn detect(raw: &[u8]) -> Option<DiskImage> {
        format_of(raw).map(|f| f.image)
    }

    /// Every format this crate reads, in table order. The census the API walks
    /// — a format is here exactly when it has a row in [`FORMATS`].
    pub fn all() -> impl Iterator<Item = DiskImage> {
        FORMATS.iter().map(|f| f.image)
    }

    /// This variant's row. Every variant has one; a variant added to the enum
    /// without a row in [`FORMATS`] is a format nothing can detect, mount or
    /// name, and the census test in this module catches it.
    fn row(self) -> &'static Format {
        FORMATS.iter().find(|f| f.image == self).expect("every DiskImage variant has a FORMATS row")
    }

    /// The acronym a story list shows beside the format: `Z6 (ADF)`, `Z6 (HFS)`.
    pub fn label(self) -> &'static str {
        self.row().label
    }

    /// The Z-machine interpreter number this medium DEFAULTS to — header byte
    /// `$1E`, ZMSD §11.1.3 — or `None` to leave whatever rule was already in
    /// force (each front-end's own default: Frotz's 6-for-v6, 1-otherwise).
    ///
    /// A default, never an override: every caller must let an explicitly
    /// requested number win over this one. That ordering is the other half of
    /// the user's rule and is pinned on both front-ends.
    ///
    /// [`DiskImage::Hfs`] answered `None` until SQ-0838, on the grounds that the
    /// number is not inert — a game reads `$1E` and can take machine-specific
    /// paths — and that the Macintosh's screen, colours and palette were not
    /// established by anything in hand, so `3` would have been a byte without a
    /// machine behind it. They are established now, out of Infocom's own
    /// Macintosh interpreter (`mac/xzip.lst`, `mac/gfx.p`), and the whole bundle
    /// ships together in `app::interpreter::InterpreterProfile::Macintosh`.
    pub fn interpreter_number(self) -> Option<u8> {
        self.row().interpreter_number
    }

    /// The filename extensions this format is CONVENTIONALLY given — lowercase,
    /// no leading dot.
    ///
    /// **A pre-filter, never evidence.** Nothing may conclude a format from a
    /// name: a front-end walking a directory uses this to decide which files are
    /// worth reading, and then asks [`DiskImage::detect`] what it actually got.
    /// So a disk image under an unexpected name still mounts by every path that
    /// sniffs content, and a `.img` full of holiday photos is opened, refused and
    /// never listed.
    ///
    /// **Why it is a property of the row.** The TUI's story picker kept its own
    /// extension list, and that list was the "nothing else anywhere" in
    /// [`FORMATS`]' doc that turned out to exist. SQ-0833 and SQ-0835 added the
    /// DOS and Atari ST rows; the picker never learned their names, so a shelf
    /// full of `.ima` and `.st` floppies that mount perfectly well was simply
    /// absent from the story list, silently, for two quests (SQ-0849). A census
    /// the table owns cannot drift from the table.
    pub fn extensions(self) -> &'static [&'static str] {
        self.row().extensions
    }
}

/// Every extension any format in [`FORMATS`] is conventionally given, in table
/// order — the whole census, for a caller that has a filename and no bytes yet.
///
/// This is what a directory scan pre-filters on; see [`DiskImage::extensions`]
/// for what it is and is not allowed to mean. The formats' sets are disjoint
/// today, and a caller must not assume it: the union is what it wants, because
/// which row claimed a spelling is [`DiskImage::detect`]'s business.
pub fn image_extensions() -> impl Iterator<Item = &'static str> {
    FORMATS.iter().flat_map(|f| f.extensions.iter().copied())
}

// ── The one table ─────────────────────────────────────────────────────────────

/// One disk format: how to recognise it, how to open it, and what it implies.
///
/// The whole point of the struct is that a format is a **row**, not a branch
/// scattered through the callers. Nothing outside this module ever sees one.
struct Format {
    /// The [`DiskImage`] variant this row is.
    image: DiskImage,
    /// See [`DiskImage::label`].
    label: &'static str,
    /// See [`DiskImage::interpreter_number`].
    interpreter_number: Option<u8>,
    /// See [`DiskImage::extensions`]. Lowercase, no dot, at least one — a row
    /// with none is a format a directory scan can never offer, and the census
    /// test in this module says so.
    extensions: &'static [&'static str],
    /// The content sniff — [`Volume::looks_like`] for the reader below.
    looks_like: fn(&[u8]) -> bool,
    /// The mount — [`Volume::mount`], boxed so the table can be one type.
    mount: fn(Vec<u8>) -> Option<Box<dyn Volume>>,
}

/// **Every disk format babelmap reads.** Adding one is a row here plus the
/// `impl Volume` it names, and nothing else anywhere: `detect`, `mount`,
/// `label`, `interpreter_number`, **the extensions a directory scan
/// pre-filters on**, the TUI's story loading and picture resolution, and
/// `zvm-cli`'s disk menu all read this table and none of them knows a format's
/// name.
///
/// The extensions column is the newest of those, and it is here because the one
/// front-end that needed it kept its own copy instead: `picker::STORY_EXTS`
/// listed `adf` and `image` and never heard about `ima`, `img` or `st`, so two
/// shipped formats were unreachable from the story list. That is exactly the
/// half-wiring the table exists to make impossible, one column late (SQ-0849).
///
/// Queued: Apple II DOS 3.3 (SQ-0852). It has no reader, so it has no row **or
/// an extension** — `.dsk` arrives with the code that opens it, in one commit,
/// which is the whole point of the table.
const FORMATS: &[Format] = &[
    Format {
        image: DiskImage::Adf,
        label: "ADF",
        interpreter_number: Some(AMIGA_INTERPRETER_NUMBER),
        // Every Amiga floppy in the corpus is `.adf`; the format has no second
        // customary spelling.
        extensions: &["adf"],
        looks_like: <Adf as Volume>::looks_like,
        mount: mount_boxed::<Adf>,
    },
    Format {
        image: DiskImage::Hfs,
        label: "HFS",
        interpreter_number: Some(MACINTOSH_INTERPRETER_NUMBER),
        // `.image` is DiskCopy 4.2's own name and what the corpus uses (`Zork
        // Zero Disk.image`). Macintosh volumes also circulate as `.img` and
        // `.dsk`; the first is already admitted by the DOS row below and the
        // union is what a scan pre-filters on, so an HFS `.img` is opened and
        // mounts. `.dsk` is deliberately absent — it is overwhelmingly Apple II
        // media, which has no reader yet (SQ-0852).
        extensions: &["image"],
        looks_like: <Hfs as Volume>::looks_like,
        mount: mount_boxed::<Hfs>,
    },
    // ── One filesystem, two machines (SQ-0833, SQ-0835) ──────────────────────
    //
    // These two rows share a reader and differ only in the sniff, because
    // GEMDOS puts its BPB at the DOS offsets and a plain DOS parser reads an
    // Atari disk with no Atari-specific code in it. The MACHINE is a separate
    // question, asked of the boot sector — see `crate::fat12`.
    Format {
        image: DiskImage::Fat12Dos,
        label: "DOS",
        // **`None`, and that is the IBM PC's answer rather than a gap.** This
        // codebase's IBM PC bundle — `app::interpreter::InterpreterProfile::IbmPc`,
        // where a DOS disk resolves — deliberately returns no number of its
        // own, because the honest one is version-dependent (Frotz's rule: 6 for
        // Version 6, 1 otherwise) and no single constant expresses it. So
        // `None` here means "the rule already in force IS the IBM PC's", which
        // is exactly true, and a DOS floppy behaves as it always has.
        //
        // Hard-coding ZMSD §11.1.3's 6 would not be inert: `BEYONDZO.DAT` sits
        // on `floppy1.ima` and *Beyond Zork* swaps Font 3 arrows for CP437
        // character graphics when it believes it is on an IBM PC. That may well
        // be the authentic Lost Treasures experience — it is also a visible
        // rendering change on real media that nothing in this lane establishes,
        // and it belongs to whoever can look at it.
        interpreter_number: None,
        // Two spellings of one thing, as this module's header already says:
        // `floppy1.ima` and `disk1.img` are the same raw sector dump and the
        // same reader opens both.
        extensions: &["ima", "img"],
        looks_like: crate::fat12::looks_like_dos,
        mount: mount_boxed::<Fat12>,
    },
    Format {
        image: DiskImage::Fat12AtariSt,
        label: "ST",
        // **5, the Atari ST** (SQ-0835's profile half). This row answered `None`
        // for one commit, on the argument that no ST press of a graphical v6
        // title exists so the number would be "a byte with no verified machine
        // behind it". That argument was wrong, and its own premise is why: the
        // failure it guards against — a number that changes what games do while
        // the artwork keeps another machine's scale — **cannot arise on a corpus
        // with no artwork in it.** All thirty-nine stories across the nine ST
        // compilations are v3, v4 or v5, so there is nothing for the number to
        // disagree with.
        //
        // What replaced the argument is evidence. Infocom's own ST interpreters
        // write 5 into `$1E` unconditionally and label it "MACHINE ID FOR ATARI
        // ST"; see [`ATARI_ST_INTERPRETER_NUMBER`], which quotes them and shows
        // the byte is a flat constant rather than the version-dependent rule
        // that keeps [`DiskImage::Fat12Dos`] at `None`.
        //
        // The rest of the bundle is in `app::interpreter::InterpreterProfile::AtariSt`,
        // and it declines the one member nothing establishes: the ST never had a
        // YZIP, so it has no Version 6 art geometry to state.
        interpreter_number: Some(ATARI_ST_INTERPRETER_NUMBER),
        // `.st` is the raw ST sector dump, which is what this reader opens and
        // what all nine compilations in the corpus are. **Not `.msa`**: Magic
        // Shadow Archiver images are RLE-compressed with their own header, not
        // FAT12 at offset zero, so `looks_like_atari_st` refuses one and a row
        // claiming the name would be advertising a format with no reader — the
        // same half-wiring the extensions column was added to end. It arrives
        // with a decompressor or not at all.
        extensions: &["st"],
        looks_like: crate::fat12::looks_like_atari_st,
        mount: mount_boxed::<Fat12>,
    },
    Format {
        image: DiskImage::ProDos,
        label: "ProDOS",
        // **`None`, and it is the Apple II's answer rather than a gap** — the
        // same shape as [`DiskImage::Fat12Dos`] above and for the same kind of
        // reason: the honest number is not a constant.
        //
        // ZMSD §11.1.3, read from the standard: *"1 DECSystem-20, 2 Apple IIe,
        // 3 Macintosh, 4 Amiga, 5 Atari ST, 6 IBM PC, 7 Commodore 128,
        // 8 Commodore 64, 9 Apple IIc, 10 Apple IIgs, 11 Tandy Color"*. That is
        // THREE numbers for one family, and ProDOS is the whole family's
        // filesystem — a ProDOS volume says "an Apple II" and nothing finer.
        //
        // The corpus proves the ambiguity is live rather than pedantic. Eight of
        // the ten images boot GS/OS and carry `SYS16` applications
        // (`SYSTEM/START.GS.OS`, `SYSTEM/TOOLS/TOOL0xx`, `BZ.SYS16`,
        // `LOST1.SYS16`), which is 16-bit Apple **IIgs** software and runs on
        // nothing else. The other two — `Arthur Quest 4 Excalibur.2mg` and
        // `Journey.2mg` — ship `INFOCOM.SYSTEM`, a ProDOS **8** `SYS` file
        // beside `BASIC.SYSTEM`, which is the 8-bit press and runs on a IIe as
        // readily as on a IIgs. One filesystem, two machines, and unlike the
        // Atari ST there is no interpreter source in hand that writes a flat
        // byte for either.
        //
        // Stating 10 anyway would also be **half-wiring**, which is the thing
        // this table exists to prevent: `app::interpreter::InterpreterProfile`
        // has no Apple arm, so `for_interpreter_number(10)` falls through to
        // `IbmPc`, whose own number is `None` — the TUI would advertise zvm's
        // default while `zvm-cli` and the launch dialog, which read this row
        // directly, advertised 10. A number is only honest here once there is an
        // Apple bundle to carry it, and this corpus makes that harder rather
        // than easier: it holds *Arthur* and *Journey*, so unlike the ST there
        // IS Version 6 artwork for a wrongly-claimed machine to disagree with.
        //
        // So `None` means "the rule already in force stands", which for a family
        // whose own number cannot be named is exactly right.
        interpreter_number: None,
        // `.2mg` is the wrapper every image in the corpus wears. **Not `.po` or
        // `.hdv`**, the conventional names for a BARE ProDOS volume: this reader
        // opens one (see [`crate::prodos`]) but nothing in `stories/` is one, so
        // claiming the spelling would be a census entry no medium here justifies.
        // A bare volume under any name still mounts when it is opened directly —
        // the extension is a directory scan's pre-filter, never the recogniser.
        extensions: &["2mg"],
        looks_like: <ProDos as Volume>::looks_like,
        mount: mount_boxed::<ProDos>,
    },
];

/// The row whose sniff claims `raw`, if any. The single point at which bytes
/// become a format — [`DiskImage::detect`] and [`MountedDisk::mount`] both go
/// through it, which is why they cannot answer differently.
fn format_of(raw: &[u8]) -> Option<&'static Format> {
    FORMATS.iter().find(|f| (f.looks_like)(raw))
}

/// [`Volume::mount`] with the concrete type erased, so one `fn` pointer type
/// serves every row.
fn mount_boxed<V: Volume + Sized + 'static>(raw: Vec<u8>) -> Option<Box<dyn Volume>> {
    V::mount(raw).map(|v| Box::new(v) as Box<dyn Volume>)
}

// ── What a mounted disk holds ─────────────────────────────────────────────────

/// A story found on a volume, with the name it was stored under.
///
/// The name is for a listing, never for identification: it is `Story.data` on
/// five of nine Amiga floppies and `STORY.DAT` on every Atari ST compilation.
/// What makes this a story is that its bytes are one — see
/// [`crate::adf::looks_like_story`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskStory {
    /// How the volume spells this file: the stored filename, prefixed by its
    /// directory on a format that has them (`HITCHHIK/STORY.DAT`). Still not an
    /// identifier — but on an ST compilation the directory is the only thing
    /// that tells four files called `STORY.DAT` apart, so it is carried.
    pub name: String,
    /// The story image, byte-exact off the disk.
    pub bytes: Vec<u8>,
}

impl From<(String, Vec<u8>)> for DiskStory {
    fn from((name, bytes): (String, Vec<u8>)) -> DiskStory {
        DiskStory { name, bytes }
    }
}

/// The native Infocom picture archive a release disk carries, with its stored
/// name. The story and the art came off the same floppy, so the pairing is
/// guaranteed by the medium and needs no configuration.
#[derive(Debug)]
pub struct DiskArt {
    /// The stored filename, e.g. `Pic.data` on an Amiga disk, `CPic.data` on a
    /// Macintosh one.
    pub name: String,
    /// The parsed archive.
    pub pictures: InfocomPics,
}

impl From<(String, InfocomPics)> for DiskArt {
    fn from((name, pictures): (String, InfocomPics)) -> DiskArt {
        DiskArt { name, pictures }
    }
}

/// Why a disk would not open. **Format-neutral on purpose**: a caller reports
/// that a disk did not mount, never that an HFS catalog B*-tree was short — the
/// front-ends do not know what a catalog is, and must not have to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountError {
    /// Not a disk image in any format this crate reads.
    NotADiskImage,
    /// Recognised as `.0`, but its filesystem would not read.
    Unreadable(DiskImage),
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountError::NotADiskImage => f.write_str("not a disk image in any format we read"),
            MountError::Unreadable(image) => {
                write!(f, "the {} filesystem on it would not read", image.label())
            }
        }
    }
}

// ── The standard API a format implements ──────────────────────────────────────

/// **The standard disk-image API.** One reader per format implements it; every
/// caller in the workspace consumes it through [`MountedDisk`], never directly.
///
/// The required surface is deliberately the raw thing a filesystem reader gives
/// — recognise, open, name yourself, list your files, and the format's own
/// answer for which story and which archive. [`Volume::stories`] is provided
/// once here, on top of `contents`, because "a story is a file whose bytes are a
/// story" is a policy no format gets to restate.
///
/// A new format's impl is pure delegation to its reader; see the two below.
pub trait Volume: std::fmt::Debug {
    /// Cheap content sniff: could `raw` be this format? Must never claim bytes
    /// [`Volume::mount`] would then refuse, since [`DiskImage::detect`] is
    /// exactly this question asked of the whole table.
    fn looks_like(raw: &[u8]) -> bool
    where
        Self: Sized;

    /// Open the image and enumerate it. `None` when its filesystem will not
    /// read; the format-neutral [`MountError`] is added by the caller.
    fn mount(raw: Vec<u8>) -> Option<Self>
    where
        Self: Sized;

    /// The volume's own name, where the format keeps one (HFS does; AmigaDOS's
    /// is not exposed by its reader).
    fn volume_name(&self) -> Option<&str>;

    /// How many files the mount found — what an error message means by "is this
    /// the boot disk?".
    fn file_count(&self) -> usize;

    /// Every file that reads, in disk order, as `(name, bytes)`.
    fn contents(&self) -> Vec<(String, Vec<u8>)>;

    /// One file by the name the user typed, or `None` when the volume has no
    /// such file.
    ///
    /// **The matching rule is the format's own and is not restated here.** It is
    /// what the user's `--pictures Pic.data` has to hit, so every reader
    /// delegates straight to its own `read_named` rather than being normalised
    /// into a shared one — AmigaDOS, HFS and FAT12 do not spell names alike, and
    /// a seam that "helpfully" agreed on a rule would break the door SQ-0838
    /// added (it is the only way to reach the Macintosh's two-colour archive).
    fn read_named(&self, name: &str) -> Option<Vec<u8>>;

    /// The story to open, by the format's own tiebreak when a disk offers more
    /// than one.
    fn story(&self) -> Option<DiskStory>;

    /// The native picture archive, if the disk carries a readable one.
    fn pictures(&self) -> Option<DiskArt>;

    /// **Every** story on the volume, in disk order — what a picker lists when a
    /// compilation disk holds four games and an InvisiClues file.
    ///
    /// Provided, not required: identifying a story by its bytes is this crate's
    /// policy and there is one copy of it.
    fn stories(&self) -> Vec<DiskStory> {
        self.contents()
            .into_iter()
            .filter(|(_, bytes)| looks_like_story(bytes))
            .map(DiskStory::from)
            .collect()
    }
}

impl Volume for Adf {
    fn looks_like(raw: &[u8]) -> bool {
        Adf::looks_like_adf(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Adf> {
        Adf::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // AmigaDOS names its root block, but `Adf` does not expose it, and the
        // seam does not invent what a reader does not report.
        None
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.name.clone(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the stored AmigaDOS name.
        Adf::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Adf::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Adf::pictures(self).map(DiskArt::from)
    }
}

impl Volume for Hfs {
    fn looks_like(raw: &[u8]) -> bool {
        Hfs::looks_like_hfs(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Hfs> {
        Hfs::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // An unnamed volume is `None`, not `Some("")` — a caller that splices
        // the name into a sentence must not be handed a hole to splice.
        Some(Hfs::volume_name(self)).filter(|n| !n.is_empty())
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.name.clone(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on the stored catalog name.
        Hfs::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Hfs::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Hfs::pictures(self).map(DiskArt::from)
    }
}

/// One impl for both FAT12 rows. The rows differ in their sniff — which machine
/// pressed the disk — and in nothing else, because the filesystem is the same
/// filesystem.
impl Volume for Fat12 {
    /// The FILESYSTEM sniff, deliberately machine-neutral. The table does not
    /// use this one: [`FORMATS`] holds `fat12::looks_like_dos` and
    /// `fat12::looks_like_atari_st`, which are this question and then the
    /// machine question, so the two rows stay disjoint.
    fn looks_like(raw: &[u8]) -> bool {
        Fat12::looks_like_fat12(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<Fat12> {
        Fat12::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // The DOS release disks label their volumes (`Tresure 1`, `DISK 1`,
        // `ZORK0 1`); no ST compilation does, and that reads as `None` rather
        // than as an empty name spliced into somebody's sentence.
        Fat12::volume_label(self)
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // Named by PATH, not by filename: four of the games on `Infocom
        // Compilation 9` are called `STORY.DAT` and only the directory tells
        // them apart.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on either the 8.3 filename or the full path — this
        // is the one format with directories, and `contents` names its files by
        // path, so the name a caller was SHOWN has to be a name it can ask for.
        Fat12::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        Fat12::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Fat12::pictures(self).map(DiskArt::from)
    }
}

impl Volume for ProDos {
    fn looks_like(raw: &[u8]) -> bool {
        ProDos::looks_like_prodos(raw)
    }

    fn mount(raw: Vec<u8>) -> Option<ProDos> {
        ProDos::mount(raw).ok()
    }

    fn volume_name(&self) -> Option<&str> {
        // Every ProDOS volume is named — the format has no unnamed one — but an
        // empty name is `None` rather than a hole spliced into somebody's
        // sentence, exactly as the HFS impl above.
        Some(ProDos::volume_name(self)).filter(|n| !n.is_empty())
    }

    fn file_count(&self) -> usize {
        self.files().len()
    }

    fn contents(&self) -> Vec<(String, Vec<u8>)> {
        // Named by PATH, like FAT12: ProDOS nests, and the GS/OS disks put most
        // of what they carry under `SYSTEM/`.
        self.files().iter().filter_map(|e| self.read(e).map(|b| (e.path(), b))).collect()
    }

    fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        // Case-insensitive on either the stored name or the full path.
        ProDos::read_named(self, name)
    }

    fn story(&self) -> Option<DiskStory> {
        ProDos::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        ProDos::pictures(self).map(DiskArt::from)
    }
}

// ── A mounted disk ────────────────────────────────────────────────────────────

/// An open release disk, whatever format it turned out to be.
///
/// This is what every front-end holds. It answers in one vocabulary, so no
/// caller has an `if adf … else if hfs` in it and none can acquire one: the
/// concrete reader is behind a [`Volume`] and its name never leaves this file.
#[derive(Debug)]
pub struct MountedDisk {
    image: DiskImage,
    volume: Box<dyn Volume>,
}

impl MountedDisk {
    /// Open `raw` as whichever format claims it.
    ///
    /// [`MountError::NotADiskImage`] is the ordinary "this is a plain story
    /// file" answer and callers fall through on it; [`MountError::Unreadable`]
    /// means a disk we recognised is damaged, which is worth reporting.
    pub fn mount(raw: Vec<u8>) -> Result<MountedDisk, MountError> {
        let format = format_of(&raw).ok_or(MountError::NotADiskImage)?;
        let volume = (format.mount)(raw).ok_or(MountError::Unreadable(format.image))?;
        Ok(MountedDisk { image: format.image, volume })
    }

    /// Which format this turned out to be.
    pub fn format(&self) -> DiskImage {
        self.image
    }

    /// See [`DiskImage::label`].
    pub fn label(&self) -> &'static str {
        self.image.label()
    }

    /// See [`DiskImage::interpreter_number`].
    pub fn interpreter_number(&self) -> Option<u8> {
        self.image.interpreter_number()
    }

    /// The volume's own name, where the format keeps one.
    pub fn volume_name(&self) -> Option<&str> {
        self.volume.volume_name()
    }

    /// How many files the mount found.
    pub fn file_count(&self) -> usize {
        self.volume.file_count()
    }

    /// Every file that reads, in disk order, as `(name, bytes)`.
    ///
    /// The unfiltered listing, for a caller that identifies files by its own
    /// test rather than by this crate's. `app`'s asset discovery needs it
    /// because [`MountedDisk::pictures`] answers with THE archive by the
    /// format's own tiebreak, and a Macintosh disk carries two — a colour
    /// `CPic.data` and a monochrome `Pic.data` — both of which a person must be
    /// able to choose between (SQ-0843).
    pub fn contents(&self) -> Vec<(String, Vec<u8>)> {
        self.volume.contents()
    }

    /// Every story on the disk, in disk order.
    pub fn stories(&self) -> Vec<DiskStory> {
        self.volume.stories()
    }

    /// One file off the volume by the name the user typed — the `--pictures`
    /// door (SQ-0838), and the other half of [`MountedDisk::contents`]: what a
    /// caller was shown, it can ask for.
    ///
    /// See [`Volume::read_named`] for why the matching rule stays each format's
    /// own.
    pub fn read_named(&self, name: &str) -> Option<Vec<u8>> {
        self.volume.read_named(name)
    }

    /// The story to open, by the format's tiebreak.
    pub fn story(&self) -> Option<DiskStory> {
        self.volume.story()
    }

    /// The disk's own artwork, if it carries a readable archive.
    pub fn pictures(&self) -> Option<DiskArt> {
        self.volume.pictures()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AmigaDOS boot block is all `looks_like_adf` needs, and it is what
    /// every `.adf` in the corpus opens with.
    fn adf_bytes() -> Vec<u8> {
        let mut v = vec![0u8; 4 * 512];
        v[0..3].copy_from_slice(b"DOS");
        v[3] = 0; // OFS
        v
    }

    #[test]
    fn an_amiga_floppy_defaults_to_interpreter_four() {
        // ZMSD §11.1.3: 4 = Amiga. The whole of SQ-0839's rule in two lines.
        assert_eq!(DiskImage::detect(&adf_bytes()), Some(DiskImage::Adf));
        assert_eq!(DiskImage::Adf.interpreter_number(), Some(4));
    }

    /// SQ-0838 lifted this block: the Macintosh's number was always known, and
    /// now its machine is too, so the medium hands the number out like any
    /// other. ZMSD §11.1.3, quoted at [`MACINTOSH_INTERPRETER_NUMBER`].
    #[test]
    fn a_macintosh_disk_defaults_to_interpreter_three() {
        assert_eq!(DiskImage::Hfs.interpreter_number(), Some(3), "ZMSD §11.1.3: 3 = Macintosh");
        assert_eq!(DiskImage::Hfs.label(), "HFS");
    }

    /// Content, not extension — and an ordinary story file is not a medium, so
    /// it never moves the number.
    #[test]
    fn an_ordinary_story_file_is_not_a_medium() {
        let mut story = vec![0u8; 0x400];
        story[0] = 3;
        story[0x12..0x18].copy_from_slice(b"840726");
        assert_eq!(DiskImage::detect(&story), None);
        assert_eq!(DiskImage::detect(&[]), None);
        assert_eq!(DiskImage::detect(b"not a disk image at all"), None);
    }

    #[test]
    fn the_label_names_the_filesystem() {
        assert_eq!(DiskImage::Adf.label(), "ADF");
    }

    // ── The seam: one mount path, every format (SQ-0840) ──────────────────────

    /// A structurally valid v6 story, so `looks_like_story` finds it and every
    /// format's sample carries the same payload.
    fn fake_story() -> Vec<u8> {
        let mut b = vec![0u8; 4096];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (4096 / 8) as u16); // file length, v6 unit
        b[0x12..0x18].copy_from_slice(b"890323");
        b
    }

    /// A real, mountable volume of `image`'s format, carrying one story and one
    /// file that is not one.
    ///
    /// **The match is exhaustive on purpose.** A new [`DiskImage`] variant does
    /// not COMPILE until it is given a sample here, and then does not PASS the
    /// property test below until [`FORMATS`] can detect and open what it
    /// returns. That is the whole guard against a format being half-wired again.
    fn sample_of(image: DiskImage) -> Vec<u8> {
        let story = fake_story();
        // `STORY.DAT` rather than the Amiga's `Story.data`, because the sample
        // has to be a name EVERY filesystem here can actually store and FAT12's
        // 8.3 directory entry would shorten the longer one. The name is not what
        // is under test; that a story is found by its bytes is.
        let files: [(&str, &[u8]); 2] = [("Readme", b"just a text file"), ("STORY.DAT", &story)];
        match image {
            DiskImage::Adf => crate::adf::tests::sample_disk(&files),
            DiskImage::Hfs => crate::hfs::tests::sample_disk(&files),
            DiskImage::Fat12Dos => {
                crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::Dos)
            }
            DiskImage::Fat12AtariSt => {
                crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::AtariSt)
            }
            DiskImage::ProDos => crate::prodos::tests::sample_disk(&files),
        }
    }

    /// The census: the enum and [`FORMATS`] must name the same formats, and the
    /// table must be where the label and the interpreter number come from.
    ///
    /// The match is exhaustive, so adding a variant fails to compile here and
    /// walks whoever added it straight to the row it is missing. A variant with
    /// no row is a format nothing can detect, mount or name — the half-wiring
    /// SQ-0840 was filed for, one enum away from happening again.
    #[test]
    fn every_variant_the_enum_declares_has_a_row_in_the_one_table() {
        let census = [
            DiskImage::Adf,
            DiskImage::Hfs,
            DiskImage::Fat12Dos,
            DiskImage::Fat12AtariSt,
            DiskImage::ProDos,
        ];
        for image in census {
            let (label, interpreter) = match image {
                DiskImage::Adf => ("ADF", Some(AMIGA_INTERPRETER_NUMBER)),
                DiskImage::Hfs => ("HFS", Some(MACINTOSH_INTERPRETER_NUMBER)),
                // One FAT12 filesystem, two machines, two different answers —
                // and the difference is the point. The IBM PC's honest number
                // is version-dependent (6 for Version 6, else 1), so no single
                // constant expresses it and its own rule is already in force.
                // The Atari ST's is a flat 5, written as such by Infocom's own
                // ST interpreters; both are argued at their rows in `FORMATS`.
                DiskImage::Fat12Dos => ("DOS", None),
                DiskImage::Fat12AtariSt => ("ST", Some(ATARI_ST_INTERPRETER_NUMBER)),
                // …and a third answer, which is the DOS one for a different
                // reason: ProDOS names the Apple II FAMILY, and ZMSD §11.1.3
                // gives that family three numbers (2 IIe, 9 IIc, 10 IIgs). The
                // corpus holds both an 8-bit and a IIgs press, so nothing on a
                // volume chooses. Argued at the row in `FORMATS`.
                DiskImage::ProDos => ("ProDOS", None),
            };
            assert!(DiskImage::all().any(|d| d == image), "{image:?} has no row in FORMATS");
            assert_eq!(image.label(), label, "{image:?}");
            assert_eq!(image.interpreter_number(), interpreter, "{image:?}");
        }
        assert_eq!(
            DiskImage::all().count(),
            census.len(),
            "FORMATS and DiskImage have drifted apart — every format is one row and one variant"
        );
    }

    /// The extensions census: every format offers at least one spelling, and
    /// [`image_extensions`] is exactly the union of the rows'.
    ///
    /// The "at least one" is the guard that matters. A row with no extension is
    /// a format a directory scan can never *offer* — it mounts perfectly when
    /// you name the file, and is invisible in the story list — which is the
    /// precise shape of the defect SQ-0849 was filed for, arrived at from the
    /// other side. Spelling rules (lowercase, no dot, no duplicates) are pinned
    /// too, because `has_story_ext` lowercases the candidate's extension and
    /// compares it as-is, so a row shouting `"ADF"` would silently match
    /// nothing.
    #[test]
    fn every_format_offers_at_least_one_extension_and_the_census_is_their_union() {
        let mut union: Vec<&str> = Vec::new();
        for image in DiskImage::all() {
            let exts = image.extensions();
            assert!(
                !exts.is_empty(),
                "{image:?} names no extension — a directory scan can never offer it"
            );
            for e in exts {
                assert!(
                    !e.is_empty() && !e.starts_with('.') && *e == e.to_ascii_lowercase(),
                    "{image:?}: {e:?} must be lowercase and dotless"
                );
                assert!(!union.contains(e), "{image:?}: {e:?} is claimed by two rows");
                union.push(e);
            }
        }
        assert_eq!(
            image_extensions().collect::<Vec<_>>(),
            union,
            "the census is the rows' extensions and nothing else"
        );
        // The spellings the corpus in `stories/` actually uses, named outright
        // so a row losing one fails here rather than in the picker.
        for want in ["adf", "image", "ima", "img", "st", "2mg"] {
            assert!(image_extensions().any(|e| e == want), "no row claims {want:?}");
        }
        // …and the queued format does NOT get a name before it gets a reader
        // (SQ-0852, Apple II DOS 3.3). Nor do the bare-ProDOS spellings, which
        // this crate CAN open and no medium in the corpus wears.
        for queued in ["dsk", "po", "hdv"] {
            assert!(
                !image_extensions().any(|e| e == queued),
                "{queued:?} is claimed by a row, and no medium in the corpus justifies it"
            );
        }
    }

    /// A name is a pre-filter, never evidence — stated as a test so the
    /// extensions column cannot quietly become a recogniser.
    ///
    /// Both halves of this module's header rule, on the new column: bytes that
    /// ARE a disk are one whatever they are called, and bytes that are not stay
    /// not one however suggestively they are named.
    #[test]
    fn the_extension_census_decides_nothing_about_what_bytes_are() {
        for image in DiskImage::all() {
            // A real image is recognised, and no extension was consulted to do
            // it — `detect` never sees a filename at all.
            assert_eq!(DiskImage::detect(&sample_of(image)), Some(image), "{image:?}");
        }
        // An ordinary story file is not a disk image, and would not become one
        // if it were called `zork.ima`.
        assert_eq!(DiskImage::detect(&fake_story()), None);
        // Nor is a file of nonsense that happens to carry a claimed extension.
        assert_eq!(DiskImage::detect(&vec![0u8; 720 * 1024]), None);
    }

    /// **The property this quest exists for**: whatever `detect` claims, `mount`
    /// opens, and the mounted disk answers every question a front-end asks —
    /// for every format alike, with no caller naming one.
    ///
    /// This is what `zvm-cli`'s old `looks_like_image` had to guard by hand: it
    /// narrowed itself to `Adf` because an HFS disk would detect and then fail
    /// to open. Under one table that cannot happen, and this test says so out
    /// loud rather than leaving it to a comment.
    #[test]
    fn whatever_detect_claims_mounts_and_answers_every_question() {
        for image in DiskImage::all() {
            let raw = sample_of(image);
            assert_eq!(DiskImage::detect(&raw), Some(image), "{image:?} is not detected");

            let disk = MountedDisk::mount(raw)
                .unwrap_or_else(|e| panic!("{image:?} detects but will not mount: {e}"));
            assert_eq!(disk.format(), image);
            assert_eq!(disk.label(), image.label());
            assert_eq!(disk.interpreter_number(), image.interpreter_number());
            assert_eq!(disk.file_count(), 2, "{image:?} lists what it mounted");

            // Identified by CONTENT: `Readme` is not a story and `STORY.DAT`
            // is, and no format is allowed to decide that by name.
            let stories = disk.stories();
            assert_eq!(stories.len(), 1, "{image:?} found {stories:?}");
            assert_eq!(stories[0].name, "STORY.DAT", "{image:?}");
            assert_eq!(stories[0].bytes, fake_story(), "{image:?} reads it byte-exact");
            assert_eq!(disk.story().map(|s| s.bytes), Some(fake_story()), "{image:?}");

            // No archive on a synthetic disk — the point is that the question is
            // answerable at all, on every format, without a panic or a chain.
            assert!(disk.pictures().is_none(), "{image:?}");
            let _ = disk.volume_name();
        }
    }

    /// The unfiltered listing every format hands back, and the reason it is on
    /// [`MountedDisk`] at all: `pictures()` answers with ONE archive by the
    /// format's own tiebreak, so a caller offering a person a choice between a
    /// disk's two archives has to see all its files (SQ-0843).
    #[test]
    fn a_mounted_disk_lists_every_file_it_holds() {
        for image in DiskImage::all() {
            let disk = MountedDisk::mount(sample_of(image)).expect("mounts");
            let contents = disk.contents();
            let names: Vec<&str> = contents.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names.len(), disk.file_count(), "{image:?} lists what it counted");
            assert!(names.contains(&"Readme"), "{image:?}: {names:?}");
            assert!(names.contains(&"STORY.DAT"), "{image:?}: {names:?}");
            // Bytes, not just names — this is what a caller identifies by.
            let readme = contents.iter().find(|(n, _)| n == "Readme").expect("present");
            assert_eq!(readme.1, b"just a text file", "{image:?}");
            // …and the story is in the listing too, unfiltered: deciding what a
            // file IS belongs to the caller, not to the mount.
            assert!(contents.iter().any(|(_, b)| *b == fake_story()), "{image:?}");
        }
    }

    /// **What a caller is shown, it can ask for** — on every format.
    ///
    /// `contents` is how the launch dialog enumerates a disk's artwork and
    /// `read_named` is how the `--pictures` door then loads it, so the two
    /// disagreeing is a file that is offered and cannot be opened. That is
    /// exactly what happened while `graphics::read_off_the_medium` still carried
    /// its own two-reader chain (SQ-0833): a FAT12 disk enumerated through the
    /// table and loaded through a chain that had never heard of it.
    ///
    /// The matching rule stays each format's own; what is pinned here is that
    /// the name in hand is one the same volume answers to.
    #[test]
    fn every_name_a_disk_lists_is_a_name_it_will_read_back() {
        for image in DiskImage::all() {
            let disk = MountedDisk::mount(sample_of(image)).expect("mounts");
            for (name, bytes) in disk.contents() {
                assert_eq!(
                    disk.read_named(&name).as_ref(),
                    Some(&bytes),
                    "{image:?}: listed {name:?} and would not read it back"
                );
                // …and case is not what decides it, on any format.
                assert_eq!(
                    disk.read_named(&name.to_ascii_lowercase()).as_ref(),
                    Some(&bytes),
                    "{image:?}: {name:?} is case-sensitive"
                );
            }
            assert_eq!(disk.read_named("NoSuchFile.data"), None, "{image:?}");
        }
    }

    /// **The table order is a formality rather than a precedence** — stated as a
    /// test over the whole corpus rather than left to [`DiskImage::detect`]'s
    /// doc comment.
    ///
    /// `detect` returns the FIRST row whose sniff fires, so two rows claiming
    /// one file would make the table's order load-bearing and a format's
    /// identity depend on where it was pasted in. This walks every file in
    /// `stories/` — Amiga, Macintosh, DOS, Atari ST and ProDOS floppies, bare
    /// story files, Blorbs, saved games and loose artwork — and insists that at
    /// most one row ever claims one, and that whatever is claimed opens.
    ///
    /// SQ-0836 is why it exists: a fifth filesystem is the point at which "they
    /// happen not to collide" stops being obvious.
    #[test]
    fn at_most_one_format_ever_claims_a_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut seen = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(raw) = std::fs::read(&path) else { continue };
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let claims: Vec<DiskImage> =
                FORMATS.iter().filter(|f| (f.looks_like)(&raw)).map(|f| f.image).collect();
            assert!(claims.len() <= 1, "{name}: claimed by {claims:?}");
            let Some(image) = claims.first().copied() else { continue };
            seen += 1;
            assert_eq!(DiskImage::detect(&raw), Some(image), "{name}");
            let disk = MountedDisk::mount(raw).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(disk.file_count() > 0, "{name}: mounted but empty");
        }
        if seen == 0 {
            eprintln!("SKIP: no release media present");
        }
    }

    /// The same property on the real disk that motivated it: an ST compilation
    /// names its files by folder, and the folder has to survive the round trip
    /// or a picker offers `HITCHHIK/STORY.DAT` and opens nothing.
    #[test]
    fn a_real_directoried_disk_reads_back_the_paths_it_lists() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Infocom Compilation 9 (19xx)(-).st");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ST disk mounts");
        assert_eq!(
            disk.read_named("HITCHHIK/STORY.DAT").map(|b| b.len()),
            Some(113444),
            "the path names the game"
        );
        assert_eq!(
            disk.read_named("cuthroat/story.dat").map(|b| b.len()),
            Some(112558),
            "…case-insensitively, like every other format here"
        );
        // The bare filename still resolves — to the first of the four, which is
        // why a picker shows the path and not this.
        assert!(disk.read_named("STORY.DAT").is_some());
    }

    /// A boot disk carries files and no game, on any format. The mount succeeds
    /// — that is what lets a caller say "is this the boot disk?" instead of
    /// "corrupt story file".
    #[test]
    fn a_disk_with_no_game_mounts_and_offers_nothing() {
        for image in DiskImage::all() {
            let files: [(&str, &[u8]); 2] =
                [("Startup-Sequence", b"LoadWB\n"), ("Desktop", b"\x00\x01\x02\x03")];
            let raw = match image {
                DiskImage::Adf => crate::adf::tests::sample_disk(&files),
                DiskImage::Hfs => crate::hfs::tests::sample_disk(&files),
                DiskImage::Fat12Dos => {
                    crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::Dos)
                }
                DiskImage::Fat12AtariSt => {
                    crate::fat12::tests::sample_disk(&files, crate::fat12::Machine::AtariSt)
                }
                DiskImage::ProDos => crate::prodos::tests::sample_disk(&files),
            };
            let disk = MountedDisk::mount(raw).expect("a boot disk still mounts");
            assert_eq!(disk.file_count(), 2, "{image:?}");
            assert!(disk.stories().is_empty(), "{image:?}");
            assert_eq!(disk.story(), None, "{image:?}");
        }
    }

    /// An ordinary story file is not a disk, and says so in the one way every
    /// caller falls through on.
    #[test]
    fn mounting_a_plain_story_file_is_not_a_disk_image() {
        let mut story = vec![0u8; 0x400];
        story[0] = 3;
        assert_eq!(MountedDisk::mount(story).map(|d| d.format()), Err(MountError::NotADiskImage));
    }

    /// The error a front-end prints must be about a disk, never about a
    /// filesystem's internals — `app` and `zvm-cli` do not know what an extents
    /// overflow file is and must never have to.
    #[test]
    fn a_mount_error_reads_as_a_disk_problem_not_a_filesystem_one() {
        assert_eq!(MountError::NotADiskImage.to_string(), "not a disk image in any format we read");
        assert_eq!(
            MountError::Unreadable(DiskImage::Hfs).to_string(),
            "the HFS filesystem on it would not read"
        );
    }

    /// Real media, every format: the disks the corpus actually holds mount
    /// through the shared path and hand back their own story — and their own
    /// art wherever the medium carries any. They live outside the repo, so each
    /// arm skips vacuously.
    ///
    /// The match is exhaustive, so a new format has to name the disk that
    /// proves it rather than quietly riding on somebody else's fixture. What
    /// each arm expects is the medium's own truth and not a shared shape: three
    /// of the four are *Zork Zero* in one press or another, and the fourth is
    /// an Atari ST compilation, because **no ST v6 release exists in this
    /// corpus** — all thirty-eight ST stories are v3, v4 or v5, and none of the
    /// nine disks carries artwork at all.
    #[test]
    fn real_release_disks_of_every_format_mount_through_one_path() {
        for image in DiskImage::all() {
            // (fixture, the story's stored name, its version, does the disk
            //  also carry a picture archive?)
            let (fixture, story_name, version, has_art) = match image {
                DiskImage::Adf => ("Zork Zero - The Revenge of Megaboz.adf", "Story.data", 6, true),
                DiskImage::Hfs => ("Zork Zero Disk.image", "Story.data", 6, true),
                // Lost Treasures I floppy5: Zork Zero's story AND its EGA art.
                // (Its CGA art is on floppy4, which is the whole of why a set
                // model is a real thing this lane does not have.)
                DiskImage::Fat12Dos => ("floppy5.ima", "ZORK0.ZIP", 6, true),
                // Four games in four folders, ALL called `STORY.DAT` — so the
                // conventional-name tiebreak cannot separate them and the
                // largest wins, which is *Bureaucracy* (v4, 243200 bytes).
                // Deterministic, and a compilation wants `stories()` anyway.
                DiskImage::Fat12AtariSt => {
                    ("Infocom Compilation 9 (19xx)(-).st", "BUREAUCR.ACY/STORY.DAT", 4, false)
                }
                // The Apple IIgs, and its own truth again: *Lost Treasures*
                // volume 2 holds five games with no conventional name between
                // them, so the largest opens — *Beyond Zork*, v5, 261 388 bytes
                // — and no ProDOS release in the corpus carries an Infocom
                // picture archive at all. (The two that DO ship artwork,
                // *Arthur* and *Journey*, keep it inside the same segmented
                // container as their story and offer no whole story file; see
                // `crate::prodos`.)
                DiskImage::ProDos => (
                    "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg",
                    "BEYOND.ZORK",
                    5,
                    false,
                ),
            };
            let path =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(fixture);
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("SKIP: {image:?} media absent at {}", path.display());
                continue;
            };
            assert_eq!(DiskImage::detect(&bytes), Some(image), "{fixture}");
            let disk = MountedDisk::mount(bytes).expect("the release disk mounts");
            assert_eq!(disk.format(), image, "{fixture}");
            let story = disk.story().expect("the release disk carries its game");
            assert_eq!(story.name, story_name, "{fixture}");
            assert_eq!(story.bytes[0], version, "{fixture}");
            match disk.pictures() {
                Some(art) => {
                    assert!(has_art, "{fixture}: unexpected artwork {}", art.name);
                    assert!(art.pictures.entries().len() > 100, "{fixture}: {}", art.name);
                }
                None => assert!(!has_art, "{fixture}: its own artwork is missing"),
            }
        }
    }

    /// **A compilation disk is a list, not a game** — and the list is what a
    /// picker shows. `Infocom Compilation 9` is the sharpest case in the
    /// corpus: four games, four folders, four files called `STORY.DAT`, and a
    /// saved game sitting beside three of them.
    #[test]
    fn a_real_compilation_disk_lists_every_game_and_no_saved_game() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Infocom Compilation 9 (19xx)(-).st");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ST disk mounts");
        assert_eq!(disk.format(), DiskImage::Fat12AtariSt);
        assert_eq!(disk.label(), "ST");
        assert_eq!(
            disk.interpreter_number(),
            Some(ATARI_ST_INTERPRETER_NUMBER),
            "an ST floppy announces the Atari ST — ZMSD §11.1.3, and the machine's own \
             `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`",
        );
        let names: Vec<String> = disk.stories().into_iter().map(|s| s.name).collect();
        assert_eq!(names, [
            "HITCHHIK/STORY.DAT",
            "BUREAUCR.ACY/STORY.DAT",
            "CUTHROAT/STORY.DAT",
            "LEATHER.GOD/STORY.DAT",
        ]);
        assert_eq!(disk.file_count(), 14, "…out of fourteen files, saves and interpreters included");
    }

    /// The same property on the Apple IIgs press (SQ-0836). *Lost Treasures*
    /// volume 2 is the ProDOS analogue of the ST compilation above: five games
    /// on one disk, four of somebody's saved games beside them, and a format
    /// with no conventional story name at all — so the LIST is the answer and
    /// `story()`'s largest-wins tiebreak is only the default.
    ///
    /// It also pins the medium's own number, which is the interesting one: a
    /// ProDOS volume answers `None`, because ZMSD §11.1.3 gives the Apple II
    /// family three numbers and nothing on a volume says which machine pressed
    /// it. See the row in [`FORMATS`].
    #[test]
    fn a_real_apple_iigs_compilation_lists_every_game_and_no_saved_game() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../stories/Lost Treasures of Infocom, The (1993)\
             (Big Red Computer Club)(Disk 2 of 7).2mg",
        );
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return;
        };
        let disk = MountedDisk::mount(bytes).expect("the ProDOS disk mounts");
        assert_eq!(disk.format(), DiskImage::ProDos);
        assert_eq!(disk.label(), "ProDOS");
        assert_eq!(disk.volume_name(), Some("INFOCOM2"));
        assert_eq!(
            disk.interpreter_number(),
            None,
            "ProDOS names the Apple II family, and ZMSD §11.1.3 numbers three machines in it",
        );
        let names: Vec<String> = disk.stories().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["ZORK.III", "ZORK.II", "ZORK.I", "HITCHHIKER", "BEYOND.ZORK"]);
        assert_eq!(disk.story().map(|s| s.name), Some("BEYOND.ZORK".to_string()), "the largest");
        assert_eq!(disk.file_count(), 10, "…out of ten files, four saved games included");
        // What a caller was shown it can ask for, on this format too.
        for (name, bytes) in disk.contents() {
            assert_eq!(disk.read_named(&name).as_ref(), Some(&bytes), "listed {name:?}");
        }
    }
}
