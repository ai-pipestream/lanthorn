//! Verb/item token-palette modal overlay.
//!
//! `draw_verb_menu` renders a two-pane (Verbs | Nouns) layout with a small
//! Prepositions group at the right. Picking a token appends it (+ space) to
//! the input line; the player submits the composed command normally.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::render::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
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

// ── Drawing ───────────────────────────────────────────────────────────────────

/// Draw the verb/item token-palette modal overlay.
///
/// Renders via `draw_dialog` (centered, `[X]` + `[Done]`) so the opaque fill
/// prevents command-panel bleed. The verb|noun|prep palette is drawn into
/// the returned content rect.
///
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing), `None` when
/// the verb menu is closed or the area is too small.
pub fn draw_verb_menu(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
) -> Option<DialogRects> {
    let Some(vm) = &state.verb_menu else { return None };

    // ── Modal geometry ────────────────────────────────────────────────────────

    let modal_w = area.width.min(area.width); // fill the full width
    let modal_h = area.height.min(area.height); // fill the full height
    if modal_w < 20 || modal_h < 4 {
        return None;
    }

    // ── Build DialogStyle from state colors ───────────────────────────────────

    let st = DialogStyle::from_colors(&state.colors);

    let buttons = &[
        DialogButton { id: ButtonId::Done, label: "Done" },
    ];

    let spec = DialogSpec {
        title: "Verb Menu",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: None,
    };

    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    // ── Column layout within content ──────────────────────────────────────────

    if content.height < 2 || content.width < 10 {
        return Some(rects);
    }

    // Divide width: verbs (~30%), nouns (~50%), preps (~20%).
    let total_w = content.width;
    let verb_w = (total_w / 10 * 3).max(12).min(total_w.saturating_sub(20));
    let prep_w = (total_w / 10 * 2).clamp(10, 16);
    let noun_w = total_w.saturating_sub(verb_w).saturating_sub(prep_w);

    let verb_x = content.x;
    let noun_x = content.x + verb_w;
    let prep_x = content.x + verb_w + noun_w;

    // First row of content: pane headers; remaining rows: lists.
    let header_y = content.y;
    let list_y = content.y + 1;
    let list_h = content.height.saturating_sub(2); // 1 header + 1 input row
    *vp_out = list_h as usize;

    let verb_header_area = Rect { x: verb_x, y: header_y, width: verb_w, height: 1 };
    let noun_header_area = Rect { x: noun_x, y: header_y, width: noun_w, height: 1 };
    let prep_header_area = Rect { x: prep_x, y: header_y, width: prep_w, height: 1 };

    draw_pane_header("Verbs", vm.pane == VerbMenuPane::Verbs, verb_header_area, buf, state);
    draw_pane_header("Nouns", vm.pane == VerbMenuPane::Nouns, noun_header_area, buf, state);
    draw_pane_header("Preps", vm.pane == VerbMenuPane::Preps, prep_header_area, buf, state);

    if list_h > 0 {
        let verb_list = Rect { x: verb_x, y: list_y, width: verb_w, height: list_h };
        let noun_list = Rect { x: noun_x, y: list_y, width: noun_w, height: list_h };
        let prep_list = Rect { x: prep_x, y: list_y, width: prep_w, height: list_h };

        draw_list(
            VERB_MENU_VERBS,
            &vm.verb_scroll,
            vm.pane == VerbMenuPane::Verbs,
            verb_list,
            buf,
            state,
        );

        let noun_strs: Vec<&str> = vm.nouns.iter().map(|s| s.as_str()).collect();
        draw_list(
            &noun_strs,
            &vm.noun_scroll,
            vm.pane == VerbMenuPane::Nouns,
            noun_list,
            buf,
            state,
        );

        draw_list(
            VERB_MENU_PREPS,
            &vm.prep_scroll,
            vm.pane == VerbMenuPane::Preps,
            prep_list,
            buf,
            state,
        );
    }

    // Show current input composition in the bottom row of content.
    if !state.input.is_empty() && content.height >= 2 {
        let input_y = content.bottom() - 1;
        let input_text = format!("Input: {}_", state.input);
        let input_style = Style::new().fg(Color::Yellow).patch(state.colors.dialog);
        crate::render::draw_str_clipped(buf, content.x, input_y, &input_text, input_style, content);
    }

    Some(rects)
}

/// Draw the column header row using dialog colors.
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
    use crate::render::dialog::ButtonId;
    use crate::state::{AppState, VerbMenuState};

    fn make_state_with_verb_menu() -> AppState {
        let mut s = AppState::default();
        s.verb_menu = Some(VerbMenuState {
            pane: VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns: vec!["mailbox".to_string(), "door".to_string()],
        });
        s
    }

    #[test]
    fn verb_menu_renders_known_verb() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_verb_menu();
        terminal.draw(|f| {
            draw_verb_menu(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("look"), "should show 'look' verb");
        assert!(content.contains("Verbs"), "should show Verbs pane header");
    }

    #[test]
    fn verb_menu_scrollbar_and_paging_on_overflow() {
        use crate::input::{apply_action, Action, VerbMenuNavKind};
        use mapper::mapper::Mapper;
        // The verbs pane (35 built-ins) overflows a short modal.
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = make_state_with_verb_menu();
        let mut vp = 0usize;
        terminal.draw(|f| {
            draw_verb_menu(&state, f.area(), f.buffer_mut(), &mut vp);
        }).unwrap();
        assert!(vp > 0 && vp < VERB_MENU_VERBS.len(), "verbs should overflow (vp={vp})");
        let has_thumb = terminal.backend().buffer().content().iter().any(|c| c.symbol() == "█");
        assert!(has_thumb, "a scrollbar thumb should be drawn when the verbs pane overflows");

        // PageDown advances the active (Verbs) pane's selection by ~one viewport.
        state.modal_list_viewport = vp;
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::PageDown), &mut state, &mut Mapper::default());
        let sel = state.verb_menu.as_ref().unwrap().verb_scroll.selected;
        assert!(sel >= vp.saturating_sub(1), "PageDown should advance ~one viewport, got {sel}");
    }

    #[test]
    fn verb_menu_renders_room_noun() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_verb_menu();
        terminal.draw(|f| {
            draw_verb_menu(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("mailbox"), "should show room noun 'mailbox'");
        assert!(content.contains("Nouns"), "should show Nouns pane header");
    }

    #[test]
    fn verb_menu_opaque_background_no_bleed() {
        // Put some text in the buffer BEFORE drawing the modal, then verify
        // those cells are covered by the dialog background (opaque fill).
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_verb_menu();
        terminal.draw(|f| {
            // First write something that would bleed through without opaque bg.
            let bleed_style = ratatui::style::Style::new()
                .fg(Color::Red)
                .bg(Color::Green);
            for x in 0..80u16 {
                for y in 0..24u16 {
                    if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                        cell.set_symbol("X").set_style(bleed_style);
                    }
                }
            }
            // Now draw the modal — it should overwrite everything.
            draw_verb_menu(&state, f.area(), f.buffer_mut(), &mut 0);
        }).unwrap();
        // Check that cell (0, 0) no longer has Green background (modal replaced it).
        let cell = terminal.backend().buffer().cell((0, 0)).unwrap();
        assert_ne!(cell.bg, Color::Green, "modal should overwrite background (no bleed)");
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
    fn verb_menu_shows_dialog_chrome() {
        // Render test: verb menu shows bordered chrome + [X] + [Done].
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use crate::render::paneframe::BorderStyle;

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let mut state = make_state_with_verb_menu();
        // Set Single border so the title is visible (default is BorderStyle::None).
        state.colors.dialog_box_style = BorderStyle::Single;

        let rects_out = draw_verb_menu(&state, area, &mut buf, &mut 0);

        // Collect all cell symbols into one string for content search.
        let content: String = buf.content().iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");

        assert!(content.contains("Verb Menu"), "title 'Verb Menu' should be present");
        assert!(content.contains("Done"), "[Done] button should be visible");
        assert!(content.contains('✕'), "[X] close button should be visible");

        let rects = rects_out.expect("draw_verb_menu should return DialogRects when open");
        assert!(rects.close.is_some(), "close rect should be present");
        assert_eq!(rects.buttons.len(), 1, "should have 1 button");
        let ids: Vec<ButtonId> = rects.buttons.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&ButtonId::Done));
    }
}
