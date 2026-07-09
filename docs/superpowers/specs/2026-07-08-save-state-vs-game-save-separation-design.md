# Separate "Save State" from the Game's `@save`/`@restore` by File Format — Design

**Date:** 2026-07-08
**Status:** Approved for planning
**Quest:** SQ-0227
**Related:** Fixes a confirmed **SQ-0163 regression** (host restore of an in-game `@save`
save misexecutes). Unblocks **SQ-0158** (Z-machine interop testing) — a game save
*becomes* a standard interop `.qzl` under this design. Supersedes the earlier
"internal `Meta` marker" idea (the file extension is the marker instead).

## Goal

Cleanly separate babelmap's two save mechanisms so they can never be confused — by
giving them **different file formats/extensions**, which also fixes the regression and
makes game saves portable to other interpreters.

## Background: two mechanisms, one confusing format

babelmap has two distinct save mechanisms that today both write a `.babelmap` archive
via the same `save_archive_meta`/`save_named` path, distinguished *only* by the Quetzal
PC convention embedded in the archive — which nothing records:

- **Emulator "Save State"** (Ctrl+S, auto-save, session resume): a full session snapshot
  taken at an input prompt. Quetzal PC = **resume point**. Restored by *resuming*
  (`restore_file`).
- **The game's own `@save`/`@restore`** (the story's `save` verb): standard Quetzal.
  Since SQ-0163, PC = **result descriptor** (§5.8). Restored by *completing the
  descriptor* (`complete_restore_success`).

**Confirmed regression (SQ-0163):** an in-game `@save` writes a `.babelmap` with
descriptor-PC, but every host restore path (`restore_state` → `restore_file`) just
resumes — landing at the descriptor and misexecuting. Verified: `restore_file` sets
`pc = 0x41` (descriptor) instead of `0x42` (resume). (See
`docs/superpowers/specs/2026-07-08-v3-standard-save-restore-design.md` for the PC
convention; SQ-0163's tests missed this because none host-restored an in-game save.)

## Design: the file format IS the kind

| Kind | Extension | Format | PC convention | How it restores |
|---|---|---|---|---|
| **Save State** (Ctrl+S / auto-save / session resume) | `.babelmap` | rich archive (map + screen + transcript + VM state) | resume point | resume (`restore_file`) |
| **Game save** (the story's `save`/`restore` verb) | `.qzl` (Z-machine only) | **bare standard Quetzal** (VM state only) | result descriptor | complete descriptor (`@restore` semantics) |

Glulx has no `@save` opcode, so Glulx has **only** Save State (`.babelmap`); there is no
Glulx game-save `.qzl`.

**The regression is fixed by construction:** no descriptor-PC data ever lands in a
`.babelmap` again — `.babelmap` archives are *always* resume-convention Save States, so
host restore of a `.babelmap` always resumes correctly. Game saves are `.qzl` and always
complete the descriptor. The two can no longer be conflated because they are different
files.

### Save side

- **In-game `@save`** (`main.rs:4263`, currently `save_named` → `.babelmap`): instead
  write a **bare standard `.qzl`** via the bare-Quetzal path (`persist_files::save_game`
  / `machine.save_quetzal()`), captured while `pending_save` is set so `save_pc()`
  yields the descriptor PC. Filename `<ifid>-<slug>.qzl` in the saves dir. It bundles
  **no** map/screen/transcript — those live in the session `.babelmap` (auto-save), and
  a bare `.qzl` is portable and standard. On `@restore` the game redraws its own screen
  (standard ZMSD §8; sidesteps SQ-0228).
- **Save State** (Ctrl+S / `save-state`, quit-save, exit-save, auto-save, named host
  slots — `main.rs:2445, 2484, 3798, 3893, 3910`): unchanged — `.babelmap` archive,
  `pending_save` is `None` so `save_pc()` = resume PC.

### Restore side — key off the extension

Restore behavior is chosen by the loaded file's extension, uniformly across every
trigger (saves-manager Load, `restore-state`/Ctrl+R, the game's `@restore`, and import):

- **`.babelmap`** → resume (`restore_file`) — full Save State restore.
- **`.qzl`** → complete the descriptor (`complete_restore_success` semantics: v3 branch
  true / v4+ store 2, advance PC), then resume.

This unifies three things that are currently separate and partly broken:
- **In-game `@restore`** already completes the descriptor (correct for `.qzl`).
- **Host restore of a game save** now completes the descriptor (fixes the regression).
- **Foreign `.qzl` import** (`main.rs:3448`, currently `restore_game` →
  `restore_quetzal`, a bare resume) now completes the descriptor — **making foreign
  standard saves import correctly** (SQ-0158's READ direction, for free).

The saves manager keeps **one list** showing both `.babelmap` and `.qzl` (it already
does, via `read_quetzal_from_file` / `list_qzl`); the load path dispatches on extension.

### Rename (naming layer)

Rename the emulator commands/labels so users see the distinction:
`save-game` → `save-state`, `load-game` → `restore-state` (slash commands `slash.rs:141-146`,
keymap Ctrl+S/Ctrl+R `keymap.rs:212-213`, `GAME_HINTS` `main.rs:153-158` + its hint
labels, dialog/prompt labels, `README.md:63-64`). The saves-manager title stays "Saves"
(it lists both kinds). Fix the now-inaccurate `docs/features/saves.md:13-17` (in-game
`@save` and Save State are different *files*, not the same archive path).

## Components / files

- `crates/app/src/persist_files.rs` — a game-save writer (`.qzl`, bare Quetzal, descriptor
  PC) + a game-save reader that completes the descriptor; `restore_game` completes the
  descriptor (foreign import).
- `crates/app/src/main.rs` — in-game `@save` writes `.qzl` (`~4263`); the load/restore
  dispatch keys off extension (`~3448, 3506-3617, 3920-3996`, launch/auto paths); rename
  hint/label strings.
- `crates/app/src/session.rs` (+ `engine.rs`) — an engine-neutral restore that dispatches
  resume vs complete-descriptor (Glulx always resumes).
- `crates/zvm/src/cpu/exec.rs` — `complete_restore_success` already exists; expose it for
  the host game-save path if not already reachable.
- `crates/app/src/slash.rs`, `keymap.rs`, `render/saves.rs`, `render/quit_dialog.rs` —
  rename `save-game`/`load-game` → `save-state`/`restore-state` + labels.
- `docs/features/saves.md`, `README.md` — correct + rename.

## Testing

- **Red→green regression (zvm):** after an in-game `@save`, restoring the produced `.qzl`
  via the game-save path resumes at the post-instruction PC (not the descriptor) with the
  game seeing "restored"; and host restore of a `.babelmap` still resumes. (The current
  behavior is red.)
- **Extension dispatch (app):** loading a `.qzl` completes the descriptor; loading a
  `.babelmap` resumes; foreign `.qzl` import completes the descriptor.
- **In-game round trip (app):** `@save` → `.qzl` on disk → `@restore` of that `.qzl`
  reproduces state (probe-based, per SQ-0158's oracle).
- **Rename:** command/keymap/hint-label tests updated (`keymap.rs:835`, `main.rs:5414`).
- Full `cargo test -p zvm -p app` green.

## Out of scope / noted

- **Glulx game saves** — none exist (no `@save` opcode); Glulx keeps only `.babelmap`.
- **Bare `.qzl` *export*** of a Save State (`main.rs:4319`): a Save State is
  resume-convention, so exporting it as a "standard" `.qzl` is not truly portable — a
  known limitation (real portability of arbitrary session snapshots is an interop concern,
  SQ-0158). Game saves *are* already standard `.qzl`, so they export/round-trip correctly.
- **Migration:** no backward compatibility — the user deletes old `.babelmap` slots that
  were written with descriptor-PC (the post-SQ-0163, pre-SQ-0227 window). Legacy
  resume-convention `.babelmap`s still restore correctly.
