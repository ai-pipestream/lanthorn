# Draw up/down alongside compass, and offset shared-side slots toward the partner (SQ-0223-followup)

**Status:** approved
**Area:** MAP / routing (`crates/mapper/src/route/mod.rs`), rendering slot geometry
**Motivating map:** `ZCODE-52-871125-4B37` (rooms 230, 247↔5, 22↔23)

## Context

On the user's map, after a refresh (both the silent `tidy_layer_silent` and the animated
`run_tidy_pipeline` produce the identical layout — verified), three symptoms remain in a
layout the overlap metric calls clean `(0 illegal, 4 crossings)`:

1. **Room 230** — `217→230` arrives on 230's Top at slot 1 while the `230↔134` reciprocal
   holds Top/slot 0; its stub jogs across the reciprocal (a "legal crossing," but a visual
   overlap).
2. **`23→22` down not drawn** — the pair 22↔23 has a compass edge (`23 E 22`, distorted)
   plus up/down edges (`22 Up 23`, `23 Down 22`); SQ-0219 suppresses the up/down connector.
3. **`247↔5`** — same shape as #1: `5→247` (Down) arrives Top/slot 1 while `247↔167`
   reciprocal holds Top/slot 0; its stub crosses the reciprocal.

The user wants both paths of a pair drawn individually (fixes #2) and the shared-side
crossings gone (fixes #1 and #3).

## Part A — up/down is a first-class connector even when the pair has a compass edge

Today `route_topology` (route/mod.rs ~646-660) drops an Up/Down edge from the routing
working set entirely when its unordered room pair also has a compass edge (`compass_pairs`).
That is SQ-0219's suppression — the reason `23→22` Down draws nothing.

**Change:** remove that exclusion so an Up/Down edge routes as its own connector regardless
of a shared compass edge. The Up/Down reciprocal pairing (`22 Up 23` + `23 Down 22` → one
vertical connector) and the compass edge (`23 E 22`) then both draw, on different sides
(Up/Down exits Top/Bottom via `route_side`; the compass exits its own side).

**Consequences / guards:**
- The "compass routing stays byte-identical" invariant deliberately no longer holds for a
  pair that has BOTH a compass and an up/down edge — that is the intended behavior change.
  Pure-compass pairs are unaffected.
- SQ-0219's comment warns that an up/down edge left in the `compass` slice can steal the
  `direct_route_losers` first-listed representative for a pair and corrupt the compass
  connector's straight-line contest. Verify the compass connector for a mixed pair still
  routes sensibly; if the contest is perturbed, keep up/down edges out of the
  `direct_route_losers` INPUT (the longest-straight-line contest) while still routing them
  as their own connectors — i.e. suppress only their contest participation, not their
  drawing.
- Update the SQ-0219 tests (`updown_never_pairs_with_a_compass_edge`, the byte-identity
  lock-in test) to the new "both drawn" intent: assert the up/down connector IS produced and
  is a separate connector from the compass one, and that the two land on different sides.

## Part B — offset a shared-side slot toward its partner so its stub never crosses center

`box_edge_anchor` places a connector's border anchor at `slot_offset(slot)` along the side:
slot 0 = centre, slot 1 = +1 (east/south), slot 2 = −1 (west/north), slot 3 = +2, … (parity
= sign, magnitude grows). `assign_side_slots` (route/mod.rs ~924) currently assigns slots
0,1,2,… in priority order (SQ-0222 winner first), so the sign a non-centre connector gets is
an accident of index parity, not geometry.

When a non-centre connector's partner is on the OPPOSITE tangent side from its assigned
offset, its perpendicular stub must run across the centre to reach the anchor — crossing
whatever holds the centre slot (a reciprocal). That is #1 and #3: `217→230` and `5→247` both
got slot 1 (+east) but their partners (217, 5) are west. Forcing them to the west offset
(slot 2) drops the multi-connector cells 4 → 2 (verified by spike).

**Change:** make `assign_side_slots` assign the NON-centre connectors' offsets by partner
geometry, keeping the SQ-0222 centre winner at slot 0:
- For each side, the tangent axis is x for Top/Bottom, y for Left/Right.
- For each non-centre connector, look at its PARTNER room's centre relative to THIS room's
  centre along that tangent axis: partner on the + side → give it a `+` offset (odd slots
  1,3,5…); partner on the − side → a `−` offset (even slots 2,4,6…); exactly on-axis →
  keep deterministic fallback (current index order).
- Within each side/direction group, assign increasing magnitude in the existing priority
  order so cells stay distinct.
- `assign_side_slots` needs room positions; thread the graph (or a `RoomId → pos` map) into
  it. `slot_offset` / `box_edge_anchor` are unchanged (parity encoding already gives the
  needed signs).

**Consequences:** slot assignment changes on any side that has a centre connector plus an
offset connector whose partner is on the − tangent side (previously slot 1/+; now slot 2/−).
This is a general improvement (offset stubs bend toward their partner), so snapshot churn is
expected and acceptable; validate that no NEW illegal overlap or crossing appears.

## Verification

- **Real-map oracle** (`ZCODE-52-871125-4B37`, via a throwaway harness): after A+B, the
  `217→230 × 230↔134` and `5→247 × 247↔167` crossings are gone (multi-connector cells drop),
  the `23↔22` plan contains BOTH a compass E connector and an up/down connector, and
  `render_overlap_stats` stays 0 illegal.
- **Unit tests:**
  - route/mod.rs: a pair with a compass edge + a reciprocal up/down edge yields TWO
    connectors on different sides (Part A).
  - route/mod.rs: `assign_side_slots` gives a non-centre connector whose partner is west a
    negative (even) slot, and one whose partner is east a positive (odd) slot; the centre
    winner keeps slot 0 (Part B).
- Full `mapper` + `app` suites green; update the SQ-0219/SQ-0222 tests that encode the old
  suppression / slot order to the new intent, keeping every protection/alignment assertion.

## Out of scope

- The `5→247 × 167→91` and `28→195 × 143→68` crossings that are path-level (not shared-side
  slot) — not addressed here unless they fall out for free.
- Changing which connector wins the centre slot (SQ-0222 stays as-is).
- Entry-side reselection (routing a connector to a different room side).
