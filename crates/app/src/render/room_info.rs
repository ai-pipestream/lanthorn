//! Room-info body: the story-facing view of one room, drawn by the room dock.
//!
//! Shows the room's notes and its EXIT CARD — one line per direction, in the matrix view's
//! vocabulary, with destination names spelled out (SQ-0666). When the displayed room is the
//! player's current room, also lists the objects in that room queried live from the Z-machine
//! object tree. (The room's NAME and layer are the dock header's job — see
//! [`crate::render::room_dock`] — so the body does not repeat them.)
//!
//! The card is the per-room form of the matrix: same seven cells, same meanings, one room at a
//! time and no numbering to decode. It replaced a plain `dir -> name` list that could not say
//! whether a direction had been tried, and — with the matrix — the room inspector's compass rose
//! and the map's untried-exits overlay, which each said less.
//!
//! SQ-0692 retired the floating-dialog wrapper this used to live in: the body draws into a plain
//! `Rect` now, so the dock owns the chrome and there is exactly one panel describing a room.

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::draw_str_clipped;
use crate::theme::resolve::Theme;

/// A destination's display name, agreeing with whatever the matrix table itself would print for
/// it (SQ-0685): the NUMBERED row label ("Maze 4") when the destination shares `labels`' layer —
/// every cell `card_detail` handles except `LeavesLayer` names one of those — falling back to the
/// room's bare name for a destination outside it, which has no row in `labels` to number. Numbers
/// are minted by discovery order, not room id, but that is `labels`' concern entirely; naming here
/// only has to keep asking the one function that knows, so the card and the matrix can never
/// disagree about what a room is called.
fn dest_name(graph: &MapGraph, labels: &mapper::matrix::MatrixLabels, layer: mapper::layer::LayerId, id: RoomId) -> String {
    if graph.layer_of(id) == layer {
        let row = labels.row_of(id);
        if !row.is_empty() {
            return row.to_string();
        }
    }
    graph.room(id).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{id}"))
}

/// One card line for a direction: the glyph, and what it means spelled out.
///
/// Deliberately more verbose than the matrix cell it mirrors. The matrix is a table you scan
/// across twelve columns; the card is one room you are reading about, so there is room to say
/// "Maze 4" instead of "4" and "back: W" instead of "⇠w".
fn card_detail(
    graph: &MapGraph,
    labels: &mapper::matrix::MatrixLabels,
    layer: mapper::layer::LayerId,
    cell: mapper::matrix::MatrixCell,
) -> (&'static str, String) {
    use mapper::matrix::MatrixCell as C;
    let name = |id: RoomId| dest_name(graph, labels, layer, id);
    match cell {
        C::Reciprocal { dest } => ("⇄", name(dest)),
        C::ReturnBy { dest, back } => {
            ("→", format!("{}  back: {}", name(dest), dir_label(back)))
        }
        C::OneWay { dest } => ("⇢", name(dest)),
        C::SelfLoop => ("↩", "leads back here".to_string()),
        C::LeavesLayer { dest } => {
            // Cross-layer: `dest` has no row in THIS layer's `labels` to number it with, exactly
            // like the matrix's own `⇱out` footnote, which names the same way.
            let raw = graph.room(dest).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{dest}"));
            ("⇱", format!("{} · {}", raw, graph.layer_name(graph.layer_of(dest))))
        }
        C::Probed => ("×", "tried, no way through".to_string()),
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

/// A room's display name as every surface that names it agrees to spell it: the matrix's NUMBERED
/// row label ("Maze 4") when its layer numbers it, the bare room name otherwise (SQ-0685).
///
/// The dock header and the exit card both call this, so they cannot disagree.
pub fn display_name(graph: &MapGraph, room_id: RoomId) -> String {
    let layer = graph.layer_of(room_id);
    let labels = mapper::matrix::labels(graph, layer);
    let row = labels.row_of(room_id);
    if !row.is_empty() {
        return row.to_string();
    }
    graph.room(room_id).map(|r| r.label().to_owned()).unwrap_or_else(|| format!("#{room_id}"))
}

/// Draw the room-info body into `area` — no chrome, no borders: the caller (the room dock) owns
/// those.
///
/// - `graph`: the mapper graph for notes/exits.
/// - `room_objects`: the objects located in this room, already queried from the
///   engine's introspection (empty when introspection is unavailable, e.g. the
///   map is in tidy-anim mode). Shown only when this is the current room.
/// - `room_id`: the room to display.
/// - `current_room`: the player's actual current room (used to gate object listing).
/// - `theme`: for the shared `map.matrix.cell:frontier` dimming, so the card and the matrix agree.
/// - `body` / `heading`: the styles for ordinary lines and for section labels.
#[allow(clippy::too_many_arguments)]
pub fn draw_room_info_body(
    graph: &MapGraph,
    room_objects: &[String],
    room_id: RoomId,
    current_room: Option<RoomId>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    body: Style,
    heading: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(room) = graph.room(room_id) else { return };
    // Computed once and threaded through every name in this body, so the card can never disagree
    // with the matrix table or its `⇲`/`⇱out` footnotes about what a room is numbered (SQ-0685):
    // both ultimately read the same `labels`.
    let layer = graph.layer_of(room_id);
    let labels = mapper::matrix::labels(graph, layer);

    // The exit card: every one of the twelve travel directions, classified exactly as the matrix
    // view classifies it. All twelve, including the untried ones — "where haven't I been?" is the
    // question this panel inherited when the untried-exits overlay was retired, and a direction
    // left off the card is a direction the player stops considering.
    let card: Vec<(Direction, &'static str, String)> = mapper::matrix::MATRIX_DIRS
        .iter()
        .map(|&d| {
            let (glyph, detail) =
                card_detail(graph, &labels, layer, mapper::matrix::classify(graph, room_id, d));
            (d, glyph, detail)
        })
        .collect();
    // Non-compass passages (xyzzy, pray) have no column in the twelve and would otherwise vanish
    // from the card entirely.
    let odd: Vec<String> =
        graph
            .connections()
            .iter()
            .filter(|c| c.origin == room_id && c.dir == Direction::Unknown)
            .map(|c| dest_name(graph, &labels, layer, c.dest))
            .collect();

    // Show objects only when this is the current room.
    let objects: Vec<String> = if current_room == Some(room_id) {
        room_objects.to_vec()
    } else {
        Vec::new()
    };

    let value_style = body;
    let section_style = heading;

    let inner_x = area.x;
    let inner_w = area.width;
    let clip = area;
    let mut row = area.y;
    let max_y = area.bottom().saturating_sub(1);

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

    // Objects (only for the current room) come BEFORE the card (SQ-0692). The card is a fixed
    // thirteen-line block, so in a dock shortened past its natural height it is the section that
    // runs off the bottom — and it degrades gracefully, because every one of its rows is the same
    // shape and the ones that fit are still readable. A short "Here:" list buried underneath it
    // was simply invisible at any dock height a normal terminal can spare.
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

    // Exits — the card. Untried and dead-end directions are dimmed with the same selector the
    // matrix dims its frontier cells with, so the two surfaces read alike.
    let frontier_style = theme.get("map.matrix.cell:frontier").style;
    if row <= max_y {
        draw_str_clipped(buf, inner_x, row, "Exits:", section_style, clip);
        row += 1;
    }
    for (dir, glyph, detail) in &card {
        if row > max_y { break; }
        let line = format!("  {:<3} {} {}", dir_label(*dir), glyph, detail);
        let style = if detail.is_empty() || *glyph == "×" { frontier_style } else { value_style };
        draw_str_clipped(buf, inner_x, row, line.trim_end(), style, clip);
        row += 1;
    }
    for dest in &odd {
        if row > max_y { break; }
        draw_str_clipped(buf, inner_x, row, &format!("  ?   ⇢ {dest}"), value_style, clip);
        row += 1;
    }
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
    use ratatui::style::{Color, Style};

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


    fn test_theme() -> crate::theme::resolve::Theme {
        crate::colors::ColorScheme::terminal_default().theme
    }

    /// Draw the body into a plain rect the way the room dock does, and return the
    /// whole buffer as text.
    fn render_body(
        g: &MapGraph,
        objects: &[String],
        room: RoomId,
        current: Option<RoomId>,
        w: u16,
        h: u16,
    ) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let theme = test_theme();
        draw_room_info_body(
            g, objects, room, current, area, &mut buf, &theme,
            Style::default(), Style::default().fg(Color::Cyan),
        );
        (0..h)
            .map(|y| {
                (0..w).map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ")).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
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

        let text = render_body(&g, &[], room1, None, 70, 30);
        assert!(text.contains("⇄ Forest Path"), "east is reciprocal, and names where it goes:\n{text}");
        assert!(text.contains("→ Cellar"), "south reaches the Cellar:\n{text}");
        assert!(text.contains("back: NE"), "…and the way back is spelled out, not left as `⇠ne`");
        assert!(text.contains("W   × tried, no way through"), "west was typed and refused:\n{text}");
        assert!(text.contains("NE  ·"), "and an untried direction is still listed:\n{text}");
        for d in ["N ", "S ", "E ", "W ", "NE", "NW", "SE", "SW", "Up", "Dn", "In", "Out"] {
            assert!(text.contains(d), "the card lists every travel direction; {d} is missing:\n{text}");
        }
    }

    /// SQ-0685: when a destination shares its bare name with other rooms on the layer, the card
    /// must name it the same way the matrix table's rows and its `⇲`/`⇱out` footnotes do — the
    /// NUMBERED form ("Maze 2"), not the bare, undisambiguating room name every one of those rooms
    /// shares. Both surfaces read the numbering off the same `mapper::matrix::labels`, so they
    /// cannot disagree about what to call the same room.
    #[test]
    fn the_exit_card_names_a_same_named_destination_by_its_matrix_number() {
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.upsert_room(2, "Maze".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);

        // Independently computed, exactly as the matrix view itself would compute it.
        let expect_room2 = mapper::matrix::labels(&g, mapper::layer::MAIN_LAYER).row_of(2).to_string();
        assert_eq!(expect_room2, "Maze 2");

        let text = render_body(&g, &[], 1, None, 60, 20);
        assert!(
            text.contains(&expect_room2),
            "the exit card names its destination the way the matrix would:\n{text}"
        );
        // SQ-0692: the room's OWN numbered name moved to the dock header, which reads it from
        // the same place — so `display_name` must agree with the matrix too.
        assert_eq!(display_name(&g, 1), "Maze 1");
        assert_eq!(display_name(&g, 2), "Maze 2");
    }

    /// SQ-0685: a destination that LEAVES the layer has no row in this room's `labels` to number
    /// it with — same as the matrix table's own `⇱out` footnote — so it keeps its bare name rather
    /// than showing an empty or wrong number.
    #[test]
    fn a_cross_layer_destination_keeps_its_bare_name_not_a_number_from_the_wrong_layer() {
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "Maze".into()); // same bare name, but on ANOTHER layer
        g.set_pos(2, (0, 0));
        let other = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Elsewhere".into());
        g.set_room_layer(2, other);
        g.add_edge(1, Direction::Down, 2);

        let text = render_body(&g, &[], 1, None, 60, 20);
        assert!(text.contains("Maze"), "the destination is still named");
        assert!(!text.contains("Maze 2"), "…but must not borrow a number that means nothing here:\n{text}");
        assert!(text.contains("Elsewhere"), "the crossing still names the destination layer:\n{text}");
        // Room 1 is alone on Main, so its own display name has no number either.
        assert_eq!(display_name(&g, 1), "Maze");
    }

    /// SQ-0638: a room note packed with multibyte chars (each '€' is 3 bytes)
    /// used to panic — the wrap loop sliced `&notes[offset..end]` at a fixed
    /// BYTE offset that could land mid-character.
    #[test]
    fn room_info_notes_with_multibyte_chars_does_not_panic() {
        let (mut g, room1, _) = make_graph_with_rooms();
        g.set_notes(room1, "€".repeat(12));
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(text.contains("€"), "the multibyte note text should still render");
    }

    #[test]
    fn room_info_body_shows_exits_but_not_the_room_name() {
        // SQ-0692: the name (and layer) belong to the dock header now, so the body
        // starts at the notes / exit card. Repeating the name inside the panel that
        // already titles itself with it was the first thing to go when the two
        // floating dialogs became one dock.
        let (g, room1, _room2) = make_graph_with_rooms();
        let text = render_body(&g, &[], room1, None, 60, 20);
        assert!(!text.contains("West of House"), "the body does not repeat the header's name:\n{text}");
        assert!(text.contains("Exits:"), "it starts at the exit card:\n{text}");
        assert!(text.contains("Forest Path"), "…which names the destination");
    }

    #[test]
    fn room_info_no_objects_for_non_current_room() {
        let (g, room1, room2) = make_graph_with_rooms();
        // room2 is not the current room, so no objects section — even with objects passed in.
        let text = render_body(&g, &["lamp".to_string()], room2, Some(room1), 60, 20);
        assert!(!text.contains("Here:"), "objects section should not appear for a non-current room");
        assert!(!text.contains("lamp"), "nor the objects themselves");
    }

    #[test]
    fn room_info_lists_objects_for_the_current_room() {
        let (g, room1, _) = make_graph_with_rooms();
        let text = render_body(&g, &["brass lantern".to_string()], room1, Some(room1), 60, 24);
        assert!(text.contains("Here:"), "the current room's objects get a section:\n{text}");
        assert!(text.contains("brass lantern"), "{text}");
    }

    #[test]
    fn room_info_body_is_silent_for_a_missing_room() {
        let g = MapGraph::new();
        let text = render_body(&g, &[], 99, None, 60, 20);
        assert!(text.trim().is_empty(), "a room that is not in the graph draws nothing:\n{text}");
    }

    #[test]
    fn room_info_body_zero_area_does_not_panic() {
        let (g, room1, _) = make_graph_with_rooms();
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        let theme = test_theme();
        draw_room_info_body(
            &g, &[], room1, None, Rect::new(0, 0, 0, 0), &mut buf, &theme,
            Style::default(), Style::default(),
        );
    }
}
