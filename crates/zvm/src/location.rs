// Mapper-facing API — current player location and read-only object tree.
//
// This module provides two signals that the future automapper consumes:
//   1. `current_location` — the object representing where the player is now.
//   2. `object_tree_view` — a read-only enumeration of all objects.
//
// # Location heuristic
//
// The Z-machine specification (ZMSD) has no standard mechanism for identifying
// the player's current location.  We use version-dependent heuristics:
//
// ## v3 (status-line games, ZMSD §8.2.2.1)
// The interpreter status line reads the current room from **global variable 0**
// (variable number 0x10, the first global).  This is the object number of the
// current room.  We read that global; if it is nonzero and within the valid
// object-number range we return its snapshot.
//
// ## v4+ (no status line / Inform games)
// There is no guaranteed status-line global.  Many Inform games still store a
// location-ish object in global 0, so we try the same strategy.  This is a
// best-effort heuristic; the automapper's "unknown direction" mechanism handles
// the occasional wrong or missing value gracefully.
//
// # Object-tree enumeration bounds
//
// The Z-machine does not store the object count explicitly.  We infer it from
// the layout: objects are stored in a compact array immediately after the
// property-defaults table; each object entry contains a pointer to its own
// property table.  The smallest property-table address found across all entries
// marks where the object entries array ends, because property tables are always
// placed after the object entries in well-formed story files.
//
// Concretely: iterate candidate objects starting from 1.  For each candidate,
// read the property-table pointer stored in its entry.  If that pointer is less
// than or equal to the start of the current candidate's own entry (meaning the
// pointer points back into the entry region itself), we have run past the end of
// the real object table.  We also stop if the pointer is zero.  A reasonable
// absolute cap of 2000 objects is applied to guard against malformed data.
//
// **Documented limitations:**
//   - The v4+ location is a best-effort guess; wrong answers are expected
//     occasionally and the automapper is designed to tolerate them.
//   - Object-count inference can be wrong for unusual story layouts (hand-crafted
//     or very old files where property tables are interleaved with entries).
//   - v8 and v7 stories use the same heuristic as v4+ for location.

use crate::cpu::exec::Machine;
use crate::objects::{entries_base, entry_size, get_parent, object_snapshot, prop_table_ptr_offset, short_name, ObjectSnapshot};
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

/// Returns the object representing the player's current location, or `None` if
/// the heuristic cannot determine a plausible location.
///
/// See module-level docs for the version-specific strategy.
pub fn current_location(machine: &Machine) -> Option<ObjectSnapshot> {
    let mem = &machine.mem;
    // Global variable 0 is at address `global_vars + 0` (var 0x10 maps to
    // global index 0, stored at global_vars base with no offset).
    let global0_addr = mem.global_vars() as u32;
    let obj_num = mem.read_word(global0_addr);

    if obj_num == 0 {
        return None;
    }

    // Validate that obj_num is within the object table.
    // We use the same bound logic as object_tree_view: check the entry would
    // lie before the first property table pointer within the table.
    let max_obj = max_object_number(mem);
    if obj_num > max_obj {
        return None;
    }

    Some(object_snapshot(mem, obj_num))
}

/// Returns a read-only enumeration of all objects in the story as snapshots.
///
/// Object count is inferred from the layout — see module docs for the approach.
pub fn object_tree_view(machine: &Machine) -> Vec<ObjectSnapshot> {
    let mem = &machine.mem;
    let n = max_object_number(mem);
    (1..=n).map(|i| object_snapshot(mem, i)).collect()
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Infer the maximum valid object number from the object table layout.
///
/// Scans entries starting from 1.  Stops when an entry's property-table pointer
/// points into or before the object-entries region (a sign we've run off the end
/// of the real table), or when the pointer is zero.  Capped at 2000 to guard
/// against pathological data.
fn max_object_number(mem: &crate::memory::Memory) -> u16 {
    let version = mem.version();
    let base = entries_base(mem);
    let esize = entry_size(version);

    let mut n: u16 = 0;
    for candidate in 1u16..=2000 {
        // Address of this candidate's entry.
        let entry_addr = base + (candidate as u32 - 1) * esize;
        // Property-table pointer is the last word of the entry.
        let prop_ptr_offset = prop_table_ptr_offset(version);
        let ptbl_addr = mem.read_word(entry_addr + prop_ptr_offset) as u32;

        // If the pointer is zero or points at or before the start of this entry,
        // we've gone past the real object table.
        if ptbl_addr == 0 || ptbl_addr <= entry_addr {
            break;
        }
        n = candidate;
    }
    n
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::exec::{Machine, StepResult};
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
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
        // Rename obj3 to "you" so it is the player.
        // "yourself" would be truncated to "yoursel" by v3 Z-char encoding (6 Z-char max),
        // so we use "you" which is in NAMES and encodes without truncation.
        let mut buf = build_v3_story();
        let name = z_name("you");
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
        // obj tree: obj3 (parent obj1), obj2 (parent obj1), obj1 (parent 0).
        // Searching from obj3 for "west" should walk up to obj1.
        let machine = make_machine(build_v3_story());
        let r = nearest_matching_ancestor(&machine, 3, "west").expect("walks up to west");
        assert_eq!(r.number, 1);
        assert!(nearest_matching_ancestor(&machine, 3, "nowhere").is_none());
    }

    // We reuse the same object-table layout as objects.rs tests:
    //   object_table = 0x0100, entries_base = 0x013E (v3)
    //   obj1 at 0x013E, obj2 at 0x0147, obj3 at 0x0150
    //   property tables at 0x0200, 0x0220, 0x0230
    //   global_vars = 0x0300

    const OBJ_TABLE: u32 = 0x0100;
    const ENTRIES_V3: u32 = OBJ_TABLE + 31 * 2; // 0x013E
    const OBJ1_ENTRY: u32 = ENTRIES_V3;
    const OBJ2_ENTRY: u32 = ENTRIES_V3 + 9;
    const OBJ3_ENTRY: u32 = ENTRIES_V3 + 18;

    const PROP1_TBL: u16 = 0x0200;
    const PROP2_TBL: u16 = 0x0220;
    const PROP3_TBL: u16 = 0x0230;

    const GLOBAL_VARS: u32 = 0x0300;

    fn put_word(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset]     = (val >> 8) as u8;
        buf[offset + 1] = (val & 0xFF) as u8;
    }

    /// Write `val` as a big-endian word into the buffer.
    fn z_name(text: &str) -> Vec<u8> {
        crate::text::encode::encode_word(text, 3) // 4 bytes = 2 Z-words for v3
    }

    /// Build a minimal v3 story with 3 objects and properly encoded short names.
    /// Object structure: obj1 (root), obj2 (child of obj1), obj3 (sibling of obj2).
    fn build_v3_story() -> Vec<u8> {
        let mut buf = sample_story(3);

        // ── obj1 entry: parent=0 sibling=0 child=2, prop=PROP1_TBL ──────────
        buf[OBJ1_ENTRY as usize + 4] = 0; // parent
        buf[OBJ1_ENTRY as usize + 5] = 0; // sibling
        buf[OBJ1_ENTRY as usize + 6] = 2; // child
        put_word(&mut buf, OBJ1_ENTRY as usize + 7, PROP1_TBL);

        // ── obj2 entry: parent=1 sibling=3 child=0 ───────────────────────────
        buf[OBJ2_ENTRY as usize + 4] = 1;
        buf[OBJ2_ENTRY as usize + 5] = 3;
        buf[OBJ2_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ2_ENTRY as usize + 7, PROP2_TBL);

        // ── obj3 entry: parent=1 sibling=0 child=0 ───────────────────────────
        buf[OBJ3_ENTRY as usize + 4] = 1;
        buf[OBJ3_ENTRY as usize + 5] = 0;
        buf[OBJ3_ENTRY as usize + 6] = 0;
        put_word(&mut buf, OBJ3_ENTRY as usize + 7, PROP3_TBL);

        // ── Property table for obj1: name "west", no properties ──────────────
        // name_words = 2 (encode_word produces 4 bytes = 2 Z-words for v3)
        let name1 = z_name("west");
        assert_eq!(name1.len(), 4);
        buf[PROP1_TBL as usize] = 2; // 2 Z-words in name
        buf[PROP1_TBL as usize + 1..PROP1_TBL as usize + 5].copy_from_slice(&name1);
        buf[PROP1_TBL as usize + 5] = 0x00; // end-of-properties sentinel

        // ── Property table for obj2: name "east" ─────────────────────────────
        let name2 = z_name("east");
        buf[PROP2_TBL as usize] = 2;
        buf[PROP2_TBL as usize + 1..PROP2_TBL as usize + 5].copy_from_slice(&name2);
        buf[PROP2_TBL as usize + 5] = 0x00;

        // ── Property table for obj3: name "hall" ─────────────────────────────
        let name3 = z_name("hall");
        buf[PROP3_TBL as usize] = 2;
        buf[PROP3_TBL as usize + 1..PROP3_TBL as usize + 5].copy_from_slice(&name3);
        buf[PROP3_TBL as usize + 5] = 0x00;

        buf
    }

    /// Build a Machine from story bytes.
    fn make_machine(buf: Vec<u8>) -> Machine {
        Machine::new(Memory::new(buf).unwrap())
    }

    // ── TDD Step 1: write the failing tests ───────────────────────────────────
    // (These were written BEFORE the implementation; the RED→GREEN cycle is
    //  documented in the task report.)

    // ── current_location: v3 hit ──────────────────────────────────────────────

    #[test]
    fn v3_current_location_from_global0() {
        let mut buf = build_v3_story();
        // Set global 0 (at GLOBAL_VARS) to object 1.
        put_word(&mut buf, GLOBAL_VARS as usize, 1);
        let machine = make_machine(buf);
        let loc = current_location(&machine).expect("should return a snapshot");
        assert_eq!(loc.number, 1);
        // Name should be "west" (our encoded name for obj1).
        assert!(loc.name.starts_with('w'), "expected name starting with 'w', got {:?}", loc.name);
    }

    // ── current_location: v3 None when global0 == 0 ──────────────────────────

    #[test]
    fn v3_current_location_none_when_global0_zero() {
        let mut buf = build_v3_story();
        // global 0 = 0 → no location
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── current_location: None when global0 exceeds max object ───────────────

    #[test]
    fn v3_current_location_none_when_global0_out_of_range() {
        let mut buf = build_v3_story();
        // 0xFFFF is never a valid object in our tiny tree
        put_word(&mut buf, GLOBAL_VARS as usize, 0xFFFF);
        let machine = make_machine(buf);
        assert!(current_location(&machine).is_none());
    }

    // ── object_tree_view: returns all 3 objects with correct fields ───────────

    #[test]
    fn v3_object_tree_view_count_and_fields() {
        let mut buf = build_v3_story();
        // global 0 = 0 (irrelevant for tree view)
        put_word(&mut buf, GLOBAL_VARS as usize, 0);
        let machine = make_machine(buf);
        let tree = object_tree_view(&machine);

        assert_eq!(tree.len(), 3, "expected exactly 3 objects, got {}", tree.len());

        // obj1: number=1 parent=0 name starts 'w'
        assert_eq!(tree[0].number, 1);
        assert_eq!(tree[0].parent, 0);
        assert!(tree[0].name.starts_with('w'), "obj1 name: {:?}", tree[0].name);

        // obj2: number=2 parent=1 name starts 'e'
        assert_eq!(tree[1].number, 2);
        assert_eq!(tree[1].parent, 1);
        assert!(tree[1].name.starts_with('e'), "obj2 name: {:?}", tree[1].name);

        // obj3: number=3 parent=1 name starts 'h'
        assert_eq!(tree[2].number, 3);
        assert_eq!(tree[2].parent, 1);
        assert!(tree[2].name.starts_with('h'), "obj3 name: {:?}", tree[2].name);
    }

    // ── Fixture-backed test (skips when minizork.z3 absent) ──────────────────

    #[test]
    fn minizork_current_location_returns_something() {
        let Some(story) = crate::fixtures::load("minizork.z3") else {
            // Fixture absent — skip.
            return;
        };
        let mem = Memory::new(story).unwrap();
        let mut machine = Machine::new(mem);
        machine.init_caps();
        // In minizork the opening location is set in global 0 before the first
        // read instruction.  Run until NeedLine (the first READ opcode) so that
        // the init code has had a chance to store the starting room into global 0.
        for _ in 0..100_000u64 {
            match machine.step() {
                StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart => break,
                StepResult::NeedChar => { machine.supply_char(b'\n'); }
                StepResult::SaveRequest => { machine.complete_save(false); }
                StepResult::RestoreRequest => { machine.complete_restore_failure(); }
                StepResult::Continue => {}
            }
        }
        let loc = current_location(&machine);
        assert!(loc.is_some(), "minizork: expected a location from global 0, got None");
    }
}
