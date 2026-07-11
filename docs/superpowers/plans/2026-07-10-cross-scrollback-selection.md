# SQ-0197 — Cross-scrollback story-pane text selection

**Goal:** Let a left-drag in the story pane select and copy text that extends beyond the
visible viewport, and keep a selection correct when the transcript scrolls. Today selection
is stored in screen cell coords and copy reads only the visible ratatui buffer, so nothing
off-screen is representable.

**Approach:** Move the selection to **stable absolute wrapped-row coordinates**, extract the
copy from the **transcript backing store** (the full wrapped rows), highlight the visible
portion during render, and **auto-scroll** the transcript when a drag reaches the pane's top
or bottom edge.

**Crate:** only `crates/app`. Zero-dep VM crates untouched.

**Commit trailers (every commit):**
```
Quest: SQ-0197
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
```
Branch off `main`: `sq-0197-cross-scrollback-selection`. Stage only edited source files by
path (the tree has pre-existing untracked files — never `git add -A`).

---

## Key facts established from the code (do not re-derive)

- Selection struct today: `crates/app/src/clipboard.rs:12` `Selection { anchor:(u16,u16), head:(u16,u16) }` (screen cells). Extraction reads the visible `Buffer` (`clipboard.rs:66` `extract`, `clipboard.rs:96` `highlight_and_extract`).
- Selection state: `crates/app/src/state.rs:1096` `pub selection: Option<crate::clipboard::Selection>`.
- Mouse mapping (already gated to the story pane): `input.rs:1031-1043` → `Action::StartSelection(col,row)`, `ExtendSelection(col,row)`, `EndSelection`. Action handlers: `input.rs:2645-2661`.
- Copy on release reads `last_panes.selection_text` and writes OSC 52 to the terminal: `main.rs:3473` area. Post-render highlight+extract stash: `main.rs:888-901` (`selection_text_out = highlight_and_extract(buf, sel_area, sel)`).
- Transcript scroll: `state.rs:1023` `pub transcript_scroll: u16` (rows up from bottom; 0 = pinned bottom). `effective_transcript_scroll()` `state.rs:1616`, `scroll_transcript_to()` `state.rs:1607`.
- The visible-window function: `render/transcript.rs:441` `visible_wrapped_lines_kinded(...) -> (Vec<WrappedRow>, usize /*total*/)`. Internally it wraps ALL rows via `wrap_lines_kinded(...)` (`transcript.rs:457`) then slices `display_rows[start..end]`. In the normal path `start = n - scroll_clamped - rows` (mirror of the scrollbar calc at `transcript.rs:1232`); in the clear-anchor path (`transcript.rs:466-487`) `start = anchor_row`, `end = n`. **The absolute index of the top visible wrapped row is `start`.**
- `WrappedRow` (`transcript.rs:21`) exposes `pub text: String` — the plain text of each wrapped row (story band, flush left). Column == char index within `text` (the hyperlink code at `transcript.rs:1211-1226` relies on exactly this: char `j` renders at column `text_x + j`).
- render_middle (`transcript.rs:1013`) draws visible row `i` at `row_y = transcript_top + i`, body text region is `body_area` (`transcript.rs:1149`, `= area.width - 1` when a scrollbar gutter is reserved). It is called at `transcript.rs:878` and returns `(bool, u16, Vec<((u16,u16),u32)>)`.

---

## Task 1 — New coordinate model + store extraction in `clipboard.rs`

Replace the screen-coord `Selection` with absolute wrapped-row coords, and make extraction
operate on wrapped-row **texts** (not the buffer).

Add:
```rust
/// A point in the wrapped transcript: absolute wrapped-row index + 0-based column
/// within the story band. Stable across scrolling (unlike screen cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point { pub row: usize, pub col: u16 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection { pub anchor: Point, pub head: Point }

/// Per-frame transcript geometry stashed by render so the mouse handlers and the
/// copy path can map screen cells ↔ absolute wrapped rows. `area` is the body text
/// region (scrollbar gutter already excluded); `first_abs_row` is the absolute
/// wrapped-row index drawn at `area.y`.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptGeom { pub area: Rect, pub first_abs_row: usize, pub total_rows: usize }

impl Selection {
    pub fn new(at: Point) -> Self { Selection { anchor: at, head: at } }
    pub fn is_empty(&self) -> bool { self.anchor == self.head }
}

fn ordered(a: Point, b: Point) -> (Point, Point) {
    if (a.row, a.col) <= (b.row, b.col) { (a, b) } else { (b, a) }
}

/// Inclusive column span [c0,c1] selected on absolute wrapped `row`, within a story
/// band `width` cells wide (0-based). `None` if `row` is outside the selection.
pub fn row_span(width: u16, sel: Selection, row: usize) -> Option<(u16, u16)> {
    if width == 0 { return None; }
    let (s, e) = ordered(sel.anchor, sel.head);
    if row < s.row || row > e.row { return None; }
    let last = width - 1;
    let c0 = if row == s.row { s.col.min(last) } else { 0 };
    let c1 = if row == e.row { e.col.min(last) } else { last };
    if c0 > c1 { return None; }
    Some((c0, c1))
}

/// True if absolute cell (row, col) is inside the selection.
pub fn contains(width: u16, sel: Selection, row: usize, col: u16) -> bool {
    match row_span(width, sel, row) { Some((c0, c1)) => col >= c0 && col <= c1, None => false }
}

/// Extract the selected text from the full set of wrapped-row texts. `rows[i]` is the
/// plain text of absolute wrapped row `i`. Rows joined with `\n`, trailing ws trimmed.
pub fn extract(rows: &[&str], width: u16, sel: Selection) -> String {
    let (s, e) = ordered(sel.anchor, sel.head);
    let mut out: Vec<String> = Vec::new();
    for row in s.row..=e.row {
        if let Some((c0, c1)) = row_span(width, sel, row) {
            let chars: Vec<char> = rows.get(row).copied().unwrap_or("").chars().collect();
            let mut line = String::new();
            for c in c0..=c1 { if let Some(ch) = chars.get(c as usize) { line.push(*ch); } }
            out.push(line.trim_end().to_string());
        }
    }
    out.join("\n")
}
```
Keep `base64_encode` + `osc52_copy_sequence` unchanged. **Delete** `row_span(area,...)` (old
screen-coord), `contains(area,...)`, `extract(buf,...)`, and `highlight_and_extract(...)` —
they are replaced. Update the module doc comment.

**Rewrite the clipboard tests** for the new model:
- `ordered` normalizes reversed points.
- `row_span`: first/last/middle rows; single-row; empty when out of range; width clamp.
- `contains`: inside/outside a multi-row selection.
- `extract`: single-row substring; multi-row (first row from `c0`, middle rows full width, last row to `c1`); trailing-ws trimmed; a `rows` index past the end yields empty for that row.
- `is_empty` when anchor==head.
- Keep `base64_known_vectors` and `osc52_wraps_base64_in_escape`.

**Build gate:** `cargo test -p app clipboard::`.

---

## Task 2 — Stash geometry + highlight + extract in `render/transcript.rs`

1. Change `visible_wrapped_lines_kinded` to also return `first_abs_row`:
   signature `-> (Vec<WrappedRow>, usize, usize)` returning `(window, n, start)`.
   - Normal path: `start` is the already-computed `start` (`transcript.rs:493`); return `(display_rows[start..end].to_vec(), n, start)`.
   - Clear-anchor path (`transcript.rs:483`): return `(display_rows[anchor_row..n].to_vec(), n, anchor_row)`.
   - Empty early-return: `(Vec::new(), 0, 0)`.
   - Update the two unit tests that destructure it (`transcript.rs:1715,1728,1732`) to bind the third value (`_first` / assert where obvious).

2. In `render_middle`, capture the third return value:
   `let (lines, total_rows, first_abs_row) = visible_wrapped_lines_kinded(...)`.

3. After the `for (i, wr) in lines.iter().enumerate()` draw loop (`transcript.rs:1185-1227`),
   stash geometry and do selection highlight + extraction. The body text region is `body_area`;
   selection columns are 0-based from `body_area.x`. Add:
   ```rust
   // Publish this frame's transcript geometry so the mouse handlers and the copy
   // path can map screen cells ↔ absolute wrapped rows. (SQ-0197)
   state.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
       area: body_area,
       first_abs_row,
       total_rows,
   }));
   if let Some(sel) = state.selection {
       let width = body_area.width;
       // Highlight the visible portion (reverse video) by absolute row.
       for (i, _wr) in lines.iter().enumerate() {
           let row_y = transcript_top + i as u16;
           if row_y >= transcript_bottom { break; }
           let abs = first_abs_row + i;
           for col in 0..width {
               if crate::clipboard::contains(width, sel, abs, col) {
                   if let Some(cell) = buf.cell_mut((body_area.x + col, row_y)) {
                       let s = cell.style();
                       cell.set_style(s.add_modifier(ratatui::style::Modifier::REVERSED));
                   }
               }
           }
       }
       // Extract the copy from the FULL wrapped set (off-screen rows included).
       if sel.is_empty() {
           *state.selection_text.borrow_mut() = None;
       } else {
           let all = wrap_lines_kinded(&filtered_lines, &filtered_kinds, &filtered_styles,
               &filtered_runs, &filtered_images, char_px, images_enabled, body_area.width);
           let texts: Vec<&str> = all.iter().map(|r| r.text.as_str()).collect();
           *state.selection_text.borrow_mut() = Some(crate::clipboard::extract(&texts, width, sel));
       }
   }
   ```
   (`wrap_lines_kinded` is `pub(crate)` in this module; the `filtered_*` locals and `char_px`/
   `images_enabled` are already in scope in `render_middle`.)

**Build gate:** `cargo build -p app` and `cargo test -p app transcript::`.

---

## Task 3 — State fields (`state.rs`)

Add to `AppState` (near the existing `selection` field, `state.rs:1096`):
```rust
/// Auto-scroll direction while a story-pane selection drag sits at an edge:
/// -1 = top edge (reveal older), +1 = bottom edge (reveal newer), 0 = interior. (SQ-0197)
pub selection_edge: i32,
/// This frame's transcript geometry, published by render for the mouse/copy paths. (SQ-0197)
pub transcript_geom: std::cell::Cell<Option<crate::clipboard::TranscriptGeom>>,
/// The selection's extracted copy text, published by render, read on mouse-release. (SQ-0197)
pub selection_text: std::cell::RefCell<Option<String>>,
```
Initialize in the constructor(s): `selection_edge: 0`, `transcript_geom: std::cell::Cell::new(None)`,
`selection_text: std::cell::RefCell::new(None)`. (Find the struct-literal initializer that sets
`selection: None` and add the three there.)

**Build gate:** `cargo build -p app`.

---

## Task 4 — Convert mouse handlers + auto-scroll (`input.rs`)

Rewrite the three action handlers (`input.rs:2645-2661`). Add a helper to map a screen cell to
an absolute `Point` using the published geometry, plus edge detection:
```rust
Action::StartSelection(col, row) => {
    state.focus = Focus::Game;
    if let Some(g) = state.transcript_geom.get() {
        if let Some(p) = screen_to_point(g, col, row) {
            state.selection = Some(crate::clipboard::Selection::new(p));
            state.selection_edge = 0;
        }
    }
}
Action::ExtendSelection(col, row) => {
    if let Some(g) = state.transcript_geom.get() {
        if let Some(sel) = &mut state.selection {
            if let Some(p) = screen_to_point(g, col, row) { sel.head = p; }
        }
        // Edge detection for auto-scroll: pointer at/above top → -1; at/below bottom → +1.
        state.selection_edge = if row <= g.area.y { -1 }
            else if row >= g.area.bottom().saturating_sub(1) { 1 }
            else { 0 };
        // Step once now so a drag that reaches the edge scrolls even without a tick.
        apply_selection_autoscroll(state);
    }
}
Action::EndSelection => {
    // Copy is emitted by the run loop from state.selection_text; just clear here.
    state.selection = None;
    state.selection_edge = 0;
}
```
Add these free functions in `input.rs` (near the tidy helpers):
```rust
/// Map a story-pane screen cell to an absolute wrapped-transcript Point, clamped to
/// the visible rows and the story band. `None` if geometry is degenerate. (SQ-0197)
pub(crate) fn screen_to_point(g: crate::clipboard::TranscriptGeom, col: u16, row: u16)
    -> Option<crate::clipboard::Point> {
    if g.area.width == 0 || g.area.height == 0 { return None; }
    let dy = row.saturating_sub(g.area.y).min(g.area.height.saturating_sub(1));
    let abs = (g.first_abs_row + dy as usize).min(g.total_rows.saturating_sub(1));
    let c = col.saturating_sub(g.area.x).min(g.area.width.saturating_sub(1));
    Some(crate::clipboard::Point { row: abs, col: c })
}

/// While a selection drag sits at an edge, scroll one wrapped row toward it and
/// advance the head in lockstep so the selection keeps growing. No-op at scroll
/// limits or when not selecting. (SQ-0197)
pub(crate) fn apply_selection_autoscroll(state: &mut AppState) {
    if state.selection_edge == 0 { return; }
    let Some(g) = state.transcript_geom.get() else { return };
    let max_scroll = g.total_rows.saturating_sub(g.area.height as usize) as u16;
    let cur = state.transcript_scroll;
    let next = if state.selection_edge < 0 { cur.saturating_add(1).min(max_scroll) }
               else { cur.saturating_sub(1) };
    if next == cur { return; } // at a limit
    state.scroll_transcript_to(next);
    if let Some(sel) = &mut state.selection {
        // Top edge reveals an older row above → head.row moves up by 1; bottom edge
        // reveals a newer row below → head.row moves down by 1.
        if state.selection_edge < 0 { sel.head.row = sel.head.row.saturating_sub(1); }
        else { sel.head.row = (sel.head.row + 1).min(g.total_rows.saturating_sub(1)); }
    }
}
```
Note: `scroll_transcript_to` (`state.rs:1607`) is the existing setter — use it so scroll
animation/clamping stays consistent. If it triggers a smooth-scroll animation that fights the
per-row stepping, fall back to setting `state.transcript_scroll = next;` directly and clearing
any `scroll_anim` — verify by manual smoke, prefer the setter first.

**Tests (input.rs):**
- `screen_to_point_maps_row_and_col`: geom `{area: Rect::new(0,0,20,10), first_abs_row: 5, total_rows: 100}`; `(col=3,row=2)` → `Point{row:7,col:3}`. Clamp: `row` past bottom clamps to `first_abs_row+height-1`; `col` past width clamps to `width-1`.
- `screen_to_point_clamps_to_total_rows`: small `total_rows` clamps `row`.
- `start_selection_sets_anchor_from_geom`: set `state.transcript_geom`, dispatch `StartSelection`, assert `state.selection` anchor==head==expected Point.
- `extend_selection_at_bottom_edge_autoscrolls_and_grows_head`: geom with `total_rows > height`, `transcript_scroll` mid-range; dispatch `ExtendSelection` at the bottom edge row; assert `transcript_scroll` decreased by 1 and `sel.head.row` increased by 1. Mirror for top edge (scroll increases, head.row decreases).
- `extend_selection_interior_sets_edge_zero_no_scroll`: interior row leaves `transcript_scroll` unchanged and `selection_edge == 0`.

**Build gate:** `cargo test -p app -- selection screen_to_point`.

---

## Task 5 — Wire copy + continuous auto-scroll in the run loop (`main.rs`)

1. **Remove** the post-render highlight/extract block (`main.rs:888-901`) — highlight and
   extraction now happen inside `render_middle`. If `selection_text_out`/`PaneRects.selection_text`
   become unused, remove them; otherwise leave the field and stop writing it.

2. **Copy on release:** in the `Action::EndSelection` path in the run loop (currently reads
   `last_panes.selection_text`, `main.rs:3473`), read `state.selection_text` instead:
   ```rust
   // (on mouse-release / EndSelection)
   if let Some(text) = state.selection_text.borrow_mut().take() {
       if !text.is_empty() {
           // …existing OSC 52 write to the terminal (osc52_copy_sequence)…
       }
   }
   ```
   Keep the existing terminal-write mechanism; only change the source of `text`.

3. **Continuous auto-scroll while holding at an edge:** the drag emits no events when the mouse
   is held still, so tick the auto-scroll from the loop. Find the input-poll/redraw "busy" gate
   (the same place animations keep the loop spinning without input — search for
   `has_active_animation` / `poll(Duration::from_millis` in the game loop). Add: when
   `state.selection.is_some() && state.selection_edge != 0`, call
   `app::input::apply_selection_autoscroll(&mut state);` each iteration and keep the loop live
   (treat it as "busy" so it redraws at the animation cadence rather than blocking on `read()`).
   Guard so it stops at scroll limits (the helper already no-ops there) — do NOT busy-spin
   forever: `apply_selection_autoscroll` returning without change means we're at a limit, so the
   loop may block again (the next real event re-arms it).

**Build gate:** `cargo build -p app` and `cargo test -p app`.

---

## Verification

```bash
cargo test -p app clipboard:: transcript:: input::
cargo test -p app
cargo build --workspace
```

**Manual smoke (add to the to-verify list):**
- In a game with lots of scrollback, left-drag within the story pane and drag past the bottom
  edge: the transcript auto-scrolls and the selection keeps extending; release copies the whole
  multi-screen selection (paste to confirm off-screen rows are present and accurate).
- Drag past the top edge: scrolls toward older history, selection grows upward.
- Start a selection, then wheel-scroll: the highlighted text stays anchored to the same
  transcript content (not the same screen rows).
- Column clamping still holds: a multi-row selection never pulls in map-pane text.

## Notes / known limits
- Absolute wrapped-row indices assume a stable wrap width; resizing the pane mid-drag can shift
  them. Selection is transient (cleared on release), so this is negligible.
- Char-index == column (matches the existing hyperlink assumption); wide/full-width glyphs are
  not special-cased, consistent with current behavior.
