//! Style model: per-declaration color + modifier parsing.
//!
//! This module owns the partial/raw style representation used by the style-file
//! subsystem. A [`Decl`] is a single CSS-ish declaration block (one selector's
//! worth of properties). [`decl_to_style`] resolves it into a ratatui [`Style`].

use std::collections::BTreeMap;

use ratatui::style::{Modifier, Style};

use crate::colors::{self, ColorScheme};
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
    /// Optional border-style name (e.g. `"single"`, `"double"`, etc.).
    /// Only interpreted for border selectors; ignored for others.
    #[serde(default)]
    pub style: Option<String>,
    /// Per-side border overrides (border selectors only): each names a line style
    /// (none/single/double/thick). A side falls back to `style` when unset.
    #[serde(default)]
    pub style_top: Option<String>,
    #[serde(default)]
    pub style_bottom: Option<String>,
    #[serde(default)]
    pub style_left: Option<String>,
    #[serde(default)]
    pub style_right: Option<String>,
    /// Whether the pane's header strip is shown (story_border / map_border only).
    #[serde(default)]
    pub header: Option<bool>,
    /// Optional shadow flag. Only interpreted for the `dialog` selector.
    #[serde(default)]
    pub shadow: Option<bool>,
    /// Optional placement token (center/top/bottom/left/right/corners). Only
    /// interpreted for the `dialog` selector.
    #[serde(default)]
    pub placement: Option<String>,
    /// Optional placement margin (cells from the anchored edge). Only interpreted
    /// for the `dialog` selector.
    #[serde(default)]
    pub margin: Option<u16>,
    /// Per-side/corner glyph overrides (border selectors only).
    #[serde(default)]
    pub glyph_top: Option<String>,
    #[serde(default)]
    pub glyph_bottom: Option<String>,
    #[serde(default)]
    pub glyph_left: Option<String>,
    #[serde(default)]
    pub glyph_right: Option<String>,
    #[serde(default)]
    pub glyph_tl: Option<String>,
    #[serde(default)]
    pub glyph_tr: Option<String>,
    #[serde(default)]
    pub glyph_bl: Option<String>,
    #[serde(default)]
    pub glyph_br: Option<String>,
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
/// existing `config::default_*` values to produce a concrete [`config::SymbolConfig`](crate::config::SymbolConfig).
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct StyleSymbols {
    pub box_style: Option<String>,
    pub arrow_set: Option<String>,
    pub portal_icons: Option<String>,
    pub path_style: Option<String>,
    pub badge_zcode: Option<String>,
    pub badge_glulx: Option<String>,
    pub badge_blorb: Option<String>,
    pub badge_save: Option<String>,
    pub badge_hint: Option<String>,
    /// Draw diagonal stubs out of room corners for ne/nw/se/sw exits (SQ-0314).
    /// `None` → the config default (on). Set false for a font without Unicode 13
    /// Legacy Computing coverage.
    pub diagonal_corners: Option<bool>,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

// ── finalize_symbols ──────────────────────────────────────────────────────────

/// Resolve a partial [`StyleSymbols`] into a concrete [`config::SymbolConfig`](crate::config::SymbolConfig).
///
/// Each `None` preset is filled with the existing `config::default_*` value.
/// The `overrides` map is copied as-is.
pub fn finalize_symbols(s: &StyleSymbols) -> crate::config::SymbolConfig {
    crate::config::SymbolConfig {
        box_style: s.box_style.clone().unwrap_or_else(crate::config::default_box_style),
        arrow_set: s.arrow_set.clone().unwrap_or_else(crate::config::default_arrow_set),
        portal_icons: s.portal_icons.clone().unwrap_or_else(crate::config::default_portal_icons),
        path_style: s.path_style.clone().unwrap_or_else(crate::config::default_path_style),
        badge_zcode: s.badge_zcode.clone().unwrap_or_else(crate::config::default_badge_zcode),
        badge_glulx: s.badge_glulx.clone().unwrap_or_else(crate::config::default_badge_glulx),
        badge_blorb: s.badge_blorb.clone().unwrap_or_else(crate::config::default_badge_blorb),
        badge_save: s.badge_save.clone().unwrap_or_else(crate::config::default_badge_save),
        badge_hint: s.badge_hint.clone().unwrap_or_else(crate::config::default_badge_hint),
        diagonal_corners: s.diagonal_corners.unwrap_or_else(crate::config::default_diagonal_corners),
        overrides: s.overrides.clone(),
    }
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
            Decl(Box<Decl>),
        }

        let raw: BTreeMap<String, SchemeOrDecl> = BTreeMap::deserialize(deserializer)?;
        let mut out = StyleColors::default();
        for (key, val) in raw {
            if key == "scheme" {
                if let SchemeOrDecl::Scheme(s) = val {
                    out.scheme = Some(s);
                }
            } else if let SchemeOrDecl::Decl(d) = val {
                out.selectors.insert(key, *d);
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
        badge_zcode: over.symbols.badge_zcode.clone().or(base.symbols.badge_zcode.clone()),
        badge_glulx: over.symbols.badge_glulx.clone().or(base.symbols.badge_glulx.clone()),
        badge_blorb: over.symbols.badge_blorb.clone().or(base.symbols.badge_blorb.clone()),
        badge_save: over.symbols.badge_save.clone().or(base.symbols.badge_save.clone()),
        badge_hint: over.symbols.badge_hint.clone().or(base.symbols.badge_hint.clone()),
        diagonal_corners: over.symbols.diagonal_corners.or(base.symbols.diagonal_corners),
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
        style_top:    over.style_top.clone().or(base.style_top.clone()),
        style_bottom: over.style_bottom.clone().or(base.style_bottom.clone()),
        style_left:   over.style_left.clone().or(base.style_left.clone()),
        style_right:  over.style_right.clone().or(base.style_right.clone()),
        header:       over.header.or(base.header),
        shadow:    over.shadow.or(base.shadow),
        placement: over.placement.clone().or(base.placement.clone()),
        margin:    over.margin.or(base.margin),
        glyph_top:    over.glyph_top.clone().or(base.glyph_top.clone()),
        glyph_bottom: over.glyph_bottom.clone().or(base.glyph_bottom.clone()),
        glyph_left:   over.glyph_left.clone().or(base.glyph_left.clone()),
        glyph_right:  over.glyph_right.clone().or(base.glyph_right.clone()),
        glyph_tl:     over.glyph_tl.clone().or(base.glyph_tl.clone()),
        glyph_tr:     over.glyph_tr.clone().or(base.glyph_tr.clone()),
        glyph_bl:     over.glyph_bl.clone().or(base.glyph_bl.clone()),
        glyph_br:     over.glyph_br.clone().or(base.glyph_br.clone()),
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
        style_top:    t.get("style_top").and_then(toml::Value::as_str).map(str::to_string),
        style_bottom: t.get("style_bottom").and_then(toml::Value::as_str).map(str::to_string),
        style_left:   t.get("style_left").and_then(toml::Value::as_str).map(str::to_string),
        style_right:  t.get("style_right").and_then(toml::Value::as_str).map(str::to_string),
        header:       t.get("header").and_then(toml::Value::as_bool),
        shadow:    t.get("shadow").and_then(toml::Value::as_bool),
        placement: t.get("placement").and_then(toml::Value::as_str).map(str::to_string),
        margin:    t.get("margin").and_then(toml::Value::as_integer).map(|n| n as u16),
        glyph_top:    t.get("glyph_top").and_then(toml::Value::as_str).map(str::to_string),
        glyph_bottom: t.get("glyph_bottom").and_then(toml::Value::as_str).map(str::to_string),
        glyph_left:   t.get("glyph_left").and_then(toml::Value::as_str).map(str::to_string),
        glyph_right:  t.get("glyph_right").and_then(toml::Value::as_str).map(str::to_string),
        glyph_tl:     t.get("glyph_tl").and_then(toml::Value::as_str).map(str::to_string),
        glyph_tr:     t.get("glyph_tr").and_then(toml::Value::as_str).map(str::to_string),
        glyph_bl:     t.get("glyph_bl").and_then(toml::Value::as_str).map(str::to_string),
        glyph_br:     t.get("glyph_br").and_then(toml::Value::as_str).map(str::to_string),
    }
}

// ── resolve ───────────────────────────────────────────────────────────────────

/// Resolve a [`StyleDoc`] into a concrete [`ColorScheme`], [`SymbolSet`](crate::symbols::SymbolSet), and warnings.
///
/// Resolution:
/// 1. Build the base `ColorScheme` from `doc.colors.scheme` via `colors::resolve_base`
///    (handles `None` → terminal-default, built-in name, or file path).
/// 2. Obtain the active `GhosttyScheme` returned by `resolve_base` (or
///    `GhosttyScheme::default()` for the terminal-default case).
/// 3. Resolve symbols via `SymbolSet::resolve(&finalize_symbols(&doc.symbols))`.
///
/// Returns all warnings: base-scheme path/parse warnings.
pub fn resolve(
    doc: &StyleDoc,
    dir: &std::path::Path,
) -> (ColorScheme, crate::symbols::SymbolSet, Vec<String>) {
    // Step 1+2: build base ColorScheme and get the active GhosttyScheme.
    let (mut cs, gs, mut warnings) =
        colors::resolve_base(doc.colors.scheme.as_deref(), dir);

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
    // The frame maps onto the existing status_header_style field (reuses the
    // boxing path). `border_fg` no longer has a legacy field to land on
    // (SQ-0309: `status_header`'s colour is theme-only now — see
    // `render/transcript.rs`'s `theme.get("status_header")`); it is parsed
    // above only for `write_style`'s round-trip.
    if let Some(b) = &doc.status_bar.border {
        cs.status_header_style = paneframe::parse_border_style(b);
    }

    // Step 4: resolve symbols.
    let set = crate::symbols::SymbolSet::resolve(&finalize_symbols(&doc.symbols));

    (cs, set, warnings)
}

// ── DEFAULT_STYLE_TOML ────────────────────────────────────────────────────────

/// The embedded built-in `default` style.
///
/// Sets single-line map and story borders as the default look.
/// An empty `[symbols]` means all presets resolve to their factory defaults via finalize_symbols.
pub const DEFAULT_STYLE_TOML: &str = r#"# babelmap built-in default style
# map_border / story_border = single; other selectors use terminal defaults.
# Empty [symbols] means all presets resolve to their factory defaults via finalize_symbols.

[colors]
"map_border" = { style = "single" }
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
        // Emit an explicit `"none"` sentinel for an UNSET colour instead of omitting
        // the key. A self-contained style file (write_style_full) is merged OVER the
        // global style.toml per-game; an omitted field would field-merge-inherit the
        // global's non-default colour (the "freeze" bug), whereas the sentinel wins at
        // merge. `"none"` resolves back to unset (parse_color_value returns None), so
        // it patches nothing — preserving both self-containment and the compositional
        // inheritance that a genuinely-unset fg/bg relies on (e.g. input:prompt).
        fg: Some(s.fg.map_or_else(|| "none".to_string(), color_to_str)),
        bg: Some(s.bg.map_or_else(|| "none".to_string(), color_to_str)),
        bold: modifier_flag(s.add_modifier, Modifier::BOLD),
        italic: modifier_flag(s.add_modifier, Modifier::ITALIC),
        underline: modifier_flag(s.add_modifier, Modifier::UNDERLINED),
        dim: modifier_flag(s.add_modifier, Modifier::DIM),
        reversed: modifier_flag(s.add_modifier, Modifier::REVERSED),
        style: None,  // color-only inverse; callers set this for border selectors
        style_top: None,
        style_bottom: None,
        style_left: None,
        style_right: None,
        header: None,
        shadow: None, // callers set this for the dialog selector
        placement: None, // callers set this for the dialog selector
        margin: None,    // callers set this for the dialog selector
        glyph_top: None,
        glyph_bottom: None,
        glyph_left: None,
        glyph_right: None,
        glyph_tl: None,
        glyph_tr: None,
        glyph_bl: None,
        glyph_br: None,
    }
}

/// Encode a [`Color`] as a string suitable for a [`Decl`] fg/bg field.
///
/// `pub(crate)`: also reused by `theme::template::commented_template` (the
/// registry-driven `style.toml` template generator, SQ-0309) as the inverse of
/// `colors::parse_color_value`.
pub(crate) fn color_to_str(c: ratatui::style::Color) -> String {
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
            if let Some(st) = &decl.style_top    { itbl.insert("style_top",    toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_bottom { itbl.insert("style_bottom", toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_left   { itbl.insert("style_left",   toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_right  { itbl.insert("style_right",  toml_edit::Value::from(st.as_str())); }
            if decl.header == Some(false)        { itbl.insert("header",       toml_edit::Value::from(false)); }
            if let Some(fg) = &decl.fg { itbl.insert("fg", toml_edit::Value::from(fg.as_str())); }
            if let Some(bg) = &decl.bg { itbl.insert("bg", toml_edit::Value::from(bg.as_str())); }
            if decl.bold      == Some(true) { itbl.insert("bold",      toml_edit::Value::from(true)); }
            if decl.italic    == Some(true) { itbl.insert("italic",    toml_edit::Value::from(true)); }
            if decl.underline == Some(true) { itbl.insert("underline", toml_edit::Value::from(true)); }
            if decl.dim       == Some(true) { itbl.insert("dim",       toml_edit::Value::from(true)); }
            if decl.reversed  == Some(true) { itbl.insert("reversed",  toml_edit::Value::from(true)); }
            if decl.shadow    == Some(true) { itbl.insert("shadow",    toml_edit::Value::from(true)); }
            if let Some(p) = &decl.placement { itbl.insert("placement", toml_edit::Value::from(p.as_str())); }
            if let Some(m) = decl.margin { itbl.insert("margin", toml_edit::Value::from(m as i64)); }
            if let Some(g) = &decl.glyph_top    { itbl.insert("glyph_top",    toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_bottom { itbl.insert("glyph_bottom", toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_left   { itbl.insert("glyph_left",   toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_right  { itbl.insert("glyph_right",  toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_tl     { itbl.insert("glyph_tl",     toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_tr     { itbl.insert("glyph_tr",     toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_bl     { itbl.insert("glyph_bl",     toml_edit::Value::from(g.as_str())); }
            if let Some(g) = &decl.glyph_br     { itbl.insert("glyph_br",     toml_edit::Value::from(g.as_str())); }
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

        // Diagonal corner stubs (SQ-0314) — a bool, not a preset name.
        match doc.symbols.diagonal_corners {
            Some(v) => { symbols["diagonal_corners"] = toml_edit::value(v); }
            None    => { symbols.remove("diagonal_corners"); }
        }

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

    // ── [[transcript.rule]] ─────────────────────────────────────────────────────
    {
        // Remove any existing transcript table, then rewrite from the doc.
        tdoc.remove("transcript");
        if !doc.transcript_rules.is_empty() {
            let mut arr = toml_edit::ArrayOfTables::new();
            for r in &doc.transcript_rules {
                let mut t = toml_edit::Table::new();
                t["match"] = toml_edit::value(r.pattern.as_str());
                if let Some(fg) = &r.decl.fg { t["fg"] = toml_edit::value(fg.as_str()); }
                if let Some(bg) = &r.decl.bg { t["bg"] = toml_edit::value(bg.as_str()); }
                if r.decl.bold == Some(true) { t["bold"] = toml_edit::value(true); }
                if r.decl.italic == Some(true) { t["italic"] = toml_edit::value(true); }
                if r.decl.underline == Some(true) { t["underline"] = toml_edit::value(true); }
                if r.decl.dim == Some(true) { t["dim"] = toml_edit::value(true); }
                if r.decl.reversed == Some(true) { t["reversed"] = toml_edit::value(true); }
                arr.push(t);
            }
            let mut transcript = toml_edit::Table::new();
            transcript.insert("rule", toml_edit::Item::ArrayOfTables(arr));
            tdoc.insert("transcript", toml_edit::Item::Table(transcript));
        }
    }

    // ── [statusbar] ─────────────────────────────────────────────────────────────
    {
        tdoc.remove("statusbar");
        let sb = &doc.status_bar;
        if sb.border.is_some() || sb.border_fg.is_some() || !sb.segments.is_empty() {
            let mut table = toml_edit::Table::new();
            if let Some(b) = &sb.border { table["border"] = toml_edit::value(b.as_str()); }
            if let Some(c) = &sb.border_fg { table["border_fg"] = toml_edit::value(c.as_str()); }
            if !sb.segments.is_empty() {
                let mut arr = toml_edit::ArrayOfTables::new();
                for seg in &sb.segments {
                    let mut t = toml_edit::Table::new();
                    t["text"] = toml_edit::value(seg.text.as_str());
                    t["align"] = toml_edit::value(seg.align.as_str());
                    if let Some(fg) = &seg.decl.fg { t["fg"] = toml_edit::value(fg.as_str()); }
                    if let Some(bg) = &seg.decl.bg { t["bg"] = toml_edit::value(bg.as_str()); }
                    if seg.decl.bold == Some(true) { t["bold"] = toml_edit::value(true); }
                    if seg.decl.italic == Some(true) { t["italic"] = toml_edit::value(true); }
                    if seg.decl.underline == Some(true) { t["underline"] = toml_edit::value(true); }
                    if seg.decl.dim == Some(true) { t["dim"] = toml_edit::value(true); }
                    if seg.decl.reversed == Some(true) { t["reversed"] = toml_edit::value(true); }
                    arr.push(t);
                }
                table.insert("segment", toml_edit::Item::ArrayOfTables(arr));
            }
            tdoc.insert("statusbar", toml_edit::Item::Table(table));
        }
    }

    std::fs::write(path, tdoc.to_string())
}

// ── write_style_full ──────────────────────────────────────────────────────────

/// Write a fully-expanded, self-contained style file.
///
/// Encodes every [`ColorScheme`] field as a selector declaration (using
/// `style_to_decl`) and every [`SymbolSet`](crate::symbols::SymbolSet) slot as an override so that
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

    // SQ-0309: the per-field `[colors]` selector export is gone — that
    // schema no longer feeds ColorScheme/theme resolution at all (see
    // `resolve()` above; `colors::resolve_base` always passes empty
    // overrides to `from_ghostty`), so re-populating it here would just
    // write dead data. The live look now round-trips through the
    // registry-driven `[elements]`/`[panel]`/... schema instead
    // (`theme::template::commented_template`), which this function does not
    // (yet) emit uncommented; only the still-live symbols/transcript-rules/
    // statusbar sections below are written.

    // Symbol slots: use default preset names, then override every slot explicitly.
    // This guarantees round-trip fidelity regardless of which preset produced the set.
    doc.symbols.box_style    = Some(crate::config::default_box_style());
    doc.symbols.arrow_set    = Some(crate::config::default_arrow_set());
    doc.symbols.portal_icons = Some(crate::config::default_portal_icons());
    doc.symbols.path_style   = Some(crate::config::default_path_style());
    // Not a preset/slot — carry the live value so a saved style round-trips it.
    doc.symbols.diagonal_corners = Some(set.diagonal_corners);

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
    ov.insert("path.diag_ul".to_string(), set.path.diag_ul.to_string());
    ov.insert("path.diag_ur".to_string(), set.path.diag_ur.to_string());
    ov.insert("path.diag_ll".to_string(), set.path.diag_ll.to_string());
    ov.insert("path.diag_lr".to_string(), set.path.diag_lr.to_string());
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

    // Export user transcript rules (CompiledRule → RawRule).
    for rule in &cs.transcript_rules {
        doc.transcript_rules.push(RawRule {
            pattern: rule.pattern.clone(),
            decl: style_to_decl(&rule.style),
        });
    }
    // Export the statusbar segments (StatusSegment → RawSegment). The frame is NOT
    // re-emitted here; it round-trips through the status_header selector export.
    for seg in &cs.statusbar_layout.segments {
        doc.status_bar.segments.push(RawSegment {
            text: seg.text.clone(),
            align: seg.align.as_str().to_string(),
            decl: style_to_decl(&seg.style),
        });
    }

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
    fn style_example_toml_parses_and_resolves_clean() {
        // The repo-root style.example.toml is the user-facing reference (SQ-0309:
        // now the registry-generated, fully-commented new-schema template — see
        // `theme::template::style_example_matches_generated_template` for the
        // byte-for-byte check). It must still parse clean under the new schema so
        // the docs cannot drift from the code.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        let parsed = crate::theme::toml_schema::parse(&text);
        assert!(parsed.is_ok(), "style.example.toml failed to parse: {parsed:?}");
    }

    #[test]
    fn write_style_full_round_trips_statusbar_and_transcript_rules() {
        use crate::colors::{Align, StatusSegment, StatusBarLayout};
        use ratatui::style::{Color, Modifier};
        let dir = std::env::temp_dir().join(format!("babelmap-sb-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sb.toml");

        let mut cs = crate::colors::ColorScheme::terminal_default();
        // A custom transcript rule.
        cs.transcript_rules.push(crate::colors::CompiledRule {
            pattern: "(?i)grue".into(),
            regex: regex::Regex::new("(?i)grue").unwrap(),
            style: Style::new().fg(Color::Red),
        });
        // A custom statusbar layout.
        cs.statusbar_layout = StatusBarLayout {
            segments: vec![
                StatusSegment { text: "{location}".into(), align: Align::Left, style: Style::new().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED) },
                StatusSegment { text: "{title}".into(), align: Align::Center, style: Style::default() },
                StatusSegment { text: "Score {score}".into(), align: Align::Right, style: Style::new().fg(Color::Yellow) },
            ],
        };
        let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
        write_style_full(&path, &cs, &set).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let doc = parse_style_toml(&text).unwrap();
        let (cs2, _set2, _w) = resolve(&doc, &dir);

        // Transcript rule survived.
        assert_eq!(cs2.transcript_rules.len(), 1);
        assert_eq!(cs2.transcript_rules[0].pattern, "(?i)grue");
        assert_eq!(cs2.transcript_rules[0].style.fg, Some(Color::Red));
        // Statusbar layout survived (text, align, style).
        assert_eq!(cs2.statusbar_layout.segments.len(), 3);
        assert_eq!(cs2.statusbar_layout.segments[0].text, "{location}");
        // underline survives the export (fidelity fix for all decl modifiers).
        assert!(cs2.statusbar_layout.segments[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(matches!(cs2.statusbar_layout.segments[1].align, Align::Center));
        assert_eq!(cs2.statusbar_layout.segments[2].style.fg, Some(Color::Yellow));
        let _ = std::fs::remove_dir_all(&dir);
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
        // border maps onto the existing status_header_style machinery. SQ-0309:
        // `border_fg` no longer lands on a legacy field (status_header's colour
        // is theme-only now) — parsing it still round-trips via `write_style`.
        assert!(matches!(cs.status_header_style, crate::render::paneframe::BorderStyle::Single));
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
    fn decl_parses_placement_and_margin() {
        let t: toml::value::Table = toml::from_str(
            "placement = \"bottom\"\nmargin = 2\n",
        ).unwrap();
        let d = parse_decl_from_table(&t);
        assert_eq!(d.placement.as_deref(), Some("bottom"));
        assert_eq!(d.margin, Some(2));
        // Absent keys parse to None.
        let empty: toml::value::Table = toml::from_str("fg = \"cyan\"\n").unwrap();
        let d2 = parse_decl_from_table(&empty);
        assert_eq!(d2.placement, None);
        assert_eq!(d2.margin, None);
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
    fn resolve_sets_border_style_and_default_is_single() {
        // default doc (DEFAULT_STYLE_TOML) => single map, single story (SQ-0357)
        let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
        let (cs, _set, _w) = resolve(&doc, std::path::Path::new("."));
        assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::Single));
        assert!(matches!(cs.story_border_style, crate::render::paneframe::BorderStyle::Single));
    }

    #[test]
    fn write_style_full_is_stable_and_back_compatible() {
        // (a) A written style file re-parses, re-resolves, and RE-WRITES byte-identically
        //     — the unset-field sentinel is stable across a second round trip.
        let dir = std::env::temp_dir().join(format!("bm-stable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.toml");
        let p2 = dir.join("b.toml");
        let cs = crate::colors::ColorScheme::terminal_default();
        let set = crate::symbols::SymbolSet::default();
        write_style_full(&p1, &cs, &set).unwrap();
        let text1 = std::fs::read_to_string(&p1).unwrap();
        let (cs_rt, set_rt, _w) = resolve(&parse_style_toml(&text1).unwrap(), &dir);
        write_style_full(&p2, &cs_rt, &set_rt).unwrap();
        let text2 = std::fs::read_to_string(&p2).unwrap();
        assert_eq!(text1, text2, "write -> read -> write must be byte-stable");

        // (b) An existing on-disk file in the OLD format (a color field omitted, no
        //     sentinel) still parses and leaves that field unset — back-compatible.
        let legacy = "[colors]\n\"input:prompt\" = { bold = true }\n";
        let doc = parse_style_toml(legacy).unwrap();
        assert_eq!(doc.colors.selectors["input:prompt"].fg, None, "legacy omitted fg stays unset");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
