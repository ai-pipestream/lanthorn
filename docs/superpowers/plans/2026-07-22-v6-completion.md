# v6 Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development.
> Controller (session lead) dispatches one agent per lane, reviews every diff, and owns
> all commits. Implementer agents DO NOT commit and DO NOT touch files outside their lane.

**Goal:** Finish v6 (graphical Z-machine) support: hybrid render (raster chrome +
crisp terminal story text with embedded images), mouse input, colour fidelity,
remaining opcodes, other-v6-game smokes, persistence, docs. Quest: SQ-0186.

**Architecture:** The raster foundation is DONE and verified against the Amiga
reference (commits `633833e9`, `8e5381fe`): Rect placement pictures drive the
game's own layout; chrome composites at native 320×200 with spec clipping;
window-0 pictures are transcript-anchored floats; status text renders at exact
pixel positions. What remains is built ON that foundation — nothing below
re-litigates it.

**Tech Stack:** Rust workspace. `zvm` stays ZERO external deps. `app` may add deps.

## Global Constraints

- zvm/gvm/scott: zero external dependencies. Never add any.
- Implementer agents: NO commits, NO `git add`. Work ONLY in your lane's listed
  files/dirs. Report status + test commands + output summaries.
- Full `cargo test` (workspace) must be green before a lane is accepted.
- Verify spec claims against ZMSD 1.1 (inform-fiction.org/zmachine/standards/z1point1)
  — never from memory. Blorb spec: eblong.com/zarf/blorb/blorb.html.
- No back-compat: pre-release, formats may break freely; no shims.
- Every new UI element themeable via style.toml (ColorScheme/theme selector).
- The v6 render pipeline: `session::v6_screen_model` → `WinNode::Layered` →
  `render/screen.rs` Layered arm → `render/v6_layout.rs` helpers →
  `render/graphics.rs::draw_v6_canvas` (single aspect-preserving scale).
- PNG oracle for visual work: composite the native canvas, upscale 3×, save PNG
  (see the pattern in git history for `v6_render_png_scratch.rs`), and STATE in
  your report what the image shows. Scratch harnesses are throwaway: name them
  `*_scratch.rs`, list them in your report for the controller to delete.

---

## Lane H — Hybrid mode (Opus) — `crates/app/src/render/{graphics,screen,v6_layout}.rs`, `crates/app/src/config.rs`

The centerpiece. `v6_render = hybrid` (already the config default, currently
routed to the raster path) becomes: chrome drawn as an image RING around the
story viewport; the story viewport renders as REAL terminal text via the
existing transcript renderer (crisp, selectable, scrollable, styled, with
inline images as bands).

### Task H1: chrome bands geometry + ring draw
- `pub fn chrome_bands(pane: Rect, viewport: Rect) -> Vec<Rect>` in v6_layout:
  up to 4 non-overlapping cell rects (top, bottom, left, right) tiling
  `pane − viewport`; edge-flush viewport ⇒ that band omitted; viewport == pane
  ⇒ empty. TDD: geometry tests first.
- `GraphicsRender::draw_chrome_band(picker, chrome_canvas, scale, band, buf)`:
  render the crop of the SCALED chrome canvas under `band`'s device region as
  one image placement; cache per (content hash, band rect) like `draw_v6_canvas`.
- The story viewport cell rect: map the win0 box (`layout.story` x_px/y_px/w_px/h_px,
  native px) through the letterbox `Scale` (`v6_layout::uniform_scale`) to device
  px, then to whole cells (round INWARD so no chrome cell overlaps the viewport).

### Task H2: terminal story in the viewport
- In the Layered arm's Hybrid branch: draw chrome bands, then call the primary-
  buffer path (`render_transcript` via the existing primary `Buffer` handling)
  with `area = viewport`; return its real `StoryPaneMetrics` (scrollbar, links).
- Inline story images (`transcript_images` sidecar) render through the EXISTING
  inline-image band renderer in that transcript path — verify Zork0's drop-cap
  and statue appear as bands (margin-float in terminal cells is NOT in scope).
- No-picker fallback and `raster` mode byte-identical to today. Tests: hybrid
  returns Some(metrics); chrome canvas viewport region untouched by story text;
  raster path unchanged (existing tests keep passing).

### Task H3: integration
- Zork0 integration test exercising both modes; `cargo test` green; clippy clean.
- Report a PNG of raster mode + a cell-dump (Buffer render to text) of hybrid.

## Lane Z — zvm v6 opcode completion (Sonnet) — `crates/zvm/src/cpu/exec.rs`, `crates/zvm/src/screen.rs`

- `scroll_window` (EXT:0x14, window, pixels signed): for grid windows 1–7 shift
  `texts[].y` by −pixels, drop runs fully outside `[1, y_size]`; shift the cell
  grid by whole rows (pixels/8, toward zero); window 0 → record a diagnostic
  once (host transcript owns win0 scrolling). Trace line under trace_screen.
- `picture_table` (EXT:0x1C): formal no-op (cache hint) + trace line; stop
  routing it through the silent stub arm.
- `buffer_screen` (EXT:0x1D): track the current mode flag and return the real
  previous mode (still no rendering effect). Verify operand/store behavior
  against ZMSD §15 text quoted in your report.
- `window_style` (EXT:0x12): keep storing attributes; ADD honoring of operation
  codes 0–3 (set/set-bits/clear-bits/XOR) if not already exact — verify + test.
- TDD each; do not touch draw_picture/set_cursor/margins paths (frozen by lane H review).

## Lane S — v6 game smokes (Sonnet) — NEW files in `crates/app/tests/` only

- `zork0_v6_gameplay.rs`: boot Zork0 (skip-if-missing pattern from
  `zork0_v6_windows.rs`), submit "ne" (and a few more moves), assert: no fault;
  compass overlay draw events re-fire on room change targeting window 1 at
  x=139 (the Rect-derived centre); the layered model stays sane (win0 box
  unchanged). Document with a comment which ops the game emits on movement.
- `v6_titles_smoke.rs`: for each of `stories/Arthur.blb`, `stories/Shogun.blb`,
  `stories/Journey.blb` (each skip-if-missing): resolve blorb → boot the ZCOD
  exec headless with picture dims + std window (mirror zork0_v6_windows setup)
  → step-capped to first input → assert no fault, layered model root, nonzero
  window geometry. Collect `machine.diagnostics` and REPORT them verbatim
  (unimplemented-opcode findings are the deliverable, not fixes).
- No production-code changes. If a smoke exposes a bug, report it; do not fix.

## Lane M — mouse input (Opus) — `crates/zvm/src/cpu/exec.rs` + `crates/app/src/{input.rs,render/graphics.rs,session.rs}`

- zvm: `Machine::set_mouse(y_px, x_px, buttons)` host API → writes the header
  EXTENSION table words 1/2 (mouse x/y; verify §8.9/§11.1.7 exact word order
  from the spec) and records button state. `read_mouse` (EXT:0x16) fills its
  4-word array (y, x, buttons, menu=0) from that state. `mouse_window`
  (EXT:0x17) stores the constraint window (−1 = none).
- Input delivery: a click while a v6 game awaits `read_char` delivers ZSCII 254
  (single) / 253 (double) — verify codes in §3.8 before coding.
- app: store the last letterbox `Scale`+offset used by `draw_v6_canvas`; map a
  terminal MouseEvent in the story pane (cell + subcell estimate) → device px →
  native game px → `set_mouse` + key delivery. Zork0 acceptance: clicking the
  banner compass at a lit direction issues a move (COMPASS-CLICK): drive it
  headlessly by calling the mapping fn directly in a test.
- Config: no new flags. Respect existing mouse handling for app chrome (only
  clicks landing INSIDE the game pane's v6 image reach the VM).

## Lane C — colour fidelity (Sonnet) — `crates/app/src/render/v6_layout.rs` (+ small session plumbing if needed)

- Status/chrome text: `px_texts` runs already carry packed fg/bg + style bits.
  Honor them in `build_chrome_canvas`: resolve `ZColour::Standard/True` to RGB
  (existing `packed_to_rgba`); style bit 1 (reverse) swaps fg/bg (with the
  window's colours as the base pair). Zork0's ribbons then show the game's
  intended dark-on-tan text.
- Story region: fill the win0 box with the window's own bg colour (win0
  `bg`/`colour_data`) before text draws, instead of leaving transparency over
  the terminal backdrop — Zork0's grey page. Themeable fallback when the game
  sets no colour (existing transcript theme bg).
- PNG oracle before/after in the report. Do not touch text-wrap/float logic.

## Lane P — v6 persistence (Opus) — `crates/app/src/{session.rs,archive.rs,reset.rs}` (+ zvm serialization if required)

- Investigate what the host Save State snapshot currently captures for v6
  (machine memory/stack yes; V6Windows? pictures_canvas? transcript_images?).
- Requirement: save mid-Zork0, restore → next render is visually identical.
  Prefer replay-free: serialize V6Windows (geometry/cursors/margins/texts) with
  the machine state, and pictures_canvas as PNG blobs in the archive; restore
  `transcript_images` floats or document their loss precisely. No back-compat
  shims — bump the snapshot format freely.
- Quetzal `@save`/`@restore` path: verify no crash and that Zork0 redraws its
  chrome after restore (the game re-runs INIT-STATUS-LINE); smoke it headless.
- PNG-compare test: render-before vs render-after-restore byte-equal (or
  documented, asserted delta).

## Lane D — docs (Sonnet) — `README.md`, `docs/features*`, `docs/architecture*`

- v6 support is a MAJOR feature: README section (lively tone, matches existing
  voice) + feature/architecture pages: the three render modes, the Rect
  placement-picture mechanism, inline floats, pixel status text, mouse.
- Verify every claim against the code as of this branch. No overclaiming;
  proportional font (SQ-0450) and any not-landed lane stay unmentioned or
  marked upcoming.

---

## Execution order (hot-file lanes; controller reviews between waves)

- **Wave 1 (parallel):** H (Opus) + Z (Sonnet) + S (Sonnet) — disjoint files.
- **Wave 2 (parallel, after Wave 1 merges):** M (Opus) + C (Sonnet).
- **Wave 3 (parallel):** P (Opus) + D (Sonnet).
- Controller: review each lane's diff, run full workspace suite, commit per lane
  with `Quest: SQ-0186` trailers, then dispatch the next wave. Final: delete all
  `*_scratch.rs`, full suite, user visual confirm (raster + hybrid in a real
  terminal), then merge to main per the no-PR workflow.
