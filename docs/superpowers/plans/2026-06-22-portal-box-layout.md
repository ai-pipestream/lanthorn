# Portal Box Layout — Implementation Plan (Feature 2 refinement)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Center and word-wrap the room name across the box's first two interior rows, move `#id`+alignment diagnostics to row 3, and in portal view (Ctrl+P) move portal icons onto the box border with destination names floating outside the box (suppressing the cardinal connector arrowheads).

**Architecture:** `draw_box_room` gains word-wrap + centering and composes row 3 (`#id` + optional align code) itself, so the separate alignment overlay in `render_map` is removed. `draw_portal_icons` branches: normal view keeps the interior right-column icons; portal view draws border icons (top/bottom/right) and floating destination names. `render_map` suppresses `draw_connector_arrows` while portal labels are shown.

**Tech Stack:** Rust workspace (`app` crate), ratatui 0.29 TUI. All changes are in `crates/app/src/render/map.rs`.

## Global Constraints

- **Box interior (both views), centered:** room name word-wrapped across interior rows 1–2 (each line centered in the 9-col interior); **row 3 = `#id`** centered, with **align diagnostics appended after it** (only when `Ctrl+A`/`show_alignment` is on).
- **Normal view (Ctrl+P off):** portal icons in the interior right column (`col = BOX_W-2`): `↑` Up→row 1, `⊙`/`⊗`/`?` mid→row 2 (precedence In ▸ Out ▸ Unknown), `↓` Down→row 3. Connector arrowheads draw normally.
- **Portal view (Ctrl+P on):** icons move onto the **border** — `↑` top-border centre, `↓` bottom-border centre, mid on the **right** border (middle row). Destination names float **outside**: up-dest above, down-dest below, in/out-dest to the right. **Connector arrowheads are NOT drawn.** Destination names are untruncated (overflow/overwrite allowed).
- **Unknown (`?`)** never shows a destination name in either view (only the glyph).
- **Notes `●`** (normal view): shifts one cell left when an up-portal claims the upper-right interior cell.
- Boxes zoom only; Compact/Overview unchanged. Determinism preserved.
- Commit messages end with: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Centered, word-wrapped box interior (name rows 1–2; #id+align row 3)

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `wrap_two`/`center` helpers; rework `draw_box_room`; thread `show_alignment` through `draw_room`; remove the alignment-overlay block in `render_map`.
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `RenderRoom { label, id, align_code }`, `AppState.show_alignment`, `put_str`, `put_char`, `zoom_box_size`.
- Produces: `fn wrap_two(s: &str, width: usize) -> [String; 2]`, `fn center(s: &str, width: usize) -> String`; `draw_box_room(room, sx, sy, style, show_alignment: bool, area, buf)` (new `show_alignment` param).

**Existing tests that MUST be updated (the layout they assert changes):**
- `room_box_shows_id` — `#id` moves from row 2 to row 3; change its assertion to read row 3 (`sy+3` → screen row index 3 for a box at (0,0)).
- `alignment_overlay_off_by_default_then_shows_code` — should still pass (the align code still appears only when on); confirm it stays green after the overlay move into `draw_box_room`.
- `room_box_shows_label_at_boxes_zoom` — name is now wrapped+centered; the existing `contains("West")` assertion still holds (row 1 = centered "West of"). Confirm green.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn box_name_wraps_centered_and_id_on_row3() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(7, "Rocky Ledge".into());
        g.set_pos(7, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes, align off
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let row = |y: u16| -> String {
            (0..11u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default()).collect()
        };
        // Name word-wraps across rows 1 and 2.
        assert!(row(1).contains("Rocky"), "row 1 has the first word: '{}'", row(1));
        assert!(row(2).contains("Ledge"), "row 2 has the second word: '{}'", row(2));
        // #id is on row 3 (moved off row 2).
        assert!(row(3).contains("#7"), "row 3 shows the id: '{}'", row(3));
        assert!(!row(2).contains("#7"), "id is no longer on row 2: '{}'", row(2));
        // Centered: a leading pad space after the left border on the name + id rows.
        assert!(row(1).starts_with("│ "), "name centered (leading pad): '{}'", row(1));
        assert!(row(3).starts_with("│ "), "id centered (leading pad): '{}'", row(3));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app box_name_wraps_centered_and_id_on_row3`
Expected: FAIL (name not wrapped/centered; `#7` still on row 2).

- [ ] **Step 3: Add the `wrap_two` and `center` helpers**

Add near the other helpers in `crates/app/src/render/map.rs` (e.g. just above `draw_box_room`):

```rust
/// Word-wrap `s` into up to two lines no wider than `width` (break on spaces; a single
/// over-long word, or overflow past two lines, is truncated to `width`).
fn wrap_two(s: &str, width: usize) -> [String; 2] {
    let mut lines = [String::new(), String::new()];
    let mut idx = 0;
    for word in s.split_whitespace() {
        if idx >= 2 {
            break;
        }
        if lines[idx].is_empty() {
            lines[idx] = word.chars().take(width).collect();
        } else if lines[idx].chars().count() + 1 + word.chars().count() <= width {
            lines[idx].push(' ');
            lines[idx].push_str(word);
        } else {
            idx += 1;
            if idx < 2 {
                lines[idx] = word.chars().take(width).collect();
            }
        }
    }
    lines
}

/// Center `s` within `width` columns (truncated to `width` if longer).
fn center(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.chars().take(width).collect();
    }
    let pad = width - len;
    let left = pad / 2;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(pad - left))
}
```

- [ ] **Step 4: Rework `draw_box_room`**

Change the signature to add `show_alignment: bool`:

```rust
fn draw_box_room(
    room: &RenderRoom,
    sx: i32,
    sy: i32,
    style: Style,
    show_alignment: bool,
    area: Rect,
    buf: &mut Buffer,
) {
```

Replace the label block + the `if h > 3 { #id on row 2 }` block (the current lines that draw the label on row 1 and the id on row 2) with:

```rust
    // Room name word-wrapped + centered across the first two interior rows.
    let iw = (w - 2) as usize; // interior width (9)
    let name_lines = wrap_two(&room.label, iw);
    put_str(buf, sx + 1, sy + 1, &center(&name_lines[0], iw), style, area);
    put_str(buf, sx + 1, sy + 2, &center(&name_lines[1], iw), style, area);

    // Row 3: #id (centered), with alignment diagnostics appended when enabled.
    let mut row3 = format!("#{}", room.id);
    if show_alignment && !room.align_code.is_empty() {
        row3.push(' ');
        row3.push_str(&room.align_code);
    }
    put_str(buf, sx + 1, sy + 3, &center(&row3, iw), style, area);
```

Leave the interior-fill loop, the `●` notes-marker block, and the borders unchanged.

- [ ] **Step 5: Thread `show_alignment` through `draw_room`**

In `draw_room`, change the Boxes arm to pass the flag:

```rust
        Zoom::Boxes => {
            draw_box_room(room, sx, sy, base_style, state.show_alignment, area, buf);
        }
```

- [ ] **Step 6: Remove the alignment overlay in `render_map`**

In `render_map`, the room-drawing loop currently is:

```rust
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        draw_room(room, state, zoom, sx, sy, area, buf);
        // Alignment overlay: Boxes zoom only, when enabled and the room is in a chain.
        if boxes && state.show_alignment && !room.align_code.is_empty() {
            let code: String = room.align_code.chars().take(9).collect();
            put_str(buf, sx + 1, sy + 3, &code, room_style(room, state), area);
        }
    }
```

Remove the alignment-overlay `if` block (now handled inside `draw_box_room`), leaving:

```rust
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        draw_room(room, state, zoom, sx, sy, area, buf);
    }
```

(If `boxes` becomes unused after this removal, leave it — it is still used by the portal-overlay guard below it. Do not remove it.)

- [ ] **Step 7: Update `room_box_shows_id`**

The id moved from row 2 to row 3. Update that test's assertion to read screen row 3 instead of row 2 (a box at (0,0) → the id row is `y = 3`). Keep the rest of the test as-is.

- [ ] **Step 8: Run the new test + the full app suite**

Run: `cargo test -p app box_name_wraps_centered_and_id_on_row3 room_box_shows_id room_box_shows_label_at_boxes_zoom alignment_overlay_off_by_default_then_shows_code`
Expected: PASS (all four).

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): centered word-wrapped room name; #id + align on row 3

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Portal view — border icons, floating destinations, suppressed arrows

**Files:**
- Modify: `crates/app/src/render/map.rs` — rework the per-room loop in `draw_portal_icons` to branch on `show_labels`; gate `draw_connector_arrows` in `render_map`.
- Test: `crates/app/src/render/map.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: the existing `chosen` slot map in `draw_portal_icons` (`[Option<(&str, Option<&str>)>; 3]`, slot 0=Up, 1=mid, 2=Down), `BOX_W`, `BOX_H`, `PORTAL_UNKNOWN`, `put_str`, `put_char`, `room_style`.
- Produces: no new public items; `draw_portal_icons` now renders border icons + floating names when `show_labels`.

**Existing tests that MUST change (they assert the OLD beside-icon labels-on behavior):**
- REMOVE `portal_labels_show_destination_when_toggled` (superseded — labels no longer render beside the interior icon).
- REPLACE `unknown_portal_shows_no_destination_name` with `unknown_portal_in_portal_view_is_border_glyph_no_name` (below) — in portal view the `?` is on the right border, still nameless.
- KEEP unchanged (normal-view behavior is unchanged): `portal_icons_render_in_room_slots`, `portal_icon_up_shifts_notes_marker`, `portal_mid_slot_in_beats_out`, `portal_glyphs_map_directions`.

- [ ] **Step 1: Write the failing tests**

Remove `portal_labels_show_destination_when_toggled` and `unknown_portal_shows_no_destination_name`, then add:

```rust
    #[test]
    fn portal_view_moves_icons_to_border_and_floats_destinations() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Mid".into());    // portal owner
        g.upsert_room(2, "Attic".into());  // up target
        g.upsert_room(3, "Cellar".into()); // down target
        g.set_pos(1, (0, 1));
        g.set_pos(2, (0, 0));
        g.set_pos(3, (0, 2));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.show_portal_labels = true;
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(0) - off.0;
        let by = rows.room_pixel(1) - off.1;
        let sym = |x: i32, y: i32| buf.cell((x as u16, y as u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Icons sit on the border (top/bottom centre), not the interior right column.
        assert_eq!(sym(bx + BOX_W / 2, by), "↑", "up icon on the top border centre");
        assert_eq!(sym(bx + BOX_W / 2, by + BOX_H - 1), "↓", "down icon on the bottom border centre");
        // Destinations float above / below the box.
        let above: String = (0..area.width).map(|x| sym(x as i32, by - 1)).collect();
        let below: String = (0..area.width).map(|x| sym(x as i32, by + BOX_H)).collect();
        assert!(above.contains("Attic"), "up destination floats above; got '{above}'");
        assert!(below.contains("Cellar"), "down destination floats below; got '{below}'");
        // The interior right-column icon is gone in portal view.
        assert_ne!(sym(bx + BOX_W - 2, by + 1), "↑", "icons leave the interior in portal view");
    }

    #[test]
    fn unknown_portal_in_portal_view_is_border_glyph_no_name() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "West of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Unknown, 2);
        let rm = render(&g);
        let mut state = AppState::default();
        state.show_portal_labels = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // ? sits on the RIGHT border (col BOX_W-1) at the middle row (row 2). Box is at (0,0).
        assert_eq!(sym((BOX_W - 1) as u16, 2), "?", "unknown portal shows ? on the right border");
        // No destination name to the right of the box on that row.
        let right: String = ((BOX_W as u16)..40).map(|x| sym(x, 2)).collect();
        assert!(!right.contains("West"), "unknown portal shows no destination name; got '{right}'");
    }

    #[test]
    fn portal_view_suppresses_connector_arrows() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = render(&g);
        let area = Rect::new(0, 0, 80, 30);
        let count_arrows = |show: bool| -> usize {
            let mut st = AppState::default();
            st.show_portal_labels = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf.content.iter().filter(|c| matches!(c.symbol(), "▶" | "◀" | "▲" | "▼")).count()
        };
        assert!(count_arrows(false) > 0, "normal view draws connector arrowheads");
        assert_eq!(count_arrows(true), 0, "portal view suppresses connector arrowheads");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app portal_view_moves_icons_to_border_and_floats_destinations portal_view_suppresses_connector_arrows unknown_portal_in_portal_view_is_border_glyph_no_name`
Expected: FAIL (icons still in interior; arrows still drawn).

- [ ] **Step 3: Rework the per-room loop in `draw_portal_icons`**

Replace the current per-room loop body (the `for room in &rm.rooms { … }` block that draws interior icons / the right-aligned-label branch) with a branch on `show_labels`:

```rust
    let icon_col = BOX_W - 2; // far-right interior column (normal view)
    for room in &rm.rooms {
        let Some(slots) = chosen.get(&room.id) else { continue };
        let Some(&rect) = placed.get(&room.id) else { continue };
        let style = room_style(room, state);
        let (bx, by) = (rect.x, rect.y);
        if show_labels {
            // Portal view: icons move onto the border; destination names float OUTSIDE the box.
            if let Some((glyph, label)) = slots[0] {
                put_str(buf, bx + BOX_W / 2 + off_x, by + off_y, glyph, style, area); // top border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by - 1 + off_y, name, style, area); // above
                }
            }
            if let Some((glyph, label)) = slots[2] {
                put_str(buf, bx + BOX_W / 2 + off_x, by + BOX_H - 1 + off_y, glyph, style, area); // bottom border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by + BOX_H + off_y, name, style, area); // below
                }
            }
            if let Some((glyph, label)) = slots[1] {
                put_str(buf, bx + BOX_W - 1 + off_x, by + 2 + off_y, glyph, style, area); // right border
                // Unknown has no target semantics → glyph only, no floating name.
                if glyph != PORTAL_UNKNOWN {
                    if let Some(name) = label {
                        put_str(buf, bx + BOX_W + off_x, by + 2 + off_y, name, style, area); // right
                    }
                }
            }
        } else {
            // Normal view: directional icons in the interior right column.
            for (slot, cell) in slots.iter().enumerate() {
                let Some((glyph, _label)) = cell else { continue };
                let row = by + 1 + slot as i32;
                put_str(buf, bx + icon_col + off_x, row + off_y, glyph, style, area);
                if slot == 0 && room.has_notes {
                    put_char(buf, bx + icon_col - 1 + off_x, row + off_y, '●', style, area);
                }
            }
        }
    }
```

Note: in the `show_labels` branch `slots[0]`/`slots[1]`/`slots[2]` are copied out (the tuple is `Copy`), so `glyph` is `&str` here — compare with `glyph != PORTAL_UNKNOWN` (no deref). In the `else` branch `cell` is a reference from `.iter()`, so `glyph` is `&&str` and is passed directly to `put_str` via deref coercion (as today).

- [ ] **Step 4: Suppress connector arrowheads in portal view**

In `render_map`, change the final arrow-drawing call:

```rust
    draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf);
```

to:

```rust
    // Portal view hides the cardinal connector arrowheads so only portal icons sit on borders.
    if !state.show_portal_labels {
        draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf);
    }
```

- [ ] **Step 5: Run the new tests + the full app suite**

Run: `cargo test -p app portal_view_moves_icons_to_border_and_floats_destinations portal_view_suppresses_connector_arrows unknown_portal_in_portal_view_is_border_glyph_no_name portal_icons_render_in_room_slots portal_icon_up_shifts_notes_marker portal_mid_slot_in_beats_out`
Expected: PASS (new + retained normal-view tests).

Run: `cargo test -p app`
Expected: PASS.

- [ ] **Step 6: Confirm clippy is clean on the lib**

Run: `cargo clippy -p app --lib`
Expected: no new warnings in `render/map.rs` (the project keeps the lib prod code warning-free).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): portal view moves icons to the border, floats destinations, hides connector arrows

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Manual verification (after both tasks)

Run lanthorn on the A129 story, Boxes zoom:
- Normal view: room names are centered and wrap across two rows; `#id` is centered on row 3 (with the align code appended when Ctrl+A is on); portal icons sit on the interior right edge; connector arrows show as usual.
- Ctrl+P: icons jump to the border (↑ top, ↓ bottom, mid right); destination names appear above/below/right of the box (overwriting neighbours/paths is fine); `?` shows no name; the cardinal arrowheads disappear. Ctrl+P again restores the normal view.

## Notes / out of scope

- Long single words or names exceeding two 9-wide lines are truncated (acceptable for room labels).
- A name line that fills all 9 interior columns loses its last char under a normal-view right-edge icon (rare; short names unaffected).
- Mid slot shows the In ▸ Out ▸ Unknown precedence winner; the dump lists any others.
