# zvm-cli Aux ("Global State") Persistence — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/zvm-cli` (frontend only — no `zvm` engine changes)
**Companion:** ships in the same zvm-cli DOS-parity push as
`2026-06-27-zvm-cli-screen-model-design.md` (independent concern: file I/O, not
rendering).

## Goal

Persist the v5 auxiliary save tables (the `save table … name` /
`restore table … name` opcode form) across `zvm-cli` sessions, so games that
write named auxiliary data (Bureaucracy-style forms, "memo"/notebook features)
keep it between runs — as the DOS interpreters did by writing named files to
disk. Today `zvm-cli` keeps these tables only in memory and drops them on exit.

## Background (current code)

- The engine fully implements the opcodes (`crates/zvm/src/cpu/exec.rs`):
  - EXT:0x00 `save table bytes name` (≥3 operands): copies `bytes` of the
    table into `self.aux_data.insert(name, data)`, sets `self.aux_dirty =
    true`, stores `1`.
  - EXT:0x01 `restore table bytes name`: reads `self.aux_data.get(&name)`,
    copies into the table, stores the byte count (`0` if absent).
  - State lives on `Machine`: `pub aux_data: BTreeMap<String, Vec<u8>>` and
    `pub aux_dirty: bool`. Neither opcode suspends (no `StepResult`); the host
    is expected to watch `aux_dirty` and persist.
- lanthorn persists this via its `aux_storage` config (per-game `<ifid>.aux`).
  `zvm-cli` does **not** touch `aux_data`/`aux_dirty` at all, so the tables are
  session-only and never reloaded.
- Because `restore table` reads the in-memory map (not disk), cross-session
  restore requires the host to **preload** the map before the game runs.

## Design

A new module `crates/zvm-cli/src/aux.rs` plus a few call sites in `main.rs`.
No engine changes.

### 1. Storage location & format

One combined file per story, next to the story file: `<story-stem>.aux`
(e.g. `leathergoddesses.z5` → `leathergoddesses.aux`). It holds the entire
`aux_data` map (the game-chosen names live inside the file, so no per-name
filenames and no name sanitization needed).

Zero-dep length-prefixed binary codec:

```
file        := magic(4 = b"ZAUX") version(1 = 1) count(u32-le)
               then `count` entries
entry       := name_len(u32-le) name(name_len bytes, UTF-8)
               data_len(u32-le) data(data_len bytes)
```

- `encode_aux(&BTreeMap<String, Vec<u8>>) -> Vec<u8>`
- `decode_aux(&[u8]) -> Result<BTreeMap<String, Vec<u8>>, AuxError>` — rejects
  a bad magic/version or truncation (returns an error; the host warns and
  starts empty rather than crashing).

### 2. Preload at startup (and on restart)

After `build_machine` succeeds, if `<story-stem>.aux` exists and decodes,
populate `machine.aux_data` from it and leave `aux_dirty = false`. A decode
error is non-fatal: `eprintln!` a `zvm: warning:` line and continue with an
empty map. `StepResult::Restart` rebuilds the machine, so it re-runs the same
preload into the fresh machine.

### 3. Flush on dirty

After each `machine.step()` (alongside the existing diagnostics drain), if
`machine.aux_dirty`: write `encode_aux(&machine.aux_data)` to
`<story-stem>.aux`, then set `machine.aux_dirty = false`. A write error is
non-fatal: warn via `eprintln!` and clear the flag (so it is not retried every
step). This mirrors the lower-window streaming cadence — persistence is
flushed as soon as the game commits a table.

### 4. `--no-aux` opt-out

A `--no-aux` flag (parsed in `main` with `--no-status`) disables both preload
and flush: `aux_data` stays in memory only, today's behavior, no disk writes.
For deterministic regression/debug runs that must not touch the filesystem.

### 5. Path handling

The aux path derives from the story `Path` given on the command line: same
directory, file stem + `.aux` extension. If the story directory is not
writable, the flush warns and continues (the game still works in-session). The
helper `aux_path(story: &Path) -> PathBuf` is pure and unit-testable.

## Testing

In `crates/zvm-cli/src/aux.rs`:

- `encode_aux`/`decode_aux` round-trip a map with multiple entries, empty
  values, and Unicode names; output is byte-stable (BTreeMap ordering).
- `decode_aux` rejects bad magic, wrong version, and truncated input with an
  error (no panic).
- `aux_path` maps `dir/story.z5` → `dir/story.aux` and handles a stem with no
  extension.
- Preload helper: given decoded bytes, populates a `Machine`'s `aux_data` and
  leaves `aux_dirty == false`.
- Flush helper: with `aux_dirty == true`, returns the bytes to write and clears
  the flag; with `--no-aux` set, the flush/preload helpers are no-ops.

An integration check (temp dir): run a tiny v5 story that `save table`s a known
blob, exit, relaunch, and confirm `restore table` returns the blob; with
`--no-aux` the second run sees nothing.

## Out of scope

- Keying the aux store by IFID instead of story filename (renaming the story
  orphans its aux file) — basic CLI uses the story stem.
- Merging aux tables into Quetzal full saves (aux is separate named storage).
- A configurable aux directory — fixed next-to-story.

## Global constraints

- 0 warnings (`cargo build`, `cargo doc`) + full `cargo test` green per task.
- `zvm-cli` stays **zero-dependency** (std only). No new crates.
- **No `zvm` engine changes** — frontend only (`aux_data`/`aux_dirty` already
  public).
- Default behavior writes `<story-stem>.aux` only when a game actually saves a
  table; games that never use the opcode produce no file. `--no-aux` fully
  disables disk I/O.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
