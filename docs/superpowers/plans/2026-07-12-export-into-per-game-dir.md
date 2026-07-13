# `/export` into the per-game dir (SQ-0288) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** All four export commands (`export-svg`/`export-dot`/`export-dump`/`export-transcript`) write into the story's per-game dir `<base>/<story-key>/` with fixed default filenames (`map.svg`/`map.dot`/`map.txt`/`transcript.txt`, overwrite) and an optional SQ-0284-style path argument.

**Architecture:** Add a shared `export::resolve_export_path` (mirrors SQ-0284's `resolve_save_input`); refactor `export_transcript` onto it; give the three map commands an optional path arg; thread the already-computed `game_dir` into the export handlers. Pure renderers unchanged. Spec: `docs/superpowers/specs/2026-07-12-export-into-per-game-dir-design.md`.

**Tech Stack:** Rust, `app` crate only.

## Global Constraints

- Branch `sq-0284-storage-layout` (stacked; continues). Subagent-driven.
- Only the `app` crate changes. `zvm`/`gvm`/`mapper` untouched. Pure renderers (`render_svg`/`render_dot`/`render_dump`) unchanged.
- Destination resolution matches SQ-0284: none → `game_dir/<default>`; bare name → `game_dir/<name>` (append the format's extension if absent); value with a path separator (`/` or `\`) or absolute → verbatim.
- Fixed default names: `map.svg`, `map.dot`, `map.txt`, `transcript.txt`. Overwrite.
- `slash.rs:657` asserts `COMMANDS.len() == 55` — this task adds/removes NO commands (only edits existing dispatch/usage), so the count stays 55; do not change it.
- Commit trailers on every commit:
  `Quest: SQ-0288`
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Staging hygiene: stage ONLY the edited `crates/app/src/*.rs` / `docs/*` files by path — never `git add -A`.

---

### Task 1: Route all exports into the per-game dir (app)

One atomic task (resolver + command args + handlers compile/test together).

**Files:**
- Modify: `crates/app/src/export.rs` (add `resolve_export_path`; refactor `export_transcript`)
- Modify: `crates/app/src/slash.rs` (`export-svg`/`export-dot`/`export-dump` dispatch + usage, ~362-370)
- Modify: `crates/app/src/input.rs` (the `Action::Export{Svg,Dot,Dump}` variants ~155-159 — add `Option<String>`)
- Modify: `crates/app/src/main.rs` (map-export handlers ~3656-3695; transcript `SlashOutcome::Export` handler ~4395-4412; remove fixed `svg_path`/`dot_path`/`dump_path` ~1933-1936)
- Test: inline `#[cfg(test)]` in `export.rs`.

**Interfaces:**
- Consumes: `game_dir` (session `PathBuf` at `main.rs:1844`), pure renderers `render_svg(&RenderMap)`, `render_dot(&MapGraph)`, `render_dump(&MapGraph)`, wrappers `export_svg(path, rm)` / `export_dot(path, graph)`.
- Produces: `export::resolve_export_path(dest: Option<&str>, game_dir: &Path, default_name: &str) -> PathBuf`; refactored `export::export_transcript(lines: &[String], dest: Option<&str>, game_dir: &Path) -> <same return type as today>` — drop the old `exports_dir`/`stamp` params, add `game_dir`, KEEP the existing return type (check whether it's `PathBuf` or `io::Result<PathBuf>` and preserve it so the caller doesn't need reworking).

- [ ] **Step 1: Failing tests for `resolve_export_path`.** In `export.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn resolve_none_uses_default_name_in_game_dir() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(None, gd, "map.svg"), PathBuf::from("/data/Zork1.z5/map.svg"));
    }
    #[test]
    fn resolve_bare_name_appends_default_ext_when_missing() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("before"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.svg"));
        // an explicit extension on the bare name is preserved
        assert_eq!(resolve_export_path(Some("before.dot"), gd, "map.svg"), PathBuf::from("/data/Zork1.z5/before.dot"));
    }
    #[test]
    fn resolve_path_bearing_value_is_verbatim() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(resolve_export_path(Some("/tmp/x.svg"), gd, "map.svg"), PathBuf::from("/tmp/x.svg"));
    }
}
```

- [ ] **Step 2: Run → FAIL** (`cargo test -p app resolve_export_path`).

- [ ] **Step 3: Implement `resolve_export_path`** in `export.rs`:
```rust
use std::path::{Path, PathBuf};

/// Resolve an export destination the SQ-0284 way: no dest → `game_dir/<default_name>`;
/// a bare name (no separator) → `game_dir/<name>` with the default's extension appended
/// if the name has none; a value containing a path separator (or absolute) → verbatim.
pub fn resolve_export_path(dest: Option<&str>, game_dir: &Path, default_name: &str) -> PathBuf {
    match dest.map(str::trim) {
        None | Some("") => game_dir.join(default_name),
        Some(d) if d.contains('/') || d.contains('\\') => PathBuf::from(d),
        Some(d) => {
            let name = if Path::new(d).extension().is_some() {
                d.to_string()
            } else if let Some(ext) = Path::new(default_name).extension().and_then(|e| e.to_str()) {
                format!("{d}.{ext}")
            } else {
                d.to_string()
            };
            game_dir.join(name)
        }
    }
}
```
Run → PASS.

- [ ] **Step 4: Refactor `export_transcript`** to use it. Change its params to `(lines: &[String], dest: Option<&str>, game_dir: &Path)` (drop `exports_dir`/`stamp`, add `game_dir`) and KEEP its current return type; replace the internal `exports_dir`/`stamp` destination logic (export.rs:4-9) with `let path = resolve_export_path(dest, game_dir, "transcript.txt");`, keep the `create_dir_all(parent)` + write + return `path`. Update its test (if any) and add one asserting the default lands at `game_dir/transcript.txt`.

- [ ] **Step 5: Give the map commands an optional path arg.**
  - `input.rs`: change `Action::ExportSvg`/`ExportDot`/`ExportDump` to carry `Option<String>` (e.g. `ExportSvg(Option<String>)`). Update any exhaustive matches the compiler flags.
  - `slash.rs` (~362-370): change each dispatch to pass the arg and update `usage`:
    ```rust
    CommandSpec { name: "export-svg", category: Category::Export, context: Context::Map,
        usage: "export-svg [file]", description: "…",
        dispatch: |a| SlashOutcome::Action(Action::ExportSvg(a.first().map(|s| s.to_string()))) },
    ```
    (same for `export-dot`→`ExportDot`, `export-dump`→`ExportDump`; keep their existing `category`/`context`/`description`). `COMMANDS.len()` stays 55.

- [ ] **Step 6: Update the handlers to use `game_dir` + the resolver.**
  - Map handlers (`main.rs` ~3656-3695): for each, resolve the path and create the parent dir before writing:
    ```rust
    Action::ExportSvg(dest) => {
        let path = app::export::resolve_export_path(dest.as_deref(), &game_dir, "map.svg");
        if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
        let rm = render_map_data(&mapper.graph);
        match app::export::export_svg(&path, &rm) {
            Ok(()) => state.push_notice(format!("Exported map → {}", abbreviate_home(&path))),
            Err(e) => state.push_notice(format!("Export failed: {e}")),
        }
    }
    ```
    `ExportDot` → `"map.dot"` + `export_dot(&path, &mapper.graph)`; `ExportDump` → `"map.txt"` + `std::fs::write(&path, render_dump(&mapper.graph))`. Match the existing notice wording/style already in these arms (grep them first; keep `push_notice`/`push_notice`-equivalent and the ok/err shape).
  - Transcript handler (`main.rs` ~4395-4412): replace the `exports_dir`/`stamp`/`export_transcript(lines, dest, exports_dir, stamp)` call with `app::export::export_transcript(&lines, dest.as_deref(), &game_dir)` (keep the `visible_transcript_indices()` snapshot and the resolved-path notice). If this handler lives inside a separate `dispatch_slash_outcome` fn (not `main()`), thread `game_dir: &Path` into that fn's signature and pass it at the call site; if it's inline in `main()`'s loop, use `game_dir` directly. Verify by build.
  - Remove the now-unused fixed `svg_path`/`dot_path`/`dump_path` bindings (`main.rs` ~1933-1936) and any now-unused `format_stamp`/`exports_dir` locals YOUR change orphaned (only if unused elsewhere — grep first; `format_stamp` may be used by other features, leave it if so).

- [ ] **Step 7: Build + test.** `cargo build -p app --tests` warning-clean; `cargo test -p app` — 0 failed (SQ-0284/0285 tests still pass).

- [ ] **Step 8: Commit** (`feat(app): /export commands write into the per-game dir with an optional path (SQ-0288)`), staging only the edited `crates/app/src/*.rs` files.

- [ ] **Step 9: Docs.** Update `docs/persistence.md` (the export section, if any) and `README`/export docs to state exports land in the per-game dir with fixed default names (`map.svg`/`map.dot`/`map.txt`/`transcript.txt`) + the optional `[file]` arg (bare → per-game dir, path → verbatim), replacing the old `maps/<ifid>` / `exports/` description. Commit (`docs: /export writes into the per-game dir (SQ-0288)`), staging only the edited docs.

---

## Verification

```bash
cargo build -p app --tests
cargo test -p app export        # resolve_export_path + export_transcript
cargo test -p app               # no regressions
```

**Manual smoke:** in-game, run `/export-svg` → writes `<game_dir>/map.svg`; `/export-svg mymap` → `<game_dir>/mymap.svg`; `/export-svg /tmp/x.svg` → `/tmp/x.svg`; `/export-transcript` → `<game_dir>/transcript.txt`. Each shows a notice with the resolved (home-abbreviated) path. `--data-dir` redirects the base.

## Notes

- The four per-format commands stay; no unified `/export` command (per spec non-goals).
- Only the destination changes — the pure `graph → String` renderers and the transcript snapshot are untouched.
