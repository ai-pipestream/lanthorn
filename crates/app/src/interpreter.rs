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
//! 2. The ART: a picture archive named outright in the per-game sidecar
//!    (`pictures = "…"`, tier 3 of the resource policy — see
//!    [`crate::graphics::PictureOverride`]). Asking for a game's EGA rendition is
//!    asking for the IBM PC that drew it; asking for its `Pic.data` is asking for
//!    the Amiga. The flavour comes from the archive's CONTENT — the two codecs
//!    are structurally distinguishable — not from its extension, which a rename
//!    can make a lie.
//! 3. The medium: a story mounted out of an Amiga `.adf` release floppy is an
//!    Amiga.
//! 4. [`InterpreterProfile::IbmPc`], for everything else.
//!
//! Step 2 cannot move the existing corpus, and that is worth stating because
//! header byte `$1E` is not inert — `zvm`'s `exec.rs` branches on
//! `read_byte(0x1E) == 6`, so a v6 story that stopped being an IBM PC would
//! start *doing* something different. Two things pin it. The key that triggers
//! the inference is new, so no story in `stories/` has one; and the only
//! non-Amiga flavour, [`Flavour::Pc`], maps to `IbmPc`, whose
//! [`interpreter_number`](InterpreterProfile::interpreter_number) is `None` and
//! therefore leaves zvm's own rule (6 for v6) exactly where it was. Nothing
//! moves unless a user writes a `pictures` key naming an Amiga archive, which is
//! precisely the request being honoured.
//!
//! Authenticity can cost readability — the Amiga's own default page is a dark
//! grey (see [`AMIGA_DEFAULT_BACKGROUND`]), and a game that picks white text
//! against it was legible on a 1989 monitor and is merely adequate in a modern
//! terminal. There is deliberately no new setting for that: `honor_game_colours`
//! already governs whether the game's colour choices are honoured at all, so
//! turning it off returns the user's theme, profile or no profile.
//!
//! It can also cost a babelmap CONVENIENCE, and that has to be paid too. §8.3's
//! Amiga has exactly two pens for the whole screen, so the transcript's built-in
//! "a whole line in brackets came from the interpreter, mute it" rule is wrong
//! twice over there — the line is the game's prose, and the mute was picked to
//! recede against the theme's page rather than the machine's. It stands down on
//! this profile only; see [`crate::colors::ColorScheme::resolve_story_style`].

use std::path::Path;

use blorb::infocom_pics::Flavour;

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
    /// else the flavour of a picture archive the user named outright, else the
    /// medium the story came out of, else [`Self::IbmPc`]. See the module docs
    /// for why step two cannot disturb the existing corpus.
    ///
    /// `story_path` is the path the user opened, which for a disk image is the
    /// image itself rather than the story inside it — that is the whole point,
    /// since the medium is what identifies the machine.
    ///
    /// `named_art` is [`crate::graphics::PictureOverride::flavour`]: `None`
    /// whenever no usable archive was named, which is every launch that does not
    /// use tier 3.
    pub fn resolve(
        story_path: &Path,
        configured_interpreter_number: Option<u8>,
        named_art: Option<Flavour>,
    ) -> Self {
        if let Some(n) = configured_interpreter_number {
            return Self::for_interpreter_number(n);
        }
        if let Some(flavour) = named_art {
            return Self::for_art_flavour(flavour);
        }
        if Self::is_adf(story_path) {
            return Self::Amiga;
        }
        Self::IbmPc
    }

    /// The machine implied by the flavour of a picture archive.
    ///
    /// [`Flavour::Pc`] covers `.MG1` (MCGA), `.EG1`/`.EG2` (EGA) and `.CG1`
    /// (CGA) — three video cards, one machine, and that machine is the IBM PC.
    /// The card is a display choice, not a Z-machine one: Frotz's DOS port picks
    /// the extension from its display mode and never consults byte `$1E` to do
    /// it, so there is nothing finer than "IBM PC" for the header to say.
    ///
    /// [`Flavour::AmigaMac`] is named for a real limit rather than a shortcut:
    /// the Amiga and the Macintosh wrote the *same* big-endian Huffman container,
    /// and nothing in it distinguishes them in general. (ZMSD §11.1.3 numbers
    /// them separately — 3 Macintosh, 4 Amiga — so the distinction would matter
    /// if it could be made.) The one lead is Spatterlight's bocfel, which
    /// reclassifies a `Pic.data` as monochrome Macintosh when the file's flags
    /// byte reads `0x0e`, with the honest comment that the flags "always *seem*
    /// to equal 0xe if the graphics are monochrome" — a heuristic, and one that
    /// separates only the B&W Mac. babelmap has no Macintosh profile to select
    /// anyway, and every colour `Pic.data` in hand is Amiga media, so the whole
    /// flavour resolves to [`Self::Amiga`]. Give this a real discriminator before
    /// a Macintosh bundle is added, not after.
    pub fn for_art_flavour(flavour: Flavour) -> Self {
        match flavour {
            Flavour::Pc => Self::IbmPc,
            Flavour::AmigaMac => Self::Amiga,
        }
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
    ///
    /// **It survived the case that was expected to move it** (SQ-0790). EGA ran
    /// 640×200 on an 8×8 cell, which reads like a display mode this knob would
    /// have to express — but 640/8 by 200/8 is 80×25, the *same character grid*
    /// the 640×400 unit screen already lays out on its 8×16 cell. Halving both
    /// axes would buy an identical grid with half the vertical precision for
    /// window and cursor coordinates, at the cost of making
    /// `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` runtime state throughout `zvm`'s screen
    /// model, its location heuristics and the app's render path. What genuinely
    /// differs about an EGA or CGA rendition is that its ART is stored at twice
    /// the horizontal density, and that belongs to the archive rather than to
    /// the machine — three video cards, one IBM PC, one cell. See
    /// [`crate::graphics::PictSource::art_scale`].
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

/// The Amiga's default background: standard colour **12**, dark grey (`$444`).
///
/// **Do not "correct" this back to 11 on the strength of `amiga/yzip.h`** — that
/// file is a development snapshot, its own `#define DEF_BACK 11 /*6*/` carries the
/// scar of a previous edit, and the value that shipped is 12 (SQ-0822).
///
/// The authority is the interpreter on the release floppy, which is the program
/// that painted the screen. `set_back()` in `amiga/yzip3.c` opens
/// `if (id == 1) id = DEF_BACK;` — "colour 1 means the default" — and `set_fore()`
/// opens with the same line on `DEF_FORE`. Both compile to a `cmpi.w #1` and a
/// `moveq`, and both appear, once each and in that order, in every Amiga Version 6
/// interpreter in `stories/`:
///
/// ```text
///   0c 47 00 01   cmpi.w #1,d7      0c 47 00 01   cmpi.w #1,d7
///   66 02         bne.s  .+2        66 02         bne.s  .+2
///   7e 09         moveq  #9,d7      7e 0c         moveq  #12,d7
///   … set_fore: DEF_FORE = 9        … set_back: DEF_BACK = 12
/// ```
///
/// and `set_color()`'s `return ((DEF_BACK << 8) | DEF_FORE)` assembles to
/// `30 3c 0c 09` — `move.w #$0C09,d0` — in all four. `$0B09` occurs in none of
/// them. Offsets, per interpreter binary extracted from its floppy: Arthur
/// (release 54) 18958/19094/18742, Journey (release 30) 17816/17952/17792,
/// Zork Zero (release 366) and Shogun (release 295) 17820/17956/17796.
///
/// It is also what the screen shows. Real Amiga captures of Journey release 30
/// (lemonamiga.com's gallery, 640×512) tally 173,994 pixels of `#444444` under
/// 25,878 of `#FFFFFF`, and MobyGames' Arthur church capture is `#444444` page,
/// `#FFFFFF` ink, with the status bar reversed to `#444444` ON `#FFFFFF` — which
/// is pen 0 and pen 1 swapped, so the page really is the text background register
/// and not artwork. `$444` is Infocom's colour 12; `$777`, colour 11, appears in
/// neither. [`crate::colors`] resolves 12 through the Amiga palette to `(66,66,66)`
/// — two units off `#444444` only because a 4-bit Amiga channel has to pass
/// through the Z-machine's 5-bit true-colour word on the way.
///
/// This does not disturb SQ-0740's window-0 gate: the evidence for that was that
/// Journey is *not black*, and a dark-grey page is not a black one.
pub const AMIGA_DEFAULT_BACKGROUND: u8 = 12;

/// The Amiga's default foreground: standard colour 9, white.
///
/// Source: `set_fore()`'s `if (id == 1) id = DEF_FORE;` in the interpreter on every
/// Infocom Amiga release floppy — `moveq #9` — agreeing with
/// `#define DEF_FORE 9  /* default Amiga foreground = white */` in `amiga/yzip.h`.
/// See [`AMIGA_DEFAULT_BACKGROUND`], where the two sources do *not* agree.
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
        assert_eq!(
            p.default_colours(),
            Some((12, 9)),
            "the release floppies' own DEF_BACK/DEF_FORE (SQ-0822)"
        );
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
        assert_eq!(InterpreterProfile::resolve(missing, None, None), InterpreterProfile::IbmPc);
        // …and an explicit number still decides without ever touching the disk.
        assert_eq!(InterpreterProfile::resolve(missing, Some(4), None), InterpreterProfile::Amiga);
    }

    #[test]
    fn a_named_archives_flavour_names_the_machine() {
        // SQ-0734 precedence 2. MCGA/EGA/CGA are three cards on ONE machine.
        assert_eq!(InterpreterProfile::for_art_flavour(Flavour::Pc), InterpreterProfile::IbmPc);
        assert_eq!(
            InterpreterProfile::for_art_flavour(Flavour::AmigaMac),
            InterpreterProfile::Amiga,
        );
    }

    #[test]
    fn the_named_archive_outranks_the_medium_and_yields_to_an_explicit_number() {
        let plain = std::path::Path::new("/nonexistent/babelmap/no-such-story.z6");
        // Naming an Amiga archive beside an ordinary file makes it an Amiga…
        assert_eq!(
            InterpreterProfile::resolve(plain, None, Some(Flavour::AmigaMac)),
            InterpreterProfile::Amiga,
        );
        // …and an explicit number still outranks it (precedence 1 over 2).
        assert_eq!(
            InterpreterProfile::resolve(plain, Some(6), Some(Flavour::AmigaMac)),
            InterpreterProfile::IbmPc,
        );
    }

    #[test]
    fn naming_a_pc_archive_cannot_move_header_byte_1e() {
        // The blast-radius pin. `zvm`'s `exec.rs` branches on `$1E == 6`, and
        // every v6 story gets that today from zvm's own Frotz rule. Naming a
        // `.MG1`/`.EG1`/`.CG1` selects IBM PC, which has NO opinion on the
        // number — so the byte the story reads is the byte it read before, and
        // the whole v6 corpus is untouched by this feature by construction.
        let profile = InterpreterProfile::for_art_flavour(Flavour::Pc);
        assert_eq!(profile, InterpreterProfile::IbmPc);
        assert_eq!(profile.interpreter_number(), None, "zvm's rule stays in force");
    }
}
