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

## Feature 2 — Portal icons + destination toggle

> **Design history:** a first pass rendered portals as glyph+name *badges in the right gutter*
> (commits `02d8e33`..`8b8db8b`). The gutter is too narrow for names, so the rendering was
> revised to **in-room directional icons** plus a **hotkey-toggled destination overlay**. The
> mapper `dest_label` field, the `portal_glyph` constants, and the dump PORTALS legend from that
> pass are kept; only the on-map rendering changes.

### Current behavior
`route_all` emits `is_stub` edges for Up/Down/In/Out/Unknown, each carrying its target room name in `dest_label` (Some for stubs). Portals currently render as stacked glyph+name badges in the right gutter — names overflow the narrow gutter.

### Design
Render portals as **directional icons inside the room box** at Boxes zoom, on the box's right
interior column. Default = icons only; a hotkey toggles the destination names.

```
default (icons only):     Ctrl+P (destinations):
╭─────────╮               ╭─────────╮
│Behind H↑│               │  Attic ↑│   row 1 (Up)   — name replaces the room label
│#79      │               │#79      │   row 2 (mid)  — kept (no mid portal here)
│        ↓│               │S of Ho ↓│   row 3 (Down) — name replaces the blank row
╰─────────╯               ╰─────────╯
```

- **Icon slots (right interior column, `col = BOX_W-2`):** `↑` Up → row 1; `⊙` In / `⊗` Out → row 2 (middle); `↓` Down → row 3. Glyphs are named, swappable constants (`portal_glyph`, already defined). The icon is drawn only for the directions a room actually has.
- **Unknown (`?`)** has no spatial direction → it shares the **middle** slot (row 2). When a room has more than one of In/Out/Unknown, the middle cell shows one by precedence **In ▸ Out ▸ Unknown**; the dump still lists every portal. Likewise if a room has multiple portals in one slot, the icon marks the slot and the dump carries the full set. **Unknown has no target semantics** — even with `Ctrl+P` on it shows only the `?` glyph, never a destination name (a name would read as the misleading "West of ?").
- **Destination toggle — `Ctrl+P` (`show_portal_labels`, default off):** when on, each portal with an icon shows its **destination room name right-aligned beside its icon on that icon's row**, replacing that row's normal content (row 1 room label, row 2 `#id`, row 3 blank). The icon stays pinned at the far-right interior cell. Names are truncated to the interior width on the map; the **full untruncated name is always in the Ctrl+D dump**. Wiring mirrors the existing `Ctrl+A` alignment toggle exactly.
- **Notes marker `●`** currently occupies the upper-right interior cell — the new `↑` slot. Rule: a room **with** an up-portal gives that corner to `↑` and shifts `●` one interior cell left (room label truncates to fit); a room **without** an up-portal keeps `●` where it is.
- **Boxes zoom only.** Compact keeps its existing bare-label `draw_stub`; Overview is unchanged.
- **Layer-ready:** icons + `dest_label` identify each target room (id resolvable, name shown), which is exactly what a future "jump to the layer containing the target" affordance needs.

### Components touched
- `mapper`: unchanged — `dest_label` (Some target name for stubs) already lands on stub edges.
- `app/state.rs`: add `show_portal_labels: bool` (default false), mirroring `show_alignment`.
- `app/input.rs`: add `Action::TogglePortalLabels`, map `Ctrl+P` to it, flip the flag in `apply_action`.
- `app/render/map.rs`: a new post-room overlay draws the directional icons (always) and, when `show_portal_labels`, the right-aligned destination names; handles the `●` shift; the prior gutter-badge rendering (`draw_portal_badge` + its loop) is removed.
- `app/main.rs`: help bar gains `Ctrl+P: portals`.
- `app/map_dump.rs`: unchanged — the PORTALS legend already shows full `glyph #id name`.

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
  route_all       ── portal stubs carry dir + dest_label (target name) (Feature 2, done)
app/render/map.rs:
  draw_room       ── diagonal arrow replaces the corner glyph (Feature 1)
  render lanes    ── trunk + T-junction joins; no arrowhead on secondary stubs (Feature 3)
  portal overlay  ── in-room directional icons; Ctrl+P shows destination names (Feature 2)
app/map_dump.rs   ── portal legend: glyph + #id + name (Feature 2); every edge still listed (Feature 3)
```

All three are **Boxes-zoom only**. The no-overlap guarantee (mapper structural + app buffer-level cleanup) and determinism must hold for the new corner anchors, merged trunks, and badges.

## Testing

`mapper`:
- Diagonal edge → exit/entry anchor is the correct box corner; reciprocal diagonal arrows at both corners.
- Portal stub carries the target room name (`dest_label`). (done)
- Multi-edge group → one trunk + N−1 secondary stubs that terminate on the trunk; no independent duplicate connector to the destination; no-overlap + determinism preserved.

`app`:
- Diagonal departure/arrival draws the diagonal arrow glyph at the correct corner; box outline otherwise intact; OFF for non-diagonal edges (byte-identical).
- Portal directional icons render in the correct in-room slots (↑ row 1, ⊙/⊗/? row 2, ↓ row 3); `Ctrl+P` reveals right-aligned destination names on those rows; full `#id name` in the dump; the `●` notes marker shifts left when an up-portal claims the corner; box outline otherwise intact; OFF byte-identical except the icons.
- Multi-edge: the A129 #239↔#77 group renders as a single trunk with the N/S/W exits merging (no three-line tangle); destination has one arrival; box-edge arrows present for each direction; no rendered overlap.

## Risks

- **Corner anchors** add a new attachment point to the lane router; must integrate with slot assignment and the no-overlap gate (corner cell shared by two edges of the box).
- **Multi-edge join** is the most involved change — a secondary stub joining a trunk mid-span is a T-junction that must not be miscounted as an illegal overlap by the render gate; the merge point selection must be deterministic.
- **Portal icons vs box content** — icons sit on the right interior column (a known fixed cell), drawn as a post-room overlay; the only contention is the `●` notes marker (resolved by the left-shift rule). The destination-name overlay overwrites interior cells on portal rows only when `Ctrl+P` is on.
- Glyph rendering depends on the terminal font; all new glyphs are named constants with easy fallbacks.
