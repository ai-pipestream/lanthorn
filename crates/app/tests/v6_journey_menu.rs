//! Journey (Infocom v6) bottom command menu — SQ-0492.
//!
//! Journey drives all gameplay through a bottom command-menu window (win1:
//! "Proceed/Back/Game", the party names, verb columns), painted as absolute
//! pixel-positioned text runs at native rows 19–24. That window is FULL-WIDTH
//! but sized to HEIGHT 0 in the window property table — its menu is pure v6
//! PAINT (persists at its screen-absolute pixels regardless of the zero size).
//!
//! The pre-fix `v6_screen_model` skipped every window with `x_size==0 ||
//! y_size==0`, so the whole menu was dropped before it ever reached the
//! ScreenModel — no render mode showed it. The fix keeps a zero-size window
//! that still holds painted runs, and grows the native raster extent to cover
//! those runs so the menu rasterizes into the bottom band.
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Journey and drive the intro vignettes until it sits in `read_char` on
/// the Praxix command-menu page (the menu painted, awaiting a keypress).
fn journey_at_menu() -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Journey (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap Enter through the intro vignettes until the command menu is up.
    for _ in 0..40 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.contains("Praxix") || r.transcript.contains("magical resources") {
            break;
        }
    }
    Some(session)
}

/// All non-blank grid px-text runs in the model, with their native row.
fn deep_runs(model: &app::engine::ScreenModel) -> Vec<(u16, String)> {
    let WinNode::Layered(items) = &model.root else { return Vec::new() };
    let mut out = Vec::new();
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if !t.text.trim().is_empty() {
                    out.push(((t.y.max(1) - 1) / 16, t.text.clone()));
                }
            }
        }
    }
    out
}

/// (a) The command menu reaches the ScreenModel as positioned grid runs at
/// native rows ≥ 19 — including the "Proceed" verb. Pre-fix this was empty.
#[test]
fn journey_menu_reaches_the_model_as_deep_grid_runs() {
    let Some(session) = journey_at_menu() else { return };
    assert_eq!(session.pending_input(), InputKind::Char, "menu sits in read_char");

    let model = session.screen();
    let runs = deep_runs(&model);
    let deep: Vec<&(u16, String)> = runs.iter().filter(|(row, _)| *row >= 19).collect();
    assert!(
        !deep.is_empty(),
        "the command menu must reach the model as deep grid runs (row ≥ 19); got {runs:?}"
    );
    assert!(
        deep.iter().any(|(_, t)| t.contains("Proceed")),
        "the 'Proceed' menu verb is a deep run; got {deep:?}"
    );
    // The party column ("Bergon"/"Praxix") and the "Game" verb are there too.
    assert!(deep.iter().any(|(_, t)| t.contains("Praxix")), "party name present: {deep:?}");
    assert!(deep.iter().any(|(_, t)| t.contains("Game")), "'Game' verb present: {deep:?}");
}

/// (b, Raster) The menu rasterizes into the bottom band of the native chrome
/// canvas — the exact canvas Raster mode uploads. Pre-fix the canvas was too
/// short (no window reached below native y≈304) so rows 19–24 were blank.
#[test]
fn journey_menu_rasterizes_into_the_bottom_band() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    let native = v6::native_extent(items);
    assert!(
        native.1 >= 385,
        "native extent must cover the menu runs (bottom at native y≈385); got {native:?}"
    );

    let layout = v6::classify_windows(items);
    let colors = app::colors::ColorScheme::terminal_default();
    let canvas = v6::build_chrome_canvas(
        &layout.chrome,
        native,
        image::Rgba([220, 220, 220, 255]),
        image::Rgba([0, 0, 0, 255]),
        &colors,
    );
    // Count opaque ink in the bottom band (native rows 19–24 → y 304..).
    let y0 = 19 * 16u32;
    let inked = (y0..canvas.height())
        .flat_map(|y| (0..canvas.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| canvas.get_pixel(x, y)[3] >= 128)
        .count();
    assert!(
        inked > 0,
        "the menu must rasterize opaque ink into the bottom band (native rows ≥ 19); got {inked} inked pixels"
    );
}

/// (b, Hybrid) In HYBRID mode the menu is BELOW the story window (rows 0–18),
/// so the screen takes the pixel-chrome RING path (not the menu-screen text
/// gate, SQ-0494) and rasterizes the menu into the bottom terminal band. The
/// bottom rows must carry rendered ink (colored halfblock cells) — pre-fix they
/// were empty.
#[test]
fn journey_hybrid_ring_shows_the_menu_band() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // A cell has ink when its symbol is not a blank space or its style set a
    // concrete (non-Reset) colour — the halfblocks encoder writes '▀' cells with
    // fg/bg colours for the rasterized menu.
    let has_ink = |x: u16, y: u16| -> bool {
        let Some(c) = buf.cell((x, y)) else { return false };
        let sym = c.symbol();
        let blank = sym == " " || sym.is_empty();
        let default_style = c.fg == ratatui::style::Color::Reset && c.bg == ratatui::style::Color::Reset;
        !blank || !default_style
    };
    let band_ink: usize = (19..25)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| has_ink(x, y))
        .count();
    assert!(
        band_ink > 0,
        "the hybrid ring must rasterize the menu into the bottom band (rows 19–24); got {band_ink} inked cells"
    );
}

/// (c) The menu is live through the app layer: a keypress (arrow) changes the
/// painted runs and the game stays in `read_char` (still on the menu).
#[test]
fn journey_menu_is_live_through_the_app_layer() {
    let Some(mut session) = journey_at_menu() else { return };
    assert_eq!(session.pending_input(), InputKind::Char);

    let sig = |s: &GameSession| -> Vec<(u16, u16, u8, String)> {
        let mut v: Vec<_> = s.machine.screen.v6.as_ref().unwrap().windows[1]
            .texts
            .iter()
            .map(|t| (t.y, t.x, t.style, t.text.clone()))
            .collect();
        v.sort();
        v
    };
    let before = sig(&session);
    // ZSCII 130 = down-arrow (v6 read_char terminating key).
    let r = session.submit_char(130);
    assert!(!r.quit, "an arrow keypress must not quit the game");
    assert_eq!(session.pending_input(), InputKind::Char, "still on the menu after the arrow");
    assert_ne!(before, sig(&session), "the arrow moved the selection — the painted runs changed");
}
