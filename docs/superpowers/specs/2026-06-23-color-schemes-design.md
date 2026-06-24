# Color Schemes (Ghostty themes) — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued (serial with mouse + auto-save; all touch the renderer/loop).
**TODO item:** "Beautify UI (color schemes, fancy console, etc.)" (L27). Realizes the color theming deferred from the symbols work (L5).

## Goal

Make babelmap's colors theme-able via **Ghostty terminal theme files**. By default babelmap uses the terminal's own colors (today's ANSI-named palette, which already respects the terminal theme). Pointing `[colors] scheme` at a Ghostty theme file (or a built-in name) recolors every element from that theme's palette. Per-element remapping is available in config.

## Format — Ghostty theme files

Ghostty themes use Ghostty's `key = value` config syntax (NOT YAML):
```
palette = 0=#1d1f21
palette = 1=#cc6666
...
palette = 15=#ffffff
background = 1d1f21
foreground = c5c8c6
cursor-color = c5c8c6
selection-background = 373b41
selection-foreground = c5c8c6
```
A scheme supplies the **16 ANSI palette colors** plus `background`, `foreground`, and (optionally) `cursor-color`, `selection-background`, `selection-foreground`. Hex may be `#rrggbb` or `rrggbb`. Unknown keys are ignored.

## Config — new `[colors]` section (extends Track B `Config`)

```toml
[colors]
scheme = "Tomorrow"      # a built-in name OR a path to a Ghostty theme file; omit = terminal colors

[colors.elements]         # optional: remap a babelmap element to a role/color
selected  = "palette:3"   # palette index
distorted = "#ff5555"     # explicit hex (truecolor)
connector = "cyan"        # ratatui named color
```
- `scheme`: `None` → terminal default colors (today's behavior). A bare name → a built-in scheme (see below). A path → a Ghostty theme file loaded from disk (relative to cwd or absolute; `~` expanded).
- `[colors.elements]`: per-element override. Value may be `palette:N` / `background` / `foreground` (a scheme role), a ratatui named color (`cyan`), a 256-index (`"17"`), or hex (`#5fafd7`). Overrides beat the scheme's default element→role mapping.

```rust
#[derive(Debug, Default, Deserialize)]
pub struct ColorsConfig {
    #[serde(default)] pub scheme: Option<String>,
    #[serde(default)] pub elements: std::collections::BTreeMap<String, String>,
}
```
`Config` gains `#[serde(default)] pub colors: ColorsConfig`.

## Components

1. **`crates/app/src/colors.rs` (new):**
   - `struct GhosttyScheme { palette: [Color; 16], background: Color, foreground: Color, cursor: Option<Color>, selection_bg: Option<Color>, selection_fg: Option<Color> }` with `pub fn parse(text: &str) -> Result<GhosttyScheme, String>` (parse the `key = value` lines: `palette = N=#hex`, `background`, `foreground`, …; hex → `Color::Rgb`).
   - `struct ColorScheme` — the resolved per-element colors the renderer uses. Fields (a `Style` or `Color` each): `room_normal`, `room_current` (rendered via REVERSED of fg/bg), `room_selected`, `connector`, `connector_distorted`, `portal_connector`, `status_bar` (REVERSED fg/bg), `transcript`, `suggestion`, `focused_border`, `help_bar` (REVERSED). (Covers "Everything": map states/connectors + chrome.)
   - `ColorScheme::terminal_default() -> ColorScheme` — reproduces TODAY's exact colors (room/connector White/Cyan/Yellow/Magenta, status/help REVERSED, focused border Cyan+Bold, suggestion DarkGray). This is the default; the existing renderer color tests must keep passing against it.
   - **Default element→role mapping** (used when a scheme IS loaded): `room_normal→foreground`, `room_current→reversed(fg,bg)`, `room_selected→palette[3]`, `connector→palette[6]`, `connector_distorted→palette[5]`, `portal_connector→palette[6]`, `status_bar→reversed(fg,bg)`, `transcript→foreground`, `suggestion→palette[8]`, `focused_border→palette[6]+bold`.
   - `ColorScheme::from_ghostty(&GhosttyScheme, overrides: &BTreeMap<String,String>) -> ColorScheme` — apply the default mapping, then the per-element overrides (parse each value as palette-role / named / index / hex).
   - `ColorScheme::resolve(cfg: &ColorsConfig, dir: &Path) -> (ColorScheme, Vec<String>)` — `scheme=None` → `terminal_default()` (then apply any element overrides on top); a built-in name → its embedded Ghostty text; a path → read the file; parse failure / missing file → warning + `terminal_default()`.
2. **Built-in schemes** — embed a few Ghostty theme texts (`include_str!`) under `crates/app/src/colors/` (e.g. `default`=terminal, `mono`, `high-contrast`, plus one popular theme like `tomorrow-night`). `scheme = "<name>"` selects one.
3. **`state.rs`** — `AppState.colors: ColorScheme` (default `terminal_default()`), set at startup from `ColorScheme::resolve(&cfg.colors, &cfg.user_dir)`.
4. **Renderer refactor** — replace the hardcoded color constants with `state.colors.*` lookups:
   - `render/map.rs`: `CURRENT_STYLE`/`SELECTED_STYLE`/`NORMAL_STYLE`/`CONNECTOR_STYLE`, the `Magenta`/`Cyan` distorted/connector inline colors, the portal connector cyan.
   - `render/transcript.rs`: `STATUS_STYLE`, transcript `NORMAL_STYLE`, the `DarkGray` suggestion color.
   - `main.rs`: the `focused_border` (Cyan+Bold) and the REVERSED title/help/overlay styles where they carry color.
   These read `state.colors` (the renderer already receives `&AppState`).

## Default & back-compat

With no `[colors]` config, `ColorScheme::terminal_default()` reproduces today's colors exactly — the existing renderer color assertions (e.g. arrowhead fg = Cyan, no Cyan/Magenta ribbon) pass unchanged. Loading a scheme swaps in the theme's palette.

## Testing

- `GhosttyScheme::parse`: a sample theme text yields the right `palette[1]`, `background`, `foreground`; malformed lines are skipped; missing palette entries error or default.
- `ColorScheme::terminal_default()` equals today's constants (assert the key fields: connector Cyan, distorted Magenta, selected Yellow).
- `from_ghostty` maps elements onto the palette (e.g. `connector == scheme.palette[6]`); an `elements` override (`selected="#ff0000"`) beats the mapping.
- `resolve`: `scheme=None` → terminal_default; a built-in name → its colors; a bad path → warning + terminal_default.
- Renderer back-compat (TestBackend): default config reproduces current colors; a built-in scheme changes a sampled cell's fg.
- Config: `[colors]` with `scheme` + `elements` parses into `ColorsConfig`.

## Out of scope / non-goals

- An in-app color picker / gallery integration (selection is config-driven for v1; a future "Colors" gallery category could come later).
- Writing/exporting Ghostty themes from babelmap.
- Per-layer or per-room colors.
- `mapper` changes (colors are app-side).
- Re-theming the structural REVERSED modifiers' monochrome behavior beyond swapping the scheme's fg/bg.

## Risks & limitations (accepted)

- **Truecolor:** scheme colors are RGB (`Color::Rgb`); terminals without truecolor approximate them. The default (no scheme) uses ANSI-named colors and is safe everywhere.
- **Ghostty format coverage:** we parse the color-relevant keys (`palette`, `background`, `foreground`, `cursor-color`, `selection-*`) and ignore the rest of a Ghostty config; we do not implement Ghostty's full config grammar.
- **Readability:** `[colors.elements]` overrides can make text unreadable (e.g. fg == bg); that is the user's choice, not validated beyond parsing.
