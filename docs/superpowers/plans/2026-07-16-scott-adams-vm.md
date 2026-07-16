# Scott Adams (ScottFree) VM Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native, zero-dependency `scott` crate that loads and interprets classic Scott Adams adventures (ScottFree `.dat` text format), wired into the app as a third engine that renders through the existing `ScreenModel` and feeds the live automapper.

**Architecture:** New `crates/scott` owns the `.dat` loader + the interpreter and exposes a `step()/supply_line()` host loop plus a real room table (mirroring `zvm`/`gvm`, which are also zero-dep). A thin `crates/app/src/scott_session.rs` adapter implements the existing `Engine` trait, builds a Grid(header)+Buffer(text) `ScreenModel` directly (no Glk), and returns `TurnResult`s carrying a real `ObjectSnapshot` location.

**Tech Stack:** Rust (edition matching the workspace). `scott` crate: **std only, zero external deps** (no `ratatui`/`crossterm`/`serde`). App adapter: normal `app` crate deps.

## Global Constraints

- **`scott` crate is zero-dependency** — std only. No `serde`, no ratatui/crossterm. (Matches `zvm`/`gvm`.)
- **Clean-room:** implement from the format tables in this plan (sourced from ScottKit/Doherty format *descriptions* and the clean-room `sk2sadat` compiler). Do **not** copy or port ScottFree's GPL C source.
- **Count convention:** header fields NumItems/NumActions/NumWords/NumRooms/NumMessages are **highest index** (count − 1). Allocate `field + 1` slots; iterate `0..=field`. Index 0 is a real slot (room 0 = limbo).
- **Verify-don't-recall:** the four items flagged "VERIFY" in tasks below (command-operand sourcing, message offset −50, noun-0 occurrence handling, GO-movement precedence) must be confirmed against the golden fixture's expected output, not assumed.
- **No real game files committed** — Scott Adams' original `.dat` games remain under copyright. Tests use the hand-authored public-domain fixture built in Task 6. Real-game smokes (e.g. *Adventureland*) are a manual user step, not a committed test.
- **Themeable:** the adapter emits neutral packed colours (`0` = default) into `ScreenModel`; no hard-coded styles. Any new user-visible chrome must be style.toml-addressable (none expected in v1).
- **Mapper unchanged:** reuse `mapper::direction::{Direction, parse_direction}` (already has `Up`/`Down`) and `zvm::ObjectSnapshot` via `LocationInfo`. No mapper edits.

---

## Reference tables (implementation source of truth)

### File layout (token stream: whitespace-separated ints, `"`-delimited strings)
1. **Header** — 12 ints (see Task 2).
2. **Actions** — `(NumActions+1)` × 8 ints.
3. **Vocabulary** — `(NumWords+1)` verb/noun string pairs (interleaved: verb, noun, verb, noun…).
4. **Rooms** — `(NumRooms+1)` × (6 exit ints + 1 string).
5. **Messages** — `(NumMessages+1)` strings.
6. **Items** — `(NumItems+1)` × (1 string + 1 location int).
7. **Trailer** — `(NumActions+1)` comment strings, then version int, adventure-number int, checksum int.

### Header fields (order)
`0` bytes/unknown (write 32767, ignore on read) · `1` NumItems (hi-idx) · `2` NumActions (hi-idx) · `3` NumWords (hi-idx) · `4` NumRooms (hi-idx) · `5` MaxCarry · `6` PlayerRoom · `7` NumTreasures · `8` WordLength · `9` LightTime (−1 = infinite) · `10` NumMessages (hi-idx) · `11` TreasureRoom.

### Action entry (8 ints → 4 words)
- `word0 = 150*verb + noun` → `verb = w0/150`, `noun = w0%150`.
- `word1..word5` = 5 condition slots; each `code = w%20`, `value = w/20`.
- `word6 = 150*cmd0 + cmd1`, `word7 = 150*cmd2 + cmd3` → four command opcodes 0–149.

### Condition codes (`value = word/20`)
`0` param (always true; push `value` as command operand) · `1` item `v` carried · `2` item `v` in room w/ player · `3` item `v` present (carried or in room) · `4` player in room `v` · `5` item `v` NOT in room w/ player · `6` item `v` NOT carried · `7` player NOT in room `v` · `8` flag `v` set · `9` flag `v` clear · `10` inventory non-empty · `11` inventory empty · `12` item `v` not present · `13` item `v` in play (loc ≠ 0) · `14` item `v` not in play (loc = 0) · `15` counter ≤ `v` · `16` counter > `v` · `17` item `v` in initial room · `18` item `v` moved from initial · `19` counter = `v`. All non-param slots ANDed.

### Command opcodes (per command value `n`)
- `0` nothing · `1–51` print message `n` · `52–101` action (below) · `102–149` print message `n − 50`. **(VERIFY offset −50.)**
- Action opcodes (operands consumed in order from the code-0 param list — **VERIFY**):
  `52` GET item (respect MaxCarry) · `53` DROP item · `54` GOTO room · `55` item→room0 · `56` set flag15(dark) · `57` clear flag15 · `58` set flag `p` · `59` item→room0 · `60` clear flag `p` · `61` death (msg, clear flag15, →last room) · `62` PUT item,room · `63` game-over · `64` describe room · `65` score · `66` inventory · `67` set flag0 · `68` clear flag0 · `69` refill lamp (reset LightTime, clear flag15) · `70` clear screen · `71` save game · `72` swap loc of item,item · `73` continue → also run next action entry · `74` SUPERGET item (ignore MaxCarry) · `75` put item1 where item2 is · `76` look · `77` counter-- (floor 0) · `78` print counter · `79` counter = `p` · `80` swap player room ↔ saved-room reg · `81` select current counter = `p` · `82` counter += `p` · `83` counter -= `p` · `84` print noun · `85` print noun + nl · `86` newline · `87` swap player room ↔ saved-room reg `p` · `88` pause ~2s · `89` draw picture `p` (no-op in text) · `90–101` unused.

### Vocabulary / runtime constants
- Verb `1` = GO, verb `10` = GET, verb `18` = DROP. Nouns `1..6` = N,S,E,W,Up,Down (room-exit order). Synonym = word starting `*` (alias of prior non-`*` word in same column). Match on first `WordLength` chars.
- Item `9` = light source. Flag `15` = darkness. Flag `16` = set when lamp expires. Allocate 32 flags, 16 counters, 16 saved-room registers (counter 0 current at start).
- Item location `0` = nowhere/limbo; carried = `-1` (also `255` on some platforms — treat both as carried); else room index.
- Item description: leading `*` = treasure (the `*` IS printed); trailing `/NOUN/` = auto GET/DROP noun (stripped from display).
- Room description: leading `*` = print literally; else prefix "I'm in a ".

### Turn order (VERIFY GO precedence + noun-0 via fixture)
1. **Occurrence pass:** for each verb-0 action in table order whose conditions pass, roll d100 ≤ noun%; if win, run its commands.
2. Parse line → verb,noun (truncate `WordLength`, resolve synonyms). A bare direction ⇒ verb GO.
3. **Command pass:** first action with `verb==vb && (noun==no || noun==0)` and all conditions passing → run commands; `73` chains next entry. At most one (plus continuations).
4. If none matched: built-in **GO** movement (verb 1 + dir 1–6: dark check → exit lookup → move/describe or "I can't go that way."), else built-in **GET/DROP** via `/NOUN/`, else default "I don't understand your command."
5. Decrement lamp timer if item 9 in play; at 0 warn + set flag 16.

---

## File Structure

- `crates/scott/Cargo.toml` — zero-dep package `scott`.
- `crates/scott/src/lib.rs` — re-exports; `pub use` of `Database`, `Vm`, `StepResult`, `Input`.
- `crates/scott/src/database.rs` — `Database`, `Room`, `Item`, `Action`, `Condition`, `Word`, header fields.
- `crates/scott/src/loader.rs` — `Tokenizer` + `Database::parse(&str) -> Result<Database, LoadError>`.
- `crates/scott/src/vm.rs` — `Vm`, `StepResult`, `step`, `supply_line`, condition/command eval, turn loop, output buffer, `current_room`, save/restore serialization.
- `crates/app/src/scott_session.rs` — `ScottSession` implementing `Engine`.
- Edits: `Cargo.toml` (workspace members + app dep), `crates/app/src/hints.rs`, `picker.rs`, `startup.rs`, `engine_helpers.rs`, `crates/app/src/lib.rs` (module decl).

---

### Task 1: `scott` crate skeleton + Database model

**Files:**
- Create: `crates/scott/Cargo.toml`, `crates/scott/src/lib.rs`, `crates/scott/src/database.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces — Produces:**
```rust
// database.rs
pub struct Database {
    pub max_carry: i32,
    pub start_room: usize,
    pub num_treasures: i32,
    pub word_length: usize,
    pub light_time: i32,          // -1 = infinite
    pub treasure_room: usize,
    pub actions: Vec<Action>,
    pub verbs: Vec<String>,       // index 0 = placeholder
    pub nouns: Vec<String>,
    pub rooms: Vec<Room>,
    pub messages: Vec<String>,
    pub items: Vec<Item>,
}
pub struct Room { pub exits: [usize; 6], pub desc: String, pub literal: bool } // N,S,E,W,Up,Down
pub struct Item { pub text: String, pub treasure: bool, pub auto_noun: Option<String>, pub start_loc: i32 }
pub struct Action { pub verb: u16, pub noun: u16, pub conditions: [Condition; 5], pub commands: [u16; 4] }
pub struct Condition { pub code: u8, pub value: u16 }
pub const LIGHT_SOURCE: usize = 9;
pub const DARK_FLAG: usize = 15;
pub const LAMP_EMPTY_FLAG: usize = 16;
pub const CARRIED: i32 = -1;
```

- [ ] **Step 1: Write the failing test** — `crates/scott/src/database.rs` (bottom):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_constructs_and_indexes_from_zero() {
        let db = Database {
            max_carry: 6, start_room: 1, num_treasures: 1, word_length: 3,
            light_time: 125, treasure_room: 1,
            actions: vec![],
            verbs: vec!["".into(), "GO".into()],
            nouns: vec!["".into(), "NORTH".into()],
            rooms: vec![
                Room { exits: [0;6], desc: "limbo".into(), literal: true },
                Room { exits: [0,0,0,0,0,0], desc: "dark forest".into(), literal: false },
            ],
            messages: vec!["".into()],
            items: vec![Item { text: "*gold*".into(), treasure: true, auto_noun: None, start_loc: 1 }],
        };
        assert_eq!(db.rooms.len(), 2);
        assert_eq!(db.start_room, 1);
        assert!(db.items[0].treasure);
        assert_eq!(DARK_FLAG, 15);
    }
}
```

- [ ] **Step 2: Create the crate.** `crates/scott/Cargo.toml`:
```toml
[package]
name = "scott"
version = "0.1.0"
edition = "2021"

[dependencies]
```
Add `"crates/scott"` to the `members` array in the workspace root `Cargo.toml`.

- [ ] **Step 3: Write `database.rs`** with the structs/consts above (all fields `pub`, derive `Debug, Clone, PartialEq` on each struct/enum). `lib.rs`:
```rust
mod database;
mod loader;
mod vm;
pub use database::*;
pub use vm::{Input, StepResult, Vm};
```
(Comment out `mod loader;`/`mod vm;` and the `vm` re-export until Tasks 2–3 create them, or create empty stubs — but prefer creating stubs so the crate always builds.)

- [ ] **Step 4: Run** `cargo test -p scott database_constructs_and_indexes_from_zero` — Expected: PASS.

- [ ] **Step 5: Commit** — `git add crates/scott Cargo.toml && git commit` with message `feat(scott): crate skeleton + Database model` and the Quest/Co-Authored-By/Claude-Session trailers.

---

### Task 2: `.dat` loader

**Files:**
- Create/replace: `crates/scott/src/loader.rs`
- Test: same file (`#[cfg(test)]`)

**Interfaces — Consumes:** `Database` (Task 1). **Produces:**
```rust
pub enum LoadError { Truncated, BadInt(String), Unterminated }
impl Database { pub fn parse(src: &str) -> Result<Database, LoadError> { .. } }
/// Content sniff for engine detection: parse the 12-int header and sanity-check.
pub fn looks_like_scott(src: &str) -> bool { .. }
```

**Tokenizer:** iterate chars; skip whitespace; a token is either a run of `[0-9-]` (parse `i32`) or a `"`-delimited string (consume through the closing `"`; a string may span newlines). Provide `next_int() -> Result<i32, LoadError>` and `next_str() -> Result<String, LoadError>`.

**Parse order:** header (12 ints) → for `0..=NumActions` read 8 ints into `Action` (decode verb/noun/conditions/commands per the reference tables) → for `0..=NumWords` read verb then noun string (strip leading `*` = synonym; record alias by copying the prior non-`*` word) → for `0..=NumRooms` read 6 exit ints + desc string (leading `*` ⇒ `literal=true`, strip it) → for `0..=NumMessages` read string → for `0..=NumItems` read desc string + location int (parse leading `*` treasure, trailing `/NOUN/` auto-noun, uppercase the noun) → trailer may be ignored (stop parsing).

**`looks_like_scott`:** parse the first 12 ints; return true iff all parse and counts are sane (e.g. `NumItems`, `NumRooms`, `NumActions`, `NumMessages` each in `0..10000`, `MaxCarry` in `0..1000`, `WordLength` in `1..10`). Used by `extract_story` (Task 7).

- [ ] **Step 1: Write the failing tests** (hand-built minimal `.dat` literal string — two rooms, one item, one GO-north action, small vocab):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    const MINI: &str = r#"
32767 0 1 1 1 6 1 0 3 125 0 1
150 1 0 0 0 0 0 0
150 1
"" ""
"GO" "NORTH"
0 0 0 0 0 0 "*limbo"
2 0 0 0 0 0 "*forest clearing"
0 0 0 0 0 0 "*swamp"
""
"" ""
"*a brass lamp/LAMP/" 1
"action comment"
0
1
1
"#;
    #[test]
    fn parses_header_rooms_items() {
        let db = Database::parse(MINI).expect("parse");
        assert_eq!(db.start_room, 1);
        assert_eq!(db.light_time, 125);
        assert_eq!(db.rooms.len(), 2);          // NumRooms=1 -> 2 slots
        assert_eq!(db.rooms[1].desc, "forest clearing");
        assert!(db.rooms[1].literal);
        assert_eq!(db.rooms[1].exits[0], 2);    // north -> room 2? (adjust MINI to be consistent)
        assert_eq!(db.items.len(), 2);
        assert_eq!(db.items[1].auto_noun.as_deref(), Some("LAMP"));
        assert!(!db.items[1].text.contains('/'));
    }
    #[test]
    fn action_decodes_verb_noun() {
        let db = Database::parse(MINI).unwrap();
        assert_eq!(db.actions[0].verb, 1);
        assert_eq!(db.actions[0].noun, 0);
    }
    #[test]
    fn sniff_accepts_scott_rejects_garbage() {
        assert!(looks_like_scott(MINI));
        assert!(!looks_like_scott("This is a plain english sentence."));
        assert!(!looks_like_scott("\x01\x02\x03 not text"));
    }
}
```
*(Implementer: reconcile `MINI`'s counts so all `0..=N` loops consume exactly the tokens present — the numbers above are illustrative; make the fixture internally consistent, that is part of this task.)*

- [ ] **Step 2: Run** `cargo test -p scott loader::` — Expected: FAIL (parse not implemented).
- [ ] **Step 3: Implement** the tokenizer, `Database::parse`, and `looks_like_scott` per the spec above.
- [ ] **Step 4: Run** `cargo test -p scott` — Expected: PASS (all loader + Task 1 tests).
- [ ] **Step 5: Commit** — `feat(scott): ScottFree .dat loader + content sniff`.

---

### Task 3: VM state, StepResult, condition evaluation

**Files:**
- Create/replace: `crates/scott/src/vm.rs`

**Interfaces — Consumes:** `Database`. **Produces:**
```rust
pub enum StepResult { Continue, NeedLine, Quit }
pub enum Input { Line(String) }
pub struct Vm {
    db: Database,
    item_loc: Vec<i32>,          // per item current location
    player: usize,               // current room
    flags: [bool; 32],
    counters: [i32; 16],
    cur_counter: usize,
    saved_rooms: [usize; 16],
    lamp: i32,                    // remaining light; -1 infinite
    out: String,                 // pending transcript
    quit: bool,
    needs_line: bool,
    last_noun: String,           // for print-noun opcodes
}
impl Vm {
    pub fn new(db: Database) -> Vm { .. }     // init item_loc from start_loc, player=start_room, lamp=light_time, describe start room
    pub fn step(&mut self) -> StepResult { .. } // returns NeedLine until a command is supplied; runs occurrences then requests input
    pub fn supply_line(&mut self, line: &str) { .. } // Task 5 fills the turn; Task 3 stub: buffer the line
    pub fn take_output(&mut self) -> String { std::mem::take(&mut self.out) }
    pub fn current_room(&self) -> usize { self.player }
    pub fn room_name(&self, r: usize) -> &str { &self.db.rooms[r].desc }
    pub fn has_quit(&self) -> bool { self.quit }
    fn eval_condition(&self, c: &Condition) -> bool { .. }  // this task's focus
}
```

- [ ] **Step 1: Write failing tests** for `eval_condition` — drive a small `Vm` and assert each code:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Condition, Database, Item, Room};
    fn vm_with(items: Vec<Item>, rooms: Vec<Room>, player: usize) -> Vm {
        let db = Database { max_carry: 6, start_room: player, num_treasures: 0, word_length: 3,
            light_time: -1, treasure_room: 0, actions: vec![], verbs: vec!["".into()],
            nouns: vec!["".into()], rooms, messages: vec!["".into()], items };
        let mut vm = Vm::new(db);
        vm.set_player(player); // test helper
        vm
    }
    #[test]
    fn cond_carried_and_present() {
        let items = vec![
            Item{text:"limbo".into(),treasure:false,auto_noun:None,start_loc:0},
            Item{text:"sword".into(),treasure:false,auto_noun:None,start_loc:-1}, // carried
            Item{text:"rock".into(),treasure:false,auto_noun:None,start_loc:2},   // in room 2
        ];
        let rooms = vec![
            Room{exits:[0;6],desc:"limbo".into(),literal:true},
            Room{exits:[0;6],desc:"r1".into(),literal:true},
            Room{exits:[0;6],desc:"r2".into(),literal:true},
        ];
        let vm = vm_with(items, rooms, 2);
        assert!(vm.eval_condition(&Condition{code:1,value:1}));   // sword carried
        assert!(vm.eval_condition(&Condition{code:2,value:2}));   // rock in room
        assert!(vm.eval_condition(&Condition{code:3,value:1}));   // sword present
        assert!(vm.eval_condition(&Condition{code:6,value:2}));   // rock not carried
        assert!(vm.eval_condition(&Condition{code:4,value:2}));   // player in room 2
        assert!(vm.eval_condition(&Condition{code:0,value:99}));  // param always true
    }
    #[test]
    fn cond_flags_and_counters() {
        let mut vm = vm_with(vec![Item{text:"x".into(),treasure:false,auto_noun:None,start_loc:0}],
            vec![Room{exits:[0;6],desc:"r".into(),literal:true}], 0);
        vm.set_flag(8, true);
        assert!(vm.eval_condition(&Condition{code:8,value:8}));
        assert!(vm.eval_condition(&Condition{code:9,value:7}));   // flag 7 clear
        vm.set_counter(5);
        assert!(vm.eval_condition(&Condition{code:15,value:5}));  // counter <= 5
        assert!(vm.eval_condition(&Condition{code:19,value:5}));  // counter == 5
        assert!(vm.eval_condition(&Condition{code:16,value:4}));  // counter > 4
    }
}
```
(Add small `#[cfg(test)]` helpers `set_player/set_flag/set_counter` on `Vm`, or make fields `pub(crate)` and set directly.)

- [ ] **Step 2: Run** `cargo test -p scott vm::tests::cond` — Expected: FAIL.
- [ ] **Step 3: Implement** `Vm::new`, the accessors, and `eval_condition` covering all codes 0–19 per the reference table. Treat `255` and `-1` both as carried in the item-location checks.
- [ ] **Step 4: Run** the two tests — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(scott): VM state + condition evaluation`.

---

### Task 4: Command execution

**Files:** Modify `crates/scott/src/vm.rs`.

**Interfaces — Produces:**
```rust
impl Vm {
    /// Execute the 4 command opcodes of one action, pulling operands from `params`
    /// (the values of that action's code-0 conditions, in slot order).
    /// Returns true if a `continue` (73) requests chaining to the next action.
    fn run_commands(&mut self, cmds: &[u16;4], params: &[u16]) -> bool { .. }
    fn print_message(&mut self, n: usize) { self.out.push_str(&self.db.messages[n]); self.out.push('\n'); }
    fn describe_room(&mut self) { .. }   // honors darkness + light source; prints exits/items
    fn print_inventory(&mut self) { .. }
    fn print_score(&mut self) { .. }
}
```

**Implementation notes:** iterate the 4 command values; classify each with the opcode table (`0` skip; `1..=51` → `print_message(n)`; `102..=149` → `print_message(n-50)`; `52..=101` → action). For actions, consume operands from a `params` iterator (built by the caller from code-0 conditions). Implement every opcode 52–89 per the reference table; `90..=101` no-op. `73` sets a `continue` return flag. `63` sets `self.quit`. `71` (save) → for v1, emit a message and set a `pending_save` flag the adapter reads (host snapshot is the primary save path; the in-game SAVE just triggers a host save — wire in Task 8).

- [ ] **Step 1: Write failing tests** — one per representative opcode class:
```rust
#[test]
fn cmd_get_drop_goto_flags_counters() {
    let mut vm = /* 3 rooms, 2 items: item1 in room1, item2 in room2; player in room1 */;
    // GET item1 (param 1)
    assert_eq!(vm.run_commands(&[52,0,0,0], &[1]), false);
    assert_eq!(vm.item_loc_of(1), CARRIED);
    // DROP item1
    vm.run_commands(&[53,0,0,0], &[1]);
    assert_eq!(vm.item_loc_of(1), vm.current_room() as i32);
    // GOTO room 3
    vm.run_commands(&[54,0,0,0], &[3]);
    assert_eq!(vm.current_room(), 3);
    // set flag 4, clear flag 4
    vm.run_commands(&[58,0,0,0], &[4]); assert!(vm.flag(4));
    vm.run_commands(&[60,0,0,0], &[4]); assert!(!vm.flag(4));
    // counter: set 7, +2, -3, select/print
    vm.run_commands(&[79,0,0,0], &[7]); assert_eq!(vm.counter(), 7);
    vm.run_commands(&[82,0,0,0], &[2]); assert_eq!(vm.counter(), 9);
    vm.run_commands(&[83,0,0,0], &[3]); assert_eq!(vm.counter(), 6);
}
#[test]
fn cmd_message_ranges() {
    let mut vm = /* messages: index1="hello", index52 present */;
    vm.run_commands(&[1,0,0,0], &[]);   assert!(vm.take_output().contains("hello"));
    // 102 -> message 52
}
#[test]
fn cmd_continue_flag() {
    let mut vm = /* any */;
    assert_eq!(vm.run_commands(&[73,0,0,0], &[]), true);
}
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** `run_commands` + helpers for the full opcode table.
- [ ] **Step 4: Run** `cargo test -p scott vm::tests::cmd` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(scott): action-command execution (opcodes 52-89 + messages)`.

---

### Task 5: Turn loop (parse, occurrences, command pass, built-in movement, lamp)

**Files:** Modify `crates/scott/src/vm.rs`.

**Interfaces — Produces (finalizes):** `Vm::step`, `Vm::supply_line`. After `supply_line(cmd)`, the next `step()` runs a full turn and returns `NeedLine` (or `Quit`).

**Turn algorithm** (per the reference "Turn order"; the four VERIFY points are resolved here against the Task 6 fixture):
1. `supply_line` stores the raw command and clears the input-wait.
2. On `step`: parse into `(verb, noun)` — split on whitespace; match each word against verb/noun tables truncated to `word_length`, resolving synonyms; a lone recognized direction ⇒ `verb = GO(1)`, `noun = dir`.
3. **Command pass:** find first `Action` with `verb==vb && (noun==no || noun==0)` and all conditions passing. Build `params` from that action's code-0 condition values (in slot order). `run_commands`; if it returns `continue`, run the next table entry's commands too (repeat). Mark `handled`.
4. If not `handled`: if `vb==GO && (1..=6).contains(&no)` do **built-in movement** — if dark (flag15 set and item9 not accessible) print the dark-danger message; else `exit = rooms[player].exits[no-1]`; if `exit!=0` set player + describe, else "I can't go in that direction."; **(VERIFY precedence)**. Else if `vb==GET`/`DROP` and the noun maps to an item's `auto_noun`, do built-in get/drop (respect MaxCarry on get). Else print the default "I don't understand your command."
5. **Occurrence pass timing:** run verb-0 actions (roll d100 ≤ noun%; noun 0 ⇒ **VERIFY** treat as continuation-target only, not a random occurrence) — ScottFree runs these each turn; place per the reference order (before the command in step 3). Use a small deterministic PRNG seeded per-Vm (store `rng_state: u32`; xorshift) so golden tests are reproducible; expose `Vm::seed_rng(u32)` for tests.
6. Decrement lamp if `light_time != -1` and item9 in play; at 0, set flag16 + warn.
7. Re-describe / leave output in `self.out`; set `needs_line=true`.

- [ ] **Step 1: Write failing integration test** using the Task 6 fixture builder (create a `fn tiny_world() -> Database` test helper now, expand in Task 6):
```rust
#[test]
fn walk_north_moves_and_describes() {
    let mut vm = Vm::new(tiny_world());
    vm.seed_rng(1);
    let _ = vm.take_output();            // discard intro
    vm.supply_line("go north");
    assert_eq!(vm.step(), StepResult::NeedLine);
    let out = vm.take_output();
    assert!(out.contains("clearing"));   // room 2 desc
    assert_eq!(vm.current_room(), 2);
}
#[test]
fn get_lamp_then_inventory() {
    let mut vm = Vm::new(tiny_world());
    let _ = vm.take_output();
    vm.supply_line("get lamp"); vm.step();
    vm.supply_line("inventory"); vm.step();
    assert!(vm.take_output().to_lowercase().contains("lamp"));
}
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** the full turn loop + xorshift PRNG.
- [ ] **Step 4: Run** `cargo test -p scott` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(scott): turn loop (parse, occurrences, movement, lamp)`.

---

### Task 6: Golden-transcript fixture + save/restore serialization

**Files:** Modify `crates/scott/src/vm.rs` (+ a `tests/` integration file `crates/scott/tests/golden.rs`).

**Interfaces — Produces:**
```rust
impl Vm {
    pub fn snapshot(&self) -> Vec<u8> { .. }             // serialize item_loc, player, flags, counters, cur_counter, saved_rooms, lamp
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), ()> { .. }
}
```

**The fixture:** author a **complete, internally consistent, public-domain** `.dat` string ("Tiny Cave": ~4 rooms, a lamp (item 9), a treasure, a darkness room, one occurrence action, one scripted action with conditions+params, a treasure-room scoring path). Store as `crates/scott/tests/tiny_cave.dat` (committed — it's our own authored content). Drive a scripted command sequence and assert the exact concatenated transcript.

- [ ] **Step 1: Write the golden test** — `crates/scott/tests/golden.rs`: load `tiny_cave.dat`, seed RNG, run a fixed command list (`["look","get lamp","north","score", ...]`), and `assert_eq!(transcript, EXPECTED)`. Also a `snapshot`→mutate→`restore` round-trip test asserting state equality.
- [ ] **Step 2: Run** `cargo test -p scott --test golden` — Expected: FAIL.
- [ ] **Step 3: Author `tiny_cave.dat`** and implement `snapshot`/`restore`; iterate the fixture + EXPECTED until the transcript is correct and the four VERIFY behaviors are pinned. **This is where GO-precedence, param-sourcing, message-offset, and noun-0 are confirmed** — encode a fixture case for each.
- [ ] **Step 4: Run** `cargo test -p scott` — Expected: PASS (all).
- [ ] **Step 5: Commit** — `test(scott): golden transcript fixture + save/restore round-trip`.

---

### Task 7: Story detection & routing

**Files:** Modify `crates/app/src/hints.rs`.

**Interfaces — Consumes:** `scott::looks_like_scott` (Task 2). **Produces:** `LoadedStory::Scott(Vec<u8>)`.

- [ ] **Step 1: Write failing test** — `crates/app/src/hints.rs` tests:
```rust
#[test]
fn detects_scott_dat() {
    let dat = std::fs::read("../scott/tests/tiny_cave.dat").unwrap(); // or embed a const
    match extract_story(dat).unwrap() { LoadedStory::Scott(_) => {}, o => panic!("{o:?}") }
}
#[test]
fn zcode_still_defaults() {
    // arbitrary non-scott, non-glulx bytes still classify as ZCode
    assert!(matches!(extract_story(vec![3,0,0,0,0,0,0,0]).unwrap(), LoadedStory::ZCode(_)));
}
```
*(Implementer: prefer embedding a small valid scott string as a test const to avoid cross-crate path fragility.)*

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** — add `Scott(Vec<u8>)` to `LoadedStory`; update `bytes()`/`into_bytes()` match arms; in `extract_story`, **before** the final `Ok(LoadedStory::ZCode(bytes))` fall-through, add:
```rust
if let Ok(s) = std::str::from_utf8(&bytes) {
    if scott::looks_like_scott(s) { return Ok(LoadedStory::Scott(bytes)); }
}
```
Add `scott` to `app`'s `Cargo.toml` deps. Update `load_story_bytes` to reject `Scott` with a clear error (Z-only path). Fix any other exhaustive `match LoadedStory` the compiler flags (compiler-driven).

- [ ] **Step 4: Run** `cargo test -p app hints::` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(app): detect Scott Adams .dat stories`.

---

### Task 8: `ScottSession` — the `Engine` adapter

**Files:** Create `crates/app/src/scott_session.rs`; modify `crates/app/src/lib.rs` (add `pub mod scott_session;`).

**Interfaces — Consumes:** `scott::{Vm, Database, StepResult}`; the `Engine` trait + `ScreenModel`/`TurnResult` types (see the interface contract in the spec). **Produces:** `ScottSession` + `pub const SCOTT_ENGINE: &str = "scott"; pub const SCOTT_SAVE_FORMAT: u32 = 1;`.

**Construction:** `ScottSession::new(bytes: Vec<u8>) -> Result<ScottSession, scott::LoadError>` → `Database::parse(str)` → `Vm::new`, run to first `NeedLine`, buffer intro transcript.

**`Engine` methods** (implement all required):
- `submit(cmd)`: `vm.supply_line(cmd); vm.step()`; build `TurnResult` — `transcript = vm.take_output()`; `location = Some(ObjectSnapshot { number: vm.current_room() as u16, parent: 0, name: vm.room_name(vm.current_room()).to_string() })`; `location_method = None`; `quit = vm.has_quit()`; other fields default/empty (see `TurnResult` shape). Populate `transcript_runs`/`transcript_elems` empty.
- `submit_key(_)` → `None` (Scott is line-only).
- `take_transcript()` → drain a stored `String` (accumulate intro + per-turn if needed) — mirror the simplest engine; return `vm.take_output()` accumulation.
- `pending_input()` → `InputKind::Line`.
- `resume_save(_)` / `resume_restore(_)` → Scott issues no async save/restore requests; return an empty `TurnResult` (the in-game SAVE opcode is mapped to a host snapshot by the app, not an engine round-trip). Keep minimal.
- `has_quit()` → `vm.has_quit()`.
- `screen()` → build a Grid(header)+Buffer ScreenModel: put the room header (name + visible items + obvious exits) into a `GridWindow` (1–2 rows), and the scrolling text into a `BufferWindow { primary: true, .. }`. Follow `screen_model_from_machine` as the structural template (Pair{ vertical, Grid, Buffer }); use packed colour `0` and `None` per-window colours; `status: StatusModel::HostManaged`.
- `save_state()` → `EngineSave::new(SCOTT_ENGINE, SCOTT_SAVE_FORMAT, vm.snapshot())`.
- `restore_state(save)` → check `save.is_engine(SCOTT_ENGINE)` (else `EngineMismatch`), `vm.restore(&save.bytes)`.
- `restore_game_save(bytes)` → treat as a raw snapshot restore (or `BadSave` if not ours).
- `aux_data`/`set_aux_data`/`aux_dirty`/`clear_aux_dirty` → back with an empty `BTreeMap` field + `dirty: bool`.
- `current_location()` → `Some(ObjectSnapshot{ number, parent:0, name })` for the current room.
- `as_any`/`as_any_mut` → standard.

- [ ] **Step 1: Write failing test** — `crates/app/src/scott_session.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    fn dat() -> Vec<u8> { include_bytes!("../../scott/tests/tiny_cave.dat").to_vec() }
    #[test]
    fn boots_and_reports_location() {
        let mut s = ScottSession::new(dat()).unwrap();
        let loc = s.current_location().expect("loc");
        assert_eq!(loc.number, 1); // start room
        let r = s.submit("north");
        assert!(r.location.is_some());
        let m = s.screen();
        assert!(m.grid().is_some());
    }
    #[test]
    fn save_restore_roundtrip() {
        let mut s = ScottSession::new(dat()).unwrap();
        s.submit("north");
        let save = s.save_state();
        s.submit("south");
        s.restore_state(&save).unwrap();
        assert_eq!(s.current_location().unwrap().number, 2);
    }
}
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** `ScottSession` + all `Engine` methods; add the module to `lib.rs`.
- [ ] **Step 4: Run** `cargo test -p app scott_session::` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(app): ScottSession Engine adapter (native -> ScreenModel)`.

---

### Task 9: Picker integration

**Files:** Modify `crates/app/src/picker.rs`.

- [ ] **Step 1: Write failing test** — assert a Scott `.dat` resolves to a launchable picker entry tagged `Engine::Scott`:
```rust
#[test]
fn picker_tags_scott() {
    // build a LoadedStory::Scott from tiny_cave bytes, run the resolve path, expect Engine::Scott + launchable
}
```
*(Follow the existing picker test patterns; if resolution needs a file path, use a tempfile or the existing test harness.)*

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** — add `Engine::Scott` to the `enum`; add the validity probe arm (`LoadedStory::Scott(b) => scott::Database::parse(std::str::from_utf8(b).unwrap_or("")).is_ok()`); add the `LoadedStory::Scott => Engine::Scott` map arm; add the per-engine metadata arm (version/format strings — e.g. `format = "Scott Adams"`, other fields blank/None).
- [ ] **Step 4: Run** `cargo test -p app picker::` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(app): story picker recognizes Scott Adams engine`.

---

### Task 10: Construction/boxing + engine helpers

**Files:** Modify `crates/app/src/startup.rs`, `crates/app/src/engine_helpers.rs`.

- [ ] **Step 1: Write failing test** — `engine_helpers.rs`: `engine_tag` returns `"scott"` for a `ScottSession`:
```rust
#[test]
fn tag_scott() {
    let s = crate::scott_session::ScottSession::new(
        include_bytes!("../../scott/tests/tiny_cave.dat").to_vec()).unwrap();
    let boxed: Box<dyn crate::engine::Engine> = Box::new(s);
    assert_eq!(engine_tag(&*boxed), "scott");
}
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** — in `startup.rs` add the `app::hints::LoadedStory::Scott(bytes) => { match ScottSession::new(bytes) { Ok(s) => Box::new(s), Err(e) => { eprintln!("babelmap: cannot load Scott Adams story: {e:?}"); std::process::exit(1); } } }` arm. In `engine_helpers.rs` extend `engine_tag` to check `ScottSession` → `scott_session::SCOTT_ENGINE`, and add `scott_session_opt`/`_opt_mut` helpers mirroring the Glulx ones. Leave `engine_supports_save` as-is (host-snapshot save works via `save_state` for all engines; confirm the save/restore UI path uses `save_state`, not the Z-only guard — if the guard gates the Save-State command, extend it to accept Scott).
- [ ] **Step 4: Run** `cargo test -p app` — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(app): construct + tag the Scott Adams engine`.

---

### Task 11: End-to-end + mapper smoke

**Files:** `crates/app/tests/` (new integration test) or extend an existing app test module.

- [ ] **Step 1: Write failing test** — construct a `Mapper`, drive `ScottSession` through `["north","east","south"]`, calling `apply_turn(&mut mapper, cmd, &result)` each turn (mirroring `finish_command_turn`), and assert the mapper graph gained the expected rooms with correct directional edges (e.g. room1 --N--> room2).
```rust
#[test]
fn scott_walk_builds_map() {
    let mut s = crate::scott_session::ScottSession::new(dat()).unwrap();
    let mut mapper = mapper::mapper::Mapper::default();
    for cmd in ["north","south"] {
        let r = s.submit(cmd);
        crate::session::apply_turn(&mut mapper, cmd, &r);
    }
    assert!(mapper.graph.rooms().count() >= 2);
}
```

- [ ] **Step 2: Run** — Expected: FAIL (or reveal a wiring gap).
- [ ] **Step 3: Fix** any integration gap so the walk produces a correct room graph with directional edges.
- [ ] **Step 4: Run** `cargo build --tests && cargo test -p app scott` — Expected: PASS. Then full `cargo test` — Expected: no regressions.
- [ ] **Step 5: Commit** — `test(app): Scott Adams end-to-end + mapper smoke`.

---

## Post-plan (not tasks)

- **Manual smoke (user):** load a real Scott Adams `.dat` (e.g. *Adventureland*) in the TUI, walk a few rooms, confirm the map builds and text renders. Add to the to-verify list.
- **README:** mention Scott Adams support under major features (defer to the README-refresh quest).
- **Deferred:** SAGA graphics, binary databases, Blorb-wrapped Scott, `glk` crate extraction — all out of scope per the spec.

## Self-review notes

- Spec coverage: loader (T2), interpreter conditions/commands/turn (T3–T5), golden + save (T6), detection (T7), adapter (T8), picker (T9), construction (T10), mapper (T11) — all spec sections covered.
- The four format ambiguities are pinned to the golden fixture (T6), not assumed.
- Type consistency: `ObjectSnapshot{number,parent,name}`, `EngineSave::new(engine,format,bytes)`, `Direction`/`parse_direction` reused — all names taken verbatim from the extracted interface contract.
