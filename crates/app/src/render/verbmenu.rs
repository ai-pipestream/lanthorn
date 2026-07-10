//! Verb/item token-palette dock: a bordered panel docked at the far left of
//! the screen (full height of the main content area), sliding in via
//! `state.verb_dock` (mirrors the inventory dock's bottom-slide, but on the
//! left). Picking a token appends it (+ space) to the input line; the player
//! submits the composed command normally.
//!
//! Unlike the old centered three-column modal, the dock is narrow, so the
//! Verbs / Nouns / Preps sections are STACKED vertically instead of side by
//! side. The caller (`main.rs`) sizes `area` from the animated `PanelSlide`
//! fraction, so `area` may be narrower than the panel's target width while a
//! slide is in flight — everything here clips to `area`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::paneframe::{draw_pane_frame, BorderStyle, PaneGlyphs};
use crate::state::{AppState, VerbMenuPane};

// ── Curated lists ─────────────────────────────────────────────────────────────

/// Curated built-in verb list for the token palette.
pub const VERB_MENU_VERBS: &[&str] = &[
    "look",
    "examine",
    "take",
    "drop",
    "open",
    "close",
    "unlock",
    "lock",
    "push",
    "pull",
    "turn",
    "put",
    "give",
    "show",
    "read",
    "eat",
    "drink",
    "wear",
    "wield",
    "enter",
    "exit",
    "search",
    "move",
    "go",
    "north",
    "south",
    "east",
    "west",
    "up",
    "down",
    "in",
    "out",
    "inventory",
    "wait",
    "again",
];

/// Curated built-in preposition list for the token palette.
pub const VERB_MENU_PREPS: &[&str] = &[
    "with", "on", "in", "to", "under", "at", "from", "of",
];

// ── Hit rects ─────────────────────────────────────────────────────────────────

/// Click targets emitted by `draw_verb_menu` for the event loop to hit-test:
/// each token row (pane + index + its rect) and each section header (pane + rect).
#[derive(Default, Clone)]
pub struct VerbMenuHits {
    pub rows: Vec<(VerbMenuPane, usize, Rect)>,
    pub headers: Vec<(VerbMenuPane, Rect)>,
}

// ── Layout ────────────────────────────────────────────────────────────────────

/// Compute the dock's fully-open target width in columns: `pct`% of
/// `full_width` (default 32, ≈ the old fixed 26-of-80 columns), clamped to
/// `[12, full_width - 4]` (leaving room for the story/map panes). 0 when the
/// dock isn't visible at all, or when the screen is too narrow to leave room
/// for the panes.
pub fn verb_dock_target_width(visible: bool, full_width: u16, pct: u16) -> u16 {
    if !visible {
        return 0;
    }
    let hi = full_width.saturating_sub(4);
    if hi < 12 {
        return hi;
    }
    let target = ((full_width as u32 * pct as u32) / 100) as u16;
    target.clamp(12, hi)
}

/// Compute the reserved dock band width in columns: `target_w` scaled by the
/// slide's current `fraction` (0.0 closed .. 1.0 fully open), rounded to the
/// nearest column. Extracted from the layout split so the arithmetic is
/// testable without a full terminal/main-loop harness (mirrors
/// `inventory_dock_height`).
pub fn verb_dock_width(target_w: u16, fraction: f64) -> u16 {
    (target_w as f64 * fraction).round() as u16
}

// ── Drawing ───────────────────────────────────────────────────────────────────

/// Draw the verb/item token-palette dock into `area` (the left-docked band
/// carved out by `main.rs` from `state.verb_dock`'s slide fraction).
///
/// Stacks three sections vertically — Verbs, Nouns, Preps — each a 1-row
/// header plus a scrollable list, with the bottom inner row reserved for the
/// composed-input line. Sets `*vp_out` to the ACTIVE pane's visible list
/// height so PageUp/PageDown paging keeps working.
///
/// No-op when `state.verb_menu` is `None` or `area` is too small to show
/// anything meaningful (mid-slide).
pub fn draw_verb_menu(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
    hits: &mut VerbMenuHits,
) {
    hits.rows.clear();
    hits.headers.clear();
    let Some(vm) = &state.verb_menu else { return };

    if area.width < 8 || area.height < 4 {
        return;
    }

    let base = state.colors.dialog;

    // Fill the band's background first so panes behind it never show through
    // while it's mid-slide (shorter than its final bordered content needs).
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    // The verb dock is not an interactive resize target (the verb menu is a
    // keyboard-modal, so resize mode and the menu can never be open at once;
    // see SQ-0238), so its border never picks up the resize highlight.
    let frame = draw_pane_frame(buf, area, BorderStyle::Single, &PaneGlyphs::default(), base);

    // Title, centered on the top border row.
    if area.height >= 2 && area.width >= 2 {
        let title = " Verbs ";
        let avail = area.width as usize;
        let tw = title.chars().count();
        let leading = avail.saturating_sub(tw) / 2;
        let start_x = area.x + leading as u16;
        crate::render::draw_str_clipped(buf, start_x, area.y, title, base, area);
    }

    let content = frame.content;
    if content.height < 4 || content.width < 4 {
        return;
    }

    // Reserve the bottom inner row for the composed-input line, then divide
    // the remainder evenly into three vertically-stacked sections (1 header
    // row + list rows each); any remainder rows go to the earlier sections.
    let usable_h = content.height.saturating_sub(1);
    let base_h = usable_h / 3;
    let rem = usable_h % 3;
    let sect_h = [
        base_h + if rem > 0 { 1 } else { 0 },
        base_h + if rem > 1 { 1 } else { 0 },
        base_h,
    ];

    let noun_strs: Vec<&str> = vm.nouns.iter().map(|s| s.as_str()).collect();
    let sections: [(&str, VerbMenuPane); 3] = [
        ("Verbs", VerbMenuPane::Verbs),
        ("Nouns", VerbMenuPane::Nouns),
        ("Preps", VerbMenuPane::Preps),
    ];

    let mut y = content.y;
    for (i, (label, pane)) in sections.iter().enumerate() {
        let h = sect_h[i];
        if h == 0 {
            continue;
        }
        // When the story is focused, NO dock pane shows the active highlight.
        let active = !vm.story_focused && vm.pane == *pane;
        let header_area = Rect { x: content.x, y, width: content.width, height: 1 };
        draw_pane_header(label, active, header_area, buf, state);
        hits.headers.push((*pane, header_area));

        let list_h = h.saturating_sub(1);
        if list_h > 0 {
            let list_area = Rect { x: content.x, y: y + 1, width: content.width, height: list_h };
            let items: &[&str] = match pane {
                VerbMenuPane::Verbs => VERB_MENU_VERBS,
                VerbMenuPane::Nouns => noun_strs.as_slice(),
                VerbMenuPane::Preps => VERB_MENU_PREPS,
            };
            let scroll = match pane {
                VerbMenuPane::Verbs => &vm.verb_scroll,
                VerbMenuPane::Nouns => &vm.noun_scroll,
                VerbMenuPane::Preps => &vm.prep_scroll,
            };
            draw_list(items, scroll, active, list_area, buf, state, *pane, &mut hits.rows);
            if active {
                *vp_out = list_h as usize;
            }
        }
        y += h;
    }

    // Show current input composition in the bottom row of content.
    if !state.input.is_empty() {
        let input_y = content.bottom() - 1;
        let input_text = format!("Input: {}_", state.input);
        let input_style = Style::new().fg(Color::Yellow).patch(base);
        crate::render::draw_str_clipped(buf, content.x, input_y, &input_text, input_style, content);
    }
}

/// Draw the section header row using dialog colors.
fn draw_pane_header(title: &str, active: bool, area: Rect, buf: &mut Buffer, state: &AppState) {
    if area.height == 0 {
        return;
    }
    let base = state.colors.dialog;
    let style = if active {
        Style::new().add_modifier(Modifier::BOLD).patch(base)
    } else {
        base
    };
    let marker = if active { "> " } else { "  " };
    let line = format!("{}{}", marker, title);
    // Fill the header row.
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
    crate::render::draw_str_clipped(buf, area.x, area.y, &line, style, area);
}

/// Draw a scrolled list of items (windowed via `scroll`) with the selected item
/// highlighted, plus a scrollbar when the items overflow the area.
fn draw_list(
    items: &[&str],
    scroll: &crate::list_scroll::ListScroll,
    active: bool,
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    pane: VerbMenuPane,
    hits: &mut Vec<(VerbMenuPane, usize, Rect)>,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let base = state.colors.dialog;
    let visible = area.height as usize;
    let total = items.len();
    let selected = scroll.selected;

    // Reserve a 1-col gutter for the scrollbar when the list overflows.
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, visible) && area.width >= 2;
    let row_w = if scrollbar_visible { area.width.saturating_sub(1) } else { area.width };
    let offset = scroll.display_offset();

    for row in 0..visible {
        let idx = offset + row;
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }

        if idx >= total {
            // Empty rows: fill with dialog background.
            for x in area.x..area.x + row_w {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(base);
                }
            }
            continue;
        }

        let item = items[idx];
        let is_selected = idx == selected;

        let style = if is_selected && active {
            Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::new().add_modifier(Modifier::BOLD).patch(base)
        } else {
            base
        };

        let marker = if is_selected && active { "> " } else { "  " };
        let line = format!("{}{}", marker, item);

        // Fill the row with the style, then write text.
        let row_area = Rect::new(area.x, y, row_w, 1);
        hits.push((pane, idx, row_area));
        for x in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        crate::render::draw_str_clipped(buf, row_area.x, y, &line, style, row_area);
    }

    if scrollbar_visible {
        let sb_area = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total,
            visible,
            scroll.target_offset(),
            state.colors.scrollbar,
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, VerbMenuState};

    fn make_state_with_verb_menu() -> AppState {
        let mut s = AppState::default();
        s.verb_menu = Some(VerbMenuState {
            pane: VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns: vec!["mailbox".to_string(), "door".to_string()],
            // Tests here exercise the pane-focused render path; production opens
            // the dock story-focused (see OpenVerbMenu). Individual tests flip this.
            story_focused: false,
        });
        s
    }

    /// The dock is narrow (~24-26 cols) and full-height, unlike the old
    /// centered 80x24 modal.
    const DOCK_AREA: Rect = Rect { x: 0, y: 0, width: 26, height: 30 };

    #[test]
    fn verb_menu_renders_known_verb() {
        let mut buf = Buffer::empty(DOCK_AREA);
        let state = make_state_with_verb_menu();
        draw_verb_menu(&state, DOCK_AREA, &mut buf, &mut 0, &mut VerbMenuHits::default());
        let content: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        assert!(content.contains("look"), "should show 'look' verb");
        assert!(content.contains("Verbs"), "should show Verbs section header");
    }

    #[test]
    fn verb_menu_renders_room_noun() {
        let mut buf = Buffer::empty(DOCK_AREA);
        let state = make_state_with_verb_menu();
        draw_verb_menu(&state, DOCK_AREA, &mut buf, &mut 0, &mut VerbMenuHits::default());
        let content: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        assert!(content.contains("mailbox"), "should show room noun 'mailbox'");
        assert!(content.contains("Nouns"), "should show Nouns section header");
    }

    #[test]
    fn verb_menu_stacked_sections_all_present() {
        // Narrow left-dock geometry: the three sections stack vertically
        // rather than side by side. All three headers must be present, plus
        // the pane border/title.
        let mut buf = Buffer::empty(DOCK_AREA);
        let state = make_state_with_verb_menu();
        draw_verb_menu(&state, DOCK_AREA, &mut buf, &mut 0, &mut VerbMenuHits::default());
        let content: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();

        assert!(content.contains("Verbs"), "Verbs header should be present");
        assert!(content.contains("Nouns"), "Nouns header should be present");
        assert!(content.contains("Preps"), "Preps header should be present");
        assert!(content.contains('\u{250C}'), "top-left border corner should be present");
        assert!(content.contains("with"), "should show a preposition");

        // The active pane (Verbs, selected index 0 => "look") should have its
        // row highlighted cyan-on-black. Content starts at (1,1) (border inset
        // by 1); the Verbs header occupies content row 0, so its first list
        // row (the selected "look") is at content row 1 => buffer (1, 2).
        let cell = buf.cell((1, 2)).unwrap().style();
        assert_eq!(cell.fg, Some(Color::Black), "selected row fg should be black");
        assert_eq!(cell.bg, Some(Color::Cyan), "selected row bg should be cyan");
    }

    #[test]
    fn verb_menu_emits_row_and_header_hits() {
        let mut buf = Buffer::empty(DOCK_AREA);
        let state = make_state_with_verb_menu();
        let mut hits = VerbMenuHits::default();
        draw_verb_menu(&state, DOCK_AREA, &mut buf, &mut 0, &mut hits);

        assert!(!hits.rows.is_empty(), "row hit-rects should be populated");
        assert!(
            hits.rows.iter().any(|(p, i, _)| *p == VerbMenuPane::Verbs && *i == 0),
            "the 'look' row (Verbs, 0) should have a hit-rect"
        );
        assert_eq!(hits.headers.len(), 3, "all three section headers should have hit-rects");
    }

    #[test]
    fn verb_menu_story_focused_has_no_cyan_highlight() {
        // When the story input is focused, no dock pane shows the active cyan
        // highlight (the selected Verbs row is NOT cyan-on-black).
        let mut buf = Buffer::empty(DOCK_AREA);
        let mut state = make_state_with_verb_menu();
        state.verb_menu.as_mut().unwrap().story_focused = true;
        draw_verb_menu(&state, DOCK_AREA, &mut buf, &mut 0, &mut VerbMenuHits::default());

        let has_cyan = buf.content().iter().any(|c| c.style().bg == Some(Color::Cyan));
        assert!(!has_cyan, "no row should be cyan-highlighted while the story is focused");
    }

    #[test]
    fn verb_menu_scrollbar_and_paging_on_overflow() {
        use crate::input::{apply_action, Action, VerbMenuNavKind};
        use mapper::mapper::Mapper;
        // The verbs pane (35 built-ins) overflows the narrow dock's per-section
        // list rows.
        let area = Rect { x: 0, y: 0, width: 26, height: 20 };
        let mut buf = Buffer::empty(area);
        let mut state = make_state_with_verb_menu();
        let mut vp = 0usize;
        draw_verb_menu(&state, area, &mut buf, &mut vp, &mut VerbMenuHits::default());
        assert!(vp > 0 && vp < VERB_MENU_VERBS.len(), "verbs should overflow (vp={vp})");
        let has_thumb = buf.content().iter().any(|c| c.symbol() == "█");
        assert!(has_thumb, "a scrollbar thumb should be drawn when the verbs pane overflows");

        // PageDown advances the active (Verbs) pane's selection by ~one viewport.
        state.modal_list_viewport = vp;
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::PageDown), &mut state, &mut Mapper::default());
        let sel = state.verb_menu.as_ref().unwrap().verb_scroll.selected;
        assert!(sel >= vp.saturating_sub(1), "PageDown should advance ~one viewport, got {sel}");
    }

    #[test]
    fn verb_menu_opaque_background_no_bleed() {
        // Put some text in the buffer BEFORE drawing the dock, then verify
        // those cells are covered by the dock background (opaque fill).
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_verb_menu();
        terminal.draw(|f| {
            let bleed_style = ratatui::style::Style::new()
                .fg(Color::Red)
                .bg(Color::Green);
            for x in 0..26u16 {
                for y in 0..24u16 {
                    if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                        cell.set_symbol("X").set_style(bleed_style);
                    }
                }
            }
            let dock_area = Rect { x: 0, y: 0, width: 26, height: 24 };
            draw_verb_menu(&state, dock_area, f.buffer_mut(), &mut 0, &mut VerbMenuHits::default());
        }).unwrap();
        let cell = terminal.backend().buffer().cell((0, 0)).unwrap();
        assert_ne!(cell.bg, Color::Green, "dock should overwrite background (no bleed)");
    }

    #[test]
    fn verb_menu_preps_list_is_non_empty() {
        assert!(!VERB_MENU_PREPS.is_empty(), "prepositions list must not be empty");
        assert!(VERB_MENU_PREPS.contains(&"with"), "must include 'with'");
        assert!(VERB_MENU_PREPS.contains(&"on"), "must include 'on'");
    }

    #[test]
    fn verb_menu_verbs_list_is_non_empty() {
        assert!(!VERB_MENU_VERBS.is_empty(), "verbs list must not be empty");
        assert!(VERB_MENU_VERBS.contains(&"look"), "must include 'look'");
        assert!(VERB_MENU_VERBS.contains(&"unlock"), "must include 'unlock'");
    }

    #[test]
    fn verb_menu_too_narrow_does_not_panic() {
        let area = Rect { x: 0, y: 0, width: 4, height: 30 };
        let mut buf = Buffer::empty(area);
        let state = make_state_with_verb_menu();
        draw_verb_menu(&state, area, &mut buf, &mut 0, &mut VerbMenuHits::default());
        // No assertion beyond "did not panic"; too narrow to draw anything.
    }

    #[test]
    fn verb_dock_width_scales_with_fraction() {
        assert_eq!(verb_dock_width(26, 0.0), 0);
        assert_eq!(verb_dock_width(26, 1.0), 26);
        assert_eq!(verb_dock_width(26, 0.5), 13);
    }

    #[test]
    fn verb_dock_target_width_is_pct_based_and_capped() {
        // 32% of 80 = 25.6 -> 25 (integer division), the new default-equivalent.
        assert_eq!(verb_dock_target_width(true, 80, 32), 25);
        // 30% of 100 = 30, well under full_width - 4.
        assert_eq!(verb_dock_target_width(true, 100, 30), 30);
        // Narrow screen: pct target (20*32/100=6) is below the 12-col floor,
        // so it clamps up to 12 (still under full_width - 4 = 16).
        assert_eq!(verb_dock_target_width(true, 20, 32), 12);
        // Very narrow: full_width - 4 < 12 lower bound -> just full_width - 4 (no panic).
        assert_eq!(verb_dock_target_width(true, 10, 32), 6);
        // Not visible: 0 regardless of width.
        assert_eq!(verb_dock_target_width(false, 100, 32), 0);
    }

    #[test]
    fn dock_band_closed_is_zero_open_reserves_target_width() {
        // Mirrors the layout split in main.rs: closed (verb_menu is None,
        // verb_dock inactive) reserves 0 cols; fully open reserves the target
        // width (fraction 1.0).
        let full_width = 100u16;
        assert_eq!(verb_dock_width(verb_dock_target_width(false, full_width, 32), 0.0), 0);

        let open_target = verb_dock_target_width(true, full_width, 32);
        assert_eq!(open_target, 32);
        assert_eq!(verb_dock_width(open_target, 1.0), 32);
    }

    #[test]
    fn verb_menu_none_is_noop() {
        let area = Rect { x: 0, y: 0, width: 26, height: 30 };
        let mut buf = Buffer::empty(area);
        let state = AppState::default();
        assert!(state.verb_menu.is_none());
        draw_verb_menu(&state, area, &mut buf, &mut 0, &mut VerbMenuHits::default());
        // No assertion beyond "did not panic"; nothing drawn since verb_menu is None.
    }
}
