//! Style model: per-declaration color + modifier parsing.
//!
//! This module owns the partial/raw style representation used by the style-file
//! subsystem. A [`Decl`] is a single CSS-ish declaration block (one selector's
//! worth of properties). [`decl_to_style`] resolves it into a ratatui [`Style`].

use std::collections::BTreeMap;

use ratatui::style::{Modifier, Style};

use crate::colors::{self, ColorScheme, GhosttyScheme};

// ── Decl ──────────────────────────────────────────────────────────────────────

/// A partial style declaration: every field is `Option` so unset fields are
/// distinguished from explicitly set ones.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct Decl {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub dim: Option<bool>,
    pub reversed: Option<bool>,
}

// ── decl_to_style ─────────────────────────────────────────────────────────────

/// Convert a [`Decl`] into a ratatui [`Style`].
///
/// - `fg`/`bg` are parsed via [`colors::parse_color_value`].
/// - Each modifier bool adds its modifier when `Some(true)`.
pub fn decl_to_style(d: &Decl, scheme: &colors::GhosttyScheme) -> Style {
    let mut s = Style::new();

    if let Some(ref fg_str) = d.fg {
        if let Some(c) = colors::parse_color_value(fg_str, scheme) {
            s = s.fg(c);
        }
    }

    if let Some(ref bg_str) = d.bg {
        if let Some(c) = colors::parse_color_value(bg_str, scheme) {
            s = s.bg(c);
        }
    }

    if d.bold == Some(true) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if d.italic == Some(true) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if d.underline == Some(true) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    if d.dim == Some(true) {
        s = s.add_modifier(Modifier::DIM);
    }
    if d.reversed == Some(true) {
        s = s.add_modifier(Modifier::REVERSED);
    }

    s
}

// ── StyleSymbols ──────────────────────────────────────────────────────────────

/// Partial symbol configuration from a style file.
///
/// Every preset field is `Option` so unset fields are distinguished from
/// explicitly set ones. [`finalize_symbols`] fills `None` fields with the
/// existing `config::default_*` values to produce a concrete [`config::SymbolConfig`].
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct StyleSymbols {
    pub box_style: Option<String>,
    pub arrow_set: Option<String>,
    pub portal_icons: Option<String>,
    pub path_style: Option<String>,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

// ── finalize_symbols ──────────────────────────────────────────────────────────

/// Resolve a partial [`StyleSymbols`] into a concrete [`config::SymbolConfig`].
///
/// Each `None` preset is filled with the existing `config::default_*` value.
/// The `overrides` map is copied as-is.
pub fn finalize_symbols(s: &StyleSymbols) -> crate::config::SymbolConfig {
    crate::config::SymbolConfig {
        box_style: s.box_style.clone().unwrap_or_else(crate::config::default_box_style),
        arrow_set: s.arrow_set.clone().unwrap_or_else(crate::config::default_arrow_set),
        portal_icons: s.portal_icons.clone().unwrap_or_else(crate::config::default_portal_icons),
        path_style: s.path_style.clone().unwrap_or_else(crate::config::default_path_style),
        overrides: s.overrides.clone(),
    }
}

// ── SELECTOR_FIELDS ───────────────────────────────────────────────────────────

/// The recognized CSS-ish selectors for color declarations.
pub const SELECTOR_FIELDS: &[&str] = &[
    "room",
    "room:current",
    "room:selected",
    "connector",
    "connector:distorted",
    "connector:portal",
    "border",
    "border:focused",
    "statusbar",
    "transcript",
    "suggestion",
    "helpbar",
];

// ── apply_color_decls ─────────────────────────────────────────────────────────

/// Apply a map of selector→[`Decl`] declarations onto a [`ColorScheme`].
///
/// For each known selector present in `decls`, patches the matching
/// `ColorScheme` field via `field = field.patch(decl_to_style(decl, scheme))`.
/// `border` with no variant is accepted and ignored (reserved, no warning).
/// Unknown selectors are collected into the returned warnings vec.
pub fn apply_color_decls(
    cs: &mut ColorScheme,
    decls: &BTreeMap<String, Decl>,
    scheme: &GhosttyScheme,
) -> Vec<String> {
    let mut warnings = Vec::new();

    for (selector, decl) in decls {
        let style = decl_to_style(decl, scheme);
        match selector.as_str() {
            "room"               => cs.room_normal = cs.room_normal.patch(style),
            "room:current"       => cs.room_current = cs.room_current.patch(style),
            "room:selected"      => cs.room_selected = cs.room_selected.patch(style),
            "connector"          => cs.connector = cs.connector.patch(style),
            "connector:distorted"=> cs.connector_distorted = cs.connector_distorted.patch(style),
            "connector:portal"   => cs.portal_connector = cs.portal_connector.patch(style),
            "border"             => {} // reserved, accepted silently
            "border:focused"     => cs.focused_border = cs.focused_border.patch(style),
            "statusbar"          => cs.status_bar = cs.status_bar.patch(style),
            "transcript"         => cs.transcript = cs.transcript.patch(style),
            "suggestion"         => cs.suggestion = cs.suggestion.patch(style),
            "helpbar"            => cs.help_bar = cs.help_bar.patch(style),
            _                    => warnings.push(format!("unknown selector: {}", selector)),
        }
    }

    warnings
}

// ── StyleColors ───────────────────────────────────────────────────────────────

/// Partial color configuration from a style file.
///
/// `scheme` is the optional named color scheme (e.g. `"tomorrow-night"`).
/// `selectors` maps CSS-ish selector names to their [`Decl`] blocks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleColors {
    pub scheme: Option<String>,
    pub selectors: BTreeMap<String, Decl>,
}

// ── StyleDoc ──────────────────────────────────────────────────────────────────

/// A complete (but partial/raw) style document combining color and symbol config.
///
/// Every field uses `Option` or `BTreeMap` so absent fields are distinguished
/// from explicitly set ones. [`merge`] combines two `StyleDoc`s with
/// present-keys-only semantics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleDoc {
    pub colors: StyleColors,
    pub symbols: StyleSymbols,
}

// ── merge ─────────────────────────────────────────────────────────────────────

/// Merge two [`StyleDoc`]s with present-keys-only semantics.
///
/// - `colors.scheme`: `over` wins if set, otherwise `base`.
/// - `colors.selectors`: union of keys; for a key in both, the `over` [`Decl`]
///   is field-merged onto the `base` [`Decl`] (each `Option` field: `over.or(base)`).
/// - `symbols` presets: `over.or(base)` per field.
/// - `symbols.overrides`: union of keys, `over` wins per key.
pub fn merge(base: &StyleDoc, over: &StyleDoc) -> StyleDoc {
    // colors.scheme
    let scheme = over.colors.scheme.clone().or(base.colors.scheme.clone());

    // colors.selectors: base ∪ over, with field-level merge for shared keys
    let mut selectors = base.colors.selectors.clone();
    for (key, over_decl) in &over.colors.selectors {
        let merged = if let Some(base_decl) = selectors.get(key) {
            merge_decl(base_decl, over_decl)
        } else {
            over_decl.clone()
        };
        selectors.insert(key.clone(), merged);
    }

    // symbols presets: over wins if set
    let symbols = StyleSymbols {
        box_style: over.symbols.box_style.clone().or(base.symbols.box_style.clone()),
        arrow_set: over.symbols.arrow_set.clone().or(base.symbols.arrow_set.clone()),
        portal_icons: over.symbols.portal_icons.clone().or(base.symbols.portal_icons.clone()),
        path_style: over.symbols.path_style.clone().or(base.symbols.path_style.clone()),
        overrides: {
            let mut ov = base.symbols.overrides.clone();
            ov.extend(over.symbols.overrides.clone());
            ov
        },
    };

    StyleDoc {
        colors: StyleColors { scheme, selectors },
        symbols,
    }
}

/// Field-level merge of two [`Decl`]s: for each `Option` field, `over` wins if set.
fn merge_decl(base: &Decl, over: &Decl) -> Decl {
    Decl {
        fg:        over.fg.clone().or(base.fg.clone()),
        bg:        over.bg.clone().or(base.bg.clone()),
        bold:      over.bold.or(base.bold),
        italic:    over.italic.or(base.italic),
        underline: over.underline.or(base.underline),
        dim:       over.dim.or(base.dim),
        reversed:  over.reversed.or(base.reversed),
    }
}

// ── parse_style_toml ─────────────────────────────────────────────────────────

/// Parse a style document from TOML text.
///
/// Accepts the format used by BOTH style files and `config.toml` override sections:
/// - `[colors]` with optional `scheme` string and selector keys as inline tables
///   (e.g. `"room:current" = { reversed = true }`).
/// - `[symbols]` with optional preset string keys and a `[symbols.overrides]` table.
///
/// Unknown keys are ignored. Returns `Err(msg)` on TOML parse failure.
pub fn parse_style_toml(text: &str) -> Result<StyleDoc, String> {
    let root: toml::Value = text.parse().map_err(|e| format!("TOML parse error: {e}"))?;

    let mut colors = StyleColors::default();
    let mut symbols = StyleSymbols::default();

    if let Some(toml::Value::Table(colors_table)) = root.get("colors") {
        for (key, val) in colors_table {
            if key == "scheme" {
                if let Some(s) = val.as_str() {
                    colors.scheme = Some(s.to_string());
                }
            } else if let toml::Value::Table(decl_table) = val {
                // Each non-scheme key whose value is a table is a selector decl.
                let decl = parse_decl_from_table(decl_table);
                colors.selectors.insert(key.clone(), decl);
            }
            // Non-table, non-scheme keys are ignored (forward-compat).
        }
    }

    if let Some(toml::Value::Table(sym_table)) = root.get("symbols") {
        for (key, val) in sym_table {
            match key.as_str() {
                "box_style"    => symbols.box_style    = val.as_str().map(str::to_string),
                "arrow_set"    => symbols.arrow_set    = val.as_str().map(str::to_string),
                "portal_icons" => symbols.portal_icons = val.as_str().map(str::to_string),
                "path_style"   => symbols.path_style   = val.as_str().map(str::to_string),
                "overrides" => {
                    if let toml::Value::Table(ov) = val {
                        for (ok, ov_val) in ov {
                            if let Some(s) = ov_val.as_str() {
                                symbols.overrides.insert(ok.clone(), s.to_string());
                            }
                        }
                    }
                }
                _ => {} // unknown symbol keys ignored
            }
        }
    }

    Ok(StyleDoc { colors, symbols })
}

/// Parse a [`Decl`] from a TOML inline table (field-by-field).
fn parse_decl_from_table(t: &toml::value::Table) -> Decl {
    Decl {
        fg:        t.get("fg").and_then(toml::Value::as_str).map(str::to_string),
        bg:        t.get("bg").and_then(toml::Value::as_str).map(str::to_string),
        bold:      t.get("bold").and_then(toml::Value::as_bool),
        italic:    t.get("italic").and_then(toml::Value::as_bool),
        underline: t.get("underline").and_then(toml::Value::as_bool),
        dim:       t.get("dim").and_then(toml::Value::as_bool),
        reversed:  t.get("reversed").and_then(toml::Value::as_bool),
    }
}

// ── style_from_config ─────────────────────────────────────────────────────────

/// Wrap already-parsed config-override sections into a [`StyleDoc`] for merging.
pub fn style_from_config(colors: &StyleColors, symbols: &StyleSymbols) -> StyleDoc {
    StyleDoc {
        colors: colors.clone(),
        symbols: symbols.clone(),
    }
}

// ── resolve ───────────────────────────────────────────────────────────────────

/// Resolve a [`StyleDoc`] into a concrete [`ColorScheme`], [`SymbolSet`], and warnings.
///
/// Resolution:
/// 1. Build the base `ColorScheme` from `doc.colors.scheme` via [`colors::resolve_base`]
///    (handles `None` → terminal-default, built-in name, or file path).
/// 2. Obtain the active `GhosttyScheme` returned by `resolve_base` (or
///    `GhosttyScheme::default()` for the terminal-default case).
/// 3. Apply `doc.colors.selectors` on top via [`apply_color_decls`], collecting
///    unknown-selector warnings.
/// 4. Resolve symbols via `SymbolSet::resolve(&finalize_symbols(&doc.symbols))`.
///
/// Returns all warnings: base-scheme path/parse warnings ++ unknown-selector warnings.
pub fn resolve(
    doc: &StyleDoc,
    dir: &std::path::Path,
) -> (ColorScheme, crate::symbols::SymbolSet, Vec<String>) {
    // Step 1+2: build base ColorScheme and get the active GhosttyScheme.
    let (mut cs, gs, mut warnings) =
        colors::resolve_base(doc.colors.scheme.as_deref(), dir);

    // Step 3: layer CSS selectors on top.
    let selector_warnings = apply_color_decls(&mut cs, &doc.colors.selectors, &gs);
    warnings.extend(selector_warnings);

    // Step 4: resolve symbols.
    let set = crate::symbols::SymbolSet::resolve(&finalize_symbols(&doc.symbols));

    (cs, set, warnings)
}

// ── DEFAULT_STYLE_TOML ────────────────────────────────────────────────────────

/// The embedded built-in `default` style.
///
/// Reproduces the terminal-default look: empty `[colors]` (no scheme, no selectors)
/// and the default symbol presets. An empty StyleDoc resolves to terminal defaults
/// (see Task 6), so this constant is the canonical baseline.
pub const DEFAULT_STYLE_TOML: &str = r#"# babelmap built-in default style
# Empty [colors] means no scheme and no selector overrides => terminal defaults.
# Symbol presets list the factory defaults; override any preset or individual symbol below.

[colors]

[symbols]
box_style = "rounded"
arrow_set = "filled"
portal_icons = "ascii"
path_style = "light"
"#;

// ── load_style ────────────────────────────────────────────────────────────────

/// Load a [`StyleDoc`] according to a pointer string.
///
/// Resolution order:
/// - `None` — if `user_dir/style.toml` exists, read and parse it; else parse
///   [`DEFAULT_STYLE_TOML`].
/// - `Some("default")` — always parse [`DEFAULT_STYLE_TOML`].
/// - `Some(path)` — `~`-expand and resolve relative to `user_dir`; read and parse
///   the file. On missing file or parse error, push exactly one warning string and
///   fall back to [`DEFAULT_STYLE_TOML`].
///
/// Never panics.
pub fn load_style(
    pointer: Option<&str>,
    user_dir: &std::path::Path,
) -> (StyleDoc, Vec<String>) {
    let default_doc = || parse_style_toml(DEFAULT_STYLE_TOML).expect("DEFAULT_STYLE_TOML must parse");

    match pointer {
        None => {
            let candidate = user_dir.join("style.toml");
            if candidate.is_file() {
                match std::fs::read_to_string(&candidate) {
                    Ok(text) => match parse_style_toml(&text) {
                        Ok(doc) => return (doc, Vec::new()),
                        Err(e) => {
                            let warn = format!(
                                "could not parse style file '{}': {}; using built-in default",
                                candidate.display(),
                                e
                            );
                            return (default_doc(), vec![warn]);
                        }
                    },
                    Err(e) => {
                        let warn = format!(
                            "could not read style file '{}': {}; using built-in default",
                            candidate.display(),
                            e
                        );
                        return (default_doc(), vec![warn]);
                    }
                }
            }
            (default_doc(), Vec::new())
        }
        Some("default") => (default_doc(), Vec::new()),
        Some(path_str) => {
            let path = colors::expand_path(path_str, user_dir);
            match std::fs::read_to_string(&path) {
                Ok(text) => match parse_style_toml(&text) {
                    Ok(doc) => (doc, Vec::new()),
                    Err(e) => {
                        let warn = format!(
                            "could not parse style file '{}': {}; using built-in default",
                            path.display(),
                            e
                        );
                        (default_doc(), vec![warn])
                    }
                },
                Err(e) => {
                    let warn = format!(
                        "could not read style file '{}': {}; using built-in default",
                        path.display(),
                        e
                    );
                    (default_doc(), vec![warn])
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn parse_style_toml_reads_selectors_scheme_symbols() {
        let text = r##"
[colors]
scheme = "tomorrow-night"
"room" = { fg = "white" }
"room:current" = { reversed = true }
"suggestion" = { fg = "#7a7a7a" }

[symbols]
box_style = "rounded"
[symbols.overrides]
"arrow.north" = "^"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.colors.scheme.as_deref(), Some("tomorrow-night"));
        assert_eq!(doc.colors.selectors["room"].fg.as_deref(), Some("white"));
        assert_eq!(doc.colors.selectors["room:current"].reversed, Some(true));
        assert_eq!(doc.colors.selectors["suggestion"].fg.as_deref(), Some("#7a7a7a"));
        assert_eq!(doc.symbols.box_style.as_deref(), Some("rounded"));
        assert_eq!(doc.symbols.overrides["arrow.north"], "^");
    }

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
}
