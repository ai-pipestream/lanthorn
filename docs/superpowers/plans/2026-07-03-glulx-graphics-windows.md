# Glulx Graphics Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Glulx game open a `wintype_Graphics` window and draw into it (filled rects + Blorb images), rendered in the terminal via `ratatui-image`, with a master `--no-images` toggle that also silences cover art.

**Architecture:** `gvm` (zero-dep) implements the graphics Glk model, gestalt, `@glk` dispatch, pixel↔cell layout, and Redraw/Arrange events — passing resource **numbers** + pixel geometry to the `GlkBackend`. The `app` `AppGlk` backend owns an RGBA canvas per graphics window, composites draws, resolves image resource numbers against the story's Blorb, and renders each canvas via `ratatui-image` into a new `WinNode::Graphics` leaf.

**Tech Stack:** Rust, ratatui 0.30, `ratatui-image` 11.0.6, `image` 0.25 (already app deps). Spec: `docs/superpowers/specs/2026-07-03-glulx-graphics-windows-design.md`.

## Global Constraints

- **`gvm` and `zvm` stay zero-dependency.** `gvm` does no image decode/compositing — it passes resource numbers, 24-bit colors, and pixel geometry to the backend, and carries a plain `bool` graphics gate. (`crates/gvm/Cargo.toml` `[dependencies]` stays empty; `blorb` remains a dev-dependency only.)
- New image work lives in the **`app` crate** (already has `image`/`ratatui-image`). No new deps.
- **Every failure path is silent** — no panics; missing/undecodable images and out-of-range coords no-op or clip.
- **Cross-platform:** the half-block fallback must render graphics on any terminal.
- **`graphics` theme selector** (ColorScheme field + `style.rs` selector + render apply), per the styling rule.
- **Master toggle default ON** (`images = true`); `--no-images` / `images = false` disables in-game graphics **and** cover art, and wins over `--image-protocol`.
- `gvm-cli` requires **no change**: the graphics gate defaults **off** via a setter (only `app` turns it on).
- Clippy stays at 0 warnings; the existing full suite stays green.
- Commit trailers on every commit:
  ```
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

## Shared constants & interfaces (used across tasks)

**Glk dispatch selectors** (authoritative, from `gi_dispa.c`): `glk_image_get_info` `0x00E0`, `glk_image_draw` `0x00E1`, `glk_image_draw_scaled` `0x00E2`, `glk_window_erase_rect` `0x00E9`, `glk_window_fill_rect` `0x00EA`, `glk_window_set_background_color` `0x00EB`. (`glk_window_get_size` `0x0025` and `glk_window_clear` `0x002A` already dispatched; `glk_window_flow_break` `0x00E8` is Surface-A, left in the catch-all.)

**Function signatures:** `glk_image_get_info(image, &w, &h) -> 1|0`; `glk_image_draw(win, image, x, y) -> success`; `glk_image_draw_scaled(win, image, x, y, w, h) -> success`; `glk_window_fill_rect(win, color, left, top, w, h)`; `glk_window_erase_rect(win, left, top, w, h)`; `glk_window_set_background_color(win, color)`. Colors are 24-bit `0xRRGGBB`.

**Gestalt selectors:** `gestalt_Graphics = 6`, `gestalt_DrawImage = 7` (arg = window type), `gestalt_GraphicsTransparency = 14`. `wintype_Graphics = 5`.

**`GlkBackend` graphics methods added in Task 2 (all no-op/`None` defaults):**
```rust
fn char_pixels(&self) -> (u32, u32) { (1, 1) }
fn image_info(&mut self, _resnum: u32) -> Option<(u32, u32)> { None }
fn graphics_fill_rect(&mut self, _win: u32, _color: u32, _left: i32, _top: i32, _w: u32, _h: u32) {}
fn graphics_erase_rect(&mut self, _win: u32, _left: i32, _top: i32, _w: u32, _h: u32) {}
fn graphics_set_background(&mut self, _win: u32, _color: u32) {}
fn graphics_draw_image(&mut self, _win: u32, _resnum: u32, _x: i32, _y: i32, _scale: Option<(u32, u32)>) {}
```

---

### Task 1: `gvm` — `WinType::Graphics`, graphics gate, gestalt

**Files:**
- Modify: `crates/gvm/src/glk.rs` (`WinType` 20-49; `Model::window_open` 660-716; `TestBackend::window_open` 373-422)
- Modify: `crates/gvm/src/exec.rs` (`Machine` struct field ~141; `with_glk` init ~272; setter near `set_acceleration` 1619; `glk_gestalt` 2913-2931; `glk_open_window` 2627-2644)
- Test: both files' `#[cfg(test)]` modules

**Interfaces:**
- Produces: `WinType::Graphics` (`from_arg(5)`, `to_arg → 5`); `Machine::set_graphics(&mut self, on: bool)` (default off); `glk_gestalt` graphics answers gated on the flag; `glk_window_open(wintype=5)` succeeds only when enabled.

- [ ] **Step 1: Write failing tests (gvm exec.rs tests)**

Add to `crates/gvm/src/exec.rs` tests (mirror existing `@glk` tests that build a machine + `TestBackend`):

```rust
#[test]
fn graphics_gestalt_gated_on_flag() {
    let mut m = super::tests::machine_with_glk(&[]); // helper that builds a Machine over minimal mem + TestBackend
    // Default: graphics OFF → gestalt reports none.
    assert_eq!(m.glk_gestalt(6, 0), 0, "gestalt_Graphics off by default");
    assert_eq!(m.glk_gestalt(7, 5), 0, "gestalt_DrawImage(Graphics) off");
    m.set_graphics(true);
    assert_eq!(m.glk_gestalt(6, 0), 1, "gestalt_Graphics on");
    assert_eq!(m.glk_gestalt(7, 5), 1, "gestalt_DrawImage(wintype_Graphics=5) on");
    assert_eq!(m.glk_gestalt(7, 3), 0, "gestalt_DrawImage(wintype_TextBuffer=3) off — Surface A deferred");
    assert_eq!(m.glk_gestalt(14, 0), 1, "gestalt_GraphicsTransparency on");
}

#[test]
fn graphics_window_open_gated_on_flag() {
    let mut m = super::tests::machine_with_glk(&[]);
    // wintype_Graphics = 5; open a root graphics window.
    assert_eq!(m.glk_open_window(0, 0, 0, 5, 0), 0, "graphics window rejected when disabled");
    m.set_graphics(true);
    assert_ne!(m.glk_open_window(0, 0, 0, 5, 0), 0, "graphics window opens when enabled");
}
```

If no `machine_with_glk` test helper exists, add one in the exec.rs test module that builds a `Machine::with_glk(Memory::new(minimal_glulx_image()), Box::new(glk::TestBackend::new()))` — reuse whatever minimal-image helper the existing `@glk` tests use (search the test module for `with_glk(`).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p gvm graphics_gestalt_gated_on_flag graphics_window_open_gated_on_flag`
Expected: FAIL — `set_graphics` not found; gestalt/open don't gate on graphics.

- [ ] **Step 3: Add `WinType::Graphics`**

In `crates/gvm/src/glk.rs`, extend the enum and both mappers (20-49):

```rust
pub enum WinType {
    Pair,
    TextBuffer,
    TextGrid,
    /// Pixel-canvas graphics window (`wintype_Graphics` = 5).
    Graphics,
}
```
`from_arg`: add `5 => Some(WinType::Graphics),`. `to_arg`: add `WinType::Graphics => 5,`.

Fix the now-non-exhaustive `TestBackend::window_open` match (glk.rs:377) by adding an arm (graphics windows record nothing in the text-only TestBackend by default):
```rust
            WinType::Graphics => {}
```
(`layout_window` at glk.rs:802 also matches `WinType` exhaustively — add `WinType::Graphics => {}` there too so it compiles; real graphics layout is Task 3.)

- [ ] **Step 4: Add the graphics gate field + setter**

In `crates/gvm/src/exec.rs`: add the field beside `acceleration` (~141):
```rust
    /// Whether Glk graphics windows are enabled (default false; hosts opt in).
    pub(crate) graphics_enabled: bool,
```
Init in `with_glk` beside `acceleration: true,` (~272):
```rust
        graphics_enabled: false,
```
Add the setter beside `set_acceleration` (~1621):
```rust
    /// Enable/disable Glk graphics windows (gestalt + graphics-window open).
    pub fn set_graphics(&mut self, on: bool) {
        self.graphics_enabled = on;
    }
```

- [ ] **Step 5: Gate gestalt and window-open**

In `glk_gestalt` (exec.rs:2918), add arms before the `_ => 0` (keep existing arms):
```rust
        6 => self.graphics_enabled as u32,                       // gestalt_Graphics
        7 => (self.graphics_enabled && val == 5) as u32,         // gestalt_DrawImage(wintype)
        14 => self.graphics_enabled as u32,                      // gestalt_GraphicsTransparency
```

In `glk_open_window` (exec.rs:2627), reject graphics when disabled — add at the top of the function body:
```rust
        if wintype == 5 && !self.graphics_enabled {
            self.diagnostics.push("glk_window_open(Graphics) rejected — graphics disabled".to_string());
            return 0;
        }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p gvm graphics_ && cargo test -p gvm` (whole gvm suite — the new `WinType` variant must not break existing window tests)
Expected: PASS, all gvm tests green.

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy -p gvm --all-targets` (expect 0 warnings), then:
```bash
git add crates/gvm/src/glk.rs crates/gvm/src/exec.rs
git commit -m "$(cat <<'EOF'
feat(gvm): WinType::Graphics + graphics gestalt gate (default off)

Adds wintype_Graphics, a graphics_enabled flag (set_graphics setter,
default false so gvm-cli is unaffected), gated glk_gestalt answers
(Graphics/DrawImage(Graphics)/GraphicsTransparency), and graphics-window
open rejection when disabled. No drawing yet.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 2: `gvm` — `GlkBackend` graphics methods + `@glk` drawing dispatch

**Files:**
- Modify: `crates/gvm/src/glk.rs` (`GlkBackend` trait 266-305; `TestBackend` struct 311-371 + impl 373-422)
- Modify: `crates/gvm/src/exec.rs` (`glk_dispatch` match — add arms; `window_clear` routing)
- Test: `crates/gvm/src/exec.rs` tests

**Interfaces:**
- Consumes: `WinType::Graphics`, `graphics_enabled` (Task 1).
- Produces: the six `GlkBackend` graphics methods (see Shared interfaces); dispatch of selectors `0x00E0/E1/E2/E9/EA/EB` to them, gated on `graphics_enabled`; `TestBackend` records graphics calls for assertions via new accessors `fills(win) -> Vec<(u32,i32,i32,u32,u32)>`, `draws(win) -> Vec<(u32,i32,i32,Option<(u32,u32)>)>`, `background(win) -> Option<u32>`.

- [ ] **Step 1: Write failing tests**

In `crates/gvm/src/exec.rs` tests, drive `@glk` graphics selectors through a machine and assert `TestBackend` recorded them. Build a tiny Glulx routine that pushes args and calls `@glk`, or call `glk_dispatch` directly if the test module already does (search existing `@glk` tests for the pattern). Prefer the direct-dispatch style:

```rust
#[test]
fn graphics_ops_dispatch_to_backend() {
    let mut m = super::tests::machine_with_glk(&[]);
    m.set_graphics(true);
    let win = m.glk_open_window(0, 0, 0, 5, 0); // graphics root
    assert_ne!(win, 0);

    // fill_rect(win, color=0xFF0000, left=1, top=2, w=3, h=4)
    m.glk_dispatch(0x00EA, &[3, 4, 2, 1, 0x00FF_0000, win]).unwrap(); // args are stack order: last-pushed first
    // set_background_color(win, 0x0000FF)
    m.glk_dispatch(0x00EB, &[0x0000_00FF, win]).unwrap();
    // image_draw(win, resnum=7, x=5, y=6)
    m.glk_dispatch(0x00E1, &[6, 5, 7, win]).unwrap();

    let tb = m.backend.as_any().downcast_ref::<glk::TestBackend>().unwrap();
    assert_eq!(tb.fills(win), vec![(0x00FF_0000, 1, 2, 3, 4)]);
    assert_eq!(tb.background(win), Some(0x0000_00FF));
    assert_eq!(tb.draws(win), vec![(7, 5, 6, None)]);
}
```

NOTE on arg order: `glk_dispatch(selector, args)` receives args already popped off the VM stack, first Glk arg first. So for `fill_rect(win, color, left, top, w, h)` the `args` slice passed to `glk_dispatch` in a real call is `[win, color, left, top, w, h]`. **Verify the exact ordering against `op_glk`/an existing multi-arg arm before finalizing the test literals** (the `a(i)` accessor reads `args[i]` as the i-th Glk parameter, so pass `&[win, color, left, top, w, h]`). Correct the test to `m.glk_dispatch(0x00EA, &[win, 0x00FF_0000, 1, 2, 3, 4])` if that is the convention — match whatever the existing `glk_window_get_size` test (selector 0x0025, args `[win, wptr, hptr]`) does.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p gvm graphics_ops_dispatch_to_backend`
Expected: FAIL — `fills`/`draws`/`background` not found; selectors hit the return-0 stub.

- [ ] **Step 3: Add the six trait methods (no-op defaults)**

In `crates/gvm/src/glk.rs` `GlkBackend` trait (after `window_clear`, before `flush`), add the six methods from Shared interfaces verbatim.

- [ ] **Step 4: Record graphics calls in `TestBackend`**

Add fields to the `TestBackend` struct (glk.rs:311):
```rust
    fills: BTreeMap<u32, Vec<(u32, i32, i32, u32, u32)>>,   // (color, left, top, w, h)
    draws: BTreeMap<u32, Vec<(u32, i32, i32, Option<(u32, u32)>)>>, // (resnum, x, y, scale)
    backgrounds: BTreeMap<u32, u32>,
```
Init them `BTreeMap::new()` in both `TestBackend::new()` and the `..Self::new()` path. Add accessors:
```rust
    pub fn fills(&self, win: u32) -> Vec<(u32, i32, i32, u32, u32)> { self.fills.get(&win).cloned().unwrap_or_default() }
    pub fn draws(&self, win: u32) -> Vec<(u32, i32, i32, Option<(u32, u32)>)> { self.draws.get(&win).cloned().unwrap_or_default() }
    pub fn background(&self, win: u32) -> Option<u32> { self.backgrounds.get(&win).copied() }
```
Implement the methods in `impl GlkBackend for TestBackend`:
```rust
    fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.fills.entry(win).or_default().push((color, left, top, w, h));
    }
    fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32) {
        // erase records as a fill with the window's background (or 0).
        let color = self.backgrounds.get(&win).copied().unwrap_or(0);
        self.fills.entry(win).or_default().push((color, left, top, w, h));
    }
    fn graphics_set_background(&mut self, win: u32, color: u32) { self.backgrounds.insert(win, color); }
    fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) {
        self.draws.entry(win).or_default().push((resnum, x, y, scale));
    }
```

- [ ] **Step 5: Dispatch the graphics selectors**

In `crates/gvm/src/exec.rs` `glk_dispatch`, add arms (before the catch-all at 2593). Each is gated on `graphics_enabled` (no-op returning 0 when off). `i32` casts reinterpret the `u32` Glk coords as signed:
```rust
        0x00E0 => {
            // glk_image_get_info(image, widthptr, heightptr) -> 1 if it exists
            if self.graphics_enabled {
                if let Some((w, h)) = self.backend.image_info(a(0)) {
                    self.glk_store_ptr(a(1), w)?;
                    self.glk_store_ptr(a(2), h)?;
                    1
                } else { 0 }
            } else { 0 }
        }
        0x00E1 => {
            // glk_image_draw(win, image, val1=x, val2=y)
            if self.graphics_enabled {
                self.backend.graphics_draw_image(a(0), a(1), a(2) as i32, a(3) as i32, None);
                1
            } else { 0 }
        }
        0x00E2 => {
            // glk_image_draw_scaled(win, image, val1=x, val2=y, width, height)
            if self.graphics_enabled {
                self.backend.graphics_draw_image(a(0), a(1), a(2) as i32, a(3) as i32, Some((a(4), a(5))));
                1
            } else { 0 }
        }
        0x00E9 => {
            // glk_window_erase_rect(win, left, top, width, height)
            if self.graphics_enabled {
                self.backend.graphics_erase_rect(a(0), a(1) as i32, a(2) as i32, a(3), a(4));
            }
            0
        }
        0x00EA => {
            // glk_window_fill_rect(win, color, left, top, width, height)
            if self.graphics_enabled {
                self.backend.graphics_fill_rect(a(0), a(1), a(2) as i32, a(3) as i32, a(4), a(5));
            }
            0
        }
        0x00EB => {
            // glk_window_set_background_color(win, color)
            if self.graphics_enabled {
                self.backend.graphics_set_background(a(0), a(1));
            }
            0
        }
```

`glk_window_clear` (0x002A) already dispatches to `self.backend.window_clear(win)`; graphics-window erase-on-clear is handled inside `AppGlk::window_clear` (Task 4), so no change here.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p gvm graphics_ops_dispatch_to_backend && cargo test -p gvm`
Expected: PASS, full gvm suite green.

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy -p gvm --all-targets` (0 warnings), then commit `crates/gvm/src/glk.rs crates/gvm/src/exec.rs` with message `feat(gvm): GlkBackend graphics methods + @glk drawing dispatch` (+ trailers).

---

### Task 3: `gvm` — pixel↔cell layout, pixel `get_size`, Redraw events

**Files:**
- Modify: `crates/gvm/src/glk.rs` (`Model` struct 507-521 add a transient `char_px`; `relayout` 779-794; `layout_window` 796-822; add a `window_pixel_size` helper)
- Modify: `crates/gvm/src/exec.rs` (`relayout_glk` 2646-2651; `glk_window_get_size` arm 2290-2297; push Redraw in `glk_open_window` and the arrangement arm 2298-2305)
- Test: both test modules

**Interfaces:**
- Consumes: `WinType::Graphics`, `GlkBackend::char_pixels` (Task 2).
- Produces: graphics-window fixed splits sized in pixels→cells via `char_pixels`; `glk_window_get_size` returns **pixels** for graphics windows; `evtype_Redraw` pushed on graphics-window open and on arrangement.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn graphics_fixed_split_converts_pixels_to_cells() {
    // A backend reporting 8x16 px cells; a 150px-tall fixed graphics window
    // below a text buffer → ceil(150/16) = 10 cells tall.
    let mut m = super::tests::machine_with_glk_charpx(80, 24, 8, 16); // helper below
    m.set_graphics(true);
    let buf = m.glk_open_window(0, 0, 0, 3, 0);           // text buffer root
    // winmethod: BELOW(0x03) | FIXED(0x10) = 0x13, size=150 px, wintype_Graphics=5
    let gfx = m.glk_open_window(buf, 0x13, 150, 5, 0);
    assert_ne!(gfx, 0);
    // get_size returns PIXELS for a graphics window: 10 cells * 16 = 160 tall,
    // 80 cells * 8 = 640 wide.
    // (drive glk_window_get_size via dispatch and read the stored ptr, or test the helper directly)
    let (w_px, h_px) = m.graphics_window_pixels(gfx).unwrap();
    assert_eq!(h_px, 160);
    assert_eq!(w_px, 640);
}

#[test]
fn graphics_window_open_pushes_redraw() {
    let mut m = super::tests::machine_with_glk_charpx(80, 24, 8, 16);
    m.set_graphics(true);
    let _gfx = m.glk_open_window(0, 0, 0, 5, 0);
    assert!(m.glk.take_pending_events().iter().any(|e| e.etype == glk::evtype::REDRAW),
        "opening a graphics window queues a Redraw");
}
```
Add test helpers: `machine_with_glk_charpx(cols, rows, cw, ch)` builds a `TestBackend` whose `char_pixels()` returns `(cw, ch)` and `screen_size()` returns `(cols, rows)` — extend `TestBackend` with a `char_px: (u32,u32)` field (default `(1,1)`), a `with_char_pixels(cw, ch)` builder, and implement `char_pixels()` to return it. `graphics_window_pixels(win)` is a thin test accessor on `Machine` returning `self.glk.window_pixel_size(win, self.backend.char_pixels())`. `take_pending_events()` drains `self.events` for assertions (add to `Model`: `pub fn take_pending_events(&mut self) -> Vec<GlkEvent> { self.events.drain(..).collect() }`).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p gvm graphics_fixed_split graphics_window_open_pushes_redraw`
Expected: FAIL — helpers/`window_pixel_size` missing; fixed split treats 150 as cells; no Redraw queued.

- [ ] **Step 3: Thread `char_pixels` into layout**

In `Model` (glk.rs:507), add a transient field:
```rust
    /// Pixel size of one character cell (w, h), set by `relayout` for graphics
    /// fixed-split conversion. (1,1) until the backend reports otherwise.
    char_px: (u32, u32),
```
Init `char_px: (1, 1),` in `Model::new`.

Change `relayout` to accept the scale and store it (glk.rs:779):
```rust
    pub fn relayout(&mut self, width: u32, height: u32, char_px: (u32, u32)) -> Vec<(u32, WinType, Rect)> {
        self.char_px = char_px;
        // ... unchanged body ...
    }
```

In `layout_window`'s `WinType::Pair` arm (glk.rs), convert a graphics key window's FIXED pixel size to cells before splitting:
```rust
        WinType::Pair => {
            let _ = key;
            // Graphics fixed-splits size in PIXELS; convert to cells.
            let key_is_graphics = self.win(child2).map(|w| w.wintype) == Some(WinType::Graphics);
            let is_fixed = (method & WINMETHOD_DIVISIONMASK) == WINMETHOD_FIXED;
            let eff_size = if key_is_graphics && is_fixed {
                let dir = method & WINMETHOD_DIRMASK;
                let vertical = dir == WINMETHOD_ABOVE || dir == WINMETHOD_BELOW;
                let cell_px = if vertical { self.char_px.1 } else { self.char_px.0 }.max(1);
                size.div_ceil(cell_px)
            } else {
                size
            };
            let (r_old, r_new) = split_rect(rect, method, eff_size);
            self.layout_window(child1, r_old);
            self.layout_window(child2, r_new);
        }
```
Add `WinType::Graphics => {}` to `layout_window`'s leaf arms if not added in Task 1.

Add the pixel-size helper (near `window_size`, glk.rs:849):
```rust
    /// A graphics window's `(width, height)` in PIXELS = cells × char_px.
    /// `None` if the window is invalid or not a graphics window.
    pub fn window_pixel_size(&self, win: u32, char_px: (u32, u32)) -> Option<(u32, u32)> {
        let w = self.win(win)?;
        if w.wintype != WinType::Graphics {
            return None;
        }
        Some((w.rect.width * char_px.0, w.rect.height * char_px.1))
    }
```

- [ ] **Step 4: Update `relayout_glk` + `get_size` + push Redraw (exec.rs)**

`relayout_glk` (exec.rs:2646) — pass `char_pixels`:
```rust
    fn relayout_glk(&mut self) {
        let (w, h) = self.backend.screen_size();
        let cp = self.backend.char_pixels();
        let layout = self.glk.relayout(w, h, cp);
        self.backend.window_layout(&layout);
    }
```

`glk_window_get_size` arm (exec.rs:2290) — return pixels for graphics windows:
```rust
        0x0025 => {
            // glk_window_get_size(win, awidthptr, aheightptr)
            let cp = self.backend.char_pixels();
            let size = self.glk.window_pixel_size(a(0), cp).or_else(|| self.glk.window_size(a(0)));
            if let Some((w, h)) = size {
                self.glk_store_ptr(a(1), w)?;
                self.glk_store_ptr(a(2), h)?;
            }
            0
        }
```

Push Redraw when a graphics window opens — in `glk_open_window` (exec.rs:2627) after `self.relayout_glk();` in the `Some(id)` arm:
```rust
                if self.glk.window_type(id) == Some(glk::WinType::Graphics) {
                    self.glk.push_event(GlkEvent { etype: glk::evtype::REDRAW, win: id, val1: 0, val2: 0 });
                }
```
And in the `glk_window_set_arrangement` arm (exec.rs:2298), after the existing Arrange push, also push a Redraw (arrangement can resize graphics windows):
```rust
                self.glk.push_event(GlkEvent { etype: glk::evtype::REDRAW, win: 0, val1: 0, val2: 0 });
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p gvm graphics_ && cargo test -p gvm`
Expected: PASS, full gvm suite green (the `relayout` signature change requires updating any existing `relayout(` call sites — search `crates/gvm/` for `.relayout(` and add `(1,1)` for pre-existing text-only callers/tests).

- [ ] **Step 6: Clippy + commit**

`cargo clippy -p gvm --all-targets` (0 warnings), then commit both files: `feat(gvm): graphics-window pixel↔cell layout, pixel get_size, Redraw events` (+ trailers).

---

### Task 4: `app` — `AppGlk` graphics canvas + backend impl

**Files:**
- Modify: `crates/app/src/glk_backend.rs` (`AppGlk` struct 90-138; `impl GlkBackend for AppGlk` 402+; `window_clear` 473+)
- Create: `crates/app/src/graphics.rs` (canvas type + compositing + resnum resolution)
- Modify: `crates/app/src/lib.rs` (`pub mod graphics;`)
- Test: `crates/app/src/graphics.rs` tests

**Interfaces:**
- Consumes: the `GlkBackend` graphics methods (Task 2); `blorb::Blorb`, `app::cover::decode`.
- Produces:
  - `graphics::Canvas` — `struct Canvas { img: image::RgbaImage, bg: image::Rgba<u8>, version: u64 }` with `fill_rect`, `erase_rect`, `set_background`, `draw_image`, and `arc(&self) -> std::sync::Arc<image::RgbaImage>`.
  - `graphics::PictSource` — resolves + caches decoded images by resource number from an optional `blorb::Blorb`.
  - `AppGlk` fields: `graphics: BTreeMap<u32, graphics::Canvas>`, `char_px: (u32,u32)`, `picts: graphics::PictSource`, all constructor-injected; the six `GlkBackend` graphics methods implemented against them.

- [ ] **Step 1: Write failing tests (graphics.rs)**

Create `crates/app/src/graphics.rs` with a test module first:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_paints_pixels_and_bumps_version() {
        let mut c = Canvas::new(10, 10);
        let v0 = c.version;
        c.fill_rect(0x00FF_0000, 2, 3, 4, 5); // red
        assert!(c.version > v0);
        let px = c.img.get_pixel(2, 3);
        assert_eq!(px.0, [0xFF, 0x00, 0x00, 0xFF]);
        // outside the rect stays transparent/default
        assert_ne!(c.img.get_pixel(9, 9).0, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn fill_rect_clips_out_of_bounds() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x0000_FF00, -2, -2, 100, 100); // green, way oversized
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0xFF, 0x00, 0xFF]);
        // no panic; whole canvas filled
        assert_eq!(c.img.get_pixel(3, 3).0, [0x00, 0xFF, 0x00, 0xFF]);
    }

    #[test]
    fn erase_uses_background_color() {
        let mut c = Canvas::new(4, 4);
        c.set_background(0x0000_00FF); // blue
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4);
        c.erase_rect(0, 0, 2, 2);
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0x00, 0xFF, 0xFF]); // erased → bg
        assert_eq!(c.img.get_pixel(3, 3).0, [0xFF, 0x00, 0x00, 0xFF]); // untouched
    }

    #[test]
    fn draw_image_composites_scaled() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 1, 1, Some((4, 4)));
        assert_eq!(c.img.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(c.img.get_pixel(4, 4).0, [10, 20, 30, 255]); // scaled to 4x4
    }

    #[test]
    fn pict_source_resolves_and_caches() {
        // No blorb → None.
        let mut none = PictSource::new(None);
        assert!(none.info(1).is_none());
        assert!(none.image(1).is_none());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app --lib graphics`
Expected: FAIL to compile — `Canvas`/`PictSource` undefined.

- [ ] **Step 3: Implement `graphics.rs`**

Prepend the implementation:
```rust
//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::HashMap;
use std::sync::Arc;

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

/// Unpack a Glk 24-bit `0xRRGGBB` color into an opaque RGBA pixel.
fn rgb(color: u32) -> Rgba<u8> {
    Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF])
}

/// A graphics window's pixel canvas.
pub struct Canvas {
    pub img: RgbaImage,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        Canvas { img: RgbaImage::new(w.max(1), h.max(1)), bg: Rgba([0, 0, 0, 0xFF]), version: 1 }
    }

    /// Resize (preserving nothing — Glk redraws) if the pixel dims changed.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (self.img.width(), self.img.height()) != (w.max(1), h.max(1)) {
            self.img = RgbaImage::from_pixel(w.max(1), h.max(1), self.bg);
            self.version += 1;
        }
    }

    pub fn set_background(&mut self, color: u32) { self.bg = rgb(color); }

    fn paint(&mut self, px: Rgba<u8>, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = (self.img.width() as i64, self.img.height() as i64);
        let x0 = left.max(0) as i64;
        let y0 = top.max(0) as i64;
        let x1 = (left as i64 + w as i64).min(cw);
        let y1 = (top as i64 + h as i64).min(ch);
        for y in y0..y1 {
            for x in x0..x1 {
                self.img.put_pixel(x as u32, y as u32, px);
            }
        }
        self.version += 1;
    }

    pub fn fill_rect(&mut self, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.paint(rgb(color), left, top, w, h);
    }

    pub fn erase_rect(&mut self, left: i32, top: i32, w: u32, h: u32) {
        let bg = self.bg;
        self.paint(bg, left, top, w, h);
    }

    /// Composite `src` at `(x, y)`, optionally scaled to `(sw, sh)`, honoring alpha.
    pub fn draw_image(&mut self, src: &DynamicImage, x: i32, y: i32, scale: Option<(u32, u32)>) {
        let scaled;
        let view: &DynamicImage = match scale {
            Some((sw, sh)) if sw > 0 && sh > 0 => {
                scaled = src.resize_exact(sw, sh, image::imageops::FilterType::Triangle);
                &scaled
            }
            _ => src,
        };
        image::imageops::overlay(&mut self.img, view, x as i64, y as i64);
        self.version += 1;
    }

    pub fn arc(&self) -> Arc<RgbaImage> { Arc::new(self.img.clone()) }
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    cache: HashMap<u32, Option<DynamicImage>>,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        PictSource { blorb, cache: HashMap::new() }
    }

    fn get(&mut self, resnum: u32) -> Option<&DynamicImage> {
        if !self.cache.contains_key(&resnum) {
            let decoded = self.blorb.as_ref()
                .and_then(|b| b.resource(b"Pict", resnum))
                .and_then(|(_ty, bytes)| crate::cover::decode(bytes));
            self.cache.insert(resnum, decoded);
        }
        self.cache.get(&resnum).and_then(|o| o.as_ref())
    }

    /// `(width, height)` of a Pict, or `None`.
    pub fn info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.get(resnum).map(|i| i.dimensions())
    }

    /// The decoded image for a Pict, or `None`.
    pub fn image(&mut self, resnum: u32) -> Option<&DynamicImage> {
        self.get(resnum)
    }
}
```
Register in `crates/app/src/lib.rs`: `pub mod graphics;`.

- [ ] **Step 4: Run graphics.rs tests**

Run: `cargo test -p app --lib graphics`
Expected: PASS (5 tests).

- [ ] **Step 5: Wire `AppGlk` fields + constructor**

In `crates/app/src/glk_backend.rs`, add fields to `AppGlk` (90):
```rust
    graphics: std::collections::BTreeMap<u32, crate::graphics::Canvas>,
    char_px: (u32, u32),
    picts: crate::graphics::PictSource,
```
Change `AppGlk::new` to accept the scale + pict source (keep `Default` working by giving it a no-image default):
```rust
    pub fn new(cols: u32, rows: u32) -> AppGlk {
        AppGlk::with_graphics(cols, rows, (1, 1), crate::graphics::PictSource::new(None))
    }
    pub fn with_graphics(cols: u32, rows: u32, char_px: (u32, u32), picts: crate::graphics::PictSource) -> AppGlk {
        AppGlk {
            cols, rows,
            layout: Vec::new(),
            grids: std::collections::BTreeMap::new(),
            buffers: std::collections::BTreeMap::new(),
            primary: None,
            heading_acc: String::new(),
            last_heading: None,
            at_line_start: true,
            in_heading: false,
            graphics: std::collections::BTreeMap::new(),
            char_px,
            picts,
        }
    }
```

- [ ] **Step 6: Implement the graphics `GlkBackend` methods on `AppGlk`**

In `impl GlkBackend for AppGlk`, add:
```rust
    fn char_pixels(&self) -> (u32, u32) { self.char_px }

    fn image_info(&mut self, resnum: u32) -> Option<(u32, u32)> { self.picts.info(resnum) }

    fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.graphics.entry(win).or_insert_with(|| self.canvas_for(win))
            .fill_rect(color, left, top, w, h);
    }
    fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32) {
        self.graphics.entry(win).or_insert_with(|| self.canvas_for(win))
            .erase_rect(left, top, w, h);
    }
    fn graphics_set_background(&mut self, win: u32, color: u32) {
        self.graphics.entry(win).or_insert_with(|| self.canvas_for(win))
            .set_background(color);
    }
    fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) {
        if let Some(src) = self.picts.image(resnum).cloned() {
            self.graphics.entry(win).or_insert_with(|| self.canvas_for(win))
                .draw_image(&src, x, y, scale);
        }
    }
```
Because `canvas_for` borrows `self` while `entry` also borrows `self.graphics`, instead compute the canvas size inline. Add a helper that does NOT borrow `self.graphics`:
```rust
    fn canvas_size(&self, win: u32) -> (u32, u32) {
        let cells = self.layout.iter().find(|&&(id, _, _)| id == win).map(|&(_, _, r)| (r.width, r.height)).unwrap_or((1, 1));
        (cells.0 * self.char_px.0, cells.1 * self.char_px.1)
    }
```
and replace each `or_insert_with(|| self.canvas_for(win))` with a size captured first:
```rust
        let (cw, ch) = self.canvas_size(win);
        self.graphics.entry(win).or_insert_with(|| crate::graphics::Canvas::new(cw, ch)) ...
```
(Apply that pattern in all four methods to satisfy the borrow checker.) In `window_layout` (glk_backend.rs:431), after storing `self.layout`, resize existing graphics canvases to their new pixel size:
```rust
        for &(id, ty, rect) in wins {
            if ty == WinType::Graphics {
                let (cw, ch) = (rect.width * self.char_px.0, rect.height * self.char_px.1);
                if let Some(c) = self.graphics.get_mut(&id) { c.resize(cw, ch); }
            }
        }
```
In `window_clear` (glk_backend.rs:473), erase a graphics window to bg:
```rust
        if let Some(c) = self.graphics.get_mut(&win) {
            let (w, h) = (c.img.width(), c.img.height());
            c.erase_rect(0, 0, w, h);
        }
```
In `window_close`, add `self.graphics.remove(&id);`.

- [ ] **Step 7: Add an `AppGlk` graphics unit test**

```rust
#[test]
fn appglk_graphics_fill_composites_into_canvas() {
    let mut g = AppGlk::with_graphics(80, 24, (2, 2), crate::graphics::PictSource::new(None));
    // Simulate a laid-out graphics window id=1 occupying 4x4 cells → 8x8 px.
    g.window_open(1, gvm::glk::WinType::Graphics);
    g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 4, height: 4 })]);
    g.graphics_fill_rect(1, 0x00FF_0000, 0, 0, 8, 8);
    let canvas = g.graphics.get(&1).unwrap();
    assert_eq!(canvas.img.dimensions(), (8, 8));
    assert_eq!(canvas.img.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF]);
}
```
(This test needs `graphics` field visibility — it is in the same crate/module, fine.)

- [ ] **Step 8: Run + clippy + commit**

Run: `cargo test -p app --lib graphics appglk_graphics && cargo clippy -p app --all-targets`
Expected: PASS; 0 warnings. Commit `crates/app/src/graphics.rs crates/app/src/glk_backend.rs crates/app/src/lib.rs`: `feat(app): AppGlk graphics canvas compositing + Pict resolution` (+ trailers).

---

### Task 5: `app` — `WinNode::Graphics` leaf + `screen_model` emission

**Files:**
- Modify: `crates/app/src/engine.rs` (`WinNode` 176-191; add `GraphicsWindow`)
- Modify: `crates/app/src/glk_backend.rs` (`screen_model` 234-288; `assemble`/`find_buffers` arms)
- Test: `crates/app/src/glk_backend.rs` tests

**Interfaces:**
- Consumes: `AppGlk.graphics` canvases (Task 4).
- Produces: `WinNode::Graphics(GraphicsWindow)` where `GraphicsWindow { win: u32, canvas: std::sync::Arc<image::RgbaImage>, version: u64 }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn screen_model_emits_graphics_leaf() {
    let mut g = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(None));
    g.window_open(1, gvm::glk::WinType::Graphics);
    g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 10, height: 4 })]);
    g.graphics_fill_rect(1, 0x00FF00, 0, 0, 10, 4);
    let model = g.screen_model();
    // The tree's single leaf is a Graphics node for window 1.
    fn find_graphics(n: &crate::engine::WinNode) -> bool {
        match n {
            crate::engine::WinNode::Graphics(_) => true,
            crate::engine::WinNode::Pair { first, second, .. } => find_graphics(first) || find_graphics(second),
            _ => false,
        }
    }
    assert!(find_graphics(&model.root), "graphics window should appear as a Graphics leaf");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app --lib screen_model_emits_graphics_leaf`
Expected: FAIL — `WinNode::Graphics` undefined.

- [ ] **Step 3: Add `GraphicsWindow` + `WinNode::Graphics`**

In `crates/app/src/engine.rs`, before `WinNode`:
```rust
/// A graphics-window leaf: a snapshot of the window's canvas for rendering.
#[derive(Debug, Clone)]
pub struct GraphicsWindow {
    pub win: u32,
    pub canvas: std::sync::Arc<image::RgbaImage>,
    pub version: u64,
}
```
Add the variant to `WinNode`:
```rust
    /// A pixel-canvas graphics window.
    Graphics(GraphicsWindow),
```
(`app` already depends on `image`, so `image::RgbaImage` is available in `engine.rs`.)

- [ ] **Step 4: Emit the leaf in `screen_model`**

In `crates/app/src/glk_backend.rs` `screen_model` (234), add a `WinType::Graphics` arm to the `match ty`:
```rust
                WinType::Graphics => {
                    let c = self.graphics.get(&id);
                    WinNode::Graphics(crate::engine::GraphicsWindow {
                        win: id,
                        canvas: c.map(|c| c.arc()).unwrap_or_else(|| std::sync::Arc::new(image::RgbaImage::new(1, 1))),
                        version: c.map(|c| c.version).unwrap_or(0),
                    })
                }
```
Add `WinNode::Graphics` arms wherever the tree is matched exhaustively in this file — `assemble`/`find_buffers`/`bounding_box` (glk_backend.rs:297-398). For `find_buffers` (which collects buffer leaves) a Graphics leaf contributes none: add `WinNode::Graphics(_) => {}` (or the match's no-op path). Compile errors will pinpoint each site.

- [ ] **Step 5: Run + fix exhaustiveness**

Run: `cargo test -p app --lib screen_model_emits_graphics_leaf` (and `cargo build -p app`)
Expected: after adding the required match arms, PASS.

- [ ] **Step 6: Clippy + commit**

`cargo clippy -p app --all-targets` (0 warnings), commit `crates/app/src/engine.rs crates/app/src/glk_backend.rs`: `feat(app): WinNode::Graphics leaf + screen_model emission` (+ trailers).

---

### Task 6: `app` — render the graphics leaf (Picker + protocol cache)

**Files:**
- Modify: `crates/app/src/state.rs` (`AppState` — add `graphics_render: std::cell::RefCell<GraphicsRender>` and a game-loop `Option<ratatui_image::picker::Picker>`)
- Create: `crates/app/src/render/graphics.rs` (`GraphicsRender` protocol cache + `render_graphics`)
- Modify: `crates/app/src/render/screen.rs` (`render_node` add `WinNode::Graphics` arm 121-161; `count_leaves` 32-43; `is_simple` 47-50)
- Modify: `crates/app/src/render/mod.rs` (`pub mod graphics;` if render submodules are declared there)
- Test: `crates/app/src/render/screen.rs` tests

**Interfaces:**
- Consumes: `WinNode::Graphics(GraphicsWindow)` (Task 5); `AppState`.
- Produces: `render::graphics::GraphicsRender` with `fn render(&mut self, picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style, buf: &mut Buffer)` caching a protocol keyed by `(win, version, area.w, area.h)`.

- [ ] **Step 1: Write the failing render test**

In `crates/app/src/render/screen.rs` tests (mirror the cover render test in main.rs / existing screen tests):
```rust
#[test]
fn graphics_leaf_renders_pixels() {
    use ratatui::layout::Rect;
    use ratatui::buffer::Buffer;
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 50, 50, 255]));
    let gw = crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1 };
    let picker = ratatui_image::picker::Picker::halfblocks();
    let mut gr = crate::render::graphics::GraphicsRender::default();
    let area = Rect::new(0, 0, 12, 6);
    let mut buf = Buffer::empty(area);
    let style = ratatui::style::Style::default();
    gr.render(&picker, &gw, area, style, &mut buf);
    let has_pixels = (area.top()..area.bottom()).any(|y| (area.left()..area.right())
        .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}")));
    assert!(has_pixels, "graphics canvas should render half-block pixels");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app --lib graphics_leaf_renders_pixels`
Expected: FAIL — `render::graphics::GraphicsRender` undefined.

- [ ] **Step 3: Implement `render/graphics.rs`**

```rust
//! Renders `WinNode::Graphics` canvases via ratatui-image, caching the built
//! protocol per (window, canvas version, area size).

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::engine::GraphicsWindow;

#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
}

impl GraphicsRender {
    pub fn render(&mut self, picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Letterbox fill behind the fitted canvas.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(letterbox);
                }
            }
        }
        let fresh = matches!(self.cache.get(&gw.win),
            Some((v, w, h, _)) if *v == gw.version && *w == area.width && *h == area.height);
        if !fresh {
            let img = image::DynamicImage::ImageRgba8((*gw.canvas).clone());
            match picker.new_protocol(img, Size::new(area.width, area.height), Resize::Fit(None)) {
                Ok(p) => { self.cache.insert(gw.win, (gw.version, area.width, area.height, p)); }
                Err(_) => return,
            }
        }
        if let Some((_, _, _, proto)) = self.cache.get(&gw.win) {
            let sz = proto.size();
            let w = sz.width.min(area.width);
            let h = sz.height.min(area.height);
            let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
            Image::new(proto).render(dest, buf);
        }
    }
}
```
Declare it: add `pub mod graphics;` to `crates/app/src/render/mod.rs` (or wherever render submodules are declared — match the existing `pub mod screen;` pattern).

- [ ] **Step 4: Add the `render_node` arm + classification**

In `crates/app/src/render/screen.rs`, `render_node` (121), add before `WinNode::Blank`:
```rust
        WinNode::Graphics(gw) => {
            if let Some(picker) = state.game_picker.as_ref() {
                state.graphics_render.borrow_mut().render(picker, gw, area, state.colors.graphics, buf);
            } else {
                fill(area, buf, &state.colors);
            }
            None
        }
```
In `count_leaves` (32) add a Graphics arm returning `(0, 0, 1)` (counts as "other", forcing the generic path — correct, since a graphics window can't use the simple text path):
```rust
        WinNode::Graphics(_) => (0, 0, 1),
```
`is_simple` (47) already treats `others > 0` as not-simple, so no change beyond `count_leaves`. (Confirm `ScreenModel::grid`'s `find` recursion in engine.rs:229-238 has a Graphics arm — add `WinNode::Graphics(_) => None` there.)

- [ ] **Step 5a: Add the `graphics` ColorScheme field (needed by the render arm)**

The render arm reads `state.colors.graphics`, so the field must exist now (Task 8 adds the full *selector* wiring on top of it). In `crates/app/src/colors.rs` add `pub graphics: Style,` (near the `story_info_cover` field ~256) and its default in BOTH constructors: `graphics: Style::new().bg(Color::Black),` in `terminal_default` (~384) and `graphics: Style::new().bg(bg),` in `from_ghostty` (~566). Build `-p app` to confirm every `ColorScheme` literal is complete.

- [ ] **Step 5: Add `AppState` fields**

In `crates/app/src/state.rs`, add to `AppState`:
```rust
    /// The in-game graphics Picker (None when images are disabled or unbuilt).
    pub game_picker: Option<ratatui_image::picker::Picker>,
    /// Cached graphics-window protocols (interior-mutable for the render pass).
    pub graphics_render: std::cell::RefCell<crate::render::graphics::GraphicsRender>,
```
Initialize both in every `AppState` constructor/`Default` (search for where `AppState` is built — set `game_picker: None,` and `graphics_render: std::cell::RefCell::new(Default::default()),`). Task 7 populates `game_picker`.

- [ ] **Step 6: Run + clippy + commit**

Run: `cargo test -p app --lib graphics_leaf_renders_pixels && cargo build -p app && cargo clippy -p app --all-targets`
Expected: PASS; builds; 0 warnings. Commit the four files: `feat(app): render graphics-window canvases via ratatui-image` (+ trailers).

---

### Task 7: `app` — `--no-images` toggle, session wiring, Pict resolution end-to-end

**Files:**
- Modify: `crates/app/src/config.rs` (`Cli` 162-183; `Config` fields + defaults; merge 570-571; test literals)
- Modify: `crates/app/src/glulx_session.rs` (`GlulxSession::new` 101-120)
- Modify: `crates/app/src/state.rs` (build `game_picker`)
- Modify: `crates/app/src/main.rs` (resolve `pict_blorb`; gate cover Picker on `cfg.images`; thread `graphics_enabled`, `char_pixels`, Pict source into the session; build `game_picker`)
- Test: `crates/app/src/config.rs` tests

**Interfaces:**
- Consumes: everything above; `graphics::PictSource`, `Machine::set_graphics` (Task 1), `AppGlk::with_graphics` (Task 4).
- Produces: `config.images: bool`; `GlulxSession::new(image, cols, rows, acceleration, graphics_enabled, char_px, pict_blorb)`.

- [ ] **Step 1: Write the failing config test**

In `crates/app/src/config.rs` tests (mirror `acceleration_defaults_true_and_no_accel_disables`):
```rust
#[test]
fn images_defaults_true_and_no_images_disables() {
    assert!(Config::default().images);
    let cli = Cli { /* ...existing fields... */ no_images: true, ..cli_fixture() };
    let cfg = resolve_with(cli); // however the existing test constructs a Config from a Cli
    assert!(!cfg.images);
}
```
(Match the exact helper the neighboring `no_accel` test uses; if it builds a `Cli` literal inline, replicate that literal with `no_images: true`.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app --lib images_defaults_true_and_no_images_disables`
Expected: FAIL — `images`/`no_images` fields don't exist.

- [ ] **Step 3: Add the config field + CLI flag + merge**

In `crates/app/src/config.rs`: add a default fn near `default_acceleration` (233):
```rust
fn default_images() -> bool { true }
```
Add the `Config` field beside `image_protocol` (429-454), runtime-only like `acceleration`:
```rust
    /// Whether image rendering (in-game graphics + cover art) is enabled.
    /// Runtime-only (set from --no-images); not persisted.
    #[serde(skip, default = "default_images")]
    pub images: bool,
```
Init in `Default for Config` beside `image_protocol: default_image_protocol()` (490): `images: default_images(),`.
Add the `Cli` flag after `image_protocol` (183):
```rust
    /// Disable all image rendering (in-game graphics + story-picker cover art).
    #[arg(long)]
    pub no_images: bool,
```
Add the merge beside `cfg.image_protocol = cli.image_protocol;` (571): `cfg.images = !cli.no_images;`.
Fix every `Cli { .. }` test literal (config.rs:757-760, 771-774, 786-789, 929-932, 1078-1081) and `Config { .. }` literal with `no_images: false,` / `images: true,` as the compiler flags them.

- [ ] **Step 4: Thread graphics into `GlulxSession::new`**

In `crates/app/src/glulx_session.rs` (101), extend the signature and body:
```rust
    pub fn new(
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
    ) -> Result<GlulxSession, GError> {
        let mem = Memory::new(image)?;
        let picts = crate::graphics::PictSource::new(pict_blorb);
        let backend = Box::new(AppGlk::with_graphics(cols, rows, char_px, picts));
        let mut machine = Machine::with_glk(mem, backend);
        machine.set_acceleration(acceleration);
        machine.set_graphics(graphics_enabled);
        // ...unchanged: drive(&mut machine), build session, refresh_screen()...
    }
```

- [ ] **Step 5: Build the game Picker + resolve Pict Blorb in main.rs**

In `crates/app/src/main.rs`:
- Build a game-loop Picker gated on `cfg.images`, reusing the cover helper: after config resolve, `let game_picker = if cfg.images { build_cover_picker(cfg.image_protocol) } else { None };` and its `char_px` = `game_picker.as_ref().map(|p| { let f = p.font_size(); (f.width as u32, f.height as u32) }).unwrap_or((8, 16))`. Store `game_picker` into `AppState.game_picker` when building state.
- Gate the cover-art path: where `draw_info_panel`/cover currently run, wrap the Picker build in `if cfg.images` (the picker path already tolerates `Option<&Picker>` = None → no cover). Simplest: in `run_story_picker`, `let cover_picker = if cfg.images { build_cover_picker(cfg.image_protocol) } else { None };`.
- Resolve the Pict Blorb from the story path (mirror `state.sound_blorb` at main.rs:1767): `let pict_blorb = if cfg.images { blorb::resolve_sound_blorb(&story_path).map(|(b, _)| b) } else { None };` — reuse `resolve_sound_blorb` (it returns the story's own Blorb / sibling), since Pict and Snd live in the same container. (If a distinct resolver is cleaner, add `blorb::resolve_blorb(&story_path)` in the `blorb` crate returning the container regardless of Snd presence; but `resolve_sound_blorb` already returns the self-blorb for a `.gblorb`, which is the common case.)
- In the Glulx launch arm (main.rs:1635), pass the new args: `GlulxSession::new(story_bytes, cols, rows, cfg.acceleration, cfg.images, char_px, pict_blorb)`.

- [ ] **Step 6: Run the whole workspace + clippy**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: builds; all tests pass; 0 warnings.

- [ ] **Step 7: Commit**

Commit `config.rs glulx_session.rs state.rs main.rs`: `feat(app): --no-images toggle + thread graphics into the Glulx session` (+ trailers).

---

### Task 8: `graphics` theme selector wiring

The `ColorScheme.graphics` **field** was already added in Task 6 (Step 5a). This task exposes it as a themeable `"graphics"` selector (the 4 `style.rs` wiring sites), so users can restyle the graphics-window letterbox.

**Files:**
- Modify: `crates/app/src/style.rs` (`SELECTOR_FIELDS` 194; `SELECTOR_GROUPS` 236; `style_for_selector` 295; `apply_color_decls` 443)
- Test: `crates/app/src/style.rs` tests

**Interfaces:**
- Consumes: `ColorScheme.graphics` (added in Task 6).
- Produces: the `"graphics"` selector recognized by `style_for_selector`/`apply_color_decls`.

- [ ] **Step 1: Write the failing test** (mirror `story_info_cover_selector_round_trips`, style.rs:1886):
```rust
#[test]
fn graphics_selector_round_trips() {
    use ratatui::style::Color;
    let mut cs = colors::ColorScheme::default();
    cs.graphics = ratatui::style::Style::new().bg(Color::Rgb(1, 2, 3));
    assert_eq!(style_for_selector(&cs, "graphics"), ratatui::style::Style::new().bg(Color::Rgb(1, 2, 3)));
    assert!(SELECTOR_FIELDS.contains(&"graphics"));
    assert!(SELECTOR_GROUPS.iter().any(|(_, s)| s.contains(&"graphics")));
}
```
- [ ] **Step 2: Run to verify it fails** — `cargo test -p app --lib graphics_selector_round_trips` → FAIL (`"graphics"` not in `SELECTOR_FIELDS`; `style_for_selector` returns default).
- [ ] **Step 3: Wire the selector** — `style.rs`: add `"graphics"` to `SELECTOR_FIELDS`; add a new `("Graphics", &["graphics"])` group to `SELECTOR_GROUPS`; `"graphics" => cs.graphics,` in `style_for_selector`; `"graphics" => cs.graphics = cs.graphics.patch(style),` in `apply_color_decls`. (The `cs.graphics` field already exists from Task 6.)
- [ ] **Step 4: Run style + colors suites** — `cargo test -p app --lib style && cargo test -p app --lib colors` → PASS (completeness test green).
- [ ] **Step 5: Clippy + commit** — `cargo clippy -p app --all-targets` (0 warnings); commit `style.rs`: `feat(app): expose graphics theme selector` (+ trailers).

---

### Task 9: Synthetic fixture, gvm-cli guard, README

**Files:**
- Create: `crates/gvm/tests/graphics_story.rs` (drive a hand-assembled Glulx graphics routine through a `Machine` + `TestBackend`)
- Modify: `crates/gvm-cli/` tests (a guard test that graphics stays off there) — OR fold into the gvm default-off test (Task 1). Confirm gvm-cli still builds/runs unchanged.
- Modify: `README.md`

**Interfaces:** Consumes the whole feature.

- [ ] **Step 1: Synthetic Glulx graphics fixture test**

Add `crates/gvm/tests/graphics_story.rs` — assemble a minimal Glulx image (reuse the assembler/helper the existing gvm integration tests use; search `crates/gvm/tests/` and `crates/gvm/src/asm.rs` for the fixture-building pattern) whose start function: opens a graphics root window (`@glk glk_window_open` split=0 wintype=5), fills a rect, and draws image #1. Run it with `Machine::with_glk(mem, TestBackend)`, `set_graphics(true)`, drive to completion, and assert `TestBackend.fills(win)` / `.draws(win)` recorded the ops. If assembling a full Glulx routine is impractical in-test, assert the same behaviors via direct `glk_dispatch` calls in a `crates/gvm/tests/` integration test instead (the Task 2/3 unit tests already cover dispatch; this fixture is the end-to-end confidence check — keep it if the assembler helper exists, otherwise `log!` that it was covered by unit tests and skip).

- [ ] **Step 2: gvm-cli unchanged guard**

Confirm `cargo test -p gvm-cli` and `cargo run -p gvm-cli -- <a graphics .gblorb>` behave exactly as before (graphics stays off — no graphics windows open, output text-only). If the crate has a conformance test harness (e.g. `glulxercise`), run it and confirm no regression. Document the result in the report. (No code change expected in gvm-cli — the default-off gate covers it; Task 1's `graphics_window_open` default-off test is the locking assertion.)

- [ ] **Step 3: README**

Add a feature bullet under the appropriate section (match the cover-art bullet style):
```markdown
- **In-game graphics (Glulx).** Games that open graphics windows now render
  their filled shapes and images in the terminal, using the best graphics
  protocol (Kitty / iTerm2 / Sixel) with a half-block fallback. Disable all
  image rendering (in-game graphics *and* cover art) with `--no-images`.
```

- [ ] **Step 4: Full verification + commit**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: green; 0 warnings. Commit the fixture + README: `test(gvm): synthetic graphics-window story; docs(readme): in-game graphics` (+ trailers).

---

## Final verification

- [ ] `cargo test --workspace` — baseline + new tests, 0 failures
- [ ] `cargo clippy --workspace --all-targets` — 0 warnings
- [ ] `crates/gvm/Cargo.toml` + `crates/zvm/Cargo.toml` `[dependencies]` unchanged (zero-dep intact); no new deps anywhere
- [ ] `gvm-cli` behavior unchanged (graphics off by default)
- [ ] Manual: a graphical Glulx game shows its graphics window (protocol + `--image-protocol halfblocks`); `--no-images` runs it text-only and hides cover art
