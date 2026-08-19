# Glk-as-a-crate + multi-format strategy (z6, TADS, legacy VMs)

**Status:** design decision record (discussed 2026-07-16). No code yet — this
captures the direction so the reasoning survives outside the conversation.
Two decisions are settled (leaning): **extract a shared `glk` crate** to host
future Glk-family VMs, and implement **v6 natively** rather than through Glk.
Sequencing is deliberately lazy — see *Timing*.

## Why this note exists

A chain of architecture questions came up while auditing Glulx/Glk completeness:
would today's `glulx + glk` structure support a different UI (browser)? A second
story format? z6? TADS? The answers converge on a small set of decisions about
where Glk lives and how new formats plug in. Recording them here.

## Current state (as built)

- **VMs are zero-dependency and terminal-free.** `zvm` (Z-machine) and `gvm`
  (Glulx), plus `mapper` and `blorb`, carry no `ratatui`/`crossterm`. All
  terminal deps live in `app` and the two CLIs. The VMs compile to WASM cleanly.
- **Two engine → presentation contracts exist today:**
  - Glulx drives Glk through the **`GlkBackend` trait** (`gvm/src/glk.rs`). The
    VM owns all Glk logic (window tree, streams, events, style hints); the trait
    is the display-side port, taking only neutral types (`WinType`, `GlkStyle`,
    `StyleColour`, `Rect`, `WinTree`, `GlkEvent`). `gvm` already ships two
    backends against it: the terminal `AppGlk` (`app/src/glk_backend.rs`) and a
    headless `TestBackend`.
  - The Z-machine has its **own** `ScreenState` + `Output` model in `zvm`,
    separate from Glk.
- **Both engines already converge on one neutral `ScreenModel`.** `app`'s
  `screen_model_from_machine()` (`session.rs:784`) mirrors the Z-machine's
  `ScreenState` into the same `ScreenModel` window tree that `AppGlk` projects
  Glk calls onto — "the one generic renderer draws both engines"
  (`glk_backend.rs` header). **So cross-engine render unification is already
  banked at the `ScreenModel` layer**, independent of Glk.
- **Input is a suspend/resume handshake** in both VMs: `step()` returns
  `NeedLine`/`NeedChar`/`NeedEvent`; the host resolves with `supply_line` /
  `supply_char` / `supply_filename`. Neutral values, no terminal types.
- **Glk API/Glulx completeness (2026-07-16 audit):** Glk 0.7.6 ~complete;
  Glulx complete for spec 3.1.2 (missing only 3.1.3's ExtUndo + double-precision
  section, both honestly gestalt-reported as unsupported). Gestalt answers are
  truthful.

## The design thesis: Glk is a VM-agnostic I/O library

Glk (Plotkin) is deliberately a two-sided, **N-VMs × M-displays** interface: any
IF VM drives it; any display library implements it; neither knows the other.
The same contract serves Z-machine, Glulx, TADS, Hugo, Alan, Level 9, Magnetic
Scrolls, Scott Adams. **Gargoyle** is the living proof: one Glk implementation
(`garglk`) hosts all of those as separate interpreters. lanthorn currently
*fuses* Glk into `gvm` — the one arrangement Glk was designed to prevent — so
extracting it is realigning with what Glk already is.

## Decision 1 — extract a shared `glk` crate

Create a VM-agnostic `glk` crate holding the `GlkBackend` trait, the value types,
the `Model` (windows/streams/filerefs/events/VFS/style hints), and a
**VM-agnostic dispatch**. Every Glk-family VM depends on it.

- **The real work is decoupling the dispatch, not moving files.** Today
  `glk_dispatch` is a method on the Glulx `Machine` (`gvm/exec.rs:2835`) that
  reads `self.mem` and the Glulx stack directly. Several selectors are
  inherently VM-touching (`glk_put_buffer` reads a memory range,
  `glk_stream_open_memory` references VM addresses, retained arrays get written
  back). The crate needs a memory/argument-marshalling abstraction the VM
  implements — this is exactly the reference **`gidispatch`** glue layer. The
  `Model` is already ~VM-neutral; the dispatch and arg decoding are the coupling.
- **Keep the crate UI-free and dependency-light** (it is shared by zero-dep VMs).
  `blorb` (resource access) is the only reasonable dependency, and even that can
  stay behind the backend trait.

### `glk_backend` stays one layer up — NOT in `gvm`, NOT bundled into `glk`

`AppGlk` is the Glk-calls → `ScreenModel` translator (~90% presentation-neutral;
its one coupling is a `ratatui::Color` → `u32` RGB conversion at
`glk_backend.rs:105`). It is a *consumer* of the trait producing one particular
scene model. Correct home is a shared, UI-free **`glk-scene`** layer that
`app` (and a future `browser`) depend on — not `gvm` (would break zero-dep and
invert the dependency arrow), and not the `glk` core (would force one scene model
on every VM and drag a scene builder into VMs that never use it). Layering:

```
blorb ─┐
       ├─ glk        (trait + types + Model + VM-agnostic dispatch)  ← every VM
gvm ───┤
zvm ───┘
           glk-scene (AppGlk → ScreenModel)                          ← every UI
app / browser  →  depend on glk-scene (+ glk + the VMs)
```

Rule: `glk` core stays UI-free and minimal; VMs depend on `glk` only, never on
`glk-scene`.

## Decision 2 — v6 (graphical Z-machine) is implemented **natively**, not via Glk

v6's defining feature — pixel-exact window layout — is the part Glk models
*least* cleanly (which is why v6 support is weak across Glk interpreters). Going
native lets us express v6's model directly, undistorted, up to the final render.

- **It integrates into `zvm` as an extension, not a rewrite.** The whole
  non-display core (execution, objects, dictionary, text, memory, most opcodes)
  is shared with v5 and already present. The v6-specific additions are additive
  and match existing patterns:
  - Admit v6 in `parse_header` (currently gated out — `unpack_routine`/
    `unpack_string` handle `3 | 4|5 | 7 | 8`, no `6`: `memory.rs:167`).
  - Add a `6 =>` packed-address arm — v6 uses the *same* `4·P + 8·offset`
    formula as v7, which already exists (`memory.rs:170`).
  - The ~18 v6 graphics/window opcodes are **EXT** opcodes; the EXT decode path
    (`0xBE`, v5+) and dispatch already exist with a warn-once fallthrough — each
    is an additive arm (same shape as the `input_stream` add, SQ-0187).
  - Version-6 behavior branches in a few existing display opcodes
    (`set_window`, `split_window`, `erase_window`, `set_cursor`, `set_colour`).
  - **The substantive new piece is the screen model:** v6 has up to 8
    pixel-positioned windows with margins/scroll regions. This *extends* the
    existing `ScreenState → ScreenModel` adapter; downstream rendering plumbing
    already exists (`blorb` parses `Pict`; `app` draws images via `graphics.rs`
    / ratatui-image).
- **Native v6 does not foreclose the multi-UI future:** the convergence point is
  `ScreenModel`, not Glk, so a native v6 emitting `ScreenModel`-with-graphics
  still renders in any future browser UI.
- **Accepted cost:** two graphics implementations in the tree (native v6 +
  gvm's Glk graphics), and no consolidation dividend. See the guardrail below.
- **Reality check:** terminal v6 fidelity is capped by pixel → character-grid
  rendering regardless of path, the screen-model work is the bulk either way,
  and the corpus is tiny (a handful of Infocom titles + a few modern). v6 is a
  deliberate stand-alone project, not a quick add.

## Format roadmap

Formats split by whether their I/O fits the Glk/parser model.

### Camp A — Glk-family parser VMs (the natural fits)
Reference Glk interpreters already exist, so in lanthorn terms the display side
is *free* (reuse `glk` + `ScreenModel` + renderer + **automapper**); the cost is
the bytecode VM itself.

| Format | VM effort | Notes |
|---|---|---|
| Scott Adams (ScottFree) | small | legacy, simple |
| Level 9 | small–moderate | legacy, documented |
| Magnetic Scrolls | moderate | legacy |
| Hugo | moderate | clean Glk port exists |
| Alan 2 / Alan 3 | moderate | ARun is Glk-based |
| AGT / Git | small / (Git = alt Glulx VM, not a new format) | |
| TADS 2 | moderate–large | own VM + object model |
| TADS 3 | **very large** | full modern OO VM + big runtime library |

**TADS wrinkle:** TADS 3 assumes **HTML-TADS** (a markup/DOM output model), not
Glk windows. A reduced TADS-over-Glk path exists (what Gargoyle ships), but
full-fidelity TADS wants an HTML renderer — pointing at a **browser UI** as
TADS's natural home more than a terminal. Double flag: large VM *and* poor Glk
display fit.

### Camp B — choice-based / web-native (out of scope for this architecture)
**Twine (Harlowe/SugarCube), Ink, Quest, ChoiceScript.** Input is
clicking choices, not parser commands; they are HTML/JS-native (browser by
design); and **they have no spatial room model, so the automapper — lanthorn's
differentiator — does not apply.** These would be a separate product direction,
naturally living in a web front-end, not a Rust VM behind a terminal.

### Prioritization lens
Weigh by **(VM effort) × (does the mapper add value) × (corpus)**:
1. **Highest fit:** legacy parser VMs (Scott Adams, Level 9, Magnetic) — cheap
   VMs, parser IF with rooms, big classic corpus once `glk` exists.
2. **Strong, more work:** Hugo, Alan.
3. **High value, high cost, partial display fit:** TADS — large VM, HTML display
   leans browser; a deliberate major effort.
4. **Out of the wheelhouse:** Twine/Ink/Quest/ChoiceScript.

## Timing / sequencing

- **Do not extract `glk` speculatively.** It earns nothing until a first
  consumer exists (the first legacy VM, or the browser UI). Deciding the
  direction now is right; doing the extraction should ride the first format that
  consumes it. (YAGNI.)
- **Native v6 and the `glk` extraction are decoupled** — v6 targets
  `ScreenModel` directly and needs nothing from the `glk` crate. Sequence them
  independently.
- **Cheap prep worth doing early:** sever the single `ratatui::Color` /
  `ColorScheme` coupling in `glk_backend.rs:105` (make it `u32`-typed), so the
  eventual `glk-scene` extraction is mechanical.

## Guardrails / open questions

- **Unify the graphics representation, not the graphics renderer.** When native
  v6 adds picture/graphics nodes to `ScreenModel`, reuse the *same* `ScreenModel`
  graphics representation the Glk path already produces (fills/draws/images).
  Two graphics *models* is the accepted cost; two divergent graphics *node
  shapes* is not — design v6's nodes to also cover Glk's image/fill-rect ops.
- **`gidispatch` boundary:** the memory/arg-marshalling trait for the `glk`
  crate is the main design artifact to get right at extraction time; look to the
  reference `gidispatch` layer for the shape.
- **z6-over-Glk is explicitly rejected** in favor of native, but the option
  remains if a future world extracts `glk` anyway and accepts Glk's v6 limits
  for a single shared engine-side model.

## Related tracking
- SQ-0186 — v6 (graphical Z-machine) + graphics via Blorb (native, per this note)
- `glk` core crate extraction — filed alongside this note
- Browser UI — future; first consumer that also justifies `glk-scene`
