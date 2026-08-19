# lanthorn — Design

**Date:** 2026-06-18
**Status:** Approved design, pre-implementation

## 1. Overview & Scope

lanthorn is a TUI interactive-fiction interpreter built around **automapping**. The
first version targets the **Z-machine** — all text-mode versions: **3, 4, 5, 7, and 8**, covering
classic Infocom games and Inform 6 output. The **graphical** version **6** is deliberately excluded
(see below). We write our **own Z-machine VM** in **Rust** so the
automapper can read the VM's object tree directly, which is what makes reliable,
game-agnostic mapping possible.

The name nods to the *Treaty of Babel* IF metadata standard; the per-game map is keyed by
the story's **IFID** (the Babel identifier).

**Explicitly in scope (v1):** Z-machine v3/v4/v5/v7/v8 execution, automapping with light manual
correction, persistent per-game maps, a split TUI with scroll/zoom, room notes, current-room
tracking, and image export.

**Why v6 is excluded:** v6 is the *graphical* Z-machine — bitmap pictures, complex multi-window
layout, and mouse input. It is a fundamentally different and much larger machine that does not fit a
text TUI. This is a principled boundary, not an oversight: all *text-mode* versions are supported.
(v1/v2 are trivial historical variants with essentially no game catalog and are also out of scope.)

**Explicitly out of scope (v1), but the architecture must not preclude:** Glulx (and other
VMs), v6 graphical support, a full freehand map editor, custom-verb direction learning.

## 2. Architecture

Four well-bounded units, each understandable and testable on its own.

### `zvm` — the Z-machine core
The only unit that understands Z-machine bytecode. Responsibilities:
- Load a story file (v3/v4/v5/v7/v8); reject other versions fast with a clear message (notably a
  specific "v6 graphical games are not supported" message).
- Execute opcodes; expose a screen model (windows, status line) and input requests.
- **Save/restore mechanism only:** `save() -> bytes` / `restore(bytes)` that serialize and
  deserialize VM state (dynamic memory, stack, program counter) to/from a standard **Quetzal** byte
  buffer. `zvm` is the only unit that knows VM internals, so it owns this serialization — but it does
  **not** touch the filesystem. Where the bytes go is `app`'s concern.
- Expose a **read-only view of the object tree** and the **current player-object location**.

Boundary: emits two kinds of signal the mapper cares about — screen/transcript output, and
"player's room object is now X". Knows nothing about rendering or map layout.

### `mapper` — the automapper
VM-agnostic. Consumes: (a) "player's room object changed to X" and (b) "last command was
direction D" (parsed from input). Maintains the **map graph** and **grid layout**. Knows
nothing about bytecode or rendering. Unit-testable with synthetic event streams.

### `tui` — the terminal UI (ratatui)
Split view: transcript pane + map pane. Collapsible map (→ full-screen transcript) and
expandable map (→ full-screen, scrollable). Renders the map graph; routes keys/commands.
Owns view state (zoom level, scroll position, current-room highlight).

### `app` — orchestration & persistence
Wires the three units together, owns the run loop, and owns all **file I/O and persistence policy**
for two distinct, independent stores:
- **Quetzal game saves:** takes the bytes from `zvm.save()` and writes them to a `.qzl` file (and
  feeds a chosen file's bytes back to `zvm.restore()`). Point-in-time, game-triggered, possibly
  multiple slots, and **portable** — other interpreters can read/write these files.
- **The map store:** a single, cumulative, per-IFID map file (see §5). Independent of game saves.

It also owns image export. The two stores are never bundled together (see §5).

### Data flow per turn
1. User types a line → `app` notes whether it is a recognized movement direction.
2. `zvm` runs the turn → reports new player location + screen output.
3. `mapper` adds/updates the room and, if a direction was recognized, the directed edge.
4. `tui` re-renders transcript + map.

The clean `zvm` ↔ `mapper` ↔ `tui` boundaries are what allow a second VM (Glulx) to be added
later as an additive step rather than a rewrite: a Glulx core would emit the same location and
screen signals the mapper and UI already consume.

## 3. Map Data Model

### Rooms
- **Identity = Z-machine object number** (stable within a game), read from the object tree —
  *not* the room name. This is how same-named rooms are differentiated: two "Forest" rooms with
  different object numbers are distinct nodes; a genuinely revisited room (same object number) is
  the same node even when its description text varies (darkness, weather).
- **Label** = the object's short name, with a player-editable override.
- **Disambiguation:** when labels collide on screen, render distinct suffixes (e.g. `Forest`,
  `Forest·2`), with the object number visible on inspect.
- **Notes:** a first-class, freeform, per-room text field (e.g. "blue key here", "troll blocks
  west until fed"). Persisted; shown on inspect; a small marker on the room indicates notes exist.
- **Grid position:** assigned by layout (see below); player-nudgeable.

### Connections
- A connection is a **directed edge** keyed by `(origin room, direction)` → destination room.
- **No symmetry is assumed anywhere.** Exits in IF are frequently non-reciprocal (enter a room
  going *north*, leave it going *west*; or one-way passages).
- The renderer draws each connector as a **routed orthogonal path** (box-drawing glyphs
  `┌ ┐ └ ┘ ─ │` with bends), not a forced straight line. This decouples two concerns:
  - **direction** is encoded by *which side of the origin room the connector departs* (a north exit
    leaves the top edge) — this preserves the per-origin-room arrow semantics, and
  - the **path** to the destination is free to bend through the gutters between rooms, so a
    destination that is not directly north on the grid can still be reached by a connector that
    *departs north* and then routes over to it.
- A reciprocal-opposite pair (A→north→B and B→south→A) may collapse to a single clean line; a
  non-reciprocal pair (A→north→B, B→west→A) renders as two separate connectors leaving from
  different sides.
- **Unknown-direction edges:** a room change after a non-compass command (or a game-initiated
  teleport) still creates the destination room, joined by an "unknown direction" edge the player
  can relabel.

### Layout
- Trizbort-style grid. Compass directions map to grid offsets.
- `up`/`down`/`in`/`out` render as labeled stubs/special connectors rather than grid moves.
- **Collisions** (two rooms wanting the same cell) are detected; the new room is placed in the
  nearest free cell and its edge is bent to reach it. Rooms **never** overlap.

### Layout modes
Layout is a per-game **mode**, persisted with the map.

- **Auto (default).** A constraint-based layout engine owns all room positions. Each directed
  edge is a relative-position constraint (e.g. "B is north of A"). As play reveals more
  constraints, the engine refines placements to (a) satisfy as many directional constraints as
  possible and (b) minimize overlapping/crossing arrows.
  - **Edge routing:** a path router places connectors as routed orthogonal paths (see Connections)
    that avoid room cells, minimize total crossings and bends, and are stable turn-to-turn. This
    requires **gutter space** between rooms — the grid keeps channels between cells for connectors to
    run, so rooms are not packed edge-to-edge.
  - **Stability:** re-layout prefers **minimal movement** — existing rooms stay put unless a new
    constraint genuinely forces a move, and routing stays stable likewise, so the map does not
    "jump" every turn.
- **Manual.** Auto is disabled; positions are frozen and the player nudges rooms freely.
  Switching Auto→Manual seeds manual positions from the current auto layout (start from what is on
  screen, not a reshuffle).

### When a clean layout is impossible
IF geography is frequently non-Euclidean (e.g. `north` four times returning to start; A north of B
*and* B north of A; tangles unrepresentable on a 2D grid without a crossing). The engine handles
this without ever forcing rooms to overlap:
- Routed orthogonal connectors (see Connections) already absorb most awkward cases, so **distorted**
  is the genuine last resort: it applies only when even a routed path cannot honor the
  departure-side direction given the placement, or the geometry is outright contradictory.
- In that case it relaxes the **least-confident** constraints and marks the affected connection as
  **distorted** — drawn with a labeled/broken connector rather than a clean directional path, so it
  is visually honest that the exit exists but does not fit the grid cleanly.
- Unavoidable arrow **crossings** are allowed as a last resort, and drawn so they read as crossings
  rather than junctions.

Per-room pinning while remaining in Auto (fix one room, let the engine place the rest) is a future
enhancement, out of scope for v1 — the v1 mode model is a clean Auto/Manual binary.

## 4. Direction Capture

Recognize `n/s/e/w/ne/nw/se/sw/u/d/in/out` and their long forms.
- Room change after a recognized direction → that directed edge.
- Room change after anything else, or a game-initiated teleport → destination room is still added,
  joined by an "unknown direction" edge.
- **Rooms are never lost.** Weird transitions are represented honestly rather than guessed at.

Custom-verb direction learning (remembering that `climb tree` meant "up" for this game) is a
deliberate future enhancement, not in v1.

## 5. Persistence & Light Correction

### Two independent stores
lanthorn keeps two kinds of saved state, owned by `app`, that are **never bundled together**:

1. **Quetzal game saves** — point-in-time VM snapshots, triggered by the game's `save`/`restore`.
   Stored as standard Quetzal `.qzl` files so they stay **portable** across interpreters. There may
   be several (slots).
2. **The map** — a *single, cumulative* per-IFID artifact (below). Not a snapshot: it accumulates
   everything discovered across all sessions and all save slots.

Keeping them separate is deliberate: bundling a map snapshot into each Quetzal save would break
Quetzal portability and fragment the map into divergent per-slot copies. A direct consequence is
that **restoring an old game save keeps your full map** — reloading never makes you "forget" rooms,
which is exactly what a mapper user wants. (Optionally tying a map view to a specific save point is a
possible future feature, not v1.)

### Map persistence

- Auto-build is the default behavior.
- **Persistence:** maps are saved per story file, keyed by **IFID**, and reload automatically when
  the same story is opened. Persisted data: the **layout mode** (Auto/Manual), rooms (identity,
  label override, notes), connections (including relabeled directions), and room positions — in
  Manual mode positions are authoritative; in Auto mode they are derived from the constraint graph
  and may be cached for fast reload. View state (zoom, scroll) is **not** persisted.
- **Manual corrections (the only editing in v1):**
  - rename a room (label override),
  - nudge a room to a free cell,
  - delete a bad connection,
  - relabel an "unknown direction" edge,
  - edit a room's notes.
- No room-adding or freehand drawing — that is a possible future full editor, out of scope for v1.

## 6. TUI Behavior

- **Layout:** split view (transcript + map) by default; map collapsible to full-screen transcript;
  map expandable to full-screen.
- **Current room:** always tracked; rendered with a clear "you are here" highlight (distinct
  border/color). Full-screen map can recenter on it.
- **Navigation (both side-pane and full-screen):** pan the viewport (arrows / `hjkl`) and zoom.
  Zoom is a set of discrete levels suited to a text grid:
  - closest: full room boxes with labels and connectors,
  - mid: smaller boxes with abbreviated labels,
  - overview: a single glyph per room for whole-map orientation.
  A key recenters on the current room. Zoom and scroll are view state, not persisted.

## 7. Image Export

A **headless map renderer**, separate from the TUI's text rendering, draws the map graph to a file
using the same graph + layout the TUI uses (so the export matches what is on screen).
- Default format: **SVG** (vector, crisp at any zoom, minimal dependencies).
- Optional: **PNG** (rasterized from the same render).
- Triggered by a key in the UI and/or a CLI flag.

## 8. Error Handling & Testing

- **VM correctness** is verified against the standard Z-machine regression suites (e.g.
  **CZECH** / **Praxix**) run as automated tests — the objective oracle for "is the interpreter
  right".
- **Mapper** is unit-tested with synthetic location/direction event streams (no VM needed),
  covering: non-reciprocal exits, grid collisions, unknown-direction edges, same-name/different-object
  rooms, revisited-room recognition, non-Euclidean/contradictory-constraint layouts (distorted-edge
  fallback, no room overlap), edge routing (departure side matches direction; paths avoid room cells;
  routing stable across incremental additions), and re-layout stability (minimal movement on
  incremental additions).
- **Unsupported inputs** (Z-machine v1/v2/v6, Glulx, corrupt files) fail fast with a clear message
  rather than mis-executing — with a specific note for v6 that graphical games are not supported.
