# Animate Room Placement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand the `AnimateTidy` diagnostics animation so it replays a graph-build stop and room-by-room incremental placement before the existing SMACOF → cleanup → repair → stack → cleanup stages.

**Architecture:** `run_tidy_pipeline` reconstructs the layout from scratch: it reads the active layer subgraph's connection list (insertion order = discovery order), rebuilds an empty graph, emits one **Build** frame carrying a connection manifest, then emits one **Placement** frame per room by replaying `mapper::layout::place_incremental`. The fully-placed rebuilt graph feeds the unchanged tidy stages; final positions are written back to the live graph as today. A new `manifest` field on `TidyFrame` tells the map renderer to draw the connection list (as text) in the map pane for the Build frame instead of rooms.

**Tech Stack:** Rust, ratatui/crossterm TUI (`app` crate), `mapper` crate (zero-dep layout). Discrete snapshot animation (no tweening).

## Global Constraints

- `zvm`/`gvm` stay zero-dependency; this work is confined to `app` (and reuses existing public `mapper` APIs). No mapper changes are required — `mapper::layout::place_incremental` is already `pub`.
- The animation remains a **discrete frame stepper** (`TidyFrame` snapshots, 700 ms hardcoded dwell). No fractional-position rendering.
- No new config keys, no new style selectors. Manifest text reuses the existing `state.colors.transcript` style.
- Never panic on game-derived state (empty layer, single room, isolated rooms must all be handled).
- The final written-back layout is byte-identical to today for a raw-incremental graph (no prior tidy). Divergence is permitted only after a prior tidy — the existing re-tidy caveat.

---

## File structure

- **`crates/app/src/state.rs`** — add `manifest: Option<Vec<String>>` to `TidyFrame`.
- **`crates/app/src/input.rs`** — new private `replay_build_and_placement` helper; rewire `run_tidy_pipeline` to call it and run stages on the rebuilt graph; set `manifest: None` at every existing `TidyFrame` construction site.
- **`crates/app/src/render/map.rs`** — in `render_map`, draw the current tidy frame's `manifest` (when `Some`) as text in the map pane and skip room drawing.
- **`crates/app/tests/`** — no new integration test file; unit tests live inline in `input.rs` and `render/map.rs` `#[cfg(test)]` modules (existing pattern).
- **`docs/features/mapping.md`** — update the "Animated layout diagnostics" bullet.

---

## Task 1: Add `manifest` field to `TidyFrame`

**Files:**
- Modify: `crates/app/src/state.rs:375-382` (struct), `crates/app/src/state.rs:2521` (test constructor)
- Modify: `crates/app/src/input.rs:1473-1607` (six `TidyFrame` constructors in `run_tidy_pipeline`)

**Interfaces:**
- Produces: `TidyFrame` gains `pub manifest: Option<Vec<String>>`. `None` on every existing frame; `Some(lines)` only on the Build frame (Task 2).

This is a cross-cutting additive field both later tasks depend on. Adding it forces every constructor to set it, so the deliverable is "workspace builds and all existing tidy tests stay green with behavior unchanged (all `None`)."

- [ ] **Step 1: Add the field to the struct**

In `crates/app/src/state.rs`, the `TidyFrame` struct becomes:

```rust
#[derive(Debug, Clone)]
pub struct TidyFrame {
    pub label: String,
    pub graph: MapGraph,
    pub description: String,
    pub stats: mapper::layout::TidyStats,
    pub stage_start: bool,
    /// When `Some`, the map pane renders these lines as text (the Build frame's
    /// connection manifest) instead of drawing rooms. `None` for every layout stage.
    pub manifest: Option<Vec<String>>,
}
```

- [ ] **Step 2: Update the in-crate test constructor**

At `crates/app/src/state.rs:2521` a test builds a `TidyFrame { ... }`. Add `manifest: None,` to it.

- [ ] **Step 3: Update all six constructors in `run_tidy_pipeline`**

In `crates/app/src/input.rs`, each of the six `TidyFrame { ... }` literals (the "before" frame at ~1473, the relayout frame at ~1486, both `cleanup_overlaps` at ~1507 and ~1571, `repair_hints` at ~1530, `stack_updown` at ~1550, `compact` at ~1593) gains `manifest: None,` as the last field.

- [ ] **Step 4: Build and run existing tidy tests**

Run: `cargo build -p app`
Expected: compiles with no errors.

Run: `cargo test -p app tidy`
Expected: all existing tidy tests PASS (behavior unchanged; field defaults to `None`).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/input.rs
git commit -m "feat(map): add manifest field to TidyFrame (default None)"
```

---

## Task 2: Replay build + placement in `run_tidy_pipeline`

**Files:**
- Modify: `crates/app/src/input.rs` — add `replay_build_and_placement`; rewire `run_tidy_pipeline` (`input.rs:1454-1633`).
- Test: inline `#[cfg(test)]` in `crates/app/src/input.rs`.

**Interfaces:**
- Consumes: `TidyFrame { …, manifest }` (Task 1); `mapper::graph::MapGraph` methods `layer_subgraph`, `connections() -> &[Connection]`, `rooms()`, `room(id)`, `upsert_room(id, name)`, `add_edge(origin, dir, dest)`, `set_room_layer(id, layer)`, `set_pos(id, (i32,i32))`; `mapper::layout::place_incremental(&mut MapGraph, prev, dest, dir)`; `mapper::layout::TidyStats::default()`; `Connection { origin, dir, dest, distorted }`.
- Produces: `run_tidy_pipeline` now returns frames ordered `Build×1 → Placement×N → relayout → cleanup → repair → stack → cleanup → compact`, and the rebuilt graph feeds the stages. `replay_build_and_placement(sub: &MapGraph, layer: LayerId, frames: &mut Vec<TidyFrame>, max_frames: usize) -> MapGraph`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/input.rs`:

```rust
#[test]
fn pipeline_prepends_build_and_placement_frames() {
    use mapper::mapper::Mapper; // constructed via Mapper::default() (Auto mode)
    use mapper::direction::Direction;

    // A →N→ B →E→ C, placed incrementally (no tidy yet).
    let mut m = Mapper::default();
    m.observe(1, "Foyer", None);
    m.observe(2, "Hall", Some(Direction::N));
    m.observe(3, "Study", Some(Direction::E));

    let layer = m.graph.layer_of(1);
    let frames = run_tidy_pipeline(&mut m.graph, layer);

    // First frame is the single Build stop, carrying a manifest and no positioned rooms.
    assert_eq!(frames[0].label, "Build");
    let manifest = frames[0].manifest.as_ref().expect("build frame has a manifest");
    assert_eq!(manifest.len(), 2, "one manifest line per connection");
    assert!(frames[0].graph.rooms().all(|r| r.pos.is_none()),
        "no room is positioned during the build stop");
    assert!(frames[0].description.contains("3 rooms"));
    assert!(frames[0].description.contains("2 connections"));

    // Next three frames are Placement, one per room, all with manifest = None.
    assert_eq!(frames[1].label, "Placement");
    assert_eq!(frames[2].label, "Placement");
    assert_eq!(frames[3].label, "Placement");
    assert!(frames[1..4].iter().all(|f| f.manifest.is_none()));

    // The last placement frame has all three rooms positioned.
    assert_eq!(frames[3].graph.rooms().filter(|r| r.pos.is_some()).count(), 3);

    // Existing tidy stages still follow the placement frames (each stage marks a
    // stage_start frame; the "before" frame is gone now).
    assert!(frames.len() > 4);
    assert!(frames[4..].iter().any(|f| f.stage_start));
}

#[test]
fn pipeline_final_positions_match_silent_for_raw_incremental() {
    use mapper::mapper::Mapper; // constructed via Mapper::default() (Auto mode)
    use mapper::direction::Direction;

    let build = || {
        let mut m = Mapper::default();
        m.observe(1, "Foyer", None);
        m.observe(2, "Hall", Some(Direction::N));
        m.observe(3, "Study", Some(Direction::E));
        m.observe(4, "Attic", Some(Direction::N));
        m
    };
    let mut animated = build();
    let mut silent = build();
    let layer = animated.graph.layer_of(1);

    let _ = run_tidy_pipeline(&mut animated.graph, layer);
    tidy_layer_silent(&mut silent.graph, layer);

    for id in [1u16, 2, 3, 4] {
        assert_eq!(
            animated.graph.room(id).unwrap().pos,
            silent.graph.room(id).unwrap().pos,
            "room {id} final position must match the silent (today's) pipeline"
        );
    }
}

#[test]
fn pipeline_single_room_layer() {
    use mapper::mapper::Mapper;
    let mut m = Mapper::default();
    m.observe(1, "Foyer", None);
    let layer = m.graph.layer_of(1);
    let frames = run_tidy_pipeline(&mut m.graph, layer);
    assert_eq!(frames[0].label, "Build");
    assert_eq!(frames[0].manifest.as_ref().unwrap().len(), 0, "no connections");
    assert_eq!(frames[1].label, "Placement");
    assert_eq!(frames[1].graph.room(1).unwrap().pos, Some((0, 0)));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app pipeline_prepends_build_and_placement_frames pipeline_final_positions_match_silent_for_raw_incremental pipeline_single_room_layer`
Expected: FAIL — `frames[0].label` is `"before"`, not `"Build"`; `manifest` field usage may not yet compile if Task 1 was skipped (Task 1 must be complete first).

- [ ] **Step 3: Add the `replay_build_and_placement` helper**

In `crates/app/src/input.rs`, above `run_tidy_pipeline`, add:

```rust
/// Rebuild the layer from scratch by replaying discovery order (the subgraph's
/// connection insertion order), emitting one "Build" frame (with the connection
/// manifest) followed by one "Placement" frame per room. Returns the fully-placed
/// rebuilt graph for the tidy stages to consume. Respects `max_frames`.
fn replay_build_and_placement(
    sub: &mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    frames: &mut Vec<crate::state::TidyFrame>,
    max_frames: usize,
) -> mapper::graph::MapGraph {
    use crate::state::TidyFrame;
    use mapper::graph::{MapGraph, RoomId};
    use mapper::layout::{place_incremental, TidyStats};

    let name_of = |g: &MapGraph, id: RoomId| -> String {
        g.room(id).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{id}"))
    };

    let conns = sub.connections();
    let mut rebuild = MapGraph::new();

    // Placement order: anchor first (origin of the first connection, else the first
    // room), then each room as it first appears in the connection list, then any
    // isolated rooms with no connections at all.
    let anchor: Option<RoomId> =
        conns.first().map(|c| c.origin).or_else(|| sub.rooms().next().map(|r| r.id));
    let mut order: Vec<RoomId> = Vec::new();
    let mut seen: std::collections::BTreeSet<RoomId> = std::collections::BTreeSet::new();
    if let Some(a) = anchor {
        order.push(a);
        seen.insert(a);
    }
    for c in conns {
        for id in [c.origin, c.dest] {
            if seen.insert(id) { order.push(id); }
        }
    }
    for r in sub.rooms() {
        if seen.insert(r.id) { order.push(r.id); }
    }

    // ── Build: construct rooms + edges (no positions) on the same layer. ──
    for &id in &order {
        rebuild.upsert_room(id, name_of(sub, id));
        rebuild.set_room_layer(id, layer);
    }
    for c in conns {
        rebuild.add_edge(c.origin, c.dir, c.dest);
    }
    let manifest: Vec<String> = conns.iter()
        .map(|c| format!("{} \u{2192}{:?}\u{2192} {}", name_of(sub, c.origin), c.dir, name_of(sub, c.dest)))
        .collect();
    if frames.len() < max_frames {
        frames.push(TidyFrame {
            label: "Build".into(),
            graph: rebuild.clone(),
            description: format!("Graph built: {} rooms, {} connections", order.len(), conns.len()),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(manifest),
        });
    }

    // ── Placement: anchor at origin, then place each room in discovery order. ──
    let mut first = true;
    let mut emit = |rebuild: &MapGraph, desc: String, first: &mut bool, frames: &mut Vec<TidyFrame>| {
        if frames.len() < max_frames {
            frames.push(TidyFrame {
                label: "Placement".into(),
                graph: rebuild.clone(),
                description: desc,
                stats: TidyStats::default(),
                stage_start: *first,
                manifest: None,
            });
        }
        *first = false;
    };

    if let Some(a) = anchor {
        rebuild.set_pos(a, (0, 0));
        emit(&rebuild, format!("placed room {} ({}) at origin", a, name_of(sub, a)), &mut first, frames);
    }
    for c in conns {
        if rebuild.room(c.dest).and_then(|r| r.pos).is_some() { continue; } // revisit
        if rebuild.room(c.origin).and_then(|r| r.pos).is_none() { continue; } // defensive
        place_incremental(&mut rebuild, c.origin, c.dest, c.dir);
        let pos = rebuild.room(c.dest).and_then(|r| r.pos).unwrap_or((0, 0));
        emit(&rebuild, format!("placed room {} ({}) {:?} of room {} at ({},{})",
            c.dest, name_of(sub, c.dest), c.dir, c.origin, pos.0, pos.1), &mut first, frames);
    }
    // Isolated rooms (no in-layer connection): place relative to the anchor.
    if let Some(a) = anchor {
        let unplaced: Vec<RoomId> =
            order.iter().copied().filter(|&id| rebuild.room(id).and_then(|r| r.pos).is_none()).collect();
        for id in unplaced {
            place_incremental(&mut rebuild, a, id, mapper::direction::Direction::Unknown);
            let pos = rebuild.room(id).and_then(|r| r.pos).unwrap_or((0, 0));
            emit(&rebuild, format!("placed room {} ({}) at ({},{})",
                id, name_of(sub, id), pos.0, pos.1), &mut first, frames);
        }
    }

    rebuild
}
```

- [ ] **Step 4: Rewire `run_tidy_pipeline` to use the rebuild**

In `crates/app/src/input.rs`, edit `run_tidy_pipeline` (`input.rs:1454-1633`):

1. Promote the frame cap to a module-visible const so the helper shares it. At the top of the function keep `const MAX_TIDY_FRAMES: usize = 2000;`.
2. Replace the subgraph seeding + the "before" frame block (`input.rs:1464-1479`) with:

```rust
    let sub = graph.layer_subgraph(layer);
    let mut frames: Vec<TidyFrame> = Vec::new();

    let mut pipe_overlaps: u32 = 0;
    let mut pipe_hints: u32 = 0;
    let mut pipe_rooms_moved: u32 = 0;
    let mut pipe_constraints: u32 = 0;

    // Build + placement replay produces the front frames and the rebuilt graph
    // that the tidy stages run on.
    let mut sub = replay_build_and_placement(&sub, layer, &mut frames, MAX_TIDY_FRAMES);
```

(Everything below — `relayout_auto_observed(&mut sub, …)` through the write-back loops — is unchanged: it still operates on `sub`, which is now the rebuilt-then-to-be-tidied graph. The write-back at `input.rs:1613-1630` reads `sub.room(id).pos` / `sub.connections()` exactly as before.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app pipeline_prepends_build_and_placement_frames pipeline_final_positions_match_silent_for_raw_incremental pipeline_single_room_layer`
Expected: all three PASS.

Run: `cargo test -p app tidy`
Expected: existing tidy tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/input.rs
git commit -m "feat(map): replay graph build + room placement in AnimateTidy pipeline"
```

---

## Task 3: Render the Build-frame manifest in the map pane

**Files:**
- Modify: `crates/app/src/render/map.rs` — `render_map` (`map.rs:435`).
- Test: inline `#[cfg(test)]` in `crates/app/src/render/map.rs`.

**Interfaces:**
- Consumes: `state.tidy_anim: Option<TidyAnim>` with `TidyAnim::current() -> &TidyFrame` and `TidyFrame.manifest`; `state.colors.transcript: Style`; `put_str`.
- Produces: when the active tidy frame has a manifest, `render_map` draws those lines in the map pane and skips room drawing.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/render/map.rs`:

```rust
#[test]
fn build_frame_manifest_drawn_in_map_pane() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use crate::state::{AppState, TidyAnim, TidyFrame};
    use mapper::graph::MapGraph;
    use mapper::layout::TidyStats;

    let mut state = AppState::default();
    state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
        label: "Build".into(),
        graph: MapGraph::new(),
        description: "Graph built: 2 rooms, 1 connections".into(),
        stats: TidyStats::default(),
        stage_start: true,
        manifest: Some(vec!["Foyer \u{2192}N\u{2192} Hall".into()]),
    }]));

    // Empty render map, built with the same helper the neighboring tests use.
    let rm = mapper::render::render(&MapGraph::new());
    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(text.contains("Foyer"), "manifest line should be drawn in the map pane");
    assert!(text.contains("Hall"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app build_frame_manifest_drawn_in_map_pane`
Expected: FAIL — the buffer is blank; "Foyer" is not present.

- [ ] **Step 3: Draw the manifest at the top of `render_map`**

In `crates/app/src/render/map.rs`, at the very top of `render_map` (right after `let zoom = …; let scroll = …;` at `map.rs:436-437`), add:

```rust
    // Build-frame manifest: when the active tidy frame carries a manifest, draw it
    // as text in the map pane and skip room drawing. Overflow past the pane is
    // truncated (diagnostic view).
    if let Some(anim) = &state.tidy_anim {
        if let Some(lines) = anim.current().manifest.as_ref() {
            for (i, line) in lines.iter().take(area.height as usize).enumerate() {
                let clamped: String = line.chars().take(area.width as usize).collect();
                put_str(buf, area.x as i32, area.y as i32 + i as i32, &clamped,
                    state.colors.transcript, area);
            }
            return;
        }
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p app build_frame_manifest_drawn_in_map_pane`
Expected: PASS.

Run: `cargo test -p app render::map`
Expected: existing map render tests still PASS (normal frames have `manifest = None`, so the new branch is skipped).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(map): render Build-frame connection manifest in map pane"
```

---

## Task 4: Document the expanded animation

**Files:**
- Modify: `docs/features/mapping.md` (the "Animated layout diagnostics" bullet).

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the diagnostics bullet**

In `docs/features/mapping.md`, replace the "Animated layout diagnostics" bullet with:

```markdown
- **Animated layout diagnostics** — step through the whole layout build stage by
  stage: a **Build** stop listing every connection, then **room-by-room placement**
  as each room drops onto the grid, then the relayout/overlap-cleanup passes — each
  move described ("moved 180 to clear overlap with 193") — to see and debug exactly
  how the map is assembled.
```

- [ ] **Step 2: Verify the workspace still builds and full app suite passes**

Run: `cargo test -p app`
Expected: all tests PASS.

- [ ] **Step 3: Commit**

```bash
git add docs/features/mapping.md
git commit -m "docs(mapping): note build + placement in the layout diagnostics animation"
```

---

## Notes for the implementer

- **Discovery-order faithfulness:** the anchor is the origin of the first connection; for a non-main layer this may not be the layer's true origin cell, but the tidy stages re-center and `AnimateTidy` already re-derives the layout, so an absolute `(0,0)` anchor is correct for the animation.
- **Intended divergence:** `run_tidy_pipeline` (animated) reconstructs from scratch while `tidy_layer_silent` (background) works from live positions. They agree for a raw-incremental graph (asserted in Task 2) and may differ after a prior tidy — this is the accepted re-tidy caveat from the spec, not a bug.
- **Frame cap:** `replay_build_and_placement` honors `MAX_TIDY_FRAMES`; on a huge layer the placement frames truncate before the tidy stages, same silent-truncation behavior the pipeline already had.
