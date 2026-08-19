# Glk file streams (in-memory VFS) — SQ-0277

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development to execute task-by-task. Steps use `- [ ]` checkboxes.

**Goal:** Implement Glk filerefs + file streams in `gvm`, backed by an in-memory virtual filesystem, so Glulx games that need external file storage (e.g. Kerkerkruip's `KerkerkruipStorage`) run instead of hitting a `P48` error.

**Architecture:** A name→bytes map (`files`) plus a fileref table live on `glk::Model`. File streams are a new `StreamKind::File` variant whose mutable read/write state lives in a `file_streams` side table keyed by stream id (so `StreamKind` stays `Copy`). Reads/writes go straight against `files[name]`. The whole VFS round-trips through the Glk save snapshot (version bump 3→4). No real disk I/O.

**Tech Stack:** Rust, `gvm` crate. `std` is allowed (`std::collections::BTreeMap`, `String`), but **NO external crate dependencies** — `gvm` stays zero-dep.

## Global Constraints

- **Zero external deps** in `gvm`. `std` only. No new `Cargo.toml` entries.
- Only `crates/gvm/src/glk.rs` and `crates/gvm/src/exec.rs` change (plus, in Task 4, docs + a smoke test file). `zvm`, `app`, `blorb` untouched.
- **Glk constants** (verified against garglk `cheapglk/glk.h`, use verbatim):
  - filemode: `Write=0x01 Read=0x02 ReadWrite=0x03 WriteAppend=0x05`
  - fileusage: `Data=0x00 SavedGame=0x01 Transcript=0x02 InputRecord=0x03 TypeMask=0x0f TextMode=0x100 BinaryMode=0x000`
  - seekmode: `Start=0 Current=1 End=2`
  - Glk selectors: fileref `create_temp=0x60 create_by_name=0x61 create_by_prompt=0x62 destroy=0x63 iterate=0x64 get_rock=0x65 delete_file=0x66 does_file_exist=0x67 create_from_fileref=0x68`; streams `open_file=0x42 open_file_uni=0x138 stream_close=0x44 set_position=0x45 get_position=0x46`; reads `get_char=0x90 get_line=0x91 get_buffer=0x92 get_char_uni=0x12C get_buffer_uni=0x12D get_line_uni=0x12E`.
- **Encoding rules:** byte stream = 1 byte per char (`ch & 0xFF`); uni stream (`_uni` open) = 4-byte **big-endian** per char in binary mode, UTF-8 in text mode (text = `usage & fileusage_TextMode != 0`). Text-mode **newline translation is intentionally omitted** (out of scope).
- **Open modes:** `Write` truncates (`files.insert(name, vec![])`, pos=0); `Read` FAILS if the file is absent (return NULL stream), else pos=0; `ReadWrite` creates-if-absent, pos=0; `WriteAppend` creates-if-absent, pos=len.
- **create_by_prompt** cannot show a TUI picker → degrade to a fixed default name per usage type: `format!("__prompt_{}__", usage & fileusage_TypeMask)`. Note as a known limitation.
- Commit trailers on EVERY commit:
  ```
  Quest: SQ-0277
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
  Commit with `git commit -F <msgfile>` (backticks in `-m` break zsh). Stage ONLY the edited source files by path — the tree has pre-existing untracked files (`docs/`, `tests/`, `ui.txt`, `stories/*`); NEVER `git add -A`.
- Branch `gvm-file-streams` (already created off `main`, HEAD 793d40b). Verify baseline before starting: `cargo test -p gvm` passes.

---

## Task 1 — VFS + fileref model + fileref selectors

**Files:** Modify `crates/gvm/src/glk.rs` (Model state + fileref methods), `crates/gvm/src/exec.rs` (fileref dispatch arms + tests).

**Produces (interfaces later tasks rely on):**
- `glk::Model` gains private fields: `files: std::collections::BTreeMap<String, Vec<u8>>`, `filerefs: Vec<Option<FileRef>>`, and (declared now, used in Task 2) `file_streams: std::collections::BTreeMap<u32, FileStream>`. Initialize all three empty in `Model::new` AND in `deserialize`.
- `struct FileRef { id: u32, rock: u32, name: String, usage: u32 }` (private).
- `struct FileStream { name: String, mode: u32, pos: usize, unicode: bool, usage: u32 }` (private; declare the struct now, it's populated in Task 2).
- Public Model methods (used by exec.rs):
  - `fileref_create(&mut self, usage: u32, name: String, rock: u32) -> u32` — sanitize `name` via `Self::sanitize_fileref_name`, alloc a fileref slot (mirror `alloc_stream`'s id = len+1 pattern), return id.
  - `fileref_create_temp(&mut self, usage: u32, rock: u32) -> u32` — synth a unique name `format!("__temp_{}__", <counter>)`; use `self.filerefs.len()` or a dedicated counter for uniqueness.
  - `fileref_create_by_prompt(&mut self, usage: u32, _fmode: u32, rock: u32) -> u32` — name = `format!("__prompt_{}__", usage & 0x0f)`.
  - `fileref_create_from(&mut self, usage: u32, oldfref: u32, rock: u32) -> u32` — clone the old fileref's `name`, new usage/rock; 0 if old invalid.
  - `fileref_destroy(&mut self, fref: u32)` — free the slot (does NOT delete the file).
  - `fileref_rock(&self, fref: u32) -> u32`.
  - `fileref_iterate(&self, fref: u32) -> (u32 next_id, u32 rock)` — next live fileref id after `fref` (or first if `fref==0`), with its rock; `(0,0)` at end. Mirror the existing window/stream iterate pattern.
  - `fileref_exists(&self, fref: u32) -> bool` — `self.filerefs.get(fref-1)` live AND `self.files.contains_key(name)`.
  - `fileref_delete(&mut self, fref: u32)` — `files.remove(name)`.
  - `fileref_name(&self, fref: u32) -> Option<(String, u32 usage)>` — for Task 2's open. (name clone + usage)
  - `Self::sanitize_fileref_name(raw: &str) -> String` — keep ASCII alphanumerics, `-`, `_`, `.`; map others to `_`; non-empty fallback `"file"`. (Glk lets the library filter the base name.)

- [ ] **Step 1: Failing test** in glk.rs test module (`mod tests`, uses `super::*`): `fileref_create_then_exists_after_write` — create a fileref by name, assert `!fileref_exists`; manually `self.files.insert(name, vec![1,2,3])` won't be reachable (files private) — instead test via the exists/delete cycle using a helper you expose, OR test create + rock + iterate + destroy semantics and defer exists-after-write to the exec.rs end-to-end test in Task 2. Concretely test: create two filerefs, `fileref_iterate` walks both then returns `(0,0)`; `fileref_rock` returns the set rock; after `fileref_destroy` the id is gone from iteration; `sanitize_fileref_name("a/b*c.sav")=="a_b_c.sav"`.
- [ ] **Step 2:** run `cargo test -p gvm fileref` → FAIL (methods missing).
- [ ] **Step 3:** Add the fields, structs, and methods above to glk.rs. Match the existing `alloc_stream`/`win`/`stream` slot-accessor idioms (id = index+1, `Vec<Option<_>>`).
- [ ] **Step 4:** run `cargo test -p gvm fileref` → PASS.
- [ ] **Step 5: exec.rs dispatch.** Replace the collapsed fileref arm (currently `0x0060 | 0x0061 | ... | 0x0068 => 0,` near exec.rs:2900, and the dedicated `0x0064` iterate arm) with real per-selector arms:
  - `0x60` → `self.glk.fileref_create_temp(a(0), a(1))`
  - `0x61` → read a C string from Glulx memory at `a(1)` (find the existing helper the code uses to read a byte string from memory — reuse it; it is the same one `glk_fileref`/`glk_put_string` C-string paths use), then `self.glk.fileref_create(a(0), name, a(2))`
  - `0x62` → `self.glk.fileref_create_by_prompt(a(0), a(1), a(2))`
  - `0x63` → `{ self.glk.fileref_destroy(a(0)); 0 }`
  - `0x64` → `{ let (next, rock) = self.glk.fileref_iterate(a(0)); self.glk_store_ptr(a(1), rock)?; next }`
  - `0x65` → `self.glk.fileref_rock(a(0))`
  - `0x66` → `{ self.glk.fileref_delete(a(0)); 0 }`
  - `0x67` → `self.glk.fileref_exists(a(0)) as u32`
  - `0x68` → `self.glk.fileref_create_from(a(0), a(1), a(2))`
- [ ] **Step 6: Update the two now-obsolete tests** at exec.rs `glk_fileref_group_degrades_silently` (~6019) and `glk_fileref_iterate_is_empty_and_silent` (~6003): `create_by_name` now returns a NON-zero fileref; `does_file_exist` on a never-written fileref is still false; iterate over one created fileref now yields it then 0. Rewrite the assertions to match the real behavior (keep the "no diagnostics" assertions). Add `glk_fileref_create_by_name_returns_live_ref`.
- [ ] **Step 7:** `cargo test -p gvm` → PASS (all, including updated tests).
- [ ] **Step 8: Commit** (`feat(gvm): Glk filerefs backed by an in-memory VFS (SQ-0277)`), staging only `crates/gvm/src/glk.rs crates/gvm/src/exec.rs`.

---

## Task 2 — File streams: open, read, write, seek, close

**Files:** Modify `crates/gvm/src/glk.rs` (StreamKind + file-stream methods), `crates/gvm/src/exec.rs` (open arms + File arms in put/read/position/close + tests).

**Consumes:** Task 1's `files`, `file_streams`, `FileStream`, `fileref_name`.

**Produces:**
- `StreamKind::File` unit-or-`{unicode: bool}` variant (KEEP `StreamKind: Copy` — do NOT put `String` in it). Put mutable state in `file_streams` keyed by stream id.
- `Model::stream_open_file(&mut self, fref: u32, fmode: u32, unicode: bool, rock: u32) -> u32` — resolve `fileref_name(fref)`; apply the open-mode rules (Global Constraints); on Read-missing return `0`. Else `alloc_stream(StreamKind::File{..})`, insert `file_streams[id] = FileStream{ name, mode: fmode, pos, unicode, usage }`, return id.
- File handling inside the existing stream ops:
  - **write** (`glk_stream_put`, exec.rs ~2164 match): new `StreamKind::File` arm → append/overwrite bytes at `pos` in `files[name]` per encoding rules, advance `pos` and `write_count`. Add a `Model` helper `file_stream_write(&mut self, sid, s: &str)` so exec.rs stays thin.
  - **read** (the 6 arms exec.rs:2648-2755): each currently early-returns on `memory_stream_read_info`→None. Add File support via new `Model` helpers, e.g. `file_stream_read_char(&mut self, sid) -> Option<u32>` (None=EOF, else char/byte; -1 mapping handled in exec.rs like the memory path), `file_stream_read_buffer`, `file_stream_read_line`. Mirror the memory-stream helpers' shapes (`memory_stream_read_info`/`_advance`, glk.rs:1397-1413) so the exec.rs arms branch: memory path OR file path OR EOF.
  - **position** (`stream_position` glk.rs:1356 + `stream_set_position` glk.rs:1364): add `StreamKind::File` arms — get returns `file_streams[sid].pos`; set applies seekmode (`Start`/`Current`/`End`) clamped to `[0, files[name].len()]`.
  - **close** (`stream_close` glk.rs:1323): for a File stream, remove `file_streams[sid]` and free the slot; return `(read_count, write_count)` like other kinds.

- [ ] **Step 1: Failing test** exec.rs (end-to-end via `@glk` opcodes, mirror `glk_memory_stream_*`): `glk_file_stream_write_then_read_roundtrips` — `create_by_name("save")` → `stream_open_file(fref, Write, rock)` → `put_char_stream`/`put_buffer` a few bytes → `stream_close` → `does_file_exist`==1 → `stream_open_file(fref, Read)` → `get_char_stream` returns the same bytes then EOF (-1). Use the `glk_call` helper + `run_with_ram`/`poke`.
- [ ] **Step 2:** `cargo test -p gvm glk_file_stream` → FAIL.
- [ ] **Step 3:** Implement StreamKind::File, the open arms (`0x42`→byte, `0x138`→uni), and all File arms/helpers above.
- [ ] **Step 4:** `cargo test -p gvm glk_file_stream` → PASS.
- [ ] **Step 5: More tests** (glk.rs Model-level): Write truncates an existing file; WriteAppend preserves + seeks end; Read of a missing file returns 0 (no stream allocated); seek Start/Current/End land at the right pos; a `_uni` binary stream round-trips a 4-byte-BE char; close syncs and frees the slot. Also assert exhaustiveness didn't regress: `cargo build -p gvm` has no non-exhaustive-match warnings.
- [ ] **Step 6:** `cargo test -p gvm` → PASS (full).
- [ ] **Step 7: Commit** (`feat(gvm): Glk file streams over the VFS — open/read/write/seek/close (SQ-0277)`), staging only the two source files.

---

## Task 3 — Snapshot round-trip (version bump 3→4)

**Files:** Modify `crates/gvm/src/glk.rs` (`serialize`/`deserialize`, `GLK_SNAPSHOT_VERSION`), add a test.

**Consumes:** Tasks 1–2 state.

- [ ] **Step 1: Failing test** glk.rs: `vfs_and_file_stream_survive_snapshot_round_trip` — build a Model, create a fileref, open+write a file stream, leave it open at a known pos, then `Model::deserialize(&m.serialize())` and assert: the file bytes are intact, the fileref still exists/iterates, and the reopened File stream reports the same `pos` and reads the same next byte. (If reproducing an *open* stream's position across restore is awkward, persist `file_streams` too — see Step 3.)
- [ ] **Step 2:** `cargo test -p gvm snapshot_round_trip` (the new test) → FAIL.
- [ ] **Step 3:** Bump `GLK_SNAPSHOT_VERSION` 3→4 (glk.rs:1776). In `serialize`: after the existing stream loop, write the `files` map (count, then per entry: name len + bytes + blob len + bytes) and the `filerefs` table (count + per live slot: id, rock, usage, name) and `file_streams` (count + per entry: sid, name, mode, pos, unicode, usage). Add stream-kind **tag `2`** for `StreamKind::File` in the per-stream serialize match. In `deserialize`: read them all back, reconstruct the maps/tables, and accept tag `2`. Follow the existing `w(..)`/`r.u32()?` helpers; for `Vec<u8>`/`String` add length-prefixed read/write (check whether a string/bytes helper already exists near serialize; if not, write raw: len then bytes).
- [ ] **Step 4:** `cargo test -p gvm` → PASS (full). Confirm the version bump doesn't break other snapshot tests (they build a fresh Model each time; a stale on-disk save is a clean `BadSave`, which is expected and acceptable — note it for the final review).
- [ ] **Step 5: Commit** (`feat(gvm): persist the file VFS in the Glk save snapshot, v3→v4 (SQ-0277)`), staging only glk.rs.

---

## Task 4 — Real-game smoke (Kerkerkruip) + docs

**Files:** Add `crates/gvm/tests/kerkerkruip_boots.rs` (skip-if-absent integration test), update `README.md` if warranted, update the memory to-verify note.

**Consumes:** Tasks 1–3.

- [ ] **Step 1:** Write `crates/gvm/tests/kerkerkruip_boots.rs`: if `stories/Kerkerkruip.gblorb` is absent, print a skip note and return (mirror `crates/app/tests/wizard_sniffer.rs`'s skip-if-absent pattern). Else load it via the gvm public API used by other integration tests (check `crates/gvm/tests/*.rs` for the loader), drive it headlessly past the screen-reader prompt + main menu (feed the New-game/SPACE or `n` then a key), stepping with a bounded budget, and assert the machine reaches a **line-input request** (i.e. it got past the `P48` menu loop) WITHOUT quitting. Keep the step budget tight.
- [ ] **Step 2:** `cargo test -p gvm --test kerkerkruip_boots` → PASS (or SKIP if the story is absent — it is gitignored, so CI skips; locally it runs).
- [ ] **Step 3:** Manually confirm (report the transcript tail): Kerkerkruip now shows a room/gameplay instead of re-drawing the menu with `*** Error on file 'KerkerkruipStorage' ***`.
- [ ] **Step 4:** README — per the "README = major features only" rule, add a single concise line under the relevant capabilities section noting Glulx file-storage (Glk file streams) support IF the README enumerates Glk capabilities; otherwise skip and note that in the report. Do not document individual selectors.
- [ ] **Step 5:** Update `/Users/marcuskellerman/.claude/projects/-Volumes-Videos-Source-lanthorn/memory/to-verify.md`: add an SQ-0277 entry (Kerkerkruip boots to gameplay; general file-using games no longer `P48`; Save State round-trips the VFS) and note that SQ-0273's garglk smoke is now finally runnable in Kerkerkruip combat.
- [ ] **Step 6: Commit** (`test(gvm): Kerkerkruip boots past its storage menu + docs (SQ-0277)`), staging the test file + README (if changed). The memory file is outside the repo — do not stage it.

---

## Verification (end to end)

```bash
cargo test -p gvm                 # all unit + integration
cargo build -p app --tests        # StreamKind change must not break the app backend's exhaustive matches
cargo test -p app                 # no regressions
```

Then the real payoff: rebuild `gvm-cli`, drive `stories/Kerkerkruip.gblorb` into combat, and (bonus) confirm SQ-0273's `garglk_set_zcolors` finally fires on the coloured monster names.

## Notes / deferred
- create_by_prompt uses a fixed per-usage name (no TUI picker) — a real file-chooser is a later app-layer follow-up.
- Text-mode newline translation omitted.
- Real cross-session disk persistence (vs snapshot-only) is the deferred alternative from the SQ-0277 design (host-backend route).
- The `StreamKind` enum change touches every exhaustive `match` over it — the app backend (`crates/app/src/glk_backend.rs`) and any other consumer must compile; the final review must confirm `cargo build -p app` is clean.
