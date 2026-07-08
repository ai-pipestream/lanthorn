# Redundant Compass Collapse + Secondary-Direction Markers — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When multiple compass directions connect the same room pair the same way (Zork house ring: `68↔217` via `S+SE`/`W+NW`), draw ONE brighter "shared path" connector and preserve every hidden command as an interior arrow marker.

**Architecture:** Render-level and non-destructive. A pre-pass in `route_topology_with` keeps the best compass edge per direction-of-travel bucket and records the rest as *secondaries* on the retained `RoutedConnector`. The app draws a connector with secondaries in a new brighter `shared_path` color (line + arrowheads + markers) and stamps the secondary directions as arrow glyphs inside the box beside the retained arrowhead.

**Tech Stack:** Rust workspace; `mapper` crate (zero-dep layout/routing), `app` crate (ratatui TUI). Quest SQ-0225.

## Global Constraints

- **Non-destructive:** never mutate `MapGraph`; graph edges stay intact. Collapse affects only the drawn connector set.
- **Boxes zoom only** for markers (interiors don't exist at Compact/Overview).
- **Themeable:** the new color is one `ColorScheme` field with a `style.toml` selector; the marker glyph honors `symbols.arrows`.
- **Up/Down excluded:** `grid_offset(dir).is_some()` gates the 8-way compass; Up/Down/In/Out/Unknown never collapse here.
- Run `cargo test -p mapper` and `cargo test -p app` per task; both must stay green (mapper ~186, app ~1050+, 2 Glulx ignored).

---

### Task 1: Route-level collapse + secondary recording (mapper)

**Files:**
- Modify: `crates/mapper/src/route/mod.rs` (RoutedConnector struct ~21-59; `route_topology_with` ~636-868; new `select_shared_paths`)
- Test: `crates/mapper/src/route/mod.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::layout::edge_is_satisfied(graph, conn) -> bool` (pub, `layout/mod.rs:161`); `grid_offset`, `opposite` (already imported in route).
- Produces: `RoutedConnector.secondary_exit: Vec<Direction>`, `RoutedConnector.secondary_entry: Vec<Direction>` (empty for ordinary connectors).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/mapper/src/route/mod.rs`:

```rust
#[test]
fn redundant_pair_collapses_to_one_shared_connector() {
    use crate::graph::MapGraph;
    use crate::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(68, "West".into());
    g.upsert_room(217, "South".into());
    g.set_pos(68, (0, 0));
    g.set_pos(217, (1, 1)); // 217 is SE of 68 → SE/NW satisfied, S/W not
    g.add_edge(68, Direction::S, 217);
    g.add_edge(68, Direction::SE, 217);
    g.add_edge(217, Direction::W, 68);
    g.add_edge(217, Direction::NW, 68);
    let plan = route_lanes(&g);
    let between: Vec<_> = plan.connectors.iter()
        .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == (68, 217))
        .collect();
    assert_eq!(between.len(), 1, "the pair must collapse to a single connector");
    let c = between[0];
    // retained = the satisfied diagonal pairing
    assert_eq!(c.exit_dir, Direction::SE);
    assert_eq!(c.entry_dir, Some(Direction::NW));
    // secondaries: S at the 68 (exit/origin) end, W at the 217 (entry/dest) end
    let (exit_dirs, entry_dirs) = if c.origin == 68 {
        (&c.secondary_exit, &c.secondary_entry)
    } else {
        (&c.secondary_entry, &c.secondary_exit)
    };
    assert!(exit_dirs.contains(&Direction::S), "S recorded at the 68 end");
    assert!(entry_dirs.contains(&Direction::W), "W recorded at the 217 end");
}

#[test]
fn three_back_edges_collapse_keeping_the_straight_pair() {
    use crate::graph::MapGraph;
    use crate::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(33, "F".into());
    g.upsert_room(175, "F".into());
    g.set_pos(33, (0, 0));
    g.set_pos(175, (1, 0)); // 175 is due E → E/W satisfied, N/S not
    g.add_edge(33, Direction::E, 175);
    g.add_edge(175, Direction::W, 33);
    g.add_edge(175, Direction::N, 33);
    g.add_edge(175, Direction::S, 33);
    let plan = route_lanes(&g);
    let c = plan.connectors.iter()
        .find(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == (33, 175))
        .expect("one connector");
    assert_eq!(plan.connectors.iter()
        .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == (33, 175)).count(), 1);
    // N and S (both origin 175) become secondaries at the 175 end
    let at_175 = if c.origin == 175 { &c.secondary_exit } else { &c.secondary_entry };
    assert!(at_175.contains(&Direction::N) && at_175.contains(&Direction::S));
}

#[test]
fn ordinary_reciprocal_pair_has_no_secondaries() {
    use crate::graph::MapGraph;
    use crate::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(1, "a".into());
    g.upsert_room(2, "b".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (1, 0));
    g.add_edge(1, Direction::E, 2);
    g.add_edge(2, Direction::W, 1);
    let plan = route_lanes(&g);
    for c in &plan.connectors {
        assert!(c.secondary_exit.is_empty() && c.secondary_entry.is_empty());
    }
}

#[test]
fn collapse_does_not_mutate_the_graph() {
    use crate::graph::MapGraph;
    use crate::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(68, "W".into());
    g.upsert_room(217, "S".into());
    g.set_pos(68, (0, 0));
    g.set_pos(217, (1, 1));
    for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                        (217, Direction::W, 68), (217, Direction::NW, 68)] {
        g.add_edge(o, d, dst);
    }
    let before = g.connections().len();
    let _ = route_lanes(&g);
    assert_eq!(g.connections().len(), before, "routing must not delete edges");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

Run: `cargo test -p mapper redundant_pair_collapses three_back_edges ordinary_reciprocal collapse_does_not_mutate`
Expected: FAIL — `secondary_exit`/`secondary_entry` fields don't exist; multiple connectors between the pair.

- [ ] **Step 3: Add the `RoutedConnector` fields**

In the `RoutedConnector` struct (`route/mod.rs:~23`), after the `merge: bool` field:

```rust
    /// Compass directions collapsed at the EXIT end (origin room): extra same-pair
    /// edges NOT drawn as their own line. The renderer stamps a marker for each,
    /// beside this connector's exit arrowhead. Empty for ordinary connectors.
    pub secondary_exit: Vec<crate::direction::Direction>,
    /// Compass directions collapsed at the ENTRY end (dest room).
    pub secondary_entry: Vec<crate::direction::Direction>,
```

Add `secondary_exit: Vec::new(), secondary_entry: Vec::new(),` to EVERY `RoutedConnector { .. }` construction in `route_topology_with` (there are three: the merge-stub branch ~696, the direct-route branch ~753, and the weaving branch ~846). Compile with `cargo build -p mapper` and fix any missed construction sites the compiler flags.

- [ ] **Step 4: Add the `select_shared_paths` pre-pass**

Add near the top of `route/mod.rs` (after imports), a helper and record type:

```rust
/// One collapsed (secondary) compass edge: its unordered pair, its origin room, and its direction.
struct Secondary {
    pair: (RoomId, RoomId),
    origin: RoomId,
    dir: Direction,
}

/// For each unordered room pair, keep the best compass edge in each direction-of-travel
/// bucket (forward = origin is the lower id, backward = the higher id); the rest are
/// secondaries. "Best" = geometrically satisfied first, then the exact-opposite of the
/// other bucket's pick (straightness), then lowest connection index. Returns the set of
/// secondary indices into `graph.connections()` and the secondary records. Up/Down and
/// non-compass edges are excluded (only `grid_offset` 8-way edges are considered).
fn select_shared_paths(
    graph: &MapGraph,
) -> (std::collections::BTreeSet<usize>, Vec<Secondary>) {
    use std::collections::{BTreeMap, BTreeSet};
    let conns = graph.connections();
    let mut by_pair: BTreeMap<(RoomId, RoomId), Vec<usize>> = BTreeMap::new();
    for (i, c) in conns.iter().enumerate() {
        if grid_offset(c.dir).is_none() {
            continue; // compass 8-way only; Up/Down/In/Out/Unknown excluded
        }
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        by_pair.entry(pair).or_default().push(i);
    }
    let mut secondary_idx: BTreeSet<usize> = BTreeSet::new();
    let mut records: Vec<Secondary> = Vec::new();
    for (pair, idxs) in by_pair {
        let fwd: Vec<usize> = idxs.iter().copied().filter(|&i| conns[i].origin == pair.0).collect();
        let bwd: Vec<usize> = idxs.iter().copied().filter(|&i| conns[i].origin == pair.1).collect();
        // Nothing to collapse unless at least one bucket has an extra edge.
        if fwd.len() <= 1 && bwd.len() <= 1 {
            continue;
        }
        // Pick the retained edge of a bucket: satisfied first, optional opposite-direction
        // preference (straightness), then lowest index.
        let pick = |bucket: &[usize], prefer_opp: Option<Direction>| -> Option<usize> {
            bucket.iter().copied().min_by_key(|&i| {
                let sat = crate::layout::edge_is_satisfied(graph, &conns[i]);
                let opp = prefer_opp == Some(conns[i].dir);
                (std::cmp::Reverse(sat), std::cmp::Reverse(opp), i)
            })
        };
        let ret_fwd = pick(&fwd, None);
        let ret_bwd = pick(&bwd, ret_fwd.map(|i| opposite(conns[i].dir)));
        for &i in fwd.iter().chain(bwd.iter()) {
            if Some(i) != ret_fwd && Some(i) != ret_bwd {
                secondary_idx.insert(i);
                records.push(Secondary { pair, origin: conns[i].origin, dir: conns[i].dir });
            }
        }
    }
    (secondary_idx, records)
}
```

- [ ] **Step 5: Filter the working set and attach secondaries in `route_topology_with`**

At the start of `route_topology_with` (before the `compass` working set is built, ~660), compute the collapse:

```rust
    let (secondary_idx, secondary_records) = select_shared_paths(graph);
```

Change the `compass` working-set builder (`route/mod.rs:660-664`) to exclude secondary indices. Because `compass` is later indexed positionally, filter on the ORIGINAL connection index:

```rust
    let compass: Vec<&crate::graph::Connection> = graph
        .connections()
        .iter()
        .enumerate()
        .filter(|(i, c)| layout_offset(c.dir).is_some() && !secondary_idx.contains(i))
        .map(|(_, c)| *c)
        .collect();
```

Then, immediately before `route_topology_with` returns its `Vec<RoutedConnector>` (the end of the function, ~868), attach the secondaries to their pair's retained COMPASS connector:

```rust
    for rec in &secondary_records {
        if let Some(conn) = connectors.iter_mut().find(|c| {
            (c.origin.min(c.dest), c.origin.max(c.dest)) == rec.pair
                && grid_offset(c.exit_dir).is_some() // the compass connector, not an Up/Down one
        }) {
            if rec.origin == conn.origin {
                conn.secondary_exit.push(rec.dir);
            } else if rec.origin == conn.dest {
                conn.secondary_entry.push(rec.dir);
            }
        }
    }
    connectors
```

(Replace the bare trailing `connectors` return with the block above. If the function's local is named differently, rename accordingly.)

- [ ] **Step 6: Run the tests**

Run: `cargo test -p mapper`
Expected: PASS — new tests green, existing route/layout tests unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/mapper/src/route/mod.rs
git commit -m "feat(route): collapse redundant compass pairs into one shared connector + record secondaries (SQ-0225)

Quest: SQ-0225"
```

---

### Task 2: `shared_path` color — field, defaults, line + arrowhead (app)

**Files:**
- Modify: `crates/app/src/colors.rs` (`ColorScheme` struct ~210; terminal defaults ~369; palette resolve ~498/553)
- Modify: `crates/app/src/render/map.rs` (`render_lane_connectors` style pick ~956; `Arrowhead` alias ~769; arrowhead build sites; `draw_connector_arrows` ~1019)
- Test: `crates/app/src/render/map.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `RoutedConnector.secondary_exit/secondary_entry` (Task 1).
- Produces: `ColorScheme.shared_path: Style`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
#[test]
fn shared_connector_line_uses_shared_path_color() {
    use crate::state::AppState;
    use mapper::graph::MapGraph;
    use mapper::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(68, "W".into());
    g.upsert_room(217, "S".into());
    g.set_pos(68, (0, 0));
    g.set_pos(217, (1, 1));
    for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                        (217, Direction::W, 68), (217, Direction::NW, 68)] {
        g.add_edge(o, d, dst);
    }
    let mut state = AppState::default_for_test(); // existing helper used by nearby tests
    state.zoom = crate::state::Zoom::Boxes;
    let rm = mapper::render::render(&g);
    let area = ratatui::layout::Rect { x: 0, y: 0, width: 60, height: 30 };
    let mut buf = ratatui::buffer::Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);
    // At least one cell painted with the shared_path style exists (the shared line/arrow).
    let shared = state.colors.shared_path;
    let found = (0..area.width).flat_map(|x| (0..area.height).map(move |y| (x, y)))
        .any(|(x, y)| buf.cell((x, y)).map(|c| c.style() == shared).unwrap_or(false));
    assert!(found, "the collapsed pair's shared path must paint with shared_path color");
}
```

(If `AppState::default_for_test`/buffer construction differs, match the pattern already used by tests near `map.rs:2019`.)

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p app shared_connector_line_uses_shared_path_color`
Expected: FAIL — `shared_path` field missing.

- [ ] **Step 3: Add the `shared_path` field + defaults**

In `crates/app/src/colors.rs`, `ColorScheme` struct after `portal_connector`:

```rust
    /// A "shared path" connector — one that collapses several same-pair compass
    /// directions into one line. Deliberately BRIGHTER than `connector`; its line,
    /// arrowheads, and secondary markers all use this color.
    pub shared_path: Style,
```

Terminal defaults (`colors.rs:~369`, beside `connector:`):

```rust
    shared_path: Style::new().fg(Color::LightCyan),
```

Palette resolve (`colors.rs:~498` add a resolver, and `~553` add the field). Use a bright palette slot:

```rust
    let shared_path_fg = resolve_element("shared_path", scheme.palette[14]); // bright cyan slot
```
```rust
    shared_path: Style::new().fg(shared_path_fg),
```

- [ ] **Step 4: Apply `shared_path` to the connector line**

In `render_lane_connectors` (`map.rs:956`), replace the style pick with:

```rust
    let has_secondary = !conn.secondary_exit.is_empty() || !conn.secondary_entry.is_empty();
    let style = if is_updown {
        colors.portal_connector
    } else if has_secondary {
        colors.shared_path
    } else if conn.distorted {
        colors.connector_distorted
    } else {
        colors.connector
    };
```

- [ ] **Step 5: Carry "shared" into the arrowhead and color it**

Extend the `Arrowhead` tuple alias (`map.rs:769`) with a trailing `shared: bool`:

```rust
type Arrowhead = ((i32, i32), String, bool, bool, RoomId, bool); // (..distorted, is_portal, room, shared)
```

At the two arrowhead push sites in `render_lane_connectors` (departure and arrival), append `has_secondary` as the new field. In `draw_connector_arrows` (`map.rs:1019`), destructure the extra field and, when `shared` is true AND the arrowhead is not selected/current-highlighted, use `colors.shared_path` — mirror the existing branch that picks `colors.connector_distorted` for a distorted arrowhead (read that arm and add a `shared` arm with higher precedence than distorted). Keep selection/current highlighting winning over `shared`.

- [ ] **Step 6: Run tests**

Run: `cargo test -p app`
Expected: PASS — new test green; existing render tests unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/render/map.rs
git commit -m "feat(render): brighter shared_path color for collapsed connectors (line + arrowheads) (SQ-0225)

Quest: SQ-0225"
```

---

### Task 3: Secondary-direction markers (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` (new `arrow_for_direction`, new `draw_secondary_markers`, call site in `render_map` ~549)
- Test: `crates/app/src/render/map.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `RoutePlan.connectors[..].secondary_exit/secondary_entry`, `box_edge_anchor` (`map.rs:1092`), `put_char` (`render/mod.rs:121`), `symbols::Arrows`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn secondary_marker_glyph_drawn_inside_room_in_shared_color() {
    use crate::state::AppState;
    use mapper::graph::MapGraph;
    use mapper::direction::Direction;
    let mut g = MapGraph::new();
    g.upsert_room(68, "W".into());
    g.upsert_room(217, "S".into());
    g.set_pos(68, (0, 0));
    g.set_pos(217, (1, 1));
    for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                        (217, Direction::W, 68), (217, Direction::NW, 68)] {
        g.add_edge(o, d, dst);
    }
    let mut state = AppState::default_for_test();
    state.zoom = crate::state::Zoom::Boxes;
    let south = state.symbols.arrows.south.to_string();
    let west = state.symbols.arrows.west.to_string();
    let shared = state.colors.shared_path;
    let rm = mapper::render::render(&g);
    let area = ratatui::layout::Rect { x: 0, y: 0, width: 60, height: 30 };
    let mut buf = ratatui::buffer::Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);
    let has_glyph = |glyph: &str| (0..area.width)
        .flat_map(|x| (0..area.height).map(move |y| (x, y)))
        .any(|(x, y)| buf.cell((x, y))
            .map(|c| c.symbol() == glyph && c.style() == shared).unwrap_or(false));
    assert!(has_glyph(&south), "S secondary marker present in shared color");
    assert!(has_glyph(&west), "W secondary marker present in shared color");
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p app secondary_marker_glyph_drawn_inside_room`
Expected: FAIL — markers not drawn yet.

- [ ] **Step 3: Add `arrow_for_direction` + `draw_secondary_markers`**

In `crates/app/src/render/map.rs`:

```rust
/// Arrow glyph for a compass Direction (used by secondary markers). Up/Down never
/// appear here (they are not collapsed into compass secondaries).
fn arrow_for_direction(dir: Direction, arrows: &crate::symbols::Arrows) -> char {
    match dir {
        Direction::N => arrows.north,
        Direction::S => arrows.south,
        Direction::E => arrows.east,
        Direction::W => arrows.west,
        Direction::NE => arrows.ne,
        Direction::NW => arrows.nw,
        Direction::SE => arrows.se,
        Direction::SW => arrows.sw,
        _ => arrows.north, // unreachable: secondaries are compass only
    }
}

/// Stamp collapsed secondary directions as arrow glyphs on the box interior, one cell
/// inward from the retained connector's arrowhead (stacking further inward for multiples).
/// Boxes zoom only; caller passes the axis tables. Color is `shared_path`.
fn draw_secondary_markers(
    rm: &RenderMap,
    cols: &PosTable,
    rows: &PosTable,
    state: &AppState,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    let (off_x, off_y) = offset;
    let style = state.colors.shared_path;
    let arrows = &state.symbols.arrows;
    let cell_of = |id: RoomId| rm.rooms.iter().find(|r| r.id == id).map(|r| r.cell);

    let mut stamp = |dirs: &[Direction], cell: (i32, i32), side: crate::router::Side, slot: u16,
                     buf: &mut Buffer| {
        let (ax, ay) = box_edge_anchor(cols, rows, cell, side, slot);
        // Inward step, perpendicular to the side.
        let (dx, dy) = match side {
            crate::router::Side::Right => (-1, 0),
            crate::router::Side::Left => (1, 0),
            crate::router::Side::Top => (0, 1),
            crate::router::Side::Bottom => (0, -1),
        };
        // Interior depth available before hitting the far border.
        let depth = match side {
            crate::router::Side::Left | crate::router::Side::Right => BOX_W - 2,
            crate::router::Side::Top | crate::router::Side::Bottom => BOX_H - 2,
        };
        for (k, dir) in dirs.iter().enumerate() {
            let step = k as i32 + 1;
            if step > depth {
                break; // interior full (never happens for the realistic ≤2 case)
            }
            let ch = arrow_for_direction(*dir, arrows);
            put_char(buf, ax + dx * step + off_x, ay + dy * step + off_y, ch, style, area);
        }
    };

    for conn in &rm.plan.connectors {
        if !conn.secondary_exit.is_empty() {
            if let Some(cell) = cell_of(conn.origin) {
                stamp(&conn.secondary_exit, cell, conn.exit, conn.exit_slot, buf);
            }
        }
        if !conn.secondary_entry.is_empty() {
            if let Some(cell) = cell_of(conn.dest) {
                stamp(&conn.secondary_entry, cell, conn.entry, conn.entry_slot, buf);
            }
        }
    }
}
```

- [ ] **Step 4: Call it from `render_map`**

In `render_map`, inside the `if !state.show_portal_labels { … if boxes { … } }` block (right after `draw_deduped_updown_border_glyphs`, ~549), add — guarded by the axis tables so it only runs in Boxes zoom:

```rust
            if let Some((cols, rows)) = &axes {
                draw_secondary_markers(rm, cols, rows, state, (off_x, off_y), area, buf);
            }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(render): draw collapsed compass directions as interior secondary markers (SQ-0225)

Quest: SQ-0225"
```

---

### Task 4: `shared_path` style.toml selector (app)

**Files:**
- Modify: `crates/app/src/style.rs` (`SELECTOR_FIELDS` ~163; `style_for_selector` ~260; apply loop ~401; serialize ~1263)
- Test: `crates/app/src/style.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `ColorScheme.shared_path` (Task 2).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn shared_path_selector_round_trips() {
    // Applying a style.toml `shared-path` color patches ColorScheme.shared_path,
    // and serializing it back preserves the selector.
    let mut cs = crate::colors::ColorScheme::default();
    let toml = "[colors.selectors]\nshared-path = { fg = \"#ff00ff\" }\n";
    apply_style_toml_str(&mut cs, toml); // use the crate's existing apply entry point
    assert_eq!(cs.shared_path.fg, Some(ratatui::style::Color::Rgb(255, 0, 255)));
    let out = serialize_color_scheme(&cs); // existing serialize entry point
    assert!(out.contains("shared-path"), "selector survives round-trip");
}
```

(Match the exact apply/serialize helper names used by the existing `connector` selector tests in `style.rs`; the assertion targets are the four wiring sites below.)

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p app shared_path_selector_round_trips`
Expected: FAIL — selector unknown.

- [ ] **Step 3: Wire the four selector sites**

Mirror the existing `connector` wiring, using the kebab-case selector `"shared-path"`:

- `SELECTOR_FIELDS` (`style.rs:~163`): add `"shared-path",`
- `style_for_selector` (`style.rs:~260`): add `"shared-path" => cs.shared_path,`
- apply loop (`style.rs:~401`): add `"shared-path" => cs.shared_path = cs.shared_path.patch(style),`
- serialize (`style.rs:~1263`): add
  `doc.colors.selectors.insert("shared-path".to_string(), style_to_decl(&cs.shared_path));`

- [ ] **Step 4: Run tests**

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/style.rs
git commit -m "feat(style): themeable shared-path selector (SQ-0225)

Quest: SQ-0225"
```

---

### Task 5: End-to-end ring acceptance (app)

**Files:**
- Test: `crates/app/src/render/map.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: everything above; `render_overlap_stats` (existing).

- [ ] **Step 1: Write the acceptance test**

Build the four-room house ring (diamond) with both cardinal and diagonal edges each way, and assert the collapse + no illegal overlap:

```rust
#[test]
fn house_ring_collapses_to_clean_lines_with_no_illegal_overlap() {
    use mapper::graph::MapGraph;
    use mapper::direction::Direction;
    let mut g = MapGraph::new();
    for (id, p) in [(143, (1, 2)), (89, (2, 3)), (217, (1, 4)), (68, (0, 3))] {
        g.upsert_room(id, "r".into());
        g.set_pos(id, p);
    }
    // Diamond ring: each adjacent pair reachable by a cardinal AND a diagonal, both ways.
    let edges = [
        (68, Direction::N, 143), (68, Direction::NE, 143),
        (143, Direction::S, 68), (143, Direction::SW, 68),
        (143, Direction::E, 89), (143, Direction::SE, 89),
        (89, Direction::W, 143), (89, Direction::NW, 143),
        (89, Direction::S, 217), (89, Direction::SW, 217),
        (217, Direction::N, 89), (217, Direction::NE, 89),
        (217, Direction::W, 68), (217, Direction::NW, 68),
        (68, Direction::S, 217), (68, Direction::SE, 217),
    ];
    for (o, d, dst) in edges { g.add_edge(o, d, dst); }
    let plan = mapper::route::route_lanes(&g);
    // One compass connector per ring pair.
    for pair in [(68, 143), (89, 143), (89, 217), (68, 217)] {
        let n = plan.connectors.iter()
            .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == pair
                && mapper::direction::grid_offset(c.exit_dir).is_some())
            .count();
        assert_eq!(n, 1, "pair {pair:?} must collapse to one compass connector");
    }
    // No illegal overlaps in the rendered result.
    assert_eq!(render_overlap_stats(&g).0, 0, "ring must render without illegal overlap");
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p app house_ring_collapses_to_clean_lines`
Expected: PASS.

- [ ] **Step 3: Manual real-map check**

Launch the app on the real map and confirm visually: one line per ring pair, each ring pair's line brighter, with the secondary arrow(s) inside the rooms.

Run: `cargo run -p app -- --map ~/Downloads/map.json` (or load via the map screen), Boxes zoom.
Expected: the four house rooms show single brighter connectors with interior secondary arrows; no crossing tangle.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "test(render): house-ring collapse acceptance (SQ-0225)

Quest: SQ-0225"
```

---

## Self-Review notes

- **Spec coverage:** collapse selection (T1), non-destructive (T1 test), shared_path color on line+arrowheads (T2) and markers (T3), themeable selector (T4), Boxes-only + stacking (T3), up/down excluded (T1 `grid_offset` gate), acceptance/overlap (T5). ✔
- **Type consistency:** `secondary_exit`/`secondary_entry: Vec<Direction>` defined T1, consumed T2/T3; `shared_path: Style` defined T2, consumed T3/T4; `arrow_for_direction`, `draw_secondary_markers` defined and called T3. ✔
- **Open verification during execution:** the exact apply/serialize helper names in T4 Step 1 and the `AppState` test constructor in T2/T3 must be matched to the existing test patterns in each file (noted inline). The `draw_connector_arrows` shared-color arm (T2 Step 5) requires reading that function's existing distorted arm before mirroring it.
