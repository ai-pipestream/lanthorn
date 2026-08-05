#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Direction {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Up,
    Down,
    In,
    Out,
    Unknown,
}

pub fn parse_direction(cmd: &str) -> Option<Direction> {
    let lower = cmd.trim().to_lowercase();
    let mut tokens = lower.split_whitespace();
    let first = tokens.next()?;

    let word = if first == "go" {
        tokens.next()?
    } else {
        first
    };

    match word {
        "n" | "north" => Some(Direction::N),
        "s" | "south" => Some(Direction::S),
        "e" | "east" => Some(Direction::E),
        "w" | "west" => Some(Direction::W),
        "ne" | "northeast" => Some(Direction::NE),
        "nw" | "northwest" => Some(Direction::NW),
        "se" | "southeast" => Some(Direction::SE),
        "sw" | "southwest" => Some(Direction::SW),
        "u" | "up" => Some(Direction::Up),
        "d" | "down" => Some(Direction::Down),
        "in" | "inside" | "enter" => Some(Direction::In),
        "out" | "outside" | "exit" => Some(Direction::Out),
        // Ship directions (Seastalker et al.): the bow/front points north.
        "fore" | "forward" | "bow" => Some(Direction::N),
        "aft" | "stern" => Some(Direction::S),
        "port" => Some(Direction::W),
        "starboard" => Some(Direction::E),
        _ => None,
    }
}

/// True for the four intercardinal directions (NE/NW/SE/SW).
pub fn is_diagonal(d: Direction) -> bool {
    matches!(d, Direction::NE | Direction::NW | Direction::SE | Direction::SW)
}

/// The directions a room can be asked "have you tried this way?" about (SQ-0391): all eight
/// compass points plus Up and Down.
///
/// In/Out are excluded — they are not part of the rose a player scans, and a game that has them
/// usually has one obvious way in.
pub const UNTRIED_DIRS: [Direction; 10] = [
    Direction::N,
    Direction::NE,
    Direction::E,
    Direction::SE,
    Direction::S,
    Direction::SW,
    Direction::W,
    Direction::NW,
    Direction::Up,
    Direction::Down,
];

/// A direction's short tag, lower case: `n`, `ne`, `u`, `d`, `i`, `o` (SQ-0666). Uppercased it is
/// the matrix view's column header; as-is it is the `⇠w` return suffix inside a cell.
pub fn short_label(d: Direction) -> &'static str {
    match d {
        Direction::N => "n",
        Direction::S => "s",
        Direction::E => "e",
        Direction::W => "w",
        Direction::NE => "ne",
        Direction::NW => "nw",
        Direction::SE => "se",
        Direction::SW => "sw",
        Direction::Up => "u",
        Direction::Down => "d",
        Direction::In => "i",
        Direction::Out => "o",
        Direction::Unknown => "?",
    }
}

pub fn grid_offset(d: Direction) -> Option<(i32, i32)> {
    match d {
        Direction::N => Some((0, -1)),
        Direction::S => Some((0, 1)),
        Direction::E => Some((1, 0)),
        Direction::W => Some((-1, 0)),
        Direction::NE => Some((1, -1)),
        Direction::NW => Some((-1, -1)),
        Direction::SE => Some((1, 1)),
        Direction::SW => Some((-1, 1)),
        Direction::Up | Direction::Down | Direction::In | Direction::Out | Direction::Unknown => {
            None
        }
    }
}

/// Layout-only directional offset: like [`grid_offset`], but Up/Down also carry a
/// vertical N/S offset (Up → north, Down → south). Used ONLY by the layout,
/// placement, and directional-scoring code so up/down lay out like N/S. Rendering,
/// routing, layer-cutting, and `mark_distorted` keep using `grid_offset` (which
/// returns None for Up/Down), so up/down still draw as dotted portal stubs and are
/// never marked distorted.
pub fn layout_offset(d: Direction) -> Option<(i32, i32)> {
    match d {
        Direction::Up => Some((0, -1)),
        Direction::Down => Some((0, 1)),
        _ => grid_offset(d),
    }
}

pub fn opposite(d: Direction) -> Direction {
    match d {
        Direction::N => Direction::S,
        Direction::S => Direction::N,
        Direction::E => Direction::W,
        Direction::W => Direction::E,
        Direction::NE => Direction::SW,
        Direction::SW => Direction::NE,
        Direction::NW => Direction::SE,
        Direction::SE => Direction::NW,
        Direction::Up => Direction::Down,
        Direction::Down => Direction::Up,
        Direction::In => Direction::Out,
        Direction::Out => Direction::In,
        Direction::Unknown => Direction::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compass_and_long_forms() {
        assert_eq!(parse_direction("n"), Some(Direction::N));
        assert_eq!(parse_direction("North"), Some(Direction::N));
        assert_eq!(parse_direction("go se"), Some(Direction::SE));
        assert_eq!(parse_direction("enter"), Some(Direction::In));
        assert_eq!(parse_direction("up"), Some(Direction::Up));
        assert_eq!(parse_direction("xyzzy"), None);
        assert_eq!(parse_direction("take lamp"), None);
    }

    #[test]
    fn parses_ship_directions() {
        // Nautical terms map onto compass directions (front of ship = north).
        assert_eq!(parse_direction("fore"), Some(Direction::N));
        assert_eq!(parse_direction("forward"), Some(Direction::N));
        assert_eq!(parse_direction("bow"), Some(Direction::N));
        assert_eq!(parse_direction("aft"), Some(Direction::S));
        assert_eq!(parse_direction("stern"), Some(Direction::S));
        assert_eq!(parse_direction("port"), Some(Direction::W));
        assert_eq!(parse_direction("starboard"), Some(Direction::E));
        // Still works after "go" and case-insensitively.
        assert_eq!(parse_direction("go Starboard"), Some(Direction::E));
    }

    #[test]
    fn offsets_and_opposites() {
        assert_eq!(grid_offset(Direction::N), Some((0, -1)));
        assert_eq!(grid_offset(Direction::SE), Some((1, 1)));
        assert_eq!(grid_offset(Direction::Up), None);
        assert_eq!(opposite(Direction::N), Direction::S);
        assert_eq!(opposite(Direction::NE), Direction::SW);
        assert_eq!(opposite(Direction::In), Direction::Out);
    }

    #[test]
    fn is_diagonal_only_for_intercardinals() {
        assert!(is_diagonal(Direction::NE));
        assert!(is_diagonal(Direction::NW));
        assert!(is_diagonal(Direction::SE));
        assert!(is_diagonal(Direction::SW));
        assert!(!is_diagonal(Direction::N));
        assert!(!is_diagonal(Direction::E));
        assert!(!is_diagonal(Direction::Up));
    }

    #[test]
    fn layout_offset_maps_updown_to_ns_but_grid_offset_stays_none() {
        use super::{grid_offset, layout_offset, Direction};
        // Up/Down get an N/S layout offset...
        assert_eq!(layout_offset(Direction::Up), Some((0, -1)));
        assert_eq!(layout_offset(Direction::Down), Some((0, 1)));
        // ...but grid_offset is untouched (still None) — rendering/layers rely on this.
        assert_eq!(grid_offset(Direction::Up), None);
        assert_eq!(grid_offset(Direction::Down), None);
        // Compass delegates to grid_offset.
        assert_eq!(layout_offset(Direction::N), grid_offset(Direction::N));
        assert_eq!(layout_offset(Direction::E), grid_offset(Direction::E));
        // In/Out/Unknown remain None.
        assert_eq!(layout_offset(Direction::In), None);
        assert_eq!(layout_offset(Direction::Unknown), None);
    }
}
