//! Room-info panel: story-facing view of a clicked room.
//!
//! Shows the room's name, notes, and its EXIT CARD — one line per direction, in the matrix view's
//! vocabulary, with destination names spelled out (SQ-0666). When the displayed room is the
//! player's current room, also lists the objects in that room queried live from the Z-machine
//! object tree.
//!
//! The card is the per-room form of the matrix: same seven cells, same meanings, one room at a
//! time and no numbering to decode. It replaced a plain `dir -> name` list that could not say
//! whether a direction had been tried, and — with the matrix — the room inspector's compass rose
//! and the map's untried-exits overlay, which each said less.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::dialog::{DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use super::draw_str_clipped;

/// One card line for a direction: the glyph, and what it means spelled out.
///
/// Deliberately more verbose than the matrix cell it mirrors. The matrix is a table you scan
/// across twelve columns; the card is one room you are reading about, so there is room to say
/// "Maze 4" instead of "4" and "back: W" instead of "⇠w".
fn card_detail(graph: &MapGraph, cell: mapper::matrix::MatrixCell) -> (&'static str, String) {
    use mapper::matrix::MatrixCell as C;
    let name = |id: RoomId| {
        graph.room(id).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{id}"))
    };
    match cell {
        C::Reciprocal { dest } => ("⇄", name(dest)),
        C::ReturnBy { dest, back } => {
            ("→", format!("{}  back: {}", name(dest), dir_label(back)))
        }
        C::OneWay { dest } => ("⇢", name(dest)),
        C::SelfLoop => ("↩", "leads back here".to_string()),
        C::LeavesLayer { dest } => {
            ("⇱", format!("{} · {}", name(dest), graph.layer_name(graph.layer_of(dest))))
        }
        C::Probed => ("_", "tried, no way through".to_string()),
        C::Untried => ("·", String::new()),
    }
}

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

    // The exit card: every one of the twelve travel directions, classified exactly as the matrix
    // view classifies it. All twelve, including the untried ones — "where haven't I been?" is the
    // question this panel inherited when the untried-exits overlay was retired, and a direction
    // left off the card is a direction the player stops considering.
    let card: Vec<(Direction, &'static str, String)> = mapper::matrix::MATRIX_DIRS
        .iter()
        .map(|&d| {
            let (glyph, detail) = card_detail(graph, mapper::matrix::classify(graph, room_id, d));
            (d, glyph, detail)
        })
        .collect();
    // Non-compass passages (xyzzy, pray) have no column in the twelve and would otherwise vanish
    // from the card entirely.
    let odd: Vec<String> = graph
        .connections()
        .iter()
        .filter(|c| c.origin == room_id && c.dir == Direction::Unknown)
        .map(|c| {
            graph.room(c.dest).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{}", c.dest))
        })
        .collect();

    // Show objects only when this is the current room.
    let objects: Vec<String> = if current_room == Some(room_id) {
        room_objects.to_vec()
    } else {
        Vec::new()
    };

    // Panel sizing.
    // Wider than the old `dir -> name` list needed: a card line carries a glyph, a room name and
    // sometimes a return direction.
    const WIDTH: u16 = 44;
    const FIXED_ROWS: u16 = 7; // border-top + name + exits-header + border-bot + OK button row + 2 spare
    let notes_lines = if room.notes.is_empty() { 0u16 } else {
        // Wrap notes to panel inner width, char/width-aware (SQ-0638): a
        // byte-offset estimate panics the caller below on a multibyte note
        // (e.g. one full of '€') long before it ever gets here, but a byte
        // COUNT here would just under/over-estimate the row count instead —
        // reuse the same wrapper the draw loop below uses so they agree.
        let inner_w = WIDTH.saturating_sub(2);
        crate::render::transcript::wrap_line(&room.notes, inner_w).len() as u16
    };
    let exit_rows = card.len() as u16 + odd.len() as u16;
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
    // No buttons: this is a corner overlay you read while you keep playing, not a
    // modal to dismiss, and an OK button implied otherwise. Closing is the ✕, Esc,
    // or a click on empty map space.
    let spec = DialogSpec {
        title: " Room Info ",
        placement: Placement::Positioned(panel),
        buttons: &[],
        show_close: true,
        default: None,
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

    // Notes (if any), word-wrapped char/width-aware (SQ-0638): a raw byte-offset
    // slice panics on a multibyte note (e.g. one full of '€') since a slice
    // boundary can land mid-character.
    if !room.notes.is_empty() && row <= max_y {
        for line in crate::render::transcript::wrap_line(&room.notes, inner_w) {
            if row > max_y { break; }
            draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
            row += 1;
        }
    }

    // Exits — the card. Untried and dead-end directions are dimmed with the same selector the
    // matrix dims its frontier cells with, so the two surfaces read alike.
    let frontier_style = dialog_style.theme.get("map.matrix.cell:frontier").style;
    if row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Exits:", section_style, clip);
        row += 1;
    }
    for (dir, glyph, detail) in &card {
        if row > max_y { break; }
        let line = format!("  {:<3} {} {}", dir_label(*dir), glyph, detail);
        let style = if detail.is_empty() || *glyph == "_" { frontier_style } else { value_style };
        draw_str_clipped(buf, inner_x, row, line.trim_end(), style, clip);
        row += 1;
    }
    for dest in &odd {
        if row > max_y { break; }
        draw_str_clipped(buf, inner_x, row, &format!("  ?   ⇢ {dest}"), value_style, clip);
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

/// List the display names of everything the player can see in room `room_id`.
///
/// Not simply the room object's direct children (SQ-0678). A Z-machine room
/// holds three kinds of visible thing, and only the first is a child of it:
///
/// - things on the floor — direct children;
/// - things on a supporter or inside an open container standing in the room —
///   children of *that furniture*, so the sack and bottle on Zork I's kitchen
///   table are two levels down, not one;
/// - shared scenery named by the room but parked in a bucket object — the
///   window at Behind House is never a child of any room.
///
/// `model` supplies the story-specific conventions needed to find the last two
/// safely; see [`zvm::world`] for how they are inferred and for the guarantee
/// that a closed container's contents never appear here.
pub(crate) fn list_room_objects(
    model: &zvm::world::WorldModel,
    mem: &zvm::memory::Memory,
    room_id: RoomId,
) -> Vec<String> {
    list_room_objects_excluding(model, mem, room_id, 0)
}

/// Same traversal as [`list_room_objects`], but skipping the object whose
/// id is `exclude` — and its whole subtree (0 excludes nothing: 0 is never a
/// valid object id). Used to keep the player object out of the command band's
/// "here" column (SQ-0667): filtering by id here, during the same walk that
/// builds the names, is what makes the exclusion exact rather than a fragile
/// name-match against whatever the player object happens to be called. Skipping
/// the subtree matters more now that the walk nests — the player is a holder
/// too, and their pockets are the *carried* column, never *here*.
pub(crate) fn list_room_objects_excluding(
    model: &zvm::world::WorldModel,
    mem: &zvm::memory::Memory,
    room_id: RoomId,
    exclude: u16,
) -> Vec<String> {
    // Name-only rooms have no backing object; never read the object table by a
    // synthetic id (it would be outside the table).
    if crate::roomid::is_synthetic_room(room_id) {
        return Vec::new();
    }
    model
        .visible_room_objects(mem, room_id, exclude)
        .into_iter()
        .map(|o| zvm::objects::short_name(mem, o))
        .filter(|n| !n.is_empty())
        .collect()
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
        let model = zvm::world::WorldModel::discover(&mem);
        assert!(list_room_objects(&model, &mem, synth).is_empty());
    }

    fn make_dialog_style() -> DialogStyle<'static> {
        use crate::render::paneframe::PaneGlyphs;
        static TEST_THEME: std::sync::LazyLock<crate::theme::resolve::Theme> =
            std::sync::LazyLock::new(|| crate::colors::ColorScheme::terminal_default().theme);
        DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default().bg(Color::Black),
            border: Style::default(), box_style: BorderStyle::Single,
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

    /// SQ-0666: the exits list became a CARD — one line per direction, in the matrix view's
    /// vocabulary, with the destination spelled out. It has to say all four things the old
    /// `dir -> name` list could not: which way a passage comes back, that it does not, that a
    /// direction was tried and refused, and that a direction was never tried at all. The last
    /// two are the coverage the retired untried-exits overlay handed over.
    #[test]
    fn the_exit_card_states_every_direction_in_the_matrix_vocabulary() {
        use mapper::direction::Direction;
        let (mut g, room1, room2) = make_graph_with_rooms();
        g.add_edge(room2, Direction::W, room1); // E is reciprocal
        g.upsert_room(3, "Cellar".into());
        g.set_pos(3, (0, 1));
        g.add_edge(room1, Direction::S, 3); // one-way
        g.add_edge(3, Direction::N, room1);
        g.relabel_connection(3, Direction::N, Direction::NE); // …no: comes back by NE
        g.mark_tried(room1, Direction::W); // typed west, hit a wall

        let backend = TestBackend::new(70, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        })
        .unwrap();
        let buf = terminal.backend().buffer().clone();
        let text: String = (0..buf.area().height)
            .map(|y| {
                (0..buf.area().width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("⇄ Forest Path"), "east is reciprocal, and names where it goes:\n{text}");
        assert!(text.contains("→ Cellar"), "south reaches the Cellar:\n{text}");
        assert!(text.contains("back: NE"), "…and the way back is spelled out, not left as `⇠ne`");
        assert!(text.contains("W   _ tried, no way through"), "west was typed and refused:\n{text}");
        assert!(text.contains("NE  ·"), "and an untried direction is still listed:\n{text}");
        for d in ["N ", "S ", "E ", "W ", "NE", "NW", "SE", "SW", "Up", "Dn", "In", "Out"] {
            assert!(text.contains(d), "the card lists every travel direction; {d} is missing:\n{text}");
        }
    }

    /// SQ-0638: a room note packed with multibyte chars (each '€' is 3 bytes)
    /// used to panic — the wrap loop sliced `&notes[offset..end]` at a fixed
    /// BYTE offset that could land mid-character. Twelve '€' at panel inner
    /// width 34 chars is comfortably past any single-byte boundary trap.
    #[test]
    fn room_info_notes_with_multibyte_chars_does_not_panic() {
        let (mut g, room1, _) = make_graph_with_rooms();
        g.set_notes(room1, "€".repeat(12));
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_room_info(&g, &[], room1, None, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(buf_contains(&buf, "€"), "the multibyte note text should still render");
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
