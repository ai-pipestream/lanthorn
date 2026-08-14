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
//! - [`InterpreterProfile::Macintosh`] is the third, for stories that came off
//!   an HFS volume (SQ-0838).
//! - [`InterpreterProfile::AtariSt`] is the fourth, for stories that came off a
//!   GEMDOS floppy (SQ-0835) — and the one that shows a profile may honestly
//!   **decline** a member of the bundle. It states a number, a default page and
//!   a palette, all read out of Infocom's own ST interpreters, and states no
//!   standard window, because Infocom never wrote a Version 6 interpreter for
//!   the ST and a standard window is a Version 6 art geometry.
//!
//!   That distinction is worth keeping straight, because it is the whole reason
//!   this profile was blocked for one commit. The bundle argument above is a
//!   warning against a number that **contradicts** the rest of the machine, and
//!   the ST corpus is where that cannot happen: all thirty-nine stories on the
//!   nine compilations are v3, v4 or v5, so there is no artwork for the number
//!   to disagree with. "I cannot verify every member" is an argument for
//!   declining the members you cannot verify, not for declining the ones you
//!   can — [`InterpreterProfile::IbmPc`] has answered `None` from
//!   [`default_colours`](InterpreterProfile::default_colours) all along.
//! - [`InterpreterProfile::AppleIIgs`] is the fifth, for stories that came off a
//!   ProDOS volume (SQ-0857), and it is the machine that makes "decline" a
//!   *judgement* rather than a shortage. Its number, its black page and its
//!   palette all come out of the Apple II YZIP — Infocom's own Version 6
//!   interpreter for the machine, which is not merely in the source archive but
//!   sitting on two of the disks in `stories/`. It states no standard window
//!   even so, because the Apple's Version 6 screen is 140x192 on a 3x9 cell and
//!   that is a different screen MODEL, not a resolution this knob can hold. See
//!   [`InterpreterProfile::std_window`].
//!
//!   It is also the profile whose NUMBER had to be argued rather than read. The
//!   Amiga, the Macintosh and the ST each write one byte and mean it; the Apple
//!   II YZIP *detects the machine at boot* and writes 2, 9 or 10 accordingly, so
//!   the medium genuinely cannot name the press. What settles it is that
//!   declining is not neutral: zvm's own rule would tell an Apple II story it is
//!   a DECSystem-20, or on Version 6 an IBM PC. §11.1.3 asks an interpreter to
//!   "choose the interpreter number most suitable for the machine it will run
//!   on", and of the three machines that YZIP will start on at all, the one a
//!   modern terminal resembles is the IIgs. [`blorb::medium`] carries the whole
//!   argument and the measurement behind it.
//!
//! **Selection**, most specific first (SQ-0734):
//!
//! 1. An explicit `interpreter_number` (config or `--interpreter`) — the
//!    number you name is the machine you are asking for, and it brings its whole
//!    profile with it.
//! 2. The ART: a picture archive named outright in the per-game sidecar
//!    (`pictures = "…"`, tier 3 of the resource policy — see
//!    [`crate::graphics::PictureOverride`]). Asking for a game's EGA rendition is
//!    asking for the IBM PC that drew it; asking for its `Pic.data` is asking for
//!    the Amiga. The flavour comes from the archive's CONTENT — the two codecs
//!    are structurally distinguishable — not from its extension, which a rename
//!    can make a lie.
//!
//!    **A codec that names two machines is refined by step 3, not settled by
//!    step 2** (SQ-0843). [`Flavour::AmigaMac`] is one container written by both
//!    the Amiga and the Macintosh, so an archive of that flavour states a codec;
//!    when the story also came off a disk, the medium states the machine and
//!    wins. That is not a reordering — [`Flavour::Pc`] is unambiguous and still
//!    beats the medium outright — it is one ambiguous answer resolved by a
//!    definite one. Without it, picking `CPic.data` off `Zork Zero Disk.image`
//!    (the archive that disk loads on its own) turned a Macintosh into an Amiga.
//! 3. The medium: a story mounted out of an Amiga `.adf` release floppy is an
//!    Amiga, one mounted out of an HFS volume is a Macintosh, one mounted out of
//!    an Atari ST GEMDOS floppy is an ST, and one mounted out of an Apple ProDOS
//!    volume is an Apple IIgs. (A DOS FAT12 floppy is the same
//!    filesystem as that last one and still resolves to the IBM PC, whose number
//!    is version-dependent and already in force — see [`blorb::medium`], where
//!    the two rows are argued side by side.) The
//!    medium→machine mapping itself is [`blorb::medium`]'s, not this module's,
//!    because `zvm-cli` has to reach the same conclusion off the same bytes and
//!    does not depend on `app` (SQ-0839).
//!
//!    **The medium is the only honest discriminator for the Macintosh**, and
//!    that is a measurement rather than a preference: the Amiga and the
//!    Macintosh wrote the *same* colour archive, and `Zork Zero Disk.image`
//!    proves it — its `CPic.data` is structurally indistinguishable from an
//!    Amiga `Pic.data`, which is why [`Self::for_art_flavour`] still answers
//!    `Amiga` for the whole of [`Flavour::AmigaMac`]. A volume, by contrast,
//!    cannot be mistaken: HFS is Apple's filesystem and nothing else wrote one.
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
    /// A Macintosh: interpreter 3, black text on a white page, and a screen
    /// that is **whichever one the artwork in hand was drawn for** — see
    /// [`Self::std_window`], which is where the interesting part is.
    Macintosh,
    /// An Atari ST: interpreter 5, black text on a white page, ZMSD §8.3.1's
    /// palette, and **no standard window at all** — the one machine here that
    /// declines a member of the bundle, because it never had a Version 6
    /// interpreter for one to describe (SQ-0835). See [`Self::std_window`].
    AtariSt,
    /// An Apple IIgs: interpreter 10, **white text on a black page** — the only
    /// dark page here that is genuinely black — ZMSD §8.3.1's palette, and no
    /// standard window (SQ-0857).
    ///
    /// The second profile to decline a member, and for a quite different reason
    /// to the Atari ST's. Infocom very much *did* write a Version 6 interpreter
    /// for this machine — the Apple II YZIP, whose own `MACHINE:` routine sits
    /// on `Journey.2mg` and `Arthur Quest 4 Excalibur.2mg` — so there is a real
    /// Version 6 screen to describe. It is 140x192 on a 3x9 cell, which is not
    /// the quantity [`Self::std_window`] holds. See that knob.
    AppleIIgs,
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
            return Self::for_art_flavour_on(flavour, Self::medium(story_path));
        }
        if let Some(n) = Self::medium_interpreter_number(story_path) {
            return Self::for_interpreter_number(n);
        }
        Self::IbmPc
    }

    /// The machine implied by an archive of `flavour` **found on `medium`** —
    /// [`Self::for_art_flavour`] with the one ambiguity its codec cannot settle
    /// resolved by the disk under it (SQ-0843).
    ///
    /// [`Flavour::AmigaMac`] is one container written by two machines, so naming
    /// such an archive states a codec and not a machine. When the story came out
    /// of a disk image, the medium states the machine — HFS is Apple's
    /// filesystem and nothing else wrote one — and a fact beats an ambiguity.
    /// [`Flavour::Pc`] is unambiguous and still beats the medium outright, which
    /// is what keeps "naming an archive is an instruction" true: an `.mg1` named
    /// on a Macintosh disk asks for the IBM PC and gets it.
    ///
    /// **This is what [`Self::for_art_flavour`] already documented and did not
    /// do.** Its own text says "a `Pic.data` on the Mac disk gets the Macintosh
    /// anyway, from the disk under it", and until SQ-0843 the named-archive step
    /// simply returned before the medium was ever consulted. The gap was
    /// invisible while `--pictures Pic.data` was the only door to a Macintosh
    /// archive; the launch-options dialog now lists both of that disk's archives,
    /// so picking `CPic.data` — the very archive the story loads on its own —
    /// would have demoted a Macintosh to an Amiga, and said so on screen two
    /// lines below the row that did it.
    ///
    /// A medium that implies neither Amiga nor Macintosh cannot refine an
    /// Amiga/Mac archive and does not try; the archive's own answer stands.
    pub fn for_art_flavour_on(flavour: Flavour, medium: Option<blorb::medium::DiskImage>) -> Self {
        if flavour == Flavour::AmigaMac {
            let refined = medium
                .and_then(|d| d.interpreter_number())
                .map(Self::for_interpreter_number)
                .filter(|p| matches!(p, Self::Amiga | Self::Macintosh));
            if let Some(profile) = refined {
                return profile;
            }
        }
        Self::for_art_flavour(flavour)
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
    /// separates only the B&W Mac.
    ///
    /// **There is a [`Self::Macintosh`] to select now (SQ-0838), and this still
    /// answers [`Self::Amiga`]** — because the archive is not what knows. The
    /// Macintosh release disk settled it by counterexample: `CPic.data` off
    /// `Zork Zero Disk.image` is a Mac colour archive, and nothing in it
    /// distinguishes it from an Amiga one. Only the MEDIUM does, which is where
    /// the Macintosh hangs (see the module docs, precedence 3). Naming an
    /// archive by hand therefore still asks for the machine the *codec* implies,
    /// and a `Pic.data` on the Mac disk gets the Macintosh anyway, from the disk
    /// under it.
    ///
    /// That last sentence describes [`Self::for_art_flavour_on`], which is where
    /// the disk is actually consulted — and until SQ-0843 nothing did it, so the
    /// claim was true of the design and false of the code. Call that one unless
    /// you genuinely have no medium to offer it.
    ///
    /// The Apple is the one flavour with no ambiguity to resolve: its codec, its
    /// 8-byte record and its 140×192 picture space are peculiar to the Apple II
    /// and no other machine shipped them, so the archive really does name the
    /// machine here (SQ-0863).
    pub fn for_art_flavour(flavour: Flavour) -> Self {
        match flavour {
            Flavour::Pc => Self::IbmPc,
            Flavour::AmigaMac => Self::Amiga,
            Flavour::Apple => Self::AppleIIgs,
        }
    }

    /// The profile a story header byte `$1E` value implies. The Amiga and the
    /// Macintosh have profiles of their own; every other machine is served by
    /// the IBM PC bundle, which is also the historical default.
    pub fn for_interpreter_number(n: u8) -> Self {
        match n {
            AMIGA_INTERPRETER_NUMBER => Self::Amiga,
            MACINTOSH_INTERPRETER_NUMBER => Self::Macintosh,
            ATARI_ST_INTERPRETER_NUMBER => Self::AtariSt,
            APPLE_IIGS_INTERPRETER_NUMBER => Self::AppleIIgs,
            _ => Self::IbmPc,
        }
    }

    /// The interpreter number the medium at `path` defaults to, or `None` when
    /// it is not release media.
    ///
    /// Content, not extension: [`blorb::medium::DiskImage::detect`] reads the
    /// filesystem, exactly as `PictSource::resolve` and `hints::read_story_file`
    /// do, so a disk image with any name (or none) is recognised and a mis-named
    /// ordinary story file is not. The MAPPING is `blorb`'s so that `zvm-cli`
    /// reaches the same conclusion off the same bytes (SQ-0839) — this only
    /// supplies the file.
    fn medium_interpreter_number(path: &Path) -> Option<u8> {
        Self::medium(path)?.interpreter_number()
    }

    /// The release medium at `path`, or `None` when it is not one. The single
    /// read this module does; see [`Self::medium_interpreter_number`] for why it
    /// is content and not extension.
    fn medium(path: &Path) -> Option<blorb::medium::DiskImage> {
        let raw = std::fs::read(path).ok()?;
        blorb::medium::DiskImage::detect(&raw)
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
            Self::Macintosh => Some(MACINTOSH_INTERPRETER_NUMBER),
            Self::AtariSt => Some(ATARI_ST_INTERPRETER_NUMBER),
            Self::AppleIIgs => Some(APPLE_IIGS_INTERPRETER_NUMBER),
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
    ///
    /// # The Macintosh has TWO screens, and the artwork picks
    ///
    /// This is the one machine whose answer is not a single pair, and the
    /// reason is that Infocom's own Macintosh interpreter sized its window and
    /// chose its picture file in **one decision**. `mac/xzip.lst`:
    ///
    /// ```text
    ///   IF ((ydisplay < 2*GFXAM_Y) OR (xdisplay < 2*GFXAM_X))
    ///     OR ((mColor = FALSE) OR (ttyToggle)) THEN
    ///     BEGIN  myBig := FALSE;  wy := GFXMAC_Y;  wx := GFXMAC_X;  END
    ///   ELSE
    ///     BEGIN  myBig := TRUE;   wy := 2*GFXAM_Y; wx := 2*GFXAM_X; END
    /// ```
    ///
    /// with `GFXAM_X = 320; GFXAM_Y = 200` ("raw" size of full-screen Amiga
    /// pics) and `GFXMAC_X = 480; GFXMAC_Y = 300` ("1.5 x Amiga sizes") — and
    /// the very same flag then names the file: *"for a small window use mono
    /// gfx, for a big window use color gfx"*, `IF myBig THEN gfxName :=
    /// 'CPic.Data' ELSE gfxName := 'Pic.Data'`.
    ///
    /// So a big colour Macintosh runs a **640×400** window off `CPic.data`
    /// (320×200 art at 2×), and a standard compact Mac runs a **480×300** window
    /// off `Pic.data` (480×300 monochrome art at 1× — `IF ge.mono OR myTiny THEN
    /// { scale 1x for display }`). The screen the game is told about is that
    /// window and nothing else:
    ///
    /// ```text
    ///   { calculate our logical screen size, based on window size }
    ///     WITH myWindow^.portRect DO
    ///       BEGIN
    ///       totRows := (bottom - top) {DIV lineheight};
    ///       totCols := ((right - left) - (2 * wMarg)) {DIV colWidth};
    ///       END;
    /// ```
    ///
    /// (the `DIV`s commented out, so both are in PIXELS, and the `2 * wMarg`
    /// takes the 4-pixel text inset back off — `totCols` is exactly `wx`.)
    ///
    /// **512×342 is the hardware, not the standard window.** The compact Mac's
    /// screen only ever appears here as `screenRect := screenBits.bounds`, the
    /// thing the window is centred *in*: `SizeWindow (myWindow, wx + (2*wMarg),
    /// wy, FALSE)` then `MoveWindow (myWindow, ((xdisplay-wx) DIV 2) - wMarg,
    /// (ydisplay-wy) DIV 2 + 17 {mbar fudge}, TRUE)`. On a 512×342 screen that
    /// is a 488×300 content rect at y = (342−300)/2 + 17 = 38 — which leaves
    /// exactly the 20-pixel menu bar and a 19-pixel title bar above it, and 4
    /// pixels below. A screenshot of a real Mac Zork Zero is therefore 512×342
    /// with the game filling nearly all of it, and the story is still being told
    /// its screen is 480×300.
    ///
    /// This profile answers with the big-colour pair, because that is the
    /// machine the disk's DEFAULT archive belongs to. The 480×300 screen is not
    /// this knob's to state: it arrives with the monochrome archive, from
    /// [`crate::graphics::PictSource::native_std_window`], exactly as it arrived
    /// with `Pic.Data` on the Mac. One decision there too.
    ///
    /// # The Atari ST has no answer, and that is a FACT about the machine
    ///
    /// [`Self::AtariSt`] returns `None`, and this is the one place in the bundle
    /// where a profile declines. It is not a gap awaiting a fixture. **Infocom
    /// never wrote a Version 6 interpreter for the Atari ST**: `st/` in
    /// `infocom-zcode-terps` holds a ZIP (`stzip.s`) and an XZIP (`stx*.s`,
    /// `xzip.c`) and no YZIP, where the repository lists one for the Apple and
    /// the Macintosh. A standard window is a Version 6 ART geometry, so there is
    /// no ST artwork for it to be the resolution *of* — which is the same fact
    /// the corpus reports from the other end, all thirty-nine stories across the
    /// nine ST compilations being v3, v4 or v5.
    ///
    /// The ST's own screen word is not this quantity in any case. `st/stx1.s`
    /// fills `PSCRWD` from the live text display rather than from a constant:
    ///
    /// ```text
    ///   st/stx1.s:615   MOVE.W  _columns,D1   * SIZE OF ATARI SCREEN DISPLAY (40 OR 80)
    ///   st/stx1.s:735   MOVE.W  _rows,D0
    ///   st/stx1.s:742   MOVE.W  D0,PSCRWD(A2) * SET SCREEN-PARAMETERS WORD
    /// ```
    ///
    /// — rows and columns of whatever display is attached, which is already what
    /// babelmap tells a story about its pane.
    ///
    /// # The Apple IIgs HAS an answer, and it is not this quantity
    ///
    /// [`Self::AppleIIgs`] also returns `None`, and the reason is the opposite of
    /// the Atari ST's (SQ-0857). Infocom wrote a Version 6 interpreter for this
    /// machine and it is in hand twice over — `apple/yzip/rel.15/` in
    /// `infocom-zcode-terps`, and its `MACHINE:` routine byte-for-byte on the two
    /// Version 6 ProDOS disks in `stories/`. Its screen is stated outright, in
    /// `apple/yzip/rel.15/apple.equ`:
    ///
    /// ```text
    ///   MAXWIDTH   EQU 140   ; 560 / 4 = max "pixels"
    ///   MAXHEIGHT  EQU 192   ; 192 screen lines
    ///   FONT_H     EQU 9     ; font height
    ///   MFONT_W    EQU 3     ; mono spaced font width, to game
    /// ```
    ///
    /// and `zboot.asm` hands exactly those to the story, `ZHWRD`/`ZVWRD` being
    /// header `$22`/`$24` and `ZSCRWD` the character grid at `$20`/`$21`:
    ///
    /// ```text
    ///   lda #MAXWIDTH      / sta ZBEGIN+ZHWRD+1     ; 140 pixels across
    ///   lda #MAXWIDTH/3    / sta ZBEGIN+ZSCRWD+1    ; 46 columns
    ///   lda #MAXHEIGHT     / sta ZBEGIN+ZVWRD+1     ; 192 pixels down
    ///   lda #MAXHEIGHT/FONT_H / sta ZBEGIN+ZSCRWD   ; 21 lines
    ///   lda #FONT_H        / sta ZBEGIN+ZFWRD       ; 9 tall
    ///   lda #3             / sta ZBEGIN+ZFWRD+1     ; 3 wide
    /// ```
    ///
    /// So the machine's Version 6 screen is **140x192 on a 3x9 cell**, 46x21
    /// characters — the 560-dot double hi-res display counted in four-dot colour
    /// pixels. That is a different screen MODEL, not a different resolution, and
    /// this knob cannot express it. What it holds is the art picture space that
    /// [`crate::session::V6_ART_SCALE`] doubles onto the 640x400 unit screen,
    /// which is then divided by [`Self::v6_font_cell`]'s fixed 8x16. Answering
    /// `Some((140, 192))` would tell the story its screen is 280x384 and 35x24
    /// characters — a machine that never existed, and further from the Apple's
    /// own 46x21 than the 80x25 it gets by declining. Honouring 140x192 honestly
    /// means making `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` runtime state, which is the
    /// same refactor this bundle already declined for the Macintosh's real 7x15
    /// cell and for EGA's 8x8 — see [`Self::v6_font_cell`].
    ///
    /// There is also nothing for it to size. Arthur's and Journey's pictures live
    /// inside the segmented `ARTHUR.D1`-`.D5` / `JOURNEY.D1`-`.D4` container,
    /// which has no reader (SQ-0852), and neither disk carries a separate archive
    /// — `MountedDisk::pictures()` answers `None` for both. If that reader
    /// arrives, the Apple's geometry becomes a live question and the cell is the
    /// thing that has to move first.
    pub fn std_window(self) -> Option<(u16, u16)> {
        match self {
            Self::IbmPc | Self::AtariSt | Self::AppleIIgs => None,
            Self::Amiga => Some(AMIGA_STD_WINDOW),
            Self::Macintosh => Some(MACINTOSH_STD_WINDOW),
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
            Self::Macintosh => Some((MAC_DEFAULT_BACKGROUND, MAC_DEFAULT_FOREGROUND)),
            Self::AtariSt => Some((ST_DEFAULT_BACKGROUND, ST_DEFAULT_FOREGROUND)),
            Self::AppleIIgs => Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        }
    }

    /// The palette the story's colour NUMBERS resolve through.
    ///
    /// The Macintosh resolves through [`zvm::screen::Palette::Standard`], and
    /// that is a reading rather than a default. `mac/xzip.lst`'s `MapColor`
    /// **is** ZMSD §8.3.1's table, one arm per colour and no others:
    ///
    /// ```text
    ///   CONST zBLACK = 2; zRED = 3; zGREEN = 4; zYELLOW = 5;
    ///         zBLUE = 6; zMAGENTA = 7; zCYAN = 8; zWHITE = 9;
    ///   CASE zcid OF
    ///     zBLACK: mcid := blackColor;   zRED:   mcid := redColor;   …
    ///     zWHITE: mcid := whiteColor;
    ///   OTHERWISE mcid := 0;  { "map Z id to Mac id, 0 if err/unchanged" }
    /// ```
    ///
    /// Those eight are QuickDraw's original saturated planar constants, so the
    /// Macintosh named the standard colours and meant them — where the Amiga
    /// loaded a palette of its own, which is why only it needs one here.
    ///
    /// **The Atari ST answers `Standard` for the same kind of reason, read the
    /// same way** (SQ-0835). `st/xzip.c`'s `color_table` is the ST's whole
    /// palette, one row per Z-machine colour id and no others, and the ids run
    /// 2..9 in ZMSD §8.3.1's order:
    ///
    /// ```text
    ///   WORD color_table[8*RGBLEN] =        /* XZIP ST color settings */
    ///       { 0x0000, 0x0000, 0x0000,       /* id 2 = black  (0.5 / 8) */
    ///         0x03A9, 0x003E, 0x003E,       /* id 3 = red */
    ///         0x003E, 0x032C, 0x003E,       /* id 4 = green */
    ///         0x03A9, 0x03A9, 0x003E,       /* id 5 = yellow */
    ///         0x003E, 0x003E, 0x03A9,       /* id 6 = blue */
    ///         0x03A9, 0x003E, 0x03A9,       /* id 7 = magenta */
    ///         0x003E, 0x03A9, 0x03A9,       /* id 8 = cyan */
    ///         0x03A9, 0x03A9, 0x03A9 };     /* id 9 = white  (7.5 / 8) */
    /// ```
    ///
    /// Those are GEM VDI intensities in the VDI's own 0–1000 range, and they are
    /// the saturated primaries: `0x03A9` is 937 and `0x003E` is 62, which the
    /// file's own comments gloss as 7.5/8 and 0.5/8. So the ST asked for §8.3.1's
    /// eight colours and meant them, and the file adds that the hardware rounded
    /// even that off — *"Realized settings are (currently) less detailed (8/8 and
    /// 0/8)"*. Nothing here is a palette of the ST's own in the sense the Amiga's
    /// is; the 512-colour hardware is not evidence, and is not consulted.
    ///
    /// What the ST could not do was show them all at once, and that is a display
    /// limit rather than a palette. `st/color.note` (5/26/87): *"The color
    /// function in the XZIP spec lists eight colors. The Atari ST, in 80-column
    /// mode, can display at most four of them at any one time"*, with only one
    /// background, always index 0. `st/xzip.c` recycles indices under an LRU to
    /// fake the rest. A terminal has no such ceiling, so there is nothing to
    /// express — this is noted so the absence reads as measured rather than
    /// missed.
    ///
    /// **The Apple IIgs answers `Standard` on the same reading, and this time the
    /// source proves itself** (SQ-0857). `apple/yzip/rel.15/tables.asm` carries
    /// the map and its inverse on consecutive lines:
    ///
    /// ```text
    ///   tables.asm:219  ZIPCOLOR: db 0,1,6,7,$C,$B,$E,$F
    ///   tables.asm:220  APLCOLOR: db 2,3,$FF,$FF,$FF,$FF,4,5,$FF,$FF,$FF,7,$FF,$FF,8,9
    /// ```
    ///
    /// `ZIPCOLOR` is indexed by Z-machine colour id zero-based from 2 —
    /// `machine.asm`'s `ZCOLOR` does `dex ; lda ZIPCOLOR,X` — so it is one entry
    /// per §8.3.1 colour, in §8.3.1's order, 2..9 and no others. `APLCOLOR` is
    /// the exact inverse, indexed by the Apple's own 16-colour double hi-res
    /// hardware value: 0 to 2, 1 to 3, 6 to 4, 7 to 5, $B to 7, $C to 6, $E to 8,
    /// $F to 9, and `$FF` — "no Z-machine colour" — for the other eight. A round
    /// trip that closes on exactly §8.3.1's eight is the machine saying it asked
    /// for the standard colours and took the nearest of the sixteen it had.
    ///
    /// So there is no palette of the Apple's own in the sense the Amiga's is, and
    /// the double hi-res hardware is not consulted: its RGB has no canonical
    /// values (it is an artefact of NTSC colour fringing and differs by monitor
    /// and by emulator), and nothing in Infocom's sources states any. Declining
    /// to invent one is the same call [`Self::AtariSt`] makes about the ST's
    /// 512-colour hardware, one paragraph up.
    pub fn palette(self) -> zvm::screen::Palette {
        match self {
            Self::IbmPc | Self::Macintosh | Self::AtariSt | Self::AppleIIgs => {
                zvm::screen::Palette::Standard
            }
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
    ///
    /// **The Macintosh is the case that genuinely differs, and it is not
    /// expressed** (SQ-0838, reported rather than quietly rounded). Infocom's
    /// Mac interpreter set `stdFont := geneva; textSize (12); lineHeight := 15
    /// {16}; colWidth := 7` — a 7×15 cell, giving 68×20 characters on the
    /// 480×300 standard-Mac screen where babelmap's 8×16 gives 60×18. Honouring
    /// it means making `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` runtime state throughout
    /// `zvm`'s screen model, its location heuristics and the app's render path,
    /// which is the same refactor this knob declined for EGA and a far larger
    /// change than the profile it would be arriving inside. The cost is a
    /// slightly larger typeface than a real Mac's and up to 4 pixels of vertical
    /// slack; see [`crate::graphics::PictSource::native_std_window`] for where
    /// that slack lands.
    ///
    /// **The Apple IIgs differs further still, and is reported the same way**
    /// (SQ-0857). Its Version 6 cell is 3x9 — `MFONT_W EQU 3` and `FONT_H EQU 9`
    /// in `apple/yzip/rel.15/apple.equ`, handed to the story as `ZFWRD` — giving
    /// 46x21 characters on the 140x192 screen where babelmap's 8x16 gives 80x25.
    /// It is the same refactor declined twice above, and declining it is what
    /// makes [`Self::std_window`] decline too: a 140x192 picture space is only
    /// the Apple's screen when read through the Apple's cell, and this knob is
    /// where that cell would have to live.
    pub fn v6_font_cell(self) -> (u16, u16) {
        (8, 16)
    }
}

/// Amiga, from the ZMSD §11.1.3 interpreter-number table. Defined by
/// [`blorb::medium`] and re-exported here: it is the number the medium→machine
/// mapping hands out, and `zvm-cli` hands out the same one (SQ-0839).
pub use blorb::medium::AMIGA_INTERPRETER_NUMBER;

/// Macintosh, from the same §11.1.3 table and the same place (SQ-0838). Read
/// from the standard, not recalled: *"1 DECSystem-20, 2 Apple IIe, **3
/// Macintosh**, 4 Amiga, 5 Atari ST, 6 IBM PC …"*.
pub use blorb::medium::MACINTOSH_INTERPRETER_NUMBER;

/// Atari ST, from the same §11.1.3 table and the same place (SQ-0835) — and the
/// only one of the three the machine's own interpreters corroborate directly,
/// `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`. [`blorb::medium`] quotes them, and
/// shows the byte is a flat constant rather than a version-dependent rule.
pub use blorb::medium::ATARI_ST_INTERPRETER_NUMBER;

/// Apple IIgs, from the same §11.1.3 table and the same place (SQ-0857) — and
/// the second of the four the machine's own interpreter corroborates directly,
/// `IIgsID EQU 10 ; ][gs Yzip`. [`blorb::medium`] quotes the Apple II YZIP in
/// full, and shows the byte is neither a flat constant (as the ST's is) nor a
/// version-dependent rule (as the IBM PC's is) but a **runtime machine
/// detection** across the family's three numbers — which is why that row's
/// argument is about which Apple II babelmap presents as, rather than about
/// which one pressed the disk.
pub use blorb::medium::APPLE_IIGS_INTERPRETER_NUMBER;

/// The Apple IIgs's default background: standard colour **2, black**.
///
/// Sourced from Infocom's own Version 6 interpreter for the machine, the Apple
/// II YZIP — `apple/yzip/rel.15/zboot.asm`, which seeds the header before the
/// story can read it. `ZCLRWD EQU 44` is decimal 44, `$2C`, and ZMSD §8.3.3 puts
/// the default BACKGROUND at `$2C` and the foreground at `$2D`:
///
/// ```text
///   apple/yzip/rel.15/zboot.asm:54   lda #9   ; the color white is the foreground color
///   apple/yzip/rel.15/zboot.asm:55   sta ZBEGIN+ZCLRWD+1   ; show Z game too
///   apple/yzip/rel.15/zboot.asm:56   lda #2   ; black is the background color
///   apple/yzip/rel.15/zboot.asm:57   sta ZBEGIN+ZCLRWD     ; tell game about it
/// ```
///
/// The comments name the colours outright, so this needs no decoding — and
/// `machine.asm`'s `ZCOLOR` says it twice more, the way the Amiga's, the Mac's
/// and the ST's do, because colour 1 means "the default" (ZMSD §8.3.1) and the
/// file has to resolve it:
///
/// ```text
///   ; just do the background color - foreground is always white/black
///   …
///   ldx #1   ; use black as default back color
///   …
///   ldx #8   ; use white as default fore color
/// ```
///
/// (Both are pre-`dex` indices into `ZIPCOLOR`, which is zero-based from colour
/// 2: `#1` resolves to index 0, colour 2, and `#8` to index 7, colour 9.)
///
/// **A genuinely BLACK page, and it is the only one here.** The Amiga's dark
/// grey (`$444`, see [`AMIGA_DEFAULT_BACKGROUND`]) is deliberately not black —
/// that distinction carried SQ-0740's window-0 gate — while the Macintosh and
/// the Atari ST both boot white. The Apple II's double hi-res display really is
/// unlit black with white text on it, so this profile is the darkest of the
/// four, and `honor_game_colours` is doing correspondingly real work: turning it
/// off returns the user's theme, as ever.
pub const APPLE_DEFAULT_BACKGROUND: u8 = 2;

/// The Apple IIgs's default foreground: standard colour 9, white. Same four
/// lines, same source — see [`APPLE_DEFAULT_BACKGROUND`].
pub const APPLE_DEFAULT_FOREGROUND: u8 = 9;

/// The Atari ST's default background: standard colour **9**, white.
///
/// Sourced exactly as the Macintosh's was, and it is the same `(back << 8) |
/// fore` idiom a third time — `st/xzip.c`, the interface half of Infocom's ST
/// XZIP, whose modification history ends "17 Sep 87 dbb … FROZEN Version A":
///
/// ```text
///   #define USE_DEF  1     /* use default ST color */
///   #define DEF_FORE 2     /* default ST foreground id = black */
///   #define DEF_BACK 9     /* default ST background id = white */
/// ```
///
/// The comments name the colours outright, so this needs no decoding, and
/// `_op_color` says it twice more the way the Amiga's and the Mac's do — colour
/// 1 means "the default" (ZMSD §8.3.1), so the file resolves it:
///
/// ```text
///   if (id1 == USE_DEF) id1 = DEF_FORE;
///   if (id2 == USE_DEF) id2 = DEF_BACK;
///   …
///   return ((DEF_BACK << 8) | DEF_FORE);   /* (used by 68K init) */
/// ```
///
/// **XZIP is the right interpreter to have read**: it is Infocom's Version 5
/// interpreter, so this is the program that actually painted *Beyond Zork* on an
/// Atari ST — the one story in the ST corpus whose behaviour this profile moves.
/// Its "Version A" is corroborated from inside the game, which answers VERSION
/// with "Atari ST Color Version A" once it is told 5.
///
/// A white page, the same as the Macintosh's and the opposite of the Amiga's
/// dark grey, so `honor_game_colours` is doing real work here too: turning it
/// off returns the user's theme, as ever.
pub const ST_DEFAULT_BACKGROUND: u8 = 9;

/// The Atari ST's default foreground: standard colour 2, black. Same file, same
/// three lines — see [`ST_DEFAULT_BACKGROUND`].
pub const ST_DEFAULT_FOREGROUND: u8 = 2;

/// The Macintosh Version 6 standard window: the **big colour** Mac's, 320×200
/// art doubled onto a 640×400 window — the same numbers as the Amiga's, and
/// from the same place. `mac/xzip.lst` sizes that window as `wy := 2*GFXAM_Y;
/// wx := 2*GFXAM_X` off `GFXAM_X = 320; GFXAM_Y = 200; { "raw" size of
/// full-screen Amiga pics }`, which says outright that the Macintosh's colour
/// artwork *is* the Amiga's picture space.
///
/// The standard Macintosh's other window — 480×300, `GFXMAC_X`/`GFXMAC_Y`,
/// "1.5 x Amiga sizes" — belongs to the monochrome archive and arrives with it.
/// See [`InterpreterProfile::std_window`], which quotes the whole decision.
pub const MACINTOSH_STD_WINDOW: (u16, u16) = (320, 200);

/// The Macintosh's default background: standard colour **9**, white.
///
/// One line of `mac/xzip.lst` settles both this and [`MAC_DEFAULT_FOREGROUND`],
/// and it is the same `(back << 8) | fore` idiom the Amiga interpreter uses
/// (see [`AMIGA_DEFAULT_BACKGROUND`], where the sources had to be weighed):
///
/// ```text
///   FUNCTION SetColor (fore, back: INTEGER): INTEGER;  { return back/fore defaults }
///   BEGIN
///     SetColor := (zWHITE*256) + zBLACK;   { Mac defaults: white under black }
/// ```
///
/// with `zWHITE = 9` and `zBLACK = 2` declared eleven lines above it. The same
/// function's fallbacks say it twice more — `IF fore = 1 THEN mcid :=
/// blackColor { default }` and `IF back = 1 THEN mcid := whiteColor` — because
/// colour 1 means "the default" (ZMSD §8.3.1), so asking for the default
/// foreground gets black and the default background white.
///
/// A white page is the Macintosh's whole visual signature and the opposite of
/// the Amiga's dark grey, so `honor_game_colours` is doing real work on this
/// profile: turning it off returns the user's theme, as ever.
pub const MAC_DEFAULT_BACKGROUND: u8 = 9;

/// The Macintosh's default foreground: standard colour 2, black. Same line,
/// same source — see [`MAC_DEFAULT_BACKGROUND`].
pub const MAC_DEFAULT_FOREGROUND: u8 = 2;

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
    fn macintosh_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own Macintosh interpreter (`mac/xzip.lst`, `mac/gfx.p`).
        let p = InterpreterProfile::Macintosh;
        assert_eq!(p.interpreter_number(), Some(3), "ZMSD §11.1.3: 3 = Macintosh");
        assert_eq!(p.std_window(), Some((320, 200)), "wx := 2*GFXAM_X, wy := 2*GFXAM_Y");
        assert_eq!(
            p.default_colours(),
            Some((9, 2)),
            "SetColor := (zWHITE*256) + zBLACK — 'Mac defaults: white under black'",
        );
        // And the page really is the LIGHT one, which is the whole visual
        // difference from the Amiga's dark grey — asserted as a relation rather
        // than a repeat of the pair, so a swapped tuple cannot pass.
        let (mac_bg, _) = p.default_colours().expect("the Mac states its defaults");
        let (amiga_bg, _) =
            InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(mac_bg, amiga_bg, "white page against dark grey");
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "MapColor IS §8.3.1's table");
    }

    #[test]
    fn atari_st_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own ST interpreters (`st/stx1.s`, `st/stzip.s`,
        // `st/xzip.c`, `st/color.note`).
        let p = InterpreterProfile::AtariSt;
        assert_eq!(p.interpreter_number(), Some(5), "ZMSD §11.1.3: 5 = Atari ST");
        assert_eq!(
            p.default_colours(),
            Some((9, 2)),
            "st/xzip.c: DEF_BACK 9 = white, DEF_FORE 2 = black",
        );
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "color_table IS §8.3.1's eight");

        // **The declined member, asserted as declined.** The ST never had a
        // YZIP, so it has no Version 6 art geometry — this is the one knob in
        // the bundle a profile other than the IBM PC answers `None` to, and it
        // must stay `None` rather than drift to a plausible-looking pair.
        assert_eq!(
            p.std_window(),
            None,
            "Infocom wrote no Version 6 interpreter for the Atari ST — there is no ST art to size",
        );

        // The ST and the Macintosh agree on the page and disagree with the
        // Amiga. Asserted as relations so a copied-and-edited tuple cannot pass.
        let (st_bg, st_fg) = p.default_colours().expect("the ST states its defaults");
        let (mac_bg, mac_fg) =
            InterpreterProfile::Macintosh.default_colours().expect("so does the Mac");
        assert_eq!((st_bg, st_fg), (mac_bg, mac_fg), "both machines default to black on white");
        let (amiga_bg, _) = InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(st_bg, amiga_bg, "white page against the Amiga's dark grey");
    }

    #[test]
    fn apple_iigs_knobs_are_the_verified_constants() {
        // Every one of these is quoted at its constant, from ZMSD §11.1.3 and
        // from Infocom's own Apple II YZIP (`apple/yzip/rel.15/apple.equ`,
        // `zboot.asm`, `machine.asm`, `tables.asm`, `bsubs.asm`).
        let p = InterpreterProfile::AppleIIgs;
        assert_eq!(p.interpreter_number(), Some(10), "ZMSD §11.1.3: 10 = Apple IIgs");
        assert_eq!(
            p.default_colours(),
            Some((2, 9)),
            "zboot.asm: `lda #2 ; black is the background color`, `lda #9 ; the color white is \
             the foreground color` — and $2C is the BACKGROUND (ZMSD §8.3.3)",
        );
        assert_eq!(p.palette(), zvm::screen::Palette::Standard, "ZIPCOLOR IS §8.3.1's eight");

        // **The declined member, asserted as declined** — and declined for a
        // different reason to the ST's, which is the whole point of having both.
        // The Apple's Version 6 screen exists and is 140x192 on a 3x9 cell; this
        // knob holds a picture space that gets doubled onto 640x400 and cut into
        // 8x16 cells, so 140x192 would state a 280x384 / 35x24 machine that never
        // existed. See `std_window`'s docs.
        assert_eq!(
            p.std_window(),
            None,
            "the Apple's 140x192 on a 3x9 cell is a different screen model, not a std window",
        );
        assert_ne!(
            p.std_window(),
            Some((140, 192)),
            "and specifically NOT the Apple's own screen numbers — this knob cannot hold them",
        );

        // **The page is genuinely black, and it is the only one that is.**
        // Asserted as relations so a copied-and-edited tuple cannot pass: the
        // Apple is the dark one against the Mac's and the ST's white, and it is
        // darker than the Amiga's dark grey rather than equal to it (SQ-0740's
        // window-0 gate turns on the Amiga NOT being black).
        let (apple_bg, apple_fg) = p.default_colours().expect("the Apple states its defaults");
        let (mac_bg, _) = InterpreterProfile::Macintosh.default_colours().expect("so does the Mac");
        let (st_bg, _) = InterpreterProfile::AtariSt.default_colours().expect("so does the ST");
        let (amiga_bg, amiga_fg) =
            InterpreterProfile::Amiga.default_colours().expect("so does the Amiga");
        assert_ne!(apple_bg, mac_bg, "black page against the Macintosh's white");
        assert_ne!(apple_bg, st_bg, "black page against the Atari ST's white");
        assert_ne!(apple_bg, amiga_bg, "black page against the Amiga's dark grey — 2 is not 12");
        // …and white ink, which the Amiga agrees on and the other two do not.
        assert_eq!(apple_fg, amiga_fg, "both machines write white on a dark page");
    }

    /// The Apple's number is a **runtime detection**, where the ST's is a flat
    /// constant and the IBM PC's is a version rule — three different shapes, and
    /// the reason this row was argued rather than read (SQ-0857).
    ///
    /// What makes 10 the honest answer is not that the medium names it: it is
    /// that DECLINING names something else. zvm's own rule would hand an Apple II
    /// story 1 or, on Version 6, 6 — the DECSystem-20 or the IBM PC. This pins
    /// the fallback that would apply, so the argument cannot rot into "None is
    /// harmless" without a test noticing.
    #[test]
    fn declining_the_apples_number_would_name_a_different_machine_entirely() {
        assert_eq!(
            InterpreterProfile::AppleIIgs.interpreter_number(),
            Some(APPLE_IIGS_INTERPRETER_NUMBER),
            "apple.equ: `IIgsID EQU 10 ; ][gs Yzip`",
        );
        // The profile a `None` would fall through to, and the numbers zvm's own
        // Frotz rule then applies — 6 for Version 6, 1 otherwise. 6 is the IBM
        // PC, which is also the value `zvm`'s `exec.rs` gates its CP437 remap on.
        let fallback = InterpreterProfile::IbmPc;
        assert_eq!(fallback.interpreter_number(), None);
        assert_eq!(
            InterpreterProfile::for_interpreter_number(6),
            InterpreterProfile::IbmPc,
            "a declined ProDOS row would land a Version 6 Apple story on the IBM PC",
        );
        assert_eq!(
            InterpreterProfile::for_interpreter_number(1),
            InterpreterProfile::IbmPc,
            "…and every other Apple story on the DECSystem-20's number",
        );
    }

    /// The ST's number is a **flat constant**, where the IBM PC's is a rule —
    /// which is the whole reason one row in `blorb::medium::FORMATS` answers
    /// `Some` and its filesystem-identical neighbour answers `None`.
    #[test]
    fn the_atari_st_states_one_number_where_the_ibm_pc_states_a_rule() {
        assert_eq!(
            InterpreterProfile::AtariSt.interpreter_number(),
            Some(ATARI_ST_INTERPRETER_NUMBER),
            "st/stx1.s: INTWRD DC.B 5 — no version arm, no condition",
        );
        assert_eq!(
            InterpreterProfile::IbmPc.interpreter_number(),
            None,
            "the IBM PC's honest number is version-dependent, so zvm's own rule stays in force",
        );
    }

    #[test]
    fn the_v6_cell_matches_what_zvm_advertises() {
        // Knob 6: stated for completeness, pinned so it cannot silently drift.
        // The Macintosh's real cell is 7×15 (Geneva 12) and is deliberately NOT
        // expressed — see `v6_font_cell`'s docs.
        for p in [
            InterpreterProfile::IbmPc,
            InterpreterProfile::Amiga,
            InterpreterProfile::Macintosh,
            InterpreterProfile::AtariSt,
            InterpreterProfile::AppleIIgs,
        ] {
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
        assert_eq!(InterpreterProfile::for_interpreter_number(3), InterpreterProfile::Macintosh);
        assert_eq!(InterpreterProfile::for_interpreter_number(5), InterpreterProfile::AtariSt);
        assert_eq!(InterpreterProfile::for_interpreter_number(10), InterpreterProfile::AppleIIgs);
        // Every other number is served by the IBM PC bundle, the historical
        // default. **2 and 9 are in this list on purpose**: they are the Apple
        // IIe and IIc, the other two machines the Apple II YZIP runs on, and
        // SQ-0857 scoped itself to the IIgs. Asking for one of them today gets
        // the IBM PC bundle with that number in `$1E` — the same "no opinion"
        // every unmapped number gets, and stated here so the gap is visible
        // rather than assumed closed.
        for n in [1u8, 2, 6, 7, 8, 9, 11] {
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

    /// The one ambiguity a codec cannot settle, settled by the disk under it
    /// (SQ-0843). `Flavour::AmigaMac` is one container written by two machines,
    /// so on release media the medium answers; `Flavour::Pc` is unambiguous and
    /// still beats the medium outright, which is what keeps naming an archive an
    /// instruction rather than a hint.
    #[test]
    fn the_medium_settles_which_machine_an_amiga_mac_archive_belongs_to() {
        use blorb::medium::DiskImage;
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, Some(DiskImage::Hfs)),
            InterpreterProfile::Macintosh,
            "an Amiga/Mac archive on an Apple filesystem is the Macintosh's",
        );
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, Some(DiskImage::Adf)),
            InterpreterProfile::Amiga,
        );
        // No medium: the archive's own answer, exactly as before.
        assert_eq!(
            InterpreterProfile::for_art_flavour_on(Flavour::AmigaMac, None),
            InterpreterProfile::Amiga,
        );
        // An unambiguous flavour is never refined — an `.mg1` named on a
        // Macintosh disk asks for the IBM PC and gets it.
        for medium in [None, Some(DiskImage::Hfs), Some(DiskImage::Adf)] {
            assert_eq!(
                InterpreterProfile::for_art_flavour_on(Flavour::Pc, medium),
                InterpreterProfile::IbmPc,
                "{medium:?}",
            );
        }
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
