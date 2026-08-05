//! Single source of truth for pane geometry: the vertical split that carves
//! out the command band, the inventory dock and the help row, and the
//! per-`Layout` split of the remaining panes area between the story and map
//! panes.
//!
//! Extracted from the inline `.constraints(...)` splits that used to live in
//! `main.rs`'s `terminal.draw` closure so the geometry is testable without a
//! full terminal/render stack. Behavior-identical to that inline code.

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

use crate::render::command_band::{band_height, band_target_height};
use crate::render::inventory_dock::{inventory_dock_height, inventory_dock_target_height};
use crate::state::{AppState, Layout};

/// The resolved pane rects for one frame. `story`/`map` are the OUTER
/// (pre-frame) rects passed to `draw_framed`; they are `Rect::default()`
/// (zero area) when that pane is hidden for the current `Layout`.
/// `command_band`/`inv_dock` are zero-area when closed.
pub struct PaneLayout {
    pub story: Rect,
    pub map: Rect,
    pub command_band: Rect,
    pub inv_dock: Rect,
    pub help_row: Rect,
}

impl PaneLayout {
    /// The combined story+map region before the per-layout split — reconstructs
    /// what was previously called `panes_area` in `main.rs`. Used as a last-resort
    /// overlay target when both panes report zero content height (e.g. a terminal
    /// so small the pane's border consumes all its rows).
    pub fn panes_area(&self) -> Rect {
        let story_empty = self.story.width == 0 && self.story.height == 0;
        let map_empty = self.map.width == 0 && self.map.height == 0;
        match (story_empty, map_empty) {
            (true, true) => Rect::default(),
            (true, false) => self.map,
            (false, true) => self.story,
            (false, false) => self.story.union(self.map),
        }
    }
}

/// Compute this frame's pane geometry. `inv_item_count` is passed in (rather
/// than computed here) so this stays free of `engine.introspect()`/rendering
/// dependencies.
pub fn compute_pane_layout(area: Rect, state: &AppState, inv_item_count: usize) -> PaneLayout {
    // ── Inventory dock: reserve a bottom band (above the help row) that
    // slides up when toggled, sized from the item list + slide fraction.
    let inv_visible = state.show_inventory || state.inv_dock.active();
    let inv_target_h = if inv_visible {
        inventory_dock_target_height(inv_item_count, area.height, state.pane_sizes.inv_dock_pct)
    } else {
        0
    };
    let inv_dock_h = inventory_dock_height(inv_target_h, state.inv_dock.fraction());

    // ── Command band: a bottom band under the story pane, above the help row
    // and above the inventory dock, sliding up when opened (SQ-0664). While
    // it is open it SUBSUMES the inventory dock — the "carried" column IS the
    // inventory — so the dock is not reserved at all and returns on close.
    let band_visible = state.command_band_visible();
    let band_target_h = band_target_height(band_visible, area.height, state.pane_sizes.band_height);
    let band_h = band_height(band_target_h, state.band_dock.fraction());
    let inv_dock_h = if band_visible { 0 } else { inv_dock_h };

    // ── Reserve bottom 1 row for help bar, the command band and the inventory
    // dock band above it ─────────────────────────────────────────────────────
    let vert = RatatuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(band_h),
            Constraint::Length(inv_dock_h),
            Constraint::Length(1),
        ])
        .split(area);
    let panes_area = vert[0];
    let band_area = vert[1];
    let inv_dock_area = vert[2];
    let help_row = vert[3];

    // The debug inspector tiles into the map slot; make sure a right-slot rect
    // exists for it even when the current layout is TranscriptFull (map hidden).
    let effective_layout = if state.debug.is_some() { Layout::Split } else { state.layout };
    let (story, map) = match effective_layout {
        Layout::TranscriptFull => (panes_area, Rect::default()),
        Layout::Split => {
            let chunks = RatatuiLayout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(state.pane_sizes.split_ratio),
                    Constraint::Percentage(100u16.saturating_sub(state.pane_sizes.split_ratio)),
                ])
                .split(panes_area);
            (chunks[0], chunks[1])
        }
    };

    PaneLayout { story, map, command_band: band_area, inv_dock: inv_dock_area, help_row }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::command_band::{default_quick, default_verbs};
    use crate::state::CommandBandState;

    fn open_band(state: &mut AppState) {
        state.overlays.command_band =
            Some(CommandBandState::new(default_verbs(), default_quick()));
        state.band_dock.toggle_to(true, true); // instant open → fraction() == 1.0
    }

    fn area80x24() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    #[test]
    fn split_layout_halves_panes() {
        let state = AppState::default();
        assert_eq!(state.layout, Layout::Split);
        let pl = compute_pane_layout(area80x24(), &state, 0);

        // Docks closed → zero area.
        assert_eq!(pl.inv_dock.width * pl.inv_dock.height, 0);
        assert_eq!(pl.command_band.width * pl.command_band.height, 0);

        // Help row is the bottom single row.
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1));

        // Story + map fill the remaining 23 rows and split the 80 columns ~evenly.
        assert_eq!(pl.story.height, 23);
        assert_eq!(pl.map.height, 23);
        assert_eq!(pl.story.y, 0);
        assert_eq!(pl.map.y, 0);
        assert_eq!(pl.story.width + pl.map.width, 80);
        assert!((pl.story.width as i32 - pl.map.width as i32).abs() <= 1);
    }

    #[test]
    fn split_matches_manual_split_of_panes_area() {
        // Parity check: reproduce the exact old inline computation (panes_area =
        // area minus the 1-row help row, docks closed) and assert the pure
        // function agrees exactly.
        let area = area80x24();
        let state = AppState::default();
        let pl = compute_pane_layout(area, &state, 0);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.map, chunks[1]);
    }

    #[test]
    fn transcript_full_hides_map() {
        let mut state = AppState::default();
        state.layout = Layout::TranscriptFull;
        let pl = compute_pane_layout(area80x24(), &state, 0);

        assert_eq!(pl.map.width * pl.map.height, 0);
        assert_eq!(pl.story, Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn help_row_always_bottom_single_row() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state, 0);
            assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1), "{layout:?}");
        }
    }

    #[test]
    fn inv_dock_open_reserves_bottom_band() {
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true); // instant open → fraction() == 1.0
        let pl = compute_pane_layout(area80x24(), &state, 3);

        // target height = item_count(3) + 2 borders = 5, capped at height/3 = 8 → 5.
        assert_eq!(pl.inv_dock.height, 5);
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1));
        // Story/map shrink to make room for the dock band above the help row.
        assert_eq!(pl.story.height + pl.inv_dock.height + pl.help_row.height, 24);
    }

    /// The band is a BOTTOM band now (SQ-0664): full width, above the help row,
    /// with the story/map panes shrinking to make room.
    #[test]
    fn command_band_open_reserves_a_bottom_band() {
        let mut state = AppState::default();
        open_band(&mut state);
        let pl = compute_pane_layout(area80x24(), &state, 0);

        assert_eq!(pl.command_band.width, 80, "full width");
        assert_eq!(pl.command_band.x, 0);
        assert_eq!(
            pl.command_band.height,
            crate::render::command_band::DEFAULT_BAND_ROWS,
            "the default-height band"
        );
        assert_eq!(pl.help_row, Rect::new(0, 23, 80, 1), "help row stays the bottom row");
        assert_eq!(pl.command_band.y + pl.command_band.height, pl.help_row.y);
        assert_eq!(pl.story.height + pl.command_band.height + pl.help_row.height, 24);
        // Story and map keep the FULL width — no left carve any more.
        assert_eq!(pl.story.x, 0);
        assert_eq!(pl.story.width + pl.map.width, 80);
    }

    /// Decision 1: while the band is open it subsumes the inventory dock (the
    /// carried column IS the inventory), which returns when the band closes.
    #[test]
    fn open_band_subsumes_the_inventory_dock() {
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inv_dock.toggle_to(true, true);

        let before = compute_pane_layout(area80x24(), &state, 3);
        assert!(before.inv_dock.height > 0, "the dock is up to begin with");

        open_band(&mut state);
        let during = compute_pane_layout(area80x24(), &state, 3);
        assert_eq!(during.inv_dock.height, 0, "the band subsumes the inventory dock");
        assert!(during.command_band.height > 0);

        // Closing the band brings it back.
        state.overlays.command_band = None;
        state.band_dock.toggle_to(false, true);
        let after = compute_pane_layout(area80x24(), &state, 3);
        assert_eq!(after.inv_dock.height, before.inv_dock.height, "the dock returns on close");
    }

    /// The configured height drives the band, clamped so it can never starve
    /// the story pane.
    #[test]
    fn band_height_follows_config_and_clamps() {
        let mut state = AppState::default();
        open_band(&mut state);
        state.pane_sizes.band_height = 10;
        assert_eq!(compute_pane_layout(area80x24(), &state, 0).command_band.height, 10);

        state.pane_sizes.band_height = 99;
        let pl = compute_pane_layout(area80x24(), &state, 0);
        assert_eq!(
            pl.command_band.height,
            crate::render::command_band::MAX_BAND_ROWS,
            "clamped to MAX_BAND_ROWS"
        );
        assert!(pl.story.height > 0, "the story pane always survives");

        // A tiny terminal wins over the configured height.
        state.pane_sizes.band_height = 14;
        let tiny = compute_pane_layout(Rect::new(0, 0, 80, 10), &state, 0);
        assert!(tiny.command_band.height <= 6, "band shrinks on a short screen");
        assert!(tiny.story.height > 0);
    }

    #[test]
    fn split_ratio_configurable_matches_manual_percentage_split() {
        // A non-default split_ratio (70/30) must match a manual
        // Percentage(70)/Percentage(30) split of the same panes_area exactly.
        let area = area80x24();
        let mut state = AppState::default();
        state.pane_sizes.split_ratio = 70;
        let pl = compute_pane_layout(area, &state, 0);

        let panes_area = Rect::new(0, 0, 80, 23);
        let chunks = RatatuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(panes_area);

        assert_eq!(pl.story, chunks[0]);
        assert_eq!(pl.map, chunks[1]);
    }

    #[test]
    fn panes_area_reconstructs_union_across_layouts() {
        for layout in [Layout::Split, Layout::TranscriptFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state, 0);
            assert_eq!(pl.panes_area(), Rect::new(0, 0, 80, 23), "{layout:?}");
        }
    }
}
