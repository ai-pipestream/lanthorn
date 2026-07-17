#[derive(Debug, Clone, PartialEq)]
pub struct Database {
    pub max_carry: i32,
    pub start_room: usize,
    pub num_treasures: i32,
    pub word_length: usize,
    pub light_time: i32, // -1 = infinite
    pub treasure_room: usize,
    pub actions: Vec<Action>,
    pub verbs: Vec<String>, // index 0 = placeholder
    pub nouns: Vec<String>,
    pub rooms: Vec<Room>,
    pub messages: Vec<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub exits: [usize; 6], // order: N,S,E,W,Up,Down
    pub desc: String,
    pub literal: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub text: String,
    pub treasure: bool,
    pub auto_noun: Option<String>,
    pub start_loc: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub verb: u16,
    pub noun: u16,
    pub conditions: [Condition; 5],
    pub commands: [u16; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Condition {
    pub code: u8,
    pub value: u16,
}

pub const LIGHT_SOURCE: usize = 9;
pub const DARK_FLAG: usize = 15;
pub const LAMP_EMPTY_FLAG: usize = 16;
pub const CARRIED: i32 = -1;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_constructs_and_indexes_from_zero() {
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 1,
            word_length: 3,
            light_time: 125,
            treasure_room: 1,
            actions: vec![],
            verbs: vec!["".into(), "GO".into()],
            nouns: vec!["".into(), "NORTH".into()],
            rooms: vec![
                Room {
                    exits: [0; 6],
                    desc: "limbo".into(),
                    literal: true,
                },
                Room {
                    exits: [0, 0, 0, 0, 0, 0],
                    desc: "dark forest".into(),
                    literal: false,
                },
            ],
            messages: vec!["".into()],
            items: vec![Item {
                text: "*gold*".into(),
                treasure: true,
                auto_noun: None,
                start_loc: 1,
            }],
        };
        assert_eq!(db.rooms.len(), 2);
        assert_eq!(db.start_room, 1);
        assert!(db.items[0].treasure);
        assert_eq!(DARK_FLAG, 15);
    }
}
