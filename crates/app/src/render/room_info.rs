//! Room-info panel: story-facing view of a clicked room.
//!
//! Shows the room's name, notes, and outgoing exits from the mapper graph.
//! When the displayed room is the player's current room, also lists the objects
//! in that room queried live from the Z-machine object tree.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

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

fn draw_border(buf: &mut Buffer, area: Rect, style: Style) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let set_cell = |buf: &mut Buffer, x: u16, y: u16, ch: char| {
        if let Some(cell) = buf.cell_mut((x, y)) {
            let mut s = [0u8; 4];
            cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
        }
    };
    let x0 = area.x;
    let y0 = area.y;
    let x1 = area.right() - 1;
    let y1 = area.bottom() - 1;
    set_cell(buf, x0, y0, '\u{250c}');
    set_cell(buf, x1, y0, '\u{2510}');
    set_cell(buf, x0, y1, '\u{2514}');
    set_cell(buf, x1, y1, '\u{2518}');
    for x in (x0 + 1)..x1 {
        set_cell(buf, x, y0, '\u{2500}');
        set_cell(buf, x, y1, '\u{2500}');
    }
    for y in (y0 + 1)..y1 {
        set_cell(buf, x0, y, '\u{2502}');
        set_cell(buf, x1, y, '\u{2502}');
    }
    let title = " Room Info ";
    let title_clip = Rect::new(x0, y0, area.width, 1);
    draw_str_clipped(buf, x0 + 1, y0, title, style, title_clip);
}

/// Render the room-info panel into the top-left corner of `map_area`.
///
/// - `graph`: the mapper graph for name/notes/exits.
/// - `machine_mem`: the Z-machine memory for listing current-room objects (`None` when
///   the map is in tidy-anim mode and the live machine isn't available).
/// - `room_id`: the room to display.
/// - `current_room`: the player's actual current room (used to gate object listing).
pub fn draw_room_info(
    graph: &MapGraph,
    machine_mem: Option<&zvm::memory::Memory>,
    room_id: RoomId,
    current_room: Option<RoomId>,
    map_area: Rect,
    buf: &mut Buffer,
) {
    let Some(room) = graph.room(room_id) else { return };

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

    // Collect objects only when this is the current room AND we have memory.
    let objects: Vec<String> = if current_room == Some(room_id) {
        if let Some(mem) = machine_mem {
            list_room_objects(mem, room_id)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Panel sizing.
    const WIDTH: u16 = 36;
    const FIXED_ROWS: u16 = 6; // border-top + name + notes-header + (blank) + exits-header + border-bot
    let notes_lines = if room.notes.is_empty() { 0u16 } else {
        // Wrap notes to panel inner width.
        let inner_w = WIDTH.saturating_sub(2) as usize;
        ((room.notes.len() + inner_w - 1) / inner_w) as u16
    };
    let exit_rows = exits.len() as u16;
    let obj_rows = if objects.is_empty() { 0u16 } else { objects.len() as u16 + 1 }; // +1 for header
    let needed_h = FIXED_ROWS + notes_lines + exit_rows + obj_rows;
    let panel_h = needed_h.min(map_area.height);
    let panel_w = WIDTH.min(map_area.width);

    if panel_w < 4 || panel_h < 4 {
        return;
    }

    // Top-left corner.
    let panel = Rect::new(map_area.x, map_area.y, panel_w, panel_h);

    let border_style = Style::default().fg(Color::Cyan);
    let label_style = Style::default().fg(Color::Cyan);
    let value_style = Style::default();
    let section_style = Style::default().fg(Color::DarkGray);

    // Fill the panel with a solid opaque background so the map does not show through.
    // Style::reset() clears all inherited attributes (fg, modifiers such as REVERSED)
    // before applying bg(Black), preventing map connector colors and reversed-block
    // modifiers from bleeding through.
    let bg_style = Style::reset().bg(Color::Black);
    for y in panel.y..panel.bottom() {
        for x in panel.x..panel.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(bg_style);
            }
        }
    }

    draw_border(buf, panel, border_style);

    let inner_x = panel.x + 1;
    let inner_w = panel.width.saturating_sub(2);
    let clip = Rect::new(inner_x, panel.y + 1, inner_w, panel.height.saturating_sub(2));
    if clip.height == 0 || clip.width == 0 {
        return;
    }

    let mut row = clip.y;
    let max_y = clip.bottom().saturating_sub(1);

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
}

/// List the display names of all direct children of the room object `room_id`
/// in the Z-machine object tree.
///
/// In Z-machine fiction, objects carried by or present in a room are children
/// of the room object (or children of containers in the room). We list only the
/// direct children here (one level deep), which covers visible items on the floor.
fn list_room_objects(mem: &zvm::memory::Memory, room_id: RoomId) -> Vec<String> {
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
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, None, room1, None, area, f.buffer_mut());
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
        // room2 is not the current room (current_room = Some(room1)), so no objects section.
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, None, room2, Some(room1), area, f.buffer_mut());
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
        terminal.draw(|f| {
            let area = f.area();
            // Room 99 does not exist; should not panic.
            draw_room_info(&g, None, 99, None, area, f.buffer_mut());
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
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, None, room1, Some(room1), area, f.buffer_mut());
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
        use ratatui::style::{Color, Modifier, Style};
        let (g, room1, _) = make_graph_with_rooms();
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
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
            draw_room_info(&g, None, room1, None, area, f.buffer_mut());
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        for y in 0..4u16 {
            for x in 0..36u16 {
                if let Some(cell) = buf.cell((x, y)) {
                    assert!(
                        !cell.style().add_modifier.contains(Modifier::REVERSED),
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
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, None, room1, None, area, f.buffer_mut());
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
}
