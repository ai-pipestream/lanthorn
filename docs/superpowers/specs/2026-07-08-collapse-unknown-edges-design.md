# Collapse redundant Unknown-direction edges (SQ-0220)

**Status:** approved
**Quest:** SQ-0220
**Area:** MAP / graph (`crates/mapper/src/graph.rs`, `mapper.rs`, `persist.rs`)

## Problem

An edge whose triggering command did not parse to a direction is stored with
`Direction::Unknown` (`Mapper::observe`: `via.unwrap_or(Direction::Unknown)`). Unknown
comes from teleports, `xyzzy`, `board`, `climb`, scripted/forced moves, etc. (`enter` /
`exit` / `in` / `out` / `up` / `down` already parse to real directions).

An Unknown edge renders as a `?` stub and contributes no layout offset
(`grid_offset(Unknown) == None`). When the same room pair is ALSO connected by a real
directional edge in the same direction of travel, that `?` is redundant clutter drawn
alongside the real connector.

## Rule

Remove an Unknown edge `A→B` **iff** the same pair already has a known-direction edge with
the **same origin→dest** — i.e. another edge with `origin == A`, `dest == B`, and
`dir != Unknown`. The known edge stays; the redundant `?` is dropped.

- **Reverse edges do NOT count.** A `B→A` known edge is ignored: return trips are not
  guaranteed to be the geometric opposite (one-way passages, mazes), so we never infer a
  forward direction from the return trip.
- **No relabeling.** The "specific known direction that replaces it" already exists as its
  own edge, so the Unknown is simply removed, never rewritten.
- An Unknown edge with no same-direction known counterpart is left untouched (this includes
  reverse-only pairs and lone `?→X` edges).

## Implementation

New method on `MapGraph`:

```rust
/// Drop every Unknown-direction edge whose room pair (same origin→dest) already carries a
/// known-direction edge; the redundant `?` stub goes, the known edge stays. Reverse
/// (dest→origin) edges do NOT count. Returns the number of edges removed. (SQ-0220)
pub fn collapse_unknown_edges(&mut self) -> usize {
    let known: std::collections::HashSet<(RoomId, RoomId)> = self
        .conns
        .iter()
        .filter(|c| c.dir != Direction::Unknown)
        .map(|c| (c.origin, c.dest))
        .collect();
    let before = self.conns.len();
    self.conns
        .retain(|c| c.dir != Direction::Unknown || !known.contains(&(c.origin, c.dest)));
    before - self.conns.len()
}
```

Call sites:

1. **On load** — `persist::from_json`, after `MapGraph::from_parts`, so every load path
   (app `load_map`, `main.rs`) collapses existing saved maps.
2. **Live during play** — `Mapper::observe`, immediately after `add_edge`, in BOTH layout
   modes (edge hygiene is independent of layout). This fires whether the Unknown arrives
   first and a directional move follows, or the reverse.

O(E) per call; consistent with `observe`'s existing per-move full-graph `mark_distorted`.

## Verification (tests)

`crates/mapper/src/graph.rs`:
- Removes an Unknown `A→B` when a known `A→B` edge exists; returns 1.
- Keeps an Unknown `A→B` when only the reverse `B→A` has a known dir.
- Keeps an Unknown `A→B` with no known counterpart at all.
- Does not touch known edges or Unknowns to a different dest (`A→C`).

`crates/mapper/src/mapper.rs`:
- `observe` Unknown then a directional move over the same pair ⇒ the Unknown is gone.
- `observe` directional first then an Unknown over the same pair ⇒ the Unknown never persists.

`crates/mapper/src/persist.rs`:
- `from_json` of a map containing a redundant Unknown + known edge ⇒ loaded graph has the
  Unknown collapsed; a lone Unknown survives the round trip.

## Out of scope

- Inferring direction from reverse/return-trip edges (rejected: not guaranteed opposite).
- Merging/deduping distinct passages that are genuinely both present.
- Any layout/rendering change beyond the `?` stub disappearing once its edge is removed.
