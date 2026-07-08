# Slot-priority: a straight-through connector keeps the center slot over a weaving one-way compass (SQ-0222)

**Status:** approved
**Quest:** SQ-0222
**Area:** MAP / routing (`crates/mapper/src/route/mod.rs`)

## Problem

On the real map (`ZCODE-52-871125-4B37`), after the SQ-0216 up/down work, exactly one
residual illegal overlap remains (2 cells, layer 0). It is a routing artifact, not a
placement one — all room positions are correct:

- Room 22 sits directly north of room 78 (same column); their edge is an Up/Down
  **reciprocal**, drawn as a straight vertical line down the shared column.
- Room 131 is NE of room 78; the edge `78 → 131` is a **one-way, weaving** compass edge
  (semantic **N**, reverse edge `131 → 78` is **SW** — bidirectional but *not* a cardinal
  N/S reciprocal).

Both endpoints land on room 78's **Top** side. `assign_side_slots` currently ranks
Up/Down **after** compass for the center slot (SQ-0216: "up/down yields the center to a
compass reciprocal"). So the weaving one-way compass connector takes the center slot and
the **straight** Up/Down reciprocal is pushed to an offset slot — forcing the renderer
(`attach_bridge`) to jog the straight line sideways one cell in the gap row. That jog runs
parallel to the compass connector's lateral run for two cells → an illegal parallel
overlap (not a legal N/S×E/W crossing).

## Root cause

SQ-0216's yield rule is too broad. It was meant to keep a **reciprocal** N/S or E/W
compass connector (which is column/row-locked) centered. Applied to a **non-reciprocal,
weaving** compass connector, it wrongly displaces a straight Up/Down line, manufacturing
the overlap. `assign_side_slots` already has a "straight-through connector keeps the center
so it stays a clean line" rule — it is simply outranked by the up/down-yield.

## Fix

Refine the center-slot priority in `assign_side_slots` so that, on a shared `(room, side)`:

1. an **axis-reciprocal compass** connector (reciprocal AND cardinal-opposite exit/entry:
   N↔S or E↔W — the column/row-locked case SQ-0216 cares about) keeps the center; then
2. a **straight-through** connector (collinear polyline) keeps the center; then
3. everything else, with Up/Down losing only on ties.

**Gating (byte-identity guard):** apply the new ordering only to sides that actually host
an Up/Down endpoint. Sides with no Up/Down connector keep the exact prior ordering
(`(is_updown, Reverse(straight), ci, is_exit)`), so all compass-only layouts remain
**byte-identical**. This preserves the SQ-0216 compass-identity invariant.

`axis_reciprocal(c)` is a pure function of the connector:
`c.reciprocal && (c.exit_dir, c.entry_dir) ∈ {(N,S),(S,N),(E,W),(W,E)}`.

### Sort key (for a group that contains an Up/Down endpoint)

`(Reverse(axis_reciprocal), Reverse(straight), is_updown, ci, is_exit)`

- Reciprocal N/S / E/W compass → center (unchanged from SQ-0216 intent).
- Else straight line → center (fixes the bug: straight Up/Down beats a weaving one-way compass).
- Else compass before Up/Down, then connector index (deterministic).

For a group with **no** Up/Down endpoint, keep the original key unchanged.

## Behavioral delta

- Compass-only sides: **no change** (byte-identical).
- Mixed Up/Down + compass sides: a **straight** Up/Down connector now keeps its center slot
  when the competing compass connector is **non-axis-reciprocal and weaving** (previously the
  compass won). A reciprocal N/S/E/W compass, and a straight compass, still keep center.

## Verification

- **Oracle (real map):** the full per-layer tidy pipeline on `ZCODE-52-871125-4B37`
  drops from 2 illegal cells to **0** (the parallel-overlap becomes a legal crossing).
  Measured via a throwaway harness during design (2→0, crossings 3→4).
- **Regression test (mapper):** minimal graph — room A with a straight Up/Down reciprocal
  to a due-north room B and a weaving one-way compass edge (N) to a NE room C, both exits on
  A's Top side. Assert the Up/Down connector gets `exit_slot == 0` and the compass connector
  `exit_slot != 0`.
- **Preserve SQ-0216:** `ns_reciprocal_outranks_updown_for_center_slot` still passes (the
  axis-reciprocal compass keeps center even vs a straight Up/Down).
- Full `mapper` + `app` suites green (no snapshot churn on compass-only layouts).

## Out of scope

- Exit-side reselection (moving the N arrow to the E side) — rejected (arrow semantics /
  compass byte-identity risk).
- Lane-accounting / channel-widening — rejected (render/mapper granularity gap, width churn).
- SQ-0221 path-cell cleanup destinations — discarded (the residual is not move-fixable).
