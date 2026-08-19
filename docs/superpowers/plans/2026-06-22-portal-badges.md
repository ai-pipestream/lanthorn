# Portal Badges Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render Up/Down/In/Out/Unknown connections as a labeled badge (direction glyph + target room name) beside the room box at Boxes zoom, and list each portal with its full `glyph #id name` in the map dump.

**Architecture:** The mapper's `route_all` already emits a `RoutedEdge` stub for every non-planar connection; we add the target room's name to that stub so the renderer needn't re-resolve it (and so the design is layer-ready — the target may live off the current layer in future). The app draws a stacked, glyph-prefixed badge in the right gutter at Boxes zoom only (Compact keeps its current bare-label behavior), and the dump gains a PORTALS legend section.

**Tech Stack:** Rust workspace (`mapper`, `app` crates), ratatui 0.29 TUI.

## Global Constraints

- Portal direction glyphs are **named, swappable constants**: `↑` Up, `↓` Down, `⊙` In, `⊗` Out, `?` Unknown. Changing a glyph must be a one-line edit.
- **Unknown portals show only the `?` glyph** — no target name (Unknown has no target semantics).
- **Boxes zoom only.** Compact and Overview zoom keep their current behavior byte-for-byte.
- On-map badge name is **truncated to the gutter width** (`PORTAL_BADGE_W = 8` cells); the dump shows the full untruncated name.
- **Connectors win** over badges on a cell collision: badges are drawn BEFORE the lane connectors so a routed line overwrites a colliding badge cell.
- Determinism: identical input graph → identical render and dump.
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Carry the target room name on stub edges (mapper)

**Files:**
- Modify: `crates/mapper/src/router.rs` (struct `RoutedEdge` ~line 126; both `RoutedEdge` construction sites in `route_all` ~lines 190 and 231)
- Test: `crates/mapper/src/router.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `MapGraph`, `crate::graph::Room::label() -> &str` (already used in `render.rs` as `room.label().to_string()`).
- Produces: `RoutedEdge { …, pub dest_label: Option<String> }` — `Some(target room label)` for stubs, `None` for routed compass edges. The app's renderer and dump read this.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/mapper/src/router.rs`:

```rust
    #[test]
    fn stub_edge_carries_dest_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Cellar".into());
        g.upsert_room(2, "Attic".into());
        g.add_edge(1, Direction::Up, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && e.is_stub).unwrap();
        assert_eq!(
            e.dest_label.as_deref(),
            Some("Attic"),
            "a portal stub must carry its target room's name"
        );
    }

    #[test]
    fn compass_edge_has_no_dest_label() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        relayout_auto(&mut g);
        let edges = route_all(&g);
        let e = edges.iter().find(|e| e.origin == 1 && !e.is_stub).unwrap();
        assert_eq!(e.dest_label, None, "routed compass edges carry no dest_label");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p mapper dest_label`
Expected: FAIL — `no field 'dest_label' on type 'RoutedEdge'`.

- [ ] **Step 3: Add the field and populate it**

In `crates/mapper/src/router.rs`, add the field to `RoutedEdge` (after `arrival_dir`):

```rust
    /// Stubs only: the display name of the target room (`dest`), resolved from the graph so
    /// the renderer can label the badge without re-resolving it (and so the target may live
    /// off the current layer in future). `None` for routed compass edges.
    pub dest_label: Option<String>,
```

In `route_all`, the **stub** construction site (currently the block that builds the `RoutedEdge` with `is_stub: true`), set:

```rust
                dest_label: graph.room(conn.dest).map(|r| r.label().to_string()),
```

In `route_all`, the **compass** construction site (the `RoutedEdge` with `is_stub: false`), set:

```rust
            dest_label: None,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mapper dest_label`
Expected: PASS (both tests).

- [ ] **Step 5: Run the full mapper suite (no regressions)**

Run: `cargo test -p mapper`
Expected: PASS (all existing tests still green).

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/router.rs
git commit -m "feat(mapper): carry target room name on portal stub edges

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Portal glyph + badge-text helpers (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` (add imports + helpers near the other rendering helpers, e.g. just above `draw_stub` ~line 672)
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `mapper::direction::Direction` (add to imports), `RoutedEdge::dir`, `RoutedEdge::dest_label` (Task 1).
- Produces:
  - `pub(crate) fn portal_glyph(dir: Direction) -> &'static str` — used by the badge renderer (Task 3) and the dump legend (Task 4).
  - `fn portal_badge_text(dir: Direction, dest_label: Option<&str>) -> String` — the on-map badge string.
  - `const PORTAL_BADGE_W: usize = 8`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn portal_glyphs_map_directions() {
        assert_eq!(portal_glyph(Direction::Up), "↑");
        assert_eq!(portal_glyph(Direction::Down), "↓");
        assert_eq!(portal_glyph(Direction::In), "⊙");
        assert_eq!(portal_glyph(Direction::Out), "⊗");
        assert_eq!(portal_glyph(Direction::Unknown), "?");
    }

    #[test]
    fn portal_badge_truncates_to_gutter_width() {
        // glyph + space + name, capped at PORTAL_BADGE_W (8) chars.
        let b = portal_badge_text(Direction::Up, Some("South of House"));
        assert_eq!(b.chars().count(), 8, "badge clipped to gutter width");
        assert!(b.starts_with("↑ "), "glyph then space then name: {b}");
    }

    #[test]
    fn portal_badge_short_name_not_padded() {
        assert_eq!(portal_badge_text(Direction::Down, Some("Attic")), "↓ Attic");
    }

    #[test]
    fn portal_badge_unknown_is_just_glyph() {
        // Unknown has no target semantics → bare "?" even with a dest.
        assert_eq!(portal_badge_text(Direction::Unknown, Some("West of House")), "?");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app portal_`
Expected: FAIL — `cannot find function 'portal_glyph'` / `'portal_badge_text'`.

- [ ] **Step 3: Add the imports, constants, and helpers**

In `crates/app/src/render/map.rs`, extend the existing mapper import line so `Direction` is in scope. The current line is:

```rust
use mapper::router::{RoutedEdge, Side};
```

Add this import beneath it:

```rust
use mapper::direction::Direction;
```

Then add the helpers (place them just above `fn draw_stub`):

```rust
// ── Portal badges ─────────────────────────────────────────────────────────────

/// Portal direction glyphs. Named so a font that renders a variant better is a one-line swap.
const PORTAL_UP: &str = "↑";
const PORTAL_DOWN: &str = "↓";
const PORTAL_IN: &str = "⊙";
const PORTAL_OUT: &str = "⊗";
const PORTAL_UNKNOWN: &str = "?";

/// Max width (cells) of a portal badge in the right gutter, before truncation.
const PORTAL_BADGE_W: usize = 8;

/// Glyph for a non-planar (portal) direction. Shared by the map badge and the dump legend.
pub(crate) fn portal_glyph(dir: Direction) -> &'static str {
    match dir {
        Direction::Up => PORTAL_UP,
        Direction::Down => PORTAL_DOWN,
        Direction::In => PORTAL_IN,
        Direction::Out => PORTAL_OUT,
        _ => PORTAL_UNKNOWN,
    }
}

/// On-map badge text for a portal: glyph + a space + the truncated target name, clipped to
/// `PORTAL_BADGE_W` chars. Unknown portals have no target semantics → just the `?` glyph.
fn portal_badge_text(dir: Direction, dest_label: Option<&str>) -> String {
    if matches!(dir, Direction::Unknown) {
        return PORTAL_UNKNOWN.to_string();
    }
    let glyph = portal_glyph(dir);
    match dest_label {
        Some(name) if !name.is_empty() => {
            format!("{glyph} {name}").chars().take(PORTAL_BADGE_W).collect()
        }
        _ => glyph.to_string(),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app portal_`
Expected: PASS (all four).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): portal glyph + badge-text helpers

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Render stacked portal badges at Boxes zoom (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `fn draw_portal_badge` (new), and change the stub loop in `render_map` (currently ~lines 313-317) to draw badges before the connectors at Boxes zoom while leaving Compact on the existing `draw_stub`.
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `portal_badge_text` (Task 2), `RoutedEdge::dest_label` (Task 1), `VRect`, `put_str`, `CONNECTOR_STYLE`, the `boxes` bool already computed in `render_map`.
- Produces: `fn draw_portal_badge(edge: &RoutedEdge, placed: &HashMap<RoomId, VRect>, stack: u16, off_x: i32, off_y: i32, area: Rect, buf: &mut Buffer)`.

**Note on ordering:** the new badge drawing must move ABOVE the lane-connector block so a connector overwrites a colliding badge cell (Global Constraint: connectors win). The existing `draw_stub` is left untouched and continues to serve Compact zoom unchanged.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn portal_badges_render_glyph_name_and_stack() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.upsert_room(3, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // placed targets (route_all skips unplaced dests)
        g.set_pos(3, (0, 1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom, scroll (0,0): room 1's gutter is on-screen
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let find = |sym: &str| -> Option<(u16, u16)> {
            (0..area.height)
                .flat_map(|y| (0..area.width).map(move |x| (x, y)))
                .find(|&(x, y)| buf.cell((x, y)).map(|c| c.symbol() == sym).unwrap_or(false))
        };
        let up = find("↑").expect("an up-portal badge must render");
        let down = find("↓").expect("a down-portal badge must render");
        // Stacked on successive rows in the same gutter column.
        assert_eq!(up.0, down.0, "stacked portals share the gutter column");
        assert_ne!(up.1, down.1, "stacked portals occupy different rows");
        // The target name follows the glyph and a space: badge "↑ Attic" → 'A' at glyph_col+2.
        let name_start = buf.cell((up.0 + 2, up.1)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(name_start, "A", "target name 'Attic' follows the glyph; got '{name_start}'");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app portal_badges_render_glyph_name_and_stack`
Expected: FAIL — no `↑`/`↓` glyph in the buffer (current `draw_stub` writes the bare `"U"`/`"D"` label).

- [ ] **Step 3: Add `draw_portal_badge`**

In `crates/app/src/render/map.rs`, add this function (place it next to `draw_stub`):

```rust
/// Draw a portal badge (glyph + truncated target name) in the right gutter at Boxes zoom.
/// `stack` is the badge's row offset below the box top, so multiple portals on one room
/// stack down successive gutter rows. Drawn BEFORE the lane connectors so a routed line
/// overwrites a colliding badge cell (the badge is informational and yields).
fn draw_portal_badge(
    edge: &RoutedEdge,
    placed: &std::collections::HashMap<RoomId, VRect>,
    stack: u16,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let Some(&origin_rect) = placed.get(&edge.origin) else {
        return;
    };
    let badge = portal_badge_text(edge.dir, edge.dest_label.as_deref());
    let lx = origin_rect.right() + off_x;
    let ly = origin_rect.y + off_y + stack as i32;
    put_str(buf, lx, ly, &badge, CONNECTOR_STYLE, area);
}
```

- [ ] **Step 4: Rewire the stub loop in `render_map`**

In `render_map`, the current block is:

```rust
    // ── 2. Boxes zoom: draw line-art connectors along their assigned lanes ────
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();
    if let Some((cols, rows)) = &axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf);
    }

    // Stub edges (translate + clip).
    for edge in &rm.edges {
        if edge.is_stub {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }
```

Replace it with (badges first at Boxes zoom; Compact keeps the old `draw_stub`; connectors drawn after so they win):

```rust
    // ── 2. Stub/portal edges. At Boxes zoom render a stacked glyph+name badge in the
    //       right gutter; multiple portals on one room stack down successive rows. Drawn
    //       BEFORE the connectors so a routed line overwrites a colliding badge cell.
    //       Compact zoom keeps its existing bare-label `draw_stub`.
    let mut portal_stack: std::collections::HashMap<RoomId, u16> = std::collections::HashMap::new();
    for edge in &rm.edges {
        if edge.is_stub {
            if boxes {
                let stack = portal_stack.entry(edge.origin).or_insert(0);
                draw_portal_badge(edge, &placed, *stack, off_x, off_y, area, buf);
                *stack += 1;
            } else {
                draw_stub(edge, &placed, off_x, off_y, area, buf);
            }
        }
    }

    // ── 3. Boxes zoom: draw line-art connectors along their assigned lanes (on top of
    //       any portal badges they cross).
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();
    if let Some((cols, rows)) = &axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf);
    }
```

(The later comment markers `// ── 3.` and `// ── 4.` for rooms and arrowheads may be left as-is or renumbered; renumbering is optional and not required for correctness.)

- [ ] **Step 5: Run the new test and the existing render/dump suites**

Run: `cargo test -p app portal_badges_render_glyph_name_and_stack`
Expected: PASS.

Run: `cargo test -p app`
Expected: PASS (all existing render/dump tests still green — Compact path unchanged, connector geometry/overlap tests use compass-only graphs).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): stacked portal badges at Boxes zoom (connectors win)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Portal legend in the map dump (app)

**Files:**
- Modify: `crates/app/src/map_dump.rs` — add a PORTALS legend section in `render_dump` (after the EDGES section, before the MAP section ~line 175), and import `portal_glyph`.
- Test: `crates/app/src/map_dump.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::render::map::portal_glyph` (Task 2), `mapper::direction::grid_offset` (already imported), `MapGraph::room`, `Room::label`, `Connection { origin, dir, dest }`.
- Produces: dump lines of the form `PORTAL <origin> <glyph> #<dest> <name>`, one per non-planar connection.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/map_dump.rs`:

```rust
    #[test]
    fn dump_portal_legend_shows_glyph_id_name() {
        let mut m = Mapper::default();
        m.observe(1, "Cellar", None);
        m.observe(2, "Attic", Some(Direction::Up)); // edge 1 →Up→ 2, both placed
        let dump = render_dump(&m.graph);
        assert!(dump.contains("# === PORTALS"), "portal legend section present:\n{dump}");
        assert!(
            dump.contains("PORTAL 1 ↑ #2 Attic"),
            "portal line shows origin, glyph, target id and full name:\n{dump}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app dump_portal_legend_shows_glyph_id_name`
Expected: FAIL — no `# === PORTALS` section in the dump.

- [ ] **Step 3: Add the import and the legend section**

In `crates/app/src/map_dump.rs`, extend the existing render-module import. The current line is:

```rust
use crate::render::map::{boxes_axes, render_map};
```

Change it to:

```rust
use crate::render::map::{boxes_axes, portal_glyph, render_map};
```

Then, in `render_dump`, immediately AFTER the EDGES loop (the `for c in conns { … EDGE … }` block) and BEFORE the `# === MAP …` section, insert:

```rust
    let portals: Vec<String> = conns
        .iter()
        .filter(|c| grid_offset(c.dir).is_none())
        .map(|c| {
            let name = graph.room(c.dest).map(|r| r.label().to_string()).unwrap_or_default();
            format!("PORTAL {} {} #{} {}", c.origin, portal_glyph(c.dir), c.dest, name)
        })
        .collect();
    if !portals.is_empty() {
        out.push_str("#\n# === PORTALS (origin glyph #target name) ===\n");
        for line in &portals {
            out.push_str(line);
            out.push('\n');
        }
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p app dump_portal_legend_shows_glyph_id_name`
Expected: PASS.

- [ ] **Step 5: Run the full app suite (no regressions)**

Run: `cargo test -p app`
Expected: PASS (existing dump tests — `dump_lists_rooms_edges_and_ids`, `dump_ascii_has_line_art_connector`, etc. — still green; the new section is additive).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/map_dump.rs
git commit -m "feat(app): PORTALS legend in map dump (glyph + #id + name)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (after all tasks)

Run the binary on the A129 story and inspect the dump, confirming:
- Each Up/Down/In/Out connection in `~/.lanthorn/maps/ZCODE-88-840726-A129.map.txt` shows a glyph + target name badge beside its origin box (e.g. `26` shows `↑`-up to `#25`, `27` shows `↓`-down to `#27`'s target), stacked when a room has several.
- The `# === PORTALS` section lists every portal with its full untruncated name.
- Where a connector crosses a badge cell, the connector line is visible (badge yields).
```bash
cargo run -p app -- ~/path/to/ZCODE-88-840726-A129.z5   # then dump the map from the TUI
```

## Notes / out of scope

- `route_all` still skips a stub whose target room is unplaced; rendering portals to off-layer/unplaced targets is deferred to the future multi-layer work (the `dest_label` field is the layer-ready hook).
- Diagonals (Feature 1) and multi-edge merge (Feature 3) are separate plans, written just before their execution.
