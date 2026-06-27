# Glulx 3b-i: Engine Abstraction (pure refactor) — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/app`
**Roadmap:** Glulx sub-project 3b, phase i. 3b-ii adds `GlulxSession` + the
generic multi-window renderer + `.gblorb` routing (Glulx plays). This phase
introduces the abstraction with **zero behavior change** — only the Z-machine
runs through it, and its output/behavior must be identical to today.

## Goal

Refactor the app so it talks to the game through an engine-neutral `Engine`
trait instead of reaching into `session.machine` directly — so a second engine
(`gvm`, in 3b-ii) and future engines (TADS/Hugo) slot in. Introduce the neutral
types the trait needs: `KeyInput`, a window-tree `ScreenModel`, an `Introspect`
capability, a reserved `Debugger` capability, and an engine-tagged save. `zvm`'s
`GameSession` implements `Engine`. **No user-visible change.**

## Why pure refactor

The app today reaches into the concrete `zvm` session in many places:
`draw_upper_window(&session.machine, …)`, `key_to_zscii(KeyEvent)` →
`submit_char`, `session.machine.save_quetzal()`, `status_line()`, location
detection, the play-aids (autocomplete dictionary, verb/noun scope, inventory,
room inspector). Each becomes an `Engine`/capability call. Because only `zvm`
exists here, the full existing app suite must stay green and rendered output
must be byte-identical.

## Design

### The `Engine` trait (app)

```rust
pub trait Engine {
    // turn cycle
    fn submit(&mut self, command: &str) -> TurnResult;
    fn submit_key(&mut self, key: KeyInput) -> TurnResult;     // replaces submit_char
    fn take_transcript(&mut self) -> (String, Vec<(usize, u8)>); // text + style runs
    fn pending_input(&self) -> InputKind;
    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult;
    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult;

    // screen (engine-neutral window tree)
    fn screen(&self) -> &ScreenModel;

    // persistence (engine-tagged)
    fn save_state(&self) -> EngineSave;
    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError>;

    // mapping
    fn current_location(&self) -> Option<LocationInfo>;

    // capabilities / escape hatch
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn introspect(&self) -> Option<&dyn Introspect> { None }
    fn debugger(&mut self) -> Option<&mut dyn Debugger> { None } // reserved; None for now
}
```

The app holds `Box<dyn Engine>` where it holds `GameSession` today.

### Neutral input — `KeyInput`

```rust
pub enum KeyInput {
    Char(char),
    Enter, Backspace, Tab, Escape,
    Up, Down, Left, Right,
    Home, End, PageUp, PageDown, Delete, Insert,
    Func(u8),  // F1..F12
}
```

The app maps crossterm `KeyEvent` → `KeyInput` (a thin neutral mapping); the
**zvm** `Engine` impl converts `KeyInput` → ZSCII via the relocated
`key_to_zscii` logic. (3b-ii: the gvm impl converts `KeyInput` → Glk keycodes.)
`submit_char(u8)` at the app boundary is replaced by `submit_key(KeyInput)`.

### Neutral screen — `ScreenModel` (window tree)

```rust
pub struct ScreenModel { pub root: WinNode }
pub enum WinNode {
    Pair { vertical: bool, split: Split, first: Box<WinNode>, second: Box<WinNode> },
    Grid(GridWindow),     // text-grid: positioned style cells
    Buffer(BufferWindow), // text-buffer: scrolling, wrapped, styled
    Blank,
}
pub struct GridWindow {
    pub logical_cols: u16, pub logical_rows: u16, // virtual size
    pub cells: Vec<Cell>,                          // logical grid
    pub cursor: (u16, u16),
    // renderer applies a viewport over the logical grid (auto-follow cursor)
}
pub struct BufferWindow { /* text lines + per-line style runs + scroll, or a stream the app drains */ }
```

This is **app-owned and engine-neutral** — Glk is one adapter into it, not the
model itself. In 3b-i the tree is always the **degenerate 2-node** form (a Grid
+ a Buffer); the full multi-window tree is exercised in 3b-ii.

### `zvm` `GameSession` → `Engine` (the adapter)

- `submit`/`submit_key`/`pending_input`/`resume_*`/`take_transcript`: today's
  methods (with `submit_key` running the relocated `key_to_zscii`).
- `screen()` builds the neutral tree from the Z-machine screen, **branching on
  version**:
  - **v3** → a 1-row `Grid` synthesized from `compute_status_line()` (location
    left, score/moves|time right, reverse styled), refreshed each turn, over a
    `Buffer` (the lower window).
  - **v4+** → a `Grid` mirroring `screen.upper` with its **logical size +
    viewport** (the existing `virtual_screen_*` auto-follow), over a `Buffer`.
  - No split / no status → zero-row `Grid` (or `Blank`) + `Buffer`.
- `introspect()` returns a thin wrapper exposing today's play-aid data
  (dictionary vocabulary, in-scope nouns, inventory, room/exit info) — the exact
  current logic, just behind the capability.
- `save_state`/`restore_state` wrap Quetzal bytes in an `EngineSave`
  (`{ engine: "zmachine", format_version, bytes }`).
- `current_location` = today's logic.

### `Introspect` capability

```rust
pub trait Introspect {
    fn vocabulary(&self) -> &[String];          // autocomplete dictionary
    fn scope_nouns(&self) -> Vec<String>;       // verb/noun menu
    fn inventory(&self) -> Vec<String>;         // inventory strip
    fn room(&self) -> Option<RoomFacts>;        // inspector / mapper hints
}
```

zvm implements it now (reads object tree + dictionary); 3b-ii's gvm returns
`None` until Inform 7 introspection exists, so the play-aids **degrade
gracefully** (an aid with no data is simply unavailable).

### `Debugger` capability (reserved)

Declared (step/breakpoint/`decode_at`/memory+object dump) but `debugger()`
returns `None` everywhere in 3b-i. It exists so the trait is future-proof for the
Story Debug + Disassembly TODO; no implementation yet.

### App rerouting (the bulk of the work)

Replace every direct `session.machine.*` / `key_to_zscii` / `draw_upper_window(&machine)`
use with the trait:
- Rendering: feed the **2-node `ScreenModel`** to the screen render. For 3b-i,
  the renderer keeps using the existing `draw_upper_window`/`render_transcript`
  code paths, fed from the `Grid`/`Buffer` nodes — collapsing the separate
  v3-status-line and v4+-upper-window paths into one `Grid` render that
  **reproduces today's output exactly**. (The generic multi-window tree renderer
  is 3b-ii.)
- Input: `KeyEvent → KeyInput → engine.submit_key(...)`.
- Save/load: `engine.save_state()/restore_state()`; the `.babelmap` archive
  records the `EngineSave` tag and refuses a mismatched-engine restore.
- Play-aids: autocomplete / verb menu / inventory / room inspector read
  `engine.introspect()`; location via `engine.current_location()`.

`TurnResult` stays the common return type; its zvm-specific fields (`beep`,
`location_method`) remain `Option`/as-is (gvm fills `None` in 3b-ii).

## Invariant: NO behavior change

This phase ships **identical** behavior. Verification:
- Full `cargo test -p app` green (846+ tests), unchanged.
- Z-machine rendered output is byte-identical: the collapsed `Grid` render
  reproduces both the v3 status bar and the v4+ upper window exactly (assert via
  the existing render/buffer tests; add equivalence tests where a path is
  restructured).
- Autocomplete / verb menu / inventory / save-load / location detection behave
  exactly as before (their tests pass unchanged).

## Out of scope (3b-i)

- `GlulxSession`, the gvm adapter, `.gblorb` routing → 3b-ii.
- The **generic multi-window tree renderer** (arbitrary Pair trees, multiple
  Buffer windows) → 3b-ii (where real Glk games exercise it).
- Any `Debugger` implementation → the Story Debug TODO.
- Glk keycode mapping (no Glk engine here yet) → 3b-ii.

## Global constraints

- Zero behavior change; full `cargo test -p app` (and `--workspace`) green per task.
- 0 warnings (`cargo build`, `cargo doc -p app --no-deps`).
- The neutral types (`KeyInput`, `ScreenModel`, `Introspect`, `Debugger`,
  `EngineSave`, `Engine`) are app-owned and engine-agnostic — no `Glk`/`Glulx`/
  `Z-machine` specifics leak into the trait surface.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
