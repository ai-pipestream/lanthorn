# v6 Side-Quest Completion Plan (2026-07-23)

> **For agentic workers:** executed via subagent lanes (Opus = architecture/judgment,
> Sonnet = mechanical). The controller reviews every lane diff, runs the full gate
> (`cargo test -p zvm -p app`, watching for COMPILE errors in test targets, not just
> failures), and commits per lane with `Quest:`/`Completes:` trailers. Lane agents do
> NOT commit.

**Scope:** every open v6-related quest after the paint-semantics work landed
(`7859b97a`): SQ-0450, 0451, 0453, 0454, 0455, 0457 (remainder), 0459, 0460, 0461.

## Global constraints

- `zvm` stays **zero external deps**. Spec claims verified against ZMSD 1.1
  (inform-fiction.org) or Frotz source — never memory.
- v6 text/pixel model is **paint semantics**: `V6Text` runs are screen-absolute,
  stamped at paint time; erase is rect-based; win0 wrapping bit routes
  stream-vs-paint. Do not regress this.
- Every new user-facing UI element is themeable via `style.toml` selectors.
- User-facing changes update docs (README only for major features; feature docs
  otherwise) in the same lane.
- Tests: TDD where the behavior is assertable; real-game smokes use the
  skip-if-missing pattern (stories are gitignored).
- Wave-1 lanes run in parallel in THIS worktree on disjoint files; do not touch
  another lane's files.

## Wave 1 (parallel, disjoint files)

### Lane V1 — SQ-0453: titles smoke → bare .z6 executables (Sonnet)
**Files:** `crates/app/tests/v6_titles_smoke.rs` only.
The three `.blb` files are resources-only; the real executables are
`stories/arthur-r74-s890714.z6` and `stories/journey-r83-s890706.z6` (Shogun is
covered by `v6_shogun_gameplay.rs` — drop it from this smoke or boot it the same
way for symmetry). Boot each bare `.z6` with its Blorb sidecar resolved via
`blorb::resolve_resource_blorb`, mirroring `v6_shogun_gameplay.rs`'s setup.
Assert: no fault, no quit at first input, `Layered` root, and (where present)
nonzero window geometry; drive 2–3 safe inputs (whatever `pending_input` asks
for) fault-free. Keep skip-if-missing.
**Accept:** smoke runs (not skips) for arthur + journey when stories exist.

### Lane V2 — SQ-0460: withhold arrow keys from v6 games (Sonnet)
**Files:** `crates/app/src/config.rs` (or wherever run options live — follow the
`--no-sound`/`v6_render` precedent), `crates/app/src/main.rs` (the arrow→ZSCII
129-132 forwarding site in the key-delivery path), docs
(`docs/features/v6-graphics.md` + config/customization reference).
Add config option + CLI flag (e.g. `v6_arrow_keys = true|false`,
`--no-v6-arrows`) — when off, arrow keypresses are NOT forwarded to a v6 story
as ZSCII 129-132 (they fall through to app-side handling, e.g. scrollback/map
panning); Enter/other keys unaffected. Default: current behavior (forward).
**Accept:** unit/integration test that the delivery function drops arrows when
configured; docs updated.

### Lane V3 — SQ-0457 remainder: v6 opcode polish (Sonnet)
**Files:** `crates/zvm/src/cpu/exec.rs` (+ its tests). erase_line is DONE
(`7859b97a`) — do not touch it.
1. `output_stream 3 ... width` (v6 third operand, ZMSD §7.1.2.1/§15): store the
   width operand with the stream-3 frame; on close, if a width was given,
   left-pad/justify per spec (positive = field width right-justified text;
   negative = left-justified) when writing the table. Verify exact wording from
   the spec FIRST and quote it in the code comment.
2. `set_font` (EXT:0x04) v6: honour the optional window operand (incl. -3 via
   `v6_window_operand`) and mirror the font into that window's prop 12
   (`font_number`); return the previous font as now.
3. `set_text_style` (VAR:0x11): in v6, mirror the style into the CURRENT
   window's prop 10 (`text_style`) so `get_wind_prop(w,10)` reads fresh.
4. `print_form`/`make_menu` stay stubs — add a one-line comment each naming
   SQ-0457 as their tracking quest. No behavior change.
**Accept:** unit tests for 1-3 (spec-quoted); full zvm suite green.

### Lane V4 — SQ-0459: Inform v6 library rendering (Opus)
**Files:** `crates/app/src/session.rs` (v6_screen_model + win0 diversion seam),
possibly `crates/zvm/src/cpu/exec.rs` print routing (coordinate with V3 — if
you must touch exec.rs, ONLY the print_text v6 routing block).
Problem: Inform 6's v6 library prints story prose into **window 7** (its
"main" window) with all windows at (1,1), height 0, cursor-partitioned layout —
our transcript diversion only watches window 0, so advent.z6 shows an empty
transcript in hybrid mode and the window model renders nothing (windows with
y_size 0 are skipped).
Investigate with `stories/advent.z6` headless (see `v6_inform_titles.rs`).
Design goal: detect the Inform-library shape (win0 never printed to, prose
flowing in a higher window with scrolling attribute set / wrapping on) and
divert THAT window's stream output into the transcript the way win0's is; the
same rules (wrapping bit = paint vs stream) apply. Zork0/Shogun behavior must
be byte-identical (their smokes are the guard).
**Accept:** advent.z6 headless: "look" text reaches the transcript
(strengthen `v6_inform_titles.rs`); all existing v6 smokes untouched-green.

## Wave 2 (after wave 1 lands; mostly disjoint render files)

### Lane V5 — SQ-0455: raster scrollback + [MORE] (Opus)
**Files:** `crates/app/src/render/screen.rs` (raster branch, `build_main_text`),
`crates/app/src/render/v6_layout.rs` (`draw_story_text`).
Raster mode renders the tail of the transcript into the story box with no
scrollback and no paging: text scrolls off unseen. Honour
`state.effective_transcript_scroll()` when selecting the transcript slice
(build_main_text), publish `TranscriptGeom` (viewport rows/cols) from the
raster path the way hybrid does so the existing scroll keys/[MORE] pager
(SQ-0404 machinery) engage, and add turn-paging: when a single turn's new text
exceeds the story box rows, hold at [MORE] per screenful (reuse the existing
pager state; the [MORE] indicator may render as terminal text under the pane —
themeable selector).
**Accept:** unit test: scrolled build_main_text slices correctly; smoke:
zork0 raster path publishes geometry; scroll offset changes the rendered rows.

### Lane V6 — SQ-0454: margin-float renderer in hybrid (Opus)
**Files:** `crates/app/src/render/transcript.rs` (+ `inline_image.rs` if
needed).
Hybrid currently renders inline story images as full-width bands (own line).
Implement margin floats: an image with `margin_px` (from the game's
set_margins-after-draw idiom) renders as a left-margin float with the
following text lines wrapped beside it (indent = ceil(scaled_img_width /
cell_width) for ceil(scaled_img_height / cell_height) rows), like the raster
path's `RasterFloat`. Fall back to band rendering when the image is wider than
~half the viewport.
**Accept:** unit test on the wrap computation; zork0 hybrid smoke still green;
visual verify deferred to user (note in report).

### Lane V7 — SQ-0450: raster font quality pass (Sonnet)
**Files:** `crates/app/src/render/bitfont.rs` only (glyph data + blit).
The 8x8 bitfont renders chunky at scale. Goal: match the Amiga look more
closely — add a proper 8x8 glyph set revision pass (cover ASCII + the
box-drawing/arrow glyphs v6 games use; document glyph provenance — must be an
ORIGINAL or clearly-free bitmap set, no copyrighted ROM fonts), and implement
smoother upscaling for text glyphs if cheap (e.g. scale2x-style edge smoothing
behind the existing scale factor). No API changes.
**Accept:** bitfont unit tests (glyph coverage, blit bounds); PNG-oracle
composite generated to `target/` for the controller to eyeball.

## Wave 3 (serialized; conflicts with earlier lanes)

### Lane V8 — SQ-0461: frameless v6 mode (Opus, after V5)
**Files:** `crates/app/src/render/screen.rs`, `crates/app/src/config.rs`, docs.
New `v6_render = "text"` (name TBD by lane — propose in report) mode: skip the
chrome/frame entirely; render the story as the normal full-pane terminal
transcript (like the cell fallback but deliberate), with inline images
rendered via the existing transcript image path and chrome-grid text (status
rows) as a compact terminal status band. Explore and propose; wire behind the
config enum with docs. This is exploratory — a working first cut plus a
report on tradeoffs beats polish.

### Lane V9 — SQ-0451: v6 newline interrupt (Opus, after V3)
**Files:** `crates/zvm/src/cpu/exec.rs`, `crates/zvm/src/screen.rs`.
ZMSD §8.8.3.2 props 8/9: countdown decrements per newline printed in a window;
at 0, fire the interrupt routine (packed addr, prop 8). Two earlier prototypes
dead-ended — read the git history (`git log --grep=newline`) and the SQ-0451
quest notes first. Fire the routine synchronously from the print path via the
same mechanism as timed-input interrupts (`run_routine`), guard re-entrancy
(prop 9 = 0 while running), and verify against Frotz `screen_new_line`. Real
risk: Zork0 uses this for [MORE]-style pauses; the gameplay smokes are the
acceptance gate.

## Review protocol (controller)

Per lane: read the diff, check constraint adherence (zero-dep, paint
semantics, theming), run the FULL gate, then commit with
`Completes: SQ-XXXX` (or `Quest:` if partial) + standard trailers. Wave 2
starts only after all wave-1 lanes are committed.
