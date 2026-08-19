//! SQ-0703: `input_line` / `suggestion_line` must be boxable from a REAL style.toml.
//!
//! The shipped template documents the spelling in so many words:
//!
//! ```toml
//! input_line      = { parent = "line" }   # add style = "single" to box it
//! suggestion_line = { parent = "line" }   # add style = "single" to box the popup
//! ```
//!
//! The renderer has always been able to draw both frames — `render_transcript`
//! boxes the prompt when `input_line_style != None`, and `render_middle` boxes
//! the completion popup when `suggestion_line_style != None`. What was missing
//! was the last hop: those two `ColorScheme` fields had exactly one writer, the
//! LEGACY `[colors]` selector table, while the shipped schema puts both
//! selectors in `[elements]`, where `style` resolved into the theme and stopped.
//! Same defect class as `upper_window_border` in SQ-0700.
//!
//! So these tests go through the loader the app actually uses
//! (`reload::reload_style` over a file on disk) in the SHIPPED schema — the
//! spelling a user writes — never by poking the `ColorScheme` fields directly.
//! The in-crate render tests already do the latter, and they passed the whole
//! time the documented spelling was inert.
//!
//! Nothing here needs a story file: the prompt bar and the completion popup are
//! app chrome, so every test runs everywhere.

use std::path::PathBuf;

use app::engine::StatusModel;
use app::render::paneframe::{BorderStyle, PaneSides};
use app::state::AppState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lanthorn-isb-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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

/// Render the game pane and return its rows as strings, so a test can look for
/// frame chrome where it expects it.
fn render_rows(state: &AppState, area: Rect) -> Vec<String> {
    let mut buf = Buffer::empty(area);
    app::render::transcript::render_transcript(
        &StatusModel::HostManaged,
        None,
        state,
        area,
        &mut buf,
        None,
    );
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect()
        })
        .collect()
}

// ── input_line ───────────────────────────────────────────────────────────────

/// The documented spelling, from the file a user actually edits: the prompt gets
/// a box — resolved onto the fields the renderer frames with, and drawn.
#[test]
fn elements_input_line_style_single_boxes_the_prompt() {
    let (mut state, dir) = state_with_style(
        "input-on",
        "version = 1\n\n[elements]\ninput_line = { style = \"single\" }\n",
    );

    assert_eq!(
        state.colors.input_line_style,
        BorderStyle::Single,
        "style = \"single\" must reach the field render_transcript boxes from"
    );
    assert_eq!(state.colors.input_line_sides, PaneSides::all(BorderStyle::Single));

    // And it draws: the bottom three rows become the framed prompt.
    state.config.command_bar = true;
    state.input.set("take lamp", true);
    let rows = render_rows(&state, Rect::new(0, 0, 60, 12));
    assert!(rows[9].starts_with('┌'), "framed prompt's top border: {:?}", rows[9]);
    assert!(rows[10].starts_with('│'), "framed prompt's content row: {:?}", rows[10]);
    assert!(rows[10].contains("take lamp"), "the input renders inside the box: {:?}", rows[10]);
    assert!(rows[11].starts_with('└'), "framed prompt's bottom border: {:?}", rows[11]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The control: a style.toml that only colours `input_line` keeps the shipped
/// default — a flat one-row prompt, no chrome — so the test above cannot pass
/// vacuously and a colour-only theme is not silently given a box.
#[test]
fn a_colour_only_input_line_stays_unframed() {
    let (mut state, dir) = state_with_style(
        "input-off",
        "version = 1\n\n[elements]\ninput_line = { fg = \"green\" }\n",
    );

    assert_eq!(state.colors.input_line_style, BorderStyle::None);
    assert_eq!(state.colors.input_line_sides, PaneSides::all(BorderStyle::None));

    state.config.command_bar = true;
    state.input.set("take lamp", true);
    let rows = render_rows(&state, Rect::new(0, 0, 60, 12));
    assert!(
        rows.iter().all(|r| !r.contains('┌') && !r.contains('└')),
        "an unstyled input line draws no frame: {rows:?}"
    );
    assert!(rows[11].contains("take lamp"), "the prompt is the flat bottom row: {:?}", rows[11]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Per-side overrides layer over the base exactly as every other border
/// selector's do: a named side wins, an unnamed one follows `style`.
#[test]
fn input_line_per_side_overrides_layer_over_the_base_style() {
    let (state, dir) = state_with_style(
        "input-sides",
        "version = 1\n\n[elements]\ninput_line = { style = \"none\", style_top = \"double\" }\n",
    );
    let sides = state.colors.input_line_sides;
    assert_eq!(sides.top, BorderStyle::Double, "the named side wins");
    assert_eq!(sides.bottom, BorderStyle::None, "an unnamed side follows the base style");
    assert_eq!(sides.left, BorderStyle::None);
    assert_eq!(sides.right, BorderStyle::None);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── suggestion_line ──────────────────────────────────────────────────────────

/// The other documented spelling: the command-palette popup gets a box.
#[test]
fn elements_suggestion_line_style_single_boxes_the_popup() {
    let (mut state, dir) = state_with_style(
        "sug-on",
        "version = 1\n\n[elements]\nsuggestion_line = { style = \"single\" }\n",
    );

    assert_eq!(
        state.colors.suggestion_line_style,
        BorderStyle::Single,
        "style = \"single\" must reach the field render_middle boxes from"
    );
    assert_eq!(state.colors.suggestion_line_sides, PaneSides::all(BorderStyle::Single));

    // And it draws: the popup becomes a 3-row framed mini-window above the prompt.
    state.input.set("/pan", true);
    state.suggestions = vec!["panh".into(), "panv".into()];
    state.suggestion_idx = 0;
    let rows = render_rows(&state, Rect::new(0, 0, 60, 12));
    let boxed = rows.iter().any(|r| r.contains('┌'))
        && rows.iter().any(|r| r.contains("panh") && r.contains('│'));
    assert!(boxed, "the completion popup must be framed with its text inside: {rows:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The control: colouring the popup does not box it.
#[test]
fn a_colour_only_suggestion_line_stays_a_flat_strip() {
    let (mut state, dir) = state_with_style(
        "sug-off",
        "version = 1\n\n[elements]\nsuggestion_line = { fg = \"green\" }\n",
    );

    assert_eq!(state.colors.suggestion_line_style, BorderStyle::None);
    assert_eq!(state.colors.suggestion_line_sides, PaneSides::all(BorderStyle::None));

    state.input.set("/pan", true);
    state.suggestions = vec!["panh".into(), "panv".into()];
    state.suggestion_idx = 0;
    let rows = render_rows(&state, Rect::new(0, 0, 60, 12));
    assert!(rows.iter().any(|r| r.contains("panh")), "the strip still shows: {rows:?}");
    assert!(
        rows.iter().all(|r| !r.contains('┌') && !r.contains('└')),
        "an unstyled suggestion line draws no frame: {rows:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── the other spellings must survive ─────────────────────────────────────────

/// The legacy `[colors]` selector table still configures both — the bridge must
/// not hijack a scheme the old spelling already set.
#[test]
fn the_legacy_colors_spelling_still_boxes_both() {
    let (state, dir) = state_with_style(
        "legacy",
        "[colors]\n\"input_line\" = { style = \"single\" }\n\"suggestion_line\" = { style = \"double\" }\n",
    );
    assert_eq!(state.colors.input_line_sides, PaneSides::all(BorderStyle::Single));
    assert_eq!(state.colors.suggestion_line_sides, PaneSides::all(BorderStyle::Double));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lowering three selectors must stay three independent lowerings: boxing the
/// prompt may not disturb the popup, the upper-window frame, or vice versa.
#[test]
fn the_three_frameable_selectors_stay_independent() {
    let (state, dir) = state_with_style(
        "independent",
        "version = 1\n\n[elements]\ninput_line = { style = \"double\" }\n",
    );
    assert_eq!(state.colors.input_line_sides, PaneSides::all(BorderStyle::Double));
    assert_eq!(
        state.colors.suggestion_line_sides,
        PaneSides::all(BorderStyle::None),
        "a silent suggestion_line keeps its default"
    );
    assert_eq!(
        state.colors.upper_window_border_sides,
        PaneSides::all(BorderStyle::None),
        "and so does a silent upper_window_border (SQ-0700's frameless default)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
