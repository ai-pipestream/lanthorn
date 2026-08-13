//! The release medium a story was opened from, and the machine it implies
//! (SQ-0839).
//!
//! The user's rule: *"in general when reading a specific disk format we should
//! default the interpreter number to match (even on the cli), but still allow an
//! override."* The medium is EVIDENCE about the machine — a story taken off an
//! AmigaDOS floppy was an Amiga's copy — and evidence has to reach every
//! front-end, not just the one that happened to grow the feature first.
//!
//! That is why the mapping lives here rather than in either front-end. The TUI
//! resolves a whole [`crate::infocom_pics::Flavour`]-aware bundle
//! (`app::interpreter::InterpreterProfile`); `zvm-cli` wants one byte and knows
//! nothing about profiles; and `app` and `zvm-cli` share exactly one dependency
//! that recognises these filesystems at all — this crate. Two copies of "an
//! `.adf` means interpreter 4" is precisely the policy that goes stale in one
//! place and not the other, which is the defect SQ-0839 was filed for.
//!
//! **Recognition is by CONTENT, never by extension** (see [`DiskImage::detect`]),
//! matching every other mount in the codebase: a disk image under any name is
//! recognised, and a mis-named ordinary story file is not.

use crate::adf::Adf;
use crate::hfs::Hfs;

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
    /// The two filesystems are disjoint by construction: AmigaDOS is identified
    /// by its `DOS` boot block and HFS by a volume signature at a fixed offset
    /// (bare, or past a DiskCopy 4.2 header), so the order of the arms is a
    /// formality rather than a precedence.
    pub fn detect(raw: &[u8]) -> Option<DiskImage> {
        if Adf::looks_like_adf(raw) {
            return Some(DiskImage::Adf);
        }
        if Hfs::looks_like_hfs(raw) {
            return Some(DiskImage::Hfs);
        }
        None
    }

    /// The acronym a story list shows beside the format: `Z6 (ADF)`, `Z6 (HFS)`.
    pub fn label(self) -> &'static str {
        match self {
            DiskImage::Adf => "ADF",
            DiskImage::Hfs => "HFS",
        }
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
        match self {
            DiskImage::Adf => Some(AMIGA_INTERPRETER_NUMBER),
            DiskImage::Hfs => None,
        }
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
}
