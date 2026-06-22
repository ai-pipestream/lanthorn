# Bidirectional-Chain Alignment + Contiguity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Force bidirectional cardinal chains onto one row/column (zero-distortion reciprocal edges) and keep them contiguous (no foreign room interleaving), and surface which alignment rules placed each room in the dump legend and a toggled in-box code.

**Architecture:** A pure `detect_chains(graph)` (union-find over reciprocal E/W and N/S edges) feeds three consumers: the constraint builder adds Y-equality (E/W chains) / X-equality (N/S chains) to the VPSC solve; a contiguity pass in `relayout_auto` packs each chain into consecutive cells and bumps foreign rooms aside; the dump and renderer derive the same chains to display rules.

**Tech Stack:** Rust 2021, crate `mapper` (layout) + crate `app` (dump, render, input). `cargo test -p mapper`, `cargo test -p app`, `cargo clippy --workspace --all-targets`.

## Global Constraints

- Integer grid cells; the solver works in f64 and snaps to i32. Determinism mandatory: union-find/iteration in array/sorted order, chain ids assigned in ascending lowest-member order, no RNG, no HashMap ordering affecting positions.
- Rooms never overlap after `relayout_auto`.
- Conflicts degrade gracefully: an equality that closes a cycle on its axis is dropped and its connection index recorded in `dropped` (→ `distorted`), via the existing `creates_cycle` mechanism.
- `relayout_auto(graph: &mut MapGraph)` keeps its exact public name/signature. Per-turn incremental placement, the app cleanup, and persistence are unchanged.
- "Bidirectional E/W edge" between A and B = `A→{E|W}→B` AND `B→opposite→A` both exist. Use `crate::direction::opposite`. E/W: `grid_offset` has `dx!=0 && dy==0`; N/S: `dy!=0 && dx==0`.
- The in-box alignment code is a pure overlay: with the toggle OFF, rendering is byte-identical to today.
- Do NOT run `git checkout`/`git restore`/`git stash`. Commit forward only.

---

### Task 1: `detect_chains` — bidirectional cardinal chains

**Files:**
- Create: `crates/mapper/src/layout/chains.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod chains;` and `pub use chains::{detect_chains, Chains};`)

**Interfaces:**
- Consumes: `crate::direction::{grid_offset, opposite}`, `crate::graph::{MapGraph, RoomId}`.
- Produces:
  - `pub struct Chains { pub ew: BTreeMap<RoomId, usize>, pub ns: BTreeMap<RoomId, usize>, pub ew_members: Vec<Vec<RoomId>>, pub ns_members: Vec<Vec<RoomId>> }`
  - `pub fn detect_chains(graph: &MapGraph) -> Chains` — `ew`/`ns` map a room to its chain id on that axis (absent if in no bidirectional pair on that axis); `*_members[id]` lists that chain's sorted member ids.

- [ ] **Step 1: Write the failing tests** (in `chains.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    #[test]
    fn reciprocal_ew_pair_is_one_chain() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal E/W
        let c = detect_chains(&g);
        assert_eq!(c.ew.get(&1), c.ew.get(&2), "both in the same E/W chain");
        assert!(c.ew.get(&1).is_some());
        assert!(c.ns.is_empty(), "no N/S chain");
    }

    #[test]
    fn non_reciprocal_pair_is_no_chain() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2); // one-way; no 2→W→1
        let c = detect_chains(&g);
        assert!(c.ew.is_empty(), "one-way edge forms no chain");
    }

    #[test]
    fn same_origin_n_and_s_is_no_chain() {
        // 3→N→7 and 3→S→7: same origin, not reciprocal (no 7→S→3 / 7→N→3).
        let mut g = MapGraph::new();
        g.upsert_room(3, "a".into());
        g.upsert_room(7, "b".into());
        g.add_edge(3, Direction::N, 7);
        g.add_edge(3, Direction::S, 7);
        let c = detect_chains(&g);
        assert!(c.ns.is_empty(), "same-origin N+S is not a reciprocal pair");
    }

    #[test]
    fn three_room_ew_chain_and_cross_chain_room() {
        // 79↔203↔193 is one E/W chain; 74↔76 is an N/S chain; 74↔79 puts 74 in the E/W chain too.
        let mut g = MapGraph::new();
        for id in [74u16, 76, 79, 193, 203] { g.upsert_room(id, "r".into()); }
        for (o, d, dst) in [
            (79, Direction::W, 203), (203, Direction::E, 79),
            (203, Direction::W, 193), (193, Direction::E, 203),
            (74, Direction::W, 79), (79, Direction::E, 74),
            (74, Direction::S, 76), (76, Direction::N, 74),
        ] { g.add_edge(o, d, dst); }
        let c = detect_chains(&g);
        // 74,79,203,193 all share one E/W chain.
        let e = c.ew.get(&74).copied();
        assert!(e.is_some());
        assert_eq!(c.ew.get(&79).copied(), e);
        assert_eq!(c.ew.get(&203).copied(), e);
        assert_eq!(c.ew.get(&193).copied(), e);
        // 74 and 76 share one N/S chain.
        assert_eq!(c.ns.get(&74), c.ns.get(&76));
        assert!(c.ns.get(&74).is_some());
        // 74 is a cross-chain room: in an E/W chain AND an N/S chain.
        assert!(c.ew.contains_key(&74) && c.ns.contains_key(&74));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p mapper chains:: 2>&1 | tail -15`
Expected: FAIL — `detect_chains`/`Chains` not found.

- [ ] **Step 3: Implement `chains.rs`**

```rust
//! Bidirectional cardinal chains: maximal runs of rooms joined by reciprocal E/W
//! (share a row) or reciprocal N/S (share a column) edges. A pure function of the
//! graph, used by the layout (alignment + contiguity) and the rules display.

use std::collections::BTreeMap;

use crate::direction::{grid_offset, opposite};
use crate::graph::{MapGraph, RoomId};

pub struct Chains {
    /// room → its E/W chain id (rooms sharing a row), if any.
    pub ew: BTreeMap<RoomId, usize>,
    /// room → its N/S chain id (rooms sharing a column), if any.
    pub ns: BTreeMap<RoomId, usize>,
    pub ew_members: Vec<Vec<RoomId>>,
    pub ns_members: Vec<Vec<RoomId>>,
}

pub fn detect_chains(graph: &MapGraph) -> Chains {
    let conns = graph.connections();
    let reciprocal = |a: RoomId, b: RoomId, dir| {
        conns.iter().any(|c| c.origin == b && c.dest == a && c.dir == opposite(dir))
    };
    let mut ew_pairs: Vec<(RoomId, RoomId)> = Vec::new();
    let mut ns_pairs: Vec<(RoomId, RoomId)> = Vec::new();
    for c in conns {
        match grid_offset(c.dir) {
            Some((dx, dy)) if dx != 0 && dy == 0 => {
                if reciprocal(c.origin, c.dest, c.dir) {
                    ew_pairs.push((c.origin, c.dest));
                }
            }
            Some((dx, dy)) if dy != 0 && dx == 0 => {
                if reciprocal(c.origin, c.dest, c.dir) {
                    ns_pairs.push((c.origin, c.dest));
                }
            }
            _ => {}
        }
    }
    let (ew, ew_members) = build(&ew_pairs);
    let (ns, ns_members) = build(&ns_pairs);
    Chains { ew, ns, ew_members, ns_members }
}

/// Union-find the pairs, then assign chain ids in ascending lowest-member order
/// (deterministic). Returns (room→chain id, chain id→sorted members).
fn build(pairs: &[(RoomId, RoomId)]) -> (BTreeMap<RoomId, usize>, Vec<Vec<RoomId>>) {
    // Union-find over the room ids present in `pairs`.
    let mut parent: BTreeMap<RoomId, RoomId> = BTreeMap::new();
    fn find(parent: &mut BTreeMap<RoomId, RoomId>, x: RoomId) -> RoomId {
        let p = *parent.get(&x).unwrap_or(&x);
        if p == x {
            x
        } else {
            let r = find(parent, p);
            parent.insert(x, r);
            r
        }
    }
    for &(a, b) in pairs {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            // Union toward the smaller root for determinism.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
        }
    }
    // Group members by root.
    let members_by_root: BTreeMap<RoomId, Vec<RoomId>> = {
        let ids: Vec<RoomId> = parent.keys().copied().collect();
        let mut m: BTreeMap<RoomId, Vec<RoomId>> = BTreeMap::new();
        for id in ids {
            let r = find(&mut parent, id);
            m.entry(r).or_default().push(id);
        }
        m
    };
    // Assign chain ids in ascending root order; only keep groups of ≥2.
    let mut room_chain: BTreeMap<RoomId, usize> = BTreeMap::new();
    let mut chains: Vec<Vec<RoomId>> = Vec::new();
    for (_root, mut members) in members_by_root {
        if members.len() < 2 {
            continue;
        }
        members.sort_unstable();
        let id = chains.len();
        for &r in &members {
            room_chain.insert(r, id);
        }
        chains.push(members);
    }
    (room_chain, chains)
}
```

- [ ] **Step 4: Run to confirm pass**

Run: `cargo test -p mapper chains:: 2>&1 | tail -15` — Expected: 4 PASS.
Then `cargo clippy -p mapper --all-targets 2>&1 | tail -8` — Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/chains.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): detect bidirectional cardinal chains (E/W rows, N/S columns)"
```

---

### Task 2: Alignment — chain equality constraints

**Files:**
- Modify: `crates/mapper/src/layout/constraints.rs` (`build_axis_constraints`)
- Test: `crates/mapper/src/layout/constraints.rs` `#[cfg(test)]` and an integration test in `mod.rs`.

**Interfaces:**
- Consumes: `super::chains::detect_chains` (Task 1), the existing `creates_cycle`, `vpsc::Constraint`.
- Produces: `build_axis_constraints` keeps its signature `(graph, ids, gap) -> AxisConstraints` but now also emits, for chain members within `ids`, **Y-equality** for E/W chains and **X-equality** for N/S chains (each equality = two `gap=0` separations, cycle-checked).

**Algorithm:** equality `coord[a] == coord[b]` is two constraints `a≤b` and `b≤a` (gap 0). Add equalities for each adjacent pair of a chain's members (sorted), on the perpendicular axis: E/W chain → equality on **Y**; N/S chain → equality on **X**. Run each through the SAME `creates_cycle` guard already used for directional constraints (so a contradictory equality is dropped). Equalities are added **before** the directional loop so a chain row/column is established first.

- [ ] **Step 1: Write the failing tests**

```rust
// in mod.rs tests (end-to-end through the constraint engine):
    #[test]
    fn reciprocal_ew_pair_shares_a_row() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal E/W → same row
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.1, p2.1, "reciprocal E/W pair must share a row: {p1:?} {p2:?}");
        assert!(p2.0 > p1.0, "and 2 is east of 1");
    }

    #[test]
    fn reciprocal_ns_pair_shares_a_column() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1); // reciprocal N/S → same column
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.0, p2.0, "reciprocal N/S pair must share a column");
        assert!(p2.1 < p1.1, "and 2 is north of 1");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p mapper reciprocal_ew_pair_shares_a_row reciprocal_ns_pair_shares_a_column 2>&1 | tail -15`
Expected: FAIL — without equality, stress need not place them on the exact same row/column.

- [ ] **Step 3: Add chain equalities in `build_axis_constraints`**

In `crates/mapper/src/layout/constraints.rs`, inside `build_axis_constraints`, AFTER the
`index`/`x_adj`/`y_adj`/`dropped` are initialized and BEFORE the `for (ci, conn) ...`
directional loop, insert:

```rust
    // Chain equalities: reciprocal E/W chains share a row (equality on Y); reciprocal N/S
    // chains share a column (equality on X). Equality coord[a]==coord[b] = a≤b and b≤a
    // (gap 0). Cycle-closing equalities are skipped (graceful conflict handling).
    let chains = super::chains::detect_chains(graph);
    let mut add_equality = |left: usize, right: usize,
                            adj: &mut Vec<Vec<usize>>, out: &mut Vec<Constraint>| {
        // a ≤ b
        if !creates_cycle(adj, left, right) {
            adj[left].push(right);
            out.push(Constraint { left, right, gap: 0.0 });
        }
        // b ≤ a
        if !creates_cycle(adj, right, left) {
            adj[right].push(left);
            out.push(Constraint { left: right, right: left, gap: 0.0 });
        }
    };
    for members in &chains.ew_members {
        for w in members.windows(2) {
            if let (Some(&a), Some(&b)) = (index.get(&w[0]), index.get(&w[1])) {
                add_equality(a, b, &mut y_adj, &mut y); // E/W chain → equal Y
            }
        }
    }
    for members in &chains.ns_members {
        for w in members.windows(2) {
            if let (Some(&a), Some(&b)) = (index.get(&w[0]), index.get(&w[1])) {
                add_equality(a, b, &mut x_adj, &mut x); // N/S chain → equal X
            }
        }
    }
```

(`index`, `x_adj`, `y_adj`, `x`, `y` are the locals already declared in `build_axis_constraints`. The closure borrows them per call; if the borrow checker objects to the `&mut` closure capturing, inline the two `add_equality` bodies instead of using a closure.)

- [ ] **Step 4: Run to confirm pass + full suite**

Run: `cargo test -p mapper 2>&1 | tail -20`
Expected: the two new tests PASS. The existing `constraint_engine_beats_sort_distortion_on_a129` must still pass (equalities reduce or hold distortion, never increase it past the sort baseline). If it regresses, report it — do not weaken it.
Then `cargo clippy --workspace --all-targets 2>&1 | tail -8`.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/constraints.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): align bidirectional chains via row/column equality constraints"
```

---

### Task 3: Contiguity — compaction + foreign-room bump

**Files:**
- Modify: `crates/mapper/src/layout/mod.rs` (`relayout_auto`, after the alignment pass / before final collision resolution)
- Test: `mod.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `chains::detect_chains`, the existing `place_preserving_alignment`.
- Produces: after the constraint solve, each chain's members occupy consecutive cells on their shared row/column and no foreign room sits between two consecutive members.

**Algorithm (per component, on the snapped + aligned `snapped: Vec<(i32,i32)>` and `index`):**
For each E/W chain whose members are all in this component: collect member dense-indices, confirm they share a row `y` (they do after equality; if not, skip — a dropped equality), sort by current `x`, set them to consecutive `(x0+k, y)`. Any non-member room currently at one of those cells is relocated with `place_preserving_alignment` to a free cell off the row. Symmetric for N/S chains (consecutive `(x, y0+k)`). The existing final `nearest_free_cell`/`place_preserving_alignment` collision loop then guarantees no overlap.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn bidirectional_chain_is_contiguous_no_interleave() {
        // 79↔203↔193 (E/W chain) plus a foreign room 180 with no edge to the chain.
        let mut g = crate::graph::MapGraph::new();
        for id in [79u16, 180, 193, 203] { g.upsert_room(id, "r".into()); }
        for (o, d, dst) in [
            (79, Direction::W, 203), (203, Direction::E, 79),
            (203, Direction::W, 193), (193, Direction::E, 203),
        ] { g.add_edge(o, d, dst); }
        // 180 connected loosely so it shares the component but has no chain edge.
        g.add_edge(180, Direction::S, 79);
        g.add_edge(79, Direction::N, 180);
        relayout_auto(&mut g);
        let p = |id| g.room(id).unwrap().pos.unwrap();
        let (a, b, c) = (p(193), p(203), p(79));
        // All three on one row, consecutive in x.
        assert_eq!(a.1, b.1);
        assert_eq!(b.1, c.1);
        let mut xs = [a.0, b.0, c.0];
        xs.sort_unstable();
        assert_eq!(xs[1] - xs[0], 1, "chain members consecutive");
        assert_eq!(xs[2] - xs[1], 1, "chain members consecutive");
        // 180 is NOT between two consecutive chain members on that row.
        let p180 = p(180);
        let between = p180.1 == a.1 && p180.0 > xs[0] && p180.0 < xs[2];
        assert!(!between, "foreign room must not interleave the chain: 180={p180:?}, chain xs={xs:?}");
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p mapper bidirectional_chain_is_contiguous_no_interleave 2>&1 | tail -15`
Expected: FAIL — without compaction a chain may have a gap a foreign room fills.

- [ ] **Step 3: Implement the contiguity pass**

In `relayout_auto`, after the free-axis alignment block and BEFORE the "Pack this component"
step (so it runs on `snapped`/`index` in local component coordinates), add a call
`contiguify(&chains_for_comp, comp, &index, &mut snapped);` and define a private helper
`fn contiguify(chains: &Chains, comp: &[RoomId], index: &BTreeMap<RoomId, usize>, snapped: &mut [(i32,i32)])`:

```rust
fn contiguify(chains: &Chains, comp: &[RoomId], index: &BTreeMap<RoomId, usize>, snapped: &mut [(i32, i32)]) {
    // Build a quick "is this index a member of a chain we're packing" set as we go.
    // E/W chains → consecutive x on a shared row; N/S chains → consecutive y on a shared column.
    let occupied_at = |snapped: &[(i32, i32)], cell: (i32, i32), except: usize| -> Option<usize> {
        (0..snapped.len()).find(|&i| i != except && snapped[i] == cell)
    };
    for members in &chains.ew_members {
        let mut idxs: Vec<usize> = members.iter().filter_map(|id| index.get(id).copied()).collect();
        if idxs.len() < 2 { continue; }
        let y = snapped[idxs[0]].1;
        if !idxs.iter().all(|&i| snapped[i].1 == y) { continue; } // dropped equality → skip
        idxs.sort_by_key(|&i| (snapped[i].0, i));
        let x0 = snapped[idxs[0]].0;
        for (k, &i) in idxs.iter().enumerate() {
            let target = (x0 + k as i32, y);
            if let Some(j) = occupied_at(snapped, target, i) {
                // bump the foreign room off this row
                let occ: std::collections::BTreeSet<(i32, i32)> =
                    (0..snapped.len()).filter(|&q| q != j).map(|q| snapped[q]).collect();
                snapped[j] = super::nearest_free_cell(&occ, (target.0, target.1 + 1));
            }
            snapped[i] = target;
        }
    }
    for members in &chains.ns_members {
        let mut idxs: Vec<usize> = members.iter().filter_map(|id| index.get(id).copied()).collect();
        if idxs.len() < 2 { continue; }
        let x = snapped[idxs[0]].0;
        if !idxs.iter().all(|&i| snapped[i].0 == x) { continue; }
        idxs.sort_by_key(|&i| (snapped[i].1, i));
        let y0 = snapped[idxs[0]].1;
        for (k, &i) in idxs.iter().enumerate() {
            let target = (x, y0 + k as i32);
            if let Some(j) = occupied_at(snapped, target, i) {
                let occ: std::collections::BTreeSet<(i32, i32)> =
                    (0..snapped.len()).filter(|&q| q != j).map(|q| snapped[q]).collect();
                snapped[j] = super::nearest_free_cell(&occ, (target.0 + 1, target.1));
            }
            snapped[i] = target;
        }
    }
}
```

`chains_for_comp` is `detect_chains(graph)` computed once before the component loop (chains never cross components, so the global result is correct; the per-component `index.get` filters to this component's members). Reference: `nearest_free_cell` is in `mod.rs` (call as `nearest_free_cell(...)` — already in scope; the `super::` in the helper is because the helper is a free fn in the same module — use the unqualified name if the helper sits in `mod.rs`).

- [ ] **Step 4: Run the test + full suite**

Run: `cargo test -p mapper 2>&1 | tail -20`
Expected: `bidirectional_chain_is_contiguous_no_interleave` PASSES; all prior tests still pass (no overlap, determinism, win test). `cargo clippy --workspace --all-targets`.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): keep bidirectional chains contiguous, bump foreign rooms aside"
```

---

### Task 4: Dump legend — alignment annotation

**Files:**
- Modify: `crates/app/src/map_dump.rs` (the `ROOM` legend line, currently `map_dump.rs:132`)
- Test: `map_dump.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `mapper::layout::detect_chains` (Task 1), `mapper::graph` (the `distorted` flag).
- Produces: each `ROOM` line gains ` align=<…>` and, if any of the room's own compass edges are distorted, ` dropped=[…]`.

**Format:** after the existing `pos=…{notes}` text, append:
- `align=row[<sorted member ids joined by ,>]` if the room is in an E/W chain, and/or `col[<ids>]` if in an N/S chain (space-separated if both); `align=none` if in neither.
- `dropped=[<origin>→<DIR>→<dest>, …]` listing the room's outgoing distorted compass edges (omit if none).

- [ ] **Step 1: Write the failing test** (`map_dump.rs` tests)

```rust
    #[test]
    fn dump_legend_shows_alignment_rules() {
        use mapper::direction::Direction;
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        // make it a reciprocal E/W pair so a chain forms
        m.graph.add_edge(2, Direction::W, 1);
        let dump = render_dump(&m.graph);
        assert!(dump.contains("align=row[1,2]"), "reciprocal pair annotated as a row chain:\n{dump}");
    }

    #[test]
    fn dump_legend_marks_ungrouped_room() {
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "A", None);
        let dump = render_dump(&m.graph);
        assert!(dump.contains("align=none"), "lone room shows align=none:\n{dump}");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p app dump_legend 2>&1 | tail -15` — Expected: FAIL (no `align=` text yet).

- [ ] **Step 3: Implement the annotation**

In `render_dump` (the function building the ROOM legend), compute `let chains = mapper::layout::detect_chains(graph);` once before the room loop, and change the `ROOM` push line to append the alignment text. Build a helper that, for a room id, returns the ` align=…` + optional ` dropped=[…]` string from `chains` and `graph.connections()`. Keep the existing `ROOM {id} {label:?} pos={pos}{notes}` prefix; append the alignment text before the newline. Exact member-id formatting: `row[` + members joined with `,` + `]` (members come sorted from `Chains`).

- [ ] **Step 4: Run to confirm pass**

Run: `cargo test -p app dump_legend 2>&1 | tail -10` and `cargo test -p app 2>&1 | grep 'test result'` — Expected: PASS, suite green. `cargo clippy -p app --all-targets`.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/map_dump.rs
git commit -m "feat(app): annotate dump legend with per-room alignment rules"
```

---

### Task 5: In-box alignment code + `Ctrl+A` toggle

**Files:**
- Modify: `crates/mapper/src/render.rs` (add `pub align_code: String` to `RenderRoom`; populate it in `render()` from `detect_chains`)
- Modify: `crates/app/src/state.rs` (`AppState.show_alignment: bool`, default false; init in `Default`)
- Modify: `crates/app/src/input.rs` (`Action::ToggleAlignment`; `Ctrl+A` in the global ctrl block; `apply_action` flips `state.show_alignment`)
- Modify: `crates/app/src/render/map.rs` (when `state.show_alignment`, draw `room.align_code` in each room box)
- Modify: `crates/app/src/main.rs` (help bar: add `Ctrl+A: align` to the Game-focus help text)
- Test: `input.rs` `#[cfg(test)]` and `render/map.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `RenderRoom.align_code` (precomputed in `mapper::render::render` from `detect_chains`), `AppState.show_alignment`.
- Produces: `Ctrl+A` toggles `show_alignment`; when on, chained rooms show their `R`/`C` code; when off, rendering is byte-identical to today.
- The renderer needs NO graph access — it reads `room.align_code` off each `RenderRoom`.

- [ ] **Step 1: Write the failing tests**

```rust
// input.rs tests:
    #[test]
    fn ctrl_a_toggles_alignment_overlay() {
        let s = AppState::default();
        assert!(!s.show_alignment, "off by default");
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::ToggleAlignment));
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(s.show_alignment, "toggled on");
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(!s.show_alignment, "toggled off");
    }
```

```rust
// render/map.rs tests — overlay is inert when off, present when on:
    #[test]
    fn alignment_overlay_off_by_default_then_shows_code() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal → row chain
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 160, 60);
        let render_buf = |show: bool| {
            let mut st = AppState::default();
            st.zoom = Zoom::Boxes;
            st.scroll = rm.bounds.0;
            st.show_alignment = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf
        };
        let off = render_buf(false);
        let on = render_buf(true);
        assert_ne!(format!("{off:?}"), format!("{on:?}"), "overlay changes the buffer when on");
        // an 'R' appears somewhere only when on
        let has_r = |b: &Buffer| (0..area.width).any(|x| (0..area.height).any(|y|
            b.cell((x, y)).map(|c| c.symbol() == "R").unwrap_or(false)));
        assert!(!has_r(&off));
        assert!(has_r(&on), "row-chain code R appears when overlay on");
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p app ctrl_a_toggles_alignment_overlay alignment_overlay_off_by_default_then_shows_code 2>&1 | tail -20`
Expected: FAIL — `show_alignment`/`Action::ToggleAlignment` absent.

- [ ] **Step 3: Implement**

1. `render.rs` (mapper): add `pub align_code: String` to `RenderRoom`. In `render()`, after positions are known, compute `let chains = crate::layout::detect_chains(graph);` once and set each room's `align_code`: `R{ew_id}` if `chains.ew` has the room and/or `C{ns_id}` if `chains.ns` has it (space-joined, e.g. `"R2 C1"`), else `""`. Update the existing `RenderRoom { … }` literal(s) in `render()` to include the new field; fix any other `RenderRoom` construction sites the compiler flags.
2. `state.rs`: add `pub show_alignment: bool` to `AppState`; set `show_alignment: false` in `Default`.
3. `input.rs`: add `ToggleAlignment` to `enum Action`; in the global `if ctrl { match key.code { … } }` add `KeyCode::Char('a') => Action::ToggleAlignment,`; in `apply_action`'s normal dispatch add `Action::ToggleAlignment => state.show_alignment = !state.show_alignment,`.
4. `render/map.rs`: in `render_map`'s per-room drawing, when `state.show_alignment` AND `!room.align_code.is_empty()`, draw `room.align_code` on the box's TOP interior row, clipped to the interior width, into blank cells only (never overwrite the room id text or an exit arrow — check the target cell is currently blank before writing). Off → skip entirely (no buffer change).
5. `main.rs`: append `Ctrl+A: align` to the Game-focus help string.

- [ ] **Step 4: Run tests + full suite + clippy**

Run: `cargo test -p app 2>&1 | grep -E 'test result|FAILED' | tail` — Expected: green, including the two new tests. `cargo clippy --workspace --all-targets`.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/input.rs crates/app/src/render/map.rs crates/app/src/main.rs
git commit -m "feat(app): Ctrl+A toggles in-box alignment codes (R/C chain ids)"
```

---

## Self-Review

**Spec coverage:**
- ✅ Chain detection (Task 1) — `detect_chains`, deterministic, reciprocal-only.
- ✅ Alignment via equality constraints (Task 2) — E/W→row, N/S→column, cycle-dropped → distorted.
- ✅ Contiguity via compaction + foreign bump (Task 3) — consecutive cells, `place_preserving_alignment`/`nearest_free_cell` keep no-overlap.
- ✅ Conflict handling — `creates_cycle` guard on equalities (Task 2), members-share-axis check skips dropped equalities (Task 3).
- ✅ Rules display: dump legend `align=`/`dropped=` (Task 4); in-box `R/C` code + `Ctrl+A` toggle, byte-identical when off (Task 5).
- ✅ Determinism, no-overlap, integer grid, `relayout_auto` signature — Global Constraints + tests.
- ✅ Deferred general clustering — not in any task (correctly out of scope).

**Placeholder scan:** the only soft spot is Task 5 Step 3's "use whichever the renderer already has" for accessing the graph in `render_map` — this is a real codebase-read instruction, not a TODO; the implementer confirms `rm.graph`/render data by reading `render.rs`. Acceptable (it's a wiring detail, the behavior + tests are fully specified).

**Type consistency:** `Chains{ew,ns,ew_members,ns_members}`, `detect_chains`, `build_axis_constraints`, `place_preserving_alignment`, `nearest_free_cell`, `Action::ToggleAlignment`, `AppState.show_alignment` are consistent across tasks. The contiguity helper operates on the same `snapped`/`index` locals the constraint path already uses.

**Risk flagged to controller:** if Task 2's equalities make `constraint_engine_beats_sort_distortion_on_a129` regress, that's a real finding (Task 2 Step 4 says report, don't weaken). The in-box code's collision-avoidance with the room id/arrow (Task 5) is the fiddliest part; its test asserts an `R` appears and that off==today.
