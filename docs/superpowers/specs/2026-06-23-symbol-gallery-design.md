# Symbol Gallery — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued behind L8 (keymap) and L5 (symbols, done).
**TODO item:** "Add sample gallery of various options for each category of symbols… user can select preference from each category to create preferred combinations."

## Goal

A full-screen modal that lets the player browse the curated symbol presets per category, preview them on a live sample map, pick one preset per category, and persist the choices to `~/.lanthorn/config.toml`. v1 scope: **pick one preset per category** (no per-glyph editing, no named themes).

## Dependencies

- **L5 configurable symbols (done):** `SymbolSet`, `BoxStyle`/`Arrows`/`PathGlyphs`/`PortalGlyphs::preset(name)`, `SymbolConfig`, `SymbolSet::resolve`.
- **L8 keymap (in progress):** the gallery is opened by `Command::OpenGallery`; this spec's keymap entry lands only after L8 merges.
- New crate dependency: `toml_edit` (format-preserving config writes).

## UX

Full-screen two-pane modal:
- **Left pane:** the four categories — Box style, Arrows, Portals, Path — with the active one marked.
- **Right pane:** the active category's presets as a radio list (current selection marked), and below it a **live preview**: a small synthetic sample map (2–3 rooms, a cardinal edge, a portal room) rendered with the `SymbolSet` resolved from the *current gallery selections*.
- **Keys** (these are gallery-sub-mode keys, hardwired while the gallery is open — NOT general rebindable commands): `↑`/`↓` move through the active category's presets (selection applies LIVE to the running map immediately); `←`/`→` switch category; `Esc` closes the gallery AND persists the four choices to config. (`Enter` also closes+persists.)

## Architecture & components

1. **`crates/app/src/render/gallery.rs` (new)** — `pub fn draw_gallery(state: &AppState, area: Rect, buf: &mut Buffer)`. Renders the two panes. The preview is produced by building a small synthetic `mapper::render::RenderMap` (a fixed sample: rooms A,B with an east edge, plus a portal-flagged room) and calling the existing `render::map::render_map` with a temporary `AppState` whose `symbols` is the gallery's current resolved set — so the preview uses the real renderer and is exact.
2. **`AppState.gallery: Option<GalleryState>`** where `GalleryState { category_idx: usize, selections: [usize; 4] }` (one selected preset index per category). `None` = closed. A helper `GalleryState::symbol_config(&self) -> SymbolConfig` maps the four selected indices to preset names.
3. **Sub-mode routing in `key_to_action` (`input.rs`)** — when `state.gallery.is_some()`, route keys to a `gallery_key_to_action` (mirrors the existing prompt / tidy-anim sub-modes): ↑↓/←→ → gallery nav actions, Esc/Enter → close. Added as a new sub-mode layer; does NOT disturb the keymap lookups.
4. **`symbols.rs` additions** — `BoxStyle::preset_names() -> &'static [&'static str]` and the same for `Arrows`/`PathGlyphs`/`PortalGlyphs`; a `SymbolSet::from_preset_names(box_, arrow, portal, path) -> SymbolSet` (delegates to `resolve` of a synthetic `SymbolConfig`).
5. **Config writer — `config::write_symbols(dir: &Path, cfg: &SymbolConfig) -> std::io::Result<()>`** — load `~/.lanthorn/config.toml` with `toml_edit` (or start a new document), set `[symbols] box_style/arrow_set/portal_icons/path_style`, and write back PRESERVING all other keys/comments. Creates the file and parent dir if absent.
6. **Keymap entry (post-L8)** — `Command::OpenGallery` → `Action::OpenGallery` (opens the gallery, seeding `GalleryState` selections from the current `AppState.symbols`/config); default binding `g` in Map focus; appears in the help screen and hint bar automatically.

## Data flow

```
gallery selections ─▶ SymbolConfig ─▶ SymbolSet::resolve
   ├─▶ AppState.symbols (live map updates as you scroll presets)
   ├─▶ the preview sub-map render
   └─▶ on close: config::write_symbols persists to ~/.lanthorn/config.toml
```

## Actions / state additions

- `Action::OpenGallery`, `Action::GalleryNext`/`GalleryPrev` (preset), `Action::GalleryCategoryNext`/`Prev`, `Action::GalleryClose`.
- `apply_action`: `OpenGallery` sets `state.gallery = Some(seeded)`; nav actions mutate indices and re-resolve `state.symbols` live; `GalleryClose` resolves final selections, calls `config::write_symbols`, clears `state.gallery`.

## Testing

- `preset_names()` returns the expected sets for each category; `SymbolSet::from_preset_names("ascii","filled","ascii","light")` equals `resolve` of the matching `SymbolConfig`.
- `write_symbols` round-trip: write into a temp `config.toml` containing an unrelated `[other]` key + a `user_dir`; re-read; assert `[symbols]` keys are set/updated AND the unrelated keys survive.
- Gallery render test (TestBackend): modal shows the four category names and the active selection marker; after selecting `ascii` box style, the preview contains `+` corners.
- Gallery key test: ↑↓ change `selections[category_idx]`; ←→ change `category_idx` (wrapping); Esc → `gallery` cleared and persist invoked (assert via a temp dir).
- Live apply: a nav action updates `state.symbols` to match the new selection.

## Out of scope / non-goals

- Per-glyph override editing in the gallery (config supports `[symbols.overrides]`, but the gallery v1 only sets presets).
- Named/saved themes.
- Color theming (separate "Beautify" item).
- `mapper` changes (none — gallery is app-side; the preview reuses the existing renderer).

## Risks & limitations (accepted)

- **`toml_edit` dependency** for format-preserving writes. Without it a plain `toml` rewrite would drop user comments; `toml_edit` is the minimal correct tool.
- **Live re-resolve cost** on each keypress is negligible (resolve is a few-dozen-field build).
- The preview is a fixed synthetic sample, not the user's real map, so portal/path variety shown is limited to what the sample exercises (kept representative: cardinal edge + portal room + a junction).
