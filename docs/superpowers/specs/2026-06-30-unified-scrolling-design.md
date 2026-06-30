# Unified Scrolling — Design

**Date:** 2026-06-30
**Status:** Draft — awaiting user review
**Crate:** `crates/app`

## Goal

Every scrollable surface in the app behaves the same way: it has a **scrollbar**,
responds to the **mouse wheel**, supports **PageUp/PageDown** (plus Home/End where
it makes sense), and scrolls with **animated smooth-scroll** consistent with the
transcript. The story directory picker (item 6) gains PageUp/Down + animation +
a scrollbar. Crucially, this is achieved by **factoring out the replicated
idioms into shared helpers** — after this change there is one scrollbar drawer,
one list-scroll state machine, and one mouse-wheel delta path, not N copies.

Covers TODO high-priority items 6 ("page up/down in Story directory list, use
animated scrolling, as with all scrolling windows") and 7 ("all windows that can
scroll should have a scrollbar; all windows with scrollbars should support mouse
scroll").

## Current state (inventory)

- **Animation primitive** (`anim.rs`): `Tween` (Instant-based, self-contained),
  `ease`/`lerp`/`Easing`, `parse_easing`. Driven by `[animation]` config
  (`enabled`, `easing`, `scroll_ms`; instant when `enabled=false` or
  `scroll_ms=0`).
- **Smooth scroll exists for the transcript only**: `ScrollAnim { from, to, tween }`
  (`state.rs:331`), `AppState.scroll_anim` (`state.rs:752`),
  `scroll_transcript_to` (`state.rs:1069`), `effective_transcript_scroll`
  (`state.rs:1094`), `has_active_animation` (`state.rs:1061`), finalized in the
  run loop (`main.rs:1488`). Every other surface jumps instantly.
- **Scrollbar**: ratatui's `Scrollbar`, drawn with an identical ~30-line idiom in
  exactly two places — `render/transcript.rs:1075` and
  `render/style_editor.rs:201`. Themed via the existing `scrollbar` style selector
  (`colors.rs:234`, `style.rs:255/388`). No other surface has one.
- **Mouse wheel**: one dispatch entry `mouse_to_action` (`input.rs:834`); invert
  applied once upstream (`input.rs:851`); a modal-precedence branch
  (`input.rs:864-893`) routes the wheel to whichever of
  gallery/saves/replay/filebrowser/verbmenu/style-editor is open (each ±1). The
  **config screen** has no wheel support. The **story picker** (`main.rs:907`) and
  **hints panel** (`main.rs:1839`) re-implement their own wheel intercepts
  out-of-band (each re-doing invert locally).
- **Paging helper**: `page_scroll(current, dir, viewport_rows, max_scroll)`
  (`input.rs:1366`) — one-page step with 1-row overlap, clamped.
- **Selection-list modals** (saves, filebrowser, gallery, history, verbmenu,
  config screen) track only a `selected: usize` index and recompute a visible
  window each frame — **no stored display offset**, so they cannot animate.
- **Story picker**: a self-contained loop in `main.rs` (`run_story_picker:849`,
  `draw_story_picker:939`) that runs *before* `AppState` exists; local `selected`
  index, window recomputed each frame; Up/Down/Home/End/Enter/Esc; wheel = ±1
  selection; **no PageUp/Down, no scrollbar, no animation**.

## Design

Three shared building blocks, then per-surface adoption.

### 1. One scrollbar drawer — `render::scroll::draw_scrollbar`

A single function (new module `crates/app/src/render/scroll.rs`) replacing both
existing copies and used by every list surface:

```rust
/// Draw a vertical scrollbar on the right edge of `area`, themed via `style`.
/// No-op when `total <= viewport` (nothing to scroll). `position` is the index
/// of the first visible row (0-based).
pub fn draw_scrollbar(buf: &mut Buffer, area: Rect, total: usize, viewport: usize, position: usize, style: Style);

/// True when a surface of `total` rows in a `viewport` needs a scrollbar (and
/// therefore should reserve a 1-column gutter): `total > viewport`.
pub fn needs_scrollbar(total: usize, viewport: usize) -> bool;
```

Internally the same ratatui idiom (`ScrollbarState::new(total).viewport_content_length(viewport).position(position)`,
`VerticalRight`, `begin_symbol(None).end_symbol(None)`, themed). `transcript.rs`
and `style_editor.rs` are migrated to call it; the inline copies are deleted.

### 2. One list-scroll state machine — `ListScroll`

The core de-duplication. A small struct (in a new `crates/app/src/list_scroll.rs`,
or `anim.rs`-adjacent) that owns a **selection index plus an animated display
offset**, replacing the ad-hoc `selected: usize` + per-frame window recompute in
every selection-list modal:

```rust
pub struct ListScroll {
    pub selected: usize,          // highlighted item
    offset: usize,                // first visible row (the target)
    anim: Option<ScrollAnim>,     // eases the *displayed* offset toward `offset`
}

impl ListScroll {
    pub fn new() -> Self;
    pub fn len(&mut self, total: usize);                 // clamp selected/offset to current item count
    // movement (clamped to [0, total-1]); each keeps `selected` visible in `viewport`
    pub fn select(&mut self, idx: usize, viewport: usize, anim: &AnimationConfig);
    pub fn move_by(&mut self, delta: isize, viewport: usize, anim: &AnimationConfig);   // Up/Down, wheel
    pub fn page(&mut self, dir: i32, viewport: usize, anim: &AnimationConfig);          // PageUp/PageDown (reuses page_scroll math)
    pub fn home(&mut self, viewport: usize, anim: &AnimationConfig);
    pub fn end(&mut self, total: usize, viewport: usize, anim: &AnimationConfig);
    // render-time reads
    pub fn display_offset(&self) -> usize;     // animated (rounded) first-visible row
    pub fn target_offset(&self) -> usize;      // settled offset (for scrollbar position)
    pub fn has_active_animation(&self) -> bool;
    pub fn finalize_if_done(&mut self);        // snap + drop a completed anim (called from the run loop)
}
```

`ScrollAnim` (today private to the transcript) is generalized: lifted to operate
on a `usize` offset target with the same `Tween`-based interpolation, and reused
by both `ListScroll` and the transcript. "Ensure selected visible" computes the
minimal offset change so `selected` is within `[offset, offset+viewport)`, then
eases toward it (instant when `anim.enabled == false || anim.scroll_ms == 0`,
exactly reproducing today's jump). This gives every adopting surface PageUp/Down,
Home/End, wheel, and animation for free.

**Adopting surfaces** (replace their bespoke `selected` + window math with a
`ListScroll`): saves manager, file browser, gallery, history/replay, verb menu,
config screen, hints panel, and the story picker. The transcript keeps its
offset-based scroll but is refactored onto the **generalized `ScrollAnim`** so
there is a single animation type, not two.

### 3. One mouse-wheel delta path

- Keep `mouse_to_action`'s single upstream invert (`input.rs:851`) as the only
  invert site.
- Extend the modal-precedence branch (`input.rs:864-893`) to include the **config
  screen**, and route each open modal's wheel to `ListScroll::move_by(±1)` (via
  its existing per-modal nav action, which now drives `ListScroll`).
- The **story picker** and **hints panel** out-of-band intercepts are rewritten to
  use a shared `wheel_delta(MouseEventKind, invert) -> Option<isize>` helper (so
  invert is applied the same way everywhere, never doubled), feeding their
  `ListScroll`. The picker cannot use `mouse_to_action` (it predates `AppState`),
  but it shares `ListScroll` + `wheel_delta`.

### Story picker (item 6) specifics

`run_story_picker`/`draw_story_picker` adopt a `ListScroll` (its `Tween` is
Instant-based, so no `AppState`/clock plumbing is needed). Add PageUp/PageDown
(and confirm Home/End) keys, draw the shared scrollbar (reserving a 1-col gutter
when overflowing), and animate the display offset. The loop already ticks for
rendering; it polls with a short timeout while `list.has_active_animation()` so
the ease is visible, mirroring the in-game run loop.

### Exemptions (explicitly out of scope)

- **The map window** (`render/map.rs`) is **exempt**. It is a 2-D pan/zoom
  surface (`scroll: (i32,i32)` + `char_pan`), not a linear scroll of
  rows/items, so the vertical scrollbar, PageUp/PageDown paging, and the
  `ListScroll` / animated-offset model **do not apply** to it. Its existing
  mouse-wheel = pan/zoom and arrow / Shift-arrow pan behavior is **unchanged**.
  "Every scrollable surface" in this spec means every surface that scrolls a
  linear list or block of text — the map is not one of those.
- **Fixed, non-scrolling panels** that truncate rather than scroll (e.g. the
  room-info and inspector overlays, the hotkeys dialog) are likewise untouched.
  A surface only adopts the shared scroll machinery if it actually scrolls
  content today or needs to.

### Run-loop integration

`has_active_animation` (`main.rs`/`state.rs`) aggregates the transcript anim plus
each in-game surface's `ListScroll::has_active_animation()` so the render loop
keeps ticking (~30–60fps) while any scroll is easing, and `finalize_if_done` snaps
each when its tween completes (generalizing the current `main.rs:1488` block).

## Testing strategy

- **`draw_scrollbar`/`needs_scrollbar`**: `needs_scrollbar` boundary
  (total<=viewport → false; total>viewport → true); rendering a known
  total/viewport/position writes a thumb at the expected rows (assert against a
  ratatui `Buffer`).
- **`ListScroll`** (the bulk — pure logic, no terminal): move_by/page/home/end
  clamp correctly; "ensure visible" keeps `selected` in the viewport with minimal
  offset change; PageUp/Down matches `page_scroll`; with `anim.enabled=false` the
  display offset jumps instantly (no tween); with animation on, `display_offset`
  interpolates between old and new and `has_active_animation` flips false after
  `scroll_ms`.
- **`wheel_delta`**: ScrollUp/ScrollDown → ∓1 (or ±1) and invert flips it, applied
  exactly once.
- **Per-surface**: each migrated modal renders a scrollbar when its content
  overflows and not otherwise; wheel + PageUp/Down move the selection; the visible
  window still contains `selected`. Story picker: PageUp/Down paging + scrollbar
  presence + selection-stays-visible.
- **No regression**: transcript scroll (the only pre-existing animated surface)
  behaves identically after being moved onto the generalized `ScrollAnim`;
  `mouse_wheel_invert` still applies once.

## Global constraints

- **Reduce replication (explicit user requirement):** exactly one scrollbar
  drawer, one `ListScroll`, one `ScrollAnim` type, one `wheel_delta`. No
  copy-pasted scrollbar idiom or bespoke selection-window recompute remains in any
  adopting surface. The final whole-branch review checks for leftover duplication.
- Scrollbars use the existing `scrollbar` style selector; no new hard-coded styles
  (per the styleable-UI standing policy — any genuinely new UI element gets a
  selector).
- Animation honors `[animation]` config (`enabled`/`easing`/`scroll_ms`); instant
  path is byte-for-byte the old behavior.
- `mouse_wheel_invert` handled once; reverse-direction (Shift) conventions and
  existing keybindings unchanged.
- 0 warnings (`cargo build --workspace`, `cargo doc --no-deps --workspace`); full
  `cargo test --workspace` green per task; TDD; one commit per task on the wave's
  worktree branch; no push; do not edit `TODO.md` mid-wave.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

## Decomposition (for the plan)

1. **Shared scrollbar drawer** (`render/scroll.rs`) + migrate the 2 existing
   call sites. Self-contained, low risk.
2. **Generalized `ScrollAnim`** lifted out of the transcript; transcript
   refactored onto it (behavior-identical). Foundation for `ListScroll`.
3. **`ListScroll`** state machine + unit tests (pure logic).
4. **`wheel_delta` helper** + fold the picker/hints intercepts and the
   `input.rs` precedence branch onto it; add the config screen.
5. **Adopt `ListScroll` + scrollbar in the in-game modals** (saves, filebrowser,
   gallery, history, verbmenu, config screen, hints panel) — can be split per
   surface if a task grows large.
6. **Story picker**: PageUp/Down + scrollbar + animation via `ListScroll`.
7. **Run-loop aggregation** of `has_active_animation`/`finalize` across surfaces.
