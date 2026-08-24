//! SQ-0712: `split_window` TILES windows 0 and 1 — it places window 0, it does not
//! merely shorten it.
//!
//! ZMSD §8.8.4.1, verbatim (fetched from inform-fiction.org, not recalled): "The
//! split_screen opcode tiles windows 0 and 1 together to fill the screen, so that
//! window 1 has the given height and is placed at the top left, while window 0 is
//! placed just below it (with its height suitably shortened, possibly making it
//! disappear altogether if window 1 occupies the whole screen)."
//!
//! lanthorn shortened window 0's height and left its origin at (1,1), so after a
//! split the story window sat *inside* the upper window's box. mysterious01.z6 is
//! the case a player hit: it splits 260px for its artwork, draws into window 1, and
//! narrates in window 0 — which it never moves, because it does not have to. The
//! prose came out printed over the picture.
//!
//! The engine half does not land alone. Once window 0 stops overlapping window 1,
//! advent's status bar stops being paint inside the story box and becomes band text
//! above it — and the band was being painted BEFORE the erase fills that are its own
//! window's ground, so the bar disappeared. Both halves are pinned here.
//!
//! Stories are gitignored (CLAUDE.md), so each case skips cleanly.


use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn boot(name: &str) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None, None)
        .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// `(x_size, y_size, x_coord, y_coord)` for one v6 window, as the engine holds it.
fn win_box(session: &GameSession, i: usize) -> (u16, u16, u16, u16) {
    let v6 = session.machine.screen.v6.as_ref().expect("a v6 screen");
    let w = &v6.windows[i];
    (w.x_size, w.y_size, w.x_coord, w.y_coord)
}

/// mysterious01 past its "Resume play on a game ?" prompt, where the split lives.
fn mysterious_in_play() -> Option<GameSession> {
    let mut session = boot("mysterious01.z6")?;
    for turn in 0..4 {
        match session.pending_input() {
            InputKind::Char if turn == 0 => session.submit_char(b'n'),
            InputKind::Char => session.submit_char(13),
            InputKind::Line => break,
            InputKind::Event => session.submit(""),
        };
        let _ = session.take_transcript();
    }
    Some(session)
}

/// The engine half. mysterious01 splits 260px and never moves window 0 itself; the
/// split has to put it below the picture.
///
/// Falsified by restoring the old "shorten only" arm: "the story window must sit
/// BELOW the picture window the split placed at the top — win0 640x140 at (1,1)
/// overlaps win1 640x260 at (1,1)".
#[test]
fn a_split_places_the_story_window_below_the_upper_one() {
    let Some(session) = mysterious_in_play() else { return };

    let (w0_w, w0_h, w0_x, w0_y) = win_box(&session, 0);
    let (w1_w, w1_h, w1_x, w1_y) = win_box(&session, 1);
    assert_eq!((w1_w, w1_h, w1_x, w1_y), (640, 260, 1, 1), "the picture window is the split's own top-left box");
    assert!(
        w0_y >= w1_y + w1_h,
        "the story window must sit BELOW the picture window the split placed at the top — \
         win0 {w0_w}x{w0_h} at ({w0_x},{w0_y}) overlaps win1 {w1_w}x{w1_h} at ({w1_x},{w1_y})"
    );
    assert_eq!(
        (w0_w, w0_h, w0_x, w0_y),
        (640, 140, 1, 261),
        "…tiled: full width, the remaining height, its origin one pixel past window 1's bottom"
    );
}

/// …and the player sees it: the prose renders clear of the game's own screen
/// content, in both cell paths and both colour modes.
///
/// The two paths draw different things above the story, so each is measured for
/// what it actually puts there. HYBRID (the default) keeps the picture, so the
/// transcript must stay out of the picture window's 260px. The CELL path draws no
/// art at all — what it keeps is the narration mysterious01 PAINTS into that
/// graphics window ("I'm in a dense SPOOKY Forest"), anchored to the pane top, and
/// the transcript must start below that.
///
/// Falsified by restoring the old "shorten only" arm: "hybrid honor=true: the story
/// transcript must render clear of the artwork — story text on pane row(s) [0, 1, 2,
/// 3, 4, 5, 6, 7], but the picture window ends at row 16".
#[test]
fn mysterious01_prose_renders_below_the_artwork() {
    let Some(session) = mysterious_in_play() else { return };
    let model = session.screen();

    // The second arm was `frameless` until SQ-0895 removed it. What it contributed
    // was the CELL path as a contrast to hybrid's pixel ring, and it is reached now
    // by dropping the picker instead of by asking for a mode.
    for (tag, force_cell) in [("hybrid", false), ("cell", true)] {
        for honor in [true, false] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker =
                (!force_cell).then(ratatui_image::picker::Picker::halfblocks);
            state.config.v6_render = app::config::V6RenderMode::Hybrid;
            state.config.honor_game_colours = honor;
            for i in 0..20 {
                state.push_transcript(&format!("story line {i} ------------------------------"));
            }
            let area = Rect::new(0, 0, 80, 25);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let rows: Vec<String> = (0..area.height)
                .map(|y| {
                    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
                })
                .collect();
            let ctx = format!("{tag} honor={honor}");

            // The first row the transcript may occupy. Keyed on `force_cell`, NOT
            // on the mode: both arms ask for Hybrid now and it is the picker that
            // separates them, so testing the mode would give the cell arm hybrid's
            // expectation (SQ-0895).
            let (reserved, what) = if !force_cell {
                // The picture window's 260px, at the cell path's 1:1 16px rows.
                (260 / 16, "the picture window ends at row 16".to_string())
            } else {
                // The cell path draws no art and keeps the game's painted narration.
                let last = rows
                    .iter()
                    .rposition(|r| r.contains("Visible items") || r.contains("SPOOKY Forest"))
                    .unwrap_or_else(|| panic!("{ctx}: the cell path keeps the game's painted narration\n{}", rows.join("\n")));
                (last + 1, format!("the game's own painted narration ends at row {last}"))
            };
            let over_art: Vec<usize> = (0..reserved).filter(|&y| rows[y].contains("story line")).collect();
            assert!(
                over_art.is_empty(),
                "{ctx}: the story transcript must render clear of the artwork — story text on pane \
                 row(s) {over_art:?}, but {what}\n{}",
                rows.join("\n")
            );
            assert!(
                rows[reserved..].iter().any(|r| r.contains("story line")),
                "{ctx}: …and it is still on screen, below it\n{}",
                rows.join("\n")
            );
        }
    }
}

/// The spec's own "disappear altogether": Zork Zero splits the FULL 400px screen for
/// its title splash, so window 0 is tiled to a zero-height box one pixel past the
/// bottom. The game re-places it with its own `move_window` when the splash goes.
#[test]
fn a_full_screen_split_makes_the_story_window_disappear() {
    let Some(mut session) = boot("zork0-r393-s890714.z6") else { return };
    // Boot → survive Megaboz's curse → the 94-years cutscene ends and the game
    // splits the whole screen for the title splash (`v6_zork0_splash`'s driver).
    let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
    let mut seen = None;
    for _ in 0..16 {
        match session.pending_input() {
            InputKind::Line => {
                session.submit(lines.next().unwrap_or("wait"));
            }
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
        let _ = session.take_transcript();
        if win_box(&session, 1).1 > 300 && session.pending_input() == InputKind::Char {
            seen = Some(win_box(&session, 0));
            break;
        }
    }
    let Some((w, h, x, y)) = seen else { panic!("never reached the ZORK ZERO title splash") };
    assert_eq!(
        (w, h, x, y),
        (640, 0, 1, 401),
        "a split that takes the whole screen leaves window 0 with no height at all (ZMSD §8.8.4.1)"
    );
}

/// The render half. advent's Inform 6 v6 library reads window 0's post-split origin
/// back and moves its own prose window there, so with the split tiling correctly the
/// status bar stops being paint inside the story box and becomes band text above it.
/// The band is painted with the rest of the run stamping, AFTER the erase fills — a
/// window's erase is the ground its own text sits on, not a lid over it.
///
/// Falsified by moving the `draw_anchored_status_band` call back above the story
/// render: "frameless honor=true: advent's status bar survives its own window's
/// erase — no pane row carries 'End Of Road'".
#[test]
fn advents_status_bar_survives_its_own_windows_erase() {
    let Some(mut session) = boot("advent.z6") else { return };
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
    let _ = session.submit("look");
    let _ = session.take_transcript();

    // The library placed its prose window at the y the split gave window 0.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let story = items
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Buffer(b) if b.primary))
        .expect("a primary story window");
    assert_eq!(
        story.y_px, 20,
        "advent reads window 0's post-split y with @get_wind_prop and moves its own prose \
         window there — the bar's 20px strip is above it, not under it"
    );

    // The second arm was `frameless` until SQ-0895 removed it. What it contributed
    // was the CELL path as a contrast to hybrid's pixel ring, and it is reached now
    // by dropping the picker instead of by asking for a mode.
    for (tag, force_cell) in [("hybrid", false), ("cell", true)] {
        for honor in [true, false] {
            let mut state = app::state::AppState::default();
            state.colors = app::colors::ColorScheme::terminal_default();
            state.game_picker =
                (!force_cell).then(ratatui_image::picker::Picker::halfblocks);
            state.config.v6_render = app::config::V6RenderMode::Hybrid;
            state.config.honor_game_colours = honor;
            let area = Rect::new(0, 0, 80, 25);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let rows: Vec<String> = (0..area.height)
                .map(|y| {
                    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
                })
                .collect();
            let ctx = format!("{tag} honor={honor}");
            let bar_y = rows.iter().position(|r| r.contains("End Of Road")).unwrap_or_else(|| {
                panic!("{ctx}: advent's status bar survives its own window's erase — no pane row carries 'End Of Road'\n{}", rows.join("\n"))
            });
            assert_eq!(bar_y, 0, "{ctx}: and it is the pane's top row\n{}", rows.join("\n"));
            assert!(
                rows[bar_y].contains("Score:") && rows[bar_y].contains("Moves:"),
                "{ctx}: with its score/moves fields intact: {:?}",
                rows[bar_y]
            );
        }
    }
}
