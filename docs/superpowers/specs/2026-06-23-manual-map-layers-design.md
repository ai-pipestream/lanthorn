# Manual Map Layers — Design Spec

**Date:** 2026-06-23
**Branch:** `main`
**Status:** Approved (design) — awaiting spec review
**Revises:** the *segment* model in `2026-06-21-incremental-segmented-automapping-design.md`
(Phases 3–4 of that plan). The layout regimes (Phase 1) and diagonals/portals/multi-edge
(Phase 2) of that design are implemented and merged; this spec replaces its **auto-derived
segments** with a **manual layers** model.

## Goal

Give the player a manual tool to organize a map into **layers** (a.k.a. segments): a layer
is a separately-viewable coordinate plane. By default the whole map is one layer and renders
exactly as it does today (up/down/in/out in-plane, disconnected areas packed side-by-side).
On demand, the player **peels** a region into its own layer — to separate a building's floors
or a genuinely disconnected area — and the edges crossing the cut become **inter-layer
portals**.

## Why this revises the 2026-06-21 segment model

The earlier design made segments **auto-derived**: up/down/in/out were portals that
automatically started a new segment, so following `down` spun up a fresh plane. The work that
actually shipped (Phase 2, `2026-06-22-updown-placement-dotted`) took the opposite, simpler
route — Up/Down rooms are placed **in-plane** (NW/SW of origin) with dotted connectors and
`↑`/`↓` badges, keeping everything on one visible plane. That is good default behavior and we
keep it.

This spec resolves the resulting conflict by making layering **fully manual**:

- **No auto-derivation.** Nothing splits on its own. Up/down/in/out stay in-plane. Genuinely
  disconnected components stay packed in one layer until the player acts.
- **Manual peel/merge** is the only way layers are created or destroyed.
- The motivations are **vertical structure** (peel a floor) and **disconnected areas** (peel a
  separate building/dream world) — both served by one peel operation.

## Layer model

- **Definition:** a layer is an explicit set of rooms sharing one coordinate plane. Every room
  belongs to exactly one layer. There is no derivation — membership is authoritative state.
- **Default:** every room starts in **layer 0**, display name **"Main"** (renameable). A fresh
  map, and every map today, is a single layer that renders identically to current behavior.
- **Per-layer coordinate planes:** a room's `pos` is interpreted **within its layer**. Rooms in
  different layers may occupy the same cell — they are never rendered together. Consequently
  **peeling does not rebase coordinates**: positions are kept as-is and the renderer/layout
  simply filter to one layer.
- **New rooms** discovered during play always join the **current room's layer**. Layers change
  only by manual peel/merge — never by gameplay alone.

## Operations

### Peel (region at portal cuts)

The one creative act. The player selects a room and peels:

1. Compute the **planar-connected region** of the selected room: rooms reachable through
   **planar edges only** (`N S E W NE NW SE SW`), stopping at **portal edges**
   (`Up Down In Out Unknown`). This is the "floor" or disconnected area containing the room.
2. If the region is the room's entire current layer (no portal edge crosses out of it), peeling
   is a **no-op** (report "nothing to peel — this region is already its own layer"); do not
   create an empty source layer.
3. Otherwise assign every room in the region to a **new layer** with a fresh `LayerId` and a
   default name (the selected room's name). Positions are unchanged.
4. Every edge with one endpoint in the peeled region and the other outside it is now an
   **inter-layer edge**. There may be **many** (multiple staircases, an `up` *and* a `down`
   between the same pair, several distinct severed room-pairs); each is handled independently
   (see Inter-layer edges).

### Merge

The player selects a layer (or a room in it) and merges its rooms back into a **target layer**
(default: that layer's `parent`, falling back to layer 0). Membership is
reassigned; the previously inter-layer edges become intra-layer again (rendered in-plane, e.g.
the dotted up/down connector returns). Position collisions in the target are resolved by the
existing placement/collision machinery; the merged layer may then be re-tidied (`R`) by the
player. Merging layer 0 away is disallowed (it is the permanent base).

### Rename

A layer's display name is editable, reusing the existing rename-prompt sub-mode
(`PromptKind`). Layer 0's name defaults to "Main" and is renameable like any other.

## Inter-layer edges

- An edge is **inter-layer** iff its two endpoints are in different layers. This is a derived
  property (compare endpoint layers), not stored.
- **Per-edge, never per-layer-pair.** Two layers may be joined by any number of edges; each is
  an independent link. Nothing collapses multiple links into one or assumes a single connection.
- **Rendering:** an inter-layer edge draws as a **portal badge** (the Phase 2 renderer) on
  **both** endpoints' layers, carrying its own destination text — direction glyph + destination
  **room name** + destination **layer name** (e.g. `↓ Cellar · Basement`). Because each badge
  names its specific target room, multiple links to the same layer remain individually
  distinguishable.
- **Routing/layout ignore inter-layer edges:** the lane router and the placement/re-tidy passes
  operate within a single layer and treat an inter-layer edge as a non-grid stub (the same way
  portals are already excluded from grid routing).

## Display

- The map pane shows **one layer at a time**: the **current room's layer** by default, or a
  layer the player is browsing.
- A **layer tab/list** shows each layer's name and room count, current highlighted; a key cycles
  or selects an entry, switching the viewed layer **without moving the player**.
- Taking an inter-layer edge **in-game** (the player walks through a severed portal)
  **auto-switches the viewed layer** to the destination room's layer, consistent with following
  the current room.
- Compact/Overview zooms render the viewed layer the same way, filtered to its rooms.

## Data model

- `LayerId` (newtype over `u16`).
- `Room.layer: LayerId` — authoritative, default `0`.
- `MapGraph` gains: an ordered map `LayerId → { name, parent: Option<LayerId> }`, and a
  `next_layer_id` counter for stable fresh ids. Peel records the source layer as the new layer's
  `parent` (so merge can default back to it). Layer 0 ("Main", `parent: None`) always exists.
- **Active layer** = the layer of the current room (derived, not stored). **Viewed layer** =
  active layer unless the player is browsing (transient UI state).

## Persistence

- **Persist:** `Room.layer` per room, the `LayerId → { name, parent }` map, and `next_layer_id`.
  Room positions are already persisted (per-layer interpretation needs no format change beyond
  the added fields).
- **Do not persist:** the browsing viewed-layer (it resets to the active layer on load).
- Backward compatibility: a save with no layer fields loads as all-rooms-in-layer-0, "Main" —
  identical to today.

## Architecture & components

- **mapper (`graph.rs`):** `LayerId`, `Room.layer`, layer-name map + counter, helpers:
  `rooms_in_layer(id)`, `layer_of(room)`, `is_interlayer(conn)`, `planar_region(room)` (the peel
  set), `peel_region(room) -> LayerId`, `merge_layer(src, dst)`, `rename_layer(id, name)`.
- **mapper (layout/route):** `relayout_auto`, incremental placement, and the lane router take a
  layer filter (operate on `rooms_in_layer`). Existing single-component logic is reused per
  layer; inter-layer edges are skipped as non-grid.
- **app (render):** the renderer draws only the viewed layer's rooms and intra-layer connectors,
  and emits a portal badge for every inter-layer edge touching the viewed layer. The layer
  tab/list is a new pane element; new key bindings drive peel, merge, rename, cycle/select layer.
- **app (state/persistence):** viewed-layer state; serialize the new graph fields.

Each unit has one responsibility and a small interface; the renderer never computes membership
(it asks the graph), and the graph never renders.

## Testing strategy

**mapper**
- `planar_region` stops at portal edges: a two-floor graph linked only by `up`/`down` returns
  exactly one floor; a diagonal-linked region is included whole.
- `peel_region` assigns the region to a new layer, leaves the rest in the source, and is a no-op
  when the region already is the whole layer.
- **Multiple inter-layer edges:** peeling a floor joined by two staircases (and a reciprocal
  up/down pair) yields multiple independent inter-layer edges, each detected by `is_interlayer`.
- Positions are unchanged by peel (no rebasing); two layers may share a cell with no overlap
  error.
- `merge_layer` reassigns membership, restores intra-layer edges, refuses to remove layer 0.
- Layout/route on a peeled layer consider only that layer's rooms; an inter-layer edge is not
  grid-routed.
- Determinism: same operations → same membership and ids.

**app**
- Renderer shows only the viewed layer; an inter-layer edge renders as a portal badge on both
  ends with destination room + layer name; two links to one layer show as two distinct badges.
- Cycling the layer list changes the viewed layer without moving the player; taking an
  inter-layer edge in-game auto-switches the viewed layer.
- Peel/merge/rename key bindings perform the graph operation and update the view.
- Persistence round-trip: layer membership, names, and counter survive save/load; a legacy save
  loads as one "Main" layer.

## Implementation phasing (one spec; phases in the plan)

1. **Data model + per-layer filtering.** `LayerId`, `Room.layer` (default 0), layer-name map +
   counter; placement/re-tidy/route/render filter by layer. Persistence of the new fields.
   Behavior with one layer is byte-identical to today.
2. **Peel + inter-layer badges.** `planar_region`/`peel_region`; inter-layer detection; portal
   badges for severed edges (both ends), handling multiple links per layer-pair.
3. **Layer view + tab/list.** Viewed-layer state, the tab/list pane, cycle/select bindings, and
   auto-switch when an inter-layer edge is taken in-game.
4. **Merge + rename.** `merge_layer`, `rename_layer`, their bindings, and the full
   persistence/round-trip including legacy-save compatibility.

## Out of scope / non-goals

- Auto-derivation of layers (the explicit reversal of the 2026-06-21 model).
- Coordinate rebasing on peel/merge.
- An all-layers-tiled view (chosen: one layer + tab/list).
- Explicit room-by-room multi-select for peeling (chosen: region-at-portal-cuts).
- Changes to the lane router internals, the zvm bridge, or the Quetzal save path.

## Risks & limitations (accepted)

- **Merge collisions.** Re-merged rooms can land on occupied cells; resolved by existing
  collision handling and an optional follow-up re-tidy, not guaranteed pretty.
- **Stale browsing view.** If the viewed (browsed) layer is merged away, the view falls back to
  the active layer.
- **Peel granularity.** Region-at-portal-cuts cannot split a single planar-connected region;
  that is the deliberate trade for one-keystroke floor peeling (a future explicit multi-select
  could lift it).
