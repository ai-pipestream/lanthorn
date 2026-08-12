//! SQ-0391's untried-exits overlay, retired by SQ-0666 — this is the file that used to pin it.
//!
//! The overlay marked every never-tried compass direction with a `?` on the room's box outline.
//! It answered "where haven't I been?" for one room at a time, in a vocabulary of exactly one
//! glyph, and it was the third UI surface for a fact the matrix view's `_`/`·` cells and the
//! room-info exit card each state more completely. One representation per fact: the overlay went,
//! along with its `toggle-untried-exits` command and its `map.untried_exit` selector.
//!
//! The tests are FLIPPED, not deleted. The knowledge the overlay carried is still recorded and
//! still displayed, so what was worth pinning is still pinned — from the surface that replaced it.

use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::mapper::Mapper;
use mapper::matrix::{classify, MatrixCell};
use ratatui::{buffer::Buffer, layout::Rect};

use app::render::map::render_map;
use app::state::AppState;

/// Two rooms, walked north from #1 to #2, plus a west move from #2 that went nowhere.
fn fixture() -> Mapper {
    let mut m = Mapper::default();
    m.observe(1, "Hall", None);
    m.observe(2, "Cave", Some(Direction::N));
    m.observe(2, "Cave", Some(Direction::W)); // west bounced off a wall
    m
}

fn render(g: &MapGraph) -> String {
    let rm = mapper::render::render(g);
    let mut st = AppState::default();
    st.scroll = rm.bounds.0;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &st, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ").to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// There is no longer any state that turns `?` marks on, so the drawn map never carries them —
/// and the box outline the overlay used to trade away for a corner rose is always intact.
#[test]
fn the_drawn_map_no_longer_marks_untried_directions() {
    let m = fixture();
    let text = render(&m.graph);
    assert!(!text.contains('?'), "no `?` marks under any setting: the overlay is gone");
    assert!(text.contains('╭'), "and the box outline is never traded for a corner rose");
    // The walked passage still draws exactly as it did.
    assert!(text.contains(AppState::default().symbols.arrows.north), "the N passage keeps its arrowhead");
}

/// The command that drove it is out of the registry; the two that replaced it are in.
#[test]
fn the_toggle_command_is_gone_and_the_view_commands_took_its_place() {
    assert!(app::slash::find_command("toggle-untried-exits").is_none());
    assert!(app::slash::find_command("view-map").is_some());
    assert!(app::slash::find_command("mark-maze-layer").is_some());
}

/// The DATA is untouched — it is what feeds `_` and `·`. This is the same assertion the old
/// `a_direction_that_bounced_off_a_wall_is_not_offered_again` made, read off the new surface.
#[test]
fn a_direction_that_bounced_off_a_wall_still_reads_differently_from_one_never_tried() {
    let m = fixture();
    assert_eq!(
        classify(&m.graph, 2, Direction::W),
        MatrixCell::Probed,
        "west was tried and refused: `_`, not the frontier"
    );
    assert_eq!(classify(&m.graph, 2, Direction::E), MatrixCell::Untried, "east was never typed: `·`");
    assert_eq!(
        classify(&m.graph, 2, Direction::S),
        MatrixCell::Untried,
        "nor south — the walked N edge's reciprocal is never assumed"
    );
    // And the cell says where it goes, which the `?` never could.
    assert_eq!(classify(&m.graph, 1, Direction::N), MatrixCell::OneWay { dest: 2 });
}
