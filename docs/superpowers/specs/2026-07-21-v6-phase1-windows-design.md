# v6 Phase 1 — Window Model + Text + Basic Pictures — Design

**Date:** 2026-07-21
**Quest:** SQ-0186 (v6 graphical Z-machine)
**Status:** Design — approved (scope + graphics boundary), pre-plan
**Builds on:** Phase 0 (`docs/superpowers/specs/2026-07-21-v6-phase0-boot-design.md`) — v6 boots text-only, all 18 v6 EXT opcodes are decode-correct no-op stubs.
**Prior art:** `docs/design/2026-07-16-glk-crate-and-format-strategy.md` (Decision 2 — native v6); `docs/superpowers/specs/2026-07-03-glulx-graphics-windows-design.md` (the Glulx graphics-window pattern this mirrors).

## Goal

Make Zork Zero render recognizably: its multi-window layout (main text + upper status + a picture window) lays out in the right regions, text lands where the game puts it, and its pictures (title screen, borders, room art) actually draw. Text uses the **cell-text-wins** policy (SQ-0186 note, 2026-07-21): v6 pixel windows quantize to cell tiles; where text and graphics overlap, cell-text wins and the picture shows through the gaps.

## Empirical grounding — what Zork0 actually does

A headless screen-opcode trace of Zork0 (boot + a few turns, captured 2026-07-21) established the real behavior — the design targets this, not the full ZMSD abstraction:

- **Three windows:** 0 (lower, scrolling main text), 1 (upper, cursor-positioned grid), 7 (graphics — `draw_picture` target). `set_window(win7)` confirms window numbers beyond the v1–5 binary 0/1.
- **Pixel geometry via `move_window`/`window_size`**, e.g. `move_window[0, 6, 6]`, `move_window[1, 1, 1]`, `window_size[7, 0, 0]`.
- **Layout is computed from queried dimensions.** `get_wind_prop` is called constantly (props 0, 2, 3, 4, 9 — y-coord, y-size, x-size, y-cursor, interrupt-countdown) and `picture_data` queries picture sizes. Both currently return **0**, so the game computes positions like `screen_dim − picture_height` with a 0 and **underflows** (the observed `window_size[0, 65531, 65526]` = −5/−10 as i16). **→ Real `get_wind_prop` AND real `picture_data` dimensions are prerequisites for correct *text* layout, not just for rendering.**
- **Text windows tile; graphics overlap.** Windows 0 and 1 tile (upper strip + lower body) — representable as a cell-quantized `Pair` tree. The overlapping/decorative content is all window-7 `draw_picture`, handled on the graphics layer.

## The pixel / cell / font model

v6 addresses windows and pictures in **pixels**; lanthorn renders in **character cells**. The bridge is the font size the interpreter advertises in the header:

- Advertise a realistic v6 font cell size — **font width/height** in header bytes 0x26/0x27 — and **screen width/height in units** (0x22–0x25) = `cols × font_w` by `rows × font_h`. (v5 currently advertises font 1×1 / units = chars — correct for v5, wrong for v6 where pictures are real-pixel-sized.)
- **Text window rects:** pixel `(x, y, w, h)` → cell rect by integer division by `font_w`/`font_h` (cell-quantize).
- **Pictures:** authored in real pixels; drawn into their window's cell rect, scaled by the existing `render/graphics.rs` image path (which already fits a canvas to a cell area).
- Exact font values are a tuning parameter (Infocom v6 art was authored ~640×400); pick values that make Zork0's proportions read correctly and document them. Picture *scaling* (window size vs picture size) is handled by the render fit, so the font choice mainly affects text-window granularity.

## Non-goals (Phase 2 / 3)

- `set_margins`, per-window left/right margins (property 6/7 stored but not laid out).
- Mouse (`read_mouse`/`mouse_window`), menus (`make_menu`), `print_form`.
- Newline-interrupt routines (property 8/9 stored, interrupt not fired).
- `scroll_window` fine behavior beyond a basic clear/scroll.
- Real-pixel overlap / z-ordered compositing (explicitly rejected — SQ-0186 note; cell-text-wins tiling only).
- Sound (Phase 2 shares the Blorb path but is separate).

## Design

### 1. Advertise v6 screen + font dimensions (`crates/zvm/src/screen.rs`)

Extend `write_screen_dims` with a v6 arm: font width/height = the chosen cell size; screen units = `cols × font_w` by `rows × font_h`. v1–5 arms unchanged. This is the header substrate the game's pixel math reads.

### 2. Multi-window model (`crates/zvm/src/screen.rs`)

Introduce a v6 window table — `windows: [ZWindow; 8]` — where each `ZWindow` carries: pixel rect `(x, y, w, h)`, a text grid (reuse/generalize `UpperWindow`), cursor `(row, col)` or pixel cursor, per-window colour/style/font, and the property fields `get_wind_prop`/`put_wind_prop` expose. `ScreenState` gains this table **only in the v6 path**; v1–5 continue to use the existing `upper: UpperWindow` + lower-window flags **byte-identically** (the constraint: no v3–8 regression). Window 0 is the buffered scrolling main window; windows 1–7 are grid/positioned.

The cleanest structure: a v6-only sub-struct on `ScreenState` (e.g. `v6: Option<V6Windows>`), populated only when `version == 6`, so every v1–5 code path is untouched and the compiler proves the isolation.

### 3. Window opcode v6 branches (`crates/zvm/src/cpu/exec.rs`)

Give these version-6 behavior (v1–5 arms unchanged):
- `split_window`/`set_window`/`erase_window` (VAR:0x0A/0x0B/0x0D) — operate on the indexed window table (window numbers 0–7), not the 0/1 + sentinel scheme.
- `set_cursor` (VAR:0x0F) — v6 3-operand form (window + pixel/char cursor).
- `set_colour` (2OP:0x1B) / `set_true_colour` (EXT:0x0D) — v6 window-operand form; colour is per-window.
- `move_window` (EXT:0x10), `window_size` (EXT:0x11), `window_style` (EXT:0x12) — set the window's rect/attributes.
- `erase_picture` (EXT:0x07) — clear a region of a graphics window (event).
- `scroll_window` (EXT:0x14) — basic scroll/clear of a window's grid.

### 4. `get_wind_prop` / `put_wind_prop` over real geometry (`crates/zvm/src/cpu/exec.rs`)

Back EXT:0x13/0x19 with the window table. Implement the ZMSD §8.4.3 window-property indices (0–15) — **verified against the ZMSD 1.1 during implementation, not memory** (per the verify-constants rule; Phase 0's EXT-signature verification is the precedent). At minimum the indices the games read (0 y-coord, 1 x-coord, 2 y-size, 3 x-size, 4 y-cursor, 5 x-cursor, 9 interrupt-countdown, 10 text-style, 11 colour, 12 font, 15 line-count) must return real values; the rest read/write the stored field.

### 5. Picture-dimension table + graphics events — the zero-dep boundary (`crates/zvm` + app)

**`zvm` stays zero-dependency.** It cannot parse Blorb or decode images. Mirroring the Glulx `gvm`→`AppGlk` split:

- **App builds a picture-dimension table** at story open — picture number → `(width, height)` in pixels — as a `Vec<(u32, u16, u16)>` / map, and **injects it into the `Machine` as a constructor parameter** (zvm holds plain numbers, stays zero-dep).
  - **Dimensions come from a cheap image-header sniff, not the `blorb` crate** (which exposes no width/height): use the `image` crate's reader `into_dimensions()` (reads only the PNG IHDR / JPEG SOF header — no full pixel decode). This is what makes the table cheap to build for a picture-heavy game (Zork0 has ~500 Pict resources; only a header read each).
  - **Injection must happen before the boot run.** `picture_data` is called during boot, and the VM runs to its first prompt *inside* `GameSession::new` (the Phase 0 boot lesson) — so the table is a `new_with_trace(...)` parameter set before the boot loop, exactly as `honor_game_colours`/`sound_available` are. The app must therefore resolve the Blorb + build the table *before* constructing the session.
  - **The Pict source is the story's own Blorb when the story is itself a Blorb.** Zork0 ships as `Zork0.blb` (Exec + Pict): the app extracts the `.z6` executable *and* retains the same Blorb for its Pict resources (mirroring how a Glulx `.gblorb` feeds both). `resolve_resource_blorb` targets sidecars (no-Exec Blorbs), so the self-blorb case needs explicit handling.
- **`zvm` answers `picture_data` (EXT:0x06)** from that injected table (width/height into the game's array, branch on availability) — this is what fixes the text-layout underflow.
- **`zvm` implements `draw_picture` (EXT:0x05) by emitting an event** — `(picture_number, window, cell_x, cell_y)` — into a per-turn draw list the app drains (a `Vec<PictureDraw>` on `Machine`, analogous to `pending_sounds`). `erase_picture` emits a clear event.
- **App resolves + decodes** the picture from Blorb at draw time into a `Canvas`/`GraphicsWindow` (reusing the `blorb` crate, `image` crate, and the existing `render/graphics.rs` path), keyed to the target window.
- `picture_table` (EXT:0x1C) sets the adaptive-picture remap table (stored; consulted when resolving a picture number) — minimal support: store and honor the mapping for `draw_picture`/`picture_data`.

### 6. Adapter → z-ordered layered render (`crates/app/src/session.rs`, `render/screen.rs`)

**Correction to the original tiling assumption (2026-07-21, grounded in the Zork0 geometry):** v6 windows OVERLAP — window 7 (graphics) is a *full-screen background* (640×192px = the whole screen), with the text body (window 0) and top strip (window 1) composited on top. This cannot be a non-overlapping `Pair` tree. v6 renders as a **z-ordered layered composite (cell-text-wins)**, not a tiling tree.

The `screen()` adapter (a `GameSession` method, so it can reach both `machine.screen.v6` and the per-window canvases) emits, for a v6 story, an ordered list of positioned windows — each carrying its **absolute cell rect** (pixel rect ÷ font size) and its leaf kind (Graphics / Grid / Buffer). This is a new `WinNode::Layered(Vec<PositionedWindow>)` variant (or an equivalent v6 render path); `render_node` gains a `Layered` arm that draws each entry in order at its absolute rect:
- **z-order:** graphics windows first (background), then text windows on top. Order = window number (0 body, then 1 strip) after the graphics layer, or the game's stacking if a clearer signal exists.
- **cell-text-wins:** a text grid paints only its **non-blank** cells (blank/space cells stay transparent so the graphics layer shows through the gaps); the buffer (window 0) paints its text; graphics paint via `render_graphics_as_cells`/`GraphicsRender` (already skip empty canvas regions).
- windows with zero size are skipped.

Set `content_size` nonzero so v6 leaves the Z-machine simple path.

### 7. App-side picture resolution + render (`crates/app`)

At story open (Plan 1a, done): dimension table injected, `zcode_pict_source` retained. **Plan 1b per turn:** drain zvm's `pending_pictures`, resolve each `PictureEvent` via `zcode_pict_source` (`PictSource::image`) and composite it into the target window's `Canvas` (a new per-v6-window `HashMap<u8, Canvas>` on `GameSession`, mirroring `AppGlk.graphics`; bump `version` for the render cache), exactly as `AppGlk::graphics_draw_image` does. The layered adapter (§6) then emits a `WinNode::Graphics` for each window with a canvas, rendered via the existing `render/graphics.rs` (kitty/sixel + cell fallback). The graphics *rendering* reuses `render/graphics.rs` wholesale; the new code is the **layered composite arm** in `render_node` and the per-window canvas store + draw handler.

### 8. Text routing

`print` goes to the current window: window 0 → the scrolling buffer (existing transcript path); windows 1–7 → the window's grid at its cursor (existing upper-window-grid write path, generalized per-window). The adapter surfaces window 0 as `Buffer` (content from `state.transcript`) and the grids as `Grid`.

## Testing strategy

| Layer | Test |
|-------|------|
| Header | v6 advertises font size + pixel screen units; v1–5 unchanged |
| Window model | split/set/erase/move/size on the 8-window table; v1–5 byte-identical (regression guard) |
| get/put_wind_prop | real geometry round-trips; property indices verified vs ZMSD |
| Pixel→cell | pixel rect quantizes to expected cell rect at the advertised font size |
| picture_data | returns injected dimensions; Zork0 layout math no longer underflows (headless assert on a window_size that previously went 65531) |
| draw_picture events | zvm emits the expected (num, window, x, y) draw list; zero-dep held |
| Adapter | v6 window table → expected `WinNode` tree (grids + graphics), cell-quantized |
| **Headless smoke** | Zork0 boots + a few turns: window geometry is sane (no underflow), the expected picture-draw events fire, the WinNode tree has the right shape |
| **TTY smoke (user)** | Zork0 renders a recognizable title screen / bordered layout with room pictures; text in the right regions |

The headless smoke is a strong oracle for everything except the final image render (which needs a real terminal) — same split as Phase 0.

## Risks & mitigations

- **v1–5 regression** (highest risk — the window opcodes are shared). Mitigation: v6 state in an `Option<V6Windows>` populated only for v6; every v1–5 arm untouched; a byte-identical v3–5 regression guard in the suite.
- **Font-size tuning** — wrong font cell size makes Zork0's proportions look off. Mitigation: document the chosen value; the TTY smoke confirms; it's one constant to adjust.
- **Picture scaling fidelity** — capped by cell granularity (accepted per cell-text-wins). Room art will be approximate, not pixel-exact.
- **ZMSD property-index errors** — a wrong index desyncs a game's layout math. Mitigation: verify the property table against ZMSD 1.1 (Phase 0 precedent).
- **Blorb Pict parsing for zvm stories** — the app's blorb path was built for Glulx; Phase 1 must load `Zork0.blb` (Exec + Pict), extract the `.z6`, AND retain that same Blorb as the Pict source (the self-blorb case `resolve_resource_blorb` doesn't cover). The Phase 0 oracle used the bare `.z6`; Phase 1 needs the Blorb for pictures. `PictSource` (app `graphics.rs`) already decodes+caches Pict resources and can supply both the dimension table (via a header-sniff variant) and the draw-time pixels.

## Cross-crate / constraints

- **`zvm` stays zero-dependency** — it emits events + holds a numeric dimension table; ALL Blorb/image work is app-side. This is the load-bearing constraint (approved 2026-07-21).
- **Additive to v3–8** — v1–5/v7/v8 screen behavior byte-identical; v6 is gated.
- Reuse the existing `render/graphics.rs`, `blorb`, and `image` machinery (no new render/deps).
- No back-compat concerns (pre-release).

## Definition of done

1. Headless suite green (window model, geometry, get/put_wind_prop, picture_data/draw events, adapter tree); `cargo test -p zvm -p app`.
2. `zvm` still zero-dep; v3–5 regression guard passes; clippy clean.
3. Zork0 headless: no geometry underflow, expected picture-draw events, correct WinNode tree shape.
4. Zork0 TTY (user smoke): recognizable layout + pictures render, text in the right regions.
5. SQ-0186 noted Phase 1 complete; Phase 2 (full graphics/sound/margins/mouse) scoped next.
