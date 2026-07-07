# Up/Down Placed Like N/S — Design

**Quest:** SQ-0216 — Treat up/down like N/S in map layout (dotted lines + up/down symbols, ignore reciprocal), but let reciprocal N/S shift up/down aside.

**Goal:** Promote vertical (Up/Down) connections from the weakest thing in the
layout to a first-class **weight-1 N/S directional hint**: an up/down move claims
the cell directly north (Up) / south (Down) of its neighbor and shoves ordinary
rooms aside to get it — while still yielding to *reciprocal* N/S adjacencies, and
without changing how up/down render (dotted lines + up/down glyphs, no arrows),
how they cut layers, or the fact that they never collapse reciprocally.

## Background: how up/down works today

Up/down is special-cased in three independent places:

1. **Placement is the weakest.** `place_incremental`
   (`crates/mapper/src/layout/incremental.rs:14-66`) gives Up the delta `(0,-1)`
   and Down `(0,1)` (same as N/S) but a `!updown` guard (line ~49) blocks the
   `shift_beyond` shove that a real cardinal uses on collision — up/down instead
   *yields* to `nearest_free_cell`. A late tidy stage `stack_updown_rooms`
   (`crates/app/src/render/map.rs:1963-2083`, observed variant ~2236, run at
   `crates/app/src/input.rs:1664` and in `tidy_layer_silent` ~1762) then tries to
   seat vertical rooms directly N/S *without shoving anyone*, guarded so it can't
   regress overlaps, directional-hint score, or exact alignment.

2. **Reciprocity is already a placement-time concept** — and up/down are already
   excluded from it. `detect_chains` (`crates/mapper/src/layout/chains.rs:19-43`)
   finds reciprocal (bidirectional) compass pairs; `build_axis_constraints`
   (`crates/mapper/src/layout/constraints.rs:54-80`) turns reciprocal chains into
   **hard equality constraints** (share a column for N/S, a row for E/W) added
   *before* the one-way directional constraints, so a conflicting one-way edge is
   the one dropped (`creates_cycle` → marked distorted), never the reciprocal
   pair. `RECIPROCAL_WEIGHT = 2` (`crates/mapper/src/layout/mod.rs:263`) makes a
   bidirectional link count double in seed placement (`edges_respected_at`
   ~265-300), `room_side_score` (~302-333), and `room_alignment_score` (~341-368),
   which the overlap/repair passes refuse to lower. The `stack_updown_rooms`
   `anchored` guard (`crates/app/src/render/map.rs:1988-1998`) already **refuses to
   pull an up/down room off a real axis-aligned cardinal neighbor** — so up/down
   already yields to reciprocal N/S.

3. **Rendering is separate and already dotted.** Up/down have `grid_offset == None`
   (`crates/mapper/src/direction.rs:66-68`), so they are stubbed by the router
   (`side_for` → None), excluded from the compass reciprocal-collapse
   (`crates/mapper/src/route/mod.rs` filters `grid_offset.is_some()`), and drawn by
   a dedicated path `draw_portal_connectors` (`crates/app/src/render/map.rs:1134-1214`)
   using dotted glyphs (`┊`/`┄`) + up/down symbols (↑/↓ or a stairs preset), with
   its own reciprocal dedupe (~1163-1172). `grid_offset == None` is also the cut
   used by `planar_region` (`crates/mapper/src/layer.rs:34-57`) to bound layers.

So the user's "dotted lines + up/down symbols instead of arrows" and "ignore
reciprocal for up/down" are **already true** and stay true for free — provided we
keep `grid_offset(Up/Down) == None`.

## Approach: layout-only

Keep `grid_offset(Up/Down) == None` (rendering, layers, reciprocal-collapse, and
the never-distorted property all unchanged). Introduce a **layout-only** notion of
an edge's directional pull that includes up/down, and apply it to the
placement/scoring stages only.

### 1. A soft layout offset

Add a helper (e.g. `layout_offset(dir) -> Option<(i32,i32)>`) =
`grid_offset(dir)` for compass, plus `Up => (0,-1)`, `Down => (0,1)`. `grid_offset`
itself is **unchanged**. `layout_offset` is used *only* by the
placement/scoring code below; render, router, `planar_region`, and `mark_distorted`
continue to use `grid_offset`.

### 2. Incremental placement shoves

In `place_incremental`, drop the `!updown` condition so an up/down move takes the
same `shift_beyond` path a cardinal takes on collision — claiming the directly
north/south cell and translating rooms beyond it aside. (The N/S delta is already
computed; only the shove is currently withheld.)

### 3. Up/down become weight-1 N/S directional hints

Switch the **directional-hint** machinery from `grid_offset` to `layout_offset`,
at weight 1, so up/down attract placement toward the N/S cell:

- `build_axis_constraints` directional loop (`constraints.rs`) — emit the one-way
  N/S inequality for up/down, *after* the reciprocal equalities (unchanged). The
  hard reciprocal equalities therefore outrank up/down automatically: on conflict
  the up/down hint is dropped, not the reciprocal column/row.
- `edges_respected_at`, `room_side_score`, `room_alignment_score`,
  `directional_hint_score` — count an up/down edge as a satisfied N/S side at
  weight 1. **Reciprocal weighting stays keyed on `grid_offset`**, so up/down never
  earn `RECIPROCAL_WEIGHT` even when both an Up and a Down edge exist between two
  rooms — they remain weight-1, matching "ignore reciprocal for up/down."
- Edge-satisfaction checks (`edge_is_satisfied` / `exact_alignment_count`,
  `crates/mapper/src/layout/mod.rs:158` and `crates/app/src/render/map.rs:2228`)
  also follow `layout_offset`, so once an up/down room is exactly N/S-aligned the
  overlap/repair guards protect that alignment the same way they protect a cardinal
  — but this protection is weight-1 and never overrides a reciprocal equality.

Result, with no new priority system: reciprocal N/S (hard equality, weight 2) >
up/down (weight-1 hint) ≥/= one-way cardinal (weight-1 hint) > unconstrained.

### 4. Retire `stack_updown_rooms`

Remove the late stacking stage (and its observed variant) from the pipeline in
`crates/app/src/input.rs` (both `run_tidy_pipeline` and `tidy_layer_silent`). Its
job — seat vertical rooms directly N/S — is now done by the unified directional
placement + `repair_directional_hints` path. Delete the stage's ~250 lines of
special-case stacking and its helpers if nothing else uses them.

## Decisions (flagged and accepted)

- **Up/down are a *soft* hint — never marked "distorted."** `mark_distorted` keeps
  using `grid_offset` (compass only), so an up/down link that can't land directly
  N/S renders as today's yielded dotted stub, not a red distorted edge.
- **Up/down vs a *one-way* cardinal is a tie (both weight 1).** Up/down bow only to
  *reciprocal* N/S; against an ordinary one-way N/S they compete on equal footing
  and existing tie-breakers (compass degree, order) decide.

## What explicitly does NOT change (Phase 1)

- `grid_offset(Up/Down)` stays `None`.
- Layers: `planar_region` still cuts on up/down; peel-layer unchanged.
- `mark_distorted` / the "distorted" red styling — unchanged.
- (Rendering and lane-routing DO change — see Phase 2 below.)

## Non-goals

- No change to `grid_offset` itself (Up/Down stay `None`); layer-cutting and the
  never-distorted property continue to key on it. Lane routing includes up/down via
  a routing-only `route_side` + `layout_offset`, not by changing `grid_offset`.
- No change to how many layers a vertical shaft occupies, or to manual peeling.
- No new config or style selectors (up/down already themeable via the existing
  `connector:portal` / portal-symbol selectors).

## Verification

Layout changes pass unit tests that share the implementation's own assumptions, so
a real-game smoke test is required (per project practice):

- **Unit:** a reciprocal N/S pair keeps its shared column when an up/down room
  contends for the same cell (asserts reciprocal > up/down); an up/down dest lands
  directly north/south of its neighbor and shoves an ordinary room aside (asserts
  the promoted shove); an up/down that cannot align does not get marked distorted.
- **Real game (oracle):** run a vertical-heavy multi-floor map through a headless
  step harness before/after and eyeball the layout — vertical shafts should read as
  clean N/S stacks, reciprocal compass rooms should keep their alignment, and no
  up/down edge should render as a red distorted arrow.

---

# Phase 2: Route up/down through the N/S lane system (rendering unification)

Phase 1 made up/down lay out like N/S but left them rendered by the separate
stub path (`draw_portal_connectors`): dotted stubs on the box's right column,
in-room icons, no lane routing. Phase 2 moves up/down onto the same **lane path**
compass connectors use, so their dotted connectors get lane assignment +
path-crossing elimination, a **border-centered** anchor, and the up/down symbol
drawn **on the room border** (where N/S arrowheads sit) — still dotted, still
no reciprocal-collapse.

## Routing (`crates/mapper/src/route/mod.rs`, `router.rs`)

- Add a routing-only `route_side(dir)` = `side_for(dir)` plus `Up => Top`,
  `Down => Bottom`. Use it in the lane router's exit-side lookups
  (`route/mod.rs:572, 634, 672`). Leave `side_for` and the old `route_all` stub
  router untouched.
- Change the lane-router working-set filter (`route/mod.rs:611`) from
  `grid_offset(c.dir).is_some()` to `layout_offset(c.dir).is_some()` — i.e.
  {compass + up/down}. Up/down now get `RoutedConnector`s, lanes, slots, and
  crossing elimination.
- **No reciprocal-collapse for up/down:** guard the back-edge pairing
  (`back_edge_idx` ~527, pairing ~667/805) so an up/down edge is never paired —
  each stays a one-way connector with `reciprocal = false` (which also suppresses
  the far-end arrow at render). Extend `oneway_entry_side` (~358) so a one-way
  up/down that dips into a channel has an entry side (Up enters dest from Bottom,
  Down from Top).
- `RoutedConnector.exit_dir` already carries `Up`/`Down` to the renderer — no new
  field required.

## Rendering (`crates/app/src/render/map.rs`)

- In `render_lane_connectors`, branch on `exit_dir == Up|Down`: draw the connector
  **body dotted** (portal `┊`/`┄`, selected per-connector) and put the **up/down
  glyph** (`sym.portal.up`/`down`, or the stairs preset) at the departure border
  anchor instead of `arrow_for_departure`. The anchor is the centered, slot-fanned
  `box_edge_anchor` (Top/Bottom), so the glyph sits mid-border and is pushed
  off-center only when a reciprocal N/S connector claims the center slot.
- **Delete** `draw_portal_connectors` and `portal_stub` (and their call in
  `render_map`). Make `draw_portal_icons` **skip Up/Down** — it keeps drawing the
  In/Out/Unknown in-room icons only.

## Phase 2 decisions (flagged and accepted)

- **In/Out/Unknown stay as in-room portal icons.** Only Up/Down move to the
  lane/border treatment.
- **Departure-glyph only** (like a one-way N/S arrow). A two-way vertical link is
  two un-collapsed connectors, so it shows an up-glyph on the lower room's top
  border and a down-glyph on the upper room's bottom border — one per direction.

## Phase 2 verification

- **Unit:** an up connection now produces a `RoutedConnector` in the plan (not a
  stub) with `reciprocal = false`; a reciprocal Up+Down pair produces **two**
  connectors, not one; rendering an up connector draws the up glyph on the border
  (not an arrow) and a dotted body; In/Out/Unknown still draw in-room icons.
- **Real game (oracle):** on a vertical-heavy map, vertical connectors route
  without breaking, don't cross other connectors, anchor at the middle of the
  top/bottom border (shifting for a reciprocal N/S), and show up/down glyphs on
  the border.
