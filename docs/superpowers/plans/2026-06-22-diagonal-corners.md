# Diagonal Corner Arrows — Implementation Plan (Feature 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render NE/NW/SE/SW edges with a diagonal arrow glyph at the matching box corner (replacing `╭╮╰╯`) and anchor the connector line at that corner, instead of collapsing the diagonal to a cardinal `▲`/`▼` arrow on a side center.

**Architecture:** The mapper carries each connector's raw compass direction(s) so the renderer knows when an exit/arrival is diagonal. The renderer resolves a diagonal exit/entry to a box *corner* anchor (not a side center), draws the diagonal glyph there, and the existing lane bridge routes orthogonally from the corner. No true diagonal lines and no spacing change (those are deferred per the spec).

**Tech Stack:** Rust workspace (`mapper`, `app` crates), ratatui 0.29 TUI.

## Global Constraints

- Corner mapping: **NE → top-right (`↗`), NW → top-left (`↖`), SE → bottom-right (`↘`), SW → bottom-left (`↙`)**. Glyphs are named, swappable constants.
- The connector line **anchors at the corner cell** for a diagonal exit/entry; routing stays orthogonal through the existing lane system.
- Reciprocal diagonal pairs get a corner arrow at **both** ends; a one-way diagonal marks only the origin corner.
- Diagonal corner arrows are connector arrows → drawn in normal view, **suppressed under `Ctrl+P`** (same `draw_connector_arrows` path).
- Boxes zoom only (corners only exist there). Determinism preserved.
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Carry compass direction(s) on the routed connector (mapper)

**Files:**
- Modify: `crates/mapper/src/direction.rs` — add `is_diagonal`.
- Modify: `crates/mapper/src/route/mod.rs` — add `exit_dir`/`entry_dir` to `RoutedConnector` (struct ~line 23) and set them at BOTH construction sites (~line 479 the direct-route push, ~line 554 the main push).
- Test: `crates/mapper/src/route/mod.rs` and `crates/mapper/src/direction.rs`.

**Interfaces:**
- Consumes: `Direction`, `Connection { dir }`, the existing `c` (this edge) and `back: Option<&Connection>` (the paired back-edge) locals in `route_topology_with`.
- Produces:
  - `pub fn is_diagonal(d: Direction) -> bool` in `direction.rs`.
  - `RoutedConnector { …, pub exit_dir: Direction, pub entry_dir: Option<Direction> }` — `exit_dir` = the origin edge's compass dir; `entry_dir` = the paired back-edge's dir (`Some` only when a bidirectional pairing collapsed), else `None`.

- [ ] **Step 1: Write the failing tests**

In `crates/mapper/src/direction.rs` tests:

```rust
    #[test]
    fn is_diagonal_only_for_intercardinals() {
        assert!(is_diagonal(Direction::NE));
        assert!(is_diagonal(Direction::NW));
        assert!(is_diagonal(Direction::SE));
        assert!(is_diagonal(Direction::SW));
        assert!(!is_diagonal(Direction::N));
        assert!(!is_diagonal(Direction::E));
        assert!(!is_diagonal(Direction::Up));
    }
```

In `crates/mapper/src/route/mod.rs` tests:

```rust
    #[test]
    fn connector_carries_diagonal_exit_dir() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let conns = route_topology(&g);
        let c = conns.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert_eq!(c.exit_dir, Direction::SW);
        assert_eq!(c.entry_dir, None, "one-way edge has no back-edge dir");
    }

    #[test]
    fn reciprocal_diagonal_carries_both_dirs() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::SW, 2);
        g.add_edge(2, Direction::NE, 1); // true reciprocal
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "reciprocal diagonal collapses to one connector");
        let c = &conns[0];
        assert!(c.reciprocal);
        let dirs = [c.exit_dir, c.entry_dir.expect("reciprocal carries entry_dir")];
        assert!(
            dirs.contains(&Direction::SW) && dirs.contains(&Direction::NE),
            "both diagonal directions carried: {dirs:?}"
        );
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p mapper is_diagonal_only_for_intercardinals connector_carries_diagonal_exit_dir reciprocal_diagonal_carries_both_dirs`
Expected: FAIL — `cannot find function 'is_diagonal'` / `no field 'exit_dir'`.

- [ ] **Step 3: Add `is_diagonal`**

In `crates/mapper/src/direction.rs`:

```rust
/// True for the four intercardinal directions (NE/NW/SE/SW).
pub fn is_diagonal(d: Direction) -> bool {
    matches!(d, Direction::NE | Direction::NW | Direction::SE | Direction::SW)
}
```

- [ ] **Step 4: Add the fields to `RoutedConnector`**

In `crates/mapper/src/route/mod.rs`, add to the `RoutedConnector` struct (after `reciprocal`):

```rust
    /// The origin edge's compass direction (so the renderer can pick a diagonal corner).
    pub exit_dir: crate::direction::Direction,
    /// The paired back-edge's compass direction, set only when a bidirectional pairing
    /// collapsed into this connector (used for the far-end diagonal corner). `None` otherwise.
    pub entry_dir: Option<crate::direction::Direction>,
```

- [ ] **Step 5: Set them at both construction sites**

In `route_topology_with`, the direct-route push (`out.push(RoutedConnector { … })` near line 479) and the main push (near line 554) each have `c` (this edge) and `back: Option<&Connection>` in scope. Add to BOTH:

```rust
                    exit_dir: c.dir,
                    entry_dir: back.map(|bk| bk.dir),
```

(Match the indentation of each push site.)

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p mapper is_diagonal_only_for_intercardinals connector_carries_diagonal_exit_dir reciprocal_diagonal_carries_both_dirs`
Expected: PASS (all three).

- [ ] **Step 7: Run the full mapper suite**

Run: `cargo test -p mapper`
Expected: PASS (no regressions).

- [ ] **Step 8: Commit**

```bash
git add crates/mapper/src/direction.rs crates/mapper/src/route/mod.rs
git commit -m "feat(mapper): carry compass direction(s) on routed connectors

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Render diagonal corner arrows (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `diagonal_arrow` + `corner_anchor`; resolve diagonal exits/entries to corner anchors in `plot_connector` (~lines 457-458); use the diagonal glyph in `render_lane_connectors` (~lines 577, 580).
- Test: `crates/app/src/render/map.rs`.

**Interfaces:**
- Consumes: `RoutedConnector { exit_dir, entry_dir, exit, entry, exit_slot, entry_slot, reciprocal, distorted }` (Task 1), `mapper::direction::{Direction, is_diagonal}`, `PosTable::room_pixel`, `BOX_W`, `BOX_H`, `box_edge_anchor`, `arrow_for_departure`.
- Produces: `fn diagonal_arrow(dir: Direction) -> &'static str`, `fn corner_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), dir: Direction) -> (i32, i32)`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn diagonal_edge_draws_corner_arrow() {
        // 1 →SW→ 2 (room 2 south-west of room 1): ↙ replaces room 1's bottom-left corner.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(1) - off.0; // room 1 at col 1
        let by = rows.room_pixel(0) - off.1; // room 1 at row 0
        let sym = buf
            .cell((bx as u16, (by + BOX_H - 1) as u16))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default();
        assert_eq!(sym, "↙", "SW edge draws ↙ at room 1's bottom-left corner");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app diagonal_edge_draws_corner_arrow`
Expected: FAIL — the bottom-left cell is `╰` (or a `▼` on the side), not `↙`.

- [ ] **Step 3: Add the glyph + corner-anchor helpers**

In `crates/app/src/render/map.rs` (near `arrow_for_departure`):

```rust
/// Diagonal arrow glyphs (swappable named constants; e.g. to `◥◤◣◢`).
const DIAG_NE: &str = "↗";
const DIAG_NW: &str = "↖";
const DIAG_SE: &str = "↘";
const DIAG_SW: &str = "↙";

/// Arrow glyph for a diagonal departure/arrival (caller guards with `is_diagonal`).
fn diagonal_arrow(dir: Direction) -> &'static str {
    match dir {
        Direction::NE => DIAG_NE,
        Direction::NW => DIAG_NW,
        Direction::SE => DIAG_SE,
        Direction::SW => DIAG_SW,
        _ => DIAG_NE, // unreachable when guarded by is_diagonal
    }
}

/// The box-corner cell (virtual pixels) for a diagonal direction: NE→top-right, NW→top-left,
/// SE→bottom-right, SW→bottom-left.
fn corner_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), dir: Direction) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    match dir {
        Direction::NE => (bx + BOX_W - 1, by),
        Direction::NW => (bx, by),
        Direction::SE => (bx + BOX_W - 1, by + BOX_H - 1),
        Direction::SW => (bx, by + BOX_H - 1),
        _ => (bx + BOX_W / 2, by), // unreachable when guarded by is_diagonal
    }
}
```

- [ ] **Step 4: Resolve diagonal anchors in `plot_connector`**

In `plot_connector`, replace the two anchor lines (currently):

```rust
    let dep_anchor = box_edge_anchor(cols, rows, origin_cell, conn.exit, conn.exit_slot);
    let arr_anchor = box_edge_anchor(cols, rows, dest_cell, conn.entry, conn.entry_slot);
```

with:

```rust
    let dep_anchor = if mapper::direction::is_diagonal(conn.exit_dir) {
        corner_anchor(cols, rows, origin_cell, conn.exit_dir)
    } else {
        box_edge_anchor(cols, rows, origin_cell, conn.exit, conn.exit_slot)
    };
    let arr_anchor = match conn.entry_dir {
        Some(d) if mapper::direction::is_diagonal(d) => corner_anchor(cols, rows, dest_cell, d),
        _ => box_edge_anchor(cols, rows, dest_cell, conn.entry, conn.entry_slot),
    };
```

- [ ] **Step 5: Use the diagonal glyph for the arrowheads**

In `render_lane_connectors`, the departure arrowhead push (currently `arrowheads.push((plot.dep_anchor, arrow_for_departure(conn.exit), conn.distorted));`) and the reciprocal far-end push (`arrowheads.push((plot.arr_anchor, arrow_for_departure(conn.entry), conn.distorted));`) become:

```rust
        let dep_glyph = if mapper::direction::is_diagonal(conn.exit_dir) {
            diagonal_arrow(conn.exit_dir)
        } else {
            arrow_for_departure(conn.exit)
        };
        arrowheads.push((plot.dep_anchor, dep_glyph, conn.distorted));
        // Far-end arrow only for true reciprocal connectors (collapsed opposite pairs).
        if conn.reciprocal {
            let arr_glyph = match conn.entry_dir {
                Some(d) if mapper::direction::is_diagonal(d) => diagonal_arrow(d),
                _ => arrow_for_departure(conn.entry),
            };
            arrowheads.push((plot.arr_anchor, arr_glyph, conn.distorted));
        }
```

- [ ] **Step 6: Run the new test, then the full app suite**

Run: `cargo test -p app diagonal_edge_draws_corner_arrow`
Expected: PASS.

Run: `cargo test -p app`
Expected: PASS (existing connector/arrow tests use cardinal edges → unaffected).

- [ ] **Step 7: Confirm clippy is clean on the lib**

Run: `cargo clippy -p app --lib`
Expected: no new warnings in `render/map.rs`.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): draw diagonal edges as corner arrows

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (after both tasks)

Run babelmap on the A129 story, Boxes zoom: room #136 (`136 →SW→ 27`) should show `↙` at its bottom-left corner with the connector line leaving from there toward #27; cardinal connectors are unchanged. Toggle `Ctrl+P` — the diagonal corner arrow disappears with the other connector arrowheads.

## Notes / out of scope

- True diagonal `╱`/`╲` lines and squared box spacing are deferred (see the spec's Feature 1 "Deferred" note); this ships corner arrows over the existing orthogonal routing so the look can be evaluated first.
- The line attaching cleanly at the corner is the visual risk to evaluate; if a corner-anchored bridge reads badly, the follow-up is to tune `attach_bridge`/the corner anchor (not in this plan).
