//! Shared vertical-scrollbar drawing. The single place the ratatui `Scrollbar`
//! idiom lives; every linearly-scrollable surface calls [`draw_scrollbar`].
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget},
};

/// True when `total` rows do not fit in `viewport` rows (so a scrollbar — and a
/// reserved 1-column gutter — are warranted).
pub fn needs_scrollbar(total: usize, viewport: usize) -> bool {
    total > viewport
}

/// Draw a themed vertical scrollbar on the right edge of `area`. No-op when the
/// content fits (`total <= viewport`) or `area` is degenerate. `position` is the
/// index of the first visible row (0-based), ranging `0..=total-viewport`.
///
/// ratatui places the thumb at the track bottom only when
/// `position == content_length - 1`; since `position` ranges `0..=max_scroll`
/// (`max_scroll = total - viewport`), the state's content length is
/// `max_scroll + 1` so the thumb spans the full track, while
/// `viewport_content_length` keeps the thumb proportional.
pub fn draw_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    total: usize,
    viewport: usize,
    position: usize,
    style: Style,
) {
    if !needs_scrollbar(total, viewport) || area.height == 0 || area.width == 0 {
        return;
    }
    let content_len = total.saturating_sub(viewport) + 1;
    let mut sb_state = ScrollbarState::new(content_len)
        .viewport_content_length(viewport)
        .position(position);
    StatefulWidget::render(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .style(style),
        area,
        buf,
        &mut sb_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};

    #[test]
    fn needs_scrollbar_only_when_overflowing() {
        assert!(!needs_scrollbar(5, 10)); // fits
        assert!(!needs_scrollbar(10, 10)); // exactly fits
        assert!(needs_scrollbar(11, 10)); // overflows
    }

    #[test]
    fn draw_scrollbar_noop_when_fits_and_draws_when_overflowing() {
        let area = Rect::new(0, 0, 8, 4);
        // fits -> nothing drawn on the right edge
        let mut b1 = Buffer::empty(area);
        draw_scrollbar(&mut b1, area, 4, 4, 0, Style::default());
        let right_col_blank = (0..area.height)
            .all(|y| b1.cell((area.right() - 1, y)).unwrap().symbol() == " ");
        assert!(right_col_blank, "no scrollbar when content fits");
        // overflows -> the right column has non-space scrollbar glyphs
        let mut b2 = Buffer::empty(area);
        draw_scrollbar(&mut b2, area, 40, 4, 0, Style::default());
        let any_glyph = (0..area.height)
            .any(|y| b2.cell((area.right() - 1, y)).unwrap().symbol() != " ");
        assert!(any_glyph, "scrollbar drawn when content overflows");
    }
}
