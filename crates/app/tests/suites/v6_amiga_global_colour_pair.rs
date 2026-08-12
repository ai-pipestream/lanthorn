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
//! **And one gate the standard does not mention.** Infocom's own Amiga interpreter
//! (`amiga/yzip3.c`) changes text colours "only in window 0, and ignore[s] requests
//! in other windows (except for the special case of bg = -1)". babelmap implements
//! that gate, because §8.3's stated purpose is to *simulate the Amiga hardware* and
//! a reading of it that diverges from that hardware defeats its own reason for
//! existing. The evidence is Journey: release 30 makes a single `set_colour(9, 2)`
//! — white ink, black page — on **window 3**, and contemporary Amiga walkthrough
//! material shows the game on light grey with white text, i.e. Infocom's own
//! `DEF_BACK 11` / `DEF_FORE 9` defaults. The machine ignored the call.
//!
//! The mechanism, and the retroactive repaint that is the hard half of it, are
//! unit-tested in `zvm::screen` where they live. What this suite adds is the part
//! only a real game can answer: that the OPCODE reaches the rule, that the gate
//! drops Journey's call, that a title which never calls `set_colour` is untouched,
//! that the IBM PC profile — the whole existing v6 corpus — does not move, and that
//! with `honor_game_colours` off the host theme still owns the screen.
//!
//! **…and that any of it reaches the SCREEN.** The first cut of this suite asserted
//! only on the screen MODEL, passed, shipped — and the user saw no change at all:
//! *"journey in amiga mode - i dont see any change from terminal colors."* The pen
//! was never lost. It arrived, and it was invisible, because Journey asks for
//! standard 9 (white) and the host theme's story ink is white too, while the pen's
//! page could not reach a window that never declared one. What was missing was the
//! machine's OWN pair: babelmap advertised the Amiga's `$2C`/`$2D` defaults to the
//! story (§8.3.3) and then painted the host terminal's colours. So the cases below
//! assert on rendered CELLS, not on the model alone.
//!
//! **Media.** Every case drives an Amiga release floppy, named with its release
//! and serial, because a disk image is a different BUILD and not the same story on
//! other media: `Journey - The Quest Begins.adf` is release 30 / serial 890322,
//! not the release 83 of `journey-r83-s890706.z6`. `stories/` is gitignored, so a
//! missing fixture skips vacuously — loudly, on stderr.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::interpreter::InterpreterProfile;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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

// ── (a) The rule itself, on the titles that exercise it ──────────────────────

/// Journey release 30 makes its one and only `set_colour(9, 2)` against window
/// **3**, and Infocom's Amiga interpreter ignores a colour set anywhere but window
/// 0. So the call lands nowhere: no window takes it, no glyph is repainted, and
/// the game is played on the machine's own default pair.
///
/// This assertion is the INVERSE of the one this suite shipped with, which read
/// the standard's "same pair for all windows" without Infocom's gate in front of
/// it and had window 0 adopting standard 9. That is not what the hardware did —
/// see the module docs for the walkthrough evidence — and it is why Journey came
/// out on a black page instead of the Amiga's light grey.
///
/// FALSIFY by deleting the `win != 0` gate in `ScreenState::set_amiga_colour_pair`:
/// every window adopts window 3's ink and this fails immediately.
#[test]
fn a_colour_set_outside_window_0_never_lands_on_an_amiga() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);

    for (i, (fg, bg)) in window_pairs(&s).iter().enumerate() {
        assert_eq!(*fg, ZColour::Default, "{who}: window {i} ink — the window-3 call is ignored");
        assert_eq!(*bg, ZColour::Default, "{who}: window {i} page — the window-3 call is ignored");
    }
    let inks = painted_foregrounds(&s);
    assert!(
        inks.iter().all(|c| c == "Default"),
        "{who}: no glyph may be repainted by a call the machine dropped, got {inks:?}",
    );

    // …and the case is only meaningful with a full frame on screen: window 1
    // carries Journey's whole line-drawing border as painted runs.
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

    // What the screen IS, then: the machine's own pair, straight off header
    // $2D/$2C, which under this profile is Infocom's `DEF_FORE 9` over
    // `DEF_BACK 11` — white on medium grey.
    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine.mem),
        Some((ZColour::Standard(9), ZColour::Standard(11))),
        "{who}: the pair the whole screen is painted with (yzip.h DEF_FORE/DEF_BACK)",
    );
}

/// A colour set from window **0** is the one the machine does take, and §8.3's
/// sharing rule then applies in full: Zork Zero release 366 boots
/// `set_colour(2, 10)` on its story window, and every other window's ink follows
/// the pen with it.
///
/// The page does not spread, and must not: the pens carry ink and page, but a page
/// nobody ever laid down is not a pixel a pen can reach. Filling one in anyway
/// paints the game's own artwork out of the frame.
///
/// FALSIFY by dropping the `repaint_amiga_pens` call: window 1 keeps `Default` ink
/// and the sharing rule is gone, while window 0 alone still shows the colour.
#[test]
fn a_colour_set_from_window_0_moves_the_pen_for_every_window() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&ZORK_ZERO, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&ZORK_ZERO, InterpreterProfile::Amiga, true);
    let pairs = window_pairs(&s);
    assert_eq!(
        pairs[0],
        (ZColour::Standard(2), ZColour::Standard(10)),
        "{who}: the story window keeps the black-on-light-grey scheme the game chose",
    );
    for (i, (fg, _)) in pairs.iter().enumerate() {
        assert_eq!(*fg, ZColour::Standard(2), "{who}: window {i} must share the foreground pen");
    }
    for i in [1usize, 2, 3] {
        assert_eq!(
            pairs[i].1,
            ZColour::Default,
            "{who}: window {i} was never given a background and must not be handed an opaque one",
        );
    }
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

// ── (c) …and on the SCREEN ───────────────────────────────────────────────────

/// A hybrid render at real kitty-ish cell metrics (8×18) — the shipped default
/// mode, and the one the report was filed against. `Picker::halfblocks()` reports
/// a 1×2 cell, a layout regime that reproduces nothing, so the harness has to run
/// at a plausible font cell (the SQ-0548 lesson).
#[allow(deprecated)]
fn render_hybrid(s: &GameSession, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    use app::engine::Engine;
    let model = s.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// The characters Journey draws its frame, rules and menu dividers with under the
/// Amiga profile. (Under IBM PC the same rules are reverse-video spaces — see
/// `v6_journey_amiga_frame.rs` — which is why this set is Amiga-specific.)
const FRAME_GLYPHS: [char; 6] = ['┌', '─', '┐', '│', '└', '┘'];

/// Every distinct `(fg, bg)` a frame glyph is drawn in, and the tally of every
/// cell background in the pane.
type CellTally = (
    std::collections::BTreeMap<(String, String), usize>,
    std::collections::BTreeMap<String, usize>,
);
fn tally(area: Rect, buf: &Buffer) -> CellTally {
    let mut frame: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut pages: std::collections::BTreeMap<String, usize> = Default::default();
    for y in 0..area.height {
        for x in 0..area.width {
            let c = buf.cell((x, y)).expect("in-bounds cell");
            *pages.entry(format!("{:?}", c.bg)).or_default() += 1;
            if FRAME_GLYPHS.contains(&c.symbol().chars().next().unwrap_or(' ')) {
                *frame.entry((format!("{:?}", c.fg), format!("{:?}", c.bg))).or_default() += 1;
            }
        }
    }
    (frame, pages)
}

/// The Amiga's own default pair as the renderer must resolve it: `DEF_FORE 9`
/// (white) over `DEF_BACK 11` (medium grey), through the ACTIVE palette rather
/// than a hardcoded RGB, so a palette change moves the expectation with it.
fn amiga_pair_rgb() -> (String, String) {
    let (r, g, b) = app::colors::standard_colour_rgb(9).expect("standard 9 is white");
    let (gr, gg, gb) = zvm::screen::grey_rgb(11);
    (format!("Rgb({r}, {g}, {b})"), format!("Rgb({gr}, {gg}, {gb})"))
}

/// **The deliverable.** Journey on its Amiga floppy renders white on medium grey —
/// not "in the model", on the cells the terminal is handed.
///
/// This is the case the first cut of SQ-0740 did not have. It shipped on a model
/// assertion, and the report back was *"journey in amiga mode - i dont see any
/// change from terminal colors"*: the pen reached the cells and was invisible
/// there, because the theme already drew white, while the Amiga's own PAGE — the
/// half a player would actually notice — was advertised to the story in header $2C
/// and never painted by anything.
///
/// FALSIFY by making `session::v6_screen_model` publish `Default` for the v6 page
/// again (drop the `amiga_screen_pair` call): every frame glyph comes back as the
/// theme's `White` on `Black`, the pane page goes back to `Reset`, and the Amiga
/// profile is once more indistinguishable from the IBM PC one on screen — which is
/// the user's symptom, verbatim.
#[test]
fn journey_renders_white_on_medium_grey_on_the_amiga_floppy() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, true);
    let (white, grey) = amiga_pair_rgb();

    // Swept across pane sizes: a colour that only lands where the numbers happen to
    // coincide is the bug in another costume (the SQ-0548/SQ-0742 lesson).
    for (cols, rows) in [(115u16, 61u16), (96, 51), (150, 71)] {
        let (area, buf) = render_hybrid(&s, true, cols, rows);
        let (frame, pages) = tally(area, &buf);
        let at = format!("{who} @ {cols}x{rows}");

        // The frame, the rules and the menu dividers: one pair, and it is the
        // machine's. Journey names no colours at all now that its window-3 call is
        // gated away, so every one of these glyphs is an INHERITED channel — which
        // is exactly the channel that used to resolve to the host terminal.
        assert!(!frame.is_empty(), "{at}: no frame glyphs drawn — this case measures nothing");
        assert_eq!(
            frame.keys().cloned().collect::<Vec<_>>(),
            vec![(white.clone(), grey.clone())],
            "{at}: every frame glyph must be the Amiga's white ink on its medium-grey page, got {frame:?}",
        );

        // …and the page runs to the pane, not merely under the glyphs.
        let cells = area.width as usize * area.height as usize;
        let on_page = pages.get(&grey).copied().unwrap_or(0);
        assert!(
            on_page * 2 > cells,
            "{at}: the machine's page must cover the pane — {on_page} of {cells} cells carry it: {pages:?}",
        );
    }
}

/// The IBM PC profile is the whole existing v6 corpus, and none of it moves: the
/// same frame, rendered under interpreter 6, is drawn in the host THEME's colours
/// exactly as it was before this quest existed — and never in the Amiga's grey.
#[test]
fn the_ibm_pc_profile_renders_in_the_host_theme_exactly_as_before() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::IbmPc, true) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::IbmPc, true);
    let (_, grey) = amiga_pair_rgb();
    let (area, buf) = render_hybrid(&s, true, 115, 61);
    let (frame, pages) = tally(area, &buf);
    assert!(!frame.is_empty(), "{who}: no frame glyphs drawn — this case measures nothing");
    assert_eq!(
        frame.keys().cloned().collect::<Vec<_>>(),
        vec![("White".to_string(), "Black".to_string())],
        "{who}: the theme's own named colours, resolved to no RGB at all, got {frame:?}",
    );
    assert_eq!(
        pages.get(&grey).copied().unwrap_or(0),
        0,
        "{who}: no cell may take an Amiga page on a machine that is not an Amiga",
    );
}

/// `honor_game_colours = false` declares the interpreter colourless to the story
/// (§8.3.2 — Flags 1 bit 0 cleared), which is the player saying the host theme owns
/// the screen. A pair the INTERPRETER paints with is still a game colour, so the
/// Amiga's page must not reach past that switch either — and with the flag off
/// `amiga_global_colour_pair` is false at the source, so the machine publishes no
/// pair at all.
#[test]
fn with_game_colours_off_the_amiga_page_never_reaches_the_cells() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(&JOURNEY, InterpreterProfile::Amiga, false) else { return };
    let who = ctx(&JOURNEY, InterpreterProfile::Amiga, false);
    let (_, grey) = amiga_pair_rgb();
    assert_eq!(
        zvm::screen::amiga_screen_pair(&s.machine.mem),
        None,
        "{who}: a colourless interpreter has no pair to paint with",
    );
    // Both render-side settings, because the model keeps what it recorded while the
    // flag was as it was: a mid-game `/set-game-colours` must be honoured by the
    // RENDER, not merely by the boot-time header.
    for honor in [false, true] {
        let (area, buf) = render_hybrid(&s, honor, 115, 61);
        let (_, pages) = tally(area, &buf);
        assert_eq!(
            pages.get(&grey).copied().unwrap_or(0),
            0,
            "{who} (rendered with honor={honor}): the theme owns the screen — no Amiga page anywhere",
        );
    }
}
