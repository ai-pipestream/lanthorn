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
- **Reciprocal collapse for up/down (refined after visual testing):** a matching
  Up(A→B) + Down(B→A) pair **does** collapse to a single `RoutedConnector`
  (`reciprocal = true`), following the normal reciprocal rules, so it draws as one
  dotted path with a glyph at each end. (An unmatched one-way up/down stays a single
  one-way connector.) Extend `oneway_entry_side` (~358) so a one-way up/down that
  dips into a channel has an entry side (Up enters dest from Bottom, Down from Top).
- **N/S reciprocal takes slot priority over an up/down reciprocal at the same
  room.** Up and N both exit the Top border (Down and S the Bottom). When a room has
  both a reciprocal N/S and a reciprocal up/down connector on the same side, the N/S
  reciprocal claims the center slot (slot 0) and the up/down connector yields to a
  fanned off-center slot — it still collapses, it just isn't centered.
- **Render-only — must NOT leak reciprocal weight into layout.** This collapse is a
  routing/render concept. Layout keeps up/down at weight 1 (reciprocal detection in
  the scoring/constraint code stays keyed on `grid_offset`, Phase 1), so a hard
  reciprocal N/S still outranks an up/down: in the very case that triggers the slot
  priority, A↔C is a reciprocal N/S placed immediately adjacent, so the up/down room
  B is shoved off C's cell by Phase-1 placement (B shifts). The refinement must not
  change that — only the drawn connector gains `reciprocal = true`, not any layout
  score/weight.
- `RoutedConnector.exit_dir` already carries `Up`/`Down` to the renderer — no new
  field required.

## Rendering (`crates/app/src/render/map.rs`)

- In `render_lane_connectors`, branch on `exit_dir == Up|Down`: draw the connector
  **body dotted** (portal `┊`/`┄`, selected per-connector) and put the **up/down
  glyph** (`sym.portal.up`/`down`, or the stairs preset) at the border anchor
  instead of `arrow_for_departure`. For a **collapsed reciprocal** up/down connector,
  draw a glyph at **both** ends (up-glyph on the lower room's top border, down-glyph
  on the upper room's bottom border) — the same both-end treatment N/S reciprocals
  get. The anchor is the centered, slot-fanned `box_edge_anchor` (Top/Bottom), so the
  glyph sits mid-border and is pushed off-center only when a reciprocal N/S connector
  claims the center slot.
- **Delete** `draw_portal_connectors` and `portal_stub` (and their call in
  `render_map`). Make `draw_portal_icons` **skip Up/Down** — it keeps drawing the
  In/Out/Unknown in-room icons only.

## Phase 2 decisions (flagged and accepted)

- **In/Out/Unknown stay as in-room portal icons.** Only Up/Down move to the
  lane/border treatment.
- **Matching Up+Down pairs collapse** (reciprocal, one dotted path, glyph at both
  ends); an unmatched one-way up/down draws one connector with a single departure
  glyph. A reciprocal N/S at the same room takes the center slot; the up/down
  reciprocal yields to a fanned slot but still collapses. (Supersedes the earlier
  "departure-glyph only / never collapse" decision.)
- **Layout stays weight-1 for up/down** so reciprocal N/S still shoves the up/down
  room aside (B shifts off C's cell). The collapse is render-only.

## Phase 2 verification

- **Unit:** an up connection now produces a `RoutedConnector` in the plan (not a
  stub); a matching Up+Down pair produces **one** connector with `reciprocal = true`;
  an unmatched one-way up/down produces one connector with `reciprocal = false`;
  rendering a reciprocal up/down connector draws the up glyph on the lower room's top
  border and the down glyph on the upper room's bottom border, dotted body; a room
  with both a reciprocal N/S and a reciprocal up/down gives the N/S the center slot;
  In/Out/Unknown still draw in-room icons.
- **Layout invariant:** with A↔C reciprocal N/S and A↔B reciprocal up/down, C sits
  immediately N/S of A and B is shifted off that cell (Phase-1 priority unchanged by
  the render collapse).
- **Real game (oracle):** on a vertical-heavy map, vertical connectors route without
  breaking, don't cross other connectors, a matching up/down pair is a single dotted
  path anchored at the top/bottom border (yielding the center to a reciprocal N/S),
  with up/down glyphs on the border.

---

# Phase 3: Refinements after visual testing (SQ-0219 + #1 shove + #3 lock-in)

Visual testing of Phase 2 surfaced three refinements. Two are behavior changes
(SQ-0219 de-dup; #1 up/down paths shove instead of cross); one is a confirmation
that an intended property already holds and only needs a regression test (#3).

## #1 — up/down paths shove rooms apart instead of crossing (scoped)

**Observed:** up/down connectors *cross* other connectors instead of forcing rooms
apart to make room. **Root cause (confirmed on disk):** the tidy's cleanup loop
exits immediately when the illegal-overlap count is zero (`overlap_stats` returns
`(illegal, crossings)`; the loop gates on `illegal`), and the crossing count is only
a **4th-place tiebreak** in the move-selection key. A *clean perpendicular crossing*
of an up-path over another path is not an illegal overlap, so it triggers no room
movement. The lane router minimizes crossings for a **fixed** layout — only the tidy
moves rooms, and nothing converts a crossing into a room-separating move.

**Design — scoped up/down crossing pressure (chosen over a general crossing
promotion to bound blast radius):**

- Extend `overlap_stats` to return a third field, `updown_crossings` — the count of
  crossing cells (the existing clean `[ns, ew]` perpendicular case) where **at least
  one** of the two crossing connectors is an up/down connector
  (`exit_dir ∈ {Up, Down}`). It is a subset of `crossings`; the existing `crossings`
  total is unchanged.
- The cleanup loop continues while `illegal > 0` **OR** `updown_crossings > 0`
  (today it breaks the moment `illegal == 0`).
- Insert `updown_crossings` into the move-selection key and acceptance test at
  **second priority — directly after `illegal`** (before alignment/side-broken and
  before the general `crossings` tiebreak). So the tidy will move rooms to eliminate
  an up/down crossing, but only when doing so does not increase illegal overlaps
  (illegal stays the hard primary key), and it will **not** relocate rooms for a
  compass-vs-compass crossing (that stays the low-priority tiebreak it is today).
- Guard `compact_empty_lines` so it reverts a compaction that *increases*
  `updown_crossings` (today it reverts only on increased illegal), so compaction
  cannot undo the shove.

**Explicitly unchanged:** compass-vs-compass crossing behavior (still just a
tiebreak); `illegal` remains the hard primary constraint; the lane router is
untouched. The change is scoped to crossings that involve an up/down path.

**The 4 deferred tests.** Phase 2 marked four layout tests `#[ignore]` ("up/down now
feed overlap_stats…pending B/A layout decision"). Phase 3 resolves that decision (A,
scoped). Each deferred test is revisited: its post-shove layout is eyeballed for
correctness, its assertions updated to the correct layout, and the `#[ignore]`
removed.

## SQ-0219 — a compass edge wins over an up/down path on the same room pair

**Observed/confirmed:** when rooms A,B are joined by **both** a compass edge (e.g.
`north`) **and** an up/down edge, the router emits **two** connectors — a compass
trunk plus an up/down merge stub — and which is which is just `connections()` order.
There is no compass-vs-portal de-dup. In/Out never route (their offset is `None`), so
"ignore in/out" is already true.

**Design:**

- **Suppress the up/down connector when a compass edge shares the pair.** In
  `route_topology_with`, precompute the set of unordered room pairs that have at least
  one **compass** edge (`grid_offset(dir).is_some()`). Skip routing any Up/Down edge
  whose pair is in that set — no trunk, no merge stub. The compass edge is drawn; the
  up/down edge contributes only its layout hint (redundant here, since a compass edge
  already governs the pair) and is not drawn as a separate dotted connector.
- **In/Out:** unchanged — never routed, always shown as room mid-slot icons.
- **Keep the portal symbol shown.** In the default/numbers views the *only* up/down
  indicator today is the connector's border glyph (the independent room-level up/down
  glyph exists only in portal-label view). Suppressing the connector would therefore
  erase the ↑/↓. Fix: in `draw_portal_icons`, for the default/numbers views, draw the
  room-level up/down border glyph (top = ↑, bottom = ↓) for an up/down stub **whose
  pair has a compass connector in the plan but no up/down connector** — i.e. exactly
  the pairs the router just de-duped. This condition is computable in the renderer
  from `rm.plan.connectors` + `rm.edges` with no new plumbing, and it must not
  double-draw when an up/down connector *is* present.

## #3 — up glyph always on the north border, down on the south (already true)

**Confirmed already satisfied on disk:** `route_side(Up) = Top`,
`route_side(Down) = Bottom`, `oneway_entry_side(Up) = Bottom`, `Down = Top`, and no
code path ever assigns an up/down connector a Left/Right side. The ↑/↓ glyph is chosen
from `exit_dir` consistently with that side, and a collapsed reciprocal pair lands the
far-end glyph on the opposite (correct) border. So the up glyph is always on a north
(top) border and the down glyph always on a south (bottom) border, by construction.

**Design:** no behavior change — add a **regression test** locking in the invariant
(`route_side(Up)=Top`/`route_side(Down)=Bottom` at the router level, plus a render
test that ↑ lands on a top border row and ↓ on a bottom border row for a reciprocal
up/down map), so a future edit cannot silently break it.

## Phase 3 decisions (flagged and accepted)

- **#1 is scoped to up/down-involved crossings only.** Compass-vs-compass crossing
  behavior is unchanged; `illegal` stays the hard primary key. This bounds regression
  risk to vertical-path layouts.
- **SQ-0219 suppresses the up/down *connector* but keeps the room's portal glyph** in
  every view, so vertical access still reads even when a compass path is drawn.
- **In/Out stay ignored** (never routed) and keep their room mid-slot icons.
- **#3 needs no behavior change** — it is already true and only gets a regression
  test.

## Phase 3 verification

- **Unit (mapper):** a pair with both a compass and an up/down edge yields exactly one
  connector (the compass one) and zero up/down connectors; a pair with only an up/down
  edge is unaffected (one up/down connector); `route_side(Up)=Top`/`Down=Bottom`.
- **Unit (app):** `overlap_stats` counts an up/down×horizontal crossing in
  `updown_crossings` but a compass×compass crossing not; a fixture where an up path
  would cross a horizontal compass path ends with rooms shoved apart
  (`updown_crossings == 0`) after tidy, while a pure compass crossing triggers no room
  movement; a de-duped pair still renders the room-level ↑/↓; a reciprocal up/down map
  renders ↑ on a top border row and ↓ on a bottom border row.
- **Real game (oracle):** on a vertical-heavy map, up/down paths no longer cross other
  paths where the tidy could make room; a room joined by both a compass and an up/down
  edge draws a single compass path and still shows its up/down symbol; compass-only
  regions are visually unchanged.
