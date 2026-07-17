//! Pure geometry + navigation for the story-picker cover gallery (SQ-0374).
//!
//! The gallery is an alternate story-picker view: a grid of cover thumbnails
//! (each a fitted frontispiece, or the story title centred when a cover is
//! metadata list. Only the layout math and 2D cursor movement live here — the
//! rendering and input wiring stay in `picker_ui`, so this module is a set of
//! small, deterministic, unit-testable functions with no terminal/state deps.

use ratatui::layout::Rect;

/// Target cover width, in cells, for one tile. Columns auto-fit the pane.
pub const TILE_W: u16 = 20;
/// Cover-image band height, in rows, per tile (the frontispiece is fitted into
/// this; a missing cover shows the story title centred here instead).
pub const TILE_COVER_H: u16 = 11;
/// Full tile height. The tile is the cover band alone — there is no separate
/// title caption (titles appear only in place of a missing cover).
pub const TILE_H: u16 = TILE_COVER_H;
/// Blank columns between adjacent tiles.
pub const GUTTER_X: u16 = 1;
/// Blank rows between adjacent tile rows.
pub const GUTTER_Y: u16 = 1;

/// How many tile columns fit in `width` cells. Always at least 1 (a pane too
/// narrow for even one full tile still shows a single, clipped column).
pub fn columns(width: u16) -> usize {
    ((width as usize + GUTTER_X as usize) / (TILE_W + GUTTER_X) as usize).max(1)
}

/// How many whole tile rows are visible in `height` rows of grid region.
/// Always at least 1.
pub fn visible_rows(height: u16) -> usize {
    ((height as usize + GUTTER_Y as usize) / (TILE_H + GUTTER_Y) as usize).max(1)
}

/// The first grid row to show so that `selected`'s row stays on screen, given
/// the current scroll `first_row`, column count, and visible-row count. Scrolls
/// the minimum needed: up when the selection is above the window, down when
/// it's below, otherwise leaves `first_row` unchanged (no jitter).
pub fn scroll_to(selected: usize, cols: usize, vis_rows: usize, first_row: usize) -> usize {
    let cols = cols.max(1);
    let vis_rows = vis_rows.max(1);
    let sel_row = selected / cols;
    if sel_row < first_row {
        sel_row
    } else if sel_row >= first_row + vis_rows {
        sel_row + 1 - vis_rows
    } else {
        first_row
    }
}

/// Move the selection within the grid by `dx` columns and `dy` rows (reading
/// order: a vertical step is `cols` indices). Clamped to `[0, total)`. A
/// horizontal step past a row edge lands on the adjacent row's end, matching
/// linear reading order.
pub fn move_index(selected: usize, cols: usize, total: usize, dx: isize, dy: isize) -> usize {
    if total == 0 {
        return 0;
    }
    let delta = dx + dy * cols.max(1) as isize;
    (selected as isize + delta).clamp(0, total as isize - 1) as usize
}

/// Screen rect of the tile at grid column `col` and *visible* row `vis_row`
/// (0-based within the scrolled window), placed inside `grid` (the region below
/// the header and above the footer). The rect is the full tile box (cover band
/// + caption); the caller splits it into image and caption sub-rects.
pub fn tile_rect(grid: Rect, col: usize, vis_row: usize) -> Rect {
    let x = grid.x + col as u16 * (TILE_W + GUTTER_X);
    let y = grid.y + vis_row as u16 * (TILE_H + GUTTER_Y);
    Rect::new(x, y, TILE_W, TILE_H)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_auto_fit_and_floor_at_one() {
        // 20-wide tiles + 1 gutter → 21 per column.
        assert_eq!(columns(80), 3); // (80+1)/21 = 3
        assert_eq!(columns(21), 1);
        assert_eq!(columns(42), 2);
        assert_eq!(columns(120), 5); // (120+1)/21 = 5
        // Too narrow for a whole tile still yields one (clipped) column.
        assert_eq!(columns(10), 1);
        assert_eq!(columns(0), 1);
    }

    #[test]
    fn visible_rows_auto_fit_and_floor_at_one() {
        // 11-tall tiles + 1 gutter → 12 per row.
        assert_eq!(visible_rows(40), 3); // (40+1)/12 = 3
        assert_eq!(visible_rows(12), 1);
        assert_eq!(visible_rows(24), 2);
        assert_eq!(visible_rows(5), 1);
        assert_eq!(visible_rows(0), 1);
    }

    #[test]
    fn scroll_keeps_selection_visible() {
        let (cols, vis) = (4usize, 3usize); // shows rows first..first+3
        // Selection already on screen: no change.
        assert_eq!(scroll_to(4, cols, vis, 0), 0); // row 1, window rows 0..3
        // Selection below window: scroll down just enough.
        assert_eq!(scroll_to(12, cols, vis, 0), 1); // row 3 → window 1..4
        assert_eq!(scroll_to(20, cols, vis, 0), 3); // row 5 → window 3..6
        // Selection above window: scroll up to it.
        assert_eq!(scroll_to(0, cols, vis, 3), 0);
        assert_eq!(scroll_to(4, cols, vis, 3), 1); // row 1 → first_row = 1
    }

    #[test]
    fn move_index_2d_reading_order_and_clamp() {
        let (cols, total) = (4usize, 10usize); // rows: 0-3,4-7,8-9
        // Right / left by one index.
        assert_eq!(move_index(0, cols, total, 1, 0), 1);
        assert_eq!(move_index(5, cols, total, -1, 0), 4);
        // Down / up by a full row (cols indices).
        assert_eq!(move_index(1, cols, total, 0, 1), 5);
        assert_eq!(move_index(5, cols, total, 0, -1), 1);
        // Clamp at the ends.
        assert_eq!(move_index(0, cols, total, -1, 0), 0);
        assert_eq!(move_index(9, cols, total, 1, 0), 9);
        // Down from the last full row clamps onto the final (partial-row) item.
        assert_eq!(move_index(7, cols, total, 0, 1), 9); // 7+4=11 → 9
        // Horizontal step past a row edge lands on the neighbouring row's end.
        assert_eq!(move_index(4, cols, total, -1, 0), 3); // row 1 start → row 0 end
    }

    #[test]
    fn move_index_empty_list_is_zero() {
        assert_eq!(move_index(0, 4, 0, 1, 0), 0);
    }

    #[test]
    fn tile_rect_places_by_grid_step() {
        let grid = Rect::new(2, 3, 80, 40);
        // Top-left tile sits at the grid origin.
        assert_eq!(tile_rect(grid, 0, 0), Rect::new(2, 3, TILE_W, TILE_H));
        // Column step is TILE_W + GUTTER_X; row step is TILE_H + GUTTER_Y.
        assert_eq!(tile_rect(grid, 1, 0), Rect::new(2 + 21, 3, TILE_W, TILE_H));
        assert_eq!(tile_rect(grid, 0, 1), Rect::new(2, 3 + 12, TILE_W, TILE_H));
        assert_eq!(tile_rect(grid, 2, 1), Rect::new(2 + 42, 3 + 12, TILE_W, TILE_H));
    }
}
