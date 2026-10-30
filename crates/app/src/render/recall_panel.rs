//! Ephemeral recall excerpts. This is a normal text modal even for games whose
//! main display is a raster image. Nothing drawn here enters the transcript.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, Borders, Clear, Widget},
};

use super::{draw_str_clipped, transcript::wrap_line};
use crate::state::AppState;

fn content_area(area: Rect) -> Rect {
    // At three rows tall the sole interior row belongs to the excerpt. The
    // footer is optional; reserving it here would make scrolling useless.
    let footer_rows = u16::from(area.height >= 4);
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2 + footer_rows),
    )
}

fn lines(state: &AppState, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let query = state.search_query.as_deref().unwrap_or("");
    let query: String = query.chars().take(120).collect();
    let mut body = format!("Query: {query}\n\n");
    if state.recall_pending_id.is_some() {
        body.push_str("Preparing local search in the background.\n\nLoading the local sentence model. If it is not installed, first use downloads about 91 MB. Your transcript stays on this machine.\n\nEsc cancels this search.");
    } else {
        if let Some(reason) = &state.recall_keyword_only_reason {
            body.push_str(&format!(
                "Keyword results only. Semantic search unavailable: {reason}\n\n"
            ));
        }
        match state.recall_excerpts.get(state.search_idx) {
            Some(excerpt) => {
                body.push_str("Observed passage:\n");
                body.push_str(excerpt);
            }
            None => body.push_str("No matching passages in the observed transcript."),
        }
    }
    body.split('\n')
        .flat_map(|line| {
            if line.is_empty() {
                vec![String::new()]
            } else {
                wrap_line(line, width)
            }
        })
        .collect()
}

/// Maximum excerpt scroll, shared by the draw and the input handler.
pub fn max_scroll(state: &AppState, area: Rect) -> usize {
    let content = content_area(area);
    if content.height == 0 {
        return 0;
    }
    lines(state, content.width)
        .len()
        .saturating_sub(content.height as usize)
}

pub fn draw_recall_panel(state: &AppState, area: Rect, buf: &mut Buffer) {
    if !state.recall_mode || area.width < 3 || area.height < 3 {
        return;
    }
    let background = state.colors.theme.get("dialog.background").style;
    let title_style = state.colors.theme.get("dialog.title").style;
    let mode = if state.recall_pending_id.is_some() {
        "searching"
    } else if state.recall_keyword_only_reason.is_some() {
        "keyword only"
    } else if state.recall_empty {
        "empty"
    } else {
        "hybrid"
    };
    let count = state.recall_excerpts.len();
    let number = if count == 0 { 0 } else { state.search_idx + 1 };
    let title = format!(" Recall ({mode}) {number}/{count} ");
    Clear.render(area, buf);
    Block::default()
        .borders(Borders::ALL)
        .style(background)
        .border_style(title_style)
        .title(title)
        .render(area, buf);
    let content = content_area(area);
    let rows = lines(state, content.width);
    let scroll = state
        .recall_preview_scroll
        .min(rows.len().saturating_sub(content.height as usize));
    for (row, line) in rows
        .iter()
        .skip(scroll)
        .take(content.height as usize)
        .enumerate()
    {
        draw_str_clipped(
            buf,
            content.x,
            content.y + row as u16,
            line,
            background,
            content,
        );
    }
    if area.height >= 4 {
        let footer = Rect::new(content.x, area.bottom().saturating_sub(2), content.width, 1);
        let keys = &state.config.search;
        let hint = format!(
            "{}:next {}:previous  Up/Down/PgUp/PgDn:scroll  Esc:close",
            keys.key_back, keys.key_forward
        );
        draw_str_clipped(buf, footer.x, footer.y, &hint, title_style, footer);
    }
}

#[cfg(all(test, feature = "t-render"))]
mod tests {
    use super::*;
    use crate::{config::V6RenderMode, engine::StatusModel, recall::RecallSnapshot};

    fn selected(excerpt: &str) -> AppState {
        let mut state = AppState::default();
        state.recall_mode = true;
        state.search_query = Some("remembered clue".into());
        state.recall_excerpts = vec![excerpt.into()];
        state.search_matches = vec![0];
        state
    }

    fn paint(state: &AppState, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        draw_recall_panel(state, area, &mut buf);
        buf
    }

    fn row_text(buf: &Buffer, x: u16, y: u16, width: u16) -> String {
        (x..x + width).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn body_text(buf: &Buffer, area: Rect) -> String {
        let body = content_area(area);
        (body.y..body.bottom())
            .map(|y| row_text(buf, body.x, y, body.width))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn recall_panel_shows_clue_beyond_1500_characters_in_one_source_line() {
        let clue = "The lantern opens the midnight gate.";
        let source = format!("{} {clue}", "The empty courtyard is quiet. ".repeat(70));
        assert!(source.find(clue).unwrap() > 1500);
        assert!(!source.contains('\n'));
        let mut state = AppState::default();
        state.push_transcript(&source);
        let passages = RecallSnapshot::from_state(&state).passages();
        let passage = passages
            .iter()
            .find(|p| p.text.contains(clue))
            .expect("tail is indexed");
        assert_eq!(
            passage.raw_line, 0,
            "the source jump alone cannot reach the suffix"
        );
        state.recall_mode = true;
        state.search_query = Some("midnight gate".into());
        state.search_matches = vec![0];
        state.recall_excerpts = vec![passage.text.clone()];
        let before = state.transcript.clone();
        let area = Rect::new(4, 3, 64, 7);
        state.recall_preview_scroll = max_scroll(&state, area);
        let rendered = body_text(&paint(&state, area), area)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            rendered.contains(clue),
            "selected suffix must be readable: {rendered:?}"
        );
        assert_eq!(
            state.transcript, before,
            "preview must never become story output"
        );
    }

    #[test]
    fn recall_panel_tiny_viewport_scroll_reaches_final_unicode_character() {
        for (width, height) in [(3, 3), (9, 3), (9, 4), (17, 5)] {
            let area = Rect::new(7, 2, width, height);
            let mut state = selected(&format!("{}FINALΩ", "observed passage ".repeat(50)));
            let max = max_scroll(&state, area);
            assert!(max > 0);
            state.recall_preview_scroll = max;
            let at_end = paint(&state, area);
            assert!(
                body_text(&at_end, area).contains('Ω'),
                "lost final character at {width}x{height}"
            );
            state.recall_preview_scroll = usize::MAX;
            assert_eq!(
                paint(&state, area),
                at_end,
                "overscroll must clamp to the same final page"
            );
        }
    }

    #[test]
    fn recall_panel_excerpts_do_not_depend_on_cell_or_raster_wrap_cache() {
        let mut state = selected("The brass lantern illuminates the cavern. FINALΩ");
        state.push_transcript(&"Unrelated long source text wraps differently. ".repeat(30));
        let area = Rect::new(0, 0, 60, 10);
        let expected = paint(&state, area);
        assert!(state.transcript_wrap.borrow().is_none());
        assert!(state.raster_wrap.borrow().is_none());

        state.config.v6_render = V6RenderMode::Raster;
        let _ = crate::render::screen::build_main_text(&state, 11, 5);
        assert!(state.raster_wrap.borrow().is_some());
        assert!(state.transcript_wrap.borrow().is_none());
        assert_eq!(
            paint(&state, area),
            expected,
            "raster-only cache must not select excerpt text"
        );

        let cell_area = Rect::new(0, 0, 37, 12);
        let mut cell_buf = Buffer::empty(cell_area);
        let _ = crate::render::transcript::render_transcript(
            &StatusModel::HostManaged,
            None,
            &state,
            cell_area,
            &mut cell_buf,
            None,
        );
        assert!(state.transcript_wrap.borrow().is_some());
        state.push_transcript("New story output makes both wrap caches stale.");
        assert_eq!(
            paint(&state, area),
            expected,
            "stale source wraps cannot hide the selected excerpt"
        );
        state.config.v6_render = V6RenderMode::Hybrid;
        assert_eq!(paint(&state, area), expected);
    }

    #[test]
    fn recall_panel_uses_actual_interior_width_and_preserves_border_cells() {
        let area = Rect::new(5, 4, 8, 4);
        let mut state = selected("ABCDEFG");
        state.recall_preview_scroll = max_scroll(&state, area);
        let buf = paint(&state, area);
        assert_eq!(
            body_text(&buf, area).trim(),
            "G",
            "seven characters require two six-cell rows"
        );
        let body = content_area(area);
        assert_eq!(buf[(area.x, body.y)].symbol(), "│");
        assert_eq!(buf[(area.right() - 1, body.y)].symbol(), "│");
        assert_eq!(buf[(area.x, area.bottom() - 1)].symbol(), "└");
    }

    #[test]
    fn recall_panel_clears_old_long_result_when_selection_changes() {
        let area = Rect::new(0, 0, 70, 12);
        let mut state = selected("OLDPASSAGE ".repeat(20).as_str());
        state.recall_excerpts.push("New short clue.".into());
        state.search_matches.push(1);
        let mut buf = paint(&state, area);
        assert!(body_text(&buf, area).contains("OLDPASSAGE"));
        state.search_idx = 1;
        draw_recall_panel(&state, area, &mut buf);
        let text = body_text(&buf, area);
        assert!(text.contains("New short clue."));
        assert!(!text.contains("OLDPASSAGE"));
    }

    #[test]
    fn recall_panel_pending_query_does_not_display_previous_excerpt() {
        let area = Rect::new(0, 0, 80, 15);
        let mut state = selected("STALEPASSAGE");
        state.recall_pending_id = Some(42);
        let buf = paint(&state, area);
        let text = body_text(&buf, area);
        assert!(text.contains("Preparing local search"));
        assert!(!text.contains("STALEPASSAGE"));
        assert!(row_text(&buf, 0, 0, area.width).contains("searching"));
    }

    #[test]
    fn recall_panel_degenerate_area_and_inactive_mode_leave_buffer_untouched() {
        let mut state = selected("a clue");
        for area in [
            Rect::new(0, 0, 0, 0),
            Rect::new(2, 1, 2, 8),
            Rect::new(2, 1, 8, 2),
        ] {
            let mut buf = Buffer::empty(area);
            let before = buf.clone();
            draw_recall_panel(&state, area, &mut buf);
            assert_eq!(buf, before);
            assert_eq!(max_scroll(&state, area), 0);
        }
        state.recall_mode = false;
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        let before = buf.clone();
        draw_recall_panel(&state, area, &mut buf);
        assert_eq!(buf, before);
    }
}
