# Shareable Style File — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move all visual settings (colors + symbols) into a standalone, shareable style file referenced from `config.toml` by `style = "<name or path>"`, with the existing config sections kept as a backward-compatible override layer and a CSS-ish element→properties color format.

**Architecture:** A new `crates/app/src/style.rs` owns a partial/“raw” style model (`StyleDoc`), its TOML (de)serialization, pointer resolution (built-in name / file path / absent), layer merge (base ⊕ override, present-keys-only), resolution into the existing `ColorScheme`/`SymbolSet`, and format-preserving writers. `config.rs` gains an optional `style` field and stops writing `[colors]`/`[symbols]`. `main.rs` loads+merges+resolves at startup and after gallery/config saves; the gallery gains an “Output all settings” export button.

**Tech Stack:** Rust, ratatui 0.29, `toml` (read) + `toml_edit` (format-preserving write), serde.

## Global Constraints

- No `mapper`/`zvm` changes. Style is an `app`-crate concern.
- No backward compatibility (app is undeployed): no legacy `elements` packed-string format, no migration — old-format `~/.babelmap` files can be deleted. An empty style (no `style` pointer, no config override) must still resolve to today's terminal-default look.
- Fixed selector set only — no general CSS cascade/selector matching. Unknown selector ⇒ warning, ignored, never crash. Missing/garbage style path ⇒ warning, fall back to built-in `default`, never crash.
- Writers are format-preserving (`toml_edit`) and MUST preserve unknown sections/keys (so future `[header]`/`[input]`/border keys survive a save).
- Commit messages: NO backticks in the body; end every commit body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Keep the build warning-clean and `cargo test --workspace` green after every task (it is currently fully warning-clean — do not introduce new warnings).
- Reuse existing helpers: `colors::parse_color_value(value, scheme)` (hex + 8-bit index today), `colors::ColorScheme` fields, `colors::GhosttyScheme`, `symbols::SymbolSet::resolve(&SymbolConfig)`, `symbols::*::preset_names()`.

## File structure

- **Create `crates/app/src/style.rs`** — the whole style-file subsystem: `Decl`, `StyleColors`, `StyleSymbols`, `StyleDoc` (partial model); `parse_decl_color`; `SELECTOR_FIELDS` table + `apply_color_decls`; `merge`; `finalize_symbols`; `resolve`; `load_style`; `write_style` / `write_style_full`; `DEFAULT_STYLE_TOML`.
- **Modify `crates/app/src/lib.rs`** — `pub mod style;`.
- **Modify `crates/app/src/config.rs`** — add `style: Option<String>` to `Config`; parse in `resolve`; drop `[colors]`/`[symbols]` writing from `write_config`.
- **Modify `crates/app/src/colors.rs`** — extend `parse_color_value` (or add `parse_color_named_or_value`) to also accept named ANSI colors; expose a way to patch a single `ColorScheme` field’s `Style` from a `Decl`.
- **Modify `crates/app/src/main.rs`** — startup: `style::load_style(...)` → `merge` with config override → `resolve` → `state.colors`/`state.symbols`. Repoint+write on gallery/config save.
- **Modify `crates/app/src/input.rs`** — `GalleryExportStyle` action; gallery-close/config-save write the personal style file and repoint.
- **Modify `crates/app/src/render/gallery.rs`** — “Output all settings to style file” footer button + key hint.

---

### Task 1: Style model + per-declaration color parsing

**Files:**
- Create: `crates/app/src/style.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod style;`)
- Modify: `crates/app/src/colors.rs` (named-color support)

**Interfaces:**
- Produces:
  - `pub struct Decl { pub fg: Option<String>, pub bg: Option<String>, pub bold: Option<bool>, pub italic: Option<bool>, pub underline: Option<bool>, pub dim: Option<bool>, pub reversed: Option<bool> }` (derive `Debug, Clone, Default, PartialEq, serde::Deserialize`)
  - `pub fn decl_to_style(d: &Decl, scheme: &crate::colors::GhosttyScheme) -> ratatui::style::Style` — turns a `Decl` into a ratatui `Style` (color strings parsed via `colors::parse_color_value`, modifiers added when the bool is `Some(true)`).
  - `pub fn parse_named_color(name: &str) -> Option<ratatui::style::Color>` in `colors.rs` (used by `parse_color_value`).

- [ ] **Step 1: Write the failing test (named colors)** — in `colors.rs` tests:
```rust
#[test]
fn parse_color_value_accepts_named_colors() {
    let scheme = GhosttyScheme::default(); // or a minimal scheme
    assert_eq!(parse_color_value("red", &scheme), Some(Color::Red));
    assert_eq!(parse_color_value("bright-blue", &scheme), Some(Color::LightBlue));
    assert_eq!(parse_color_value("white", &scheme), Some(Color::White));
}
```

- [ ] **Step 2: Run it, confirm it fails** — `cargo test -p app parse_color_value_accepts_named_colors` → FAIL (named not handled).

- [ ] **Step 3: Implement named-color support** — add `parse_named_color` mapping the ratatui named set (`black red green yellow blue magenta cyan gray/grey white`, plus `bright-*`/`dark-*` → the `Light*`/`Dark*` ratatui variants) and call it first inside `parse_color_value` before the hex/index paths.

- [ ] **Step 4: Run it, confirm it passes.**

- [ ] **Step 5: Write the failing test (decl_to_style)** — in `style.rs` tests:
```rust
#[test]
fn decl_to_style_sets_fg_and_modifiers() {
    use ratatui::style::{Color, Modifier};
    let scheme = crate::colors::GhosttyScheme::default();
    let d = Decl { fg: Some("cyan".into()), bold: Some(true), reversed: Some(true), ..Default::default() };
    let s = decl_to_style(&d, &scheme);
    assert_eq!(s.fg, Some(Color::Cyan));
    assert!(s.add_modifier.contains(Modifier::BOLD));
    assert!(s.add_modifier.contains(Modifier::REVERSED));
    assert_eq!(s.bg, None); // bg omitted => unset
}
```

- [ ] **Step 6: Run it, confirm it fails** (module/fn missing).

- [ ] **Step 7: Implement `Decl`, `decl_to_style`, and the module skeleton** in `style.rs`; add `pub mod style;` to `lib.rs`. `decl_to_style` parses `fg`/`bg` via `colors::parse_color_value` and adds `Modifier::{BOLD,ITALIC,UNDERLINED,DIM,REVERSED}` for each `Some(true)`.

- [ ] **Step 8: Run it, confirm it passes; confirm no warnings** (`cargo build -p app` clean).

- [ ] **Step 9: Commit** — `git add -A && git commit` (message per Global Constraints): "feat(style): style Decl model + named color parsing".

---

### Task 2: Selector → ColorScheme field application

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `Decl`, `decl_to_style` (Task 1); `colors::ColorScheme`, `colors::GhosttyScheme`.
- Produces:
  - `pub const SELECTOR_FIELDS: &[&str]` — the recognized selectors: `["room","room:current","room:selected","connector","connector:distorted","connector:portal","border","border:focused","statusbar","transcript","suggestion","helpbar"]`.
  - `pub fn apply_color_decls(cs: &mut ColorScheme, decls: &BTreeMap<String, Decl>, scheme: &GhosttyScheme) -> Vec<String>` — for each known selector present in `decls`, patch the matching `ColorScheme` field by merging the decl’s `Style` onto it (`field = field.patch(decl_to_style(..))`). `:variant` selectors patch the variant field (e.g. `room:current` → `cs.room_current`). Unknown selectors are collected into the returned warnings vec. Mapping:
    | selector | field |
    |---|---|
    | room | room_normal |
    | room:current | room_current |
    | room:selected | room_selected |
    | connector | connector |
    | connector:distorted | connector_distorted |
    | connector:portal | portal_connector |
    | border:focused | focused_border |
    | statusbar | status_bar |
    | transcript | transcript |
    | suggestion | suggestion |
    | helpbar | help_bar |
    (`border` with no variant is accepted and currently ignored — reserved for the unfocused border; record nothing, no warning.)

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn apply_color_decls_patches_correct_fields() {
    use ratatui::style::{Color, Modifier};
    let scheme = crate::colors::GhosttyScheme::default();
    let mut cs = crate::colors::ColorScheme::terminal_default();
    let mut decls = std::collections::BTreeMap::new();
    decls.insert("connector".to_string(), Decl { fg: Some("magenta".into()), ..Default::default() });
    decls.insert("border:focused".to_string(), Decl { fg: Some("yellow".into()), bold: Some(true), ..Default::default() });
    let warns = apply_color_decls(&mut cs, &decls, &scheme);
    assert!(warns.is_empty());
    assert_eq!(cs.connector.fg, Some(Color::Magenta));
    assert_eq!(cs.focused_border.fg, Some(Color::Yellow));
    assert!(cs.focused_border.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn apply_color_decls_warns_on_unknown_selector() {
    let scheme = crate::colors::GhosttyScheme::default();
    let mut cs = crate::colors::ColorScheme::terminal_default();
    let mut decls = std::collections::BTreeMap::new();
    decls.insert("bogus".to_string(), Decl { fg: Some("red".into()), ..Default::default() });
    let warns = apply_color_decls(&mut cs, &decls, &scheme);
    assert_eq!(warns.len(), 1);
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `SELECTOR_FIELDS` + `apply_color_decls`** with a `match selector { ... }` patching each field via `Style::patch`.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): selector→ColorScheme field application".

---

### Task 3: Symbol finalize + resolve

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `config::SymbolConfig`, `symbols::SymbolSet`.
- Produces:
  - `pub struct StyleSymbols { pub box_style: Option<String>, pub arrow_set: Option<String>, pub portal_icons: Option<String>, pub path_style: Option<String>, pub overrides: BTreeMap<String,String> }` (derive `Debug, Clone, Default, PartialEq, Deserialize`).
  - `pub fn finalize_symbols(s: &StyleSymbols) -> config::SymbolConfig` — fills each `None` preset with the existing `config::default_*` value, copies `overrides`, producing a concrete `SymbolConfig` ready for `SymbolSet::resolve`.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn finalize_symbols_fills_defaults_and_keeps_overrides() {
    let mut s = StyleSymbols::default();
    s.box_style = Some("thick".into());
    s.overrides.insert("arrow.north".into(), "^".into());
    let cfg = finalize_symbols(&s);
    assert_eq!(cfg.box_style, "thick");
    assert_eq!(cfg.arrow_set, crate::config::default_arrow_set()); // unspecified => default
    assert_eq!(cfg.overrides.get("arrow.north").map(String::as_str), Some("^"));
    // resolve must succeed
    let _set = crate::symbols::SymbolSet::resolve(&cfg);
}
```
(If `config::default_arrow_set` etc. are private, make them `pub(crate)` as part of this task.)

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `StyleSymbols` + `finalize_symbols`** (and `pub(crate)` the `default_*` fns if needed).
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): StyleSymbols finalize to SymbolConfig".

---

### Task 4: Layer merge (present-keys-only)

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `Decl`, `StyleSymbols` (Tasks 1, 3).
- Produces:
  - `pub struct StyleColors { pub scheme: Option<String>, pub selectors: BTreeMap<String, Decl> }` (derive `Debug, Clone, Default, PartialEq`).
  - `pub struct StyleDoc { pub colors: StyleColors, pub symbols: StyleSymbols }` (derive `Debug, Clone, Default, PartialEq`).
  - `pub fn merge(base: &StyleDoc, over: &StyleDoc) -> StyleDoc` — produces a new doc where: `colors.scheme = over.scheme.clone().or(base.scheme.clone())`; `colors.selectors` = base ∪ over with, per selector key, the over `Decl` field-merged onto the base `Decl` (each `Option` field: `over.or(base)`); `symbols`: each preset `over.or(base)`, `overrides` = base ∪ over (over wins).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn merge_override_only_affects_present_keys() {
    let mut base = StyleDoc::default();
    base.colors.selectors.insert("room".into(), Decl { fg: Some("white".into()), ..Default::default() });
    base.colors.selectors.insert("connector".into(), Decl { fg: Some("cyan".into()), ..Default::default() });
    base.symbols.box_style = Some("rounded".into());

    let mut over = StyleDoc::default();
    over.colors.selectors.insert("room".into(), Decl { fg: Some("red".into()), ..Default::default() });
    // over does not mention connector or box_style

    let m = merge(&base, &over);
    assert_eq!(m.colors.selectors["room"].fg.as_deref(), Some("red"));   // overridden
    assert_eq!(m.colors.selectors["connector"].fg.as_deref(), Some("cyan")); // base preserved
    assert_eq!(m.symbols.box_style.as_deref(), Some("rounded"));          // base preserved
}

#[test]
fn merge_field_level_decl_patch() {
    let mut base = StyleDoc::default();
    base.colors.selectors.insert("room".into(), Decl { fg: Some("white".into()), bold: Some(true), ..Default::default() });
    let mut over = StyleDoc::default();
    over.colors.selectors.insert("room".into(), Decl { fg: Some("red".into()), ..Default::default() }); // only fg
    let m = merge(&base, &over);
    assert_eq!(m.colors.selectors["room"].fg.as_deref(), Some("red")); // over wins
    assert_eq!(m.colors.selectors["room"].bold, Some(true));            // base bold kept
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `StyleColors`, `StyleDoc`, `merge`** (include a private `merge_decl(base, over) -> Decl` doing per-`Option` `over.clone().or(base.clone())`).
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): present-keys-only layer merge".

---

### Task 5: Parse a StyleDoc from TOML (file + config override) incl. legacy elements

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `StyleDoc`, `StyleColors`, `StyleSymbols`, `Decl`.
- Produces:
  - `pub fn parse_style_toml(text: &str) -> Result<StyleDoc, String>` — parses the format used by BOTH the style file and `config.toml`'s override sections: `[colors]` with optional `scheme` and selector keys as inline tables (`"room:current" = { reversed = true }`) into `selectors`; `[symbols]` presets + `[symbols.overrides]`. Uses `toml::Value` so unknown keys are tolerated. (No legacy `elements` map — app is undeployed, no backward compat.)
  - `pub fn style_from_config(colors: &StyleColors, symbols: &StyleSymbols) -> StyleDoc` — wraps the config-override partial sections (already in the new format, see Task 9) into a `StyleDoc` for merging.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn parse_style_toml_reads_selectors_scheme_symbols() {
    let text = r#"
[colors]
scheme = "tomorrow-night"
"room" = { fg = "white" }
"room:current" = { reversed = true }
"suggestion" = { fg = "#7a7a7a" }

[symbols]
box_style = "rounded"
[symbols.overrides]
"arrow.north" = "^"
"#;
    let doc = parse_style_toml(text).unwrap();
    assert_eq!(doc.colors.scheme.as_deref(), Some("tomorrow-night"));
    assert_eq!(doc.colors.selectors["room"].fg.as_deref(), Some("white"));
    assert_eq!(doc.colors.selectors["room:current"].reversed, Some(true));
    assert_eq!(doc.colors.selectors["suggestion"].fg.as_deref(), Some("#7a7a7a"));
    assert_eq!(doc.symbols.box_style.as_deref(), Some("rounded"));
    assert_eq!(doc.symbols.overrides["arrow.north"], "^");
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `parse_style_toml` and `style_from_config`** (parse via `toml::Value`; map inline tables to `Decl` field by field; ignore unknown selector keys at parse time — they’re warned later in resolve).
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): parse style TOML + config override layer".

---

### Task 6: Resolve a StyleDoc into (ColorScheme, SymbolSet)

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: everything above; `colors::{ColorScheme, GhosttyScheme}`, `colors::resolve`-style logic, `symbols::SymbolSet`.
- Produces:
  - `pub fn resolve(doc: &StyleDoc, dir: &std::path::Path) -> (ColorScheme, SymbolSet, Vec<String>)` — (1) build the base `ColorScheme` from `doc.colors.scheme` exactly as `colors::ColorScheme::resolve` does today for a scheme/built-in/path/none (reuse that code path); (2) obtain the active `GhosttyScheme` (or a default for terminal-default) for color-value palette refs; (3) `apply_color_decls(&mut cs, &doc.colors.selectors, &scheme)` to layer the CSS selectors on top, collecting warnings; (4) `SymbolSet::resolve(&finalize_symbols(&doc.symbols))`. Returns merged warnings (path/scheme warnings + unknown-selector warnings).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn resolve_terminal_default_with_selector_override() {
    use ratatui::style::Color;
    let mut doc = StyleDoc::default(); // no scheme => terminal default base
    doc.colors.selectors.insert("connector".into(), Decl { fg: Some("magenta".into()), ..Default::default() });
    let (cs, _set, warns) = resolve(&doc, std::path::Path::new("."));
    assert!(warns.is_empty());
    assert_eq!(cs.connector.fg, Some(Color::Magenta));
    // a field with no decl keeps the terminal-default value:
    let def = crate::colors::ColorScheme::terminal_default();
    assert_eq!(cs.transcript, def.transcript);
}

#[test]
fn resolve_empty_doc_equals_terminal_default() {
    let doc = StyleDoc::default();
    let (cs, set, _w) = resolve(&doc, std::path::Path::new("."));
    assert_eq!(cs, crate::colors::ColorScheme::terminal_default());
    assert_eq!(set, crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default()));
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `resolve`** — reuse `colors` scheme/path/built-in handling (refactor `colors.rs` to expose a helper that returns `(ColorScheme, GhosttyScheme, Vec<String>)` for a given scheme string + legacy elements, or call the existing `resolve` then re-derive the scheme; keep it DRY). Then apply decls and finalize symbols.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): resolve StyleDoc to ColorScheme + SymbolSet".

---

### Task 7: load_style (pointer resolution + built-in default + fallback)

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `parse_style_toml`, `StyleDoc`.
- Produces:
  - `pub const DEFAULT_STYLE_TOML: &str` — embedded built-in `default` style reproducing today's terminal look (empty `[colors]` with no scheme/selectors + default `[symbols]` presets is sufficient, since empty resolves to terminal default; include a comment header).
  - `pub fn load_style(pointer: Option<&str>, user_dir: &std::path::Path) -> (StyleDoc, Vec<String>)` — resolution order: `None` ⇒ if `user_dir/style.toml` exists, parse it; else parse `DEFAULT_STYLE_TOML`. `Some("default")` ⇒ `DEFAULT_STYLE_TOML`. `Some(path)` ⇒ `~`-expand + resolve relative to `user_dir`, read+parse; on missing/parse error push a warning and fall back to `DEFAULT_STYLE_TOML`. Never panics.

- [ ] **Step 1: Write the failing test** (use a `tempfile`-style temp dir as other fs tests do)
```rust
#[test]
fn load_style_default_name_parses_builtin() {
    let (doc, warns) = load_style(Some("default"), std::path::Path::new("/nonexistent"));
    assert!(warns.is_empty());
    let _ = doc; // parses without error
}

#[test]
fn load_style_missing_path_warns_and_falls_back() {
    let (doc, warns) = load_style(Some("/no/such/style.toml"), std::path::Path::new("/tmp"));
    assert_eq!(warns.len(), 1);
    assert_eq!(doc, parse_style_toml(DEFAULT_STYLE_TOML).unwrap());
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `DEFAULT_STYLE_TOML` + `load_style`** (mirror `colors.rs` path/tilde handling).
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): load_style pointer resolution + builtin default".

---

### Task 8: Writers — write_style + write_style_full (format-preserving, preserve unknown)

**Files:**
- Modify: `crates/app/src/style.rs`
- Test: in `style.rs` tests

**Interfaces:**
- Consumes: `StyleDoc`; `ColorScheme`, `SymbolSet` for the full export.
- Produces:
  - `pub fn write_style(path: &std::path::Path, doc: &StyleDoc) -> std::io::Result<()>` — load existing file with `toml_edit` (or new doc), write `scheme`, selector inline tables, legacy elements, and `[symbols]` presets/overrides; PRESERVE any other tables/keys/comments.
  - `pub fn write_style_full(path: &std::path::Path, cs: &ColorScheme, set: &SymbolSet) -> std::io::Result<()>` — write a fully-expanded, self-contained style: every selector (derived from each `ColorScheme` field via a `style_to_decl(Style) -> Decl` inverse) and every symbol preset/override currently in effect. Still preserves unknown tables.

- [ ] **Step 1: Write the failing test (round-trip + preserve unknown)**
```rust
#[test]
fn write_style_preserves_unknown_sections() {
    let dir = /* temp dir */;
    let path = dir.join("style.toml");
    std::fs::write(&path, "# my style\n[header]\ntitle = \"book\"\n").unwrap();
    let mut doc = StyleDoc::default();
    doc.colors.selectors.insert("connector".into(), Decl { fg: Some("cyan".into()), ..Default::default() });
    write_style(&path, &doc).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("[header]"));          // unknown section survived
    assert!(text.contains("title = \"book\""));
    // re-parse reflects the written selector
    let reparsed = parse_style_toml(&text).unwrap();
    assert_eq!(reparsed.colors.selectors["connector"].fg.as_deref(), Some("cyan"));
}

#[test]
fn write_style_full_is_self_contained() {
    let dir = /* temp dir */;
    let path = dir.join("full.toml");
    let cs = crate::colors::ColorScheme::terminal_default();
    let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
    write_style_full(&path, &cs, &set).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let doc = parse_style_toml(&text).unwrap();
    // resolving the exported doc with NO base reproduces the same scheme
    let (cs2, set2, _w) = resolve(&doc, dir.path());
    assert_eq!(cs2, cs);
    assert_eq!(set2, set);
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `write_style`, `write_style_full`, and `style_to_decl`** (inverse of `decl_to_style`: emit `fg`/`bg` as `#rrggbb` for `Color::Rgb`, index for `Indexed`, name for named; emit each modifier bool present). Follow the existing `config::write_symbols` toml_edit pattern for format preservation.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(style): format-preserving style writers + full export".

---

### Task 9: Config integration — `style` field + drop style writing

**Files:**
- Modify: `crates/app/src/config.rs`
- Test: in `config.rs` tests

**Interfaces:**
- Consumes: `Config`, `Config::resolve`, `write_config` (existing); `style::{StyleColors, StyleSymbols}` (Tasks 4, 3).
- Produces: `Config.style: Option<String>`; `Config.colors: StyleColors` and `Config.symbols: StyleSymbols` (replacing the old `ColorsConfig`/`SymbolConfig` so the config override layer uses the new format); `resolve` reads `style` from file; `write_config` no longer emits `[colors]`/`[symbols]`. (`config::SymbolConfig` stays as the concrete type that `finalize_symbols` produces for `SymbolSet::resolve`; `ColorsConfig` may be removed if no longer referenced.)

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn config_reads_style_pointer() {
    let cfg: Config = toml::from_str("style = \"neon\"\n").unwrap();
    assert_eq!(cfg.style.as_deref(), Some("neon"));
}

#[test]
fn write_config_does_not_emit_style_sections() {
    let dir = /* temp dir */;
    // seed a config with functional + a [keymap] to confirm preservation
    std::fs::write(dir.join("config.toml"), "auto_save = true\n[keymap]\nquit = \"q\"\n").unwrap();
    let mut cfg = Config::default();
    cfg.auto_save = true;
    write_config(dir.path(), &cfg).unwrap();
    let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
    assert!(!text.contains("[colors]"));
    assert!(!text.contains("[symbols]"));
    assert!(text.contains("[keymap]")); // functional sections preserved
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** — add `#[serde(default)] pub style: Option<String>` to `Config` and to `Default`; change `Config.colors` to `style::StyleColors` and `Config.symbols` to `style::StyleSymbols` (both `#[serde(default)]`, deserializing the new selector/preset format); copy `style`/`colors`/`symbols` in `resolve`’s file-merge block; remove the `[colors]`/`[symbols]` writing from `write_config` (leave the functional keys). Update any code that referenced the old `ColorsConfig`/`SymbolConfig` fields (mainly the startup resolution, replaced in Task 10).
- [ ] **Step 4: Run, confirm pass; full `cargo test --workspace` green; build clean.**
- [ ] **Step 5: Commit** — "feat(config): add style pointer; stop writing style sections".

---

### Task 10: Startup wiring + save/repoint + gallery export button

**Files:**
- Modify: `crates/app/src/main.rs`
- Modify: `crates/app/src/input.rs`
- Modify: `crates/app/src/render/gallery.rs`
- Test: a pure helper test in `style.rs` or `main.rs` for the repoint logic

**Interfaces:**
- Consumes: `style::{load_style, style_from_config, merge, resolve, write_style, write_style_full}`; `config::Config`.
- Produces:
  - `pub fn personal_style_path(user_dir: &Path) -> PathBuf` (= `user_dir/style.toml`) in `style.rs`.
  - `Action::GalleryExportStyle` (input.rs).
  - Startup: replace the current `state.symbols`/`state.colors` resolution (today `SymbolSet::resolve(&cfg.symbols)` + `ColorScheme::resolve(&cfg.colors, ...)`) with: `let (base, w1) = style::load_style(cfg.style.as_deref(), &cfg.user_dir); let over = style::style_from_config(&cfg.colors, &cfg.symbols); let (cs, set, w2) = style::resolve(&style::merge(&base, &over), &cfg.user_dir); state.colors = cs; state.symbols = set;` and surface `w1`+`w2` the same way color/keymap warnings are surfaced today.

- [ ] **Step 1: Write the failing test (repoint helper)** — extract the “save current style + repoint config” into a pure helper and test it:
```rust
// in style.rs
#[test]
fn personal_style_path_is_user_dir_style_toml() {
    let p = personal_style_path(std::path::Path::new("/home/u/.babelmap"));
    assert_eq!(p, std::path::Path::new("/home/u/.babelmap/style.toml"));
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement `personal_style_path`** and wire startup resolution in `main.rs` (as in Interfaces). Build + run existing tests.
- [ ] **Step 4: Implement gallery/config save path** — on gallery close and config save, instead of `config::write_symbols`/writing `[colors]` to config.toml: build the resolved `ColorScheme`+`SymbolSet` from the edited selections, call `style::write_style_full(personal_style_path(&user_dir), &cs, &set)`, set `state.config.style = Some(personal_style_path.to_string())` and persist that pointer via `config::write_config`. Re-resolve `state.colors`/`state.symbols`.
- [ ] **Step 5: Implement `GalleryExportStyle`** — add the action; in `render/gallery.rs` add a footer button + key hint ("Output all settings"); in `input.rs` route the key to `Action::GalleryExportStyle`; handler does the same `write_style_full` + repoint as Step 4 but on demand. Add/extend a gallery render test asserting the footer shows the new hint.
- [ ] **Step 6: Run full `cargo test --workspace`; confirm green + warning-clean. Manually sanity-check (note in report): launch with no `style` → today's look; gallery edit writes `~/.babelmap/style.toml` and sets `style`.**
- [ ] **Step 7: Commit** — "feat(style): wire style file at startup, gallery export + repoint".

---

## Self-Review

**Spec coverage:**
- Model/pointer/layers → Tasks 6, 7, 9, 10. ✅
- CSS-ish color format + fixed selectors → Tasks 1, 2, 5. ✅
- Symbols relocated (presets+overrides) → Tasks 3, 5. ✅
- Inline-include override (style file base ⊕ config override, present-keys-only) → Task 4 (merge) + Task 10 (wiring). ✅
- Unset-vs-default → Task 4 (partial `Option` model is the mechanism). ✅
- Config override sections use the new format (no legacy) → Task 9 (`Config.colors/symbols` become `StyleColors`/`StyleSymbols`). ✅
- Edit→personal file, fork+repoint → Task 10. ✅
- Gallery "Output all settings" export → Tasks 8 (`write_style_full`), 10 (button/action). ✅
- Writers preserve unknown sections (future border/header keys) → Task 8. ✅
- Built-in `default` → Task 7. ✅
- Default look (no pointer/override = today) → Task 6 (`resolve_empty_doc_equals_terminal_default`). ✅
- Never-crash on bad path/selector → Tasks 2, 7. ✅

**Placeholder scan:** No TBD/vague steps; each code step has concrete code or a concrete test. Temp-dir setup in fs tests should follow the existing pattern in `config.rs`/`persist_files.rs` tests (noted inline as `/* temp dir */`; implementer copies the established pattern).

**Type consistency:** `Decl`, `StyleColors{scheme,selectors,legacy_elements}`, `StyleSymbols{...,overrides}`, `StyleDoc{colors,symbols}`, and fn names (`decl_to_style`, `apply_color_decls`, `finalize_symbols`, `merge`, `parse_style_toml`, `style_from_config`, `resolve`, `load_style`, `write_style`, `write_style_full`, `style_to_decl`, `personal_style_path`) are used consistently across tasks.

## Notes for the executor

- One genuinely tricky area is keeping `colors.rs` DRY when `resolve` (Task 6) needs both the `ColorScheme` AND the active `GhosttyScheme`. Prefer refactoring `colors.rs` to expose an internal helper returning both, rather than duplicating the scheme/built-in/path match.
- The TUI wiring in Task 10 is the only partly-non-unit-testable task; lean on the pure helpers (`personal_style_path`, `write_style_full` round-trip) for coverage and verify the rest by reasoning + a gallery render-test.
