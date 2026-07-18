# Sectioned Debug Trace — Design

**Date:** 2026-07-18
**Status:** Approved (v1 trade-offs accepted)
**Supersedes:** the ad-hoc `--glk-trace` prototype (backed out; preserved at
`scratchpad/glk-trace-prototype.patch` for reusable pieces).

## Goal

A debug trace that shows exactly what the running story and the interpreter are
doing, split into independently toggleable **sections**, written to one tagged
log file. Replaces the single-purpose glk-only prototype with a design that
spans subsystems while keeping the zero-dependency VM/mapper crates clean.

## Global Constraints

- `zvm` and `gvm` VM crates stay **zero-dependency**; `mapper` keeps its
  existing `serde`/`serde_json` footprint and takes **no new deps**. The trace
  stays `std`-only in all three — no tracing/log framework.
- **Determinism** everywhere: no wall-clock timestamps, no `Math.random`-style
  ordering. Trace output for a given run + input must be reproducible.
- **Cross-platform** (Windows/Linux/macOS): file paths via `PathBuf`, no
  shell-specific assumptions.
- Real diagnostics/faults keep flowing to the transcript as `Warning` lines —
  the trace is a **separate** channel, never coupled to `diagnostics`.

## Sections

Three sections, each produced by the crate(s) that own the data:

| Tag | Covers | Origin |
|-----|--------|--------|
| `screen` | Engine-neutral **display instructions from the story**. A session runs exactly one engine, so exactly one mechanism populates it: **Glulx** — Glk/garglk structural calls (windows, styles, colours, streams, events); **Z-machine** — screen-control opcodes (`split_window`, `set_window`, `set_cursor`, `set_text_style`, `set_colour`/`set_true_colour`, `erase_window`/`erase_line`, `buffer_mode`, `set_font`). Both skip the text-print stream. | `gvm` + `zvm` |
| `map` | Automapper pipeline stages as the render worker already labels them — `detect chains → place rooms → route edges → route lanes`, plus room/edge capture and z-level peel / tidy compaction. One line per stage per rebuild. | `mapper` (via existing `render_traced`) |
| `hostio` | Save/restore snapshots, VFS file reads/writes/deletes, and input/events (line/char input delivered, Glk event dispatch). | `app` |

The `screen` line content self-identifies the mechanism — `glk_window_open(...)`
for Glulx, `@set_colour(...)` for Z-machine — so one section tag serves both
without the user needing to know which engine their story uses.

**Out of section scope for v1** (see Out of Scope): render/layout (served by
`/dump-windows`), VM opcode execution beyond the screen-control subset, the
text-print stream (Glk `put_*` / Z-machine `print*`), per-section verbosity.

## Control Surface

Same comma-separated grammar at launch and at runtime.

### Launch
```
--trace <list>      e.g. --trace screen,map   |   --trace all   |   (omitted → none)
```
Sets the active section set before the boot drive, so boot-time Glk/window/
style/colour calls are captured (they happen before the UI exists).

### Runtime
```
/trace              show every section and its current on/off state
/trace <list>       SET the active set to exactly <list> (replace semantics)
/trace all          enable every section
/trace none         disable every section
```
`<list>` is comma-separated section names. Replace semantics mirror the CLI:
`/trace screen,map` makes the active set exactly `{screen, map}`. (Trade-off accepted:
dropping one section means retyping the rest. Chosen for CLI/runtime symmetry.)

Unknown section names are reported (status line) and ignored; the valid set is
unchanged for that call.

Command registry entry: `trace` (Help/diagnostic category, global context),
usage `trace [sections|all|none]`.

## Output

- **File:** `<user_dir>/trace.log` (`~/.babelmap/trace.log` by default).
- **Lifecycle:** truncated fresh at boot so each run's trace stands alone;
  appended per turn thereafter.
- **Line format:** `[<section>] <message>`, section tag left-padded to a common
  width so columns align, e.g.:
  ```
  [screen] @split_window(1)
  [screen] @set_colour(fg=2, bg=9, win=upper)
  [hostio] vfs_read(cm.glkdata, 4096 bytes)
  [map]    room_captured("West of House", exits=N,E,S,W)
  [map]    route(a3->a7, len=4, meander)
  ```
  (A Glulx story's `screen` lines instead read `glk_window_open(...)`,
  `glk_stylehint_set(TextGrid, Normal, ReverseColor, 1)`, etc.)
- **Transcript pointer:** when any section is active at boot, one `Meta` line
  `[trace → <path>: screen,map]`. On a runtime `/trace` change, a status line
  `[trace: screen,map]` (or `[trace: off]`).

## Architecture — buffer-drain

Chosen over a live shared sink because it keeps the VM/mapper crates zero-dep,
stays deterministic (no shared clock), and rides the existing `diagnostics`
drain pattern and the "map work → worker thread" direction (a worker returns
its trace buffer alongside its result).

### Zero-dep crate side (`zvm`, `gvm`, `mapper`)
Each emitter owns:
- a per-section **enable bool** (set by the app), and
- a dedicated **`Vec<String>` trace buffer**, drained by the app.

An emitter formats and pushes a line **only when its enable bool is true** — no
formatting cost and no allocation when the section is off. The buffer is
**separate from `diagnostics`** (which continues to carry real notices to the
transcript).

- `gvm::Machine`: `trace_screen: bool` + `screen_trace: Vec<String>`. The hook
  sits at the top of `glk_dispatch`: for every selector where
  `!is_glk_text_io(selector)`, push `"<name>(<decoded args>)"`. Drained at boot
  and after each turn.
- `zvm` (machine): `trace_screen: bool` + `screen_trace: Vec<String>`. The hook
  sits at the screen-control opcode dispatch sites (`split_window`,
  `set_window`, `set_cursor`, `set_text_style`, `set_colour`, `set_true_colour`,
  `erase_window`, `erase_line`, `buffer_mode`, `set_font`), pushing
  `"@<op>(<decoded args>)"` — window numbers as `upper`/`lower`, colour numbers
  and style bits by name. The text-print opcodes are **not** hooked.
- `mapper`: **no new field.** The pipeline already exposes stage labels via
  `render_traced(graph, on_step)` / `render_layer_traced`, and the app's
  background render worker already pushes them into a shared
  `render_steps: Arc<Mutex<Vec<String>>>` that survives the worker→app boundary.
  The `trace_map` flag lives on the app `State` beside `render_steps`; the app
  routes that buffer into the log when `map` is active (enriching a few labels
  with keys — room count, bounds, route endpoints). Only label strings may be
  added mapper-side; `std`-only, no new deps.

Crates emit **untagged** content; the app prefixes the `[section]` tag on drain.
Because a session runs one engine, only that engine's `screen_trace` buffer is
ever non-empty.

### App side (`app`)
- `TraceSections { screen: bool, map: bool, hostio: bool }` — `Copy`, dep-free.
  Parsed from `--trace` (and `/trace`) via a `parse` that handles names,
  `all`, `none`, and reports unknowns. Held on `Config` (`#[serde(skip)]`,
  runtime-only) and mutable at runtime.
- **`Engine` trait extension** — so the app drains `screen` without knowing the
  engine: `fn set_trace_screen(&mut self, on: bool)` and
  `fn take_screen_trace(&mut self) -> Vec<String>` (default no-op / empty;
  `zvm` and `gvm` sessions override to hit their machine's bool + buffer). A
  boot-drain variant covers lines emitted before the first turn.
- `trace` module — free functions (no handle to thread), mirroring the existing
  best-effort `log_gvm_fault`: `truncate(user_dir)` starts a fresh log at boot;
  `write(user_dir, section, lines)` tags + appends. A failed open is silently
  skipped.
- **Wiring:** on session create and on every `/trace` change, push the section
  bools down — `engine.set_trace_screen(sections.screen)`; set the `trace_map`
  flag on `State`. The app emits its own `hostio` lines directly through
  `trace::write` at the save/VFS/input call sites.
- **Drain points:** at boot (`screen` via `take_screen_trace` after the seed
  turn) and after each turn (`screen` via `take_screen_trace`, then the render
  worker's `render_steps` when `map` is on). The app writes them tagged, in that
  fixed subsystem order.

### Ordering (the one honest limitation)
Interleaving is **turn-granular**, subsystem-ordered within a turn
(`screen` from the VM step → `hostio` → `map` from the pipeline), which
approximates real execution order. No global wall-clock ordering — that would
need a shared cross-thread sink and would fight determinism.

## Reuse from the prototype

Pull these from `scratchpad/glk-trace-prototype.patch` into `gvm` (they decode
gvm's own selector numbers, so they belong there):
- `glk_selector_name` — the authoritative selector→name table (taken from
  `glk_dispatch`, not the Glk spec constants).
- `glk_wintype_name`, `glk_style_name`, `glk_hint_name`, `glk_color_hex`,
  `glk_trace_args` — the colour/style/hint arg decoders.
- `is_glk_text_io` — the per-char text-I/O skip predicate.

The `zvm` screen-opcode decoders (window/colour/style-bit names) are **new**,
written on the `zvm` side against its own opcode operands.

The prototype's coupling to `diagnostics` and the transcript-vs-file routing
(`startup.rs`/`turn.rs`) is **not** reused; the dedicated buffer + `TraceLog`
replace it.

## Testing

- **gvm:** enabling `trace_screen` populates `screen_trace` with decoded lines;
  disabled → empty; `is_glk_text_io` selectors are skipped; the decoder tests
  from the prototype (`glk_selector_name`, `glk_trace_args`).
- **zvm:** the screen-control opcodes (`set_colour`, `set_text_style`,
  `split_window`, …) populate `screen_trace` with decoded lines when
  `trace_screen` is on; off → empty; text-print opcodes emit nothing.
- **mapper/app:** `render_traced` emits the expected ordered stage labels; the
  app routes `render_steps` into the log tagged `[map]` only when `trace_map` is
  on.
- **app:** `TraceSections::parse` for `screen,map` / `all` / `none` / unknowns;
  `/trace` command set-replace + bare-show; `Engine::take_screen_trace` drains
  the active engine (test both `zvm` and `gvm` sessions); `TraceLog` tagging,
  alignment, and truncate-at-boot vs append; the `hostio` emit points fire on
  save/VFS/input.

## Out of Scope (v1)

- Render/layout tracing — `/dump-windows` already dumps the live window tree.
- General VM opcode / instruction-level tracing (excluded — volume). Only the
  Z-machine **screen-control** opcode subset is hooked, as the `screen` section.
- The text-print stream — Glk `put_*` and Z-machine `print*` (skipped for
  readability; the `screen` section is display *control*, not printed text).
- Per-section verbosity levels / sub-sections.
- Wall-clock timestamps and true global cross-thread ordering.
- Rotating/size-capped logs (single truncate-at-boot file is enough for v1).
