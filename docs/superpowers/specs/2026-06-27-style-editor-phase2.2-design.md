# Live Style Editor — Phase 2.2 (save/load model + property-pane polish)

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Builds on:** merged Phase 1 / 1.1 / 2 / 2.1 style editor; the per-game styles
model (`docs/superpowers/specs/2026-06-25-per-game-styles-design.md`).

## Goal

Make the live style editor aware of the global-vs-per-game style model it already
lives inside, and finish three property-pane usability gaps. Two themes:

- **Theme A — save/load model:** the editor should open showing the *active*
  look (global merged with any per-game override) and let the user explicitly
  save to either the global style file or the current game's per-game file.
- **Theme B — property-pane polish:** make the shared color controls
  unambiguous and stable (fg/bg target indicator, an obvious hex edit box, and
  a non-reshuffling color MRU).

## Background (current code)

- **Live look** is `merge(global, per-game)` then `resolve`, computed in
  `reload_style` (`crates/app/src/reload.rs:32`); the per-game file is
  `user_dir/styles/<ifid>.toml` (`crates/app/src/styles.rs:7`,
  `per_game_style_path`). Merge is `style::merge(base, over)` (over wins per key).
- **The editor loads global only.** `open_style_editor`
  (`crates/app/src/input.rs:3050`) calls
  `load_style(state.config.style.as_deref(), &user_dir)` — the global personal
  style file — ignoring any per-game override. So for a game with a per-game
  file, the editor does **not** show the live look.
- **The editor saves global only.** `Action::StyleSave`
  (`crates/app/src/input.rs:2399`) takes the editor, saves the MRU, resolves the
  doc into live `state.colors`/`state.symbols`, and closes. The run loop's
  `style_save` flag then calls `save_style_and_repoint`
  (`crates/app/src/main.rs:2718`, def at `:98`), which writes the full live look
  to the global personal `style.toml` via `write_style_full`
  (`crates/app/src/style.rs:1149`) and repoints `config.style`.
- **`create-game-style`** (`slash.rs:312` → `Action::GameStyle`,
  `input.rs:1797`) only *scaffolds* an empty `styles/<ifid>.toml` with a header
  comment — it does not capture the editor's edits.
- **Property pane** (`crates/app/src/render/style_editor.rs`): row 1 = fg
  label/value, row 2 = fg swatch row, row 4 = bg label/value, row 5 = bg swatch
  row, row 7 = MRU strip, row 8 = custom hex entry rendered as bare `# <buf>`.
  The MRU strip and the custom field are **shared** across fg/bg and apply to
  `ed.color_target` (`false` = fg, `true` = bg). `StyleFocus::Fg`/`Bg` set the
  target; the active label shows a focus cursor but the shared region gives no
  indication of which target it will affect.
- **Color MRU** `push_mru` (`crates/app/src/style_mru.rs:59`) is move-to-front
  (`retain` + `insert(0)` + `truncate(CAP)`), so reusing or adding a color
  reshuffles swatch positions.

## Design

### Theme A — save/load model

#### A1. Open over the active (merged) style

`open_style_editor` loads the **active** style:

- Parse the global doc with `load_style(state.config.style.as_deref(), &user_dir)`.
- If `state.ifid` is non-empty and `per_game_style_path(&user_dir, &ifid)` exists
  and parses, set `doc = style::merge(&global, &per_game)`; otherwise `doc =
  global`. This mirrors `reload_style`'s resolution so the editor doc equals the
  live look.
- Resolve the preview from `doc` as today.

With no game loaded (empty `ifid`), behavior is unchanged (global only).

#### A2. Two explicit save buttons

Replace the single editor **Save** button with two, in the button row
**`[ Save Global Style ] [ Save Game Style ] [ Cancel ] [✕]`**:

- **Save Global Style** → existing behavior: apply the editor doc live, close,
  and (run-loop `style_save` flag) `save_style_and_repoint` writes the global
  personal `style.toml` and repoints `config.style`. Wired to the existing
  `Action::StyleSave`.
- **Save Game Style** → new `Action::StyleSaveGame`: apply the editor doc live,
  close, and (new run-loop `style_save_game` flag) write the doc **self-contained**
  to `per_game_style_path(user_dir, ifid)` via `write_style_full` (using the live
  `state.colors`/`state.symbols`, as `save_style_and_repoint` does). Per the
  approved semantics this is the *full* look, not a diff. It does **not** repoint
  `config.style` — `config.style` keeps pointing at the global file, and the
  per-game file is auto-merged over it on the next `reload_style`/launch.
  - **Disabled when no game is loaded** (`state.ifid.is_empty()`): the button is
    drawn dimmed and a click is a no-op that sets status `"no game loaded"`,
    mirroring the current `GameStyle` guard. The keyboard path does the same.

Both buttons take the editor and apply the resolved look live before closing, so
the live session immediately reflects the save (identical to today's Save).

Button wiring: add a `ButtonId::SaveGame` (or equivalent) and the
`Action::StyleSaveGame` it maps to in `style_dialog_action`
(`input.rs:635` neighborhood); keep the existing keyboard `s` = Save Global
(`input.rs:1213`) and add a second key for Save Game (e.g. `g`), both honoring
the no-game guard.

#### A3. Remove `create-game-style`

The Save Game Style button supersedes the scaffold (it writes real content):

- Remove the `create-game-style` `CommandSpec` (`slash.rs:312`) and
  `Action::GameStyle` (`input.rs:1797`).
- Update the registry-count assertion (48 → 47) and the verb-noun / help tests
  that reference `create-game-style` (`slash.rs:601`, `:617`).
- If `create-game-style`/`game-style` is bound in the default keymap, drop that
  binding too.

### Theme B — property-pane polish

#### B1. fg/bg target indicator

Make the shared region state its target and emphasize the active fg/bg label:

- The MRU strip label and the custom-hex label read `Recent (→ FG)` /
  `Custom (→ FG)` (or `→ BG`), following `ed.color_target`.
- The active target's label (`Foreground` / `Background`) is rendered with an
  emphasis style (e.g. bold/reverse via the editor's existing focused-label
  style) so the active target is obvious at a glance.
- Tab and clicking a target's label/swatch row still switch `color_target`
  (unchanged behavior); only the rendering changes.

#### B2. Obvious hex edit box

Render row 8 as a bracketed edit field with a visible cursor instead of bare
`# <buf>`:

- `Custom (→ FG): [ #ab12cd▏ ]` — a bordered/bracketed field whose interior shows
  the current `custom_buf` and a cursor glyph when the Custom focus is active.
- Themed via the editor's existing selectors (no new style selector); the
  `custom_rect` hit-rect already exists and stays the click target.

#### B3. Stable color MRU

Replace move-to-front with stable-position semantics in a new helper (the
existing `push_mru` callers switch to it; the glyph MRU `push_glyph_mru` may
adopt the same logic):

- **Already present:** leave the entry in place (no reorder).
- **New, room remaining (`len < CAP`):** append to the end.
- **New, full (`len == CAP`):** overwrite the oldest slot (ring) — the only case
  where a position's color changes.

This fixes the two-entry cycling so swatch click targets stay put during normal
use.

## Testing

- `push_mru` (stable): dedup-keeps-position, appends new to end, ring-evicts the
  oldest when full. Non-vacuous (assert specific index contents).
- Save Game Style: `Action::StyleSaveGame` over a hermetic temp `user_dir` with a
  set `ifid` writes `styles/<ifid>.toml`; assert the file exists, is
  self-contained, and round-trips via `load_style`/`parse_style_toml`. Assert the
  no-game guard (empty `ifid` → no file written, status set).
- Open-over-merged: with a global doc + a per-game override on disk, `open_style_editor`
  (hermetic temp dir, `ifid` set) yields `ed.doc` reflecting the per-game value
  for an overridden selector and the global value for a non-overridden one.
- Render (TestBackend buffer scrape, like the Phase 2.1 tests): the target
  indicator text (`→ FG` / `→ BG`) appears and follows `color_target`; the hex
  box brackets render; the button row shows `Save Global Style` and
  `Save Game Style` with the Game button dimmed when `ifid` is empty.
- All style-editor tests use the Phase 2.1 hermetic helper
  (`open_style_editor_hermetic`) so they never read the contributor's real
  `~/.lanthorn`.

## Out of scope

- Thin-diff per-game files (decided: self-contained full look).
- Separate per-fg/per-bg custom fields (decided: keep the shared-target model
  with an indicator).
- Reworking the per-game merge resolution itself (already implemented in
  `reload_style`).

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main` (no push without explicit instruction); TDD wave.
- Commit trailers: `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  and `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Every styleable element stays themeable; reuse existing selectors (no
  hard-coded styles, no new selector unless required).
