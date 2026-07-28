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
//! # Status — FIXED (wave 4), these now guard the fixes
//!
//! 1. The v6 `erase_window` arms went back to `grid.clear()`: a v6 character
//!    grid is a compositing layer drawn OVER the picture canvases, so a blank
//!    cell must stay `ZColour::Default`/transparent. §8.8.5.3's erase colour
//!    still reaches the host — as each window's own `bg` and as the number-0
//!    canvas-clear event the same arm already queues. (The v1–5 upper window
//!    keeps `clear_to(bg)`: it has no art beneath it, so an explicit background
//!    there is exactly §8.7.3.2's "cleared to background colour".)
//! 2. `v6_screen_model` no longer publishes `screen.current_fg/bg` as
//!    `ScreenModel.bg/fg`. That field is the PANE PAGE, and a v6 story has none
//!    — every window carries its own pair (§8.3) and publishes it on its own
//!    node. The §8.3 mirror itself stays: v6 prose runs still get the current
//!    window's pair in their `TextAttrs`.
//! 3. `v6_screen_model` skips window 0 while it is still the untouched
//!    whole-screen boot window (§8.8.3.3) with nothing in it — no painted runs,
//!    no streamed characters, no canvas. `get_wind_prop` keeps reporting the
//!    real 640×400 dimensions (the wave-1 spec fix); only the RENDER declines to
//!    treat an empty placeholder as the story page.
//!
//! Both colour modes are covered: the three cases above run with
//! `honor_game_colours = true` (the shipped default), and each has a
//! theme-only (`false`) companion pinning where the two modes diverge.

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
///
/// The raster composite is resized+encoded on a background worker (SQ-0469) and
/// only appears once `poll_v6_job` installs it, so — like the real app, which
/// polls every frame — this renders until the encode lands before counting.
/// (The hybrid ring's band images are drawn synchronously; the settle only
/// matters for the whole-canvas raster path, which hybrid also falls through to
/// when there is no story window.)
fn bg_census(model: &ScreenModel, mode: app::config::V6RenderMode) -> (usize, usize) {
    let mut state = state_for(mode);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    for _ in 0..200 {
        let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
        if !state.graphics_render.borrow().v6_encode_in_flight() {
            break; // this path draws no whole-canvas raster — nothing to wait for
        }
        if state.graphics_render.borrow_mut().poll_v6_job() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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

/// Regression 2 (hybrid). **Mode: `honor_game_colours = true`** — the shipped
/// default, and the only mode in which this ever broke. Zork Zero's frame art
/// must reach the pane. Broken build: 2400 white / 0 art, a solid white pane.
/// Fixed: 0 white / 950 art.
#[test]
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

/// Regression 1 (raster). **Mode: `honor_game_colours = true`.** The whole PANE
/// must not be repainted in the current window's background. Broken build: 2400
/// white cells (every cell of the pane). Fixed: ~1269 — the raster composite's
/// own story page, which legitimately IS the game's white (SQ-0510,
/// `story_bg_rgba`: a game-set story-window colour wins), surrounded by real
/// frame art. The threshold separates "the game's page inside its frame" from
/// "the pane has been flooded".
#[test]
fn zork0_raster_page_is_not_flooded_white_when_game_colours_are_honoured() {
    let Some(s) = boot_v6("zork0-r393-s890714.z6", true) else { return };
    let (white, art) = bg_census(&s.screen(), app::config::V6RenderMode::Raster);
    eprintln!("Zork0 raster census: white={white} art={art} (of 2400)");
    assert!(white < 2000, "the raster page is flooded with opaque white (white={white} of 2400)");
    assert!(art > 0, "Zork0's frame art vanished from the raster pane (white={white}, art={art})");
}

/// Paired companion to the two above. **Mode: `honor_game_colours = false`** —
/// the theme-only path, where the game's `set_colour` is declined outright.
/// Both modes must reach the pane with real art and no flood; the DIVERGENCE
/// that must persist is only in the model: with colours honoured the story
/// window carries the game's explicit pair (§8.3), with them declined it stays
/// `Default`. Neither may ever become the pane page.
#[test]
fn zork0_renders_its_frame_in_both_colour_modes() {
    for colours in [true, false] {
        let Some(s) = boot_v6("zork0-r393-s890714.z6", colours) else { return };
        let model = s.screen();
        assert_eq!(
            (model.bg, model.fg),
            (0, 0),
            "colours={colours}: a v6 model never publishes a pane page — the host theme owns it"
        );
        let w0 = &s.machine.screen.v6.as_ref().expect("v6 screen").windows[0];
        if colours {
            assert_ne!(w0.bg, zvm::screen::ZColour::Default, "the game's window-0 background is recorded (§8.3)");
        } else {
            assert_eq!(w0.bg, zvm::screen::ZColour::Default, "colours declined: the game never set one");
        }
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            let (white, art) = bg_census(&model, mode);
            eprintln!("Zork0 {mode:?} colours={colours}: white={white} art={art} (of 2400)");
            assert!(art > 0, "colours={colours} {mode:?}: the frame art vanished (white={white}, art={art})");
            assert!(white < 2000, "colours={colours} {mode:?}: the pane is flooded white ({white} of 2400)");
        }
    }
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
/// 25×80 grid holds no text at all, yet every one of its 2000 blank cells used
/// to carry an explicit background.
///
/// **Modes: both.** The grid must be transparent either way; the divergence is
/// the window-level pair, which is where §8.8.5.3's erase colour actually lives
/// (together with the number-0 canvas-clear event on the picture queue).
#[test]
fn erase_window_must_not_make_a_blank_text_grid_opaque() {
    for colours in [true, false] {
        let Some(s) = boot_v6("zork0-r393-s890714.z6", colours) else { return };
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
        eprintln!(
            "Zork0 window 7 (colours={colours}): {explicit} of {} blank cells carry an explicit bg",
            w7.grid.cells.len()
        );
        assert_eq!(
            explicit, 0,
            "colours={colours}: a blank erased cell must stay transparent to the compositor, or its \
             window covers the art beneath it ({explicit} of {} cells are opaque)",
            w7.grid.cells.len()
        );
        // The divergence: the erase colour still reaches the WINDOW when the
        // game's colours are honoured — it just never reaches the grid cells.
        if colours {
            assert_eq!(w7.bg, zvm::screen::ZColour::Standard(9), "the game's window-7 background is kept (§8.3)");
        } else {
            assert_eq!(w7.bg, zvm::screen::ZColour::Default, "colours declined: no window background is set");
        }
    }
}

/// Regression 3. Shogun's title splash is a full-screen picture drawn before the
/// game splits any window. It must keep reaching the hybrid pane.
///
/// **Modes: both** — this one is purely the window-0 boot-geometry change and
/// reproduced with `honor_game_colours` either way, so it is driven in both and
/// the two must agree (Shogun sets no colour at all on this screen).
///
/// The wave-1 §8.8.3.3 fix itself is NOT reverted: window 0 really does report
/// the whole 640×400 screen to `get_wind_prop` (asserted below). It simply must
/// not be RENDERED as the story page while it is an empty placeholder.
#[test]
fn shogun_splash_still_shows_its_art_in_hybrid() {
    for colours in [true, false] {
        let Some(s) = boot_v6("shogun-r322-s890706.z6", colours) else { return };

        // The spec win survives: the model still holds a whole-screen window 0.
        let w0 = &s.machine.screen.v6.as_ref().expect("v6 screen").windows[0];
        assert_eq!(
            (w0.x_size, w0.y_size),
            (640, 400),
            "§8.8.3.3: window 0 still occupies the whole screen in the model"
        );

        // Structural mechanism: an empty whole-screen window 0 must not become
        // the story page, or hybrid carves a pane-sized transcript viewport over
        // the splash and the chrome ring collapses to zero thickness.
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let layout = app::render::v6_layout::classify_windows(items);
        let native = app::render::v6_layout::native_extent(items);
        if let Some(story) = layout.story {
            assert!(
                !(story.w_px >= native.0 && story.h_px >= native.1),
                "colours={colours}: before the game splits anything, window 0 covers the entire \
                 {native:?} screen ({}x{}), so hybrid renders a story page over the splash instead \
                 of showing the art",
                story.w_px,
                story.h_px
            );
        }

        let (white, art) = bg_census(&model, app::config::V6RenderMode::Hybrid);
        eprintln!("Shogun splash hybrid census (colours={colours}): white={white} art={art} (of 2400)");
        assert!(
            art > 200,
            "colours={colours}: Shogun's splash art vanished from the hybrid pane (white={white}, art={art})"
        );
    }
}

/// The other half of fix 3: once the game GIVES window 0 something, it becomes
/// the story window again — the skip is a placeholder rule, not a removal.
/// **Modes: both**, since it is geometry/content driven, not colour driven.
#[test]
fn shogun_gets_its_story_window_back_once_the_game_uses_it() {
    for colours in [true, false] {
        let Some(mut s) = boot_v6("shogun-r322-s890706.z6", colours) else { return };
        for _ in 0..6 {
            let r = match s.pending_input() {
                InputKind::Line => s.submit("look"),
                InputKind::Char => s.submit_char(13),
                InputKind::Event => s.submit(""),
            };
            assert!(r.fault.is_none(), "Shogun faulted: {:?}", r.fault);
        }
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let layout = app::render::v6_layout::classify_windows(items);
        let story = layout.story.expect("colours={colours}: in play, window 0 IS the story window");
        let native = app::render::v6_layout::native_extent(items);
        assert!(
            story.w_px < native.0 || story.h_px < native.1,
            "colours={colours}: and it is the game's own box, not the whole screen"
        );
    }
}
