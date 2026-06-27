# Glk I/O Layer (sub-project 3a) — Design

**Date:** 2026-06-27
**Status:** Approved (build 3a; 3b designed later)
**Crate:** `crates/gvm` (Glk model) + `crates/gvm-cli` (terminal backend)
**Roadmap:** Glulx sub-project 3. 3a = Glk core + interactive CLI (this spec).
3b = babelmap TUI integration (separate, designed with the user before touching
app code).

## Context

The Glulx VM (sub-projects 1–2) is complete and headless. Its `@glk` opcode
currently dispatches a few selectors that print to a single `Output` sink with
**no window model**. Real Glk programs (every Inform 7 Glulx game) open windows,
select an output stream, set styles, and call `glk_select` to wait for input.
3a replaces the `Output` placeholder with a proper Glk window/stream/event model
plus a pluggable **display backend**, and a terminal backend in `gvm-cli` so it
becomes an interactive Glulx player — finally enabling the **glulxercise**
compliance run. No app/TUI changes (that's 3b).

## Scope

The **IF subset** of Glk (parser games), not the whole API:
- Windows: **TextBuffer** (scrolling main), **TextGrid** (status/upper),
  **Pair** (layout tree). Graphics/Blank windows out.
- Streams: **window** streams and **memory** streams; file streams arrive with
  `@save`/`@restore` wiring (3a-2/save).
- Events: **line input**, **char input**, **arrange** (window resize). Timer/
  hyperlink/mouse out.
- Styles: the Glk style classes (Normal/Emphasized/Header/Subheader/Alert/…)
  mapped to text attributes; `glk_set_style`/`stylehint` (hints best-effort).
- Gestalt for Glk (report the supported capabilities).

Out of scope (3a): graphics, sound, hyperlinks, the full function set; the app
TUI backend (3b); cross-interpreter file save format (revisit the gvm save
snapshot then).

## Design

### `GlkBackend` trait (replaces the `Output` placeholder)

A display backend the VM drives for all output-side effects. The Glk **state**
(window tree, streams, current stream, current style) lives in `gvm`; the
backend renders it. Roughly:

```rust
pub trait GlkBackend {
    fn window_open(&mut self, id: u32, wintype: WinType, /*split info*/ …);
    fn window_close(&mut self, id: u32);
    fn window_layout(&mut self, /*the resolved window rects*/ …); // on arrange
    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str);    // text-buffer
    fn grid_put(&mut self, win: u32, x: u16, y: u16, style: GlkStyle, s: &str);
    fn grid_clear(&mut self, win: u32);
    fn window_clear(&mut self, win: u32);
    fn flush(&mut self);
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any; // test downcast
}
```

`BufferOutput` is replaced by a `TestBackend` (records per-window text/grid) so
the existing gvm tests assert on the right window's content. `Machine::with_glk(mem,
Box<dyn GlkBackend>)` supersedes `with_output`.

### Glk model in `gvm` (`glk.rs`)

Owns: the window tree (ids, types, rocks, the pair-split geometry, per-grid cell
buffer + cursor), streams (ids, type, the current output stream, memory-stream
buffers + read/write counts), the current style, and (3a-2) pending input
requests + filerefs. `@glk` selectors operate on this model and call the backend
for display. Window sizing: Glk asks the backend for the available size on
arrange; the model computes child rects from the pair splits.

### `@glk` selector subset (3a-1 output)

`glk_window_open`/`close`/`get_size`/`get_rock`/`get_type`/`get_root`/`get_parent`/
`get_sibling`/`iterate`, `glk_window_clear`, `glk_window_move_cursor` (grid),
`glk_set_window`, `glk_window_get_stream`, `glk_set_style`/`glk_stylehint_*`,
`glk_put_char`/`_string`/`_buffer` (+ `_uni`, + `_stream` variants),
`glk_stream_open_memory`(`_uni`)/`get_position`/`set_position`/`close`/`set_current`/
`get_current`, `glk_gestalt`(`_ext`), `glk_exit`. Route stream output: the
current stream → if a window stream, `put_text`/`grid_put` on that window via the
backend; memory stream → its buffer; null → discard. `streamchar`/`streamnum`/
`streamstr` (the Glulx opcodes) emit through the current Glk stream under iosys 2.

### Input + `glk_select` (3a-2)

`glk_request_line_event`/`request_char_event`(`_uni`) record a pending request on
a window. `glk_select(event_addr)` **suspends** the VM until an event arrives:
the run loop returns a new `StepResult` describing the pending request (kind +
window); the host supplies the event (the typed line written into the request
buffer, or the keycode) and **resumes**, and the VM fills the `event` struct and
continues. This mirrors `zvm`'s `NeedLine`/`NeedChar` suspend/resume, so the same
model serves both the CLI and the app (3b). `glk_cancel_line_event`,
`glk_request_timer_events` (no-op/diagnostic), `arrange` events on resize.

### `gvm-cli` terminal backend

Implement `GlkBackend` over a terminal, reusing the **zvm-cli screen-model**
patterns: the **TextGrid** (status) window pinned at the top via an ANSI
scroll-region + cursor addressing; the **TextBuffer** (main) window scrolling
below; Glk styles → SGR (emphasis/header → bold/reverse, etc.). Degrade to plain
streaming when not a TTY. Input (3a-2): handle the input `StepResult` by reading
a line / one raw key from stdin (reuse zvm-cli's cooked/raw input + restore),
supply the event, resume. Result: `gvm-cli <file.ulx|.gblorb>` plays a real
Glulx game.

### glulxercise compliance (3a-2 capstone)

Vendor the `glulxercise.ulx` fixture. A compliance smoke runs it under `gvm-cli`
with a scripted input sequence (selecting/auto-running the test groups) and
asserts it reports passing — the Glulx analogue of the Z-machine czech/praxix
smoke. This is the capstone the earlier phases deferred.

## Phasing (build order)

- **3a-1** (build now): the `GlkBackend` trait, the Glk window/stream/output
  model, the `@glk` output subset, the `gvm-cli` terminal **output** backend.
  Migrate the existing gvm tests off `Output`/`BufferOutput` to the model +
  `TestBackend`. Output-only programs render correctly (status grid + scrolling
  buffer).
- **3a-2** (next): input events + `glk_select` suspend/resume + the new input
  `StepResult` + interactive `gvm-cli` input + the **glulxercise** compliance run.

## Testing

- 3a-1: window open/close/tree + size computation from splits; `glk_set_window`
  routes `put_*` to the right window; text-buffer accumulates text with styles;
  text-grid honors `move_cursor`/`clear`; memory streams read/write/position;
  gestalt reports the supported caps; the existing VM tests pass against the new
  model (no behavior loss for plain output). A `gvm-cli` smoke renders a status
  grid + buffer output (assert the produced terminal bytes via the backend or a
  non-TTY plain mode).
- 3a-2: line/char request + `glk_select` suspend → host supplies event → resume
  fills the event struct; a scripted interactive session; the glulxercise run.

## Out of scope (3a, permanently or deferred)

- The babelmap **TUI** Glk backend + the `GameSession`-over-both-engines refactor
  → **3b** (designed with the user before touching app code).
- Graphics/sound/hyperlink windows and the non-IF Glk surface.
- Cross-interpreter Quetzal file compatibility for `@save`/`@restore` (revisit
  the gvm save snapshot when file streams land).

## Global constraints

- `gvm` stays zero-dependency (std only); `gvm-cli` depends only on `gvm`
  (+ `blorb`). No new crates.
- The Glk model + selectors transcribed from the **Glk specification** (Andrew
  Plotkin) — extend `crates/gvm/GLULX_NOTES.md` (or a `GLK_NOTES.md`).
- No panics on malformed Glk calls — diagnostic + continue/Quit.
- 0 warnings + full `cargo test --workspace` green per task.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
