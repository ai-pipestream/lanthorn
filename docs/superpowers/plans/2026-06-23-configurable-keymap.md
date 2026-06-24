# Configurable Keymap + Help Screen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the named map/global commands rebindable from `[keymap]` config, drive the bottom hint bar from the bindings, and add an F1 full-screen help overlay listing every binding — defaults reproduce today's key handling exactly.

**Architecture:** New app-side `crates/app/src/keymap.rs` owns `Command`, `KeySpec`, `KeyMap`. `Config` gains a `[keymap]` section; `KeyMap::resolve(&Config)` builds the set; `AppState` carries it; `key_to_action` does context-aware KeyMap lookups in place of the hardcoded match arms, preserving the existing dispatch ORDER. `mapper` untouched.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28 (`KeyCode`/`KeyModifiers`), serde + toml (already in `crates/app`).

## Global Constraints

- Defaults MUST reproduce today's key handling exactly. EVERY existing test in `crates/app/src/input.rs` MUST keep passing UNCHANGED at every task — they are the equivalence guard for the refactor.
- Hardwired, NOT rebindable: `Ctrl+Q`/`Ctrl+C` → `Quit` (always wins); the prompt sub-mode (`prompt_key_to_action`); Game-focus text entry (printable→`InputChar`, `Enter`→`SubmitCommand`, `Backspace`); and `Tab`'s stateful autocomplete-or-`ToggleFocus` special-case (see Task 4).
- `mapper` crate is NOT modified.
- Override that conflicts with an existing binding in the same context → reject the override, keep the default, surface a one-time status message.
- TDD, YAGNI, surgical. No backticks in commit bodies; end each with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

### Exact current bindings (the defaults — transcribe verbatim into `KeyMap::default()`)

Source of truth: `crates/app/src/input.rs` `key_to_action` (lines 129-319). Contexts: **Global** (reached in any focus when no prompt/anim), **Map** (Map focus), **Anim** (tidy-anim sub-mode). Game-focus nav keys (Shift+arrows/Home/PageUp/PageDown) are part of the hardwired Game text path, NOT keymap commands.

- **Global (ctrl block, input.rs:168-187):** `Ctrl+S`→SaveGame, `Ctrl+R`→RestoreGame, `Ctrl+E`→ExportSvg, `Ctrl+G`→ExportDot, `Ctrl+D`→ExportDump, `Ctrl+L`→CycleLayout, `Ctrl+T`→Retidy, `Ctrl+Y`→AnimateTidy, `Ctrl+A`→ToggleAlignment, `Ctrl+P`→TogglePortalLabels, `Ctrl+Left/Right/Up/Down`→Nudge{Left,Right,Up,Down}. Plus `Tab`→ToggleFocus (default key for the rebindable ToggleFocus; the Tab KEY itself stays hardwired, see Task 4).
- **Map (input.rs:266-318):** `Left/Right/Up/Down` (plain) and `Shift+Left/Right/Up/Down` and `h/l/k/j`→Pan{Left,Right,Up,Down}; `+`/`=`/`Shift++`→ZoomIn; `-`→ZoomOut; `c`→Recenter; `n`→SelectNext; `p`→SelectPrev; `Shift+N`→RenameLayer; `Shift+P`→PeelLayer; `Shift+M`→MergeLayer; `Shift+R`→Retidy; `]`→CycleLayerNext; `[`→CycleLayerPrev; `r`→RenameRoom; `o`→EditNotes; `d`→DeleteSelectedConnection; `e`→RelabelSelectedEdge; `i`→ToggleInspector; `Esc`→ToggleFocus.
- **Anim (input.rs:146-164):** `Shift+Left/Right/Up/Down` and `h/l/k/j`→Pan*; `+`/`=`→ZoomIn; `-`→ZoomOut; `Left`→AnimStepBack; `Right`→AnimStepFwd; `Space`→AnimTogglePlay; `Esc`/`Enter`→AnimExit.

Note the deliberate multi-binding cases the defaults MUST keep: Pan has plain-arrows + Shift-arrows + hjkl in Map; Retidy is both `Ctrl+T` (Global) and `Shift+R` (Map); ZoomIn is `+`/`=`/`Shift++`.

---

### Task 1: `keymap.rs` core types — `Command`, `KeySpec`, `Context`, `Command::to_action`

**Files:**
- Create: `crates/app/src/keymap.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod keymap;`)

**Interfaces:**
- Produces: `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Context { Global, Map, Anim }`; `pub enum Command { … all variants from the spec … }` (`#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]`); `pub struct KeySpec { pub code: crossterm::event::KeyCode, pub ctrl: bool, pub shift: bool, pub alt: bool }` (`Clone, Copy, PartialEq, Eq`); `impl Command { pub fn to_action(self) -> crate::input::Action; pub fn name(self) -> &'static str /* snake_case */; pub fn label(self) -> &'static str /* hint text */ }`.

- [ ] **Step 1: Write the failing test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;
    #[test]
    fn command_to_action_maps_directionals_and_names() {
        assert!(matches!(Command::PanLeft.to_action(), Action::Pan(-1, 0)));
        assert!(matches!(Command::NudgeUp.to_action(), Action::NudgeSelected(0, -1)));
        assert!(matches!(Command::CycleLayerNext.to_action(), Action::CycleLayer(1)));
        assert!(matches!(Command::AnimStepBack.to_action(), Action::AnimStep(-1)));
        assert!(matches!(Command::SaveGame.to_action(), Action::SaveGame));
        assert_eq!(Command::ToggleFocus.name(), "toggle_focus");
    }
}
```

- [ ] **Step 2: Run to verify it fails:** `cargo test -p app keymap::tests::command_to_action` — FAIL.
- [ ] **Step 3: Implement** the three enums and the structs. `Command` variants: `Quit, SaveGame, RestoreGame, ExportSvg, ExportDot, ExportDump, CycleLayout, Retidy, AnimateTidy, ToggleAlignment, TogglePortalLabels, ToggleFocus, ToggleHelp, ZoomIn, ZoomOut, Recenter, SelectNext, SelectPrev, RenameRoom, RenameLayer, EditNotes, DeleteSelectedConnection, RelabelSelectedEdge, ToggleInspector, PeelLayer, MergeLayer, PanLeft, PanRight, PanUp, PanDown, NudgeLeft, NudgeRight, NudgeUp, NudgeDown, CycleLayerNext, CycleLayerPrev, AnimStepFwd, AnimStepBack, AnimTogglePlay, AnimExit`. `to_action`: directionals → `Pan`/`NudgeSelected(±1)`/`CycleLayer(±1)`/`AnimStep(±1)`; `ToggleHelp` → a new `Action::ToggleHelp` (added in Task 5; for now if `Action::ToggleHelp` does not yet exist, add it as part of this task's `input.rs` — but DO NOT wire it elsewhere). All others map to the same-named `Action`. `name()` returns the snake_case string; `label()` returns short hint text (e.g. `ZoomIn` → "zoom in").
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app keymap::` — PASS; `cargo build -p app` clean.
- [ ] **Step 5: Commit:** "feat(keymap): Command/KeySpec/Context core types and to_action".

---

### Task 2: `KeySpec` parsing + labels

**Files:**
- Modify: `crates/app/src/keymap.rs`

**Interfaces:**
- Produces: `impl std::str::FromStr for KeySpec` (Err on unknown token); `impl KeySpec { pub fn label(&self) -> String }`; `pub fn from_key_event(k: crossterm::event::KeyEvent) -> KeySpec` (normalize a live event to a KeySpec for lookups).

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn keyspec_parse_and_label_roundtrip() {
    use crossterm::event::KeyCode;
    let s: KeySpec = "ctrl+s".parse().unwrap();
    assert_eq!((s.ctrl, s.code), (true, KeyCode::Char('s')));
    assert_eq!("shift+left".parse::<KeySpec>().unwrap().code, KeyCode::Left);
    assert_eq!("+".parse::<KeySpec>().unwrap().code, KeyCode::Char('+'));
    assert_eq!("f1".parse::<KeySpec>().unwrap().code, KeyCode::F(1));
    assert_eq!("space".parse::<KeySpec>().unwrap().code, KeyCode::Char(' '));
    assert!("nope".parse::<KeySpec>().is_err());
    assert_eq!("ctrl+s".parse::<KeySpec>().unwrap().label(), "Ctrl+S");
}
```

- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement** `FromStr`: split on `+`, treat `ctrl`/`shift`/`alt` tokens as modifier flags (case-insensitive), and the final token as the key: named keys `left/right/up/down/tab/space/esc/enter/home/pageup/pagedown/f1..f12` → the matching `KeyCode`; otherwise a single char → `KeyCode::Char(c)` (Err if not length 1). Special-case: a lone `+` token (since split on `+` yields empties) — parse it as `KeyCode::Char('+')`. `label()` renders modifiers (`Ctrl+`, `Shift+`, `Alt+`) then the key (`←/→/↑/↓` for arrows, uppercased letter, `Tab`, `Space`, etc.). `from_key_event` reads `KeyModifiers` into the bool flags and copies `code`.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app keymap::` — PASS.
- [ ] **Step 5: Commit:** "feat(keymap): KeySpec parsing and labels".

---

### Task 3: `KeyMap::default()` (full inventory) + lookup + `resolve` from config

**Files:**
- Modify: `crates/app/src/keymap.rs`
- Modify: `crates/app/src/config.rs` (add `KeymapConfig`, `Config.keymap`)

**Interfaces:**
- Produces: `pub struct KeyMap { bindings: Vec<(KeySpec, Command, Context)> }`; `impl KeyMap { pub fn default() -> KeyMap; pub fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<Command>; pub fn primary_key(&self, cmd: Command) -> Option<KeySpec>; pub fn for_context(&self, ctx: Context) -> impl Iterator<Item=(&KeySpec,&Command)>; pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String> /*warnings*/) }`. In `config.rs`: `#[derive(Debug, Default, Deserialize)] pub struct KeymapConfig { #[serde(default)] pub overrides: BTreeMap<String, String> }` and `#[serde(default)] pub keymap: KeymapConfig` on `Config`.
- `lookup(spec, Map)` falls through to `Global`; `lookup(spec, Global)` and `(spec, Anim)` do not fall through.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test]
fn default_keymap_matches_todays_bindings() {
    let km = KeyMap::default();
    let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
    use crossterm::event::KeyCode::*;
    assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Global), Some(Command::SaveGame));
    assert_eq!(km.lookup(&g(Char('n'), false, false), Context::Map), Some(Command::SelectNext));
    assert_eq!(km.lookup(&g(Char('h'), false, false), Context::Map), Some(Command::PanLeft));
    // Map falls through to Global:
    assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Map), Some(Command::SaveGame));
    // multi-binding default preserved:
    assert_eq!(km.lookup(&g(Left, false, true), Context::Map), Some(Command::PanLeft));
}
#[test]
fn resolve_applies_override_and_rejects_conflict() {
    let mut cfg = crate::config::KeymapConfig::default();
    cfg.overrides.insert("zoom_in".into(), "z".into());
    let (km, warns) = KeyMap::resolve(&cfg);
    use crossterm::event::KeyCode::*;
    assert_eq!(km.lookup(&KeySpec{code:Char('z'),ctrl:false,shift:false,alt:false}, Context::Map), Some(Command::ZoomIn));
    assert!(warns.is_empty());
    // binding to an already-used key in the same context is rejected:
    let mut cfg2 = crate::config::KeymapConfig::default();
    cfg2.overrides.insert("zoom_in".into(), "n".into()); // 'n' is SelectNext in Map
    let (km2, warns2) = KeyMap::resolve(&cfg2);
    assert_eq!(km2.lookup(&KeySpec{code:Char('n'),ctrl:false,shift:false,alt:false}, Context::Map), Some(Command::SelectNext));
    assert!(!warns2.is_empty());
}
```

- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement.** `default()` pushes EVERY binding from the "Exact current bindings" inventory above as `(KeySpec, Command, Context)` tuples (include all multi-bindings). `lookup` matches a `KeySpec` in the given context (Map also searches Global). `resolve`: clone `default()`; for each `(name, value)` override, resolve `name`→`Command` (snake_case match; unknown → warning, skip) and parse the comma-separated `value`→`Vec<KeySpec>` (parse error → warning, skip). Determine the command's context from a `Command::context()` helper (Global/Map/Anim by variant). REMOVE the command's existing default bindings in that context, then for each new KeySpec: if it is already bound to a DIFFERENT command in that context, push a warning and skip it; else add it. Collect warnings.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app keymap:: config::` — PASS.
- [ ] **Step 5: Commit:** "feat(keymap): default bindings, context lookup, resolve from config".

---

### Task 4: Refactor `key_to_action` to consult the `KeyMap` (the equivalence-critical task)

**Files:**
- Modify: `crates/app/src/input.rs` (`key_to_action`, `map_key_to_action`, the ctrl block, the anim block)
- Modify: `crates/app/src/state.rs` (add `pub keymap: crate::keymap::KeyMap` field, default `KeyMap::default()`)
- Modify: `crates/app/src/main.rs` (set `state.keymap = KeyMap::resolve(&cfg.keymap).0;` after `state` is created; surface `.1` warnings on the status line)

**Interfaces:**
- Consumes: `KeyMap::lookup`, `from_key_event`, `Command::to_action`, `AppState.keymap`.

PRESERVE THE EXACT DISPATCH ORDER AND SEMANTICS:
1. `Ctrl+Q`/`Ctrl+C` → `Quit` (unchanged, hardwired).
2. `state.prompt.is_some()` → `prompt_key_to_action` (unchanged).
3. `state.tidy_anim.is_some()` → `from_key_event(key)` then `state.keymap.lookup(spec, Context::Anim)`; if `Some(cmd)` return `cmd.to_action()`, else `Action::None`. (NO fallthrough to Global — matches today: anim mode returns early.)
4. `Tab` with no modifiers → KEEP the hardwired autocomplete-or-`ToggleFocus` special-case exactly as today (input.rs:188-199). (The `Tab` key stays special; `ToggleFocus` is still additionally rebindable to other keys via the keymap, reached in step 5/6.)
5. `ctrl` modifier present → `state.keymap.lookup(spec, Context::Global)`; `Some(cmd)`→`cmd.to_action()`, else `Action::None`. (Matches today's "any ctrl returns from the global block".)
6. Per focus: `Focus::Game` → `game_key_to_action` UNCHANGED (text + Shift-arrow/Home/PageX nav stays hardwired); if it returns `Action::None`, do a `Context::Global` keymap lookup and return that command's action if matched. RATIONALE: this is what makes `F1`→ToggleHelp reachable from Game focus — F1 is non-printable so `game_key_to_action` returns `None`, and ctrl-globals are already handled in step 5, so the only non-ctrl Global bindings this exposes are the new `F1`/help ones. Back-compat holds: printable keys are captured by `game_key_to_action` first, and no other non-ctrl Global binding exists by default. `Focus::Map` → `state.keymap.lookup(spec, Context::Map)` (which falls through to Global); `Some(cmd)`→action, else `Action::None`. Remove the giant `map_key_to_action` match (now data-driven) but KEEP its `Esc`→ToggleFocus default (it lives in the KeyMap as a Map binding).

- [ ] **Step 1: Write the failing equivalence test** in `input.rs` tests: a table of representative `(KeyEvent, expected Action)` covering every context (a global ctrl shortcut, a ctrl-arrow nudge, several map letters, a shift-map command, plain+shift+hjkl pan, anim step/play/exit, a map zoom, Esc→ToggleFocus) asserted through `key_to_action` with a default `AppState`. Many of these already exist as separate tests — ADD any missing context to one consolidated `key_to_action_equivalence_sample` test. (It passes today; it must still pass after the refactor — that is the guard.)
- [ ] **Step 2: Run to verify it passes BEFORE refactor** (it encodes current behavior): `cargo test -p app input::` — PASS. (This test is RED only if you mis-transcribe; it guards the refactor.)
- [ ] **Step 3: Implement the refactor** per the ordered semantics above. Add `state.keymap` and the `main.rs` wiring. Keep `game_key_to_action`, `prompt_key_to_action`, the Tab special-case, and the hardwired quit verbatim.
- [ ] **Step 4: Run the FULL input suite:** `cargo test -p app input::` AND `cargo test -p app` — ALL pre-existing input tests plus the equivalence test PASS. `cargo build -p app` clean.
- [ ] **Step 5: Commit:** "refactor(input): drive key_to_action from the configurable KeyMap".

---

### Task 5: Bottom hint bar generated from the KeyMap

**Files:**
- Modify: `crates/app/src/main.rs` (`draw_frame` `help_text`)

**Interfaces:**
- Consumes: `state.keymap.primary_key(cmd).label()`.

- [ ] **Step 1: Write the failing test:** extract the hint construction into a pure `fn hint_line(keymap: &KeyMap, ctx: Context) -> String` (in `main.rs` or a small helper module) and test: with a default keymap the Map hint contains "zoom" with key `+`; after `KeyMap::resolve` with `zoom_in = "z"`, the Map hint shows `z`.
- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement** `hint_line` with a fixed curated `&[(Command, &str)]` per context (Map: ToggleFocus, CycleLayout, ZoomIn, Recenter, SelectNext, Retidy, ToggleInspector, ToggleHelp; Game: ToggleFocus, SaveGame, RestoreGame, ToggleHelp; Anim: AnimStepFwd, AnimTogglePlay, AnimExit). Render each `"{key}: {label}"` joined by " | ", key via `primary_key(cmd).map(|k|k.label())`. Replace the hardcoded `help_text` strings in `draw_frame` (keeping the tidy-anim progress prefix). Always append `F1: help`.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app` — PASS.
- [ ] **Step 5: Commit:** "feat(keymap): generate the bottom hint bar from the keymap".

---

### Task 6: Full-screen help overlay (F1)

**Files:**
- Create: `crates/app/src/render/help.rs`
- Modify: `crates/app/src/render/mod.rs` (register), `crates/app/src/state.rs` (`pub show_help: bool`), `crates/app/src/input.rs` (`Action::ToggleHelp` already added in Task 1; ensure `apply_action` flips `state.show_help`; ensure `ToggleHelp` is bound — add `F1` (Global) and `?` (Map) defaults to `KeyMap::default()` in Task 3's inventory — verify they are present), `crates/app/src/main.rs` (render overlay in `draw_frame` when `show_help`)

**Interfaces:**
- Consumes: `state.keymap.for_context`, `KeySpec::label`, `Command::label`.

- [ ] **Step 1: Write the failing tests:** (a) render test (TestBackend): with `show_help=true`, `draw_help(&keymap, area, buf)` writes a known command label (e.g. "save game") and its key ("Ctrl+S") into the buffer, grouped under a "Global" heading; with `show_help=false` `draw_frame` draws nothing extra. (b) key test: `F1` (no modifiers) → `Action::ToggleHelp`; `apply_action(ToggleHelp)` flips `state.show_help`.
- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement.** Add `F1`→ToggleHelp (Global) and `?`→ToggleHelp (Map) to `KeyMap::default()` if not already added; `Action::ToggleHelp` handling in `apply_action`; `show_help` field; `draw_help` rendering a centered bordered overlay that iterates `Context::{Global,Map,Anim}`, lists each command's `label()` + all its keys' `label()`s; render it from `draw_frame` when `state.show_help` (Esc/F1 close via the existing toggle). Add `i: inspect`-style is already covered by the hint task; ensure `F1: help` hint shows.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app` AND `cargo build -p app` — PASS/clean.
- [ ] **Step 5: Commit:** "feat(keymap): F1 full-screen help overlay listing all bindings".

---

## Notes for the implementer

- Task 4 is the risk. Do NOT change `game_key_to_action`, `prompt_key_to_action`, the Tab special-case, or the hardwired quit. The whole pre-existing `input.rs` test suite must pass unchanged — if any fails, your KeyMap default or lookup semantics differ from today; fix the data, not the test.
- The anim sub-mode must NOT fall through to Global (today it returns early) — keep Anim lookups self-contained.
- `KeyMap::default()` is the single source of truth for back-compat; transcribe the inventory carefully and let `default_keymap_matches_todays_bindings` + the equivalence test catch errors.
- Snake_case command names: `SelectNext`→`select_next`, `CycleLayerNext`→`cycle_layer_next`, etc. Keep `Command::name()` and the config lookup in sync.
