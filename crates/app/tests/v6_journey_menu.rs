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

/// (d, SQ-0500) In HYBRID mode the bottom command menu is a PURE-TEXT chrome band
/// (no artwork behind it), so it paints as real terminal CELLS — "Proceed" is
/// findable as buffer text in the bottom rows — while the LEFT picture column
/// (win3 graphics) stays in the pixel ring (image half-block cells, not text).
#[test]
fn journey_hybrid_menu_is_terminal_cells_picture_stays_ring() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // "Proceed" is a real menu verb painted as terminal text in the bottom band.
    let menu_text: String = (18..area.height).map(|y| row_text(y) + "\n").collect();
    assert!(
        menu_text.contains("Proceed"),
        "the command menu renders as terminal CELLS ('Proceed' as buffer text); got:\n{menu_text}"
    );
    assert!(menu_text.contains("Praxix"), "party column as cells too:\n{menu_text}");

    // The left picture column stays a pixel-ring image: its cells are half-block
    // graphics (▀/▄) or coloured, NOT terminal text. Scan the upper-left region.
    let is_image_cell = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let sym = c.symbol();
        sym == "\u{2580}" || sym == "\u{2584}" || c.bg != ratatui::style::Color::Reset
    };
    let pic_ink = (1..17)
        .flat_map(|y| (0..24).map(move |x| (x, y)))
        .filter(|&(x, y)| is_image_cell(x, y))
        .count();
    assert!(pic_ink > 0, "the left picture column stays imaged in the pixel ring (got {pic_ink} image cells)");
}

/// (e, SQ-0499 raster) In the pixel canvas Raster mode uploads, the menu header
/// ("The Party" | "Individual Commands", both reverse-video) reads as ONE solid
/// bar — the wide gap between the two labels fills with the reverse block. The
/// menu BODY row (reversed column dividers among NON-reversed verb text) is left
/// alone: the cells between its dividers stay bare, not a full-width bar.
#[test]
fn journey_raster_reverse_header_bar_is_solid_body_untouched() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items);
    let layout = v6::classify_windows(items);
    let colors = app::colors::ColorScheme::terminal_default();
    let canvas = v6::build_chrome_canvas(
        &layout.chrome, native, image::Rgba([220, 220, 220, 255]), image::Rgba([0, 0, 0, 255]), &colors,
    );
    // A whole 8-px cell is "filled" when every pixel column has opaque ink.
    let cell_filled = |cx: u32, row: u32| -> bool {
        let (x0, y0) = (cx * 8, row * 16);
        (x0..(x0 + 8).min(canvas.width())).all(|x| {
            (y0..(y0 + 16).min(canvas.height())).any(|y| canvas.get_pixel(x, y)[3] >= 128)
        })
    };
    // Header row 19: "The Party" ends ~col28, "Individual Commands" starts ~col47.
    // Every cell between them (the old bare gap) must now be a filled block.
    let header_gap_solid = (30..46).all(|cx| cell_filled(cx, 19));
    assert!(header_gap_solid, "row-19 header gap between the two labels fills into one solid reverse bar");
    // Body row 20 has reversed dividers at cols ~15/31/47/63 with normal verb text
    // between — the cells mid-column (e.g. 35..46, between two dividers, no text)
    // must NOT be block-filled (that row is mixed, not a pure reverse bar).
    let body_between_dividers_bare = (35..46).any(|cx| !cell_filled(cx, 20));
    assert!(body_between_dividers_bare, "row-20 menu body stays normal (not over-filled) between its dividers");
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
