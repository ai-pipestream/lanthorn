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

    fn flag_set(&mut self, idx: usize, v: bool) {
        if let Some(f) = self.flags.get_mut(idx) {
            *f = v;
        }
    }

    fn set_item_loc(&mut self, idx: usize, loc: i32) {
        if let Some(l) = self.item_loc.get_mut(idx) {
            *l = loc;
        }
    }

    fn carried_count(&self) -> i32 {
        (0..self.item_loc.len())
            .filter(|&i| self.item_carried(i))
            .count() as i32
    }

    /// Execute the 4 command opcodes of one action, pulling operands in order from
    /// `params`. Returns true if a `continue` (73) was executed.
    // Only exercised by tests until the turn loop (later task) calls it.
    #[allow(dead_code)]
    fn run_commands(&mut self, cmds: &[u16; 4], params: &[u16]) -> bool {
        let mut p = params.iter().copied();
        let mut did_continue = false;
        for &n in cmds {
            match n {
                0 => {}
                1..=51 => self.print_message(n as usize),
                102..=149 => self.print_message(n as usize - 50),
                52 => {
                    let item = p.next().unwrap_or(0) as usize;
                    if self.carried_count() < self.db.max_carry {
                        self.set_item_loc(item, CARRIED);
                    } else {
                        self.out.push_str("You are carrying too much.\n");
                    }
                }
                53 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, self.player as i32);
                }
                54 => {
                    let room = p.next().unwrap_or(0) as usize;
                    if room < self.db.rooms.len() {
                        self.player = room;
                    }
                }
                55 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, 0);
                }
                56 => self.flag_set(DARK_FLAG, true),
                57 => self.flag_set(DARK_FLAG, false),
                58 => {
                    let flag = p.next().unwrap_or(0) as usize;
                    self.flag_set(flag, true);
                }
                59 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, 0);
                }
                60 => {
                    let flag = p.next().unwrap_or(0) as usize;
                    self.flag_set(flag, false);
                }
                61 => {
                    self.out.push_str("You have died.\n");
                    self.flag_set(DARK_FLAG, false);
                    if let Some(last) = self.db.rooms.len().checked_sub(1) {
                        self.player = last;
                    }
                }
                62 => {
                    let item = p.next().unwrap_or(0) as usize;
                    let room = p.next().unwrap_or(0) as usize;
                    if room < self.db.rooms.len() {
                        self.set_item_loc(item, room as i32);
                    }
                }
                63 => self.quit = true,
                64 => self.describe_room(),
                65 => self.print_score(),
                66 => self.print_inventory(),
                67 => self.flag_set(0, true),
                68 => self.flag_set(0, false),
                69 => {
                    self.lamp = self.db.light_time;
                    self.flag_set(LAMP_EMPTY_FLAG, false);
                }
                70 => self.out.clear(),
                71 => { /* save game: host wires this later (v1 no-op) */ }
                72 => {
                    let a = p.next().unwrap_or(0) as usize;
                    let b = p.next().unwrap_or(0) as usize;
                    if let (Some(&la), Some(&lb)) = (self.item_loc.get(a), self.item_loc.get(b)) {
                        self.set_item_loc(a, lb);
                        self.set_item_loc(b, la);
                    }
                }
                73 => did_continue = true,
                74 => {
                    let item = p.next().unwrap_or(0) as usize;
                    self.set_item_loc(item, CARRIED);
                }
                75 => {
                    let a = p.next().unwrap_or(0) as usize;
                    let b = p.next().unwrap_or(0) as usize;
                    if let Some(&lb) = self.item_loc.get(b) {
                        self.set_item_loc(a, lb);
                    }
                }
                76 => self.describe_room(),
                77 => {
                    let c = &mut self.counters[self.cur_counter];
                    *c = (*c - 1).max(0);
                }
                78 => {
                    let v = self.counters[self.cur_counter];
                    self.out.push_str(&v.to_string());
                    self.out.push('\n');
                }
                79 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.counters[self.cur_counter] = v;
                }
                80 => {
                    std::mem::swap(&mut self.player, &mut self.saved_rooms[0]);
                }
                81 => {
                    let idx = p.next().unwrap_or(0) as usize;
                    self.cur_counter = idx.min(self.counters.len() - 1);
                }
                82 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.counters[self.cur_counter] += v;
                }
                83 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.counters[self.cur_counter] -= v;
                }
                84 => {
                    let noun = self.last_noun.clone();
                    self.out.push_str(&noun);
                }
                85 => {
                    let noun = self.last_noun.clone();
                    self.out.push_str(&noun);
                    self.out.push('\n');
                }
                86 => self.out.push('\n'),
                87 => {
                    let idx = (p.next().unwrap_or(0) as usize).min(self.saved_rooms.len() - 1);
                    std::mem::swap(&mut self.player, &mut self.saved_rooms[idx]);
                }
                88 => { /* pause: host handles timing */ }
                89 => { /* draw picture: no-op for text interpreter */ }
                _ => {} // 90..=101 unused
            }
        }
        did_continue
    }

    fn print_message(&mut self, n: usize) {
        if let Some(msg) = self.db.messages.get(n) {
            self.out.push_str(msg);
            self.out.push('\n');
        }
    }

    /// BASIC room description: room text + visible items. Light/darkness handling
    /// and detailed exit listing are a later task.
    fn describe_room(&mut self) {
        if let Some(room) = self.db.rooms.get(self.player) {
            self.out.push_str(&room.desc);
            self.out.push('\n');
        }
        let visible: Vec<&str> = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.item_in_room(*i))
            .map(|(_, it)| it.text.as_str())
            .collect();
        if !visible.is_empty() {
            self.out.push_str("You can see: ");
            self.out.push_str(&visible.join(", "));
            self.out.push('\n');
        }
    }

    fn print_inventory(&mut self) {
        let carried: Vec<&str> = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| self.item_carried(*i))
            .map(|(_, it)| it.text.as_str())
            .collect();
        if carried.is_empty() {
            self.out.push_str("You are carrying nothing.\n");
        } else {
            self.out.push_str("You are carrying: ");
            self.out.push_str(&carried.join(", "));
            self.out.push('\n');
        }
    }

    fn print_score(&mut self) {
        let total = self.db.num_treasures;
        let in_treasure_room = self
            .db
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.treasure)
            .filter(|(i, _)| self.item_loc_of(*i) == Some(self.db.treasure_room as i32))
            .count() as i32;
        self.out.push_str(&format!(
            "You have {in_treasure_room} out of {total} treasures.\n"
        ));
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
    #[cfg(test)]
    pub(crate) fn item_loc_at(&self, idx: usize) -> i32 {
        self.item_loc[idx]
    }
    #[cfg(test)]
    pub(crate) fn flag_at(&self, i: usize) -> bool {
        self.flags[i]
    }
    #[cfg(test)]
    pub(crate) fn counter_at(&self) -> i32 {
        self.counters[self.cur_counter]
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

    fn rooms4() -> Vec<Room> {
        (0..4)
            .map(|i| Room {
                exits: [0; 6],
                desc: format!("room{i}"),
                literal: true,
            })
            .collect()
    }

    #[test]
    fn cmd_get_drop_goto_flags_counters() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "item1".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 1,
            },
            Item {
                text: "item2".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 2,
            },
        ];
        let mut vm = vm_with(items, rooms4(), 1);

        // GET item1
        assert!(!vm.run_commands(&[52, 0, 0, 0], &[1]));
        assert_eq!(vm.item_loc_at(1), CARRIED);

        // DROP item1
        vm.run_commands(&[53, 0, 0, 0], &[1]);
        assert_eq!(vm.item_loc_at(1), vm.current_room() as i32);

        // GOTO room 3
        vm.run_commands(&[54, 0, 0, 0], &[3]);
        assert_eq!(vm.current_room(), 3);

        // set/clear flag 4
        vm.run_commands(&[58, 0, 0, 0], &[4]);
        assert!(vm.flag_at(4));
        vm.run_commands(&[60, 0, 0, 0], &[4]);
        assert!(!vm.flag_at(4));

        // counter set/add/sub
        vm.run_commands(&[79, 0, 0, 0], &[7]);
        assert_eq!(vm.counter_at(), 7);
        vm.run_commands(&[82, 0, 0, 0], &[2]);
        assert_eq!(vm.counter_at(), 9);
        vm.run_commands(&[83, 0, 0, 0], &[3]);
        assert_eq!(vm.counter_at(), 6);
    }

    #[test]
    fn cmd_message_ranges() {
        let mut messages = vec!["".to_string(); 53];
        messages[1] = "hello".into();
        messages[52] = "deep".into();
        let items = vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }];
        let db = Database {
            max_carry: 6,
            start_room: 0,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs: vec!["".into()],
            nouns: vec!["".into()],
            rooms: vec![Room {
                exits: [0; 6],
                desc: "r".into(),
                literal: true,
            }],
            messages,
            items,
        };
        let mut vm = Vm::new(db);

        vm.run_commands(&[1, 0, 0, 0], &[]);
        assert!(vm.take_output().contains("hello"));
        vm.run_commands(&[102, 0, 0, 0], &[]); // 102 -> message 52
        assert!(vm.take_output().contains("deep"));
    }

    #[test]
    fn cmd_continue_flag_and_quit() {
        let items = vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }];
        let mut vm = vm_with(items.clone(), rooms4(), 0);
        assert!(vm.run_commands(&[73, 0, 0, 0], &[]));

        let mut vm2 = vm_with(items, rooms4(), 0);
        vm2.run_commands(&[63, 0, 0, 0], &[]);
        assert!(vm2.has_quit());
    }
}
