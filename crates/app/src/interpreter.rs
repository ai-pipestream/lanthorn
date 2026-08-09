//! The interpreter profile: which historical machine babelmap presents itself
//! as (SQ-0719).
//!
//! A Version 6 story asks the header what it is running on and then behaves
//! differently — Zork Zero picks a whole colour scheme from it, Beyond Zork
//! swaps Font 3 arrows for CP437 character graphics. Byte `$1E` alone is not
//! enough to answer that question honestly, because the answer is a *bundle*:
//! the machine's screen, its interpreter number, the colours it says are its
//! defaults, and the palette its colour numbers name. Setting one of those in
//! isolation produces an incoherent machine — a byte that changes what games do
//! without changing the machine it implies — which is exactly what happened
//! when `interpreter_number = 4` was set by hand and the artwork kept its IBM PC
//! scale while the text turned white-on-grey.
//!
//! So the bundle is one named thing, with two members:
//!
//! - [`InterpreterProfile::IbmPc`] is **today's behaviour, named**. Nothing here
//!   is new: interpreter number by Frotz's rule (6 for v6, 1 otherwise), the
//!   Blorb `Reso` chunk as the standard window, default colours taken from the
//!   user's terminal, ZMSD §8.3.1 colour resolution, the 8×16 v6 cell. Every
//!   knob below returns "no opinion" for it, which is what makes it byte-for-byte
//!   what shipped.
//! - [`InterpreterProfile::Amiga`] is the sibling, for stories that came off
//!   Amiga media.
//!
//! **Selection**, most specific first (SQ-0734):
//!
//! 1. An explicit `interpreter_number` (config or `--interpreter-number`) — the
//!    number you name is the machine you are asking for, and it brings its whole
//!    profile with it.
//! 2. The medium: a story mounted out of an Amiga `.adf` release floppy is an
//!    Amiga.
//! 3. [`InterpreterProfile::IbmPc`], for everything else.
//!
//! Authenticity can cost readability — the Amiga's own default page is a medium
//! grey, and a game that picks white text against it was legible on a 1989
//! monitor and is merely adequate in a modern terminal. There is deliberately no
//! new setting for that: `honor_game_colours` already governs whether the game's
//! colour choices are honoured at all, so turning it off returns the user's
//! theme, profile or no profile.

use std::path::Path;

/// The machine babelmap presents itself to the story as. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InterpreterProfile {
    /// Today's behaviour, named: an IBM PC (interpreter 6 on v6, 1 elsewhere),
    /// the container's own declared art resolution, the host terminal's colours
    /// and ZMSD §8.3.1's palette.
    #[default]
    IbmPc,
    /// An Amiga: interpreter 4, a 320×200 standard window doubled onto the
    /// 640×400 screen, the Amiga's own default colours, and the palette
    /// Infocom's Amiga interpreter loaded.
    Amiga,
}

impl InterpreterProfile {
    /// Resolve the profile for a launch: an explicit interpreter number wins,
    /// else the medium the story came out of, else [`Self::IbmPc`].
    ///
    /// `story_path` is the path the user opened, which for a disk image is the
    /// image itself rather than the story inside it — that is the whole point,
    /// since the medium is what identifies the machine.
    pub fn resolve(story_path: &Path, configured_interpreter_number: Option<u8>) -> Self {
        if let Some(n) = configured_interpreter_number {
            return Self::for_interpreter_number(n);
        }
        if Self::is_adf(story_path) {
            return Self::Amiga;
        }
        Self::IbmPc
    }

    /// The profile a story header byte `$1E` value implies. Only the Amiga has a
    /// profile of its own so far; every other machine is served by the IBM PC
    /// bundle, which is also the historical default.
    pub fn for_interpreter_number(n: u8) -> Self {
        if n == AMIGA_INTERPRETER_NUMBER { Self::Amiga } else { Self::IbmPc }
    }

    /// Does `path` hold an Amiga `.adf` release floppy?
    ///
    /// Content, not extension: `Adf::looks_like_adf` checks the AmigaDOS boot
    /// block, exactly as `PictSource::resolve` and `hints::read_story_file` do,
    /// so a disk image with any name (or none) is recognised and a mis-named
    /// ordinary story file is not.
    fn is_adf(path: &Path) -> bool {
        std::fs::read(path).map(|raw| blorb::adf::Adf::looks_like_adf(&raw)).unwrap_or(false)
    }

    /// The interpreter number to advertise in header `$1E`, or `None` to leave
    /// the VM's own default rule in force.
    ///
    /// [`Self::IbmPc`] returns `None` on purpose rather than computing 6-or-1
    /// here: zvm's existing default (Frotz's rule — 6 for Version 6, 1
    /// otherwise) *is* the IBM PC rule, and deferring to it means naming the
    /// profile cannot possibly change what the corpus advertises.
    pub fn interpreter_number(self) -> Option<u8> {
        match self {
            Self::IbmPc => None,
            Self::Amiga => Some(AMIGA_INTERPRETER_NUMBER),
        }
    }

    /// The standard window — the machine's native ART resolution — when the
    /// resource container declares none, or `None` to keep the container's
    /// answer as the only one.
    ///
    /// Blorb §11 lets a resource file declare its art's intended resolution in a
    /// `Reso` chunk, and babelmap scales v6 artwork by 2 onto the 640×400 unit
    /// screen only when such a declaration exists — a file with no `Reso`
    /// declares its images non-scalable, so scopa and mysterious01 correctly
    /// draw at 1:1 (SQ-0715). A native Amiga `Pic.data` archive has no `Reso`
    /// chunk because **the format has no such concept**, not because anyone
    /// declared anything, and reading that absence as a declaration is what left
    /// Zork Zero's 320×200 art at half size on a 640×400 screen (SQ-0736). The
    /// machine, not the container, is what knows the answer there — so the
    /// profile supplies it, and the existing rule fires unchanged.
    ///
    /// [`Self::IbmPc`] returns `None`: a Blorb-sourced story keeps deciding for
    /// itself, exactly as before.
    pub fn std_window(self) -> Option<(u16, u16)> {
        match self {
            Self::IbmPc => None,
            Self::Amiga => Some(AMIGA_STD_WINDOW),
        }
    }

    /// The default background/foreground colour numbers this machine reports in
    /// header bytes `$2C`/`$2D` (ZMSD §8.3.3), or `None` to report the host
    /// terminal's own colours.
    ///
    /// [`Self::IbmPc`] returns `None`, which is right for a terminal-native
    /// experience: babelmap tells the story what the player's terminal actually
    /// looks like, so "default" means what the player sees. A profile whose
    /// entire purpose is to present as an Amiga should not be describing the
    /// user's terminal, so [`Self::Amiga`] answers with the Amiga's own pair.
    pub fn default_colours(self) -> Option<(u8, u8)> {
        match self {
            Self::IbmPc => None,
            Self::Amiga => Some((AMIGA_DEFAULT_BACKGROUND, AMIGA_DEFAULT_FOREGROUND)),
        }
    }

    /// The palette the story's colour NUMBERS resolve through.
    pub fn palette(self) -> zvm::screen::Palette {
        match self {
            Self::IbmPc => zvm::screen::Palette::Standard,
            Self::Amiga => zvm::screen::Palette::Amiga,
        }
    }

    /// The Version 6 character cell in native pixels, `(width, height)`.
    ///
    /// The sixth knob of the bundle, and the one that turned out to need no
    /// work: both machines used an 8×16 cell for Version 6 (80×25 characters on
    /// a 640×400 screen), which is already what zvm advertises. It is stated
    /// here so the bundle is complete rather than partial, and pinned by a test
    /// against `zvm::screen::V6_FONT_WIDTH`/`V6_FONT_HEIGHT` so the two cannot
    /// drift apart unnoticed.
    pub fn v6_font_cell(self) -> (u16, u16) {
        (8, 16)
    }
}

/// Amiga, from the ZMSD §11.1.3 interpreter-number table (1 DECSystem-20,
/// 2 Apple IIe, 3 Macintosh, **4 Amiga**, 5 Atari ST, 6 IBM PC, 7 Commodore 128,
/// 8 Commodore 64, 9 Apple IIc, 10 Apple IIgs, 11 Tandy Color).
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

/// The Amiga Version 6 standard window: 320×200 art, doubled onto the 640×400
/// hi-res interlaced screen the games lay themselves out on. This is the same
/// resolution every Infocom Blorb's `Reso` chunk declares — those Blorbs are
/// Amiga conversions — which is why asserting it here restores exactly the
/// scaling a Blorb-sourced copy of the same game already gets.
pub const AMIGA_STD_WINDOW: (u16, u16) = (320, 200);

/// The Amiga's default background: standard colour 11, medium grey.
///
/// Source: `#define DEF_BACK 11 /*6*/  /* default Amiga background = med gray */`
/// in `amiga/yzip.h` of Infocom's released Amiga Version 6 interpreter source.
pub const AMIGA_DEFAULT_BACKGROUND: u8 = 11;

/// The Amiga's default foreground: standard colour 9, white.
///
/// Source: `#define DEF_FORE 9  /* default Amiga foreground = white */` in
/// `amiga/yzip.h` of Infocom's released Amiga Version 6 interpreter source.
pub const AMIGA_DEFAULT_FOREGROUND: u8 = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibm_pc_is_the_default_and_has_no_opinion_anywhere() {
        // The acceptance criterion for SQ-0719 in one test: naming today's
        // behaviour must not BE a behaviour. Every knob defers.
        let p = InterpreterProfile::default();
        assert_eq!(p, InterpreterProfile::IbmPc);
        assert_eq!(p.interpreter_number(), None, "defer to zvm's Frotz rule");
        assert_eq!(p.std_window(), None, "defer to the container's Reso chunk");
        assert_eq!(p.default_colours(), None, "defer to the host terminal");
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "ZMSD §8.3.1");
    }

    #[test]
    fn amiga_knobs_are_the_verified_constants() {
        let p = InterpreterProfile::Amiga;
        assert_eq!(p.interpreter_number(), Some(4), "ZMSD §11.1.3: 4 = Amiga");
        assert_eq!(p.std_window(), Some((320, 200)));
        assert_eq!(p.default_colours(), Some((11, 9)), "yzip.h DEF_BACK/DEF_FORE");
        assert_eq!(p.palette(), zvm::screen::Palette::Amiga);
    }

    #[test]
    fn the_v6_cell_matches_what_zvm_advertises() {
        // Knob 6: stated for completeness, pinned so it cannot silently drift.
        for p in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
            assert_eq!(
                p.v6_font_cell(),
                (zvm::screen::V6_FONT_WIDTH, zvm::screen::V6_FONT_HEIGHT),
                "{p:?} v6 cell",
            );
        }
    }

    #[test]
    fn an_explicit_interpreter_number_selects_the_whole_profile() {
        // SQ-0734 precedence 1, and the fix for the incoherent machine the user
        // hit: asking for interpreter 4 asks for the Amiga, not just the byte.
        assert_eq!(InterpreterProfile::for_interpreter_number(4), InterpreterProfile::Amiga);
        // Every other number is served by the IBM PC bundle, the historical default.
        for n in [1u8, 2, 3, 5, 6, 7, 8, 9, 10, 11] {
            assert_eq!(
                InterpreterProfile::for_interpreter_number(n),
                InterpreterProfile::IbmPc,
                "interpreter {n}",
            );
        }
    }

    #[test]
    fn a_missing_file_is_not_a_disk_image() {
        let missing = std::path::Path::new("/nonexistent/babelmap/no-such-story.z6");
        assert_eq!(InterpreterProfile::resolve(missing, None), InterpreterProfile::IbmPc);
        // …and an explicit number still decides without ever touching the disk.
        assert_eq!(InterpreterProfile::resolve(missing, Some(4)), InterpreterProfile::Amiga);
    }
}
