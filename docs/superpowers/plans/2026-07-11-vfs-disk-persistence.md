# Cross-session disk persistence for the Glk file VFS — SQ-0278

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development, task-by-task. Steps use `- [ ]` checkboxes.

**Goal:** Make the in-memory Glk file VFS (`glk::Model.files`, added in SQ-0277) survive a full quit-and-relaunch — automatically, no explicit save — by persisting it to a per-story sidecar file, exactly the way the Z-machine's aux store already auto-persists. So e.g. Kerkerkruip's scores/preferences stick across sessions.

**Architecture:** The established split is **VM = pure bytes, host = disk I/O**. gvm gains a files-only codec + `Machine` accessors (`vfs_bytes`/`load_vfs`) + a `vfs_dirty` flag (mirroring `aux_dirty`). Each host loads the sidecar at story-open and flushes it per-turn (dirty-gated): the **app** keys it `<save_dir>/<ifid>.gvfs` (mirroring `<save_dir>/<ifid>.aux`), **gvm-cli** keys it `<story>.glkvfs` (mirroring `<story>.glksave`). This coexists with Save State (which already embeds the full VFS per-slot, SQ-0277 v4 snapshot).

**Tech Stack:** Rust. `gvm` stays zero external deps (std only). `app`/`gvm-cli` may use their existing deps.

## Global Constraints

- **`gvm` zero external deps** (std only). `app`/`gvm-cli` unchanged dep-wise.
- **Precedent to mirror precisely** (do not invent a parallel mechanism):
  - Z-machine aux store: `crates/app/src/aux_store.rs` (`encode_aux`/`decode_aux` = BE `u32 count`, per entry `u16 name_len`+name, `u32 data_len`+data; `aux_path(save_dir, ifid)` = `<save_dir>/<sanitized-ifid>.aux`; `read_global_aux`/`write_global_aux`). Lifecycle in `crates/app/src/main.rs`: load at ~1886-1891 (`set_aux_data(read_global_aux(...))`), flush per-turn dirty-gated in `persist_aux_after_turn` (~4920-4933) via `aux_dirty()`/`clear_aux_dirty()`.
  - zvm-cli: `crates/zvm-cli/src/aux.rs` + `main.rs` `aux_preload` (~472-486, called ~779) / `aux_flush` (~488-497, called ~841, dirty-gated on `machine.aux_dirty`).
- **Story identity:** `compute_ifid(&story_bytes)` (`crates/app/src/ifid.rs` → `zvm::ifid`) is deterministic for ANY bytes (Glulx included — it reads header offsets that yield a stable, if Z-nominal, string); the app already keys ALL per-story Glulx data with it. Reuse it for the app sidecar. gvm-cli has no ifid — key on the story path (`format!("{path}.glkvfs")`, mirroring the `.glksave` at `gvm-cli/src/main.rs:221`).
- **Exclude transient entries** from the sidecar: never persist VFS keys beginning with `__temp_` (Glk temp files are session-scoped). Persist everything else.
- **Sidecar codec** is files-only (name→bytes), factored from the existing inline block at `crates/gvm/src/glk.rs:2009-2014`; big-endian `u32` length prefixes, same shape as `aux_store`. A small magic+version header (`b"GVFS"` + `u32 1`) is REQUIRED so a corrupt/foreign file is rejected cleanly (return empty map, never panic) — mirror zvm-cli's `ZAUX` tolerance.
- **Coexistence, not replacement:** this does NOT change Save State (`machine.save_state()`), which already embeds the full VFS. The sidecar is the automatic, no-explicit-save layer.
- Commit trailers on EVERY commit:
  ```
  Quest: SQ-0278
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
  `git commit -F <msgfile>` (backticks in `-m` break zsh). Stage ONLY edited source/doc files by path; NEVER `git add -A` (pre-existing untracked: docs/mapping*, docs/superpowers/plans/*, tests/, ui.txt, stories/).
- Branch `vfs-disk-persistence` off `main` (create it first). Baseline: `cargo test -p gvm && cargo build -p app` pass before starting.

---

## Task 1 — gvm: files-only codec + Machine VFS accessors + `vfs_dirty`

**Files:** `crates/gvm/src/glk.rs`, `crates/gvm/src/exec.rs`.

**Produces (host-facing API):**
- `glk::encode_files(files: &BTreeMap<String,Vec<u8>>) -> Vec<u8>` and `glk::decode_files(bytes: &[u8]) -> BTreeMap<String,Vec<u8>>` (pub). Format: magic `b"GVFS"` + `u32 version=1` + `u32 count` + per entry length-prefixed name + blob (big-endian). `decode_files` is fully tolerant: wrong magic/version/truncation → empty map (never panics/errs). **`encode_files` skips keys starting with `__temp_`.**
- `Model::vfs_bytes(&self) -> Vec<u8>` = `encode_files(&self.files)`; `Model::load_vfs(&mut self, bytes: &[u8])` = `self.files = decode_files(bytes)` (merge-or-replace: REPLACE is fine — sidecar is loaded once at open before the game runs). Refactor the inline snapshot block at glk.rs:2009-2014 to call a shared internal writer so the two codecs don't drift (or leave the snapshot as-is and note it; do NOT change the snapshot wire format/version).
- `Model` gains a `vfs_dirty: bool` field (init false in `new` AND `deserialize`); set true in every VFS mutation: `stream_open_file` when it truncates/creates (Write/ReadWrite/Append), `file_stream_write`, `fileref_delete`. Methods `Model::vfs_dirty(&self) -> bool` / `Model::clear_vfs_dirty(&mut self)`.
- `Machine::vfs_bytes(&self) -> Vec<u8>`, `Machine::load_vfs(&mut self, &[u8])`, `Machine::vfs_dirty(&self) -> bool`, `Machine::clear_vfs_dirty(&mut self)` — thin delegations to `self.glk` (mirror the existing `save_state`/`style_colour` delegation style in exec.rs).

- [ ] **Step 1 (TDD):** glk.rs test `encode_files_roundtrips_and_skips_temp` — a map with `"save"`, `"data"`, `"__temp_0__"` → `decode_files(encode_files(m))` equals the map MINUS the `__temp_` key; a `decode_files` of random/short bytes and of wrong-magic bytes → empty map (no panic).
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** implement `encode_files`/`decode_files`, the `Model` accessors, the `vfs_dirty` field + setters at the three mutation sites, and the `Machine` delegations.
- [ ] **Step 4:** run → PASS.
- [ ] **Step 5:** test `vfs_dirty_tracks_mutations` — fresh Model not dirty; after `stream_open_file(Write)` dirty; `clear_vfs_dirty` clears; `file_stream_write` re-dirties; `fileref_delete` dirties. And `machine_vfs_roundtrip` at the Machine level (open+write via the Model, `vfs_bytes()`, fresh Model `load_vfs()`, file present).
- [ ] **Step 6:** `cargo test -p gvm` (all) → PASS; `cargo build -p app` compiles (new pub API only, no breakage).
- [ ] **Step 7: Commit** `feat(gvm): files-only VFS codec + Machine accessors + vfs_dirty (SQ-0278)` (stage glk.rs, exec.rs).

---

## Task 2 — gvm-cli: load/flush a `.glkvfs` sidecar

**Files:** `crates/gvm-cli/src/main.rs` (+ a small helper module if it keeps `main.rs` clean).

**Consumes:** Task 1's `Machine::vfs_bytes/load_vfs/vfs_dirty/clear_vfs_dirty`.

- [ ] **Step 1:** Compute `let vfs_path = std::path::PathBuf::from(format!("{path}.glkvfs"));` next to the existing `save_path` (~main.rs:221).
- [ ] **Step 2:** **Load before `drive`** (~after `set_acceleration`, main.rs:203): if `vfs_path` exists, `machine.load_vfs(&fs::read(vfs_path)?)` then `machine.clear_vfs_dirty()` (loading isn't a game mutation). Tolerate a missing/unreadable file silently.
- [ ] **Step 3:** **Flush dirty-gated** — mirror zvm-cli: add a `vfs_flush(&mut machine, &vfs_path)` called inside the `drive` loop each iteration (or immediately after each turn) gated on `machine.vfs_dirty()`: write `machine.vfs_bytes()`, `clear_vfs_dirty()`. If threading it into `drive` is awkward, a single post-`drive` flush (main.rs:~255, alongside `machine.flush()`) is the acceptable minimum — but prefer the dirty-gated in-loop flush for crash-safety. Report which you did.
- [ ] **Step 4:** A unit/integration test at the level gvm-cli already tests (it has tests using `drive` with canned readers, ~main.rs:517/541): drive a tiny hand-assembled Glulx program that opens a fileref by name, writes bytes, and quits; assert the `.glkvfs` file is written and, on a second `drive` with a fresh machine that `load_vfs`es it, the bytes are readable. If a full program is too heavy, at minimum test the load→flush helper functions directly against a temp path. Use a temp dir; clean up.
- [ ] **Step 5:** `cargo test -p gvm-cli` → PASS; `cargo build -p gvm-cli` clean.
- [ ] **Step 6: Commit** `feat(gvm-cli): auto-persist the Glk file VFS to a .glkvfs sidecar (SQ-0278)`.

---

## Task 3 — app: load/flush the VFS for Glulx sessions

**Files:** `crates/app/src/engine.rs` (trait), `crates/app/src/glulx_session.rs` + `crates/app/src/session.rs` (impls), `crates/app/src/main.rs` (lifecycle), and a small `crates/app/src/vfs_store.rs` (mirror `aux_store.rs`) OR reuse `aux_store`'s codec generically — prefer a tiny dedicated `vfs_store.rs` calling `gvm::glk::encode_files`/`decode_files` so the app doesn't re-implement the wire format.

**Consumes:** Task 1's `Machine` accessors.

- [ ] **Step 1:** `crates/app/src/vfs_store.rs`: `vfs_path(save_dir, ifid) -> PathBuf` = `<save_dir>/<sanitized-ifid>.gvfs` (reuse the SAME ifid sanitization as `aux_store::aux_path` — factor or copy it), `read_vfs(save_dir, ifid) -> Vec<u8>` (raw bytes, empty if absent), `write_vfs(save_dir, ifid, &[u8]) -> io::Result<()>` (`create_dir_all` + `fs::write`). The bytes ARE the gvm sidecar blob (`machine.vfs_bytes()`), so this module just does path + fs, not codec. Unit-test path derivation + round-trip through a temp dir.
- [ ] **Step 2:** Engine trait (`engine.rs`, near the `aux_data`/`set_aux_data` decls ~400-402): add `fn vfs_bytes(&self) -> Vec<u8> { Vec::new() }`, `fn load_vfs(&mut self, _bytes: &[u8]) {}`, `fn vfs_dirty(&self) -> bool { false }`, `fn clear_vfs_dirty(&mut self) {}` with default no-op bodies (so the Z-machine session inherits no-ops — Z has no Glk VFS). Implement the four on the **Glulx** session (find the Glulx `Engine` impl — likely in `glulx_session.rs`/`session.rs`) by delegating to `self.machine.vfs_bytes()` etc.
- [ ] **Step 3:** Lifecycle in `main.rs`, mirroring aux exactly:
  - **Load at story-open**: near the aux load (~1886-1891), for the Glulx engine, `session.load_vfs(&app::vfs_store::read_vfs(&save_dir, &ifid))` then `session.clear_vfs_dirty()`. (Guard on the session being Glulx, or rely on the no-op default for Z-machine — simpler: call unconditionally; Z-machine's no-op ignores it and read_vfs of an absent file is empty.)
  - **Flush per-turn**: add a `persist_vfs_after_turn` mirroring `persist_aux_after_turn` (~4920-4933), gated on `session.vfs_dirty()`: `write_vfs(save_dir, ifid, &session.vfs_bytes())` then `session.clear_vfs_dirty()`. Call it at the same turn-boundary site(s) `persist_aux_after_turn` is called.
- [ ] **Step 4:** Tests: `vfs_store` path/round-trip unit tests (Step 1). A session-level test if the harness supports it: a Glulx session that writes a VFS file reports `vfs_dirty()==true` and `vfs_bytes()` non-empty; after `load_vfs` on a fresh session the file is readable. (Follow existing `session.rs` Glulx test patterns; skip if no headless Glulx session harness exists — note it.)
- [ ] **Step 5:** `cargo test -p app` (all) → PASS; `cargo build -p app` clean.
- [ ] **Step 6: Commit** `feat(app): auto-persist the Glk file VFS per story for Glulx games (SQ-0278)` (stage engine.rs, glulx_session.rs/session.rs, vfs_store.rs, main.rs).

---

## Task 4 — Docs: a clear persistence-model page + README + notes

**Files:** `docs/persistence.md` (new — check `docs/` first for an existing saves/persistence doc to extend instead), `README.md`, and the memory `to-verify.md` (edit, don't stage).

- [ ] **Step 1:** Check `docs/` for any existing save/persistence documentation. If one exists, extend it; else create `docs/persistence.md`.
- [ ] **Step 2:** Write `docs/persistence.md` covering, clearly and concretely:
  - **The three layers**, with what each captures, when it triggers, and what survives:
    1. **The game's own save** — Z-machine `@save`/`@restore` → **Quetzal** (`.qzl`); Glulx `@save`/`@restore` → the host **Save State** blob. In-game, player-initiated.
    2. **Save State / Restore State** (host emulator snapshot) — save-anywhere full-machine snapshot; **includes the entire Glk file VFS** (SQ-0277 v4). Explicit, per-slot; archived in `.babelmap` (`game.qzl`/`game.glksave`).
    3. **Automatic per-story persistence** (no explicit save) — Z-machine **aux data** (`<save_dir>/<ifid>.aux`) and, NEW (SQ-0278), the Glulx **Glk file VFS** (`<save_dir>/<ifid>.gvfs` in the app; `<story>.glkvfs` in gvm-cli). Auto-loaded at story-open, auto-flushed per-turn. This is what makes a game's own external-storage files (e.g. Kerkerkruip scores) survive a plain quit.
  - **Per host**: the app (TUI) file locations under the user dir; gvm-cli/zvm-cli sidecars next to the story.
  - **Terminology note**: "Save State/Restore State" = host snapshot (engine-neutral, save-anywhere); "@save/@restore" = the game's in-game standard path (Quetzal on Z-machine).
  - A short table mapping {layer × engine × host → file}.
  - Note the two known SQ-0277 limitations still in effect for the VFS: `create_by_prompt` fixed-name, text-mode newline translation omitted.
- [ ] **Step 3:** README — add ONE concise line (per "README = major features only") under the capabilities/engines section: Glulx games' external file storage (Glk file streams) now auto-persists across sessions, with a link to `docs/persistence.md`. Do not enumerate selectors/paths in the README.
- [ ] **Step 4:** Update `/Users/marcuskellerman/.claude/projects/-Volumes-Videos-Source-babelmap/memory/to-verify.md`: add an SQ-0278 entry — manual smoke: run a Glulx game that writes a file (Kerkerkruip once past its menu, or any file-using game), QUIT WITHOUT saving, relaunch, confirm the data persisted (app `<save_dir>/<ifid>.gvfs`; gvm-cli `<story>.glkvfs`); confirm a `__temp_` file does NOT persist; confirm deleting the sidecar resets the game's stored data. Do NOT stage the memory file.
- [ ] **Step 5: Commit** `docs: document the save/persistence model incl. auto VFS persistence (SQ-0278)` (stage docs/persistence.md, README.md).

---

## Verification (end to end)

```bash
cargo test -p gvm && cargo test -p gvm-cli && cargo test -p app
cargo build -p app --tests && cargo build -p gvm-cli
```
Then a manual smoke (report the result): with a small Glulx program (or gvm-cli tests) that writes a file, confirm the sidecar appears on disk, and a fresh run reads it back.

## Notes / deferred
- create_by_prompt fixed-name and text-mode newline translation remain out of scope (SQ-0277 limitations; SQ-0279 covers the picker).
- The app already persists the full VFS inside Save State snapshots; this task adds only the automatic, no-explicit-save layer.
- No IFID for Glulx (gvm/blorb expose none); the app's deterministic `compute_ifid` pseudo-key is reused for keying, consistent with how the app already keys Glulx maps/covers/archives.
