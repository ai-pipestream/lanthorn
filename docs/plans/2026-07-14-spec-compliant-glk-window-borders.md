# Spec-compliant Glk window arrangement + borders (SQ-0325)

**Goal:** Render Glk window borders per the spec — a separator *between* two
sibling windows, honoring the `winmethod_Border`/`NoBorder` hint (default =
Border, since `winmethod_Border == 0`), in every split direction, with the
border consuming layout space so `glk_window_get_size` stays honest.

**Root problem being removed:** the app currently *reconstructs* the window
tree from bare leaf rectangles (`glk_backend::assemble`). That reconstruction
cannot recover a pair's border bit for nested layouts and is the shared cause
of two bugs (grid+buffer L/R/B mis-routing → SQ-0325 base fix; and missing
borders). The compliant foundation is to consume gvm's *real* tree.

## Spec basis (verified against Glk-Spec-076 + glk.h)
- Border is *between a window and its sibling* (not a box around a window).
- Honoring the hint is optional (a library may border all/none), but honoring
  it is correct and matches Gargoyle/glkterm.
- `winmethod_Border = 0x000` (default), `winmethod_NoBorder = 0x100`.
- A border is *extra* space: a fixed window keeps its requested size; the
  border is "allowed for already" (comes from the parent/other side).
- Appearance is library-defined; we follow the glkterm text convention: a
  1-cell box-drawing rule.

## Architecture decisions (user: "compliant all the way")
1. gvm reserves **1 cell** for each bordered split (border extra, honoring
   NoBorder). Honest `window_size`.
2. gvm **exposes its real pair tree** (`window_tree()`); the app builds the
   `ScreenModel` directly from it. Delete `assemble`/`bounding_box`.
3. **Every Glulx game** renders through the generic tree path (incl. CM —
   moved off its status box onto the compliant separator). The **Z-machine**
   keeps its byte-identical simple/box path, discriminated by
   `content_size == (0,0)` (Z-machine) vs a real extent (Glulx).

## Tasks (each ends green: `cargo build --tests` + `cargo test`)

### T1 — gvm: border-aware geometry
- `split_rect(rect, method, size)`: when the split has a border, reserve 1
  cell between the children (row for Above/Below, col for Left/Right). Fixed
  window keeps `size`; border extra; other child = total − size − border.
  Proportional: reserve border first, apportion the remainder.
- `axis_splits_exact` / `clean_dims`: subtract the border cell before the
  divisibility check so proportional halves stay equal *with* the border.
- Tests: bordered split reserves a cell; `window_size` reflects it; a bordered
  50% split is equal halves; NoBorder unchanged from today.

### T2 — gvm: expose the real tree
- `pub enum WinTree { Leaf { id, wintype, rect }, Pair { vertical, border,
  split, rect, first, second } }` where `split` = first child's extent along
  the split axis, `border` = presence bool.
- `pub fn window_tree(&self) -> Option<WinTree>` walking from `root`.
- Deliver it across the backend: add `GlkBackend::window_tree(&WinTree)`
  (default no-op so gvm-cli is untouched); gvm calls it beside
  `window_layout` after relayout. `window_layout` (flat leaves) stays for
  mouse/hyperlink hit-testing + gvm-cli.
- Tests: tree shape/border/split for above/below/left/right + nested.

### T3 — app: build the model from the tree
- `WinNode::Pair` gains `border: bool`.
- `AppGlk` stores the delivered `WinTree`; `screen_model` converts it directly
  (leaf → Grid/Buffer/Graphics node, pair → `WinNode::Pair`). Delete
  `assemble`/`bounding_box` + the leaf-loop. `content_size` = tree root extent.
- Update the Z-machine builder (`session.rs`) + all `WinNode::Pair`
  constructions/matches to carry `border`.

### T4 — app: render the separator + reroute Glulx
- `render_node`: reserve the gutter (3-way split) and draw the separator rule
  when `border`; theme it. Update `collect_graphics_rects`/`dialog_bounds`/
  `content_bounds`/`fill_margin` for the gutter.
- `is_simple`: true only for the Z-machine (`content_size == (0,0)`); all
  Glulx → generic. Remove the force-frameless Grid arm (borders now come from
  pairs). Revise the SQ-0303 test to the new semantics (explicit hint honored;
  game-owns-layout preserved for NoBorder).

### T5 — verify
- gvm + app unit tests green; `cargo test --workspace`.
- Headless smokes: CM (status separator, no box), a lone-buffer game, the
  three-window nesting. windowtest border/noborder in each direction.
- User visual smokes (TUI): windowtest `open left/right/below … border|noborder`;
  CM status look; Kerkerkruip panels vs Gargoyle.

## Follow-ups (separate quests)
- Kerkerkruip per-window background shading (darker L/R panels) — not borders.
- gvm-cli honors L/R/B geometry + borders (currently draws buffer full-width).
