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
    pub(crate) current_counter: i32, // the live counter all counter ops/conditions act on
    pub(crate) counters: [i32; 16],  // backup registers, swapped via op 81
    pub(crate) saved_room: usize,    // op-80 register (swapped with `player`)
    pub(crate) saved_rooms: [usize; 16], // op-87 registers
    pub(crate) lamp: i32, // remaining light; -1 = infinite
    pub(crate) out: String, // pending transcript
    pub(crate) quit: bool,
    pub(crate) needs_line: bool,
    pub(crate) last_noun: String, // for print-noun opcodes
    pub(crate) rng_state: u32, // xorshift PRNG state
    pub(crate) pending_line: Option<String>, // buffered command
}

impl Vm {
    /// Initialize: item_loc from each item's start_loc, player=start_room, lamp=light_time,
    /// flags/counters cleared, current_counter=0, saved_room/saved_rooms=0, needs_line=true.
    /// Describes the starting room into `out` so the host can read the intro via
    /// `take_output()` before driving input.
    pub fn new(db: Database) -> Vm {
        let item_loc = db.items.iter().map(|i| i.start_loc).collect();
        let player = db.start_room;
        let lamp = db.light_time;
        let mut vm = Vm {
            db,
            item_loc,
            player,
            flags: [false; 32],
            current_counter: 0,
            counters: [0; 16],
            saved_room: 0,
            saved_rooms: [0; 16],
            lamp,
            out: String::new(),
            quit: false,
            needs_line: true,
            last_noun: String::new(),
            rng_state: 0x1234_5678,
            pending_line: None,
        };
        // Run the opening occurrence pass before the first prompt, so auto-events
        // that fire at game start (e.g. "A Voice BOOOMS out:") are shown on load —
        // matching ScottFree/Gargoyle, where occurrences run at the top of the turn
        // loop rather than only when a command is entered.
        vm.run_occurrences();
        vm.needs_line = true;
        vm
    }

    // --- accessors used by later tasks + the host adapter ---
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }
    pub fn current_room(&self) -> usize {
        self.player
    }
    pub fn room_name(&self, r: usize) -> &str {
        self.db.rooms.get(r).map(|room| room.desc.as_str()).unwrap_or("")
    }
    pub fn has_quit(&self) -> bool {
        self.quit
    }
    /// Current location of item `idx` (-1/255 = carried, 0 = nowhere, else room index).
    pub fn item_loc(&self, idx: usize) -> i32 {
        self.item_loc.get(idx).copied().unwrap_or(0)
    }
    /// Whether flag `idx` is set.
    pub fn flag(&self, idx: usize) -> bool {
        self.flags.get(idx).copied().unwrap_or(false)
    }
    /// Value of the live current counter.
    pub fn counter(&self) -> i32 {
        self.current_counter
    }

    /// Serialize mutable game state: item locations, player room, flags,
    /// the live current counter, backup counters, the op-80 saved room, the
    /// op-87 saved-room registers, and lamp fuel. Manual little-endian byte
    /// encoding, zero-dep.
    pub fn snapshot(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(self.item_loc.len() as u32).to_le_bytes());
        for v in &self.item_loc {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&(self.player as u32).to_le_bytes());
        for &f in &self.flags {
            buf.push(f as u8);
        }
        buf.extend_from_slice(&self.current_counter.to_le_bytes());
        for c in &self.counters {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&(self.saved_room as u32).to_le_bytes());
        for &r in &self.saved_rooms {
            buf.extend_from_slice(&(r as u32).to_le_bytes());
        }
        buf.extend_from_slice(&self.lamp.to_le_bytes());
        buf
    }

    /// Restore state from `snapshot` bytes. Rejects short/malformed input, a
    /// mismatched item count, or an out-of-range room/counter index.
    #[allow(clippy::result_unit_err)]
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), ()> {
        fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, ()> {
            let end = pos.checked_add(4).ok_or(())?;
            let slice = bytes.get(*pos..end).ok_or(())?;
            *pos = end;
            Ok(u32::from_le_bytes(slice.try_into().unwrap()))
        }
        fn read_i32(bytes: &[u8], pos: &mut usize) -> Result<i32, ()> {
            read_u32(bytes, pos).map(|v| v as i32)
        }

        let mut pos = 0usize;
        let item_count = read_u32(bytes, &mut pos)? as usize;
        if item_count != self.item_loc.len() {
            return Err(());
        }
        let mut item_loc = Vec::with_capacity(item_count);
        for _ in 0..item_count {
            item_loc.push(read_i32(bytes, &mut pos)?);
        }
        let player = read_u32(bytes, &mut pos)? as usize;
        if player >= self.db.rooms.len() {
            return Err(());
        }
        let flags_slice = bytes.get(pos..pos + 32).ok_or(())?;
        let mut flags = [false; 32];
        for (i, &b) in flags_slice.iter().enumerate() {
            flags[i] = b != 0;
        }
        pos += 32;
        let current_counter = read_i32(bytes, &mut pos)?;
        let mut counters = [0i32; 16];
        for c in counters.iter_mut() {
            *c = read_i32(bytes, &mut pos)?;
        }
        let saved_room = read_u32(bytes, &mut pos)? as usize;
        if saved_room >= self.db.rooms.len() {
            return Err(());
        }
        let mut saved_rooms = [0usize; 16];
        for r in saved_rooms.iter_mut() {
            let v = read_u32(bytes, &mut pos)? as usize;
            if v >= self.db.rooms.len() {
                return Err(());
            }
            *r = v;
        }
        let lamp = read_i32(bytes, &mut pos)?;

        if pos != bytes.len() {
            return Err(());
        }

        self.item_loc = item_loc;
        self.player = player;
        self.flags = flags;
        self.current_counter = current_counter;
        self.counters = counters;
        self.saved_room = saved_room;
        self.saved_rooms = saved_rooms;
        self.lamp = lamp;
        Ok(())
    }

    /// Run one turn if a line is buffered, then request the next.
    pub fn step(&mut self) -> StepResult {
        if self.quit {
            return StepResult::Quit;
        }
        if let Some(cmd) = self.pending_line.take() {
            self.run_turn(&cmd);
            if self.quit {
                return StepResult::Quit;
            }
        }
        StepResult::NeedLine
    }

    /// Buffer the command; clear needs_line.
    pub fn supply_line(&mut self, line: &str) {
        self.pending_line = Some(line.to_string());
        self.needs_line = false;
    }

    /// Seed the deterministic PRNG (used by occurrence percentage rolls).
    pub fn seed_rng(&mut self, s: u32) {
        self.rng_state = s.max(1);
    }

    /// xorshift32 PRNG, deterministic given `rng_state`.
    fn next_rand(&mut self) -> u32 {
        let mut x = self.rng_state.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        x
    }

    fn roll_100(&mut self) -> u32 {
        self.next_rand() % 100
    }

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
            15 => self.current_counter <= c.value as i32,
            16 => self.current_counter > c.value as i32,
            19 => self.current_counter == c.value as i32,
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
                64 => { /* LOOK: the host redraws the room panel every frame */ }
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
                76 => { /* LOOK (redraw): the host room panel is always current */ }
                77 => {
                    self.current_counter = (self.current_counter - 1).max(-1);
                }
                78 => {
                    let v = self.current_counter;
                    self.out.push_str(&v.to_string());
                    self.out.push('\n');
                }
                79 => {
                    self.current_counter = p.next().unwrap_or(0) as i32;
                }
                80 => {
                    std::mem::swap(&mut self.player, &mut self.saved_room);
                }
                81 => {
                    // Swap the live current counter with backup register `param`.
                    let idx = (p.next().unwrap_or(0) as usize).min(self.counters.len() - 1);
                    std::mem::swap(&mut self.current_counter, &mut self.counters[idx]);
                }
                82 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.current_counter += v;
                }
                83 => {
                    let v = p.next().unwrap_or(0) as i32;
                    self.current_counter = (self.current_counter - v).max(-1);
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

    /// Run one full turn: parse, command match, built-in fallback, lamp, then the
    /// occurrence pass for the *next* prompt. ScottFree runs occurrences at the top
    /// of its loop, so they fire after the command that precedes them (seeing its
    /// effects); the opening pass runs in `Vm::new`.
    fn run_turn(&mut self, cmd: &str) {
        let mut w = cmd.split_whitespace();
        let w1 = w.next();
        let w2 = w.next();
        let mut vb = w1.and_then(|s| self.db.match_verb(s)).unwrap_or(0);
        let no = match w2 {
            Some(s) => self.db.match_noun(s).unwrap_or(0),
            None => w1.and_then(|s| self.db.match_noun(s)).unwrap_or(0),
        };
        if vb == 0 && (1..=6).contains(&no) {
            vb = 1;
        }
        self.last_noun = w2.or(w1).unwrap_or("").to_uppercase();

        let matched = self
            .db
            .actions
            .iter()
            .enumerate()
            .find(|(_, a)| {
                a.verb == vb
                    && (a.noun == no || a.noun == 0)
                    && a.conditions.iter().all(|c| self.eval_condition(c))
            })
            .map(|(i, _)| i);
        let mut handled = false;
        if let Some(idx) = matched {
            self.run_action_chain(idx);
            handled = true;
        }

        if !handled {
            if vb == 1 {
                if (1..=6).contains(&no) {
                    let dir = no as usize - 1;
                    if self.is_dark() {
                        self.out.push_str("It is dangerous to move in the dark!\n");
                    }
                    let dest = self
                        .db
                        .rooms
                        .get(self.player)
                        .map(|room| room.exits[dir])
                        .unwrap_or(0);
                    if dest != 0 {
                        self.player = dest;
                    } else {
                        self.out.push_str("I can't go in that direction.\n");
                    }
                } else {
                    self.out.push_str("I need a direction.\n");
                }
            } else if vb == 10 {
                if self.last_noun == "ALL" {
                    self.get_all();
                } else {
                    match self.find_auto_noun_item() {
                        Some(idx) if self.item_loc_of(idx) == Some(self.player as i32) => {
                            if self.carried_count() < self.db.max_carry {
                                self.set_item_loc(idx, CARRIED);
                                self.out.push_str("OK.\n");
                            } else {
                                self.out.push_str("You are carrying too much.\n");
                            }
                        }
                        Some(_) => self.out.push_str("It's beyond my power to do that.\n"),
                        None => self.out.push_str("I don't understand your command.\n"),
                    }
                }
            } else if vb == 18 {
                if self.last_noun == "ALL" {
                    self.drop_all();
                } else {
                    match self.find_auto_noun_item() {
                        Some(idx) if self.item_carried(idx) => {
                            self.set_item_loc(idx, self.player as i32);
                            self.out.push_str("OK.\n");
                        }
                        _ => self.out.push_str("I don't understand your command.\n"),
                    }
                }
            } else {
                self.out.push_str("I don't understand your command.\n");
            }
        }

        if self.db.light_time != -1 && self.item_in_play(LIGHT_SOURCE) && self.lamp > 0 {
            self.lamp -= 1;
            if self.lamp == 0 {
                self.flag_set(LAMP_EMPTY_FLAG, true);
                self.out.push_str("Your light has run out.\n");
            }
        }

        // Occurrences for the next prompt (ScottFree's top-of-loop pass), unless the
        // command ended the game.
        if !self.quit {
            self.run_occurrences();
        }

        self.needs_line = true;
    }

    /// GET ALL: take every item in the current room that has an auto-get noun,
    /// respecting MaxCarry. Reports each taken item; a full pack stops the sweep.
    fn get_all(&mut self) {
        let mut took_any = false;
        for i in 0..self.item_loc.len() {
            if self.item_in_room(i) && self.db.items[i].auto_noun.is_some() {
                if self.carried_count() < self.db.max_carry {
                    self.set_item_loc(i, CARRIED);
                    let text = self.db.items[i].text.clone();
                    self.out.push_str(&text);
                    self.out.push_str(": OK.\n");
                    took_any = true;
                } else {
                    self.out.push_str("You are carrying too much.\n");
                    return;
                }
            }
        }
        if !took_any {
            self.out.push_str("I don't understand your command.\n");
        }
    }

    /// DROP ALL: drop every carried item that has an auto-get noun.
    fn drop_all(&mut self) {
        let mut dropped_any = false;
        for i in 0..self.item_loc.len() {
            if self.item_carried(i) && self.db.items[i].auto_noun.is_some() {
                self.set_item_loc(i, self.player as i32);
                let text = self.db.items[i].text.clone();
                self.out.push_str(&text);
                self.out.push_str(": OK.\n");
                dropped_any = true;
            }
        }
        if !dropped_any {
            self.out.push_str("I don't understand your command.\n");
        }
    }

    /// Index of the item whose `auto_noun` matches the last typed noun, if any.
    fn find_auto_noun_item(&self) -> Option<usize> {
        // Scott matching is significant only to `word_length` characters, so the
        // typed noun and the item's auto-noun must be compared truncated — a full
        // typed word (e.g. "LAMP") still matches a shorter auto-noun ("LAM") when
        // word_length is 3. `last_noun` and `auto_noun` are already uppercase.
        let wl = self.db.word_length;
        let trunc = |s: &str| s.chars().take(wl).collect::<String>();
        let key = trunc(&self.last_noun);
        if key.is_empty() {
            return None;
        }
        self.db.items.iter().position(|it| {
            it.auto_noun.as_deref().map(|n| trunc(n) == key).unwrap_or(false)
        })
    }

    /// Occurrence pass: walk actions in order; for each verb==0 action, evaluate
    /// its conditions AT THAT POINT (so an earlier fired occurrence's state change
    /// is visible to a later guard), then fire it iff a d100 roll passes the
    /// noun-as-percent chance. A roll against 0 never passes, so noun==0 never
    /// fires standalone (those are continuation targets reached only via opcode 73).
    fn run_occurrences(&mut self) {
        for idx in 0..self.db.actions.len() {
            let (is_occ, noun, pass) = {
                let a = &self.db.actions[idx];
                let is_occ = a.verb == 0;
                let pass = is_occ && a.conditions.iter().all(|c| self.eval_condition(c));
                (is_occ, a.noun, pass)
            };
            if is_occ && pass && self.roll_100() < noun as u32 {
                self.run_action_chain(idx);
            }
        }
    }

    /// Run action `start`'s commands. If they executed `continue` (opcode 73),
    /// walk forward over CONSECUTIVE continuation lines (verb==0 && noun==0),
    /// running each one's commands only if its own conditions re-evaluate true.
    /// The walk stops at the first action whose vocab is not 0/0 (regardless of
    /// whether the intermediate lines themselves carry a 73).
    fn run_action_chain(&mut self, start: usize) {
        if !self.run_action_line(start) {
            return;
        }
        let mut idx = start + 1;
        while idx < self.db.actions.len() {
            let (is_cont, pass) = {
                let a = &self.db.actions[idx];
                let is_cont = a.verb == 0 && a.noun == 0;
                let pass = is_cont && a.conditions.iter().all(|c| self.eval_condition(c));
                (is_cont, pass)
            };
            if !is_cont {
                break;
            }
            if pass {
                self.run_action_line(idx);
            }
            idx += 1;
        }
    }

    /// Run one action line's commands, rebuilding its param list (the values of
    /// its code-0 conditions) fresh. Returns true if it executed `continue`.
    fn run_action_line(&mut self, idx: usize) -> bool {
        let (cmds, params) = {
            let a = &self.db.actions[idx];
            let params: Vec<u16> = a
                .conditions
                .iter()
                .filter(|c| c.code == 0)
                .map(|c| c.value)
                .collect();
            (a.commands, params)
        };
        self.run_commands(&cmds, &params)
    }

    fn print_message(&mut self, n: usize) {
        if let Some(msg) = self.db.messages.get(n) {
            self.out.push_str(msg);
            self.out.push('\n');
        }
    }

    /// True when the current room is unlit: the darkness flag is set and no light
    /// source (item 9) is either carried or present in the room.
    pub fn is_dark(&self) -> bool {
        self.flags[DARK_FLAG]
            && !(self.item_carried(LIGHT_SOURCE) || self.item_in_room(LIGHT_SOURCE))
    }

    /// The current room's display block, in the classic Scott Adams "top window"
    /// form: the room line, then "Obvious exits:", then "I can also see:". The host
    /// shows this as a persistent panel above the scrolling transcript (the CLI
    /// prints it inline), so it is a pure query and never touches `out`.
    ///
    /// A `*`-literal room prints verbatim; a non-literal room gets the "I'm in a "
    /// prefix. When the room is dark, only the darkness line is returned.
    pub fn room_block(&self) -> String {
        if self.is_dark() {
            return "It is too dark to see.".to_string();
        }
        let mut s = String::new();
        if let Some(room) = self.db.rooms.get(self.player) {
            if room.literal {
                s.push_str(&room.desc);
            } else {
                s.push_str("I'm in a ");
                s.push_str(&room.desc);
            }
            let names = ["North", "South", "East", "West", "Up", "Down"];
            let exits: Vec<&str> = room
                .exits
                .iter()
                .enumerate()
                .filter(|(_, &dest)| dest != 0)
                .map(|(i, _)| names[i])
                .collect();
            s.push_str("\n\nObvious exits: ");
            s.push_str(&if exits.is_empty() {
                "none".to_string()
            } else {
                format!("{}.", exits.join(". "))
            });
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
            s.push_str("\n\nI can also see:");
            for item in &visible {
                s.push_str("\n  ");
                s.push_str(item);
            }
        }
        s
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
        self.current_counter = v;
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
        self.current_counter
    }
    #[cfg(test)]
    pub(crate) fn saved_room_at(&self) -> usize {
        self.saved_room
    }
    #[cfg(test)]
    pub(crate) fn backup_counter_at(&self, i: usize) -> i32 {
        self.counters[i]
    }
    #[cfg(test)]
    pub(crate) fn saved_rooms_at(&self, i: usize) -> usize {
        self.saved_rooms[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Condition, Database, Item, Room};
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
            adventure_number: 0,
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

        // counter set/add/sub (all act on the single live current counter)
        vm.run_commands(&[79, 0, 0, 0], &[7]);
        assert_eq!(vm.counter_at(), 7);
        vm.run_commands(&[82, 0, 0, 0], &[2]);
        assert_eq!(vm.counter_at(), 9);
        vm.run_commands(&[83, 0, 0, 0], &[3]);
        assert_eq!(vm.counter_at(), 6);
    }

    fn one_item() -> Vec<Item> {
        vec![Item {
            text: "x".into(),
            treasure: false,
            auto_noun: None,
            start_loc: 0,
        }]
    }

    // CRIT-2: op 81 SWAPs the live current counter with a backup register; a
    // stash-change-restore round trip must recover the original value. Under the
    // old "select an index" model the final value would be 3, not 7.
    #[test]
    fn cmd_counter_swap_op81() {
        let mut vm = vm_with(one_item(), rooms4(), 0);
        vm.run_commands(&[79, 0, 0, 0], &[7]); // current = 7
        assert_eq!(vm.counter_at(), 7);
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap into backup 2: current<-0, backup2<-7
        assert_eq!(vm.counter_at(), 0);
        assert_eq!(vm.backup_counter_at(2), 7);
        vm.run_commands(&[79, 0, 0, 0], &[3]); // change current
        assert_eq!(vm.counter_at(), 3);
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap back: current<-7 (restored)
        assert_eq!(vm.counter_at(), 7);
        assert_eq!(vm.backup_counter_at(2), 3);
    }

    // CRIT-2 / MIN-7: ops 77 and 83 floor the current counter at -1.
    #[test]
    fn cmd_counter_floors_at_minus_one() {
        let mut vm = vm_with(one_item(), rooms4(), 0);
        vm.run_commands(&[79, 0, 0, 0], &[0]); // current = 0
        vm.run_commands(&[77, 0, 0, 0], &[]); // dec -> -1
        assert_eq!(vm.counter_at(), -1);
        vm.run_commands(&[77, 0, 0, 0], &[]); // dec again -> stays -1
        assert_eq!(vm.counter_at(), -1);
        vm.run_commands(&[79, 0, 0, 0], &[2]); // current = 2
        vm.run_commands(&[83, 0, 0, 0], &[10]); // sub 10 -> floor at -1
        assert_eq!(vm.counter_at(), -1);
    }

    // MIN-6: op 80 (saved_room scalar) and op 87 (saved_rooms array) are distinct
    // registers. Previously op 80 aliased saved_rooms[0]; here changing the array
    // must not disturb the op-80 register and vice-versa.
    #[test]
    fn cmd_room_registers_distinct_op80_op87() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        // Seed saved_rooms[0] = 2 via op 87, then reset player to 1.
        vm.set_player(2);
        vm.run_commands(&[87, 0, 0, 0], &[0]); // swap player(2) <-> saved_rooms[0](0)
        assert_eq!(vm.current_room(), 0);
        assert_eq!(vm.saved_rooms_at(0), 2);
        vm.set_player(1);
        // op 80 swaps player <-> the SEPARATE saved_room scalar (still 0), NOT array[0].
        vm.run_commands(&[80, 0, 0, 0], &[]);
        assert_eq!(vm.current_room(), 0); // would be 2 if op80 wrongly used saved_rooms[0]
        assert_eq!(vm.saved_room_at(), 1);
        assert_eq!(vm.saved_rooms_at(0), 2); // op80 left the op87 array untouched
    }

    // MIN-8: GET ALL / DROP ALL sweep every auto-noun item in the room / pack.
    #[test]
    fn get_all_and_drop_all() {
        let items = vec![
            Item {
                text: "limbo".into(),
                treasure: false,
                auto_noun: None,
                start_loc: 0,
            },
            Item {
                text: "a coin".into(),
                treasure: false,
                auto_noun: Some("COIN".into()),
                start_loc: 1,
            },
            Item {
                text: "a key".into(),
                treasure: false,
                auto_noun: Some("KEY".into()),
                start_loc: 1,
            },
        ];
        let mut verbs = vec![String::new(); 19];
        verbs[1] = "GO".into();
        verbs[10] = "GET".into();
        verbs[18] = "DROP".into();
        let db = Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs,
            nouns: vec![String::new(); 7], // no "ALL" in vocab -> matched via last_noun
            rooms: rooms4(),
            messages: vec![String::new()],
            items,
            adventure_number: 0,
        };
        let mut vm = Vm::new(db);
        vm.take_output();

        vm.supply_line("get all");
        vm.step();
        let out = vm.take_output();
        assert!(out.contains("a coin"), "get all names each item: {out:?}");
        assert!(out.contains("a key"), "get all names each item: {out:?}");
        assert_eq!(vm.item_loc_at(1), CARRIED);
        assert_eq!(vm.item_loc_at(2), CARRIED);

        vm.supply_line("drop all");
        vm.step();
        vm.take_output();
        assert_eq!(vm.item_loc_at(1), 1); // dropped back into room 1
        assert_eq!(vm.item_loc_at(2), 1);
    }

    // Snapshot/restore must round-trip the new current_counter scalar, the backup
    // registers, and the op-80 saved_room register.
    #[test]
    fn snapshot_restore_counter_and_room() {
        let mut vm = vm_with(one_item(), rooms4(), 1);
        vm.run_commands(&[79, 0, 0, 0], &[9]); // current = 9
        vm.run_commands(&[81, 0, 0, 0], &[2]); // swap -> current=0, backup2=9
        vm.run_commands(&[79, 0, 0, 0], &[5]); // current = 5
        vm.run_commands(&[80, 0, 0, 0], &[]); // player(1) <-> saved_room(0): player=0, saved_room=1
        assert_eq!(vm.current_room(), 0);
        assert_eq!(vm.saved_room_at(), 1);

        let snap = vm.snapshot();

        vm.run_commands(&[79, 0, 0, 0], &[99]); // current = 99
        vm.run_commands(&[80, 0, 0, 0], &[]); // player=1, saved_room=0
        assert_ne!(vm.counter_at(), 5);

        vm.restore(&snap).expect("restore");
        assert_eq!(vm.counter_at(), 5);
        assert_eq!(vm.backup_counter_at(2), 9);
        assert_eq!(vm.saved_room_at(), 1);
        assert_eq!(vm.current_room(), 0);
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
            adventure_number: 0,
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

    fn tiny_world() -> Database {
        let mut items = Vec::new();
        for i in 0..10 {
            if i == 9 {
                items.push(Item {
                    text: "lamp".into(),
                    treasure: false,
                    auto_noun: Some("LAMP".into()),
                    start_loc: 1,
                });
            } else {
                items.push(Item {
                    text: format!("filler{i}"),
                    treasure: false,
                    auto_noun: None,
                    start_loc: 0,
                });
            }
        }
        let mut verbs = vec![String::new(); 19];
        verbs[1] = "GO".into();
        verbs[10] = "GET".into();
        verbs[18] = "DROP".into();
        let mut nouns = vec![String::new(); 7];
        nouns[1] = "NORTH".into();
        nouns[2] = "SOUTH".into();
        nouns[3] = "EAST".into();
        nouns[4] = "WEST".into();
        nouns[5] = "UP".into();
        nouns[6] = "DOWN".into();
        let rooms = vec![
            Room {
                exits: [0; 6],
                desc: "limbo".into(),
                literal: true,
            },
            Room {
                exits: [2, 0, 0, 0, 0, 0],
                desc: "forest clearing".into(),
                literal: true,
            },
            Room {
                exits: [0, 1, 0, 0, 0, 0],
                desc: "damp cave".into(),
                literal: true,
            },
        ];
        Database {
            max_carry: 6,
            start_room: 1,
            num_treasures: 0,
            word_length: 3,
            light_time: -1,
            treasure_room: 0,
            actions: vec![],
            verbs,
            nouns,
            rooms,
            messages: vec![String::new()],
            items,
            adventure_number: 0,
        }
    }

    #[test]
    fn walk_north_moves_and_describes() {
        let mut vm = Vm::new(tiny_world());
        vm.seed_rng(1);
        vm.supply_line("go north");
        assert_eq!(vm.step(), StepResult::NeedLine);
        assert_eq!(vm.current_room(), 2);
        // The destination is described by the room panel (room_block), not the
        // scrolling transcript.
        let block = vm.room_block();
        assert!(block.to_lowercase().contains("cave"), "moved into the cave: {block:?}");
        assert!(block.contains("Obvious exits: South."), "exits listed: {block:?}");
    }

    #[test]
    fn room_block_prefix_exits_items_and_darkness() {
        let mut db = tiny_world();
        // A non-literal noun room with North + Down exits; the lamp (item 9) is here.
        db.rooms[1].literal = false;
        db.rooms[1].desc = "forest".into();
        db.rooms[1].exits = [2, 0, 0, 0, 0, 3];
        let mut vm = Vm::new(db);

        // Non-literal rooms get the "I'm in a " prefix; exits and items are
        // period-separated (C64 style).
        assert_eq!(
            vm.room_block(),
            "I'm in a forest\n\nObvious exits: North. Down.\n\nI can also see:\n  lamp"
        );

        // With the darkness flag set and no light source in play, only the dark
        // line shows — no room text, exits, or items.
        vm.set_item_loc(LIGHT_SOURCE, 0); // banish the lamp to nowhere
        vm.flag_set(DARK_FLAG, true);
        assert!(vm.is_dark());
        assert_eq!(vm.room_block(), "It is too dark to see.");
    }

    #[test]
    fn get_specific_item_matches_at_word_length() {
        let mut db = tiny_world(); // word_length 3
        // A real game stores auto-nouns at word-length significance: the lamp
        // (item 9, starting in room 1) carries the 3-char auto-noun "LAM".
        db.items[9].auto_noun = Some("LAM".into());
        let mut vm = Vm::new(db);
        // Typing the full word "lamp" must still match "LAM" (both truncate to 3).
        vm.supply_line("get lamp");
        vm.step();
        let out = vm.take_output();
        assert!(out.contains("OK."), "get lamp succeeds: {out:?}");
        assert!(vm.item_carried(9), "the lamp is now carried");

        // And dropping it by the full word works too.
        vm.supply_line("drop lamp");
        vm.step();
        assert!(!vm.item_carried(9), "the lamp was dropped");
    }

    #[test]
    fn opening_occurrence_fires_before_first_prompt() {
        let mut db = tiny_world();
        db.messages = vec![String::new(), "A Voice BOOOMS out: Beware!".into()];
        // An always-occurrence (verb 0, noun 100) printing message 1, no guard.
        db.actions.push(Action {
            verb: 0,
            noun: 100,
            conditions: [Condition { code: 0, value: 0 }; 5],
            commands: [1, 0, 0, 0],
        });
        // The opening message is available immediately, before any command.
        let mut vm = Vm::new(db);
        let intro = vm.take_output();
        assert!(
            intro.contains("A Voice BOOOMS out"),
            "opening occurrence shows on load: {intro:?}"
        );
    }

    #[test]
    fn bare_direction_moves() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("north");
        vm.step();
        assert_eq!(vm.current_room(), 2);
        vm.supply_line("south");
        vm.step();
        assert_eq!(vm.current_room(), 1);
    }

    #[test]
    fn get_lamp_then_inventory() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("get lamp");
        vm.step();
        vm.supply_line("inventory");
        vm.step(); // no INVENTORY verb in tiny_world -> "don't understand"; instead assert the item moved
        // Better: assert the lamp is now carried.
        assert_eq!(vm.item_loc_at(LIGHT_SOURCE), CARRIED);
    }

    #[test]
    fn blocked_direction_reports() {
        let mut vm = Vm::new(tiny_world());
        let _ = vm.take_output();
        vm.supply_line("east"); // room1 has no east exit
        vm.step();
        assert!(vm.take_output().to_lowercase().contains("can't go"));
        assert_eq!(vm.current_room(), 1);
    }
}
