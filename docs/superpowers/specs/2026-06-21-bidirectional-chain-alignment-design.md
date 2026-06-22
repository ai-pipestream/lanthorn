# Bidirectional-Chain Alignment + Contiguity — Design Spec

**Date:** 2026-06-21
**Branch:** `chain-alignment`
**Status:** Approved (design) — awaiting spec review
**Builds on:** `2026-06-21-constraint-relayout-engine-design.md` (the VPSC + SMACOF re-tidy engine).

## Goal

Make the re-tidy layout honor two structural intents the stress engine currently ignores:

1. **Align bidirectional cardinal pairs.** A reciprocal E/W pair (`A→E→B` *and* `B→W→A`)
   should share **one row** (zero row-distortion); a reciprocal N/S pair should share **one
   column**. The layout grows as needed so these are exactly satisfied.
2. **Keep bidirectional chains contiguous.** The members of a bidirectional chain (e.g.
   `79↔203↔193`) occupy **consecutive** cells on their shared row/column, so an unrelated
   room (e.g. `#180`) cannot interleave between them.

Plus a diagnostic surface: **show which alignment rules placed each room**, both as a
compact toggled code in the room box and as detailed text in the `.map.txt` dump.

## Background

The constraint engine satisfies *separation* (B is east of A) but never forces a cardinal
edge's two rooms onto the same row/column, so reciprocal cardinal edges render diagonal
(distorted), and unrelated rooms drop into the gaps of a chain (`#180` between `#193` and
`#203`). The existing perpendicular-alignment pass only moves rooms that are *free* on an
axis; it cannot align a reciprocal pair where both rooms are otherwise constrained, and it
does nothing about interleaving. This spec adds the missing structure.

## Decisions (from brainstorming)

1. **Grouping basis = bidirectional cardinal chains** (precise, low-risk). General
   connectivity clustering is explicitly **deferred** to a later phase.
2. **Alignment via equality constraints** in the VPSC solve (not a loose post-pass).
3. **Contiguity via a compaction + foreign-room-bump pass** after the solve.
4. **Conflicts degrade gracefully** — a chain/equality that contradicts another is dropped
   and its edge stays `distorted` (the existing mechanism). Conflicts are expected to be
   the exception.
5. **Rules display = both**: a toggled compact code in the room box, and detailed
   per-room text in the dump legend. Chain membership is *derived from the graph*, so no
   alignment metadata is threaded through the layout.

## Chain detection (`mapper`, pure function of the graph)

```rust
/// Maximal chains of bidirectional cardinal edges, per axis. A "bidirectional E/W edge"
/// between A and B means both `A→E→B` (or `A→W→B`) and the reverse `B→W→A` (or `B→E→A`)
/// exist. Union-find over those edges yields E/W chains; over bidirectional N/S edges,
/// N/S chains. A room may be in at most one E/W chain and at most one N/S chain.
pub struct Chains {
    /// room → its E/W chain id (rooms sharing a row), if any.
    pub ew: BTreeMap<RoomId, usize>,
    /// room → its N/S chain id (rooms sharing a column), if any.
    pub ns: BTreeMap<RoomId, usize>,
    /// chain id → sorted member room ids (for ew and ns respectively).
    pub ew_members: Vec<Vec<RoomId>>,
    pub ns_members: Vec<Vec<RoomId>>,
}

pub fn detect_chains(graph: &MapGraph) -> Chains
```

- "Bidirectional" uses `direction::opposite`: an E/W edge `A→dir→B` (dir ∈ {E,W}) is
  bidirectional iff some `B→opposite(dir)→A` exists. Same for N/S {N,S}.
- Deterministic: union-find over connections in array order; members sorted; chain ids
  assigned in ascending lowest-member order.
- Singletons (a room in no bidirectional cardinal pair) get no chain id.

## Alignment — equality constraints (`mapper/src/layout/constraints.rs`, `stress`)

For each E/W chain, add **equality on Y** between consecutive members; for each N/S chain,
equality on X. Equality `coord[a] == coord[b]` is expressed to VPSC as two separations with
`gap = 0` (`a ≤ b` and `b ≤ a`), which the block-merge collapses to one shared block — both
rooms get the identical coordinate on that axis.

- Add equality constraints **before** the directional separation constraints, and run the
  same deterministic DAG-ify cycle check that already drops contradictory constraints: an
  equality that would close a cycle on its axis is dropped and its connection index recorded
  in `dropped` (→ `distorted`). This is the conflict handling.
- A cross-chain room (e.g. `#74` in an E/W chain *and* an N/S chain) gets a Y-equality from
  the E/W chain and an X-equality from the N/S chain — independent axes, no rigidity issue.
- The stress objective still spreads members along the *parallel* axis (separation), so the
  row/column "grows" to fit the chain; non-chain rooms are pushed aside by their own
  separation constraints. This realizes "grow positions so bidirectional paths have no
  distortion."

`build_axis_constraints` gains the chain equalities; its signature stays
`build_axis_constraints(graph, ids, gap) -> AxisConstraints` (it calls `detect_chains`
internally, restricted to the component's ids).

## Contiguity — compaction + foreign bump (`mapper/src/layout/mod.rs`, after snap)

After the stress solve, snap, and the existing free-axis alignment, run a contiguity pass
per component:

1. For each E/W chain (now all on one row `y` after equality + snap): sort members by `x`,
   and **reassign them to consecutive cells** `(x0, y), (x0+1, y), …` where `x0` is the
   chain's current min x. For each cell that a *foreign* room (non-member) currently
   occupies, move that foreign room off the row with `place_preserving_alignment` (the
   axis-preserving displacement already in `mod.rs`) so it isn't re-bumped onto another
   chain. Symmetric for N/S chains (consecutive cells down a column).
2. Re-run the existing collision resolution so the final grid has no overlap.

Deterministic: chains processed in id order, members in `x` (then id) order, foreign rooms
bumped in ascending id order. The result: `#193 #203 #79` is a solid run; `#180` sits beside
it, not inside.

## Rules display

**Chain membership is derived** by `detect_chains`, so both surfaces call it directly; the
distorted/dropped reasons come from the existing `Connection.distorted` flag.

**Dump legend (`app/src/map_dump.rs`).** Each `ROOM` line gains an `align=` annotation:
- `align=row[79,203,193]` if the room is in that E/W chain (its members), and/or
  `col[74,76]` for an N/S chain; `align=none` if in neither.
- Append any of the room's own compass edges that are `distorted` (the rules that could not
  be applied), e.g. `dropped=[25→W→76]`.

**In-box code (`app/src/render/map.rs`), toggled.** A new `AppState.show_alignment: bool`
toggled by **`Ctrl+A`** (new `Action::ToggleAlignment`). When on, each room box renders a
compact code on its top border row — `R{ew_id}` and/or `C{ns_id}` (e.g. `R2 C1`), or
nothing if ungrouped. Drawn within the box's interior width; never overwrites the room id
or an exit arrow. Off by default; pure overlay, no layout effect. The help bar shows
`Ctrl+A: align` while in story focus.

## Architecture / data flow

```
mapper::layout::detect_chains(graph) ─┬─► build_axis_constraints (equality)  ─► stress solve
                                      │                                          ─► snap
                                      └─► (re-used by) contiguity pass ─► collision resolve
app:
  render::map  ─ detect_chains(graph) ─► in-box R/C codes (when show_alignment)
  map_dump     ─ detect_chains(graph) ─► align=/dropped= legend text
```

`relayout_auto` keeps its signature; per-turn incremental placement, the app cleanup, and
persistence are unchanged. `show_alignment` is view state (not persisted).

## Testing

`mapper`:
- `detect_chains`: a reciprocal E/W pair → one ew chain; a non-reciprocal pair → none;
  `239→N→77` + `239→S→77` (same origin, not reciprocal) → none; a cross-chain room appears
  in one ew and one ns chain.
- Alignment: reciprocal `A↔B` E/W ⇒ same row (`y` equal); N/S ⇒ same column; a 3-room
  bidirectional chain ⇒ all one row; an equality that cycles is dropped → `distorted`, no
  overlap.
- Contiguity (A129 house): `#193 #203 #79` occupy consecutive cells on one row and `#180`
  is not between any two consecutive chain members; no room overlap; deterministic.
- Cross-chain `#74` lands on its E/W chain's row **and** its N/S chain's column.
- The constraint-engine win test still holds (distortion not worse than before this change).

`app`:
- Dump legend shows `align=row[…]`/`col[…]` for chained rooms and `dropped=[…]` for a
  distorted edge; `align=none` for an ungrouped room.
- `Ctrl+A` toggles `show_alignment`; with it on, a chained room's box shows its `R`/`C`
  code and an ungrouped room shows none; the code never overwrites the room id or an arrow;
  with it off, rendering is byte-identical to today.

## Out of scope

- General connectivity / modularity clustering (deferred — a later phase).
- Aligning *non*-bidirectional cardinal edges beyond the existing free-axis pass.
- Persisting the alignment overlay state or the chain ids.
- Any change to per-turn incremental placement, the app router/cleanup, or the segment /
  diagonal / theming phases.

## Risks

- **Over-constraint conflicts** (a room pulled by an E/W chain and contradictory N/S
  geometry) are handled by dropping the cycle-closing equality → `distorted`; expected
  rare, surfaced by the rules display.
- **Contiguity vs. compactness**: bumping foreign rooms off a chain's row can grow the map;
  accepted per the "grow positions" intent.
- **In-box code space**: the 11×5 box has limited room; the code is clipped to the interior
  and suppressed if it would collide with the id/arrow (documented, tested).
