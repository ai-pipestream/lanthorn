# Configurable Map Symbols — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — awaiting spec review
**TODO item:** "Add support for configurable symbols: room outlines (normal, selected, portal, current position), arrows, portal icons, paths, portal paths."
**Foundation for:** the symbol gallery (TODO: "Add sample gallery of various options for each category of symbols…").

## Goal

Let users reconfigure the glyphs the map renderer uses, without touching code. Today every glyph is a literal scattered through `crates/app/src/render/map.rs`. This foundation centralizes them into one `SymbolSet`, resolved from config (named presets per category, plus optional per-glyph overrides), with **defaults that reproduce today's rendering byte-for-byte**. Colors are explicitly out of scope (deferred to the separate "Beautify UI" item).

## Configuration model

**Named presets per category, with optional per-glyph overrides** (chosen over presets-only and raw-only):

```toml
[symbols]
box_style    = "rounded"   # rounded | thick | double | ascii | borderless  (default: rounded)
arrow_set    = "filled"    # filled | line | nerdfont                        (default: filled)
portal_icons = "ascii"     # ascii | nerdfont                                (default: ascii)
path_style   = "light"     # light | heavy | dotted                          (default: light)

[symbols.overrides]         # optional; a slot here beats the preset
"arrow.north"      = "↑"
"room.selected.tl" = "▛"
"portal.up"        = "⬆"
```

- A missing `[symbols]` table (or any missing field) falls back to the default preset → today's exact glyphs.
- `[symbols.overrides]` keys are dotted slot paths (see Slot map below). An override beats the preset for that one slot.
- The default preset for every category equals the current hardcoded glyph set, so an absent config changes nothing.

## Architecture

**App-side `SymbolSet`, resolved once at startup, carried in `AppState`.** `mapper` is untouched — it is pixel-agnostic; glyphs are an app rendering concern.

```
config.toml [symbols] ──▶ symbols::SymbolSet::resolve(&Config) ──▶ AppState.symbols ──▶ render/map.rs reads state.symbols.*
```

Rejected alternatives: threading a `&SymbolSet` parameter through every render fn (more churn — `AppState` is already passed everywhere the renderer needs); a global/`OnceCell` set (hostile to testing and to the future live gallery, which mutates the set in place).

## Components

### 1. `crates/app/src/symbols.rs` (new)

- `struct BoxStyle { tl, tr, bl, br, h, v: char }` — one room outline.
- `struct SymbolSet`:
  - `room_normal, room_current, room_portal, room_selected: BoxStyle`
  - `arrows_cardinal: { north, south, east, west: &str }` (filled ▲▼▶◀ today)
  - `arrows_diagonal: { ne, nw, se, sw: &str }` (the `diagonal_arrow` set)
  - `path: PathGlyphs` — the line-art table that `glyph_for(mask)` returns (─ │ ┌ ┐ └ ┘ ├ ┤ ┬ ┴ ┼ …)
  - `portal_icons: { up, down, in_, out, unknown: &str }`
  - `portal_path: char` — the connector used by `draw_portal_connectors`
- Preset constructors per category: `BoxStyle::preset(name)`, `arrow set / portal icons / path` presets. Each `preset` returns `Option` (unknown name → fall back to default + ignored).
- `SymbolSet::default()` — equals today's literals exactly.
- `SymbolSet::resolve(cfg: &Config) -> SymbolSet` — start from per-category presets named in `[symbols]`, then apply `[symbols.overrides]` slot-by-slot.

### 2. Config integration (extends Track B's `Config`, already merged)

- Add `symbols: SymbolConfig` to `Config` with `#[serde(default)]`, where `SymbolConfig` holds the four preset names (each defaulting to today's preset) and an `overrides: BTreeMap<String, String>`.
- `Config` already loads `~/.lanthorn/config.toml`; this is purely additive.

### 3. Renderer integration (`crates/app/src/render/map.rs`)

- Replace the literal 6-tuples at `map.rs:1131` (`draw_compact_room`) and `map.rs:1224` (`draw_box_room`), `diagonal_arrow`, `arrow_for_departure`, `glyph_for`, the portal-icon code in `draw_portal_icons`, and the portal connector in `draw_portal_connectors` with lookups into `state.symbols.*`.
- **Outline flavor precedence:** `current > portal > selected > normal`. Structural portal marking (the double outline) is preserved even when a room is selected; selection remains *always* visible via the existing yellow `SELECTED_STYLE` color, which is unchanged. Colors stay orthogonal and out of scope.
- `render_map`/`draw_*` already receive `&AppState`; they read `state.symbols` instead of constants.

### Slot map (override keys)

```
room.{normal,current,portal,selected}.{tl,tr,bl,br,h,v}
arrow.{north,south,east,west,ne,nw,se,sw}
path.{h,v,tl,tr,bl,br,vr,vl,hd,hu,cross}      # glyph_for mask slots
portal.{up,down,in,out,unknown}
portal.path
```

## Data flow / back-compat

- With no `[symbols]` config, `SymbolSet::resolve` returns `SymbolSet::default()` == today's glyphs → the map renders identically.
- `room_selected` defaults to the **normal** outline (today selection is color-only); a user opts into a distinct selected outline via preset/override.
- Override validation: each override value must be a single display-width character. A value that is empty, multi-char, or wide (e.g. an emoji that occupies two cells) is rejected at resolve time — the slot keeps its preset glyph. This protects the fixed-cell grid. (Nerdfont presets are curated to single-width; the default presets are pure Unicode box-drawing, safe on any terminal.)

## Testing

- **`symbols::resolve` units:** `default()` equals the current literals (assert each `BoxStyle`/arrow/path/portal glyph against the values pulled from today's `map.rs`); preset selection swaps the set; an override beats its preset; a bad-width / empty / multi-char override is rejected and the preset glyph survives.
- **Renderer snapshot (TestBackend):** a frame rendered with `SymbolSet::default()` reproduces the current map output (guards byte-for-byte back-compat); a frame with `box_style = "ascii"` shows the ASCII corners instead of rounded.
- **Config round-trip:** a `[symbols]` TOML with presets + overrides parses into `SymbolConfig` and resolves to the expected `SymbolSet`.

## Out of scope / non-goals

- **Colors / theming** (current/selected/distorted/connector styles) — deferred to the "Beautify UI" TODO item.
- **The interactive gallery** — a separate TODO; this spec only provides the `SymbolSet` abstraction and presets it will browse/select.
- **Per-layer or per-room symbol sets** — one set applies map-wide.
- **`mapper` changes** — none; glyphs stay app-side.

## Risks & limitations (accepted)

- **Nerdfont presets need a patched font / compatible terminal.** Defaults are pure Unicode; nerdfont sets are opt-in. Documented, not guarded at runtime beyond the single-width check.
- **Override width validation is display-width based.** Width is computed with the same method the renderer assumes (one cell per glyph); exotic combining sequences are treated conservatively (rejected) rather than risking grid corruption.
- **Centralizing every literal touches a lot of `map.rs`.** The change is mechanical (literal → field lookup) and guarded by the back-compat snapshot test.
