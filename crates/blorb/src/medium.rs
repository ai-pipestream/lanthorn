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
//! ## A story that is on no single disk (SQ-0864)
//!
//! One release in the corpus does not fit "a volume holds a story": the Apple II
//! 5.25-inch presses of *Shogun* and *Zork Zero* page one story across **five**
//! and **four** floppies, and no one of them carries a whole game. That is not a
//! filesystem question — every volume mounts perfectly well and lists its
//! segment — so it is not a format's business and there is no row for it.
//!
//! It is [`MountedDisk::mount_set`]'s: one image is opened exactly as ever, and
//! the other volumes of its release are offered alongside so that the container
//! spanning them ([`crate::infocom_packed`]) can be asked. Every format gets
//! this and none of them pays for it — the companions are consulted only when
//! the named volume has no story of its own, which is true of a Shogun floppy
//! and false of every compilation disk here. [`MountedDisk::mount`] is that
//! call with no companions, so nothing that does not want a set sees one.
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

/// Apple IIgs, from the same §11.1.3 table — *"… 9 Apple IIc, **10 Apple
/// IIgs**, 11 Tandy Color"* — and, like the Atari ST's, corroborated by
/// Infocom's own interpreter rather than by the standard alone (SQ-0857).
///
/// The corroboration here is unusually direct, because the interpreter is **on
/// two of the disks in `stories/`**. `apple/yzip/rel.15/apple.equ` in
/// `github.com/erkyrath/infocom-zcode-terps` — the Apple II YZIP, Infocom's
/// Version 6 interpreter for the machine — names all three of the family's
/// numbers as constants:
///
/// ```text
///   apple/yzip/rel.15/apple.equ:136   IIeID   EQU  2   ; Apple ][e Yzip
///   apple/yzip/rel.15/apple.equ:137   IIcID   EQU  9   ; ][c Yzip
///   apple/yzip/rel.15/apple.equ:138   IIgsID  EQU  10  ; ][gs Yzip
/// ```
///
/// and `zboot.asm` writes whichever one it settled on into header `$1E`
/// (`ZINTWD EQU 30`, decimal 30 = `$1E`, in `zip.equ`):
///
/// ```text
///   apple/yzip/rel.15/zboot.asm:7   lda  ARG2+LO         ; get machine id!
///   apple/yzip/rel.15/zboot.asm:8   sta  ZBEGIN+ZINTWD   ; save before it gets zeroed
/// ```
///
/// **And it is NOT a flat constant — it is detected at boot**, which is the
/// question worth asking here because a version-dependent rule is exactly why
/// [`DiskImage::Fat12Dos`] answers `None`. `bsubs.asm`'s `MACHINE:` reads
/// ProDOS's own machine-ID bytes and picks one of the three:
///
/// ```text
///   ; Make sure we are on a good machine, like a ][c or ][e+/][gs
///   MACHINE:
///       lda MACHID1 / cmp #6 / bne BADMACH   ; nothing below an enhanced ][e
///       lda MACHID2 / bne MACH1
///       lda #IIcID                            ; Apple ][c thank you
///   MACH1:
///       sec / jsr MACHCHK / bcs OLDMACH       ; check for 'new' machine
///       lda #IIgsID                           ; this is a ][gs
///   OLDMACH:
///       lda #IIeID                            ; this is IIe
///   MACH2:
///       sta ARG2+LO                           ; save machine id
/// ```
///
/// `apple/yzip/rel.13/boot.lst` assembles that routine to
/// `AD B3 FB C9 06 D0 19 AD C0 FB D0 05 A9 09 4C CB 26 38 20 1F FE B0 04 A9 0A
/// D0 02 A9 02 85 65 60`, and the same bytes — with the one `jmp MACH2` operand
/// relocated by one, `4C CC 26` — occur **verbatim and byte-identical** in
/// `INFOCOM.SYSTEM` on both `Journey.2mg` and `Arthur Quest 4 Excalibur.2mg`, at
/// offset 1711 of each. Those are the ProDOS 8 launchers of the corpus's two
/// Version 6 releases. (The 22,528-byte `INFOCOM` interpreter beside them
/// differs between the two disks in exactly two bytes, so one interpreter serves
/// both presses.)
///
/// So the number is a property of **the machine the interpreter is running on**
/// and not of the disk, and §11.1.3 says as much: *"An interpreter should choose
/// the interpreter number most suitable for the machine it will run on."* The
/// row below is where babelmap answers that question; see it for why the answer
/// is the top of the family.
pub const APPLE_IIGS_INTERPRETER_NUMBER: u8 = 10;

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
    /// 10 Apple IIgs. Infocom's own Apple interpreter settles which by *detecting
    /// the machine at boot* rather than by pressing three disks, so the ambiguity
    /// is a fact about the medium and not a gap in the evidence. See this
    /// variant's row in [`FORMATS`] for why
    /// [`DiskImage::interpreter_number`] nevertheless answers, and with what.
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
/// **The Apple II 5.25-inch press arrived without one** (SQ-0864), and that is
/// the table working rather than the table being bypassed. SQ-0852 queued
/// `shogun_s1..s5.dsk` and `zork_zero_1..4.dsk` here as a sixth format, on the
/// reading that a bare 143,360-byte DOS-order sector dump carries the packed
/// volume directly, through a per-disk block map, with no filesystem under it.
///
/// Re-derived, that reading was one layer too low. The block map is a **ProDOS
/// index block**, because the image is a **ProDOS volume** — the whole of the
/// difference from a `.2mg` is that its sectors are in the order the 5.25-inch
/// drive numbers them. De-interleave it ([`crate::dos_order`]) and block 2 is an
/// ordinary volume directory naming `SHOGUN.1`…`SHOGUN.5` and `ZORK0.1`…
/// `ZORK0.4`, each holding its segment as an ordinary ProDOS file
/// (`SHOGUN.D3` is a tree file; the hand-rolled map could not have read it).
/// So the medium is ProDOS, it wears the ProDOS row, it answers the Apple IIgs
/// like every other ProDOS volume, and `.dsk` is a spelling that row claims.
///
/// What was genuinely missing was not a row but a **set**: the segments are on
/// five different volumes and [`Volume::mount`] takes one image. That is
/// [`MountedDisk::mount_set`], above the table and format-neutral, so the two
/// sets ship without this list growing an entry.
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
        // `.dsk`; the first is admitted by the DOS row below and the second by
        // the ProDOS row at the bottom, and the union is what a scan
        // pre-filters on — so an HFS volume under either name is opened, and
        // `looks_like_hfs` is what then claims it. Which row spells a name is
        // never which format the bytes are.
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
        // **10, the Apple IIgs** (SQ-0857). This row answered `None` from
        // SQ-0836 until SQ-0857, and the argument then was the one immediately
        // above at [`DiskImage::Fat12Dos`]: ProDOS names the Apple II FAMILY, not
        // a machine, and ZMSD §11.1.3 numbers three of them (2 IIe, 9 IIc,
        // 10 IIgs). That premise is not merely still true, it is now proven from
        // Infocom's own code — see [`APPLE_IIGS_INTERPRETER_NUMBER`], where the
        // Apple II YZIP's `MACHINE:` routine picks between all three at boot, and
        // where those bytes are located on the two Version 6 disks in `stories/`.
        //
        // **What changed is the realisation that `None` is not neutral here.**
        // The `Fat12Dos` row can decline because zvm's own rule — Frotz's, 6 for
        // Version 6 and 1 otherwise — *is* the IBM PC's rule, so declining leaves
        // a DOS floppy describing itself correctly. On a ProDOS volume the same
        // deferral lands on 1 (DECSystem-20) or, for Version 6, on 6 — the IBM
        // PC, a machine on another continent, and the one value `zvm`'s `exec.rs`
        // gates its CP437 remap on. Declining does not leave the Apple II
        // unnamed; it names it something else.
        //
        // §11.1.3 asks the question this row actually has to answer: *"An
        // interpreter should choose the interpreter number most suitable for the
        // machine it will run on."* The number is a property of the machine in
        // front of the player — which is precisely why Infocom detected it rather
        // than pressing it — so the question is which Apple II babelmap is. Of
        // the three the YZIP will run on at all (`cmp #6 / bne BADMACH` refuses
        // anything below an enhanced IIe), the IIgs is the top, and it is the one
        // a modern terminal with colour and a large screen actually resembles.
        // The other two remain reachable by naming them: `--interpreter 2`
        // or `9` outranks this row, as every front-end pins.
        //
        // **Measured, not assumed** — the same way SQ-0835 settled the ST's 5.
        // All thirty-one stories on the ten `.2mg` images were traced under the
        // default rule and under 10. Twenty-four are byte-identical (every
        // Version 3 story, including the high-ASCII-serial *Leather Goddesses*
        // SQ-0856 made visible, plus *A Mind Forever Voyaging* and *Bureaucracy*).
        // Five print the number in their VERSION block and are otherwise
        // unchanged (Hitchhiker's, Trinity, Sherlock, Border Zone, Nord and
        // Bert). **One behaves differently, twice, and it is the right
        // difference**: *Beyond Zork* r57 s871221 — on both the GS/OS `BZ.DAT`
        // press and the Lost Treasures `BEYOND.ZORK` one — stops asking "Is this
        // a VT220?" and goes straight to BEGIN/RESTORE/QUIT, because an Apple
        // IIgs is not a terminal that might or might not have line-drawing
        // characters. That is the identical finding SQ-0835 recorded for the ST,
        // on the identical game. It also answers VERSION with **"Apple //gs
        // Color Version A"** where it used to say "DEC-20" — the story naming
        // the machine in Infocom's own spelling, which no part of this codebase
        // supplied.
        //
        // The rest of the bundle is `app::interpreter::InterpreterProfile::AppleIIgs`,
        // and it declines the one member nothing here establishes: the Apple's
        // Version 6 screen is 140x192 on a 3x9 cell, which is not a standard
        // window in this codebase's sense at all. See that knob's docs.
        interpreter_number: Some(APPLE_IIGS_INTERPRETER_NUMBER),
        // Two spellings, one filesystem. `.2mg` is the wrapper every 3.5-inch
        // image in the corpus wears; `.dsk` is what a 5.25-inch dump is called,
        // and SQ-0864 established that those are ProDOS volumes too — the same
        // reader, one de-interleave earlier (see [`crate::dos_order`]). Nine of
        // them are in `stories/`, so the spelling is earned rather than assumed.
        //
        // `.dsk` is also a Macintosh spelling, which costs nothing: the census
        // is a union a directory scan pre-filters on, and `looks_like_hfs`
        // claims an HFS `.dsk` before this row is ever asked.
        //
        // `.po` is the third, and it is the corpus that earned it (SQ-0863).
        // SQ-0864 declined the spelling on the ground that nothing in `stories/`
        // was a BARE ProDOS volume — this reader has always opened one, but a
        // census entry no medium justifies is a guess. `Journey.po` is now here:
        // the 3.5-inch consolidated pressing of *Journey*, volume `JOURNEY.3.5`,
        // carrying all five `JOURNEY.1/`…`JOURNEY.5/` segments where the flat
        // `Journey.2mg` beside it carries four. It mounted the day it arrived,
        // because the reader falls back to a bare volume at offset 0 — and it
        // was invisible in the story list, which is the only place most people
        // look (SQ-0849's defect, exactly).
        //
        // **Not `.hdv`**, still: nothing in the corpus is one, and the argument
        // above is the whole argument. A bare volume under any name still mounts
        // when it is opened directly — the extension is a directory scan's
        // pre-filter, never the recogniser.
        extensions: &["2mg", "dsk", "po"],
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
    ///
    /// **A story need not be a file.** *Arthur* and *Journey* page theirs out of
    /// a packed Apple volume spread over the disk's `.D1`…`.D5` segments, none
    /// of which is a story on its own, so no per-file test can find it. That
    /// container is asked for here rather than in one format's impl because it
    /// is not a property of any filesystem — the same index addresses the raw
    /// 5.25-inch pressings, which have no filesystem at all (SQ-0852).
    fn stories(&self) -> Vec<DiskStory> {
        let contents = self.contents();
        let mut found: Vec<DiskStory> = contents
            .iter()
            .filter(|(_, bytes)| looks_like_story(bytes))
            .cloned()
            .map(DiskStory::from)
            .collect();
        found.extend(crate::infocom_packed::story(&contents).map(DiskStory::from));
        found
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
    /// The files on the OTHER volumes of this image's multi-disk release, when
    /// there are any and this one had no story of its own. Empty for every
    /// ordinary mount, which is nearly all of them; see
    /// [`MountedDisk::mount_set`].
    across: Vec<(String, Vec<u8>)>,
}

impl MountedDisk {
    /// Open `raw` as whichever format claims it.
    ///
    /// [`MountError::NotADiskImage`] is the ordinary "this is a plain story
    /// file" answer and callers fall through on it; [`MountError::Unreadable`]
    /// means a disk we recognised is damaged, which is worth reporting.
    pub fn mount(raw: Vec<u8>) -> Result<MountedDisk, MountError> {
        MountedDisk::mount_set(raw, Vec::new)
    }

    /// Open `raw`, with the other volumes of its multi-disk release available to
    /// it — the mount a story that is on **no single disk** needs (SQ-0864).
    ///
    /// `companions` yields those other images, in disk order and without `raw`
    /// itself. It is a closure and not a list because reading seven 800 KB
    /// floppies to open one of them would be an absurd price for a library scan
    /// to pay per row: **it is called only when the named volume turns out to
    /// have no story of its own**, which is exactly the case that cannot be
    /// answered without them. Every compilation disk in the corpus answers for
    /// itself and never asks.
    ///
    /// What the companions can add is one thing and one thing only: the story a
    /// release pages ACROSS its volumes, which is [`crate::infocom_packed`]'s
    /// container. They do not become part of this volume — [`MountedDisk::contents`],
    /// [`MountedDisk::read_named`], [`MountedDisk::file_count`] and
    /// [`MountedDisk::volume_name`] all still describe the disk that was named,
    /// because that is the disk the caller has. Ordinary files on a sibling
    /// floppy are that floppy's business and it can be mounted.
    ///
    /// Format-neutral by construction: the companions are opened through the
    /// **same row** that claimed `raw`, and one that the row declines is
    /// dropped. No format implements anything for this and none can opt out.
    pub fn mount_set(
        raw: Vec<u8>,
        companions: impl FnOnce() -> Vec<Vec<u8>>,
    ) -> Result<MountedDisk, MountError> {
        let format = format_of(&raw).ok_or(MountError::NotADiskImage)?;
        let volume = (format.mount)(raw).ok_or(MountError::Unreadable(format.image))?;
        let across = if volume.stories().is_empty() {
            companions()
                .into_iter()
                .filter(|raw| (format.looks_like)(raw))
                .filter_map(|raw| (format.mount)(raw))
                .flat_map(|v| v.contents())
                .collect()
        } else {
            Vec::new()
        };
        Ok(MountedDisk { image: format.image, volume, across })
    }

    /// The story this release keeps across its volumes, reassembled out of all
    /// of them — or `None` when no companions were offered, or they hold no such
    /// container, or the one they hold is incomplete.
    ///
    /// The named volume's files come FIRST, so the container's own
    /// "which segment carries the index" search (and therefore the name it
    /// reports) does not depend on which floppy of the set a person happened to
    /// open. Reassembly is verified against the story's own header checksum by
    /// [`crate::infocom_packed`], so a set that does not belong together is
    /// refused rather than handed over as plausible-looking Z-code.
    fn story_across_the_set(&self) -> Option<DiskStory> {
        if self.across.is_empty() {
            return None;
        }
        let mut all = self.volume.contents();
        all.extend(self.across.iter().cloned());
        crate::infocom_packed::story(&all).map(DiskStory::from)
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

    /// Every story on the disk, in disk order — plus the one its release pages
    /// across its volumes, when [`MountedDisk::mount_set`] was given them.
    ///
    /// Naming any floppy of a set therefore lists the same one game, which is
    /// what makes a set behave like a shelf rather than like five refusals. The
    /// duplicate rows that produces across a set are the browser's to fold, and
    /// it already does (`app::picker::dedupe_within_sets`, SQ-0844).
    pub fn stories(&self) -> Vec<DiskStory> {
        let mut found = self.volume.stories();
        if let Some(story) = self.story_across_the_set() {
            if !found.iter().any(|f| f.name == story.name) {
                found.push(story);
            }
        }
        found
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

    /// The story to open, by the format's tiebreak — falling back to the one the
    /// release pages across its volumes.
    ///
    /// The order matters and is the conservative one: a volume that carries a
    /// game answers with its own, always. Only a volume with nothing on it
    /// reaches for the set, which is precisely the Shogun floppy's case.
    pub fn story(&self) -> Option<DiskStory> {
        self.volume.story().or_else(|| self.story_across_the_set())
    }

    /// The artwork this release keeps across its volumes, merged out of all of
    /// them — or `None` when no companions were offered, or they hold no such
    /// container, or the artwork it holds is not all here.
    ///
    /// [`Self::story_across_the_set`]'s sibling, in the same shape and for the
    /// same reason (SQ-0863/SQ-0867): the named volume's files come FIRST so the
    /// container's "which segment carries the index" search does not depend on
    /// which floppy a person opened, and [`crate::infocom_packed::pictures`]
    /// refuses a partial set rather than handing back a picture space with whole
    /// rooms missing.
    ///
    /// This is why *Shogun*'s and *Journey*'s five-volume 5.25-inch presses draw
    /// and `Journey.2mg` does not, and the three of them are one rule seen from
    /// both sides: Shogun's `SGTPICOF` fields name an archive on all five
    /// segments and Journey's on four, and on the `.dsk` sets every segment is
    /// on the shelf. `Journey.2mg` is a genuinely short pressing — it declares
    /// five segments and holds four, and the missing `JOURNEY.D5` carries a
    /// quarter of the artwork — so the merge refuses it whole rather than
    /// serving a picture space with rooms missing, exactly as [`Self::story`]
    /// refuses the story the same segment would have completed.
    fn pictures_across_the_set(&self) -> Option<DiskArt> {
        if self.across.is_empty() {
            return None;
        }
        let mut all = self.volume.contents();
        all.extend(self.across.iter().cloned());
        crate::infocom_packed::pictures(&all).map(DiskArt::from)
    }

    /// The disk's own artwork, if it carries a readable archive — falling back
    /// to the artwork the release pages across its volumes.
    ///
    /// The order is [`Self::story`]'s and is conservative for the same reason: a
    /// volume that carries an archive of its own answers with it, always, and
    /// only a volume with none reaches for the set.
    pub fn pictures(&self) -> Option<DiskArt> {
        self.volume.pictures().or_else(|| self.pictures_across_the_set())
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

    /// SQ-0857 lifted this block too, and for the opposite reason to the
    /// Macintosh's: the Apple II's number was never a constant — Infocom's own
    /// YZIP picks between 2, 9 and 10 at boot — but declining left a ProDOS
    /// story being told it was a DECSystem-20 or an IBM PC. Argued in full at
    /// the row and at [`APPLE_IIGS_INTERPRETER_NUMBER`].
    #[test]
    fn a_prodos_volume_defaults_to_the_apple_iigs() {
        assert_eq!(
            DiskImage::ProDos.interpreter_number(),
            Some(10),
            "ZMSD §11.1.3: 10 = Apple IIgs, and `IIgsID EQU 10 ; ][gs Yzip`",
        );
        assert_eq!(DiskImage::ProDos.label(), "ProDOS");
        // …and it is the ONE number, not one of the family's three: the row has
        // to name a machine, and the other two stay reachable by asking for them.
        assert_ne!(DiskImage::ProDos.interpreter_number(), Some(2), "the IIe is not the default");
        assert_ne!(DiskImage::ProDos.interpreter_number(), Some(9), "nor is the IIc");
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
                // …and the Apple II answers like the ST rather than like DOS,
                // which is the reversal SQ-0857 argued at the row. ProDOS still
                // names the FAMILY and §11.1.3 still numbers three machines in
                // it — but declining lands a ProDOS story on 1 or 6, the
                // DECSystem-20 or the IBM PC, so `None` names the wrong machine
                // rather than no machine. 10 is the top of the family the Apple
                // YZIP will run on, and `--interpreter` still reaches the
                // other two.
                DiskImage::ProDos => ("ProDOS", Some(APPLE_IIGS_INTERPRETER_NUMBER)),
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
        // `dsk` joined them in SQ-0864, on the ProDOS row: the fourteen
        // 5.25-inch images in `stories/` are ProDOS volumes whose sectors are in
        // the drive's order rather than the filesystem's, and they mount through
        // the same reader one de-interleave earlier.
        // `po` joined them in SQ-0863, on the same row and by the same rule: the
        // corpus acquired four bare ProDOS volumes (`Arthur.po`, `Journey.po`,
        // `ZorkZero.po` and a `Shogun.po` that is really a DiskCopy wrapper), and
        // three of them mount. A spelling is claimed when a medium wears it.
        for want in ["adf", "image", "ima", "img", "st", "2mg", "dsk", "po"] {
            assert!(image_extensions().any(|e| e == want), "no row claims {want:?}");
        }
        // …and a spelling still does NOT get a name before a medium wears it:
        // `hdv` is the other bare-ProDOS convention, this crate can open one,
        // and nothing in the corpus is called that.
        const QUEUED: &str = "hdv";
        assert!(
            !image_extensions().any(|e| e == QUEUED),
            "{QUEUED:?} is claimed by a row, and no medium in the corpus justifies it"
        );
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

    // ── A story on no single disk (SQ-0864) ───────────────────────────────────

    /// `stories/`, gitignored — every case below skips vacuously without it, and
    /// CI has none of it at all.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    /// The 5.25-inch sets, as the filenames in `stories/` spell them.
    const SHOGUN_SET: [&str; 5] = [
        "shogun_s1.dsk",
        "shogun_s2.dsk",
        "shogun_s3.dsk",
        "shogun_s4.dsk",
        "shogun_s5.dsk",
    ];
    const ZORK_ZERO_SET: [&str; 4] =
        ["zork_zero_1.dsk", "zork_zero_2.dsk", "zork_zero_3.dsk", "zork_zero_4.dsk"];

    /// Every volume of `set`, or `None` when any of them is absent.
    fn read_set(set: &[&str]) -> Option<Vec<Vec<u8>>> {
        set.iter().map(|n| std::fs::read(stories_dir().join(n)).ok()).collect()
    }

    /// **The headline.** *Shogun* is on five floppies and *Zork Zero* on four,
    /// and not one of the nine carries a game — so this is the only test in the
    /// file whose subject is a release rather than a disk.
    ///
    /// Every member is opened in turn with the other volumes as companions,
    /// because that is what a person does: they name whichever floppy is on top.
    /// All five must give the same story, under the same name, and it must be
    /// the real one — pinned by release, serial and the story's own header
    /// checksum, which is the oracle that says the pages were reassembled in the
    /// right order rather than merely plausibly (ZMSD §11.1.6).
    /// One 5.25-inch press: its volumes, and the build the whole set carries.
    struct Press {
        /// Every floppy of the release, in disk order.
        set: &'static [&'static str],
        /// The segment carrying the index, which names the reassembled story.
        story_name: &'static str,
        /// The story's declared length, header `$1A` in v6 units.
        length: usize,
        /// Header `$02`.
        release: u16,
        /// Header `$12..$18`.
        serial: &'static str,
        /// Header `$1C` — the oracle that says the pages went back in order.
        checksum: u16,
    }

    #[test]
    fn a_release_pressed_across_five_floppies_opens_from_any_one_of_them() {
        let releases = &[
            Press {
                set: &SHOGUN_SET,
                story_name: "SHOGUN.D1",
                length: 344_224,
                release: 311,
                serial: "890510",
                checksum: 0xE200,
            },
            Press {
                set: &ZORK_ZERO_SET,
                story_name: "ZORK0.D1",
                length: 299_392,
                release: 383,
                serial: "890602",
                checksum: 0x6F7F,
            },
        ];
        let mut ran = 0;
        for Press { set, story_name, length, release, serial, checksum } in releases {
            let Some(images) = read_set(set) else {
                eprintln!("SKIP: {} is not complete in stories/", set[0]);
                continue;
            };
            ran += 1;
            for (n, named) in images.iter().enumerate() {
                let rest: Vec<Vec<u8>> = images
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != n)
                    .map(|(_, b)| b.clone())
                    .collect();
                let who = set[n];
                let disk = MountedDisk::mount_set(named.clone(), || rest)
                    .unwrap_or_else(|e| panic!("{who}: {e}"));
                // It is a ProDOS volume like any other, and answers so.
                assert_eq!(disk.format(), DiskImage::ProDos, "{who}");
                assert_eq!(disk.label(), "ProDOS", "{who}");
                assert_eq!(
                    disk.interpreter_number(),
                    Some(APPLE_IIGS_INTERPRETER_NUMBER),
                    "{who}: a 5.25-inch press is Apple II media like the 3.5-inch one",
                );

                let stories = disk.stories();
                assert_eq!(stories.len(), 1, "{who}: {:?}", stories.len());
                let story = &stories[0];
                assert_eq!(story.name, *story_name, "{who}: named for the index segment");
                assert_eq!(disk.story().map(|s| s.name), Some(story.name.clone()), "{who}");
                assert_eq!(disk.story().map(|s| s.bytes), Some(story.bytes.clone()), "{who}");

                let bytes = &story.bytes;
                assert_eq!(bytes.len(), *length, "{who}: the story's declared length");
                assert_eq!(bytes[0], 6, "{who}: Version 6");
                assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), *release, "{who}");
                assert_eq!(&bytes[0x12..0x18], serial.as_bytes(), "{who}");
                let sum = bytes[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
                assert_eq!(sum, *checksum, "{who}: ZMSD §11.1.6 checksum");
                assert_eq!(u16::from_be_bytes([bytes[0x1c], bytes[0x1d]]), sum, "{who}");
            }
        }
        // CI carries no `stories/`, so the premise is guarded and not asserted.
        assert!(ran > 0 || !stories_dir().join(SHOGUN_SET[0]).exists());
        if ran == 0 {
            eprintln!("SKIP: no 5.25-inch media present");
        }
    }

    /// **What a lone volume of a set does**, which is the question a person asks
    /// by double-clicking `shogun_s3.dsk`.
    ///
    /// It mounts. It is a real ProDOS volume with a real name and a real file on
    /// it, and it is honest that the file is not a game — so the front-ends'
    /// existing "no story file on the disk image (N files on SHOGUN.3; is this
    /// the boot disk?)" is what a person is told, rather than a refusal that
    /// would suggest the floppy was unreadable or a truncated story that would
    /// crash later. **Nothing is ever handed over half-assembled**: the header
    /// checksum in [`crate::infocom_packed`] refuses four fifths of a game.
    #[test]
    fn a_lone_volume_of_a_set_mounts_and_says_it_has_no_game() {
        // (image, volume name, files on it)
        let lone: &[(&str, &str, usize)] = &[
            ("shogun_s1.dsk", "SHOGUN.1", 4), // the one carrying the index…
            ("shogun_s3.dsk", "SHOGUN.3", 1), // …and one that carries only pages
            ("zork_zero_1.dsk", "ZORK0.1", 4),
            ("zork_zero_4.dsk", "ZORK0.4", 1),
        ];
        let mut ran = 0;
        for (file, volume, files) in lone {
            let Ok(raw) = std::fs::read(stories_dir().join(file)) else { continue };
            ran += 1;
            assert_eq!(DiskImage::detect(&raw), Some(DiskImage::ProDos), "{file}");
            let disk = MountedDisk::mount(raw).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(disk.volume_name(), Some(*volume), "{file}");
            assert_eq!(disk.file_count(), *files, "{file}");
            assert!(disk.stories().is_empty(), "{file}: a fifth of a game is not a game");
            assert_eq!(disk.story(), None, "{file}");
        }
        assert!(ran > 0 || !stories_dir().join("shogun_s1.dsk").exists());
        if ran == 0 {
            eprintln!("SKIP: no 5.25-inch media present");
        }
    }

    /// **Volumes that are not one release are refused, not spliced.**
    ///
    /// Shogun's index disk with Zork Zero's three page disks is a set that
    /// pairs by NAME perfectly well — every `SHOGUN.D2`…`D5` is simply absent —
    /// and giving it Shogun's own disk 2 beside three of Zork Zero's is the
    /// sharper case: the names now resolve and the pages do not. Either way
    /// nothing is handed out, which is the property that makes name-based
    /// pairing safe at all (`infocom_packed`'s header-checksum oracle).
    #[test]
    fn volumes_from_two_different_releases_assemble_nothing() {
        let (Some(shogun), Some(zork)) = (read_set(&SHOGUN_SET), read_set(&ZORK_ZERO_SET)) else {
            eprintln!("SKIP: both 5.25-inch sets are needed");
            assert!(!stories_dir().join(SHOGUN_SET[0]).exists());
            return;
        };
        // Shogun's index disk, with Zork Zero's volumes for company.
        let strangers: Vec<Vec<u8>> = zork[1..].to_vec();
        let disk = MountedDisk::mount_set(shogun[0].clone(), || strangers).expect("mounts");
        assert!(disk.stories().is_empty(), "no story spans these five");
        assert_eq!(disk.story(), None);

        // And the other direction, with one real sibling in the mix.
        let mixed: Vec<Vec<u8>> = vec![shogun[1].clone(), zork[1].clone(), zork[2].clone()];
        let disk = MountedDisk::mount_set(shogun[0].clone(), || mixed).expect("mounts");
        assert!(disk.stories().is_empty(), "three of Shogun's five are still not Shogun");
    }

    /// **Nobody else pays for the set.** The companions closure is called only
    /// when the named volume has no story of its own, so a library scan does not
    /// read seven 800 KB floppies to list one of them.
    ///
    /// Stated over every format's synthetic disk, because it is a property of
    /// the seam and not of ProDOS: each sample carries `STORY.DAT`, so each must
    /// answer for itself and never ask.
    #[test]
    fn a_volume_with_its_own_story_never_asks_for_its_siblings() {
        for image in DiskImage::all() {
            let asked = std::cell::Cell::new(false);
            let disk = MountedDisk::mount_set(sample_of(image), || {
                asked.set(true);
                Vec::new()
            })
            .expect("mounts");
            assert!(!asked.get(), "{image:?} read its siblings to open itself");
            assert_eq!(disk.stories().len(), 1, "{image:?}");
        }
        // …and a volume with nothing on it does ask, which is the other half.
        let files: [(&str, &[u8]); 1] = [("Readme", b"just a text file")];
        let asked = std::cell::Cell::new(false);
        let disk = MountedDisk::mount_set(crate::adf::tests::sample_disk(&files), || {
            asked.set(true);
            Vec::new()
        })
        .expect("mounts");
        assert!(asked.get(), "a volume with no game must consult its release");
        assert!(disk.stories().is_empty());
    }

    /// A 5.25-inch dump that is not a ProDOS volume is not a disk image here —
    /// refused outright rather than de-interleaved into something that gets
    /// misread. `.dsk` is overwhelmingly Apple II media and most of it is DOS
    /// 3.3, which this crate does not read.
    #[test]
    fn a_five_and_a_quarter_inch_image_that_is_not_prodos_is_refused() {
        let blank = vec![0u8; crate::dos_order::DOS_ORDER_LEN];
        assert_eq!(DiskImage::detect(&blank), None);
        assert_eq!(MountedDisk::mount(blank).err(), Some(MountError::NotADiskImage));
        // Noise of the right size, likewise — the volume directory decides.
        let noise: Vec<u8> =
            (0..crate::dos_order::DOS_ORDER_LEN).map(|i| (i * 31 + 7) as u8).collect();
        assert_eq!(DiskImage::detect(&noise), None);
    }

    /// The same property on the Apple IIgs press (SQ-0836). *Lost Treasures*
    /// volume 2 is the ProDOS analogue of the ST compilation above: five games
    /// on one disk, four of somebody's saved games beside them, and a format
    /// with no conventional story name at all — so the LIST is the answer and
    /// `story()`'s largest-wins tiebreak is only the default.
    ///
    /// It also pins the medium's own number, which is the interesting one: a
    /// ProDOS volume answers **10, the Apple IIgs** (SQ-0857, reversing SQ-0836).
    /// ZMSD §11.1.3 does give the Apple II family three numbers, and nothing on a
    /// volume says which machine pressed it — Infocom's own Apple II YZIP settles
    /// that by detecting the machine at boot. What makes 10 the answer anyway is
    /// that declining names a machine too, and the one it names is the
    /// DECSystem-20 or the IBM PC. See the row in [`FORMATS`].
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
            Some(APPLE_IIGS_INTERPRETER_NUMBER),
            "ProDOS names the Apple II family; 10 names the member babelmap presents as",
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
