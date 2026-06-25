# v4+ Room Detection via Status-Line Name Resolution — Design

**Date:** 2026-06-25
**Status:** Approved, ready for planning

## Goal

Make the live automapper detect the current room for v4+ games — both Infocom
(Hitchhiker v5, Bureaucracy v4, AMFV v4) and Inform — which it currently fails
to do. Today `current_location()` reads global variable 0 for every version;
that is correct only for v3 status-line games. Also surface *which* detection
method produced the room, via a hideable indicator in the map's bottom-right
corner.

## Root cause (verified against the live games)

`crates/zvm/src/location.rs::current_location()` reads global variable 0 (G0)
for all versions. Evidence from running the real stories headlessly:

- **Hitchhiker (v5):** the room is in a game-specific global (G41, "HERE" =
  obj 150 "Bedroom"), **not** G0; `current_location()` returns `None` every
  turn. The player objects (`yourself` = obj 19, `Arthur Dent` = obj 152) are
  parented to an unnamed holder (obj 58), and rooms are parented to a separate
  unnamed container (obj 30). So Infocom does **not** re-parent the player into
  the room — neither G0 nor a "player → parent" walk finds the room.
- The status line *does* carry the room name: Hitchhiker row 1 =
  `Bedroom, in the bed       Score: 0   Moves: 1` (also `Darkness` unlit).

So for Infocom v4+ there is no universal object-tree path to the room; for
Inform v4+ the player *is* re-parented into the room. A single approach must
handle both.

## Prior art

Two dominant techniques for IF room detection:

1. **Transcript text-parsing** (Trizbort, Trizbort.io, IFMapper, Parchmap):
   detect the room short-name from the text, disambiguate same-named rooms by
   the room *description*, recommend VERBOSE mode. Identity is **name-based**;
   the most popular mappers use this as their primary mechanism.
2. **Z-machine object introspection:** v3 → global 0; v4+ → the player object's
   parent. Works for Inform games; **fails for Infocom games** (proven above).

Our **status line** is a better name source than transcript parsing — the name
is isolated, present every turn, no VERBOSE mode, no capitalization guessing.

## Chosen approach

For v4+, read the room name from the status line and use it both as the room
name and as a **validator** for the player-parent heuristic:

1. **Validated player-parent (preferred):** find the player object; if its
   parent's short name matches the status-line name, the game re-parents the
   player → use that parent object. This is maze-safe (the exact room object,
   even when two rooms share a display name) and handles Inform games.
2. **Status-line name → object:** otherwise resolve the status-line name to an
   object by short-name match. Handles Infocom games.
3. **Name-only:** if no object matches, map by the name with a synthetic id.
4. **None:** if there is no usable status-line name, observe nothing.

Player-parent is **only** used when the status line confirms it, so a
non-re-parenting (Infocom) game can never land on the wrong holder object.

## Components & data flow

### 1. zvm — `detect_location`

New public entry point in `crates/zvm/src/location.rs`:

```rust
/// How the current room was determined (drives the map indicator label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationMethod {
    GlobalVar0,   // v3 status variable
    PlayerParent, // v4+ player object's parent, validated by the status line
    StatusName,   // v4+ status-line name resolved to an object
    NameOnly,     // v4+ status-line name with no matching object
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
    pub fn object(&self) -> Option<&ObjectSnapshot>;
    /// The detection method tag.
    pub fn method(&self) -> LocationMethod;
}

/// Best-effort current room. Version-gated:
/// - v3 (and below): global 0 (existing logic) -> GlobalVar0, or None.
/// - v4+: validated player-parent -> status-name -> name-only -> None.
pub fn detect_location(machine: &Machine) -> Option<Location>;
```

- The existing `current_location() -> Option<ObjectSnapshot>` (the G0 logic) is
  retained and used internally for the v3 path; its tests stay valid.
- zvm does **not** synthesize RoomIds — `NameOnly` carries the raw name and the
  app owns id policy.

### 2. zvm — status-line extraction

A pure helper (testable without a Machine):

```rust
/// Extract a candidate room name from the v4+ status-line grid, or None.
pub fn status_line_room_name(upper: &UpperWindow, active_rows: u16) -> Option<String>;
```

Rules:
- Scan at most the first 2 rows (`min(active_rows, 2)`).
- Build each row's string from its cells.
- **Label form:** if any row has a segment matching `^\s*Location:\s*`
  (case-insensitive), take the text after the label.
- **Else (common form):** take row 1's first segment after splitting on runs of
  2+ spaces (drops the right-aligned `Score`/`Moves`/`Time`/`Date` block).
- Strip the posture suffix after the first comma (`Bedroom, in the bed` →
  `Bedroom`). Trim. Empty result → `None`.

Display name keeps original case (`Bedroom`).

### 3. zvm — player-parent validation and name resolution

```rust
/// The current player object, found by short name in a fixed set
/// {"yourself","you","me","myself","self"} (normalized). Returns the lowest
/// matching object number, or None.
fn find_player_object(machine: &Machine) -> Option<u16>;

/// Find the object whose short name matches `name` (normalized), or None.
/// Ties resolve to the lowest object number.
fn resolve_room_object(machine: &Machine, name: &str) -> Option<ObjectSnapshot>;
```

**Normalization** (matching only): trim, collapse internal whitespace to single
spaces, lowercase.

`detect_location` v4+ flow, given `name = status_line_room_name(..)`:
- `None` → return `None`.
- Else:
  1. `player = find_player_object(machine)`; `P = parent(player)`. If `P != 0`
     and `normalize(short_name(P)) == normalize(name)` →
     `PlayerParent(snapshot(P))`.
  2. Else `resolve_room_object(name)` → `StatusName(snapshot)` if it hits.
  3. Else `NameOnly(name)`.

### 4. app — `TurnResult` conversion + synthetic id + method

`crates/app/src/session.rs` converts `Location` into the existing
`TurnResult.location: Option<ObjectSnapshot>` (so `main.rs`'s ~10
`snap.number as RoomId` sites are unchanged) and adds the method:

```rust
// added to TurnResult:
pub location_method: Option<zvm::location::LocationMethod>,
```

- `location.object()` is `Some(snap)` → `TurnResult.location = Some(snap.clone())`.
- `Location::NameOnly(name)` → `TurnResult.location =
  Some(ObjectSnapshot { number: synthetic_room_id(&name), parent: 0, name })`.
- `TurnResult.location_method = Some(location.method())`.
- `None` → `location = None`, `location_method = None`.

Synthetic-id helper (new, app crate — owns `RoomId = u16` policy):

```rust
/// RoomIds with the high bit set denote name-only rooms (no backing object).
pub const SYNTHETIC_ROOM_FLAG: u16 = 0x8000;
/// Deterministic, save/reload-stable id for a name-only room. High bit always
/// set, so it can never collide with a real object number (no IF game has
/// >= 32768 objects). Normalizes the name before hashing.
pub fn synthetic_room_id(name: &str) -> u16; // 0x8000 | (fnv1a(normalize(name)) & 0x7FFF)
/// True when a RoomId denotes a name-only (non-object) room.
pub fn is_synthetic_room(id: u16) -> bool;   // id & SYNTHETIC_ROOM_FLAG != 0
```

### 5. app — guard VM-by-id reads

Code that treats a `RoomId` as a VM object number must skip synthetic ids:

- `crates/app/src/render/room_info.rs::list_room_objects(mem, room_id)` calls
  `get_child(mem, room_id)`. Guard: if `is_synthetic_room(room_id)`, return an
  empty `Vec` (no garbage reads outside the object table). Exits still come from
  the mapper graph, so a name-only room shows exits but no object list.
- The `objects_here` filter in `main.rs` (`get_parent(mem, o) == current_loc`)
  is already safe for synthetic ids (no real object's parent equals a synthetic
  value → empty set).

### 6. app — detection-method indicator (map bottom-right)

- **State:** `AppState.loc_method: Option<LocationMethod>`, updated each turn:
  `state.loc_method = result.location_method.or(state.loc_method)` (retain the
  last known method when a turn yields no location, to avoid flicker).
- **Visibility:** `AppState.show_loc_method: bool`, **default false** (hidden).
  Toggled by a new command `toggle-loc-method` (mirroring
  `toggle-room-numbers`/`show_room_numbers`), persisted via a new config field
  `show_loc_method: bool` (default `false`).
- **Render:** when `show_loc_method` is true and `loc_method` is `Some`, draw a
  one-line label in the **bottom-right** corner of the map pane content area,
  right-justified, clipped if the pane is too narrow. Labels (descriptive):
  - `GlobalVar0`   → `via status variable`
  - `PlayerParent` → `via player object`
  - `StatusName`   → `via name match`
  - `NameOnly`     → `via name (unlinked)`
- **Theming:** new style selector `loc_indicator` (a `Style`), wired into
  `SELECTOR_FIELDS`, `apply_color_decls`, `write_style_full`, and both
  `ColorScheme` constructors. Default: a dim foreground (e.g.
  `Style::new().fg(Color::DarkGray)` for `terminal_default`, palette-dim for
  `from_ghostty`).

## Error handling / edge cases

- **No status line / empty** (Bureaucracy intro): `detect_location` → `None`,
  no room observed (same as today). Rooms appear once a status line exists.
- **`Darkness`** (Hitchhiker unlit): maps as a room named "Darkness"
  (name-only). Honest and acceptable; not special-cased.
- **AMFV `(undefined)`:** maps by name "(undefined)".
- **v3 unchanged:** Zork/Planetfall/Spellbreaker keep mapping via global 0
  (method `GlobalVar0`).
- **Player in a sub-object** (e.g. "in the bed"): the player's parent is the
  sub-object, whose name won't match the status-line room name, so player-parent
  validation fails and we fall through to status-name resolution — correct.

## Testing

zvm:
- `status_line_room_name`: Hitchhiker row 1 with score/moves → "Bedroom" (after
  comma strip); `Darkness` → "Darkness"; `Location:  Foo` label form → "Foo";
  empty grid → None; multi-space split correctness.
- `find_player_object`: an object named "yourself" → its number; none → None;
  tie → lowest number.
- `resolve_room_object`: matching short name → snapshot; non-matching → None;
  tie → lowest number.
- `detect_location`: v3 → `GlobalVar0` from global 0 (existing behavior);
  v4+ player-parent validated → `PlayerParent`; v4+ name match → `StatusName`;
  v4+ unmatched → `NameOnly`; no status line → `None`.
- `Location::object()` / `method()` per variant.

app:
- `synthetic_room_id`: deterministic across calls; high bit always set;
  normalization collapses case/whitespace variants to one id; different names →
  different ids (spot-check).
- `is_synthetic_room` true for synthesized ids, false for small object numbers.
- `list_room_objects` returns empty for a synthetic id (guard), non-empty for a
  real object id (existing behavior).
- session conversion: each `Location` variant → correct `TurnResult.location`
  (synthetic for `NameOnly`, pass-through otherwise) and `location_method`.
- indicator: label mapping per method; hidden when `show_loc_method == false` or
  `loc_method == None`; `toggle-loc-method` flips and persists `show_loc_method`;
  `loc_indicator` selector parses/applies/exports.

## Out of scope (deferred)

- **Maze / duplicate-name disambiguation by description** — now handled *for
  re-parenting (Inform) games* by validated player-parent; still deferred for
  Infocom-style games (status-name resolution merges same-named rooms).
- Per-game configuration of a location global.
- Special handling of darkness as a non-room.
- Configurable player-name set (the fixed set + status-line validation suffices;
  an unrecognized player just falls through to status-name resolution).
