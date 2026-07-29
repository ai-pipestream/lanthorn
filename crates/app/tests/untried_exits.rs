//! SQ-0391: the optional "where haven't I been?" overlay.

use mapper::direction::Direction;
use mapper::graph::MapGraph;
use mapper::mapper::Mapper;
use ratatui::{buffer::Buffer, layout::Rect};

use app::render::map::render_map;
use app::state::AppState;

/// Two rooms, walked north from #1 to #2, plus an east move from #1 that went nowhere.
fn fixture() -> Mapper {
    let mut m = Mapper::default();
    m.observe(1, "Hall", None);
    m.observe(2, "Cave", Some(Direction::N));
    m.observe(2, "Cave", Some(Direction::W)); // west bounced off a wall
    m
}

fn render_with(overlay: bool, g: &MapGraph) -> String {
    let rm = mapper::render::render(g);
    let mut st = AppState::default();
    st.show_untried_exits = overlay;
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

#[test]
fn the_overlay_is_off_until_asked_for() {
    let m = fixture();
    assert!(!AppState::default().show_untried_exits, "off by default — it is an aid, not decor");
    assert!(
        !render_with(false, &m.graph).contains('?'),
        "no marks at all until toggle-untried-exits turns it on"
    );
}

#[test]
fn a_direction_that_bounced_off_a_wall_is_not_offered_again() {
    let m = fixture();
    // #2's untried set is the cardinals minus W (typed, went nowhere) and minus S (the walked
    // N edge's reciprocal is NOT assumed — only what was actually typed or walked counts).
    let untried = m.graph.untried(2);
    assert!(!untried.contains(&Direction::W), "west was tried and refused: stop offering it");
    assert!(untried.contains(&Direction::E), "east was never typed here");
    assert!(untried.contains(&Direction::N), "nor north");
}

#[test]
fn marks_cover_the_whole_rose_and_never_replace_a_real_exit() {
    let m = fixture();
    let text = render_with(true, &m.graph);
    assert!(text.contains('?'), "the overlay marks what is untried");
    // #1 walked north, so its top border carries the connector's arrowhead, not a `?`.
    let arrows = AppState::default().symbols.arrows;
    assert!(text.contains(arrows.north), "the walked N passage keeps its arrowhead");
    // The vertical pair rides beside N and S as lower-case letters, so an unexplored staircase
    // reads differently from an unexplored compass direction at a glance.
    assert!(text.contains('u'), "an unexplored way up is marked");
    assert!(text.contains('d'), "and an unexplored way down");
    // The diagonals sit ON the corners: the overlay is transient, so trading the outline for a
    // full rose is the deliberate bargain (the box comes straight back when you toggle it off).
    assert!(
        !render_with(true, &m.graph).lines().any(|l| l.contains('╭')),
        "with the overlay ON the corners carry diagonal marks"
    );
    assert!(
        render_with(false, &m.graph).contains('╭'),
        "and the outline returns intact the moment it is off"
    );
}
