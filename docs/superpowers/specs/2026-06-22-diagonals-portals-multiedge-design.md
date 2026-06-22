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
- Diagonal corner arrows are connector arrows, so (like `▶◀▲▼`) they show in normal view and are hidden under `Ctrl+P` (portal view).

> **Deferred — true diagonal `╱`/`╲` lines:** rendering actual diagonal connector lines (and squaring the box spacing for them) was discussed and parked. The orthogonal lane router can't model diagonal runs without a new collision/clipping system, and the box stride isn't square. We first ship the corner arrows with orthogonal routing and evaluate the look before deciding whether diagonal lines are worth that cost.

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
Two views, toggled by `Ctrl+P` (`show_portal_labels`, default off). Boxes zoom only.

**Box interior (BOTH views), centered:** the room **name word-wraps across interior rows 1–2**,
each line **centered** in the 9-col interior; **row 3 is `#id`**, centered, with the **alignment
diagnostics appended after it** (shown when `Ctrl+A` is on). Centering uses the full interior
width; the right-edge portal icons (normal view) overlay on top, so only a name line that fills
all 9 columns loses its last char under an icon (rare; short names unaffected).

```
no portals:      with portals:    Ctrl+A (align):
╭─────────╮      ╭─────────╮       ╭─────────╮
│  Rocky  │      │  Rocky ↑│       │  Rocky ↑│
│  Ledge  │      │  Ledge ?│       │  Ledge ?│
│   #26   │      │   #26  ↓│       │ #26 R3 ↓│
╰─────────╯      ╰─────────╯       ╰─────────╯
```

**Normal view (`Ctrl+P` off):** portal icons sit in the **interior right column**
(`col = BOX_W-2`): `↑` Up → row 1; `⊙` In / `⊗` Out / `?` Unknown → row 2 (middle); `↓` Down →
row 3. Connector arrowheads (`▶◀▲▼`) draw on the border as usual. No destination text.

**Portal view (`Ctrl+P` on):** the icons **move onto the border** on the side matching their
destination, and the **destination names float outside** the box:
- `↑` Up → top border; destination name **above** the box.
- `↓` Down → bottom border; destination name **below** the box.
- `⊙`/`⊗`/`?` mid → right border (middle row), the **In ▸ Out ▸ Unknown** precedence winner (the dump lists any others); In/Out destination name to the **right**.
- **Connector arrowheads are NOT drawn** in portal view (only portal icons sit on the border); the connector *lines* still draw and may be overwritten by the floating destination text — that overwrite is acceptable.
- Destination names are **untruncated** (overflow/overwrite of neighbours and paths allowed).

```
 Canyon View
╭────↑────╮
│  Rocky  │
│  Ledge  ?
│   #26   │
╰────↓────╯
 Canyon Bottom
```

- **Glyphs** are named, swappable constants (`portal_glyph`). An icon is drawn only for the directions a room actually has.
- **Mid precedence** when a room has more than one of In/Out/Unknown (lower wins): **In ▸ Out ▸ Unknown**; the dump lists every portal.
- **Unknown (`?`) has no target semantics** — it never shows a destination name in either view (only the `?` glyph; a name would read as the misleading "West of ?").
- **Notes marker `●`** (normal view): a room **with** an up-portal gives the upper-right interior cell to `↑` and shifts `●` one cell left; a room **without** an up-portal keeps `●` there.
- **Boxes zoom only.** Compact keeps its existing bare-label `draw_stub`; Overview is unchanged.
- **Layer-ready:** icons + `dest_label` identify each target room (id resolvable, name shown).

### Components touched
- `mapper`: unchanged — `dest_label` (Some target name for stubs) already lands on stub edges. (done)
- `app/state.rs`: `show_portal_labels: bool` (default false), mirroring `show_alignment`. (done)
- `app/input.rs`: `Action::TogglePortalLabels`, `Ctrl+P` mapping, flag flip in `apply_action`. (done)
- `app/render/map.rs` `draw_box_room`: word-wrap + center the name on rows 1–2; move `#id` to row 3, centered, with the align diagnostics appended (the `Ctrl+A` overlay writes after `#id` on row 3, not row 3 from col 1).
- `app/render/map.rs` portal overlay (`draw_portal_icons`): normal view draws interior right-column icons (+ `●` shift); portal view draws border icons (top/bottom/right) and the floating destination names outside the box; the right-aligned-beside-icon labels of the prior pass are replaced by the float-outside placement.
- `app/render/map.rs` `render_map`: suppress connector arrowheads (`draw_connector_arrows`) when `show_portal_labels` is on.
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
