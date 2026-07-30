//! Arthur (Infocom v6) top status bar — SQ-0500 / SQ-0499 / SQ-0504.
//!
//! Arthur paints a reverse-video status bar (location + "St Anne's Day, Compline"
//! date) as pixel-positioned runs at native row 12, ABOVE the story buffer (which
//! starts at native row 13) and BELOW a graphics panel (native rows 0–11). The
//! panel's frame is a full-screen graphics window, but its INTERIOR — where the
//! status text sits — is transparent, so the status row is a pure-TEXT strip
//! sandwiched between art strips.
//!
//! HYBRID mode (SQ-0500): the status row decomposes into its own terminal-CELL
//! strip (crisp reverse bar) while the graphics panel above stays the pixel ring.
//! The bar reads solid across the FULL pane width (SQ-0504) — the game paints its
//! runs only from col ~4 to ~75, but a pure reverse-video row fills edge to edge.
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.
//!
//! **Colour mode: `honor_game_colours = true`** — the app's shipped config
//! default, so these render assertions are made in the mode real players run.
//! (Before SQ-0532 wave 4 every v6 smoke booted with the game's colours
//! DECLINED, which is exactly why three colour-driven render regressions
//! shipped unseen. The theme-only `false` path is covered by the paired cases
//! in `v6_game_colour_regression.rs`.)

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Arthur past the sword-in-the-stone intro to ordinary gameplay, where the
/// top status bar (location + date) is painted and the story buffer is streaming.
fn arthur_at_status() -> Option<GameSession> {
    let story_path = stories_dir().join("arthur-r74-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        None
    })?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None)
            .expect("Arthur (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Tap through the intro; answer 'n' to the "restore a saved position?" prompt.
    for _ in 0..12 {
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(session)
}

/// The status runs reach the model as reverse-video grid runs at native row 12,
/// carrying the "St Anne's Day" date text — a sanity gate for the harness.
#[test]
fn arthur_status_reaches_the_model() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let mut date = false;
    let mut reversed = false;
    for pw in items {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if (t.y.max(1) - 1) / 16 == 12 {
                    reversed |= t.style & 1 != 0;
                    if t.text.contains('A') || t.text.contains('n') {
                        date = true;
                    }
                }
            }
        }
    }
    assert!(date, "status runs present at native row 12");
    assert!(reversed, "status bar is reverse-video");
}

/// (SQ-0500 + SQ-0499 + SQ-0504) HYBRID: the status row renders as terminal CELLS
/// — the "St Anne's Day, Compline" date is real buffer text — and its reverse bar
/// spans the FULL pane width (no unreversed gap anywhere on the row). The graphics
/// panel above the status row stays the pixel ring (half-block image cells).
#[test]
fn arthur_hybrid_status_row_is_solid_terminal_bar() {
    let Some(session) = arthur_at_status() else { return };
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
    // Locate the terminal row carrying the status date as real cell text.
    let status_y = (0..area.height)
        .find(|&y| row_text(y).contains("Anne"))
        .expect("status date renders as terminal cells");

    // The reverse bar spans the FULL pane width (SQ-0504): the game paints its
    // status runs only from col ~4 to ~75, but the bar reads edge to edge — every
    // cell on the row is reverse-video, including the leading/trailing cells the
    // game left bare and the lone gap before the date (old SQ-0499 hole).
    let holes: Vec<u16> = (0..area.width)
        .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
        .collect();
    assert!(
        holes.is_empty(),
        "the reverse status bar spans the full pane [0,{}) with no unreversed gap; holes at {holes:?}\nrow: {:?}",
        area.width,
        row_text(status_y)
    );

    // The graphics panel above the status row stays the pixel ring.
    let is_image_cell = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != ratatui::style::Color::Reset
    };
    let panel_ink = (0..status_y.saturating_sub(1))
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| is_image_cell(x, y))
        .count();
    assert!(panel_ink > 0, "the graphics panel above the status stays imaged in the ring (got {panel_ink} image cells)");
}

/// (SQ-0509) At a pane size whose letterbox scale is not 1.0 (100×30 → scale 1.2),
/// Arthur's status text — emitted as single-glyph runs at fixed 8-px pixel starts —
/// must still read "Churchyard" as ONE unbroken word: the strip merges the abutting
/// fragments before mapping, instead of rounding each independently into stray cell
/// gaps ("Chu rch yard"). The right-hand date field stays in the right portion of
/// the bar (its runs are separated from the location by a real pixel gap, so they
/// do NOT merge into the location text).
#[test]
fn arthur_hybrid_status_churchyard_is_one_word_date_right() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 100×30 → scale 1.2: at 1.0 the fixed 8-px runs already lined up; only a
    // non-unit scale exposed the per-run rounding that split the word.
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let status_y = (0..area.height).find(|&y| row_text(y).contains("Anne")).expect("status row present");
    let row = row_text(status_y);
    // The location reads as one unbroken word — no stray gap inside it.
    assert!(row.contains("Churchyard"), "location must read 'Churchyard' as one word; got: {row:?}");
    // The date field is intact and sits in the RIGHT portion of the bar (well past
    // the location on the left), never merged into the location run.
    let date_at = row.find("St Anne's Day, Compline").expect("date field intact and contiguous");
    let loc_at = row.find("Churchyard").unwrap();
    assert!(date_at > loc_at + "Churchyard".len(), "the date stays to the right of the location");
    assert!(date_at * 2 > area.width as usize, "the date sits in the right half of the {}-col bar (at {date_at})", area.width);
}

/// (SQ-0505 dynamic hybrid layout) At a TALL pane (90×40 — taller than the 8:5
/// native aspect, so there is vertical letterbox dead space), Arthur has NO bottom
/// chrome: header art + status bar on top, side borders, nothing below the story.
/// The ring is top-anchored (no vertical centering) and the story text viewport
/// extends all the way to the pane BOTTOM at its constant inset width. Where the
/// side art ends, the flanks below it are the theme backdrop — no stretched art.
#[test]
fn arthur_hybrid_tall_pane_extends_story_to_bottom() {
    use ratatui::style::Color;
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // 90×40 → scale 1.40625 (width-limited), ~238 px of vertical letterbox slack.
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    // The story viewport reaches the pane bottom (dead space reclaimed).
    assert_eq!(vp.bottom(), area.bottom(), "story viewport extends to the pane bottom; got {vp:?}");
    // It keeps its side insets — the story column did NOT reflow to full width.
    assert!(vp.x > 0 && vp.right() < area.right(), "story keeps its constant inset width beside the side borders; got {vp:?}");
    // The top chrome ring (header art + status bar) is still imaged above the story.
    let is_image = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    let top_ring = (0..vp.y).flat_map(|y| (0..area.width).map(move |x| (x, y))).filter(|&(x, y)| is_image(x, y)).count();
    assert!(top_ring > 0, "the header/status ring stays imaged above the top-anchored story ({top_ring} cells)");
    // A flank cell in a LEFT-band row above the story-native bottom carries side
    // art; a flank cell deep below it is the theme backdrop (Reset), proving the
    // art is not stretched or tiled into the reclaimed space.
    let flank_art = (vp.y..vp.y + 6).any(|y| buf.cell((vp.x - 1, y)).unwrap().bg != Color::Reset);
    assert!(flank_art, "the side border art shows in the flank beside the top of the story");
    let deep = buf.cell((vp.x - 1, area.height - 2)).unwrap();
    assert_eq!(deep.bg, Color::Reset, "the flank below the side art is the theme backdrop, not stretched art: {deep:?}");
}

/// (SQ-0549) FRAMELESS: Arthur's status bar must ANCHOR TO THE TOP of the pane.
///
/// Frameless drops the pixel chrome, so the 12-row graphics panel above the bar is
/// never drawn — but the bar itself used to be stamped absolutely at its NATIVE row
/// 12, leaving it floating about a quarter of the way down an otherwise blank pane.
/// The frameless band split is now a RELATION ("the chrome text above the story
/// window"), not the old `native row < 4` constant, so the bar lands on row 0 with
/// the transcript starting beneath it. Pinned at two pane sizes so the position
/// can't be re-derived from the pane geometry, and in BOTH `honor_game_colours`
/// modes — the reverse bar is a style bit, not a game colour, so it must read solid
/// either way.
#[test]
fn arthur_frameless_status_bar_anchors_to_the_top() {
    let Some(session) = arthur_at_status() else { return };
    let model = session.screen();

    for honor in [true, false] {
        for (w, h) in [(80u16, 25u16), (100, 40)] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            state.config.v6_render = app::config::V6RenderMode::Frameless;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

            let row_text = |y: u16| -> String {
                (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
            };
            let status_y = (0..area.height)
                .find(|&y| row_text(y).contains("Anne"))
                .unwrap_or_else(|| panic!("status date renders as terminal cells (honor={honor}, {w}x{h})"));
            assert_eq!(status_y, 0, "the status bar is the TOP row, not floating at its native row 12 (honor={honor}, {w}x{h})");

            // A classic anchored status line: location flush left, date flush right.
            let row = row_text(0);
            assert!(row.starts_with("Churchyard"), "location flush at col 0 (honor={honor}, {w}x{h}): {row:?}");
            // The SQ-0509 fragment merge reaches the band too: Arthur emits one run
            // per GLYPH, so without it every letter became its own anchor group.
            let date = "St Anne's Day, Compline";
            let date_at = row
                .find(date)
                .unwrap_or_else(|| panic!("the date reads as one contiguous field (honor={honor}, {w}x{h}): {row:?}"));
            assert_eq!(date_at + date.len(), area.width as usize, "the date is flush RIGHT: {row:?}");

            // The bar reads solid edge to edge, and nothing is stranded at row 12.
            let holes: Vec<u16> = (0..area.width)
                .filter(|&x| !buf.cell((x, 0)).unwrap().modifier.contains(Modifier::REVERSED))
                .collect();
            assert!(holes.is_empty(), "the reverse bar spans the pane (honor={honor}, {w}x{h}); holes at {holes:?}");
            assert!(
                row_text(12).trim().is_empty(),
                "nothing is stamped at the old absolute native row 12 (honor={honor}, {w}x{h}): {:?}",
                row_text(12)
            );
        }
    }
}

/// Boot Arthur, then issue the in-game `map` command. `after_enter` additionally
/// dismisses the map with a bare Enter. The two states differ ONLY in the story
/// window's native height: `map` grows win0 from 584×128 to 584×192 (so its bottom
/// reaches the native screen bottom, 400), and dismissing it shrinks it back.
fn arthur_showing_map(after_enter: bool) -> Option<GameSession> {
    let mut session = arthur_at_status()?;
    let _ = session.submit("map");
    if after_enter {
        let _ = session.submit("");
    }
    Some(session)
}

/// A hybrid-mode render at a real terminal cell size (8×17, the reporter's
/// Ghostty) with the Kitty protocol — the mode and geometry players actually run.
fn render_hybrid(model: &app::engine::ScreenModel, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // from_fontsize is deprecated in favour of a live stdio query a headless test
    // can't do; the fixed cell is the point here — the defect is cell-size driven.
    #[allow(deprecated)]
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 17));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// SQ-0571(a): Arthur's header picture must not MOVE when the in-game `map`
/// command runs.
///
/// `map` grows win0 to 584×192, putting its bottom at the native screen bottom
/// (400). `hybrid_bottom_plan` read that as "an enclosing frame" and — finding no
/// full-height side ART to stretch (Arthur's side borders are GRID windows, so the
/// graphics-only canvas is empty there) — fell back to `Letterbox`, whose CENTRED
/// vertical offset dropped the header art and the map drawn into it half the
/// letterbox slack down the pane: a band of blank rows opened above the picture,
/// and dismissing the map jumped it back to the top. A story window's height must
/// not decide where the header art lands, so a header panel above the story now
/// keeps the ring top-anchored (`Extend`) in both states.
#[test]
fn arthur_map_does_not_move_the_header_art() {
    let Some(shown) = arthur_showing_map(false) else { return };
    let Some(dismissed) = arthur_showing_map(true) else { return };

    for (cols, rows) in [(95u16, 51u16), (96, 51), (99, 51), (100, 51), (138, 51)] {
        let mut seen = Vec::new();
        for (label, session) in [("map shown", &shown), ("map dismissed", &dismissed)] {
            let model = session.screen();
            let (area, buf) = render_hybrid(&model, cols, rows);
            let ink = |x: u16, y: u16| -> bool {
                let c = buf.cell((x, y)).unwrap();
                c.symbol() != " " || c.bg != ratatui::style::Color::Reset
            };
            let first_ink_row = (0..area.height)
                .find(|&y| (0..area.width).any(|x| ink(x, y)))
                .unwrap_or_else(|| panic!("{label} at {cols}x{rows} drew nothing at all"));
            assert_eq!(
                first_ink_row, 0,
                "{label} at {cols}x{rows}: the header art starts at the pane TOP, with no blank band above it"
            );
            let status_y = (0..area.height)
                .find(|&y| {
                    (0..area.width)
                        .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                        .collect::<String>()
                        .contains("Anne")
                })
                .unwrap_or_else(|| panic!("{label} at {cols}x{rows}: status bar renders as terminal cells"));
            seen.push((label, status_y));
        }
        assert_eq!(
            seen[0].1, seen[1].1,
            "the status bar sits on the SAME terminal row whether the map is shown or dismissed \
             at {cols}x{rows} ({} → row {}, {} → row {})",
            seen[0].0, seen[0].1, seen[1].0, seen[1].1
        );
    }
}

/// SQ-0571(b): Arthur's status bar must render as crisp terminal CELLS at EVERY
/// pane width — the width-dependent "corrupted location bar".
///
/// Under the `Extend` plan the chrome ring bands are clipped to the header art's
/// own vertical extent, at `ceil(art_bottom · s / cell_h)`, so the flanks below the
/// art stay theme backdrop. But `run_cell` maps a chrome run's native top by
/// ROUNDing, and Arthur's art ends at native y=192 exactly where its status row
/// begins. Whenever `192·s / cell_h` had a fraction ≥ 0.5 the ceil and the round
/// agreed, the clip landed exactly ON the status row and evicted it from the band:
/// no `Text` strip covered it, so `clear_text_rows` never carved it out of the band
/// canvas and the bar painted as a squashed raster slice of the frame instead of
/// cells — half-height, sliced glyphs. On an 8×17 cell that broke widths 96..=99
/// and left 95 and 100 clean, which is exactly how it was reported.
///
/// Sweeps every width across two full periods of that fraction and requires the bar
/// to be real cells, solid edge to edge (SQ-0504), at all of them.
#[test]
fn arthur_status_bar_is_terminal_cells_at_every_pane_width() {
    let Some(session) = arthur_showing_map(true) else { return };
    let model = session.screen();

    let mut broken = Vec::new();
    for cols in 88u16..=112 {
        let (area, buf) = render_hybrid(&model, cols, 51);
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        let Some(status_y) = (0..area.height).find(|&y| row_text(y).contains("Anne")) else {
            broken.push((cols, "status bar never reached the cell layer".to_string()));
            continue;
        };
        let row = row_text(status_y);
        if !row.contains("Churchyard") {
            broken.push((cols, format!("location text broken up: {row:?}")));
            continue;
        }
        // SQ-0504: a pure reverse-video row reads solid across the whole pane.
        let holes: Vec<u16> = (0..area.width)
            .filter(|&x| !buf.cell((x, status_y)).unwrap().modifier.contains(Modifier::REVERSED))
            .collect();
        if !holes.is_empty() {
            broken.push((cols, format!("unreversed gaps at {holes:?} in {row:?}")));
        }
    }
    assert!(broken.is_empty(), "the status bar must be crisp cells at every width; failures: {broken:#?}");
}

/// A hybrid render at 8×17/Kitty that reports the recorded letterbox anchor: the
/// device-pixel top of the drawn game image inside the pane, rounded to a whole
/// pixel. This is what "the frame moved" means numerically.
fn hybrid_anchor(model: &app::engine::ScreenModel, cols: u16, rows: u16) -> i32 {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    #[allow(deprecated)]
    let mut picker = ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 17));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    let anchor = state.graphics_render.borrow().last_v6_map.as_ref().map(|m| m.img_y.round() as i32);
    anchor.expect("the hybrid path records a v6 click map")
}

/// SQ-0571(c): none of Arthur's full-screen takeovers may MOVE the frame when they
/// open or close.
///
/// Each of them resizes win0 so its bottom reaches the native screen bottom (400):
/// `map` grows it to 584×192, and the F6 text page opens it at 640×384.
/// `hybrid_bottom_plan` read a story window reaching the bottom as an enclosing
/// frame and — with no full-height side ART to stretch, since Arthur's borders are
/// drawn into the full-screen window 7 and erased by these very screens — fell back
/// to `Letterbox`, whose CENTRED vertical offset pushed the whole screen half the
/// letterbox slack down the pane (F6: 193 device px, ~11 rows). Dismissing the
/// screen shrank win0 back below the bottom, flipped the plan to `Extend`, and
/// everything jumped to the top. Where the frame sits must not depend on the story
/// window's height, so that arm now top-anchors too.
///
/// Asserts the recorded letterbox anchor is unchanged across the transition, and
/// that it is the pane top — a centred offset is exactly the defect.
#[test]
fn arthur_screen_swaps_do_not_move_the_frame() {
    // `None` → the `map` command; `Some(t)` → a line read terminated by function
    // key ZSCII `t` (F1 = 133, so F3..F6 are 135..138) — how Arthur's keypad
    // screens are actually invoked.
    for (label, term) in [
        ("map command", None),
        ("F3", Some(135u8)),
        ("F4", Some(136)),
        ("F5", Some(137)),
        ("F6", Some(138)),
    ] {
        let Some(mut session) = arthur_at_status() else { return };
        match term {
            None => {
                let _ = session.submit("map");
            }
            Some(t) => {
                let _ = session.submit_line_with_terminator("", t);
            }
        }
        let shown = hybrid_anchor(&session.screen(), 96, 51);
        let _ = session.submit("");
        let dismissed = hybrid_anchor(&session.screen(), 96, 51);
        assert_eq!(
            shown, dismissed,
            "{label}: the frame sits at the same place shown and dismissed \
             (shown img_y={shown}, dismissed img_y={dismissed})"
        );
        assert_eq!(shown, 0, "{label}: the frame is anchored to the pane TOP, not centred (img_y={shown})");
    }
}
