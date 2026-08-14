//! SQ-0838 (second half): the Macintosh interpreter profile, and the screen
//! model that comes with it.
//!
//! The first half taught `InfocomPics` to read the monochrome archive on
//! `stories/Zork Zero Disk.image` — **Zork Zero v6 release 296, serial 881019**.
//! This half answers the question that archive raised: *what machine is this,
//! and how big is its screen?*
//!
//! # Everything here is quoted from Infocom's own Macintosh interpreter
//!
//! `mac/xzip.lst` and `mac/gfx.p` in `github.com/erkyrath/infocom-zcode-terps`,
//! the sources already cited throughout `blorb::infocom_pics`. The Mac sizes its
//! window and picks its picture file in ONE decision:
//!
//! ```text
//!   GFXAM_Y  = 200;  GFXAM_X  = 320;   { "raw" size of full-screen Amiga pics }
//!   GFXMAC_Y = 300;  GFXMAC_X = 480;   { 1.5 x Amiga sizes }
//!   …
//!   IF ((ydisplay < 2*GFXAM_Y) OR (xdisplay < 2*GFXAM_X))
//!     OR ((mColor = FALSE) OR (ttyToggle)) THEN
//!     BEGIN  myBig := FALSE;  wy := GFXMAC_Y;  wx := GFXMAC_X;  END
//!   ELSE
//!     BEGIN  myBig := TRUE;   wy := 2*GFXAM_Y; wx := 2*GFXAM_X; END
//!   …
//!   IF myBig THEN gfxName := 'CPic.Data' ELSE gfxName := 'Pic.Data';
//! ```
//!
//! and the screen the STORY is told about is that window, in pixels:
//!
//! ```text
//!   { calculate our logical screen size, based on window size }
//!     WITH myWindow^.portRect DO
//!       BEGIN
//!       totRows := (bottom - top) {DIV lineheight};
//!       totCols := ((right - left) - (2 * wMarg)) {DIV colWidth};
//!       END;
//! ```
//!
//! So there are **two Macintosh screens**, and the artwork picks:
//!
//! | archive          | picture space | display scale | screen  |
//! |------------------|---------------|---------------|---------|
//! | `CPic.data`      | 320×200       | 2× (`myBig`)  | 640×400 |
//! | `Pic.data` (b&w) | 480×300       | 1× (`ge.mono`)| 480×300 |
//!
//! **512×342 is the hardware and never the standard window.** The compact Mac's
//! screen appears in the interpreter only as `screenRect := screenBits.bounds`,
//! the thing the window is centred inside — `MoveWindow (myWindow, …,
//! (ydisplay-wy) DIV 2 + 17 {mbar fudge}, TRUE)` puts a 300-pixel content rect
//! at y = 38 on a 342-pixel screen, which is exactly a 20-pixel menu bar plus a
//! 19-pixel title bar. A photograph of a real Mac Zork Zero is therefore 512×342
//! with the game filling nearly all of it, while the game itself is being told
//! 480×300.
//!
//! **Neither path has non-square pixels.** Every scaling arm in `gfx.p` moves
//! both axes by the same factor — `ge.dy := ge.ry * 2; ge.dx := ge.rx * 2`,
//! `(ge.ry * 3) DIV 2` with `(ge.rx * 3) DIV 2`, or 1× for both — unlike the
//! IBM PC's EGA rendition, whose pixels really are half as wide (SQ-0790). Both
//! Mac archives are 1.6:1 and stay 1.6:1.

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};
use std::path::PathBuf;

const RELEASE: u16 = 296;
const SERIAL: &[u8] = b"881019";

fn mac_disk() -> Option<PathBuf> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/Zork Zero Disk.image");
    if !path.exists() {
        eprintln!("SKIP: gitignored Macintosh medium missing at {}", path.display());
        return None;
    }
    Some(path)
}

/// What a launch resolved, so a failure can say which link decided.
struct Launch {
    session: GameSession,
    std_window: Option<(u16, u16)>,
    art_scale: Option<(u32, u32)>,
    monochrome: bool,
}

/// Boot the disk exactly as `startup.rs` does, optionally naming an archive by
/// hand (`--pictures`) and optionally overriding the interpreter number.
fn launch(pictures: Option<&str>, honor_game_colours: bool, explicit: Option<u8>) -> Launch {
    let path = mac_disk().expect("caller checked the fixture is present");
    let bytes = match app::hints::load_story(&path).expect("Story.data mounts") {
        app::hints::LoadedStory::ZCode(b) => b,
        other => panic!("expected Z-code, got {other:?}"),
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "this disk carries r{RELEASE}");
    assert_eq!(&bytes[0x12..0x18], SERIAL);

    let dir = std::env::temp_dir().join(format!("babelmap-mac-profile-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let over = match pictures {
        Some(name) => PictureOverride::resolve_with_session(&path, &dir, Some(name)),
        None => PictureOverride::Unset,
    };
    let named_art_std_window = over.std_window();
    let profile = InterpreterProfile::resolve(&path, explicit, over.flavour());
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve_with_override(&path, over);
    let picture_dims = picts.all_pict_dims();
    // The four links, in `startup.rs`'s order.
    let std_window = picts
        .std_window()
        .or(named_art_std_window)
        .or_else(|| picts.native_std_window())
        .or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    let monochrome = picts.is_monochrome();
    let default_colours = honor_game_colours.then(|| profile.default_colours()).flatten();
    let mut session = GameSession::new_with_art_scale(
        bytes,
        honor_game_colours,
        false,
        explicit.or_else(|| profile.interpreter_number()),
        false,
        picture_dims,
        std_window,
        art_scale,
        default_colours,
        None,
        None,
    )
    .expect("Zork Zero boots off the Macintosh disk");
    assert!(!session.quit, "quit during boot");
    assert!(session.machine.fault_trace.is_none(), "faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);
    Launch { session, std_window, art_scale, monochrome }
}

/// Ask the game for its VERSION line, which is where Zork Zero names the machine
/// it thinks it is on.
fn version_line(s: &mut GameSession) -> String {
    let mut said = String::new();
    for _ in 0..24 {
        match s.pending_input() {
            InputKind::Line => {
                said = s.submit("version").transcript;
                break;
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    said.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── The machine ──────────────────────────────────────────────────────────────

/// **The acceptance criterion, in the game's own words.** A profile is a claim
/// about the machine, and header `$1E` is not inert — so the test is not that
/// `resolve` returns a variant, it is that *Zork Zero release 296 reads the byte
/// and says what it read*.
///
/// FALSIFIED IN-LINE by the second half: ask for interpreter 6 on the same disk
/// and the same game says "IBM Interpreter" instead. Before SQ-0838 an HFS
/// volume resolved to the IBM PC default, so the first half of this test is
/// exactly the behaviour that changed.
#[test]
fn zork_zero_off_the_macintosh_disk_says_it_is_on_a_macintosh() {
    if mac_disk().is_none() {
        return;
    }
    let mut mac = launch(None, true, None);
    assert_eq!(mac.session.machine.mem.read_byte(0x1E), 3, "header $1E — ZMSD §11.1.3, 3 = Mac");
    let said = version_line(&mut mac.session);
    assert!(
        said.contains("Macintosh Interpreter version"),
        "the game answered VERSION with {said:?}, which does not name the Macintosh"
    );
    assert!(said.contains("Release 296 / Serial number 881019"), "{said:?}");

    // …and the medium is a DEFAULT, never an override (SQ-0839's other half).
    let mut pc = launch(None, true, Some(6));
    assert_eq!(pc.session.machine.mem.read_byte(0x1E), 6, "the explicit number wins");
    let said = version_line(&mut pc.session);
    assert!(
        !said.contains("Macintosh Interpreter"),
        "asked for interpreter 6 off a Macintosh disk and the game still said {said:?}"
    );
}

/// **Naming one of the disk's own archives must not change what machine this
/// is** (SQ-0843).
///
/// The launch-options dialog now lists both archives on this volume, so the
/// once-theoretical case is a row a person clicks. Until SQ-0843,
/// `InterpreterProfile::resolve` returned on the named archive's flavour before
/// the medium was ever consulted — and `Flavour::AmigaMac` is one container
/// written by two machines, so picking `CPic.data`, the very archive this disk
/// loads on its own, made Zork Zero announce itself on an **Amiga**. The medium
/// is what knows (`for_art_flavour`'s own doc comment said so all along), and
/// this is the acceptance criterion in the game's own words.
///
/// Pinned in both `honor_game_colours` modes: header `$1E` is not a colour and
/// must not move with one, and the two-colour archive turns the flag off at
/// startup on its own, which is exactly the kind of interaction a single-mode
/// suite has masked before.
#[test]
fn naming_either_of_the_disks_archives_still_boots_a_macintosh() {
    if mac_disk().is_none() {
        return;
    }
    for pictures in ["CPic.data", "Pic.data"] {
        for honour in [true, false] {
            let mut l = launch(Some(pictures), honour, None);
            assert_eq!(
                l.session.machine.mem.read_byte(0x1E),
                3,
                "--pictures {pictures} (honour={honour}) must leave header $1E a Macintosh",
            );
            let said = version_line(&mut l.session);
            assert!(
                said.contains("Macintosh Interpreter version"),
                "picked {pictures} off the Macintosh disk and the game said {said:?}",
            );
            assert!(said.contains("Release 296 / Serial number 881019"), "{said:?}");
        }
    }
    // …and the archive still decides the SCREEN, which is the other half: one
    // disk, one machine, two screens (see the test below).
    assert!(launch(Some("Pic.data"), true, None).monochrome);
    assert!(!launch(Some("CPic.data"), true, None).monochrome);
}

/// The rest of the bundle reaches the header: a white page and black ink, which
/// is the Macintosh's whole visual signature and the exact opposite of the
/// Amiga's dark grey.
///
/// Source: `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK; { Mac defaults:
/// white under black }`, with `zWHITE = 9` and `zBLACK = 2`.
///
/// Pinned in BOTH `honor_game_colours` modes, per the project's colour
/// convention: `true` is the shipped default and the machine's own pair, `false`
/// hands the screen back to the user's theme and the profile must not smuggle
/// its colours in behind that switch.
#[test]
fn the_macintosh_page_is_white_and_only_when_game_colours_are_honoured() {
    if mac_disk().is_none() {
        return;
    }
    let honoured = launch(None, true, None);
    assert_eq!(honoured.session.machine.mem.read_byte(0x2C), 9, "header $2C background: white");
    assert_eq!(honoured.session.machine.mem.read_byte(0x2D), 2, "header $2D foreground: black");

    let themed = launch(None, false, None);
    let (bg, fg) = (
        themed.session.machine.mem.read_byte(0x2C),
        themed.session.machine.mem.read_byte(0x2D),
    );
    assert!(
        (bg, fg) != (9, 2),
        "honor_game_colours=false must hand the page back to the theme, got ({bg}, {fg})"
    );
}

// ── The screen, which is the interesting half ────────────────────────────────

/// **The two Macintosh screens, on the real disk.** The colour archive is the
/// big Mac's — 320×200 doubled, the Amiga's own unit space — and the monochrome
/// archive is the standard Mac's 480×300, which does not double at all.
///
/// FALSIFY by restoring `native_std_window`'s old fixed `INFOCOM_V6_STD_WINDOW`:
/// the monochrome launch is then told its screen is 640×400 while its artwork is
/// 480×300, which is the condition SQ-0841's missing right pillar was reported
/// under.
#[test]
fn the_archive_in_hand_picks_which_macintosh_screen_the_game_is_told_about() {
    if mac_disk().is_none() {
        return;
    }

    let colour = launch(None, true, None);
    assert!(!colour.monochrome, "the disk's default archive is the colour one");
    assert_eq!(colour.std_window, Some((320, 200)), "CPic.data's picture space");
    assert_eq!(colour.art_scale, Some((2, 2)), "wx := 2*GFXAM_X — doubled, as on the Amiga");
    assert_eq!(colour.session.machine.mem.read_word(0x22), 640, "screen width, header $22");
    assert_eq!(colour.session.machine.mem.read_word(0x24), 400, "screen height, header $24");
    assert_eq!(colour.session.machine.mem.read_byte(0x21), 80, "columns");
    assert_eq!(colour.session.machine.mem.read_byte(0x20), 25, "rows");

    let mono = launch(Some("Pic.data"), true, None);
    assert!(mono.monochrome, "--pictures Pic.data selects the two-colour archive");
    // Both doors give the same screen: naming the archive by hand (the link
    // that actually fires here) and mounting it off the volume it shipped on.
    // A mono-only Mac disk would reach the second, and must not land elsewhere.
    let mounted = PictSource::resolve_with_override(
        &mac_disk().expect("fixture"),
        PictureOverride::resolve_with_session(
            &mac_disk().expect("fixture"),
            &std::env::temp_dir(),
            Some("Pic.data"),
        ),
    );
    assert_eq!(mounted.native_std_window(), Some((480, 300)), "the archive's own picture space");
    assert_eq!(mounted.art_scale(), Some((1, 1)));
    assert_eq!(mono.std_window, Some((480, 300)), "GFXMAC_X/GFXMAC_Y — '1.5 x Amiga sizes'");
    assert_eq!(mono.art_scale, Some((1, 1)), "IF ge.mono OR myTiny THEN scale 1x for display");
    assert_eq!(mono.session.machine.mem.read_word(0x22), 480, "screen width, header $22");
    assert_eq!(mono.session.machine.mem.read_byte(0x21), 60, "columns");
    // 300 is not a whole number of 16-pixel v6 cells — a real Mac fitted 20 rows
    // of its own 15-pixel Geneva into exactly 300, and babelmap's cell is fixed
    // at 8×16 (see `InterpreterProfile::v6_font_cell`). The screen is rounded to
    // the NEAREST cell, 19 rows, so the game's screen contains its own 300-pixel
    // artwork with four pixels to spare; rounding down would have handed it a
    // 288-pixel screen and clipped the bottom twelve pixels off the plate.
    assert_eq!(mono.session.machine.mem.read_word(0x24), 304, "screen height, header $24");
    assert_eq!(mono.session.machine.mem.read_byte(0x20), 19, "rows");
    assert!(
        mono.session.machine.mem.read_word(0x24) >= 300,
        "the screen must contain the 480×300 plate, never clip it"
    );

    // The two screens really are different screens, which is the whole finding.
    assert_ne!(
        (colour.session.machine.mem.read_word(0x22), colour.session.machine.mem.read_word(0x24)),
        (mono.session.machine.mem.read_word(0x22), mono.session.machine.mem.read_word(0x24)),
    );
}

/// The monochrome plate lands 1:1 on that screen, and the colour plate lands
/// doubled on its own — so in BOTH cases the game's picture table and its screen
/// are in the same space, which is what a v6 game's layout arithmetic assumes.
///
/// This is the invariant SQ-0736 was reported for, generalised: it used to be
/// "art doubles", which was true of every rendition Infocom shipped except one.
#[test]
fn every_macintosh_plate_and_its_screen_are_in_the_same_space() {
    if mac_disk().is_none() {
        return;
    }
    for (name, pictures, space) in
        [("colour", None, (320u16, 200u16)), ("mono", Some("Pic.data"), (480, 300))]
    {
        let l = launch(pictures, true, None);
        let screen = (
            l.session.machine.mem.read_word(0x22),
            l.session.machine.mem.read_word(0x24),
        );
        let (sx, sy) = l.art_scale.expect("a native archive states its density");
        // A full-screen plate, scaled, is the screen — to within the cell
        // rounding the 8×16 grid imposes on the 300-pixel one.
        let drawn = (u32::from(space.0) * sx, u32::from(space.1) * sy);
        assert_eq!(drawn.0, u32::from(screen.0), "{name}: width");
        assert!(
            u32::from(screen.1) >= drawn.1 && u32::from(screen.1) - drawn.1 < 16,
            "{name}: a {drawn:?} plate on a {screen:?} screen is off by more than one cell",
        );
        // …and the game's own picture table is in that same space: picture 1 is
        // the full-screen plate on both archives.
        let first = l
            .session
            .machine
            .picture_dims
            .iter()
            .find(|(n, _, _)| *n == 1)
            .copied()
            .expect("picture 1 is in the table");
        assert_eq!(
            (u32::from(first.1), u32::from(first.2)),
            drawn,
            "{name}: picture_data must report unit-space sizes",
        );
    }
}
