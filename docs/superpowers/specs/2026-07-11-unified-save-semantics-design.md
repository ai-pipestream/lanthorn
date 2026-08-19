# Unified save semantics across Z-machine and Glulx (SQ-0283)

**Status:** design, pending review
**Date:** 2026-07-11
**Quest:** SQ-0283
**Spec references:** Glulx Spec (Plotkin) §1.3.2 (call stubs), §1.8 (save format), §1.8.1
(dynamic memory), §1.8.2 (stack + save stub), §1.8.5 (state NOT saved); Quetzal
(Frost) §3 (CMem RLE). Local copies: `scratchpad/glulx.md`, `scratchpad/quetzal.txt`.

## Problem

lanthorn has two deliberately-distinct save mechanisms:

- **Save State / Restore State** — the host emulator snapshot (Ctrl+S / `/save-state`),
  a rich `.lanthorn` archive (VM + Glk model + map + screen + transcript + history).
- **`@save` / `@restore`** — the game's *own* in-game save, invoked when the player
  types `SAVE` / `RESTORE`.

On the **Z-machine** these are already correct and consistent: `@save` writes a
bare standard Quetzal `.qzl` (VM state only, interop-tested against dfrotz); Save
State writes `.lanthorn`.

On **Glulx** they are conflated: the game's `@save` is served by the **host
snapshot** (`machine.save_state()` — the full superset including `GReg` and the
`Glk ` chunk that embeds the entire VFS), written to a `.qzl`/`.glksave`. Two
defects follow:

1. **Correctness bug ("Save failed." in Counterfeit Monkey).** Glulx `@save`
   (opcode 0x0123) ignores its `L1` output-stream operand and routes the snapshot to
   a host file. The game's Glk library checks the *write count* of the stream it
   opened; it reads 0 and prints "Save failed." Confirmed by instrumentation:
   crediting the stream flips CM to "Ok." Affects **both** `gvm-cli` and the app.
2. **Format inconsistency.** A Glulx in-game save is the full host-snapshot superset,
   not a real, portable in-game save like the Z-machine's.

## Goal

Make the two engines behave identically:

- **Save State / Restore State →** `.lanthorn` host snapshot, both engines. *(Already true — unchanged.)*
- **`@save` / `@restore` →** a real, **spec-conformant standard** in-game save, both engines. *(New for Glulx.)*
- **In-game save/restore UI identical** across engines. *(Already true in the app; brought into line in `gvm-cli`.)*
- **VFS holds only the game's own external files** (transcripts, recordings, data) —
  saves are fully decoupled from it (§7). This also fixes a latent bug where a Glulx
  `@restore` only worked after a prior save had materialized a VFS slot.

**Chosen approach — "B-lite": implement the spec-conformant standard Glulx-Quetzal
in-game save now, verify its round-trip inside gvm now, and defer only
cross-interpreter *golden-file* interop testing to SQ-0229.** Delivery stays
host-intercepted (`SaveRequest`), matching how the Z-machine's own `@save` already
works — so this is genuine parity, not a divergent mechanism.

Rejected alternatives: (A) route Glulx `@save` through the live Glk output stream +
VFS — diverges from the Z-machine's host-intercept model, moves saves out of the
saves manager, needs recursion special-casing. (C) keep a gvm-internal `GReg`-bearing
bare format — achieves UI/semantic consistency but the in-game save is not portable,
so it fails to match the Z-machine's interop quality bar and would be rebuilt for
SQ-0229 anyway.

## Design

### 1. Spec-conformant standard Glulx-Quetzal in-game save (`gvm`)

Add to `crates/gvm/src/exec.rs`, beside `save_state()` / `restore_state()`:

- **`pub fn save_quetzal(&self) -> Vec<u8>`** — a standard Glulx save: `FORM IFZS`
  with **`IFhd + CMem + Stks + MAll` only**. No `GReg`, no `Glk ` chunk. Reuses the
  existing `IFhd`/`compress_ram`(`CMem`)/`Stks`/`MAll` builders verbatim; it differs
  from `save_state()` solely by *omitting* the two non-standard chunks. Per §1.8, PC,
  FP, and SP are **not** serialized separately — they are recovered from the saved
  stack plus the `@save` call stub (see §2).

- **`pub fn restore_quetzal(&mut self, blob: &[u8]) -> Result<(), GError>`** — restores
  RAM/stack/heap from `IFhd`/`CMem`(or `UMem`)/`Stks`/`MAll`, then **pops the call
  stub off the restored stack and stores the result code (-1) into it** to resume just
  after the original `@save` (§1.8.2). It **must leave live interpreter state
  untouched** per §1.8.5: the Glk model (windows/streams/VFS), `iosys_mode`/`rock`,
  the string-decoding table, and the protect range all keep their current live
  values — this is exactly the mid-session `@restore` semantics. Contrast
  `restore_state()`, which *replaces* `iosys`/`stringtbl`/etc. from `GReg` and the Glk
  model from the `Glk ` chunk (correct only for a cold, cross-session Save State into
  a fresh machine).
  - Memory (§1.8.1): read the 4-byte memsize, resize memory to it, reset
    `[RAMSTART, EndMem)` to the original image (zero-filling above EXTSTART), apply
    the XOR-diff RLE, **honoring the live protect range** (protected bytes keep their
    pre-restore values). Must accept `UMem` as well as `CMem` (writing `CMem` only is
    fine). This mirrors the existing `restore_state` memory path — factor it out and
    share it.

Refactor the shared memory/stack/heap restore into a private helper used by both
`restore_state` and `restore_quetzal`; the only differences are the register source
(`GReg` vs. call stub) and whether `iosys`/`stringtbl`/`protect`/Glk are replaced or
left live.

### 2. `@save` pushes a call stub (the resume-core change)

gvm already implements the spec's 4-word call stub — `call_function` (exec.rs:1110)
pushes `(DestType, DestAddr, PC, FramePtr)` via `Dest::to_stub()`, and `return_value`
(exec.rs:1133) pops it and stores per `DestType`. `@save`/`@restore` reuse this.

Rework the `@save` handler (exec.rs 0x0123). **Old:** store -1 into `S1`, stash `dest`
in `pending_saveload`, rely on `GReg` for the PC. **New (§1.8.2):**
- Read `L1` (output stream) and `S1` (`dest`).
- Push a call stub for `S1`: `(dtype, daddr) = dest.to_stub()`, `PC = self.pc` (already
  advanced past the opcode — the "instruction after" per §1.3.2), `FramePtr = self.fp`.
- Mark a save pending (no baked -1, no stored `dest` needed — the stub carries it).
- **Shim** (§2 below): credit `L1` with `save_quetzal().len()`.
- Suspend → `SaveRequest`.

**`complete_save(ok)`** (exec.rs:3403): instead of `store(dest, 0|1)`, **pop the call
stub and store the run result** — 0 on success, 1 on failure — resuming at the stub's
PC (just after `@save`) with `FramePtr` restored. Factor `return_value`'s
stub-pop-and-store tail into a shared `pop_save_stub_and_store(v)` (it must NOT do the
`sp = fp` frame-teardown that `return_value` does — `@save` pushed only a stub, not a
new frame).

`@restore` (0x0124) still suspends → `RestoreRequest`; its own `S1` is written only on
failure (§2: a successful `@restore` "never returns a value" because execution jumps
into the restored `@save`'s stub instead).

### 3. Glk stream write-count shim (the "Save failed." fix)

The library's write-count check must pass even though the real bytes go to the host
file, and we must NOT write them into the VFS (a bare save omits the `Glk ` chunk, so
it doesn't self-embed — but writing it into the VFS would still bloat future
`save_state` snapshots, and it's simply unnecessary).

- Add **`pub fn note_stream_write(&mut self, id: u32, n: u32)`** to
  `crates/gvm/src/glk.rs`: bump the stream's `write_count` by `n`, storing no bytes.
- The `@save` handler credits `L1` with `save_quetzal().len()` (§1 above).

The only theoretical loss is that the stream reports bytes it doesn't physically hold
— invisible to games, which never read back their own save stream. The shim is
independent of the save *format*; it is required because delivery is host-intercepted
(as on the Z-machine), not because of anything Glulx-specific in the bytes.

### 4. In-game restore completion for Glulx

Add **`pub fn complete_restore_quetzal(&mut self, blob: &[u8]) -> bool`** mirroring
`complete_restore_success` but calling `restore_quetzal` (stub-based, live-state
preserving) instead of `restore_state`. Hosts call it for an in-game `@restore` of a
`.qzl`; `restore_state` / `complete_restore_success` remains the Save State path.

### 5. Host wiring

**`gvm-cli` (`crates/gvm-cli/src/main.rs`):** bring its in-game save UI into line with
`zvm-cli` (prompt for a filename; bare `.qzl`), replacing the fixed `<story>.glksave`
slot.
- `SaveRequest` → prompt "Save to file:" (mirror zvm-cli), write `machine.save_quetzal()`
  to the chosen path, `complete_save(ok)`.
- `RestoreRequest` → prompt "Restore from file:", read the file,
  `machine.complete_restore_quetzal(&bytes)` (else `complete_restore_failure`).
- The `<story>.glksave` fixed slot is removed. The `<story>.glkvfs` VFS sidecar is
  unchanged. `create_by_prompt` SavedGame auto-resolve (`__prompt_1__`) stays; the real
  filename is taken at the `SaveRequest`/`RestoreRequest` prompt, matching zvm-cli.

**app (`crates/app/src/`):**
- `persist_files.rs:save_game_named_bytes` (Glulx in-game save) → write
  `session.save_quetzal()` bytes instead of `session.save_state().bytes`; keep the
  `<ifid>-<slug>.qzl` naming, unifying with Z-machine's `save_game_named`.
- Glulx in-game `@restore` (the `.qzl` branch of `Action::SavesLoad` / `resume_restore`)
  → `complete_restore_quetzal`. The `.lanthorn`-picked-at-`@restore` fall-through to a
  full session resume (SQ-0227) is unchanged.
- Expose `save_quetzal` / `complete_restore_quetzal` through `Engine` /
  `glulx_session.rs` as needed.
- Save State (Ctrl+S → `.lanthorn`, inner `game.glksave` = `save_state()`) is unchanged.

### 6. Bonus consistency fix (zvm-cli)

`zvm-cli`'s in-game `@restore` calls `restore_quetzal()` directly instead of the
completion method, so it does not advance the `@save` descriptor forward the way the
app does. Switch it to `complete_restore_success` so both hosts complete an in-game
restore identically. *(The zvm `restore_quetzal` is a different crate's method; the
name coincidence with the new gvm method is harmless.)*

### 7. SavedGame Glk streams become host conduits, not VFS files (VFS cleanup)

With saves living in host `.qzl` files, the `SavedGame` `create_by_prompt` → stream
path no longer needs the VFS at all. Today `stream_open_file` (glk.rs:1522) for a
`SavedGame` fileref **materializes an empty `__prompt_1__` entry in `self.files`** on
Write (persisted to `.glkvfs`, embedded in Save States), and on Read **fails unless
that entry exists** (glk.rs:1533) — so a Glulx `@restore` only reaches the `@restore`
opcode if a prior save created the slot. That empty slot is both vestigial storage and
a latent restore bug.

Fix: make `SavedGame`-usage streams **null conduits**, decoupled from the VFS.
- Add `StreamKind::Null` (a `Copy` variant): writes are discarded (write count still
  bumpable — see the shim), reads return EOF, no VFS backing, no `file_streams` entry.
- In `stream_open_file`, when `usage & 0x0f == fileusage_SavedGame (0x01)`: create a
  `Null` stream **without touching `self.files`**, and **succeed for every mode**
  (crucially, Read succeeds even with no prior save). The game therefore always reaches
  `@save`/`@restore`, and the **host** decides success — `@save` writes the `.qzl` (the
  shim credits the conduit's write count); `@restore` reads the `.qzl` or
  `complete_restore_failure`s. This also fixes the "restore only works after a first
  save" bug.
- Handle `Null` in `stream_close` (free like a memory stream; no `file_streams` entry),
  and in `file_stream_write`/`read_char`/position as discard/EOF/no-op. `note_stream_write`
  already works on any stream via `stream_mut`.
- Because `SavedGame` files never enter `self.files`, they no longer persist to
  `.glkvfs`/`.gvfs` nor ride inside Save States. The `__prompt_` name and its
  `file_names()`/persistence filters (glk.rs:1043, 2262) become dead for saves; leave
  the host `create_by_prompt` SavedGame auto-resolve as-is (the conduit ignores the
  name) to keep the diff contained.

Net: the VFS now holds only the game's genuine external files (transcripts, command
recordings, data files); saves are host `.qzl`; Save States are `.lanthorn`.

## Documentation (required deliverable)

- **`docs/persistence.md`** — primary update:
  - Terminology (~line 18): Glulx `@save` produces a bare **standard Glulx-Quetzal**
    save, not the host snapshot blob.
  - Layer 1 (~lines 31–33): Glulx now writes a standard in-game save (VM state only,
    call-stub resume, no VFS embed); note interop is verified internally now,
    cross-interpreter goldens tracked under SQ-0229.
  - "Where each thing lands" table (~lines 92–96): Glulx `@save`/`@restore` rows → bare
    `.qzl` for `gvm-cli` (now a prompted filename) and app.
  - `create_by_prompt` section (~lines 102–112): `SavedGame` no longer resolves into a
    VFS slot — its stream is a host conduit and the save is a `.qzl`. Clarify that the
    VFS (Layer 3) now holds only the game's transcripts, recordings, and data files.
- **`docs/features/saves.md`** — extend "Standard in-game save/restore" to state Glulx
  now also writes a real, standard in-game save (VM state only, distinct from
  `.lanthorn`), with the SQ-0229 caveat that Glulx *cross-interpreter* interop is not
  yet golden-tested.
- **`crates/gvm/GLULX_NOTES.md` §14** — document the new `save_quetzal`/`restore_quetzal`
  standard path (call-stub resume; `IFhd/CMem/Stks/MAll`; §1.8.5 live-state exclusions)
  alongside the existing `GReg`-based `save_state` snapshot, and why the two differ.
- **README.md** — no change (consistency fix + bugfix, not a new major feature).

## Testing

- **`gvm` unit tests (from-spec conformance + round-trip):**
  - `save_quetzal` emits exactly `IFhd/CMem/Stks/MAll` — assert `GReg` and `Glk ` are
    **absent** (mirror the `strip_chunk`/Glk-chunk test at exec.rs:5906).
  - `@save` pushes a call stub, and a `save_quetzal → complete_save(true)` round-trip
    resumes just after `@save` with `S1 == 0` (success) in the live run.
  - Full `@save`/`restore_quetzal` round-trip: RAM + stack + heap restored; execution
    resumes at the post-`@save` PC with `S1 == -1`; **live Glk model, iosys, string
    table, and protect range are preserved** (open a window/stream/VFS file and set a
    non-default iosys before saving; assert they survive `restore_quetzal`, i.e. are
    NOT reset — the key §1.8.5 property that distinguishes it from `restore_state`).
  - `restore_quetzal` reads a `UMem` variant as well as `CMem`.
  - Protect range honored: a `@protect` range keeps its pre-restore bytes across
    `restore_quetzal` (§1.8.5).
  - `note_stream_write` bumps `write_count` without growing the backing VFS file; the
    `@save` handler credits its `L1` stream.
  - **SavedGame conduit (§7):** opening a `SavedGame` stream creates **no** `self.files`
    entry (Write) and **succeeds with no prior save** (Read); a `Null` stream discards
    writes / reads EOF; after a Glulx `@save`, the story's VFS / `.glkvfs` and a Save
    State's `Glk ` chunk contain **no** save slot. Assert an `@restore` reaches the
    opcode (host decides) even when nothing was saved this session.
  - Regression: `save_state`/`restore_state` (Save State) bytes and round-trip
    unchanged after the shared-restore refactor.
- **`gvm-cli`:** extend the headless drive tests for the prompted-filename
  save/restore round-trip with the standard serializer.
- **Real-game smoke (manual, user):** Counterfeit Monkey in both `gvm-cli` and the app
  — `save` prints "Ok." (not "Save failed."); `restore` restores. A Glulx game with an
  open transcript window: `@restore` must NOT wipe the live transcript window/VFS
  (validates the §1.8.5 live-state exclusion end-to-end).
- **Regression:** existing Z-machine `@save`/`@restore` and both engines' Save State /
  `.lanthorn` paths unchanged; `cargo test` across the workspace.

## Out of scope

- **Cross-interpreter Glulx save interop *testing*** — reading/writing saves against
  Glulxe/Git golden files and the interop harness → **SQ-0229**. (The format we write
  here is spec-conformant by construction and internally round-trip-verified; SQ-0229
  proves the cross-interpreter half.)
- Routing Glulx `@save` through the live Glk output stream + VFS (Design A).
- Any Save State / `.lanthorn` format change.

## Risks

- Persistence + the save/resume core are the highest-risk areas. Mitigations: the
  Save State (`save_state`/`restore_state`/`GReg`/`Glk `) path is untouched; the new
  path reuses gvm's already-tested call-stub machinery (`call_function`/`return_value`)
  and existing `CMem`/`Stks`/`MAll` builders; the shared-restore refactor is guarded by
  a "Save State bytes unchanged" regression test.
- Getting §1.8.5 wrong (resetting live iosys/stringtbl/Glk on `@restore`) would corrupt
  a mid-session restore — covered explicitly by the live-state-preservation test.
