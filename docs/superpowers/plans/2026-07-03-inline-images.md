# Inline Images Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render Glk images drawn into text-buffer windows (the primary transcript and non-primary buffer windows) as full-width inline blocks that flow with the text.

**Architecture:** An image drawn via `glk_image_draw` into a buffer window becomes a logical transcript/buffer unit that expands to an N-row image band during wrapping. The existing wrap/scroll/scrollbar machinery already counts wrapped rows, so bands ride it unchanged; only the draw loop special-cases band rows and blits via `ratatui-image` (mirroring `GraphicsRender`). Pixels travel with the element as `Arc<RgbaImage>`, exactly like `GraphicsWindow.canvas`.

**Tech Stack:** Rust workspace. App crate uses `ratatui`, `ratatui-image` (11.0.6), `image` (0.25). VMs (`zvm`, `gvm`) are zero-dependency.

## Global Constraints

- `zvm` and `gvm` crates stay **zero-dependency**. All image decode/blit lives in the `app` crate.
- Every new UI element is **themeable** via a `style.toml` selector (here: `inline_image`).
- Cross-platform (Windows/Linux/macOS); no platform-specific APIs.
- README covers **major features only** — inline images qualify for a README note (Task 11); no per-title notes.
- Never silently `panic!`/`unwrap()` on game-controlled input; unresolved images degrade to "skip".
- Match surrounding style; touch only what each task requires.

## Decisions (from the approved spec `docs/superpowers/specs/2026-07-03-inline-images-design.md`)

- **Alignment:** accept all 5 `imagealign` modes; render every image as a full-width block. `MarginRight` → right-aligned within the band width; all others → left-aligned. Store the raw `align` for a future float renderer.
- **Targets:** primary transcript AND non-primary buffer windows.
- **Fallback:** when images can't render (`state.game_picker` is `None`), an image unit emits **0 band-rows** — invisible, zero footprint, reappears if a picker becomes available. The image unit is ALWAYS stored (so a live toggle re-shows it); only the renderer decides 0 vs N rows.
- **`glk_window_flow_break`:** accepted, no-op in block mode.

## File Structure

- Create: `crates/app/src/inline_image.rs` — `ImageAlign`, `InlineImage`, `ImageAlign::from_glk`, `InlineImage::fitted_cells`. One focused home for the flow-image value type + geometry.
- Create: `crates/app/src/render/inline_image.rs` — `InlineImageRender`: per-row image-strip blit with a protocol cache (mirrors `render/graphics.rs`).
- Modify: `crates/app/src/glk_backend.rs` — `BufElem` log enum; `graphics_draw_image` buffer branch; `take_transcript_elems`; `log_to_lines` images.
- Modify: `crates/app/src/session.rs` — `TranscriptElem`, `TurnResult.transcript_elems`.
- Modify: `crates/app/src/glulx_session.rs` — fill `transcript_elems` in `finish_turn`.
- Modify: `crates/app/src/main.rs` — run-loop: iterate `transcript_elems`.
- Modify: `crates/app/src/state.rs` — `transcript_images` Vec; `push_transcript_image`; length-sync in existing push fns.
- Modify: `crates/app/src/engine.rs` — `BufferWindow.images`; re-export inline-image types if convenient.
- Modify: `crates/app/src/render/transcript.rs` — `WrappedRow` struct with optional band; `wrap_lines_kinded` band expansion; draw-loop blit.
- Modify: `crates/app/src/render/screen.rs` — `render_inline_buffer` band support.
- Modify: `crates/gvm/src/exec.rs` — accept `glk_window_flow_break` selector (no-op).
- Modify: `crates/app/src/colors.rs`, `crates/app/src/style.rs` — `inline_image` theme.
- Modify: `crates/app/src/lib.rs` (or wherever modules are declared) — `mod inline_image;`.
- Modify: `README.md` — inline-images note.

---

### Task 1: Inline-image value type + geometry

**Files:**
- Create: `crates/app/src/inline_image.rs`
- Modify: module declaration file (the crate root that lists `mod ...;` — likely `crates/app/src/main.rs` or `lib.rs`; add `pub mod inline_image;`)

**Interfaces:**
- Produces:
  - `pub enum ImageAlign { InlineUp, InlineDown, InlineCenter, MarginLeft, MarginRight }` (derive `Clone, Copy, Debug, PartialEq, Eq`)
  - `pub struct InlineImage { pub pixels: std::sync::Arc<image::RgbaImage>, pub align: ImageAlign, pub scaled: Option<(u32, u32)> }` (derive `Clone, Debug`)
  - `pub fn ImageAlign::from_glk(v: u32) -> ImageAlign` — Glk `imagealign` constants (VERIFY values against `GLULX_NOTES`/Glk spec before writing; the standard values are InlineUp=1, InlineDown=2, InlineCenter=3, MarginLeft=4, MarginRight=5). Unknown → `InlineUp`.
  - `pub fn InlineImage::fitted_cells(&self, width: u16, char_px: (u16, u16)) -> (u16, u16)` — returns `(cols, rows)` the image occupies, aspect-preserved, capped to `width`.

- [ ] **Step 1: Write the failing test**

Create `crates/app/src/inline_image.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn img(w: u32, h: u32) -> InlineImage {
        InlineImage { pixels: Arc::new(image::RgbaImage::new(w, h)), align: ImageAlign::InlineUp, scaled: None }
    }

    #[test]
    fn align_decodes_all_glk_constants() {
        assert_eq!(ImageAlign::from_glk(1), ImageAlign::InlineUp);
        assert_eq!(ImageAlign::from_glk(2), ImageAlign::InlineDown);
        assert_eq!(ImageAlign::from_glk(3), ImageAlign::InlineCenter);
        assert_eq!(ImageAlign::from_glk(4), ImageAlign::MarginLeft);
        assert_eq!(ImageAlign::from_glk(5), ImageAlign::MarginRight);
        assert_eq!(ImageAlign::from_glk(999), ImageAlign::InlineUp); // unknown → default
    }

    #[test]
    fn fitted_cells_native_when_it_fits() {
        // 16x16 px, cell 8x8 → 2x2 cells; width 40 leaves it native.
        let (cols, rows) = img(16, 16).fitted_cells(40, (8, 8));
        assert_eq!((cols, rows), (2, 2));
    }

    #[test]
    fn fitted_cells_scales_down_to_width_preserving_aspect() {
        // 800x400 px, cell 8x8 → native 100x50 cells; width 40 → scale to 40 cols,
        // height scales by 40/100 → 20 cells.
        let (cols, rows) = img(800, 400).fitted_cells(40, (8, 8));
        assert_eq!(cols, 40);
        assert_eq!(rows, 20);
    }

    #[test]
    fn fitted_cells_uses_scaled_dims_when_present() {
        let mut i = img(16, 16);
        i.scaled = Some((80, 40)); // 80x40 px scaled request overrides native 16x16
        // 80x40 px, cell 8x8 → 10x5 cells; width 40 fits.
        assert_eq!(i.fitted_cells(40, (8, 8)), (10, 5));
    }

    #[test]
    fn fitted_cells_floor_is_one() {
        // Tiny image never disappears to 0 cells.
        assert_eq!(img(1, 1).fitted_cells(40, (8, 8)), (1, 1));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app inline_image::tests -- --nocapture`
Expected: FAIL to compile (`ImageAlign`/`InlineImage` not defined).

- [ ] **Step 3: Write the implementation**

Prepend to `crates/app/src/inline_image.rs`:

```rust
//! The value type for an image that flows inline with text-buffer output
//! (Glk `glk_image_draw` into a text-buffer window), plus its cell geometry.
//! Rendered as a full-width block; the raw `align` is retained for a future
//! margin-float renderer.

use std::sync::Arc;

/// Glk `imagealign_*` argument for a buffer-window `glk_image_draw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAlign {
    InlineUp,
    InlineDown,
    InlineCenter,
    MarginLeft,
    MarginRight,
}

impl ImageAlign {
    /// Decode a Glk `imagealign` constant. Unknown values default to `InlineUp`.
    pub fn from_glk(v: u32) -> ImageAlign {
        match v {
            1 => ImageAlign::InlineUp,
            2 => ImageAlign::InlineDown,
            3 => ImageAlign::InlineCenter,
            4 => ImageAlign::MarginLeft,
            5 => ImageAlign::MarginRight,
            _ => ImageAlign::InlineUp,
        }
    }
}

/// An image drawn into a text-buffer window, carrying its pixels (shared, like
/// `GraphicsWindow.canvas`), its alignment, and an optional scaled target size.
#[derive(Clone, Debug)]
pub struct InlineImage {
    pub pixels: Arc<image::RgbaImage>,
    pub align: ImageAlign,
    pub scaled: Option<(u32, u32)>,
}

impl InlineImage {
    /// The `(cols, rows)` this image occupies at the given band `width` and
    /// terminal cell pixel size, aspect-preserved and capped to `width`.
    /// Both dimensions floor at 1.
    pub fn fitted_cells(&self, width: u16, char_px: (u16, u16)) -> (u16, u16) {
        let (cell_w, cell_h) = (char_px.0.max(1) as u32, char_px.1.max(1) as u32);
        let (pw, ph) = self.scaled.unwrap_or_else(|| {
            let d = &self.pixels;
            (d.width().max(1), d.height().max(1))
        });
        let (pw, ph) = (pw.max(1), ph.max(1));
        let max_px_w = width.max(1) as u32 * cell_w;
        let (dw, dh) = if pw <= max_px_w {
            (pw, ph)
        } else {
            // Scale down to fit width, preserving aspect ratio.
            let dh = ((ph as u64 * max_px_w as u64) / pw as u64) as u32;
            (max_px_w, dh.max(1))
        };
        let cols = dw.div_ceil(cell_w).clamp(1, width.max(1) as u32) as u16;
        let rows = dh.div_ceil(cell_h).max(1) as u16;
        (cols, rows)
    }
}
```

Add `pub mod inline_image;` to the crate root module list (find the existing `mod ...;` block; place it alphabetically near `mod graphics;`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app inline_image::tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/inline_image.rs crates/app/src/main.rs
git commit -m "feat(app): inline-image value type and cell geometry"
```

---

### Task 2: Transcript image sidecar + `push_transcript_image`

**Files:**
- Modify: `crates/app/src/state.rs` (struct field near `state.rs:918`; push fns `state.rs:1531-1639`)

**Interfaces:**
- Consumes: `crate::inline_image::InlineImage` (Task 1).
- Produces:
  - New `AppState` field `pub transcript_images: Vec<Option<crate::inline_image::InlineImage>>` — parallel to `transcript`, **not persisted** (mirror the exact mechanism `transcript_styles` uses to stay out of `transcript.json`; likely `#[serde(skip)]` or exclusion from the persisted subset — replicate whatever `transcript_styles` does).
  - `pub fn push_transcript_image(&mut self, img: crate::inline_image::InlineImage)` — appends one image unit (empty text, `Story` kind, `None` style, empty runs, `Some(img)`), keeping all five parallel Vecs length-synced.

- [ ] **Step 1: Write the failing test**

Add to `state.rs` tests module:

```rust
#[test]
fn push_transcript_image_keeps_parallel_vecs_synced() {
    let mut st = AppState::default();
    st.push_transcript("hello");
    let dummy = crate::inline_image::InlineImage {
        pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
        align: crate::inline_image::ImageAlign::InlineUp,
        scaled: None,
    };
    st.push_transcript_image(dummy);
    st.push_transcript("world");
    let n = st.transcript.len();
    assert_eq!(st.transcript_kinds.len(), n);
    assert_eq!(st.transcript_styles.len(), n);
    assert_eq!(st.transcript_runs.len(), n);
    assert_eq!(st.transcript_images.len(), n);
    // The image unit sits between the two text lines.
    let img_idx = st.transcript_images.iter().position(|o| o.is_some()).unwrap();
    assert_eq!(st.transcript[img_idx], "");
    assert_eq!(st.transcript_kinds[img_idx], TranscriptKind::Story);
    assert!(st.transcript_images.iter().filter(|o| o.is_some()).count() == 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app push_transcript_image_keeps_parallel_vecs_synced`
Expected: FAIL to compile (`transcript_images`, `push_transcript_image` missing).

- [ ] **Step 3: Write the implementation**

1. Add the field after `transcript_runs` (`state.rs:918`), mirroring `transcript_styles`' non-persistence:

```rust
    /// Optional inline image parallel to `transcript` (always same length).
    /// `Some` marks a logical unit that renders as an image band instead of
    /// text; its `transcript` entry is an empty placeholder. In-memory only
    /// (not persisted — pixels don't serialize).
    pub transcript_images: Vec<Option<crate::inline_image::InlineImage>>,
```

Initialize it wherever `AppState` is constructed / `Default`-derived (if `AppState` derives `Default`, an added `Vec` field needs no manual init; if it has a manual constructor, add `transcript_images: Vec::new()`). If it derives `Default`, ensure `InlineImage` does NOT need `Default` (the Vec default is empty — fine).

2. In EACH existing appender that pushes to the parallel Vecs — `push_transcript_kind` (`:1531`), `push_transcript_styled` (`:1543`), and `push_transcript_runs` (`:1560`) — add a matching `self.transcript_images.push(None);` next to each `self.transcript_runs.push(...)`, and add `self.transcript_images.resize(self.transcript.len(), None);` next to each existing `self.transcript_runs.resize(...)` self-heal line. (`push_transcript` delegates to `push_transcript_kind`, so it needs no change.)

3. Add the new appender near the others:

```rust
    /// Append a logical image unit: an empty placeholder line tagged `Story`
    /// carrying an inline image, keeping the parallel Vecs length-synced.
    pub fn push_transcript_image(&mut self, img: crate::inline_image::InlineImage) {
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_images.resize(self.transcript.len(), None);
        self.transcript.push(String::new());
        self.transcript_kinds.push(TranscriptKind::Story);
        self.transcript_styles.push(None);
        self.transcript_runs.push(Vec::new());
        self.transcript_images.push(Some(img));
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app push_transcript_image_keeps_parallel_vecs_synced && cargo test -p app transcript`
Expected: PASS (new test + no regression in existing transcript tests).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/state.rs
git commit -m "feat(app): transcript image sidecar + push_transcript_image"
```

---

### Task 3: Backend log becomes an ordered `BufElem`

**Files:**
- Modify: `crates/app/src/glk_backend.rs` (`BufBuf` `:76-85`; `put_text_attr` `:505`; `take_transcript` `:188`; `log_to_lines` `:419`)

**Interfaces:**
- Produces: `enum BufElem { Text { bits: u8, fg: u32, bg: u32, text: String }, Image(crate::inline_image::InlineImage) }` (module-private). `BufBuf.log: Vec<BufElem>`.
- This task is a **behavior-preserving refactor**: no images are produced yet, so all existing text behavior is identical.

**Interfaces consumed:** `crate::inline_image::InlineImage` (Task 1).

- [ ] **Step 1: Adapt the existing tests (they are the safety net)**

No new test. The existing `glk_backend` tests (`take_transcript`, `log_to_lines`, buffer-window tests at `:604+`) and `glulx_session` tests are the regression net. Confirm they currently pass:

Run: `cargo test -p app glk_backend && cargo test -p app glulx_session`
Expected: PASS (baseline before refactor).

- [ ] **Step 2: Introduce `BufElem` and migrate the log type**

Replace the `BufBuf` doc+struct (`:76-85`):

```rust
/// One entry in a text-buffer window's ordered output log.
enum BufElem {
    /// A run of printed text with its style bits and packed colours.
    Text { bits: u8, fg: u32, bg: u32, text: String },
    /// An image drawn into this buffer window (Glk `glk_image_draw`).
    Image(crate::inline_image::InlineImage),
}

/// A text-buffer window's ordered output log (text runs + inline images).
#[derive(Default)]
struct BufBuf {
    log: Vec<BufElem>,
    /// Number of leading log entries already drained by `take_transcript*`.
    drained: usize,
    /// Scrollback offset for an inline (non-primary) buffer window.
    scroll: u16,
}
```

- [ ] **Step 3: Update `put_text_attr` to push `BufElem::Text`**

At `put_text_attr` (`:505`), change the push from the tuple to:

```rust
buf.log.push(BufElem::Text { bits, fg, bg, text: s.to_owned() });
```

(Keep the surrounding logic — the exact field derivation of `bits/fg/bg/s` — unchanged; only the pushed value's shape changes. Read the current lines to match variable names.)

- [ ] **Step 4: Update `take_transcript` to skip images (text-only drain)**

At `take_transcript` (`:188`), the loop `for (bits, fg, bg, s) in &buf.log[buf.drained..]` becomes a match that ignores images (they are surfaced by `take_transcript_elems` in Task 4; the text-only drain — used for the banner — simply omits them):

```rust
for elem in &buf.log[buf.drained..] {
    let BufElem::Text { bits, fg, bg, text: s } = elem else { continue };
    // ... existing body using `bits`, `fg`, `bg`, `s` unchanged ...
}
```

Keep `buf.drained = buf.log.len();` (or the existing advance) as-is.

- [ ] **Step 5: Update `log_to_lines` signature to take `&[BufElem]`**

At `log_to_lines` (`:419`), change the parameter to `log: &[BufElem]` and the loop to match `BufElem::Text` (ignore `Image` in this task; Task 6 adds image handling):

```rust
fn log_to_lines(log: &[BufElem]) -> (Vec<String>, Vec<Vec<StyleRun>>) {
    let mut lines: Vec<String> = vec![String::new()];
    let mut runs: Vec<Vec<StyleRun>> = vec![Vec::new()];
    for elem in log {
        let BufElem::Text { bits, fg, bg, text } = elem else { continue };
        for ch in text.chars() {
            // ... existing per-char body unchanged (uses *bits, *fg, *bg) ...
        }
    }
    (lines, runs)
}
```

Update every caller of `log_to_lines` (grep `log_to_lines(`) — they pass `&buf.log`, whose element type changed, so they compile unchanged.

- [ ] **Step 6: Fix any other `buf.log` consumers**

Grep `\.log` within `glk_backend.rs` for other tuple-destructuring sites and migrate them to match `BufElem`. Update in-file test fixtures that build `log` tuples to `BufElem::Text { .. }`.

- [ ] **Step 7: Run tests to verify no regression**

Run: `cargo test -p app glk_backend && cargo test -p app glulx_session`
Expected: PASS (identical behavior).

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/glk_backend.rs
git commit -m "refactor(app): text-buffer log carries ordered BufElem entries"
```

---

### Task 4: Route buffer-window image draws + ordered element drain

**Files:**
- Modify: `crates/app/src/glk_backend.rs` (`graphics_draw_image` `:586`; add `take_transcript_elems`)
- Modify: `crates/app/src/session.rs` (add `TranscriptElem`)

**Interfaces:**
- Consumes: `BufElem` (Task 3), `InlineImage`/`ImageAlign::from_glk` (Task 1).
- Produces:
  - In `session.rs`: `pub enum TranscriptElem { Text { text: String, runs: Vec<(usize, u8, ZColour, ZColour)> }, Image(crate::inline_image::InlineImage) }`.
  - `AppGlk::take_transcript_elems(&mut self) -> Vec<TranscriptElem>` — drains the primary window's undrained log into ordered elements, coalescing consecutive `Text` runs into one `TranscriptElem::Text` (so the existing per-line run walk in `push_transcript_runs` keeps working), and emitting each image as `TranscriptElem::Image`.

- [ ] **Step 1: Write the failing tests**

Add to `glk_backend.rs` tests:

```rust
#[test]
fn image_draw_to_buffer_window_records_image_elem() {
    let mut glk = AppGlk::new(80, 24);
    glk.window_open(1, WinType::TextBuffer); // primary buffer
    // Register a Pict so the draw resolves. Use whatever the existing tests use
    // to seed PictSource; if none exists, this asserts the routing path only.
    glk.put_text_attr(1, "before\n", 0, 0, 0); // match real put_text_attr signature
    glk.graphics_draw_image(1, /*resnum*/ 0, /*imagealign*/ 1, 0, None);
    glk.put_text_attr(1, "after", 0, 0, 0);
    let elems = glk.take_transcript_elems();
    // Expect: Text("before\n"), [Image if resolvable], Text("after") — order preserved.
    let kinds: Vec<&str> = elems.iter().map(|e| match e {
        crate::session::TranscriptElem::Text { .. } => "T",
        crate::session::TranscriptElem::Image(_) => "I",
    }).collect();
    assert_eq!(kinds.first().map(|s| *s), Some("T"));
    assert_eq!(kinds.last().map(|s| *s), Some("T"));
}

#[test]
fn image_draw_to_graphics_window_still_hits_canvas() {
    let mut glk = AppGlk::new(80, 24);
    glk.window_open(5, WinType::Graphics);
    // A graphics-window draw must NOT push a buffer image elem; it updates a Canvas.
    glk.graphics_draw_image(5, 0, 10, 10, None);
    // Primary buffer is absent → elems empty.
    assert!(glk.take_transcript_elems().is_empty());
}
```

Adjust `put_text_attr` argument list / `WinType::Graphics` variant name to the real ones (read the signatures first). If seeding a `Pict` is nontrivial in a unit test, keep the routing assertions that don't require a resolvable image (order of the surrounding `Text` elems), and cover resolvable-image behavior in the Task 5 integration test via `glulx_session`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app image_draw_to_buffer_window_records_image_elem image_draw_to_graphics_window_still_hits_canvas`
Expected: FAIL to compile (`take_transcript_elems`, `TranscriptElem` missing).

- [ ] **Step 3: Add `TranscriptElem` in `session.rs`**

Near `TurnResult` (`session.rs:116`):

```rust
/// One ordered piece of a turn's buffer output: a text run (with its style
/// chunks) or an inline image. Preserves emission order so images land between
/// the right lines.
pub enum TranscriptElem {
    Text { text: String, runs: Vec<(usize, u8, ZColour, ZColour)> },
    Image(crate::inline_image::InlineImage),
}
```

- [ ] **Step 4: Branch `graphics_draw_image` on window type**

Replace `graphics_draw_image` (`:586`):

```rust
fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) {
    // Buffer-window target: `x` is really the Glk imagealign flag; the image
    // flows inline with the window's text rather than onto a pixel canvas.
    if self.buffers.contains_key(&win) {
        if let Some(src) = self.picts.image(resnum).cloned() {
            let img = crate::inline_image::InlineImage {
                pixels: std::sync::Arc::new(src.to_rgba8()),
                align: crate::inline_image::ImageAlign::from_glk(x as u32),
                scaled: scale,
            };
            if let Some(buf) = self.buffers.get_mut(&win) {
                buf.log.push(BufElem::Image(img));
            }
        }
        return;
    }
    // Graphics-window target: existing canvas path.
    if let Some(src) = self.picts.image(resnum).cloned() {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .draw_image(&src, x, y, scale);
    }
}
```

(Confirm `DynamicImage::to_rgba8()` is the right conversion; `PictSource::image` returns `&DynamicImage` per recon. `to_rgba8()` yields an `RgbaImage`.)

- [ ] **Step 5: Add `take_transcript_elems`**

Next to `take_transcript` (`:188`):

```rust
/// Drain the primary window's undrained log into ordered transcript elements
/// (consecutive text runs coalesced; images preserved in place).
pub fn take_transcript_elems(&mut self) -> Vec<crate::session::TranscriptElem> {
    use crate::session::TranscriptElem;
    let Some(pid) = self.primary else { return Vec::new() };
    let Some(buf) = self.buffers.get_mut(&pid) else { return Vec::new() };
    let mut out: Vec<TranscriptElem> = Vec::new();
    // Accumulate consecutive Text runs into one element, matching the char-count
    // chunk shape `push_transcript_runs` expects: (char_count, bits, fg, bg).
    let mut cur_text = String::new();
    let mut cur_runs: Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)> = Vec::new();
    let flush = |out: &mut Vec<TranscriptElem>, text: &mut String, runs: &mut Vec<_>| {
        if !text.is_empty() {
            out.push(TranscriptElem::Text { text: std::mem::take(text), runs: std::mem::take(runs) });
        } else {
            runs.clear();
        }
    };
    for elem in &buf.log[buf.drained..] {
        match elem {
            BufElem::Text { bits, fg, bg, text } => {
                let n = text.chars().count();
                if n > 0 {
                    // Convert packed u32 colours back to ZColour to match the
                    // chunk type push_transcript_runs consumes.
                    let (f, b) = (crate::state::unpack_zcolour(*fg), crate::state::unpack_zcolour(*bg));
                    cur_runs.push((n, *bits, f, b));
                    cur_text.push_str(text);
                }
            }
            BufElem::Image(img) => {
                flush(&mut out, &mut cur_text, &mut cur_runs);
                out.push(TranscriptElem::Image(img.clone()));
            }
        }
    }
    flush(&mut out, &mut cur_text, &mut cur_runs);
    buf.drained = buf.log.len();
    out
}
```

Confirm `crate::state::unpack_zcolour` is `pub` (it is — `state.rs:228`). If `take_transcript` and `take_transcript_elems` would double-advance `drained` when both run in a turn, ensure the turn path (Task 5) calls only ONE of them per drain; `take_transcript_elems` supersedes `take_transcript` for the game turn.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p app image_draw_to_buffer_window_records_image_elem image_draw_to_graphics_window_still_hits_canvas && cargo test -p app glk_backend`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/glk_backend.rs crates/app/src/session.rs
git commit -m "feat(app): route buffer-window image draws into ordered transcript elems"
```

---

### Task 5: Feed elements through the turn into the transcript

**Files:**
- Modify: `crates/app/src/session.rs` (`TurnResult` `:117`)
- Modify: `crates/app/src/glulx_session.rs` (`finish_turn` `:199-225`)
- Modify: `crates/app/src/main.rs` (run loop `:3125-3131`)

**Interfaces:**
- Consumes: `take_transcript_elems` (Task 4), `push_transcript_image` (Task 2), `TranscriptElem` (Task 4).
- Produces: `TurnResult.transcript_elems: Vec<TranscriptElem>`; run loop pushes text via `push_transcript_runs` and images via `push_transcript_image`, in order.

- [ ] **Step 1: Write the failing test**

Add to `glulx_session.rs` tests a story that prints text, draws an image into the primary buffer, then prints more, and assert the resulting `TurnResult.transcript_elems` order is Text, Image, Text. Model it on the existing `submit_echoes_line_with_runs_and_screen_is_two_window_tree` test (`:476`) for how a `GlulxSession` is built and driven. If seeding a Pict in gvm test harness is impractical, instead unit-test the run-loop dispatch directly: build an `AppState`, a `Vec<TranscriptElem>` = `[Text{"a\n",..}, Image(dummy), Text{"b",..}]`, run the same dispatch loop used in `main.rs`, and assert the transcript has "a", image unit, "b" in order.

```rust
#[test]
fn turn_elems_interleave_text_and_image_in_transcript() {
    use crate::session::TranscriptElem;
    let mut st = crate::state::AppState::default();
    let dummy = crate::inline_image::InlineImage {
        pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
        align: crate::inline_image::ImageAlign::InlineUp, scaled: None,
    };
    let elems = vec![
        TranscriptElem::Text { text: "a".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default)] },
        TranscriptElem::Image(dummy),
        TranscriptElem::Text { text: "b".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default)] },
    ];
    // Mirror the run-loop dispatch (extract to a helper `apply_transcript_elems`
    // so it is testable without the full loop — see Step 3).
    crate::main_support::apply_transcript_elems(&mut st, &elems);
    assert_eq!(st.transcript, vec!["a".to_string(), "".to_string(), "b".to_string()]);
    assert!(st.transcript_images[1].is_some());
    assert!(st.transcript_images[0].is_none() && st.transcript_images[2].is_none());
}
```

(If there is no `main_support`/lib module to host a testable helper, place `apply_transcript_elems` as a `pub fn` in `state.rs` or `session.rs` instead and adjust the path. Keeping it out of `main.rs`'s binary-only scope is what makes it testable.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app turn_elems_interleave_text_and_image_in_transcript`
Expected: FAIL to compile (`transcript_elems`, `apply_transcript_elems` missing).

- [ ] **Step 3: Implement**

1. Add to `TurnResult` (`session.rs:117`):

```rust
    /// Ordered buffer output for this turn (text runs + inline images). Empty
    /// for the Z-machine path (no images); the Glulx path fills it and the run
    /// loop pushes from it. When empty, the loop falls back to `transcript` +
    /// `transcript_runs`.
    pub transcript_elems: Vec<TranscriptElem>,
```

Update EVERY `TurnResult { .. }` constructor (grep `TurnResult {`) to set `transcript_elems: Vec::new()` — the zvm `session.rs` constructors and the glulx ones default to empty; only glulx `finish_turn` overrides.

2. In `glulx_session.rs` `finish_turn` (`:199`): it currently calls `self.appglk().take_transcript()` (`:204`). Add a parallel drain of elements BEFORE that call would double-advance `drained` — replace the text drain with the element drain and derive the flat `transcript`/`transcript_runs` from it for the existing consumers:

Read `finish_turn` fully first. Replace the `let (raw, raw_runs) = self.appglk().take_transcript();` line with:

```rust
    let elems = self.appglk().take_transcript_elems();
    // Flat text + chunks for existing consumers (banner-strip, location, tests).
    let mut raw = String::new();
    let mut raw_runs: Vec<(usize, u8, ZColour, ZColour)> = Vec::new();
    for e in &elems {
        if let crate::session::TranscriptElem::Text { text, runs } = e {
            raw.push_str(text);
            raw_runs.extend(runs.iter().copied());
        }
    }
```

Then keep the existing `strip_read_prompt`/`clamp_runs` handling of `raw`/`raw_runs`, and add `transcript_elems: elems,` to the `TurnResult { .. }` it builds (`:215`). NOTE: if `strip_read_prompt` shortens `raw`, the element list still contains the untrimmed trailing prompt text; for correctness, apply the same trailing-prompt strip to the LAST `Text` element of `elems` (or accept the minor cosmetic prompt echo). Simplest correct approach: after computing the stripped length, trim the final `Text` element's `text`/`runs` to match. Implement the trim; add a test asserting a trailing `>` prompt is stripped from the last element.

3. In `main.rs` run loop (`:3125-3131`), replace the single `push_transcript_runs` line with a dispatch that prefers elements:

```rust
if result.transcript_elems.is_empty() {
    state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs);
} else {
    app::state::apply_transcript_elems(&mut state, &result.transcript_elems);
}
```

4. Add the testable helper (in `state.rs`, `pub fn`):

```rust
/// Push a turn's ordered elements into the transcript: text runs via
/// `push_transcript_runs`, images via `push_transcript_image`, in order.
pub fn apply_transcript_elems(state: &mut AppState, elems: &[crate::session::TranscriptElem]) {
    use crate::session::TranscriptElem;
    for e in elems {
        match e {
            TranscriptElem::Text { text, runs } => {
                state.push_transcript_runs(text, TranscriptKind::Story, runs);
            }
            TranscriptElem::Image(img) => state.push_transcript_image(img.clone()),
        }
    }
}
```

(If `apply_transcript_elems` is a free fn in `state.rs`, adjust the earlier test path accordingly.)

Also apply the same dispatch anywhere else `main.rs` pushes a `TurnResult`'s transcript for Glulx (resume_save/resume_restore/timed paths, if they push transcript). Grep `push_transcript_runs(&result` and `push_transcript_runs(&r` in `main.rs` and route each through `apply_transcript_elems` when `transcript_elems` is non-empty.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app turn_elems_interleave_text_and_image_in_transcript && cargo test -p app glulx_session && cargo build -p app`
Expected: PASS + clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/glulx_session.rs crates/app/src/main.rs crates/app/src/state.rs
git commit -m "feat(app): carry inline images through the turn into the transcript"
```

---

### Task 6: Non-primary buffer windows carry images

**Files:**
- Modify: `crates/app/src/engine.rs` (`BufferWindow` `:154`)
- Modify: `crates/app/src/glk_backend.rs` (`log_to_lines` `:419`; the `buffer_node`/BufferWindow builder that calls it — near `:282`)

**Interfaces:**
- Produces: `BufferWindow.images: Vec<Option<crate::inline_image::InlineImage>>` (parallel to `lines`/`runs`). `log_to_lines` returns image markers.

- [ ] **Step 1: Write the failing test**

Add to `glk_backend.rs` tests: open a NON-primary buffer window, print "a\n", draw an image into it, print "b", build its `BufferWindow`, and assert `images` has a `Some` at the line index between "a" and "b".

```rust
#[test]
fn non_primary_buffer_carries_inline_image() {
    let mut glk = AppGlk::new(80, 24);
    glk.window_open(1, WinType::TextBuffer); // primary
    glk.window_open(2, WinType::TextBuffer); // non-primary
    glk.put_text_attr(2, "a\n", 0, 0, 0);
    glk.graphics_draw_image(2, 0, 1, 0, None); // may be a no-op if Pict unresolved
    glk.put_text_attr(2, "b", 0, 0, 0);
    // Build the BufferWindow for window 2 (use the same builder the snapshot uses).
    let bw = glk.buffer_window_for_test(2); // add a small test accessor if needed
    assert_eq!(bw.lines.len(), bw.images.len());
    assert_eq!(bw.lines.len(), bw.runs.len());
}
```

(If a Pict can't be seeded, the assertion reduces to the length-sync invariant; the image-present case is covered where Pict seeding is available. Add a minimal `#[cfg(test)]` accessor if no public path builds one `BufferWindow`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app non_primary_buffer_carries_inline_image`
Expected: FAIL to compile (`images` field / accessor missing).

- [ ] **Step 3: Implement**

1. Add to `BufferWindow` (`engine.rs:154`):

```rust
    /// Optional inline image parallel to `lines` (always same length). `Some`
    /// marks a line that renders as an image band instead of text.
    pub images: Vec<Option<crate::inline_image::InlineImage>>,
```

2. Change `log_to_lines` (`:419`) to also emit an `images` parallel vec and handle `BufElem::Image`:

```rust
fn log_to_lines(log: &[BufElem]) -> (Vec<String>, Vec<Vec<StyleRun>>, Vec<Option<crate::inline_image::InlineImage>>) {
    let mut lines: Vec<String> = vec![String::new()];
    let mut runs: Vec<Vec<StyleRun>> = vec![Vec::new()];
    let mut images: Vec<Option<crate::inline_image::InlineImage>> = vec![None];
    for elem in log {
        match elem {
            BufElem::Text { bits, fg, bg, text } => {
                for ch in text.chars() {
                    if ch == '\n' {
                        lines.push(String::new()); runs.push(Vec::new()); images.push(None);
                        continue;
                    }
                    // ... existing per-char styling body unchanged ...
                }
            }
            BufElem::Image(img) => {
                // Image occupies its own logical line; start a fresh line after it.
                if !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                    lines.push(String::new()); runs.push(Vec::new()); images.push(None);
                }
                *images.last_mut().unwrap() = Some(img.clone());
                lines.push(String::new()); runs.push(Vec::new()); images.push(None);
            }
        }
    }
    (lines, runs, images)
}
```

3. Update the `BufferWindow` builder (grep `log_to_lines(` in `glk_backend.rs`, near `:282`) to destructure the third return value and set `images` on the constructed `BufferWindow`.

4. Update every other `BufferWindow { .. }` literal (grep) to include `images: Vec::new()` or the built vec.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app non_primary_buffer_carries_inline_image && cargo test -p app glk_backend`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/engine.rs crates/app/src/glk_backend.rs
git commit -m "feat(app): non-primary buffer windows carry inline images"
```

---

### Task 7: Wrap image units into bands

**Files:**
- Modify: `crates/app/src/render/transcript.rs` (`WrappedRow` alias `:19`; `wrap_lines_kinded` `:340`; `visible_wrapped_lines_kinded` `:385`)

**Interfaces:**
- Produces:
  - `WrappedRow` becomes a struct: `pub(crate) struct WrappedRow { pub text: String, pub kind: TranscriptKind, pub style: Style, pub runs: Vec<StyleRun>, pub band: Option<ImageBand> }`.
  - `pub(crate) struct ImageBand { pub image: crate::inline_image::InlineImage, pub cols: u16, pub rows: u16, pub row: u16, pub x_off: u16 }` — one per band terminal row (`row` in `0..rows`).
  - `wrap_lines_kinded` and `visible_wrapped_lines_kinded` gain `images: &[Option<InlineImage>]`, `char_px: (u16, u16)`, `images_enabled: bool`.

- [ ] **Step 1: Write the failing tests**

Add to `transcript.rs` tests:

```rust
fn dummy_img(w: u32, h: u32, align: crate::inline_image::ImageAlign) -> crate::inline_image::InlineImage {
    crate::inline_image::InlineImage { pixels: std::sync::Arc::new(image::RgbaImage::new(w, h)), align, scaled: None }
}

#[test]
fn image_unit_expands_to_band_rows() {
    // Two text lines with an image unit between; image 16x24 px, cell 8x8 →
    // 2 cols x 3 rows band.
    let transcript = vec!["hi".to_string(), String::new(), "bye".to_string()];
    let kinds = vec![TranscriptKind::Story; 3];
    let styles = vec![Style::default(); 3];
    let runs = vec![Vec::new(); 3];
    let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp)), None];
    let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &images, (8, 8), true, 40);
    // 1 (hi) + 3 (band) + 1 (bye) = 5 rows.
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].band.is_none(), true);
    assert_eq!(rows[1].band.as_ref().unwrap().rows, 3);
    assert_eq!(rows[1].band.as_ref().unwrap().row, 0);
    assert_eq!(rows[3].band.as_ref().unwrap().row, 2);
    assert_eq!(rows[4].text, "bye");
}

#[test]
fn image_unit_emits_zero_rows_when_disabled() {
    let transcript = vec!["hi".to_string(), String::new()];
    let kinds = vec![TranscriptKind::Story; 2];
    let styles = vec![Style::default(); 2];
    let runs = vec![Vec::new(); 2];
    let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp))];
    let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &images, (8, 8), false, 40);
    assert_eq!(rows.len(), 1); // only "hi"
}

#[test]
fn band_reflows_narrower_on_smaller_width() {
    // 800x400 px, cell 8x8: width 40 → 40x20; width 20 → 20x10.
    let transcript = vec![String::new()];
    let kinds = vec![TranscriptKind::Story];
    let styles = vec![Style::default()];
    let runs = vec![Vec::new()];
    let images = vec![Some(dummy_img(800, 400, crate::inline_image::ImageAlign::InlineUp))];
    let wide = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &images, (8, 8), true, 40);
    let narrow = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &images, (8, 8), true, 20);
    assert_eq!(wide.len(), 20);
    assert_eq!(narrow.len(), 10);
}

#[test]
fn margin_right_sets_x_offset() {
    let transcript = vec![String::new()];
    let kinds = vec![TranscriptKind::Story];
    let styles = vec![Style::default()];
    let runs = vec![Vec::new()];
    let images = vec![Some(dummy_img(16, 8, crate::inline_image::ImageAlign::MarginRight))]; // 2x1 cells
    let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &images, (8, 8), true, 40);
    assert_eq!(rows[0].band.as_ref().unwrap().x_off, 38); // 40 - 2
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app render::transcript::tests::image_unit`
Expected: FAIL to compile (new params/`band` field missing).

- [ ] **Step 3: Migrate `WrappedRow` to a struct + expand bands**

1. Replace the `WrappedRow` alias (`:19`) with the struct above and add `ImageBand`. Update the return types of `wrap_lines_kinded`/`visible_wrapped_lines_kinded`/`wrap_line_hanging`-mapped tuples: every place that builds `(row, kind, style, runs)` now builds `WrappedRow { text: row, kind, style, runs, band: None }`.

2. Change `wrap_lines_kinded`'s signature and add the image branch inside the `flat_map`:

```rust
pub(crate) fn wrap_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    width: u16,
) -> Vec<WrappedRow> {
    transcript.iter().enumerate().flat_map(|(i, line)| {
        if let Some(Some(img)) = images.get(i) {
            if !images_enabled { return Vec::new(); }
            let (cols, rows) = img.fitted_cells(width, char_px);
            let x_off = match img.align {
                crate::inline_image::ImageAlign::MarginRight => width.saturating_sub(cols),
                _ => 0,
            };
            return (0..rows).map(|r| WrappedRow {
                text: String::new(),
                kind: TranscriptKind::Story,
                style: Style::default(),
                runs: Vec::new(),
                band: Some(ImageBand { image: img.clone(), cols, rows, row: r, x_off }),
            }).collect();
        }
        // ... existing text-wrapping branch, wrapped in `WrappedRow { .., band: None }` ...
    }).collect()
}
```

3. Thread the three new args through `visible_wrapped_lines_kinded` (its signature + the two internal `wrap_lines_kinded` calls, including the `clear_anchor` recursion at `:412` — pass the sliced `&images[..a.min(images.len())]` there too, plus `char_px`, `images_enabled`).

4. Update ALL callers of both functions (grep both names) — the render path caller in `render_middle` (Task 8 supplies `images`/`char_px`/`images_enabled`) and every test caller. For test callers that don't care about images, pass `&[]`, `(1, 1)`, `false`. NOTE: `&[]` shorter than `transcript` is fine — `images.get(i)` yields `None`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app render::transcript`
Expected: PASS (new band tests + existing wrap/scroll tests via the `WrappedRow` struct).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/transcript.rs
git commit -m "feat(app): expand inline-image units into wrapped image bands"
```

---

### Task 8: Blit image bands in the transcript draw loop

**Files:**
- Create: `crates/app/src/render/inline_image.rs`
- Modify: `crates/app/src/render/transcript.rs` (draw loop `:1091`; caller of `visible_wrapped_lines_kinded` `:1075-1086`)
- Modify: `crates/app/src/render/mod.rs` (declare `pub mod inline_image;`)
- Modify: `crates/app/src/state.rs` (add an `InlineImageRender` holder if the renderer is cached on `State`, mirroring `graphics_render`)

**Interfaces:**
- Consumes: `ImageBand` (Task 7), `state.game_picker: Option<Picker>`.
- Produces: `InlineImageRender::render_row(&mut self, picker, band: &ImageBand, dest: Rect, letterbox: Style, buf)` — blits the horizontal strip for `band.row` of the fitted image into a 1-row `dest`.

- [ ] **Step 1: Write the failing test**

Add to `render/inline_image.rs` a test that renders a solid-red 2×2-cell image band into a `Buffer` with a halfblocks `Picker` and asserts the destination cells are non-blank (mirror how `render/graphics.rs` behavior is exercised, if it has tests; otherwise assert the render call does not panic and writes at least one non-space symbol in `dest`).

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui_image::picker::Picker;

    #[test]
    fn renders_band_row_without_panic() {
        let mut px = image::RgbaImage::new(16, 16);
        for p in px.pixels_mut() { *p = image::Rgba([200, 0, 0, 255]); }
        let img = crate::inline_image::InlineImage { pixels: std::sync::Arc::new(px), align: crate::inline_image::ImageAlign::InlineUp, scaled: None };
        let band = crate::render::transcript::ImageBand { image: img, cols: 2, rows: 2, row: 0, x_off: 0 };
        let picker = Picker::halfblocks();
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let mut r = InlineImageRender::default();
        r.render_row(&picker, &band, Rect::new(0, 0, 2, 1), ratatui::style::Style::default(), &mut buf);
        // No panic == pass; the halfblock protocol writes into (0,0)..(2,1).
    }
}
```

(If `ImageBand`/`WrappedRow` are `pub(crate)`, this test in a sibling module can see them.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app render::inline_image`
Expected: FAIL to compile (`InlineImageRender` missing).

- [ ] **Step 3: Implement `InlineImageRender`**

`crates/app/src/render/inline_image.rs` — mirror `render/graphics.rs`, but slice one terminal-row strip per call and cache the fitted per-(image-identity, cols, rows) full image so strips are cheap:

```rust
//! Blits inline-image bands (one terminal-row strip per call) via ratatui-image,
//! mirroring `render/graphics.rs`. Each band row renders the corresponding
//! horizontal strip of the fitted image, so partial-scroll degrades cleanly.

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
use ratatui_image::{Image, Resize};

use crate::render::transcript::ImageBand;

#[derive(Default)]
pub struct InlineImageRender;

impl std::fmt::Debug for InlineImageRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineImageRender").finish()
    }
}

impl InlineImageRender {
    /// Blit the strip for `band.row` (of `band.rows`) into the 1-row `dest`.
    pub fn render_row(&mut self, picker: &Picker, band: &ImageBand, dest: Rect, letterbox: Style, buf: &mut Buffer) {
        if dest.width == 0 || dest.height == 0 { return; }
        // Letterbox the destination first (padding when the image is narrower).
        for y in dest.top()..dest.bottom() {
            for x in dest.left()..dest.right() {
                if let Some(c) = buf.cell_mut((x, y)) { c.set_symbol(" ").set_style(letterbox); }
            }
        }
        // Fit the whole image to the band's cell box in pixels, then crop the
        // strip for this row. Cell pixel size comes from the picker font.
        let (fw, fh) = picker.font_size();
        let (fw, fh) = (fw.max(1) as u32, fh.max(1) as u32);
        let box_w = band.cols as u32 * fw;
        let box_h = band.rows as u32 * fh;
        if box_w == 0 || box_h == 0 { return; }
        let full = image::DynamicImage::ImageRgba8((*band.image.pixels).clone())
            .resize_exact(box_w, box_h, image::imageops::FilterType::Triangle);
        let strip_y = band.row as u32 * fh;
        if strip_y >= box_h { return; }
        let strip_h = fh.min(box_h - strip_y);
        let strip = image::DynamicImage::from(full).crop_imm(0, strip_y, box_w, strip_h);
        match picker.new_protocol(strip, Size::new(band.cols, 1), Resize::Fit(None)) {
            Ok(proto) => Image::new(&proto).render(dest, buf),
            Err(_) => {}
        }
    }
}
```

(Confirm `Image::new` takes `&Protocol` vs `Protocol` — `render/graphics.rs:52` uses `Image::new(proto)` where `proto: &Protocol`; match that. `resize_exact`/`crop_imm` are `image` 0.25 APIs — verify names. If per-call protocol build proves too slow in practice, add a small cache keyed by `(Arc::as_ptr(&band.image.pixels) as usize, band.cols, band.rows, band.row)`; leave uncached for the first pass and note it.)

- [ ] **Step 4: Wire the draw loop**

In `render_middle`:
1. Compute `char_px` and `images_enabled` before the `visible_wrapped_lines_kinded` call (`:1075`):

```rust
let images_enabled = state.game_picker.is_some();
let char_px = state.game_picker.as_ref()
    .map(|p| { let (w, h) = p.font_size(); (w, h) })
    .unwrap_or((1, 1));
```

Pass `&filtered_images`, `char_px`, `images_enabled` into `visible_wrapped_lines_kinded` (build `filtered_images` alongside `filtered_styles`/`filtered_runs` at `:1043-1061` — index `state.transcript_images` by the same `visible_transcript_indices`).

2. In the draw loop (`:1091`), destructure the struct and branch on `band`:

```rust
for (i, wr) in lines.iter().enumerate() {
    let row_y = transcript_top + i as u16;
    if row_y >= transcript_bottom { break; }
    if let Some(band) = &wr.band {
        if let Some(picker) = state.game_picker.as_ref() {
            let dest = Rect::new(body_area.x + band.x_off.min(body_area.width), row_y, band.cols.min(body_area.width.saturating_sub(band.x_off)), 1);
            state.inline_image_render.borrow_mut().render_row(picker, band, dest, state.colors.inline_image, buf);
        }
        continue;
    }
    // ... existing text-row drawing using wr.kind / wr.text / wr.style / wr.runs ...
}
```

3. Add `inline_image_render: RefCell<InlineImageRender>` to `State` (mirror `graphics_render` — grep `graphics_render` in `state.rs`/`main.rs` for the exact holder type and init). `state.colors.inline_image` comes from Task 10.

Because Task 10 (theming) supplies `state.colors.inline_image`, either do Task 10 first or temporarily use `state.colors.graphics` and switch in Task 10. Recommendation: reorder so Task 10 precedes Task 8, OR use `state.colors.graphics` here and change the one reference in Task 10.

- [ ] **Step 5: Run tests + manual smoke**

Run: `cargo test -p app render && cargo build -p app`
Expected: PASS + clean build. (Visual smoke happens in Task 11's manual check with a real story.)

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render/inline_image.rs crates/app/src/render/transcript.rs crates/app/src/render/mod.rs crates/app/src/state.rs
git commit -m "feat(app): blit inline-image bands in the transcript"
```

---

### Task 9: Bands in non-primary buffer windows

**Files:**
- Modify: `crates/app/src/render/screen.rs` (`render_inline_buffer` `:294`)

**Interfaces:**
- Consumes: `BufferWindow.images` (Task 6), `InlineImageRender`/`ImageBand` (Tasks 7-8), the shared band expansion.

- [ ] **Step 1: Write the failing test**

Add a `screen.rs` test that builds a `BufferWindow` with `lines = ["a", "", "b"]` and `images[1] = Some(dummy)`, renders it via `render_inline_buffer` into a `Buffer` with a halfblocks picker in `state.game_picker`, and asserts the row that would hold "b" is pushed down by the band height (i.e., "b" no longer sits on row 1). Model construction on existing `screen.rs` buffer tests (`inline_buffer_renders_styled_runs`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app render::screen::tests` (the new test name)
Expected: FAIL (image ignored → "b" on the wrong row) or compile error if `images` unused.

- [ ] **Step 3: Implement**

`render_inline_buffer` currently wraps `bw.lines`/`bw.runs` and draws each row. Refactor it to use the SAME band-expansion as the transcript: build the wrapped rows via `wrap_lines_kinded` (passing `&bw.images`, `char_px`, `images_enabled`, width), then draw each row branching on `band` exactly like Task 8's loop (text rows via `draw_str_runs`, band rows via `state.inline_image_render`). Extract the per-row draw branch from Task 8 into a shared `fn draw_wrapped_row(...)` if it reduces duplication; otherwise mirror it.

Note: `BufferWindow` rows use `TranscriptKind::Story`-style wrapping already; supply `kinds`/`styles` vectors of the right length (all `Story`/default) to `wrap_lines_kinded`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app render::screen && cargo build -p app`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/screen.rs
git commit -m "feat(app): inline-image bands in non-primary buffer windows"
```

---

### Task 10: Accept `glk_window_flow_break` (no-op)

**Files:**
- Modify: `crates/gvm/src/exec.rs` (Glk selector dispatch near the "unhandled selector" arm `:2760`)

**Interfaces:**
- Produces: the `glk_window_flow_break` selector is consumed silently (returns 0), not reported as unhandled. No visible effect (block mode already breaks the flow).

- [ ] **Step 1: Write the failing test**

In `gvm` tests, drive a program that issues the `glk_window_flow_break` selector and assert no diagnostic/unhandled-selector output is produced (mirror an existing selector test in `exec.rs` tests). VERIFY the selector constant value against `GLULX_NOTES`/Glk spec (glk_window_flow_break) before writing — do not hardcode from memory.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p gvm flow_break`
Expected: FAIL (selector falls into the unhandled arm).

- [ ] **Step 3: Implement**

Add a selector arm alongside the graphics arms (`:2707-2758`) that accepts `glk_window_flow_break`, pops/ignores its window arg, and pushes `0`:

```rust
0x00?? /* glk_window_flow_break */ => {
    let _win = a(0);
    // Block-mode inline images already break the text flow; nothing to do.
    self.push_glk_result(0); // match the surrounding arms' result convention
}
```

Match the exact result-return convention used by neighboring arms (some store via pointer, some push). Keep `gvm` zero-dep.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p gvm && cargo build -p gvm`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/gvm/src/exec.rs
git commit -m "feat(gvm): accept glk_window_flow_break as a no-op"
```

---

### Task 11: Theme the inline-image band + README

**Files:**
- Modify: `crates/app/src/colors.rs` (add `inline_image` field, mirror `graphics`)
- Modify: `crates/app/src/style.rs` (add `inline_image` selector, mirror the `graphics` selector)
- Modify: `crates/app/src/render/transcript.rs` + `render/screen.rs` (switch the band letterbox arg from `state.colors.graphics` to `state.colors.inline_image` if Task 8/9 used the placeholder)
- Modify: `README.md`

**Interfaces:**
- Produces: `ColorScheme.inline_image: Style` with a `style.toml` `inline_image` selector; README note under major features.

- [ ] **Step 1: Write the failing test**

If `colors.rs`/`style.rs` have selector round-trip tests (grep for a `graphics` selector test), add a parallel one asserting `inline_image` parses from `style.toml` and lands on `ColorScheme.inline_image`. Otherwise add a minimal test asserting `ColorScheme::default().inline_image` exists and the selector name resolves.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app inline_image` (the theme test)
Expected: FAIL to compile (`inline_image` field/selector missing).

- [ ] **Step 3: Implement**

1. Add `pub inline_image: Style` to `ColorScheme` (mirror `graphics`: same default — a neutral letterbox). 2. Add the `"inline_image"` selector arm in `style.rs` wherever `"graphics"` is handled (parse + apply). 3. Switch the two band-render call sites to `state.colors.inline_image`. 4. Add a README note under major features (per the README-major-features policy): a short paragraph that lanthorn now renders Glk inline images in text-buffer windows as blocks, honoring the terminal's image protocol, themeable via the `inline_image` selector.

- [ ] **Step 4: Run tests + full workspace check**

Run: `cargo test -p app inline_image && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS + clippy clean.

- [ ] **Step 5: Manual smoke test**

Run the interpreter on a Glulx story known to draw inline images in its main window and confirm the image renders as a block in the transcript, scrolls with the text, reflows on resize, and disappears cleanly when no image protocol is available. Note the story used.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs crates/app/src/render/transcript.rs crates/app/src/render/screen.rs README.md
git commit -m "feat(app): themeable inline-image band + README note"
```

---

## Final Review

After all tasks: dispatch the whole-branch code review (superpowers:requesting-code-review), then finish via superpowers:finishing-a-development-branch. Confirm: `cargo test --workspace` green, `cargo clippy --workspace --all-targets -- -D warnings` clean, `zvm`/`gvm` still zero-dep (`cargo tree -p zvm`, `cargo tree -p gvm` unchanged).

## Self-Review notes (author)

- **Spec coverage:** alignment scope (T7 block + x_off), targets transcript (T5/T7/T8) + non-primary buffers (T6/T9), skip-when-off (T7 `images_enabled`), flow_break no-op (T10), theming (T11), zero-dep VMs (only `gvm` selector touched, no deps). ✓
- **Type consistency:** `InlineImage`/`ImageAlign` (T1) used identically in state (T2), backend (T3/T4), engine (T6), transcript (T7); `WrappedRow` struct migration (T7) consumed by draw loop (T8) and buffer render (T9); `ImageBand` produced in T7, consumed T8/T9; `TranscriptElem` produced T4, consumed T5. ✓
- **Ordering caveat:** T8 references `state.colors.inline_image` (T11). Either run T11 before T8 or use `state.colors.graphics` as a placeholder in T8/T9 and switch in T11 (noted in T8 Step 4 and T11 Step 3).
- **Verify-before-write flags:** Glk `imagealign` constants (T1), `glk_window_flow_break` selector value + result convention (T10), `image` 0.25 API names `to_rgba8`/`resize_exact`/`crop_imm` (T4/T8), `Picker::font_size`/`Image::new(&proto)` shapes (T8), and the exact non-persistence mechanism for `transcript_styles` (T2) — each task instructs the implementer to confirm against the real code before writing.
