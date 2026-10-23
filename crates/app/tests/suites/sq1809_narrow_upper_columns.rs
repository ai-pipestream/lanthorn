//! SQ-1809: what is drawn when the upper-window grid is wider than the pane.
//!
//! The grid is the GAME's screen (`upper.cols`, the width it was told); the pane may be
//! narrower. `render::upper_window::column_plan` decides which game columns are drawn
//! (blank columns first, then a status band's content middle-out, else a cursor-following
//! viewport), and a wider pane gets the row's own styling extended into its margins.
//!
//! Real-story cases skip vacuously without `stories/`; each names its release and turn
//! count and guards the frame's shape (a non-vacuity assertion) before measuring.
use std::path::PathBuf;

use app::colors::ColorScheme;
use app::engine::{GridWindow, WinNode};
use app::render::paneframe::{BorderStyle, PaneSides};
use app::render::upper_window::{column_plan, draw_upper_window, ColumnStrategy};
use app::session::{screen_model_from_machine, GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn story(file: &str) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(file);
    let b = std::fs::read(&path).ok();
    if b.is_none() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
    }
    b
}

fn boot(file: &str) -> Option<GameSession> {
    let bytes = story(file)?;
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, Some((2, 9)), None)
            .expect("story boots"),
    )
}

/// Drive one prompt: `cmd` at a line prompt, Enter at a char prompt.
fn step(s: &mut GameSession, cmd: &str) {
    let r = match s.pending_input() {
        InputKind::Line => s.submit(cmd),
        InputKind::Char => s.submit_char(13),
        InputKind::Event => s.submit(""),
    };
    assert!(r.fault.is_none(), "fault: {:?}", r.fault);
}

fn model_grid(s: &GameSession) -> GridWindow {
    match screen_model_from_machine(&s.machine).root {
        WinNode::Pair { first, .. } => match *first {
            WinNode::Grid(g) => g,
            other => panic!("expected a Grid, got {other:?}"),
        },
        other => panic!("expected a Pair root, got {other:?}"),
    }
}

fn colors() -> ColorScheme {
    let mut c = ColorScheme::terminal_default();
    c.virtual_window_border = BorderStyle::None;
    c.upper_window_border_sides = PaneSides::all(BorderStyle::None);
    c
}

fn render(grid: &GridWindow, w: u16, char_mode: bool, honor: bool) -> (Buffer, u16) {
    let area = Rect::new(0, 0, w, 30);
    let mut buf = Buffer::empty(area);
    let used = draw_upper_window(grid, char_mode, &colors(), area, &mut buf, honor, &mut Vec::new());
    (buf, used)
}

fn row(buf: &Buffer, y: u16, w: u16) -> String {
    (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect()
}

fn hline(g: &GridWindow, r: u16) -> String {
    (1..=g.cols).map(|c| g.cell(r, c).ch).collect()
}

// ── Zork I r52 (Solid Gold), a game that bakes an 80-column bar ─────────────

const ZORK_SG: &str = "zork1-invclues-r52-s871125.z5";

/// Zork I r52, two turns in (`look`, `north`), told 80 columns: the grid is 80 wide,
/// the bar `North of House ... Score: 0     Moves: 2`.
fn zork_after_two_turns() -> Option<(GameSession, GridWindow)> {
    let mut s = boot(ZORK_SG)?;
    step(&mut s, "look");
    step(&mut s, "north");
    let g = model_grid(&s);
    // Non-vacuity: the 80-column single-row band, or the cases below prove nothing.
    assert_eq!((g.cols, g.active_rows), (80, 1), "Zork I r52's baked bar");
    assert!(hline(&g, 1).starts_with(" North of House"), "{:?}", hline(&g, 1));
    assert!((1..=80).all(|c| g.cell(1, c).style & 1 != 0), "the bar is one reverse band");
    Some((s, g))
}

#[test]
fn zork1_r52_status_at_45_columns_reads_whole_after_a_move() {
    let Some((_s, g)) = zork_after_two_turns() else { return };
    for honor in [true, false] {
        let (buf, used) = render(&g, 45, false, honor);
        assert_eq!(used, 1);
        let bar = row(&buf, 0, 45);
        assert!(bar.starts_with(" North of House"), "honor={honor}: {bar:?}");
        assert!(bar.contains("Score: 0") && bar.trim_end().ends_with("Moves: 2"), "honor={honor}: {bar:?}");
        assert!(!bar.contains("0 2st"), "the garble of a clipped, mid-baked bar: {bar:?}");
        for x in 0..45 {
            assert!(
                buf.cell((x, 0)).unwrap().modifier.contains(Modifier::REVERSED),
                "honor={honor}: the band reaches column {x}"
            );
        }
    }
}

#[test]
fn zork1_r52_reverse_band_runs_to_both_pane_edges_in_a_wider_pane() {
    let Some((_s, g)) = zork_after_two_turns() else { return };
    for honor in [true, false] {
        let (buf, _) = render(&g, 120, false, honor);
        let bar = row(&buf, 0, 120);
        assert_eq!(bar.trim(), "North of House                                     Score: 0     Moves: 2");
        for x in [0u16, 10, 19, 20, 99, 100, 110, 119] {
            assert!(
                buf.cell((x, 0)).unwrap().modifier.contains(Modifier::REVERSED),
                "honor={honor}: pane column {x} is part of the band"
            );
        }
    }
}

#[test]
fn zork1_r52_hint_menu_at_45_columns_takes_the_viewport() {
    let Some(mut s) = boot(ZORK_SG) else { return };
    step(&mut s, "hint");
    step(&mut s, "hint"); // second turn: the InvisiClues menu
    let g = model_grid(&s);
    // Non-vacuity: the full-screen menu, a char read with the caret on the first topic.
    assert_eq!((g.cols, g.active_rows), (80, 23));
    assert!(hline(&g, 1).contains("INVISICLUES"), "{:?}", hline(&g, 1));
    assert!(hline(&g, 5).contains("Above Ground"), "{:?}", hline(&g, 5));
    assert!(g.cursor_active && s.pending_input() == InputKind::Char);

    let plan = column_plan(&g, g.active_rows, 45, g.cursor, true);
    assert_eq!(plan.strategy, ColumnStrategy::Viewport, "a menu is not a status band");
    assert_eq!(plan.kept, (1..=45).collect::<Vec<u16>>(), "the caret is in column 3: no scroll");
    let (buf, _) = render(&g, 45, true, true);
    let first: String = hline(&g, 5).chars().take(45).collect();
    assert_eq!(row(&buf, 4, 45), first, "drawn exactly the viewport's columns");
}

// ── Beyond Zork r57 ─────────────────────────────────────────────────────────

fn beyond_zork(steps: &[&str]) -> Option<GameSession> {
    let mut s = boot("beyondzork-r57-s871221.z5")?;
    for c in steps {
        step(&mut s, c);
    }
    Some(s)
}

#[test]
fn beyond_zork_character_menu_at_45_columns_drops_its_margins() {
    // Turn count: no, begin, 2 keypresses = the Character Setup menu.
    let Some(s) = beyond_zork(&["no", "begin", "", ""]) else { return };
    let g = model_grid(&s);
    assert_eq!((g.cols, g.active_rows), (80, 14));
    assert!(hline(&g, 5).contains("Character Setup"), "{:?}", hline(&g, 5));
    let plan = column_plan(&g, g.active_rows, 45, g.cursor, false);
    assert_eq!(plan.strategy, ColumnStrategy::DropBlank, "the menu's margins go, its text stays");
    let (buf, _) = render(&g, 45, false, true);
    let drawn = row(&buf, 4, 45);
    assert!(drawn.contains("Character Setup"), "{drawn:?}");
    assert!(row(&buf, 9, 45).contains("Create your own character"), "{:?}", row(&buf, 9, 45));
}

#[test]
fn beyond_zork_status_panel_at_45_columns_keeps_the_viewport_aligned() {
    // Turn count: no, begin, 8 keypresses/blank lines, then `look` = Hilltop with its panels.
    let Some(mut s) = beyond_zork(&["no", "begin", "", "", "", "", "", "", "", ""]) else { return };
    step(&mut s, "look");
    let g = model_grid(&s);
    assert_eq!((g.cols, g.active_rows), (80, 12));
    assert!(hline(&g, 1).contains("Hilltop") && hline(&g, 2).contains("EN:16"), "{:?}", hline(&g, 2));
    let plan = column_plan(&g, g.active_rows, 45, g.cursor, false);
    // The prose and compass art are not a status band.
    assert_eq!(plan.strategy, ColumnStrategy::Viewport);
    let (buf, _) = render(&g, 45, false, true);
    for r in 1..=12u16 {
        let expect: String = hline(&g, r).chars().take(45).collect();
        assert_eq!(row(&buf, r - 1, 45), expect, "row {r} is one aligned slice of the game's row");
    }
}

// ── Bureaucracy r116: the form's field stays visible ────────────────────────

#[test]
fn bureaucracy_form_keeps_the_typed_field_visible_at_45_columns() {
    let Some(mut s) = boot("bureaucracy-r116-s870602.z4") else { return };
    // Turn count: five prompts answered with Enter/blank = the licence form.
    for _ in 0..5 {
        step(&mut s, "");
    }
    let g = model_grid(&s);
    assert_eq!((g.cols, g.active_rows), (80, 23));
    assert!(hline(&g, 5).contains("SOFTWARE LICENCE APPLICATION"), "{:?}", hline(&g, 5));
    assert!(g.cursor_active);
    let (crow, ccol) = g.cursor;
    let plan = column_plan(&g, g.active_rows, 45, g.cursor, true);
    assert_ne!(plan.strategy, ColumnStrategy::Fits);
    let at = plan.screen_col(ccol).expect("the caret's column is never dropped");
    for honor in [true, false] {
        let (buf, _) = render(&g, 45, true, honor);
        let label = hline(&g, crow);
        assert!(label.contains("Street name:"), "{label:?}");
        assert!(
            row(&buf, crow - 1, 45).contains("Street name:"),
            "the field's label stays: {:?}",
            row(&buf, crow - 1, 45)
        );
        assert!(
            buf.cell((at, crow - 1)).unwrap().modifier.contains(Modifier::REVERSED),
            "honor={honor}: caret drawn at screen column {at}"
        );
    }
}

// ── The Hints tab: standalone InvisiClues programs are told at least 72 ─────

#[test]
fn hints_tab_floors_a_narrow_panel_at_72_and_draws_through_the_column_plan() {
    use app::hints_tab::{self, HINTS_MIN_COLS};
    use app::journal::JournalTab;
    use app::state::{AppState, HintSource};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    for file in ["stationinv.z5", "spellbinv.z5"] {
        let path = root.join(file);
        if !path.is_file() {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            continue;
        }
        let mut st = AppState::default();
        st.config.user_dir = app::scratch_dir("sq1809-hints-floor");
        st.set_journal_tab(JournalTab::Hints);
        hints_tab::show_program(&mut st, path.clone());
        assert!(hints_tab::ensure_started_in(&mut st, &path, "SQ1809", None), "{file} starts");
        // A 50-column panel: 52 outer, less the frame.
        let outer = Rect::new(0, 0, 52, 40);
        let mut buf = Buffer::empty(outer);
        hints_tab::draw(&st, outer, &mut buf);
        let (_, cols) = st.hints_tab.panel_screen().expect("drawn");
        assert_eq!(cols, HINTS_MIN_COLS, "{file}: a 50-column panel is told the floor");
        hints_tab::sync_screen_dims(&mut st);
        // Past the banner into the topic menu (turn count: one keypress).
        {
            let HintSource::Zcode(vm) = &mut st.overlays.hints.as_mut().unwrap().source;
            step(vm, "");
        }
        let HintSource::Zcode(vm) = &st.overlays.hints.as_ref().unwrap().source;
        let g = model_grid(vm);
        assert_eq!(g.cols, HINTS_MIN_COLS, "{file}: the program laid itself out at the floor");
        assert!(hline(&g, 1).contains("N=Next"), "{file}: non-vacuity, the title/legend row: {:?}", hline(&g, 1));
        let mut buf = Buffer::empty(outer);
        hints_tab::draw(&st, outer, &mut buf);
        let hits = st.hints_tab.hits();
        let line: String = (hits.area.x..hits.area.right())
            .map(|x| buf.cell((x, hits.area.y)).unwrap().symbol().to_string())
            .collect();
        let plan = column_plan(&g, g.active_rows, hits.area.width, g.cursor, false);
        let expect: String = plan.kept.iter().map(|&c| g.cell(1, c).ch).collect();
        assert_eq!(line, expect, "{file}: row 1 is the game's row through the plan, no collision");
    }
}
