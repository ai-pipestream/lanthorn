# Scott Adams (ScottFree) VM — design spec

**Status:** design decision record (approved 2026-07-16). Quest: SQ-0388.
Captures the agreed shape before an implementation plan is written.

## Goal

Add a native interpreter for classic **Scott Adams** adventure games (the
ScottFree `.dat` text database format) as babelmap's first additional story
format beyond Z-machine and Glulx — playable in the TUI with the live
automapper, and reachable through the existing story picker.

## Why this format, and why native

Scott Adams was chosen as the highest bang-for-buck additional VM: the
interpreter is tiny and well-understood, the games are pure parser-with-rooms
IF (so the automapper — babelmap's differentiator — applies directly), and it
carries a large classic corpus (Scott Adams' originals plus hundreds of
fan/modern games authored via ScottKit, all distributed in the `.dat` format).

**It is implemented natively, talking straight to `ScreenModel`, with no Glk in
its path.** A Scott Adams game's entire display is one scrolling text area plus
a room/status header — simpler than the Z-machine, which already emits
`ScreenModel` directly without touching Glk. Routing this VM through Glk would
mean extracting a shared `glk` crate and pushing a one-window game through the
full windowing/streams machinery purely to prove the layering, for a VM that
gains no feature from it and is too simple to meaningfully exercise Glk. That
contradicts the standing guidance in
`docs/design/2026-07-16-glk-crate-and-format-strategy.md`: *"Do not extract
`glk` speculatively. It earns nothing until a first consumer exists."* A Scott
Adams VM that bypasses Glk is not a consumer of Glk, so it does not justify the
extraction. The real first consumer of an extracted `glk` is a VM that needs
Glk windowing — Hugo, Alan, or TADS-over-Glk — and that extraction remains a
separate future effort.

## Non-goals (v1 boundary)

- **No graphics.** The illustrated "SAGA" / Questprobe variants are out of scope.
- **No original per-platform binary databases** (TRS-80/Apple/C64/TI-99 native
  dumps). Target is the ScottFree `.dat` text format only; most classic games
  are already available converted to it.
- **No Blorb-wrapped Scott games.** Could be added later via a new `ExecKind`;
  not needed for v1.
- **No `glk` crate extraction.** Explicitly deferred to a future Glk-family VM.

## Format: ScottFree `.dat`

A plain-text file of whitespace-separated integers and double-quoted strings,
in a fixed order:

1. **Header** — a fixed list of counts and parameters: number of items,
   actions, words (verb/noun vocabulary pairs), rooms, max carried items,
   starting room, number of treasures, significant word length, lamp/light
   duration, number of messages, and treasure-deposit room.
2. **Action table** — one entry per action: a packed verb/noun key, five
   packed condition slots (each a condition code plus a value operand), and
   packed command slots.
3. **Vocabulary** — verb/noun word pairs (a leading character marks synonyms).
4. **Rooms** — per room: six exit room-numbers (N, S, E, W, Up, Down) and a
   description string (a leading marker distinguishes a literal description
   from a "look" auto-prefixed one).
5. **Messages** — indexed message strings.
6. **Items** — per item: description string (a leading `*` marks a treasure; a
   trailing `/NOUN/` marks an auto-get/drop noun) and a starting room number.
7. **Action comment strings**, then a trailer (version, adventure number,
   checksum).

**The exact condition-code and command-opcode tables are deliberately NOT fixed
in this spec.** They are enumerated and verified against an authoritative
reference at plan-writing time, per the project's verify-don't-recall rule — not
reproduced from memory here.

**Licensing:** the interpreter is implemented clean-room from the documented
format and behavior. ScottFree's own source is GPL and is **not** copied or
ported; nothing GPL enters the workspace.

## Architecture

A new **zero-dependency `scott` crate** sits beside `zvm`/`gvm`/`blorb`. It owns
both the loader and the interpreter (the loaded database *is* the VM's state,
so they belong together, mirroring how `zvm` owns its own memory parser). It
carries no `ratatui`/`crossterm` — all terminal coupling stays in `app`.

The crate exposes the same host-facing shape the app already drives for the
other VMs: a `step() -> StepResult` loop resolved by `supply_line()`, plus a
real room table for the mapper.

### Crate layout

```
crates/scott/
  src/lib.rs        — crate root, public API surface
  src/database.rs   — parsed game model: header, rooms, items, actions, vocab, messages
  src/loader.rs     — ScottFree .dat text parser (integers + quoted strings)
  src/vm.rs         — interpreter: turn loop, action-table evaluation,
                      automatic (verb-0) actions, light/darkness + lamp timer,
                      treasures + treasure-room scoring, bit-flags, counters,
                      saved-room registers; StepResult + supply_line
```

(Exact module split is provisional and may be refined in the plan; `vm.rs` may
sub-split if it grows unwieldy.)

### App integration

A thin adapter, no VM logic in `app`:

- `crates/app/src/scott_session.rs` — `ScottSession` implementing the existing
  `Engine` trait (`crates/app/src/engine.rs`). It drives the `scott` VM's
  step/supply loop and builds a `ScreenModel` directly: a Grid window for the
  room/exits/visible-items header plus a scrolling Buffer window for game text
  (the same degenerate Grid+Buffer pair the Z-machine path produces).

The wiring seams (identified from the current architecture):

- **Detection:** add `LoadedStory::Scott(Vec<u8>)` and a content sniff in
  `hints.rs::extract_story` (validate the leading header integers for sane
  counts; the `.dat` extension is a hint, content validation is the gate).
- **Picker:** add `picker::Engine::Scott` with a validity probe.
- **Construction:** add a match arm in `startup.rs` boxing `ScottSession`
  behind `dyn Engine`; add a downcast helper in `engine_helpers.rs`.

### Mapper integration — the payoff

Scott rooms have genuine indices and real N/S/E/W/Up/Down exits. `ScottSession`
implements `Engine::current_location()` returning an honest
`ObjectSnapshot { number: room_index, name }`, and each move maps the typed
verb to a real `mapper::Direction`. No synthetic heading-hash IDs like the
Glulx path.

**Assumption to verify at plan time:** that `mapper::Direction` covers Up and
Down (Scott's fifth and sixth exits). If it does not, extending it is a small
additive change and becomes an early task in the plan.

### Save / restore

Scott VM state is small and fully serializable: current room, per-item current
location, the bit-flags, counters, saved-room registers, and the lamp/light
counter. It plugs directly into the host **Save State / Restore State**
snapshot system (engine-neutral, save-anywhere). The game's own in-database
SAVE command (a command opcode) is handled within the VM turn loop and is a
separate concern addressed in the plan.

## Data flow

```
load .dat  →  Database  →  Vm::new(Database)  →  step() ⇒ NeedLine
   ⇄ supply_line(command)
   →  parse verb/noun against vocabulary
   →  evaluate matching action-table entries (conditions → commands)
   →  run automatic (verb-0) actions
   →  emit output text + updated player location
   →  ScottSession builds ScreenModel + TurnResult
   →  mapper.observe(room, name, direction)
```

## Testing strategy

- **Golden-transcript tests:** run a known game headless (e.g. *Adventureland*)
  through a scripted command sequence and assert the produced transcript —
  the same real-game-oracle approach used for the other engines.
- **Loader unit tests:** parse a small hand-written `.dat` and assert the
  decoded header counts, a room's exits, an item's start location and
  treasure/auto-get markers, and a vocabulary pair.
- **Interpreter unit tests:** per condition code and per command opcode, drive
  a minimal database and assert the state transition (item moved, flag set,
  counter changed, room changed, lamp decremented, treasure scored).
- **Mapper smoke:** confirm a short walk produces the expected room graph with
  correct directional edges.

## Related tracking

- SQ-0388 — this quest (Scott Adams / ScottFree VM).
- `docs/design/2026-07-16-glk-crate-and-format-strategy.md` — the strategy note
  this decision refines (native-not-Glk for Scott Adams; `glk` extraction
  deferred to Hugo/Alan/TADS).
