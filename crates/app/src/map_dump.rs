//! Map dump: a self-contained, human-readable text snapshot of the explored map
//! for offline evaluation and regression fixtures.
//!
//! The dump has three parts:
//!   1. a ROOM legend (`id`, name, grid pos, notes),
//!   2. an EDGE list (directed `origin DIR dest`, marked `distorted` when the
//!      layout could not satisfy the compass direction),
//!   3. an ASCII rendering of the actual map — produced by rendering the real
//!      `render_map` into an off-screen buffer at Boxes zoom and serializing the
//!      colored ribbons back into `─│┼` line-art — so it faithfully shows the
//!      routing (clearances, perpendicular crossings, overlaps) the TUI draws.
//!
//! Lines starting with `#` are comments: the file is meant to be annotated (mark
//! which room ids look wrong) and handed back for analysis.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::render::render;

use crate::render::map::render_map;
use crate::state::{AppState, Zoom};

/// Boxes-zoom stride (must match `Zoom::Boxes.steps()`); used to size the dump buffer.
const STEP: i32 = 29;
/// Max dump buffer dimension (cells) to bound memory on very large maps.
const MAX_DIM: i32 = 4000;

/// Short label for a connection direction (matches the TUI / DOT conventions).
fn dir_str(d: Direction) -> &'static str {
    match d {
        Direction::N => "N",
        Direction::S => "S",
        Direction::E => "E",
        Direction::W => "W",
        Direction::NE => "NE",
        Direction::NW => "NW",
        Direction::SE => "SE",
        Direction::SW => "SW",
        Direction::Up => "U",
        Direction::Down => "D",
        Direction::In => "IN",
        Direction::Out => "OUT",
        Direction::Unknown => "?",
    }
}

/// Map a NESW connectivity bitmask (N=1, E=2, S=4, W=8) to a box-drawing glyph.
fn mask_glyph(mask: u8) -> char {
    match mask {
        1 | 4 | 5 => '│',
        2 | 8 | 10 => '─',
        3 => '└',
        6 => '┌',
        12 => '┐',
        9 => '┘',
        7 => '├',
        14 => '┬',
        13 => '┤',
        11 => '┴',
        15 => '┼',
        _ => '·', // isolated path cell (no orthogonal path neighbour)
    }
}

/// True if `(x, y)` in `buf` is a connector cell (ribbon background — also covers
/// embedded arrowheads, which carry the same background).
fn is_path(buf: &Buffer, x: i32, y: i32, area: Rect) -> bool {
    if x < 0 || y < 0 || x >= area.width as i32 || y >= area.height as i32 {
        return false;
    }
    buf.cell((x as u16, y as u16))
        .map(|c| c.bg == Color::Cyan || c.bg == Color::Magenta)
        .unwrap_or(false)
}

/// Render the map to a line-art ASCII string (rooms as boxes, connectors as
/// `─│┼` lines, exits as arrowheads).
#[allow(clippy::field_reassign_with_default)]
fn ascii_map(graph: &MapGraph) -> String {
    let rm = render(graph);
    if rm.rooms.is_empty() {
        return "(empty map)".to_string();
    }
    let ((min_col, min_row), (max_col, max_row)) = rm.bounds;
    let cols = max_col - min_col + 1;
    let rows = max_row - min_row + 1;

    // Pad by 2 room-steps each side so connectors detouring outside the room
    // bounding box are captured rather than clipped.
    let area_w = ((cols + 4) * STEP).clamp(STEP, MAX_DIM);
    let area_h = ((rows + 4) * STEP).clamp(STEP, MAX_DIM);
    let area = Rect::new(0, 0, area_w as u16, area_h as u16);

    // Render at Boxes zoom (AppState default) with scroll set to pad the map.
    let mut state = AppState::default();
    state.zoom = Zoom::Boxes;
    state.scroll = (min_col - 2, min_row - 2);

    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    // Serialize: real symbols pass through; blank ribbon cells become line-art.
    let mut lines: Vec<String> = Vec::with_capacity(area_h as usize);
    for y in 0..area_h {
        let mut line = String::new();
        for x in 0..area_w {
            let sym = buf.cell((x as u16, y as u16)).map(|c| c.symbol()).unwrap_or(" ");
            if sym != " " {
                line.push_str(sym);
            } else if is_path(&buf, x, y, area) {
                let mut mask = 0u8;
                if is_path(&buf, x, y - 1, area) { mask |= 1; }
                if is_path(&buf, x + 1, y, area) { mask |= 2; }
                if is_path(&buf, x, y + 1, area) { mask |= 4; }
                if is_path(&buf, x - 1, y, area) { mask |= 8; }
                line.push(mask_glyph(mask));
            } else {
                line.push(' ');
            }
        }
        lines.push(line.trim_end().to_string());
    }

    // Trim leading/trailing all-blank rows.
    let first = lines.iter().position(|l| !l.is_empty()).unwrap_or(0);
    let last = lines.iter().rposition(|l| !l.is_empty()).unwrap_or(0);
    lines[first..=last].join("\n")
}

/// Produce the full map dump string for `graph`.
pub fn render_dump(graph: &MapGraph) -> String {
    let mut rooms: Vec<&mapper::graph::Room> = graph.rooms().collect();
    rooms.sort_by_key(|r| r.id);
    let conns = graph.connections();

    let mut out = String::new();
    out.push_str("# babelmap map dump\n");
    let current = graph.current().map(|id| format!("#{id}")).unwrap_or_else(|| "none".into());
    out.push_str(&format!(
        "# rooms: {}, edges: {}, current: {}\n#\n",
        rooms.len(),
        conns.len(),
        current
    ));

    if rooms.is_empty() {
        out.push_str("# (empty map)\n");
        return out;
    }

    out.push_str("# === ROOMS (id  name  pos  notes) ===\n");
    for r in &rooms {
        let pos = r.pos.map(|(x, y)| format!("{x},{y}")).unwrap_or_else(|| "?".into());
        let notes = if r.notes.is_empty() {
            String::new()
        } else {
            format!("  notes={:?}", r.notes)
        };
        out.push_str(&format!("ROOM {} {:?} pos={}{}\n", r.id, r.label(), pos, notes));
    }

    out.push_str("#\n# === EDGES (origin DIR dest) ===\n");
    for c in conns {
        let dist = if c.distorted { "  distorted" } else { "" };
        out.push_str(&format!("EDGE {} {} {}{}\n", c.origin, dir_str(c.dir), c.dest, dist));
    }

    out.push_str("#\n# === MAP (#id = room, lines = connectors, ▶◀▲▼ = exits) ===\n");
    out.push_str(&ascii_map(graph));
    out.push_str("\n#\n# Annotate problems below — lines starting with # are comments:\n#\n");

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    #[test]
    fn dump_lists_rooms_edges_and_ids() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        let dump = render_dump(&m.graph);

        assert!(dump.contains("# babelmap map dump"));
        assert!(dump.contains("ROOM 1 \"West of House\""), "room legend: {dump}");
        assert!(dump.contains("ROOM 2 \"Forest\""));
        assert!(dump.contains("EDGE 1 N 2"), "edge list: {dump}");
        // The ASCII map shows room ids.
        assert!(dump.contains("#1"));
        assert!(dump.contains("#2"));
    }

    #[test]
    fn dump_ascii_has_line_art_connector() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let dump = render_dump(&m.graph);
        // A horizontal connector run should serialize to box-drawing line-art.
        let has_line = dump.contains('─') || dump.contains('│') || dump.contains('┼');
        assert!(has_line, "expected line-art connectors in:\n{dump}");
        // And an exit arrow somewhere.
        let has_arrow = dump.contains('▶') || dump.contains('◀') || dump.contains('▲') || dump.contains('▼');
        assert!(has_arrow, "expected an exit arrow in:\n{dump}");
    }

    #[test]
    fn empty_map_dump_is_safe() {
        let g = MapGraph::new();
        let dump = render_dump(&g);
        assert!(dump.contains("# babelmap map dump"));
        assert!(dump.contains("(empty map)"));
    }
}
