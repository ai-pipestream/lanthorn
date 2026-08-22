//! Zork Zero's post-intro title screen + status repaint (SQ-0497 / SQ-0498).
//!
//! Both defects surface right after the Banquet Hall prologue: the player
//! survives Megaboz's curse (dives under a table), the game freezes everyone for
//! 94 years, and on the next keypress it puts up the full-screen "ZORK ZERO — The
//! Revenge of Megaboz" title splash before dropping the adventurer into the
//! Great Hall.
//!
//! * SQ-0497: the splash is `@draw_picture(1)` into window 1 *after*
//!   `@split_window(400)` sizes that window to the whole 400px screen. v6 used
//!   to treat `split_window` as a no-op, so the picture clipped to window 1's
//!   78px banner box and the still-open story window painted the transcript over
//!   it. Honoring the split resizes window 1 to full screen (splash fills it) and
//!   collapses window 0 to zero height (no story viewport is carved over it).
//! * SQ-0498: entering the Great Hall repaints the status location. The game
//!   blanks the previous, longer name ("Banquet Hall") with all-space runs, then
//!   paints the shorter "Great Hall". Those blanks used to be non-erasing, so the
//!   old tail survived ("Great Hall" + a stale "ll" = "Great Hallll").
//!
//! Story asset is gitignored (local-only) → this test **skips cleanly** when
//! absent, mirroring the other `zork0_v6_*` tests.
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

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot_zork0() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, true, false, None, false, picture_dims, picts.std_window(), None, None)
            .expect("Zork0 (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// Window 1's live pixel height (the upper/banner window). 0 if no v6 state.
fn win1_h(session: &GameSession) -> u16 {
    session.machine.screen.v6.as_ref().map(|v| v.windows[1].y_size).unwrap_or(0)
}
fn win0_h(session: &GameSession) -> u16 {
    session.machine.screen.v6.as_ref().map(|v| v.windows[0].y_size).unwrap_or(0)
}

/// Drive boot → "get under table" (survive the curse) → keep advancing until the
/// game splits window 1 to the full screen for the title splash. Returns true
/// with the session parked exactly on the splash (pending a keypress).
fn drive_to_splash(session: &mut GameSession) -> bool {
    let mut lines = ["get under table", "wait", "wait", "wait", "wait", "wait"].into_iter();
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
        // The split_window(400) that sets up the title fires the instant the
        // 94-years cutscene ends; window 1 jumps from the 78px banner to the full
        // 400px screen, and the game waits for a keypress on the splash.
        if win1_h(session) > 300 && session.pending_input() == InputKind::Char {
            return true;
        }
    }
    false
}

#[test]
fn zork0_title_splash_fills_the_screen_and_carves_no_story_viewport() {
    let Some(mut session) = boot_zork0() else { return };
    assert!(drive_to_splash(&mut session), "never reached the ZORK ZERO title splash");

    // SQ-0497 engine fix: split_window(400) resized window 1 to the whole screen
    // and gave window 0 the (zero) remainder.
    assert!(win1_h(&session) > 300, "split_window sized the title window to full screen: got {}", win1_h(&session));
    assert_eq!(win0_h(&session), 0, "the story window collapsed to zero height under the full-screen split");

    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 root is Layered") };

    // The splash picture (window 1) is a full-screen graphics window, NOT clipped
    // to the 78px banner: its canvas covers the whole 400px screen height.
    let splash = items.iter().find_map(|pw| match &pw.node {
        WinNode::Graphics(g) if g.win == 1 => Some((g.canvas.width(), g.canvas.height(), pw.h_px)),
        _ => None,
    });
    let (_cw, ch, box_h) = splash.expect("window 1 carries the splash graphics");
    assert!(ch > 300 && box_h > 300, "splash canvas/box fill the screen (not the 78px banner): canvas_h={ch}, box_h={box_h}");

    // The core of the render fix: with window 0 collapsed there is NO story
    // window to classify, so the hybrid renderer never carves an inset text
    // viewport over the picture — it falls through to the full-art path.
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    assert!(layout.story.is_none(), "no story viewport is carved over the full-screen splash");

    // And the hybrid render must not paint the prior transcript over the splash.
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // Seed a distinctive transcript line as the running app would have.
    state.push_transcript("The king is dead!");
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let screen: String = (0..area.height)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .map(|(x, y)| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
        .collect();
    assert!(
        !screen.contains("king is dead"),
        "the stale story transcript must not be painted over the title splash:\n{screen}"
    );
}

#[test]
fn zork0_great_hall_status_has_no_stale_tail() {
    let Some(mut session) = boot_zork0() else { return };
    assert!(drive_to_splash(&mut session), "never reached the ZORK ZERO title splash");

    // Two keypresses past the splash: dismiss the title, then the "94 years
    // later" intro, landing in the Great Hall with its status repainted.
    for _ in 0..4 {
        if session.pending_input() == InputKind::Char {
            let r = session.submit_char(13);
            if r.transcript.contains("Great Hall") {
                break;
            }
        } else {
            session.submit("look");
        }
    }

    let v6 = session.machine.screen.v6.as_ref().expect("v6 state");
    // The location sits on the banner ribbon (window 1) at native row 11. Collect
    // every non-blank painted run in the LEFT (location) half of that row.
    let loc_runs: Vec<(u16, String)> = v6.windows[1]
        .texts
        .iter()
        .filter(|t| t.y <= 20 && t.x < 300 && !t.text.trim().is_empty())
        .map(|t| (t.x, t.text.clone()))
        .collect();

    // Exactly one location run — "Great Hall" at the ribbon's left edge — with NO
    // stray remnant of the previous, longer "Banquet Hall" ("e" at x=111, "ll" at
    // x=151) surviving to its right. (SQ-0498)
    assert_eq!(
        loc_runs,
        vec![(71, "Great Hall".to_string())],
        "the shorter Great Hall must fully replace Banquet Hall with no stale tail; got runs: {loc_runs:?}"
    );
}
