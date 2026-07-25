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
