//! SQ-1817: distinct game text rows never share a terminal row in the v6 hybrid ring.
//!
//! `run_cell` rounds every chrome run's row on its own. Under a pane scale below one
//! a terminal row spans more than a game row, so two lines the game drew 16 native
//! pixels apart can round onto ONE terminal row. The ring's text strip then holds
//! fewer rows than it has game rows, and the last lines of the strip are dropped (or,
//! drawn over art, overwrite each other). Shogun's title card is the specimen: between
//! 40 and 76 columns "Copyright (c) 1988 by Infocom" / "All rights reserved.", the
//! trademark line and the interpreter line were lost; at 52..57 columns it is the
//! copyright pair the report named. `ChromeRowOracle::compute_shifts` now pushes a
//! game row down exactly as far as it needs to clear the row above it.
//!
//! Specimen: `shogun-r322-s890706.z6` (release 322, serial 890706) booted as the IBM
//! PC press, ONE Enter past the splash picture (the title card with its nine lines;
//! the first frame is the splash art). Zork Zero r393 serial 890714 at boot is the
//! second specimen: its two-row status band collided the same way at 64..73 columns
//! ("Banquet0 ... Flath 0a"). Hybrid on the half-blocks backend, `honor_game_colours`
//! on (the shipped default); the host-facing `hybrid_chrome_layout` is checked on kitty.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The title card's nine lines, as prefixes short enough to survive the narrowest
/// pane's clipping, in the order Shogun prints them.
const CARD: [&str; 9] = [
    "SHOGUN",
    "A Story of Japan",
    "Copyright (c) 1988",
    "All rights",
    "SHOGUN is a",
    "Original Lit",
    "Licensed by",
    "Release 322",
    "IBM Interp",
];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str, profile: Option<InterpreterProfile>) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = match app::hints::load_story(&story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    let mut picts = PictSource::resolve(&story_path, None);
    let picture_dims = picts.all_pict_dims();
    let screen_px = picts.std_window().or_else(|| profile.and_then(|p| p.std_window()));
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.and_then(|p| p.interpreter_number()),
        false,
        picture_dims,
        screen_px,
        profile.and_then(|p| p.default_colours()),
        None,
    )
    .expect("v6 story should boot");
    if let Some(p) = profile {
        session.machine.set_palette(p.palette());
    }
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// The title card: Shogun r322 after one Enter.
fn title_card() -> Option<GameSession> {
    let mut s = boot("shogun-r322-s890706.z6", Some(InterpreterProfile::IbmPc))?;
    let _ = match s.pending_input() {
        InputKind::Line => s.submit(""),
        InputKind::Char => s.submit_char(13),
        InputKind::Event => s.submit(""),
    };
    Some(s)
}

fn render(model: &app::engine::ScreenModel, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.set_game_picker(Some(ratatui_image::picker::Picker::halfblocks()));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = true;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
}

/// The row each card line is on, or `None` where the line is not on screen.
fn card_rows(rows: &[String]) -> Vec<Option<usize>> {
    CARD.iter().map(|l| rows.iter().position(|r| r.contains(l))).collect()
}

/// Every line of the card is on the screen, one terminal row each, in print order,
/// across the whole 40..=100 column sweep (every third width at two taller panes).
///
/// FALSIFY by making `ChromeRowOracle::compute_shifts` return an empty map: lines are
/// missing from 40 up to 76 columns (the copyright pair at 52..57).
#[test]
fn title_card_lines_keep_their_own_rows_at_every_width() {
    let Some(s) = title_card() else { return };
    let model = s.screen();
    for pane_rows in [25u16, 30, 40] {
        // Every width at the common height; every third at the others (each render is slow in debug).
        let step = if pane_rows == 25 { 1 } else { 3 };
        for cols in (40u16..=100).step_by(step) {
            let (area, buf) = render(&model, cols, pane_rows);
            let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
            let at = card_rows(&rows);
            let where_ = format!("{cols}x{pane_rows}: {at:?}\n{}", rows.join("\n"));
            assert!(at.iter().all(Option::is_some), "a card line is missing at {where_}");
            let at: Vec<usize> = at.into_iter().flatten().collect();
            assert!(at.windows(2).all(|w| w[0] < w[1]), "rows are not strictly increasing at {where_}");
        }
    }
}

/// The two lines the report named are whole, not just present: both strings read
/// through at the reported width (54) and its neighbours.
#[test]
fn copyright_and_rights_lines_are_legible_at_the_reported_width() {
    let Some(s) = title_card() else { return };
    let model = s.screen();
    for cols in [52u16, 54, 57] {
        let (area, buf) = render(&model, cols, 25);
        let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
        let copy = rows.iter().position(|r| r.contains("Copyright (c) 1988 by Infocom"));
        let rights = rows.iter().position(|r| r.contains("All rights reserved."));
        assert!(copy.is_some() && rights.is_some(), "{cols} cols: {copy:?} {rights:?}\n{}", rows.join("\n"));
        assert_eq!(rights, copy.map(|c| c + 1), "{cols} cols: the rights line sits directly under the copyright line");
    }
}

/// Where nothing collided nothing moved: the rows main drew before the fix at 77..=100
/// columns are pinned (card lines on consecutive rows 3..=11, one lower at 100x40).
#[test]
fn wide_panes_are_unchanged() {
    let Some(s) = title_card() else { return };
    let model = s.screen();
    let first = |cols: u16, pane_rows: u16| {
        let (area, buf) = render(&model, cols, pane_rows);
        let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
        card_rows(&rows).into_iter().flatten().collect::<Vec<_>>()
    };
    let nine = |top: usize| (top..top + 9).collect::<Vec<_>>();
    for cols in [77u16, 80, 90, 100] {
        assert_eq!(first(cols, 25), nine(3), "{cols}x25");
    }
    assert_eq!(first(80, 40), nine(3), "80x40");
    assert_eq!(first(100, 40), nine(4), "100x40");
}

/// The host-facing layout agrees with the terminal: no two card lines share a row in
/// `hybrid_chrome_layout`, at the reported width and its neighbours, kitty at 8x16.
#[test]
fn host_chrome_layout_gives_each_card_line_its_own_row() {
    let Some(s) = title_card() else { return };
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let cell = zvm::screen::V6Cell::DEFAULT;
    let native = app::render::v6_layout::native_extent(items, &app::native_font::TextFace::cell_only(cell));
    let layout = app::render::v6_layout::classify_windows(items, cell);
    for cols in [52u16, 54, 57, 60, 70] {
        let mut st = app::state::AppState::default();
        st.colors = app::colors::ColorScheme::terminal_default();
        st.set_game_picker(Some(app::render::graphics::kitty_picker(8, 16)));
        st.config.v6_render = app::config::V6RenderMode::Hybrid;
        st.config.honor_game_colours = true;
        let pane = Rect::new(0, 0, cols, 25);
        let Some(ring) = app::render::screen::hybrid_chrome_layout(&layout, native, pane, (8, 16), &st) else {
            panic!("{cols} cols: the title card has a chrome ring");
        };
        let rows: Vec<Option<i32>> = CARD
            .iter()
            .map(|l| ring.runs.iter().find(|r| r.run.text.contains(l)).map(|r| r.row))
            .collect();
        assert!(rows.iter().all(Option::is_some), "{cols} cols: a card line is missing: {rows:?}");
        let rows: Vec<i32> = rows.into_iter().flatten().collect();
        assert!(rows.windows(2).all(|w| w[0] < w[1]), "{cols} cols: card rows not strictly increasing: {rows:?}");
    }
}

/// Zork Zero's status band had the same collision: "Banquet Hall" / "Flatheadia" were
/// overwritten by the Moves/Score row at 64..=73 columns.
#[test]
fn zork_zero_status_band_is_legible_where_it_used_to_collide() {
    let Some(s) = boot("zork0-r393-s890714.z6", None) else { return };
    let model = s.screen();
    for cols in [64u16, 68, 73] {
        let (area, buf) = render(&model, cols, 25);
        let rows: Vec<String> = (0..area.height).map(|y| row_text(&buf, area, y)).collect();
        let text = rows.join("\n");
        for w in ["Banquet Hall", "Flatheadia", "Moves:", "Score"] {
            assert!(text.contains(w), "{cols} cols: {w:?} missing\n{text}");
        }
    }
}
