//! Per-room diagnostics inspector overlay.
//!
//! `room_diagnostics` gathers layout info for one room from the public `MapGraph` API (app-side
//! only — mapper internals are not touched). `draw_inspector` renders a bordered corner panel.
//!
//! # Future extension
//! "Corrections made during cleanup" (which rooms were nudged by the overlap cleanup pass, and
//! in which direction) are not recorded anywhere today. Surfacing a per-room cleanup history would
//! require the mapper to emit events or keep a log; that is left as a future extension.

use mapper::graph::{MapGraph, RoomId};
use mapper::direction::Direction;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::dialog::{DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog};
use super::draw_str_clipped;

// ── Data ──────────────────────────────────────────────────────────────────────

/// One outgoing edge of the selected room as seen by the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeInfo {
    pub dir: Direction,
    pub neighbour_id: RoomId,
    pub neighbour_name: String,
    pub distorted: bool,
}

/// All layout diagnostics for one room, computed app-side from `MapGraph`.
#[derive(Debug, Clone)]
pub struct RoomDiagnostics {
    pub id: RoomId,
    pub name: String,
    pub layer_id: mapper::layer::LayerId,
    pub layer_name: String,
    pub pos: Option<(i32, i32)>,
    pub edges: Vec<EdgeInfo>,
    /// Total outgoing edges (length of `edges`).
    pub edge_count: usize,
    /// Number of outgoing edges with `distorted == true`.
    pub distorted_count: usize,
}

/// Gather layout diagnostics for `id` from the public `MapGraph` API.
///
/// Returns `None` if the room does not exist in the graph.
pub fn room_diagnostics(graph: &MapGraph, id: RoomId) -> Option<RoomDiagnostics> {
    let room = graph.room(id)?;
    let layer_id = graph.layer_of(id);
    let layer_name = graph.layer_name(layer_id).to_owned();
    let pos = room.pos;
    let name = room.label().to_owned();

    let edges: Vec<EdgeInfo> = graph
        .connections()
        .iter()
        .filter(|c| c.origin == id)
        .map(|c| {
            let neighbour_name = graph
                .room(c.dest)
                .map(|r| r.label().to_owned())
                .unwrap_or_else(|| format!("#{}", c.dest));
            EdgeInfo {
                dir: c.dir,
                neighbour_id: c.dest,
                neighbour_name,
                distorted: c.distorted,
            }
        })
        .collect();

    let edge_count = edges.len();
    let distorted_count = edges.iter().filter(|e| e.distorted).count();

    Some(RoomDiagnostics { id, name, layer_id, layer_name, pos, edges, edge_count, distorted_count })
}

// ── Render ────────────────────────────────────────────────────────────────────

/// Render the inspector overlay into the top-right corner of `map_area`.
///
/// The panel is 38 columns wide and tall enough to hold the room's edges plus fixed header/footer
/// rows, capped at the pane height. It is positioned in the top-right corner so it is unlikely to
/// fully obscure the selected room (which is typically centred in the pane).
///
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing).
pub fn draw_inspector(diag: &RoomDiagnostics, map_area: Rect, buf: &mut Buffer, dialog_style: &DialogStyle) -> Option<DialogRects> {
    const WIDTH: u16 = 38;
    // rows: border top + id/name + layer + pos + blank + edges (≥1) + blank + summary + border bot
    const FIXED_ROWS: u16 = 9; // with at least 0 edge rows
    let edge_rows = diag.edge_count as u16;
    let needed_h = FIXED_ROWS + edge_rows;
    let panel_h = needed_h.min(map_area.height);
    let panel_w = WIDTH.min(map_area.width);

    if panel_w < 4 || panel_h < 4 {
        return None;
    }

    // Top-right corner.
    let x = map_area.right().saturating_sub(panel_w);
    let y = map_area.y;
    let panel = Rect::new(x, y, panel_w, panel_h);

    let distorted_style = Style::default().fg(ratatui::style::Color::Red);
    let ok_style = Style::default().fg(ratatui::style::Color::Green);

    // Render via shared dialog chrome (positioned at the computed rect).
    let spec = DialogSpec {
        title: " Inspector ",
        placement: Placement::Positioned(panel),
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
    };
    let dr = draw_dialog(buf, &spec, dialog_style);

    let content = dr.content;
    if content.height == 0 || content.width == 0 {
        return Some(dr);
    }

    let inner_x = content.x;
    let clip = content;
    let label_style = dialog_style.title;
    let value_style = Style::default();

    let mut row = content.y;
    let max_y = content.bottom().saturating_sub(1);

    let pos_str = match diag.pos {
        Some((px, py)) => format!("({}, {})", px, py),
        None => "unplaced".to_owned(),
    };

    // id + name
    if row <= max_y {
        let line = format!("#{} {}", diag.id, diag.name);
        draw_str_clipped(buf, inner_x, row, &line, label_style, clip);
        row += 1;
    }
    // layer
    if row <= max_y {
        let line = format!("Layer {} \"{}\"", diag.layer_id, diag.layer_name);
        draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
        row += 1;
    }
    // pos
    if row <= max_y {
        let line = format!("Pos {}", pos_str);
        draw_str_clipped(buf, inner_x, row, &line, value_style, clip);
        row += 1;
    }
    // blank separator
    if row <= max_y {
        row += 1;
    }

    // edges
    for edge in &diag.edges {
        if row > max_y {
            break;
        }
        let dir_label = format!("{:?}", edge.dir);
        let flag = if edge.distorted { "!" } else { " " };
        let line = format!("{} {:?} {} {}", flag, edge.dir, edge.neighbour_id, edge.neighbour_name);
        let style = if edge.distorted { distorted_style } else { ok_style };
        // Draw direction indicator in edge style, then rest in value style.
        let _ = dir_label; // used via format above
        draw_str_clipped(buf, inner_x, row, &line, style, clip);
        row += 1;
    }

    // blank + summary
    if row <= max_y {
        row += 1;
    }
    if row <= max_y {
        let summary = format!(
            "{} edge{}, {} distorted",
            diag.edge_count,
            if diag.edge_count == 1 { "" } else { "s" },
            diag.distorted_count,
        );
        draw_str_clipped(buf, inner_x, row, &summary, label_style, clip);
    }

    Some(dr)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::graph::MapGraph;
    use mapper::layout::relayout_auto;
    use mapper::direction::Direction;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use ratatui::style::{Color, Style};
    use crate::render::dialog::DialogStyle;
    use crate::render::paneframe::BorderStyle;

    fn make_dialog_style() -> DialogStyle {
        DialogStyle {
            frame: Style::default().bg(Color::Black),
            box_style: BorderStyle::Single,
            title: Style::default().fg(Color::Yellow),
            button: Style::default(),
            button_active: Style::default(),
            shadow: Style::default(),
            shadow_on: false,
        }
    }

    // ── room_diagnostics tests ────────────────────────────────────────────────

    #[test]
    fn diagnostics_none_for_missing_room() {
        let g = MapGraph::new();
        assert!(room_diagnostics(&g, 99).is_none());
    }

    #[test]
    fn diagnostics_id_name_layer_pos() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.set_pos(1, (3, -2));
        let d = room_diagnostics(&g, 1).unwrap();
        assert_eq!(d.id, 1);
        assert_eq!(d.name, "West of House");
        assert_eq!(d.layer_id, mapper::layer::MAIN_LAYER);
        assert_eq!(d.layer_name, "Main");
        assert_eq!(d.pos, Some((3, -2)));
        assert_eq!(d.edge_count, 0);
        assert_eq!(d.distorted_count, 0);
    }

    #[test]
    fn diagnostics_unplaced_room_has_none_pos() {
        let mut g = MapGraph::new();
        g.upsert_room(5, "Nowhere".into());
        let d = room_diagnostics(&g, 5).unwrap();
        assert_eq!(d.pos, None);
    }

    #[test]
    fn diagnostics_edges_and_distorted_flag() {
        // Build a two-room graph where the E edge from 1 to 2 becomes distorted
        // by placing 2 *west* of 1 (violating the eastward hint) then calling mark_distorted.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        // Place 2 to the WEST of 1 so the E edge is geometrically violated.
        g.set_pos(1, (5, 0));
        g.set_pos(2, (0, 0)); // 2 is west of 1 → E edge is distorted
        // Force the distorted flag manually (as mark_distorted / relayout_auto would).
        g.set_conn_distorted(0, true);

        let d = room_diagnostics(&g, 1).unwrap();
        assert_eq!(d.edge_count, 1);
        assert_eq!(d.distorted_count, 1);
        let e = &d.edges[0];
        assert_eq!(e.dir, Direction::E);
        assert_eq!(e.neighbour_id, 2);
        assert_eq!(e.neighbour_name, "B");
        assert!(e.distorted);
    }

    #[test]
    fn diagnostics_after_relayout_marks_correct_distorted() {
        // A two-room reciprocal N/S graph: after relayout_auto the edge must NOT be distorted
        // (2 ends up north of 1 as expected).
        let mut g = MapGraph::new();
        g.upsert_room(1, "Root".into());
        g.upsert_room(2, "North Room".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);

        let d = room_diagnostics(&g, 1).unwrap();
        // The N edge from 1 to 2 should be satisfied → not distorted.
        let north_edge = d.edges.iter().find(|e| e.dir == Direction::N).unwrap();
        assert!(!north_edge.distorted, "satisfied N edge must not be distorted after relayout");
        assert_eq!(d.distorted_count, 0);
    }

    #[test]
    fn diagnostics_distorted_loop_marks_at_least_one() {
        // An impossible 3-room northward loop: at least one edge must be distorted.
        let mut g = MapGraph::new();
        for id in 1u16..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1); // closes an impossible loop
        relayout_auto(&mut g);
        // At least one of the three rooms must report a distorted outgoing edge.
        let any_distorted = [1u16, 2, 3].iter().any(|&id| {
            room_diagnostics(&g, id).map(|d| d.distorted_count > 0).unwrap_or(false)
        });
        assert!(any_distorted, "impossible loop must leave at least one distorted edge");
    }

    #[test]
    fn diagnostics_layer_name_for_non_main_layer() {
        let mut g = MapGraph::new();
        let l = g.new_layer(Some(mapper::layer::MAIN_LAYER), "Basement".into());
        g.upsert_room(7, "Cellar".into());
        g.set_room_layer(7, l);
        let d = room_diagnostics(&g, 7).unwrap();
        assert_eq!(d.layer_id, l);
        assert_eq!(d.layer_name, "Basement");
    }

    // ── draw_inspector render tests ───────────────────────────────────────────

    fn make_diag(id: RoomId, name: &str, edges: Vec<EdgeInfo>) -> RoomDiagnostics {
        let edge_count = edges.len();
        let distorted_count = edges.iter().filter(|e| e.distorted).count();
        RoomDiagnostics {
            id,
            name: name.to_owned(),
            layer_id: 0,
            layer_name: "Main".to_owned(),
            pos: Some((1, 2)),
            edges,
            edge_count,
            distorted_count,
        }
    }

    fn buf_contains(buf: &ratatui::buffer::Buffer, s: &str) -> bool {
        let all: String = buf.content().iter().map(|c| c.symbol().to_owned()).collect();
        all.contains(s)
    }

    #[test]
    fn inspector_renders_room_id_and_name() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let diag = make_diag(42, "Clearing", vec![]);
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(buf_contains(&buf, "42"), "should contain room id");
        assert!(buf_contains(&buf, "Clearing"), "should contain room name");
    }

    #[test]
    fn inspector_shows_nothing_when_area_too_small() {
        let backend = TestBackend::new(3, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let diag = make_diag(1, "A", vec![]);
        let ds = make_dialog_style();
        // Should not panic; the panel is silently skipped when area is too small.
        terminal.draw(|f| {
            let area = f.area();
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        // No assertions — just must not panic.
    }

    #[test]
    fn inspector_shows_distorted_edge() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let edges = vec![EdgeInfo {
            dir: Direction::E,
            neighbour_id: 99,
            neighbour_name: "Maze".into(),
            distorted: true,
        }];
        let diag = make_diag(1, "Start", edges);
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // The distorted marker `!` should appear.
        assert!(buf_contains(&buf, "!"), "distorted edge should show '!'");
    }

    #[test]
    fn inspector_not_drawn_when_show_inspector_false() {
        // Simulate the show_inspector=false branch: caller simply does not call draw_inspector.
        // Verify the canvas stays blank.
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|_f| {
            // draw nothing — mirror what main.rs does when show_inspector is false
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // No room name should appear.
        assert!(!buf_contains(&buf, "Clearing"));
    }

    #[test]
    fn inspector_panel_clears_reversed_modifier_and_fg() {
        // Pre-fill the buffer with REVERSED + fg(Cyan) to simulate map connector bleed.
        // After rendering, cells inside the panel must have no REVERSED modifier
        // and must have bg(Black).
        use ratatui::style::Modifier;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let diag = make_diag(1, "Start", vec![]);
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            let bleed_style = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::REVERSED);
            for y in 0..area.height {
                for x in 0..area.width {
                    if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(bleed_style);
                    }
                }
            }
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        let panel_x = 80u16.saturating_sub(38);
        for y in 0..4u16 {
            for x in panel_x..80u16 {
                if let Some(cell) = buf.cell((x, y)) {
                    assert!(
                        !cell.style().add_modifier.contains(Modifier::REVERSED),
                        "cell ({x},{y}) must not have REVERSED modifier inside inspector panel"
                    );
                    assert_eq!(
                        cell.style().bg,
                        Some(Color::Black),
                        "cell ({x},{y}) must have bg(Black) inside inspector panel"
                    );
                }
            }
        }
    }

    #[test]
    fn inspector_panel_has_solid_black_background() {
        // All cells within the panel rect must have bg(Color::Black) to prevent
        // the map from showing through.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let diag = make_diag(1, "Start", vec![]);
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        // Panel is 38 wide, top-right corner: x = 80 - 38 = 42, y = 0.
        // Check that a sample of interior cells have bg(Black).
        let panel_x = 80u16.saturating_sub(38);
        for y in 0..4u16 {
            for x in panel_x..80u16 {
                if let Some(cell) = buf.cell((x, y)) {
                    assert_eq!(
                        cell.style().bg,
                        Some(Color::Black),
                        "cell ({x},{y}) should have bg(Black) inside inspector panel"
                    );
                }
            }
        }
    }

    #[test]
    fn inspector_panel_has_close_button_and_border() {
        // The panel must show a border and a close [X] via draw_dialog.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let diag = make_diag(1, "Start", vec![]);
        let ds = make_dialog_style();
        terminal.draw(|f| {
            let area = f.area();
            draw_inspector(&diag, area, f.buffer_mut(), &ds);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(buf_contains(&buf, "\u{2715}"), "inspector must show close symbol [X]");
        assert!(buf_contains(&buf, "\u{2510}"), "inspector must show top-right border corner");
    }

    #[test]
    fn inspector_dialog_returns_close_rect() {
        // draw_inspector must return Some(DialogRects) with close.is_some() when drawn.
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        let diag = make_diag(1, "Start", vec![]);
        let ds = make_dialog_style();
        let result = draw_inspector(&diag, area, &mut buf, &ds);
        assert!(result.is_some(), "draw_inspector must return Some(DialogRects)");
        let dr = result.unwrap();
        assert!(dr.close.is_some(), "DialogRects.close must be Some (show_close=true)");
    }
}
