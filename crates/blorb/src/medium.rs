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
//! ProDOS `2IMG` length field reads zero.

use crate::adf::{Adf, looks_like_story};
use crate::hfs::Hfs;
use crate::infocom_pics::InfocomPics;

/// Amiga, from the ZMSD §11.1.3 interpreter-number table (1 DECSystem-20,
/// 2 Apple IIe, 3 Macintosh, **4 Amiga**, 5 Atari ST, 6 IBM PC, 7 Commodore 128,
/// 8 Commodore 64, 9 Apple IIc, 10 Apple IIgs, 11 Tandy Color).
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

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
    /// **[`DiskImage::Hfs`] deliberately answers `None`, and this is not an
    /// oversight.** ZMSD §11.1.3 numbers the Macintosh 3, and that constant is
    /// verifiable — but the number is not inert: a game reads `$1E` and can take
    /// machine-specific paths, and the Macintosh's default colours, palette and
    /// screen geometry are not established by anything in the corpus. Telling
    /// *Zork Zero* release 296 that it is on a Macintosh while rendering it as a
    /// PC is a behaviour change with no evidence behind it. A Mac disk therefore
    /// keeps resolving to the IBM PC default, pinned in the app's
    /// `real_media_releases` and `hfs_disk_image` suites, until SQ-0838 lands the
    /// profile half that would make the number honest.
    pub fn interpreter_number(self) -> Option<u8> {
        self.row().interpreter_number
    }
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
    /// The content sniff — [`Volume::looks_like`] for the reader below.
    looks_like: fn(&[u8]) -> bool,
    /// The mount — [`Volume::mount`], boxed so the table can be one type.
    mount: fn(Vec<u8>) -> Option<Box<dyn Volume>>,
}

/// **Every disk format babelmap reads.** Adding one is a row here plus the
/// `impl Volume` it names, and nothing else anywhere: `detect`, `mount`,
/// `label`, `interpreter_number`, the TUI's story loading and picture
/// resolution, and `zvm-cli`'s disk menu all read this table and none of them
/// knows a format's name.
///
/// Queued: DOS/ST FAT12 (SQ-0833) and ProDOS (SQ-0836).
const FORMATS: &[Format] = &[
    Format {
        image: DiskImage::Adf,
        label: "ADF",
        interpreter_number: Some(AMIGA_INTERPRETER_NUMBER),
        looks_like: <Adf as Volume>::looks_like,
        mount: mount_boxed::<Adf>,
    },
    Format {
        image: DiskImage::Hfs,
        label: "HFS",
        interpreter_number: None,
        looks_like: <Hfs as Volume>::looks_like,
        mount: mount_boxed::<Hfs>,
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
    /// The stored filename, exactly as the volume spells it.
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

    fn story(&self) -> Option<DiskStory> {
        Hfs::story(self).map(DiskStory::from)
    }

    fn pictures(&self) -> Option<DiskArt> {
        Hfs::pictures(self).map(DiskArt::from)
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

    /// Every story on the disk, in disk order.
    pub fn stories(&self) -> Vec<DiskStory> {
        self.volume.stories()
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

    /// SQ-0838's block, pinned so it cannot be "fixed" by someone who reads
    /// §11.1.3 and stops there. The Macintosh's number is known; its machine is
    /// not, and this crate hands out numbers for machines we can present.
    #[test]
    fn a_macintosh_disk_names_itself_but_defaults_no_number() {
        assert_eq!(DiskImage::Hfs.interpreter_number(), None, "SQ-0838, not an oversight");
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
        let files: [(&str, &[u8]); 2] = [("Readme", b"just a text file"), ("Story.data", &story)];
        match image {
            DiskImage::Adf => crate::adf::tests::sample_disk(&files),
            DiskImage::Hfs => crate::hfs::tests::sample_disk(&files),
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
        let census = [DiskImage::Adf, DiskImage::Hfs];
        for image in census {
            let (label, interpreter) = match image {
                DiskImage::Adf => ("ADF", Some(AMIGA_INTERPRETER_NUMBER)),
                DiskImage::Hfs => ("HFS", None),
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

            // Identified by CONTENT: `Readme` is not a story and `Story.data`
            // is, and no format is allowed to decide that by name.
            let stories = disk.stories();
            assert_eq!(stories.len(), 1, "{image:?} found {stories:?}");
            assert_eq!(stories[0].name, "Story.data", "{image:?}");
            assert_eq!(stories[0].bytes, fake_story(), "{image:?} reads it byte-exact");
            assert_eq!(disk.story().map(|s| s.bytes), Some(fake_story()), "{image:?}");

            // No archive on a synthetic disk — the point is that the question is
            // answerable at all, on every format, without a panic or a chain.
            assert!(disk.pictures().is_none(), "{image:?}");
            let _ = disk.volume_name();
        }
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
    /// through the shared path and hand back their own story and their own art.
    /// They live outside the repo, so each arm skips vacuously.
    #[test]
    fn real_release_disks_of_every_format_mount_through_one_path() {
        for image in DiskImage::all() {
            let fixture = match image {
                DiskImage::Adf => "Zork Zero - The Revenge of Megaboz.adf",
                DiskImage::Hfs => "Zork Zero Disk.image",
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
            assert_eq!(story.name, "Story.data", "{fixture}");
            assert_eq!(story.bytes[0], 6, "both Zork Zeros are v6: {fixture}");
            let art = disk.pictures().expect("…and its own artwork");
            assert!(art.pictures.entries().len() > 100, "{fixture}: {}", art.name);
        }
    }
}
