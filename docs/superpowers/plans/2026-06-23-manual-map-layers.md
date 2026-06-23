# Manual Map Layers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a manual "layers" feature: the player peels a room's planar-connected region (cut at up/down/in/out portals) into its own named coordinate plane, with severed edges rendered as per-edge inter-layer portal badges; everything defaults to one layer ("Main") and renders identically to today.

**Architecture:** A `LayerId` is added to each `Room` (default `0`); `MapGraph` gains a layer-name/parent table and a counter. Rendering and routing become layer-aware through ONE new chokepoint — `render_layer(graph, layer)` builds a per-layer sub-graph (reusing the unchanged `route_all`/`route_lanes`) and appends inter-layer edges as portal badges. Peel/merge/region algorithms live in a new `mapper::layer` module. The app gains a viewed-layer, peel/merge/rename/cycle key bindings, and a layer tab/list.

**Tech Stack:** Rust workspace (`mapper`, `app` crates), ratatui 0.29 TUI, serde/serde_json persistence.

## Global Constraints

- **Default = identical to today.** A map with a single layer (everything in layer `0`) must render and route byte-for-byte as it does now. Layer `0`'s default display name is `"Main"`.
- **No auto-derivation.** Layers are created/destroyed ONLY by explicit peel/merge. New rooms always join the current room's layer. Up/Down/In/Out stay in-plane within a layer.
- **Peel = planar-connected region, cut at portals.** The peel set is rooms reachable from the selected room through planar edges only (`grid_offset(dir).is_some()`), stopping at portal edges (`Up Down In Out Unknown`). Peeling a region that is already its whole layer is a no-op.
- **No coordinate rebasing.** Peel/merge never change `Room.pos`. Layers are separated by filtering, not by renumbering. Two rooms in different layers may share a cell.
- **Inter-layer edges are per-edge, never per-layer-pair.** An edge is inter-layer iff its endpoints' layers differ (derived, not stored). Each renders as its own portal badge (glyph + destination room name + destination layer name) on BOTH endpoints' layers. Multiple links between two layers stay individually distinct.
- **Layer 0 is permanent** — it cannot be merged away or removed.
- **Backward compatibility:** a save with no layer fields loads as all-rooms-in-layer-0, name "Main".
- **Determinism:** identical operations/input → identical membership, ids, render, and dump.
- **Glyphs** reuse the existing portal glyph constants in `crates/app/src/render/map.rs` (`PORTAL_UP` etc. via `portal_glyph(dir)`).
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` (avoid backticks in the commit body — zsh command substitution).

---

## File Structure

- **Create** `crates/mapper/src/layer.rs` — `LayerId`, `LayerMeta`, and the peel/merge/region algorithms operating on `&mut MapGraph` / `&MapGraph`. One responsibility: layer membership logic.
- **Modify** `crates/mapper/src/graph.rs` — `Room.layer` field; `MapGraph` layer table (`layers`, `next_layer_id`) and accessors; extend `from_parts`.
- **Modify** `crates/mapper/src/lib.rs` — declare `pub mod layer;`.
- **Modify** `crates/mapper/src/render.rs` — add `render_layer(graph, layer)` and `layer_subgraph` use; inter-layer badge edges.
- **Modify** `crates/mapper/src/persist.rs` — persist layer fields with `#[serde(default)]`.
- **Modify** `crates/app/src/state.rs` — `viewed_layer` view state + helpers.
- **Modify** `crates/app/src/input.rs` — peel/merge/rename/cycle-layer actions + key bindings; route the map render through the viewed layer.
- **Modify** `crates/app/src/render/map.rs` — draw via `render_layer`; inter-layer badge text (room + layer name); a layer tab/list strip.
- **Modify** `crates/app/src/main.rs` — pass viewed layer to the render call (`render_map_data`).

The `direction.rs` planar/portal predicate already exists: a direction is **planar** iff `grid_offset(dir).is_some()`, **portal** iff `None` (covers `Up Down In Out Unknown`).

---

## Phase 1 — Data model + per-layer rendering chokepoint

### Task 1: `LayerId`, `LayerMeta`, and the layer module skeleton

**Files:**
- Create: `crates/mapper/src/layer.rs`
- Modify: `crates/mapper/src/lib.rs` (add `pub mod layer;` near the other `pub mod` lines)

**Interfaces:**
- Produces: `pub type LayerId = u16;` and `pub struct LayerMeta { pub name: String, pub parent: Option<LayerId> }` (both `Debug, Clone, serde::Serialize, serde::Deserialize`); `pub const MAIN_LAYER: LayerId = 0;`.

- [ ] **Step 1: Create the module with the types**

`crates/mapper/src/layer.rs`:
```rust
//! Map layers ("segments"): a manual organizing tool. Every room belongs to exactly
//! one layer (default `MAIN_LAYER`). Layers are created/destroyed only by explicit
//! peel/merge — never derived. See docs/superpowers/specs/2026-06-23-manual-map-layers-design.md.

use std::collections::BTreeSet;

use crate::direction::grid_offset;
use crate::graph::{MapGraph, RoomId};

/// Stable layer identifier. Layer `0` (`MAIN_LAYER`) always exists.
pub type LayerId = u16;

/// The permanent base layer every room starts in.
pub const MAIN_LAYER: LayerId = 0;

/// Per-layer metadata: a display name and the layer it was peeled from (for merge default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMeta {
    pub name: String,
    pub parent: Option<LayerId>,
}

impl LayerMeta {
    /// Metadata for the base "Main" layer.
    pub fn main() -> Self {
        LayerMeta { name: "Main".to_string(), parent: None }
    }
}
```

- [ ] **Step 2: Declare the module**

In `crates/mapper/src/lib.rs`, add alongside the existing `pub mod` declarations:
```rust
pub mod layer;
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p mapper`
Expected: builds with no errors (unused-import warnings for `BTreeSet`/`grid_offset`/`MapGraph`/`RoomId` are fine — later tasks use them; if the linter rejects, add `#![allow(unused)]`-free usage by deferring the imports to Task 4. Simplest: omit the `use` lines here and add them in Task 4).

- [ ] **Step 4: Commit**

```
git add crates/mapper/src/layer.rs crates/mapper/src/lib.rs
git commit -m "feat(mapper): add LayerId and LayerMeta types"
```

### Task 2: `Room.layer` field + `MapGraph` layer table & accessors

**Files:**
- Modify: `crates/mapper/src/graph.rs` (`Room` struct ~lines 7-14; `MapGraph` struct ~lines 33-38; `from_parts` ~lines 46-53; `upsert_room` ~lines 71-88)
- Test: `crates/mapper/src/graph.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::layer::{LayerId, LayerMeta, MAIN_LAYER}`.
- Produces on `MapGraph`:
  - `fn layer_of(&self, id: RoomId) -> LayerId`
  - `fn set_room_layer(&mut self, id: RoomId, layer: LayerId)`
  - `fn rooms_in_layer(&self, layer: LayerId) -> Vec<RoomId>` (sorted ascending)
  - `fn layers(&self) -> &BTreeMap<LayerId, LayerMeta>`
  - `fn layer_name(&self, layer: LayerId) -> &str` (empty `""` if unknown)
  - `fn set_layer_name(&mut self, layer: LayerId, name: String)`
  - `fn new_layer(&mut self, parent: Option<LayerId>, name: String) -> LayerId` (allocates `next_layer_id`, inserts meta)
  - `fn remove_layer(&mut self, layer: LayerId)` (removes meta; refuses `MAIN_LAYER`)
- `Room` gains `pub layer: LayerId` (serde `#[serde(default)]`).
- `from_parts` signature becomes: `from_parts(rooms, connections, current, layers: BTreeMap<LayerId, LayerMeta>, next_layer_id: LayerId)`.

- [ ] **Step 1: Write the failing test**

Add to `crates/mapper/src/graph.rs` tests module:
```rust
    #[test]
    fn rooms_default_to_main_layer_and_can_move() {
        use crate::layer::MAIN_LAYER;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_name(MAIN_LAYER), "Main");
        let l = g.new_layer(Some(MAIN_LAYER), "Basement".into());
        g.set_room_layer(2, l);
        assert_eq!(g.layer_of(2), l);
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1]);
        assert_eq!(g.rooms_in_layer(l), vec![2]);
        assert_eq!(g.layer_name(l), "Basement");
    }

    #[test]
    fn new_layer_ids_are_unique_and_main_cannot_be_removed() {
        let mut g = MapGraph::new();
        let a = g.new_layer(None, "A".into());
        let b = g.new_layer(None, "B".into());
        assert_ne!(a, b);
        g.remove_layer(crate::layer::MAIN_LAYER); // no-op
        assert_eq!(g.layer_name(crate::layer::MAIN_LAYER), "Main");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper graph::tests::rooms_default_to_main_layer_and_can_move`
Expected: FAIL — `layer_of`/`new_layer`/etc. not found, and `Room` has no `layer`.

- [ ] **Step 3: Add the `layer` field to `Room`**

In `crates/mapper/src/graph.rs`, change the `Room` struct and its construction sites:
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub label_override: Option<String>,
    pub notes: String,
    pub pos: Option<(i32, i32)>,
    #[serde(default)]
    pub layer: crate::layer::LayerId,
}
```
In `upsert_room`'s `Entry::Vacant` arm, add `layer: 0,` to the `Room { .. }` initializer.

- [ ] **Step 4: Add the layer table to `MapGraph` and accessors**

Add `use std::collections::BTreeMap;` (already present) and `use crate::layer::{LayerId, LayerMeta, MAIN_LAYER};`.
Change the struct and `from_parts`, and add accessors:
```rust
#[derive(Debug, Clone)]
pub struct MapGraph {
    rooms: BTreeMap<RoomId, Room>,
    conns: Vec<Connection>,
    current: Option<RoomId>,
    layers: BTreeMap<LayerId, LayerMeta>,
    next_layer_id: LayerId,
}

impl Default for MapGraph {
    fn default() -> Self {
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        Self { rooms: BTreeMap::new(), conns: Vec::new(), current: None, layers, next_layer_id: 1 }
    }
}
```
Replace the `#[derive(... Default ...)]` on `MapGraph` with `#[derive(Debug, Clone)]` (we now hand-write `Default`).
Update `from_parts`:
```rust
    pub fn from_parts(
        rooms: Vec<Room>,
        connections: Vec<Connection>,
        current: Option<RoomId>,
        layers: BTreeMap<LayerId, LayerMeta>,
        next_layer_id: LayerId,
    ) -> Self {
        let rooms = rooms.into_iter().map(|r| (r.id, r)).collect();
        let mut layers = layers;
        if layers.is_empty() {
            layers.insert(MAIN_LAYER, LayerMeta::main());
        }
        let next_layer_id = next_layer_id.max(1);
        Self { rooms, conns: connections, current, layers, next_layer_id }
    }
```
Add accessors (inside `impl MapGraph`):
```rust
    pub fn layer_of(&self, id: RoomId) -> LayerId {
        self.rooms.get(&id).map(|r| r.layer).unwrap_or(MAIN_LAYER)
    }
    pub fn set_room_layer(&mut self, id: RoomId, layer: LayerId) {
        if let Some(r) = self.rooms.get_mut(&id) { r.layer = layer; }
    }
    pub fn rooms_in_layer(&self, layer: LayerId) -> Vec<RoomId> {
        self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect()
    }
    pub fn layers(&self) -> &BTreeMap<LayerId, LayerMeta> { &self.layers }
    pub fn layer_name(&self, layer: LayerId) -> &str {
        self.layers.get(&layer).map(|m| m.name.as_str()).unwrap_or("")
    }
    pub fn set_layer_name(&mut self, layer: LayerId, name: String) {
        if let Some(m) = self.layers.get_mut(&layer) { m.name = name; }
    }
    pub fn new_layer(&mut self, parent: Option<LayerId>, name: String) -> LayerId {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.insert(id, LayerMeta { name, parent });
        id
    }
    pub fn remove_layer(&mut self, layer: LayerId) {
        if layer != MAIN_LAYER { self.layers.remove(&layer); }
    }
    pub fn next_layer_id(&self) -> LayerId { self.next_layer_id }
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p mapper graph::tests`
Expected: PASS. Then `cargo build` (the workspace) WILL fail at the two `from_parts` call sites (`persist.rs`) — that is fixed in Task 3; do not fix it here. If `cargo test -p mapper` fails to compile only due to `persist.rs`, proceed to Task 3 before re-running.

- [ ] **Step 6: Commit**

```
git add crates/mapper/src/graph.rs
git commit -m "feat(mapper): per-room layer field and MapGraph layer table"
```

### Task 3: Persist layer fields (backward compatible)

**Files:**
- Modify: `crates/mapper/src/persist.rs` (`PersistState` struct, `to_json`, `from_json`)
- Test: `crates/mapper/src/persist.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `MapGraph::{layers, next_layer_id, layer_of}`, `crate::layer::{LayerId, LayerMeta}`.
- `PersistState` gains `#[serde(default)] layers: BTreeMap<LayerId, LayerMeta>` and `#[serde(default)] next_layer_id: LayerId`; `from_json` passes them to `from_parts`. Because `Room.layer` is `#[serde(default)]`, a legacy save deserializes every room into layer 0.

- [ ] **Step 1: Write the failing test**

Add to `crates/mapper/src/persist.rs` tests:
```rust
    #[test]
    fn round_trips_layers() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let l = m.graph.new_layer(Some(0), "Basement".into());
        m.graph.set_room_layer(2, l);
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.layer_of(2), l);
        assert_eq!(m2.graph.layer_name(l), "Basement");
        assert_eq!(m2.graph.next_layer_id(), m.graph.next_layer_id());
    }

    #[test]
    fn legacy_save_without_layers_loads_as_main() {
        // A v1 save predating layers: no `layer` on rooms, no `layers`/`next_layer_id`.
        let json = r#"{"version":1,"mode":"Auto",
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).unwrap();
        assert_eq!(m.graph.layer_of(1), 0);
        assert_eq!(m.graph.layer_name(0), "Main");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper persist::tests::round_trips_layers`
Expected: FAIL to compile (`from_parts` arity / missing fields).

- [ ] **Step 3: Extend `PersistState` and the conversions**

In `crates/mapper/src/persist.rs`:
```rust
use std::collections::BTreeMap;
use crate::graph::{Connection, MapGraph, Room, RoomId};
use crate::layer::{LayerId, LayerMeta};
use crate::layout::LayoutMode;
use crate::mapper::Mapper;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistState {
    pub version: u32,
    pub mode: LayoutMode,
    pub rooms: Vec<Room>,
    pub connections: Vec<Connection>,
    pub current: Option<RoomId>,
    #[serde(default)]
    pub layers: BTreeMap<LayerId, LayerMeta>,
    #[serde(default)]
    pub next_layer_id: LayerId,
}

pub fn to_json(mapper: &Mapper) -> String {
    let state = PersistState {
        version: 1,
        mode: mapper.mode,
        rooms: mapper.graph.rooms().cloned().collect(),
        connections: mapper.graph.connections().to_vec(),
        current: mapper.graph.current(),
        layers: mapper.graph.layers().clone(),
        next_layer_id: mapper.graph.next_layer_id(),
    };
    serde_json::to_string_pretty(&state).expect("PersistState is always serializable")
}

pub fn from_json(s: &str) -> Result<Mapper, serde_json::Error> {
    let state: PersistState = serde_json::from_str(s)?;
    let graph = MapGraph::from_parts(
        state.rooms, state.connections, state.current, state.layers, state.next_layer_id,
    );
    Ok(Mapper { graph, mode: state.mode })
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p mapper persist::tests`
Expected: PASS (both new tests + the existing `round_trips_full_state`).

- [ ] **Step 5: Run the whole mapper suite (catch other `from_parts` callers)**

Run: `cargo test -p mapper`
Expected: PASS. If any other call site of `from_parts` exists, update it to pass `Default::default()` for `layers` and `1` for `next_layer_id`.

- [ ] **Step 6: Commit**

```
git add crates/mapper/src/persist.rs
git commit -m "feat(mapper): persist layer membership, names, and counter (back-compatible)"
```

### Task 4: `layer_subgraph` + `render_layer` (no-op for one layer)

**Files:**
- Modify: `crates/mapper/src/graph.rs` (add `layer_subgraph`)
- Modify: `crates/mapper/src/render.rs` (add `render_layer`)
- Test: `crates/mapper/src/render.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `MapGraph::layer_subgraph(&self, layer: LayerId) -> MapGraph` — a graph containing only `layer`'s rooms (with `pos`, `current` preserved iff the current room is in `layer`) and only connections whose BOTH endpoints are in `layer`.
- Produces: `pub fn render_layer(graph: &MapGraph, layer: LayerId) -> RenderMap` in `render.rs`.
- For a single-layer graph, `render_layer(graph, 0)` returns a `RenderMap` equal to `render(graph)`.

- [ ] **Step 1: Write the failing test**

Add to `crates/mapper/src/render.rs` tests:
```rust
    #[test]
    fn render_layer_matches_render_for_single_layer() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let all = render(&m.graph);
        let only = render_layer(&m.graph, 0);
        assert_eq!(only.rooms.len(), all.rooms.len());
        assert_eq!(only.bounds, all.bounds);
        assert_eq!(only.edges.len(), all.edges.len());
    }

    #[test]
    fn render_layer_shows_only_its_layer() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let l = m.graph.new_layer(Some(0), "Other".into());
        m.graph.set_room_layer(2, l);
        let main = render_layer(&m.graph, 0);
        assert!(main.rooms.iter().any(|r| r.id == 1));
        assert!(!main.rooms.iter().any(|r| r.id == 2), "room 2 lives in another layer");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper render::tests::render_layer_matches_render_for_single_layer`
Expected: FAIL — `render_layer` not found.

- [ ] **Step 3: Implement `layer_subgraph` on `MapGraph`**

In `crates/mapper/src/graph.rs`:
```rust
    /// A sub-graph containing only `layer`'s rooms and the connections whose BOTH
    /// endpoints are in `layer`. Positions are preserved; `current` carries over only
    /// if the current room is in `layer`. Layer metadata is not copied (not needed for routing).
    pub fn layer_subgraph(&self, layer: LayerId) -> MapGraph {
        let in_layer: std::collections::BTreeSet<RoomId> =
            self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect();
        let rooms: BTreeMap<RoomId, Room> = self
            .rooms
            .values()
            .filter(|r| in_layer.contains(&r.id))
            .cloned()
            .collect();
        let conns: Vec<Connection> = self
            .conns
            .iter()
            .filter(|c| in_layer.contains(&c.origin) && in_layer.contains(&c.dest))
            .cloned()
            .collect();
        let current = self.current.filter(|id| in_layer.contains(id));
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        MapGraph { rooms, conns, current, layers, next_layer_id: 1 }
    }
```

- [ ] **Step 4: Implement `render_layer`**

In `crates/mapper/src/render.rs`, add (and `use crate::layer::LayerId;`):
```rust
/// Build a `RenderMap` for a single layer. Rooms and grid connectors come from the
/// layer's sub-graph (so the existing routers are reused unchanged). Inter-layer edges
/// (Phase 2) are appended by `interlayer_badges`, which is empty while there is one layer.
pub fn render_layer(graph: &MapGraph, layer: LayerId) -> RenderMap {
    let sub = graph.layer_subgraph(layer);
    let mut rm = render(&sub);
    rm.edges.extend(crate::layer::interlayer_badges(graph, layer));
    rm
}
```
Add a stub in `crates/mapper/src/layer.rs` (filled in Phase 2):
```rust
use crate::router::RoutedEdge;

/// Portal-badge edges for connections leaving `layer` to another layer. Empty in Phase 1.
pub fn interlayer_badges(_graph: &MapGraph, _layer: LayerId) -> Vec<RoutedEdge> {
    Vec::new()
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p mapper render::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```
git add crates/mapper/src/graph.rs crates/mapper/src/render.rs crates/mapper/src/layer.rs
git commit -m "feat(mapper): render_layer chokepoint over a per-layer sub-graph"
```

---

## Phase 2 — Peel, merge, region, and inter-layer badges

### Task 5: `planar_region` and `peel_region`

**Files:**
- Modify: `crates/mapper/src/layer.rs`
- Test: `crates/mapper/src/layer.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `MapGraph::{connections, layer_of, set_room_layer, new_layer, rooms_in_layer}`, `crate::direction::grid_offset`.
- Produces:
  - `pub fn planar_region(graph: &MapGraph, start: RoomId) -> BTreeSet<RoomId>` — rooms reachable from `start` via planar edges (`grid_offset(c.dir).is_some()`) within `start`'s current layer, treating edges as undirected; stops at portal edges.
  - `pub fn peel_region(graph: &mut MapGraph, start: RoomId) -> Option<LayerId>` — computes `planar_region(start)`; if it equals all of `start`'s current layer, returns `None` (no-op); otherwise allocates a new layer (parent = `start`'s current layer, name = `start`'s room label) and moves the region into it; returns the new `LayerId`.

- [ ] **Step 1: Write the failing tests**

Add to `crates/mapper/src/layer.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_floors() -> MapGraph {
        // Floor 1: 1-E-2 (planar). 1-Down-3 (portal) to floor 2: 3-E-4 (planar).
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Study"), (3, "Cellar"), (4, "Wine")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        g
    }

    #[test]
    fn planar_region_stops_at_portals() {
        let g = two_floors();
        let region = planar_region(&g, 3);
        let want: BTreeSet<RoomId> = [3, 4].into_iter().collect();
        assert_eq!(region, want, "down-portal cuts the cellar off from the upper floor");
    }

    #[test]
    fn peel_region_moves_floor_to_new_layer() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).expect("a proper sub-region peels");
        assert_eq!(g.layer_of(3), l);
        assert_eq!(g.layer_of(4), l);
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_of(2), MAIN_LAYER);
        assert_eq!(g.layer_name(l), "Cellar");
    }

    #[test]
    fn peel_whole_layer_is_noop() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        assert_eq!(peel_region(&mut g, 1), None, "region is the whole layer → no-op");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper layer::tests::planar_region_stops_at_portals`
Expected: FAIL — functions not found.

- [ ] **Step 3: Implement `planar_region` and `peel_region`**

In `crates/mapper/src/layer.rs` (ensure imports: `use std::collections::{BTreeSet, VecDeque};`, `use crate::direction::grid_offset;`, `use crate::graph::{MapGraph, RoomId};`):
```rust
/// Rooms reachable from `start` through PLANAR edges only (cardinals + diagonals),
/// staying within `start`'s current layer. Portal edges (Up/Down/In/Out/Unknown) are
/// not traversed — they are the cut. Edges are treated as undirected for reachability.
pub fn planar_region(graph: &MapGraph, start: RoomId) -> BTreeSet<RoomId> {
    let layer = graph.layer_of(start);
    let mut seen = BTreeSet::new();
    seen.insert(start);
    let mut q = VecDeque::new();
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        for c in graph.connections() {
            if grid_offset(c.dir).is_none() {
                continue; // portal edge → cut
            }
            let other = if c.origin == cur {
                c.dest
            } else if c.dest == cur {
                c.origin
            } else {
                continue;
            };
            if graph.layer_of(other) == layer && seen.insert(other) {
                q.push_back(other);
            }
        }
    }
    seen
}

/// Peel `start`'s planar region into a fresh layer. Returns the new `LayerId`, or `None`
/// when the region already spans the whole source layer (nothing to separate).
pub fn peel_region(graph: &mut MapGraph, start: RoomId) -> Option<LayerId> {
    let src = graph.layer_of(start);
    let region = planar_region(graph, start);
    let whole_layer: BTreeSet<RoomId> = graph.rooms_in_layer(src).into_iter().collect();
    if region == whole_layer {
        return None;
    }
    let name = graph.room(start).map(|r| r.label().to_string()).unwrap_or_default();
    let new = graph.new_layer(Some(src), name);
    for id in region {
        graph.set_room_layer(id, new);
    }
    Some(new)
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p mapper layer::tests`
Expected: PASS (all three).

- [ ] **Step 5: Commit**

```
git add crates/mapper/src/layer.rs
git commit -m "feat(mapper): planar_region and peel_region (cut at portal edges)"
```

### Task 6: Inter-layer detection + badge edges

**Files:**
- Modify: `crates/mapper/src/layer.rs` (`interlayer_badges`, plus `is_interlayer`)
- Test: `crates/mapper/src/layer.rs`

**Interfaces:**
- Consumes: `MapGraph::{connections, layer_of, room, layer_name}`, `Room::{pos, label}`, `crate::router::{RoutedEdge}`, `crate::router::fine_cell` (verify export; if private, replicate the `*2`-doubled mapping used in `router.rs`'s `fine_cell`).
- Produces:
  - `pub fn is_interlayer(graph: &MapGraph, conn: &Connection) -> bool` — endpoints in different layers.
  - `pub fn interlayer_badges(graph: &MapGraph, layer: LayerId) -> Vec<RoutedEdge>` — for every connection with exactly one endpoint in `layer`, a stub `RoutedEdge` anchored at that endpoint, `is_stub: true`, `label = Some(direction-aware stub label)`, `dest_label = Some("<room name> · <dest layer name>")`. Per-edge — never merged.

- [ ] **Step 1: Write the failing test**

Add to `crates/mapper/src/layer.rs` tests:
```rust
    #[test]
    fn interlayer_badges_are_per_edge() {
        // Two staircases between the same two layers must yield two distinct badges.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "HallN"), (2, "HallS"), (3, "CellarN"), (4, "CellarS")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::E, 2); // keep them in one planar region up top
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3); // staircase A
        g.add_edge(2, Direction::Down, 4); // staircase B
        let l = peel_region(&mut g, 3).expect("peel cellar");
        g.set_room_layer(4, l); // ensure both cellar rooms in the new layer
        g.set_pos(3, (0, 0));
        g.set_pos(4, (0, 1));
        let up = interlayer_badges(&g, MAIN_LAYER);
        assert_eq!(up.len(), 2, "two independent staircases → two badges");
        assert!(up.iter().all(|e| e.is_stub));
        // peel_region(3) named the layer after room 3's label ("CellarN"); badge text is
        // "<dest room> · <dest layer>". Assert the shape, not a brittle literal.
        assert!(up.iter().all(|e| e.dest_label.as_deref().is_some_and(|s| s.contains(" · "))));
        assert!(up.iter().any(|e| e.dest_label.as_deref() == Some("CellarN · CellarN")));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper layer::tests::interlayer_badges_are_per_edge`
Expected: FAIL — `interlayer_badges` still returns empty / `is_interlayer` missing.

- [ ] **Step 3: Implement detection + badges**

Replace the Phase-1 stub `interlayer_badges` in `crates/mapper/src/layer.rs`:
```rust
use crate::graph::Connection;
use crate::router::{stub_label, RoutedEdge}; // verify `stub_label` visibility; see note below

/// True iff the connection's endpoints are in different layers.
pub fn is_interlayer(graph: &MapGraph, conn: &Connection) -> bool {
    graph.layer_of(conn.origin) != graph.layer_of(conn.dest)
}

/// One portal badge per connection that crosses out of `layer`. Anchored at the
/// in-layer endpoint; carries the destination room name and destination layer name.
pub fn interlayer_badges(graph: &MapGraph, layer: LayerId) -> Vec<RoutedEdge> {
    let mut out = Vec::new();
    for c in graph.connections() {
        if !is_interlayer(graph, c) {
            continue;
        }
        // Determine the in-layer endpoint and the remote endpoint.
        let (here, there, dir) = if graph.layer_of(c.origin) == layer {
            (c.origin, c.dest, c.dir)
        } else if graph.layer_of(c.dest) == layer {
            (c.dest, c.origin, c.dir)
        } else {
            continue;
        };
        let Some(here_pos) = graph.room(here).and_then(|r| r.pos) else { continue };
        let fine = (here_pos.0 * 2, here_pos.1 * 2); // doubled-coords anchor (matches router::fine_cell)
        let dest_layer = graph.layer_of(there);
        let dest_label = graph
            .room(there)
            .map(|r| format!("{} · {}", r.label(), graph.layer_name(dest_layer)));
        out.push(RoutedEdge {
            origin: here,
            dest: there,
            dir,
            points: vec![fine, (fine.0, fine.1 - 1)],
            distorted: false,
            is_stub: true,
            label: Some(stub_label(dir).to_string()),
            arrival_dir: None,
            dest_label,
        });
    }
    out
}
```
**Implementation note for the implementer:** `RoutedEdge`, `fine_cell`, and `stub_label` live in `crates/mapper/src/router.rs`. Confirm `RoutedEdge`'s exact field set (origin, dest, dir, points, distorted, is_stub, label, arrival_dir, dest_label) by reading the struct (~line 126) and that `stub_label`/`fine_cell` are reachable (make them `pub(crate)` if not). Use `fine_cell(here_pos)` instead of the inline `*2` if it is exported, to stay DRY.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p mapper layer::tests`
Expected: PASS.

- [ ] **Step 5: Run the full mapper suite**

Run: `cargo test -p mapper`
Expected: PASS (no regressions; single-layer maps still emit zero inter-layer badges).

- [ ] **Step 6: Commit**

```
git add crates/mapper/src/layer.rs crates/mapper/src/router.rs
git commit -m "feat(mapper): inter-layer edge detection and per-edge portal badges"
```

### Task 6b: Layer-scoped incremental placement (inherit layer; shift within layer)

**Files:**
- Modify: `crates/mapper/src/layout/incremental.rs` (`place_incremental` ~lines 14-62; `shift_beyond` ~lines 66+)
- Modify: `crates/mapper/src/layout/mod.rs` (add `occupied_cells_in_layer`; `occupied_cells` ~the existing helper)
- Test: `crates/mapper/src/layout/incremental.rs` (`#[cfg(test)] mod tests`)

**Why:** After a peel, layers are independent planes whose cells may overlap. Placement must (a) put a new room in the current room's layer, and (b) compute occupied cells and shift-beyond **within that layer only**, or it will move other layers' rooms and corrupt their planes. With a single layer this is a no-op.

**Interfaces:**
- Consumes: `MapGraph::{layer_of, set_room_layer, rooms_in_layer}`.
- Produces: `pub fn occupied_cells_in_layer(graph: &MapGraph, layer: LayerId) -> BTreeSet<(i32,i32)>` in `layout/mod.rs`. `place_incremental` derives `layer = graph.layer_of(prev)`, assigns `dest` to it, and scopes occupancy/shift to it. `shift_beyond` gains a `layer: LayerId` parameter and only moves rooms in that layer.

- [ ] **Step 1: Write the failing test**

Add to `crates/mapper/src/layout/incremental.rs` tests:
```rust
    #[test]
    fn new_room_inherits_prev_layer() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let l = g.new_layer(Some(0), "B".into());
        g.set_room_layer(1, l);
        g.upsert_room(2, "C".into());
        place_incremental(&mut g, 1, 2, Direction::E);
        assert_eq!(g.layer_of(2), l, "new room joins the previous room's layer");
    }

    #[test]
    fn shift_beyond_does_not_move_other_layers() {
        // Two layers share cells. Placing/shifting in layer 0 must not move the layer-1 room.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into());
        g.set_pos(2, (1, 0)); // east of 1, same cell-line, layer 0
        g.add_edge(1, Direction::E, 2);
        let l = g.new_layer(Some(0), "Other".into());
        g.upsert_room(9, "X".into());
        g.set_room_layer(9, l);
        g.set_pos(9, (1, 0)); // SAME cell as room 2, but different layer
        // Insert a new room east of 1: forces a shift-beyond of room 2 within layer 0.
        g.upsert_room(3, "New".into());
        place_incremental(&mut g, 1, 3, Direction::E);
        assert_eq!(g.room(9).unwrap().pos, Some((1, 0)), "other-layer room must not move");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper layout::incremental::tests::shift_beyond_does_not_move_other_layers`
Expected: FAIL — room 9 was shifted (it shares a cell and `shift_beyond` moves all layers).

- [ ] **Step 3: Add `occupied_cells_in_layer` and scope placement**

In `crates/mapper/src/layout/mod.rs`, beside `occupied_cells`:
```rust
/// Occupied grid cells among rooms in `layer` only.
pub fn occupied_cells_in_layer(graph: &MapGraph, layer: crate::layer::LayerId) -> BTreeSet<(i32, i32)> {
    graph.rooms().filter(|r| r.layer == layer).filter_map(|r| r.pos).collect()
}
```
In `place_incremental`: after the early returns, add `let layer = graph.layer_of(prev); graph.set_room_layer(dest, layer);`. Replace each `occupied_cells(graph)` with `occupied_cells_in_layer(graph, layer)`, and the `shift_beyond(graph, ideal, delta)` call with `shift_beyond(graph, ideal, delta, layer)`. Import `occupied_cells_in_layer`.

- [ ] **Step 4: Scope `shift_beyond` to the layer**

Change its signature to `fn shift_beyond(graph: &mut MapGraph, ideal: (i32, i32), step: (i32, i32), layer: crate::layer::LayerId)` and skip rooms not in `layer`:
```rust
    let ids: Vec<RoomId> = graph.rooms().filter(|r| r.layer == layer).map(|r| r.id).collect();
```
(Replace the existing `graph.rooms().map(...)` collect.)

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p mapper layout::incremental::tests` then `cargo test -p mapper`
Expected: PASS (single-layer placement tests unchanged; new layer-scoping tests pass).

- [ ] **Step 6: Commit**

```
git add crates/mapper/src/layout/incremental.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): scope incremental placement to the room's layer"
```

---

## Phase 3 — Viewed layer + tab/list + view switching

### Task 7: Viewed-layer state and render through it

**Files:**
- Modify: `crates/app/src/state.rs` (`AppState` struct ~lines 134-160; `Default` ~lines 162-180)
- Modify: `crates/app/src/main.rs` (the `render_map_data(&mapper.graph)` call ~line 471 and the tidy-anim branch ~line 78)
- Modify: `crates/app/src/render/map.rs` (the entry that calls `mapper::render::render`; ~line 1241 and `render_map_data`)
- Test: `crates/app/src/state.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `mapper::render::render_layer`, `mapper::layer::{LayerId, MAIN_LAYER}`, `MapGraph::{layer_of, current, layers}`.
- Produces on `AppState`:
  - `pub viewed_layer: Option<LayerId>` (None = follow the current room's layer).
  - `fn active_layer(&self, graph: &MapGraph) -> LayerId` — `viewed_layer` if set and still present, else the current room's layer, else `MAIN_LAYER`.
  - `fn set_viewed_layer(&mut self, layer: Option<LayerId>)`.
- The render path uses `render_layer(graph, state.active_layer(graph))` instead of `render(graph)`.

- [ ] **Step 1: Write the failing test**

Add to `crates/app/src/state.rs` tests:
```rust
    #[test]
    fn active_layer_follows_current_then_view_override() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_current(1);
        let l = g.new_layer(Some(0), "B".into());
        let mut s = AppState::default();
        assert_eq!(s.active_layer(&g), 0, "defaults to current room's layer");
        s.set_viewed_layer(Some(l));
        assert_eq!(s.active_layer(&g), l, "explicit view wins");
        s.set_viewed_layer(Some(999)); // stale id (no such layer)
        assert_eq!(s.active_layer(&g), 0, "stale view falls back to current room's layer");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app state::tests::active_layer_follows_current_then_view_override`
Expected: FAIL — field/methods missing.

- [ ] **Step 3: Add the field, default, and methods**

In `crates/app/src/state.rs`, add to `AppState`: `pub viewed_layer: Option<mapper::layer::LayerId>,` and `viewed_layer: None,` in `Default`. Add methods:
```rust
    pub fn set_viewed_layer(&mut self, layer: Option<mapper::layer::LayerId>) {
        self.viewed_layer = layer;
    }
    pub fn active_layer(&self, graph: &mapper::graph::MapGraph) -> mapper::layer::LayerId {
        use mapper::layer::MAIN_LAYER;
        if let Some(l) = self.viewed_layer {
            if graph.layers().contains_key(&l) {
                return l;
            }
        }
        graph.current().map(|id| graph.layer_of(id)).unwrap_or(MAIN_LAYER)
    }
```

- [ ] **Step 4: Route the renderer through the active layer**

In `crates/app/src/render/map.rs`, change the `render_map_data` body (the `mapper::render::render(graph)` call ~line 1241) so the function takes the active layer and calls `render_layer`. Update its signature to `pub fn render_map_data(graph: &MapGraph, layer: mapper::layer::LayerId) -> RenderMap` and call `mapper::render::render_layer(graph, layer)`. In `crates/app/src/main.rs`, update both call sites (`render_map_data(&mapper.graph)` ~line 471, and the tidy-anim branch ~line 78 which renders `frame.graph`) to pass `state.active_layer(&mapper.graph)` (for the anim frame, pass the layer active when the anim started — use `state.active_layer(&frame.graph)`).
**Note:** the `.map.txt` dump (`map_dump.rs`) and SVG/DOT exports keep calling `render(graph)` (all layers) for now; per-layer dump is out of scope for this plan.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p app state::tests` then `cargo test -p app`
Expected: PASS. With one layer, `active_layer` is 0 and `render_layer(g,0) == render(g)`, so all existing render tests stay green.

- [ ] **Step 6: Commit**

```
git add crates/app/src/state.rs crates/app/src/main.rs crates/app/src/render/map.rs
git commit -m "feat(app): viewed-layer state; render through the active layer"
```

### Task 7b: Scope re-tidy to the active layer

**Files:**
- Modify: `crates/app/src/input.rs` (`run_tidy_pipeline` ~lines 277-307; the `Action::Retidy` apply arm ~lines 378-386)
- Test: `crates/app/src/input.rs` (`#[cfg(test)] mod tests`)

**Why:** `relayout_auto` and the cleanup passes treat the whole graph as one plane. After a peel, re-tidy must reshuffle ONLY the active layer, leaving other layers' positions untouched. With one layer this is identical to today.

**Interfaces:**
- Consumes: `MapGraph::{layer_subgraph, rooms_in_layer, room, set_pos}`, `AppState::active_layer`.
- `run_tidy_pipeline` signature becomes `run_tidy_pipeline(graph: &mut MapGraph, layer: LayerId) -> Vec<TidyFrame>`: it tidies a `layer_subgraph(layer)`, snapshots frames from that sub-graph, and writes the sub-graph's final positions back into `graph` for the layer's rooms. Frames are sub-graph clones (the animation shows just the tidied layer).

- [ ] **Step 1: Write the failing test**

Add to `crates/app/src/input.rs` tests:
```rust
    #[test]
    fn retidy_only_moves_the_active_layer() {
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        // Layer 0: a 3-room tangle that relayout will move.
        g.upsert_room(1, "A".into()); g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into()); g.set_pos(2, (5, 5));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        // Layer 1: a room with a fixed position that must NOT move.
        let l = g.new_layer(Some(0), "Other".into());
        g.upsert_room(9, "X".into()); g.set_room_layer(9, l); g.set_pos(9, (3, 3));
        let _frames = run_tidy_pipeline(&mut g, l); // tidy the OTHER layer
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)), "layer-0 room 1 untouched");
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "layer-0 room 2 untouched");
        // Room 9 is the only room in layer l → relayout anchors it at the origin.
        assert_eq!(g.room(9).unwrap().pos, Some((0, 0)), "lone room in tidied layer is anchored");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app input::tests::retidy_only_moves_the_active_layer`
Expected: FAIL — `run_tidy_pipeline` takes one arg / tidies all layers.

- [ ] **Step 3: Re-scope `run_tidy_pipeline`**

Rewrite it to operate on a sub-graph and write back:
```rust
pub(crate) fn run_tidy_pipeline(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
) -> Vec<crate::state::TidyFrame> {
    use crate::render::map::{cleanup_overlaps, compact_empty_lines, repair_directional_hints, stack_updown_rooms};
    use crate::state::TidyFrame;

    let mut sub = graph.layer_subgraph(layer);
    let mut frames = vec![TidyFrame { label: "before".into(), graph: sub.clone() }];
    let snap = |g: &mapper::graph::MapGraph, label: &str, frames: &mut Vec<TidyFrame>| {
        frames.push(TidyFrame { label: label.into(), graph: g.clone() });
    };
    mapper::layout::relayout_auto(&mut sub);
    snap(&sub, "relayout", &mut frames);
    cleanup_overlaps(&mut sub, 3, 40);
    snap(&sub, "cleanup overlaps", &mut frames);
    repair_directional_hints(&mut sub, 3, 40);
    snap(&sub, "repair hints", &mut frames);
    stack_updown_rooms(&mut sub);
    snap(&sub, "stack up/down", &mut frames);
    cleanup_overlaps(&mut sub, 3, 40);
    snap(&sub, "cleanup overlaps", &mut frames);
    compact_empty_lines(&mut sub);
    snap(&sub, "compact", &mut frames);

    // Write the tidied positions back into the live graph for this layer's rooms.
    for id in graph.rooms_in_layer(layer) {
        if let Some(p) = sub.room(id).and_then(|r| r.pos) {
            graph.set_pos(id, p);
        }
    }
    frames
}
```

- [ ] **Step 4: Update the `Action::Retidy` apply arm**

In the `Action::Retidy` arm (~line 378), pass the active layer to both the instant and animated calls:
```rust
        Action::Retidy => {
            let layer = state.active_layer(&mapper.graph);
            if /* instant path condition, unchanged */ {
                run_tidy_pipeline(&mut mapper.graph, layer);
            } else {
                let frames = run_tidy_pipeline(&mut mapper.graph, layer);
                /* existing frame-install code, unchanged */
            }
        }
```
(Preserve the existing branch structure; only thread `layer` through.)

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p app input::tests` then `cargo test -p app`
Expected: PASS (existing single-layer tidy tests still pass — `active_layer` is 0 and the sub-graph equals the whole graph).

- [ ] **Step 6: Commit**

```
git add crates/app/src/input.rs
git commit -m "feat(app): scope re-tidy to the active layer"
```

### Task 8: Cycle/select layer + auto-switch on inter-layer move + tab strip

**Files:**
- Modify: `crates/app/src/input.rs` (`Action` enum ~line 33; map-focus key table ~lines 230-260; the action-apply match ~line 378)
- Modify: `crates/app/src/render/map.rs` (a one-line layer tab/list strip)
- Modify: `crates/app/src/main.rs` (after a turn, set viewed layer to the current room's layer)
- Test: `crates/app/src/input.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `AppState::set_viewed_layer`, `MapGraph::{layers, layer_of, current}`.
- Produces: `Action::CycleLayer(i32)` (delta over the sorted layer-id list of non-empty layers); a key binding (`[` / `]` in map focus) → `CycleLayer(-1)` / `CycleLayer(1)`; the apply arm cycles `viewed_layer` across present layers.

- [ ] **Step 1: Write the failing test**

Add to `crates/app/src/input.rs` tests:
```rust
    #[test]
    fn bracket_keys_cycle_layer_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, plain(KeyCode::Char(']'))), Action::CycleLayer(1)));
        assert!(matches!(key_to_action(&s, plain(KeyCode::Char('['))), Action::CycleLayer(-1)));
    }
```
(Use the test crate's existing key-event helper; if it is named differently than `plain`, match the file's convention — see how other map-focus binding tests build a `KeyEvent`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app input::tests::bracket_keys_cycle_layer_in_map_focus`
Expected: FAIL — `Action::CycleLayer` missing.

- [ ] **Step 3: Add the action, binding, and apply logic**

Add `CycleLayer(i32)` to the `Action` enum. In the map-focus key table (the block with `KeyCode::Char('R') if shift => Action::Retidy`), add:
```rust
        KeyCode::Char(']') => Action::CycleLayer(1),
        KeyCode::Char('[') => Action::CycleLayer(-1),
```
In the apply-action match (where `Action::Retidy` is handled, ~line 378), add an arm that reads the sorted list of layer ids that currently contain ≥1 room, finds the active layer's index, steps by the delta (clamped or wrapped — choose clamp for determinism), and calls `state.set_viewed_layer(Some(new_id))`:
```rust
        Action::CycleLayer(delta) => {
            let mut ids: Vec<_> = mapper.graph.layers().keys().copied()
                .filter(|&l| !mapper.graph.rooms_in_layer(l).is_empty())
                .collect();
            ids.sort_unstable();
            if !ids.is_empty() {
                let cur = state.active_layer(&mapper.graph);
                let i = ids.iter().position(|&l| l == cur).unwrap_or(0) as i32;
                let j = (i + delta).clamp(0, ids.len() as i32 - 1) as usize;
                state.set_viewed_layer(Some(ids[j]));
            }
        }
```

- [ ] **Step 4: Auto-switch the view when the player crosses layers in-game**

In `crates/app/src/main.rs`, after a turn updates the current room (where `mapper.observe`/turn application happens), set the viewed layer to follow the player: `state.set_viewed_layer(None);` (None already means "follow current room's layer", so the simplest correct behavior is to clear any manual browse override on a real move). Add a test-backed helper if the turn path is unit-tested; otherwise this is a one-line change at the turn-application site.

- [ ] **Step 5: Draw the layer tab/list strip**

In `crates/app/src/render/map.rs`, draw a single-row strip at the top of the map pane listing each non-empty layer as `name(count)`, with the active layer highlighted (reverse video via `Style`). Use the existing `draw_str_clipped` helper from `render/mod.rs`. Keep it to one line; if only layer 0 exists, draw nothing (so single-layer maps are visually unchanged). Gate the strip on `Boxes`/`Compact` zoom (skip in `Overview`).

- [ ] **Step 6: Run to verify it passes**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 7: Commit**

```
git add crates/app/src/input.rs crates/app/src/render/map.rs crates/app/src/main.rs
git commit -m "feat(app): layer tab strip, cycle-layer keys, auto-follow on move"
```

---

## Phase 4 — Peel/merge/rename bindings + persistence round-trip

### Task 9: Peel and merge actions

**Files:**
- Modify: `crates/app/src/input.rs` (`Action` enum; map-focus key table; apply match)
- Test: `crates/app/src/input.rs`

**Interfaces:**
- Consumes: `mapper::layer::{peel_region, merge_layer}`, `AppState::{selected_room, set_viewed_layer}`, `MapGraph::layer_of`.
- Produces: `Action::PeelLayer` and `Action::MergeLayer`; bindings `KeyCode::Char('P')`/`KeyCode::Char('M')` in map focus (uppercase, shift). Peel acts on `state.selected_room` (fall back to `graph.current()`); on success switches the view to the new layer. Merge folds the active layer into its parent.
- Requires `merge_layer` from Task 10's interface; implement Task 10 first OR stub `merge_layer` here and complete it in Task 10. (Recommended order: do Task 10's `merge_layer` function before wiring this `MergeLayer` arm.)

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn shift_p_peels_and_shift_m_merges_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::PeelLayer));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::MergeLayer));
    }
```
(Match the file's existing shift-key event helper.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app input::tests::shift_p_peels_and_shift_m_merges_in_map_focus`
Expected: FAIL — actions missing.

- [ ] **Step 3: Add actions, bindings, and apply arms**

Add `PeelLayer` and `MergeLayer` to `Action`. Bindings in the map-focus table:
```rust
        KeyCode::Char('P') if shift => Action::PeelLayer,
        KeyCode::Char('M') if shift => Action::MergeLayer,
```
Apply arms (near `Action::Retidy`):
```rust
        Action::PeelLayer => {
            if let Some(room) = state.selected_room.or_else(|| mapper.graph.current()) {
                if let Some(new) = mapper::layer::peel_region(&mut mapper.graph, room) {
                    state.set_viewed_layer(Some(new));
                }
            }
        }
        Action::MergeLayer => {
            let active = state.active_layer(&mapper.graph);
            mapper::layer::merge_layer(&mut mapper.graph, active); // merges into parent (Task 10)
            state.set_viewed_layer(None);
        }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p app input::tests`
Expected: PASS (binding tests; the apply arms compile once `merge_layer` exists).

- [ ] **Step 5: Commit**

```
git add crates/app/src/input.rs
git commit -m "feat(app): peel (Shift+P) and merge (Shift+M) layer bindings"
```

### Task 10: `merge_layer` (fold a layer into its parent)

**Files:**
- Modify: `crates/mapper/src/layer.rs`
- Test: `crates/mapper/src/layer.rs`

**Interfaces:**
- Consumes: `MapGraph::{rooms_in_layer, set_room_layer, layers, remove_layer}`, `LayerMeta::parent`.
- Produces: `pub fn merge_layer(graph: &mut MapGraph, layer: LayerId) -> LayerId` — reassigns every room in `layer` to its `parent` (default `MAIN_LAYER` if no parent), removes `layer`'s metadata, and returns the target. A no-op returning `MAIN_LAYER` when `layer == MAIN_LAYER`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn merge_layer_folds_into_parent_and_removes_meta() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).unwrap(); // parent = MAIN_LAYER
        let target = merge_layer(&mut g, l);
        assert_eq!(target, MAIN_LAYER);
        assert_eq!(g.layer_of(3), MAIN_LAYER);
        assert_eq!(g.layer_of(4), MAIN_LAYER);
        assert!(!g.layers().contains_key(&l), "merged layer's metadata is removed");
    }

    #[test]
    fn merge_main_is_noop() {
        let mut g = two_floors();
        assert_eq!(merge_layer(&mut g, MAIN_LAYER), MAIN_LAYER);
        assert!(g.layers().contains_key(&MAIN_LAYER));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper layer::tests::merge_layer_folds_into_parent_and_removes_meta`
Expected: FAIL — `merge_layer` not found.

- [ ] **Step 3: Implement `merge_layer`**

```rust
/// Fold every room in `layer` into its parent (or `MAIN_LAYER` if it has none) and drop
/// the layer's metadata. Returns the target layer. No-op (returns `MAIN_LAYER`) for `MAIN_LAYER`.
pub fn merge_layer(graph: &mut MapGraph, layer: LayerId) -> LayerId {
    if layer == MAIN_LAYER {
        return MAIN_LAYER;
    }
    let target = graph.layers().get(&layer).and_then(|m| m.parent).unwrap_or(MAIN_LAYER);
    for id in graph.rooms_in_layer(layer) {
        graph.set_room_layer(id, target);
    }
    graph.remove_layer(layer);
    target
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p mapper layer::tests`
Expected: PASS. Then `cargo test -p app` to confirm the Task 9 `MergeLayer` arm now links.

- [ ] **Step 5: Commit**

```
git add crates/mapper/src/layer.rs
git commit -m "feat(mapper): merge_layer folds a layer into its parent"
```

### Task 11: Rename a layer + full persistence round-trip

**Files:**
- Modify: `crates/app/src/state.rs` (`PromptKind` enum ~lines 83-89 — add `RenameLayer(LayerId)`)
- Modify: `crates/app/src/input.rs` (a `Shift+N` binding in map focus → start the rename prompt for the active layer; the prompt-submit handler that maps `PromptKind` to a graph mutation)
- Test: `crates/app/src/input.rs` and an end-to-end persistence test in `crates/app/src/persist_files.rs`

**Interfaces:**
- Consumes: `MapGraph::set_layer_name`, the existing prompt sub-mode (`Prompt`, `PromptKind`), `save_map`/`load_map`.
- Produces: `PromptKind::RenameLayer(LayerId)`; on submit, `graph.set_layer_name(id, buffer)`.

- [ ] **Step 1: Write the failing persistence test**

Add to `crates/app/src/persist_files.rs` tests:
```rust
    #[test]
    fn save_load_round_trips_layers_and_names() {
        use mapper::direction::Direction;
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-layers-{}", std::process::id()));
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let l = mapper::layer::peel_region(&mut m.graph, 2).expect("peel");
        m.graph.set_layer_name(l, "Basement".into());
        save_map(&path, &m).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.layer_of(2), l);
        assert_eq!(loaded.graph.layer_name(l), "Basement");
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app persist_files::tests::save_load_round_trips_layers_and_names`
Expected: PASS already IF peel works through save (it should — Task 3 persists layers). If it FAILS, fix the persistence gap before continuing. (This test guards the end-to-end app save path, not just the mapper unit.)

- [ ] **Step 3: Add `RenameLayer` to the prompt sub-mode and wire submit**

Add `RenameLayer(mapper::layer::LayerId)` to `PromptKind`. Add a `Shift+N` binding in map focus that returns an action starting the prompt seeded with the active layer's current name (mirror how `RenameRoom` opens its prompt). In the prompt-submit handler, add the `PromptKind::RenameLayer(id) => mapper.graph.set_layer_name(id, buffer)` arm.

- [ ] **Step 4: Add a binding-level test**

```rust
    #[test]
    fn shift_n_starts_layer_rename_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // Match the action your binding emits (e.g. Action::RenameLayer or a prompt-open action).
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::RenameLayer));
    }
```

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 6: Commit**

```
git add crates/app/src/state.rs crates/app/src/input.rs crates/app/src/persist_files.rs
git commit -m "feat(app): rename layer (Shift+N); end-to-end layer persistence test"
```

### Task 12: Inter-layer badge rendering in the TUI

**Files:**
- Modify: `crates/app/src/render/map.rs` (the portal-badge / stub draw path — `draw_portal_icons`/`draw_stub`/`draw_portal_connectors`, ~lines 338-376, 742-890)
- Test: `crates/app/src/render/map.rs`

**Interfaces:**
- Consumes: `RenderMap.edges` now includes inter-layer stub `RoutedEdge`s (from `render_layer` → `interlayer_badges`), each with `is_stub: true` and `dest_label: Some("<room> · <layer>")`.
- Produces: the renderer draws those inter-layer stubs using the SAME badge path as existing portal stubs, so an inter-layer link shows `glyph + dest_label` beside its in-layer room.

- [ ] **Step 1: Write the failing test**

Add a render test that builds a two-layer graph, peels, renders the main layer via `render_map_data(&g, 0)`, draws into a `Buffer`, and asserts the destination layer name text (e.g. `"Basement"`) appears in the buffer. Mirror an existing buffer-assertion test in this file (search for tests that render into a `Buffer` and scan cells).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app render::map::tests::<your_test_name>`
Expected: FAIL — inter-layer badge text not drawn (the stub path may need to read `dest_label` for inter-layer edges specifically).

- [ ] **Step 3: Implement**

In the stub/badge draw path, ensure stub edges whose `dest_label` is set render the destination text (room · layer). Since inter-layer edges are produced as `is_stub` `RoutedEdge`s identical in shape to portal stubs, the existing badge drawing should already pick them up; the change is to confirm the badge text uses `dest_label` (already carries `room · layer`) and that inter-layer stubs are not filtered out by any "planar only" guard. Add only what the failing test requires.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Manual visual check (non-blocking)**

Run the TUI on a real story, peel a floor (`Shift+P` on a cellar room), confirm: the cellar disappears from the main layer, a `↓ <room> · <layer>` badge appears on the stair room, `]`/`[` switch layers, the tab strip lists both, and `Shift+M` merges back. (This is a human check, not an automated test.)

- [ ] **Step 6: Commit**

```
git add crates/app/src/render/map.rs
git commit -m "feat(app): render inter-layer edges as portal badges with destination layer"
```

---

## Notes for the implementer

- **Run order:** Task 10 (`merge_layer`) is referenced by Task 9's apply arm; implement Task 10's function before compiling Task 9's `MergeLayer` arm (or land them together).
- **Verify `RoutedEdge` fields** by reading `crates/mapper/src/router.rs` (~line 126) before Task 6 — the field list in this plan must match exactly, and `fine_cell`/`stub_label` may need `pub(crate)`.
- **Key-event test helpers:** `crates/app/src/input.rs` already has helpers for building `KeyEvent`s in tests (used by existing binding tests). Reuse them; the `plain(...)`/`shift(...)`/`ctrl(...)` names in this plan are placeholders for whatever that file already uses.
- **Single-layer invariant:** after every task, `cargo test -p mapper && cargo test -p app` must stay green, because all defaults keep a one-layer map identical to today.
