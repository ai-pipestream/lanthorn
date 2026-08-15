//! Shogun's credits/menu screen keeps its side panels and its ground in HYBRID —
//! SQ-0886.
//!
//! The report, comparing an official Amiga screenshot with ours on the same build:
//! one keypress past the splash, the ornate gold-and-black side panels are gone and
//! a full-width black block covers the top half of the screen where the machine's
//! grey ground belongs. Raster mode, on the same frame and from the same bytes, was
//! already right.
//!
//! MEASURED, before the fix, `James Clavell's Shogun.adf` (release 295, serial
//! 890321) at a 100x40 pane under kitty: `#000000` across 761 of 800 columns on
//! device rows 18..179 and not one image placement on the screen, against the
//! original's `#424542` — the Amiga palette's colour 12 — dominating every row with
//! the panels down both edges.
//!
//! WHAT IT WAS. Not the border reader, and not a fill painted over the panels: the
//! hybrid ring never ran at all. A v6 screen that prints chrome text INSIDE the
//! story window's box is classed a painted menu takeover and routed to the all-text
//! CELL path (SQ-0484, so Shogun's boot menu stops being split between the pixel
//! ring and the terminal overlay). That path draws no art whatsoever, so the panels
//! were never composed and the story window's page flooded the pane. SQ-0886
//! keeps the escape and changes its DESTINATION for one case: a takeover screen
//! with the game's own ARTWORK behind it takes the composite, which draws every
//! pixel at the game's own coordinates and is the frame raster mode already shipped.
//!
//! THREE RENDITIONS, because the medium is incidental — the same screen fails the
//! same way from an Amiga floppy, an IBM Blorb and a five-volume ProDOS set, and a
//! fix measured on one proves nothing about the others (CLAUDE.md). Each is named
//! by its exact release. `stories/` is gitignored, so every case skips vacuously.

use std::path::PathBuf;
use std::sync::Mutex;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global (an Amiga medium loads the Amiga
/// palette), so no two renditions may boot at once.
static PALETTE: Mutex<()> = Mutex::new(());

/// One press of Shogun, by the exact build it carries.
struct Rendition {
    /// The file a person names to open it — for the ProDOS set, any one volume.
    file: &'static str,
    release: u16,
    serial: &'static str,
}

/// The three presses in the corpus. The `.po` in `stories/` is a different (and
/// unbootable) image and is deliberately not here.
const RENDITIONS: &[Rendition] = &[
    Rendition { file: "James Clavell's Shogun.adf", release: 295, serial: "890321" },
    Rendition { file: "shogun-r322-s890706.z6", release: 322, serial: "890706" },
    // The five-volume 5.25-inch ProDOS press (SHOGUN.1…SHOGUN.5); `app::hints`
    // mounts the whole set from whichever volume is named.
    Rendition { file: "shogun_s1.dsk", release: 311, serial: "890510" },
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile and the artwork both come from
/// the medium — after checking the build is the one this file measured.
fn boot(r: &Rendition) -> Option<GameSession> {
    let path = stories_dir().join(r.file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        r.release,
        "{}: this medium carries a DIFFERENT build than the table says",
        r.file
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), r.serial, "{}: serial", r.file);
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", r.file));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // One keypress past the title splash is the reported screen: the credits, the
    // prompt and the START / RESTORE / QUIT menu.
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    Some(s)
}

#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn render_state(mode: app::config::V6RenderMode, honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // Halfblocks is the protocol, which is what lets a case assert on the pane's
    // own CELLS: the image lands in them.
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    state
}

/// Render one frame and hand back the pane and the path the render took.
fn frame(session: &GameSession, mode: app::config::V6RenderMode, honor: bool, pane: (u16, u16)) -> (Buffer, String) {
    let model = session.screen();
    let state = render_state(mode, honor);
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    (buf, path)
}

/// Every colour a row shows, left to right, as `(bg, fg)` per cell — the pane as a
/// player sees it, whichever half of a half-block carries the ink.
fn row_colours(buf: &Buffer, y: u16, w: u16) -> Vec<(ratatui::style::Color, ratatui::style::Color)> {
    (0..w).map(|x| (buf[(x, y)].bg, buf[(x, y)].fg)).collect()
}

/// Is this colour the game's own ink rather than a grey? The story page, the theme
/// backdrop and the text are all neutral on this screen in every press, so a
/// channel spread of any size is artwork.
fn is_chromatic(c: &ratatui::style::Color) -> bool {
    let ratatui::style::Color::Rgb(r, g, b) = *c else { return false };
    let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
    hi - lo >= 24
}

/// The premise: this really is the credits/menu screen. The three items are printed
/// through a 1px caret window one glyph at a time, so they are matched as runs.
fn is_menu_screen(session: &GameSession) -> bool {
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { return false };
    let mut seen = String::new();
    for it in items {
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                seen.push_str(&t.text);
            }
        }
    }
    seen.contains("START") && seen.contains("RESTORE") && seen.contains("QUIT")
}

const PANES: &[(u16, u16)] = &[(100, 40), (80, 30), (159, 61)];

// ── 1. The reported symptom, stated directly ────────────────────────────────

/// The game's own COLOURED artwork reaches the pane.
///
/// Every press frames this screen with decoration and none of it is grey: the Amiga
/// and IBM presses run gold-on-black filigree panels down both edges, and the Apple
/// IIgs press lays red-and-green strips above and below the credits. So the test is
/// chromatic, which is what makes it rendition-agnostic — the defect left the pane
/// carrying nothing but the story page, the theme backdrop and white text, all of
/// them grey, and not one coloured pixel of the game's own art anywhere on it.
#[test]
fn the_games_own_artwork_reaches_the_pane() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(session) = boot(r) else { continue };
        assert!(is_menu_screen(&session), "{}: one keypress past the splash is the credits/menu screen", r.file);
        ran += 1;
        for honor in [true, false] {
            for &(w, h) in PANES {
                let (buf, _) = frame(&session, app::config::V6RenderMode::Hybrid, honor, (w, h));
                let coloured: usize = (0..h)
                    .flat_map(|y| row_colours(&buf, y, w))
                    .filter(|(bg, fg)| is_chromatic(bg) || is_chromatic(fg))
                    .count();
                assert!(
                    coloured >= 32,
                    "{} [release {}] hybrid honor={honor} {w}x{h}: the game's artwork is not on the \
                     pane — only {coloured} of {} cells carry a colour that is not a grey. That is the \
                     reported defect: the screen came out as the story window's page and nothing else \
                     (`#000000` across 761 of 800 columns), with the decoration never composed at all.",
                    r.file,
                    r.release,
                    w as usize * h as usize
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun rendition present — every case skipped");
}

// ── 3. Hybrid and raster agree on this frame ────────────────────────────────

/// The quest's own statement: raster was correct and hybrid was not, from the same
/// bytes on the same frame. They must now draw the same screen.
///
/// This is the assertion that pins WHERE the fix lives. A menu takeover with the
/// game's artwork behind it takes the composite in hybrid too, so the two modes
/// resolve to one pane; the cell path that used to take this frame could not have
/// matched it at any pane size, having no pixels to draw at all.
#[test]
fn hybrid_draws_the_frame_raster_already_drew() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for r in RENDITIONS {
        let Some(session) = boot(r) else { continue };
        ran += 1;
        for honor in [true, false] {
            for &(w, h) in PANES {
                let (hy, path) = frame(&session, app::config::V6RenderMode::Hybrid, honor, (w, h));
                assert_eq!(
                    path, "raster",
                    "{} [release {}] hybrid honor={honor} {w}x{h}: a menu takeover over the game's own \
                     artwork takes the COMPOSITE. Routed to the cell path instead (`cell — painted menu \
                     takeover routed here`) it loses every pixel the game drew: no side panels, and the \
                     story window's page flooded across the pane.",
                    r.file,
                    r.release
                );
                let (ra, _) = frame(&session, app::config::V6RenderMode::Raster, honor, (w, h));
                let diff = (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .filter(|&(x, y)| hy[(x, y)] != ra[(x, y)])
                    .count();
                assert_eq!(
                    diff, 0,
                    "{} [release {}] honor={honor} {w}x{h}: hybrid and raster must draw the same \
                     credits screen — {diff} of {} cells differ",
                    r.file,
                    r.release,
                    w as usize * h as usize
                );
            }
        }
    }
    assert!(ran > 0 || !stories_dir().exists(), "no Shogun rendition present — every case skipped");
}

// ── 4. …and nothing else moved ──────────────────────────────────────────────

/// A takeover screen with NO artwork behind it keeps the all-text cell path.
///
/// The escape SQ-0484 added is narrowed, not removed: advent.z6 paints its boot
/// popup over a story window in a game with no artwork in it anywhere, and that
/// screen must still render as one coherent all-text page. `draw_erase_fills` draws
/// its painted panel; there are no pixels for a composite to add.
#[test]
fn a_menu_takeover_with_no_art_still_takes_the_cell_path() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let path = stories_dir().join("advent.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_win, None, None)
        .expect("advent.z6 boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // advent's boot pops a panel over the story on the first turn.
    match s.pending_input() {
        InputKind::Char => s.submit_char(13),
        _ => s.submit(""),
    };
    let (_, path) = frame(&s, app::config::V6RenderMode::Hybrid, true, (100, 40));
    assert!(
        path.starts_with("cell"),
        "advent's boot popup has no artwork behind it, so it keeps the coherent all-text path \
         SQ-0484 put it on — got {path:?}"
    );
}
