//! SQ-0532 wave-3: reproducers for the three v6 render regressions the
//! compliance waves introduced, found by driving the real games with the app's
//! REAL config defaults.
//!
//! # Why the existing v6 smokes all missed these
//!
//! `Config::honor_game_colours` defaults to **true**, but every pre-existing v6
//! test boots with `honor_game_colours = false`. With colours declined the games
//! never call `set_colour`, so the two wave-1 changes below have nothing to
//! propagate and the render looks fine. Turn the flag on — as every real player
//! run does — and Zork Zero's pane goes solid white. The tests here therefore
//! boot with `colours = true`, and the Shogun case renders the SPLASH (which no
//! existing test does) rather than gameplay.
//!
//! # The three failures, and their mechanisms
//!
//! **1 + 2 — Zork Zero is entirely white in raster and hybrid, with no art.**
//! Two independent wave-1 deltas stack:
//!
//! - `erase_window` now blanks a window's character grid with `clear_to(bg)`
//!   instead of `clear()`, stamping an EXPLICIT background colour into every
//!   cell. Zork0 boots with `set_colour(fg=2, bg=9, window=0)`, then
//!   `set_colour(..., window=7)`, then `erase_window(-1)`. §8.8.5.3.1 says -1
//!   erases to window 0's background, so all 2000 cells of window 7 — the
//!   FULL-SCREEN decorative window — become explicitly white. The screen model
//!   paints graphics first and text windows on top, so that stale opaque white
//!   grid covers every picture for the rest of the session. (The erase is
//!   already expressed to the pixel path as a canvas-clear event; stamping the
//!   text grid too applies it twice, on the layer that wins.)
//! - `mirror_v6_colours` now copies the current window's pair into
//!   `screen.current_fg/current_bg`, which `v6_screen_model` publishes as
//!   `ScreenModel.bg/fg` — the pane's page colour. It used to stay
//!   `ZColour::Default` for v6 (nothing wrote it), letting the host theme supply
//!   the page; it is now an explicit white, which floods the raster page.
//!
//! Measured on `zork0-r393-s890714.z6` at 80×30, counting cell backgrounds:
//!
//! | model under test                        | hybrid white / art | raster white |
//! |-----------------------------------------|--------------------|--------------|
//! | as shipped                              | 2400 / 0           | 2400         |
//! | with `ScreenModel.bg` neutralised       |  950 / 0           |    0         |
//! | with the grid cell bgs neutralised      | 1450 / 950         | 2400         |
//! | with both neutralised (≈ pre-wave)      |    0 / 950         |    0         |
//!
//! so BOTH deltas must be corrected — neither alone restores the picture.
//!
//! **3 — Shogun's splash is blank in hybrid** (raster still shows it). Wave 1
//! made v6 window 0 boot at the full screen size (§8.8.3.3, "Window 0 occupies
//! the whole screen") instead of height 0. `v6_screen_model` skips zero-size
//! windows, so before the game splits, window 0 used to be absent, and
//! `classify_windows` reported `story: None` — which is exactly how hybrid mode
//! decides to fall through to the full-art raster path. Now window 0 is present
//! at 640×400, hybrid takes the story-viewport path, and the terminal transcript
//! viewport covers the whole pane with the splash art reduced to a chrome ring
//! of zero thickness. This one reproduces with colours off too.
//!
//! # Status
//!
//! These tests assert the CORRECT behaviour and therefore FAIL on this build.
//! They are `#[ignore]`d so the suite stays green for unrelated work; remove the
//! `#[ignore]` attributes as part of the fix. Run them with:
//!
//! ```text
//! cargo test -p app --test v6_game_colour_regression -- --ignored
//! ```

use std::path::PathBuf;

use app::engine::{Engine, ScreenModel, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way the app does. `colours` is `honor_game_colours`,
/// whose config default is `true`.
fn boot_v6(file: &str, colours: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, colours, false, None, false, dims, picts.std_window(), Some((2, 9)))
        .expect("v6 story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn state_for(mode: app::config::V6RenderMode) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    st.config.v6_render = mode;
    st
}

/// Render `model` into an 80×30 pane and count cell backgrounds:
/// `(pure-white cells, non-white coloured "art" cells)`. In the halfblock cell
/// path the scaled picture arrives as `Rgb` cell backgrounds, so a healthy v6
/// frame has many art cells; a flooded one is uniformly white.
fn bg_census(model: &ScreenModel, mode: app::config::V6RenderMode) -> (usize, usize) {
    let mut state = state_for(mode);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    let (mut white, mut art) = (0, 0);
    for y in 0..area.height {
        for x in 0..area.width {
            match buf.cell((x, y)).unwrap().bg {
                Color::Rgb(255, 255, 255) => white += 1,
                Color::Reset => {}
                _ => art += 1,
            }
        }
    }
    (white, art)
}

/// Regression 2 (hybrid). With the app's real `honor_game_colours = true`,
/// Zork Zero's frame art must still reach the pane. Observed on this build:
/// 2400 white cells, 0 art cells — a solid white pane. With the two wave-1
/// deltas neutralised the same drive yields 0 white / 950 art.
#[test]
#[ignore = "SQ-0532 regression: erase_window clear_to + mirror_v6_colours flood the pane white; un-ignore with the fix"]
fn zork0_hybrid_keeps_its_art_when_game_colours_are_honoured() {
    let Some(mut s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    for _ in 0..3 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit("ne"),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        assert!(r.fault.is_none(), "Zork0 faulted: {:?}", r.fault);
    }
    let (white, art) = bg_census(&s.screen(), app::config::V6RenderMode::Hybrid);
    eprintln!("Zork0 hybrid census: white={white} art={art} (of 2400)");
    assert!(art > 0, "Zork0's frame art vanished from the hybrid pane (white={white}, art={art})");
    assert!(white < 2000, "the hybrid pane is flooded with opaque white (white={white} of 2400)");
}

/// Regression 1 (raster). Same story, raster mode: the page fill must not be
/// forced to the game's white. Observed on this build: 2400 white cells.
#[test]
#[ignore = "SQ-0532 regression: ScreenModel.bg is forced to the game's explicit white; un-ignore with the fix"]
fn zork0_raster_page_is_not_flooded_white_when_game_colours_are_honoured() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let (white, _) = bg_census(&s.screen(), app::config::V6RenderMode::Raster);
    eprintln!("Zork0 raster census: white={white} (of 2400)");
    assert!(white < 2000, "the raster page is flooded with opaque white (white={white} of 2400)");
}

/// The precise mechanism behind both: `erase_window` must not turn a window's
/// whole CHARACTER GRID into opaque explicitly-coloured cells. ZMSD §8.8.5.3.1
/// does say -1 erases the screen to window 0's background — but that is a paint
/// onto the screen/canvas, which the engine already reports separately as a
/// canvas-clear event. Stamping it into the persistent text grid as well makes
/// the grid an opaque layer that the compositor draws OVER every picture, for
/// the rest of the session.
///
/// Zork Zero's window 7 is the full-screen decorative window: after boot its
/// 25×80 grid holds no text at all, yet every one of its 2000 blank cells now
/// carries an explicit background.
#[test]
#[ignore = "SQ-0532 regression: erase_window(-1) stamps an opaque bg into every text-grid cell; un-ignore with the fix"]
fn erase_window_must_not_make_a_blank_text_grid_opaque() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let v6 = s.machine.screen.v6.as_ref().expect("v6 screen");
    let w7 = &v6.windows[7];
    assert_eq!((w7.x_size, w7.y_size), (640, 400), "window 7 is Zork0's full-screen decorative window");
    assert!(w7.texts.is_empty(), "it holds no painted text — it is pure backdrop");
    let explicit = w7
        .grid
        .cells
        .iter()
        .filter(|c| c.bg != zvm::screen::ZColour::Default)
        .count();
    eprintln!("Zork0 window 7: {explicit} of {} blank cells carry an explicit bg", w7.grid.cells.len());
    assert_eq!(
        explicit, 0,
        "a blank erased cell must stay transparent to the compositor, or its window \
         covers the art beneath it ({explicit} of {} cells are opaque)",
        w7.grid.cells.len()
    );
}

/// Regression 3. Shogun's title splash is a full-screen picture drawn before the
/// game splits any window. It must keep reaching the hybrid pane. Reproduces
/// with `honor_game_colours` either way — this is purely the window-0 boot
/// geometry change.
#[test]
#[ignore = "SQ-0532 regression: v6 window 0 booting full-height makes hybrid treat the splash as a story page; un-ignore with the fix"]
fn shogun_splash_still_shows_its_art_in_hybrid() {
    let Some(s) = boot_v6("shogun-r322-s890706.z6", true) else { return };

    // Structural mechanism: at the splash the game has sized nothing, yet
    // window 0 now reports the whole screen and so classifies as the story page.
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items);
    let native = app::render::v6_layout::native_extent(items);
    if let Some(story) = layout.story {
        assert!(
            !(story.w_px >= native.0 && story.h_px >= native.1),
            "before the game splits anything, window 0 covers the entire {native:?} screen \
             ({}x{}), so hybrid renders a story page over the splash instead of showing the art",
            story.w_px,
            story.h_px
        );
    }

    let (white, art) = bg_census(&model, app::config::V6RenderMode::Hybrid);
    eprintln!("Shogun splash hybrid census: white={white} art={art} (of 2400)");
    assert!(art > 200, "Shogun's splash art vanished from the hybrid pane (white={white}, art={art})");
}
