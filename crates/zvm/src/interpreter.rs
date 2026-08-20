//! The machine table: what each ZMSD §11.1.3 interpreter number *is* (SQ-0872).
//!
//! Header byte `$1E` names a machine, and a story that reads it expects the rest
//! of the machine to match — its default page and ink (`$2C`/`$2D`, §8.3.3), the
//! palette its colour numbers resolve through (§8.3.1.1), and the screen rules
//! §8.3 gives that machine by name. Setting the byte alone produces a machine
//! that says one thing and looks like another, which is exactly what `zvm-cli`
//! did until this module existed: it told *Beyond Zork* off a ProDOS disk that it
//! was an Apple IIgs while the header still carried zvm's own §8.3.2 seed.
//!
//! # Why the table lives here, and what deliberately does not
//!
//! This is zvm's domain already. The screen model has carried a per-machine rule
//! for one machine — the Amiga's global colour pens — since SQ-0740, expressed as
//! a special case beside a lone `AMIGA_INTERPRETER_NUMBER`. A second machine
//! arrived (the Macintosh, SQ-0846) and landed in `app::session` instead, because
//! that is where the profile was; one concept, two crates, and `zvm-cli` able to
//! see only one of them. Making the special case a table fixes both directions at
//! once: `zvm` has an empty `[dependencies]` and both front-ends already depend
//! on it, so the CLI gets the whole bundle with no new crate anywhere.
//!
//! **The table is keyed by the interpreter NUMBER**, not by an enum, and that is
//! the same choice `blorb::medium` made for the medium→machine question. A number
//! is a compact, published, stable encoding that needs no shared type, which is
//! what lets three crates that must not depend on each other talk about the same
//! machine. `blorb` says which machine a DISK implies; this says what that machine
//! IS; `app::interpreter` is the only place that maps a path to either, because
//! reading a file is I/O policy and zvm has none.
//!
//! So what stays in `app::interpreter` stays there for a reason:
//!
//! - `InterpreterProfile::resolve` reads the medium off disk — I/O policy, against
//!   zvm's charter.
//! - the art-flavour preference needs `blorb::infocom_pics::Flavour`, and zvm takes
//!   zero external dependencies.
//! - `std_window` is a Version 6 picture space stated by an ARCHIVE rather than by
//!   the machine, which is why `AppleIIgs`'s is `None` (SQ-0857) while its number,
//!   page and palette are all right here.
//!
//! # Sourcing
//!
//! Every value below is quoted at its constant, from ZMSD §11.1.3 and from
//! Infocom's own interpreters in `github.com/erkyrath/infocom-zcode-terps`. The
//! Atari ST row is the standard the rest are held to: it sources its 5 from the
//! standard *and* from `st/stx1.s`'s `INTWRD DC.B 5 * MACHINE ID FOR ATARI ST`.
//! Where a value cannot be sourced it is declined outright — [`MACHINES`] carries
//! only what is known, and [`machine`] answers `None` for a number no row states,
//! so a front-end can say "I do not model that machine" instead of quietly
//! dressing it as an IBM PC.

use crate::memory::Memory;
use crate::screen::{Palette, ZColour};

// ---------------------------------------------------------------------------
// §11.1.3 interpreter numbers
// ---------------------------------------------------------------------------

/// Apple IIe, from the ZMSD §11.1.3 interpreter-number table: *"1 DECSystem-20,
/// **2 Apple IIe**, 3 Macintosh, 4 Amiga, 5 Atari ST, 6 IBM PC …"* — and
/// corroborated by Infocom's own Apple II YZIP, which picks its own identity at
/// boot. `apple/yzip/rel.15/bsubs.asm`'s `MACHINE:` routine chooses between the
/// family's three:
///
/// ```text
///   IIeID  EQU 2   ; ][e Yzip
///   IIcID  EQU 9   ; ][c Yzip
///   IIgsID EQU 10  ; ][gs Yzip
/// ```
///
/// The three machines share the entire Apple bundle and differ only in this byte
/// (SQ-0857 established that and scoped itself to the IIgs; SQ-0872 filled the
/// other two in). Which one a ProDOS *disk* implies is `blorb::medium`'s question
/// and it still answers 10 — the medium cannot name the press, and of the three
/// the one a modern terminal resembles is the IIgs. This constant exists so that
/// a player who names 2 outright gets the Apple, not the IBM PC.
pub const APPLE_IIE_INTERPRETER_NUMBER: u8 = 2;

/// Macintosh, from the same §11.1.3 table (SQ-0838). Read from the standard, not
/// recalled: *"1 DECSystem-20, 2 Apple IIe, **3 Macintosh**, 4 Amiga, 5 Atari ST,
/// 6 IBM PC …"*.
pub const MACINTOSH_INTERPRETER_NUMBER: u8 = 3;

/// Amiga, from the same §11.1.3 table. This is the byte §8.3's own Amiga rule
/// tests the header against — see [`MachineProfile::global_colour_pens`].
pub const AMIGA_INTERPRETER_NUMBER: u8 = 4;

/// Atari ST, from the same §11.1.3 table (SQ-0835) — and the first of the rows
/// the machine's own interpreters corroborate directly, `st/stx1.s`:
///
/// ```text
///   INTWRD  DC.B 5   * MACHINE ID FOR ATARI ST
/// ```
///
/// No version arm, no condition: where the IBM PC's honest number is a rule
/// ([`crate::screen::default_interpreter_number`]) the ST's is a flat constant.
pub const ATARI_ST_INTERPRETER_NUMBER: u8 = 5;

/// IBM PC, from the same §11.1.3 table. The machine whose number is a **rule**
/// rather than a constant — [`crate::screen::default_interpreter_number`] is
/// Frotz's 6-for-Version-6, 1-otherwise, and it is the reason
/// [`MachineProfile::default_colours`] is `None` on this row: an IBM PC in a
/// terminal is the player's terminal, so "default" should mean what the player
/// actually sees.
pub const IBM_PC_INTERPRETER_NUMBER: u8 = 6;

/// The IBM PC's default page: §8.3.1's **blue** (SQ-0928).
///
/// Observed rather than read out of Infocom's source, and the observation is
/// unusually well supported. Three DOS captures — *Shogun* r322 at its menu,
/// *Arthur* r74 mid-game, *Zork Zero* at the Banquet Hall — plus the report that
/// Zork Zero's BOOT sequence is blue/white before the game runs, i.e. before it
/// can have set anything. Against that, a trace of every `@set_colour` each game
/// issues: Arthur (941 screen ops) and Journey (532) name no colour at all and
/// are blue; Shogun names one, on a 548x32 status strip, and is blue everywhere
/// else; Zork Zero names one on a window the size of the screen and is white.
/// Four games, one rule, no exceptions.
///
/// **What is NOT being claimed is a shade.** These are §8.3.1 colour NUMBERS and
/// they resolve through the palette and the player's theme, so unlike the period
/// look ([`PeriodLook`]) there is no emulator-dependent RGB here to be wrong about.
///
/// **And stating it is not the same as applying it.** The gate is the app's
/// `Config::machine_default_colours`: this pair belongs to the IBM PC as a machine
/// a medium named, not to the fallback every story with no medium falls through to.
pub const IBM_PC_DEFAULT_BACKGROUND: u8 = 6;

/// The IBM PC's default ink: §8.3.1's **white**. See [`IBM_PC_DEFAULT_BACKGROUND`].
pub const IBM_PC_DEFAULT_FOREGROUND: u8 = 9;

/// Commodore 128, from the same §11.1.3 table (SQ-0869)/// Commodore 128, from the same §11.1.3 table (SQ-0869) — the one row
/// corroborated by a DISK rather than by an interpreter source tree.
/// `TRINITY1.D64` opens with the Commodore 128's `CBM` autoboot signature and
/// boots an interpreter that touches the C128's own MMU register `$FF00` forty
/// times, which no Commodore 64 has. `blorb::medium` shows the evidence and
/// argues why the `.d64` row answers 7 where the family's other number is 8.
pub const COMMODORE_128_INTERPRETER_NUMBER: u8 = 7;

/// Apple IIc, from the same §11.1.3 table and the same `MACHINE:` routine as
/// §11.1.3's Commodore 64.
///
/// No medium selects it — a `.d64` is a 1541 image both Commodore machines read —
/// so it is reached by asking for it, exactly as the Apple IIe and IIc are on a
/// ProDOS volume that cannot name which of the family pressed it.
pub const COMMODORE_64_INTERPRETER_NUMBER: u8 = 8;

/// [`APPLE_IIE_INTERPRETER_NUMBER`] — `IIcID EQU 9 ; ][c Yzip`.
pub const APPLE_IIC_INTERPRETER_NUMBER: u8 = 9;

/// Apple IIgs, from the same §11.1.3 table (SQ-0857) — and the second of the
/// rows the machine's own interpreter corroborates directly, `IIgsID EQU 10 ;
/// ][gs Yzip`. `blorb::medium` quotes the Apple II YZIP in full and shows the
/// byte is neither a flat constant (as the ST's is) nor a version rule (as the
/// IBM PC's is) but a **runtime machine detection** across the family's three
/// numbers.
pub const APPLE_IIGS_INTERPRETER_NUMBER: u8 = 10;

// ---------------------------------------------------------------------------
// §8.3.3 default colour pairs
// ---------------------------------------------------------------------------

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
/// neither. `app::colors` resolves 12 through the Amiga palette to `(66,66,66)`
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
pub const ST_DEFAULT_BACKGROUND: u8 = 9;

/// The Atari ST's default foreground: standard colour 2, black. Same file, same
/// three lines — see [`ST_DEFAULT_BACKGROUND`].
pub const ST_DEFAULT_FOREGROUND: u8 = 2;

/// The Apple II family's default background: standard colour **2, black**.
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
/// **One page for three machines.** `zboot.asm` is the whole family's boot path —
/// the `MACHINE:` routine picks the IIe's 2, the IIc's 9 or the IIgs's 10 *after*
/// these four lines run — so the IIe and IIc rows state this same pair from the
/// same source rather than from an analogy (SQ-0872).
///
/// **A genuinely BLACK page, and it is the only one here.** The Amiga's dark
/// grey (`$444`, see [`AMIGA_DEFAULT_BACKGROUND`]) is deliberately not black —
/// that distinction carried SQ-0740's window-0 gate — while the Macintosh and
/// the Atari ST both boot white.
pub const APPLE_DEFAULT_BACKGROUND: u8 = 2;

/// The Apple II family's default foreground: standard colour 9, white. Same four
/// lines, same source — see [`APPLE_DEFAULT_BACKGROUND`].
pub const APPLE_DEFAULT_FOREGROUND: u8 = 9;

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// Everything zvm knows about one ZMSD §11.1.3 machine.
///
/// The shape a machine's own interpreter drew its input cursor as (SQ-0873).
///
/// Three forms across five machines, and **none of them is what "invert the cell
/// under the cursor" gives** — which is what a terminal front-end draws by default
/// and what lanthorn drew before this. Measured off the captures in
/// `machine-screenshots/`, cell sizes included, so the proportions are the
/// machine's rather than a guess at them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// One pixel wide and a line tall, in the gap AFTER the last glyph rather than
    /// over a cell — the Macintosh insertion caret.
    ///
    /// Measured on three captures across two Mac models: `mac-zork1.jpg` (Mac Plus,
    /// 16 rows) and `mac-arthur.jpg` / `mac-zorkzero.jpg` (colour, 13 rows). A scan
    /// for isolated tall thin runs finds exactly one candidate per frame.
    Bar,
    /// A solid block filling the cell, or nearly: 7x14 of a 7x16 cell on the Apple
    /// II (`appleiie-planetfall.png`), a full 8x16 on the Amiga
    /// (`amiga-spellbreaker.png`, `amiga-lurking.png`).
    Block,
    /// One scanline on the cell's bottom row — 8x1 on both Commodores
    /// (`c64-hitchhiker.png`, `c128-trinity.png`).
    Underscore,
}

/// How a machine's own interpreter set the status line apart from the story
/// (SQ-0873).
///
/// **Five machines, four behaviours, and not one of them derives from the body
/// pair** — which is the finding that shaped [`PeriodLook`]. A field carrying only
/// a page and an ink could express none of the last three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBand {
    /// The body pair swapped, across the full width. The Apple II and the
    /// Commodore 128.
    FullReverse,
    /// The body pair swapped, but only behind each RUN of text — the page shows
    /// between them.
    ///
    /// The Amiga, and the reason this variant exists. Row-censused across
    /// `amiga-spellbreaker.png` at mid-band: 47..182 white behind "Council
    /// Chamber", **376 px of page**, then 559..680 white behind "Score: 0/0".
    /// `dos-hitchhiker.png` is the same shape: 611 contiguous px of page run
    /// through the middle of its status row.
    ///
    /// **No row uses it, by the user's ruling** — both machines that measure this
    /// way draw a full-width reverse instead, because a band broken into pieces
    /// reads as damage in a terminal where it read as design on the original
    /// monitor. Kept for the same reason [`Self::Own`] is: the measurement is real
    /// and does not stop being real because we chose not to draw it.
    PerRun,
    /// A ground and ink of its own, which are neither the body pair nor its
    /// reverse.
    ///
    /// The Commodore 64: a black ground under grey characters where its body is
    /// white on grey. A true reverse would be a white ground with grey characters,
    /// and black is a colour the body never uses as a ground at all.
    Own {
        /// The band's ground.
        ground: (u8, u8, u8),
        /// The band's characters.
        ink: (u8, u8, u8),
    },
    /// No distinguishing ground; the band is separated from the story by rules.
    ///
    /// The Macintosh, which draws the status line in the body's own white-under-
    /// black and puts solid black rules above and below it.
    Ruled,
}

/// What a machine's screen LOOKED LIKE, for a story that has no opinion about it.
///
/// # Why this is a separate field and not more of `default_colours`
///
/// **Colour arrives with Version 5.** `set_colour` and the `$2C`/`$2D` header bytes
/// are v5+, so a v1-v4 story has no colour concept at all: it never sets, never
/// reads, never branches. Anything shown for one is the interpreter's presentation.
/// [`MachineProfile::default_colours`] is the opposite kind of claim — a fact the
/// story can read, sourced from Infocom's own code (`mac/xzip.lst`, `zboot.asm`,
/// `st/stx1.s`) — and the two must not be confused. Hence two fields.
///
/// **And the two genuinely differ.** The Amiga's row reports 12/9, a grey page under
/// white ink, from its v5-era interpreter; its v3 interpreter draws a BLUE page. The
/// Commodore 64's grey page cannot be expressed as a §8.3.1 colour number at all
/// (2..9 contain no grey, and `clamp_default_colour` accepts 10..12 only for v6), so
/// reusing `default_colours` would have silently clamped it to black. Two of five.
///
/// # Provenance, and the standard it does NOT meet
///
/// Every value here is **observed from an emulator capture in
/// `machine-screenshots/`**, row-censused rather than sampled, not read out of
/// Infocom's source. The rest of this table meets a documented-source standard and
/// this field does not, by the user's explicit decision (SQ-0873) to go with the
/// captures in hand rather than chase emulator names, versions and palette settings.
///
/// What that costs is bounded and worth stating: the Amiga's `#074BA1` and the
/// Commodore 64's `#6C6C6C` are values a palette choice can move — `#074BA1` is not
/// a bit-replicated OCS value (those widen to `0x00, 0x11, ... 0xFF`), so the
/// underlying register is almost certainly Workbench's `$05A`, which replicates to
/// `#0055AA`; what is recorded is what the emulator drew. The other rows cannot
/// move: the Mac Plus is 1-bit, the C128's VDC is RGBI (0/85/170/255 exactly), and
/// the Apple II is monochrome — though "monochrome" there means a WHITE-phosphor
/// rendering, and green and amber monitors were as common.
///
/// # The Commodore 64 is the row this field argued into existence
///
/// Interpreter 8 had no row when it was first measured; see [`MACHINES`] for why
/// that was wrong. Its values come from `c64-zork1-solidgold.png`, whose banner
/// reads "Interpreter 8 Version J" — the machine naming its own number.
///
/// **Two C64 captures disagree, and the row follows the later one.**
/// `c64-hitchhiker.png` (1984) is a GREY page `#6C6C6C` under white ink with a
/// status band of `#6C6C6C` on `#000000` — neither the body pair nor its reverse,
/// which is the case [`StatusBand::Own`] exists for and the only measured instance
/// of it. The Solid Gold press (release 52, serial 871125) is black under white
/// with a plain full-width reverse. One machine, one publisher, two looks three
/// years apart: the period look is a property of the interpreter BUILD as much as
/// the machine, and this table has one row per machine. `Own` is kept because the
/// earlier build is evidence that a band need not be derivable, whether or not the
/// row that reported it still does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodLook {
    /// The body's ground.
    pub page: (u8, u8, u8),
    /// The body's characters.
    pub ink: (u8, u8, u8),
    /// How the status line was set apart. Never derivable from the pair above.
    pub status: StatusBand,
    /// The input cursor's shape.
    pub cursor_shape: CursorShape,
    /// The input cursor's colour, which on two of five machines is **neither the
    /// page nor the ink** — the Amiga's orange and the Commodore 64's black.
    pub cursor_colour: (u8, u8, u8),
}

/// The Apple II family's period look, shared by interpreters 2, 9 and 10.
///
/// Measured on `appleiie-planetfall.png` (Planetfall r29, v3, 560x384 = 80 columns
/// at 1x2). Shared across the three rows for the same reason they already share
/// [`APPLE_DEFAULT_BACKGROUND`] and [`APPLE_DEFAULT_FOREGROUND`]: one Infocom
/// interpreter, one 80-column text screen. The capture cannot say which of the
/// three machines ran it, and treating that as three separate declines would lose a
/// measurement to a distinction the interpreter itself does not draw.
///
/// White on black is the white-monitor rendering; see [`PeriodLook`].
const APPLE_PERIOD_LOOK: PeriodLook = PeriodLook {
    page: (0x00, 0x00, 0x00),
    ink: (0xFF, 0xFF, 0xFF),
    status: StatusBand::FullReverse,
    cursor_shape: CursorShape::Block,
    cursor_colour: (0xFF, 0xFF, 0xFF),
};

/// A row is the machine's *bundle*: the byte it writes into `$1E`, the page and
/// ink it reports in `$2C`/`$2D`, the palette its colour numbers resolve through,
/// and the §8.3 screen rules the standard gives it by name. Any member a row
/// cannot source is declined (`None` / `false`) rather than guessed — see the
/// module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachineProfile {
    /// The §11.1.3 number this machine writes into header `$1E`.
    pub number: u8,
    /// The machine's name, as §11.1.3 spells it. Diagnostic (`zvm-cli` names the
    /// machine it is presenting as); never shown to the story.
    pub name: &'static str,
    /// The `(background, foreground)` standard colour numbers this machine
    /// reports in header `$2C`/`$2D` (ZMSD §8.3.3), or `None` to report the host
    /// terminal's own colours.
    ///
    /// `None` is a *decline*, and it means two different things on two rows. The
    /// IBM PC declines because a PC in a terminal IS the player's terminal, so
    /// "default" should mean what the player sees. The Commodore 128 declines
    /// because nothing in hand states the pair Infocom's Commodore interpreter
    /// reported for a v5 story (SQ-0869).
    ///
    /// That second decline stands, but its old wording no longer does. It used to
    /// read that "the machine's famous light-blue-on-blue boot screen is the
    /// hardware's reputation rather than the interpreter's evidence"; SQ-0873 then
    /// measured the machine, and Infocom's interpreter draws light CYAN on BLACK,
    /// which is neither the reputation nor anything like it. What that measurement
    /// cannot do is fill this field, because it was taken on *Trinity* — a v4
    /// story, which has no colour concept for a default pair to be the default OF.
    /// It lives in [`MachineProfile::period_look`] instead, which is the whole
    /// point of that field being separate.
    pub default_colours: Option<(u8, u8)>,
    /// The palette this machine's colour NUMBERS resolve to true colours through
    /// (ZMSD §8.3.1.1, which makes it an interpreter choice rather than a law).
    ///
    /// Only the Amiga loaded a palette of its own. The Macintosh, the Atari ST
    /// and the Apple II each asked for §8.3.1's eight colours and meant them —
    /// the readings are quoted on [`Palette`] and in `app::interpreter`. Where a
    /// machine's hardware palette is famous but its interpreter never states one
    /// (the ST's 512, the Apple's double hi-res, the C64's sixteen), this stays
    /// [`Palette::Standard`]: reputation is not evidence.
    pub palette: Palette,
    /// §8.3's Amiga rule: this machine has **one pair of text pens for the whole
    /// screen**, so a `set_colour` moves the screen rather than a window.
    ///
    /// ZMSD §8.3 states it of the Amiga by name, and Infocom's own `amiga/yzip3.c`
    /// says the two text colours "are now 'global', meaning they *can't* be
    /// changed for a single word on the screen, or for a certain window".
    /// [`crate::screen::amiga_global_colour_pair`] carries the full rule; this
    /// flag is the machine half of it, and no other row sets it.
    pub global_colour_pens: bool,
    /// This machine's [`default_colours`](Self::default_colours) are not advice
    /// about a terminal — they **are** the Version 6 screen, the ground every
    /// window that names no colour of its own is read on.
    ///
    /// Two machines answer, and this flag is what
    /// [`crate::screen::machine_screen_pair`] reads (SQ-0846, SQ-0872). The Amiga
    /// for §8.3's own reason — one pair of pens for the whole screen, shared and
    /// unmoving — and the Macintosh for a plainer one: a white page under black
    /// ink was what a Mac window WAS, and `mac/xzip.lst` states it outright.
    ///
    /// The Apple II rows do **not** set it, and that is scope rather than a
    /// finding: their black page is as real as the Mac's white one, but no Apple
    /// v6 frame has been measured against it, and SQ-0846's evidence was a
    /// measured screen rather than a plausible one. Flipping this flag is what
    /// adding them would take.
    pub v6_screen_page: bool,
    /// This machine shows the whole screen through **one palette**, so loading a
    /// picture's colours recolours everything already drawn — border art
    /// included, not just the pictures an archive declares adaptive.
    ///
    /// A v6 framebuffer holds palette INDICES. What that means for the screen
    /// depends on how many colour registers the machine has, and the corpus
    /// splits cleanly:
    ///
    /// * **The Amiga has one set.** Measured on `James Clavell's Shogun.adf`
    ///   (release 295, serial 890321), whose `Pic.data` declares NO adaptive
    ///   pictures and gives all 48 a palette of their own: picture 3 is the
    ///   ornate border and its own table is gold (`#CCAA66`), picture 7 the
    ///   storm on deck (blues — `#99BBDD`, `#334499`) and picture 8 below decks
    ///   (reds and creams — `#CC0000`, `#CCAA88`). On the real machine the
    ///   border is drawn once and is blue-and-white in the storm and red-on-cream
    ///   below decks, because the scene's palette is the screen's palette.
    /// * **The MCGA does not, and the SAME GAME proves it.** Shogun's DOS press
    ///   leaves its side panels one colour throughout, where the Amiga's follow
    ///   the scene — one story, one border, two machines, two behaviours, which
    ///   is as clean a control as this corpus offers. The reason is the hardware:
    ///   the MCGA's DAC has 256 entries and Infocom used them. Arthur's map
    ///   screen lays down seven pictures carrying THREE distinct palettes at
    ///   once and each keeps its own; forcing one onto all seven is what SQ-0881
    ///   fixed — grey ground, rainbow scrolls.
    /// * **EGA and CGA never ask**: a hardware table outranks any loaded palette
    ///   (SQ-0794), so the question does not arise for them.
    ///
    /// **Only the Amiga sets it, and the others are DECLINED rather than
    /// assumed** — the same standard as the rest of this table. The Macintosh
    /// and the Atari ST plausibly had one screen palette too, but no frame of
    /// either has been measured showing a border following a scene, and a
    /// plausible-looking inference is exactly what this table refuses. Turning
    /// one on is one measurement's work: find a title whose border art and scene
    /// art carry different palettes, and look.
    pub one_screen_palette: bool,
    /// What this machine's screen LOOKED like, for a story with no opinion about
    /// it — presentation, not a fact the story can read. See [`PeriodLook`],
    /// which is also where the provenance and its limits are stated.
    ///
    /// `None` is a decline for want of a capture, not a claim that the machine had
    /// no look — and **no row declines any more**. The IBM PC's arrived with
    /// `dos-hitchhiker.png` (SQ-0873/SQ-0928) and the Atari ST's, the last one,
    /// with `st-zork1.png` (SQ-0933). The variant stays because the next machine
    /// added to this table will start out unmeasured, and stating a look by
    /// inference is what `the_machines_with_no_capture_decline` exists to stop.
    pub period_look: Option<PeriodLook>,
}

/// Every §11.1.3 machine zvm models, in number order.
///
/// **The gaps are deliberate and each is a decline.** 1 (DECSystem-20) is absent
/// because it is what declining a number already falls through to, and whether it
/// deserves a bundle of its own or is honestly "a terminal, the same as the IBM
/// PC" is a decision rather than a datum. 11 (Tandy Color) is absent because there
/// is no fixture and no sourced constant, and anything written here would be
/// guesswork — better absent than invented.
///
/// **8 (Commodore 64) used to be absent and should not have been.** The stated
/// reason was that a `.d64` is a 1541 image both Commodore machines read, so the
/// medium cannot choose between 7 and 8 — which is true, and is not a reason for a
/// row to be missing. ProDOS cannot choose between 2, 9 and 10 either, and all
/// three of those have rows; a medium that names a family rather than a machine
/// means the number is asked for rather than inferred, not that the machine goes
/// unmodelled. The other half of the old reason — no Infocom Commodore interpreter
/// read for a v5 pair — is equally true of the Commodore 128, which has a row and
/// declines [`MachineProfile::default_colours`]; declining is what a row is FOR.
/// SQ-0873 then measured the C64's period look, giving it more evidence than
/// several rows that already existed.
///
/// [`machine`] answering `None` is therefore meaningful: it says "this number
/// names a machine I do not model", which a front-end can report instead of
/// silently dressing the story as an IBM PC.
pub const MACHINES: &[MachineProfile] = &[
    MachineProfile {
        number: APPLE_IIE_INTERPRETER_NUMBER,
        name: "Apple IIe",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        period_look: Some(APPLE_PERIOD_LOOK),
    },
    MachineProfile {
        number: MACINTOSH_INTERPRETER_NUMBER,
        name: "Macintosh",
        default_colours: Some((MAC_DEFAULT_BACKGROUND, MAC_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: true,
        // mac-zork1.jpg: Zork I r88/840726 on a Mac Plus, screen 512x342. A 1-bit
        // screen, so no palette can move these; the status line is set apart by
        // rules rather than by ground, and the caret is 1px by the line height.
        period_look: Some(PeriodLook {
            page: (0xFF, 0xFF, 0xFF),
            ink: (0x00, 0x00, 0x00),
            status: StatusBand::Ruled,
            cursor_shape: CursorShape::Bar,
            cursor_colour: (0x00, 0x00, 0x00),
        }),
    },
    MachineProfile {
        number: AMIGA_INTERPRETER_NUMBER,
        name: "Amiga",
        default_colours: Some((AMIGA_DEFAULT_BACKGROUND, AMIGA_DEFAULT_FOREGROUND)),
        palette: Palette::Amiga,
        global_colour_pens: true,
        one_screen_palette: true,
        v6_screen_page: true,
        // amiga-spellbreaker.png (r87/860904) and amiga-lurking.png, both v3 and
        // both giving the identical palette in 7-8 exact colours. Note the page
        // is BLUE where default_colours above reports 12/9, a grey — the v3 and
        // v5 interpreters differ, which is the whole reason this field exists.
        period_look: Some(PeriodLook {
            page: (0x07, 0x4B, 0xA1),
            ink: (0xFF, 0xFF, 0xFF),
            // SQ-0873: the capture reverses PER RUN — 376 px of page show between
            // "Council Chamber" and "Score: 0/0" on `amiga-spellbreaker.png` — and
            // a full-width reverse is what we draw, on the user's ruling. A band
            // broken into pieces reads as damage in a terminal where it read as
            // design on a 1989 monitor. The measurement stands in
            // `StatusBand::PerRun`'s own doc; only the rendering is simplified.
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Block,
            cursor_colour: (0xFF, 0x7E, 0x1C),
        }),
    },
    MachineProfile {
        number: ATARI_ST_INTERPRETER_NUMBER,
        name: "Atari ST",
        default_colours: Some((ST_DEFAULT_BACKGROUND, ST_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        // SQ-0933, from `machine-screenshots/st-zork1.png` — Zork I revision 88 /
        // serial 840726, the same release `stories/` carries. A v3 story, so it has
        // no colour concept at all and every pixel is the interpreter's own
        // presentation: this is a period-look capture in the same sense
        // `dos-hitchhiker.png` is, and it is the one that empties
        // `the_machines_with_no_capture_decline`.
        //
        // **READ THE CAPTURE'S NOTES BEFORE TRUSTING A NUMBER OFF IT.** The
        // emulator applied a scanline filter, and it does not merely dim the frame
        // — it INVERTS the answer. Censused whole, st-zork1.png is 53.6% black
        // against 45.9% near-white, which reads as a black page. Split by row
        // parity, the even rows are 99.2% pure black and the odd rows 92.0%
        // #EBEBEB: the black is the gap between scanlines, and the page is white.
        // Only the odd rows are picture.
        //
        // Corroborated from the other direction, which is why this row is stated
        // rather than hedged: the machine's own v5 pair is already here from
        // Infocom's `st/` interpreter — DEF_BACK 9 (white) under DEF_FORE 2 (black)
        // — and the v3 capture shows the same two. What a v5 story is TOLD and what
        // a v3 story is SHOWN agree, exactly as they did for the IBM PC.
        period_look: Some(PeriodLook {
            // WHITE, not the capture's #EBEBEB, for the same reason the IBM PC row
            // takes EGA's #0000AA over a screenshot's #0F009E: the dimming belongs
            // to the emulator's scanline filter, and standard colours 9 and 2 are
            // white and black with no shade to be wrong about.
            page: (0xFF, 0xFF, 0xFF),
            ink: (0x00, 0x00, 0x00),
            // Measured edge to edge on all eight rows of the band — x=0 and x=639
            // both inked, the last row solid across — with the labels reversed out
            // of it in the page colour.
            status: StatusBand::FullReverse,
            // A filled 8x8 cell immediately right of the `>` prompt.
            cursor_shape: CursorShape::Block,
            cursor_colour: (0x00, 0x00, 0x00),
        }),
    },
    MachineProfile {
        number: IBM_PC_INTERPRETER_NUMBER,
        name: "IBM PC",
        // SQ-0928: blue under white, observed from DOS captures and corroborated
        // by a trace of which games name colours and which do not. This row used
        // to decline, on the ground that "an IBM PC in a terminal is the player's
        // terminal" — which was right about the LAUNCH and wrong about the
        // MACHINE. The two are separated now: the machine states its pair here,
        // and the app presents it only when a medium named this machine (see
        // `IBM_PC_DEFAULT_BACKGROUND`). A bare story file still gets the player's
        // own terminal, which is what that reasoning was protecting.
        default_colours: Some((IBM_PC_DEFAULT_BACKGROUND, IBM_PC_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        // SQ-0873/SQ-0928, from `machine-screenshots/dos-hitchhiker.png` — the last
        // period-look capture bar the Atari ST's. Hitchhiker's r47/840914 under a
        // CGA colour display, and **Version 3** is what makes it a period look
        // rather than a default pair: colour arrives with v5, so nothing on that
        // screen was asked for by the story.
        //
        // Row- and column-censused: page blue at 92.4% of the frame, light grey
        // ink, and a cursor at x 44..58 by y 717..724 — 15px of a 14.5px cell
        // wide and 8px of a ~29px cell tall, so one cell wide across its bottom
        // quarter, in the ink colour. A DOS underscore.
        //
        // **THE RGB IS THE HARDWARE'S, NOT THE CAPTURE'S**, and this row is the
        // one place in the period-look table where that distinction can be made.
        // The capture measures #0F009E and #A0A0A0; the EGA/CGA palette is DIGITAL
        // and exactly specified, so the honest values are its entries 1 and 7 —
        // `blorb::infocom_pics::EGA_PALETTE`, verified there against four sources
        // that agree entry for entry. The gap is video scaling, and it shows in
        // the corpus: `dos-shogun.png` and `dos-arthur.png` measure #0000A1..A6
        // with no red tint at all, while this capture's #0F009E carries 0x0F of
        // red that no EGA colour has.
        //
        // Every other row here records what an emulator drew, because those
        // machines' palettes are analogue (the VIC-II) or the register mapping is
        // in doubt (the Amiga's #074BA1 against a bit-replicated $05A = #0055AA).
        // This one does not have to guess.
        //
        // A PC's look was its display adapter's answer, which is why this row
        // declined for so long; what is recorded is the CGA colour rendition, the
        // same way the Apple's records the white-monitor one.
        //
        // **THE STATUS LINE IS SIMPLIFIED, DELIBERATELY.** The capture shows
        // `PerRun` structure — 611 contiguous px of page run through the middle of
        // the row — but its runs are #A2000D RED on grey, a colour in neither
        // channel of the body pair and appearing nowhere else in the frame. That is
        // `Own` colours applied per run, which `StatusBand` cannot express: its
        // variants carry a structure or a pair, never both. Recorded as a plain
        // `PerRun` on the user's instruction rather than growing the type for one
        // glyph colour on one row. The red is real and is not modelled; see
        // `machine-screenshots/info.txt`.
        period_look: Some(PeriodLook {
            page: (0x00, 0x00, 0xAA),
            ink: (0xAA, 0xAA, 0xAA),
            // Per-run in the capture, full-width here — the same ruling as the
            // Amiga's above, and the same reason.
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0xAA, 0xAA, 0xAA),
        }),
    },
    MachineProfile {
        number: COMMODORE_128_INTERPRETER_NUMBER,
        name: "Commodore 128",
        // Declined: no Infocom Commodore interpreter has been read for a v5
        // story's default pair (SQ-0869). The measured look is period_look below.
        default_colours: None,
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        // c128-trinity.png: Trinity (v4) at the first prompt, two colours exactly.
        // #55FFFF is RGBI light cyan on the nose, so this row is as close to
        // palette-independent as an emulator capture gets.
        period_look: Some(PeriodLook {
            page: (0x00, 0x00, 0x00),
            ink: (0x55, 0xFF, 0xFF),
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0x55, 0xFF, 0xFF),
        }),
    },
    MachineProfile {
        number: COMMODORE_64_INTERPRETER_NUMBER,
        name: "Commodore 64",
        // Declined for the same reason the C128's is: no Infocom Commodore
        // interpreter has been read for a v5 story's default pair (SQ-0869).
        default_colours: None,
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        // c64-zork1-solidgold.png: Zork I release 52 / serial 871125, whose own
        // banner reads "Interpreter 8 Version J" — the machine naming itself, which
        // is as direct a statement of the number as this table has anywhere.
        //
        // Black page, white ink, and the status line a plain full-width reverse
        // running x 37..522 with no interior gap wider than a glyph.
        period_look: Some(PeriodLook {
            page: (0x00, 0x00, 0x00),
            ink: (0xFF, 0xFF, 0xFF),
            status: StatusBand::FullReverse,
            cursor_shape: CursorShape::Underscore,
            cursor_colour: (0xFF, 0xFF, 0xFF),
        }),
    },
    MachineProfile {
        number: APPLE_IIC_INTERPRETER_NUMBER,
        name: "Apple IIc",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        period_look: Some(APPLE_PERIOD_LOOK),
    },
    MachineProfile {
        number: APPLE_IIGS_INTERPRETER_NUMBER,
        name: "Apple IIgs",
        default_colours: Some((APPLE_DEFAULT_BACKGROUND, APPLE_DEFAULT_FOREGROUND)),
        palette: Palette::Standard,
        global_colour_pens: false,
        one_screen_palette: false,
        v6_screen_page: false,
        period_look: Some(APPLE_PERIOD_LOOK),
    },
];

/// The machine header `$1E` value `number` names, or `None` for a §11.1.3 number
/// this table does not model. See [`MACHINES`] for which are absent and why.
pub fn machine(number: u8) -> Option<&'static MachineProfile> {
    MACHINES.iter().find(|m| m.number == number)
}

/// The machine a story's header currently claims to be running on, or `None`
/// when `$1E` names one this table does not model.
///
/// Read back out of the HEADER rather than held as a field, which is what makes
/// every rule built on it survive a `@restart`, a Quetzal `@restore` and a host
/// Save State without anybody carrying it.
pub fn machine_of(mem: &Memory) -> Option<&'static MachineProfile> {
    machine(mem.read_byte(0x1E))
}

/// The `(foreground, background)` pair the header currently publishes, as
/// §8.3.1 standard colour numbers — `$2D` over `$2C` (ZMSD §8.3.3).
///
/// Shared by [`crate::screen::amiga_screen_pair`] and
/// [`crate::screen::machine_screen_pair`], which differ only in which machines
/// they ask it of.
pub(crate) fn header_pair(mem: &Memory) -> (ZColour, ZColour) {
    (ZColour::Standard(mem.read_byte(0x2D)), ZColour::Standard(mem.read_byte(0x2C)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_reachable_by_its_own_number_and_the_table_is_sorted() {
        let mut last = 0u8;
        for m in MACHINES {
            assert!(m.number > last, "{} out of order at {}", m.name, m.number);
            last = m.number;
            assert_eq!(machine(m.number).map(|r| r.name), Some(m.name));
        }
    }

    /// The gaps, asserted as gaps. A future row is welcome; a future row arriving
    /// *by accident* — say by widening a match — is what this catches.
    #[test]
    fn the_unmodelled_numbers_answer_none_rather_than_a_substitute() {
        for n in [0u8, 1, 11, 12, 255] {
            assert!(machine(n).is_none(), "interpreter {n} is not modelled and must say so");
        }
    }

    #[test]
    fn the_amiga_is_the_only_machine_with_global_colour_pens() {
        let pens: Vec<_> =
            MACHINES.iter().filter(|m| m.global_colour_pens).map(|m| m.number).collect();
        assert_eq!(pens, vec![AMIGA_INTERPRETER_NUMBER], "ZMSD §8.3 names the Amiga and no other");
        // …and a machine with global pens necessarily states the page they paint.
        for m in MACHINES.iter().filter(|m| m.global_colour_pens) {
            assert!(m.v6_screen_page, "{} has pens but states no page", m.name);
            assert!(m.default_colours.is_some(), "{} has pens but no pair", m.name);
        }
    }

    /// SQ-0846's two machines, and only those two. The Apple rows state a page
    /// as real as the Mac's and deliberately do not claim it — see
    /// [`MachineProfile::v6_screen_page`].
    #[test]
    fn two_machines_state_the_version_6_page() {
        let pages: Vec<_> = MACHINES.iter().filter(|m| m.v6_screen_page).map(|m| m.number).collect();
        assert_eq!(pages, vec![MACINTOSH_INTERPRETER_NUMBER, AMIGA_INTERPRETER_NUMBER]);
    }

    /// The Apple family is one bundle with three numbers — `bsubs.asm`'s
    /// `MACHINE:` picks between `IIeID 2`, `IIcID 9` and `IIgsID 10` at boot, and
    /// `zboot.asm` seeds the same `$2C`/`$2D` for all three before it runs.
    #[test]
    fn the_three_apples_differ_only_in_their_number() {
        let apples: Vec<_> = [
            APPLE_IIE_INTERPRETER_NUMBER,
            APPLE_IIC_INTERPRETER_NUMBER,
            APPLE_IIGS_INTERPRETER_NUMBER,
        ]
        .into_iter()
        .map(|n| *machine(n).expect("modelled"))
        .collect();
        for a in &apples {
            assert_eq!(a.default_colours, Some((2, 9)), "zboot.asm: black page, white ink");
            assert_eq!(a.palette, Palette::Standard, "ZIPCOLOR IS §8.3.1's eight");
            assert_eq!(a.global_colour_pens, apples[0].global_colour_pens);
            assert_eq!(a.v6_screen_page, apples[0].v6_screen_page);
        }
        // …and the numbers really are three, so this is a family and not a copy.
        assert_eq!(apples.iter().filter(|a| a.number == apples[0].number).count(), 1);
    }

    /// Sourced values, one assertion per machine, quoted where the constant is.
    #[test]
    fn the_pairs_are_the_sourced_ones() {
        let pair = |n| machine(n).expect("modelled").default_colours;
        assert_eq!(pair(AMIGA_INTERPRETER_NUMBER), Some((12, 9)), "the floppies' DEF_BACK/DEF_FORE");
        assert_eq!(pair(MACINTOSH_INTERPRETER_NUMBER), Some((9, 2)), "SetColor := zWHITE*256 + zBLACK");
        assert_eq!(pair(ATARI_ST_INTERPRETER_NUMBER), Some((9, 2)), "DEF_BACK 9 / DEF_FORE 2");
        assert_eq!(pair(APPLE_IIGS_INTERPRETER_NUMBER), Some((2, 9)), "zboot.asm lda #2 / lda #9");
        // SQ-0928: the one pair here that is OBSERVED rather than read out of
        // Infocom's source — but observed twice over, from DOS captures and from a
        // trace of which games name colours and which do not. And what is observed
        // is a §8.3.1 NUMBER, not a shade, so unlike `PeriodLook` there is no
        // emulator-dependent RGB to be wrong about.
        //
        // It used to decline, "the player's terminal" — which was right about the
        // LAUNCH and wrong about the MACHINE. The two are separated now: the row
        // states the machine, and a front-end presents it only when a medium named
        // this machine. A bare story file still gets the player's own terminal.
        assert_eq!(pair(IBM_PC_INTERPRETER_NUMBER), Some((6, 9)), "blue under white");
        assert_eq!(pair(COMMODORE_128_INTERPRETER_NUMBER), None, "no source read");
        // Only the Amiga loaded a palette of its own.
        let amiga: Vec<_> =
            MACHINES.iter().filter(|m| m.palette != Palette::Standard).map(|m| m.number).collect();
        assert_eq!(amiga, vec![AMIGA_INTERPRETER_NUMBER]);
    }

    /// **A period look is not a `default_colours` pair**, and the two rows where
    /// they diverge are the reason the field exists (SQ-0873).
    ///
    /// The Amiga reports 12/9 — a GREY page — from its v5-era interpreter, and its
    /// v3 interpreter draws a blue one. The Commodore 128 reports nothing at all and
    /// yet has a measured look. If a later change ever makes one derivable from the
    /// other, this case is what should stop it.
    #[test]
    fn the_period_look_is_not_the_default_pair() {
        let amiga = machine(AMIGA_INTERPRETER_NUMBER).expect("modelled");
        assert_eq!(amiga.default_colours, Some((12, 9)), "grey page, white ink, from v5");
        let look = amiga.period_look.expect("measured on two v3 floppies");
        assert_eq!(look.page, (0x07, 0x4B, 0xA1), "and the v3 interpreter's page is BLUE");
        // Grey is 10..12 in §8.3.1; whatever blue resolves to, it is not that.
        assert_ne!(look.page.0, look.page.1, "a grey has equal channels; this does not");

        let c128 = machine(COMMODORE_128_INTERPRETER_NUMBER).expect("modelled");
        assert_eq!(c128.default_colours, None, "no v5 source was ever read");
        assert!(c128.period_look.is_some(), "which did not stop the machine being measured");
    }

    /// The three sub-decisions are independent — the finding that fixed this
    /// field's shape. Five machines produced four status behaviours and three
    /// cursor shapes, and neither follows from the body pair.
    #[test]
    fn the_status_band_and_cursor_do_not_follow_from_the_body_pair() {
        let look = |n| machine(n).expect("modelled").period_look.expect("measured");
        let (apple, mac, amiga, c128) = (
            look(APPLE_IIE_INTERPRETER_NUMBER),
            look(MACINTOSH_INTERPRETER_NUMBER),
            look(AMIGA_INTERPRETER_NUMBER),
            look(COMMODORE_128_INTERPRETER_NUMBER),
        );
        // Four measured machines, three distinct status behaviours between them.
        assert_eq!(apple.status, StatusBand::FullReverse);
        assert_eq!(c128.status, StatusBand::FullReverse);
        assert_eq!(mac.status, StatusBand::Ruled, "rules, not a ground");
        // SQ-0873: the Amiga's capture reverses PER RUN and it draws a full-width
        // reverse, on the user's ruling — see `StatusBand::PerRun`. What this case
        // is really about survives the change: the four measured behaviours are
        // still not derivable from the body pair, and the Macintosh's is the proof.
        assert_eq!(amiga.status, StatusBand::FullReverse, "drawn whole, measured per run");

        // Three cursor shapes, and the Macintosh's bar is the one no cell-inverting
        // front-end would ever draw.
        assert_eq!(mac.cursor_shape, CursorShape::Bar);
        assert_eq!(apple.cursor_shape, CursorShape::Block);
        assert_eq!(amiga.cursor_shape, CursorShape::Block);
        assert_eq!(c128.cursor_shape, CursorShape::Underscore);

        // And on two machines the cursor's colour is neither the page nor the ink,
        // so it cannot be dropped and recomputed.
        assert!(
            amiga.cursor_colour != amiga.page && amiga.cursor_colour != amiga.ink,
            "the Amiga's cursor is orange on a blue page under white ink",
        );
        assert_eq!(c128.cursor_colour, c128.ink, "…but the C128's is simply its ink");
    }

    /// Declines are declines, and the ones here are for want of a capture rather
    /// than for want of a decision. Fails when someone fills a row by inference.
    #[test]
    fn the_machines_with_no_capture_decline() {
        let missing: Vec<_> =
            MACHINES.iter().filter(|m| m.period_look.is_none()).map(|m| m.number).collect();
        // The list is EMPTY now, and that is the point of keeping the case rather
        // than deleting it: the IBM PC left when `dos-hitchhiker.png` arrived
        // (SQ-0873), the Atari ST when `st-zork1.png` did (SQ-0933), and the next
        // machine anyone adds starts out unmeasured. This fails the moment a row
        // states a look with nothing in `machine-screenshots/` behind it.
        assert_eq!(
            missing,
            Vec::<u8>::new(),
            "every modelled machine has been measured; a new row must be too",
        );
        // The Atari ST's capture is the one that emptied the list, and its two
        // channels agree — which is the whole reason it could be stated. A v3
        // frame shows white under black; the machine's own v5 pair, sourced from
        // Infocom's `st/` interpreter long before any capture existed, is
        // DEF_BACK 9 / DEF_FORE 2 — white and black in §8.3.1.
        let st = machine(ATARI_ST_INTERPRETER_NUMBER).expect("modelled");
        let look = st.period_look.expect("st-zork1.png, Zork I r88/840726");
        assert_eq!(st.default_colours, Some((9, 2)), "told white under black");
        assert_eq!(look.page, (0xFF, 0xFF, 0xFF), "…and shown the same");
        assert_eq!(look.ink, (0x00, 0x00, 0x00));
        // Not the capture's #EBEBEB: that dimming is the emulator's scanline
        // filter, and a §8.3.1 white has no shade to be wrong about. The same
        // judgement the IBM PC row makes about #0F009E.
        assert_ne!(look.page, (0xEB, 0xEB, 0xEB), "the emulator's dimming is not the machine's white");
        assert_eq!(look.status, StatusBand::FullReverse, "measured edge to edge on all eight rows");
        assert_eq!(look.cursor_shape, CursorShape::Block);
        assert_eq!(look.cursor_colour, look.ink, "a filled cell in the ink, like the C128's");

        // The Apple family shares one measurement, exactly as it shares one pair.
        let apples: Vec<_> = [
            APPLE_IIE_INTERPRETER_NUMBER,
            APPLE_IIC_INTERPRETER_NUMBER,
            APPLE_IIGS_INTERPRETER_NUMBER,
        ]
        .into_iter()
        .map(|n| machine(n).expect("modelled").period_look)
        .collect();
        assert!(apples.windows(2).all(|w| w[0] == w[1]), "one interpreter, one text screen");
    }
}
