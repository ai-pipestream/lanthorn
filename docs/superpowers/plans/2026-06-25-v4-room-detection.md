# v4+ Room Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This plan is executed by a background worktree agent: each task is self-contained with complete code and exact tests. Work serially in task order; each task ends green before the next.

**Goal:** Detect the current room for v4+ games (Infocom + Inform) by reading the status-line room name, validating the player-parent heuristic against it, and resolving to an object (or a name-only synthetic id); surface the detection method in a hideable map indicator.

**Architecture:** zvm gains a stateless `detect_location` returning a `Location` enum (`GlobalVar0|PlayerParent|StatusName|NameOnly`). The app converts that into the existing `TurnResult.location` (synthesizing a high-bit id for name-only rooms) plus a `location_method`, guards VM-by-id reads against synthetic ids, and renders the method in the map's bottom-right corner.

**Tech Stack:** Rust workspace (crates `zvm`, `app`); ratatui 0.29 / crossterm 0.28.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-25-v4-room-detection-design.md`.
- v3 (and below) detection unchanged: global variable 0 (`current_location`).
- `main.rs`'s ~10 `snap.number as RoomId` sites and the room-keying-by-object-number model stay unchanged; name-only rooms get a synthetic `ObjectSnapshot`.
- `RoomId = u16`, `ObjectSnapshot { number: u16, parent: u16, name: String }`.
- Synthetic id: `0x8000 | (fnv1a(normalize(name)) & 0x7FFF)`; high bit always set; never collides with real object numbers.
- Normalization (matching/hashing): trim, collapse internal whitespace to single spaces, lowercase.
- `status_name_matches`: equality OR `short` is a leading prefix of `candidate` ending on a word boundary (next char non-alphanumeric or end). `short` non-empty.
- Detection is stateless (pure function of the machine), re-run each turn.
- Indicator: bottom-right of the map pane, hidden by default (`show_loc_method=false`), descriptive labels, themeable via `loc_indicator`.
- Commit trailers (zsh — NO backticks in the commit body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Run suites with `cargo test -p <crate> <filter>`; keep `cargo build -p app` warning-free.

## Existing interfaces (verified — use exactly these)

- `zvm::objects::short_name(mem: &Memory, obj: u16) -> String`
- `zvm::objects::get_parent(mem: &Memory, obj: u16) -> u16` (0 = none)
- `zvm::objects::object_snapshot(mem: &Memory, obj: u16) -> ObjectSnapshot`
- `zvm::objects::get_child` / `get_sibling` (used by `list_room_objects`)
- `zvm::location::current_location(machine: &Machine) -> Option<ObjectSnapshot>` (v3 G0 path; keep)
- `zvm::location::object_tree_view(machine) -> Vec<ObjectSnapshot>`
- `max_object_number(mem: &Memory) -> u16` — private fn already in `location.rs` (same module as the new code).
- `Machine.mem: Memory` (pub), `Machine.screen: ScreenState` (pub).
- `ScreenState.upper: UpperWindow` (pub), `ScreenState.upper_window_rows: u16` (pub).
- `UpperWindow.cell(row: u16, col: u16) -> Cell`, `.cols: u16`, `.rows: u16`; `Cell.ch: char`.
- `Memory.version(&self) -> u8`.

---

### Task 1: zvm — `status_line_room_name` extraction

**Files:**
- Modify: `crates/zvm/src/location.rs` — add the extraction helper + private string helpers + tests.

**Interfaces:**
- Produces: `pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String>`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/zvm/src/location.rs`:

```rust
use crate::screen::UpperWindow;

fn upper_with(rows: &[&str]) -> UpperWindow {
    let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u16;
    let mut u = UpperWindow::default();
    u.resize(rows.len() as u16, cols.max(1));
    for (r, line) in rows.iter().enumerate() {
        for (c, ch) in line.chars().enumerate() {
            u.put((r + 1) as u16, (c + 1) as u16, ch, 0);
        }
    }
    u
}

#[test]
fn status_room_name_common_form_strips_score_and_posture() {
    let u = upper_with(&[" Bedroom, in the bed                              Score: 0     Moves: 1"]);
    assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Bedroom"));
}

#[test]
fn status_room_name_plain() {
    let u = upper_with(&[" Darkness                                         Score: 0     Moves: 0"]);
    assert_eq!(status_line_room_name(&u, 1).as_deref(), Some("Darkness"));
}

#[test]
fn status_room_name_location_label_form() {
    let u = upper_with(&[
        " Mode:  Communications Mode                                Time:  7:07pm",
        " Location:  Foo Bar                                        Date:  3/16/2031",
    ]);
    assert_eq!(status_line_room_name(&u, 2).as_deref(), Some("Foo Bar"));
}

#[test]
fn status_room_name_empty_grid_is_none() {
    let u = upper_with(&["                                "]);
    assert_eq!(status_line_room_name(&u, 1), None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm status_room_name 2>&1 | tail -15`
Expected: compile error (`status_line_room_name` not found).

- [ ] **Step 3: Implement**

Add to `crates/zvm/src/location.rs` (module scope, after the imports):

```rust
use crate::screen::UpperWindow;

/// Normalize for matching/hashing: trim, collapse whitespace, lowercase.
pub(crate) fn normalize_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Strip the posture suffix after the first comma, then trim.
fn clean_room_text(s: &str) -> String {
    s.split(',').next().unwrap_or(s).trim().to_string()
}

/// Extract a candidate room name from the v4+ status-line grid, or None.
///
/// Scans at most the first 2 active rows. Prefers a `Location:` label segment;
/// otherwise takes row 1's first segment (text before the first run of 2+
/// spaces, which separates the left-justified room name from the right-aligned
/// score/moves/time block). Strips a trailing posture suffix after a comma.
pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String> {
    let scan = active_rows.min(2).min(upper.rows);
    let row_text = |r: u16| -> String {
        let mut s = String::new();
        for c in 1..=upper.cols {
            s.push(upper.cell(r, c).ch);
        }
        s
    };

    // 1. Label form: any scanned row containing a "Location:" segment.
    for r in 1..=scan {
        let line = row_text(r);
        let lower = line.to_lowercase();
        if let Some(idx) = lower.find("location:") {
            let after = line[idx + "location:".len()..].trim_start();
            let value = after.split("  ").next().unwrap_or("").trim();
            let candidate = clean_room_text(value);
            if !candidate.is_empty() {
                return Some(candidate);
            }
        }
    }

    // 2. Common form: row 1's first segment (before the first 2+ space run).
    if scan >= 1 {
        let line = row_text(1);
        let first = line.split("  ").next().unwrap_or("").trim();
        let candidate = clean_room_text(first);
        if !candidate.is_empty() {
            return Some(candidate);
        }
    }

    None
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm status_room_name 2>&1 | tail -15` → all 4 pass.
Run: `cargo test -p zvm 2>&1 | tail -5` → full zvm suite green.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/location.rs
git commit -m "feat(zvm): status_line_room_name extracts the v4+ room name

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: zvm — matching, player, ancestor walk-up, resolution

**Files:**
- Modify: `crates/zvm/src/location.rs` — add `status_name_matches`, `find_player_object`, `nearest_matching_ancestor`, `resolve_room_object` + tests.

**Interfaces:**
- Consumes: `normalize_name` (Task 1), `max_object_number` (existing private), `short_name`, `get_parent`, `object_snapshot`.
- Produces: `pub fn status_name_matches(candidate: &str, short: &str) -> bool`; the other three are private to `location.rs`.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/zvm/src/location.rs` (reuse `build_v3_story`/`make_machine` already there; obj1="west", obj2="east", obj3="hall"):

```rust
#[test]
fn status_name_matches_rules() {
    assert!(status_name_matches("Bedroom", "Bedroom"));            // equal
    assert!(status_name_matches("Bedroom (messy)", "Bedroom"));    // trailing decoration
    assert!(status_name_matches("Bedroom, north end", "Bedroom")); // (post-strip safety net)
    assert!(!status_name_matches("Hallway", "Hall"));              // word-boundary guard
    assert!(status_name_matches("hall  ", "Hall"));                // case + whitespace
    assert!(!status_name_matches("Kitchen", "Bedroom"));          // unrelated
    assert!(!status_name_matches("Bedroom", ""));                 // empty short
}

#[test]
fn find_player_object_by_name() {
    // Rename obj3 to "yourself" so it is the player.
    let mut buf = build_v3_story();
    let name = z_name("yourself");
    buf[PROP3_TBL as usize] = (name.len() / 2) as u8;
    buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 1 + name.len()].copy_from_slice(&name);
    let machine = make_machine(buf);
    assert_eq!(find_player_object(&machine), Some(3));
}

#[test]
fn resolve_room_object_matches_short_name() {
    let machine = make_machine(build_v3_story()); // obj1 "west", obj2 "east", obj3 "hall"
    let r = resolve_room_object(&machine, "hall").expect("hall resolves");
    assert_eq!(r.number, 3);
    assert!(resolve_room_object(&machine, "nowhere").is_none());
}

#[test]
fn nearest_matching_ancestor_walks_up() {
    // obj tree: obj3 (parent obj2), obj2 (parent obj1), obj1 (parent 0).
    // Searching from obj3 for "west" should walk up to obj1.
    let machine = make_machine(build_v3_story());
    let r = nearest_matching_ancestor(&machine, 3, "west").expect("walks up to west");
    assert_eq!(r.number, 1);
    assert!(nearest_matching_ancestor(&machine, 3, "nowhere").is_none());
}
```

Note: `build_v3_story` sets obj2.parent=1, obj3.parent=1 (both children of obj1). For the walk-up test, obj3's parent is obj1 ("west") directly, so the nearest match is obj1 — the assertion holds. If the implementer finds obj3.parent=1 makes the walk trivial, that is fine; the test still verifies the walk returns obj1 and that a non-match returns None.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm -- status_name_matches find_player resolve_room nearest_matching 2>&1 | tail -15`
Expected: compile errors (functions not found).

- [ ] **Step 3: Implement**

Add to `crates/zvm/src/location.rs` (module scope):

```rust
use crate::objects::{get_parent, object_snapshot, short_name};

/// True if `short` names the room shown as `candidate`: equality, or `short` is
/// a leading prefix of `candidate` ending on a word boundary (next char
/// non-alphanumeric, or end of string). Both normalized; `short` non-empty.
pub fn status_name_matches(candidate: &str, short: &str) -> bool {
    let c = normalize_name(candidate);
    let s = normalize_name(short);
    if s.is_empty() {
        return false;
    }
    if c == s {
        return true;
    }
    match c.strip_prefix(&s) {
        Some(rest) => rest.chars().next().map_or(true, |ch| !ch.is_alphanumeric()),
        None => false,
    }
}

/// The current player object: the lowest-numbered object whose normalized short
/// name is one of {yourself, you, me, myself, self}. None if not found.
fn find_player_object(machine: &Machine) -> Option<u16> {
    const NAMES: [&str; 5] = ["yourself", "you", "me", "myself", "self"];
    let n = max_object_number(&machine.mem);
    (1..=n).find(|&obj| {
        let nm = normalize_name(&short_name(&machine.mem, obj));
        NAMES.contains(&nm.as_str())
    })
}

/// Nearest ancestor of `start` (exclusive) whose short name matches `name` via
/// `status_name_matches`. Depth-bounded (32) to tolerate cycles.
fn nearest_matching_ancestor(machine: &Machine, start: u16, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let mut cur = get_parent(mem, start);
    for _ in 0..32 {
        if cur == 0 {
            break;
        }
        if status_name_matches(name, &short_name(mem, cur)) {
            return Some(object_snapshot(mem, cur));
        }
        cur = get_parent(mem, cur);
    }
    None
}

/// The object whose short name matches `name` (longest match wins; ties -> lowest
/// number), or None.
fn resolve_room_object(machine: &Machine, name: &str) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    let mut best: Option<(usize, u16)> = None; // (normalized short-name length, object)
    for obj in 1..=n {
        let sn = short_name(mem, obj);
        if status_name_matches(name, &sn) {
            let len = normalize_name(&sn).len();
            if best.map_or(true, |(blen, _)| len > blen) {
                best = Some((len, obj));
            }
        }
    }
    best.map(|(_, obj)| object_snapshot(mem, obj))
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm -- status_name_matches find_player resolve_room nearest_matching 2>&1 | tail -15` → pass.
Run: `cargo test -p zvm 2>&1 | tail -5` → green.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/location.rs
git commit -m "feat(zvm): name matching, player detection, ancestor walk-up, resolution

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: zvm — `Location` enum + `LocationMethod` + `detect_location`

**Files:**
- Modify: `crates/zvm/src/location.rs` — add the enums + entry point + tests.
- Modify: `crates/zvm/src/lib.rs:14` — export the new public items.

**Interfaces:**
- Consumes: Tasks 1-2 helpers, `current_location` (v3).
- Produces: `pub enum LocationMethod`, `pub enum Location` (+ `object()`, `method()`), `pub fn detect_location(machine: &Machine) -> Option<Location>`.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/zvm/src/location.rs`:

```rust
#[test]
fn detect_location_v3_uses_global0() {
    let mut buf = build_v3_story();
    put_word(&mut buf, GLOBAL_VARS as usize, 1); // global 0 = obj 1
    let machine = make_machine(buf);
    match detect_location(&machine) {
        Some(Location::GlobalVar0(s)) => assert_eq!(s.number, 1),
        other => panic!("expected GlobalVar0, got {other:?}"),
    }
    assert_eq!(detect_location(&machine).unwrap().method(), LocationMethod::GlobalVar0);
}

#[test]
fn location_object_and_method_accessors() {
    let s = ObjectSnapshot { number: 5, parent: 0, name: "Hall".into() };
    assert_eq!(Location::StatusName(s.clone()).object().map(|o| o.number), Some(5));
    assert_eq!(Location::NameOnly("X".into()).object(), None);
    assert_eq!(Location::NameOnly("X".into()).method(), LocationMethod::NameOnly);
    assert_eq!(Location::PlayerParent(s).method(), LocationMethod::PlayerParent);
}
```

(The v4+ paths of `detect_location` are exercised end-to-end by the app/session tests in Task 5 with real stories; the zvm unit tests cover the v3 path and the accessors, plus the helper tests from Tasks 1-2.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm detect_location 2>&1 | tail -15`
Expected: compile error (`Location`/`detect_location` not found).

- [ ] **Step 3: Implement**

Add to `crates/zvm/src/location.rs` (module scope):

```rust
/// How the current room was determined (drives the map indicator label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationMethod {
    GlobalVar0,
    PlayerParent,
    StatusName,
    NameOnly,
}

/// The mapper-facing location signal for one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    GlobalVar0(ObjectSnapshot),
    PlayerParent(ObjectSnapshot),
    StatusName(ObjectSnapshot),
    NameOnly(String),
}

impl Location {
    /// The backing object snapshot, or None for a name-only room.
    pub fn object(&self) -> Option<&ObjectSnapshot> {
        match self {
            Location::GlobalVar0(s) | Location::PlayerParent(s) | Location::StatusName(s) => Some(s),
            Location::NameOnly(_) => None,
        }
    }
    /// The detection method tag.
    pub fn method(&self) -> LocationMethod {
        match self {
            Location::GlobalVar0(_) => LocationMethod::GlobalVar0,
            Location::PlayerParent(_) => LocationMethod::PlayerParent,
            Location::StatusName(_) => LocationMethod::StatusName,
            Location::NameOnly(_) => LocationMethod::NameOnly,
        }
    }
}

/// Best-effort current room, version-gated:
/// - v3 and below: global variable 0 -> GlobalVar0, or None.
/// - v4+: validated player-parent -> status-name -> name-only -> None.
///
/// Stateless: a pure function of the machine, re-run each turn.
pub fn detect_location(machine: &Machine) -> Option<Location> {
    if machine.mem.version() <= 3 {
        return current_location(machine).map(Location::GlobalVar0);
    }
    let name = status_line_room_name(&machine.screen.upper, machine.screen.upper_window_rows)?;
    if let Some(player) = find_player_object(machine) {
        if let Some(room) = nearest_matching_ancestor(machine, player, &name) {
            return Some(Location::PlayerParent(room));
        }
    }
    if let Some(obj) = resolve_room_object(machine, &name) {
        return Some(Location::StatusName(obj));
    }
    Some(Location::NameOnly(name))
}
```

Update `crates/zvm/src/lib.rs:14` from:

```rust
pub use location::{current_location, object_tree_view};
```
to:
```rust
pub use location::{current_location, detect_location, object_tree_view, Location, LocationMethod};
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm detect_location 2>&1 | tail -15` → pass.
Run: `cargo test -p zvm 2>&1 | tail -5` → green.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/location.rs crates/zvm/src/lib.rs
git commit -m "feat(zvm): detect_location returns Location {GlobalVar0|PlayerParent|StatusName|NameOnly}

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: app — synthetic room-id helpers

**Files:**
- Create: `crates/app/src/roomid.rs`.
- Modify: `crates/app/src/lib.rs` — add `pub mod roomid;`.

**Interfaces:**
- Produces: `pub const SYNTHETIC_ROOM_FLAG: u16`, `pub fn is_synthetic_room(id: u16) -> bool`, `pub fn synthetic_room_id(name: &str) -> u16`.

- [ ] **Step 1: Write the failing test**

Create `crates/app/src/roomid.rs`:

```rust
//! RoomId policy for name-only rooms (no backing Z-machine object).
//!
//! RoomIds with the high bit set are synthetic: derived from a room's displayed
//! name when it could not be resolved to a game object. The high bit guarantees
//! no collision with real object numbers (no IF game has >= 32768 objects).

/// Set on a RoomId to mark it a name-only (non-object) room.
pub const SYNTHETIC_ROOM_FLAG: u16 = 0x8000;

/// True when `id` denotes a name-only room (high bit set).
pub fn is_synthetic_room(id: u16) -> bool {
    id & SYNTHETIC_ROOM_FLAG != 0
}

/// Deterministic, save/reload-stable RoomId for a name-only room. Normalizes the
/// name (trim, collapse whitespace, lowercase) then FNV-1a hashes it into the
/// low 15 bits, with the high bit set.
pub fn synthetic_room_id(name: &str) -> u16 {
    let norm: String = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let mut h: u32 = 0x811c_9dc5;
    for b in norm.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    SYNTHETIC_ROOM_FLAG | (h as u16 & 0x7FFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_id_high_bit_set_and_deterministic() {
        let a = synthetic_room_id("Bedroom");
        assert_eq!(a & SYNTHETIC_ROOM_FLAG, SYNTHETIC_ROOM_FLAG, "high bit set");
        assert_eq!(a, synthetic_room_id("Bedroom"), "deterministic");
        assert!(is_synthetic_room(a));
        assert!(!is_synthetic_room(150)); // a real object number
    }

    #[test]
    fn synthetic_id_normalizes_name() {
        assert_eq!(synthetic_room_id("Bedroom"), synthetic_room_id("  bedroom  "));
        assert_eq!(synthetic_room_id("Foo Bar"), synthetic_room_id("foo   bar"));
    }

    #[test]
    fn synthetic_id_differs_for_distinct_names() {
        assert_ne!(synthetic_room_id("Bedroom"), synthetic_room_id("Kitchen"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Add `pub mod roomid;` to `crates/app/src/lib.rs` (alongside the other `pub mod` lines), then:
Run: `cargo test -p app roomid 2>&1 | tail -15`
Expected: tests run and pass once the module compiles — if you skipped the lib.rs line first, expect "unresolved module". (Add the line, then the tests should pass; this module is self-contained, so RED is just the missing-module state.)

- [ ] **Step 3: Implement**

Already written in Step 1. Ensure `crates/app/src/lib.rs` contains `pub mod roomid;`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app roomid 2>&1 | tail -15` → 3 pass.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/roomid.rs crates/app/src/lib.rs
git commit -m "feat(app): synthetic_room_id / is_synthetic_room for name-only rooms

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: app — session uses `detect_location`; `TurnResult.location_method`

**Files:**
- Modify: `crates/app/src/session.rs` — switch to `detect_location`, add `location_method`, synthesize name-only ids.
- Modify (compiler-driven): every other `TurnResult { .. }` literal to add `location_method: None` — `crates/app/src/main.rs` (×7), `crates/app/src/input.rs` (×2), `crates/app/tests/headless.rs` (×1), and any in-file session test literals.

**Interfaces:**
- Consumes: `zvm::detect_location`, `zvm::Location`, `zvm::LocationMethod`, `crate::roomid::synthetic_room_id`.
- Produces: `TurnResult.location_method: Option<zvm::LocationMethod>`; `TurnResult.location` now also set for name-only rooms (synthetic).

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/app/src/session.rs` (mirror the existing `turn_result_has_empty_sound_fields_by_default` construction):

```rust
#[test]
fn turn_result_carries_location_method_field() {
    // Build the same way the sibling submit tests do; the field just needs to exist
    // and default to a value. For a v3 fixture with global 0 set, method is GlobalVar0.
    let mut sess = /* same construction as sibling submit test */;
    let r = sess.submit("look");
    // The field exists and is an Option<LocationMethod>; on a v3 story with a
    // location it is Some(GlobalVar0), otherwise None — either is acceptable here.
    let _ = r.location_method;
}
```

If the sibling tests construct the session inline, copy that exact construction. This test exists mainly to lock the new field's presence and type; the real v4+ behavior is verified manually against Hitchhiker after the wave (see plan tail).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app turn_result_carries_location_method 2>&1 | tail -15`
Expected: compile error (`location_method` not a field).

- [ ] **Step 3: Implement**

In `crates/app/src/session.rs`:

Replace the import `use zvm::location::current_location;` with:
```rust
use zvm::location::{detect_location, Location, LocationMethod};
```

Add to `TurnResult` (after the `diagnostics` field):
```rust
    /// How the current room was detected this turn (drives the map indicator).
    pub location_method: Option<LocationMethod>,
```

In BOTH `submit` and `submit_char`, replace the line
`let location = current_location(&self.machine);`
with:
```rust
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(|loc| match loc {
            Location::NameOnly(name) => zvm::ObjectSnapshot {
                number: crate::roomid::synthetic_room_id(name),
                parent: 0,
                name: name.clone(),
            },
            _ => loc.object().expect("non-NameOnly variants carry an object").clone(),
        });
        let location_method = detected.as_ref().map(Location::method);
```

And add `location_method` to each method's returned `TurnResult { .. }` literal:
```rust
        TurnResult { transcript, location, quit, info, beep, diagnostics, location_method }
```

- [ ] **Step 4: Fix the other construction sites**

Build and let the compiler list every `TurnResult` literal missing the field; add `location_method: None` to each (these are synthetic/seed/restore results in `main.rs`, `input.rs`, `headless.rs`, and any session test literals):

Run: `cargo build -p app 2>&1 | grep -A3 "missing field" | head -40`
Add `location_method: None,` to each named site until it builds clean.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p app turn_result_carries_location_method 2>&1 | tail -10` → pass.
Run: `cargo test -p app 2>&1 | tail -5` → full app suite green; `cargo build -p app` → 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/main.rs crates/app/src/input.rs crates/app/tests/headless.rs
git commit -m "feat(app): session uses detect_location; TurnResult carries location_method

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: app — guard `list_room_objects` against synthetic ids

**Files:**
- Modify: `crates/app/src/render/room_info.rs` — guard the object-tree walk.

**Interfaces:**
- Consumes: `crate::roomid::is_synthetic_room`.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/app/src/render/room_info.rs`:

```rust
#[test]
fn list_room_objects_empty_for_synthetic_id() {
    // A synthetic RoomId (high bit set) must not read the object table.
    let buf = zvm::header::tests_support::sample_story(5);
    let mem = zvm::memory::Memory::new(buf).unwrap();
    let synth = crate::roomid::SYNTHETIC_ROOM_FLAG | 0x0123;
    assert!(super::list_room_objects(&mem, synth).is_empty());
}
```

(If `zvm::header::tests_support` / `zvm::memory::Memory` are not reachable from this test module, build the `Memory` the same way other app tests in this file do; the assertion is what matters: synthetic id → empty.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app list_room_objects_empty_for_synthetic 2>&1 | tail -15`
Expected: FAIL (the unguarded walk reads garbage and likely returns non-empty / or compile error if helper visibility needs adjusting — make `list_room_objects` reachable from the test, it is in the same module).

- [ ] **Step 3: Implement**

In `crates/app/src/render/room_info.rs`, add the guard as the first lines of `list_room_objects`:

```rust
fn list_room_objects(mem: &zvm::memory::Memory, room_id: RoomId) -> Vec<String> {
    // Name-only rooms have no backing object; never read the object table by a
    // synthetic id (it would be outside the table).
    if crate::roomid::is_synthetic_room(room_id) {
        return Vec::new();
    }
    use zvm::objects::{get_child, get_sibling, short_name};
    // ... existing body unchanged ...
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app list_room_objects 2>&1 | tail -10` → pass (the new test + any existing ones).
Run: `cargo test -p app 2>&1 | tail -5` → green.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/room_info.rs
git commit -m "fix(app): list_room_objects returns empty for synthetic room ids

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: app — state, config, command wiring for the indicator toggle

**Files:**
- Modify: `crates/app/src/state.rs` — `AppState.loc_method`, `AppState.show_loc_method` (+ defaults).
- Modify: `crates/app/src/config.rs` — `show_loc_method` field (default/merge/write/test).
- Modify: `crates/app/src/keymap.rs` — `Command::ToggleLocMethod` (+ action/name/display/context/list).
- Modify: `crates/app/src/input.rs` — `Action::ToggleLocMethod` (+ handler).
- Modify: `crates/app/src/main.rs` — apply config to state; set `loc_method` each turn.

**Interfaces:**
- Consumes: `zvm::LocationMethod`, `TurnResult.location_method` (Task 5), the existing `apply_turn_events(state, result)` helper.
- Produces: `AppState.loc_method`, `AppState.show_loc_method`, `Config.show_loc_method`, `Command::ToggleLocMethod`, `Action::ToggleLocMethod`.

This mirrors the `show_room_numbers` / `ToggleRoomNumbers` wiring exactly. Do NOT add a config-screen row (out of scope; the setting is controlled by the command and the config file).

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/app/src/config.rs`:

```rust
#[test]
fn config_show_loc_method_default_false_and_round_trips() {
    assert_eq!(Config::default().show_loc_method, false);
    let cfg: Config = toml::from_str("show_loc_method = true\n").unwrap();
    assert_eq!(cfg.show_loc_method, true);
}
```

Add to the tests module in `crates/app/src/input.rs` (near the `ToggleRoomNumbers` handling, mirroring any existing toggle test; if none, this minimal one):

```rust
#[test]
fn toggle_loc_method_flips_state() {
    let mut s = app::state::AppState::default();
    assert!(!s.show_loc_method);
    apply_action(Action::ToggleLocMethod, &mut s, &mut mapper::Mapper::default());
    assert!(s.show_loc_method);
}
```

(Match the real signature/visibility of `apply_action` used by sibling input tests; if it is `pub(crate)` and tests call it differently, follow the existing pattern in that file.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app show_loc_method toggle_loc_method 2>&1 | tail -15`
Expected: compile errors (fields/variants not found).

- [ ] **Step 3: Implement — state.rs**

Add fields to `AppState` (near `show_room_numbers`):
```rust
    /// How the current room was detected (for the map indicator). Retained
    /// across turns; updated when a turn reports a method.
    pub loc_method: Option<zvm::location::LocationMethod>,
    /// Whether the detection-method indicator is shown. Default false.
    pub show_loc_method: bool,
```
Add to the `AppState` default (near `show_room_numbers: false`):
```rust
            loc_method: None,
            show_loc_method: false,
```

- [ ] **Step 4: Implement — config.rs**

Mirror `show_room_numbers` (4 sites):
- field (after `show_room_numbers`):
```rust
    /// Show the room-detection-method indicator in the map corner. Default false.
    #[serde(default)]
    pub show_loc_method: bool,
```
- `Default for Config` (after `show_room_numbers: false,`): `show_loc_method: false,`
- merge-from-file block (after `cfg.show_room_numbers = from_file.show_room_numbers;`):
  `cfg.show_loc_method = from_file.show_loc_method;`
- `write_config` (after the `show_room_numbers` line):
  `doc["show_loc_method"] = toml_edit::value(cfg.show_loc_method);`
- the test fixture struct literal near line 617 (the one that lists `show_room_numbers: false,`): add `show_loc_method: false,`.

- [ ] **Step 5: Implement — keymap.rs**

Mirror `ToggleRoomNumbers` (5 sites):
- `Command` enum (after `ToggleRoomNumbers,`): `ToggleLocMethod,`
- `command_to_action` (after the `Command::ToggleRoomNumbers => Action::ToggleRoomNumbers,` arm): `Command::ToggleLocMethod => Action::ToggleLocMethod,`
- serialize name: `Command::ToggleLocMethod => "toggle_loc_method",`
- display name: `Command::ToggleLocMethod => "location method",`
- context: `Command::ToggleLocMethod => Context::Global,`
- the all-commands list (the `Command::ToggleRoomNumbers,` entry near line 393): add `Command::ToggleLocMethod,`

- [ ] **Step 6: Implement — input.rs**

- `Action` enum (after `ToggleRoomNumbers,`): `ToggleLocMethod,`
- `apply_action` (after `Action::ToggleRoomNumbers => state.show_room_numbers = !state.show_room_numbers,`):
  `Action::ToggleLocMethod => state.show_loc_method = !state.show_loc_method,`

- [ ] **Step 7: Implement — main.rs**

- After `state.show_room_numbers = cfg.show_room_numbers;` (~line 692):
  `state.show_loc_method = cfg.show_loc_method;`
- In `apply_turn_events` (the helper added for sound, called on both turn paths), append:
```rust
    state.loc_method = result.location_method.or(state.loc_method);
```

- [ ] **Step 8: Run to verify pass**

Run: `cargo test -p app show_loc_method toggle_loc_method 2>&1 | tail -10` → pass.
Run: `cargo test -p app 2>&1 | tail -5` → green; `cargo build -p app` → 0 warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/config.rs crates/app/src/keymap.rs crates/app/src/input.rs crates/app/src/main.rs
git commit -m "feat(app): loc_method state + show_loc_method config + toggle-loc-method command

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 8: app — `loc_indicator` themeable selector

**Files:**
- Modify: `crates/app/src/colors.rs` — `loc_indicator: Style` field + defaults in both constructors.
- Modify: `crates/app/src/style.rs` — `SELECTOR_FIELDS`, `apply_color_decls`, `write_style_full`.

This mirrors the `sound_beep_high`/`sound_beep_low` wiring already in the codebase.

**Interfaces:**
- Produces: `ColorScheme.loc_indicator: Style`.

- [ ] **Step 1: Write the failing tests**

Add to `crates/app/src/colors.rs` tests:
```rust
#[test]
fn loc_indicator_default_is_dim() {
    let cs = ColorScheme::terminal_default();
    assert_eq!(cs.loc_indicator.fg, Some(Color::DarkGray));
}
```
Add to `crates/app/src/style.rs` tests:
```rust
#[test]
fn loc_indicator_selector_parses() {
    let doc = parse_style_toml("[colors]\n\"loc_indicator\" = { fg = \"green\" }\n").unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(cs.loc_indicator.fg, Some(ratatui::style::Color::Green));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app loc_indicator 2>&1 | tail -15` → compile error (field not found).

- [ ] **Step 3: Implement**

`crates/app/src/colors.rs`:
- struct field (end of `ColorScheme`, after `sound_beep_low`):
```rust
    /// Room-detection-method indicator (map corner).
    pub loc_indicator: Style,
```
- `terminal_default()` (after `sound_beep_low: ...`):
```rust
            loc_indicator: Style::new().fg(Color::DarkGray),
```
- `from_ghostty` (after `sound_beep_low: ...`): use a dim palette foreground:
```rust
            loc_indicator: Style::new().fg(scheme.palette[8]),
```

`crates/app/src/style.rs`:
- `SELECTOR_FIELDS` (after `"sound_beep_low",`): `"loc_indicator",`
- `apply_color_decls` (after the `"sound_beep_low" => ...` arm):
  `"loc_indicator" => cs.loc_indicator = cs.loc_indicator.patch(style),`
- `write_style_full` (after the `sound_beep_low` insert):
  `doc.colors.selectors.insert("loc_indicator".to_string(), style_to_decl(&cs.loc_indicator));`

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app loc_indicator 2>&1 | tail -10` → pass.
Run: `cargo test -p app style 2>&1 | tail -5` → green (incl. `write_style_full_is_self_contained`).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(app): themeable loc_indicator selector for the detection-method label

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 9: app — render the method indicator (map bottom-right)

**Files:**
- Modify: `crates/app/src/render/map.rs` — draw the label in `render_map_layered`; add a label helper + test.

**Interfaces:**
- Consumes: `AppState.show_loc_method`, `AppState.loc_method`, `ColorScheme.loc_indicator`, `zvm::LocationMethod`.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/app/src/render/map.rs`:

```rust
#[test]
fn loc_method_label_strings() {
    use zvm::location::LocationMethod::*;
    assert_eq!(loc_method_label(GlobalVar0), "via status variable");
    assert_eq!(loc_method_label(PlayerParent), "via player object");
    assert_eq!(loc_method_label(StatusName), "via name match");
    assert_eq!(loc_method_label(NameOnly), "via name (unlinked)");
}

#[test]
fn indicator_drawn_bottom_right_when_enabled() {
    use mapper::graph::MapGraph;
    let g = MapGraph::default();
    let rm = std::collections::HashMap::new(); // empty render map; see sibling render tests for the real type
    let mut state = AppState::default();
    state.show_loc_method = true;
    state.loc_method = Some(zvm::location::LocationMethod::StatusName);
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    render_map_layered(&rm, &g, &state, area, &mut buf);
    // The label "via name match" ends at the bottom-right; check its last char.
    let row = area.bottom() - 1;
    let last = buf.cell((area.right() - 1, row)).unwrap().symbol().to_string();
    assert_eq!(last, "h", "expected the 'h' of 'via name match' in the corner");
}
```

NOTE: match the real first argument type of `render_map_layered` (the `&rm` render-map). Read the sibling tests `render_map_layered_no_in_content_strip_when_border_present` in this file and copy their exact `rm`/graph construction; the assertion (label present in the bottom-right) is the point.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app -- loc_method_label indicator_drawn 2>&1 | tail -15`
Expected: compile error (`loc_method_label` not found) / assertion fail.

- [ ] **Step 3: Implement**

In `crates/app/src/render/map.rs`, add the label helper (module scope):

```rust
/// Descriptive label for the room-detection method shown in the map corner.
pub(crate) fn loc_method_label(m: zvm::location::LocationMethod) -> &'static str {
    use zvm::location::LocationMethod::*;
    match m {
        GlobalVar0 => "via status variable",
        PlayerParent => "via player object",
        StatusName => "via name match",
        NameOnly => "via name (unlinked)",
    }
}
```

At the END of `render_map_layered` (just before it returns), add:

```rust
    // Detection-method indicator: bottom-right corner, hidden by default.
    if state.show_loc_method {
        if let Some(m) = state.loc_method {
            let label = loc_method_label(m);
            let w = label.chars().count() as u16;
            if area.width >= 1 && area.height >= 1 {
                let y = area.bottom() - 1;
                let x = area.right().saturating_sub(w.min(area.width));
                let style = state.colors.loc_indicator;
                let mut cx = x;
                for ch in label.chars() {
                    if cx >= area.right() {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        let mut b = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut b)).set_style(style);
                    }
                    cx += 1;
                }
            }
        }
    }
```

(If `render_map_layered` has multiple early returns, draw the indicator on the normal path that renders the map content; one draw at the end of the main body is sufficient.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app -- loc_method_label indicator_drawn 2>&1 | tail -10` → pass.
Run: `cargo test -p app 2>&1 | tail -5` → green; `cargo build -p app` → 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): render the room-detection-method indicator in the map corner

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 10: README — document v4+ room detection + the indicator

**Files:**
- Modify: `README.md`.

- [ ] **Step 1: Update the docs**

Under "### Live automapping", add a bullet after the automatic-room-placement bullet:

```markdown
- **v4+ room detection** — for v4/v5 games that don't expose the room in the
  classic v3 status variable (Hitchhiker, Bureaucracy, A Mind Forever Voyaging),
  the room is read from the status line and resolved to a game object — preferring
  the player object's room when the game re-parents the player (Inform), falling
  back to a name-only room otherwise. A hideable indicator in the map's
  bottom-right corner shows how the current room was found (`toggle-loc-method`,
  persisted via `show_loc_method`; styled by `loc_indicator`): `via player
  object`, `via name match`, `via name (unlinked)`, or `via status variable`.
```

- [ ] **Step 2: Verify**

Run: `grep -n "v4+ room detection\|toggle-loc-method" README.md` → the bullet appears once.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: README — v4+ room detection and the method indicator

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Post-wave manual verification (controller, not the background agent)

The v4+ end-to-end behavior needs a real story; verify after the wave:
`cargo run -p app -- stories/hitchhik.z5`, then in-game: `turn on light` → the
map should show the **Bedroom** room (method `via name match`, visible after
`Ctrl+K` → toggle location method, or with `show_loc_method = true`). Confirm
v3 games (e.g. a `.z3`) still map (method `via status variable`).

## Self-Review

**Spec coverage:**
- `status_line_room_name` (label + common form, posture strip) → Task 1. ✓
- `status_name_matches` (word-boundary prefix) → Task 2. ✓
- `find_player_object`, `nearest_matching_ancestor` (walk-up), `resolve_room_object` (longest match) → Task 2. ✓
- `Location` enum + `LocationMethod` + `object()`/`method()` + `detect_location` (v3 G0; v4+ player-parent → status-name → name-only → None) → Task 3. ✓
- Synthetic id (`0x8000` high bit, fnv1a, normalized, deterministic) + `is_synthetic_room` → Task 4. ✓
- Session conversion (NameOnly → synthetic ObjectSnapshot; method) + `location_method` field + construction-site fixups → Task 5. ✓
- `list_room_objects` guard → Task 6. ✓
- State/config/command wiring (default hidden, `toggle-loc-method`) → Task 7. ✓
- `loc_indicator` theming → Task 8. ✓
- Bottom-right indicator render + descriptive labels → Task 9. ✓
- v3 unchanged; `main.rs` snap.number sites unchanged (only additive `location_method`) → Tasks 3/5. ✓
- Deferred (mazes for Infocom) — not implemented, correct. ✓

**Placeholder scan:** No TBD/TODO. The two test-construction notes (Task 5 session helper, Task 9 render-map argument) explicitly instruct the executor to copy the sibling tests' exact construction, because those helper names/types are file-local and must match what already exists.

**Type consistency:** `Location`/`LocationMethod` used identically across Tasks 3/5/7/9. `synthetic_room_id`/`is_synthetic_room`/`SYNTHETIC_ROOM_FLAG` consistent across Tasks 4/5/6. `status_name_matches` signature consistent (Tasks 2/3). `loc_indicator` field consistent (Tasks 8/9). `loc_method`/`show_loc_method` consistent (Tasks 7/9).
