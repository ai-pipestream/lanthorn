# Inline Images in Text-Buffer Windows — Design

**Date:** 2026-07-03
**Status:** Approved (design), pending spec review
**Surface:** "Surface A" from `2026-07-03-glulx-graphics-windows-design.md` (previously deferred)

## Goal

Render Glk images drawn into **text-buffer windows** (the main transcript and
non-primary buffer windows) as they flow with the text, via
`glk_image_draw` / `glk_image_draw_scaled` with an `imagealign` argument. This
is distinct from the already-shipped **graphics windows** (dedicated pixel
canvases).

## Decisions (locked)

| Question | Decision |
|----------|----------|
| Alignment scope | **Inline + margin-as-block.** Accept all 5 `imagealign` modes; render every image as a full-width block. Honor `glk_window_flow_break` (no-op in block mode). Store the real alignment so true float can be added later as a pure render change. |
| Target windows | **Primary transcript AND non-primary buffer windows.** |
| Fallback (no image protocol / graphics toggled off) | **Skip entirely** — image occupies zero rows, reappears if graphics re-enabled. |
| Vertical inline alignment (`InlineUp/Down/Center`) | Dropped in block mode; the flag value is still stored. |
| Horizontal placement | `MarginRight` → right-aligned within the band width; all others → left-aligned. |

## Non-goals (this pass)

- True margin **text-wrap-around** float (text beside the image).
- Any **visible** effect from `glk_window_flow_break` (accepted but a no-op).
- Mouse / hyperlink interaction on images.
- Persisting inline images across save/restore.

## Background: the existing seam

Reconnaissance established the exact model this design leans on.

**Transcript model** (`crates/app/src/state.rs:908`): the primary transcript is a
set of length-synced parallel Vecs indexed by *logical line*:
`transcript: Vec<String>`, `transcript_kinds: Vec<TranscriptKind>`,
`transcript_styles: Vec<Option<Style>>` (in-memory only),
`transcript_runs: Vec<Vec<StyleRun>>`. `TranscriptKind` (`state.rs:191`) is a
category (`Story`/`Input`/`Meta`/`Warning`), not a content-type discriminator.

**Wrapping** (`crates/app/src/render/transcript.rs:340` `wrap_lines_kinded`):
turns each logical line into N `WrappedRow = (String, TranscriptKind, Style,
Vec<StyleRun>)`. Crucially, every downstream consumer already counts *wrapped
rows*, not logical lines:
- scroll clamp / window (`transcript.rs:428`)
- scrollbar sizing (`transcript.rs:1114`)
- clear-anchor top-pin (`transcript.rs:407`)
- smooth-scroll animation (`state.rs:1363`)

So a logical unit that expands to N wrapped rows is *already* a supported shape.
The only place that assumes "1 wrapped row = 1 text row" is the **draw loop**
(`transcript.rs:1091`), which maps each wrapped row to one buffer row and calls
`draw_str_runs`.

**Append path** (`crates/app/src/glk_backend.rs`): the VM prints into a
per-window `BufBuf { log: Vec<(u8,u32,u32,String)>, drained, scroll }`
(`glk_backend.rs:76`). The **primary** buffer is drained each turn by
`take_transcript` (`glk_backend.rs:188`) into a `TurnResult`
(`session.rs:117`), then pushed into State's parallel Vecs by
`push_transcript_runs` (`state.rs:1560`). **Non-primary** buffers are rebuilt
each frame from the same log by `log_to_lines` (`glk_backend.rs:419`) into a
`BufferWindow` (`engine.rs:154`). `AppGlk` never touches `AppState`; everything
crosses as a per-turn snapshot.

**Graphics precedent** (`crates/app/src/render/graphics.rs`): pixels reach the
renderer as `Arc<RgbaImage>` on `GraphicsWindow` (`engine.rs:177`), and
`GraphicsRender::render` (`graphics.rs:26`) builds/caches a `ratatui-image`
`Protocol` per `(win, version, w, h)` and blits it. Inline images reuse this
blit + cache approach and the same `state.game_picker`.

## The design

### 1. New types (app crate)

```rust
// crates/app/src/state.rs (or a small new module reused by engine.rs)
pub enum ImageAlign { InlineUp, InlineDown, InlineCenter, MarginLeft, MarginRight }

pub struct InlineImage {
    pub pixels: Arc<image::RgbaImage>, // travels with the element, like GraphicsWindow.canvas
    pub align: ImageAlign,
    pub scaled: Option<(u32, u32)>,    // requested target px for image_draw_scaled; None = native
}
```

`InlineImage` is **not** a `StyleRun` (a `StyleRun` is a char-span within one
line's text and cannot represent a non-text element).

### 2. Data-model additions

- **Primary transcript:** one new parallel Vec on `AppState`:
  `transcript_images: Vec<Option<InlineImage>>`, length-synced with the other
  transcript Vecs, **non-persisted** (reset/ignored on save/restore, like
  `transcript_styles`). An image is pushed as a logical unit with: an empty
  placeholder `String`, `kind = TranscriptKind::Story` (so existing
  `visible_transcript_indices` filtering includes it with no enum change),
  `transcript_styles = None`, empty `runs`, and
  `transcript_images[i] = Some(InlineImage{..})`.
- **Non-primary buffers:** `BufferWindow` (`engine.rs:154`) gains
  `images: Vec<Option<InlineImage>>`, length-synced with `lines`/`runs`.

### 3. Backend log becomes an ordered enum

`BufBuf.log`'s element type changes from the text-only tuple to:

```rust
enum BufElem {
    Text { bits: u8, fg: u32, bg: u32, text: String },
    Image { img: InlineImage },
}
```

so text and images interleave in true emission order. `put_text_attr`
(`glk_backend.rs:505`) pushes `BufElem::Text`; the new image path pushes
`BufElem::Image`.

### 4. VM → app routing

`gvm` already forwards both args of `glk_image_draw`/`_scaled` to
`AppGlk::graphics_draw_image(win, resnum, val1, val2, scale)`
(`gvm/src/exec.rs:2724`, `2733`). No `gvm` change is needed for the draw itself.

`AppGlk::graphics_draw_image` (`glk_backend.rs:586`) **branches on the target
window's type**:
- **Graphics window** → existing `Canvas` path, unchanged (`val1`/`val2` = x/y).
- **Buffer window** → reinterpret **`val1` as the `imagealign` flag** (`val2`
  ignored), resolve the Pict via `self.picts.image(resnum)` into an
  `Arc<RgbaImage>`, and push `BufElem::Image` into that window's log. Scaled
  draws carry `scale` into `InlineImage.scaled`.

`glk_window_flow_break`: add a selector arm that accepts and ignores it (block
mode already breaks the flow), replacing the current "unhandled selector"
diagnostic. **Verify the selector constant and the `imagealign_*` constant
values against the Glk spec / `GLULX_NOTES` during planning** — do not hardcode
from memory.

`zvm`/`gvm` stay zero-dependency: all image handling lives in the app crate.

### 5. Drains carry images through

- `take_transcript` (`glk_backend.rs:188`) and the `TurnResult`
  (`session.rs:117`) gain an **ordered element channel** (text + image markers)
  instead of a bare `String` + chunk list. `push_transcript_runs`
  (`state.rs:1560`) consumes it: text elements split into lines as today; image
  elements push one image unit (§2) at their in-order position.
- `log_to_lines` (`glk_backend.rs:419`) emits image markers into
  `BufferWindow.images` at their in-order position.

### 6. Rendering: block bands

`wrap_lines_kinded` (`transcript.rs:340`) is passed `char_px` and the
`transcript_images` slice. For an image unit:
- Compute the fitted cell footprint from `pixels` (or `scaled`) × band width ×
  `char_px`, **aspect-preserved and capped to the band width**.
- Emit that many placeholder `WrappedRow`s, each tagged with the image and a
  sub-row index. `WrappedRow` is extended to carry an optional band descriptor
  (`Option<ImageBand>`); text rows leave it `None`.

The draw loop (`transcript.rs:1091`) detects band rows and blits the image into
the band rect via the `ratatui-image` `Picker`/`Protocol` cache (shared with or
mirroring `GraphicsRender`). Horizontal placement per §Decisions. Padding cells
around a narrower-than-band image use the themed letterbox (§8).

**Partial scroll:** when a band is partly scrolled out of the viewport,
proportionally **crop the source image** to the visible sub-band and fit the
crop into the visible rect, so scrolling through an image degrades cleanly
rather than overflowing the transcript area.

The same band-expansion + blit logic is factored into a shared helper used by
both `render_transcript` and `render_inline_buffer` (`render/screen.rs:294`) to
avoid duplication.

### 7. Fallback

When `state.game_picker` is `None` **or** game graphics are toggled off, an
image unit emits **0 band-rows** in `wrap_lines_kinded` — invisible, zero
footprint, reappears if graphics are re-enabled.

### 8. Theming

Add an `inline_image` letterbox/background style to `ColorScheme` plus a
`style.toml` selector (mirrors the existing `graphics` letterbox), governing the
padding cells when an image is narrower than its band. No placeholder chrome to
theme (fallback is skip).

## Testing

- `wrap_lines_kinded`:
  - image unit expands to the expected band-row count at a given width/char_px;
  - **0 rows** when images are off;
  - band-row count **reflows** when width changes;
  - `MarginRight` right-aligns; others left-align;
  - interleaving order (text / image / text) is preserved.
- Backend routing:
  - `image_draw` to a **buffer** window produces a `BufElem::Image`;
  - `image_draw` to a **graphics** window still hits the `Canvas` path;
  - `image_draw_scaled` records `scaled`.
- Drain order: a text/image/text emission sequence survives `take_transcript`
  and `push_transcript_runs` in order (primary), and `log_to_lines` in order
  (non-primary).

## Touched components (for planning)

- `crates/gvm/src/exec.rs` — accept `glk_window_flow_break` selector (no-op).
- `crates/app/src/glk_backend.rs` — `BufElem` enum; `graphics_draw_image`
  window-type branch; `take_transcript` / `log_to_lines` carry images.
- `crates/app/src/session.rs` — `TurnResult` element channel.
- `crates/app/src/state.rs` — `InlineImage`/`ImageAlign`; `transcript_images`
  Vec; `push_transcript_runs`.
- `crates/app/src/engine.rs` — `BufferWindow.images`.
- `crates/app/src/render/transcript.rs` — `wrap_lines_kinded` band expansion;
  `WrappedRow` band descriptor; draw-loop blit; partial-scroll crop.
- `crates/app/src/render/screen.rs` — `render_inline_buffer` band support.
- `crates/app/src/render/graphics.rs` — shared/mirrored blit + protocol cache.
- `crates/app/src/colors.rs`, `style.rs` — `inline_image` theme selector.

## Global constraints

- `zvm` and `gvm` crates stay **zero-dependency**; all image decode/blit stays
  in the app crate.
- Every new UI element is **themeable** via `style.toml` (the `inline_image`
  selector).
- README covers **major features only** — inline images qualify for a README
  note; per-title fixes do not.
- Cross-platform (Windows/Linux/macOS); no platform-specific APIs.
