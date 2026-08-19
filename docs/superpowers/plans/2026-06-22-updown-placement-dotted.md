# Up/Down Default Placement + Dotted Connectors — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Up-direction rooms a default home NW of their origin and Down-direction rooms SW (fallback only), and draw a dotted connector for the Up/Down link (from the origin's north side for Up, south for Down) when no compass connector already joins the pair. The `↑`/`↓` portal icons stay.

**Architecture:** A one-branch change in `place_incremental` sets the NW/SW default for Up/Down at discovery (it already no-ops on an already-placed target, so it's a pure fallback). A self-contained render pass in `app/render/map.rs` draws the dotted Up/Down lines (Boxes zoom), clipped out of room interiors, skipping pairs already joined by a compass connector and de-duplicating reciprocal Up/Down pairs.

**Tech Stack:** Rust workspace (`mapper`, `app` crates), ratatui 0.29 TUI.

## Global Constraints

- Default placement: **Up → NW (`prev + (-1,-1)`)**, **Down → SW (`prev + (-1,+1)`)**. Fallback only (never moves an already-placed target). In/Out/Unknown keep nearest-free-cell placement.
- Dotted connector drawn only when **no compass connector joins the pair**; a reciprocal Up/Down pair is drawn once (from the Up side). The `↑`/`↓` icons are unaffected.
- Up dotted line leaves the origin's **north** (top) side; Down leaves the **south** (bottom) side. No arrowhead. Drawn in both normal and portal (Ctrl+P) views.
- Boxes zoom only. Determinism preserved.
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Up→NW / Down→SW default placement (mapper)

**Files:**
- Modify: `crates/mapper/src/layout/incremental.rs` — the `None` branch of `place_incremental` (~lines 44-49).
- Test: `crates/mapper/src/layout/incremental.rs` (`#[cfg(test)] mod tests`).

**Interfaces:**
- Consumes: `Direction`, `prev_pos`, `occupied_cells`, `nearest_free_cell`.
- Produces: no new signature; the non-planar placement branch now gives Up/Down a diagonal ideal home.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/mapper/src/layout/incremental.rs`:

```rust
    #[test]
    fn up_room_defaults_north_west() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "attic".into());
        place_incremental(&mut g, 1, 2, Direction::Up);
        assert_eq!(g.room(2).unwrap().pos, Some((-1, -1)), "Up target defaults NW of origin");
    }

    #[test]
    fn down_room_defaults_south_west() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "cellar".into());
        place_incremental(&mut g, 1, 2, Direction::Down);
        assert_eq!(g.room(2).unwrap().pos, Some((-1, 1)), "Down target defaults SW of origin");
    }

    #[test]
    fn up_room_already_placed_is_not_moved() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "attic".into());
        g.set_pos(2, (5, 5));
        place_incremental(&mut g, 1, 2, Direction::Up);
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "fallback only — placed target untouched");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p mapper up_room_defaults_north_west down_room_defaults_south_west up_room_already_placed_is_not_moved`
Expected: FAIL on the first two (Up/Down currently land at the nearest free cell near `prev`, i.e. adjacent to (0,0), not the diagonal). The third already passes (the early-return guard).

- [ ] **Step 3: Give Up/Down a diagonal ideal home**

In `place_incremental`, replace the `None` branch (currently):

```rust
        None => {
            // Portal / unknown: nearest free cell starting from prev.
            let occ = occupied_cells(graph);
            let cell = nearest_free_cell(&occ, prev_pos);
            graph.set_pos(dest, cell);
        }
```

with:

```rust
        None => {
            // Up/Down get a default diagonal home (NW / SW); other non-planar directions
            // (In/Out/Unknown) take the nearest free cell near prev. Either way this is a
            // fallback — the early return above already left any placed target untouched.
            let ideal = match dir {
                Direction::Up => (prev_pos.0 - 1, prev_pos.1 - 1),   // NW
                Direction::Down => (prev_pos.0 - 1, prev_pos.1 + 1), // SW
                _ => prev_pos,
            };
            let occ = occupied_cells(graph);
            let cell = nearest_free_cell(&occ, ideal);
            graph.set_pos(dest, cell);
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mapper up_room_defaults_north_west down_room_defaults_south_west up_room_already_placed_is_not_moved`
Expected: PASS (all three).

- [ ] **Step 5: Run the full mapper suite**

Run: `cargo test -p mapper`
Expected: PASS. (Note: the existing `portal_dir_places_adjacent_without_overlap` test uses `Down` and only asserts the target is not on `prev` — `(-1,1) != (0,0)` still holds.)

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/layout/incremental.rs
git commit -m "feat(mapper): default Up rooms NW and Down rooms SW of origin

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Dotted Up/Down connectors (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `draw_portal_connectors` and call it in `render_map` (Boxes zoom, after the lane-connector block, before the room loop).
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`).

**Interfaces:**
- Consumes: `RenderMap { edges, rooms }`, `RoutedEdge { is_stub, dir, origin, dest }`, `placed: HashMap<RoomId, VRect>`, `Direction`, `BOX_W`, `BOX_H`, `put_char`, `Color`, `Style`, the `boxes` bool in `render_map`.
- Produces: `fn draw_portal_connectors(rm: &RenderMap, placed: &HashMap<RoomId, VRect>, off_x: i32, off_y: i32, area: Rect, buf: &mut Buffer)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn up_portal_draws_dotted_connector_when_no_compass_edge() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (0, 0)); // NW of room 1
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(has_dotted, "an Up portal with no compass edge draws a dotted connector");
    }

    #[test]
    fn up_portal_no_dotted_connector_when_compass_edge_joins_pair() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0)); // due north of room 1
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::N, 2); // a compass connector already joins the pair
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(!has_dotted, "no dotted line when a compass edge already joins the pair");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app up_portal_draws_dotted_connector_when_no_compass_edge up_portal_no_dotted_connector_when_compass_edge_joins_pair`
Expected: the first FAILS (no dotted glyph drawn yet); the second passes vacuously (also no dotted glyph). After implementation both must pass.

- [ ] **Step 3: Add `draw_portal_connectors`**

Add to `crates/app/src/render/map.rs` (near the other connector helpers):

```rust
/// Dotted-line glyphs for Up/Down portal connectors.
const DOTTED_V: char = '┊';
const DOTTED_H: char = '┄';

/// Draw dotted connectors for Up/Down portals whose pair is NOT already joined by a compass
/// connector. Up leaves the origin's north side, Down the south side, routing (vertical-first L)
/// to the placed target, clipped out of every room's box interior. A reciprocal Up/Down pair is
/// drawn once (from the Up side). No arrowhead — the `↑`/`↓` icon already marks the direction.
fn draw_portal_connectors(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let style = Style::new().fg(Color::Cyan);
    let interiors: Vec<VRect> = placed.values().copied().collect();
    let in_interior = |x: i32, y: i32| {
        interiors
            .iter()
            .any(|r| x > r.x && x < r.x + BOX_W - 1 && y > r.y && y < r.y + BOX_H - 1)
    };

    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        let up = match edge.dir {
            Direction::Up => true,
            Direction::Down => false,
            _ => continue, // In/Out/Unknown get no dotted line
        };
        // A reciprocal Up/Down pair is drawn once, from the Up side: skip the Down edge when a
        // matching Up edge (dest→Up→origin) exists.
        if !up
            && rm
                .edges
                .iter()
                .any(|e| e.dir == Direction::Up && e.origin == edge.dest && e.dest == edge.origin)
        {
            continue;
        }
        // Skip when a compass connector already joins the pair (either direction).
        let joined = rm.edges.iter().any(|e| {
            !e.is_stub
                && ((e.origin == edge.origin && e.dest == edge.dest)
                    || (e.origin == edge.dest && e.dest == edge.origin))
        });
        if joined {
            continue;
        }
        let (Some(&o), Some(&t)) = (placed.get(&edge.origin), placed.get(&edge.dest)) else {
            continue;
        };
        let ocx = o.x + BOX_W / 2;
        let start_y = if up { o.y - 1 } else { o.y + BOX_H };
        let tcx = t.x + BOX_W / 2;
        let tcy = t.y + BOX_H / 2;
        // Vertical-first L: down/up the origin's centre column to the target's mid-row, then
        // across to the target's centre column. Clipped out of room interiors.
        for y in start_y.min(tcy)..=start_y.max(tcy) {
            if !in_interior(ocx, y) {
                put_char(buf, ocx + off_x, y + off_y, DOTTED_V, style, area);
            }
        }
        for x in ocx.min(tcx)..=ocx.max(tcx) {
            if !in_interior(x, tcy) {
                put_char(buf, x + off_x, tcy + off_y, DOTTED_H, style, area);
            }
        }
    }
}
```

- [ ] **Step 4: Call it in `render_map`**

In `render_map`, after the lane-connector block (the `if let Some((cols, rows)) = &axes { … render_lane_connectors … }`) and before the room-drawing loop, add (gated on Boxes zoom):

```rust
    if boxes {
        draw_portal_connectors(rm, &placed, off_x, off_y, area, buf);
    }
```

(Placing it before the rooms lets the box borders draw over any dotted cell that lands on a border; the interior clip keeps it out of box interiors.)

- [ ] **Step 5: Run the new tests + the full app suite**

Run: `cargo test -p app up_portal_draws_dotted_connector_when_no_compass_edge up_portal_no_dotted_connector_when_compass_edge_joins_pair`
Expected: PASS (both).

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 6: Confirm clippy is clean on the lib**

Run: `cargo clippy -p app --lib`
Expected: no new warnings in `render/map.rs`.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): dotted connectors for Up/Down portals

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (after both tasks)

Run lanthorn on the A129 story, Boxes zoom: room #203 (`203 →Up→ 201` Attic, with #201 placed NW and no compass edge between them) should show a dotted line from #203's north side up to #201, with the `↑` icon still on #203 and `↓` on #201. A new exploration that goes Up should drop the new room to the NW; Down to the SW.

## Notes / out of scope

- The NW/SW default applies at discovery (`place_incremental`); a full global relayout may reposition such rooms by other constraints. Revisit only if the default needs to survive relayout.
- The dotted pass is a standalone draw (not lane-routed), so a long dotted line may cross other connectors/paths; this is acceptable for evaluation. If it reads badly, the follow-up is to route it through the lane system.
