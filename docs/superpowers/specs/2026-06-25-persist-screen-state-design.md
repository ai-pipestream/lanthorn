# Persist Z-Machine Screen State in Saves (Step A) — Design

**Date:** 2026-06-25
**Status:** Approved (quick spec) — implement directly on main.
**Context:** Step **A** of the Bureaucracy restore investigation. The game-initiated
restore path (step B, shipped) redraws the screen itself. The **snapshot path**
(babelmap's Ctrl+R / auto-load at launch) bypasses the game's redraw: it slams the
Quetzal in at a READ point. For a game that splits its upper window only once at
startup (Bureaucracy), a fresh-launch auto-load leaves `upper_window_rows = 0`, so
the status line is blank. Confirmed by the user: "Ctrl+R / auto-load the upper
window is not restored; the story's restore command works."

## Goal

Persist the Z-machine **screen state** (upper-window split height + grid + cursor +
current window) in the `.babelmap` archive, and re-apply it after a host-mediated
restore (`restore_file`), so the upper window shows the saved status line on
Ctrl+R / auto-load.

## Why this is the fix

The Quetzal save records dynamic memory + stack + PC, never the screen. On a
host-mediated restore the game does not redraw (it resumes at a READ, and
once-split games don't re-split). Restoring the **saved grid** makes the upper
window display the status exactly as it was at save time (no game redraw needed),
and restoring the **split height** (the game's own value) lets the game's per-turn
content redraw land correctly thereafter. This is the standard snapshot/autosave
approach (iOS Frotz / Glk autosave persist window state alongside the VM).

## Design

### 1. zvm — make the screen state serializable

`crates/zvm/src/screen.rs`: derive `serde::{Serialize, Deserialize}` on `Cell`,
`UpperWindow`, and `ScreenState` (currently `#[derive(Debug, ...)]` only). `zvm`
already depends on `serde` (used elsewhere); add the derives (and the dep if a
feature gate is needed). `ScreenState` fields: `upper_window_rows`,
`current_window`, `text_style`, `cursor_row`, `cursor_col`, `buffer_mode`,
`show_status_requested`, `upper: UpperWindow { cols, rows, cells: Vec<Cell> }`.

### 2. archive — persist + load `screen.json`

`crates/app/src/archive.rs`:
- `save_archive_meta`: write a new zip entry `screen.json` =
  `serde_json::to_string(&machine.screen)`. (`save_archive_meta` already has
  `&machine`.)
- `load_archive`: read `screen.json` if present →
  `ArchiveContents.screen: Option<zvm::screen::ScreenState>`. Missing entry (old
  archives) → `None` (back-compat; no behavior change).

### 3. app — re-apply on the host-mediated restore paths

After `session.machine.restore_file(&ac.save)` at the **snapshot** restore sites
(auto-load at launch, Ctrl+R / SavesLoad non-in-game branch, slash-load,
RestoreGame, the resume helper), apply the saved screen:

```rust
if let Some(scr) = ac.screen.clone() { session.machine.screen = scr; }
```

The **in-game** restore path (`resume_restore`, step B) does NOT apply it — the
game redraws itself there. Only the host-mediated paths apply the saved screen.

## Notes / limitations

- Only saves made **after** this change carry `screen.json`; restoring an older
  save still shows a blank upper window (nothing to restore). New saves fix it.
- The saved grid reflects the status at save time; the game refreshes it on the
  next turn at the restored split height.

## Testing

- **archive round-trip:** save with a non-trivial `ScreenState` (e.g.
  `upper_window_rows = 1`, a grid cell set), reload, the `screen` round-trips
  (rows + a known cell glyph); an old archive (no `screen.json`) → `screen == None`.
- **manual / controller (bureaucr.z4):** make a save where the upper window is
  split + populated, then restore it via the host path (`restore_file` + apply
  screen) and assert `upper_window_rows > 0` and the grid is non-empty.

## Out of scope

- Step B (already shipped). The 3 save/restore UX refinements (story-only saved
  transcript, `[Game restored from …]` message, save-prompt styling) — done next,
  separately.
