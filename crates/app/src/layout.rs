//! Single source of truth for pane geometry: the vertical split that carves
//! out the inventory dock + help row, the horizontal split that carves out
//! the verb dock, and the per-`Layout` split of the remaining panes area
//! between the story and map panes.
//!
//! Extracted from the inline `.constraints(...)` splits that used to live in
//! `main.rs`'s `terminal.draw` closure so the geometry is testable without a
//! full terminal/render stack. Behavior-identical to that inline code.

use ratatui::layout::{Constraint, Direction, Layout as RatatuiLayout, Rect};

use crate::render::inventory_dock::{inventory_dock_height, inventory_dock_target_height};
use crate::render::verbmenu::{verb_dock_target_width, verb_dock_width};
use crate::state::{AppState, Layout};

/// The resolved pane rects for one frame. `story`/`map` are the OUTER
/// (pre-frame) rects passed to `draw_framed`; they are `Rect::default()`
/// (zero area) when that pane is hidden for the current `Layout`.
/// `verb_dock`/`inv_dock` are zero-area when their dock is closed.
pub struct PaneLayout {
    pub story: Rect,
    pub map: Rect,
    pub verb_dock: Rect,
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

    // ── Reserve bottom 1 row for help bar (and the inventory dock band above
    // it) ────────────────────────────────────────────────────────────────────
    let vert = RatatuiLayout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(inv_dock_h), Constraint::Length(1)])
        .split(area);
    let main_area = vert[0];
    let inv_dock_area = vert[1];
    let help_row = vert[2];

    // ── Verb dock: reserve a left band (full height of main_area) that
    // slides in when toggled, sized from a fixed target width + slide
    // fraction (mirrors the inventory dock's bottom-slide, but on the left).
    let verb_visible = state.verb_menu.is_some() || state.verb_dock.active();
    let verb_target_w =
        verb_dock_target_width(verb_visible, main_area.width, state.pane_sizes.verb_dock_pct);
    let verb_dock_w = verb_dock_width(verb_target_w, state.verb_dock.fraction());
    let horiz = RatatuiLayout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(verb_dock_w), Constraint::Min(0)])
        .split(main_area);
    let verb_dock_area = horiz[0];
    let panes_area = horiz[1];

    let (story, map) = match state.layout {
        Layout::TranscriptFull => (panes_area, Rect::default()),
        Layout::MapFull => (Rect::default(), panes_area),
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

    PaneLayout { story, map, verb_dock: verb_dock_area, inv_dock: inv_dock_area, help_row }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{VerbMenuPane, VerbMenuState};

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
        assert_eq!(pl.verb_dock.width * pl.verb_dock.height, 0);

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
    fn map_full_hides_story() {
        let mut state = AppState::default();
        state.layout = Layout::MapFull;
        let pl = compute_pane_layout(area80x24(), &state, 0);

        assert_eq!(pl.story.width * pl.story.height, 0);
        assert_eq!(pl.map, Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn help_row_always_bottom_single_row() {
        for layout in [Layout::Split, Layout::TranscriptFull, Layout::MapFull] {
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

    #[test]
    fn verb_dock_open_reserves_left_band() {
        let mut state = AppState::default();
        state.verb_menu = Some(VerbMenuState {
            pane: VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns: vec![],
            story_focused: false,
        });
        state.verb_dock.toggle_to(true, true); // instant open → fraction() == 1.0
        let pl = compute_pane_layout(area80x24(), &state, 0);

        // target width = 32% of 80 = 25 (well under full_width - 4).
        assert_eq!(pl.verb_dock.width, 25);
        assert_eq!(pl.verb_dock.x, 0);
        // Panes (story+map) start right after the dock.
        assert_eq!(pl.story.x, 25);
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
        for layout in [Layout::Split, Layout::TranscriptFull, Layout::MapFull] {
            let mut state = AppState::default();
            state.layout = layout;
            let pl = compute_pane_layout(area80x24(), &state, 0);
            assert_eq!(pl.panes_area(), Rect::new(0, 0, 80, 23), "{layout:?}");
        }
    }
}
