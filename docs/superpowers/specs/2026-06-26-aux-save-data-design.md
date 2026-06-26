# Auxiliary Save Data (v5 `save/restore table`) — Design

**Date:** 2026-06-26
**Status:** Approved design (pending spec review) → implementation plan next
**TODO item:** #44 — "v5 save/restore to a memory table (vs file)"

## Goal

Implement the v5 auxiliary `save table bytes name` / `restore table bytes name`
opcodes (EXT:0x00 / EXT:0x01, store form, with operands). Games use these to
persist a **named region of memory** independently of a full game save — for
data meant to survive between sessions: "you've played before" flags, which
hints you've seen, a saved character/config.

Today both EXT opcodes ignore their operands and always perform a full
game-state save (`SaveRequest`/`RestoreRequest`). This design adds the
operand (table) form while keeping the 0-operand form unchanged.

## Background / constraints

- **The Z-machine is filesystem-free in production.** Full save/restore
  *suspend* to the host (`StepResult::SaveRequest`) because they need user
  interaction (pick a file). The engine never calls `std::fs` (only test
  helpers do).
- **The aux form needs no interaction**: the filename comes from the `name`
  operand, and the result (1/0, or bytes-read) is returned synchronously to the
  game. Routing it through the suspend/dialog flow would be wrong and would
  ripple across session/app/CLI/headless.
- **babelmap already persists per game** in a single `.babelmap` zip archive
  (`game.sav` + map + metadata), auto-saved each turn and at the per-game
  default path `archive_path(save_dir, ifid)`.

## Design overview

The **engine owns an in-memory aux table**; the **app persists it**, in one of
two modes chosen once by the user. The engine is mode-agnostic — it never
touches the filesystem.

```
  game: save table bytes name        game: restore table bytes name
            │                                   │
            ▼                                   ▼
   Machine.aux_data.insert(name,data)   Machine.aux_data.get(name) → copy back
   Machine.aux_dirty = true             (store bytes-read, or 0 if absent)
            │
            ▼  (post-turn, app side)
   app persists aux_data per `aux_storage`:
     • archive → fold into the .babelmap zip (aux.dat entry)
     • global  → write …/<ifid>.aux
   and clears aux_dirty.
```

## Components

### 1. zvm engine (mode-agnostic, no suspend)

- `Machine.aux_data: BTreeMap<String, Vec<u8>>` — the in-memory aux table
  (deterministic ordering → byte-stable serialization). Public so the host can
  read it (to persist) and replace it (on load). Default empty.
- `Machine.aux_dirty: bool` — set true on every successful `save table`. The
  host checks it to know aux data changed (the "notify"), then clears it after
  persisting. Default false.
- **EXT:0x00 `save`** — branch on operand count:
  - `ops.len() >= 3` (table form): `table = ops[0]`, `bytes = ops[1]`,
    `name_addr = ops[2]` (a 4th `prompt` operand, if present, is ignored).
    Read the name (see below), read `bytes` bytes from `table`
    (**bounds-clamped** to memory length — never panic), `aux_data.insert(name,
    data)`, set `aux_dirty`, **store 1**, return `Continue`. (In-memory insert
    cannot fail, so it always reports success; the host's later persistence is
    best-effort and orthogonal to the game-visible result.)
  - else (0 operands): existing full game-state save (`SaveRequest`).
- **EXT:0x01 `restore`** — branch on operand count:
  - `ops.len() >= 3` (table form): look up `name` in `aux_data`. If present,
    copy `min(bytes, data.len())` bytes into `table` (bounds-clamped) and
    **store that count**; if absent, **store 0**. Return `Continue`.
  - else: existing full restore (`RestoreRequest`).
- **`read_aux_name(addr) -> String`** — the `name` operand points to a
  **length-prefixed ASCII** string: byte 0 = length `n`, then `n` ASCII bytes
  (per the Standard's filename convention; *not* Z-encoded). Defensive: return
  an empty string if `addr == 0` or any read would exceed memory; the empty
  string is a valid map key (a game's "default" aux slot).

The engine has no concept of "archive" vs "global" — it only owns the table and
the dirty flag.

### 2. Aux blob codec (shared by both persistence backends)

A compact, length-prefixed binary encoding of `BTreeMap<String, Vec<u8>>`
(arbitrary keys + binary values, no base64 bloat). Used both for the zip
`aux.dat` entry and the global `…/<ifid>.aux` file:

```
u32  count
repeat count times:
  u16  name_len   name bytes (UTF-8)
  u32  data_len   data bytes
```

`encode_aux(&BTreeMap) -> Vec<u8>` / `decode_aux(&[u8]) -> BTreeMap` (decode is
tolerant: a malformed/truncated blob yields an empty map rather than erroring).

### 3. Config: `aux_storage`

- New global-config field `aux_storage: AuxStorage` where
  `enum AuxStorage { Ask, Archive, Global }`, serialized as `"ask"` / `"archive"`
  / `"global"`, **default `Ask`**.
- Standard config plumbing: field + `Default` + merge + `write_config` +
  round-trip/literal tests (mirror existing config fields).
- Surfaced in the **in-app config screen** as a 3-way selectable setting (so the
  user can change it after the first-use prompt).

### 4. App persistence modes

Driven by `aux_storage` once resolved to `Archive` or `Global`:

- **Archive mode** — aux rides inside the `.babelmap` zip:
  - **Write:** every `.babelmap` write (per-turn auto-save *and* manual Ctrl+S /
    save-as / in-game save) embeds the current `aux_data` as an `aux.dat` zip
    entry (`encode_aux`); the entry is omitted when the table is empty. Clearing
    `aux_dirty` after a write is a notify-bookkeeping detail — correctness does
    not depend on it, since every archive write embeds the latest table.
  - **Load:** `load_archive` also returns the decoded aux map (empty if no
    `aux.dat`); every archive-load site sets `session.machine.aux_data` from it.
- **Global mode** — aux lives in a single per-game file `aux_path(save_dir,
  ifid)` = `…/<sanitized-ifid>.aux`, independent of save slots:
  - **Startup:** on game launch, if the file exists, `decode_aux` it into
    `machine.aux_data` (this is what lets a *new* playthrough see prior data).
  - **Write:** when `aux_dirty`, write `encode_aux(aux_data)` to the file; clear
    `aux_dirty`.
  - **Override:** in this mode the `.babelmap` zip carries **no** `aux.dat`, and
    any existing zip aux entry is ignored on load — the global file is the sole
    source of truth.
  - The IFID is already filesystem-safe (`ZCODE-…`) but is sanitized
    defensively (restrict to `[A-Za-z0-9._-]`, fixed `.aux` extension, no path
    separators) when forming the path.

### 5. First-use prompt (`Ask` → set config once, for all games)

- Trigger: the first time `aux_dirty` is observed while `aux_storage == Ask`
  (i.e. the first aux **save** by any story). A `restore`-only first contact
  needs no prompt — there is nothing to persist and, on a genuinely first run,
  nothing saved to find (correctly returns 0).
- Action: a post-turn dialog (common dialog chrome, keyboard + mouse, TAB nav):

  > **This story saves persistent side-data** (e.g. remembering past
  > playthroughs). Where should babelmap keep it?
  > **[ With each save file ]   [ Globally for all stories ]**

  - "With each save file" → `aux_storage = Archive`.
  - "Globally for all stories" → `aux_storage = Global`; immediately also write
    the current `aux_data` to the global file (the save that triggered the
    prompt is not lost).
  - Either choice is **written back to the global config file** and governs all
    games from then on (no further prompts until the user changes it in the
    config screen).
- The dialog is themeable like the other dialogs (style selectors); no
  hard-coded colors.

## Data flow summary

- **Save path:** game `save table` → engine writes `aux_data` + `aux_dirty` →
  post-turn: if `Ask`, prompt (resolves to Archive/Global, writes config +
  persists); else persist per mode; clear `aux_dirty`.
- **Restore path:** game `restore table` → engine reads `aux_data` (already
  populated at startup in Global mode, or at archive-load in Archive mode) →
  stores bytes-read / 0. Fully synchronous.
- **Startup:** Global mode pre-loads `…/<ifid>.aux` into `aux_data`; Archive
  mode starts empty until an archive is loaded.

## Edge cases & decisions

- **No mid-instruction suspend.** Aux ops always return `Continue`. The one
  ordering question (first `restore` before the mode is chosen) is moot:
  prompting happens on the first *save*, when nothing is yet saved, so a
  post-turn prompt loses no data.
- **Bounds safety.** `table`/`bytes`/`name_addr` are game-controlled; all reads
  clamp to memory length (no panics), echoing the object-scan EOF guard.
- **Path safety.** Global-mode filename derives from the IFID and is sanitized;
  the game's `name` string never reaches the filesystem in global mode (it's
  only a `BTreeMap` key). In archive mode the name is likewise only a map key.
- **Mode switch.** If the user later changes `aux_storage`, data in the
  now-unused location is simply ignored (no migration). Acceptable.
- **CLI / headless.** They construct a `GameSession` and never wire archive
  persistence; `aux_data` works in-memory for the session and is simply not
  persisted (and `Ask` never prompts — there's no dialog host). Aux ops still
  behave correctly within the run.

## Testing strategy

- **zvm (unit, deterministic):** call `exec_ext` directly with a mock-populated
  memory: `save table` inserts + stores 1 + sets `aux_dirty`; `restore table`
  round-trips bytes + stores the count; missing key stores 0; empty/0 name
  address handled; out-of-bounds `table`/`bytes`/`name` do not panic; 0-operand
  `save`/`restore` still return `SaveRequest`/`RestoreRequest`.
- **codec (unit):** `encode_aux`/`decode_aux` round-trip incl. binary values and
  empty map; truncated blob decodes to empty.
- **app (unit):** archive write+read includes/round-trips `aux.dat`; global-file
  write+read round-trips; `aux_path` sanitization stays within `save_dir`;
  config round-trip for `aux_storage`.
- **integration (fixture-gated, best-effort):** a v5 story that issues `save
  table`/`restore table` round-trips through the chosen backend. If no bundled
  fixture exercises it, document the gap (as the in-game save/restore test does).

## Out of scope

- **#41 `input_stream`** — explicitly left unsupported (keeps its "not
  implemented" warning), per separate decision.
- **Spec-style fully-independent aux files** beyond the global-mode per-game
  file (e.g. arbitrary multi-file aux namespaces on disk). Global mode collapses
  a story's aux data into one file, which covers the real use cases.
- **Suspend-based first-restore prompting** (option (c) in design discussion) —
  unnecessary given the prompt-on-first-save resolution.

## Open questions

None blocking. Field/enum names (`aux_storage`, `AuxStorage`) and exact prompt
wording may be refined during planning.
