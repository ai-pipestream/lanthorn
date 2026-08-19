# Z-Machine UNDO (save_undo / restore_undo) — Design

**Date:** 2026-06-25
**Status:** Draft, pending user review
**TODO:** the UNDO item — `save_undo` (EXT:0x09) / `restore_undo` (EXT:0x0A) are stubbed to fail (`exec.rs:1060-1065`).
**Sequencing:** the config field touches `crates/app/src/config.rs`, which the in-flight live-reload branch also edits — execute after live-reload merges.

## Goal

Make the game's own **UNDO** command work: implement `save_undo` and
`restore_undo` as a bounded, multi-level, in-memory undo, reusing the VM's tested
Quetzal save/restore so undo inherits correct state capture and store/resume
semantics. The depth is a config option.

## Background (current state)

- `save_undo` (EXT:0x09) and `restore_undo` (EXT:0x0A) are stubbed at
  `exec.rs:1060-1065` to store `-1` (0xFFFF), telling the game undo is
  unsupported.
- The VM already has tested Quetzal save/restore (`crates/zvm/src/quetzal.rs`):
  - `save_quetzal(&Machine) -> Vec<u8>` snapshots dynamic memory (XOR-diff vs
    `Machine.original_dynamic`, RLE-compressed), the call `frames`, the
    `eval_stack`, and the `pc`.
  - `restore_quetzal(&mut Machine, &[u8]) -> Result<(), ZError>` replaces dynamic
    memory + frames + eval stack + pc. It does **not** itself store the result
    value — it only loads state.
- **File `save`/`restore` are host-mediated:** the opcode arms return
  `StepResult::SaveRequest` / `RestoreRequest` to suspend the VM so the app does
  the file I/O, then a host callback stores the `1`/`2`/`0` result into the
  captured store target. **Undo is different** — it is entirely in-memory, so
  `save_undo`/`restore_undo` execute **inline** (`do_store` immediately, like the
  current `-1` stubs) and never suspend.
- Because the snapshot's PC points just past `save_undo`, restoring resumes there;
  the `restore_undo` arm must then store `2` into the **original `save_undo`'s**
  store target — so each snapshot must remember that target.
- `Machine { mem, state, original_dynamic, … }`; `State { pc, frames, eval_stack }`;
  `do_store(store, value)` writes the result into a decoded store target.
- The app (`crates/app`) creates the VM via `GameSession::new` → `Machine::new`.
  Config lives in `crates/app/src/config.rs`.

## Design

### 1. Undo store on the Machine

Add to `Machine`:

- `undo_stack: Vec<UndoSnapshot>` — newest on top, where
  `UndoSnapshot { blob: Vec<u8>, store: <store-target type> }` pairs the
  `save_quetzal` bytes with the **`save_undo` instruction's own store target**
  (so `restore_undo` can write `2` back into it).
- `undo_cap: usize` — maximum retained snapshots (default 16). Session-only;
  never written into `.lanthorn` saves.

`Machine::new` initializes `undo_stack` empty and `undo_cap` to a default (16).

### 2. Opcode semantics (inline; in-memory)

Both arms execute **inline** (no `SaveRequest`/`RestoreRequest` suspend) and use
`do_store` directly, replacing the current `do_store(store, 0xFFFF)` stubs.

- **`save_undo` (EXT:0x09):** (`store` = this instruction's store target)
  - `undo_cap == 0` → `do_store(store, -1)` (0xFFFF; undo disabled — the game can
    tell the player). Push nothing.
  - else: push `UndoSnapshot { blob: self.save_quetzal(), store }`; if
    `len > undo_cap`, drop the oldest (front); `do_store(store, 1)`.
- **`restore_undo` (EXT:0x0A):** (`store` = this instruction's store target)
  - `undo_stack` empty → `do_store(store, 0)` (nothing to undo), state unchanged.
  - else: pop the newest `UndoSnapshot { blob, store: save_store }`;
    `self.restore_quetzal(&blob)` (loads memory + frames + eval stack + pc back to
    the `save_undo` point), then `do_store(save_store, 2)` — i.e. the original
    `save_undo` "returns" `2", and execution continues from the restored PC. The
    popped snapshot is consumed, so repeated `restore_undo` walks back successive
    turns.
  - If `restore_quetzal` returns `Err` (should not happen for our own blobs),
    `do_store(store, 0)` and leave state unchanged.

This matches §15 of the Z-machine standard. Snapshotting captures state at the
post-`save_undo` PC (the standard `step()` advance contract), so restoring resumes
correctly; storing `2` into `save_store` is what the game observes as the undo.

### 3. Configurable depth

- `crates/app/src/config.rs`: add `undo_levels: usize` (default **16**;
  `#[serde(default = "default_undo_levels")]`). `0` disables undo.
- The app sets the VM cap from config at session creation: after
  `GameSession::new`, `session.machine.undo_cap = config.undo_levels`
  (`undo_cap` is a public field, or via a `set_undo_cap` setter — implementer's
  choice). So `undo_levels = 0` makes `save_undo` report unsupported.

### 4. App / map interaction

No app changes beyond wiring the cap. The game's `UNDO` command drives the
opcodes; the app renders the resulting turn like any other. The map's
**current-room highlight follows the undo** (it is derived from the VM location
each turn), while already-discovered rooms remain drawn — a sensible superset.
No lanthorn-level undo hotkey (the game's command is the interface).

## Architecture / components

- `crates/zvm/src/cpu/exec.rs`:
  - A small `UndoSnapshot { blob: Vec<u8>, store: <store-target type> }` (the
    `store` field uses the same type the EXT arm's `store` binding has, so it can
    be replayed via `do_store`).
  - `Machine` gains `undo_stack: Vec<UndoSnapshot>` and `undo_cap: usize`
    (+ defaults in `Machine::new`: empty stack, cap 16).
  - Replace the `save_undo` / `restore_undo` stub arms (~1060-1065) with the §2
    inline logic, using `self.save_quetzal()` / `self.restore_quetzal()` and
    `self.do_store(...)`. No `SaveRequest`/`RestoreRequest` suspension.
- `crates/app/src/config.rs`: `undo_levels` field + `default_undo_levels()` (16)
  + the file-merge copy (`cfg.undo_levels = from_file.undo_levels`).
- `crates/app/src/main.rs` (or `session.rs`): set `session.machine.undo_cap`
  from `config.undo_levels` after the session is created (and for any session
  re-creation paths — game reset, restore).
- `style.example.toml` is unaffected; document `undo_levels` in the config docs
  (README config section), not the style reference.

## Error handling

- `undo_cap == 0` → `save_undo` stores `-1` (disabled); `restore_undo` finds an
  empty stack → stores `0`.
- `restore_undo` on an empty stack → store `0` (no state change).
- `restore_quetzal` error on our own blob → store `0`, state unchanged (defensive;
  not expected).
- Cap overflow → oldest snapshot dropped (FIFO past the cap).

## Testing

- `save_undo` stores `1` (cap > 0); after `save_undo` → mutate dynamic memory →
  `restore_undo`, the memory + PC + stack revert to the snapshot.
- Multi-level: two `save_undo`s with different memory states, then two
  `restore_undo`s, walk back through both in LIFO order.
- `restore_undo` on an empty stack stores `0` and changes nothing.
- `undo_cap == 0`: `save_undo` stores `-1` (0xFFFF) and pushes nothing.
- Cap drop: with `undo_cap = N`, pushing `N+1` keeps the newest `N` (oldest
  dropped); a `restore_undo` then lands on the correct (newest) snapshot.
- Store value plumbing: `save_undo` stores `1` into its own target; on
  `restore_undo`, the value `2` lands in the **original `save_undo`'s** target
  (the resumed instruction "returns" 2), while an empty-stack `restore_undo` and a
  `cap == 0` `save_undo` store `0` / `-1` into their own targets. A focused test:
  `save_undo` storing to global G, then `restore_undo`, asserts G == 2 and the PC
  resumed at the post-`save_undo` address.
- Config: `undo_levels` defaults to 16; `0` parses; the file-merge carries it.
- Round-trip sanity: a `save_undo` snapshot taken and restored leaves the machine
  byte-identical in dynamic memory and stack to before the intervening mutation
  (reuses the Quetzal round-trip already covered by save/restore tests).

## Out of scope (deferred)

- Persisting undo history into `.lanthorn` saves (session-only).
- A lanthorn-level undo hotkey / app-driven undo (the game's `UNDO` is the path).
- Reverting the accumulated map on undo (the current-room highlight follows; rooms
  remain a discovered superset).
- `save_undo`/`restore_undo` for pre-v5 story files that lack the opcodes (these
  are EXT opcodes; only v5+ stories invoke them).
