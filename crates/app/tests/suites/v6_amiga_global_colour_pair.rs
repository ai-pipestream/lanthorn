//! ZMSD §8.3's Amiga rule, on the real release floppies — SQ-0740.
//!
//! §8.3 gives every Version 6 window its own foreground/background pair, and then
//! names one machine where that is wrong:
//!
//! > "Note that a Version 6 interpreter going under the Amiga interpreter number
//! > must use the same pair of colours for all windows when running Infocom's
//! > games. If either is changed, then the interpreter must change the colour of
//! > all text on the screen to match. This simulates the Amiga hardware, which
//! > used two logical colours for text and switched palette to change their
//! > physical colour."
//!
//! The mechanism, and the retroactive repaint that is the hard half of it, are
//! unit-tested in `zvm::screen` where they live. What this suite adds is the part
//! only a real game can answer: that the OPCODE reaches the rule, that a title
//! which never calls `set_colour` is untouched by it, that the IBM PC profile —
//! the whole existing v6 corpus — does not move, and that with
//! `honor_game_colours` off the host theme still owns the screen.
//!
//! **Media.** Every case drives an Amiga release floppy, named with its release
//! and serial, because a disk image is a different BUILD and not the same story on
//! other media: `Journey - The Quest Begins.adf` is release 30 / serial 890322,
//! not the release 83 of `journey-r83-s890706.z6`. `stories/` is gitignored, so a
//! missing fixture skips vacuously — loudly, on stderr.
//!
//! **Why Journey is the load-bearing case.** It is the one title in the corpus
//! that colours a window OTHER than the one its text lives in: release 30 makes a
//! single `set_colour(9, 2)` against window 3 and paints its whole line-drawing
//! frame through window 1. Per §8.3 that one call is the machine's pens, so the
//! frame, the prose and every other window take standard 9 with it. Without the
//! rule they keep `Default` and render in the host theme — the game asked for
//! white and got whatever the terminal felt like, which is the shape of the
//! original report.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::interpreter::InterpreterProfile;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use zvm::screen::ZColour;

/// `zvm::screen::set_palette` is process-global (the profile's colour numbers
/// resolve through it), so a case that boots one profile must not run beside a
/// case that boots the other.
static PALETTE: Mutex<()> = Mutex::new(());

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One Amiga release floppy, pinned to the build it carries. The table in
/// `real_media_releases.rs` is the authority; these rows repeat it so a failure
/// here names the release it measured, which is the single most expensive kind of
/// failure this project has had.
struct Floppy {
    file: &'static str,
    release: u16,
    serial: &'static str,
    turns: usize,
}

const JOURNEY: Floppy = Floppy {
    file: "Journey - The Quest Begins.adf",
    release: 30,
    serial: "890322",
    turns: 12,
};
const ZORK_ZERO: Floppy = Floppy {
    file: "Zork Zero - The Revenge of Megaboz.adf",
    release: 366,
    serial: "890323",
    turns: 12,
};
const ARTHUR: Floppy =
    Floppy { file: "Arthur - The Quest for Excalibur.adf", release: 54, serial: "890606", turns: 12 };

fn ctx(f: &Floppy, profile: InterpreterProfile, honor: bool) -> String {
    format!(
        "{} [release {}, serial {} — {profile:?} profile, honor_game_colours={honor}]",
        f.file, f.release, f.serial
    )
}

/// Boot `f` off its floppy under `profile` and drive it to a settled frame, or
/// `None` when the gitignored medium is absent.
fn boot(f: &Floppy, profile: InterpreterProfile, honor: bool) -> Option<GameSession> {
    let path = stories_dir().join(f.file);
    let bytes = match app::hints::load_mounted_story(&path) {
        Ok((loaded, _)) => loaded.bytes().to_vec(),
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    // The build in hand is the build the row names. Nothing measured after this
    // can be attributed to the wrong release.
    assert_eq!(bytes[0], 6, "{}: Z-machine version", ctx(f, profile, honor));
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        f.release,
        "{}: this medium carries a DIFFERENT build than the row says",
        ctx(f, profile, honor)
    );
    assert_eq!(
        String::from_utf8_lossy(&bytes[0x12..0x18]),
        f.serial,
        "{}: serial",
        ctx(f, profile, honor)
    );

    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        honor,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(f, profile, honor)));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    for _ in 0..f.turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        assert!(!s.quit, "{}: quit while driving", ctx(f, profile, honor));
        assert!(s.machine.fault_trace.is_none(), "{}: faulted while driving", ctx(f, profile, honor));
    }
    Some(s)
}

/// Every window's foreground, and every window's background, as the v6 screen
/// model holds them.
fn window_pairs(s: &GameSession) -> Vec<(ZColour, ZColour)> {
    s.machine.screen.v6.as_ref().expect("a v6 story").windows.iter().map(|w| (w.fg, w.bg)).collect()
}

/// Every distinct FOREGROUND carried by a glyph that is currently painted on the
/// screen — the grids, the pixel-positioned runs, the streamed prose and the
/// prose frozen behind a window that moved.
fn painted_foregrounds(s: &GameSession) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for w in &s.machine.screen.v6.as_ref().expect("a v6 story").windows {
        for c in w.grid.cells.iter().filter(|c| c.ch != ' ') {
            out.insert(format!("{:?}", c.fg));
        }
        for t in w.texts.iter().chain(w.streamed.iter()).chain(w.retired.iter()) {
            out.insert(format!("{:?}", t.fg));
        }
    }
    out
}

// ── (a) The rule itself, on the title that exercises it ──────────────────────

/// Journey release 30 colours window **3** and paints through window **1**. Under
/// the Amiga interpreter number that one call is the machine's two pens, so every
/// window — and every glyph already drawn — carries standard 9.
///
/// FALSIFY by restoring the per-window `set_colour` arm in `zvm/src/cpu/exec.rs`
/// (delete the `amiga_global_colour_pair` branch): window 1's several hundred
/// frame runs come back as `Default`, and this fails on the "one foreground for
/// the whole screen" assertion — the game asked for white and the screen shows
/// the host theme instead.
#[test]
fn one_pair_serves_every_window_of_an_amiga_release() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);

    // The game really did make the call this case is about.
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[3].0,
        ZColour::Standard(9),
        "{who}: window 3 is the one the game coloured, and it must hold what it asked for",
    );

    // §8.3: "the same pair of colours for all windows".
    for (i, (fg, _)) in pairs.iter().enumerate() {
        assert_eq!(*fg, ZColour::Standard(9), "{who}: window {i} must share the foreground pen");
    }

    // …and "the colour of all text on the screen" matches it. Window 1 carries
    // the whole line-drawing frame, which is the bulk of the glyphs on screen.
    let inks = painted_foregrounds(&s);
    assert_eq!(
        inks,
        ["Standard(9)".to_string()].into_iter().collect(),
        "{who}: every glyph on the screen must be drawn in the one foreground pen, got {inks:?}",
    );
    let painted: usize = s
        .machine
        .screen
        .v6
        .as_ref()
        .unwrap()
        .windows
        .iter()
        .map(|w| w.texts.len() + w.streamed.len() + w.retired.len())
        .sum();
    assert!(painted > 100, "{who}: this case is only meaningful with a full frame drawn: {painted} runs");
}

/// A window the game never gave a background keeps painting nothing behind its
/// glyphs. The pens carry ink and page; a page nobody ever laid down is not a
/// pixel the pen can reach, and filling one in anyway paints Journey's own
/// illustration out of the frame.
#[test]
fn the_shared_pair_lays_down_no_page_that_was_not_there() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);
    let pairs = window_pairs(&s);
    for i in [0usize, 1, 2] {
        assert_eq!(
            pairs[i].1,
            ZColour::Default,
            "{who}: window {i} was never given a background and must not be handed an opaque one",
        );
    }
    assert_eq!(pairs[3].1, ZColour::Standard(2), "{who}: the window that DID ask keeps its page");
}

/// Zork Zero release 366 prints its banner labels over the ribbon artwork with a
/// background of **-1** — "the colour of the pixel under the cursor" (§8.3.1).
/// That names no colour, so it loads no pen, and the labels stay transparent no
/// matter what the pens do. (Infocom's own Amiga interpreter carves the same case
/// out, in `amiga/yzip3.c`.)
#[test]
fn a_label_drawn_over_artwork_stays_over_the_artwork() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&ZORK_ZERO, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ZORK_ZERO, InterpreterProfile::Amiga, true);
    let v6 = s.machine.screen.v6.as_ref().unwrap();
    let banner = &v6.windows[1];
    assert!(!banner.texts.is_empty(), "{who}: the banner window must have painted its labels");
    for t in &banner.texts {
        assert_eq!(
            t.bg,
            ZColour::Default,
            "{who}: banner label {:?} must keep the artwork showing through it",
            t.text,
        );
        assert_eq!(t.fg, ZColour::Standard(2), "{who}: …in the ink the game selected");
    }
    // The prose page, meanwhile, is a real background and keeps it.
    assert_eq!(
        (v6.windows[0].fg, v6.windows[0].bg),
        (ZColour::Standard(2), ZColour::Standard(10)),
        "{who}: the story window keeps the black-on-light-grey scheme the game chose",
    );
}

/// A title that never calls `set_colour` is not touched by a rule about calling
/// it. Arthur release 54 makes none, on either profile.
#[test]
fn a_title_that_never_sets_a_colour_is_untouched() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&ARTHUR, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ARTHUR, InterpreterProfile::Amiga, true);
    for (i, (fg, bg)) in window_pairs(&s).iter().enumerate() {
        assert_eq!(*fg, ZColour::Default, "{who}: window {i} foreground");
        assert_eq!(*bg, ZColour::Default, "{who}: window {i} background");
    }
}

// ── (b) What must NOT move ───────────────────────────────────────────────────

/// The IBM PC profile is the whole existing v6 corpus, and §8.3's carve-out names
/// the Amiga alone. Under interpreter 6 every window keeps its own pair: Journey
/// colours window 3 and window 3 only.
///
/// FALSIFY by dropping the interpreter-number term from
/// `zvm::screen::amiga_global_colour_pair`: this fails immediately, with window 0
/// having quietly adopted window 3's ink.
#[test]
fn the_ibm_pc_profile_keeps_one_pair_per_window() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::IbmPc, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::IbmPc, true);
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[3],
        (ZColour::Standard(9), ZColour::Standard(2)),
        "{who}: the window the game coloured still takes the colour",
    );
    for i in [0usize, 1, 2, 4, 5, 6, 7] {
        assert_eq!(
            pairs[i],
            (ZColour::Default, ZColour::Default),
            "{who}: window {i} must be untouched by a colour set on window 3",
        );
    }
    // And the text on it is drawn in the interpreter's default ink, exactly as
    // before — no pen ever reached it.
    assert!(
        painted_foregrounds(&s).iter().all(|c| c == "Default"),
        "{who}: no glyph may adopt another window's colour",
    );
}

/// `honor_game_colours = false` declares the interpreter colourless to the story
/// (§8.3.2 — Flags 1 bit 0 is cleared), which is the user saying the host theme
/// owns the screen. The Amiga rule must not reach past that switch, on either
/// profile: a shared pen is still a game colour.
#[test]
fn with_game_colours_off_the_theme_owns_the_screen_on_both_profiles() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for profile in [InterpreterProfile::IbmPc, InterpreterProfile::Amiga] {
        let Some(s) = boot(&JOURNEY, profile, false) else { return };
        let who = ctx(&JOURNEY, profile, false);
        let pairs = window_pairs(&s);
        for i in [0usize, 1, 2, 4, 5, 6, 7] {
            assert_eq!(
                pairs[i].0,
                ZColour::Default,
                "{who}: window {i} must keep the theme's ink when colours are off",
            );
        }
        assert!(
            painted_foregrounds(&s).iter().all(|c| c == "Default"),
            "{who}: no glyph may be repainted by a rule the player switched off",
        );
    }
}
