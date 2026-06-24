//! Verb/item token-palette modal overlay.
//!
//! `draw_verb_menu` renders a two-pane (Verbs | Nouns) layout with a small
//! Prepositions group at the right. Picking a token appends it (+ space) to
//! the input line; the player submits the composed command normally.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

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
/// The modal fills the full `area`, with an opaque background to prevent
/// panel bleed.  Layout: verbs column (left) | nouns column (middle) |
/// prepositions column (right).
pub fn draw_verb_menu(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(vm) = &state.verb_menu else { return };

    // Opaque background (Style::reset() clears any existing cell style to prevent bleed).
    let bg = Style::reset().bg(Color::DarkGray);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(bg);
            }
        }
    }

    if area.height < 3 || area.width < 20 {
        return;
    }

    // Title row.
    let title = "Verb Menu  Tab/\u{2190}\u{2192}: pane  \u{2191}\u{2193}: move  Enter/Space: pick  Esc/q: close";
    let title_style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD).bg(Color::DarkGray);
    crate::render::draw_str_clipped(buf, area.x + 1, area.y, title, title_style, area);

    let content_y = area.y + 1;
    let content_h = area.height.saturating_sub(2); // title + bottom padding

    if content_h == 0 {
        return;
    }

    // Divide width: verbs (~30%), nouns (~50%), preps (~20%).
    let total_w = area.width;
    let verb_w = (total_w / 10 * 3).max(12).min(total_w.saturating_sub(20));
    let prep_w = (total_w / 10 * 2).max(10).min(16);
    let noun_w = total_w.saturating_sub(verb_w).saturating_sub(prep_w);

    let verb_x = area.x;
    let noun_x = area.x + verb_w;
    let prep_x = area.x + verb_w + noun_w;

    let verb_area = Rect { x: verb_x, y: content_y, width: verb_w, height: content_h };
    let noun_area = Rect { x: noun_x, y: content_y, width: noun_w, height: content_h };
    let prep_area = Rect { x: prep_x, y: content_y, width: prep_w, height: content_h };

    draw_pane_header("Verbs", vm.pane == VerbMenuPane::Verbs, verb_area, buf);
    draw_pane_header("Nouns", vm.pane == VerbMenuPane::Nouns, noun_area, buf);
    draw_pane_header("Preps", vm.pane == VerbMenuPane::Preps, prep_area, buf);

    if content_h < 2 {
        return;
    }

    let list_y = content_y + 1;
    let list_h = content_h.saturating_sub(1);
    let verb_list = Rect { x: verb_x, y: list_y, width: verb_w, height: list_h };
    let noun_list = Rect { x: noun_x, y: list_y, width: noun_w, height: list_h };
    let prep_list = Rect { x: prep_x, y: list_y, width: prep_w, height: list_h };

    draw_list(
        &VERB_MENU_VERBS.iter().map(|s| *s).collect::<Vec<_>>(),
        vm.verb_idx,
        vm.pane == VerbMenuPane::Verbs,
        verb_list,
        buf,
    );

    let noun_strs: Vec<&str> = vm.nouns.iter().map(|s| s.as_str()).collect();
    draw_list(
        &noun_strs,
        vm.noun_idx,
        vm.pane == VerbMenuPane::Nouns,
        noun_list,
        buf,
    );

    draw_list(
        &VERB_MENU_PREPS.iter().map(|s| *s).collect::<Vec<_>>(),
        vm.prep_idx,
        vm.pane == VerbMenuPane::Preps,
        prep_list,
        buf,
    );

    // Show current input composition in the bottom row.
    if !state.input.is_empty() && area.height >= 3 {
        let input_y = area.bottom() - 1;
        let input_text = format!("Input: {}_", state.input);
        let input_style = Style::new().fg(Color::Yellow).bg(Color::DarkGray);
        crate::render::draw_str_clipped(buf, area.x + 1, input_y, &input_text, input_style, area);
    }
}

/// Draw the column header row.
fn draw_pane_header(title: &str, active: bool, area: Rect, buf: &mut Buffer) {
    if area.height == 0 {
        return;
    }
    let style = if active {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD).bg(Color::DarkGray)
    } else {
        Style::new().fg(Color::Gray).bg(Color::DarkGray)
    };
    let marker = if active { "> " } else { "  " };
    let line = format!("{}{}", marker, title);
    // Fill the header row to mark active column visually.
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
    crate::render::draw_str_clipped(buf, area.x, area.y, &line, style, area);
}

/// Draw a scrolled list of items with the selected item highlighted.
///
/// The list scrolls so the selected item is always visible. Items that do not
/// fit vertically are clipped; the area height is the visible row count.
fn draw_list(items: &[&str], selected: usize, active: bool, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let visible = area.height as usize;

    // Compute scroll offset so selected is visible.
    let scroll = if selected >= visible {
        selected - visible + 1
    } else {
        0
    };

    for row in 0..visible {
        let idx = scroll + row;
        let y = area.y + row as u16;
        if y >= area.bottom() {
            break;
        }

        if idx >= items.len() {
            // Empty rows: fill with background.
            for x in area.x..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(Style::new().bg(Color::DarkGray));
                }
            }
            continue;
        }

        let item = items[idx];
        let is_selected = idx == selected;

        let style = if is_selected && active {
            Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::new().fg(Color::White).bg(Color::DarkGray).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::White).bg(Color::DarkGray)
        };

        let marker = if is_selected && active { "> " } else { "  " };
        let line = format!("{}{}", marker, item);

        // Fill the row with the style, then write text.
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        crate::render::draw_str_clipped(buf, area.x, y, &line, style, area);
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
            verb_idx: 0,
            noun_idx: 0,
            prep_idx: 0,
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
            draw_verb_menu(&state, f.area(), f.buffer_mut());
        }).unwrap();
        let content: String = terminal.backend().buffer().content().iter()
            .map(|c| c.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(content.contains("look"), "should show 'look' verb");
        assert!(content.contains("Verbs"), "should show Verbs pane header");
    }

    #[test]
    fn verb_menu_renders_room_noun() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = make_state_with_verb_menu();
        terminal.draw(|f| {
            draw_verb_menu(&state, f.area(), f.buffer_mut());
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
        // those cells have DarkGray background (modal bg overwrote them).
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
            draw_verb_menu(&state, f.area(), f.buffer_mut());
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
}
