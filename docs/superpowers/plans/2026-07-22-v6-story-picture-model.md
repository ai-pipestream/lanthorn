# v6 Story-Window Picture Model — Correcting the Zork Zero Render

**Goal:** Render v6 story-window pictures (drop-caps, inline art) at their game-commanded
positions with text flowing correctly around them, matching Zork Zero's reference layout.

**Architecture:** The Zork Zero boot trace (verified deterministically, `v6_optrace_scratch`)
shows the game draws story content as: a full-screen frame (win7: banner #5 + columns
#497/#498), a per-room compass assembled from 8 overlay tiles into the banner strip (win1
@ (3,3)→abs (4,4), mostly occluded), and **inline story pictures into win0** — an illuminated
drop-cap #2 (42×35, an ornate "A") at (12,12), followed by `@set_margins(left=56, win=0)` so
the paragraph wraps to its right. We had been mis-modelling win0 pictures as a relocated
"room illustration" drawn above the text.

**Tech Stack:** Rust; `crates/app` render pipeline (`render/v6_layout.rs`, `render/screen.rs`),
zero-dep `crates/zvm` for the engine trace.

## Global Constraints

- zvm/gvm/scott stay ZERO external deps; app may add deps.
- Stage files explicitly by path; never `git add -A`. Ask before commit/push.
- Full `cargo test` (workspace) green before declaring done.
- No back-compat shims (pre-release).
- Delete all throwaway scratch harnesses before the final commit
  (`v6_optrace_scratch.rs`, `v6_pics_scratch.rs`, `v6_roomtrace_scratch.rs`,
  `v6_faithful_scratch.rs`, `v6_render_png_scratch.rs`, `v6_iter_scratch.rs`).
- Commit trailers: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`,
  `Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh`, `Quest: SQ-0186`.

## What is already done (this session, uncommitted)

`build_chrome_canvas` (`v6_layout.rs`) now blits each chrome graphics window's canvas **1:1
in native pixels** at the window origin, instead of scaling it to the window's declared box.
This fixes the compass being squashed from 48×43 into the 320×5 status window. Task 1 covers
committing it with a regression test.

## The deterministic facts (do not re-derive)

Boot draw order & positions (window origins: win7=(0,0), win1=(1,1), win0=(6,6);
`draw_picture` coords are window-relative pixels):

| pic | win | rel (x,y) | abs (x,y) | size | role |
|-----|-----|-----------|-----------|------|------|
| 5   | 7   | (1,1)     | (1,1)     | 320×34 | banner bg (compass baked at centre) |
| 497 | 7   | (1,35)    | (1,35)    | 36×166 | left column |
| 498 | 7   | (284,35)  | (284,35)  | 37×166 | right column |
| 17,10,11,20,13,22,15,24 | 1 | (3,3) | (4,4) | 45×40 ea | compass overlay tiles (stack → per-room rose) |
| 481 | 1   | (5,5)     | (6,6)     | 10×7   | small marker |
| 2   | 0   | (6,6)     | (12,12)   | 42×35  | **illuminated drop-cap "A"** |
| 216 | 0   | (6,6)     | (12,12)   | 21×21  | small inline glyph |

After #2: `@set_margins(left=56, right=0, win=0)`. After #216: `@set_margins(left=32, win=0)`,
then reset. These margins are the game telling us the text gutter to reserve beside each pic.

## File Structure

- `crates/app/src/render/v6_layout.rs` — replace `draw_story_gfx` (text-below model) with a
  drop-cap placement + gutter helper; `MainText`/`draw_story_text` gain a leading-indent band.
- `crates/app/src/render/screen.rs` — the `Layered` arm wires picture geometry → text wrap.
- Tests colocated in each file's `#[cfg(test)]` module.

---

## Task 1: Commit the compass 1:1-blit fix with a regression test

**Files:**
- Modify: `crates/app/src/render/v6_layout.rs` (already edited — `build_chrome_canvas`)
- Test: `crates/app/src/render/v6_layout.rs` `#[cfg(test)]`

**Steps:**
- [ ] Add a test `chrome_graphics_blits_native_not_scaled_to_window`: build a chrome
  `PositionedWindow` whose Graphics canvas is 48×43 but whose `w_px×h_px` is 320×5, place an
  opaque marker pixel at canvas (40,38); assert the composited native canvas has that pixel
  opaque at (origin_x+40, origin_y+38) — i.e. NOT squashed into a 5px band.
- [ ] `cargo test -p app render::v6_layout` → PASS.
- [ ] Commit (ask first) with the trailers.

## Task 2: Draw win0 pictures at commanded position; drop-cap gutter for text

**Files:**
- Modify: `crates/app/src/render/v6_layout.rs` — `draw_story_gfx` → `place_story_gfx` returning
  `StoryGfx { gutter_cols: u16, band_rows: u16 }` instead of a text-top y. It blits the win0
  canvas at the story origin (sx,sy) and measures the opaque picture's cell footprint:
  `gutter_cols = ceil(opaque_w/8) + 1`, `band_rows = ceil(opaque_h/8)`.
- Modify: `MainText` — add `gutter_cols: u16` and `band_rows: u16` (default 0). `draw_story_text`
  draws the first `band_rows` rows shifted right by `gutter_cols*8` px; rows past the band start
  at `ox`.

**Steps:**
- [ ] Write test `story_text_wraps_right_of_dropcap`: a `MainText` with `gutter_cols=6`,
  `band_rows=4`, and lines long enough to exceed `cols-6`; assert (via a rasterization probe or
  a pure wrap helper) that the first 4 rows begin at column 6 and later rows at column 0.
- [ ] Implement `place_story_gfx` + the `MainText` band fields + `draw_story_text` indent.
- [ ] `cargo test -p app` → PASS.

## Task 3: Wire the gutter into build_main_text + the Layered arm

**Files:**
- Modify: `crates/app/src/render/screen.rs` — `build_main_text(state, cols, rows, gutter_cols,
  band_rows)`: wrap the leading `band_rows` display rows at width `cols - gutter_cols`, the rest
  at `cols`. The `Layered` arm calls `place_story_gfx` first, passes its `gutter_cols/band_rows`
  into `build_main_text`, and stops using the old text-top return.

**Steps:**
- [ ] Test `build_main_text_narrows_leading_band`: transcript with one long paragraph, `cols=40`,
  `gutter_cols=7`, `band_rows=4` → first 4 rows ≤ 33 cols wide, row 5+ up to 40.
- [ ] Implement; delete the now-unused `draw_story_gfx` text-top path.
- [ ] `cargo test -p app` → PASS.
- [ ] Render `v6_render_png_scratch` and visually confirm: drop-cap "A" top-left, intro text
  wrapping to its right, banner/columns intact, compass where the game commands it.

## Task 4: Cleanup + full suite

**Steps:**
- [ ] Delete all throwaway scratch test files listed in Global Constraints.
- [ ] `cargo test` (whole workspace) → PASS; `cargo clippy -p app` clean.
- [ ] Commit (ask first).

## Deferred to Phase 2 (capture as side-quest, do NOT build now)

Full inline-image-in-transcript + margin-float renderer: route win0 `draw_picture` into
`TranscriptElem::Image` at the live text position and honor `set_margins` per-paragraph as text
prints (the "future margin-float renderer" the `inline_image.rs` comment anticipates). This makes
pictures anywhere in the scrolling story flow correctly — not just a leading drop-cap. The Task 2/3
geometry heuristic is correct for the boot screen (leading drop-cap) but ties the gutter to the
top-of-transcript, so it degrades once the drop-cap paragraph scrolls off. Phase 2 removes that
limitation.
