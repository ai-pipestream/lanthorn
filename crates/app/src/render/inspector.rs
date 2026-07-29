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
    /// How the mapper first worked out the player was here (SQ-0527), recorded on
    /// the room at discovery. `None` for rooms mapped before it was recorded.
    pub loc_method: Option<String>,
    /// Which of the twelve travel directions have been explored FROM this room (SQ-0391), for
    /// the compass rose. Carried as a list so the panel needs no second look at the graph.
    pub explored: Vec<(mapper::direction::Direction, bool)>,
}

/// The twelve directions the inspector's compass rose reports on, in the order it draws them:
/// the eight compass points, then Up, In, Out, Down. Wider than the map overlay's set, which
/// deliberately omits In/Out — here there is room to say everything.
const ROSE_DIRS: [Direction; 12] = [
    Direction::NW, Direction::N, Direction::NE,
    Direction::W, Direction::E,
    Direction::SW, Direction::S, Direction::SE,
    Direction::Up, Direction::In, Direction::Out, Direction::Down,
];

/// The rose's compass letter for a direction, lower-case, or `""` for the four
/// portal directions — those draw their assigned map glyph instead, via
/// [`rose_cell`], so the inspector and the map name a staircase the same way.
fn rose_label(d: Direction) -> &'static str {
    match d {
        Direction::N => "n", Direction::NE => "ne", Direction::E => "e", Direction::SE => "se",
        Direction::S => "s", Direction::SW => "sw", Direction::W => "w", Direction::NW => "nw",
        Direction::Up | Direction::Down | Direction::In | Direction::Out => "",
        Direction::Unknown => "?",
    }
}

/// One rose cell's text. Compass points use their letter — lower-case untried,
/// UPPER-case once explored. The four portal directions use the themed glyph from
/// `portal_icons` / `[map.overrides]` in BOTH states: a glyph has no case, so
/// exploration is carried by style alone (muted → accent) for those.
fn rose_cell(d: Direction, portal: &crate::symbols::PortalGlyphs, explored: bool) -> String {
    match d {
        Direction::Up => portal.up.to_string(),
        Direction::Down => portal.down.to_string(),
        Direction::In => portal.in_.to_string(),
        Direction::Out => portal.out.to_string(),
        _ if explored => rose_label(d).to_uppercase(),
        _ => rose_label(d).to_string(),
    }
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

    let loc_method = room.loc_method.clone();
    let explored = ROSE_DIRS.iter().map(|&d| (d, graph.is_tried(id, d))).collect();
    Some(RoomDiagnostics { id, name, layer_id, layer_name, pos, edges, edge_count, distorted_count, loc_method, explored })
}

// ── Render ────────────────────────────────────────────────────────────────────

/// Render the inspector overlay into the top-right corner of `map_area`.
///
/// The panel is 38 columns wide and tall enough to hold the room's edges plus fixed header/footer
/// rows, capped at the pane height. It is positioned in the top-right corner so it is unlikely to
/// fully obscure the selected room (which is typically centred in the pane).
///
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing).
/// `portal` supplies the up/down/in/out glyphs for the explored-directions rose,
/// so the inspector names a staircase with the same character the map draws.
pub fn draw_inspector(diag: &RoomDiagnostics, map_area: Rect, buf: &mut Buffer, dialog_style: &DialogStyle, portal: &crate::symbols::PortalGlyphs) -> Option<DialogRects> {
    const WIDTH: u16 = 38;
    // rows: border top + id/name + layer + pos + blank + edges (≥1) + blank + summary + border bot
    const FIXED_ROWS: u16 = 13; // with at least 0 edge rows; +4 for the compass rose (SQ-0391)
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
        field: None,
    };
    let bounds = *buf.area();
    let dr = draw_dialog(buf, bounds, &spec, dialog_style);

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
    // How this room was first detected (SQ-0527). Kept on the room, so it is
    // still here long after the turn that discovered it — which the old map-corner
    // indicator never was.
    if let (Some(m), true) = (diag.loc_method.as_deref(), row <= max_y) {
        draw_str_clipped(buf, inner_x, row, &format!("Found by {m}"), value_style, clip);
        row += 1;
    }
    // Compass rose: which ways OUT of this room have been explored (SQ-0391). Lower-case italic
    // for a direction never tried, upper-case plain once it has been — so the shape of what is
    // left to do is readable at a glance, without counting letters. The vertical/portal block
    // sits to the right so it never competes with the compass for the same visual grid.
    //
    //     nw  n  ne    u
    //      w  +  e    i o
    //     sw  s  se    d
    if row + 3 <= max_y {
        row += 1; // breathing room under the detail lines
        let explored = |d: Direction| diag.explored.iter().any(|&(k, v)| k == d && v);
        // Exploration reads as colour, not emphasis: muted for a way out you have
        // never taken, accent once you have. The four portal slots keep the same
        // glyph in both states, so for them this styling is the whole signal.
        let tried_style = dialog_style.theme.get("accent").style;
        let untried_style = dialog_style.theme.get("muted").style;
        // (column, direction) per row; `None` is the rose's centre mark.
        let rows: [[(u16, Option<Direction>); 5]; 3] = [
            [(0, Some(Direction::NW)), (4, Some(Direction::N)), (7, Some(Direction::NE)),
             (12, Some(Direction::Up)), (14, None)],
            [(1, Some(Direction::W)), (4, None), (7, Some(Direction::E)),
             (11, Some(Direction::In)), (13, Some(Direction::Out))],
            [(0, Some(Direction::SW)), (4, Some(Direction::S)), (7, Some(Direction::SE)),
             (12, Some(Direction::Down)), (14, None)],
        ];
        for (dy, cells) in rows.iter().enumerate() {
            for &(dx, dir) in cells {
                let Some(d) = dir else { continue };
                let tried = explored(d);
                let text = rose_cell(d, portal, tried);
                let style = if tried { tried_style } else { untried_style };
                draw_str_clipped(buf, inner_x + dx, row + dy as u16, &text, style, clip);
            }
        }
        // The centre mark, so the cross reads as a compass rather than loose letters.
        draw_str_clipped(buf, inner_x + 4, row + 1, "+", label_style, clip);
        row += 3;
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
    use crate::render::dialog::{DialogPlacement, DialogStyle};
    use crate::render::paneframe::BorderStyle;

    pub(super) fn make_dialog_style() -> DialogStyle<'static> {
        use crate::render::paneframe::PaneGlyphs;
        static TEST_THEME: std::sync::LazyLock<crate::theme::resolve::Theme> =
            std::sync::LazyLock::new(|| crate::colors::ColorScheme::terminal_default().theme);
        DialogStyle {
            theme: &TEST_THEME,
            frame: Style::default().bg(Color::Black),
            border: Style::default(), box_style: BorderStyle::Single,
            glyphs: PaneGlyphs::default(),
            title: Style::default().fg(Color::Yellow),
            button: Style::default(),
            button_active: Style::default(),
            shadow: Style::default(),
            shadow_on: false,
            placement: DialogPlacement::Center,
            margin: 0,
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
            loc_method: Some("via status variable".to_owned()),
            // A room where north and up have been explored and nothing else has.
            explored: ROSE_DIRS
                .iter()
                .map(|&d| (d, matches!(d, Direction::N | Direction::Up)))
                .collect(),
        }
    }

    /// SQ-0527: the inspector reports how the mapper first found the room. It
    /// used to be a transient corner indicator describing only the LAST detection,
    /// which was gone by the time you wanted to know about a given room.
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
        }).unwrap();
        let buf = terminal.backend().buffer().clone();
        assert!(buf_contains(&buf, "42"), "should contain room id");
        assert!(buf_contains(&buf, "Clearing"), "should contain room name");
        // SQ-0527: and how the mapper first found the room. This used to be a
        // transient corner indicator describing only the LAST detection, gone by
        // the time you wanted to know about a given room.
        assert!(
            buf_contains(&buf, "Found by via status variable"),
            "the inspector names the detection method recorded on the room"
        );
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
        }).unwrap();
        // No assertions — just must not panic.
    }

    /// SQ-0391: the rose says which ways OUT of this room have been explored.
    /// Compass points go lower-case → UPPER-case; the four portal slots keep their
    /// themed glyph in both states, so for those exploration is style-only (muted →
    /// accent), asserted separately below.
    #[test]
    fn inspector_rose_capitalises_the_directions_already_explored() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.upsert_room(2, "North of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2); // walked north
        g.mark_tried(1, Direction::E); // typed east, went nowhere — still explored
        g.mark_tried(1, Direction::Down);
        let diag = room_diagnostics(&g, 1).unwrap();
        let area = Rect::new(0, 0, 44, 24);
        let mut buf = Buffer::empty(area);
        draw_inspector(&diag, area, &mut buf, &make_dialog_style(), &crate::symbols::SymbolSet::default().portal);

        // The rose, drawn as three rows. A walked edge and a foiled attempt both
        // count. Up/Down/In/Out show the default portal glyphs ↑/↓/⊙/⊗ regardless of
        // whether they have been tried — only their colour moves.
        assert!(buf_contains(&buf, "nw  N  ne   \u{2191}"), "north walked -> N; up keeps its glyph");
        assert!(buf_contains(&buf, " w  +  E   \u{2299} \u{2297}"), "east tried though it went nowhere -> E");
        assert!(buf_contains(&buf, "sw  s  se   \u{2193}"), "down tried -> still the glyph, not 'D'");
    }

    /// The rose signals exploration with COLOUR, not emphasis: muted for a way out
    /// never taken, accent once taken. That is the only signal the four portal
    /// slots have, since a glyph carries no case.
    #[test]
    fn inspector_rose_styles_untried_muted_and_tried_accent() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.set_pos(1, (0, 0));
        g.mark_tried(1, Direction::Down); // down taken; up never
        let diag = room_diagnostics(&g, 1).unwrap();
        let area = Rect::new(0, 0, 44, 24);
        let mut buf = Buffer::empty(area);
        let ds = make_dialog_style();
        draw_inspector(&diag, area, &mut buf, &ds, &crate::symbols::SymbolSet::default().portal);

        let accent = ds.theme.get("accent").style;
        let muted = ds.theme.get("muted").style;
        assert_ne!(accent, muted, "the test is meaningless if the two roles look alike");

        let find = |glyph: char| -> Style {
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let c = buf.cell((x, y)).unwrap();
                    if c.symbol().starts_with(glyph) {
                        return c.style();
                    }
                }
            }
            panic!("glyph {glyph:?} not found in the rose");
        };
        // ↓ was taken, ↑ was not — same glyph family, opposite styling.
        assert_eq!(find('\u{2193}').fg, accent.fg, "a tried portal draws in accent");
        assert_eq!(find('\u{2191}').fg, muted.fg, "an untried portal draws in muted");
        // A compass point never tried is muted too (and stays lower-case).
        assert_eq!(find('w').fg, muted.fg, "an untried compass point draws in muted");
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
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
            draw_inspector(&diag, area, f.buffer_mut(), &ds, &crate::symbols::SymbolSet::default().portal);
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
        let result = draw_inspector(&diag, area, &mut buf, &ds, &crate::symbols::SymbolSet::default().portal);
        assert!(result.is_some(), "draw_inspector must return Some(DialogRects)");
        let dr = result.unwrap();
        assert!(dr.close.is_some(), "DialogRects.close must be Some (show_close=true)");
    }
}

