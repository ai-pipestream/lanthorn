# Multiple Save Files — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued behind the archive wiring (L7) and L8 (keymap, done).
**TODO item:** "Add support for multiple save files."

## Goal

Let the player keep several named saves per story, each a self-contained `.lanthorn` archive, managed through one saves-manager modal (list / load / save-as / delete). The existing `Ctrl+S` quick-save to the default slot is unchanged.

## Model (decided)

- **Named slots.** Each named save is `<dir>/<ifid>-<name>.lanthorn`. The quick-save default is `<dir>/<ifid>.lanthorn` (from the archive wiring). Each save bundles the map + Quetzal game state (the archive container).
- **Unified saves-manager modal** for named saves: list, Load, Save As, Delete. `Ctrl+S` keeps quick-saving to the default slot silently.

## Dependencies

- **Archive wiring (L7):** `.lanthorn` as the save/load container and the default-slot semantics. THIS SPEC ASSUMES that brief is implemented first.
- **L8 keymap (done):** the modal is opened by a `Command::OpenSaves`; add the command + default binding to the keymap.
- `archive.rs` (`save_archive`/`load_archive`/`Meta`), already on `main`.

## Components

1. **`archive.rs` — extend `Meta`** from `{ format_version, ifid }` to `{ format_version, ifid, name: Option<String>, turns: u32, saved_at: String }`. New fields `#[serde(default)]` so archives written before this change still load. `saved_at` is an RFC3339 string from `std::time::SystemTime` (the app binary may use system time; only Workflow scripts cannot). `save_archive` gains the metadata (either extra params or a `Meta` argument — keep the existing 3-arg form working via a thin wrapper if simpler).
2. **`persist_files.rs` — slot helpers:**
   - `pub struct SaveInfo { pub path: PathBuf, pub name: String, pub turns: u32, pub saved_at: String, pub is_default: bool }`
   - `pub fn list_saves(dir: &Path, ifid: &str) -> Vec<SaveInfo>` — glob `<ifid>.lanthorn` and `<ifid>-*.lanthorn`, read each archive's `Meta`, sort (default first, then by `saved_at` desc). Files that fail to parse are skipped.
   - `pub fn save_named(dir: &Path, ifid: &str, name: &str, mapper: &Mapper, machine: &Machine, turns: u32) -> io::Result<()>` — sanitize `name` to a filesystem-safe slug, write `<ifid>-<slug>.lanthorn` with `Meta`.
   - `pub fn delete_save(path: &Path) -> io::Result<()>`.
3. **`render/saves.rs` (new)** — `pub fn draw_saves(state: &AppState, area: Rect, buf: &mut Buffer)`: a centered modal listing `SaveInfo` rows (`name`, `turn N`, short time; default row labelled "(default)"), current row marked, with a key hint footer.
4. **`AppState` additions:**
   - `pub saves: Option<SavesState>` where `SavesState { entries: Vec<SaveInfo>, selected: usize }` (None = closed).
   - `pub turns: u32` — a session turn counter incremented in `apply_action` on each `SubmitCommand` (non-empty command). Written into `Meta` on every save (quick-save and named).
5. **Sub-mode routing (`input.rs`)** — when `state.saves.is_some()`, route keys via a new `saves_key_to_action` (a sub-mode layer, mirroring the prompt / tidy-anim sub-modes, placed alongside them in `key_to_action`): `↑`/`↓` → `SavesNav(±1)`; `Enter` → `SavesLoad`; `s` → `SavesSaveAs`; `d` → `SavesDelete`; `Esc` → `SavesClose`. These sub-mode keys are hardwired (not rebindable), like the prompt/anim sub-modes.
6. **Actions + `apply_action`:**
   - `OpenSaves` → populate `state.saves = Some({ list_saves(...), 0 })`.
   - `SavesNav(d)` → move `selected` (clamped/wrapping).
   - `SavesLoad` → `archive::load_archive(entry.path)`, replace `mapper`, `machine.restore_quetzal(&ac.save)`, close modal.
   - `SavesSaveAs` → open the existing **prompt** sub-mode with a new `PromptKind::SaveAs`; on submit, `save_named(...)` then refresh the list.
   - `SavesDelete` → open a **confirm prompt** (`PromptKind::ConfirmDeleteSave(path)`); on confirm, `delete_save` + refresh.
   - `SavesClose` → `state.saves = None`.
7. **Keymap entry:** `Command::OpenSaves` + `Action::OpenSaves`, default binding (e.g. `Ctrl+O` or a Map-focus key), shown in help/hint automatically.

## Cross-cutting notes

- **`PromptKind` gains `SaveAs` and `ConfirmDeleteSave(PathBuf)`** (in `state.rs`); `apply_action`'s prompt-submit handler routes them. Reuses the entire existing prompt machinery (buffer, Enter/Esc).
- **Turn counter source:** simplest is app-tracked (`AppState.turns`, ++ per submitted command). If a reliable in-VM move counter is trivially available it may be used instead, but the app counter is the spec default.

## Testing

- `list_saves`: with two named `.lanthorn` files + the default, returns 3 `SaveInfo` with correct names/turns; a non-archive file in the dir is ignored; ordering (default first).
- `save_named` round-trip: save with name "before-troll", turns 42; `list_saves` shows it; `load_archive` restores the map + game bytes; name slug is filesystem-safe.
- `delete_save`: removes the file; subsequent `list_saves` omits it.
- `Meta` back-compat: an archive written with only `{format_version, ifid}` loads with `name=None, turns=0, saved_at=""`.
- Modal render test (TestBackend): lists names + turn counts; default labelled "(default)"; selection marker on the active row.
- Sub-mode key test: `↑↓` move `selected`; `s` → opens a `SaveAs` prompt; `d` → opens a confirm prompt; `Esc` → `saves` cleared.
- `turns` increments on `SubmitCommand` and is persisted into `Meta`.

## Out of scope / non-goals

- Auto-save / timestamped snapshots (separate TODO L21).
- Cross-story or cross-IFID save browsing.
- Cloud/remote saves; import/export of saves outside `<dir>`.
- Renaming an existing save in place (delete + save-as covers it for v1).

## Risks & limitations (accepted)

- **Name collisions:** saving a name that slugs to an existing file overwrites it — acceptable (it is a deliberate "save over"); the manager could warn, but v1 overwrites silently.
- **Metadata read cost:** `list_saves` opens each archive to read `Meta`. Save counts per story are small; fine. (A future optimization could read only the `meta.json` zip entry.)
- **Turn counter** is session-scoped (resets per launch unless restored from `Meta` on load) — acceptable; the displayed value is "turns at save time".
