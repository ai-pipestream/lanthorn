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
        _ => None,
    }
}

/// True for the four intercardinal directions (NE/NW/SE/SW).
pub fn is_diagonal(d: Direction) -> bool {
    matches!(d, Direction::NE | Direction::NW | Direction::SE | Direction::SW)
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
}
