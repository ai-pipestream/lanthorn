# Diagonal Corners, Portal Badges & Multi-Edge Merge — Design Spec

**Date:** 2026-06-22
**Branch:** (to be created off `main` @ current head)
**Status:** Approved (design) — awaiting spec review
**Builds on:** the merged chain-alignment work (lane router in `mapper/route`, box render in `app/render/map.rs`, dump in `app/map_dump.rs`).

## Goal

Three independent rendering improvements to the Boxes-zoom map, driven by user feedback on the A129 house:

1. **Diagonal corners** — show NE/NW/SE/SW edges with a diagonal arrow at the box *corner*, instead of collapsing them to a cardinal arrow that hides the diagonal sense.
2. **Portal badges** — render Up/Down/In/Out connections as a labeled badge (glyph + target room name) instead of a bare `U`/`D` letter, and keep the design ready for future multi-layer navigation.
3. **Multi-edge merge** — when several edges connect the same room pair, draw every exit from each box (via the existing edge arrows) but **merge their connector lines into a single trunk** to the destination, instead of one tangled line per edge.

These are independent; the implementation plan may sequence them but they don't depend on each other.

## Non-goals (this spec)

- Actual multi-layer maps / inter-layer navigation (portals stay single-layer badges; only the *affordance* is designed to be layer-ready).
- Compact/Overview zoom changes — all three features target **Boxes zoom** only (Compact/Overview keep current behavior).
- Teaching the greedy crossing-minimizer about even-coord direct runs (separate deferred item).

---

## Feature 1 — Diagonal corners

### Current behavior
`router::side_for` maps NE/NW→`Top`, SE/SW→`Bottom`, so a diagonal edge exits a cardinal side and renders with a `▲`/`▼` arrow. The diagonal direction is lost in the glyph (layout still places the room diagonally via `grid_offset`).

### Design
A diagonal edge's departure/arrival glyph is a **diagonal arrow that replaces the box's corner glyph** — the same way a cardinal arrow replaces an edge glyph today:

| Direction | Corner replaced | Default glyph |
|-----------|-----------------|---------------|
| NE | top-right (`╮`/`┐`) | `↗` |
| NW | top-left (`╭`/`┌`) | `↖` |
| SE | bottom-right (`╯`/`┘`) | `↘` |
| SW | bottom-left (`╰`/`└`) | `↙` |

```
╭─────────↗        ╭─────────╮
│#77      │        │#143     │
╰─────────╯        ↙─────────╯
```

- Glyph set is a single named constant (`↗↖↘↙`), swappable to filled wedges (`◥◤◣◢`) in one place if a font renders those better.
- The connector **attaches at that corner cell** and routes orthogonally through the lane system to the destination (no true diagonal line — big boxes + gutters make a clean diagonal impossible and it would cut through routing channels). The corner is a new exit/arrival anchor in addition to the four side-centers.
- Both ends of a reciprocal diagonal pair get the corner arrow (departure on origin, arrival on dest).
- The corner glyph is only replaced when a diagonal edge actually attaches there; otherwise the box keeps its normal corner.

### Components touched
- `mapper/route` (or `router.rs`): a diagonal exit/entry resolves to a *corner* anchor, not a side-center. The lane route starts/ends at the corner cell.
- `app/render/map.rs`: draw the diagonal arrow glyph at the box corner cell (replacing `╭╮╰╯`) for diagonal departures/arrivals; the connector's first/last segment meets that corner.

---

## Feature 2 — Portal badges

### Current behavior
`route_all` emits `is_stub` edges for Up/Down/In/Out/Unknown with a one-letter `label` (`U`/`D`/`IN`/`OUT`/`?`); `draw_stub` writes just that letter in the box's top-right gutter. No target identity.

### Design
Render each portal as a **badge** beside the room box:

```
╭─────────╮ ↑ Attic        ↑ up-portal → #201 "Attic"
│#203     │ ↓ S of House   (stacks downward for multiple)
│         │
╰─────────╯
```

- **Glyph + target room name.** Direction glyphs: `↑` Up, `↓` Down, `⊙` In, `⊗` Out (named constants, swappable). Unknown (`?`) stays as today's `?` (it has no target semantics).
- **Name truncated to the available gutter width** on the map; the dump legend shows the full `<glyph> #<id> <name>` for each portal.
- **Stacking:** multiple portals on one room stack on successive gutter rows beneath the first.
- **Placement:** the right gutter, top-aligned with the box (as the current stub does). Placement avoids overwriting routed connector cells where possible; if a portal badge and a connector would collide, the connector wins (the badge is informational) — verified by the existing buffer-level overlap accounting.
- **Layer-ready:** the badge identifies the *target room* (id + name), which is exactly what a future "jump to the layer containing #201" affordance needs. Nothing in this design assumes single-layer beyond rendering on the current layer.

### Components touched
- `mapper`: portal stub edges already carry direction + endpoints; expose the target room id so the renderer can resolve the name (the graph has it).
- `app/render/map.rs` `draw_stub`: render glyph + truncated target name, stack multiple, clip to area.
- `app/map_dump.rs`: portal legend lines show `<glyph> #<id> <name>` per portal.

---

## Feature 3 — Multi-edge merge (fan-in)

### Current behavior
`route_topology_with` collapses exactly ONE reciprocal pair between a room pair into a single connector; every *additional* edge between the same pair (e.g. `239→N→77`, `239→S→77` alongside the `77↔239` E/W pair) draws as its own full connector → the three-line tangle between #239 and #77.

### Design
For every group of edges connecting the same **unordered room pair `{A,B}`**, draw **one trunk** between the two rooms and **merge the extra edges' connector lines into it** instead of drawing an independent line per edge:

```
                     ╭────▲────╮      #239 keeps its N exit arrow (box edge)
   ╭─────────╮       │#239     │
   │#77     ◀────────◀         │      W exit = the trunk straight to #77
   ╰─────────╯       │         │
               ▲     ╰────▼────╯      #239 keeps its S exit arrow (box edge)
               │          │
               └──────────┘           N & S lines curve around and MERGE into
                                       the single trunk → #77 gets one arrival
```

- **Every exit is still drawn on its origin box edge** via the existing edge-arrow glyph (`▲▼◀▶`), so the player sees that N, S, and W all leave #239. The arrows on the boxes carry the direction information.
- **The connector lines merge:** the extra exits route through the lanes and join the primary trunk (T-junction `├┤┬┴`) so the destination (#77) receives a **single** incoming line rather than three.
- **No standalone arrowheads on the merging stubs** — the box-edge exit arrows already encode each direction; the merged lines are plain connectors. Arrowheads remain only at the box edges (departure on origin sides, arrival on destination side).
- **Reciprocal handling unchanged:** a true-opposite pair still shows an arrow at both ends; the extra same-pair edges merge into that connector.
- **The dump still lists every individual edge** (`EDGE 239 N 77`, `EDGE 239 S 77`, …) — merging is render-only.

### Grouping rule
Group drawn edges by unordered room pair `{A,B}`. Within the group, the existing reciprocal collapse still picks the primary trunk; every remaining edge in the group becomes an additional exit stub on its origin's appropriate side that **routes to and joins the trunk** rather than drawing an independent line to the destination. The merge point is on the trunk near the destination side.

### Components touched
- `mapper/route` `route_topology_with` + lane routing: produce, per room pair, one trunk plus secondary exit stubs that terminate ON the trunk (a join), instead of independent connectors. The join must keep the no-overlap / lane invariants.
- `app/render/map.rs`: render the join as a T-junction into the trunk; secondary stubs carry no arrowhead (only the box-edge arrow).

---

## Architecture / data flow

```
mapper:
  route_topology  ── per pair: 1 trunk + secondary exit stubs joining it (Feature 3)
                  ── diagonal edges resolve to a CORNER anchor (Feature 1)
  route_all       ── portal stubs already carry dir+endpoints; expose target id (Feature 2)
app/render/map.rs:
  draw_room       ── diagonal arrow replaces the corner glyph (Feature 1)
  render lanes    ── trunk + T-junction joins; no arrowhead on secondary stubs (Feature 3)
  draw_stub       ── portal badge = glyph + truncated target name, stacked (Feature 2)
app/map_dump.rs   ── portal legend: glyph + #id + name (Feature 2); every edge still listed (Feature 3)
```

All three are **Boxes-zoom only**. The no-overlap guarantee (mapper structural + app buffer-level cleanup) and determinism must hold for the new corner anchors, merged trunks, and badges.

## Testing

`mapper`:
- Diagonal edge → exit/entry anchor is the correct box corner; reciprocal diagonal arrows at both corners.
- Portal stub exposes target room id.
- Multi-edge group → one trunk + N−1 secondary stubs that terminate on the trunk; no independent duplicate connector to the destination; no-overlap + determinism preserved.

`app`:
- Diagonal departure/arrival draws the diagonal arrow glyph at the correct corner; box outline otherwise intact; OFF for non-diagonal edges (byte-identical).
- Portal badge renders glyph + truncated name on the map; full `#id name` in the dump; multiple portals stack; never breaks the box outline.
- Multi-edge: the A129 #239↔#77 group renders as a single trunk with the N/S/W exits merging (no three-line tangle); destination has one arrival; box-edge arrows present for each direction; no rendered overlap.

## Risks

- **Corner anchors** add a new attachment point to the lane router; must integrate with slot assignment and the no-overlap gate (corner cell shared by two edges of the box).
- **Multi-edge join** is the most involved change — a secondary stub joining a trunk mid-span is a T-junction that must not be miscounted as an illegal overlap by the render gate; the merge point selection must be deterministic.
- **Portal badge vs connector collision** in a dense gutter — badges yield to connectors; verify the buffer-level overlap accounting treats badges correctly (informational, not a routed line).
- Glyph rendering depends on the terminal font; all new glyphs are named constants with easy fallbacks.
