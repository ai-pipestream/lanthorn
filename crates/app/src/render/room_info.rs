//! Room-info panel: story-facing view of a clicked room.
//!
//! Shows the room's name, notes, and outgoing exits from the mapper graph.
//! When the displayed room is the player's current room, also lists the objects
//! in that room queried live from the Z-machine object tree.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::dialog::{ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use super::draw_str_clipped;

// Direction display labels (cardinal + diagonal + portal).
fn dir_label(dir: Direction) -> &'static str {
    match dir {
        Direction::N  => "N",
        Direction::NE => "NE",
        Direction::E  => "E",
        Direction::SE => "SE",
        Direction::S  => "S",
        Direction::SW => "SW",
        Direction::W  => "W",
        Direction::NW => "NW",
        Direction::Up => "Up",
        Direction::Down => "Dn",
        Direction::In => "In",
        Direction::Out => "Out",
        Direction::Unknown => "?",
    }
}

/// Render the room-info panel into the top-left corner of `map_area`.
///
/// - `graph`: the mapper graph for name/notes/exits.
/// - `room_objects`: the objects located in this room, already queried from the
///   engine's introspection (empty when introspection is unavailable, e.g. the
///   map is in tidy-anim mode). Shown only when this is the current room.
/// - `room_id`: the room to display.
/// - `current_room`: the player's actual current room (used to gate object listing).
/// - `dialog_style`: dialog chrome colors from state.colors.
///
/// Returns `Some(DialogRects)` when the panel was drawn (for mouse hit-testing).
pub fn draw_room_info(
    graph: &MapGraph,
    room_objects: &[String],
    room_id: RoomId,
    current_room: Option<RoomId>,
    map_area: Rect,
    buf: &mut Buffer,
    dialog_style: &DialogStyle,
) -> Option<DialogRects> {
    let room = graph.room(room_id)?;

    // Compute exits for this room.
    let exits: Vec<_> = graph
        .connections()
        .iter()
        .filter(|c| c.origin == room_id)
        .map(|c| {
            let dest_name = graph.room(c.dest).map(|r| r.label().to_owned())
                .unwrap_or_else(|| format!("#{}", c.dest));
            (c.dir, dest_name)
        })
        .collect();

    // Show objects only when this is the current room.
    let objects: Vec<String> = if current_room == Some(room_id) {
        room_objects.to_vec()
    } else {
        Vec::new()
    };

    // Panel sizing.
    const WIDTH: u16 = 36;
    const FIXED_ROWS: u16 = 7; // border-top + name + exits-header + border-bot + OK button row + 2 spare
    let notes_lines = if room.notes.is_empty() { 0u16 } else {
        // Wrap notes to panel inner width.
        let inner_w = WIDTH.saturating_sub(2) as usize;
        room.notes.len().div_ceil(inner_w) as u16
    };
    let exit_rows = exits.len() as u16;
    let obj_rows = if objects.is_empty() { 0u16 } else { objects.len() as u16 + 1 }; // +1 for header
    let needed_h = FIXED_ROWS + notes_lines + exit_rows + obj_rows;
    let panel_h = needed_h.min(map_area.height);
    let panel_w = WIDTH.min(map_area.width);

    if panel_w < 4 || panel_h < 4 {
        return None;
    }

    // Top-left corner.
    let panel = Rect::new(map_area.x, map_area.y, panel_w, panel_h);

    // Render via shared dialog chrome (positioned at the computed rect).
    let ok_btn = DialogButton { id: ButtonId::Ok, label: "OK" };
    let spec = DialogSpec {
        title: " Room Info ",
        placement: Placement::Positioned(panel),
        buttons: &[ok_btn],
        show_close: true,
        default: Some(ButtonId::Ok),
        focus: None,
        field: None,
    };
    let bounds = *buf.area();
    let dr = draw_dialog(buf, bounds, &spec, dialog_style);

    let content = dr.content;
    if content.height == 0 || content.width == 0 {
        return Some(dr);
    }

    let value_style = Style::default();
    let section_style = dialog_style.frame;
    let label_style = dialog_style.title;

    let inner_x = content.x;
    let inner_w = content.width;
    let clip = content;
    let mut row = content.y;
    let max_y = content.bottom().saturating_sub(1);

    // Room name.
    if row <= max_y {
        draw_str_clipped(buf, inner_x, row, room.label(), label_style, clip);
        row += 1;
    }

    // Notes (if any), word-wrapped naively by character width.
    if !room.notes.is_empty() && row <= max_y {
        let inner_w_usize = inner_w as usize;
        let notes = &room.notes;
        let mut offset = 0;
        while offset < notes.len() && row <= max_y {
            let end = (offset + inner_w_usize).min(notes.len());
            draw_str_clipped(buf, inner_x, row, &notes[offset..end], value_style, clip);
            offset = end;
            row += 1;
        }
    }

    // Exits.
    if row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Exits:", section_style, clip);
        row += 1;
    }
    for (dir, dest) in &exits {
        if row > max_y { break; }
        let line = format!("  {} -> {}", dir_label(*dir), dest);
        draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
        row += 1;
    }
    if exits.is_empty() && row <= max_y {
        draw_str_clipped(buf, inner_x, row, "  (none)", value_style, clip);
        row += 1;
    }

    // Objects (only for current room).
    if !objects.is_empty() && row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Here:", section_style, clip);
        row += 1;
        for name in &objects {
            if row > max_y { break; }
            let line = format!("  {}", name);
            draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
            row += 1;
        }
    }

    Some(dr)
}

/// List the display names of all direct children of the room object `room_id`
/// in the Z-machine object tree.
///
/// In Z-machine fiction, objects carried by or present in a room are children
/// of the room object (or children of containers in the room). We list only the
/// direct children here (one level deep), which covers visible items on the floor.
pub(crate) fn list_room_objects(mem: &zvm::memory::Memory, room_id: RoomId) -> Vec<String> {
    // Name-only rooms have no backing object; never read the object table by a
    // synthetic id (it would be outside the table).
    if crate::roomid::is_synthetic_room(room_id) {
        return Vec::new();
    }
    use zvm::objects::{get_child, get_sibling, short_name};
    let mut result = Vec::new();
    let mut child = get_child(mem, room_id);
    while child != 0 {
        let name = short_name(mem, child);
        if !name.is_empty() {
            result.push(name);
        }
        child = get_sibling(mem, child);
    }
    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::graph::MapGraph;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use ratatui::style::{Color, Style};
    use crate::render::dialog::{DialogPlacement, DialogStyle};
    use crate::render::paneframe::BorderStyle;

    #[test]
    fn list_room_objects_empty_for_synthetic_id() {
        // A synthetic RoomId (high bit set) must not read the object table.
        // Build a minimal v5 story in the same style as headless.rs's minimal_machine.
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 5; // version = 5
        buf[0x04] = 0x00; buf[0x05] = 0x40; // high_mem_base = 0x0040
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x00; buf[0x09] = 0x80; // dict = 0x0080
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev = 0x0060
        buf[0x0040] = 0xba; // quit opcode
        let mem = zvm::memory::Memory::new(buf).unwrap();
        let synth = crate::roomid::SYNTHETIC_ROOM_FLAG | 0x0123;
        assert!(list_room_objects(&mem, synth).is_empty());
    }

    fn make_dialog_style() -> DialogStyle {
        use crate::render::paneframe::PaneGlyphs;
        DialogStyle {
            frame: Style::default().bg(Color::Black),
            box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(),
            title: Style::default().fg(Color::Cyan),
            button: Style::default(),
            button_active: Style::default(),
            shadow: Style::default(),
            shadow_on: false,
            placement: DialogPlacement::Center,
            margin: 0,
        }
    }

    fn buf_contains(buf: &ratatui::buffer::Buffer, s: &str) -> bool {
        let all: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        all.contains(s)
    }

    fn make_graph_with_rooms() -> (MapGraph, RoomId, RoomId) {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.upsert_room(2, "Forest Path".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        (g, 1, 2)
    }

    #[test]
    fn room_info_shows_name_and_exit() {
        let (g, room1, _room2) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(buf_contains(&buf, "West of House"), "should show room name");
        assert!(buf_contains(&buf, "E"), "should show exit direction");
        assert!(buf_contains(&buf, "Forest Path"), "should show destination");
    }

    #[test]
    fn room_info_no_objects_for_non_current_room() {
        let (g, room1, room2) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        // room2 is not the current room (current_room = Some(room1)), so no objects section.
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room2, Some(room1), area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // "Here:" section should not appear for non-current rooms.
        assert!(!buf_contains(&buf, "Here:"), "objects section should not appear for non-current room");
    }

    #[test]
    fn room_info_shows_nothing_for_missing_room() {
        let g = MapGraph::new();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            // Room 99 does not exist; should not panic.
            draw_room_info(&g, &[], 99, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        // No assertion — just must not panic.
    }

    #[test]
    fn room_info_shows_objects_section_header_for_current_room_when_no_zvm() {
        // Even without machine_mem, the "Here:" section should not appear
        // (no objects to show means no header either).
        let (g, room1, _) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, Some(room1), area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // Without machine_mem, objects list is empty, so "Here:" should not appear.
        assert!(!buf_contains(&buf, "Here:"), "Here: should not appear without machine_mem");
    }

    #[test]
    fn room_info_panel_clears_reversed_modifier_and_fg() {
        // Pre-fill the buffer with REVERSED + fg(Cyan) to simulate map connector bleed.
        // After rendering, cells inside the panel must have no REVERSED modifier
        // and must have bg(Black).
        use ratatui::style::Modifier;
        let (g, room1, _) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            let bleed_style = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::REVERSED);
            // Pre-fill every cell with the bleed style.
            for y in 0..area.height {
                for x in 0..area.width {
                    if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(bleed_style);
                    }
                }
            }
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        for y in 0..4u16 {
            for x in 0..36u16 {
                if let Some(cell) = buf.cell((x, y)) {
                    assert!(
                        !cell.style().add_modifier.contains(ratatui::style::Modifier::REVERSED),
                        "cell ({x},{y}) must not have REVERSED modifier inside panel"
                    );
                    assert_eq!(
                        cell.style().bg,
                        Some(Color::Black),
                        "cell ({x},{y}) must have bg(Black) inside panel"
                    );
                }
            }
        }
    }

    #[test]
    fn room_info_panel_has_solid_black_background() {
        // All cells within the panel rect must have bg(Color::Black) to prevent
        // the map from showing through.
        let (g, room1, _) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // Panel is at (0,0) with width=36 (or area width if smaller).
        // Check a sample of cells inside the panel top-left region.
        for y in 0..4u16 {
            for x in 0..36u16 {
                if let Some(cell) = buf.cell((x, y)) {
                    assert_eq!(
                        cell.style().bg,
                        Some(Color::Black),
                        "cell ({x},{y}) should have bg(Black) inside panel"
                    );
                }
            }
        }
    }

    #[test]
    fn room_info_panel_has_close_button_and_border() {
        // The panel must show a border (single-line box) and a close [X] via draw_dialog.
        let (g, room1, _) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // The dialog chrome shows the close symbol.
        assert!(buf_contains(&buf, "\u{2715}"), "panel must show close symbol [X]");
        // Border: top-left corner of single-line box
        assert!(buf_contains(&buf, "\u{250c}"), "panel must show top-left border corner");
    }

    #[test]
    fn room_info_dialog_returns_close_rect() {
        // draw_room_info must return Some(DialogRects) with close.is_some() when drawn.
        let (g, room1, _) = make_graph_with_rooms();
        let area = ratatui::layout::Rect::new(0, 0, 60, 20);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let ds = make_dialog_style();
        let result = draw_room_info(&g, &[], room1, None, area, &mut buf, &ds);
        assert!(result.is_some(), "draw_room_info must return Some(DialogRects)");
        let dr = result.unwrap();
        assert!(dr.close.is_some(), "DialogRects.close must be Some (show_close=true)");
    }
}
