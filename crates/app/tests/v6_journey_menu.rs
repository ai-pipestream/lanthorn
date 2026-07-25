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

/// (e, SQ-0504 raster) In the pixel canvas Raster mode uploads, the menu header
/// ("The Party" | "Individual Commands", both reverse-video) reads as ONE solid
/// bar spanning the ENTIRE screen width — the reverse block fills from the left
/// edge, between the two labels, and out to the right edge, not just the span
/// between them. The menu BODY row (reversed column dividers among NON-reversed
/// verb text) is left alone: the cells between its dividers stay bare, not a bar.
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
    let ncols = (canvas.width() / 8) as u32;
    // A whole 8-px cell is "filled" when every pixel column has opaque ink.
    let cell_filled = |cx: u32, row: u32| -> bool {
        let (x0, y0) = (cx * 8, row * 16);
        (x0..(x0 + 8).min(canvas.width())).all(|x| {
            (y0..(y0 + 16).min(canvas.height())).any(|y| canvas.get_pixel(x, y)[3] >= 128)
        })
    };
    // Header row 19 is a pure reverse bar: EVERY cell from the left edge to the
    // right edge is a filled block — the leading gap (cols 0.., before "The
    // Party"), the gap between the labels, and the trailing gap (past "Individual
    // Commands") all fill, so the whole native row is solid.
    let header_full_width = (0..ncols).all(|cx| cell_filled(cx, 19));
    assert!(header_full_width, "row-19 header fills into ONE solid reverse bar across the full screen width");
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

/// (f, SQ-0504 hybrid) The menu header reverse bar spans the ENTIRE pane width in
/// the hybrid cell path: on the terminal row carrying "The Party", every cell from
/// the left edge to the right edge is reverse-video — the bar reads edge to edge,
/// not just between the two labels.
#[test]
fn journey_hybrid_header_bar_spans_full_pane_width() {
    use ratatui::style::Modifier;
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
    let header_y = (0..area.height).find(|&y| row_text(y).contains("The Party")).expect("header row present");
    let holes: Vec<u16> = (0..area.width)
        .filter(|&x| !buf.cell((x, header_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the header reverse bar spans the full pane [0,{}) with no gaps; non-reversed cols {holes:?}\nrow: {:?}",
        area.width,
        row_text(header_y)
    );
}

/// (h, SQ-0508) At a pane size whose letterbox scale spreads the menu's native rows
/// across MORE terminal rows (100×30 → scale 1.2, so native rows 19–24 map to
/// terminal rows 23,24,25,26,gap-27,28,29), the whole menu panel reads as ONE solid
/// black block and the reversed vertical column dividers stay CONTINUOUS through the
/// scale-introduced gap row — no broken lines, no theme backdrop through the gaps.
#[test]
fn journey_hybrid_menu_full_black_panel_dividers_continuous() {
    use ratatui::style::{Color, Modifier};
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 100×30 → pane_dev 800×480, native 640×400, scale = min(1.25, 1.2) = 1.2:
    // the menu's six native rows spread to seven terminal rows (one is a gap).
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let reversed = |x: u16, y: u16| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED);

    let header_y = (0..area.height).find(|&y| row_text(y).contains("The Party")).expect("header row");
    let game_y = (header_y..area.height).find(|&y| row_text(y).contains("Game")).expect("last menu row (Game)");
    assert!(game_y > header_y + 4, "the menu spans several rows (header {header_y}, Game {game_y})");

    // (a) The ENTIRE menu panel is a black background — every cell in the strip
    // (reversed or not, the stored bg stays black under the REVERSED modifier) is
    // Color::Black, so no theme backdrop shows through the gaps between the runs.
    for y in header_y..=game_y {
        for x in 0..area.width {
            assert_eq!(
                buf.cell((x, y)).unwrap().bg,
                Color::Black,
                "menu cell ({x},{y}) must be on the black panel bg; row: {:?}",
                row_text(y)
            );
        }
    }

    // (b) Divider continuity. Collect the reversed column dividers from a body row
    // (the "Proceed" row), then assert EVERY menu body row — including the blank
    // scale-introduced gap row — is reversed at each of those columns.
    let proceed_y = (header_y..=game_y).find(|&y| row_text(y).contains("Proceed")).expect("Proceed row");
    let dividers: Vec<u16> = (0..area.width)
        .filter(|&x| reversed(x, proceed_y) && row_text(proceed_y).chars().nth(x as usize) == Some(' '))
        .collect();
    assert!(dividers.len() >= 4, "the menu body has ≥4 reversed column dividers; got {dividers:?}");
    // The gap row: a menu body row whose text is entirely blank (no verb text) — it
    // exists only because the scale spread the rows. It MUST still carry the dividers.
    let gap_y = ((header_y + 1)..game_y).find(|&y| row_text(y).trim().is_empty());
    assert!(gap_y.is_some(), "the scale-spread menu has a blank gap row between body rows");
    for y in (header_y + 1)..=game_y {
        for &x in &dividers {
            assert!(
                reversed(x, y),
                "column divider at col {x} must stay reversed on row {y} (continuous vertical line); row: {:?}",
                row_text(y)
            );
        }
    }
}

/// (i, SQ-0509 regression guard) The menu items are SEPARATE runs with real pixel
/// gaps + reversed dividers between them — the SQ-0509 fragment-merging must NOT
/// blob them into one word. Even at scale 1.2 the body row still reads its verbs as
/// distinct, space-separated tokens ("Praxix", "Cast", "Examine"), and the dividers
/// keep them apart.
#[test]
fn journey_hybrid_menu_items_not_merged() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let body: String = (0..area.height).map(|y| row_text(y) + "\n").collect();
    // Each verb is present as its own token, and adjacent items never fuse.
    for verb in ["Proceed", "Bergon", "Praxix", "Cast", "Examine", "Inventory"] {
        assert!(body.contains(verb), "menu verb {verb:?} present as its own run; got:\n{body}");
    }
    assert!(!body.contains("PraxixCast") && !body.contains("CastExamine"), "adjacent menu items must NOT merge:\n{body}");
    // The Praxix row keeps its items spaced apart (a divider/space between them).
    let prow = (0..area.height).map(row_text).find(|r| r.contains("Praxix")).expect("Praxix row");
    let pi = prow.find("Praxix").unwrap();
    let ci = prow.find("Cast").unwrap();
    assert!(ci > pi + "Praxix".len() + 1, "Praxix and Cast stay separated by a gap; row: {prow:?}");
}

/// (g, SQ-0504) The pixel-band canvas EXCLUDES the pure-text menu rows: after the
/// text-strip rows are carved out (exactly what the hybrid path feeds its bands),
/// the menu's native rows carry NO ink — so the rasterized menu can never appear
/// behind the terminal cells — while Journey's authentic vertical picture/text
/// divider (reversed runs at native x≈233, beside the story box, NOT a text strip)
/// stays inked and thus keeps rendering in the left picture-column ring.
#[test]
fn journey_pixel_band_canvas_excludes_menu_keeps_divider() {
    use app::render::v6_layout as v6;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items);
    let layout = v6::classify_windows(items);
    let story = layout.story.expect("story window");
    let story_bottom = (story.y_px + story.h_px) as u32;
    let colors = app::colors::ColorScheme::terminal_default();
    let mut canvas = v6::build_chrome_canvas(
        &layout.chrome, native, image::Rgba([220, 220, 220, 255]), image::Rgba([0, 0, 0, 255]), &colors,
    );

    // The menu runs sit BELOW the story box; collect their native tops.
    let menu_tops: Vec<u16> = layout
        .chrome
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
        .filter(|t| (t.y.max(1) - 1) as u32 >= story_bottom)
        .map(|t| t.y.max(1) - 1)
        .collect();
    assert!(!menu_tops.is_empty(), "menu runs below the story box present");

    let row_has_ink = |c: &image::RgbaImage, y: u32| (0..c.width()).any(|x| c.get_pixel(x, y)[3] >= 128);
    // Before carving: the menu rows are inked (rasterized text).
    let a_menu_row = menu_tops.iter().map(|&t| t as u32 + 4).min().unwrap();
    assert!(row_has_ink(&canvas, a_menu_row), "menu row {a_menu_row} is rasterized before carving");
    // The divider column (native x≈233) has ink across the story rows.
    let divider_ink_before = (0..story_bottom).filter(|&y| canvas.get_pixel(233, y)[3] >= 128).count();
    assert!(divider_ink_before > 200, "the vertical divider is inked before carving ({divider_ink_before} rows)");

    // Carve the menu rows out — exactly what screen.rs feeds the pixel bands.
    v6::clear_text_rows(&mut canvas, &menu_tops);

    // Every carved menu row is now fully transparent.
    for &top in &menu_tops {
        for y in top as u32..(top as u32 + 16).min(canvas.height()) {
            assert!(!row_has_ink(&canvas, y), "menu native row {y} must be carved out of the band canvas");
        }
    }
    // The divider is UNTOUCHED — it is beside the story, not a text strip.
    let divider_ink_after = (0..story_bottom).filter(|&y| canvas.get_pixel(233, y)[3] >= 128).count();
    assert_eq!(divider_ink_before, divider_ink_after, "the picture/text divider survives the carve (stays in the ring)");
}

/// (SQ-0505 dynamic hybrid layout) At a TALL pane (90×40 — vertical letterbox
/// slack below the native 8:5 frame) Journey has BOTTOM TEXT CHROME: the command
/// menu. The menu strip anchors to the pane BOTTOM edge and the story viewport
/// fills the space between the top-anchored chrome and the menu at constant width.
#[test]
fn journey_hybrid_tall_pane_menu_pinned_to_bottom() {
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // The command menu renders as terminal CELLS pinned at the pane bottom: the
    // "Proceed" verb sits in the last few rows, and the menu reaches the very
    // bottom row (its dead space below the letterboxed menu is reclaimed).
    let proceed_y = (0..area.height).find(|&y| row_text(y).contains("Proceed")).expect("'Proceed' renders as terminal cells");
    assert!(proceed_y >= area.height - 8, "the menu is anchored to the pane bottom (Proceed at row {proceed_y} of {})", area.height);
    let last_text_y = (0..area.height).filter(|&y| !row_text(y).trim().is_empty()).max().unwrap();
    assert!(last_text_y >= area.height - 2, "the menu strip reaches the pane bottom edge (last text row {last_text_y})");

    // The story viewport fills the space ABOVE the menu, from near the pane top.
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    assert!(vp.y <= 2, "the story is top-anchored (viewport top {} near the pane top)", vp.y);
    assert!(vp.bottom() <= proceed_y, "the story viewport stops above the bottom-anchored menu (vp bottom {}, menu at {proceed_y})", vp.bottom());
    assert!(vp.x > 0 && vp.right() < area.right(), "the story keeps its constant inset beside the picture column; got {vp:?}");
}

/// (SQ-0511 divider continuity) At a TALL pane the reclaimed space between the story
/// and the bottom-anchored menu is now spanned by the STRETCHED left flank band: the
/// picture column AND its authentic vertical divider (reversed text runs at native
/// x≈233, rasterized into the chrome canvas → part of the left side band, not a text
/// strip) run UNBROKEN from the frame top down to the menu strip. Probe first: the
/// divider is left-flank ring ART (image cells), not terminal text; then assert the
/// left flank carries a ring image cell on EVERY row from the top to the menu — no
/// clipped gap where the old reclaim blanked the flank below the picture's art.
#[test]
fn journey_hybrid_tall_pane_divider_reaches_menu() {
    use ratatui::style::Color;
    let Some(session) = journey_at_menu() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    // The menu is bottom-anchored terminal cells; find its top ("Proceed" row).
    let proceed_y = (0..area.height).find(|&y| row_text(y).contains("Proceed")).expect("'Proceed' menu row");
    assert!(vp.bottom() <= proceed_y, "the reclaimed story stops above the menu (vp bottom {}, menu {proceed_y})", vp.bottom());

    // The divider/flank is chrome-ring ART, not terminal text: a cell is a ring image
    // when it is a halfblock glyph or carries a concrete background colour.
    let is_img = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    // Every row from the frame top down to the menu strip carries a left-flank ring
    // image cell — the picture column + divider run unbroken to the menu (the old
    // clip-to-art-extent reclaim left a bare-backdrop gap below the picture art).
    for y in 0..proceed_y {
        assert!(
            (0..vp.x).any(|x| is_img(x, y)),
            "the left picture-column/divider must be continuous to the menu — no ring image on row {y} (menu at {proceed_y}); row: {:?}",
            row_text(y)
        );
    }
    // And it abuts the story viewport with no seam at a deep reclaimed row.
    let dy = proceed_y.saturating_sub(1);
    let lmax = (0..vp.x).rev().find(|&x| is_img(x, dy)).expect("left flank ring cells present near the menu");
    assert_eq!(lmax, vp.x - 1, "the divider/flank abuts the story viewport with no seam (lmax {lmax}, vp.x {})", vp.x);

    // SQ-0511 fix: the left picture column keeps its ASPECT — it is NOT vertically
    // stretched to fill the reclaimed slack. A picture pixel is a real image glyph
    // (▀/▄ halfblock with per-cell colour), so the picture art's rendered rows are
    // exactly the rows carrying WIDE glyph coverage. Under the uniform scale those
    // rows stay within the story's native bottom mapped through that scale; the old
    // stretch filled the WHOLE column with picture glyphs right down to the menu.
    use app::engine::WinNode;
    use app::render::v6_layout as v6;
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = v6::native_extent(items);
    let story = v6::classify_windows(items).story.expect("story window");
    let fs = ratatui_image::picker::Picker::halfblocks().font_size();
    let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let s = v6::uniform_scale(native, (area.width as u32 * cw, area.height as u32 * ch)).s;
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    // Top-anchored (off_y = 0): the story's native bottom maps to this device row.
    let story_bottom_row = (story_bottom as f32 * s / ch as f32).floor() as u16;
    let is_glyph = |x: u16, y: u16| {
        let s = buf.cell((x, y)).unwrap().symbol();
        s == "\u{2580}" || s == "\u{2584}"
    };
    let picture_row = |y: u16| (0..vp.x).filter(|&x| is_glyph(x, y)).count() as u16 > vp.x / 2;
    let picture_bottom = (0..proceed_y).filter(|&y| picture_row(y)).max().expect("picture rows present");
    assert!(
        picture_bottom <= story_bottom_row + 1,
        "the picture keeps its aspect (uniform scale): its glyph coverage bottoms out at row {picture_bottom}, \
         within the story's uniform-scaled native bottom (row {story_bottom_row}); NOT stretched to the menu {proceed_y}"
    );
    // A genuine reclaimed gap remains below the aspect-correct picture, and NO wide
    // picture glyphs bleed into it (the old stretch put picture glyphs on every row).
    assert!(
        proceed_y > picture_bottom + 3,
        "a reclaimed gap remains below the aspect-correct picture (picture {picture_bottom}, menu {proceed_y})"
    );
    for y in (picture_bottom + 1)..proceed_y {
        assert!(
            !picture_row(y),
            "row {y} below the picture carries no stretched picture glyphs; row: {:?}",
            row_text(y)
        );
    }
    // The grey divider (native [232,240), ~[220,220,220]) — NOT the dark gap fill — is
    // carried on down through the reclaimed gap to the menu: on a deep reclaimed row
    // the flank cell abutting the story is markedly lighter than the interior fill.
    let lum = |x: u16, y: u16| match buf.cell((x, y)).unwrap().bg {
        Color::Rgb(r, g, b) => r as u16 + g as u16 + b as u16,
        _ => 0,
    };
    let deep = proceed_y.saturating_sub(2);
    assert!(deep > picture_bottom, "the deep probe row {deep} is inside the reclaimed gap (picture {picture_bottom})");
    assert!(
        lum(vp.x - 1, deep) > 400 && lum(vp.x - 1, deep) > lum(vp.x / 2, deep) + 200,
        "the grey divider reaches the reclaimed row {deep}: edge lum {} stands out over interior lum {}",
        lum(vp.x - 1, deep),
        lum(vp.x / 2, deep)
    );
}
