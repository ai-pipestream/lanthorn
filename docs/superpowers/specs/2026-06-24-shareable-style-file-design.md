# Shareable Style File — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** "Move all style configuration settings to a separate style settings file. allow new style settings to be loaded based on the filepath. This allows users to share their styles." (#43)
**Touches:** `crates/app/src/config.rs`, `crates/app/src/colors.rs`, `crates/app/src/symbols.rs`, `crates/app/src/state.rs`, `crates/app/src/main.rs`, `crates/app/src/input.rs` (gallery/config save paths). New module likely: `crates/app/src/style.rs` (style-file load/merge/write). No `mapper`/`zvm` changes.

## Goal

Move all **visual** settings (colors + symbols, and — later — borders/header/input) out of `config.toml` into a standalone, shareable **style file**, loadable by name or filepath, while keeping **functional** settings (`user_dir`, `use_default_map`, `auto_load`, `auto_save`, `record_history`, `background_tidy`, `keymap`, `hotkeys`) in `config.toml`. Users share a look by handing someone a `style.toml`.

This elevates the existing precedent — `colors.scheme` already loads a Ghostty theme by path with `[colors].elements` layered on top — to the whole visual bundle.

## Model (decided)

`config.toml` gains one optional key:

```toml
style = "<built-in name or file path>"
```

The resolved look is built in **layers**, lowest to highest:

1. **Base** = the style file named by `style`:
   - a **built-in name** (embedded; v1 ships exactly one: `default` = today's terminal look), OR
   - a **file path** (`~` expanded, relative paths resolved against `user_dir`).
   - If `style` is **absent**: base = the user's personal `~/.babelmap/style.toml` if it exists, else the built-in `default`.
2. **Local overrides** = the existing `config.toml` `[colors]` / `[symbols]` sections, layered on top (backward compatible — with no `style` pointer and no style file, these fully define the look exactly as today).
3. The merged result resolves into `ColorScheme` + `SymbolSet` as now and is stored in `AppState.colors` / `AppState.symbols`.

**No general cascade/selector engine** (ratatui is immediate-mode; there is nothing like Textual TCSS to defer to). We implement a small, fixed selector set ourselves.

## Style file format

A standalone TOML with two sections in v1: `[colors]` and `[symbols]`. Unknown sections/keys are **preserved on rewrite** (so the beautify items' future `[header]`/`[input]`/border keys survive a gallery/config save).

### `[colors]` — CSS-ish element → declaration

```toml
[colors]
scheme = "tomorrow-night"            # optional base palette: Ghostty built-in name OR file path; selectors override it
"room"                = { fg = "white" }
"room:current"        = { reversed = true }
"room:selected"       = { fg = "yellow" }
"connector"           = { fg = "cyan" }
"connector:distorted" = { fg = "magenta" }
"connector:portal"    = { fg = "cyan" }
"border"              = { fg = "gray" }
"border:focused"      = { fg = "cyan", bold = true }
"statusbar"           = { reversed = true }
"transcript"          = { fg = "white" }
"suggestion"          = { fg = "#7a7a7a" }
"helpbar"             = { reversed = true }
```

- **Declaration properties:** `fg`, `bg`, `bold`, `italic`, `underline`, `dim`, `reversed`. All optional; absent = inherit from the layer below (base palette / built-in default).
- **Color values:** named ANSI (`black red green yellow blue magenta cyan gray white` + `bright-*` / `dark-*` as the existing parser supports), 8-bit index `0–255`, or hex `#rrggbb`. Reuse the existing color-parsing in `colors.rs` (the Ghostty hex parser) — extend with named/index if not already covered.
- **Fixed selector + variant set** (the ONLY recognized selectors; anything else = warning, ignored), each mapping to a `ColorScheme` field:
  | Selector | ColorScheme field |
  |---|---|
  | `room` | `room_normal` |
  | `room:current` | `room_current` (merged over `room`) |
  | `room:selected` | `room_selected` (merged over `room`) |
  | `connector` | `connector` |
  | `connector:distorted` | `connector_distorted` |
  | `connector:portal` | `portal_connector` |
  | `border` | (unfocused border base — currently unset; reserved) |
  | `border:focused` | `focused_border` |
  | `statusbar` | `status_bar` |
  | `transcript` | `transcript` |
  | `suggestion` | `suggestion` |
  | `helpbar` | `help_bar` |
  A `:variant` declaration is resolved as the base selector's Style patched by the variant's properties.

- **`scheme`** (optional): a base palette as today (Ghostty built-in name or file path). Selector declarations override it per element. This preserves the current Ghostty integration.

### Legacy compatibility

The previous `[colors].elements = { name = "color-string" }` map is still parsed: each entry sets that element's `fg`. New files use the selector tables; old configs keep working. Both feed the same merge.

### `[symbols]` — presets + overrides (unchanged shape)

Glyph selection is not a natural fit for CSS properties, so symbols keep today's shape, relocated into the style file:

```toml
[symbols]
box_style = "rounded"        # BoxStyle::preset_names()
arrow_set = "filled"         # Arrows::preset_names()
portal_icons = "nerdfont"    # PortalGlyphs::preset_names()
path_style = "light"         # PathGlyphs::preset_names()
[symbols.overrides]
"arrow.north" = "^"          # dotted per-glyph overrides as today
```

## Layering / merge semantics

`ColorsConfig` and `SymbolConfig` need a **merge(base, override)** where the override only affects keys it explicitly sets. This requires distinguishing "unset" from "default":

- The **override-layer** structs (parsed from `config.toml`'s `[colors]`/`[symbols]` and from a style file used as an override) use `Option` fields / presence, NOT defaulted strings. (Today `SymbolConfig` presets are `String` with `#[serde(default=...)]`, which cannot express "unset" — introduce an internal "raw/partial" form for merging, then finalize to the existing concrete `SymbolConfig`/`ColorsConfig` for resolution.)
- **Colors merge:** `scheme = override.scheme.or(base.scheme)`; per-selector declarations: base map ∪ override map, override wins per selector; within a selector, override properties patch base properties (per-property).
- **Symbols merge:** each preset = `override.preset.or(base.preset)`; `overrides` map = base ∪ override, override wins per glyph key.
- After merge, resolve via the existing `ColorScheme::resolve` / `SymbolSet::resolve` (adapted to consume the merged form + the new selector declarations).

## Editing (gallery `g` / config screen `F2`) — writes to the personal style file

Today the gallery writes `[symbols]` to `config.toml` and the config screen writes presets/`[colors].scheme` to `config.toml`. New behavior:

- Edits write to the user's **personal style file** `~/.babelmap/style.toml` (created on first edit), format-preserving via `toml_edit`, **preserving unknown sections** (future `[header]` etc.).
- **Fork-on-edit:** the edit session starts from the currently-resolved style (whatever `style` points at, plus overrides), and on save writes the result to `~/.babelmap/style.toml` AND sets `config.toml`'s `style` to point at the personal file (or clears it, since absent ⇒ personal file). So: load a friend's style, tweak it, and it becomes *your* style — what you see is what you save.
- **Gallery "Output all settings to style file" button:** an explicit, discoverable gallery action (footer button + key) that writes the **complete current style fully expanded** — every color selector and every symbol preset/override, not just deltas — to the personal `~/.babelmap/style.toml`, producing a **self-contained, shareable** file (no reliance on a base `scheme` or inherited defaults). It also repoints `config.toml`'s `style` to the personal file. This is the deliberate "export my whole look" affordance; fork-on-edit is the implicit version that fires on any normal save.
- `write_config` no longer writes `[colors]`/`[symbols]` (those move to the style file); it keeps writing the functional keys. A new `style::write_style(path, &resolved_style)` handles the style file. The legacy `[colors]`/`[symbols]` already in a user's `config.toml` are left untouched (still read as the override layer) but are no longer the write target.

## Components

- **`crates/app/src/style.rs` (new):** the `Style file` model (raw/partial structs for `[colors]` selector declarations + `[symbols]`), `load_style(name_or_path, user_dir) -> (StyleDoc, warnings)`, `merge(base, override) -> StyleDoc`, `resolve(StyleDoc, user_dir) -> (ColorScheme, SymbolSet, warnings)`, `write_style(path, &StyleDoc)` (toml_edit, preserve unknown), `write_style_full(path, &ColorScheme, &SymbolSet)` (the fully-expanded, self-contained export for the gallery button), and the built-in `default` embedded text. The fixed selector→field table lives here.
- **`render/gallery.rs`:** add the "Output all settings to style file" footer button + its key hint.
- **`input.rs`:** a gallery action (e.g. `GalleryExportStyle`) that calls `write_style_full` to the personal file and repoints `config.toml` `style`.
- **`config.rs`:** add `style: Option<String>` to `Config`; `Config::resolve` reads it; stop requiring `[colors]`/`[symbols]` here (still parse them as the override layer). `write_config` drops the style sections.
- **`colors.rs`:** extend the color-value parser to accept named/index/hex uniformly; adapt `ColorScheme` construction to consume merged selector declarations (base palette via `scheme` + per-selector patches).
- **`symbols.rs`:** `SymbolSet::resolve` consumes the merged symbol form (already preset+override; minimal change).
- **`main.rs`:** at startup, `load_style(config.style, user_dir)` → merge with config override layer → resolve → `state.colors`/`state.symbols`. Re-resolve after gallery/config save (writing the style file + repointing `style`).
- **`input.rs`:** gallery-close and config-save actions write the style file and repoint, instead of writing `config.toml` style sections.

## Scope boundary (#43 only)

- #43 builds the **machinery** (style file load + layering + edit-target + fork-on-edit) and moves **colors + symbols** into it.
- #43 does **not** pre-create empty `[borders]`/`[header]`/`[input]` sections (YAGNI). The beautify items (#42/#44/#45/#46) add those selectors/sections to the style file when built; the "preserve unknown sections on rewrite" rule guarantees they survive in the meantime.

## Testing

- **Pointer resolution:** built-in `default` name; a file path (`~` + relative-to-user_dir); absent ⇒ personal `style.toml` if present else `default`; missing/garbage path ⇒ warning + fall back to `default` (never crash).
- **Layer merge:** override-only-sets-present-keys (a base with `room.fg=white` + override `room.fg=red` ⇒ red; base `connector.fg=cyan` untouched when override omits it); per-property patch within a selector; `scheme` override-or-base; symbols preset override-or-base + glyph-map union.
- **Backward compat:** a `config.toml` with the OLD `[colors].elements`/`[symbols]` and no `style` pointer resolves to exactly today's look (golden test against current `ColorScheme`/`SymbolSet`).
- **Selector → field mapping:** each selector + `:variant` lands on the correct `ColorScheme` field with the right patched Style; unknown selector ⇒ warning, ignored, no crash.
- **Color value parsing:** named, `#rrggbb`, index `0-255` each parse; bad value ⇒ warning, ignored.
- **Write round-trip:** `write_style` writes selectors+symbols, is format-preserving, and PRESERVES an unrelated `[header]` section + comments (proves future beautify keys survive). Re-reading yields the same resolved style.
- **Fork-on-edit:** saving from the gallery/config writes `~/.babelmap/style.toml` and sets `config.toml` `style` to it; a subsequent load reflects the edit.
- **Gallery export-all:** the "Output all settings" action writes a fully-expanded style file — every selector + every symbol key present, no inherited gaps — and re-loading it WITHOUT any base scheme/overrides reproduces the same `ColorScheme`/`SymbolSet` (self-contained); the `style` pointer is repointed to it.

## Out of scope / non-goals

- A general CSS cascade / arbitrary selector matching (fixed selector set only).
- Layout-from-style (docking/grid) — ratatui doesn't support it; layout stays in code.
- Borders/header/input *rendering* and their style fields (those are #42/#44/#45/#46).
- Migrating users' existing `config.toml` style sections out automatically (they stay as the override layer).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Two override formats** (new selector tables + legacy `elements` strings) both parsed — small extra parsing, bounded and tested.
- **"Unset vs default" refactor** for the override layer is the trickiest part — handled by an internal partial/raw form distinct from the finalized concrete config structs.
- **Fork-on-edit repointing** changes `config.toml`'s `style` when you edit while a foreign/built-in style is active — intentional (what you see is what you save); documented behavior, covered by a test.
