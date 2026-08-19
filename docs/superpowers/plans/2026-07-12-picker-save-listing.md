# Story-picker save listing (SQ-0285) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Show a selected story's save files and their on-disk location in the picker info panel — `.lanthorn` Save States, `.qzl` in-game saves, and the `default.aux`/`default.glkvfs` sidecars, under a per-game-dir header.

**Architecture:** Extend the lazily-resolved `StoryAux` (`picker.rs::resolve_aux`) with the game dir, the `.qzl` saves, and which sidecars exist; extend the panel's existing Saves section (`main.rs::draw_info_panel`) to render the dir header, filenames, `.qzl` rows, and a `Sidecars:` line. Reuses SQ-0284's per-game dir. Spec: `docs/superpowers/specs/2026-07-12-picker-save-listing-design.md`.

**Tech Stack:** Rust, `app` crate only.

## Global Constraints

- Branch `sq-0284-storage-layout` (stacked; SQ-0285 continues on it). Subagent-driven.
- Only the `app` crate changes. `zvm`/`gvm` untouched. Display-only — no new actions, no storage/format change.
- Reuse SQ-0284's `storage::{story_key, game_dir}`; do not re-key anything.
- Commit trailers on every commit:
  `Quest: SQ-0285`
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Staging hygiene: stage ONLY the edited `crates/app/src/*.rs` files by path — never `git add -A` (the tree has many pre-existing untracked files).

---

### Task 1: Picker save listing — data + render (app)

One atomic task (the render depends on the new data; compiles/tests as a unit).

**Files:**
- Modify: `crates/app/src/persist_files.rs` (add `QzlInfo` + `list_qzl`)
- Modify: `crates/app/src/main.rs` (the current `list_qzl` here — generalize/move to `persist_files`; extend `draw_info_panel` Saves section ~1591-1601; extend the `info_panel_renders_*` test ~7199)
- Modify: `crates/app/src/picker.rs` (`StoryAux` struct ~145; `resolve_aux` ~157)

**Interfaces:**
- Consumes: `storage::{story_key, game_dir}`, `persist_files::{SaveInfo, list_saves}` (SQ-0284).
- Produces: `persist_files::list_qzl(game_dir: &Path) -> Vec<SaveInfo>` (MOVED verbatim from `main.rs:4850`, now sorted newest-first before returning); `StoryAux { …, game_dir: PathBuf, qzl_saves: Vec<SaveInfo>, sidecars: Vec<&'static str> }`. **Reuse the existing `SaveInfo`** — a `.qzl` row is already `turns: 0`, `is_default: false`, `saved_at = persist_files::rfc3339_mtime(&p)`. The panel tells `.qzl` from `.lanthorn` by the file **extension**, not by a new type. Do NOT add a `QzlInfo` type (`combined_saves` merges `Vec<SaveInfo>`).

- [ ] **Step 1: Failing test for `list_qzl`.** In `persist_files.rs` tests (mirror the temp-dir pattern the SQ-0284 `list_saves` tests in this file already use — reuse whatever helper/crate they use):
```rust
#[test]
fn list_qzl_lists_qzl_by_stem_newest_first_and_skips_lanthorn() {
    let dir = /* temp dir, same pattern as list_saves tests */;
    std::fs::write(dir.join("default.lanthorn"), b"x").unwrap();
    std::fs::write(dir.join("quick.qzl"), b"x").unwrap();
    std::fs::write(dir.join("older.qzl"), b"x").unwrap();
    let out = list_qzl(&dir);
    assert_eq!(out.len(), 2);                              // .lanthorn excluded
    assert!(out.iter().all(|q| q.path.extension().unwrap() == "qzl"));
    assert!(out.iter().all(|q| !q.is_default && q.turns == 0));
    assert!(out.iter().any(|q| q.name == "quick"));        // name = stem
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test -p app list_qzl` — `persist_files::list_qzl` doesn't exist yet).

- [ ] **Step 3: Move `list_qzl` into `persist_files.rs`.** Cut the existing `fn list_qzl(game_dir) -> Vec<SaveInfo>` from `main.rs:4850` (and its doc comment) into `persist_files.rs` as `pub fn list_qzl`, VERBATIM except: (a) drop the `app::persist_files::` path prefixes now that it's in-module (`SaveInfo`, `rfc3339_mtime` are local), and (b) sort newest-first before returning: `out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));`. Then update `main.rs::combined_saves` (line 4840) to call `app::persist_files::list_qzl(game_dir)` and any other `list_qzl` caller in `main.rs` likewise. Keep `combined_saves`'s own final sort. Run → PASS. Also confirm the pre-existing `list_qzl_lists_game_saves_in_game_dir_and_skips_lanthorn` test (main.rs:5785) still passes — move/retarget it to `persist_files` if it referenced the private fn.

- [ ] **Step 4: Extend `StoryAux` + `resolve_aux` (`picker.rs`).** Add fields `game_dir: PathBuf`, `qzl_saves: Vec<crate::persist_files::SaveInfo>`, `sidecars: Vec<&'static str>`. In `resolve_aux`, after computing `game_dir` (already at ~line 168):
```rust
let qzl_saves = crate::persist_files::list_qzl(&game_dir);
let mut sidecars = Vec::new();
if game_dir.join("default.aux").exists() { sidecars.push("default.aux"); }
if game_dir.join("default.glkvfs").exists() { sidecars.push("default.glkvfs"); }
StoryAux { assoc_blorb, saves, hints_available, game_dir, qzl_saves, sidecars }
```
Add a `resolve_aux` unit test: temp game dir with `default.lanthorn` + `quick.qzl` + `default.aux` → `StoryAux` has the right `game_dir`, 1 save, 1 qzl_save, `sidecars == ["default.aux"]`. Run → PASS.

- [ ] **Step 5: Extend the panel Saves section (`main.rs::draw_info_panel` ~1591-1601).** Replace the current block with:
```rust
// Saves + sidecars (SQ-0285).
if let Some(a) = aux {
    let has_any = !a.saves.is_empty() || !a.qzl_saves.is_empty() || !a.sidecars.is_empty();
    if has_any {
        lines.push((String::new(), cs.story_info_value));
        // Header: "Saves · <dir>" with $HOME abbreviated to ~.
        let dir = abbreviate_home(&a.game_dir);
        lines.push((format!("Saves · {dir}"), cs.story_info_label));
        for s in &a.saves {
            let when = s.saved_at.get(0..10).unwrap_or(&s.saved_at);
            let fname = s.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            lines.push((format!(" {}  turn {} · {}  {}", s.name, s.turns, when, fname), cs.story_info_value));
        }
        for q in &a.qzl_saves {
            let when = q.saved_at.get(0..10).unwrap_or(&q.saved_at);
            let fname = q.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            lines.push((format!(" {}  {}  {}", q.name, when, fname), cs.story_info_value));
        }
        if !a.sidecars.is_empty() {
            lines.push((format!("Sidecars: {}", a.sidecars.join(" · ")), cs.story_info_value));
        }
    }
}
```
Add a small helper near `human_size`:
```rust
/// Abbreviate a leading $HOME in a path to `~` for display.
fn abbreviate_home(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = s.strip_prefix(&home) { return format!("~{rest}"); }
        }
    }
    s
}
```

- [ ] **Step 6: Extend the render test** `info_panel_renders_metadata_features_and_resources` (main.rs ~7199) — rename if appropriate (e.g. `..._and_saves`). Build a `StoryAux` with one `.lanthorn` `SaveInfo`, one `QzlInfo`, and `sidecars = vec!["default.aux"]`; render into a buffer; assert the buffer text contains `"Saves ·"`, the `.lanthorn` filename, the `.qzl` filename, and `"Sidecars:"`. (Grep how the existing test constructs `StoryAux` and reads the buffer; follow that pattern.)

- [ ] **Step 7: Build + test.** `cargo build -p app --tests` warning-clean (fix anything the moved `list_qzl` orphaned in `main.rs`). `cargo test -p app` — 0 failed.

- [ ] **Step 8: Commit** (`feat(app): show a story's saves, in-game saves, and sidecars with their location in the picker (SQ-0285)`), staging only the edited `crates/app/src/*.rs` files.

---

## Verification

```bash
cargo build -p app --tests
cargo test -p app        # list_qzl, resolve_aux, panel render — 0 failed
```

**Manual smoke:** open the picker, highlight a story that has saves; the info panel Saves section shows `Saves · <dir>`, each `.lanthorn` and `.qzl` file by name + filename, and a `Sidecars:` line for `default.aux`/`default.glkvfs`. A story with no saves shows no Saves section. `--data-dir` is reflected (the header points at the configured base).

## Notes

- Display choice (per spec, confirm at review): dir-as-header + filenames, not a full absolute path per row.
- One function move (`main.rs::list_qzl` → `persist_files::list_qzl`) unifies the saves-manager and picker on one implementation.
