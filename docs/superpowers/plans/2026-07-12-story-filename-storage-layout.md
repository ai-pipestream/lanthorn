# Story-filename storage layout (SQ-0284) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Key all save/sidecar storage by the story's filename inside a flat per-game directory `<base>/<story-key>/`, add `--data-dir` to all three hosts, and drop IFID from *storage* keying (keeping it for title/hint lookup).

**Architecture:** Each host computes a sanitized `story_key` (the story basename incl. extension) and a `game_dir = base.join(story_key)`. Inside it: `default.aux`, `default.glkvfs`, `default.babelmap` (auto/singleton), plus `<slug>.babelmap` / `<slug>.qzl` (named). `--data-dir` overrides `base`; default base is the story dir (CLIs) or `~/.babelmap/saves` (app). Spec: `docs/superpowers/specs/2026-07-12-story-filename-storage-layout-design.md`.

**Tech Stack:** Rust workspace. app uses clap; both CLIs hand-roll arg parsing. No new deps needed.

## Global Constraints

- Branch `sq-0284-storage-layout` (stacked on `sq-0283-unified-saves`; already checked out). Subagent-driven, review between tasks.
- `zvm`/`gvm` VM crates stay zero-dep and **untouched** — all changes live in `zvm-cli`, `gvm-cli`, `app`.
- IFID stays for title lookup (`session::known_title`) and hints (`hints::HintIndex`); do **not** remove `compute_ifid`, `state.ifid`, or `StoryMeta.ifid`. Only *storage* stops using IFID.
- Inner auto/singleton stem is exactly `default`; `default` is a reserved save slug.
- Glulx VFS extension is `.glkvfs` everywhere (app's old `.gvfs` is renamed; bytes unchanged).
- No migration — existing `<ifid>.*` files orphan. Note it in docs.
- Commit trailers on every commit:
  `Quest: SQ-0284`
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Staging hygiene: the tree has many pre-existing untracked files (`docs/mapping-*.md`, `docs/superpowers/plans/2026-07-*.md`, `tests/`, `ui.txt`, `.superpowers/`). Stage ONLY the edited source files by path — never `git add -A`.

Shared reference — the sanitizer + resolver (each host defines its own copy; the tiny duplication mirrors the existing per-crate `sanitize_ifid`/`sanitize`):

```rust
/// Per-game directory name: the story file's basename (incl. extension),
/// sanitized to a filesystem-safe token. Empty -> "game".
fn story_key(story_path: &std::path::Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
}
```

---

### Task 1: zvm-cli — story-key aux dir + `--data-dir` + interactive `@save` resolution

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (`struct Args` 306-313, `parse_args` 315-334, aux setup ~747-748, `SaveRequest` 999-1013, `RestoreRequest` 1015-1031)
- Modify: `crates/zvm-cli/src/aux.rs` (`aux_path` 25-31)
- Test: inline `#[cfg(test)]` in `aux.rs` (path builders) and `main.rs` (resolver) — small unit tests only.

**Interfaces:**
- Produces: `story_key(&Path) -> String`; `aux::aux_path(game_dir: &Path) -> PathBuf` = `game_dir.join("default.aux")`; `resolve_save_input(input: &str, game_dir: &Path) -> PathBuf`.

- [ ] **Step 1: Failing tests for `aux_path` + `story_key` + `resolve_save_input`.**

In `crates/zvm-cli/src/aux.rs` add:
```rust
#[cfg(test)]
mod path_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn aux_path_is_default_aux_in_the_game_dir() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(aux_path(gd), PathBuf::from("/data/Zork1.z5/default.aux"));
    }
}
```
In `crates/zvm-cli/src/main.rs` (its existing `#[cfg(test)] mod tests`, or add one) add:
```rust
#[test]
fn story_key_keeps_extension_and_sanitizes() {
    use std::path::Path;
    assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
    assert_ne!(story_key(Path::new("/g/Zork1.z5")), story_key(Path::new("/g/Zork1.gblorb")));
    assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
    assert_eq!(story_key(Path::new("")), "game");
}

#[test]
fn resolve_save_input_bare_vs_path() {
    use std::path::{Path, PathBuf};
    let gd = Path::new("/data/Zork1.z5");
    assert_eq!(resolve_save_input("quick", gd), PathBuf::from("/data/Zork1.z5/quick.qzl"));
    assert_eq!(resolve_save_input("quick.qzl", gd), PathBuf::from("/data/Zork1.z5/quick.qzl"));
    assert_eq!(resolve_save_input("/tmp/foo.qzl", gd), PathBuf::from("/tmp/foo.qzl"));
}
```

- [ ] **Step 2: Run tests → FAIL** (`cargo test -p zvm-cli` — `story_key`/`resolve_save_input`/new `aux_path` signature undefined).

- [ ] **Step 3: Implement.**
  - Add `story_key` (from Global Constraints block) and `resolve_save_input` to `main.rs`:
    ```rust
    fn resolve_save_input(input: &str, game_dir: &std::path::Path) -> std::path::PathBuf {
        let t = input.trim();
        if t.contains('/') || t.contains('\\') {
            std::path::PathBuf::from(t)
        } else {
            let name = if t.ends_with(".qzl") { t.to_string() } else { format!("{t}.qzl") };
            game_dir.join(name)
        }
    }
    ```
  - `aux.rs`: change `aux_path` to `pub fn aux_path(game_dir: &Path) -> PathBuf { game_dir.join("default.aux") }` (drop the `story_path`/`ifid` params and the `sanitize_ifid` call — remove `sanitize_ifid` if it becomes unused, but only if YOUR change orphaned it).
  - `Args` (306-313): add `data_dir: Option<String>`.
  - `parse_args` (315-334): add arm `"--data-dir" => { i += 1; if i < argv.len() { a.data_dir = Some(argv[i].clone()); } }` (mirror the `--volume` value-skip pattern; guard the index).
  - aux setup (~747-748): compute
    ```rust
    let base = args.data_dir.as_deref().map(std::path::PathBuf::from)
        .unwrap_or_else(|| story_path.parent().filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from(".")));
    let game_dir = base.join(story_key(&story_path));
    let aux_file = aux::aux_path(&game_dir);
    ```
    (drop the now-unused `ifid` from the aux path; `ifid` itself stays if used elsewhere — grep; zvm-cli computes it at 747 only for aux, so it can be removed if nothing else uses it. Confirm by build.)
  - `aux_flush` (489-497): `std::fs::create_dir_all(&game_dir).ok();` before `fs::write(aux_file, …)`. (Thread `game_dir` or derive the parent via `aux_file.parent()`.)
  - `SaveRequest` (999-1013): replace the raw `fs::write(filename, …)` target with `let path = resolve_save_input(&filename, &game_dir); std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).ok(); fs::write(&path, machine.save_quetzal())`. Echo the resolved path back to the user.
  - `RestoreRequest` (1015-1031): `let path = resolve_save_input(&filename, &game_dir); fs::read(&path)`.

- [ ] **Step 4: Run tests → PASS** (`cargo test -p zvm-cli`). Then `cargo build -p zvm-cli` clean (no unused-var/import warnings from your change).

- [ ] **Step 5: Commit** (`feat(zvm-cli): key aux + interactive saves by story filename in a per-game dir; add --data-dir (SQ-0284)`), staging only `crates/zvm-cli/src/main.rs` and `crates/zvm-cli/src/aux.rs`.

---

### Task 2: gvm-cli — story-key VFS dir + `--data-dir` + interactive `@save` resolution

**Files:**
- Modify: `crates/gvm-cli/src/main.rs` (arg scan 346-358, `vfs_path` 402, VFS load 403-409, VFS write in `drive` 251-254, `SaveRequest` 300-316, `RestoreRequest` 320-338)
- Test: inline `#[cfg(test)]` in `main.rs`.

**Interfaces:**
- Produces: `story_key(&Path) -> String`; `resolve_save_input(&str, &Path) -> PathBuf` (same as Task 1); VFS path = `game_dir.join("default.glkvfs")`.

- [ ] **Step 1: Failing tests** — add to `crates/gvm-cli/src/main.rs` a `#[cfg(test)] mod tests` (if none) with the same `story_key_keeps_extension_and_sanitizes` and `resolve_save_input_bare_vs_path` tests as Task 1, plus:
```rust
#[test]
fn vfs_path_is_default_glkvfs_in_game_dir() {
    use std::path::{Path, PathBuf};
    let gd = Path::new("/data/Advent.gblorb");
    assert_eq!(gd.join("default.glkvfs"), PathBuf::from("/data/Advent.gblorb/default.glkvfs"));
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test -p gvm-cli`).

- [ ] **Step 3: Implement.**
  - Add `story_key` + `resolve_save_input` (same code as Task 1).
  - Arg scan (346-358): parse `--data-dir <path>` — after the existing `any(|a| == "--no-accel")` scans, do a positional scan: `let data_dir = { let mut d=None; let mut it=argv.iter(); while let Some(a)=it.next() { if a=="--data-dir" { d=it.next().cloned(); } } d };`.
  - Base + game_dir (before line 402):
    ```rust
    let story_path = std::path::PathBuf::from(&path);
    let base = data_dir.map(std::path::PathBuf::from)
        .unwrap_or_else(|| story_path.parent().filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from(".")));
    let game_dir = base.join(story_key(&story_path));
    let vfs_path = game_dir.join("default.glkvfs");
    ```
    (replaces `let vfs_path = PathBuf::from(format!("{path}.glkvfs"));` at 402).
  - VFS write (`drive`, 251-254): `std::fs::create_dir_all(vfs_path.parent().unwrap_or(std::path::Path::new("."))).ok();` before `fs::write(vfs_path, …)`. (`drive` already takes `vfs_path: &Path`; only construction changed — pass `&game_dir` through if needed for save resolution, see below.)
  - Thread `game_dir` into `drive` (or into the Save/Restore arms): `SaveRequest` (300-316) → `let p = resolve_save_input(&name, &game_dir); std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new("."))).ok(); fs::write(&p, machine.save_quetzal())`; `RestoreRequest` (320-338) → `let p = resolve_save_input(&name, &game_dir); fs::read(&p)`. Echo the resolved path.

- [ ] **Step 4: Run → PASS** (`cargo test -p gvm-cli`) + `cargo build -p gvm-cli` clean.

- [ ] **Step 5: Commit** (`feat(gvm-cli): key VFS + interactive saves by story filename in a per-game dir; add --data-dir (SQ-0284)`), staging only `crates/gvm-cli/src/main.rs`.

---

### Task 3: app — per-game dir storage keyed by story filename + `--data-dir`

This is the large task: swap IFID→story-key for *storage* across the app's path builders and their call sites, move into `<base>/<story-key>/`, rename `.gvfs`→`.glkvfs`, add the reserved `default` slug, and add `--data-dir`. IFID stays for titles/hints. It compiles/tests as one unit (signatures + callers move together).

**Files:**
- Modify: `crates/app/src/config.rs` (`struct Cli` 162-187, add `--data-dir`; `default_user_dir` context)
- Modify: `crates/app/src/ifid.rs` (`archive_path` 3-5) — or introduce a `storage.rs`; see Step 3.
- Modify: `crates/app/src/aux_store.rs` (`aux_path` 50-57), `crates/app/src/vfs_store.rs` (`vfs_path` 12-19, `.gvfs`→`.glkvfs`)
- Modify: `crates/app/src/persist_files.rs` (`list_saves` 30-86, `save_named` 92-121, `save_game_named` 223-231, `save_game_named_bytes` 236-244)
- Modify: `crates/app/src/main.rs` (`saves_dir` 172-173; game-session setup 1840-1910; per-turn persist 4975-5007, 2562, 2601; in-game save dispatch 4547-4561, 4190; `combined_saves`/`list_qzl` 4835-4866; picker setup 1066-1076; auto-save arc_file 5058)
- Modify: `crates/app/src/picker.rs` (`compute_ifid` use 465-466 stays; `resolve_aux` 167 `list_saves` call; badge `compute_row_badges` 566-570)
- Test: inline `#[cfg(test)]` in the touched modules.

**Interfaces:**
- Consumes: `story_key` (define an app copy in a shared spot, e.g. `crates/app/src/storage.rs` or reuse `ifid.rs`), Task-1/2 layout conventions.
- Produces: `storage::story_key(&Path) -> String`; `storage::game_dir(base: &Path, key: &str) -> PathBuf`; rewritten builders keyed by `(game_dir)` or `(base, key)`; reserved-slug guard `is_reserved_slug(&str) -> bool` (true for `"default"`).

- [ ] **Step 1: Failing tests.** Create `crates/app/src/storage.rs` with `story_key`, `game_dir`, `is_reserved_slug`, and tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    #[test] fn key_keeps_ext_and_sanitizes() {
        assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
        assert_ne!(story_key(Path::new("/g/z.z5")), story_key(Path::new("/g/z.gblorb")));
        assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
        assert_eq!(story_key(Path::new("")), "game");
    }
    #[test] fn game_dir_joins() {
        assert_eq!(game_dir(Path::new("/base"), "Zork1.z5"), PathBuf::from("/base/Zork1.z5"));
    }
    #[test] fn default_is_reserved() {
        assert!(is_reserved_slug("default"));
        assert!(!is_reserved_slug("quicksave"));
    }
}
```
Add `pub mod storage;` to `crates/app/src/lib.rs` (or wherever modules are declared — grep `mod ifid`).

- [ ] **Step 2: Run → FAIL** (`cargo test -p app storage`).

- [ ] **Step 3: Implement `storage.rs`.**
```rust
use std::path::{Path, PathBuf};
pub fn story_key(story_path: &Path) -> String { /* body from Global Constraints */ }
pub fn game_dir(base: &Path, key: &str) -> PathBuf { base.join(key) }
pub fn is_reserved_slug(slug: &str) -> bool { slug == "default" }
```
Run → PASS.

- [ ] **Step 4: Rewrite the path builders (keyed by the per-game dir).** For each, change the `<ifid>`-keyed signature to take the game dir and emit fixed inner names. Suggested signatures:
  - `ifid::archive_path(base, ifid)` → **`storage::default_state_path(game_dir) = game_dir.join("default.babelmap")`** (move/rename; update `mod`/`use`).
  - `aux_store::aux_path(save_dir, ifid)` → `aux_path(game_dir) = game_dir.join("default.aux")`.
  - `vfs_store::vfs_path(save_dir, ifid)` → `vfs_path(game_dir) = game_dir.join("default.glkvfs")` (**note `.glkvfs`**).
  - `persist_files::save_named(dir, ifid, slug, …)` → path `game_dir.join(format!("{slug}.babelmap"))`; **reject/suffix** `slug` when `storage::is_reserved_slug(slug)` (append `-1` or return an error surfaced to the UI — match how the saves UI reports name errors; if none, suffix to `default-1`).
  - `persist_files::save_game_named(dir, ifid, slug, …)` / `save_game_named_bytes(...)` → `game_dir.join(format!("{slug}.qzl"))`; same reserved-slug guard.
  - `persist_files::list_saves(dir, ifid)` → `list_saves(game_dir)`: enumerate `*.babelmap` in `game_dir`; `default.babelmap` is the default slot, `<slug>.babelmap` are named (slug = filename stem). `list_qzl` (main.rs 4842-4866) → enumerate `*.qzl` similarly (skip `default.qzl` if it ever appears; there is no default qzl).
  Add builder unit tests (default + named + reserved-slug) in the respective modules.

- [ ] **Step 5: Add `--data-dir` to `Cli`** (`config.rs` 162-187): `#[arg(long, value_name = "PATH")] pub data_dir: Option<PathBuf>,`. Resolve the storage base: `let data_base = cli.data_dir.clone().unwrap_or_else(|| saves_dir(&cfg.user_dir));` (`saves_dir` stays `<user_dir>/saves`). Thread `data_base` to both the picker (1066) and the game session (1842).

- [ ] **Step 6: Update the call sites** (main.rs + picker.rs) to compute `let key = storage::story_key(&story_path); let gdir = storage::game_dir(&data_base, &key);` once per session, create it (`create_dir_all(&gdir)`) before first write, and pass `&gdir` into every rewritten builder — replacing the `&save_dir, &ifid` argument pairs at the call sites enumerated in this task's **Files** list (main.rs setup 1840-1910; per-turn persist 4975-5007, 2562, 2601; in-game save dispatch 4547-4561, 4190; `combined_saves`/`list_qzl` 4835-4866; and the picker sites). Grep `&ifid` / `ifid.clone()` in `main.rs`+`picker.rs` and reclassify each: *storage* uses switch to `&gdir`/`key`; *title/hint/display* uses stay `ifid`. Keep `state.ifid = compute_ifid(...)` (titles/hints/display) unchanged. `arc_file` (1843/5058) becomes `storage::default_state_path(&gdir)`.

- [ ] **Step 7: Picker badge + save presence** (picker.rs 566-570, resolve_aux 167; main.rs 1066-1076): replace `save_names.starts_with(ifid)` with a per-game-dir check. Compute each row's `game_dir = storage::game_dir(&data_base, &storage::story_key(&entry.path))`; badge is true iff that dir exists and contains a `.babelmap` or `.qzl`. `resolve_aux`'s `list_saves` call passes the row's game_dir. Title/hint lookups keep using `entry.meta.ifid`. Add a test: badge true with a save present, false for empty/absent dir.

- [ ] **Step 8: Full build + tests.** `cargo build -p app --tests` clean (no warnings from your change — remove imports YOUR change orphaned, e.g. an unused `ifid` in a storage call). `cargo test -p app` — 0 failed. Verify the pre-existing Save State round-trip tests (`restore_from_file_completes_qzl_descriptor_and_resumes_babelmap_sq0163`, `save_state_is_tagged_and_round_trips_with_guard`) still pass, and that `compute_ifid`/`known_title`/hint tests are untouched.

- [ ] **Step 9: Commit** (`feat(app): key saves/sidecars by story filename in a per-game dir; add --data-dir (SQ-0284)`), staging only the edited `crates/app/src/*.rs` files.

---

### Task 4: Docs + changelog

**Files:**
- Modify: `docs/persistence.md` (storage-location section), `README.md` (storage note + `--data-dir`), `docs/features/saves.md` if it names paths.
- Modify/create: the changelog or a "storage layout changed" note.

- [ ] **Step 1:** Update `docs/persistence.md` to describe the flat per-game dir (`<base>/<story-key>/` with `default.aux`/`default.glkvfs`/`default.babelmap` + `<slug>.babelmap`/`<slug>.qzl`), the per-host default base (app `~/.babelmap/saves`, CLIs story dir), and the `--data-dir` override. State the key is the story filename (IFID retained only for title/hint lookup).
- [ ] **Step 2:** Add the `--data-dir` flag to the README usage for app + both CLIs; note the interactive `@save` bare-name → per-game-dir resolution (path-bearing values verbatim).
- [ ] **Step 3:** Add a **no-migration** note: existing `<ifid>.*` saves/sidecars orphan under the new filename-keyed layout (alpha); users re-create or manually move.
- [ ] **Step 4:** Per project convention (memory: README major-features-only) — confirm this is a major storage change worth a README line; keep per-title/bugfix noise out.
- [ ] **Step 5: Commit** (`docs: story-filename per-game storage layout + --data-dir (SQ-0284)`), staging only the edited docs.

---

## Verification (end to end)

```bash
cargo build --workspace --tests
cargo test -p zvm-cli && cargo test -p gvm-cli && cargo test -p app
```

**Manual smoke:**
- app: open a story, make a Save State + an in-game `@save quicksave`; confirm files land in `~/.babelmap/saves/<story-filename>/` as `default.babelmap` + `quicksave.qzl`. `--data-dir /tmp/bm` redirects them.
- app picker: a story with a save in its per-game dir shows the save badge; one without doesn't.
- zvm-cli `<story>`: aux writes to `<story-dir>/<story-filename>/default.aux`; `@save quick` → `<story-dir>/<story-filename>/quick.qzl`; `@save /tmp/x.qzl` → verbatim. `--data-dir /tmp/bm` redirects.
- gvm-cli `<story>`: VFS writes to `<story-dir>/<story-filename>/default.glkvfs`; same `@save` behavior + `--data-dir`.
- Reserved slug: naming a Save State `default` in the app is rejected/suffixed, not silently clobbering the auto slot.

## Notes

- Deferred: `/export` slash-command surface (SQ-0288); SQ-0285 (picker lists each save's full path) builds directly on Task 3's per-game dir.
- The zvm-cli `ZAUX` vs app `ZAX1` aux formats stay distinct (non-goal): a shared per-game dir used by both tools lands the same `default.aux` name but each ignores the other's magic (no corruption).
