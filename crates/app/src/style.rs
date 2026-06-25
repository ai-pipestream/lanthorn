//! Style model: per-declaration color + modifier parsing.
//!
//! This module owns the partial/raw style representation used by the style-file
//! subsystem. A [`Decl`] is a single CSS-ish declaration block (one selector's
//! worth of properties). [`decl_to_style`] resolves it into a ratatui [`Style`].

use std::collections::BTreeMap;

use ratatui::style::{Modifier, Style};

use crate::colors::{self, ColorScheme, GhosttyScheme};
use crate::render::paneframe;

// ── Decl ──────────────────────────────────────────────────────────────────────

/// A partial style declaration: every field is `Option` so unset fields are
/// distinguished from explicitly set ones.
///
/// The `style` field is only meaningful for border selectors (`map_border`,
/// `story_border`, `status_header`, `input_line`); it is ignored for other selectors.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct Decl {
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub dim: Option<bool>,
    pub reversed: Option<bool>,
    /// Optional border-style name (e.g. `"picture-frame"`, `"single"`, etc.).
    /// Only interpreted for border selectors; ignored for others.
    #[serde(default)]
    pub style: Option<String>,
    /// Optional shadow flag. Only interpreted for the `dialog` selector.
    #[serde(default)]
    pub shadow: Option<bool>,
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
    "transcript:input",
    "transcript:meta",
    "transcript:warning",
    "transcript:location",
    "transcript:system",
    "warning_marker",
    "suggestion",
    "meta_marker",
    "helpbar",
    "map_border",
    "story_border",
    "story_title",
    "map_layer_tab",
    "map_layer_tab_active",
    "status_header",
    "input_line",
    "dialog",
    "dialog:title",
    "dialog:button",
    "dialog:button:active",
    "dialog:shadow",
    "upper_window",
    "upper_window_border",
    "sound_beep_high",
    "sound_beep_low",
    "loc_indicator",
];

// ── apply_color_decls ─────────────────────────────────────────────────────────

/// Apply a map of selector→[`Decl`] declarations onto a [`ColorScheme`].
///
/// For each known selector present in `decls`, patches the matching
/// `ColorScheme` field via `field = field.patch(decl_to_style(decl, scheme))`.
/// `border` with no variant is accepted and ignored (reserved, no warning).
/// For `map_border` and `story_border`, an optional `style` key in the `Decl`
/// also sets `cs.map_border_style`/`cs.story_border_style`.
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
            "transcript:input"    => cs.transcript_input = cs.transcript_input.patch(style),
            "transcript:meta"     => cs.transcript_meta = cs.transcript_meta.patch(style),
            "transcript:warning"  => cs.transcript_warning = cs.transcript_warning.patch(style),
            "transcript:location" => cs.transcript_location = cs.transcript_location.patch(style),
            "transcript:system"   => cs.transcript_system = cs.transcript_system.patch(style),
            "warning_marker"      => cs.warning_marker = cs.warning_marker.patch(style),
            "suggestion"         => cs.suggestion = cs.suggestion.patch(style),
            "meta_marker"        => cs.meta_marker = cs.meta_marker.patch(style),
            "helpbar"            => cs.help_bar = cs.help_bar.patch(style),
            "map_border" => {
                cs.map_border = cs.map_border.patch(style);
                if let Some(ref s) = decl.style {
                    cs.map_border_style = paneframe::parse_border_style(s);
                }
            }
            "story_border" => {
                cs.story_border = cs.story_border.patch(style);
                if let Some(ref s) = decl.style {
                    cs.story_border_style = paneframe::parse_border_style(s);
                }
            }
            "story_title"        => cs.story_title = cs.story_title.patch(style),
            "map_layer_tab"      => cs.map_layer_tab = cs.map_layer_tab.patch(style),
            "map_layer_tab_active" => cs.map_layer_tab_active = cs.map_layer_tab_active.patch(style),
            "status_header" => {
                cs.status_header = cs.status_header.patch(style);
                if let Some(ref s) = decl.style {
                    cs.status_header_style = paneframe::parse_border_style(s);
                }
            }
            "input_line" => {
                cs.input_line = cs.input_line.patch(style);
                if let Some(ref s) = decl.style {
                    cs.input_line_style = paneframe::parse_border_style(s);
                }
            }
            "dialog" => {
                cs.dialog = cs.dialog.patch(style);
                if let Some(ref s) = decl.style {
                    cs.dialog_box_style = paneframe::parse_border_style(s);
                }
                if let Some(shadow_on) = decl.shadow {
                    cs.dialog_shadow_on = shadow_on;
                }
            }
            "dialog:title"         => cs.dialog_title = cs.dialog_title.patch(style),
            "dialog:button"        => cs.dialog_button = cs.dialog_button.patch(style),
            "dialog:button:active" => cs.dialog_button_active = cs.dialog_button_active.patch(style),
            "dialog:shadow"        => cs.dialog_shadow = cs.dialog_shadow.patch(style),
            "upper_window"         => cs.upper_window = cs.upper_window.patch(style),
            "upper_window_border" => {
                cs.upper_window_border = cs.upper_window_border.patch(style);
                if let Some(ref s) = decl.style {
                    cs.virtual_window_border = paneframe::parse_border_style(s);
                }
            }
            "sound_beep_high"    => cs.sound_beep_high = cs.sound_beep_high.patch(style),
            "sound_beep_low"     => cs.sound_beep_low = cs.sound_beep_low.patch(style),
            "loc_indicator"      => cs.loc_indicator = cs.loc_indicator.patch(style),
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

impl<'de> serde::Deserialize<'de> for StyleColors {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // The `[colors]` section is a flat map: `scheme` is a string and every
        // other key is a selector whose value is a [`Decl`] inline table. We
        // deserialize into a tolerant intermediate that accepts either shape per
        // key, mirroring `parse_style_toml`.
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum SchemeOrDecl {
            Scheme(String),
            Decl(Decl),
        }

        let raw: BTreeMap<String, SchemeOrDecl> = BTreeMap::deserialize(deserializer)?;
        let mut out = StyleColors::default();
        for (key, val) in raw {
            if key == "scheme" {
                if let SchemeOrDecl::Scheme(s) = val {
                    out.scheme = Some(s);
                }
            } else if let SchemeOrDecl::Decl(d) = val {
                out.selectors.insert(key, d);
            }
            // Unknown shapes (e.g. non-string scheme) are ignored, never fatal.
        }
        Ok(out)
    }
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
    /// User story-styling rules from `[[transcript.rule]]`, in file order.
    pub transcript_rules: Vec<RawRule>,
    /// The status-bar block from `[statusbar]` / `[[statusbar.segment]]`.
    pub status_bar: RawStatusBar,
}

/// A raw (uncompiled) user transcript-styling rule from `[[transcript.rule]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawRule {
    /// The regex source string (from the rule's `match` key).
    pub pattern: String,
    /// The fg/bg/bold/italic style fields applied on a match.
    pub decl: Decl,
}

/// A raw (uncompiled) status-bar segment from `[[statusbar.segment]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSegment {
    /// Text template (literal text mixed with `{placeholder}` tokens).
    pub text: String,
    /// Cluster name: `left` | `center` | `right` (unknown → `left` at resolve).
    pub align: String,
    /// The fg/bg/bold/italic style fields for this segment.
    pub decl: Decl,
}

/// A raw `[statusbar]` block: optional frame + ordered segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawStatusBar {
    pub border: Option<String>,
    pub border_fg: Option<String>,
    pub segments: Vec<RawSegment>,
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

    let transcript_rules = if over.transcript_rules.is_empty() {
        base.transcript_rules.clone()
    } else {
        over.transcript_rules.clone()
    };

    let status_bar = RawStatusBar {
        border: over.status_bar.border.clone().or(base.status_bar.border.clone()),
        border_fg: over.status_bar.border_fg.clone().or(base.status_bar.border_fg.clone()),
        segments: if over.status_bar.segments.is_empty() {
            base.status_bar.segments.clone()
        } else {
            over.status_bar.segments.clone()
        },
    };

    StyleDoc {
        colors: StyleColors { scheme, selectors },
        symbols,
        transcript_rules,
        status_bar,
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
        style:     over.style.clone().or(base.style.clone()),
        shadow:    over.shadow.or(base.shadow),
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

    let mut transcript_rules: Vec<RawRule> = Vec::new();
    if let Some(toml::Value::Table(tr_table)) = root.get("transcript") {
        if let Some(toml::Value::Array(rules)) = tr_table.get("rule") {
            for item in rules {
                if let toml::Value::Table(rt) = item {
                    let pattern = rt
                        .get("match")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if pattern.is_empty() {
                        continue; // a rule with no `match` is skipped
                    }
                    let decl = parse_decl_from_table(rt);
                    transcript_rules.push(RawRule { pattern, decl });
                }
            }
        }
    }

    let mut status_bar = RawStatusBar::default();
    if let Some(toml::Value::Table(sb)) = root.get("statusbar") {
        status_bar.border = sb.get("border").and_then(toml::Value::as_str).map(str::to_string);
        status_bar.border_fg = sb.get("border_fg").and_then(toml::Value::as_str).map(str::to_string);
        if let Some(toml::Value::Array(segs)) = sb.get("segment") {
            for item in segs {
                if let toml::Value::Table(st) = item {
                    let text = st.get("text").and_then(toml::Value::as_str).unwrap_or("").to_string();
                    let align = st.get("align").and_then(toml::Value::as_str).unwrap_or("left").to_string();
                    let decl = parse_decl_from_table(st);
                    status_bar.segments.push(RawSegment { text, align, decl });
                }
            }
        }
    }

    Ok(StyleDoc { colors, symbols, transcript_rules, status_bar })
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
        style:     t.get("style").and_then(toml::Value::as_str).map(str::to_string),
        shadow:    t.get("shadow").and_then(toml::Value::as_bool),
    }
}

// ── style_from_config ─────────────────────────────────────────────────────────

/// Wrap already-parsed config-override sections into a [`StyleDoc`] for merging.
pub fn style_from_config(colors: &StyleColors, symbols: &StyleSymbols) -> StyleDoc {
    StyleDoc {
        colors: colors.clone(),
        symbols: symbols.clone(),
        transcript_rules: Vec::new(),
        status_bar: RawStatusBar::default(),
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

    // Compile user transcript rules; an invalid regex warns and is skipped.
    for r in &doc.transcript_rules {
        match regex::Regex::new(&r.pattern) {
            Ok(rx) => cs.transcript_rules.push(crate::colors::CompiledRule {
                pattern: r.pattern.clone(),
                regex: rx,
                style: decl_to_style(&r.decl, &gs),
            }),
            Err(e) => warnings.push(format!("invalid transcript rule regex '{}': {}", r.pattern, e)),
        }
    }

    // Compile the [statusbar] block. Segments replace the default layout only when
    // present; an empty block keeps the built-in default (today's bar).
    if !doc.status_bar.segments.is_empty() {
        let mut segments = Vec::with_capacity(doc.status_bar.segments.len());
        for raw in &doc.status_bar.segments {
            let align = match raw.align.as_str() {
                "left" => crate::colors::Align::Left,
                "center" => crate::colors::Align::Center,
                "right" => crate::colors::Align::Right,
                other => {
                    warnings.push(format!("unknown statusbar align '{}'; using left", other));
                    crate::colors::Align::Left
                }
            };
            segments.push(crate::colors::StatusSegment {
                text: raw.text.clone(),
                align,
                style: decl_to_style(&raw.decl, &gs),
            });
        }
        cs.statusbar_layout = crate::colors::StatusBarLayout { segments };
    }
    // The frame maps onto the existing status_header fields (reuses the boxing path).
    if let Some(b) = &doc.status_bar.border {
        cs.status_header_style = paneframe::parse_border_style(b);
    }
    if let Some(c) = &doc.status_bar.border_fg {
        if let Some(color) = colors::parse_color_value(c, &gs) {
            cs.status_header = cs.status_header.fg(color);
        }
    }

    // Step 4: resolve symbols.
    let set = crate::symbols::SymbolSet::resolve(&finalize_symbols(&doc.symbols));

    (cs, set, warnings)
}

// ── DEFAULT_STYLE_TOML ────────────────────────────────────────────────────────

/// The embedded built-in `default` style.
///
/// Sets a picture-frame map border and a single-line story border as the default look.
/// An empty `[symbols]` means all presets resolve to their factory defaults via finalize_symbols.
pub const DEFAULT_STYLE_TOML: &str = r#"# babelmap built-in default style
# map_border = picture-frame; story_border = single (titled book header without the
# ornate frame); other selectors use terminal defaults.
# Empty [symbols] means all presets resolve to their factory defaults via finalize_symbols.

[colors]
"map_border" = { style = "picture-frame" }
"story_border" = { style = "single" }
"dialog" = { style = "single", bg = "black" }
"dialog:title" = { fg = "cyan" }
"dialog:button" = { fg = "white" }
"dialog:button:active" = { fg = "black", bg = "cyan" }
"dialog:shadow" = { bg = "dark-gray" }

[symbols]
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

// ── personal_style_path ───────────────────────────────────────────────────────

/// The path to the user's personal style file: `user_dir/style.toml`.
///
/// This is the file written by gallery/config saves and the "Output all settings"
/// export; `config.style` is repointed at it so the saved look persists.
pub fn personal_style_path(user_dir: &std::path::Path) -> std::path::PathBuf {
    user_dir.join("style.toml")
}

// ── style_to_decl ─────────────────────────────────────────────────────────────

/// Inverse of [`decl_to_style`]: convert a ratatui [`Style`] into a [`Decl`].
///
/// Color encoding:
/// - `Color::Rgb(r,g,b)` → `"#rrggbb"` hex string.
/// - `Color::Indexed(n)` → decimal index string (e.g. `"17"`).
/// - Named colors (Black, Red, … White, DarkGray, Light*, Reset) → lowercase name.
/// - `None` (unset) → `None` in the Decl (field omitted from TOML output).
///
/// Modifier encoding: each modifier flag set in `add_modifier` becomes `Some(true)`.
///
/// Invariant: relies on `Style::patch` only ADDING modifiers (never removing), which holds
/// because every ColorScheme constructor carries REVERSED/BOLD modifiers on the relevant fields.
fn style_to_decl(s: &Style) -> Decl {
    Decl {
        fg: s.fg.map(color_to_str),
        bg: s.bg.map(color_to_str),
        bold: modifier_flag(s.add_modifier, Modifier::BOLD),
        italic: modifier_flag(s.add_modifier, Modifier::ITALIC),
        underline: modifier_flag(s.add_modifier, Modifier::UNDERLINED),
        dim: modifier_flag(s.add_modifier, Modifier::DIM),
        reversed: modifier_flag(s.add_modifier, Modifier::REVERSED),
        style: None,  // color-only inverse; callers set this for border selectors
        shadow: None, // callers set this for the dialog selector
    }
}

/// Encode a [`Color`] as a string suitable for a [`Decl`] fg/bg field.
fn color_to_str(c: ratatui::style::Color) -> String {
    use ratatui::style::Color::*;
    match c {
        Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        Indexed(n) => n.to_string(),
        Black => "black".to_string(),
        Red => "red".to_string(),
        Green => "green".to_string(),
        Yellow => "yellow".to_string(),
        Blue => "blue".to_string(),
        Magenta => "magenta".to_string(),
        Cyan => "cyan".to_string(),
        Gray => "gray".to_string(),
        White => "white".to_string(),
        DarkGray => "dark-gray".to_string(),
        LightRed => "light-red".to_string(),
        LightGreen => "light-green".to_string(),
        LightYellow => "light-yellow".to_string(),
        LightBlue => "light-blue".to_string(),
        LightMagenta => "light-magenta".to_string(),
        LightCyan => "light-cyan".to_string(),
        Reset => "reset".to_string(),
    }
}

/// Return `Some(true)` if `modifiers` contains `flag`, else `None`.
fn modifier_flag(modifiers: Modifier, flag: Modifier) -> Option<bool> {
    if modifiers.contains(flag) { Some(true) } else { None }
}

// ── write_style ───────────────────────────────────────────────────────────────

/// Write a [`StyleDoc`] to a TOML file at `path`, preserving existing content.
///
/// Uses `toml_edit` for format-preserving writes: existing tables, comments, and
/// unknown sections are left intact. Only the keys owned by the style model
/// (`[colors]` scheme + selectors, `[symbols]` presets + overrides) are written.
///
/// If the file does not exist it is created (parent directory must exist).
pub fn write_style(path: &std::path::Path, doc: &StyleDoc) -> std::io::Result<()> {
    // Load existing content or start fresh.
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut tdoc: toml_edit::DocumentMut = existing.parse().unwrap_or_default();

    // ── [colors] ──────────────────────────────────────────────────────────────
    {
        let colors = tdoc.entry("colors")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "[colors] is not a table"))?;

        // scheme key
        match &doc.colors.scheme {
            Some(s) => { colors["scheme"] = toml_edit::value(s.as_str()); }
            None    => { colors.remove("scheme"); }
        }

        // Remove selector keys that are no longer present (we rewrite all of them).
        // Collect first to avoid mutating while iterating.
        let existing_selector_keys: Vec<String> = colors.iter()
            .filter(|(k, _)| *k != "scheme")
            .map(|(k, _)| k.to_string())
            .collect();
        for k in &existing_selector_keys {
            colors.remove(k);
        }

        // Write each selector as an inline table.
        for (selector, decl) in &doc.colors.selectors {
            let mut itbl = toml_edit::InlineTable::new();
            if let Some(st) = &decl.style  { itbl.insert("style",     toml_edit::Value::from(st.as_str())); }
            if let Some(fg) = &decl.fg { itbl.insert("fg", toml_edit::Value::from(fg.as_str())); }
            if let Some(bg) = &decl.bg { itbl.insert("bg", toml_edit::Value::from(bg.as_str())); }
            if decl.bold      == Some(true) { itbl.insert("bold",      toml_edit::Value::from(true)); }
            if decl.italic    == Some(true) { itbl.insert("italic",    toml_edit::Value::from(true)); }
            if decl.underline == Some(true) { itbl.insert("underline", toml_edit::Value::from(true)); }
            if decl.dim       == Some(true) { itbl.insert("dim",       toml_edit::Value::from(true)); }
            if decl.reversed  == Some(true) { itbl.insert("reversed",  toml_edit::Value::from(true)); }
            if decl.shadow    == Some(true) { itbl.insert("shadow",    toml_edit::Value::from(true)); }
            colors[selector.as_str()] = toml_edit::Item::Value(toml_edit::Value::InlineTable(itbl));
        }
    }

    // ── [symbols] ─────────────────────────────────────────────────────────────
    {
        let symbols = tdoc.entry("symbols")
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "[symbols] is not a table"))?;

        // Presets (only write if set; remove if absent).
        macro_rules! write_preset {
            ($field:ident, $key:literal) => {
                match &doc.symbols.$field {
                    Some(v) => { symbols[$key] = toml_edit::value(v.as_str()); }
                    None    => { symbols.remove($key); }
                }
            };
        }
        write_preset!(box_style,    "box_style");
        write_preset!(arrow_set,    "arrow_set");
        write_preset!(portal_icons, "portal_icons");
        write_preset!(path_style,   "path_style");

        // [symbols.overrides] — get or create sub-table.
        if !doc.symbols.overrides.is_empty() {
            let overrides = symbols.entry("overrides")
                .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .ok_or_else(|| std::io::Error::new(
                    std::io::ErrorKind::InvalidData, "[symbols.overrides] is not a table"))?;
            for (k, v) in &doc.symbols.overrides {
                overrides[k.as_str()] = toml_edit::value(v.as_str());
            }
        }
    }

    std::fs::write(path, tdoc.to_string())
}

// ── write_style_full ──────────────────────────────────────────────────────────

/// Write a fully-expanded, self-contained style file.
///
/// Encodes every [`ColorScheme`] field as a selector declaration (using
/// [`style_to_decl`]) and every [`SymbolSet`] slot as an override so that
/// re-parsing and resolving with no base scheme reproduces the same
/// `ColorScheme`/`SymbolSet` exactly.
///
/// Still preserves unknown tables already present in the file.
pub fn write_style_full(
    path: &std::path::Path,
    cs: &ColorScheme,
    set: &crate::symbols::SymbolSet,
) -> std::io::Result<()> {
    // Build a StyleDoc with every selector populated.
    let mut doc = StyleDoc::default();

    // Color selectors (inverse mapping of apply_color_decls).
    doc.colors.selectors.insert("room".to_string(),              style_to_decl(&cs.room_normal));
    doc.colors.selectors.insert("room:current".to_string(),      style_to_decl(&cs.room_current));
    doc.colors.selectors.insert("room:selected".to_string(),     style_to_decl(&cs.room_selected));
    doc.colors.selectors.insert("connector".to_string(),         style_to_decl(&cs.connector));
    doc.colors.selectors.insert("connector:distorted".to_string(), style_to_decl(&cs.connector_distorted));
    doc.colors.selectors.insert("connector:portal".to_string(),  style_to_decl(&cs.portal_connector));
    doc.colors.selectors.insert("border:focused".to_string(),    style_to_decl(&cs.focused_border));
    doc.colors.selectors.insert("statusbar".to_string(),         style_to_decl(&cs.status_bar));
    doc.colors.selectors.insert("transcript".to_string(),        style_to_decl(&cs.transcript));
    doc.colors.selectors.insert("transcript:input".to_string(),    style_to_decl(&cs.transcript_input));
    doc.colors.selectors.insert("transcript:meta".to_string(),     style_to_decl(&cs.transcript_meta));
    doc.colors.selectors.insert("transcript:warning".to_string(),  style_to_decl(&cs.transcript_warning));
    doc.colors.selectors.insert("transcript:location".to_string(), style_to_decl(&cs.transcript_location));
    doc.colors.selectors.insert("transcript:system".to_string(),   style_to_decl(&cs.transcript_system));
    doc.colors.selectors.insert("warning_marker".to_string(),      style_to_decl(&cs.warning_marker));
    doc.colors.selectors.insert("suggestion".to_string(),        style_to_decl(&cs.suggestion));
    doc.colors.selectors.insert("meta_marker".to_string(),       style_to_decl(&cs.meta_marker));
    doc.colors.selectors.insert("helpbar".to_string(),           style_to_decl(&cs.help_bar));
    // New pane border/title/tab/header/input selectors.
    {
        let mut d = style_to_decl(&cs.map_border);
        d.style = Some(paneframe::border_style_name(cs.map_border_style).to_string());
        doc.colors.selectors.insert("map_border".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.story_border);
        d.style = Some(paneframe::border_style_name(cs.story_border_style).to_string());
        doc.colors.selectors.insert("story_border".to_string(), d);
    }
    doc.colors.selectors.insert("story_title".to_string(),        style_to_decl(&cs.story_title));
    doc.colors.selectors.insert("map_layer_tab".to_string(),      style_to_decl(&cs.map_layer_tab));
    doc.colors.selectors.insert("map_layer_tab_active".to_string(), style_to_decl(&cs.map_layer_tab_active));
    {
        let mut d = style_to_decl(&cs.status_header);
        if cs.status_header_style != paneframe::BorderStyle::None {
            d.style = Some(paneframe::border_style_name(cs.status_header_style).to_string());
        }
        doc.colors.selectors.insert("status_header".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.input_line);
        if cs.input_line_style != paneframe::BorderStyle::None {
            d.style = Some(paneframe::border_style_name(cs.input_line_style).to_string());
        }
        doc.colors.selectors.insert("input_line".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.dialog);
        d.style = Some(paneframe::border_style_name(cs.dialog_box_style).to_string());
        if cs.dialog_shadow_on {
            d.shadow = Some(true);
        }
        doc.colors.selectors.insert("dialog".to_string(), d);
    }
    doc.colors.selectors.insert("dialog:title".to_string(),         style_to_decl(&cs.dialog_title));
    doc.colors.selectors.insert("dialog:button".to_string(),        style_to_decl(&cs.dialog_button));
    doc.colors.selectors.insert("dialog:button:active".to_string(), style_to_decl(&cs.dialog_button_active));
    doc.colors.selectors.insert("dialog:shadow".to_string(),        style_to_decl(&cs.dialog_shadow));
    doc.colors.selectors.insert("upper_window".to_string(),         style_to_decl(&cs.upper_window));
    {
        let mut d = style_to_decl(&cs.upper_window_border);
        d.style = Some(paneframe::border_style_name(cs.virtual_window_border).to_string());
        doc.colors.selectors.insert("upper_window_border".to_string(), d);
    }
    doc.colors.selectors.insert("sound_beep_high".to_string(), style_to_decl(&cs.sound_beep_high));
    doc.colors.selectors.insert("sound_beep_low".to_string(),  style_to_decl(&cs.sound_beep_low));
    doc.colors.selectors.insert("loc_indicator".to_string(), style_to_decl(&cs.loc_indicator));

    // Symbol slots: use default preset names, then override every slot explicitly.
    // This guarantees round-trip fidelity regardless of which preset produced the set.
    doc.symbols.box_style    = Some(crate::config::default_box_style());
    doc.symbols.arrow_set    = Some(crate::config::default_arrow_set());
    doc.symbols.portal_icons = Some(crate::config::default_portal_icons());
    doc.symbols.path_style   = Some(crate::config::default_path_style());

    // Write every slot key so that overrides fully define the resolved SymbolSet.
    let ov = &mut doc.symbols.overrides;
    // Box styles (room variants)
    ov.insert("room.normal.tl".to_string(),   set.room_normal.tl.to_string());
    ov.insert("room.normal.tr".to_string(),   set.room_normal.tr.to_string());
    ov.insert("room.normal.bl".to_string(),   set.room_normal.bl.to_string());
    ov.insert("room.normal.br".to_string(),   set.room_normal.br.to_string());
    ov.insert("room.normal.h".to_string(),    set.room_normal.h.to_string());
    ov.insert("room.normal.v".to_string(),    set.room_normal.v.to_string());
    ov.insert("room.current.tl".to_string(),  set.room_current.tl.to_string());
    ov.insert("room.current.tr".to_string(),  set.room_current.tr.to_string());
    ov.insert("room.current.bl".to_string(),  set.room_current.bl.to_string());
    ov.insert("room.current.br".to_string(),  set.room_current.br.to_string());
    ov.insert("room.current.h".to_string(),   set.room_current.h.to_string());
    ov.insert("room.current.v".to_string(),   set.room_current.v.to_string());
    ov.insert("room.portal.tl".to_string(),   set.room_portal.tl.to_string());
    ov.insert("room.portal.tr".to_string(),   set.room_portal.tr.to_string());
    ov.insert("room.portal.bl".to_string(),   set.room_portal.bl.to_string());
    ov.insert("room.portal.br".to_string(),   set.room_portal.br.to_string());
    ov.insert("room.portal.h".to_string(),    set.room_portal.h.to_string());
    ov.insert("room.portal.v".to_string(),    set.room_portal.v.to_string());
    ov.insert("room.selected.tl".to_string(), set.room_selected.tl.to_string());
    ov.insert("room.selected.tr".to_string(), set.room_selected.tr.to_string());
    ov.insert("room.selected.bl".to_string(), set.room_selected.bl.to_string());
    ov.insert("room.selected.br".to_string(), set.room_selected.br.to_string());
    ov.insert("room.selected.h".to_string(),  set.room_selected.h.to_string());
    ov.insert("room.selected.v".to_string(),  set.room_selected.v.to_string());
    // Arrows
    ov.insert("arrow.north".to_string(), set.arrows.north.to_string());
    ov.insert("arrow.south".to_string(), set.arrows.south.to_string());
    ov.insert("arrow.east".to_string(),  set.arrows.east.to_string());
    ov.insert("arrow.west".to_string(),  set.arrows.west.to_string());
    ov.insert("arrow.ne".to_string(),    set.arrows.ne.to_string());
    ov.insert("arrow.nw".to_string(),    set.arrows.nw.to_string());
    ov.insert("arrow.se".to_string(),    set.arrows.se.to_string());
    ov.insert("arrow.sw".to_string(),    set.arrows.sw.to_string());
    // Path glyphs
    ov.insert("path.ew".to_string(),    set.path.ew.to_string());
    ov.insert("path.ns".to_string(),    set.path.ns.to_string());
    ov.insert("path.se".to_string(),    set.path.se.to_string());
    ov.insert("path.sw".to_string(),    set.path.sw.to_string());
    ov.insert("path.ne".to_string(),    set.path.ne.to_string());
    ov.insert("path.nw".to_string(),    set.path.nw.to_string());
    ov.insert("path.nse".to_string(),   set.path.nse.to_string());
    ov.insert("path.nsw".to_string(),   set.path.nsw.to_string());
    ov.insert("path.ews".to_string(),   set.path.ews.to_string());
    ov.insert("path.ewn".to_string(),   set.path.ewn.to_string());
    ov.insert("path.cross".to_string(), set.path.nesw.to_string());
    // Portal glyphs
    ov.insert("portal.up".to_string(),      set.portal.up.to_string());
    ov.insert("portal.down".to_string(),    set.portal.down.to_string());
    ov.insert("portal.in".to_string(),      set.portal.in_.to_string());
    ov.insert("portal.out".to_string(),     set.portal.out.to_string());
    ov.insert("portal.unknown".to_string(), set.portal.unknown.to_string());
    ov.insert("portal.path".to_string(),    set.portal.path.to_string());
    ov.insert("portal.marker".to_string(),  set.portal.marker.to_string());
    ov.insert("gutter.meta".to_string(),    set.meta_gutter.to_string());
    ov.insert("gutter.warning".to_string(), set.warning_gutter.to_string());

    write_style(path, &doc)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statusbar_block_parses_segments_and_border() {
        let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"

[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "cyan"
bold = true

[[statusbar.segment]]
text = "Score: {score}"
align = "right"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.status_bar.border.as_deref(), Some("single"));
        assert_eq!(doc.status_bar.border_fg.as_deref(), Some("cyan"));
        assert_eq!(doc.status_bar.segments.len(), 2);
        assert_eq!(doc.status_bar.segments[0].text, "{location}");
        assert_eq!(doc.status_bar.segments[0].align, "left");
        assert_eq!(doc.status_bar.segments[0].decl.fg.as_deref(), Some("cyan"));
        assert_eq!(doc.status_bar.segments[0].decl.bold, Some(true));
        assert_eq!(doc.status_bar.segments[1].align, "right");
    }

    #[test]
    fn resolve_statusbar_segments_border_and_align() {
        use crate::colors::Align;
        let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"
[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "yellow"
[[statusbar.segment]]
text = "{title}"
align = "center"
[[statusbar.segment]]
text = "{score}"
align = "bogus"
"##;
        let doc = parse_style_toml(text).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        // Three segments, with the unknown align defaulting to Left + a warning.
        assert_eq!(cs.statusbar_layout.segments.len(), 3);
        assert!(matches!(cs.statusbar_layout.segments[0].align, Align::Left));
        assert!(matches!(cs.statusbar_layout.segments[1].align, Align::Center));
        assert!(matches!(cs.statusbar_layout.segments[2].align, Align::Left));
        assert_eq!(cs.statusbar_layout.segments[0].style.fg, Some(ratatui::style::Color::Yellow));
        assert!(warnings.iter().any(|w| w.contains("align")), "unknown align warns: {warnings:?}");
        // border maps onto the existing status_header machinery.
        assert!(matches!(cs.status_header_style, crate::render::paneframe::BorderStyle::Single));
        assert_eq!(cs.status_header.fg, Some(ratatui::style::Color::Cyan));
    }

    #[test]
    fn resolve_no_statusbar_keeps_default_layout() {
        let (cs, _set, _w) = resolve(&StyleDoc::default(), std::path::Path::new("."));
        assert_eq!(cs.statusbar_layout, crate::colors::StatusBarLayout::default());
    }

    #[test]
    fn merge_replaces_statusbar_segments_when_override_has_any() {
        let mut base = StyleDoc::default();
        base.status_bar.segments.push(RawSegment { text: "a".into(), align: "left".into(), decl: Decl::default() });
        let mut over = StyleDoc::default();
        over.status_bar.segments.push(RawSegment { text: "b".into(), align: "right".into(), decl: Decl::default() });
        over.status_bar.border = Some("double".into());
        let m = merge(&base, &over);
        assert_eq!(m.status_bar.segments.len(), 1);
        assert_eq!(m.status_bar.segments[0].text, "b");
        assert_eq!(m.status_bar.border.as_deref(), Some("double"));
        // Empty override keeps base segments.
        let m2 = merge(&base, &StyleDoc::default());
        assert_eq!(m2.status_bar.segments[0].text, "a");
    }

    #[test]
    fn transcript_rules_parse_compile_in_order() {
        let text = r##"
[colors]
[[transcript.rule]]
match = "^>.*"
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"
fg = "red"
"##;
        let doc = parse_style_toml(text).unwrap();
        assert_eq!(doc.transcript_rules.len(), 2);
        assert_eq!(doc.transcript_rules[0].pattern, "^>.*");
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(cs.transcript_rules.len(), 2);
        assert!(cs.transcript_rules[0].regex.is_match("> go north"));
        assert!(cs.transcript_rules[1].regex.is_match("A lurking GRUE!"));
        use ratatui::style::Color;
        assert_eq!(cs.transcript_rules[0].style.fg, Some(Color::Magenta));
    }

    #[test]
    fn invalid_transcript_rule_warns_and_skips() {
        let text = r##"
[colors]
[[transcript.rule]]
match = "("
fg = "red"

[[transcript.rule]]
match = "ok"
fg = "green"
"##;
        let doc = parse_style_toml(text).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert_eq!(warnings.len(), 1, "exactly one invalid-regex warning: {warnings:?}");
        assert_eq!(cs.transcript_rules.len(), 1, "valid rule still loads");
        assert!(cs.transcript_rules[0].regex.is_match("ok"));
    }

    #[test]
    fn merge_replaces_transcript_rules_when_override_has_any() {
        let mut base = StyleDoc::default();
        base.transcript_rules.push(RawRule { pattern: "a".into(), decl: Decl::default() });
        let mut over = StyleDoc::default();
        over.transcript_rules.push(RawRule { pattern: "b".into(), decl: Decl::default() });
        let m = merge(&base, &over);
        assert_eq!(m.transcript_rules.len(), 1);
        assert_eq!(m.transcript_rules[0].pattern, "b");
        // Empty override keeps base rules.
        let m2 = merge(&base, &StyleDoc::default());
        assert_eq!(m2.transcript_rules[0].pattern, "a");
    }

    #[test]
    fn transcript_category_selectors_parse_and_apply() {
        let doc = parse_style_toml(
            "[colors]\n\
             \"transcript:input\" = { fg = \"green\" }\n\
             \"transcript:meta\" = { fg = \"blue\" }\n\
             \"transcript:warning\" = { fg = \"red\" }\n\
             \"transcript:location\" = { bold = true }\n\
             \"transcript:system\" = { fg = \"magenta\" }\n\
             \"warning_marker\" = { fg = \"red\" }\n"
        ).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert!(warnings.is_empty(), "{warnings:?}");
        use ratatui::style::{Color, Modifier};
        assert_eq!(cs.transcript_input.fg, Some(Color::Green));
        assert_eq!(cs.transcript_meta.fg, Some(Color::Blue));
        assert_eq!(cs.transcript_warning.fg, Some(Color::Red));
        assert!(cs.transcript_location.add_modifier.contains(Modifier::BOLD));
        assert_eq!(cs.transcript_system.fg, Some(Color::Magenta));
        assert_eq!(cs.warning_marker.fg, Some(Color::Red));
    }

    #[test]
    fn write_style_full_round_trips_transcript_categories() {
        use ratatui::style::Color;
        let dir = std::env::temp_dir().join(format!("babelmap-style-tcat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tcat.toml");
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.transcript_input = Style::new().fg(Color::Green);
        cs.transcript_warning = Style::new().fg(Color::Magenta);
        let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        write_style_full(&path, &cs, &set).unwrap();
        let doc = parse_style_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let (cs2, _set2, _w) = resolve(&doc, &dir);
        assert_eq!(cs2.transcript_input.fg, Some(Color::Green));
        assert_eq!(cs2.transcript_warning.fg, Some(Color::Magenta));
        let _ = std::fs::remove_dir_all(&dir);
    }

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
    fn loc_indicator_selector_parses() {
        let doc = parse_style_toml("[colors]\n\"loc_indicator\" = { fg = \"green\" }\n").unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(cs.loc_indicator.fg, Some(ratatui::style::Color::Green));
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

    #[test]
    fn write_style_preserves_unknown_sections() {
        let dir = std::env::temp_dir()
            .join(format!("babelmap-style-test-preserve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn personal_style_path_is_user_dir_style_toml() {
        let p = personal_style_path(std::path::Path::new("/home/u/.babelmap"));
        assert_eq!(p, std::path::Path::new("/home/u/.babelmap/style.toml"));
    }

    #[test]
    fn resolve_sets_border_style_and_default_is_picture_frame() {
        // default doc (DEFAULT_STYLE_TOML) => picture-frame map, single story
        let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
        let (cs, _set, _w) = resolve(&doc, std::path::Path::new("."));
        assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::PictureFrame));
        assert!(matches!(cs.story_border_style, crate::render::paneframe::BorderStyle::Single));
    }

    #[test]
    fn border_selector_reads_style_and_color() {
        let doc = parse_style_toml("[colors]\n\"map_border\" = { style = \"double\", fg = \"cyan\" }\n").unwrap();
        let (cs, _s, _w) = resolve(&doc, std::path::Path::new("."));
        assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::Double));
        assert_eq!(cs.map_border.fg, Some(ratatui::style::Color::Cyan));
    }

    #[test]
    fn write_style_full_is_self_contained() {
        let dir = std::env::temp_dir()
            .join(format!("babelmap-style-test-full-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full.toml");
        let cs = crate::colors::ColorScheme::terminal_default();
        let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        write_style_full(&path, &cs, &set).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = parse_style_toml(&text).unwrap();
        // resolving the exported doc with NO base reproduces the same scheme
        let (cs2, set2, _w) = resolve(&doc, &dir);
        assert_eq!(cs2, cs);
        assert_eq!(set2, set);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dialog_selectors_resolve_with_box_style_and_default() {
        let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
        let (cs,_s,_w) = resolve(&doc, std::path::Path::new("."));
        assert!(matches!(cs.dialog_box_style, crate::render::paneframe::BorderStyle::Single));
        let d2 = parse_style_toml("[colors]\n\"dialog\" = { style = \"double\", bg = \"black\" }\n\"dialog:button\" = { fg = \"cyan\" }\n").unwrap();
        let (cs2,_s,_w) = resolve(&d2, std::path::Path::new("."));
        assert!(matches!(cs2.dialog_box_style, crate::render::paneframe::BorderStyle::Double));
        assert_eq!(cs2.dialog_button.fg, Some(ratatui::style::Color::Cyan));
    }

    #[test]
    fn write_style_full_round_trips_non_none_border_styles() {
        use crate::render::paneframe::BorderStyle;

        let dir = std::env::temp_dir()
            .join(format!("babelmap-style-test-border-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("border-full.toml");

        // Build a ColorScheme with non-None border styles.
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.map_border_style   = BorderStyle::PictureFrame;
        cs.story_border_style = BorderStyle::Double;

        let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        write_style_full(&path, &cs, &set).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let doc = parse_style_toml(&text).unwrap();
        let (cs2, _set2, _w) = resolve(&doc, &dir);

        assert!(
            matches!(cs2.map_border_style, BorderStyle::PictureFrame),
            "map_border_style must survive write_style_full -> parse -> resolve; got {:?}",
            cs2.map_border_style
        );
        assert!(
            matches!(cs2.story_border_style, BorderStyle::Double),
            "story_border_style must survive write_style_full -> parse -> resolve; got {:?}",
            cs2.story_border_style
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_style_full_round_trips_dialog_shadow_and_box_style() {
        use crate::render::paneframe::BorderStyle;

        let dir = std::env::temp_dir()
            .join(format!("babelmap-style-test-shadow-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shadow-full.toml");

        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.dialog_shadow_on = true;
        cs.dialog_box_style = BorderStyle::Double;

        let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        write_style_full(&path, &cs, &set).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let doc = parse_style_toml(&text).unwrap();
        let (cs2, _set2, _w) = resolve(&doc, &dir);

        assert!(
            cs2.dialog_shadow_on,
            "dialog_shadow_on must survive write_style_full -> parse -> resolve"
        );
        assert!(
            matches!(cs2.dialog_box_style, BorderStyle::Double),
            "dialog_box_style must survive write_style_full -> parse -> resolve; got {:?}",
            cs2.dialog_box_style
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upper_window_selectors_parse_and_default() {
        // default border is single
        let (cs, _, _) = resolve(&parse_style_toml(DEFAULT_STYLE_TOML).unwrap(), std::path::Path::new("."));
        assert_eq!(cs.virtual_window_border, crate::render::paneframe::BorderStyle::Single);
        // selector applies fg
        let doc = parse_style_toml("[colors]\n\"upper_window\" = { fg = \"cyan\" }\n").unwrap();
        let (cs2, _, _) = resolve(&doc, std::path::Path::new("."));
        assert_eq!(cs2.upper_window.fg, Some(ratatui::style::Color::Cyan));
    }

    #[test]
    fn sound_beep_selectors_parse_and_apply() {
        let doc = parse_style_toml(
            "[colors]\n\"sound_beep_high\" = { fg = \"red\" }\n\"sound_beep_low\" = { fg = \"blue\" }\n"
        ).unwrap();
        let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
        assert!(warnings.is_empty(), "known selectors must not warn: {warnings:?}");
        assert_eq!(cs.sound_beep_high.fg, Some(ratatui::style::Color::Red));
        assert_eq!(cs.sound_beep_low.fg, Some(ratatui::style::Color::Blue));
    }
}
