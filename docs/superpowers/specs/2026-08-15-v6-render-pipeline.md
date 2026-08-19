# The v6 render pipeline, end to end

**Status:** understanding document. No design, no plan, no code.
**Written against:** `main` @ `155b9b91`, tree clean.
**Why it exists:** SQ-0892's first lane was stopped because it was chasing symptoms without a
model of this path. This is the model. SQ-0892's measurements — the `$30` string-measurement
mechanism, the group-origin quantization finding, the text-over-art exception, the cleanup
mandate, the rasterization-inventory gate — are not repeated here; they stand as written.

## How to read this

Every claim carries a `file:line`. Every claim is tagged:

- **MEASURED** — I drove a capture on this machine and read the number off it. The captures are
  reproduced inline so you can re-run them.
- **READ** — I read it in the code at the cited line.
- **INFERRED** — I reasoned it from things I read. Treat with suspicion; this project has been
  burned by inference dressed as measurement more than once.

Unless stated otherwise, MEASURED numbers come from:

```sh
cargo run -q -p app --example pty_capture -- --story <fixture> \
  --user-dir <fresh> --size 100x40 --keys "…" --out <report>
```

which negotiates kitty at an 8x18 cell, giving a story pane of **98x37 cells = 784x666 device
pixels** inside the app frame. `/dump-windows` was driven through `--keys "…,text:/dump-windows,cr"`
and read out of `<user-dir>/dump-windows.log`.

---

## 1. The stages, in order

### 1.0 Before the renderer: the model

The renderer is handed a fresh `ScreenModel` **every frame**, not every turn.

`crates/app/src/main.rs:613` calls `engine.screen_now()` inside the `terminal.draw` closure.
`GameSession::screen_now` (`crates/app/src/session.rs:4424`) dispatches to `v6_screen_model`
for a v6 story. That function walks the eight v6 windows out of live machine state and emits
`WinNode::Layered(items)` (`crates/app/src/session.rs:3302-3308`). Nothing is cached: a repaint
with no turn in between rebuilds the whole tree. **READ.**

Each `PositionedWindow` carries the window's box in the game's **native pixels**
(`x_px`/`y_px` are the ZMSD §8.8.1 1-based coords minus one — `crates/app/src/session.rs:2997-3006`),
clipped to the screen by `v6_clip_box`. Three node kinds come out:

- `WinNode::Graphics` — a window with a live canvas, sorted by draw order and flattened
  (`crates/app/src/session.rs:3008-3027`, sort at `:3217`).
- `WinNode::Buffer { primary: true }` — the prose window, when it still has the wrapping
  attribute set (`crates/app/src/session.rs:3095-3117`). Its text is **not** in the node; the
  transcript is.
- `WinNode::Grid` with `px_texts: Vec<PxText>` — everything else. A `PxText` is one printed
  run at 1-based native pixel `(x, y)` with the Z-machine style byte and packed colours
  (`crates/app/src/session.rs:3186-3197`, type at `crates/app/src/engine.rs:174`). Runs are
  emitted exactly as the game printed them, with **no grouping or merging at production time** —
  this is why Shogun's menu arrives as fifteen single-character runs, which is SQ-0892's whole
  subject. **READ.**

One synthetic entry matters later: **frozen prose re-enters the model as a chrome Grid.** When
the game moves or resizes window 0 out from under prose it already printed, the engine hands the
host those runs in `win.retired`, and the session publishes them as a `WinNode::Grid` whose box
is the frozen text's own extent — not the window's (`crates/app/src/session.rs:3042-3088`). From
the renderer's point of view retired prose is indistinguishable from a status bar. **READ.**

### 1.1 `render_story_pane` — the entry

`crates/app/src/render/screen.rs:236` → `render_story_pane_frame` (`:249`).

Decides: the pane page (`state.v6_page_pair` at `:267`, the §8.3 Amiga/Macintosh machine pair),
and whether the model is "simple" (v1–5 grid-over-buffer) or generic. A v6 `Layered` root is
never simple. `content_bounds` (`:364`) deliberately returns the **whole pane** for a
`Layered` root (`:370-372`) — v6 content is pixels to be scaled, not cells to be clamped.

Hands on: `render_node(&model.root, …, inner=area, …)` at `:326`.

### 1.2 `render_node` → the `WinNode::Layered` arm

`crates/app/src/render/screen.rs:566`, Layered arm at `:665`. Everything below happens inside
this one arm. It is ~1,300 lines and it is where all four routing decisions and every layout
decision live.

**Stage A — routing predicates, before anything is scaled.**

| what | line | reads |
|---|---|---|
| `frameless` | `:681` | config |
| `story_box` (native rows/cols of the primary Buffer) | `:700-708` | the model |
| `has_menu` — a non-blank chrome run inside the story box on both axes, below `STATUS_BAND_ROWS` | `:709-735` | the model + `story_box` |
| `menu_over_art` — `has_menu && hybrid &&` any chrome Graphics window with an opaque pixel | `:757-762` | the model |
| the gate | `:772` | all of the above |

The gate at `:772` reads
`!any_modal_overlay_open() && !frameless && !(has_menu && hybrid && !menu_over_art)`.
Failing it drops to the **cell path** at `:2105`. **READ.**

**Stage B — classification.** `native_extent(items)` (`:776`, impl `v6_layout.rs:238`) gives the
native screen size; `classify_windows(items)` (`:777`, impl `v6_layout.rs:283`) splits the items
into `story` (the *first* primary Buffer), `story_gfx` (the *first* Graphics with `win == 0`),
and `chrome` (everything else, in input order). **READ.**

**Stage C — the ring/raster fork.** `:797-798`:

```rust
if state.config.v6_render == Hybrid && !menu_over_art {
    if let Some(story) = layout.story.filter(|s| !picture_takeover(s, &layout.chrome, layout.story_gfx, native)) {
```

`picture_takeover` (`:3942`) is four OR'd escape hatches; see §4. A frame that passes both takes
the **hybrid ring**. Everything else falls through to **raster** at `:2034`.

### 1.3 The hybrid ring, in execution order

This is the stage list a reader needs. Each entry is `line — what it decides — what it hands on`.

| # | line | decides | hands on |
|---|---|---|---|
| 1 | `:805-823` | frame-log boundary; drop the raster cache; force a band re-upload if the last frame took another path | — |
| 2 | `:838-845` | flood the pane with the story window's explicit bg, if any | `v6_story_page` |
| 3 | `:846` | `build_chrome_canvas(chrome, native, …)` — every chrome window's art **and rasterised text** into one native RGBA image | `canvas` |
| 4 | `:855-879` | painted ground, then window pages, onto `canvas` | `canvas` |
| 5 | `:886` | `uniform_scale(native, pane_dev)` — **scale #1, letterboxed and centred** | `scale_center` |
| 6 | `:892` | `build_graphics_canvas` — art only, no glyphs. This is the oracle for "is there art here?" | `gfx` |
| 7 | `:893-901` | flatten every chrome Grid's `px_texts` into one list | `chrome_runs` |
| 8 | `:910-912` | `slack`, then `hybrid_bottom_plan(story, gfx, chrome_runs, native, slack)` | `plan` ∈ {Letterbox, Extend, Frame, Menu} |
| 9 | `:922` | `top_scale` — **scale #2**, same `s`, `off_y: 0` | `top_scale` |
| 10 | `:923-973` | per plan: pick the scale, **fix the story viewport box**, optionally a third bottom-anchored `menu_scale` | `(scale, viewport, menu)` |
| 11 | `:982-1014` | `overlaid_status_strip` — a bar the game paints *on* the story window; **the viewport's top is pushed down a second time** | `viewport` (rebound) |
| 12 | `:1027` | `chrome_bands(area, viewport)` — up to four rects tiling pane − viewport | `bands` |
| 13 | `:1035-1042` | under the Menu plan, split the bottom band off as `menu_bands` | `ring_bands`, `menu_bands` |
| 14 | `:1059-1146` | the **Extend clip**: trim ring bands to the art's lowest opaque row, raised past text rows, floored at the viewport; a flank `v6_border::recognize` accepts is exempt | mutated `ring_bands` |
| 15 | `:1164` | `decompose_chrome_strips` — full-width bands split row-by-row into Art/Text; side bands stay one Art strip | `strips` |
| 16 | `:1246-1329` | the **SQ-0747 remainder walk**: full-width strips that are the story box's quantization remainder are removed and their rows given to the flank strip | mutated `strips` |
| 17 | `:1352` | `clear_text_rows` — carve every Text strip's native rows out of `canvas` | mutated `canvas` |
| 18 | `:1392-1415` | `flank_borders` — per flank strip, its inner and outer border columns, each `BorderInk::Band` or `::Glyph` | `flank_borders` |
| 19 | `:1436` | `clear_text_columns` — carve glyph border columns out of `canvas` | mutated `canvas` |
| 20 | `:1455-1478` | **narrow each flank strip** so its art stops where a glyph border begins | mutated `strips` |
| 21 | `:1494-1516` | `flank_panels` (Menu plan only) | `flank_panels` |
| 22 | `:1526-1539` | `tiled_flanks` — the extended flank source, per flank strip (never under Menu) | `tiled_flanks` |
| 23 | `:1550-1569` | `tile_cols` by backend; `art_tiles` splits a full-width strip into 8-column tiles | — |
| 24 | `:1606` | `retain_chrome_bands(live)` — cache eviction | — |
| 25 | `:1607-1664` | **the draw loop** — every Art strip through one of four arms, every Text strip as cells | pixels |
| 26 | `:1677-1701` | divider extensions: `Band` → stretched crop, `Glyph` → stamped character | pixels |
| 27 | `:1702-1711` | the bottom-anchored menu strips | pixels |
| 28 | `:1773` | `record_hybrid_click_map` | — |
| 29 | **`:1776`** | **`render_node(&story.node, …, viewport, …)` — the story text is rendered LAST, into the box fixed at step 10** | metrics |
| 30 | `:1915-1921` | erase fills and secondary prose windows over the transcript | pixels |
| 31 | `:1929-1965` | **the chrome-text overlay**: every chrome run whose native origin is inside the story box is stamped over the transcript, glyphs only, no background fill | pixels |

The draw loop's four arms for one Art strip (`:1639-1658`), in order, first match wins:

1. `flank_panels` hit → `fill_pane_page` + `draw_chrome_band_stretched` (Menu plan).
2. `tiled_flanks` hit → `draw_chrome_band_image` with the extended source.
3. `Frame` plan and narrow → `flank_crop` + `draw_chrome_band_stretched` (vertical **stretch**).
4. otherwise → `draw_chrome_band` per tile — a straight sub-rect of the one frame-shared
   scaled canvas.

**READ**, all of it.

### 1.4 Where raster diverges, and why

Raster is `:2034-2102`, canvas built by `build_v6_raster_canvas` (`:2880`). Three divergences
matter for this document:

**(a) Raster composes the whole frame at native size and scales once.** The band machinery has no
equivalent there, which is exactly why SQ-0886's defect was hybrid-only: a per-strip fill has
nowhere to live in raster. `extend_raster_flanks` (`:2968`, impl `:4411`) runs **before** the
single resize, so a raster flank cannot have a seam by construction — the comment at `:4381-4388`
says this outright and it is correct.

**(b) Raster derives the text region from what the chrome leaves. Hybrid does not.**
`story_clear_native` (`v6_layout.rs:1408`) insets the story window edge by edge until no edge
touches an opaque pixel of the art-only canvas, and raster calls it at `screen.rs:2981`. Hybrid
calls `story_viewport_box` (`v6_layout.rs:1492`) instead, whose own doc says it *"does NOT inset
around opaque chrome pixels — the raw window box is the viewport, and the chrome ring is drawn
around it"* (`v6_layout.rs:1489-1491`). This is the single most important structural fact in this
document; see §3. **READ.**

**(c) Raster keeps its pixel composite for menus on purpose** (`screen.rs:695-698`), because a
raster user asked for the pixel aesthetic.

### 1.5 The other two exits

- **Painted hint/menu takeover** (`:1968-2018`): reached when hybrid found no story window and
  there are painted runs but no painted ground. Draws the runs and discards every pixel.
- **Cell path** (`:2105-2427`): no image protocol, a modal overlay, frameless, or a menu takeover
  that is not `menu_over_art`. Lays out by *relation* to the story box — above → top band,
  below → bottom band, inside → over the transcript (`:2157-2231`). Draws no art at all.

---

## 2. Who decides what, and when

### The story window's box is fixed FIRST, and everything else is derived from it.

`story_viewport_box` at `screen.rs:925/934/940` maps the game's declared win0 rect through the
chosen scale and rounds **inward** to whole cells (`v6_layout.rs:1516-1519`). At that moment no
chrome has been classified, no strip exists, and no flank has been recognised. `chrome_bands`
(`:1027`) then computes the ring as *pane minus viewport* (`v6_layout.rs:1540-1560`). The chrome
is literally defined as "what the story does not occupy". **READ.**

That is the inverse of the ordering the user wants. It is also why the pipeline has an
irreducible seam at the viewport's edge: see §5.

### Chrome bands are computed once, at step 12; chrome *strips* are computed at step 15 and then mutated twice more.

- `strips` is produced at `:1164`.
- The SQ-0747 remainder walk **removes full-width strips and grows the flank strip into their
  rows** at `:1272-1284` and `:1316-1327`.
- The glyph-border pass **narrows the flank strip's columns** at `:1468-1475`.

Only after both mutations do `flank_panels` (`:1494`) and `tiled_flanks` (`:1526`) read the
strip's rect. **READ.**

### The flank is classified twice, from two different column ranges.

`v6_border::recognize` (`v6_border.rs:216`) is called from:

- `flank_border_art` (`screen.rs:4332`), called at `:1136` on the **un-mutated band rect**, to
  decide whether the Extend clip spares this flank;
- `flank_tiled_source` (`screen.rs:4353`) → `v6_border::flank_source` (`v6_border.rs:875`), called
  at `:1533` on the **twice-mutated strip rect**;
- `extend_raster_flanks` (`screen.rs:4428`) on a third range: the columns either side of the story
  window's declared native rect, with no reference to any band at all.

All three feed the same three-way classifier whose discriminators are `painted_widths` over
`[x0, x1)` (`v6_border.rs:110`, `:221`). A different column range is a different measurement, so
these three can in principle disagree. On the current corpus they do not, because the mutations
that would separate them (the glyph trim) only fire on flanks whose border is a *character*, and
those flanks return `Band`/`None` from the artwork path anyway (`screen.rs:1402`). **INFERRED**
from the code; not falsified by a capture.

### Text becomes cells or pixels at three different points.

1. **Chrome text in a full-width band** → cells, at classification time (`:5130-5136`), then
   drawn by `draw_chrome_text_strip` (`:1660`), which packs one game row per terminal row.
2. **Chrome text over art, or beside the story** → pixels, because it stays in `canvas` (step 3)
   and ships inside an Art band.
3. **Chrome text inside the story box** → cells, at the very end (`:1929-1965`), stamped over the
   transcript with no background fill, positioned by `round()` through the ring scale.
4. **Story prose** → cells, at step 29, through the ordinary transcript renderer.

Route 3 is the one SQ-0892 is about: it rounds **each run independently** (`:1944-1945`, and
`run_cell` at `:4945` does the same for classification), which is precisely the per-run rounding
SQ-0892's first note identified as the cause of `"SI(RT th e ga me"`. **READ.**

### Decisions made more than once, or from data a later stage changes

This is the list the squished-band defect is evidence for:

| decision | first made | changed later |
|---|---|---|
| the story viewport | `:925/934/940` | pushed down at `:1005-1009` by `overlay_strip` |
| the flank strip's rows | `:1164` (`decompose_chrome_strips`) | grown at `:1279-1281` and `:1323` |
| the flank strip's columns | `:1164` | narrowed at `:1475` |
| the flank's border classification | `:1136` (band rect) | recomputed at `:1533` (strip rect) |
| the scale | `:886` (`scale_center`) | `top_scale` at `:922`, `menu_scale` at `:939`, and three more effective factors in the draw arms (§5) |
| "is there art here?" | `strip_has_art` at `:1181` (device→native inverse, `as u32` truncation) | `region_has_opaque` at `:5088` (native rect, exact) — two different inverse mappings answering one question |

None of these is a bug on today's corpus. All of them are places where a change to one stage
silently changes another's input, which is the shape of failure the stopped lane hit.

---

## 3. Can the pipeline express the user's ordering?

The ordering, restated:

> (a) render the side panels with tiling, as one continuous column at one scale over the full pane
> height; (b) determine the valid text region from what the panels leave; (c) communicate that
> region to the story.

### (a) — one continuous column at one scale, full pane height

**Not expressible today, and the obstacle is structural rather than incidental.**

There is no object in this pipeline that represents "the flank". The left flank is whatever falls
inside `chrome_bands`' third rect, `Rect::new(pane.x, vy, vx - pane.x, vb - vy)`
(`v6_layout.rs:1554`) — a rect whose vertical extent is *the story viewport's*, by definition. The
columns above and below the viewport belong to the full-width top and bottom bands
(`v6_layout.rs:1550-1552`), which own the corners and are drawn by a completely different routine.

So a flank column is composed of up to three pieces before any of this code makes a choice about
it. §5 has the measurement.

To get "one continuous column, full pane height" the ring would have to be carved by *content*
rather than by *pane minus viewport*, and that is a change to `chrome_bands` and to everything
downstream that assumes `bands` tile the complement of the viewport exactly — the remainder walk
(`:1246-1329`), the clip (`:1059-1146`), the live-key set (`:1582-1605`), and the `strip_has_art`
skip (`:1609`).

The tiling half already exists and works: `flank_source` (`v6_border.rs:875`) builds the whole
extended column in native rows from row 0, applies the per-title recipe, and crops the caller's
window out of it. It is capable of answering for the full pane height; it is simply never asked
for more than the side band's rows.

### (b) — the text region from what the panels leave

**The mechanism exists, is correct, and hybrid does not use it.**

`story_clear_native` (`v6_layout.rs:1408-1442`) is exactly step (b): it shrinks the story window's
native box edge by edge, interleaved, until no edge touches an opaque pixel. Raster calls it
(`screen.rs:2981`). Hybrid calls `story_viewport_box` instead, which is the raw declared box.

There is even a cell-space wrapper — `story_viewport` (`v6_layout.rs:1448`) — that does exactly
what (b) asks: shrink-until-clear, then quantize to cells. **It has no production caller.**
`grep -rn "story_viewport(" crates/` returns only its own two unit tests
(`v6_layout.rs:2688`, `:2700`). **MEASURED** (the grep).

So the answer to (b) is: the pipeline can express it, the function is written and tested, and
switching hybrid to it is a one-line change *in principle* — with the large caveat that
`story_viewport` measures against `chrome_canvas`, i.e. the canvas that carries **rasterised
chrome text as opaque pixels**, which SQ-0728 already established is the wrong oracle (see
`screen.rs:2938-2950`: measured against the full canvas, Shogun's declared 548x64 box came back
548x16). Raster works around this by passing the art-only `obstruction` canvas
(`screen.rs:2951`, `:2981`). `story_viewport`'s own signature does not distinguish them.

### (c) — communicating the region to the story

**There is a real channel, we do not use it, and for v6 it is deliberately switched off.**

What the story can ask, and what we answer:

| query | answered at | with |
|---|---|---|
| `$20`/`$21` (rows/cols) | `crates/zvm/src/screen.rs:1933-1934` | the *fixed native* rows/cols |
| `$22` (screen width, units) | `crates/zvm/src/screen.rs:1939` | `cols * V6_FONT_WIDTH` = 640 |
| `$24` (screen height, units) | `crates/zvm/src/screen.rs:1940` | `rows * V6_FONT_HEIGHT` = 400 |
| `$26`/`$27` (font height/width — the V5↔V6 swap) | `crates/zvm/src/screen.rs:1946-1947` | 16 / 8 |
| `@get_wind_prop` 2/3 (`y_size`/`x_size`) | `crates/zvm/src/screen.rs:642-662`, seeded `crates/zvm/src/cpu/exec.rs:495-500` and `:781-785` | last stored, seeded from the same fixed dims |
| `@get_wind_prop` 13 (`font_size`) | seeded `crates/zvm/src/cpu/exec.rs:481-482` | `(16 << 8) | 8` |
| `$30` (string width, stream 3) | `crates/zvm/src/screen.rs:1374` | `bytes * 8`, or `wrap_stream3_text`'s line-width sum |

The fixed dims are computed once, pre-boot, from the **art**: `art_w * scale`, `art_h * scale`,
rounded to whole cells (`crates/app/src/session.rs:919-949`). 320x200 at ×2 → 640x400 → 80x25.

Every path that could feed the pane size in is explicitly v6-exempt:

- `Engine::set_screen_dims` returns early for v6 — `crates/app/src/session.rs:4351-4353`, with the
  rationale at `:4346-4350`: *"Feeding the pane's cell size in would resize the game's coordinate
  system underneath its own hardcoded art placement."*
- `poll_zvm_screen_dims` returns false for v6 — `crates/app/src/loop_tick.rs:140-142`.
- `reconcile_restored_screen_size` returns early for v6 — `crates/app/src/session.rs:3860-3862`.
- the user's `virtual_screen_rows/cols` pin is guarded `version() != 6` — `crates/app/src/startup.rs:697`.

And for v1–5/7/8 the app **does** do exactly what (c) asks: `story_screen_dims`
(`crates/app/src/render/screen.rs:440-461`) subtracts the upper-window border, twice the text
margin, and a scrollbar gutter from the pane before reporting it. So the *idea* of narrowing the
reported screen for chrome is already implemented in this codebase — for every version except the
one that needs it. **READ**, from the subagent trace, spot-verified at `session.rs:4339-4356`,
`loop_tick.rs:138-160` and `zvm/src/screen.rs:1925-1950`.

**So: we report the full native screen and nothing else.** Nothing anywhere reflects the region
the chrome leaves. The `$30` answer in particular is `chars × 8` with no reference to any window,
margin or panel, which is the assumption SQ-0892 already documented.

### The honest summary of question 3

- (a) is **precluded by the current structure**, because the ring is defined as the complement of
  the story box and the flank has no representation of its own.
- (b) is **available but unused**; the function exists, is dead, and would need its oracle canvas
  fixed.
- (c) is **available and deliberately disabled** for v6, with a stated reason that is about the
  *screen size*, not about the *text region*. Reporting a narrower window through
  `@get_wind_prop` `x_size`/`y_size`, or a different `$30`, is a different question from resizing
  the screen, and nothing in the code has considered it. Note the standing constraint from
  `crates/app/src/session.rs:2733-2742`: the app deliberately does not clip what `get_wind_prop`
  reports today.

---

## 4. The accumulated special cases

Every predicate and escape hatch on the hybrid path, what it was added for, and — **marked as my
judgement** — whether it is a structural distinction or a patch the right ordering would dissolve.

### Routing, before the fork

| predicate | line | added for | judgement |
|---|---|---|---|
| `frameless` | `:681` | SQ-0461, a mode the user has said may be removed | structural (it is a user choice), but its constituency is gone |
| `has_menu` | `:709-735` | SQ-0484, then narrowed twice: `STATUS_BAND_ROWS` for Arthur (SQ-0494), both-axes for Journey's `│` rules under the Amiga profile (SQ-0742) | **patch.** Two narrowings in two quests is the signature of a proxy. What it is really asking is "can the ring lay this screen out?", and it answers by pattern-matching a screen shape |
| `STATUS_BAND_ROWS = 4` | `:3510`, applied `:730` | keeping an ordinary status bar from reading as a menu | **patch**, and an undocumented magic number with no stated derivation |
| `menu_over_art` | `:757-762` | SQ-0886 — the cell path draws no art, so a menu screen the game framed with artwork lost its panels | **patch, and the most load-bearing one on this path.** It is a correction to `has_menu`'s over-reach, not a distinction of its own. If the ring could lay the screen out, neither predicate would exist |
| `any_modal_overlay_open` | `:772` | image placements composite above cells | structural |

### The ring/raster fork

`picture_takeover` (`:3942-3953`) is four hatches OR'd:

- `story_covers_screen && art_paints_anything` (SQ-0725) — `:3961`
- `... && art_fills_screen` — `:4055`
- `... && art_encloses_screen` — `:4091`
- `story_plate_escapes_story_window` (SQ-0739) — `:4001`

**Judgement:** all four are the same statement — *"there are pixels here that the ring has nowhere
to put"* — expressed four times because the ring's only container for pixels is
`pane − viewport`. SQ-0888's diagnostic note said this explicitly: a chrome window painting inside
the story box "is invisible to all three". A ring built from content rather than from the
viewport's complement would need one of these, not four.

### Layout

| mechanism | line | added for | judgement |
|---|---|---|---|
| `BottomPlan` × 4 | `:3864`, chosen `:4181` | SQ-0505/0511/0571/0830 | **structural.** Games really do differ in what is under the story window |
| `Menu` asked first | `:4188` | SQ-0830 | structural |
| the Extend clip | `:1059-1146` | SQ-0587 (Arthur's poles) | patch |
| clip raised past text rows | `:1088-1107` | SQ-0571 (the width-dependent corrupted location bar) | **patch**, and its comment documents a genuine ceil-vs-round collision |
| clip floored at the viewport | `:1115` | SQ-0582 (advent's overlaid bar) | patch |
| flank exempt from the clip | `:1135-1138` | SQ-0698 | patch on a patch |
| `overlay_strip` viewport push | `:982-1014` | SQ-0582 | **patch.** It exists because the viewport was fixed from the raw window box before anyone looked at what the game painted on it — i.e. it is step (b) done by hand, for one case |
| the SQ-0747 remainder walk | `:1246-1329` | the story box's two quantized edges | **patch, and the clearest evidence for §5.** Sixty lines and four measured pane sizes to give one row of a flank column back to the flank |
| glyph-border strip narrowing | `:1455-1478` | SQ-0779 | structural in spirit (SQ-0750's rule), patchy in placement — it mutates a rect three stages after it was computed |
| `tiled_flanks` excludes Menu | `:1526-1528` | SQ-0819 (Journey's picture column is not a border) | structural |
| `strip_has_art` skip | `:1181-1195`, `:1609` | SQ-0585/0750 | structural |
| `BAND_TILE_COLS = 8` | `:5238` | SQ-0818 | structural, and the only constant on this path with a full measured derivation in its doc |
| `SINGLE_PIECE_MIN_WIDTH = 24` | `v6_border.rs:77` | SQ-0881 (Arthur's Macintosh flank took Shogun's recipe) | structural. The measured gap is 12→44, so it is a threshold in name only |

### Transcript

| mechanism | line | added for | judgement |
|---|---|---|---|
| `frozen_head` suppression | `session.rs:1667-1668`, applied `:378` | SQ-0890 — the composite gave Shogun's credits screen a four-row prose box and the transcript replayed the credits into it, across the menu | **patch conditional on a routing decision.** It exists *because* `menu_over_art` sends the frame to the composite. SQ-0892's cleanup mandate already names it |
| `froze_whole` gate | `session.rs:1667` | a partial retirement has no single separating offset | structural |
| `ContentSplash` skip in `build_main_text` | `:3292` | SQ-0461 | structural — it is the picture-side statement of the same "already on screen" rule |
| retired prose republished as a chrome Grid | `session.rs:3042-3088` | SQ-0697 | **structural but under-appreciated.** This is the coupling that made SQ-0886 and SQ-0890 one screen: frozen prose is chrome to the renderer, so it feeds `has_menu`, `decompose_chrome_strips`, and the overlay pass |

---

## 5. Where the two-piece flank comes from

### It is not latent. It is the current, shipped composition, and I measured it.

**MEASURED**, `stories/zork0-r393-s890714.z6` at 100x40, kitty negotiated, `/dump-windows`:

```
v6 layout — current window 0, input window 0, scale 1.22, cell 8x18px, y-offset 0
  render path: hybrid-ring
  pane 98x37 at (1,1) · story viewport 70x31 at (15,7)
  win0  468x320 at (87,79)
  chrome ring strips:
    strip:art 98x6 at (1,1)
    strip:art 14x31 at (1,7)
    strip:art 14x31 at (85,7)
  ring: plan frame, ring not clipped
  band 8x6@(1,1): … native 52x88@(0,0) · resample 640x400->784x490 x:nearest y:nearest
  band 8x6@(9,1): … native 53x88@(52,0) · resample 640x400->784x490 …
  …thirteen tiles across…
  band 14x31@(1,7)  [Art, tiled]: … source 91x456 native px · resample 91x456->112x558 …
  band 14x31@(85,7) [Art, tiled]: … source 91x456 native px · resample 91x456->112x558 …
```

The oracle agrees — 13 top tiles at rows 1..6 plus two flank placements at rows 7..37
(`--out` report, "the placements a real terminal emulator would draw").

Read the flank column, cells 1..8, top to bottom:

- **Rows 1..6** are drawn by the first tile of the **full-width top band**, via
  `draw_chrome_band` (`graphics.rs:1357`), which crops the one frame-shared scaled canvas.
  Magnification: `784/640 = 490/400 = ` **1.2250 on both axes**. Source: the **raw** chrome
  canvas, native rows 0..88.
- **Rows 7..37** are drawn by the **left side band**, via `draw_chrome_band_image`
  (`graphics.rs:1696`), which `resize_directional`s a 91x456 source into 112x558.
  Magnification: **1.2308 horizontally, 1.2237 vertically**. Source: the **extended** flank,
  `v6_border::flank_source` (`v6_border.rs:875`), native rows 88..544 of a column that only has
  400 rows of real art.

So **one column, two images, three different magnifications, two different source canvases.** At
the seam (native column 91, the flank's right edge) the top band puts a pixel at device x 111.5
and the side band at 112.0 — half a pixel of horizontal shear, at the exact row where the two
meet.

### Why it does not show today, and what would make it show

It does not show because on every corpus frame the two pieces happen to be reading *the same
continuous artwork* across the seam, at magnifications that differ by 0.5%. The disagreement is
sub-pixel and the eye has nothing to lock onto.

Three things would break that, and all three are reachable:

1. **The extension recipe rewriting rows above `crop_top`.** `flank_source` builds the *whole*
   column from row 0, applies the per-title recipe, and only then crops
   (`v6_border.rs:898-917`). `shogun()` stamps a flipped whole copy at `border_height - 4`
   (`v6_border.rs:446-448`); `extend_pillars` calls `erase_below(total_height - foot_height)`
   (`v6_border.rs:349`). If a flank band's `crop_top` lands inside a rewritten region, the side
   band shows recipe output where the top band above it shows original art. The `shogun()` doc at
   `v6_border.rs:432-435` already records one occurrence of exactly this class of mismatch —
   a 64-native-row black band from reading `border_height` off one canvas and pixels off another
   — and says *"the flank crop starts below it in every pane we have measured"*, which is a
   statement about the corpus, not a guarantee. **READ + INFERRED.**

2. **A small, low story window.** The seam sits at the viewport's top edge. Shogun's credits
   screen parks window 0 in a 548x64 box near the native bottom (SQ-0890's note), so the top band
   becomes almost the entire flank and the side band a sliver — the two pieces at their most
   unequal, with the whole extension question hinging on a `crop_top` deep in the art. This is
   precisely the frame the stopped lane was working on. **INFERRED** from the geometry; I could
   not drive it, because that frame currently takes the composite (`menu_over_art`) and the
   ring never runs on it.

3. **The `Frame`-plan stretch fallback.** Arm 3 of the draw loop (`screen.rs:1646-1650`) is still
   live for an *unrecognised* flank: `flank_crop` + `draw_chrome_band_stretched`, which stretches
   a native crop into the band's device box with **no aspect constraint at all**. Above the
   viewport the same column is drawn at the uniform scale. A game whose flank
   `v6_border::recognize` does not know gets a hard vertical scale discontinuity at the viewport's
   top edge. `docs/features/v6-graphics.md:1142` says flanks are *"TILED down the flank, never
   stretched into it"*, which is false for this arm. **READ.**

### What cannot happen in the current code, stated plainly

**One column cannot receive two different *side-band* answers.** `chrome_bands` emits at most one
left and one right rect (`v6_layout.rs:1553-1556`), and `decompose_chrome_strips` returns a narrow
band as a single Art strip without splitting it (`screen.rs:5117-5121`). The bands tile
pane − viewport with no overlap, and the SQ-0747 walk preserves that (it removes a strip and grows
another into its rows). So there is no path by which two `flank_tiled_source` calls both draw the
same cells. **READ**, and worth saying because it rules out the most obvious hypothesis.

The seam is between the flank and the *full-width* bands that own the same columns above and below
it. That is the whole answer, and it is a consequence of §2: the ring is the complement of the
story box, so the flank's vertical extent is the story box's, and the story box was fixed before
anyone looked at the panels.

---

## 6. Findings log

Things I noticed while building the model that fit none of the five questions. **Not fixed. Not
chased.** Each is one line of why it matters.

### Dead or unreachable

1. **`story_viewport` has no production caller** — `crates/app/src/render/v6_layout.rs:1448`.
   Only its own two unit tests (`:2688`, `:2700`) call it. **MEASURED** (grep over `crates/`).
   *Matters because it is the shrink-until-clear cell-space viewport — literally step (b) of the
   ordering this document was commissioned to assess — sitting dead in the file hybrid reads
   from.* Untidiness, not a user-visible bug.

2. **`flank_native_bottom`'s `BottomPlan::Menu` arm is unreachable** —
   `crates/app/src/render/screen.rs:1152-1155`. Its only consumer is `flank_crop` at `:1647`,
   which is gated `matches!(plan, BottomPlan::Frame)`. **MEASURED** (grep: two hits, `:1152` and
   `:1647`). *Matters because the doc comment at `:1147-1151` describes the Menu case as live
   and explains at length what it is for — a reader will trust it.* Untidiness.

3. **`let _ = &classes[i];` is a no-op** — `crates/app/src/render/screen.rs:5154`, inside the
   SQ-0508 bridge loop. **READ.** *Looks like a borrow-checker workaround that outlived its
   cause.* Untidiness.

### Two places deciding the same thing by different rules

4. **"Is there art here?" has two implementations with different inverse mappings** —
   `strip_has_art` (`screen.rs:1181-1195`) inverts device→native with `as u32` **truncation** on
   both axes; `decompose_chrome_strips`'s `over_art` (`screen.rs:5084-5089`) works in exact native
   coordinates via `region_has_opaque`. **READ.** *The two are asked about overlapping regions on
   the same frame and can disagree by one native pixel at a boundary — which is the ceil-vs-round
   class of bug CLAUDE.md names as the usual cause of v6 geometry defects.* Unclear whether this
   matters; would need a boundary-crafted fixture to settle.

5. **`recognize` is called on three different column ranges for one flank** — `screen.rs:1136`
   (band rect), `:1533` (twice-mutated strip rect), `:4428` (story-box-derived, raster).
   **READ.** *A three-way classifier whose discriminators are width ratios, asked about three
   different widths. Today they agree; nothing enforces it.* See §2.

6. **The v6 exemption list is enumerated at five sites rather than expressed once** —
   `session.rs:4351`, `loop_tick.rs:140`, `session.rs:3860`, `startup.rs:697`, `session.rs:962`.
   **READ** (from the subagent trace, spot-verified at the first two). *A sixth path that forgets
   the check would resize a v6 game's coordinate system under its own art.* And in fact:

7. **`zvm-cli` does NOT exempt v6 from terminal-derived screen dims** —
   `crates/zvm-cli/src/main.rs:550` and `:1089` pass raw terminal rows/cols to
   `Machine::set_screen_dims` unconditionally. **READ** (subagent; not independently verified).
   *A v6 story under `zvm-cli` gets a different `$22`/`$24`/`$20`/`$21` from the same story under
   `lanthorn`, which makes the CLI a misleading oracle for exactly the v6 layout questions people
   reach for it to answer.* Looks like a live behavioural divergence, though `zvm-cli` is a
   debugging tool, so the blast radius is small.

### Constants and formulas

8. **`STATUS_BAND_ROWS = 4` has no stated derivation** — `screen.rs:3510`. **READ.** *It gates
   `has_menu`, which gates `menu_over_art`, which is the routing decision this whole screen turns
   on. Every other threshold on this path (`SINGLE_PIECE_MIN_WIDTH`, `BAND_TILE_COLS`, the 9/10
   width ratio) carries a measured table; this one carries nothing.*

9. **`pop_stream3`'s no-width measurement counts raw ZSCII bytes including embedded ZSCII 13** —
   `crates/zvm/src/screen.rs:1359`: `frame.buf.len() as u32 * V6_FONT_WIDTH`. **READ.**
   *A measured string containing a newline reports 8 units too wide per newline. The wrapping
   path (`:1404`) correctly excludes hard breaks from width; the unwrapped path does not.*
   Unclear whether any game measures a string containing ZSCII 13 — would need a corpus trace to
   settle.

10. **`wrap_stream3_text` returns the SUM of every line's width, not the maximum** —
    `crates/zvm/src/screen.rs:1398-1399` (documented) and `:1417`, `:1426`. **READ.**
    *ZMSD §7.1.2.1 says "the total width of printing", which the code reads as a sum. For a
    single-line measurement — the only case SQ-0892's probe observed — sum and max coincide, so
    nothing has exercised the difference. A game measuring a wrapped block would get a number
    several times too large.* Unresolved; would need a corpus trace of multi-line stream-3 use.

### Tests weaker than their names

11. **The Shogun render suites pin the HALFBLOCKS backend, not kitty** —
    `crates/app/tests/suites/v6_shogun_menu_ground.rs:118`, with the reason stated at `:116-117`
    ("which is what lets a case assert on the pane's own CELLS"). **READ.** *This is an honest,
    documented trade, but it means the flagship regression suite for this screen cannot see a
    kitty placement defect at all — and SQ-0890 recorded that this suite passed while the credit
    collision was on screen. Any future assertion about placements on these frames has to go
    through `pty_stream`, not this suite.* Worth knowing before writing SQ-0892's tests.

12. **`hybrid_draws_the_frame_raster_already_drew`** — `v6_shogun_menu_ground.rs:219`. **READ.**
    Asserts hybrid/raster parity. SQ-0890's note already recorded that this cannot catch a defect
    both modes share, and it didn't. *Named as a check on hybrid; is actually a check on
    agreement.* Recorded because the name will mislead the next reader.

### Docs that contradict the code

13. **`docs/features/v6-graphics.md:1142`: "TILED down the flank, never stretched into it."**
    The `Frame`-plan stretch fallback (`screen.rs:1646-1650`) and the Menu-plan panel stretch
    (`screen.rs:1641`) both survive. **READ.** *This is the one stale doc claim with a
    user-visible consequence — see §5.3.*

14. **`docs/features/v6-graphics.md:1230`: "Reaching the bottom now means reaching it to within
    one text row."** SQ-0881 replaced the bottom-only tolerance with the whole inset
    (`v6_border.rs:222-223`). **READ.** *The doc's sentence is now false as written; the code is
    right and the doc describes the bug SQ-0881 fixed.*

15. **`docs/features/v6-graphics.md:1018`: "Whether 'whole' means text or pixels is decided the
    same way as above."** It is not: `menu_over_art` asks for an opaque chrome *graphics window*
    (`screen.rs:760`); the no-story painted path asks for a *painted ground*
    (`screen.rs:1997`). **READ.** *An `erase_window` fill satisfies one and not the other, and
    that difference is exactly what keeps advent on the cell path.*

16. **`docs/architecture.md:135` names `render/graphics.rs::draw_v6_canvas`, which does not
    exist.** The raster composite is `build_v6_raster_canvas` (`screen.rs:2880`) +
    `spawn_v6_encode`/`redraw_v6`. Two stale comment references survive at
    `crates/app/src/render/graphics.rs:392` and `crates/app/src/render/screen.rs:8697`.
    **READ** (subagent). *A reader following the architecture doc into the code lands nowhere.*

17. **`docs/architecture.md:132` describes the `Layered` arm as "per-cell without an image
    protocol, or as one native-pixel-space canvas".** That is raster and the cell fallback; it
    omits **hybrid**, which is the default, and frameless, and the painted-menu page.
    `architecture.md` never mentions `v6_render` or names any of the four modes. **READ**
    (subagent). *The default render path for graphical v6 is absent from the architecture
    document.*

### Silent in the docs (not contradictions — omissions that cost a reader)

18. `BottomPlan`'s four arms as named entities (`screen.rs:3864`, `:4181`), including that `Menu`
    is asked first since SQ-0830 — and `/dump-windows` prints these exact strings
    (`screen.rs:1052`). **READ** (subagent).
19. The Extend-plan ring clip in full (`screen.rs:1059-1145`), including the flank exemption and
    the `v6_ring_clip` dump field. **READ** (subagent).
20. The chrome-text-over-transcript overlay pass (`screen.rs:1929-1965`) — the mechanism that
    puts Shogun's menu on the story strip, and the site SQ-0892's group quantization would have
    to change. Mentioned once, obliquely, as "the hybrid story-strip overlay" in a colour list
    (`v6-graphics.md:2060`). **READ** (subagent).
21. `STATUS_BAND_ROWS` (`screen.rs:3510`) — an undocumented threshold under a decision the docs
    do describe. **READ** (subagent).
22. Frozen prose re-entering the renderer as a chrome Grid (`session.rs:3042-3088`) — the
    coupling that made SQ-0886 and SQ-0890 the same screen. `v6-graphics.md:1752-1781` explains
    the freeze semantically and never says this. **READ** (subagent).
23. The two story-viewport functions and which mode uses which (`v6_layout.rs:1448` vs `:1492`);
    `v6-graphics.md:1584` presents shrink-until-clear as *the* algorithm. **READ** (subagent).
24. `classify_windows`'s first-match semantics (`v6_layout.rs:283`) — "the first primary Buffer"
    and "the first Graphics with `win == 0`". Order-sensitive classification, undocumented, and
    the seam fmvpoker crosses. **READ** (subagent).

### Documentation that is ACCURATE (recorded so the docs are not written off wholesale)

The subagent verified fourteen substantive claims as matching the code, including the four-band
ring, the Art/Text strip rule, the SQ-0508 bridge, all three `picture_takeover` arms, the
`menu_over_art` routing, the SQ-0890 freeze rule, the post-SQ-0888 Apple margin float (with
`story_text_box` correctly absent from both code and docs), `menu_band_rows`, the Menu-plan
tiling exclusion on both hybrid and raster, the backend-conditional band tiling, the frameless
layout-by-relation rule, and `native_extent`'s 640x400. `docs/features/v6-graphics.md` is
substantially trustworthy; `docs/architecture.md`'s v6 section is not.

---

## 7. One-paragraph summary

A v6 turn's output becomes a fresh `ScreenModel` on every render frame; `render_node`'s `Layered`
arm picks one of five destinations from four predicates; and in the default hybrid destination it
fixes the story window's cell box from the game's declared win0 rect **first**, defines the chrome
ring as the pane minus that box, carves the ring into strips, mutates those strips twice more,
composes each flank's source and scale from the twice-mutated rect, draws every band, and only
then renders the story text into the box it fixed at the start. The user's ordering — panels,
then the region they leave, then tell the story — is the exact reverse of that, and the reversal
is why the flank has no representation of its own, why four separate escape hatches exist for
"pixels the ring cannot hold", why a sixty-line walk is needed to give one row back to a flank,
and why the same column is drawn by two images at three magnifications with half a pixel of shear
between them.

---

## 8. Addendum, 2026-08-16 — what SQ-0896 and SQ-0897 changed

The body above is left exactly as written; this records where it has stopped being
true, so a reader is not sent to the old answer.

**§3(b) — "available but unused" is now "used".** Hybrid derives its story viewport
from `story_text_native` (`v6_layout.rs`), which is `story_clear_native` past the
frame art followed by `story_prose_box` for what window 0's own plate leaves. The
oracle question §3(b) flagged is settled the way raster settles it: the inset is
measured against the ART-ONLY chrome canvas, and the plate is measured separately by
the largest-free-rectangle sweep, never fed to the inset. That split is not
cosmetic — MEASURED, fmvpoker's hollow 640x400 table insets to `(320,54,0,322)`,
width zero, while the sweep finds the `(22,234,594,158)` hole the game prints in;
and Arthur's centred 584x392 plate touches no edge at all, so only the sweep sees it.

One deliberate limit, which §3(b) did not anticipate: the inset is **advisory**. An
inset leaving less than one 8x16 text cell is discarded and the window keeps its
declared box, because four edges converging on nothing is a BACKDROP swallowing the
window rather than a measurement of the text region. Raster's own floor, reused.

**§4's judgement on `picture_takeover` — one of four retired, three kept.** The
judgement that all four are one statement was right about the *statement* and wrong
about the *arms*. `story_plate_escapes_story_window` is gone (SQ-0746 removed its
premise, SQ-0896 its need). The other three were each disabled and driven:

- `art_paints_anything` — kept. Disabling it moves mysterious01 alone, and the ring
  reads its two 512x192 title cards as one 79x37 side FLANK and runs them through the
  border-extension recipe (`[Art, tiled] source 516x544 native px` for 384 rows of
  art). §5.3's warning about a picture column being treated as a border, one title over.
- `art_fills_screen` — kept. It is the only arm reading CHROME art on a frame the
  advisory-inset floor hands its declared box back to, i.e. a full-screen backdrop
  under a full-screen story window. Nothing else would draw it.
- `art_encloses_screen` — kept. `story_window_is_a_canvas` reuses it, and the canvas
  reading of window 0 lives on the raster path alone.

**§6, finding 1 is void twice over.** `story_viewport` was deleted by SQ-0894, and
the shrink-until-clear step it stood for is now live in hybrid through
`story_text_native`.

**New instrument.** `picture_takeover_reason` names the arm that fired, `ring_scout`
prints it per frame, and `picture_takeover_arms_across_the_corpus` pins the census.
Four OR'd predicates cannot be audited with a boolean: they are tried in order, first
match wins, and §4's own reading of them as interchangeable is what that hides.

**Unmeasured, and named so.** Zork Zero's `map` — §1's opening example and one of
SQ-0896's acceptance frames — could not be driven headlessly: `--turns 1 --cmd map`
leaves window 0 at its ordinary `(86,78,468,320)` on release 393, so whatever reaches
the full-screen map takeover is further into the game than a scout can walk. Every
claim above about that frame is inference from the code, not measurement.
