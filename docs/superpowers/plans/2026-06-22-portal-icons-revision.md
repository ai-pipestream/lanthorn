# Portal Icons + Destination Toggle — Implementation Plan (Feature 2 revision)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the right-gutter portal name-badges (too narrow for names) with directional portal icons drawn inside the room box, plus a `Ctrl+P` hotkey that toggles each portal's destination name on its icon row.

**Architecture:** A post-room overlay (`draw_portal_icons`) draws a direction glyph in the room's right interior column — `↑` row 1, `⊙`/`⊗`/`?` row 2, `↓` row 3 — at Boxes zoom. A new `show_portal_labels` AppState flag (toggled by `Ctrl+P`, mirroring the existing `Ctrl+A` alignment toggle) makes the overlay additionally render each portal's destination name right-aligned beside its icon. The mapper `dest_label`, the `portal_glyph` constants, and the dump PORTALS legend from the first pass are reused unchanged; only the gutter-badge rendering is removed.

**Tech Stack:** Rust workspace (`mapper`, `app` crates), ratatui 0.29 TUI.

## Global Constraints

- **Icon slots (Boxes zoom), right interior column `col = BOX_W-2` (= 9):** `↑` Up → row 1; `⊙` In / `⊗` Out / `?` Unknown → row 2 (middle); `↓` Down → row 3. Glyphs come from the existing `portal_glyph`.
- **Mid-slot precedence** when a room has more than one of In/Out/Unknown (lower wins): **In ▸ Out ▸ Unknown**. The dump still lists every portal.
- **`Ctrl+P` toggles `show_portal_labels`** (default false), wired exactly like `Ctrl+A`/`show_alignment`. When on, each portal's destination name renders right-aligned on its icon row (icon pinned far-right), name truncated to fit; the **full untruncated name is always in the Ctrl+D dump**.
- **Notes marker `●`:** in the default (icons-only) view, an up-portal claims the upper-right corner and the `●` shifts one interior cell left.
- **Boxes zoom only.** Compact keeps its existing bare-label `draw_stub`; Overview is unchanged. The dump PORTALS legend is unchanged.
- Determinism: identical graph + flag state → identical render.
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: In-room portal icons (replace gutter badges)

**Files:**
- Modify: `crates/app/src/render/map.rs` — remove the first-pass gutter-badge code; add `portal_slot`, `mid_precedence`, `draw_portal_icons`; rewire `render_map`.
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `RoutedEdge { is_stub, dir, origin, dest_label }`, `portal_glyph(dir) -> &'static str`, `room_style(room, state) -> Style`, `RenderMap`, `RenderRoom { id, has_notes }`, `VRect`, `BOX_W`, `put_str`, `put_char`.
- Produces: `fn draw_portal_icons(rm: &RenderMap, placed: &HashMap<RoomId, VRect>, state: &AppState, show_labels: bool, off_x: i32, off_y: i32, area: Rect, buf: &mut Buffer)`. (Task 2 flips the `show_labels` argument from the literal `false` to `state.show_portal_labels`.)

**Removals (first-pass code now superseded):** delete `fn draw_portal_badge`, `const PORTAL_BADGE_W`, `fn portal_badge_text`, and these tests: `portal_badge_truncates_to_gutter_width`, `portal_badge_short_name_not_padded`, `portal_badge_unknown_is_just_glyph`, `portal_badges_render_glyph_name_and_stack`. KEEP `portal_glyph` and its test `portal_glyphs_map_directions`. KEEP `draw_stub` (Compact zoom).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn portal_icons_render_in_room_slots() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.upsert_room(3, "Cellar".into());
        g.upsert_room(4, "Vault".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // placed portal targets (route_all skips unplaced dests)
        g.set_pos(3, (0, 1));
        g.set_pos(4, (1, 0));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(1, Direction::In, 4);
        let rm = render(&g);
        let state = AppState::default(); // Boxes, scroll (0,0), labels off
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Box of room 1 is at screen (0,0); right interior column is col 9 (BOX_W-2).
        assert_eq!(sym(9, 1), "↑", "up icon in upper-right interior (row 1)");
        assert_eq!(sym(9, 2), "⊙", "in icon in middle-right interior (row 2)");
        assert_eq!(sym(9, 3), "↓", "down icon in lower-right interior (row 3)");
    }

    #[test]
    fn portal_icon_up_shifts_notes_marker() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.set_notes(1, "stuff".into());
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym(9, 1), "↑", "up icon claims the upper-right corner");
        assert_eq!(sym(8, 1), "●", "notes marker shifts one cell left of the up icon");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app portal_icons_render_in_room_slots portal_icon_up_shifts_notes_marker`
Expected: FAIL (icons not drawn — gutter badges render instead).

- [ ] **Step 3: Remove the superseded gutter-badge code**

In `crates/app/src/render/map.rs`:
- Delete the function `draw_portal_badge`.
- Delete `const PORTAL_BADGE_W: usize = 8;` and the function `portal_badge_text`.
- Delete the four superseded tests named in the **Removals** note above (keep `portal_glyphs_map_directions`).

- [ ] **Step 4: Add the slot helpers + the overlay function**

Add near the other rendering helpers in `crates/app/src/render/map.rs`:

```rust
/// In-room icon slot for a portal direction: 0 = row 1 (Up), 1 = row 2 (mid: In/Out/Unknown),
/// 2 = row 3 (Down). Cardinal directions have no portal slot.
fn portal_slot(dir: Direction) -> Option<usize> {
    match dir {
        Direction::Up => Some(0),
        Direction::Down => Some(2),
        Direction::In | Direction::Out | Direction::Unknown => Some(1),
        _ => None,
    }
}

/// Mid-slot precedence when a room has several of In/Out/Unknown (lower wins): In ▸ Out ▸ Unknown.
fn mid_precedence(dir: Direction) -> u8 {
    match dir {
        Direction::In => 0,
        Direction::Out => 1,
        _ => 2, // Unknown
    }
}

/// Draw in-room portal indicators at Boxes zoom as a post-room overlay (so icons sit on top of
/// the box interior). Each room's portal (stub) edges map to a right-interior-column slot:
/// Up→row 1, In/Out/Unknown→row 2 (middle, by `mid_precedence`), Down→row 3. Default = the
/// direction glyph in that slot's far-right interior cell. When `show_labels` is set, the
/// portal's destination name is drawn right-aligned on that row with the icon pinned far-right.
/// In the default view an up-portal claims the upper-right corner, shifting the `●` notes marker
/// one cell left so both stay visible.
fn draw_portal_icons(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    state: &AppState,
    show_labels: bool,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    use std::collections::HashMap;
    // Per room, the chosen (glyph, dest_label) for each of the 3 slots; mid slot by precedence.
    let mut chosen: HashMap<RoomId, [Option<(&str, Option<&str>)>; 3]> = HashMap::new();
    let mut mid_rank: HashMap<RoomId, u8> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        let Some(slot) = portal_slot(edge.dir) else { continue };
        let glyph = portal_glyph(edge.dir);
        let label = edge.dest_label.as_deref();
        let slots = chosen.entry(edge.origin).or_insert([None, None, None]);
        if slot == 1 {
            let rank = mid_precedence(edge.dir);
            let cur = mid_rank.entry(edge.origin).or_insert(u8::MAX);
            if rank < *cur {
                *cur = rank;
                slots[1] = Some((glyph, label));
            }
        } else if slots[slot].is_none() {
            slots[slot] = Some((glyph, label));
        }
    }

    let icon_col = (BOX_W - 2) as i32; // far-right interior column
    for room in &rm.rooms {
        let Some(slots) = chosen.get(&room.id) else { continue };
        let Some(&rect) = placed.get(&room.id) else { continue };
        let style = room_style(room, state);
        for (slot, cell) in slots.iter().enumerate() {
            let Some((glyph, label)) = cell else { continue };
            let row = rect.y + 1 + slot as i32;
            if show_labels {
                if let Some(name) = label {
                    let n: String = name.chars().take(7).collect();
                    let text = format!("{n:>7} {glyph}");
                    put_str(buf, rect.x + 1 + off_x, row + off_y, &text, style, area);
                    continue;
                }
            }
            put_str(buf, rect.x + icon_col + off_x, row + off_y, glyph, style, area);
            if slot == 0 && room.has_notes {
                put_char(buf, rect.x + icon_col - 1 + off_x, row + off_y, '●', style, area);
            }
        }
    }
}
```

- [ ] **Step 5: Rewire `render_map`**

Replace the first-pass portal-badge loop (the block that builds `portal_stack` and calls `draw_portal_badge`/`draw_stub`) with a Compact-only stub loop:

```rust
    // Stub (portal) edges at non-Boxes zoom keep the bare-label `draw_stub`; Boxes zoom draws
    // the in-room portal-icon overlay after the rooms (below).
    for edge in &rm.edges {
        if edge.is_stub && !boxes {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }
```

Then, AFTER the room-drawing loop (the `for room in &rm.rooms { … draw_room … }` block, including its alignment overlay) and before/around the `draw_connector_arrows` call, add:

```rust
    // Portal-icon overlay (Boxes zoom): directional icons on the right interior column.
    // Drawn after the rooms so icons sit on the box interior. (Task 2 turns the `false`
    // into `state.show_portal_labels` to render destination names.)
    if boxes {
        draw_portal_icons(rm, &placed, state, false, off_x, off_y, area, buf);
    }
```

- [ ] **Step 6: Run the new tests, then the full app suite**

Run: `cargo test -p app portal_icons_render_in_room_slots portal_icon_up_shifts_notes_marker`
Expected: PASS.

Run: `cargo test -p app`
Expected: PASS (the four removed tests are gone; everything else green; Compact/dump unaffected).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): in-room portal icons replace gutter badges

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Ctrl+P toggles portal destination names

**Files:**
- Modify: `crates/app/src/state.rs` — add `show_portal_labels: bool` (default false).
- Modify: `crates/app/src/input.rs` — add `Action::TogglePortalLabels`, map `Ctrl+P`, flip the flag in `apply_action`.
- Modify: `crates/app/src/render/map.rs` — pass `state.show_portal_labels` into `draw_portal_icons`.
- Modify: `crates/app/src/main.rs` — add `Ctrl+P: portals` to the Boxes/map help bar string.
- Test: `crates/app/src/input.rs` and `crates/app/src/render/map.rs`.

**Interfaces:**
- Consumes: the existing `Ctrl+A`/`ToggleAlignment`/`show_alignment` wiring as the exact pattern to mirror; `draw_portal_icons` (Task 1).
- Produces: `AppState.show_portal_labels: bool`; `Action::TogglePortalLabels`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/input.rs` tests (mirror the existing `ToggleAlignment` test — find it for the exact `ctrl(...)` helper and `apply_action(action, &mut s, &mut m)` setup):

```rust
    #[test]
    fn ctrl_p_toggles_portal_labels() {
        let s = AppState::default();
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::TogglePortalLabels
        ));
        let mut s = AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        assert!(!s.show_portal_labels, "default off");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(s.show_portal_labels, "Ctrl+P turns labels on");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(!s.show_portal_labels, "Ctrl+P toggles back off");
    }
```

(If the existing alignment test constructs the `Mapper` differently, match that construction exactly.)

In `crates/app/src/render/map.rs` tests:

```rust
    #[test]
    fn portal_labels_show_destination_when_toggled() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let area = Rect::new(0, 0, 80, 40);
        let row1 = |show: bool| -> String {
            let mut state = AppState::default();
            state.show_portal_labels = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &state, area, &mut buf);
            (1u16..=9)
                .map(|x| buf.cell((x, 1)).map(|c| c.symbol().to_string()).unwrap_or_default())
                .collect()
        };
        let on = row1(true);
        let off = row1(false);
        assert!(on.contains("Attic"), "toggled on: up-portal destination on row 1; got '{on}'");
        assert!(on.ends_with("↑"), "icon stays pinned at the far-right cell; got '{on}'");
        assert!(!off.contains("Attic"), "toggled off: no destination name; got '{off}'");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app ctrl_p_toggles_portal_labels portal_labels_show_destination_when_toggled`
Expected: FAIL — no `show_portal_labels` field / no `TogglePortalLabels` action.

- [ ] **Step 3: Add the state flag**

In `crates/app/src/state.rs`, mirror `show_alignment`: add the field with a doc comment and default `false`:

```rust
    /// When true, portal icons additionally show their destination room name (Boxes zoom only).
    /// Toggled by `Ctrl+P`.
    pub show_portal_labels: bool,
```

Add `show_portal_labels: false,` to the `Default`/constructor where `show_alignment: false` is set.

- [ ] **Step 4: Add the action, key mapping, and handler**

In `crates/app/src/input.rs`:
- Add `TogglePortalLabels,` to the `Action` enum (next to `ToggleAlignment`).
- In `key_to_action`, in the same `ctrl` match arm group as `KeyCode::Char('a') => Action::ToggleAlignment`, add:

```rust
            KeyCode::Char('p') => Action::TogglePortalLabels,
```

- In `apply_action`, next to the `ToggleAlignment` arm, add:

```rust
        Action::TogglePortalLabels => state.show_portal_labels = !state.show_portal_labels,
```

- [ ] **Step 5: Wire the flag into the renderer**

In `crates/app/src/render/map.rs` `render_map`, change the Task 1 call:

```rust
    if boxes {
        draw_portal_icons(rm, &placed, state, state.show_portal_labels, off_x, off_y, area, buf);
    }
```

- [ ] **Step 6: Update the help bar**

In `crates/app/src/main.rs`, in the map/Boxes help-bar string (the one already containing `Ctrl+A: align`), add `Ctrl+P: portals` (place it next to `Ctrl+A: align`).

- [ ] **Step 7: Run the new tests, then the full app suite**

Run: `cargo test -p app ctrl_p_toggles_portal_labels portal_labels_show_destination_when_toggled`
Expected: PASS.

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/input.rs crates/app/src/render/map.rs crates/app/src/main.rs
git commit -m "feat(app): Ctrl+P toggles portal destination names

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (after both tasks)

Run lanthorn on the A129 story, Boxes zoom:
- Each room with a portal shows the direction icon in its slot (↑ upper-right, ↓ lower-right, ⊙/⊗/? middle-right). Rooms with notes still show `●` (shifted left where an up-portal shares the corner).
- `Ctrl+P` reveals destination names right-aligned beside the icons; pressing again hides them. Full names remain in the Ctrl+D dump.
- Compact zoom still shows the old bare letter labels; Overview unchanged.

## Notes / out of scope

- A room with both an In and an Out portal shows only the In icon in the middle slot (precedence); both still appear in the dump. Same for multiple portals in one slot.
- If `Ctrl+A` (alignment) and `Ctrl+P` are both on, a down-portal label and the row-3 alignment code contend for row 3; the portal overlay draws last and wins its cells. Acceptable; revisit only if it reads badly.
