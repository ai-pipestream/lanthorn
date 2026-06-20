# Crossing-Aware Layout Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the layout's routability repair also minimize path crossings by shifting rooms whose edges fan the same way, so the `#74/#25/#76` corner stops crossing.

**Architecture:** Add a per-room same-octant neighbour conflict count (pure sign geometry, no path routing) as a second objective in `repair_routability`, and widen candidate moves from ±1 to a Chebyshev radius so the hill-climb can cross the routability valley between `(0,0)` and `(0,2)`. Score becomes lexicographic `(unroutable_count, side_conflicts, displacement)`. All changes are in `crates/mapper/src/layout/routability.rs`, plus an end-to-end render assertion in `app`.

**Tech Stack:** Rust workspace. `mapper` (layout) + `app` (ratatui renderer). Tests via `cargo test -p <crate>`.

## Global Constraints

- All layout changes live in `crates/mapper/src/layout/routability.rs`. `relayout_auto`, the renderer, persistence, DOT export, dump format, and the zvm bridge are untouched.
- Rooms stay on integer grid cells; no new external dependencies.
- Layout is DETERMINISTIC: same graph → identical positions. No RNG, no time. Moves accepted only on strict lexicographic improvement, fixed iteration order. The existing `relayout_is_deterministic` test must keep passing.
- The crossing model is geometric only (octants) — NO path/route modelling in the layout.
- No room overlap may ever be introduced — every move targets a currently-free cell.
- Milestone-5 behaviour preserved: `edge_routable`, `BBOX_MARGIN`, `MAX_REPAIR_PASSES`, `occupied_map`, `bbox_of`, `unroutable_count`, `displacement` are unchanged.

**Reference — current `repair_routability` (the function Task 2 rewrites)** lives at `crates/mapper/src/layout/routability.rs` and today scores `(unroutable_count, displacement)`, generates ±1 candidate moves for endpoints of unroutable edges only, and breaks when `unroutable_count == 0`.

**Reference — `MapGraph`/`Direction`:** `graph.connections()` returns `&[Connection]` (a `Vec`, insertion order) where `Connection { origin: RoomId, dir: Direction, dest: RoomId, distorted: bool }`. `grid_offset(dir).is_some()` is true exactly for the 8 compass directions (the drawn edges). `RoomId = u16`.

---

### Task 1: Octant conflict helpers

**Files:**
- Modify: `crates/mapper/src/layout/routability.rs` (add helpers + unit tests; near the other private helpers, e.g. after `displacement`)

**Interfaces:**
- Consumes: `crate::direction::grid_offset`, `crate::graph::{MapGraph, RoomId}`, `std::collections::{BTreeMap, BTreeSet}` (all already imported in this file).
- Produces (used by Task 2):
  - `fn octant(dx: i32, dy: i32) -> (i8, i8)`
  - `fn neighbours(graph: &MapGraph, r: RoomId) -> BTreeSet<RoomId>`
  - `fn side_conflicts(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> usize`
  - `fn conflict_rooms(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> BTreeSet<RoomId>`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/mapper/src/layout/routability.rs`:

```rust
    #[test]
    fn octant_is_sign_pair() {
        assert_eq!(octant(-1, 2), (-1, 1));
        assert_eq!(octant(-1, 0), (-1, 0));
        assert_eq!(octant(3, -4), (1, -1));
        assert_eq!(octant(0, 0), (0, 0));
    }

    #[test]
    fn side_conflicts_counts_same_octant_neighbour_pairs() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // #25 connects to #74 and #76 (drawn compass edges). At (0,0) both neighbours
        // are SW (same octant) → 1 conflict. Move #25 to (0,2): #74 is NW, #76 is W
        // (different octants) → 0 conflicts.
        let mut g = MapGraph::new();
        for id in [25u16, 74, 76] { g.upsert_room(id, "r".into()); }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);

        let crammed: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 0)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &crammed), 1, "both #25 neighbours are SW");
        assert!(conflict_rooms(&g, &crammed).contains(&25), "the conflicted room is flagged");

        let spread: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 2)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &spread), 0, "#74 is NW, #76 is W → no shared octant");
        assert!(conflict_rooms(&g, &spread).is_empty());
    }

    #[test]
    fn reciprocal_pair_is_one_neighbour_no_self_conflict() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // A reciprocal pair a<->b must contribute ONE neighbour each way, never a
        // self-conflict (a room with a single neighbour has no pair).
        let mut g = MapGraph::new();
        for id in [1u16, 2] { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1); // reciprocal
        let pos: BTreeMap<RoomId, (i32, i32)> = [(1u16, (0, 1)), (2, (0, 0))].into_iter().collect();
        assert_eq!(neighbours(&g, 1), [2u16].into_iter().collect());
        assert_eq!(side_conflicts(&g, &pos), 0, "one neighbour each → no pair → no conflict");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p mapper octant_is_sign_pair side_conflicts_counts reciprocal_pair_is_one`
Expected: FAIL to compile — `octant`, `side_conflicts`, `conflict_rooms`, `neighbours` are not yet defined.

- [ ] **Step 3: Implement the helpers**

Add to `crates/mapper/src/layout/routability.rs` (after `displacement`, before `repair_routability`):

```rust
/// Compass octant of a direction vector: (sign(dx), sign(dy)), each in {-1,0,1}.
/// Two neighbours of a room sharing an octant fan the same way → crossing pressure.
fn octant(dx: i32, dy: i32) -> (i8, i8) {
    (dx.signum() as i8, dy.signum() as i8)
}

/// Distinct neighbour rooms of `r` via DRAWN (compass) edges, either direction.
/// Reciprocal pairs collapse naturally — a neighbour is listed once.
fn neighbours(graph: &MapGraph, r: RoomId) -> BTreeSet<RoomId> {
    let mut ns = BTreeSet::new();
    for c in graph.connections() {
        if grid_offset(c.dir).is_none() {
            continue;
        }
        if c.origin == r {
            ns.insert(c.dest);
        } else if c.dest == r {
            ns.insert(c.origin);
        }
    }
    ns
}

/// Total per-room same-octant neighbour conflicts: for each placed room, each
/// unordered pair of its placed neighbours that share an octant relative to it.
fn side_conflicts(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> usize {
    let mut total = 0;
    for (&r, &rp) in pos {
        let ns: Vec<RoomId> = neighbours(graph, r).into_iter().filter(|n| pos.contains_key(n)).collect();
        for i in 0..ns.len() {
            for j in (i + 1)..ns.len() {
                let a = pos[&ns[i]];
                let b = pos[&ns[j]];
                if octant(a.0 - rp.0, a.1 - rp.1) == octant(b.0 - rp.0, b.1 - rp.1) {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Rooms involved in any same-octant conflict: the room itself plus the two
/// neighbours of each conflicting pair (the set the repair is allowed to move).
fn conflict_rooms(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> BTreeSet<RoomId> {
    let mut rooms = BTreeSet::new();
    for (&r, &rp) in pos {
        let ns: Vec<RoomId> = neighbours(graph, r).into_iter().filter(|n| pos.contains_key(n)).collect();
        for i in 0..ns.len() {
            for j in (i + 1)..ns.len() {
                let a = pos[&ns[i]];
                let b = pos[&ns[j]];
                if octant(a.0 - rp.0, a.1 - rp.1) == octant(b.0 - rp.0, b.1 - rp.1) {
                    rooms.insert(r);
                    rooms.insert(ns[i]);
                    rooms.insert(ns[j]);
                }
            }
        }
    }
    rooms
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mapper octant_is_sign_pair side_conflicts_counts reciprocal_pair_is_one`
Expected: PASS (3 tests).

- [ ] **Step 5: Run the full mapper suite + clippy**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass. Note: `conflict_rooms` may be reported as dead code by clippy at THIS step because only Task 2 consumes it — if so, that warning is expected and clears in Task 2. If clippy fails the build on it, add `#[allow(dead_code)]` on `conflict_rooms` with a `// consumed by repair_routability in the next task` comment, to be removed in Task 2.

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/layout/routability.rs
git commit -m "feat(mapper): per-room same-octant neighbour conflict helpers"
```

---

### Task 2: Crossing-aware repair (score + radius moves)

**Files:**
- Modify: `crates/mapper/src/layout/routability.rs` (rewrite `repair_routability`; add a const; add tests)

**Interfaces:**
- Consumes: `octant`, `neighbours`, `side_conflicts`, `conflict_rooms` (Task 1); `edge_routable`, `occupied_map`, `bbox_of`, `unroutable_count`, `displacement`, `MAX_REPAIR_PASSES` (existing).
- Produces: same public signature `pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId, (i32, i32)>)` — behaviour extended.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/mapper/src/layout/routability.rs`:

```rust
    #[test]
    fn repair_removes_same_octant_crossing_pressure() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // The A129 corner after Milestone-5 routability repair: 0 unroutable, but #25's
        // two neighbours (#74, #76) both sit SW → 1 conflict. The crossing-aware repair
        // must drop conflicts to 0 by moving #25 down off row 0 (verified empirically:
        // (0,2)/(0,3)/(1,2) all render 0 crossings), without re-introducing an
        // unroutable edge or a room overlap.
        let mut g = MapGraph::new();
        for id in [25u16, 74, 76] { g.upsert_room(id, "r".into()); }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);

        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 0)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &pos), 1, "precondition: the corner has a same-octant conflict");

        repair_routability(&g, &mut pos);

        assert_eq!(side_conflicts(&g, &pos), 0, "repair must remove the conflict; got {pos:?}");
        // Still routable, still no overlap.
        let occ: BTreeMap<(i32, i32), RoomId> = pos.iter().map(|(&id, &c)| (c, id)).collect();
        let xs: Vec<i32> = pos.values().map(|p| p.0).collect();
        let ys: Vec<i32> = pos.values().map(|p| p.1).collect();
        let bb = (
            xs.iter().min().unwrap() - BBOX_MARGIN, ys.iter().min().unwrap() - BBOX_MARGIN,
            xs.iter().max().unwrap() + BBOX_MARGIN, ys.iter().max().unwrap() + BBOX_MARGIN,
        );
        for c in g.connections() {
            assert!(edge_routable(pos[&c.origin], pos[&c.dest], c.dir, &occ, bb),
                "edge {}->{} must stay routable; {pos:?}", c.origin, c.dest);
        }
        let cells: BTreeSet<_> = pos.values().collect();
        assert_eq!(cells.len(), pos.len(), "no overlap");
        assert!(pos[&25].1 >= 2, "#25 must move down off row 0; got {:?}", pos[&25]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper repair_removes_same_octant_crossing_pressure`
Expected: FAIL — the current repair stops at 0 unroutable and never reduces `side_conflicts`, so the post-condition `side_conflicts == 0` (and `#25` moving down) fails.

- [ ] **Step 3: Rewrite `repair_routability`**

Replace the entire current `repair_routability` function in `crates/mapper/src/layout/routability.rs` with:

```rust
/// Greedily shift rooms (into free cells, within a small radius) until neither the
/// number of un-routable drawn edges nor the per-room same-octant conflict count can
/// be reduced. Score is lexicographic `(unroutable, side_conflicts, displacement)`;
/// only strictly-improving moves are accepted, so the search is deterministic and
/// terminates. Multi-cell moves let the climb cross a routability valley (e.g. #25
/// (0,0)→(0,2) past the blocked (0,1)).
pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let drawn: Vec<(RoomId, RoomId, Direction)> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some())
        .map(|c| (c.origin, c.dest, c.dir))
        .collect();
    if drawn.is_empty() {
        return;
    }
    let stress = pos.clone();

    let score = |p: &BTreeMap<RoomId, (i32, i32)>| -> (usize, usize, i64) {
        (unroutable_count(p, &drawn), side_conflicts(graph, p), displacement(p, &stress))
    };

    for _ in 0..MAX_REPAIR_PASSES {
        let base = score(pos);

        // Candidate rooms: endpoints of un-routable edges ∪ rooms in a same-octant conflict.
        let occ_now = occupied_map(pos);
        let bb = bbox_of(pos);
        let mut cands: BTreeSet<RoomId> = BTreeSet::new();
        for &(o, d, dir) in &drawn {
            if !edge_routable(pos[&o], pos[&d], dir, &occ_now, bb) {
                cands.insert(o);
                cands.insert(d);
            }
        }
        cands.extend(conflict_rooms(graph, pos));
        if cands.is_empty() {
            break; // 0 unroutable AND 0 conflicts
        }

        // (room, target cell, score)
        let mut best: Option<(RoomId, (i32, i32), (usize, usize, i64))> = None;
        for &room in &cands {
            let from = pos[&room];
            for dy in -MOVE_RADIUS..=MOVE_RADIUS {
                for dx in -MOVE_RADIUS..=MOVE_RADIUS {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let to = (from.0 + dx, from.1 + dy);
                    if pos.values().any(|&p| p == to) {
                        continue; // occupied → would overlap
                    }
                    let mut trial = pos.clone();
                    trial.insert(room, to);
                    let s = score(&trial);
                    if s < base && best.as_ref().is_none_or(|&(_, _, bs)| s < bs) {
                        best = Some((room, to, s));
                    }
                }
            }
        }

        match best {
            Some((room, to, _)) => {
                pos.insert(room, to);
            }
            None => break, // no strictly-improving move
        }
    }
}
```

And add the move-radius constant next to `MAX_REPAIR_PASSES`:

```rust
/// Candidate moves reach any free cell within this Chebyshev radius, so the climb can
/// cross a routability valley (a one-cell step that worsens the primary term) to reach
/// a strictly-better cell beyond it.
const MOVE_RADIUS: i32 = 3;
```

- [ ] **Step 4: Run the new test to verify it passes**

Run: `cargo test -p mapper repair_removes_same_octant_crossing_pressure`
Expected: PASS — `side_conflicts` drops to 0, `#25` moves to row ≥ 2, edges stay routable, no overlap.

- [ ] **Step 5: Run the FULL mapper suite + clippy (regression gate)**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass, no warnings (`conflict_rooms` is now consumed — remove any `#[allow(dead_code)]` added in Task 1).

IMPORTANT — if `repair_opens_channel_for_a129_corner` (a Milestone-5 test in `crates/mapper/src/layout/mod.rs`) now FAILS on its `assert!(!e.distorted)` for `25→W→76`: do NOT silently weaken it. That edge is geometrically contradictory (the full A129 map marks it distorted), so a changed layout legitimately making it distorted is correct, not a regression — but it is a plan/test conflict the CONTROLLER must adjudicate. STOP and report it (status DONE_WITH_CONCERNS) with the failing assertion and the new `#25/#74/#76` positions, rather than editing that test yourself.

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/layout/routability.rs
git commit -m "feat(mapper): crossing-aware repair minimizes same-octant conflicts"
```

---

### Task 3: End-to-end render verification (no crossing on A129)

**Files:**
- Modify: `crates/app/src/render/map.rs` (add one integration test to the `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes: `render_map` (this module), `mapper::layout::relayout_auto`, `mapper::render::render`, `mapper::graph::MapGraph`, `crate::state::{AppState, Zoom}`, `ratatui::{buffer::Buffer, layout::Rect, style::Color}`.
- Produces: nothing (a verification test).

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn a129_full_map_renders_without_crossing_or_unrouted() {
        // The real ZCODE-88-840726-A129 graph: after relayout_auto (with crossing-aware
        // repair) the rendered map must have NO unrouted (DarkGray) ribbon and NO
        // perpendicular-crossing ribbon cell — the corner the user kept reporting.
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;

        let mut g = MapGraph::new();
        for (id, name) in [(25, "Canyon View"), (74, "Clearing"), (76, "Forest"),
                           (79, "Behind House"), (80, "South of House"), (180, "West of House")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(180, mapper::direction::Direction::S, 80);
        g.add_edge(80, mapper::direction::Direction::E, 79);
        g.add_edge(79, mapper::direction::Direction::S, 80);
        g.add_edge(80, mapper::direction::Direction::S, 76);
        g.add_edge(76, mapper::direction::Direction::N, 74);
        g.add_edge(74, mapper::direction::Direction::S, 76);
        g.add_edge(74, mapper::direction::Direction::E, 25);
        g.add_edge(25, mapper::direction::Direction::W, 76);
        g.set_current(76);
        relayout_auto(&mut g);

        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 300, 200);
        let mut buf = Buffer::empty(area);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = (-7, -7);
        render_map(&rm, &st, area, &mut buf);

        let is_ribbon = |b: &Buffer, x: i32, y: i32| {
            if x < 0 || y < 0 || x >= 300 || y >= 200 { return false; }
            matches!(
                b.cell((x as u16, y as u16)).map(|c| c.bg),
                Some(Color::Cyan) | Some(Color::Magenta) | Some(Color::DarkGray)
            )
        };
        let (mut unrouted, mut crossings) = (0, 0);
        for y in 0..200i32 {
            for x in 0..300i32 {
                if matches!(buf.cell((x as u16, y as u16)).map(|c| c.bg), Some(Color::DarkGray)) {
                    unrouted += 1;
                }
                if is_ribbon(&buf, x, y)
                    && is_ribbon(&buf, x - 1, y) && is_ribbon(&buf, x + 1, y)
                    && is_ribbon(&buf, x, y - 1) && is_ribbon(&buf, x, y + 1)
                {
                    crossings += 1;
                }
            }
        }
        assert_eq!(unrouted, 0, "no edge may render unrouted (DarkGray)");
        assert_eq!(crossings, 0, "no perpendicular crossing may remain in the corner");
    }
```

- [ ] **Step 2: Run to verify it passes (Tasks 1–2 already landed the fix)**

Run: `cargo test -p app a129_full_map_renders_without_crossing_or_unrouted`
Expected: PASS — with the crossing-aware repair in place, `relayout_auto` moves `#25` down and the rendered corner has 0 crossings and 0 unrouted cells.

Note: this test depends on Tasks 1–2 being committed first. If it FAILS (non-zero crossings), that means the repair did not eliminate the rendered crossing for the full graph — STOP and report (status DONE_WITH_CONCERNS) with the crossing count and the final room positions (`g.rooms()` `pos`), rather than weakening the assertion. The controller will investigate (the octant proxy may need tuning for the full graph).

- [ ] **Step 3: Run the full app suite + workspace clippy**

Run: `cargo test -p app && cargo clippy --workspace`
Expected: all pass, no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "test(app): A129 full map renders with no crossing or unrouted edge"
```

---

## Self-Review

**Spec coverage:**
- Octant + per-room conflict count (spec Component 1) → Task 1 (`octant`, `neighbours`, `side_conflicts`, `conflict_rooms`). ✓
- Repair score `(unroutable, side_conflicts, displacement)` + conflict candidate rooms + radius moves (spec Component 2) → Task 2. ✓
- Multi-cell moves to cross the routability valley (spec decision 3) → Task 2 `MOVE_RADIUS`. ✓
- Determinism preserved → Task 2 Step 5 runs `relayout_is_deterministic`. ✓
- Reciprocal collapse / no self-conflict (spec Edge Cases) → Task 1 `reciprocal_pair_is_one_neighbour_no_self_conflict`. ✓
- End-to-end render acceptance gate (0 unrouted AND 0 crossings) (spec Testing) → Task 3. ✓
- Milestone-5 interaction risk (the `repair_opens_channel_for_a129_corner` distorted assertion) → flagged explicitly in Task 2 Step 5 for controller adjudication. ✓

**Placeholder scan:** No TBD/TODO; every code step has complete code. The two "if it fails, stop and report" notes are deliberate escalation paths, not placeholders. ✓

**Type consistency:** `side_conflicts`/`conflict_rooms`/`neighbours`/`octant` signatures match between Task 1 (definition) and Task 2 (use). `repair_routability(&MapGraph, &mut BTreeMap<RoomId,(i32,i32)>)` unchanged. `MOVE_RADIUS: i32` defined and used in Task 2. The render test uses `Color::DarkGray`/`Cyan`/`Magenta` consistent with the renderer's `PATH_BG_*` styles. ✓
