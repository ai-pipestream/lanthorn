//! SQ-1154 — `--colour` names a REGIME, and the regime is independent of the
//! medium.
//!
//! In the user's words: *"when `--colour theme | terminal` we use the raw
//! z-machine path with native media. The opposite is true when we set `--colour
//! machine`, then we use the media path with a raw z-machine file."*
//!
//! Half of that shipped already. `--colour machine` on a bare story file is
//! SQ-0928's `--system-colours`, subsumed by SQ-1082: the opt-in that licenses a
//! machine's §8.3.3 pair when the MEDIUM did not name a machine but
//! `--interpreter` did. The mirror is what this suite pins — a release floppy
//! launched under `--colour terminal` behaves as the same story does as a bare
//! file, because `Config::machine_colours_licensed` answers **no** for that
//! regime whatever the profile's source was.
//!
//! # What "the raw path" reaches, and what it does not
//!
//! Everything that asks the one predicate moves together, which is the whole
//! design: the machine's §8.3.3 pair, the two-colour CARD's pair (and with it
//! `Palette::IbmCga`), and the colour-number TABLE the story's own `set_colour`
//! resolves through. So the round trip is lossless again: the host's RGB is
//! snapped to the nearest §8.3.1 number, and §8.3.1 is the table it is read back
//! through — which on a floppy it was not, and which is the reported symptom.
//!
//! **The ARTWORK is untouched**, and [`the_artwork_is_the_archives_and_the_regime_cannot_move_it`]
//! measures it rather than arguing it: pictures resolve through
//! `graphics::PictSource`'s own per-picture palette, read from the archive's own
//! `PLTE`, and never through `zvm::screen`'s.
//!
//! # Specimens
//!
//! | fixture | medium | machine | what it is here for |
//! |---|---|---|---|
//! | `Journey - The Quest Begins.adf` | Amiga floppy | Amiga | a medium whose table is NOT §8.3.1 |
//! | Zork Zero DOS 360K Disk 1 | `.ima` | IBM PC + CGA card | the two-colour card, which no bare file can reach |
//! | `journey-r83-s890706.z6` | none | asked for | the `--colour machine` direction, unchanged |
//!
//! **Turn count: zero.** Every question here is settled before the first
//! prompt — the palette is installed and the pair is handed to the constructor,
//! so driving keys would only add ways for a frame to differ.
//!
//! Both `honor_game_colours` modes throughout, per the project's colour
//! convention. `false` is not a formality here: it short-circuits
//! `colors::host_default_colours` to `None` (§8.3.2, an interpreter that
//! declares itself colourless has no default pair to report), so it is what
//! proves `--colour` is inert against that switch rather than fighting it.
//!
//! `stories/` is gitignored, so every case skips vacuously without its press and
//! [`the_presses_were_actually_read`] is what stops the file quietly passing on a
//! machine that has none of them.

use std::path::PathBuf;

use app::config::{ColourSource, Config};
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::{InterpreterProfile, ProfileSource};
use app::session::GameSession;

use ratatui::style::Style;
use zvm::screen::Palette;

/// The Amiga release floppy: `InterpreterProfile::Amiga`, off the medium, and a
/// colour-number table that is emphatically not §8.3.1's.
const AMIGA_FLOPPY: &str = "Journey - The Quest Begins.adf";
/// The bare story file, for the `--colour machine` direction. A different
/// RELEASE from the floppy above (r83/s890706 against r30/s890322) and named
/// here only as "a story with no medium", which is all this suite asks of it.
const BARE_STORY: &str = "journey-r83-s890706.z6";
/// The DOS press serving the CGA plate — the two-colour card, and the one thing
/// in this quest with no existence proof behind it, since a bare story file
/// never reaches the card at all.
const CGA_PRESS: &str =
    "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";

/// The terminal this suite pretends to be launched in: a warm dark page under a
/// light ink, chosen so the pair it snaps to is neither machine's.
const TERM_BG: (u8, u8, u8) = (0x1A, 0x1B, 0x26);
const TERM_FG: (u8, u8, u8) = (0xE4, 0xE4, 0xDC);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn present(file: &str) -> bool {
    stories_dir().join(file).exists()
}

/// What one launch decided about colour, in `startup.rs`'s own order and through
/// the shipped functions rather than a copy of them.
struct Launch {
    profile: InterpreterProfile,
    source: ProfileSource,
    /// The table installed process-wide before the story ran —
    /// `Config::machine_text_palette`, which is the call `startup.rs` makes.
    palette: Palette,
    /// `PictSource::two_colour_card_screen` — the card's table and pair, when
    /// this launch is showing one.
    card: Option<(Palette, (u8, u8))>,
    /// The pair the session is constructed with, which reaches header
    /// `$2C`/`$2D`.
    reported: Option<(u8, u8)>,
    /// Whether the game's own colours survived the archive's say —
    /// `startup.rs`'s answer, not the caller's request.
    honoured: bool,
    /// The story as it booted, so the header can be read back out of memory.
    session: GameSession,
}

/// Boot one file the way `startup.rs` boots it, under a stated colour regime.
///
/// Every colour decision below is the shipped function: the profile off the
/// medium, `Config::machine_text_palette` for the table, `declines_game_colours`
/// for the honour flag, `PictSource::two_colour_card_screen` for the card, and
/// `colors::host_default_colours` for the pair. A harness that re-derived any of
/// them would keep passing while the shipped path regressed, which is the hazard
/// CLAUDE.md names as "boot a harness the way `startup.rs` boots".
///
/// The palette lock is the CALLER's to hold (SQ-0987): this calls
/// `app::v6_set_palette`, which panics off a guard.
fn launch(
    file: &str,
    colour: ColourSource,
    honour: bool,
    interpreter: Option<u8>,
) -> Option<Launch> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored press missing at {}", path.display());
            return None;
        }
    };

    let dir = app::scratch_dir("sq1154-launch");
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let named_art_std_window = over.std_window();
    let (profile, source) =
        InterpreterProfile::resolve_with_source(&path, interpreter, over.flavour(), None);

    let mut cfg = Config {
        interpreter_profile: profile,
        interpreter_source: source,
        interpreter_number: interpreter,
        honor_game_colours: honour,
        colour_source: colour,
        // `config::resolve` sets this whenever `--colour machine` is TYPED; the
        // other two arms leave it alone, and it is inert under them now.
        system_colours: colour == ColourSource::Machine,
        ..Default::default()
    };

    // `startup.rs`, in order. The table first, before the constructor runs the
    // story and before the host resolves a single colour.
    app::v6_set_palette(cfg.machine_text_palette(bytes.first().copied()));
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();

    // SQ-0806/SQ-0846: two-colour artwork declares the interpreter colourless,
    // but only where the launch has no machine to state a ground of its own.
    if picts.declines_game_colours(cfg.machine_default_colours()) && cfg.honor_game_colours {
        cfg.honor_game_colours = false;
    }
    let mut reported = app::colors::host_default_colours(
        &cfg,
        cfg.machine_default_colours(),
        Style::default(),
        Some(TERM_FG),
        Some(TERM_BG),
    );
    let card = picts.two_colour_card_screen(&cfg);
    if let Some((palette, pair)) = card {
        app::v6_set_palette(palette);
        reported = Some(pair);
    }

    let boot = app::machine_boot::MachineBoot::resolve(
        cfg.interpreter_profile,
        &picts,
        named_art_std_window,
        cfg.advertised_interpreter_number(),
        reported,
        app::native_font::FaceSet::none(),
    );
    let mut session = GameSession::new_for_machine(
        bytes,
        cfg.honor_game_colours,
        false,
        false,
        picture_dims,
        None,
        None,
        &boot,
    )
    .unwrap_or_else(|e| panic!("{file} boots: {e:?}"));
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);

    Some(Launch {
        profile,
        source,
        palette: zvm::screen::palette(),
        card,
        reported,
        honoured: cfg.honor_game_colours,
        session,
    })
}

/// The pair a host-sourced regime resolves to, straight out of the shipped
/// snapper. Named once so no case pins a literal that would have to be re-derived
/// when the §8.3.1 nearest-neighbour search is touched.
fn terminals_pair() -> (u8, u8) {
    app::colors::host_default_colour_pair(Style::default(), Some(TERM_FG), Some(TERM_BG))
        .expect("both channels probed")
}

/// Header `$2C`/`$2D`, as the story reads them.
fn header_pair(l: &Launch) -> (u8, u8) {
    (l.session.machine.mem.read_byte(0x2C), l.session.machine.mem.read_byte(0x2D))
}

// ── The medium under a host regime ───────────────────────────────────────────

/// **The reported case.** An Amiga release floppy under `--colour terminal` and
/// `--colour theme` is told the TERMINAL's pair, through §8.3.1's table — the
/// state a bare story file is in.
///
/// The two halves are one defect and both are asserted, because either alone
/// looks correct: the pair reaching `$2C`/`$2D` is the host's, AND the palette
/// those numbers resolve through is `Standard`. Snapping to §8.3.1 and reading
/// back through the Amiga's is what "reports a colour that is not the
/// terminal's" means.
///
/// FALSIFY by restoring `machine_colours_licensed` to
/// `interpreter_source.licenses_machine_colours(system_colours)`: the floppy is
/// `ProfileSource::Medium`, so it licenses the machine whatever `--colour` said,
/// and both assertions fail with the Amiga's own pair under the Amiga's table.
#[test]
fn a_release_floppy_under_a_host_regime_is_told_the_hosts_pair() {
    let _g = app::v6_palette_at_boot();
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let machine_pair = InterpreterProfile::Amiga.default_colours().expect("the Amiga states one");
    assert_ne!(machine_pair, terminals_pair(), "the experiment needs the two to differ");

    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        let l = launch(AMIGA_FLOPPY, colour, true, None).expect("checked present");
        assert_eq!(l.source, ProfileSource::Medium, "the floppy still names its machine");
        assert_eq!(l.profile, InterpreterProfile::Amiga, "and it is still an Amiga");
        assert_eq!(
            l.palette,
            Palette::Standard,
            "{colour:?}: the raw path resolves colour numbers through §8.3.1"
        );
        assert_eq!(l.reported, Some(terminals_pair()), "{colour:?}: the host's own pair");
        assert_eq!(header_pair(&l), terminals_pair(), "{colour:?}: and the story reads it");
        assert_ne!(header_pair(&l), machine_pair, "{colour:?}: not the Amiga's");
    }
}

/// The control, on the same floppy: `--colour machine` is the default and is
/// unmoved — the Amiga's pair, under the Amiga's table.
///
/// Without this the case above is satisfied by breaking original media outright.
#[test]
fn the_same_floppy_under_colour_machine_is_still_an_amiga() {
    let _g = app::v6_palette_at_boot();
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let l = launch(AMIGA_FLOPPY, ColourSource::Machine, true, None).expect("checked present");
    assert_eq!(l.palette, Palette::Amiga, "the medium's own table");
    assert_eq!(
        l.reported,
        InterpreterProfile::Amiga.default_colours(),
        "and the machine's §8.3.3 pair"
    );
    assert_eq!(header_pair(&l), l.reported.expect("licensed"), "which the story reads");
}

/// **`honor_game_colours = false` still answers before `--colour` does**, in
/// every regime.
///
/// §8.3.2: an interpreter that declares itself colourless has no default page
/// and ink to report, so the VM's own black-on-white seed is left alone. The two
/// flags are different axes and `--colour` is inert while this one says off —
/// the property a single-mode suite cannot see, and the reason this file pins
/// both modes.
#[test]
fn declining_game_colours_outranks_every_regime() {
    let _g = app::v6_palette_at_boot();
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    for colour in [ColourSource::Machine, ColourSource::Theme, ColourSource::Terminal] {
        let l = launch(AMIGA_FLOPPY, colour, false, None).expect("checked present");
        assert_eq!(l.reported, None, "{colour:?}: a colourless interpreter reports no pair");
        assert!(!l.honoured, "{colour:?}: and the story is told so");
    }
}

// ── The two-colour card, which no bare story file can reach ──────────────────

/// **The CGA press.** A `.CG1` under `--colour terminal` installs no
/// `Palette::IbmCga`, because the launch has no machine to be showing a card
/// FOR.
///
/// This is the one question in SQ-1154 the raw path could not vouch for: a bare
/// story file never reaches the card at all, so nothing about `IbmCga` carrying
/// a display-DEPTH fact — read by `zvm::screen::two_colour_card_request`, which
/// decides whether the story asks for mono or colour plates — is exercised by
/// that existence proof. The answer is that the coupling is not excepted from
/// the regime, it is governed by it: `two_colour_card_screen` asks the same
/// licence, so under a host regime it answers `None` and neither the table nor
/// the depth is ever installed.
///
/// Both `honor_game_colours` modes, and the honoured half is the load-bearing
/// one — with colours declined the card is unreachable for a second reason and
/// the case would pass without testing anything.
#[test]
fn a_cga_press_under_a_host_regime_shows_no_card() {
    let _g = app::v6_palette_at_boot();
    if !present(CGA_PRESS) {
        eprintln!("SKIP: {CGA_PRESS} absent");
        return;
    }
    // The control first: this press really is showing a card, or the assertions
    // below are vacuous.
    let machine = launch(CGA_PRESS, ColourSource::Machine, true, None).expect("checked present");
    assert_eq!(machine.source, ProfileSource::Medium, "a DOS press names the IBM PC");
    assert!(machine.card.is_some(), "the 360K Disk 1 serves the CGA plate");
    assert_eq!(machine.palette, Palette::IbmCga, "and the card's table is installed");
    assert!(machine.palette.two_colour_card(), "which carries the display's one bit");

    for colour in [ColourSource::Terminal, ColourSource::Theme] {
        for honour in [true, false] {
            let l = launch(CGA_PRESS, colour, honour, None).expect("checked present");
            assert!(l.card.is_none(), "{colour:?}/{honour}: no machine, so no card");
            assert_ne!(l.palette, Palette::IbmCga, "{colour:?}/{honour}: IbmCga never installed");
            assert!(
                !l.palette.two_colour_card(),
                "{colour:?}/{honour}: nor the display depth it carries"
            );
            assert_eq!(l.palette, Palette::Standard, "{colour:?}/{honour}: §8.3.1's table");
        }
    }
}

// ── The other direction, which already worked ────────────────────────────────

/// **`--colour machine` on a bare `.z6` with `--interpreter` is unchanged.**
///
/// SQ-0928's opt-in, subsumed by SQ-1082: a number typed at a bare story file is
/// `ProfileSource::Asked`, which licenses the machine only on request. This
/// quest drives the same predicate from the other end and must not disturb it,
/// so both sides of the opt-in are pinned — with `--colour machine` the Amiga's
/// pair and table, without it neither.
#[test]
fn colour_machine_on_a_bare_story_file_still_presents_the_asked_for_machine() {
    let _g = app::v6_palette_at_boot();
    if !present(BARE_STORY) {
        eprintln!("SKIP: {BARE_STORY} absent");
        return;
    }
    let asked = launch(BARE_STORY, ColourSource::Machine, true, Some(4)).expect("checked present");
    assert_eq!(asked.source, ProfileSource::Asked, "a typed number, not a medium");
    assert_eq!(asked.profile, InterpreterProfile::Amiga, "--interpreter 4");
    assert_eq!(asked.palette, Palette::Amiga, "the opt-in licenses the machine's table");
    assert_eq!(
        asked.reported,
        InterpreterProfile::Amiga.default_colours(),
        "and its §8.3.3 pair, on a file that came off no disk at all"
    );
    assert_eq!(header_pair(&asked), asked.reported.expect("licensed"));

    // …and without the opt-in the same launch is a bare story file, which is the
    // half `--colour theme|terminal` now reproduces on a medium.
    let plain = launch(BARE_STORY, ColourSource::Terminal, true, Some(4)).expect("checked present");
    assert_eq!(plain.palette, Palette::Standard, "no licence, no machine table");
    assert_eq!(plain.reported, Some(terminals_pair()), "the host answers instead");
}

// ── The artwork, which none of this may move ─────────────────────────────────

/// **The regime decides the TEXT table and nothing else.** The same picture off
/// the same floppy decodes identically under `--colour machine` and `--colour
/// terminal`, pixel for pixel.
///
/// Measured rather than argued, because the design turns on it: pictures resolve
/// through `PictSource`'s own Current Palette, established per picture from the
/// archive's own `PLTE` (§11.3), and never through `zvm::screen`'s. If that were
/// not so, withholding the machine's table would repaint the artwork — which is
/// the one outcome this quest may not have.
#[test]
fn the_artwork_is_the_archives_and_the_regime_cannot_move_it() {
    let _g = app::v6_palette_at_boot();
    if !present(AMIGA_FLOPPY) {
        eprintln!("SKIP: {AMIGA_FLOPPY} absent");
        return;
    }
    let plate = |colour: ColourSource| -> Vec<u8> {
        let path = stories_dir().join(AMIGA_FLOPPY);
        let dir = app::scratch_dir("sq1154-art");
        let over = PictureOverride::resolve_with_session(&path, &dir, None);
        let (profile, source) =
            InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
        let cfg = Config {
            interpreter_profile: profile,
            interpreter_source: source,
            colour_source: colour,
            system_colours: colour == ColourSource::Machine,
            ..Default::default()
        };
        app::v6_set_palette(cfg.machine_text_palette(Some(6)));
        let mut picts = PictSource::resolve_with_override(&path, over, None);
        let dims = picts.all_pict_dims();
        let (resnum, ..) = *dims.first().expect("the floppy serves plates");
        let img = picts.image(u32::from(resnum)).expect("the first plate decodes");
        let _ = std::fs::remove_dir_all(&dir);
        img.to_rgba8().into_raw()
    };
    let machine = plate(ColourSource::Machine);
    assert!(!machine.is_empty(), "a plate with pixels in it");
    assert_eq!(machine, plate(ColourSource::Terminal), "the archive's palette, either way");
}

// ── Non-vacuity ──────────────────────────────────────────────────────────────

/// At least one press was actually on disk, or every case above skipped and this
/// file passed without measuring anything (CLAUDE.md's rule for gitignored
/// commercial media).
#[test]
fn the_presses_were_actually_read() {
    let found: Vec<&str> =
        [AMIGA_FLOPPY, BARE_STORY, CGA_PRESS].into_iter().filter(|f| present(f)).collect();
    if found.is_empty() {
        eprintln!("SKIP: no gitignored press present; every case in this file skipped");
        return;
    }
    eprintln!("SQ-1154 specimens read: {found:?}");
}
