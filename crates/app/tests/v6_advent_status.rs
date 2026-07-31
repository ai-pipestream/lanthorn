//! advent.z6's OVERLAID status bar — SQ-0581 (mapping) / SQ-0582 (render).
//!
//! Every other v6 story here reserves its status band by placing the story window
//! BELOW it: Zork Zero opens window 0 at y=79, Shogun at y=33, Arthur at y=209 under
//! a graphics panel. advent.z6 (release 10 / 011123) does the opposite — window 0
//! covers the whole 640×380 screen and window 1, full width and ONE row tall, is
//! hung over its first row, painting "At End Of Road   Score: 36   Moves: 1" there.
//!
//! That single difference broke both halves of the feature:
//!   - the automapper's v6 band is "everything painted above the story window", and
//!     nothing is above a story window that starts at the screen top, so no room was
//!     ever detected (SQ-0581);
//!   - the hybrid renderer's chrome ring is the area AROUND the story viewport, and
//!     an overlaid bar leaves no ring, so its runs were stamped glyph-by-glyph over
//!     the transcript — a ribbon with holes between the fields (SQ-0582).
//!
//! Skip-if-missing per the other gitignored-story smokes.
//!
//! **Colour mode**: the render case is pinned in BOTH `honor_game_colours` modes.
//! advent sets no colours of its own on the bar (`colour_data = 0x0101`, both
//! channels default), so the theme's `upper_window` style has to carry it either
//! way — and a mode-specific regression here would otherwise hide.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot advent.z6 and tap through its intro to the first line prompt.
fn advent_in_play(honor: bool) -> Option<GameSession> {
    let story_path = stories_dir().join("advent.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, honor, false, None, false, picture_dims, picts.std_window(), None)
            .expect("advent.z6 (v6) should load and boot without a ZError");
    assert!(!session.quit, "quit during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    for _ in 0..10 {
        match session.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
        let _ = session.take_transcript();
    }
    Some(session)
}

/// SQ-0581: the room named by the overlaid bar reaches the mapper, and it CHANGES
/// on a move. "At End Of Road" is the opening room; east enters the building.
#[test]
fn advent_v6_overlaid_status_bar_yields_a_room() {
    let Some(mut session) = advent_in_play(true) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let r = session.submit("look");
    assert!(!r.quit && r.fault.is_none(), "\"look\" faulted/quit: {:?}", r.fault);
    let _ = session.take_transcript();

    let start = session.current_location().expect("advent's opening room must be detected");
    assert!(
        start.name.contains("End Of Road"),
        "expected the opening room At End Of Road, got {:?}",
        start.name
    );

    let r = session.submit("e");
    assert!(!r.quit && r.fault.is_none(), "the \"e\" move faulted/quit: {:?}", r.fault);
    let _ = session.take_transcript();

    let after = session.current_location().expect("a room must still be detected after the move");
    assert_ne!(
        after.number, start.number,
        "the room id must change across the move (was #{} {:?}, still #{} {:?})",
        start.number, start.name, after.number, after.name
    );
    assert!(
        after.name.contains("Inside Building"),
        "expected Inside Building after \"e\", got {:?}",
        after.name
    );
    // The score/moves fields are never mistaken for the room.
    assert!(
        !after.name.contains("Score") && !after.name.contains("Moves"),
        "a status field leaked in as the room name: {:?}",
        after.name
    );
}

/// SQ-0582 (both colour modes): HYBRID renders the overlaid bar as a SOLID full-width
/// terminal strip — the room name is real cell text, every cell on the row carries the
/// bar's background (no holes between the fields), and the transcript begins BELOW it
/// rather than scrolling through it.
fn advent_hybrid_bar_is_solid(honor: bool) {
    let Some(mut session) = advent_in_play(honor) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let bar_y = (0..area.height)
        .find(|&y| row_text(y).contains("End Of Road"))
        .unwrap_or_else(|| panic!("the status bar renders as terminal cells (honor={honor})"));
    assert_eq!(bar_y, 0, "the bar sits on the pane's top row (honor={honor})");

    // Solid: every cell on the row carries the same background as the cell under the
    // room name — the theme's bar, not the backdrop showing through the gaps.
    let name_x = row_text(bar_y).find("End Of Road").expect("located above") as u16;
    let bar_bg = buf.cell((name_x, bar_y)).unwrap().bg;
    let holes: Vec<u16> =
        (0..area.width).filter(|&x| buf.cell((x, bar_y)).unwrap().bg != bar_bg).collect();
    assert!(
        holes.is_empty(),
        "the bar spans the full pane [0,{}) with no background hole (honor={honor}); holes at {holes:?}\nrow: {:?}",
        area.width,
        row_text(bar_y),
    );

    // Crisp CELLS, not a rasterized slice of the frame: no half-block image glyphs.
    let row = row_text(bar_y);
    assert!(
        !row.contains('\u{2580}') && !row.contains('\u{2584}'),
        "the bar draws as text, not as a pixel-ring slice (honor={honor}): {row:?}"
    );

    // Exactly ONE row is reserved: advent's bar window is 20px — 1.25 terminal cells —
    // and rounding its declared height up would steal a row of story from the
    // transcript below (the strip is measured from its runs, not its height).
    let barred: Vec<u16> =
        (0..area.height).filter(|&y| (0..area.width).all(|x| buf.cell((x, y)).unwrap().bg == bar_bg)).collect();
    assert_eq!(barred, vec![bar_y], "the bar occupies one row only (honor={honor}); full-bar rows: {barred:?}");
}

/// The same bar in FRAMELESS mode, which drops the chrome ring by design: there the
/// runs are stamped inside the story box by the painted-screen path, so a full-width
/// flood — not the viewport inset — is what keeps the row solid. advent's runs carry
/// no reverse bit, so the reverse-only flood rule (SQ-0515) skipped them entirely.
#[test]
fn advent_frameless_bar_is_solid() {
    let Some(mut session) = advent_in_play(true) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Frameless;
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let bar_y = (0..area.height)
        .find(|&y| row_text(y).contains("End Of Road"))
        .expect("the status bar renders as terminal cells in frameless mode");
    let name_x = row_text(bar_y).find("End Of Road").expect("located above") as u16;
    let bar_bg = buf.cell((name_x, bar_y)).unwrap().bg;
    let holes: Vec<u16> =
        (0..area.width).filter(|&x| buf.cell((x, bar_y)).unwrap().bg != bar_bg).collect();
    assert!(
        holes.is_empty(),
        "the frameless bar spans the full pane with no background hole; holes at {holes:?}\nrow: {:?}",
        row_text(bar_y),
    );
}

#[test]
fn advent_hybrid_bar_is_solid_honoring_game_colours() {
    advent_hybrid_bar_is_solid(true);
}

#[test]
fn advent_hybrid_bar_is_solid_theme_only() {
    advent_hybrid_bar_is_solid(false);
}
