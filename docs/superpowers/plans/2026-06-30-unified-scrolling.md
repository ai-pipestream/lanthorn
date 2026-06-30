# Unified Scrolling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every linearly-scrollable surface in the app a scrollbar, mouse-wheel support, PageUp/PageDown, and consistent animated smooth-scroll — built on three shared helpers (one scrollbar drawer, one `ListScroll` state machine, one `wheel_delta`) so no copy-pasted scroll idiom remains. Plus: the story picker gains PageUp/Down + animation + a scrollbar.

**Architecture:** Factor the existing ratatui `Scrollbar` idiom into `render::scroll::draw_scrollbar`; generalize the transcript's `ScrollAnim` into a reusable animated-offset type; build a `ListScroll` (selection index + animated display offset) that replaces the ad-hoc `selected:usize` + per-frame window recompute in every selection-list modal; consolidate wheel handling onto one `wheel_delta` helper. The MAP window is exempt (2-D pan, not linear scroll).

**Tech Stack:** Rust, ratatui, crossterm; existing `crates/app/src/anim.rs` (`Tween`/`ease`/`lerp`/`Easing`).

**Spec:** `docs/superpowers/specs/2026-06-30-unified-scrolling-design.md`.

## Global Constraints

- **Reduce replication (explicit user requirement):** exactly one scrollbar drawer, one `ListScroll`, one animated-offset type, one `wheel_delta`. No copy-pasted scrollbar idiom or bespoke selection-window recompute may remain in any adopting surface. The final whole-branch review checks for leftover duplication.
- **Map window is exempt** (`render/map.rs`): 2-D pan/zoom, not linear scroll — do not add a scrollbar/paging/`ListScroll` to it; leave its wheel=pan/zoom + arrow-pan unchanged.
- Scrollbars use the existing `scrollbar` style selector (`colors.rs` `scrollbar` field, `style.rs` selector). No new hard-coded styles (styleable-UI policy).
- Animation honors `[animation]` config: `enabled` (default true), `easing` (default ease-out), `scroll_ms` (default 120; 0 = instant). With `enabled=false` or `scroll_ms=0` the path is byte-for-byte the old instant behavior.
- `mouse_wheel_invert` is applied exactly once (today at `input.rs:851`); the picker/hints intercepts must not double-invert.
- Z-machine/Glulx behavior unaffected; existing keybindings + Shift-reverse conventions unchanged.
- 0 warnings (`cargo build --workspace`, `cargo doc --no-deps --workspace`); full `cargo test --workspace` green per task; TDD; one commit per task on the wave's worktree branch; no push; do not edit `TODO.md`.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

> **Anchors:** line numbers below are from the spec's inventory and may have shifted slightly (the score-bar + BeyondZork-fix merges touched `transcript.rs`/`main.rs`). Re-locate each symbol by name before editing; the names are stable.

---

## Task 1: Shared scrollbar drawer (`render::scroll`)

**Files:**
- Create: `crates/app/src/render/scroll.rs`
- Modify: `crates/app/src/render/mod.rs` (add `pub mod scroll;`)
- Modify: `crates/app/src/render/transcript.rs` (replace the inline scrollbar block ~`1075-1104`)
- Modify: `crates/app/src/render/style_editor.rs` (replace the inline scrollbar block ~`201-218`)

**Interfaces:**
- Produces: `pub fn draw_scrollbar(buf: &mut Buffer, area: Rect, total: usize, viewport: usize, position: usize, style: Style)` and `pub fn needs_scrollbar(total: usize, viewport: usize) -> bool`.

- [ ] **Step 1: Write the failing test** (in `render/scroll.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};

    #[test]
    fn needs_scrollbar_only_when_overflowing() {
        assert!(!needs_scrollbar(5, 10));   // fits
        assert!(!needs_scrollbar(10, 10));  // exactly fits
        assert!(needs_scrollbar(11, 10));   // overflows
    }

    #[test]
    fn draw_scrollbar_noop_when_fits_and_draws_when_overflowing() {
        let area = Rect::new(0, 0, 8, 4);
        // fits -> nothing drawn on the right edge
        let mut b1 = Buffer::empty(area);
        draw_scrollbar(&mut b1, area, 4, 4, 0, Style::default());
        let right_col_blank = (0..area.height)
            .all(|y| b1.cell((area.right() - 1, y)).unwrap().symbol() == " ");
        assert!(right_col_blank, "no scrollbar when content fits");
        // overflows -> the right column has non-space scrollbar glyphs
        let mut b2 = Buffer::empty(area);
        draw_scrollbar(&mut b2, area, 40, 4, 0, Style::default());
        let any_glyph = (0..area.height)
            .any(|y| b2.cell((area.right() - 1, y)).unwrap().symbol() != " ");
        assert!(any_glyph, "scrollbar drawn when content overflows");
    }
}
```

- [ ] **Step 2: Run the test, verify it fails** — `cargo test -p app scroll::tests` → FAIL (module/functions absent).

- [ ] **Step 3: Implement `render/scroll.rs`** (lift the idiom verbatim from `transcript.rs:1075-1104`):

```rust
//! Shared vertical-scrollbar drawing. The single place the ratatui `Scrollbar`
//! idiom lives; every linearly-scrollable surface calls `draw_scrollbar`.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};

/// True when `total` rows do not fit in `viewport` rows (so a scrollbar — and a
/// reserved 1-column gutter — are warranted).
pub fn needs_scrollbar(total: usize, viewport: usize) -> bool {
    total > viewport
}

/// Draw a themed vertical scrollbar on the right edge of `area`. No-op when the
/// content fits. `position` is the index of the first visible row (0-based).
pub fn draw_scrollbar(buf: &mut Buffer, area: Rect, total: usize, viewport: usize, position: usize, style: Style) {
    if !needs_scrollbar(total, viewport) || area.height == 0 || area.width == 0 {
        return;
    }
    let mut sb_state = ScrollbarState::new(total)
        .viewport_content_length(viewport)
        .position(position);
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .style(style)
        .render(area, buf, &mut sb_state);
}
```
(Use the exact `Scrollbar` construction the two existing call sites use; copy their orientation/symbol/style choices so behavior is identical. `Scrollbar::render` needs `ratatui::widgets::StatefulWidget` in scope — match the existing import.)

- [ ] **Step 4: Migrate `transcript.rs`** — replace the inline scrollbar block (~1075-1104) with a `crate::render::scroll::draw_scrollbar(buf, <scrollbar_area>, total_rows, transcript_rows, start, state.colors.scrollbar)` call, preserving the same area/total/viewport/position it computed. Delete the now-unused local `ScrollbarState`/`Scrollbar` imports if they become unused.

- [ ] **Step 5: Migrate `style_editor.rs`** — replace the inline block (~201-218) with the same helper call, using its existing total/viewport/position and `state.colors.scrollbar`. Keep the gutter-reserve logic (`scrollbar_visible`) but have it call `needs_scrollbar`.

- [ ] **Step 6: Verify + commit** — `cargo test --workspace` green, `cargo build --workspace` + `cargo doc --no-deps --workspace` 0 warnings. Commit: `refactor(app): factor the scrollbar idiom into render::scroll::draw_scrollbar`.

---

## Task 2: Generalize `ScrollAnim` (offset-based animated scroll)

**Files:**
- Modify: `crates/app/src/state.rs` (`ScrollAnim` ~`331-345`, `scroll_transcript_to` ~`1069-1089`, `effective_transcript_scroll` ~`1094-1098`, `has_active_animation` ~`1061-1063`)
- (Possibly relocate `ScrollAnim` next to `anim.rs` types — keep it in `state.rs` if simpler; the goal is one type reused, not relocation for its own sake.)

**Interfaces:**
- Produces: a reusable `ScrollAnim` operating on a `usize` offset target:
  `ScrollAnim::to(from: usize, to: usize, cfg: &AnimationConfig) -> Option<Self>` (None when animation disabled → caller jumps instantly), `current(&self) -> f64`, `target(&self) -> usize`, `done(&self) -> bool`. The transcript continues to use it; `ListScroll` (Task 3) reuses it.

- [ ] **Step 1: Write a failing test** (in `state.rs` tests) capturing the generalized contract — instant when disabled, interpolating when enabled:

```rust
#[test]
fn scroll_anim_instant_when_disabled() {
    let cfg = AnimationConfig { enabled: false, easing: Easing::EaseOut, scroll_ms: 120 };
    assert!(ScrollAnim::to(0, 10, &cfg).is_none(), "disabled animation arms nothing");
}

#[test]
fn scroll_anim_interpolates_then_settles() {
    let cfg = AnimationConfig { enabled: true, easing: Easing::Linear, scroll_ms: 40 };
    let a = ScrollAnim::to(0, 10, &cfg).expect("armed");
    assert_eq!(a.target(), 10);
    let c = a.current();
    assert!((0.0..=10.0).contains(&c), "current within range during ease: {c}");
}
```

- [ ] **Step 2: Run, verify it fails** (signature/behavior mismatch with the transcript-only version).

- [ ] **Step 3: Implement** — generalize `ScrollAnim` to `{ from: usize, to: usize, tween: Tween }` with the constructor returning `None` when `!cfg.enabled || cfg.scroll_ms == 0`; `current()` = `lerp(from as f64, to as f64, tween.progress())`; `target()` = `to`; `done()` = `tween.done()`. Rewrite `scroll_transcript_to` to use `ScrollAnim::to(current_display, target, cfg)` (instant jump + clear when `None`), and `effective_transcript_scroll` to round `current()`.

- [ ] **Step 4: Verify the transcript still behaves identically** — keep/extend the existing transcript-scroll tests; they must stay green. `cargo test --workspace`.

- [ ] **Step 5: Commit** — `refactor(app): generalize ScrollAnim to a reusable animated offset`.

---

## Task 3: `ListScroll` state machine

**Files:**
- Create: `crates/app/src/list_scroll.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod list_scroll;` or `mod`)

**Interfaces:**
- Consumes: `ScrollAnim` (Task 2), `AnimationConfig`, `page_scroll` (`input.rs:1366` — make it `pub(crate)` if not already, or duplicate its tiny math here and have `input.rs` call this one — prefer the latter to keep one paging helper).
- Produces: `ListScroll` per the spec signatures (`new`, `len`, `select`, `move_by`, `page`, `home`, `end`, `display_offset`, `target_offset`, `has_active_animation`, `finalize_if_done`).

- [ ] **Step 1: Write failing tests** (pure logic — the bulk of the coverage):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::Easing;
    fn anim_off() -> AnimationConfig { AnimationConfig { enabled: false, easing: Easing::EaseOut, scroll_ms: 0 } }

    #[test]
    fn move_clamps_and_keeps_selection_visible() {
        let mut l = ListScroll::new();
        l.len(20);
        l.move_by(5, 5, &anim_off());          // select 5, viewport 5
        assert_eq!(l.selected, 5);
        // offset advanced so 5 is visible in a 5-row viewport (rows 1..=5)
        assert!(l.target_offset() <= 5 && 5 < l.target_offset() + 5);
        l.move_by(-100, 5, &anim_off());       // clamp to 0
        assert_eq!(l.selected, 0);
        assert_eq!(l.target_offset(), 0);
    }

    #[test]
    fn page_moves_by_a_viewport() {
        let mut l = ListScroll::new();
        l.len(100);
        l.page(1, 10, &anim_off());            // PageDown
        assert!(l.selected >= 9);              // ~one page (with overlap)
        let before = l.selected;
        l.page(-1, 10, &anim_off());           // PageUp
        assert!(l.selected < before);
    }

    #[test]
    fn home_end_jump_to_bounds() {
        let mut l = ListScroll::new();
        l.len(50);
        l.end(50, 10, &anim_off());
        assert_eq!(l.selected, 49);
        l.home(10, &anim_off());
        assert_eq!(l.selected, 0);
        assert_eq!(l.target_offset(), 0);
    }

    #[test]
    fn instant_when_animation_disabled() {
        let mut l = ListScroll::new();
        l.len(100);
        l.move_by(40, 10, &anim_off());
        assert_eq!(l.display_offset(), l.target_offset(), "no easing when disabled");
        assert!(!l.has_active_animation());
    }
}
```

- [ ] **Step 2: Run, verify failures.**

- [ ] **Step 3: Implement `ListScroll`** — fields `selected: usize`, `offset: usize`, `anim: Option<ScrollAnim>`. Each movement clamps `selected` to `[0, total-1]`, then computes the minimal `offset` change so `selected ∈ [offset, offset+viewport)` ("ensure visible"), and arms `ScrollAnim::to(prev_display_offset, offset, cfg)` (instant when `None`). `page` reuses the `page_scroll` math (one viewport with 1-row overlap). `display_offset()` rounds `anim.current()` or returns `offset` when no anim; `target_offset()` returns `offset`; `has_active_animation()` = `anim.as_ref().map_or(false, |a| !a.done())`; `finalize_if_done()` drops a completed anim.

- [ ] **Step 4: Verify + commit** — `cargo test -p app list_scroll`. Commit: `feat(app): add ListScroll (selection + animated display offset)`.

---

## Task 4: `wheel_delta` helper + consolidate wheel handling

**Files:**
- Modify: `crates/app/src/input.rs` (`mouse_to_action` ~`834`, invert site ~`851`, precedence branch ~`864-893`; add config screen)
- Modify: `crates/app/src/main.rs` (story-picker intercept ~`907`, hints intercept ~`1839`)

**Interfaces:**
- Produces: `pub(crate) fn wheel_delta(kind: crossterm::event::MouseEventKind, invert: bool) -> Option<isize>` (ScrollUp → -1, ScrollDown → +1, swapped when `invert`, `None` for non-wheel).

- [ ] **Step 1: Failing test** (in `input.rs` tests):

```rust
#[test]
fn wheel_delta_maps_and_inverts_once() {
    use crossterm::event::MouseEventKind::*;
    assert_eq!(wheel_delta(ScrollUp, false), Some(-1));
    assert_eq!(wheel_delta(ScrollDown, false), Some(1));
    assert_eq!(wheel_delta(ScrollUp, true), Some(1));
    assert_eq!(wheel_delta(ScrollDown, true), Some(-1));
    assert_eq!(wheel_delta(Moved, false), None);
}
```

- [ ] **Step 2: Run, verify fail.**

- [ ] **Step 3: Implement** `wheel_delta`; refactor `mouse_to_action`'s wheel normalization to call it (keep the single upstream invert). Rewrite the story-picker (`main.rs:907`) and hints (`main.rs:1839`) intercepts to call `wheel_delta(kind, cfg.mouse_wheel_invert)` instead of re-implementing invert. Add the **config screen** to the modal-precedence branch so its wheel drives its nav.

- [ ] **Step 4: Verify + commit** — existing wheel behavior unchanged (manual reasoning + tests). Commit: `refactor(app): single wheel_delta helper; add config-screen wheel`.

---

## Task 5: Adopt `ListScroll` + scrollbar in the in-game modals

**Files (one modal at a time; split into sub-commits if large):** `render/saves.rs` + `SavesState` (`state.rs:382`), `render/filebrowser.rs` + `FileBrowserState` (`state.rs:474`), `render/gallery.rs` + `GalleryState` (`state.rs:395`), `render/history.rs` + `ReplayState` (`state.rs:271`), `render/verbmenu.rs` + `VerbMenuState`, `render/config_screen.rs` + `ConfigScreenState` (`state.rs:643`), `render/hints_panel.rs` + `HintSession` (`state.rs:34`). Nav actions in `input.rs` (`SavesNav`/`FbNav`/`GalleryPrev|Next`/`ReplayStep`/`VerbMenuNav`/`StyleNav`).

**Interfaces:** Consumes `ListScroll` (Task 3), `draw_scrollbar` (Task 1).

- [ ] **Step 1:** For each surface, write a failing render/state test: when items > viewport, a scrollbar is drawn (assert via `draw_scrollbar`'s right-column glyph in a `Buffer`) and PageDown moves `selected` by ~a viewport with the selection staying visible. (Model the test on the per-surface render-test pattern already in that module.)
- [ ] **Step 2:** Run, verify fail.
- [ ] **Step 3:** Replace each surface's bare `selected: usize` + per-frame window recompute with a `ListScroll` field; wire its nav action(s) to `move_by(±1)`/`page`/`home`/`end`; render rows from `display_offset()`; reserve a 1-col gutter + call `draw_scrollbar` when `needs_scrollbar`. Drive the wheel via the Task-4 precedence branch. Keep each surface's existing keys working (Up/Down) and add PageUp/PageDown.
- [ ] **Step 4:** Verify + commit per surface or as one commit: `feat(app): scrollbar + ListScroll (paging/wheel/anim) across modal lists`.

> Reviewer focus: NO bespoke window-recompute or inline scrollbar idiom may remain in any migrated module.

---

## Task 6: Story picker — PageUp/Down + scrollbar + animation

**Files:** `crates/app/src/main.rs` (`run_story_picker` ~`849-935`, `draw_story_picker` ~`939-998`).

**Interfaces:** Consumes `ListScroll`, `draw_scrollbar`, `wheel_delta`.

- [ ] **Step 1:** Write a failing test for a pure helper if extractable (e.g. the picker's visible-window computation now delegated to `ListScroll`); otherwise rely on `ListScroll`'s own tests + a `draw_story_picker` render test asserting a scrollbar appears when entries overflow the rows.
- [ ] **Step 2:** Run, verify fail.
- [ ] **Step 3:** Replace the picker's local `selected` + `first` recompute (`main.rs:974`) with a `ListScroll` (its `Tween` is `Instant`-based — no `AppState` needed). Add `KeyCode::PageUp`/`PageDown` → `page(∓1)`, confirm `Home`/`End`. Draw the shared scrollbar (gutter when overflowing). Drive the wheel via `wheel_delta`. While `list.has_active_animation()`, poll the event loop with a short timeout (e.g. 16-33ms) so the ease renders; finalize when done.
- [ ] **Step 4:** Verify + commit — `feat(app): story picker gains PageUp/Down, scrollbar, animated scroll`.

---

## Task 7: Run-loop animation aggregation

**Files:** `crates/app/src/main.rs` (the anim finalize block ~`1488-1498`), `crates/app/src/state.rs` (`has_active_animation` ~`1061`).

- [ ] **Step 1:** Write a failing test: `AppState::has_active_animation()` returns true when any in-game `ListScroll` (saves/fb/gallery/history/verbmenu/config/hints, whichever are open) has an active animation, not just the transcript.
- [ ] **Step 2:** Run, verify fail.
- [ ] **Step 3:** Extend `has_active_animation` to OR the transcript anim with each live surface's `ListScroll::has_active_animation()`; extend the run-loop finalize to call `finalize_if_done()` on each. Keep the ~30-60fps tick-while-animating behavior.
- [ ] **Step 4:** Verify + commit — `feat(app): drive all surface scroll animations from the run loop`.

---

## Self-review checklist (before final whole-branch review)

- One scrollbar drawer (`render::scroll`), one `ListScroll`, one `ScrollAnim`, one `wheel_delta` — grep confirms no leftover inline `Scrollbar::new(` outside `render/scroll.rs` and no bespoke `selected + first/window` recompute in migrated modules.
- Every linear list surface: scrollbar when overflowing, wheel, PageUp/Down, animated scroll; map window untouched (still pan/zoom); fixed panels untouched.
- Animation honors `[animation]` config (instant when disabled); `mouse_wheel_invert` applied once.
- 0 warnings; full `cargo test --workspace` green; styleable via the `scrollbar` selector.
