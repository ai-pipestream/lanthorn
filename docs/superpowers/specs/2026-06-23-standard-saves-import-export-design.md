# Import / Export Standard Saves (.qzl) — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued. Touches `render/saves.rs`, a new `render/filebrowser.rs`, `state.rs`, `input.rs`, `main.rs`.
**TODO item:** "Ability to import (load) and export (save) standard saves."

## Goal

Interchange standard **Quetzal** save files (`.qzl`/`.sav`) with other Z-machine interpreters: export the current game state to a `.qzl`, and import a foreign `.qzl`. Reached from the existing **saves manager** modal; file selection via a **simple file browser**.

## Feasibility — the I/O already exists

`machine.save_quetzal()` / `restore_quetzal()` are standard Quetzal (the same bytes the `.lanthorn` bundles), and `persist_files::save_game(path, &machine)` / `restore_game(path, &mut machine)` already write/read those bytes to/from a plain file. So:
- **Export** = `save_game(target.qzl, &session.machine)`.
- **Import** = `restore_game(source.qzl, &mut session.machine)` then re-observe the current location (`apply_turn("", &seed)` as elsewhere). The map is NOT part of a standard save, so it is **kept** as-is; after import the current-room global updates and the map continues from there.

No conversion or new serialization is needed — this feature is UI (a file browser) + wiring.

## UI

In the saves-manager modal (`render/saves.rs`, opened by `OpenSaves`/Ctrl+O), add two keys to the footer:
- **`e` = Export** → open the file browser in **PickDir** mode; choose a directory, then a filename prompt (default `<ifid>.qzl`) → `save_game(dir/name, &machine)`; status message; back to live.
- **`i` = Import** → open the file browser in **PickFile** mode listing `.qzl`/`.sav` files; pick one → `restore_game(path, &mut machine)` + re-observe; back to live.

## File browser (`render/filebrowser.rs`, new + a sub-mode)

`AppState.file_browser: Option<FileBrowserState>`:
```rust
pub struct FileBrowserState {
    pub cwd: PathBuf,
    pub entries: Vec<FbEntry>,   // sorted: ".." (if not root), then dirs, then matching files
    pub selected: usize,
    pub mode: FbMode,            // PickFile (import) | PickDir (export)
    pub export_default_name: String, // <ifid>.qzl, for the export filename prompt
}
pub struct FbEntry { pub name: String, pub is_dir: bool }
```
- Built from `std::fs::read_dir(cwd)`: directories always; in `PickFile`, also files ending `.qzl`/`.sav`. Starting `cwd` = `config.user_dir` (fall back to cwd).
- Keys (a new sub-mode in `key_to_action`, like saves/gallery): `↑/↓` move; `Enter` → if a directory, `cd` into it and rebuild; if `..`, go to parent; if a file (PickFile), **import it**; `s` (or Enter on no-dir in PickDir) → **choose the current dir** for export → open a filename **prompt** (`PromptKind::ExportSaveName(dir)`); `Esc`/`q` → cancel (back to live). Opaque background (`Style::reset().bg(...)`).

## Prompt

`PromptKind::ExportSaveName(PathBuf)` (the chosen directory) — reuse the text-entry prompt (default-filled with `<ifid>.qzl`); on submit → `save_game(dir/typed_name, &machine)`. Mirrors the saves-manager `SaveAs` prompt flow.

## Components

- **`render/filebrowser.rs` (new)** — `draw_file_browser(state, area, buf)`: the `cwd` header + entry list (dirs marked, files plain) + footer (`↑↓ · Enter open/pick · s here · Esc cancel`).
- **`render/saves.rs`** — add the `e`/`i` hints + the actions that open the browser.
- **`state.rs`** — `file_browser` field + `FileBrowserState`/`FbEntry`/`FbMode`; `PromptKind::ExportSaveName`.
- **`input.rs`** — `Action::SavesExport`/`SavesImport` (open the browser in the right mode), `FbNav`, `FbEnter`, `FbChooseDir`, `FbClose`; the file-browser sub-mode; `apply_action` / the prompt-submit handler doing `save_game`/`restore_game`.
- **`main.rs`** — render the browser when `file_browser.is_some()`; perform `restore_game` + re-observe on import and `save_game` on export-name submit (these need `&session.machine` / `&mut session.machine` + `story`-derived re-observe, which live in `main.rs`).
- **`persist_files.rs`** — reuse `save_game`/`restore_game` as-is (optionally thin `export_save`/`import_save` aliases for clarity).

## Testing

- `FileBrowserState` building: `read_dir` of a temp dir with subdirs + a `.qzl` + a `.txt` → entries list dirs + the `.qzl` (PickFile) but not the `.txt`; `..` present when not root; `PickDir` lists only dirs.
- Navigation: `Enter` on a dir changes `cwd` and rebuilds; `..` goes up.
- Export round-trip: pick a dir + name → `save_game` writes a file whose bytes equal `machine.save_quetzal()`; re-importing it via `restore_game` restores the same VM state (reuse the existing `persist_files` round-trip test pattern).
- Import keeps the map: after `restore_game`, `mapper` is unchanged; a re-observe seeds the current room.
- Sub-mode keys: the saves `e`/`i` open the browser in the right mode; `Esc` closes; the export `s`→prompt flow routes a typed name to `save_game`.

## Out of scope / non-goals

- Reading/writing other save formats (only Quetzal `.qzl`/`.sav`; that IS the standard).
- A full file manager (rename/delete/mkdir) — navigate + pick only.
- Importing a foreign save's map (standard saves have none; the current map is kept).
- `mapper`/`zvm` changes (Quetzal I/O already exists).

## Risks & limitations (accepted)

- **Foreign-save compatibility:** a `.qzl` from a DIFFERENT story will `restore_quetzal`-fail or misbehave; `restore_game` already returns an error on a bad/incompatible save — surface it on the status line, do not crash.
- **Path traversal UX:** the minimal browser navigates dirs and picks files; symlinks/permission errors are skipped gracefully (entries that fail to read are omitted).
- **Map vs game mismatch after import:** importing a save from a far-future point shows the kept map (which may have fewer/more rooms than that point); acceptable — the map is cumulative knowledge, and exploration continues normally.
