//! advent.z6's OVERLAID status bar — SQ-0581 (mapping) / SQ-0582 (render) — and the
//! window its prose actually streams through (SQ-0583).
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
        GameSession::new_with_trace(story_bytes, honor, false, None, false, picture_dims, picts.std_window(), None, None)
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

/// SQ-0583: the story window is whichever window the game STREAMS prose through, not
/// window 0 by fiat. advent (Inform 6's v6 library) prints into window 7 and never
/// touches window 0, so window 0 keeps its boot-time full-screen rect for the whole
/// game. Its own `style` command proves the difference: it splits the screen into a
/// text window on top, the status bar in the middle (y=181) and prose in the bottom
/// 200px (y=201) — at which point a story rect that still claims the whole screen
/// puts the transcript viewport, and with it the input line, nowhere near the game's
/// own `>` prompt, and leaves the mapper reading a band above a window that starts at
/// the screen top.
#[test]
fn advent_style_command_moves_the_story_window() {
    use app::engine::WinNode;

    let Some(mut session) = advent_in_play(true) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();

    // The prose window's rect, whichever window carries it.
    let story_box = |s: &GameSession| -> (u16, u16) {
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let story = items
            .iter()
            .find(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
            .expect("a primary story window");
        (story.y_px, story.h_px)
    };

    let before = story_box(&session);
    assert_eq!(
        before,
        (20, 380),
        "prose starts below the status bar: advent's @split_window(20) TILES windows 0 and 1 \
         (ZMSD §8.8.4.1 — window 0 \"is placed just below it\"), the library reads window 0's \
         y back with @get_wind_prop and moves its own window 7 there (SQ-0712)"
    );

    let r = session.submit("style");
    assert!(!r.quit && r.fault.is_none(), "\"style\" faulted/quit: {:?}", r.fault);
    let _ = session.take_transcript();

    let after = story_box(&session);
    assert_eq!(
        after,
        (200, 200),
        "the story window follows the game's prose window into the bottom 200px \
         (window 0 still claims 0..380 — taking it would aim the viewport, and the \
         input line with it, at the whole screen)"
    );

    // And the mapper keeps working across the split: the bar moved down beside the
    // prose, so the band is only found by asking the RIGHT window where prose starts.
    let loc = session.current_location().expect("the room is still detected after the split");
    assert!(loc.name.contains("End Of Road"), "expected At End Of Road, got {:?}", loc.name);
}

/// SQ-0584: `help` opens a hint-style menu — window 1 split to 160px, ERASED, then
/// painted with the subject list. On a real interpreter every v6 window is a clipping
/// region over ONE screen bitmap, so that erase is opaque paint and the menu hides the
/// story behind it. babelmap composites layers instead, and an erased window used to
/// be simply transparent: the menu's text floated over the room description with
/// nothing behind it. The panel must read solid, and the story must come back
/// untouched when the menu closes (advent erases and reprints on the way out).
///
/// SQ-0712 narrowed the leak scan from the whole pane to the PANEL'S OWN ROWS,
/// which is what "hides the story behind it" ever meant. Before the split placed
/// window 0, the prose window covered the entire screen and the menu was an
/// overlay on top of it, so any story text anywhere was leakage. Now `help` tiles
/// the screen the way the game asks — the menu owns the top 160px and advent's
/// library moves its own prose window to 161 — so the transcript below the panel
/// is the game's layout, not a hole in it. Rendered to PNG, the change is the other
/// way round from a regression: before, the story window's full-screen box buried
/// the subject list entirely and the raster composite showed nothing but story
/// lines; now the list is legible with the transcript beneath it.
fn advent_help_panel_is_opaque(honor: bool) {
    let Some(mut session) = advent_in_play(honor) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();

    let area = Rect::new(0, 0, 80, 25);
    let render = |session: &GameSession| -> Buffer {
        let model = session.screen();
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        state.config.honor_game_colours = honor;
        // Story text in the transcript, so anything showing THROUGH the panel is
        // unmistakable — this is exactly what the panel used to fail to cover.
        for i in 0..20 {
            state.push_transcript(&format!("story line {i} ------------------------------"));
        }
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        buf
    };
    let row_of = |buf: &Buffer, y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };

    let _ = session.submit("help");
    let _ = session.take_transcript();
    let buf = render(&session);

    // The menu is there…
    let menu_y = (0..area.height)
        .find(|&y| row_of(&buf, y).contains("Instructions for playing"))
        .unwrap_or_else(|| panic!("the help menu renders (honor={honor})"));
    // …and NOT one line of the story shows through it. The panel is the erased
    // window's own rect, taken from the model rather than guessed: the cell path is
    // 1:1 with native 8x16 cells, so 160px of window is the pane's top 10 rows.
    let panel_rows = {
        use app::engine::WinNode;
        let model = session.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let panel = items
            .iter()
            .filter(|pw| matches!(&pw.node, WinNode::Grid(g) if g.fill.is_some()))
            .find(|pw| pw.y_px == 0 && pw.h_px >= 16)
            .expect("the help menu's erased window is published");
        panel.h_px / 16
    };
    assert!(menu_y < panel_rows, "the menu items are inside the panel (honor={honor})");
    let leaks: Vec<u16> = (0..panel_rows).filter(|&y| row_of(&buf, y).contains("story line")).collect();
    assert!(
        leaks.is_empty(),
        "the erased menu panel hides the story behind it (honor={honor}); story visible on panel \
         rows {leaks:?} of 0..{panel_rows}\n{}",
        leaks.iter().map(|&y| row_of(&buf, y)).collect::<Vec<_>>().join("\n"),
    );
    // The panel is a filled field, not bare backdrop between the items: the blank row
    // under the menu item carries the same background as the item's own row.
    let item_bg = buf.cell((4, menu_y)).unwrap().bg;
    let gap_bg = buf.cell((4, menu_y.saturating_sub(1))).unwrap().bg;
    assert_eq!(item_bg, gap_bg, "the rows between menu items are part of the panel (honor={honor})");

    // Q resumes: the fill is gone and the story is visible again, with the one-row
    // status bar back on top.
    let _ = session.submit_char(b'Q');
    let _ = session.take_transcript();
    let buf = render(&session);
    assert!(
        (0..area.height).any(|y| row_of(&buf, y).contains("story line")),
        "closing the menu brings the story back (honor={honor}):\n{}",
        (0..area.height).map(|y| row_of(&buf, y)).collect::<Vec<_>>().join("\n"),
    );
    assert!(
        row_of(&buf, 0).contains("End Of Road"),
        "and the status bar is back on the top row (honor={honor}): {:?}",
        row_of(&buf, 0),
    );
}

#[test]
fn advent_help_panel_is_opaque_honoring_game_colours() {
    advent_help_panel_is_opaque(true);
}

#[test]
fn advent_help_panel_is_opaque_theme_only() {
    advent_help_panel_is_opaque(false);
}

/// SQ-0582 follow-up: advent's BOOT POPUP must not blank the rows behind it.
///
/// After the first keypress the game shrinks window 1 to 640x1 — one pixel tall —
/// and paints a message box ("Use the command 'style'…") 50px further down. v6 text
/// is paint and outlives its window's geometry (ZMSD §8.8.4: resizing "does not
/// change the current display"), so the runs stay put. The full-width-strip flood
/// judged that window a status bar by its declared HEIGHT alone, so every row the
/// popup touched was flooded edge to edge and the banner behind it disappeared into
/// dark bands. A bar has to paint inside its own rect.
#[test]
fn the_boot_popup_does_not_blank_the_rows_behind_it() {
    let story_path = stories_dir().join("advent.z6");
    let Ok(bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(bytes, true, true, None, false, dims, picts.std_window(), None, None)
            .expect("boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let banner = session.take_transcript();
    // One keypress: the game paints its popup over the banner.
    session.submit_char(13);

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    for line in banner.lines() {
        state.push_transcript(line);
    }
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    // The popup is on screen…
    assert!(
        (0..area.height).any(|y| row(y).contains("Use the command")),
        "the popup renders:\n{}",
        (0..area.height).map(row).collect::<Vec<_>>().join("\n")
    );
    // …and the banner behind it survives, both to its left and to its right. These
    // are the rows that came back as solid dark bands.
    let all: Vec<String> = (0..area.height).map(row).collect();
    for needle in ["Note:  This file was comp", "there being no graphics f", "Please report any Version"] {
        assert!(
            all.iter().any(|r| r.contains(needle)),
            "banner text {needle:?} must survive behind the popup:\n{}",
            all.join("\n")
        );
    }
    // Nothing is flooded edge to edge: no row is a single solid background from the
    // pane's left edge to its right.
    let popup_row = (0..area.height).find(|&y| all[y as usize].contains("Use the command")).unwrap();
    let bg_of = |x: u16, y: u16| buf.cell((x, y)).unwrap().bg;
    assert_ne!(
        bg_of(0, popup_row),
        bg_of(40, popup_row),
        "the popup fills its own box, not the whole row: {:?}",
        all[popup_row as usize]
    );
}

/// SQ-0582 follow-up: the bar stays ONE row at a pane whose scale is not 1:1.
///
/// advent's bar window is 20px — 1.25 native text rows — and hybrid scales art by
/// PIXEL while text is one terminal row per native row (the standing v6 hazard). At a
/// tall pane the bar's pixel rect rounds to two terminal rows, so its erase fill
/// spilled a second row into the story below and the bar read two rows deep. Chrome
/// above the viewport is the ring's to paint; only windows starting inside the
/// viewport fill there.
// `Picker::from_fontsize` is deprecated in favour of querying the real terminal —
// which a headless test cannot do, and the whole point here is to pin SPECIFIC cell
// sizes. `halfblocks()` hard-codes 10x20, the one size where the old rounding
// happened to be right.
#[allow(deprecated)]
#[test]
fn the_status_bar_is_one_row_at_any_pane_scale() {
    let Some(mut session) = advent_in_play(true) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.submit("look");
    let _ = session.take_transcript();
    let model = session.screen();

    // Cell sizes spanning the real terminals this is drawn on; each gives the
    // hybrid ring a different scale, and the 20px bar lands on a different fraction
    // of a row in every one.
    for cell in [(8u16, 16u16), (9, 18), (10, 20), (8, 17)] {
        for (w, h) in [(80u16, 25u16), (97, 52), (120, 60)] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(cell.into()));
            state.config.v6_render = app::config::V6RenderMode::Hybrid;
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

            let row = |y: u16| -> String {
                (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
            };
            let bar_y = (0..h)
                .find(|&y| row(y).contains("End Of Road"))
                .unwrap_or_else(|| panic!("bar renders at {w}x{h} cell {cell:?}"));
            let bar_bg = buf.cell((1, bar_y)).unwrap().bg;
            let barred: Vec<u16> =
                (0..h).filter(|&y| (0..w).all(|x| buf.cell((x, y)).unwrap().bg == bar_bg)).collect();
            assert_eq!(
                barred,
                vec![bar_y],
                "the bar is one row at {w}x{h} with a {cell:?} cell; full-bar rows: {barred:?}"
            );
        }
    }
}
