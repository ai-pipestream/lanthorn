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
}
