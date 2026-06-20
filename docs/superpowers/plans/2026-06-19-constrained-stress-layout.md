# Constrained Stress-Majorization Layout — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `mapper` crate's greedy grid layout with constrained stress majorization — neato's stress objective subject to hard compass separation constraints — so dense maps lay out cleanly with directions honored and north pinned up.

**Architecture:** Convert `crates/mapper/src/layout.rs` into a `layout/` module: a pure 1-D separation-constraint projector (`vpsc.rs`), a compass→axis-constraint builder with deterministic cycle-breaking (`constraints.rs`), a stress-majorization driver (`stress.rs`), and an orchestrator (`mod.rs`) that seeds with the old greedy layout, solves per connected component, snaps to the integer grid, removes residual overlaps, packs components, anchors, and marks `distorted`.

**Tech Stack:** Rust, `mapper` crate (no new dependencies — pure `f64` math). Tests via `cargo test -p mapper`.

## Global Constraints

- Determinism is mandatory: identical graph ⇒ byte-identical positions on every `relayout_auto` call (fixed iteration count, fixed node ordering by ascending `RoomId`, deterministic seed and cycle-breaking, **no RNG**).
- Room positions remain integer grid cells (`Room.pos: Option<(i32,i32)>`). No renderer, persistence, IFID, save/restore, or DOT-export changes.
- No two placed rooms may share a cell (hard invariant — preserved by overlap resolution).
- `mapper` stays filesystem-free and adds no dependencies.
- Manual mode and `nudge` are untouched.
- Tunable constants (in `layout/mod.rs`): `GAP = 1.0`, `ITERS = 60`, `MAX_NODES = 400`. VPSC tolerance `TOL = 1e-9`.
- `cargo test -p mapper` and `cargo clippy -p mapper` must be clean after every task.

---

## File Structure

- `crates/mapper/src/layout/mod.rs` — was `layout.rs`; orchestration (`relayout_auto`), constants, `seed_layout`, `connected_components`, `mark_distorted`, and retained pub helpers (`LayoutMode`, `occupied_cells`, `nearest_free_cell`, `edge_is_satisfied`, `pair_offset`). Declares `mod vpsc; mod constraints; mod stress;`.
- `crates/mapper/src/layout/vpsc.rs` — `Constraint` type + `solve_axis` projector. Pure, no graph knowledge.
- `crates/mapper/src/layout/constraints.rs` — `AxisConstraints` + `build_axis_constraints` (compass → axis constraints + cycle-break).
- `crates/mapper/src/layout/stress.rs` — `all_pairs_dist` + `stress_layout` (SMACOF + per-axis VPSC projection).

---

### Task 1: Convert `layout.rs` to a `layout/` module directory

**Files:**
- Move: `crates/mapper/src/layout.rs` → `crates/mapper/src/layout/mod.rs`

**Interfaces:**
- Produces: unchanged public surface of the `layout` module (`relayout_auto`, `LayoutMode`, `occupied_cells`, `nearest_free_cell`, `edge_is_satisfied`, `pair_offset`).

- [ ] **Step 1: Move the file**

```bash
mkdir -p crates/mapper/src/layout
git mv crates/mapper/src/layout.rs crates/mapper/src/layout/mod.rs
```

- [ ] **Step 2: Verify the build and tests are unchanged**

Run: `cargo test -p mapper`
Expected: PASS — same test count as before the move (Rust resolves `mod layout` to `layout/mod.rs`; `lib.rs` needs no change).

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "refactor(mapper): convert layout.rs to layout/ module dir"
```

---

### Task 2: VPSC 1-D separation-constraint projector

**Files:**
- Create: `crates/mapper/src/layout/vpsc.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod vpsc;` near the top, after the doc comment / `use` lines)

**Interfaces:**
- Produces: `pub struct Constraint { pub left: usize, pub right: usize, pub gap: f64 }` and `pub fn solve_axis(desired: &[f64], weight: &[f64], constraints: &[Constraint]) -> Vec<f64>`.

Solves, per axis: minimize `Σ weight_i·(x_i − desired_i)²` subject to `x[c.right] − x[c.left] ≥ c.gap`. Uses Dwyer's VPSC block-merge to feasibility (the optional split-for-optimality step is omitted as a documented simplification; re-projecting every stress iteration recovers quality).

- [ ] **Step 1: Add the module declaration to `mod.rs`**

Add this line near the top of `crates/mapper/src/layout/mod.rs` (after the existing `use` statements):

```rust
mod vpsc;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/mapper/src/layout/vpsc.rs`:

```rust
//! Variable Placement with Separation Constraints (VPSC): 1-D projection.
//!
//! Solves, for one axis: minimise `Σ weight_i·(x_i − desired_i)²`
//! subject to `x[c.right] − x[c.left] ≥ c.gap` for every constraint `c`.
//!
//! Implementation: Dwyer's block-merge "satisfy" algorithm. Variables start in
//! singleton blocks; the most-violated cross-block constraint is repeatedly made
//! active by merging its two blocks (which fixes their relative offset and moves
//! the merged block to its weight-optimal position) until no constraint is
//! violated. The optional split-for-optimality pass is omitted: the result is
//! always feasible, and the outer stress loop re-projects each iteration.

/// `x[right] − x[left] ≥ gap`. `left`/`right` are variable indices.
#[derive(Debug, Clone, Copy)]
pub struct Constraint {
    pub left: usize,
    pub right: usize,
    pub gap: f64,
}

const TOL: f64 = 1e-9;

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: &[f64], b: &[f64]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-6, "{a:?} != {b:?}");
        }
    }

    #[test]
    fn feasible_input_unchanged() {
        // Already satisfies x1 - x0 >= 1; projection must not move anything.
        let out = solve_axis(&[0.0, 5.0], &[1.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[0.0, 5.0]);
    }

    #[test]
    fn single_constraint_pushes_to_gap() {
        // desired both 0, equal weight, need x1 - x0 >= 1 → symmetric split to -0.5, 0.5.
        let out = solve_axis(&[0.0, 0.0], &[1.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[-0.5, 0.5]);
    }

    #[test]
    fn chain_of_three() {
        // desired all 0, gaps 1 → -1, 0, 1.
        let cs = [
            Constraint { left: 0, right: 1, gap: 1.0 },
            Constraint { left: 1, right: 2, gap: 1.0 },
        ];
        let out = solve_axis(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &cs);
        approx(&out, &[-1.0, 0.0, 1.0]);
    }

    #[test]
    fn weight_biases_merged_position() {
        // x0 desired 0 (weight 3), x1 desired 0 (weight 1), gap 1.
        // Merged block position = (3*(0-0) + 1*(0-1))/4 = -0.25; x0=-0.25, x1=0.75.
        let out = solve_axis(&[0.0, 0.0], &[3.0, 1.0], &[Constraint { left: 0, right: 1, gap: 1.0 }]);
        approx(&out, &[-0.25, 0.75]);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p mapper vpsc`
Expected: FAIL — `solve_axis` not found.

- [ ] **Step 4: Implement `solve_axis`**

Add to `crates/mapper/src/layout/vpsc.rs`, above the `#[cfg(test)]` module:

```rust
/// Project `desired` onto the feasible region of `constraints` (closest feasible
/// point under the weighted L2 objective). Returns one position per variable.
pub fn solve_axis(desired: &[f64], weight: &[f64], constraints: &[Constraint]) -> Vec<f64> {
    let n = desired.len();
    if n == 0 {
        return Vec::new();
    }
    // Block of each variable, and the variable's fixed offset within its block.
    let mut block: Vec<usize> = (0..n).collect();
    let mut offset: Vec<f64> = vec![0.0; n];
    // Per block: member variable indices, total weight, position.
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut bweight: Vec<f64> = weight.to_vec();
    let mut bpos: Vec<f64> = desired.to_vec();

    loop {
        // Find the most-violated constraint whose endpoints are in different blocks.
        let mut worst: Option<usize> = None;
        let mut worst_v = TOL;
        for (ci, c) in constraints.iter().enumerate() {
            if block[c.left] == block[c.right] {
                continue;
            }
            let pl = bpos[block[c.left]] + offset[c.left];
            let pr = bpos[block[c.right]] + offset[c.right];
            let v = c.gap - (pr - pl);
            if v > worst_v {
                worst_v = v;
                worst = Some(ci);
            }
        }
        let Some(ci) = worst else { break };
        let c = &constraints[ci];
        let bl = block[c.left];
        let br = block[c.right];

        // Merge br into bl, keeping bl's frame and making this constraint active:
        // after the merge, offset[right] - offset[left] == gap exactly.
        let shift = (offset[c.left] + c.gap) - offset[c.right];
        let moved: Vec<usize> = std::mem::take(&mut members[br]);
        for &v in &moved {
            offset[v] += shift;
            block[v] = bl;
        }
        members[bl].extend(moved);
        bweight[bl] += bweight[br];
        bweight[br] = 0.0;

        // Re-derive the merged block's weight-optimal position.
        let mut num = 0.0;
        for &v in &members[bl] {
            num += weight[v] * (desired[v] - offset[v]);
        }
        bpos[bl] = num / bweight[bl];
    }

    (0..n).map(|i| bpos[block[i]] + offset[i]).collect()
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p mapper vpsc`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(mapper): VPSC 1-D separation-constraint projector"
```

---

### Task 3: Compass → axis constraints with deterministic cycle-breaking

**Files:**
- Create: `crates/mapper/src/layout/constraints.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod constraints;`)

**Interfaces:**
- Consumes: `vpsc::Constraint`, `crate::direction::grid_offset`, `crate::graph::{MapGraph, RoomId}`.
- Produces: `pub struct AxisConstraints { pub x: Vec<Constraint>, pub y: Vec<Constraint>, pub dropped: BTreeSet<usize> }` and `pub fn build_axis_constraints(graph: &MapGraph, ids: &[RoomId], gap: f64) -> AxisConstraints`. `dropped` holds GLOBAL indices into `graph.connections()` whose direction was dropped to keep an axis acyclic. Node indices in the returned constraints are local: the position of the room id in `ids`.

- [ ] **Step 1: Add the module declaration to `mod.rs`**

```rust
mod constraints;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/mapper/src/layout/constraints.rs`:

```rust
//! Build axis-separated separation constraints from compass edges, dropping the
//! minimal set that would otherwise make an axis's precedence graph cyclic
//! (a geometric contradiction). Dropped connections feed the `distorted` flag.

use std::collections::BTreeSet;

use crate::direction::grid_offset;
use crate::graph::{MapGraph, RoomId};

use super::vpsc::Constraint;

/// Separation constraints split by axis, plus the global connection indices whose
/// direction had to be dropped to keep each axis acyclic.
pub struct AxisConstraints {
    pub x: Vec<Constraint>,
    pub y: Vec<Constraint>,
    pub dropped: BTreeSet<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_rooms() -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g
    }

    #[test]
    fn east_makes_x_constraint_origin_left() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::E, 2); // B east of A → x[B] >= x[A] + gap
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.x.len(), 1);
        assert_eq!((ac.x[0].left, ac.x[0].right), (0, 1)); // local idx: A=0 left, B=1 right
        assert!(ac.y.is_empty());
        assert!(ac.dropped.is_empty());
    }

    #[test]
    fn north_makes_y_constraint_dest_left() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2); // B north of A → y[B] <= y[A] → B is "left" on y
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.y.len(), 1);
        assert_eq!((ac.y[0].left, ac.y[0].right), (1, 0)); // B(idx1) left, A(idx0) right
        assert!(ac.x.is_empty());
    }

    #[test]
    fn diagonal_constrains_both_axes() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::NE, 2); // B north-east of A
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert_eq!(ac.x.len(), 1, "NE has an east component");
        assert_eq!(ac.y.len(), 1, "NE has a north component");
        assert!(ac.dropped.is_empty());
    }

    #[test]
    fn contradiction_drops_one_constraint() {
        // A→N→B and B→N→A: both want the other north → cycle on the y axis.
        let mut g = two_rooms();
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 1);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        // First N kept, second N dropped (would close a cycle).
        assert_eq!(ac.y.len(), 1, "exactly one y constraint survives");
        assert_eq!(ac.dropped.len(), 1, "the cycle-closing connection is dropped");
    }

    #[test]
    fn non_compass_edges_make_no_constraints() {
        let mut g = two_rooms();
        g.add_edge(1, Direction::Up, 2);
        let ac = build_axis_constraints(&g, &[1, 2], 1.0);
        assert!(ac.x.is_empty() && ac.y.is_empty() && ac.dropped.is_empty());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p mapper constraints`
Expected: FAIL — `build_axis_constraints` not found.

- [ ] **Step 4: Implement the builder**

Add to `crates/mapper/src/layout/constraints.rs`, above the tests:

```rust
/// Is `a` reachable from `b` in the precedence graph `adj`? If so, adding the
/// edge `a → b` (a must be left of b) would close a cycle.
fn creates_cycle(adj: &[Vec<usize>], a: usize, b: usize) -> bool {
    let mut seen = vec![false; adj.len()];
    let mut stack = vec![b];
    seen[b] = true;
    while let Some(u) = stack.pop() {
        if u == a {
            return true;
        }
        for &v in &adj[u] {
            if !seen[v] {
                seen[v] = true;
                stack.push(v);
            }
        }
    }
    false
}

/// Build axis separation constraints for the component whose rooms are `ids`
/// (local index = position in `ids`). Connections in array order; a constraint
/// that would close a cycle on its axis is skipped and its connection index
/// recorded in `dropped`.
pub fn build_axis_constraints(graph: &MapGraph, ids: &[RoomId], gap: f64) -> AxisConstraints {
    let index: std::collections::HashMap<RoomId, usize> =
        ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let n = ids.len();
    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut x_adj = vec![Vec::new(); n];
    let mut y_adj = vec![Vec::new(); n];
    let mut dropped = BTreeSet::new();

    for (ci, conn) in graph.connections().iter().enumerate() {
        let (Some(&o), Some(&d)) = (index.get(&conn.origin), index.get(&conn.dest)) else {
            continue;
        };
        let Some((dx, dy)) = grid_offset(conn.dir) else {
            continue;
        };
        let mut this_dropped = false;

        // X: positive dx = dest east of origin = larger x; precedence left → right.
        if dx != 0 {
            let (left, right) = if dx > 0 { (o, d) } else { (d, o) };
            if creates_cycle(&x_adj, left, right) {
                this_dropped = true;
            } else {
                x_adj[left].push(right);
                x.push(Constraint { left, right, gap });
            }
        }
        // Y: north = smaller y. dy < 0 (north) ⇒ dest has smaller y ⇒ dest is "left".
        if dy != 0 {
            let (left, right) = if dy > 0 { (o, d) } else { (d, o) };
            if creates_cycle(&y_adj, left, right) {
                this_dropped = true;
            } else {
                y_adj[left].push(right);
                y.push(Constraint { left, right, gap });
            }
        }
        if this_dropped {
            dropped.insert(ci);
        }
    }

    AxisConstraints { x, y, dropped }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p mapper constraints`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(mapper): compass→axis separation constraints with cycle-breaking"
```

---

### Task 4: Stress-majorization driver

**Files:**
- Create: `crates/mapper/src/layout/stress.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod stress;`)

**Interfaces:**
- Consumes: `super::vpsc::{self, Constraint}`.
- Produces: `pub fn all_pairs_dist(n: usize, adjacency: &[Vec<usize>]) -> Vec<Vec<f64>>` and `pub fn stress_layout(n: usize, dist: &[Vec<f64>], x_constraints: &[Constraint], y_constraints: &[Constraint], seed: &[(f64,f64)], iters: usize) -> Vec<(f64,f64)>`.

- [ ] **Step 1: Add the module declaration to `mod.rs`**

```rust
mod stress;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/mapper/src/layout/stress.rs`:

```rust
//! Constrained stress majorization: minimise neato's stress over graph-theoretic
//! distances, projecting onto axis separation constraints each iteration (SMACOF
//! Guttman transform + per-axis VPSC).

use super::vpsc::{self, Constraint};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_pairs_dist_path_graph() {
        // 0 - 1 - 2 path.
        let adj = vec![vec![1], vec![0, 2], vec![1]];
        let d = all_pairs_dist(3, &adj);
        assert_eq!(d[0][2], 2.0);
        assert_eq!(d[0][1], 1.0);
        assert_eq!(d[1][2], 1.0);
        assert_eq!(d[0][0], 0.0);
    }

    #[test]
    fn east_constraint_orders_x() {
        // Two nodes, ideal distance 1, with x[1] - x[0] >= 1. Seed reversed on x.
        let dist = vec![vec![0.0, 1.0], vec![1.0, 0.0]];
        let xc = vec![Constraint { left: 0, right: 1, gap: 1.0 }];
        let yc = vec![];
        let seed = vec![(5.0, 0.0), (0.0, 0.0)]; // node 0 east of node 1 initially
        let out = stress_layout(2, &dist, &xc, &yc, &seed, 60);
        assert!(out[1].0 - out[0].0 >= 1.0 - 1e-6, "constraint x1 >= x0 + 1 must hold: {out:?}");
    }

    #[test]
    fn deterministic() {
        let dist = vec![vec![0.0, 1.0, 2.0], vec![1.0, 0.0, 1.0], vec![2.0, 1.0, 0.0]];
        let seed = vec![(0.0, 0.0), (1.0, 0.3), (2.0, -0.2)];
        let a = stress_layout(3, &dist, &[], &[], &seed, 60);
        let b = stress_layout(3, &dist, &[], &[], &seed, 60);
        assert_eq!(a, b, "same inputs must give identical output");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p mapper stress`
Expected: FAIL — `all_pairs_dist` / `stress_layout` not found.

- [ ] **Step 4: Implement the driver**

Add to `crates/mapper/src/layout/stress.rs`, above the tests:

```rust
/// All-pairs shortest-path hop counts over an undirected adjacency list (local
/// indices). Unreachable pairs are `f64::INFINITY` (callers pass one connected
/// component, so all finite in practice).
pub fn all_pairs_dist(n: usize, adjacency: &[Vec<usize>]) -> Vec<Vec<f64>> {
    let mut dist = vec![vec![f64::INFINITY; n]; n];
    for s in 0..n {
        let mut depth = vec![usize::MAX; n];
        let mut q = std::collections::VecDeque::new();
        depth[s] = 0;
        q.push_back(s);
        while let Some(u) = q.pop_front() {
            for &v in &adjacency[u] {
                if depth[v] == usize::MAX {
                    depth[v] = depth[u] + 1;
                    q.push_back(v);
                }
            }
        }
        for t in 0..n {
            if depth[t] != usize::MAX {
                dist[s][t] = depth[t] as f64;
            }
        }
    }
    dist
}

/// One axis of the SMACOF Guttman transform: returns `(desired, weight)` where
/// `weight_i = Σ_j w_ij` and `desired_i` is the stress-minimising target for axis
/// `a` (0 = x, 1 = y) given current positions `p`. `w_ij = 1/d_ij²`.
fn guttman_axis(p: &[(f64, f64)], dist: &[Vec<f64>], axis: usize) -> (Vec<f64>, Vec<f64>) {
    let n = p.len();
    let comp = |q: &(f64, f64)| if axis == 0 { q.0 } else { q.1 };
    let mut desired = vec![0.0; n];
    let mut weight = vec![0.0; n];
    for i in 0..n {
        let mut num = 0.0;
        let mut den = 0.0;
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = dist[i][j];
            if !d.is_finite() || d == 0.0 {
                continue;
            }
            let w = 1.0 / (d * d);
            let dx = p[i].0 - p[j].0;
            let dy = p[i].1 - p[j].1;
            let norm = (dx * dx + dy * dy).sqrt();
            let target = if norm > 1e-9 {
                comp(&p[j]) + d * (comp(&p[i]) - comp(&p[j])) / norm
            } else {
                comp(&p[j])
            };
            num += w * target;
            den += w;
        }
        if den > 0.0 {
            desired[i] = num / den;
            weight[i] = den;
        } else {
            desired[i] = comp(&p[i]);
            weight[i] = 1.0;
        }
    }
    (desired, weight)
}

/// Constrained stress majorization. Seeds from `seed`, runs `iters` SMACOF
/// iterations, projecting each axis onto its separation constraints via VPSC.
/// Returns final continuous positions.
pub fn stress_layout(
    n: usize,
    dist: &[Vec<f64>],
    x_constraints: &[Constraint],
    y_constraints: &[Constraint],
    seed: &[(f64, f64)],
    iters: usize,
) -> Vec<(f64, f64)> {
    let mut p = seed.to_vec();
    if n <= 1 {
        return p;
    }
    for _ in 0..iters {
        let (dx, wx) = guttman_axis(&p, dist, 0);
        let nx = vpsc::solve_axis(&dx, &wx, x_constraints);
        for i in 0..n {
            p[i].0 = nx[i];
        }
        let (dy, wy) = guttman_axis(&p, dist, 1);
        let ny = vpsc::solve_axis(&dy, &wy, y_constraints);
        for i in 0..n {
            p[i].1 = ny[i];
        }
    }
    p
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p mapper stress`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(mapper): constrained stress-majorization driver"
```

---

### Task 5: Extract `seed_layout`, `connected_components`, `mark_distorted` (behavior-preserving)

**Files:**
- Modify: `crates/mapper/src/layout/mod.rs`

**Interfaces:**
- Produces (module-private): `fn connected_components(graph: &MapGraph, ids: &[RoomId]) -> Vec<Vec<RoomId>>` (each component sorted ascending; components in ascending-root order); `fn seed_layout(graph: &MapGraph) -> BTreeMap<RoomId,(i32,i32)>` (the existing greedy grid placement as a pure function — no graph mutation, no distorted sweep); `fn mark_distorted(graph: &mut MapGraph, dropped: &BTreeSet<usize>)`.
- This task is a pure refactor: `relayout_auto` is rewritten to `seed_layout` + write + `mark_distorted(.., &empty)`, producing identical positions and `distorted` flags. All existing `layout` tests stay green.

- [ ] **Step 1: Add helpers and rewrite `relayout_auto` as a thin wrapper**

In `crates/mapper/src/layout/mod.rs`, ensure these imports exist at the top:

```rust
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::grid_offset;
use crate::graph::{Connection, MapGraph, RoomId};
```

Replace the entire body of `pub fn relayout_auto(graph: &mut MapGraph)` with:

```rust
pub fn relayout_auto(graph: &mut MapGraph) {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return;
    }
    clear_all_positions(graph, &ids);

    let seed = seed_layout(graph);
    for &id in &ids {
        if let Some(&p) = seed.get(&id) {
            graph.set_pos(id, p);
        }
    }
    mark_distorted(graph, &BTreeSet::new());
}
```

Add these functions to `crates/mapper/src/layout/mod.rs` (the bodies of `connected_components` and the greedy placement are the logic currently inside the old `relayout_auto` — Steps 2–4 below give the complete code):

```rust
/// Connected components over the undirected projection of the graph. Each
/// component is sorted ascending; components are returned in ascending-root order.
fn connected_components(graph: &MapGraph, ids: &[RoomId]) -> Vec<Vec<RoomId>> {
    let mut adjacency: BTreeMap<RoomId, Vec<RoomId>> = BTreeMap::new();
    for &id in ids {
        adjacency.entry(id).or_default();
    }
    for conn in graph.connections() {
        adjacency.entry(conn.origin).or_default().push(conn.dest);
        adjacency.entry(conn.dest).or_default().push(conn.origin);
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut visited: BTreeSet<RoomId> = BTreeSet::new();
    let mut components: Vec<Vec<RoomId>> = Vec::new();
    for &id in ids {
        if visited.contains(&id) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue: VecDeque<RoomId> = VecDeque::new();
        queue.push_back(id);
        visited.insert(id);
        while let Some(cur) = queue.pop_front() {
            component.push(cur);
            if let Some(neighbors) = adjacency.get(&cur) {
                for &nb in neighbors {
                    if visited.insert(nb) {
                        queue.push_back(nb);
                    }
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

/// The greedy grid placement (former `relayout_auto` body) as a pure function:
/// BFS from each component's lowest-id root, placing neighbours at `pair_offset`
/// deltas with spiral collision avoidance. No graph mutation; used only to seed
/// the stress solver and as the large-graph fallback.
fn seed_layout(graph: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    let mut pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    if ids.is_empty() {
        return pos;
    }
    let components = connected_components(graph, &ids);
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();

    for component in &components {
        let root = *component.iter().min().unwrap();
        let anchor = nearest_free_cell(&occupied, (0, 0));
        pos.insert(root, anchor);
        occupied.insert(anchor);

        let mut bfs: VecDeque<RoomId> = VecDeque::new();
        let mut bfs_visited: BTreeSet<RoomId> = BTreeSet::new();
        bfs.push_back(root);
        bfs_visited.insert(root);

        while let Some(placed_id) = bfs.pop_front() {
            let placed_pos = *pos.get(&placed_id).unwrap();
            let incident: Vec<Connection> = graph
                .connections()
                .iter()
                .filter(|c| c.origin == placed_id || c.dest == placed_id)
                .cloned()
                .collect();
            let compass_first: Vec<&Connection> = incident
                .iter()
                .filter(|c| grid_offset(c.dir).is_some())
                .chain(incident.iter().filter(|c| grid_offset(c.dir).is_none()))
                .collect();

            for conn in compass_first {
                let neighbor_id = if conn.origin == placed_id { conn.dest } else { conn.origin };
                if bfs_visited.contains(&neighbor_id) || !component.contains(&neighbor_id) {
                    continue;
                }
                bfs_visited.insert(neighbor_id);
                let desired = if conn.origin == placed_id {
                    match pair_offset(graph, placed_id, neighbor_id) {
                        Some(delta) => (placed_pos.0 + delta.0, placed_pos.1 + delta.1),
                        None => placed_pos,
                    }
                } else {
                    match pair_offset(graph, neighbor_id, placed_id) {
                        Some(delta) => (placed_pos.0 - delta.0, placed_pos.1 - delta.1),
                        None => placed_pos,
                    }
                };
                let cell = nearest_free_cell(&occupied, desired);
                pos.insert(neighbor_id, cell);
                occupied.insert(cell);
                bfs.push_back(neighbor_id);
            }
        }
    }
    pos
}

/// Set the `distorted` flag on every connection: a compass edge is distorted if
/// its connection index is in `dropped`, or its final grid geometry violates its
/// direction. Non-compass edges are never distorted.
fn mark_distorted(graph: &mut MapGraph, dropped: &BTreeSet<usize>) {
    let n_conns = graph.connections().len();
    for idx in 0..n_conns {
        let conn = graph.connections()[idx].clone();
        let distorted = match grid_offset(conn.dir) {
            None => false,
            Some(_) => dropped.contains(&idx) || !edge_is_satisfied(graph, &conn),
        };
        graph.set_conn_distorted(idx, distorted);
    }
}
```

> Note: delete the now-duplicated component-finding / placement / distortion-sweep code that remained inline in the old `relayout_auto`, and remove the old `place_room` helper if it is no longer referenced (it was only used by the inline placement). Keep `clear_all_positions`, `pair_offset`, `nearest_free_cell`, `occupied_cells`, `edge_is_satisfied`, `LayoutMode` unchanged.

- [ ] **Step 2: Run the full mapper test suite**

Run: `cargo test -p mapper`
Expected: PASS — identical results to Task 1 (this is a behavior-preserving refactor; positions and `distorted` flags are unchanged).

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p mapper`
Expected: no warnings (remove any now-unused imports/functions the refactor orphaned).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor(mapper): extract seed_layout/connected_components/mark_distorted"
```

---

### Task 6: Wire constrained stress majorization into `relayout_auto`

**Files:**
- Modify: `crates/mapper/src/layout/mod.rs`

**Interfaces:**
- Consumes: `constraints::build_axis_constraints`, `stress::{all_pairs_dist, stress_layout}`, `seed_layout`, `connected_components`, `mark_distorted`.
- Produces: the new `relayout_auto` (same signature) that lays out via constrained stress majorization, snaps to grid, removes overlaps, packs components, anchors the lowest-id room at `(0,0)`, and marks `distorted`.

- [ ] **Step 1: Add constants and the new `relayout_auto`**

In `crates/mapper/src/layout/mod.rs`, add near the top (after imports):

```rust
/// Separation gap and ideal edge length (in grid cells).
const GAP: f64 = 1.0;
/// Fixed SMACOF iterations (determinism + bounded cost).
const ITERS: usize = 60;
/// Above this node count, skip the O(ITERS·n²) solve and use the seed grid.
const MAX_NODES: usize = 400;
```

Replace `relayout_auto` (the thin wrapper from Task 5) with:

```rust
pub fn relayout_auto(graph: &mut MapGraph) {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return;
    }
    clear_all_positions(graph, &ids);

    let seed = seed_layout(graph);

    // Large-graph fallback: keep the deterministic seed grid.
    if ids.len() > MAX_NODES {
        for &id in &ids {
            if let Some(&p) = seed.get(&id) {
                graph.set_pos(id, p);
            }
        }
        mark_distorted(graph, &BTreeSet::new());
        return;
    }

    let components = connected_components(graph, &ids);
    let mut dropped_all: BTreeSet<usize> = BTreeSet::new();
    let mut final_pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0; // left edge for the next component

    for comp in &components {
        let n = comp.len();
        let index: std::collections::HashMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Local undirected adjacency for BFS distances.
        let mut adj = vec![Vec::new(); n];
        for c in graph.connections() {
            if let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        let dist = stress::all_pairs_dist(n, &adj);
        let ac = constraints::build_axis_constraints(graph, comp, GAP);
        dropped_all.extend(ac.dropped.iter().copied());

        let seed_local: Vec<(f64, f64)> = comp
            .iter()
            .map(|&id| {
                let p = seed.get(&id).copied().unwrap_or((0, 0));
                (p.0 as f64, p.1 as f64)
            })
            .collect();

        let cont = stress::stress_layout(n, &dist, &ac.x, &ac.y, &seed_local, ITERS);
        let mut snapped: Vec<(i32, i32)> =
            cont.iter().map(|&(x, y)| (x.round() as i32, y.round() as i32)).collect();

        // Pack this component to the right of the previous, top-aligned.
        let min_x = snapped.iter().map(|p| p.0).min().unwrap();
        let min_y = snapped.iter().map(|p| p.1).min().unwrap();
        for p in &mut snapped {
            p.0 += pack_x - min_x;
            p.1 -= min_y;
        }

        // Resolve residual same-cell collisions in ascending room-id order.
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let cell = nearest_free_cell(&occupied, snapped[i]);
            occupied.insert(cell);
            final_pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2; // 1-cell gap between components
    }

    // Anchor the lowest-id room at (0,0) for a stable reference.
    if let Some(&(ax, ay)) = final_pos.get(&ids[0]) {
        for p in final_pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }

    for (&id, &p) in &final_pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &dropped_all);
}
```

- [ ] **Step 2: Adapt the existing position-exact tests to relational assertions**

In the `#[cfg(test)] mod tests` of `crates/mapper/src/layout/mod.rs`, replace the bodies that assert exact deltas. The new optimum is not the unit grid, so assert direction/relation + invariants instead. Apply these replacements:

`places_rooms_by_compass_offsets` → assert B is north of Center:

```rust
    #[test]
    fn places_rooms_by_compass_offsets() {
        let mut m = Mapper::default();
        m.observe(1, "Center", None);
        m.observe(2, "North Room", Some(Direction::N));
        relayout_auto(&mut m.graph);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "north room must be above center: {p2:?} vs {p1:?}");
    }
```

`dynamic_layout_re_derives_from_scratch` → assert root anchored at (0,0) and room 2 north:

```rust
    #[test]
    fn dynamic_layout_re_derives_from_scratch() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.set_pos(1, (5, 5));
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        assert_eq!(graph.room(1).unwrap().pos, Some((0, 0)), "lowest-id room anchors at origin");
        let p2 = graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < 0, "room 2 must be north of the anchor: {p2:?}");
    }
```

`dynamic_relayout_updates_positions` → assert room 2 ends up north and moved:

```rust
    #[test]
    fn dynamic_relayout_updates_positions() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.add_edge(1, Direction::Unknown, 2);
        relayout_auto(&mut graph);
        let pos2_before = graph.room(2).unwrap().pos.unwrap();
        graph.remove_connection(1, Direction::Unknown);
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        let pos2_after = graph.room(2).unwrap().pos.unwrap();
        assert!(pos2_after.1 < graph.room(1).unwrap().pos.unwrap().1, "room 2 must be north now");
        assert_ne!(pos2_before, pos2_after, "room 2 must reposition when the constraint changes");
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }
```

`combined_offset_places_northeast` → assert north AND east (not exact (1,-1)):

```rust
    #[test]
    fn combined_offset_places_northeast() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0 && pb.1 < pa.1, "B must be north-east of A: {pb:?} vs {pa:?}");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }
```

`reciprocal_places_one_step_north` → assert due north (north, same column):

```rust
    #[test]
    fn reciprocal_places_one_step_north() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no east/west constraint → B stays in A's column");
    }
```

Leave these tests unchanged (they assert invariants that still hold): `layout_mode_default_is_auto`, `collision_places_nearest_free_and_marks_distorted`, `collision_direct_distorted_flag`, `rooms_never_overlap_random_walk`, `relayout_is_deterministic`, `disconnected_component_gets_placed`, `contradictory_geometry_marks_distorted_not_overlap`, `nearest_free_cell_returns_from_if_free`, `nearest_free_cell_spirals_outward`, `manual_mode_freezes_and_allows_nudge`.

- [ ] **Step 3: Add new end-to-end tests for the stress layout**

Append to the same test module:

```rust
    #[test]
    fn east_room_is_east() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        relayout_auto(&mut m.graph);
        let pa = m.graph.room(1).unwrap().pos.unwrap();
        let pb = m.graph.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0, "east room must be to the right: {pb:?} vs {pa:?}");
    }

    #[test]
    fn orientation_pinned_north_is_up() {
        // North must map to smaller y every solve (no rotation), regardless of ids.
        let mut g = crate::graph::MapGraph::new();
        for id in 1..=4 {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::S, 4);
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "room 2 (north of 1) must be above it: {p2:?} vs {p1:?}");
    }
```

- [ ] **Step 4: Run the full mapper suite**

Run: `cargo test -p mapper`
Expected: PASS — all adapted + new + unchanged tests green.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -p mapper`
Expected: no warnings.

- [ ] **Step 6: Verify determinism explicitly**

Run: `cargo test -p mapper relayout_is_deterministic`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(mapper): constrained stress-majorization layout (neato + compass hints)"
```

---

## Verification (whole-feature)

- `cargo test -p mapper` green (VPSC, constraints, stress, layout suites).
- `cargo test --workspace` green (app/zvm unaffected — positions are still grid cells).
- `cargo clippy --workspace` clean.
- Manual smoke: run a game, explore a dense area, confirm rooms separate with compass directions honored and north up. (Visual; reviewer/user confirms.)
