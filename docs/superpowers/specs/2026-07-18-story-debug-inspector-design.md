# Story Debug + Disassembly (SQ-0169) — Design

**Date:** 2026-07-18
**Status:** Approved, ready for planning
**Crates:** `crates/app` (debug panel + `Debugger` impl), `crates/zvm` (public disassembly formatter)
**Quest:** SQ-0169 — "Story debug + disassembly", a developer/curiosity feature riding the
SP3b `Engine` abstraction's reserved `Debugger` capability seam.

## Revision 2 (2026-07-18) — tiled pane, not a full-screen modal

The first build made the inspector a **full-screen modal** (component 3 below, "The debug
panel"). Superseded by user direction: the debugger must be a **tiled pane that lives in
the main layout alongside the story pane**, so the game stays visible and playable while
inspecting. This section governs where it disagrees with component 3.

- **Placement:** `/debug` **replaces the map pane's slot** with the debug region while
  active (map hidden until closed). Active layout is three columns:
  `Story │ Disasm (full height) │ Locals (top) / Stack (bottom)` — the debug region is the
  existing three-pane Execution view occupying the map's rect. The World-state view
  (`Globals │ Objects/Dictionary`, Memory in the cycle) is still reachable via the view
  toggle. The `DebugPanelState` view/pane model is unchanged.
- **Not a modal.** It does NOT swallow all keys and is NOT an `any_overlay_open` overlay.
  Reuse the app's **map focus model**: `Tab` focuses the debug region like it focuses the
  map today; when the debug region is focused, arrows/PgUp-PgDn scroll the focused sub-pane
  and `Tab`/`Shift-Tab` (or the view toggle) cycle panes/views, `g` jumps to PC. When the
  **story** is focused, keys type to the game as normal. `Esc` or `/debug` closes (map
  returns). The game keeps running beside the inspector — timers are **not** frozen.
- **Live refresh** each turn (unchanged from below).
- **Carries over unchanged:** the zvm disassembler (component 2), the `Debugger` seam +
  `GameSession` impl (components 1–2), the `DebugPanelState` nav logic, and the theme
  selectors. **Reworked:** the render integrates into the layout (draw into the map rect,
  not the overlay path), and the wiring becomes a layout mode + pane-focus key routing
  (the modal key-intercept and the `e6c3c066` `any_overlay_open` inclusion are removed).

Where component 3 ("The debug panel", "full-screen takeover") and the modal wiring in the
plan conflict with this section, **this section wins**.

## Revision 3 (2026-07-18) — three tabbed windows, wheel scroll, PC-follow, hints

After TTY use, the internal model of the debug region changes from a two-view toggle
(Execution / World-state) to **three independently-tabbed windows**. This supersedes the
`DebugPane`/`DebugView` model where it conflicts.

**Window model** — three windows in the same geometry as Revision 2 (left full height,
right split top/bottom); each window is a tabbed panel (default = first tab):
- **Left (full height):** tabs `Disassembly | Globals`
- **Right-top:** tabs `Locals | Objects | Dictionary`
- **Right-bottom:** tabs `Stack | Memory`

State per window: active tab index + content scroll offset. Plus the disasm address/history
and memory address (as today), and the current `pc` (for PC-follow + highlight).

**Interaction:**
- `Tab` / `Shift-Tab` → cycle **window focus** across the three windows (wrapping). (Changed
  from Revision 2, where Tab cycled the seven sections.)
- `←` / `→` → switch the **focused window's active tab** (e.g. Disassembly↔Globals).
- `↑`/`↓`, `PgUp`/`PgDn`, `Home`/`End` → scroll the focused window's content.
- **Mouse wheel** over any window → scroll *that* window (hit-tested by cursor position,
  focused or not).
- **Mouse click** on a tab label → activate that tab + focus its window; click in a window
  body → focus that window.
- `g` → jump the disassembly to the current PC.
- `Esc` → focus back to the story (panel stays open); `/debug` closes.

**Disassembly follows execution.** On every per-turn refresh, the disassembly re-anchors to
the live PC (set disasm address = `pc`, clear the scroll-back history) so the executing
instruction is shown at the top of the Disassembly tab and **highlighted** (new themeable
`debug_disasm_pc` selector). `g` re-anchors on demand after scrolling within a turn.

**Hint bar.** While the debug region is focused, the bottom hint bar shows debug keys
(`Tab: window  ←→: section  ↑↓: scroll  g: PC  Esc: back`) instead of the map hints.

**New themeable selectors:** `debug_disasm_pc` (PC line), `debug_tab` (inactive tab label),
`debug_tab:active` (active tab label) — registered through the full styling chain like the
existing `debug_pane*` selectors.

This section supersedes Revision 2's `DebugPane`/`DebugView` and the "Tab cycles panes"
navigation. The tiled placement, `Focus::Map` reuse, live-refresh, and non-modal nature from
Revision 2 all still hold.

## Revision 4 (2026-07-18) — execution-coverage marking in the disassembly

**Problem.** Inspect-only shows the VM parked at input between turns, and the parser's
`@read` is at a fixed address, so `dbg.pc()` is ~constant every turn — "PC-follow" anchors
the disassembly to the idle input loop, which never usefully changes. A stepper (to watch
mid-command execution) is still out of scope.

**Solution (works within inspect-only): mark the instructions that ran during the last
command.** Record the set of instruction start-PCs executed during each turn, and prefix
each disassembly line whose address is in that set with a `|` marker. This shows the actual
code path the last command took — the useful signal PC-follow couldn't give.

- **zvm (stays zero-dep; `std` only):** `Machine` gains `pub trace_exec: bool` and
  `pub exec_pcs: std::collections::HashSet<u32>`. In `step()`, when `trace_exec`, insert the
  instruction's start PC (`instr_start_pc`) into `exec_pcs`. Bounded by unique addresses hit
  (coverage), not instruction count. (Mirrors the existing `trace_screen`/`screen_trace`
  opt-in pattern.)
- **Session:** enable `trace_exec` while the debug pane is open (via a new `Engine` method
  the `/debug` toggle calls), disable + clear when it closes. Clear `exec_pcs` at the start
  of each turn (`submit`/`submit_key`) so it holds only the **last** command's coverage.
- **Debugger seam:** `fn executed_pcs(&self) -> std::collections::HashSet<u32>` (the app-side
  neutral view; returns a clone). Panel stores it in the snapshot on refresh.
- **Render:** for each Disassembly line, extract its leading `{:06x}` address and, if present
  in the executed set, draw a `|` marker in a leading column (new themeable `debug_exec_mark`
  selector); otherwise a blank column. Disassembly stays anchored at `pc` (the resume point,
  which IS executed, so marks flow from there through the command's linear path); `g` and
  scrolling unchanged.

New themeable selector: `debug_exec_mark`. "Since the last **step**" in the request means
since the last **command/turn** (no stepper exists).

### Backward disassembly scroll (scroll before the PC)

Z-machine instructions are variable-length and cannot be decoded backwards, so the current
disassembly can only scroll *up* through addresses previously scrolled *down* from (a history
stack), which is empty at the anchor — hence "can't scroll before the PC". Fix with the
standard linear-sweep technique:

- **zvm:** `pub fn prev_instr(mem, addr, version) -> u32` — scan forward from
  `addr.saturating_sub(WINDOW)` (WINDOW ≈ 16–24 bytes, > max instruction length), decoding
  instructions and returning the start of the one whose `next_pc == addr`. If none aligns
  exactly (a data region or the very start of memory), fall back to the nearest start `< addr`
  found in the sweep, or `addr.saturating_sub(1)`. Never returns `>= addr`.
- **debug_panel:** the disassembly scroll model becomes symmetric — `disasm_addr` is the top;
  scroll-down = `next_instr(disasm_addr)`, scroll-up = `prev_instr(disasm_addr)`. Drop the
  `disasm_history` stack. This lets the view scroll freely before the PC.

## Goal

Give a developer a read-only window into the running Z-machine: disassemble code at any
address, and inspect live VM state (call stack, locals, globals, object tree, dictionary,
memory) between turns. Surfaced as a full-screen `/debug` panel in the app. The VM step
loop is **not touched** — this is pure inspection.

## Scope & non-goals

**v1 is Z-machine only, inspect-only.**

- **In scope:** disassembly (navigable), call stack + locals, globals, object tree,
  dictionary, memory hex — all read-only, refreshed each turn.
- **Out of v1 (clean future increments on this foundation):**
  - **Stepper** — single-step, run-to-address, breakpoints on PC/opcode. Requires hooking
    the session drive loop to halt mid-turn; deliberately deferred.
  - **Glulx (gvm) support** — gvm has no public pure disassembler and its PC/stack/frame
    state is private (see "Why Z-machine first"). Its own follow-up sub-project.
  - Memory or state **editing**, watchpoints, conditional breakpoints.

### Why Z-machine first

The two VMs are asymmetric today. zvm is essentially debugger-ready; gvm is not:

| | **zvm** | **gvm** |
|---|---|---|
| Instruction decoder | public pure `decode()` (`decode.rs:358`) | only `pub(crate)` decoder that *mutates* PC |
| PC / frames / eval stack | `Machine.state` fully public (`exec.rs:120`) | `stack`/`sp`/`fp`/`locals` all private |
| Objects / dict / memory | all public | memory public; objects N/A for Glulx |
| Opcode → mnemonic | `opcode_name` exists but private (`exec.rs:23`) | ~7 opcodes only |

Building zvm first delivers a useful feature immediately; gvm's foundation work (public
accessors + a from-scratch disassembler) is decoupled into a later quest.

### Inspect-only, and what that means for the panes

Because we inspect *between turns*, the VM is always parked at the `@read` / `@read_char`
instruction awaiting input. So the **live** PC, locals, and eval stack reflect that parked
state, and the call stack is typically shallow (the game's main input loop). The
consequences, accepted by design:

- The **disassembly** pane is the workhorse: it is *navigable* (point it at any routine
  address), not locked to PC, so it is useful regardless of where execution is parked.
- **Globals** and the **object tree** are the most valuable live-state panes for IF
  debugging (they show world state: where things are, which flags are set).
- Locals / eval stack are a "parked snapshot" — thin in v1, and where a future stepper
  would make them shine.

## Architecture

```
┌──────────────────────── app crate ────────────────────────┐
│  /debug command (slash::COMMANDS)                          │
│         │ opens                                            │
│         ▼                                                  │
│  DebugPanel  ──renders──►  Vec<String> per pane            │
│         │ pulls text via                                   │
│         ▼                                                  │
│  Engine::debugger() -> Option<&mut dyn Debugger>           │
│         │                                                  │
│    ┌────┴─────────────┐                                    │
│    ▼                  ▼                                    │
│  GameSession(zvm)   GlulxSession(gvm)                      │
│  impl Debugger       debugger()=None (unchanged)           │
│    │ reads                                                 │
│    ▼                                                       │
│  zvm public API: decode()+format, Machine.state,           │
│  objects, dictionary, Memory   (+ new pub disasm helper)   │
└────────────────────────────────────────────────────────────┘
```

**Key principle:** the `Debugger` trait returns **pre-formatted `Vec<String>` per pane**,
mirroring the existing `Engine::window_dump() -> Vec<String>` idiom. Each VM formats its
own text; the panel just paints strings and owns scroll/cursor state. This keeps the trait
engine-neutral (Glulx slots in later without touching the app render code) and keeps
Z-machine-specific formatting (mnemonics, object short-names) inside zvm.

## Components

### 1. The `Debugger` trait (widened) — `crates/app/src/engine.rs`

The reserved stub today is:

```rust
pub trait Debugger {
    fn step(&mut self);                          // reserved for the future stepper
    fn decode_at(&self, addr: u32) -> String;
}
```

Replace with a read-only inspection surface (drop `step` from v1 — it belongs with the
stepper increment; re-add then). All methods return already-formatted lines:

```rust
pub trait Debugger {
    /// The instruction pointer the VM is parked at (for "jump to PC").
    fn pc(&self) -> u32;

    /// Disassemble `lines` instructions starting at `addr`. Each string is one
    /// formatted line: "4a2f  CALL_VS 5c10 -> sp".
    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String>;

    /// Address of the instruction that follows the one at `addr` (i.e. the decoded
    /// `Instr.next_pc`, clamped to memory). Lets the panel advance the disassembly
    /// view by whole instructions on scroll-down.
    fn next_instr(&self, addr: u32) -> u32;

    /// Call stack, outermost-last: one or more lines per frame
    /// ("#1 4b00 GET  ret=4a35  args=2").
    fn stack_lines(&self) -> Vec<String>;

    /// Locals of the innermost frame ("L0=005c  L1=0000").
    fn locals_lines(&self) -> Vec<String>;

    /// The 240 global variables, formatted ("g00=0012  g01=...").
    fn globals_lines(&self) -> Vec<String>;

    /// The object tree, indented ("[12] West of House { light, ... }").
    fn object_tree_lines(&self) -> Vec<String>;

    /// Dictionary words, one region per line or paginated.
    fn dictionary_lines(&self) -> Vec<String>;

    /// Hex+ASCII dump: `rows` rows of 16 bytes starting at `addr`.
    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String>;

    /// Total addressable memory length, so the panel can clamp scroll.
    fn memory_len(&self) -> u32;
}
```

`Engine::debugger()` keeps its signature; only `GameSession` overrides it to return
`Some(self)`. `GlulxSession` and the Scott session keep the `None` default — unchanged.

### 2. zvm public disassembly helper — `crates/zvm/src/...`

zvm already exposes `decode(&Memory, pc, version) -> Instr` (`decode.rs:358`) and the
`Instr` struct (opcode, operands, store, branch, text, `next_pc`). The only missing piece
is turning an `Instr` into a mnemonic line — `opcode_name(count, opcode) -> String` is
**private** in `exec.rs:23`.

Add a small public formatter that stays zero-dependency:

```rust
// e.g. crates/zvm/src/cpu/disasm.rs (new) or a pub fn beside decode
/// Format one decoded instruction as "MNEMONIC op, op -> store ?branch".
pub fn format_instr(instr: &Instr, version: u8) -> String;
```

Implementation reuses the existing (now `pub(crate)` or lifted) opcode-name table and the
public `Operand`/`Branch` types. The `GameSession` `Debugger::disassemble` impl loops
`decode` → `format_instr`, prepending each `pc` as hex. No new deps; `doctest = false`
crate config unchanged.

The other panes read already-public zvm API directly from `GameSession`'s `machine`:

- **stack / locals:** `machine.state.frames` (`Frame`: `return_pc`, `locals`, `store_var`,
  `arg_count`, `func_addr`) + `machine.state.eval_stack`.
- **globals:** `machine.global(n)` for n in 0..240 (`exec.rs:1913`).
- **objects:** `objects::object_tree_view` / `short_name` (already re-exported).
- **dictionary:** `Dictionary::load(mem).words(mem)`.
- **memory:** `machine.mem.raw_bytes()` / `read_byte`, header accessors for labels.

### 3. The debug panel (app UI) — `crates/app/src/render/debug_panel.rs` (new)

A full-screen panel following the `config_screen.rs` / `style_editor.rs` template
(full-screen takeover, not a floating modal). Two switchable three-pane views:

```
Execution view                         World-state view
┌───────────┬─────────────┐            ┌───────────┬─────────────┐
│           │   Locals    │            │           │   Objects   │
│ Disassem- ├─────────────┤            │  Globals  ├─────────────┤
│   bly     │   Stack     │            │           │ Dictionary  │
└───────────┴─────────────┘            └───────────┴─────────────┘
        (memory-hex reachable in the world-state view cycle)
```

**Panel state (`DebugPanel`):**
- `focus`: which pane is focused (an enum over the 7 panes).
- `disasm_addr`: current top-of-pane address for the disassembly pane (starts at `pc()`).
- `disasm_history`: a stack of previous top addresses. Because Z-machine instructions are
  variable-length and cannot be decoded backwards, scroll-down pushes the current
  `disasm_addr` and advances via `next_instr(disasm_addr)`; scroll-up pops the history
  (or, if empty, is a no-op). `g` clears the history and re-anchors to `pc()`.
- `mem_addr`: current top address for the memory-hex pane.
- per-pane scroll offsets for the list panes (globals, objects, dict, stack).

**Navigation:**
- `Tab` / `Shift-Tab`: cycle focus across all panes; cycling past the last pane of a view
  rolls over into the other view (one cycler, honoring the standing "Shift-Tab reverses any
  Tab-cycler" policy). The focused pane's view is the one displayed.
- `↑`/`↓`, `PgUp`/`PgDn`, `Home`/`End`: scroll the focused pane. For disassembly, `↓`
  advances `disasm_addr` by one instruction via `next_instr` (pushing history) and `↑` pops
  history; for memory it moves `mem_addr` by rows. List panes move their scroll offset.
- `g` (go-to-PC): reset the disassembly pane to `pc()`.
- `Esc`: close the panel.

**Refresh:** the panel holds no snapshot of its own — every draw re-pulls formatted lines
from `Engine::debugger()`, so it always reflects current VM state (which changes each turn
while the panel is closed; between turns it is static).

**Theming:** every pane, its border, and its title are styleable via new `style.toml`
selectors (`debug_pane`, `debug_pane:focused`, `debug_title`, `debug_disasm_pc`, …), per
the project's styleable-UI-elements policy. No hard-coded colors.

### 4. Command registration — `crates/app/src/slash.rs`

Add one `CommandSpec` to `slash::COMMANDS` (`slash.rs:158`), `Category::Help`, modeled on
`dump-windows` / `trace`:

```rust
CommandSpec {
    name: "debug",
    category: Category::Help,
    context: Context::InGame,
    usage: "/debug",
    description: "Open the Z-machine debug inspector (disassembly + live VM state).",
    dispatch: |_args| SlashOutcome::OpenDebug,
}
```

`SlashOutcome::OpenDebug` is handled in `main.rs::dispatch_slash_outcome`: if
`engine.debugger().is_some()`, open the panel; else print a transcript line
"debugger not available for this engine" (Glulx/Scott). Registering the `CommandSpec`
gives `/help` listing and Tab-autocomplete for free.

Key routing while the panel is open follows the existing full-panel pattern (the panel
intercepts keys, `Esc` closes) — see `config_screen` / `style_editor` key handling.

## Data flow

1. User types `/debug` → `SlashOutcome::OpenDebug` → `main` checks `engine.debugger()`.
   - `None` → transcript line, no panel.
   - `Some` → `state.debug_panel = Some(DebugPanel::new(dbg.pc()))`.
2. Each render frame, if the panel is open, `draw_debug_panel(state, engine, area, buf)`:
   for each visible pane, call the matching `Debugger` method with the panel's cursor/scroll
   args → `Vec<String>` → paint clipped to the pane rect.
3. Keys route to `DebugPanel::key(...)` → mutate focus / addresses / scroll offsets.
4. `Esc` → `state.debug_panel = None`.

The panel never mutates VM state; `Debugger` methods take `&self` except none need `&mut`
in v1 (the trait object is obtained via `&mut dyn` only to match the reserved signature —
the impl uses shared reads).

## Error handling

- **Wrong engine:** `debugger()` returns `None` for gvm/Scott → friendly transcript line,
  panel never opens. No panic path.
- **Out-of-range addresses:** `disassemble` / `memory_hex` clamp `addr` to
  `[0, memory_len())` and stop at the memory end (short `Vec` rather than reading past the
  buffer). `decode` on malformed bytes returns whatever `Instr` it can; the formatter emits
  a `"???"` mnemonic line rather than erroring — the debugger must never crash the app on a
  bad address the user scrolled to.
- **Empty/zero state:** at game start (before the first turn) the stack may be a single
  frame with no locals; panes render "(none)" rather than blank.

## Testing

zvm (zero-dep unit tests):
- `format_instr` produces expected mnemonic strings for a representative set of opcodes
  across operand forms (long/short/var), store, and branch — decode a known routine from a
  small test story and assert the lines.
- Disassembly clamps at memory end (no out-of-range read).

app (lib + bin tests):
- `GameSession` implements `Debugger`; `debugger()` is `Some` for zvm, `None` for a Glulx
  session (guard test), matching the wrong-engine path.
- `DebugPanel` navigation: `Tab`/`Shift-Tab` cycles focus with view rollover; scrolling the
  disassembly pane advances `disasm_addr` by whole instructions; `g` resets to `pc()`;
  memory scroll clamps at `memory_len()`.
- `SlashOutcome::OpenDebug` opens the panel on zvm and emits the not-available line on gvm.
- Render smoke: `draw_debug_panel` on a small story produces non-empty pane content for
  disassembly, stack, globals, objects, dictionary, and memory.

No headless-unfriendly paths — all logic is testable without a TTY; a final manual TTY
smoke confirms the panel *looks* right (theming, layout, scrolling feel).

## Global constraints

- **zvm stays zero-dependency.** The new formatter uses only in-crate types; no external
  crates, no `std` features beyond what zvm already uses.
- **Engine-neutral seam.** All Z-machine specifics stay behind the `Debugger` impl; the app
  render code sees only `Vec<String>`. Glulx support is a future impl of the same trait.
- **Cross-platform.** App-side only; no platform-specific code.
- **Styleable.** Every new UI element is themeable via `style.toml` selectors; no
  hard-coded styles.
- **No stepper / no VM-loop changes in v1.** The session drive loop
  (`run_until_input`, `session.rs:576`) is untouched.

## Future increments (not this spec)

1. **Z-machine stepper:** re-add `Debugger::step` + run-to-address + PC/opcode breakpoints;
   hook `run_until_input` to optionally halt and hand control to the panel. This is where
   locals/eval-stack become dynamic.
2. **Glulx debugger:** add public PC/stack/frame accessors and a real disassembler to gvm,
   then impl `Debugger` for `GlulxSession`.
