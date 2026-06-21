# Lane-Based Connector Routing — Design Spec

**Date:** 2026-06-20
**Branch:** `fix-tui-runtime`
**Status:** Approved (design) — awaiting spec review

## Goal

Make every connection on the map a **clean, separate, traceable line**. Replace the
per-edge A* router (which falls back to overlapping "unrouted" grey lines when a dense
map congests) with **channel/lane routing**: edges run in reserved lanes within the
gaps between rooms, gaps grow to fit the lanes they carry, and the only path meetings
are perpendicular crossings. No two connectors ever overlap or run alongside each
other, on any map.

## Background

On dense, mostly-distorted maps (e.g. A129 with 14 of 19 edges distorted) the current
renderer scatters connected rooms and forces congested edges to the DarkGray unrouted
fallback, which overlaps and is untraceable. The user cannot tell how clusters of
rooms connect. The fix is real orthogonal channel routing with dynamically-sized gaps.

## Decisions (from brainstorming)

1. **Dynamic gaps** — each row/column gap widens to exactly fit the lanes routing
   through it. Guarantees zero unrouted/overlap on any map; the map spreads more on
   busy maps (more pan/zoom), accepted.
2. **Split layout + renderer** — `mapper` owns the *logical* channel router (lane
   indices, channel traffic; no pixels). `app` owns *pixel realization* (gap widths,
   non-uniform room positions, drawing).
3. **Boxes-zoom only** — lane routing applies at Boxes zoom (the detailed view with
   real gutters). Compact (8×3) and Overview (1×1) keep today's uniform-stride
   schematic rendering, unchanged.
4. **Crossing minimization deferred** — v1 guarantees lane *separation* and legal
   perpendicular crossings; it does not also reorder lanes to minimize the *count* of
   crossings. (Follow-up.)
5. **Line-art rendering** — connectors draw as 1-cell box-drawing polylines, not solid
   ribbons. The solid-ribbon renderer and the entire unrouted/DarkGray/`▒` concept are
   removed.

## Architecture

```
mapper:
  relayout_auto(graph)            → room cells (unchanged)
  route_lanes(graph) -> RoutePlan ← NEW: logical channel router + lane assignment

app (Boxes zoom only):
  RoutePlan + room cells
    → channel pixel widths (lanes × spacing)
    → cumulative non-uniform room positions
    → draw line-art connectors along lanes
  (replaces route_ortho, the A* search, the unrouted fallback, the solid ribbons,
   and the uniform-stride cell→pixel math at Boxes zoom)

map_dump:
  near-direct buffer→text copy (line-art already in the buffer; mask code removed)
```

## Channel model (logical)

Rooms sit on the integer grid at `(col, row)` after `relayout_auto`. Define:

- **Horizontal channel `H[r]`** — the gap *below* room-row `r` (between rows `r` and
  `r+1`). Horizontal connector segments run here. An edge leaving a room's bottom enters
  `H[r]`; leaving the top enters `H[r-1]`.
- **Vertical channel `V[c]`** — the gap to the *right* of room-column `c` (between cols
  `c` and `c+1`). Vertical segments run here. Right side → `V[c]`; left side → `V[c-1]`.

A **segment** is one axis-aligned run of an edge, tagged with its channel and its
**extent** (the inclusive range of the perpendicular coordinate it spans — for a
horizontal segment in `H[r]`, the column range; for a vertical segment in `V[c]`, the
row range). A **lane** is an index within a channel; segments in the same channel on
the same lane must have disjoint extents.

## Stage 1 — Route topology (`mapper`)

For each drawn (compass) edge `A(cA,rA) → B(cB,rB)`:

1. **Exit side** = the compass departure side of `A` (`side_for(dir)`), as today.
2. **Entry side** = `B`'s closest free side facing `A` (the existing closest-location
   arrival logic, lifted to the logical grid).
3. Build a **Manhattan route** of ≤3 segments connecting `A`'s exit to `B`'s entry,
   alternating channels and turning only at channel intersections. The canonical forms:
   - exit horizontal + 1 vertical + enter horizontal (a "Z" when `A`,`B` differ on both
     axes), or a single bend "L" when they share a row or column, or a straight run when
     adjacent and aligned.
   - The vertical run uses the column-channel adjacent to whichever box it turns at; the
     horizontal runs use the row-channels at `A`'s and `B`'s rows.
4. Emit the segments with their channels and extents. Stubs (Up/Down/In/Out/Unknown)
   are excluded (rendered as labelled stubs, unchanged).

Reciprocal pairs (`A→B` and `B→A`) collapse to one routed connector (drawn once with an
arrow at each end), matching today's reciprocal-reuse.

The exact segment construction per (exit side, entry side) combination is enumerated in
the plan; the invariant is that the route is a valid orthogonal path from `A`'s exit
anchor to `B`'s entry anchor that never enters a room cell.

## Stage 2 — Lane assignment (`mapper`)

Per channel, collect its segments and assign lanes by the **left-edge (interval
graph) algorithm**: sort segments by extent start; greedily place each in the lowest
lane whose occupant intervals don't overlap it. Result: each segment has a lane index;
each channel has `lane_count` = lanes used. **Overlapping segments are guaranteed
distinct lanes**, so they never coincide in pixels — the no-overlap guarantee.

`RoutePlan` (the mapper output) is:

```rust
struct LaneSeg { channel: Channel, lane: u16, /* endpoints in grid+lane terms */ }
enum Channel { H(i32), V(i32) }   // H(r) or V(c)
struct RoutedConnector { origin: RoomId, dest: RoomId, distorted: bool,
                         exit: Side, entry: Side, segs: Vec<LaneSeg> }
struct RoutePlan {
    connectors: Vec<RoutedConnector>,
    h_lanes: BTreeMap<i32, u16>,   // H[r] → lane_count
    v_lanes: BTreeMap<i32, u16>,   // V[c] → lane_count
}
```

All integer/logical — no pixels. Deterministic (sorted inputs, greedy).

## Stage 3 — Pixel realization (`app`, Boxes zoom)

- `BOX = (11,5)`, `LANE_SPACING` (cells between adjacent lanes, e.g. 2 so lines are
  visually separable), `MIN_GUTTER` (minimum gap even when a channel is empty, e.g. 2).
- **Channel width:** `V[c]` width = `max(MIN_GUTTER, v_lanes[c] × LANE_SPACING)`;
  `H[r]` height = `max(MIN_GUTTER, h_lanes[r] × LANE_SPACING)`.
- **Room positions:** non-uniform cumulative sums. Column `c`'s pixel-x = Σ over columns
  `< c` of `(BOX.w + V[col].width)`. Rows analogous with `H`. A `cell→pixel` lookup
  (prefix-sum tables) replaces the uniform `cell × stride` at Boxes zoom; scroll/pan use
  the same tables.
- **Drawing:** each `LaneSeg` is drawn at its lane's pixel offset inside its channel as a
  1-cell box-drawing polyline; turns at channel corners render `┌┐└┘`, straight runs
  `─`/`│`, T-junctions `├┤┬┴` where an edge's own segments meet, and perpendicular
  crossings of two *different* connectors render `┼`. Foreground color: Cyan (normal) /
  Magenta (distorted). Arrowheads `▶◀▲▼` (filled, outgoing compass direction) at each
  departure anchor; reciprocal connectors also draw the far-end arrow.

## Data flow

Per Boxes-zoom frame the app obtains both `mapper::render(graph) -> RenderMap` (room
cells, as today) and `mapper::route_lanes(graph) -> RoutePlan`, and passes both into
`render_map(rm: &RenderMap, plan: &RoutePlan, state, area, buf)`. The app builds
prefix-sum position tables from the plan's channel widths, positions rooms, and draws
line-art connectors along the plan's lanes. No A*, no unrouted state, no solid ribbons.

## What is removed

- `route_ortho` and its A* (and the Tier-1 / unrouted `Option<None>` path).
- `PATH_BG`, `PATH_BG_DISTORTED`, `PATH_BG_UNROUTED` solid-ribbon styles and the
  ribbon blit; `unrouted_l`, `unrouted_cells`.
- `map_dump.rs` mask machinery: `is_path`, `is_unrouted`, `mask_glyph`, the NESW
  reconstruction — replaced by a direct buffer→text copy.
- At Boxes zoom: the uniform-stride `cell_to_virtual` math (replaced by prefix-sum
  tables). Compact/Overview keep uniform stride.

## What is kept

- `relayout_auto` and its repairs (routability, crossing-aware) — still position rooms;
  lane routing no longer *needs* them for correctness, but they keep the map compact and
  reduce crossings, so they stay.
- Arrow semantics (outgoing compass direction; reciprocal far-end arrow).
- The closest-free-side arrival choice (now computed on the logical grid for entry side).
- `Zoom::steps`/`zoom_box_size` for Compact/Overview; box `11×5` for Boxes.

## Testing

`mapper` (`route_lanes`):
- **Core invariant:** for every channel, no two segments sharing a lane have
  overlapping extents (the no-overlap guarantee).
- Every drawn edge yields a complete route from origin exit to dest entry; reciprocal
  pairs collapse to one connector.
- `h_lanes`/`v_lanes` counts equal the max lane used per channel.
- A hand-checked small graph (e.g. 3 rooms, 2 edges sharing a channel → 2 lanes).
- Deterministic: same graph → identical `RoutePlan`.

`app`:
- **0 overlapping connector cells and 0 unrouted cells** on the A129 graphs (the
  acceptance gate — replaces the old crossing/unrouted probes).
- Room positions follow cumulative gap widths (a busy channel pushes later rooms
  further than an empty one).
- Connectors render as box-drawing glyphs (not background ribbons); arrowheads present;
  distorted edges Magenta.
- Scroll/pan at Boxes zoom stays geometry-consistent (virtual-space invariance).

End-to-end: the reported A129 clusters (`#79/#78/#76/#74`, `#78/#143/#77/#75`) render
with each connection a distinct traceable line.

## Out of Scope

- Crossing-count minimization (lane reordering) — separation only in v1.
- Lane routing at Compact/Overview zoom.
- Changing `relayout_auto` / the layout repairs.
- Bridge/"jump" glyphs at crossings (plain `┼` in v1).

## Limitations (accepted)

- Busy maps spread out more (dynamic gaps); pan/zoom absorbs it.
- v1 may show more perpendicular crossings than a crossing-minimizing router would;
  they're legal and traceable, just not minimal.
- `LANE_SPACING`/`MIN_GUTTER` are fixed constants; tuning is a follow-up if maps feel
  too sparse or too tight.
