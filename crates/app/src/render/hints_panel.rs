//! Hints panel modal overlay — a centered mini-terminal for the hint session.
//!
//! When `AppState.hints` is `Some(HintSession)`, `draw_hints_panel` renders a
//! dialog with the hint session transcript, an optional built-in-HINT suggestion
//! line, and an input row.  It mirrors the `draw_gallery`/`draw_reset_dialog`
//! pattern: dialog chrome via `draw_dialog`, content drawn into `rects.content`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogSpec, DialogStyle, Placement, draw_dialog};
use crate::render::transcript::wrap_line;
use crate::state::AppState;

// Minimum dimensions for the hints panel.
const MIN_W: u16 = 40;
const MIN_H: u16 = 10;

// Dialog dimensions (capped by terminal size at render time).
const DIALOG_W: u16 = 70;
const DIALOG_H: u16 = 24;

// ── HintsPanelRects ───────────────────────────────────────────────────────────

/// Hit-rects returned by `draw_hints_panel` for mouse event routing.
pub struct HintsPanelRects {
    /// Full dialog area (border included).
    pub area: Rect,
    /// The [X] close button, if rendered.
    pub close: Option<Rect>,
    /// The input row inside the dialog content area.
    pub input: Rect,
}

// ── draw_hints_panel ──────────────────────────────────────────────────────────

/// Draw the Hints panel modal centered over `area`.
///
/// Returns `None` when `state.hints` is `None` or the area is too small.
/// Returns `Some(HintsPanelRects)` with hit-rects for the close button and
/// the input row.
pub fn draw_hints_panel(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<HintsPanelRects> {
    let session = state.hints.as_ref()?;

    let modal_w = DIALOG_W.min(area.width.saturating_sub(4));
    let modal_h = DIALOG_H.min(area.height.saturating_sub(2));

    if modal_w < MIN_W || modal_h < MIN_H {
        return None;
    }

    // Build DialogStyle from state colors (mirrors gallery.rs / reset_dialog.rs).
    let st = DialogStyle {
        frame: state.colors.dialog,
        box_style: state.colors.dialog_box_style,
        glyphs: state.colors.dialog_glyphs.clone(),
        title: state.colors.dialog_title,
        button: state.colors.dialog_button,
        button_active: state.colors.dialog_button_active,
        shadow: state.colors.dialog_shadow,
        shadow_on: state.colors.dialog_shadow_on,
    };

    let buttons = &[DialogButton { id: ButtonId::Close, label: "Close" }];
    let spec = DialogSpec {
        title: &session.label,
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Close),
        focus: None,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    if content.height == 0 || content.width == 0 {
        return Some(HintsPanelRects {
            area: rects.area,
            close: rects.close,
            input: Rect::new(content.x, content.y, content.width, 0),
        });
    }

    // The last row of content is always the input row.
    // Everything above it is the transcript area (possibly preceded by the
    // built-in-HINT suggestion line).
    let input_y = content.bottom().saturating_sub(1);
    let input_rect = Rect::new(content.x, input_y, content.width, 1);

    // Draw "> <input>" on the input row.
    let input_line = format!("> {}", session.input);
    let input_style = state.colors.dialog;
    crate::render::draw_str_clipped(buf, content.x, input_y, &input_line, input_style, content);

    // The transcript display area: content rows above the input row.
    if content.height < 2 {
        return Some(HintsPanelRects { area: rects.area, close: rects.close, input: input_rect });
    }
    let transcript_area = Rect::new(content.x, content.y, content.width, content.height - 1);

    // Draw the content, bottom-up from the input row:
    //   (a) builtin_hint suggestion line (if set) — topmost reserved row.
    //   (b) wrapped transcript lines, scrolled by session.scroll.

    // Decide how many rows the built-in hint line occupies (0 or 1).
    let hint_row_count: u16 = if session.builtin_hint { 1 } else { 0 };

    // Draw built-in hint suggestion on the very first content row (row 0 of transcript_area).
    if session.builtin_hint && transcript_area.height >= 1 {
        let suggestion = "This game has its own hints \u{2014} type HINT in the story.";
        let dim_style = Style::new()
            .fg(Color::Yellow)
            .add_modifier(Modifier::DIM)
            .patch(state.colors.dialog);
        crate::render::draw_str_clipped(
            buf,
            transcript_area.x,
            transcript_area.y,
            suggestion,
            dim_style,
            transcript_area,
        );
    }

    // Transcript body area: below the hint suggestion line.
    if transcript_area.height <= hint_row_count {
        return Some(HintsPanelRects { area: rects.area, close: rects.close, input: input_rect });
    }
    let body_top = transcript_area.y + hint_row_count;
    let body_h = transcript_area.bottom() - body_top;
    let body_area = Rect::new(transcript_area.x, body_top, transcript_area.width, body_h);

    // Word-wrap each logical transcript line to the content width, then display
    // the window of `body_h` rows honoring `session.scroll`.
    let wrapped: Vec<String> = session
        .transcript
        .iter()
        .flat_map(|line| wrap_line(line, body_area.width))
        .collect();

    let n = wrapped.len();
    let scroll = session.scroll as usize;
    let rows = body_h as usize;
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    let visible = &wrapped[start..end];

    let body_style = state.colors.dialog;
    for (i, line) in visible.iter().enumerate() {
        let row_y = body_top + i as u16;
        if row_y >= body_area.bottom() {
            break;
        }
        crate::render::draw_str_clipped(buf, body_area.x, row_y, line, body_style, body_area);
    }

    Some(HintsPanelRects { area: rects.area, close: rects.close, input: input_rect })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a minimal `HintSession` backed by the minizork.z3 fixture.
    ///
    /// The fixture path is resolved relative to CARGO_MANIFEST_DIR (the app
    /// crate root). If the fixture is absent the test is skipped (the helper
    /// returns `None`).
    fn make_hint_session() -> Option<crate::state::HintSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None;
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes).expect("GameSession::new");
        Some(crate::state::HintSession {
            source: crate::state::HintSource::Zcode(session),
            transcript: vec!["pick a topic".to_string()],
            scroll: 0,
            input: "3".to_string(),
            label: "Hints: X".to_string(),
            builtin_hint: true,
        })
    }

    #[test]
    fn hints_panel_renders_title_transcript_suggestion_and_input() {
        let Some(hint_session) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };

        let mut state = crate::state::AppState::default();
        state.hints = Some(hint_session);

        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;

        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();

        let r = rects.expect("draw_hints_panel should return rects when hints is Some");
        assert!(r.close.is_some(), "close button rect should be present");

        // Collect all rendered chars into a flat string for assertions.
        let all: String = terminal.backend().buffer().content().iter()
            .flat_map(|c| c.symbol().chars())
            .collect();

        assert!(all.contains("Hints: X"), "title 'Hints: X' must appear in the buffer");
        assert!(all.contains("pick a topic"), "transcript text must appear in the buffer");
        assert!(all.contains("HINT"), "built-in hint suggestion ('type HINT') must appear");
        assert!(all.contains("3"), "input '3' must appear in the buffer");
    }

    #[test]
    fn hints_panel_returns_none_when_no_session() {
        let state = crate::state::AppState::default(); // hints = None
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;
        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();
        assert!(rects.is_none(), "draw_hints_panel must return None when hints is None");
    }

    #[test]
    fn hints_panel_returns_none_on_small_terminal() {
        let Some(hint_session) = make_hint_session() else {
            eprintln!("SKIP: minizork.z3 fixture absent");
            return;
        };
        let mut state = crate::state::AppState::default();
        state.hints = Some(hint_session);

        let backend = TestBackend::new(20, 5); // too small
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rects: Option<HintsPanelRects> = None;
        terminal.draw(|f| {
            rects = draw_hints_panel(&state, f.area(), f.buffer_mut());
        }).unwrap();
        assert!(rects.is_none(), "draw_hints_panel must return None on very small terminals");
    }
}
