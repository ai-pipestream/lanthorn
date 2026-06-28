# Compliant Glulx Saves — Sub-project A (Glk file layer, gvm) Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Glk **file layer** in `gvm` — real filerefs + file streams — routed through the `GlkBackend` so gvm's core stays pure and the **host** owns the filesystem. This is the substrate that `@save`/`@restore` (sub-project B) writes to. gvm-only; the CLI/app integrations land in sub-project C.

**Spec:** `docs/superpowers/specs/2026-06-27-compliant-glulx-saves-design.md` (sub-project A).

## Architecture

A **fileref** is an opaque handle the **backend** resolves to a real file. gvm holds a fileref table (`id → { usage, rock, token }`) where `token: String` is a backend-chosen identifier (a path, for the real backends). gvm never builds paths — it relays tokens. A **file stream** is buffer-backed (like a memory stream): on open it loads the file's bytes from the backend; reads/writes/seeks hit the in-memory buffer; on close (writable modes) it flushes the buffer to the backend. The interactive `create_by_prompt` **suspends** the VM (a new `StepResult::NeedFile`) so an async host can run a dialog, then resumes with the chosen token — mirroring the existing `glk_select` → `NeedLine` suspend/`supply_line` resume.

## Real interfaces (current gvm)

- `crates/gvm/src/glk.rs`: `pub trait GlkBackend` (defaulted methods + `as_any`); `enum StreamKind { Window(u32), Memory { addr, len, pos, unicode } }`; `Model { windows, streams, root, cur_stream, cur_style, ... }` with `alloc_stream(kind, rock) -> u32`, `stream_close(id) -> Option<(u32,u32)>`, `stream_position`, the put/get paths; `Model::serialize/deserialize` (the side-car model, from the prior phase). `TestBackend` lives in the tests.
- `crates/gvm/src/exec.rs`: `@glk` dispatch (the big `match selector`); the fileref group is currently stubbed at `0x0064 => …` and `0x0060 | 0x0061 | 0x0062 | 0x0063 | 0x0065 | 0x0066 | 0x0067 | 0x0068 => 0`. `enum StepResult { Continue, Quit, NeedLine { win }, NeedChar { win, unicode } }`; `suspend_result()`, the pending-input struct (`pi`), `supply_line`/`supply_char`. `glk_store_ptr(ptr, v)`. Memory-stream selectors: `0x0043` open_memory, `0x0139` open_memory_uni, `0x0044` close, `0x0045` set_position, `0x0046` get_position. `glk_stream_open_file` (`0x0042`) and `_uni` (`0x0138`) are currently unhandled (fall through).
- `crates/gvm/GLULX_NOTES.md`: document the file layer + the `NeedFile` suspend.

### Glk constants (for reference)

- `fileusage`: Data `0x00`, SavedGame `0x01`, Transcript `0x02`, InputRecord `0x03`; mode flags TextMode `0x100`, BinaryMode `0x000` (mask the usage with `0x100`/`0xff`).
- `filemode`: Write `0x01`, Read `0x02`, ReadWrite `0x03`, WriteAppend `0x05`.

## Global Constraints

- `gvm` stays zero-dep; **all** real file I/O is the host's via `GlkBackend` (gvm never touches `std::fs`). New `GlkBackend` methods are **defaulted** (None/false) so existing backends compile unchanged.
- File streams + filerefs are **transient** (not serialized into the Glk-model side-car); `Model::serialize` must skip `StreamKind::File` cleanly.
- Existing window/memory stream behavior, the Z-machine, and all current tests stay green. 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green per task.
- TDD; one commit per task on the phase's worktree branch; no push; do not edit `TODO.md`.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

## Task 1: GlkBackend file methods + fileref table + non-interactive fileref handlers

**Files:** `crates/gvm/src/glk.rs`, `crates/gvm/src/exec.rs`, `GLULX_NOTES.md`, the gvm test backend.

- [ ] **Step 1: Extend `GlkBackend`** with defaulted file methods:
  ```rust
  fn file_token_by_name(&mut self, _usage: u32, _name: &str) -> Option<String> { None }
  fn file_token_temp(&mut self, _usage: u32) -> Option<String> { None }
  fn file_exists(&self, _token: &str) -> bool { false }
  fn file_delete(&mut self, _token: &str) -> bool { false }
  fn file_read(&mut self, _token: &str) -> Option<Vec<u8>> { None }
  fn file_write(&mut self, _token: &str, _data: &[u8]) -> bool { false }
  ```
- [ ] **Step 2: Fileref table** in `Model`: `filerefs: Vec<Option<FileRef>>` with `struct FileRef { usage: u32, rock: u32, token: String }`; methods `alloc_fileref(usage, rock, token) -> u32`, `fileref(id) -> Option<&FileRef>`, `destroy_fileref(id)`, `fileref_iterate(id) -> (u32, u32)` (next id + rock, mirroring `window_iterate`/`stream_iterate`).
- [ ] **Step 3: Extend the gvm test backend** (the `TestBackend` used by exec tests) with an in-memory filesystem: a `RefCell<HashMap<String, Vec<u8>>>` (or `&mut` map); `file_token_by_name(usage, name)` returns `Some(format!("/{usage}/{name}"))`; `file_token_temp` returns a deterministic temp token; `file_read`/`file_write`/`file_exists`/`file_delete` hit the map.
- [ ] **Step 4: Failing tests** (gvm, hand-assembled `@glk` calls via the existing `glk_call` helper): with a file present in the test backend,
  - `glk_fileref_create_by_name(usage, name_addr, rock)` (`0x0061`) returns a non-zero fileref id; `glk_fileref_does_file_exist` (`0x0067`) returns 1 for it, 0 after `glk_fileref_delete_file` (`0x0066`); `glk_fileref_get_rock` (`0x0065`) returns the rock; `glk_fileref_iterate` (`0x0064`) walks the table then returns 0; `glk_fileref_destroy` (`0x0063`) frees the slot. `glk_fileref_create_temp` (`0x0060`) and `create_from_fileref` (`0x0068`) return non-zero. No diagnostics.
- [ ] **Step 5: Implement** the fileref handlers in the `@glk` dispatch, replacing the stub arm: read the C-string `name` from memory for `by_name`; call the backend; `alloc_fileref` on success (else return 0). Note `by_prompt` (`0x0062`) stays stubbed-to-0 in this task (Task 3 makes it suspend). Document the table + handlers in `GLULX_NOTES.md`.
- [ ] **Step 6: Run + commit** — `feat(gvm): glk filerefs (create/iterate/exist/delete) over a host file backend`.

---

## Task 2: File streams (open_file/_uni, read/write/seek, close→flush)

**Files:** `crates/gvm/src/glk.rs`, `crates/gvm/src/exec.rs`, `GLULX_NOTES.md`.

- [ ] **Step 1: Add `StreamKind::File`** `{ token: String, buf: Vec<u8>, pos: usize, writable: bool, unicode: bool }`. Ensure `Model::serialize` **skips** `File` streams (they are transient) and deserialization tolerates their absence.
- [ ] **Step 2: Failing tests** (gvm): a program opens a file by name, opens a **write** stream on it, `glk_put_char`/`glk_put_buffer` some bytes, closes it (the bytes now live in the test backend); then opens a **read** stream on the same fileref and `glk_get_char`/`glk_get_buffer` reads the same bytes back; `glk_stream_set_position`/`get_position` (`0x0045`/`0x0046`) seek within the buffer; WriteAppend starts at end. Read at EOF returns -1.
- [ ] **Step 3: Implement**:
  - `glk_stream_open_file(fref, fmode, rock)` (`0x0042`) and `_uni` (`0x0138`): resolve `fref.token`; `buf` = read modes → `backend.file_read(token).unwrap_or_default()`, Write → empty, WriteAppend → existing; `pos` = WriteAppend → `buf.len()`, else 0; `writable = fmode != Read`; `alloc_stream(StreamKind::File{..})`, return id.
  - Extend the stream **put** (char/buffer), **get** (char/buffer), and **set/get_position** paths to handle `File` like `Memory` (operate on `buf`/`pos`, growing `buf` on write).
  - Extend `stream_close`: for a writable `File` stream, `backend.file_write(&token, &buf)` before returning the `(read, write)` counts.
- [ ] **Step 4: Run + commit** — `feat(gvm): glk file streams (open_file, read/write/seek, flush-on-close)`.

---

## Task 3: Interactive `create_by_prompt` via a `NeedFile` suspend

**Files:** `crates/gvm/src/exec.rs`, `GLULX_NOTES.md`.

- [ ] **Step 1: Add `StepResult::NeedFile { usage: u32, fmode: u32 }`** and the pending state to carry the in-flight `@glk` store destination (so the resumed call stores the fileref id), mirroring how `glk_select`/`NeedLine` defers its store. Add `supply_file(&mut self, token: Option<String>)`: `Some` → `alloc_fileref(pending.usage, pending.rock, token)` and store that id to the pending store dest; `None` (cancel) → store 0.
- [ ] **Step 2: Failing tests** (gvm): a program calls `glk_fileref_create_by_prompt(usage, fmode, rock)` (`0x0062`) then `glk_select`/quits; driving it returns `StepResult::NeedFile { usage, fmode }`; `supply_file(Some(token))` resumes and the program sees a non-zero fileref (assert via a subsequent `does_file_exist`/store); `supply_file(None)` yields a zero fileref. Confirm `step` produces `NeedFile` via `suspend_result()`.
- [ ] **Step 3: Implement**: `0x0062` records the pending prompt (usage, fmode, rock, store dest) and signals the suspend; `suspend_result()` returns `NeedFile` when a file prompt is pending; `supply_file` completes it. Document the suspend + the `supply_file` contract in `GLULX_NOTES.md` (and that a host with no UI may call `supply_file(None)` to decline).
- [ ] **Step 4: Run + commit** — `feat(gvm): glk_fileref_create_by_prompt suspends with NeedFile for host file selection`.

---

## Self-review checklist (run before final review)

- Filerefs create/iterate/exist/delete/get_rock/destroy over the backend; `by_name`/`temp`/`from_fileref` resolve tokens; no diagnostics for the fileref group.
- File streams round-trip bytes through the backend (write→close→reopen→read); seek/append/EOF behave; `stream_close` flushes writable file streams.
- `create_by_prompt` suspends with `NeedFile`; `supply_file(Some)`/`(None)` resume to a fileref / NULL; the store lands on the original `@glk` destination.
- gvm still zero-dep (no `std::fs`); `Model::serialize` skips `File` streams; window/memory streams + the Z-machine unchanged.
- 0 warnings; full `cargo test --workspace` green; `GLULX_NOTES.md` documents the file layer + `NeedFile`.
