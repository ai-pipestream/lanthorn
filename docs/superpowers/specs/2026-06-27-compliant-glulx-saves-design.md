# Compliant Glulx Saves — Design

**Date:** 2026-06-27
**Status:** Approved (design), ready for per-sub-project planning
**Crates:** `crates/gvm`, `crates/app`, `crates/gvm-cli`

## Goal

Let Glulx games save and restore through their **own** in-game SAVE/RESTORE (the
`@save`/`@restore` opcodes writing/reading a Glk file stream), producing files in
the **standard Glulx Quetzal format** that other interpreters (Glulxe, Git,
Lectrote) can read — and that lanthorn can read back from them (full
bidirectional). In doing so, **unify Glulx saves with the Z-machine model**: the
portable bytes are real Quetzal wrapped in a `.lanthorn` archive (with a display
side-car), exactly as Z-machine already works. The lanthorn-specific Glulx
snapshot format (the custom `GReg` chunk) is **retired**.

## Decisions (from brainstorming)

1. **Full bidirectional** interop: read AND write standard Glulx Quetzal.
2. **Unify on compliant Quetzal; drop the custom snapshot.** Glulx
   `EngineSave.bytes` become spec-compliant Glulx Quetzal (memory + stack,
   resume via the `@save` call-stub — no `GReg`). The Glk window/stream model
   (today embedded as the `Glk ` chunk from the prior phase) moves to an archive
   **side-car**, mirroring the Z-machine's `screen.json`.
3. **In-game SAVE/RESTORE uses the same `.lanthorn` path as Z-machine** — same
   handlers, the Glulx guards lifted. The portable Quetzal lives inside.
4. **Extensions for raw interchange:** `.qzl` for Z-machine (already used),
   `.glksave` for Glulx. The `.lanthorn` wrapper stays one extension (self-
   describing via `engine.txt`). **Import detects the engine by content** (the
   IFZS/`IFhd` signature), not the extension; the existing foreign-engine guard
   rejects a Z↔Glulx mismatch. Extension only drives the picker filter + default
   save name.

## Background (current state)

- gvm has `@saveundo`/`@restoreundo` (0x125/0x126, in-memory undo), `@protect`
  (0x127), and a self-contained `save_state`/`restore_state` writing `FORM IFZS`
  with `IFhd`/`CMem`/`Stks`/`MAll` **plus a custom `GReg`** (sp/fp/pc/iosys/
  string-table/protect) **and a `Glk ` chunk** (window/stream model). It does
  **not** implement the `@save` (0x0123) / `@restore` (0x0124) **stream** opcodes
  or `@restart` (0x0122). `glk::StreamKind` has `Window` + `Memory` only — no
  `File`. Filerefs are stubbed no-ops (return NULL/false).
- The app already does Z-machine in-game save/restore through the `.lanthorn`
  archive (`PendingIo::Save`/`Restore` suspend → host writes/reads the archive
  whose `game.sav` is real Quetzal; `screen.json` is the zvm-only display
  side-car) and raw `.qzl`/`.sav` import/export (default export `<ifid>.qzl`).
  Glulx is guarded out of the raw import/export path.

## The compliant Glulx Quetzal format (sub-project B core)

`FORM IFZS`, per the Glulx spec §6, **read and write**:
- **`IFhd`** — the first 128 bytes of memory (the ROM header), to identify the
  game on restore (mismatch → refuse, like the Z-machine IFhd check).
- **`CMem`** (write) / **`CMem` or `UMem`** (read) — memory contents. `CMem` =
  the changed dynamic memory XOR-diffed against the original game image, zero-run
  RLE'd, length-prefixed by the current memory size (which may have grown via
  `setmemsize`/heap). We **write `CMem`**; we **read both** `CMem` and `UMem`
  (foreign saves may use either).
- **`Stks`** — the call stack in the Glulx spec's exact serialization (per frame:
  frame length, locals-format, locals, then the value stack). The **top frame is
  the resume call-stub** (`DestType`/`DestAddr`/`PC`/`FramePtr`) — this is how the
  resume point is encoded, replacing `GReg`.
- **heap** — match the Glulx spec / Glulxe representation for an active heap
  (reconcile gvm's current `MAll` with the standard; verify byte-exact against
  Glulxe). If the heap is empty, omit.

**No `GReg`, no `Glk ` chunk** in the portable bytes. `iosys`/string-table are
re-established by the resumed code path; the Glk model lives in the side-car.

### Resume mechanics — the call-stub

- **`@save L1 S1`**: build the resume call-stub for `S1` (the store dest) + the
  PC after `@save` + current FramePtr; serialize memory+stack (stub on top) to
  stream `L1`; store `0` to `S1` (success). On a later restore, the stub makes
  execution resume after `@save` with `S1` receiving **−1** (the "you were just
  restored" signal). `@restore L1 S1` reads the stream, validates `IFhd`,
  rebuilds memory+stack, and resumes via the top call-stub storing −1; on failure
  it stores into `S1` and continues.
- **App-initiated quick-save (Ctrl+S) at `glk_select`** (the nuance): the VM is
  blocked inside `@glk(select)`, not inside `@save`. Synthesize the **same shape
  of call-stub** for the `@glk` store + PC-after-`@glk` + FramePtr, so the
  app-snapshot is *also* valid compliant Quetzal. Resuming the pending input
  event additionally needs the Glk library's pending line/char request — that
  lives in the **side-car** (below), so app-snapshot restore = Quetzal +
  side-car. A game-initiated `@save` (taken inside the game's save routine) has
  no pending event, so its raw `.glksave` is fully portable on its own.

### The Glk side-car (replaces the embedded `Glk ` chunk)

A new glulx-only archive entry (e.g. `glk.json`), written/read **alongside**
`game.sav`, exactly as `screen.json` is for the Z-machine. It carries the Glk
window/stream model **and the pending input-event request** (window, kind,
buffer addr/maxlen) so a load-on-launch / quick-save restore reinstates the
windows and the input wait. Reuse the model serialization built in the prior
phase, relocated from inside the bytes to this side-car (extend it to include the
pending input request if it does not already). The raw `.qzl`/`.glksave` export
is **only** `game.sav` (no side-car) — portable; the side-car is lanthorn display
polish, like `screen.json`.

### Back-compat

**None.** lanthorn is pre-release, so the legacy `GReg`+`Glk ` Glulx snapshots
written earlier this session are simply dropped — `restore_state` reads only the
compliant Quetzal format. (Z-machine archives are unaffected; they were always
real Quetzal.)

## Decomposition (each sub-project: its own plan → build)

### Sub-project A — Glk file layer

Real filerefs + file streams, routed through the **`GlkBackend`** so gvm's core
stays pure and the **host** owns the filesystem and the SAVE/RESTORE file choice.
- gvm: `glk::StreamKind::File` (host-backed); a fileref table (usage + name +
  rock); wire the fileref group (`create_by_name`/`by_prompt`/`temp`/`destroy`/
  `does_file_exist`/`delete`/`iterate`) and `glk_stream_open_file` (0x0042) +
  read/write/seek/get-position/close to backend hooks. Replace the stubbed
  no-ops. Serialize file streams in the Glk model only as references (handles),
  not file contents.
- `GlkBackend` trait: file ops — open/create by name, prompt for a save/restore
  file (host UI), read/write/seek/close, exists/delete. The app implements these
  against a per-game save directory + a file dialog; gvm-cli against the
  filesystem + a stdin prompt.

### Sub-project B — `@save`/`@restore`/`@restart` + compliant Quetzal

gvm only.
- Implement `@save` (0x0123) / `@restore` (0x0124) writing/reading compliant
  Quetzal to/from a Glk stream via the call-stub mechanism; `@restart` (0x0122)
  resets to the initial image.
- Replace `save_state`/`restore_state` with the compliant format (write `CMem`,
  read `CMem`/`UMem`; spec-exact `Stks` with the synthesized `glk_select`
  call-stub for app snapshots; reconcile the heap chunk). Drop `GReg` + the
  embedded `Glk ` chunk; expose the Glk-model serialization separately for the
  side-car. No legacy `GReg` read path (pre-release; old snapshots are dropped).
- Verify byte-exact against the Glulx spec + Glulxe (manual cross-load) and the
  glulxercise save group.

### Sub-project C — wiring, UX, extensions, conformance

- App: route Glulx in-game SAVE/RESTORE through the **same** `.lanthorn` handlers
  as Z-machine (lift the remaining Glulx guards); write/read the `glk.json`
  side-car next to `game.sav` (glulx-only), mirroring `screen.json`.
- Raw interchange: **ungate** `.qzl`/`.sav` import/export for Glulx; make the
  picker engine-aware — Glulx default export `<ifid>.glksave`, filter accepts
  `.glksave`/`.qzl`/`.sav`; **import sniffs content** for the engine and the
  foreign-engine guard rejects mismatches. Z-machine `.qzl` behavior unchanged.
- gvm-cli: in-game SAVE/RESTORE via filesystem + a stdin file prompt.
- Conformance: glulxercise save tests green; document a manual round-trip with
  Lectrote/Glulxe.

## Testing strategy

- **gvm (B):** a program `@save`s to a memory stream, a fresh machine `@restore`s
  it → memory+stack+resume match and execution continues after `@save` with −1;
  cross-format read of a hand-built `UMem` save; `@restart` resets; the
  `glk_select` synthesized-stub app-snapshot round-trips. Reconcile the heap
  chunk with a heap-active save.
- **gvm (A):** fileref create/exist/delete + file-stream open/write/seek/read/
  close drive the backend hooks (a test backend with an in-memory filesystem);
  `glk_stream_open_file` round-trips bytes.
- **app (C):** a Glulx in-game SAVE writes a `.lanthorn` whose `game.sav` is
  compliant Quetzal + a `glk.json` side-car (no `screen.json`); RESTORE round-
  trips; raw `.glksave` export is bare Quetzal and re-imports; a foreign-engine
  raw file is refused by content sniff + guard; Z-machine `.qzl` path unchanged.

## Global constraints

- gvm stays zero-dep (file I/O is the **host's** via `GlkBackend`, not gvm's).
- Z-machine save/restore/restart/import/export stays **byte-for-byte unchanged**.
- Z-machine `.lanthorn` archives still load (always real Quetzal). Legacy Glulx
  `GReg` snapshots are intentionally dropped (pre-release, no back-compat).
- The compliant format is verified against the Glulx spec + a real interpreter,
  not just self-round-trip.
- 0 warnings + full `cargo test --workspace` green per task; TDD; one commit per
  task on the phase's worktree branch; no push; do not edit `TODO.md` mid-wave.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
