# Mouse Support — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued behind the hotkey-dialog track (shares `input.rs`/`main.rs`/`state.rs`).
**TODO items:** "Add mouse support with clickable rooms… left click shows game/story related information, right click shows diagnostics" (L16) and "Add mouse wheel support for map scroll" (L18). Plus middle-button drag-to-pan.

## Goal

Make the map mouse-interactive: click a room to inspect it (left = story info, right = diagnostics), use the wheel to pan/zoom the map (and scroll the transcript over the story pane), and drag with the middle button to pan.

## Mouse capture

- Enable `crossterm::event::EnableMouseCapture` once, right after `EnterAlternateScreen` in `main`. Add `DisableMouseCapture` to `restore_terminal()` (so both the clean-exit path and the panic hook release it). Without this, no mouse events arrive.
- In the event loop, handle `Event::Mouse(MouseEvent)` alongside the existing `Event::Key`.

## Pane routing

`draw_frame` currently returns only `map_area: Rect`. Change it to return both pane rects (e.g. `struct PaneRects { map: Rect, story: Rect }`, or `(Rect, Rect)`). The loop uses them to route a mouse event by which rect contains `(event.column, event.row)`:
- inside `map` → map interactions (below);
- inside `story` → transcript wheel scroll;
- elsewhere → ignored.

## Hit-testing (screen → room)

Add to `render/map.rs` (inverse of `cell_to_screen` at map.rs:236):
- `pub fn screen_to_cell(screen: (i32,i32), zoom: Zoom, scroll: (i32,i32), area: Rect) -> (i32,i32)` — `cell = ((screen.x - area.x)/step_w + scroll.0, (screen.y - area.y)/step_h + scroll.1)` using `zoom.steps()`.
- `pub fn room_at_cell(graph: &MapGraph, layer: LayerId, cell: (i32,i32)) -> Option<RoomId>` — the room in `layer` whose `pos == Some(cell)`, else None (a click in the gutter between boxes returns None).

## Interactions (`input.rs::mouse_to_action`)

A new `pub fn mouse_to_action(state: &AppState, m: MouseEvent, map: Rect, story: Rect) -> Action` mirroring `key_to_action`. `m.modifiers` carries Shift/Ctrl. Mapping by `m.kind`:

- **`Down(Left)`** in `map`: `room_at_cell(...)` → `Some(id)` ⇒ `Action::ShowRoomInfo(id)` (also selects the room); `None` ⇒ `Action::CloseRoomPanel`.
- **`Down(Right)`** in `map`: `Some(id)` ⇒ `Action::ShowRoomDiagnostics(id)` (selects + diagnostics); `None` ⇒ `Action::CloseRoomPanel`.
- **`Down(Middle)`** in `map`: `Action::BeginDragPan(col,row)` — records the drag anchor.
- **`Drag(Middle)`**: `Action::DragPanTo(col,row)` — pans by the movement since the anchor (see Drag-pan below).
- **`Up(Middle)`**: `Action::EndDragPan`.
- **`ScrollUp`/`ScrollDown`** in `map`: default `Pan(0,-1)`/`Pan(0,1)`; with `Shift` → `Pan(-1,0)`/`Pan(1,0)`; with `Ctrl` → `ZoomIn`/`ZoomOut`.
- **`ScrollUp`/`ScrollDown`** in `story`: scroll the transcript (decrement/increment `transcript_scroll`, reusing the existing transcript-scroll path).
- **`ScrollLeft`/`ScrollRight`** (terminals that send them) in `map`: `Pan(-1,0)`/`Pan(1,0)`.
- Other kinds (`Moved`, button up for left/right) → `Action::None`.

## Drag-pan (middle button)

Grab-and-drag: the map content follows the cursor, so the cell grabbed stays under it. Scroll is grid-cell granular while the mouse moves per terminal cell, so accumulate:
- `AppState.drag: Option<DragState>` where `DragState { last: (u16,u16), acc_x: i32, acc_y: i32 }`.
- `BeginDragPan(c,r)` → `drag = Some({ last:(c,r), acc:0,0 })`.
- `DragPanTo(c,r)` → let `(dx,dy) = (c - last.x, r - last.y)` in terminal cells; add to `acc`; while `|acc_x| >= step_w` pan one grid cell in that direction and subtract `step_w` (same for y with `step_h`); set `last=(c,r)`. Panning direction = grab-and-drag (dragging right scrolls the view left so content moves right). Uses `state.zoom.steps()`.
- `EndDragPan` → `drag = None`.

## Room panels (`state.rs` + `render/room_info.rs`)

- `AppState.room_panel: Option<RoomPanel>` where `RoomPanel { id: RoomId, mode: RoomPanelMode }`, `enum RoomPanelMode { Info, Diagnostics }`. `selected_room` is set alongside.
- **`ShowRoomInfo(id)`** → `room_panel = Some({id, Info})` + `selected_room = Some(id)`.
- **`ShowRoomDiagnostics(id)`** → `room_panel = Some({id, Diagnostics})` + select.
- **`CloseRoomPanel`** → `room_panel = None`.
- `draw_frame` renders, when `room_panel` is set: `Info` → `render/room_info.rs::draw_room_info`; `Diagnostics` → the existing `render/inspector.rs::draw_inspector` (reused; the `i`-key / `toggle_inspector` path keeps working). The keyboard inspector toggle and the mouse paths share `room_panel`.
- **`render/room_info.rs` (new)** — `draw_room_info(graph, &GameSession, room_id, current_room, area, buf)`: shows the room's **name**, **notes**, and **exits** (each outgoing connection: direction glyph → destination room name, from the graph). If `room_id == current_room`, also list the **objects in the current room** queried live from the Z-machine (use the existing `zvm` object/location API — `crates/zvm/src/objects.rs` / `location.rs`; list the contents of the player's current location object). For non-current rooms, omit the objects section (their live contents are unknown).

## New actions (`input.rs`)

`ShowRoomInfo(RoomId)`, `ShowRoomDiagnostics(RoomId)`, `CloseRoomPanel`, `BeginDragPan(u16,u16)`, `DragPanTo(u16,u16)`, `EndDragPan`. Wheel reuses `Pan`/`ZoomIn`/`ZoomOut` and the transcript-scroll action.

## Testing

- `screen_to_cell` is the inverse of `cell_to_screen` for sample cells/zooms/scrolls; `room_at_cell` finds a placed room and returns None for an empty cell.
- `mouse_to_action`: left-down on a room cell → `ShowRoomInfo(id)`; right-down → `ShowRoomDiagnostics(id)`; left-down on gutter → `CloseRoomPanel`; wheel up over map → `Pan(0,-1)`, `Shift` → `Pan(-1,0)`, `Ctrl` → `ZoomIn`; wheel over the story rect → transcript scroll; `Down(Middle)`/`Drag`/`Up` → the drag actions.
- Drag accumulator: a `DragPanTo` whose movement exceeds `step_w` pans exactly one cell; sub-step movement pans zero; direction is grab-and-drag.
- `room_info` render (TestBackend): shows name + an exit line; shows an objects section only when `room_id == current_room`.
- Mouse capture: `restore_terminal` issues `DisableMouseCapture` (and the panic hook path does too).

## Out of scope / non-goals

- Drag-to-select or rubber-band selection; dragging rooms with the mouse (the map is auto-laid-out / nudged by keyboard).
- Clicking connectors/edges.
- Mouse interaction inside the other overlays (gallery/saves/hotkey dialog stay keyboard-driven for now).
- `mapper` changes beyond the two pure hit-test helpers in the app's `render/map.rs` (no `mapper` crate edits).

## Risks & limitations (accepted)

- **Terminal support varies:** some terminals don't send `ScrollLeft/Right` or `Drag` motion events; vertical wheel + click + Shift/Ctrl-wheel work broadly, and the keyboard remains a full fallback.
- **Drag granularity:** panning snaps to grid cells (no sub-cell scroll), so a slow middle-drag moves in ~`step_w`-column increments — acceptable and matches the grid model.
- **Objects only for the current room:** by design — the Z-machine object tree only reliably reflects the room the player is in.
- **Sequencing:** touches `input.rs`/`main.rs`/`state.rs`; dispatch only after the hotkey-dialog track merges.
