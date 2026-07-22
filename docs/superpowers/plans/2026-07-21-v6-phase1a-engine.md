# v6 Phase 1a — Engine + Graphics Boundary — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give zvm a v6 multi-window model with real geometry and the zero-dep picture boundary, so Zork0 boots headless with no layout underflow, `picture_data` returns real dimensions, and `draw_picture` emits draw events — everything needed for rendering (Plan 1b) except the render itself.

**Architecture:** v6-gated state added to `ScreenState` (an `Option<V6Windows>`) so v1–5/v7/v8 stay byte-identical. v6 window opcodes operate on an 8-window table addressed in pixels; `get`/`put_wind_prop` expose the ZMSD window-property array. Picture dimensions are injected app-side (header-sniff, no full decode) as a `Machine` constructor parameter (set before the boot run); `draw_picture` emits events the app will drain in Plan 1b — mirroring `pending_sounds`.

**Tech Stack:** Rust. `zvm` stays zero-dependency; app uses the existing `blorb`/`image`/`graphics.rs` machinery.

**Spec:** `docs/superpowers/specs/2026-07-21-v6-phase1-windows-design.md`

## Global Constraints

- **`zvm` stays zero-dependency.** `crates/zvm/Cargo.toml` `[dependencies]` must remain empty. zvm holds only plain numbers (a picture-dimension `Vec`, an event `Vec`); ALL Blorb/image work is app-side.
- **Additive to v3–8.** v1–5/v7/v8 screen behavior must be BYTE-IDENTICAL. All v6 state lives behind `ScreenState.v6: Option<V6Windows>`, populated only when `version == 6`; every existing v1–5 opcode arm is untouched. A v3/v5 regression guard must stay green.
- **Boot-time injection.** `picture_data` is called during boot, which runs inside `GameSession::new`. The picture-dimension table is a `new_with_trace` parameter set before the boot loop (Phase 0 lesson).
- **ZMSD-verified property indices** (Task 6) — verified against ZMSD 1.1 §8.4.3, not memory (Phase 0 precedent).
- Font cell size is a tuning constant; document it (Task 1).

## Design reference (locked here, used by all tasks)

**`ZWindow`** — one v6 window; its fields ARE the ZMSD window-property array (index = property number):

```rust
// crates/zvm/src/screen.rs
#[derive(Debug, Clone, Default)]
pub struct ZWindow {
    pub y_coord: u16,          // prop 0  (pixels)
    pub x_coord: u16,          // prop 1
    pub y_size: u16,           // prop 2  (height, pixels)
    pub x_size: u16,           // prop 3  (width, pixels)
    pub y_cursor: u16,         // prop 4
    pub x_cursor: u16,         // prop 5
    pub left_margin: u16,      // prop 6
    pub right_margin: u16,     // prop 7
    pub interrupt_routine: u16,// prop 8
    pub interrupt_countdown: u16, // prop 9
    pub text_style: u16,       // prop 10
    pub colour_data: u16,      // prop 11 (high byte bg, low byte fg — ZMSD)
    pub font_number: u16,      // prop 12
    pub font_size: u16,        // prop 13 (high byte height, low byte width)
    pub attributes: u16,       // prop 14 (bit0 wrap, bit1 scroll, bit2 copy-to-transcript, bit3 buffered)
    pub line_count: u16,       // prop 15
    /// Character grid for this window (grid windows 1–7). Window 0 scrolls (buffered),
    /// its text goes to the transcript stream, not a grid.
    pub grid: crate::screen::UpperWindow,
    pub fg: crate::screen::ZColour,
    pub bg: crate::screen::ZColour,
}

impl ZWindow {
    /// Read property `n` (0–15). Out-of-range → 0.
    pub fn get_prop(&self, n: u16) -> u16 { /* Task 6 */ 0 }
    /// Write property `n` (0–15). Out-of-range → ignored.
    pub fn put_prop(&mut self, n: u16, v: u16) { /* Task 6 */ }
}

#[derive(Debug, Clone, Default)]
pub struct V6Windows {
    pub windows: [ZWindow; 8],
    pub current: u8, // 0–7
}
```

**Font/pixel constants** (Task 1, in `screen.rs`):
```rust
pub const V6_FONT_WIDTH: u16 = 8;   // pixels per char cell — tuning param
pub const V6_FONT_HEIGHT: u16 = 8;  // (documented; adjust in Plan 1b TTY smoke if proportions look off)
```
v6 addresses everything in pixels; the app quantizes to cells by dividing by these (Plan 1b). Screen units advertised in the header = `cols * V6_FONT_WIDTH` by `rows * V6_FONT_HEIGHT`.

**Picture-dimension table + draw events** (Tasks 7–8, on `Machine`):
```rust
// crates/zvm/src/cpu/exec.rs
pub struct PictureEvent { pub number: u16, pub window: u8, pub x: u16, pub y: u16 }
// Machine fields:
pub picture_dims: Vec<(u16, u16, u16)>, // (picture_number, width_px, height_px), injected at construction
pub pending_pictures: Vec<PictureEvent>, // drained by the app each turn (Plan 1b)
```

---

### Task 1: v6 header screen + font dimensions

**Files:** Modify `crates/zvm/src/screen.rs` (`write_screen_dims`); Test: same file.

**Interfaces:** Produces: v6 stories advertise a realistic pixel screen (units = cols·8 × rows·8) and font size 8×8, so the game's pixel geometry math has sane inputs. v1–5 unchanged.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn v6_advertises_pixel_screen_and_font() {
    let mut m = Memory::new(sample_story(6)).unwrap();
    write_screen_dims(&mut m, 24, 80);
    assert_eq!(m.read_byte(0x20), 24, "rows");
    assert_eq!(m.read_byte(0x21), 80, "cols");
    assert_eq!(m.read_word(0x22), 80 * V6_FONT_WIDTH, "screen width in pixels");
    assert_eq!(m.read_word(0x24), 24 * V6_FONT_HEIGHT, "screen height in pixels");
    assert_eq!(m.read_byte(0x26), V6_FONT_WIDTH as u8, "font width");
    assert_eq!(m.read_byte(0x27), V6_FONT_HEIGHT as u8, "font height");
}
```
(`sample_story(6)` requires Phase 0, already merged.)

- [ ] **Step 2: Run — expect FAIL** (`cargo test -p zvm --lib screen::tests::v6_advertises_pixel_screen_and_font`): v6 currently takes the `version >= 5` arm → width = cols, font = 1.

- [ ] **Step 3: Implement** — add the `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` consts, and a v6 arm in `write_screen_dims` before the `version >= 5` arm:
```rust
    if version == 6 {
        mem.write_byte(0x20, rows);
        mem.write_byte(0x21, cols);
        mem.write_word(0x22, cols as u16 * V6_FONT_WIDTH);   // screen width, pixels
        mem.write_word(0x24, rows as u16 * V6_FONT_HEIGHT);  // screen height, pixels
        mem.write_byte(0x26, V6_FONT_WIDTH as u8);           // font width, pixels
        mem.write_byte(0x27, V6_FONT_HEIGHT as u8);          // font height, pixels
        return;
    }
```
Document the font consts as a Plan-1b tuning param.

- [ ] **Step 4: Run — expect PASS** (`cargo test -p zvm --lib screen`). Verify v1–5 dim tests still pass.

- [ ] **Step 5: Commit** — stage only `crates/zvm/src/screen.rs`. Message: `feat(zvm): advertise v6 pixel screen + font dimensions` + trailers (`Quest: SQ-0186`, Co-Authored-By, Claude-Session).

---

### Task 2: v6 window model on ScreenState

**Files:** Modify `crates/zvm/src/screen.rs` (`ZWindow`, `V6Windows`, `ScreenState.v6`, make `UpperWindow::idx` `pub(crate)`); `crates/zvm/src/cpu/exec.rs` (`with_output` init). Test: `screen.rs`.

**Interfaces:** Produces: `ScreenState.v6: Option<V6Windows>` populated for v6 only; `ZWindow`/`V6Windows` per the design reference. Consumed by Tasks 3–8.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn v6_screen_state_has_window_table() {
    let m = Machine::new(Memory::new(sample_story(6)).unwrap());
    let v6 = m.screen.v6.as_ref().expect("v6 story has a window table");
    assert_eq!(v6.windows.len(), 8);
    assert_eq!(v6.current, 0);
}
#[test]
fn non_v6_has_no_window_table() {
    let m = Machine::new(Memory::new(sample_story(5)).unwrap());
    assert!(m.screen.v6.is_none(), "v5 keeps the classic 2-window model");
}
```

- [ ] **Step 2: Run — expect FAIL** (compile error: no `v6` field).

- [ ] **Step 3: Implement**
  - Add `ZWindow`, `V6Windows` (per the design reference; `get_prop`/`put_prop` bodies stubbed to `0`/no-op for now — Task 6 fills them), and the font consts if not already.
  - Add `pub v6: Option<V6Windows>` to `ScreenState`; in `Default` set `v6: None`.
  - Change `UpperWindow::idx` from `fn` to `pub(crate) fn` (ZWindow.grid reuses it via its own methods, or expose grid put/cell — the `grid: UpperWindow` already has `pub put`/`pub cell`, so no idx change may be needed; confirm and only widen visibility if a v6 handler needs raw indexing).
  - In `with_output` (`exec.rs`), after constructing the machine, when `mem.version() == 6` set `screen.v6 = Some(V6Windows::default())`. (Place it where `screen: ScreenState::default()` is built, or right after — a small post-init assignment.)

- [ ] **Step 4: Run — expect PASS** (`cargo test -p zvm --lib`). Full lib suite green (regression: v1–5 untouched).

- [ ] **Step 5: Commit** — stage `crates/zvm/src/screen.rs`, `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 8-window model (gated on ScreenState.v6)` + trailers.

---

### Task 3: split_window / set_window / erase_window v6 branches

**Files:** Modify `crates/zvm/src/cpu/exec.rs` (VAR:0x0A/0x0B/0x0D handlers). Test: `exec.rs`.

**Interfaces:** Consumes Task 2. Produces: these opcodes route to the v6 table when `self.screen.v6.is_some()`, else the existing v1–5 path unchanged.

Semantics (v6, ZMSD §8):
- `set_window(w)` → `v6.current = w as u8` (0–7); reset that window's cursor to its top-left margin.
- `split_window(n)` → v6: ZMSD says split_window is defined but v6 games mostly use `window_size`/`move_window`; implement the ZMSD v6 meaning (set window 1 height to `n` lines and reposition), or — if the trace shows Zork0 doesn't call it in v6 — make it a no-op that does not disturb the table. Confirm against the Zork0 trace (it did NOT appear in the captured trace) and the ZMSD; document the choice.
- `erase_window(w)` → v6: `-1` unsplit+clear all, `-2` clear all without unsplit, else clear window `w`'s grid + reset its cursor.

- [ ] **Step 1: Write failing tests** — `set_window(7)` sets `v6.current == 7`; `erase_window(3)` clears window 3's grid; a v5 machine still routes through the classic path (assert `current_window` on a v5 machine still works).

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** — in each handler, branch `if self.screen.v6.is_some() { /* v6 table path */ } else { /* existing v1–5 code, unchanged */ }`. Keep the `trace_screen` push lines.

- [ ] **Step 4: Run — expect PASS**, full `cargo test -p zvm --lib cpu::exec` green.

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 split/set/erase_window over the window table` + trailers.

---

### Task 4: move_window / window_size / window_style bodies

**Files:** Modify `crates/zvm/src/cpu/exec.rs` (EXT:0x10/0x11/0x12 — currently in the Phase 0 no-op group). Test: `exec.rs`.

**Interfaces:** Consumes Task 2. Produces: these set the addressed window's rect/attributes in the v6 table.

Semantics (operands are `(window, a, b)`; ZMSD §15):
- `move_window(win, y, x)` → set `windows[win].y_coord = y; .x_coord = x` (pixels).
- `window_size(win, y, x)` → set `.y_size = y; .x_size = x`; resize the window's grid to the cell-equivalent (`y / V6_FONT_HEIGHT` rows × `x / V6_FONT_WIDTH` cols, min 1).
- `window_style(win, flags, operation)` → set/clear/toggle `.attributes` per the 2-bit operation (0 set, 1 clear, 2 set-bits, 3 clear-bits — confirm against ZMSD).

- [ ] **Step 1: Write failing tests** — `move_window(1,6,6)` then assert `windows[1].y_coord==6 && x_coord==6`; `window_size(1, 40, 80)` sets sizes and resizes the grid to `40/8=5` rows × `80/8=10` cols.

- [ ] **Step 2: Run — expect FAIL** (they're no-ops today).

- [ ] **Step 3: Implement** — remove `0x10 | 0x11 | 0x12` from the Phase 0 no-op group; add explicit arms operating on `self.screen.v6`. If `v6` is `None` (shouldn't happen for these v6-only opcodes), no-op. Add `trace_screen` pushes.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 move_window/window_size/window_style` + trailers.

---

### Task 5: set_cursor / set_colour v6 forms

**Files:** Modify `crates/zvm/src/cpu/exec.rs` (VAR:0x0F set_cursor, 2OP:0x1B set_colour, EXT:0x0D set_true_colour). Test: `exec.rs`.

**Interfaces:** Consumes Task 2. Produces: cursor + colour are per-window in v6.

Semantics:
- `set_cursor` v6: `(row, col)` addresses the CURRENT window's grid cursor (ZMSD keeps set_cursor line/column based even in v6 for grid windows); a negative `row` (−1 turn cursor off, −2 on) is v6-specific — handle if present. Write to `windows[current].y_cursor/x_cursor` (and the grid cursor used by text routing). A 3rd operand selects the window in v6 — read `ops.get(2)` when present.
- `set_colour(fg, bg, [window])` v6: the optional 3rd operand names the window; write `windows[win].fg/bg` (and `colour_data`). Without the 3rd operand, apply to the current window. NOTE: verify the dispatch — a 3-operand `set_colour` may decode as VAR not 2OP; confirm which handler receives it and implement there.
- `set_true_colour` similarly per-window in v6.

- [ ] **Step 1: Write failing tests** — `set_window(2)` then `set_cursor(3,4)` sets window 2's cursor; `set_colour` with a window operand sets that window's colour.

- [ ] **Step 2–4: red → implement (v6 branch; v1–5 unchanged) → green.**

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 per-window set_cursor/set_colour` + trailers.

---

### Task 6: get_wind_prop / put_wind_prop over the property array

**Files:** Modify `crates/zvm/src/screen.rs` (`ZWindow::get_prop`/`put_prop`), `crates/zvm/src/cpu/exec.rs` (EXT:0x13/0x19). Test: both.

**Interfaces:** Consumes Task 2. Produces: real per-window property reads/writes — the fix for the layout underflow.

- [ ] **Step 1: VERIFY property indices against ZMSD 1.1 §8.4.3** (hard gate — WebFetch the standard). Confirm the 0–15 mapping in the design reference (y-coord, x-coord, y-size, x-size, y-cursor, x-cursor, left-margin, right-margin, interrupt-routine, interrupt-countdown, text-style, colour-data, font-number, font-size, attributes, line-count) and note any v1.1 additions (16/17 true colours). Correct the struct/mapping if the ZMSD differs.

- [ ] **Step 2: Write the failing test**
```rust
#[test]
fn v6_get_put_wind_prop_round_trip() {
    let mut m = Machine::new(Memory::new(sample_story(6)).unwrap());
    // put via opcode: window 1, prop 2 (y-size) = 40
    m.exec_ext(0x19, &[1, 2, 40], None, None); // put_wind_prop(win, prop, val)
    // get via opcode: window 1, prop 2 -> store
    // (drive get_wind_prop and assert the stored value is 40)
    let v = m.screen.v6.as_ref().unwrap().windows[1].get_prop(2);
    assert_eq!(v, 40);
}
```
(Confirm `put_wind_prop` operand order `(window, property, value)` against ZMSD.)

- [ ] **Step 3: Run — expect FAIL** (get_prop returns 0; put_wind_prop is a no-op stub).

- [ ] **Step 4: Implement** — `get_prop`/`put_prop` as a `match n { 0 => self.y_coord, ... 15 => self.line_count, _ => 0 }`. Wire EXT:0x13 (`get_wind_prop`): read `windows[ops[0]].get_prop(ops[1])`, `do_store`. Wire EXT:0x19 (`put_wind_prop`): `windows[ops[0]].put_prop(ops[1], ops[2])`. Remove 0x13 from the Phase 0 stub and 0x19 from the no-op group. Keep `trace_screen`.

- [ ] **Step 5: Run — expect PASS**, full `cargo test -p zvm --lib` green.

- [ ] **Step 6: Commit** — stage `crates/zvm/src/screen.rs`, `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 get/put_wind_prop over the window property array` + trailers.

---

### Task 7: Picture-dimension table + picture_data

**Files:** Modify `crates/zvm/src/cpu/exec.rs` (`Machine.picture_dims` field, `with_output` init, EXT:0x06 `picture_data`). Test: `exec.rs`.

**Interfaces:** Produces: `Machine` holds an injected `picture_dims: Vec<(u16,u16,u16)>`; `picture_data` answers from it. A setter `set_picture_dims(Vec<(u16,u16,u16)>)` is called before the boot run (Task 9 injects the real data).

Semantics: `picture_data(number, array)` — if picture `number` is in the table, write its `(height, width)` into the game's 2-word array (`array[0]=height, array[1]=width` — CONFIRM order against ZMSD §15) and BRANCH (data available); else don't branch. A `number` of 0 with an array asks for "number of pictures + release" (ZMSD) — handle: write count + a release number, branch.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn v6_picture_data_reports_injected_dims() {
    let mut m = Machine::new(Memory::new(sample_story(6)).unwrap());
    m.set_picture_dims(vec![(5, 100, 60)]); // picture 5 = 100w × 60h
    // picture_data(5, array_addr) writes height/width to array, branches true.
    // Drive EXT:0x06 with a real branch + array in dynamic mem; assert the
    // words written and that the branch was taken.
    // (construct array_addr in dynamic memory; call via a decoded instruction
    //  or directly exercise the handler helper.)
}
```

- [ ] **Step 2: Run — expect FAIL** (returns no data / branches false).

- [ ] **Step 3: Implement** — add `pub picture_dims: Vec<(u16,u16,u16)>` (init `Vec::new()` in `with_output`, next to `pending_sounds`), a `pub fn set_picture_dims(&mut self, t: Vec<(u16,u16,u16)>)`. Rewrite EXT:0x06 to look up `ops[0]` in `picture_dims`, write height/width to `ops[1]` array via `self.mem.write_word`, `do_branch(branch, found)`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 picture_data from injected dimension table` + trailers.

---

### Task 8: draw_picture / erase_picture → pending_pictures events

**Files:** Modify `crates/zvm/src/cpu/exec.rs` (`PictureEvent`, `Machine.pending_pictures`, EXT:0x05 draw_picture, EXT:0x07 erase_picture). Test: `exec.rs`.

**Interfaces:** Produces: `Machine.pending_pictures: Vec<PictureEvent>`; `draw_picture` pushes an event; drained by the app in Plan 1b.

Semantics: `draw_picture(number, y, x)` — push `PictureEvent { number, window: v6.current, x, y }` (coords are pixels within the current window; ZMSD — confirm operand order `(number, y, x)`). `erase_picture` — push an event with a sentinel (or a separate `PictureErase` variant / a `number: 0` convention; pick one and document).

- [ ] **Step 1: Write the failing test** — `set_window(7)` then `draw_picture(5, 1, 1)` pushes `PictureEvent{number:5, window:7, y:1, x:1}` to `pending_pictures`.

- [ ] **Step 2: Run — expect FAIL** (no-op today).

- [ ] **Step 3: Implement** — add `PictureEvent` struct + `pub pending_pictures: Vec<PictureEvent>` (init in `with_output`). Rewrite EXT:0x05 to push; handle EXT:0x07 erase. Remove them from the Phase 0 no-op group. Keep `trace_screen`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): v6 draw_picture emits pending_pictures events` + trailers.

---

### Task 9: App — self-blorb Pict source, dimension table, inject at construction

**Files:** Modify `crates/app/src/graphics.rs` (`PictSource` header-sniff dims), `crates/app/src/session.rs` (`GameSession::new`/`new_with_trace` picture-dims param), `crates/app/src/startup.rs` (resolve self-blorb Pict source before session construction, build table, pass it in). Test: `graphics.rs` + an app integration test.

**Interfaces:** Consumes Tasks 7. Produces: a real dimension table injected into the v6 `Machine` before boot. NOTE: this touches the app crate — `zvm` unaffected.

- [ ] **Step 1: Cheap dims in `PictSource`** — add `pub fn dims(&mut self, resnum: u32) -> Option<(u32,u32)>` that header-sniffs without full decode: read the Pict bytes via `self.blorb`, then `image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()`. Add `pub fn all_pict_dims(&mut self) -> Vec<(u16,u16,u16)>` that iterates `blorb.resources()` filtered to `b"Pict"` and collects `(number, w, h)`. Test: on a tiny in-memory Blorb with one PNG, `all_pict_dims` returns its `(num, w, h)`.

- [ ] **Step 2: Thread the table into the session** — add a `picture_dims: Vec<(u16,u16,u16)>` parameter to `GameSession::new_with_trace` (and default `Vec::new()` from `new`); call `machine.set_picture_dims(picture_dims)` right after `machine.set_sound_available(...)`, BEFORE `init_caps()`/the boot loop. Update all `new`/`new_with_trace` call sites (grep them) to pass the table (empty for non-v6).

- [ ] **Step 3: Resolve the self-blorb Pict source in startup** — where the Z-machine story is loaded (`startup.rs`), when the story is itself a Blorb (`blorb::Blorb::is_blorb(bytes)`), parse it, extract the `.z6` executable for the VM, AND build the dimension table from the same Blorb's Pict resources (via a `PictSource` over that Blorb → `all_pict_dims()`). Retain the Blorb/PictSource on `AppState` for Plan 1b's draw-time decode (mirror `state.sound_blorb`). Pass the table into `GameSession::new_with_trace`.

- [ ] **Step 4: Tests** — `graphics.rs` unit test for `dims`/`all_pict_dims`; an app test that constructing a v6 session with a dimension table makes `machine.picture_dims` non-empty before the first turn. Run `cargo test -p app -p zvm`.

- [ ] **Step 5: Commit** — stage `crates/app/src/graphics.rs`, `crates/app/src/session.rs`, `crates/app/src/startup.rs` (+ any call-site files). Message: `feat(app): resolve v6 Pict dimensions + inject into the session at boot` + trailers.

---

### Task 10: Zork0 headless engine smoke

**Files:** Create `crates/app/tests/zork0_v6_windows.rs` (needs the app-side Blorb resolution, so it lives in `app`, not `zvm`).

**Interfaces:** Consumes all prior tasks. The Plan 1a acceptance gate.

- [ ] **Step 1: Write the smoke** — load `stories/Zork0.blb` (graceful-skip if absent, per `crates/gvm/tests/kerkerkruip_boots.rs`), build a v6 `GameSession` with the injected dimension table, drive to the first prompt + a couple turns. Assert: (a) no `Fault`; (b) `machine.screen.v6` window sizes are sane — e.g. window 0's `x_size`/`y_size` are nonzero and NOT the `0xFFFB`-range underflow values the Phase 0 build produced; (c) `machine.pending_pictures` accumulated the expected draw events (non-empty, with `window == 7` among them); (d) `picture_data` was answered (a queried picture's dims are nonzero). Capture what to assert from a `--nocapture` run first, then lock the assertions.

- [ ] **Step 2: Run** `cargo test -p app --test zork0_v6_windows -- --nocapture`. MUST pass with `stories/Zork0.blb` present. A geometry underflow or fault means a wrong opcode body — debug before committing.

- [ ] **Step 3: Commit** — stage `crates/app/tests/zork0_v6_windows.rs`. Message: `test(app): Zork0 v6 window-geometry headless smoke` + trailers.

---

## Final verification (after all tasks)

- [ ] `cargo test -p zvm -p app` green (unit + Zork0 window smoke).
- [ ] `cargo clippy -p zvm -p app --all-targets` clean.
- [ ] `crates/zvm/Cargo.toml` `[dependencies]` still empty (zero-dep invariant).
- [ ] v3/v5 regression: existing screen/exec tests unchanged and green (byte-identical non-v6 path).
- [ ] Zork0 headless: no geometry underflow, picture_data answers, draw events fire.

## Definition of done

The v6 window model + graphics boundary exist and are headless-verified against Zork0: geometry is sane, `picture_data` real, `draw_picture` events emitted, `zvm` still zero-dep, v1–5 byte-identical. Plan 1b (adapter + render) can now consume the window table + draw events to put Zork0 on screen.
