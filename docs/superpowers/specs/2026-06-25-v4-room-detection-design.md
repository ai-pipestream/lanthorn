# v4+ Room Detection via Status-Line Name Resolution — Design

**Date:** 2026-06-25
**Status:** Approved, ready for planning

## Goal

Make the live automapper detect the current room for Infocom v4+ games (and v4+
games generally), which it currently fails to do. Today `current_location()`
reads global variable 0 for every version; that is correct only for v3
status-line games, so Hitchhiker (v5), Bureaucracy (v4), and A Mind Forever
Voyaging (v4) never produce rooms.

## Root cause (verified against the live games)

`crates/zvm/src/location.rs::current_location()` reads global variable 0 (G0)
for all versions. Evidence from running the real stories headlessly:

- **Hitchhiker (v5):** the room is in a game-specific global (G41, "HERE" =
  obj 150 "Bedroom"), **not** G0; `current_location()` returns `None` every
  turn. The player objects (`yourself` = obj 19, `Arthur Dent` = obj 152) are
  parented to an unnamed holder (obj 58), and rooms are parented to a separate
  unnamed container (obj 30). So Infocom does **not** re-parent the player into
  the room — neither G0 nor a "player → parent" walk finds the room.
- This rules out the "player's parent" heuristic that some Z-machine mappers
  use (it works for Inform games but fails for classic Infocom games).

There is no universal object-tree path to the room for Infocom v4+; the room
lives only in a per-game global.

## Prior art

Two dominant techniques exist for IF room detection:

1. **Transcript text-parsing** (Trizbort, Trizbort.io, IFMapper, Parchmap):
   detect the room short-name from the text (heuristic: the line whose
   4+-letter words are capitalized), disambiguate same-named rooms by the room
   *description*, recommend the game's VERBOSE mode. Identity is **name-based**;
   interpreter internals are never touched. This is how the most popular
   mappers work as their *primary* mechanism.
2. **Z-machine object introspection:** v3 → global 0; v4+ → the player object's
   parent (player found by an object named "yourself"/"self"). Works for Inform
   games; **fails for Infocom games** (proven above).

Key takeaway: name-based mapping is well-precedented and robust, and our
**status line** is a *better* name source than transcript-header parsing — the
room name is isolated on the status line every turn, with no VERBOSE mode and
no capitalization guessing.

## Chosen approach

**Read the room name from the status line, resolve it to a game object when
possible (keeping stable object identity), and fall back to mapping by name
when no object matches.**

### Status-line content (verified)

The captured upper-window grid (`machine.screen.upper`, populated by the v4+
screen model) holds the status line. Observed formats:

- **Hitchhiker (v5):** row 1 = `Bedroom, in the bed       Score: 0   Moves: 1`
  (also `Darkness` when unlit, `Bedroom` after standing). Location is the
  left-justified segment; score/moves is a right-aligned block; posture follows
  a comma.
- **AMFV (v4):** a labeled row — `Location:  (undefined)` — alongside
  Mode/Time/Date.
- **Bureaucracy (v4):** no room status line at the intro (upper window empty,
  later becomes a form).

## Components & data flow

### 1. zvm — `detect_location`

New public entry point in `crates/zvm/src/location.rs`:

```rust
/// The mapper-facing location signal for one turn.
pub enum LocationId {
    /// Resolved to a real game object (v3 global 0, or a v4+ status-line name
    /// that matched an object's short name).
    Object(ObjectSnapshot),
    /// A v4+ status-line room name that matched no object — map by name only.
    Named(String),
}

/// Best-effort current room. Version-gated:
/// - v3 (and below): global 0 (existing logic) -> Object, or None.
/// - v4+: status-line room name -> resolve to object (Object) or Named; None
///   when there is no usable status-line name.
pub fn detect_location(machine: &Machine) -> Option<LocationId>;
```

- The existing `current_location() -> Option<ObjectSnapshot>` (the G0 logic) is
  retained and used internally by `detect_location` for the v3 path; its tests
  stay valid.
- zvm does **not** synthesize RoomIds — it returns `Named(String)` and lets the
  app own id policy.

### 2. zvm — status-line extraction

A pure helper (testable without a Machine), operating on the upper-window grid
text:

```rust
/// Extract a candidate room name from the v4+ status-line grid, or None.
pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String>;
```

Rules:
- Consider the active upper-window rows, scanning at most the first 2 rows
  (`min(active_rows, 2)`).
- Build each row's string from its cells.
- **Label form:** if any row contains a segment matching `^\s*Location:\s*`
  (case-insensitive), take the text after the label.
- **Else (common form):** take row 1's first segment after splitting on runs of
  2+ spaces (drops the right-aligned `Score`/`Moves`/`Time`/`Date` block).
- Strip the posture suffix after the first comma (`Bedroom, in the bed` →
  `Bedroom`).
- Trim. If the result is empty, return `None`.

Display name keeps original case (`Bedroom`).

### 3. zvm — name → object resolution

```rust
/// Find the object whose short name matches `name` (normalized), if any.
/// Ties resolve to the lowest object number.
fn resolve_room_object(machine: &Machine, name: &str) -> Option<ObjectSnapshot>;
```

- **Normalization** (for matching only): trim, collapse internal whitespace to
  single spaces, lowercase.
- Compare the normalized extracted name to each object's normalized
  `short_name`. On match, return that object's snapshot (real number + its
  canonical short name).
- `detect_location` for v4+: `Some(name)` from extraction → if
  `resolve_room_object` hits, return `Object(snapshot)`; else
  `Named(extracted_name)`. Extraction `None` → `detect_location` returns `None`.

### 4. app — `TurnResult.location` conversion + synthetic id

`crates/app/src/session.rs` converts `LocationId` into the existing
`TurnResult.location: Option<ObjectSnapshot>`, so `crates/app/src/main.rs` (which
uses `snap.number as RoomId` in ~10 places) is **unchanged**:

- `Object(snap)` → `snap`.
- `Named(name)` → `ObjectSnapshot { number: synthetic_room_id(&name),
  parent: 0, name }`.

Synthetic-id helper (new, app crate — owns `RoomId = u16` policy):

```rust
/// RoomIds with the high bit set denote name-only rooms (no backing object).
pub const SYNTHETIC_ROOM_FLAG: u16 = 0x8000;

/// Deterministic, save/reload-stable id for a name-only room. The high bit is
/// always set, so it can never collide with a real object number (no IF game
/// has >= 32768 objects).
pub fn synthetic_room_id(name: &str) -> u16; // 0x8000 | (fnv1a(normalized(name)) & 0x7FFF)

/// True when a RoomId denotes a name-only (non-object) room.
pub fn is_synthetic_room(id: u16) -> bool; // id & SYNTHETIC_ROOM_FLAG != 0
```

`synthetic_room_id` normalizes the name the same way as resolution (trim,
collapse whitespace, lowercase) before hashing, so capitalization/spacing
variants of the same status name map to one room.

### 5. app — guard VM-by-id reads

Any code that treats a `RoomId` as a VM object number must skip synthetic ids:

- `crates/app/src/render/room_info.rs::list_room_objects(mem, room_id)` calls
  `get_child(mem, room_id)`. Guard: if `is_synthetic_room(room_id)`, return an
  empty `Vec` (no garbage reads outside the object table). Exits still come from
  the mapper graph, so a name-only room shows its exits but no object list.
- The `objects_here` computation in `main.rs` (filter `get_parent(mem, o) ==
  current_loc`) is already safe for synthetic ids — no real object's parent
  equals a synthetic value, so it yields an empty set — but the guarded path
  keeps it explicit.

## Error handling / edge cases

- **No status line / empty** (Bureaucracy intro): `detect_location` → `None`,
  no room observed (same as today). Rooms appear once a status line exists.
- **`Darkness`** (Hitchhiker unlit): maps as a room named "Darkness"
  (name-only). Honest and acceptable; not special-cased in v1.
- **AMFV `(undefined)`:** maps by name "(undefined)"; AMFV is a known edge case.
- **v3 unchanged:** Zork/Planetfall/Spellbreaker keep mapping via global 0.

## Testing

zvm:
- `status_line_room_name`: Hitchhiker row 1 with score/moves → "Bedroom" (after
  comma strip); `Darkness` → "Darkness"; `Location:  Foo` label form → "Foo";
  empty grid → None; multi-space split correctness.
- `resolve_room_object`: a grid name matching an object short name → `Object`
  with that number; non-matching name → caller gets `Named`; tie → lowest
  number.
- `detect_location`: v3 path returns `Object` from global 0 (existing behavior);
  v4+ resolved vs name-only.

app:
- `synthetic_room_id`: deterministic for the same name across calls; high bit
  always set; different names → different ids (spot-check); normalization
  (case/whitespace variants collapse to one id).
- `is_synthetic_room` true for synthesized ids, false for small object numbers.
- `list_room_objects` returns empty for a synthetic id (guard), non-empty for a
  real object id (existing behavior).
- session conversion: `Named` → `ObjectSnapshot` with a synthetic high-bit
  number; `Object` passed through unchanged.

## Out of scope (deferred)

- **Maze / duplicate-name disambiguation by description** — prior art uses the
  room description as a secondary key when names collide; deferred to a later
  pass. v1 merges rooms that share a displayed name.
- Per-game configuration of a location global.
- Special handling of darkness as a non-room.
- Re-parenting/player-object heuristics (rejected — fails for Infocom).
