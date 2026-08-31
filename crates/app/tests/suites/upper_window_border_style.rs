//! SQ-0700: the upper-window frame must be switchable off from a REAL style.toml.
//!
//! The frame lanthorn draws around a v4+ game's status/upper window is ours, not
//! the game's — so the theme owns it, and `upper_window_border = { style = "none" }`
//! is how a player turns it off. The reported symptom was that it did nothing for
//! `anchor.z8` (a v8 story): the frame stayed, and kept its row/column reservation.
//!
//! These tests go through the loader the app actually uses (`reload::reload_style`
//! over a file on disk, in the SHIPPED `style.toml` schema — the one
//! `style.example.toml` seeds), never the internal compile helper, because the
//! schema the file is written in is exactly what the bug was about.
//!
//! The render half drives the real story, so it skips vacuously when the
//! gitignored fixture is absent; the loader half always runs.

use std::path::PathBuf;

use app::engine::{Engine, GridWindow, WinNode};
use app::render::paneframe::BorderStyle;
use app::render::upper_window::draw_upper_window;
use app::session::{GameSession, InputKind};
use app::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;


fn temp_dir(tag: &str) -> PathBuf {
    app::scratch_dir(&format!("uwb-{tag}"))
}

/// Write `style_toml` into a fresh user dir and load it exactly the way the app
/// does at startup and on `/reload style`.
fn state_with_style(tag: &str, style_toml: &str) -> (AppState, PathBuf) {
    let dir = temp_dir(tag);
    let path = dir.join("style.toml");
    std::fs::write(&path, style_toml).unwrap();

    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.config.style = Some(path.to_string_lossy().to_string());
    match app::reload::reload_style(&mut state) {
        app::reload::ReloadOutcome::Reloaded { .. } => {}
        app::reload::ReloadOutcome::Failed { msg } => panic!("style.toml must load: {msg}"),
    }
    (state, dir)
}

/// Anchorhead, driven past its two startup quote boxes to an ordinary turn whose
/// upper window is the one-row status line.
fn anchor_in_play() -> Option<GameSession> {
    let path = fixture_path("anchor.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("anchor.z8 should load and boot without a ZError");
    for _ in 0..4 {
        if matches!(s.pending_input(), InputKind::Char) {
            let _ = s.submit_char(13);
        }
    }
    let _ = s.submit("look");
    Some(s)
}

fn find_grid(n: &WinNode) -> Option<&GridWindow> {
    match n {
        WinNode::Grid(g) => Some(g),
        WinNode::Pair { first, second, .. } => find_grid(first).or_else(|| find_grid(second)),
        _ => None,
    }
}

/// Render the story's live upper window with `state`'s resolved colours, and
/// report (rows consumed, every box-drawing glyph the frame put on screen).
fn render_upper(session: &GameSession, state: &AppState) -> (u16, Vec<String>) {
    let model = session.screen();
    let grid = find_grid(&model.root).expect("a v8 story in play has an upper window");
    assert!(grid.active_rows > 0, "the status line is one active row");

    let area = Rect::new(0, 0, 100, 10);
    let mut buf = Buffer::empty(area);
    let used = draw_upper_window(grid, false, &state.colors, area, &mut buf, true, &mut Vec::new());

    let mut chrome = Vec::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let sym = buf.cell((x, y)).unwrap().symbol().to_string();
            if sym.chars().next().is_some_and(|c| ('\u{2500}'..='\u{257f}').contains(&c)) {
                chrome.push(sym);
            }
        }
    }
    (used, chrome)
}

// ── The reported bug ─────────────────────────────────────────────────────────

/// `[elements] upper_window_border = { style = "none" }` — the spelling the
/// shipped template's section layout implies — must reach the renderer: no frame
/// glyphs, no extra rows consumed, and no rows/columns reserved in the size the
/// story is told it has.
#[test]
fn style_none_turns_the_upper_window_frame_off() {
    let (state, dir) = state_with_style(
        "off",
        "version = 1\n\n[elements]\nupper_window_border = { style = \"none\" }\n",
    );

    // Resolved scheme: every side off.
    assert_eq!(
        state.colors.upper_window_border_sides,
        app::render::paneframe::PaneSides::all(BorderStyle::None),
        "style = \"none\" must reach the sides the renderer frames with"
    );
    assert_eq!(state.colors.virtual_window_border, BorderStyle::None);

    // Nothing reserved: the story keeps the pane's full height.
    let (rows, _cols) = app::render::screen::story_screen_dims(Rect::new(0, 0, 100, 30), &state)
        .expect("a non-empty pane reports a screen size");
    assert_eq!(rows, 30, "a frameless upper window reserves no rows from the story's screen");

    // Nothing drawn: the real story, rendered.
    let Some(session) = anchor_in_play() else { return };
    let (used, chrome) = render_upper(&session, &state);
    assert!(chrome.is_empty(), "no frame may be drawn around a frameless upper window: {chrome:?}");
    assert_eq!(used, 1, "a frameless one-row status line consumes exactly its content row");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The control: the same file asking for a frame gets one, drawn and paid for —
/// so the test above cannot pass vacuously. (The shipped default is frameless
/// since SQ-0700, so this is the side that now has to be spelled out.)
#[test]
fn style_single_draws_the_upper_window_frame() {
    let (state, dir) = state_with_style(
        "on",
        "version = 1\n\n[elements]\nupper_window_border = { style = \"single\" }\n",
    );

    assert_eq!(
        state.colors.upper_window_border_sides,
        app::render::paneframe::PaneSides::all(BorderStyle::Single),
        "style = \"single\" must reach the sides the renderer frames with"
    );
    let (rows, _cols) = app::render::screen::story_screen_dims(Rect::new(0, 0, 100, 30), &state)
        .expect("a non-empty pane reports a screen size");
    assert_eq!(rows, 28, "a framed upper window costs the story its top and bottom rows");

    let Some(session) = anchor_in_play() else { return };
    let (used, chrome) = render_upper(&session, &state);
    assert!(chrome.contains(&"┌".to_string()), "the requested frame is drawn: {chrome:?}");
    assert_eq!(used, 3, "one content row plus the top and bottom border rows");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The shipped default (SQ-0700): a style.toml that says nothing about the frame
/// gets no frame — the game's status line sits flush against the story, and the
/// whole pane is the screen we declare to it.
#[test]
fn the_default_upper_window_is_frameless() {
    let (state, dir) = state_with_style("default", "version = 1\n\n[elements]\ntranscript = { fg = \"green\" }\n");

    assert_eq!(
        state.colors.upper_window_border_sides,
        app::render::paneframe::PaneSides::all(BorderStyle::None),
        "the built-in default draws no frame around a game's upper window"
    );
    let (rows, _cols) = app::render::screen::story_screen_dims(Rect::new(0, 0, 100, 30), &state)
        .expect("a non-empty pane reports a screen size");
    assert_eq!(rows, 30, "and so costs the story no rows");

    let Some(session) = anchor_in_play() else { return };
    let (used, chrome) = render_upper(&session, &state);
    assert!(chrome.is_empty(), "nothing framed out of the box: {chrome:?}");
    assert_eq!(used, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Per-side overrides layer over the base the same way every other border
/// selector's do: an explicit side wins, the rest follow `style`.
#[test]
fn per_side_overrides_layer_over_the_base_style() {
    let (state, dir) = state_with_style(
        "sides",
        "version = 1\n\n[elements]\nupper_window_border = { style = \"none\", style_top = \"double\" }\n",
    );
    let sides = state.colors.upper_window_border_sides;
    assert_eq!(sides.top, BorderStyle::Double, "the named side wins");
    assert_eq!(sides.bottom, BorderStyle::None, "an unnamed side follows the base style");
    assert_eq!(sides.left, BorderStyle::None);
    assert_eq!(sides.right, BorderStyle::None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The legacy `[colors]` selector table still works — the bridge must not
/// hijack a scheme that the old spelling already configured.
#[test]
fn the_legacy_colors_spelling_still_turns_the_frame_off() {
    let (state, dir) = state_with_style(
        "legacy",
        "[colors]\n\"upper_window_border\" = { style = \"none\" }\n",
    );
    assert_eq!(
        state.colors.upper_window_border_sides,
        app::render::paneframe::PaneSides::all(BorderStyle::None)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
