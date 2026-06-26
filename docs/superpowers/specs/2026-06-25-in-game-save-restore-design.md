# In-Game (Game-Initiated) Save / Restore — Design

**Date:** 2026-06-25
**Status:** Approved (design) — pending spec review.
**Context:** Step **B** of the Bureaucracy restore investigation. The game's own
SAVE/RESTORE commands currently auto-fail with "this game's in-game save/restore
isn't wired; use Ctrl+S/Ctrl+R". Wiring them makes the **standard path** work:
on restore the VM resumes inside the game's own save-routine, so the **game
redraws its own screen** (status line) — the behavior a normal interpreter has,
and the thing babelmap's snapshot Ctrl+S/Ctrl+R can't do. (Step **A** — persisting
screen state for the auto-load snapshot path — is deferred.)

## Goal

When the player types SAVE or RESTORE *to the game* (v4+), perform real file I/O
through babelmap's saves UI and complete the VM's `@save`/`@restore` correctly, so
the game continues — and, on restore, **redraws itself**.

## Background (verified)

- The VM already implements `@save` (`0OP:0x05` v4 / `EXT:0x00` v5) → `SaveRequest`
  and `@restore` (`0OP:0x06` / `EXT:0x01`) → `RestoreRequest`, capturing the result
  destination (`pending_save.result_dest`) / (`pending_restore_store`). exec.rs.
- `run_until_input` (session.rs) currently **auto-fails** both
  (`complete_save(false)` / `complete_restore_failure()`), and session.rs surfaces
  the "isn't wired" info line.
- **Store-2 semantics (the crux):** on a successful restore it is the original
  **`@save`** that "returns 2" in the restored state (the game checks `result == 2`
  → redraw), *not* `@restore`. babelmap's saved PC is **post-instruction** (the
  standard convention); for a v4+ `@save` (a store instruction) the store-variable
  byte is the **last byte of the instruction**, i.e. at **`saved_pc - 1`**. So
  restore stores `2` into `mem[saved_pc - 1]` — no PC-convention change needed.
  (Verified: `decode` sets `next_pc` past the store byte; `save_quetzal` records
  `state.pc` as-is.)
- `restore_quetzal(bytes)` loads dynamic memory + frames + eval stack + PC from the
  Quetzal; it does **not** store a result. `restore_file` = `restore_quetzal` +
  clears the undo stack.

## Decisions (settled with the user)

- **Save format:** in-game SAVE always writes babelmap's `.babelmap` archive
  (map + transcript + Quetzal), even when the map is empty.
- **Restore sources:** the restore picker lists **both** `.babelmap` and plain
  **`.qzl`** (standard Quetzal) files; either can be restored.
- **UI:** reuse babelmap's existing **saves-manager dialog** (save mode / restore
  mode), driven this time by the game's opcode rather than Ctrl+S/Ctrl+R.
- **Scope:** **v4+** only (store form). **v3** (branch form) in-game restore is
  deferred — recovering the branch from a post-instruction PC is ambiguous; v3
  games keep using host-mediated Ctrl+R for now. Documented as a follow-up.
- **Interop:** because babelmap's saved PC follows the standard convention, plain
  `.qzl` saves (incl. from other interpreters) restore correctly. (This does *not*
  necessarily make babelmap's `.babelmap`-embedded Quetzal loadable by other
  interpreters — that's a separate CMem/Stks-format question, out of scope.)

## Architecture / data flow

### 1. VM — restore-success completion (zvm)

Add `Machine::complete_restore_success(&mut self, data: &[u8]) -> Result<(), ZError>`:
- `restore_quetzal(data)?` (loads state; `state.pc` ← saved post-`@save` PC).
- v4+: `let store_var = self.mem.read_byte(self.state.pc - 1); self.do_store(Some(store_var), 2);`
- Clear `undo_stack` (a restore invalidates undo history, like `restore_file`).
- Clear `pending_restore_store` (the `@restore`'s own target is unused on success).
- Returns `Err` (state untouched) if `restore_quetzal` fails → caller calls
  `complete_restore_failure()`.

The existing `complete_save(ok)` (stores 1/0 into the `@save` target) and
`complete_restore_failure()` (stores 0 into the `@restore` target) handle the
write-success and cancel/failure cases unchanged.

### 2. Session — surface the request instead of auto-failing (app/session.rs)

`run_until_input` returns a richer stop reason:

```rust
enum RunStop { Input(InputKind), Quit, SavePending, RestorePending }
```

On `SaveRequest`/`RestoreRequest` it **returns** `SavePending`/`RestorePending`
(does NOT auto-fail). `submit`/`submit_char` set a new field on `TurnResult`:

```rust
pub pending_io: Option<PendingIo>,   // Save | Restore
```

New resume methods drive the VM forward after the host does the I/O:
- `resume_save(&mut self, wrote_ok: bool) -> TurnResult` → `complete_save(wrote_ok)`
  then continue `run_until_input`, returning the rest of the turn (which may itself
  end in another pending I/O, Quit, or Input).
- `resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult` → on `Some(bytes)`
  `complete_restore_success(bytes)` (fall back to `complete_restore_failure()` if it
  errs); on `None` (cancel) `complete_restore_failure()`. Then continue.

`run_until_input` (and these resumes) preserve the existing return contract for the
caller (quit flag, pending input kind, transcript, location, diagnostics).

### 3. App — drive the saves dialog in "in-game" mode (app/main.rs + state.rs)

`AppState` gains `ingame_io: Option<PendingIo>`. After `submit` (or `resume_*`), if
`result.pending_io` is `Some`, the run loop:
- **Save:** open the saves-manager dialog in **save mode** tagged in-game. On the
  user's confirm, write a `.babelmap` (reuse the Ctrl+S archive-write path:
  `save_archive_meta` with the current mapper/machine/transcript), then call
  `session.resume_save(true)`; on cancel, `resume_save(false)`. Clear `ingame_io`.
- **Restore:** open the saves dialog in **restore mode** tagged in-game, its file
  list including `*.babelmap` **and** `*.qzl` from the save dir. On confirm, obtain
  the Quetzal bytes — for `.babelmap`, read its `game.sav` entry (and load its map
  into the mapper, as Ctrl+R does); for `.qzl`, the file bytes directly — then call
  `session.resume_restore(Some(bytes))`; on cancel, `resume_restore(None)`. Clear
  `ingame_io`.
- The returned `TurnResult` is rendered as usual. On restore success the VM resumed
  inside the game's routine, so the game's next output **includes its own redraw**.

The dialog itself is the existing saves UI; "in-game" is a mode flag so its
confirm/cancel calls `resume_*` (VM completion) instead of the Ctrl+S/Ctrl+R direct
path. The "isn't wired" info line is removed.

### 4. Quetzal extraction helper (app/archive.rs)

Add `read_quetzal_from_file(path) -> io::Result<Vec<u8>>`: if the file is a
`.babelmap` zip, return its `game.sav` entry; otherwise return the raw bytes (a
plain `.qzl`). Used by the in-game restore path.

## Error handling

- Restore of a corrupt/incompatible Quetzal → `complete_restore_success` returns
  `Err` → fall back to `complete_restore_failure()` (game sees 0 / "failed"), state
  untouched; surface a transcript note.
- Save write failure → `resume_save(false)` (game sees 0); surface a note.
- Dialog cancel → failure result (standard "Ok./Failed." game messaging).
- v3 game issuing `@save`/`@restore` → keep the current "use Ctrl+S/Ctrl+R" info
  line (v3 in-game I/O is out of scope).

## Testing

- **VM unit:** `complete_restore_success` stores 2 into `mem[pc-1]`'s variable after
  a round-trip — build a v4 story where `@save` stores into global G0; save_quetzal
  at that point; `complete_restore_success(blob)` ⇒ `global(0) == 2` and PC resumed.
  Error path: corrupt blob ⇒ `Err`, state unchanged.
- **Session:** a `@save`-issuing step returns `TurnResult.pending_io == Some(Save)`;
  `resume_save(true)` continues and the game proceeds; `@restore` → `Some(Restore)`;
  `resume_restore(Some(blob))` resumes and the game continues.
- **Bureaucracy (v4) end-to-end (fixture-gated):** from a running game, an in-game
  SAVE writes a `.babelmap`; a later in-game RESTORE of it resumes and the
  **upper-window status line is non-empty** after the resumed turn (the redraw the
  whole exercise is about).
- **`.qzl` import:** `read_quetzal_from_file` returns the game.sav for a `.babelmap`
  and raw bytes for a `.qzl`; a `.qzl` restores via `complete_restore_success`.
- Headless smoke test still passes; existing save/restore tests unaffected.

## Out of scope / deferred

- **Step A:** persisting screen state for babelmap's snapshot path (Ctrl+R /
  auto-load) — the fresh-launch case where the window was never split. Separate
  spec.
- **v3 in-game save/restore** (branch form) — follow-up.
- `EXT:0x00/0x01` save/restore **to a memory table** (operand form) — already out of
  scope (a different TODO).
- Making babelmap's Quetzal byte-compatible with other interpreters' *readers*
  (the dfrotz "Error reading save file" — a CMem/Stks question), separate.
