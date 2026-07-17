use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    NeedLine,
    Quit,
}

#[derive(Debug, Clone)]
pub enum Input {
    Line(String),
}

pub struct Vm {
    pub(crate) db: Database,
    pub(crate) item_loc: Vec<i32>, // per item: current location (room index; -1/255 = carried; 0 = nowhere)
    pub(crate) player: usize,      // current room
    pub(crate) flags: [bool; 32],
    pub(crate) counters: [i32; 16],
    pub(crate) cur_counter: usize,
    #[allow(dead_code)] // used in later task
    pub(crate) saved_rooms: [usize; 16],
    #[allow(dead_code)] // used in later task
    pub(crate) lamp: i32, // remaining light; -1 = infinite
    pub(crate) out: String, // pending transcript
    pub(crate) quit: bool,
    pub(crate) needs_line: bool,
    #[allow(dead_code)] // used in later task
    pub(crate) last_noun: String, // for print-noun opcodes (later task)
    #[allow(dead_code)] // used in later task
    pub(crate) rng_state: u32, // xorshift PRNG state (used by later task; init to a fixed nonzero)
    #[allow(dead_code)] // used in later task
    pub(crate) pending_line: Option<String>, // buffered command (later task consumes)
}

impl Vm {
    /// Initialize: item_loc from each item's start_loc, player=start_room, lamp=light_time,
    /// flags/counters cleared, cur_counter=0, saved_rooms=[start_room? or 0], needs_line=true.
    /// Do NOT describe the room yet (room description is a later task); leave `out` empty.
    pub fn new(db: Database) -> Vm {
        let item_loc = db.items.iter().map(|i| i.start_loc).collect();
        let player = db.start_room;
        let lamp = db.light_time;
        Vm {
            db,
            item_loc,
            player,
            flags: [false; 32],
            counters: [0; 16],
            cur_counter: 0,
            saved_rooms: [0; 16],
            lamp,
            out: String::new(),
            quit: false,
            needs_line: true,
            last_noun: String::new(),
            rng_state: 0x1234_5678,
            pending_line: None,
        }
    }

    // --- accessors used by later tasks + the host adapter ---
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }
    pub fn current_room(&self) -> usize {
        self.player
    }
    pub fn room_name(&self, r: usize) -> &str {
        &self.db.rooms[r].desc
    }
    pub fn has_quit(&self) -> bool {
        self.quit
    }

    // --- minimal step/supply stubs (FULL turn logic is a later task) ---
    /// For now: if a line is pending, clear it and return Continue; else if not quit, return NeedLine.
    pub fn step(&mut self) -> StepResult {
        if self.quit {
            return StepResult::Quit;
        }
        if self.pending_line.take().is_some() {
            StepResult::Continue
        } else {
            self.needs_line = true;
            StepResult::NeedLine
        }
    }

    /// Buffer the command; clear needs_line.
    pub fn supply_line(&mut self, line: &str) {
        self.pending_line = Some(line.to_string());
        self.needs_line = false;
    }

    // --- THIS TASK'S FOCUS ---
    // eval_condition (and the field/helper reads it drives) is only exercised by
    // tests until the turn loop (later task) calls it; allow(dead_code) keeps
    // `cargo clippy` (without --tests) clean in the meantime.
    #[allow(dead_code)]
    pub(crate) fn eval_condition(&self, c: &Condition) -> bool {
        let value = c.value as usize;
        match c.code {
            0 => true,
            1 => self.item_carried(value),
            2 => self.item_in_room(value),
            3 => self.item_present(value),
            4 => self.player == value,
            5 => !self.item_in_room(value),
            6 => !self.item_carried(value),
            7 => self.player != value,
            8 => self.flag_get(value),
            9 => !self.flag_get(value),
            10 => (0..self.item_loc.len()).any(|i| self.item_carried(i)),
            11 => !(0..self.item_loc.len()).any(|i| self.item_carried(i)),
            12 => !self.item_present(value),
            13 => self.item_in_play(value),
            14 => !self.item_in_play(value),
            15 => self.counters[self.cur_counter] <= c.value as i32,
            16 => self.counters[self.cur_counter] > c.value as i32,
            19 => self.counters[self.cur_counter] == c.value as i32,
            17 => self.item_at_start(value),
            18 => !self.item_at_start(value),
            _ => false,
        }
    }

    fn item_loc_of(&self, idx: usize) -> Option<i32> {
        self.item_loc.get(idx).copied()
    }

    fn item_carried(&self, idx: usize) -> bool {
        matches!(self.item_loc_of(idx), Some(-1) | Some(255))
    }

    fn item_in_room(&self, idx: usize) -> bool {
        self.item_loc_of(idx) == Some(self.player as i32)
    }

    fn item_present(&self, idx: usize) -> bool {
        self.item_carried(idx) || self.item_in_room(idx)
    }

    fn item_in_play(&self, idx: usize) -> bool {
        matches!(self.item_loc_of(idx), Some(loc) if loc != 0)
    }

    fn item_at_start(&self, idx: usize) -> bool {
        match (self.item_loc_of(idx), self.db.items.get(idx)) {
            (Some(loc), Some(item)) => loc == item.start_loc,
            _ => false,
        }
    }

    fn flag_get(&self, idx: usize) -> bool {
        self.flags.get(idx).copied().unwrap_or(false)
    }

    // --- test-only helpers (or make fields pub(crate) and set directly) ---
    #[cfg(test)]
    pub(crate) fn set_player(&mut self, r: usize) {
        self.player = r;
    }
    #[cfg(test)]
    pub(crate) fn set_flag(&mut self, i: usize, v: bool) {
        self.flags[i] = v;
    }
    #[cfg(test)]
    pub(crate) fn set_counter(&mut self, v: i32) {
        self.counters[self.cur_counter] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Condition, Database, Item, Room};
    fn vm_with(items: Vec<Item>, rooms: Vec<Room>, player: usize) -> Vm {
        let db = Database {
            max_carry: 6,
            start_room: player,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs: vec!["".into()],
            nouns: vec!["".into()],
            rooms,
            messages: vec!["".into()],
            items,
        };
        let mut vm = Vm::new(db);
        vm.set_player(player);
        vm
    }
    #[test]
    fn cond_carried_and_present() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "sword".into(),
                treasure: false,
                auto_noun: None,
                start_loc: -1,
            }, // carried
            Item {
                text: "rock".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 2,
            }, // in room 2
        ];
        let rooms = vec![
            Room {
                exits: [0; 6],
                desc: "limbo".into(),
                literal: true,
            },
            Room {
                exits: [0; 6],
                desc: "r1".into(),
                literal: true,
            },
            Room {
                exits: [0; 6],
                desc: "r2".into(),
                literal: true,
            },
        ];
        let vm = vm_with(items, rooms, 2);
        assert!(vm.eval_condition(&Condition { code: 1, value: 1 })); // sword carried
        assert!(vm.eval_condition(&Condition { code: 2, value: 2 })); // rock in room
        assert!(vm.eval_condition(&Condition { code: 3, value: 1 })); // sword present
        assert!(vm.eval_condition(&Condition { code: 6, value: 2 })); // rock not carried
        assert!(vm.eval_condition(&Condition { code: 4, value: 2 })); // player in room 2
        assert!(vm.eval_condition(&Condition { code: 0, value: 99 })); // param always true
        assert!(vm.eval_condition(&Condition { code: 5, value: 1 })); // sword NOT in room 2 (it's carried)
        assert!(vm.eval_condition(&Condition { code: 13, value: 2 })); // rock in play
        assert!(vm.eval_condition(&Condition { code: 17, value: 2 })); // rock still in initial room 2
    }
    #[test]
    fn cond_flags_and_counters() {
        let mut vm = vm_with(
            vec![Item {
                text: "x".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            }],
            vec![Room {
                exits: [0; 6],
                desc: "r".into(),
                literal: true,
            }],
            0,
        );
        vm.set_flag(8, true);
        assert!(vm.eval_condition(&Condition { code: 8, value: 8 }));
        assert!(vm.eval_condition(&Condition { code: 9, value: 7 })); // flag 7 clear
        vm.set_counter(5);
        assert!(vm.eval_condition(&Condition { code: 15, value: 5 })); // counter <= 5
        assert!(vm.eval_condition(&Condition { code: 19, value: 5 })); // counter == 5
        assert!(vm.eval_condition(&Condition { code: 16, value: 4 })); // counter > 4
    }
}
