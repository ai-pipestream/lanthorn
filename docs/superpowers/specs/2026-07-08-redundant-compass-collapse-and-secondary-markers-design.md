# Redundant Compass Collapse + Secondary-Direction Markers — Design

**Date:** 2026-07-08
**Status:** Approved for planning
**Related:** SQ-0220 (collapse Unknown edges), SQ-0216/0222/0224 (slot priority, up/down first-class + `draw_deduped_updown_border_glyphs`)

## Goal

When two rooms are connected by **more than one compass direction in the same
direction of travel** (e.g. Zork's around-the-house ring, where `#68` reaches
`#217` by both `S` *and* `SE`, and returns by both `W` *and* `NW`), draw a
**single clean connector** for the pair instead of two crossing lines — and
preserve every hidden command by stamping a small **secondary-direction marker**
inside the room next to the retained connector's arrowhead.

## Problem

In `~/Downloads/map.json` the four house rooms are placed as a diamond (`#143`
N, `#89` E, `#217` S, `#68` W). Each adjacent pair carries **four directed
compass edges** — two each way. For `#68 ↔ #217`:

| edge | matches placed geometry | `distorted` |
|---|---|---|
| `68 →S→ 217`  | no  | **true**  |
| `217 →W→ 68`  | no  | **true**  |
| `68 →SE→ 217` | yes | false |
| `217 →NW→ 68` | yes | false |

The router pairs one reciprocal (`S↔W`) into a connector and the leftover
forward+back (`SE↔NW`) into a **second** connector on different box sides, so
the two lines cross. ×4 pairs = a tangle. The simpler `#33 ↔ #175` case
(one forward `E`, three back `W/N/S`) already renders acceptably because its
extras share the reciprocal connector's endpoints and collapse.

**Key lever:** the layout already marks the geometrically-wrong edge
`distorted` (cardinals here) and the right one clean (diagonals). We reuse
`edge_is_satisfied` / the `distorted` flag as the selector — no new heuristic.

## Design

### Part A — Redundant compass collapse (render-level, non-destructive)

Applied in `route_topology_with` (`crates/mapper/src/route/mod.rs`), the
connector-building pass. **The graph is never mutated** — every edge remains a
real, walkable command; only the *drawn connector set* is deduped.

**Selection — best edge per direction-of-travel bucket.** For each unordered
room pair, split its directed **compass** edges (`grid_offset(dir).is_some()` —
the 8-way directions; Up/Down excluded, see below) into a **forward** bucket
(origin = the lower room id) and a **backward** bucket (origin = the higher id).

- **Retained forward** = the bucket's edge that is `edge_is_satisfied` (matches
  placed geometry); ties broken by (a) direction is the exact `opposite` of the
  other bucket's retained pick (maximizes straightness), then (b) lowest edge
  index. If none is satisfied, pick lowest index (still collapse — nothing is
  hidden, because the marker preserves it).
- **Retained backward** = same rule in the backward bucket.
- The retained forward + retained backward form the connector exactly as today
  (paired by `back_edge_idx`, or a one-way connector if a bucket is empty).
- **Every other directed edge in either bucket is a *secondary*** — removed from
  the routing working set (it draws neither its own connector nor a merge stub)
  and recorded against the retained connector, attributed to the **end whose
  room is its origin**, carrying its own `Direction`.

For `#68 ↔ #217`: retained `SE↔NW`; secondaries `S` at the `#68` end, `W` at
the `#217` end. For `#33 ↔ #175`: retained `E↔W`; secondaries `N` and `S` both
at the `#175` end (a room end may hold several).

**Why always collapse (no separate-draw fallback):** because the secondary
marker keeps the hidden command visible, collapsing never loses information —
even when both buckets are distorted. This is simpler than a
"keep-separate-if-ambiguous" fallback and never produces the crossing tangle.

**Data model.** `RoutedConnector` gains two fields:

```rust
/// Compass directions collapsed at the EXIT end (origin room) — extra same-pair
/// edges that are not drawn as their own line; the renderer stamps a marker for
/// each next to this connector's exit arrowhead. Empty for the common case.
pub secondary_exit: Vec<crate::direction::Direction>,
/// Compass directions collapsed at the ENTRY end (dest room).
pub secondary_entry: Vec<crate::direction::Direction>,
```

Existing `RoutedConnector` constructions add `secondary_exit: Vec::new(),
secondary_entry: Vec::new()`. After connectors are built, a small pass attaches
each recorded secondary to its pair's retained connector, splitting by whether
the secondary edge's origin equals the connector's `origin` (→ `secondary_exit`)
or `dest` (→ `secondary_entry`).

**Excluded from collapse:**
- **Up/Down** edges — a vertical passage and a horizontal one between the same
  pair are genuinely distinct; SQ-0224 already draws both and
  `draw_deduped_updown_border_glyphs` handles their dedup. Up/Down never enters
  the forward/backward buckets and is never a secondary here.
- **In/Out/Unknown** — no layout offset, never routed here; Unknown collapse is
  SQ-0220's job at the graph level.

### Part B — Secondary-direction marker (render)

Drawn in `crates/app/src/render/map.rs`, Boxes zoom only (interiors don't exist
at Compact/Overview), in a pass analogous to `draw_deduped_updown_border_glyphs`.

- **Glyph:** the **same themeable arrow symbol** the connector arrowheads use
  (`state.symbols.arrows`), for the secondary `Direction`. Reuse the existing
  direction→arrow-glyph mapping.
- **Placement:** the box **interior cell just inside** the retained connector's
  arrowhead at that end (the arrowhead sits on the border at `exit`/`entry` Side
  + slot; the marker is one cell inward). The retained arrow keeps its border
  slot — **the marker never participates in `assign_side_slots` and never
  displaces the primary arrow** (the path's own direction owns the primary
  slot). Multiple secondaries at one end **stack** along the interior edge
  inward from the arrowhead.
- **Color:** the marker uses the **same color as its retained connector** — no
  separate marker color. A connector that carries secondaries is a *shared path*
  (it combines several directions between one pair) and is drawn in a new,
  deliberately **brighter** connector color; its line, its arrowheads, and its
  secondary markers all share that one color, so the whole shared path reads as a
  single, brighter unit. Ordinary (non-shared) connectors are unchanged.
- **Precedence:** primary connector arrowhead drawn first (unchanged); secondary
  markers drawn after rooms, before/at the arrowhead pass, so they sit on the
  box interior.

### Theming (required — every new UI element is styleable)

- **One** new `ColorScheme` field, e.g. `shared_path` (a **brighter** default
  than `connector`), in `crates/app/src/colors.rs`.
- A retained connector with a non-empty `secondary_exit`/`secondary_entry` is
  rendered with `shared_path` for its **line, arrowheads, and markers**; every
  other connector keeps `connector`. The markers never get their own color.
- New `style.toml` selector (e.g. `map.shared-path`) wired through
  `crates/app/src/style.rs` / `styles.rs`, parsed and applied like existing
  connector colors, and exposed in the style editor if connector colors are.
- The marker glyph honors the existing configurable `symbols.arrows`.

## Components / files

- `crates/mapper/src/route/mod.rs` — bucket selection, secondary recording,
  `RoutedConnector` fields, attach pass. (Core change.)
- `crates/app/src/render/map.rs` — secondary-marker draw pass.
- `crates/app/src/colors.rs` — `secondary_exit` color field + default.
- `crates/app/src/style.rs` / `styles.rs` — selector parse/apply.
- `crates/app/src/render/style_editor.rs` — expose the new color (if peers are).

## Data flow

graph (unchanged) → `route_topology_with`: build compass working set → per-pair
bucket selection → retained edges routed as today; secondaries recorded →
attach secondaries to retained connectors → `RoutePlan` → render: draw
connectors + primary arrowheads → **new:** a connector with secondaries is drawn
in `shared_path` (line + arrowheads), and its secondary arrow glyphs are stamped
in that same `shared_path` color on the interior cells beside its arrowheads.

## Edge cases

- **One reciprocal pairing only (2 edges) / one-way (1 edge):** buckets have ≤1
  edge each → no secondaries → behavior unchanged.
- **≥3 edges but all in one direction of travel** (e.g. two forwards, no back):
  forward bucket picks best, extra forward → secondary at that end; connector is
  one-way as today.
- **Several secondaries at one end** (`#175`: `N`,`S`): stack inward; if the
  stack would exceed the interior height, cap and drop extras silently is NOT
  acceptable — instead cap at the interior and `log`/note; realistically ≤2.
- **All edges distorted:** still collapse (lowest-index pick per bucket); markers
  preserve the rest. Never hides a command.
- **Interior collision** with room number/label: arrowheads exit at box
  edges/corners, so the adjacent interior cell is edge/corner-adjacent, away from
  the top-left number and centered label; markers overwrite only their own cell.

## Testing

**mapper (`route/mod.rs`):**
- House-ring pair `#68/#217`: exactly **one** connector; retained is the
  satisfied diagonal `SE↔NW`; `secondary_exit`/`secondary_entry` capture `S`/`W`
  at the correct ends.
- `#33/#175`: one `E↔W` connector; `#175` end carries secondaries `N` and `S`.
- Tie/straightness: when both buckets have a satisfied edge, the exact-opposite
  pair is chosen (straight).
- Non-destructive: `graph.connections()` identical before/after routing.
- Up/Down on a pair that also has a compass edge is NOT collapsed into secondaries.

**app (`render/map.rs`):**
- On `~/Downloads/map.json`: `render_overlap_stats(&g)` illegal overlaps stay 0
  and crossings **drop** vs. the pre-change baseline (the ring tangle is gone).
- A connector with secondaries (its line, arrowheads, and markers) renders in
  `shared_path`; a connector without them renders in `connector`.
- The secondary marker glyph is drawn on the expected interior cell beside the
  retained arrowhead, Boxes zoom only; absent at Compact/Overview.
- Multiple secondaries stack without overwriting the arrowhead or each other.

**style:**
- The new `shared_path` selector parses from `style.toml`, applies to the shared
  connector + its markers, and round-trips.

**Real-map oracle:** `~/Downloads/map.json` (house-ring build) is the acceptance
fixture — visually one line per ring pair, each with its bright secondary arrow(s).

## Out of scope

- Collapsing Up/Down vs compass (SQ-0224 owns that).
- Graph-level deletion of edges (rejected — destructive; all commands stay real).
- A general "show every exit as an interior compass" feature — markers appear
  ONLY for collapsed redundant same-pair edges.
- The two pre-existing SQ-0224 path-level crossings unrelated to the ring.
