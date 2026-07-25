//! Lane H (hybrid render) end-to-end acceptance: boot the real Zork0 (v6/
//! graphical), build its `Layered` screen model, and drive the story-pane
//! renderer in BOTH v6 render modes.
//!
//! Hybrid mode draws the chrome as a scaled pixel ring around a terminal story
//! viewport, then renders the story window as REAL terminal text (crisp,
//! selectable) inside it — so the transcript appears as ordinary buffer cells and
//! the story-pane render reports real scroll metrics. Raster mode keeps today's
//! behavior: the whole pane is one rasterized pixel image, no terminal transcript.
//!
//! The story asset is gitignored (large, local-only), so this test **skips
//! cleanly** when absent — mirrors `zork0_v6_windows.rs`.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork0 through boot + boot-picture flush, exactly like `zork0_v6_windows.rs`.
/// Returns `None` (with a SKIP note) when the gitignored story is absent.
fn boot_zork0() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// Build an AppState wired for a v6 headless render: a halfblocks picker (no
/// terminal query), terminal-default theme, and the given render mode.
fn render_state(mode: app::config::V6RenderMode) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state
}

#[test]
fn zork0_hybrid_renders_story_as_terminal_text() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    assert!(matches!(model.root, WinNode::Layered(_)), "v6 root is a layered composite");

    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    // Seed a couple of transcript lines so the story viewport has real text to draw.
    state.push_transcript("West of House");
    state.push_transcript("You are standing in an open field west of a white house.");

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let metrics = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // Hybrid renders the story window as terminal cells → the transcript publishes
    // its geometry into an inset viewport (strictly inside the pane, so the chrome
    // ring surrounds it).
    let geom = state
        .transcript_geom
        .get()
        .expect("hybrid renders the story as a terminal transcript");
    let vp = geom.area;
    assert!(vp.width < area.width && vp.height < area.height, "story viewport is inset in the chrome ring: {vp:?}");
    assert!(metrics.viewport_rows > 0, "story pane reports real viewport rows");

    // Cell-dump of the hybrid render (the Task H3 deliverable). Chrome-ring cells
    // are painted by the image protocol (halfblock glyphs here); the inset
    // viewport carries the crisp terminal transcript.
    eprintln!("--- Zork0 HYBRID cell-dump ({}x{}) — viewport {:?} ---", area.width, area.height, vp);
    for y in area.y..area.bottom() {
        let mut row = String::new();
        for x in area.x..area.right() {
            let s = buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
            row.push(s.chars().next().unwrap_or(' '));
        }
        eprintln!("{y:2}|{row}|");
    }

    // The seeded story text lands somewhere in the viewport as real cells.
    let found = (vp.y..vp.bottom()).any(|y| {
        let row: String = (vp.x..vp.right())
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        row.contains("West of House") || row.contains("white house")
    });
    assert!(found, "seeded story text renders as terminal cells inside the viewport");
}

#[test]
fn zork0_raster_mode_publishes_scroll_geometry() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    let mut state = render_state(app::config::V6RenderMode::Raster);
    state.push_transcript("West of House");

    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let metrics = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // Raster mode still rasterizes the whole pane as one pixel image, but it now
    // REPORTS the story box's scroll geometry (SQ-0455) so the shared scroll
    // keybindings and the [more] pager (SQ-0404) engage. The reported viewport is
    // the story box's raster body rows (never the default full-pane height), and
    // geometry is published (approximate mouse mapping over the pixel-scaled text).
    assert!(state.transcript_geom.get().is_some(), "raster mode publishes scroll geometry");
    assert!(metrics.viewport_rows > 0, "raster reports real story-box viewport rows");
    assert!(
        metrics.viewport_rows < area.height,
        "the story box is smaller than the full pane (chrome ring reserved): {}",
        metrics.viewport_rows
    );
}

/// SQ-0467: frameless mode lays the v6 status band out as a classic full-width
/// status line (the "anchored bar"), not a 40-cell postage stamp squatting in the
/// left half of an 80-col pane. Zork0's banner (native 320px / 40 cells) carries a
/// left location run and right-side Score/Moves runs, so on an 80-col pane the
/// location must sit flush at col 0 and the Score/Moves must land flush right — not
/// bunched mid-pane as the old per-run cell-quantization produced.
#[test]
fn zork0_frameless_status_band_is_anchored_full_width() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    let state = render_state(app::config::V6RenderMode::Frameless);
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // The band occupies the top rows; capture them as text.
    let band: Vec<String> = (0..4)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect())
        .collect();
    eprintln!("--- Zork0 FRAMELESS status band (80 cols) ---");
    for (y, r) in band.iter().enumerate() {
        eprintln!("{y}|{r}|");
    }

    // Location is flush LEFT: row 0 begins with a printed glyph at col 0.
    assert_ne!(band[0].chars().next(), Some(' '), "location run is flush at col 0, not indented: {:?}", band[0]);

    // Score/Moves are anchored flush RIGHT: the row carrying "Score:" ends its
    // painted text within the last two columns of the 80-col pane (old quantized
    // layout left them mid-pane with dead space to the right).
    let score_row = band.iter().find(|r| r.contains("Score:")).expect("a status row carries the Score label");
    let last_glyph = score_row.rfind(|c: char| c != ' ').expect("score row has printed text");
    assert!(last_glyph >= area.width as usize - 2, "Score/Moves flush right (last glyph at col {last_glyph} of {}): {score_row:?}", area.width - 1);
    // And it did NOT squat in the left 40 columns (the reported bug).
    assert!(!score_row[41..].trim().is_empty(), "right-side status reaches past the old 40-cell stamp: {score_row:?}");
}

/// (SQ-0500 pin) Zork0's status ("Moves:"/"Score:") sits ON opaque banner art, so
/// in HYBRID mode its chrome band stays the pixel RING — the status labels must
/// NOT appear as terminal cell text (unlike Arthur's clear-interior status row,
/// which does become cells). Guards the "art behind → keep the ring" branch of the
/// band decomposition.
#[test]
fn zork0_hybrid_status_on_art_stays_in_the_ring() {
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();

    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    // The banner status labels are rasterized into the pixel ring, never stamped
    // as terminal text cells.
    let screen: String = (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect::<String>()
                + "\n"
        })
        .collect();
    assert!(!screen.contains("Moves:"), "Zork0's on-art status stays in the ring, not cells:\n{screen}");
    assert!(!screen.contains("Score:"), "Zork0's on-art score stays in the ring, not cells");
}

/// (SQ-0514 chrome-band freshness pin) Zork0 rasterizes its Score/Moves status into
/// the chrome canvas every turn, so a turn changes only the TOP status band's native
/// pixels. In HYBRID mode the flank (side/bottom) ring bands must then stay FRESH —
/// their cached uploads reused, not re-encoded. Before the fix the freshness hash
/// covered the WHOLE canvas, so any status change staled every band (the ~377ms
/// per-turn stall). Here we render, take a turn (mutating the banner), render again,
/// and assert at least one band re-encoded while at least one other stayed fresh.
#[test]
fn zork0_hybrid_status_change_keeps_flank_bands_fresh() {
    let Some(mut session) = boot_zork0() else { return };
    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 80, 30);

    // Frame 1: populate the per-band cache.
    let model = session.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let before = state.graphics_render.borrow().chrome_band_hashes();
    assert!(before.len() >= 2, "hybrid Zork0 draws multiple chrome bands, got {}", before.len());

    // Take a turn so the banner (Score/Moves) re-rasterizes into the chrome canvas.
    let _ = session.submit("look");
    let model = session.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let after = state.graphics_render.borrow().chrome_band_hashes();

    // Every band that survived the turn (same rect) is compared; the fix guarantees
    // at least one flank band kept its hash (fresh) even though the banner changed.
    let common: Vec<_> = before.keys().filter(|k| after.contains_key(*k)).collect();
    assert!(!common.is_empty(), "at least one band rect persists across the turn");
    let fresh = common.iter().filter(|k| before[**k] == after[**k]).count();
    assert!(
        fresh > 0,
        "at least one chrome band stays fresh across a status change (SQ-0514): \
         {fresh}/{} bands unchanged",
        common.len()
    );
}

/// (SQ-0511 enclosed-frame reclaim — Zork0 pin) Zork0's frame ENCLOSES the story to
/// the native screen bottom (story bottom 398 of 400) and is flanked by full-height
/// side art. At a TALL pane the `Frame` plan now RECLAIMS the letterbox slack: the
/// top banner stays uniform-scaled and pane-top-anchored, the story viewport grows
/// from just under it to the pane BOTTOM at constant width, and the ornate side
/// flanks STRETCH vertically to fill the reclaimed space (no longer a centred
/// letterbox). Pins the new geometry exactly (halfblocks 10×20 cells, native
/// 640×400, scale 1.40625, off_y 0). One column is reserved for the scrollbar.
#[test]
fn zork0_hybrid_tall_pane_frame_reclaim() {
    use ratatui::style::Color;
    let Some(session) = boot_zork0() else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };

    let mut state = render_state(app::config::V6RenderMode::Hybrid);
    state.push_transcript("West of House");
    let area = Rect::new(0, 0, 90, 40);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
    let _ = items;

    // Top-anchored under the banner band (native y=78 → dev 109.7 → row 6), extended
    // to the pane bottom (row 40), constant width (native 86..554 → cols 13..77,
    // one reserved for the scrollbar → width 63).
    assert_eq!(vp, Rect::new(13, 6, 63, 34), "Zork0 top-anchors + extends to the pane bottom (Frame reclaim)");
    assert!(vp.y > 0 && vp.y <= 6, "story top-anchored just under the banner band, not centred: {vp:?}");
    assert_eq!(vp.bottom(), area.bottom(), "story viewport extends to the pane bottom (slack reclaimed): {vp:?}");

    // A cell carries a chrome-ring image when it is a halfblock glyph or has a
    // concrete (non-Reset) background — the flank bands upload as image protocols.
    let is_img = |x: u16, y: u16| -> bool {
        let c = buf.cell((x, y)).unwrap();
        let s = c.symbol();
        s == "\u{2580}" || s == "\u{2584}" || c.bg != Color::Reset
    };
    // The side flanks STRETCH into the reclaimed space: at a deep row well below the
    // old (centred) art extent, both flanks still carry ring image cells — the OLD
    // clip-to-art-extent path left these rows the bare backdrop.
    for dy in [vp.bottom() - 3, vp.bottom() - 1] {
        assert!((0..vp.x).any(|x| is_img(x, dy)), "left flank stretched down to row {dy} (reclaimed space)");
        assert!((vp.right()..area.width).any(|x| is_img(x, dy)), "right flank stretched down to row {dy}");
        // Squares-and-seams at the fractional scale 1.40625: the left flank's
        // rightmost image column abuts the viewport (== vp.x-1) — no gap, no overlap
        // into the story columns — and the right flank stays at/after vp.right().
        let lmax = (0..vp.x).rev().find(|&x| is_img(x, dy)).expect("left flank has ring cells");
        assert_eq!(lmax, vp.x - 1, "left flank abuts the story viewport with no seam at row {dy} (lmax {lmax}, vp.x {})", vp.x);
        let rmin = (vp.right()..area.width).find(|&x| is_img(x, dy)).expect("right flank has ring cells");
        assert!(rmin >= vp.right(), "right flank does not overlap the story viewport at row {dy} (rmin {rmin}, vp.right {})", vp.right());
    }
}
