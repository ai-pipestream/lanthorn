//! Styling-role redesign (SQ-0309): the declarative selector registry and the
//! resolver/theme model built on top of it.
//!
//! [`registry`] is the single source of truth — one row per themeable selector
//! (name, section, kind, parent, default delta). Later tasks add the resolver
//! and TOML schema that consume it.

pub mod registry;
pub mod resolve;
pub mod template;
pub mod toml_schema;

// ── describe_theme ──────────────────────────────────────────────────────────

/// Describe a resolved [`resolve::Theme`] as printable lines for the `/colors`
/// dump: a header per registry [`registry::Section`] (style `None`), then one
/// line per selector `  <name>: fg=<fg> bg=<bg><attrs>` carrying that
/// selector's resolved [`ratatui::style::Style`]. Mirrors the shape of the
/// pre-SQ-0309 `style::describe_scheme` grouped output. `Section::Statusbar`
/// is skipped (dynamic `[[statusbar.segment]]` rows, no fixed registry rows).
pub fn describe_theme(theme: &resolve::Theme) -> Vec<(String, Option<ratatui::style::Style>)> {
    use ratatui::style::Modifier;

    const SECTIONS: &[(registry::Section, &str)] = &[
        (registry::Section::Roles, "roles"),
        (registry::Section::Elements, "elements"),
        (registry::Section::Panel, "panel"),
        (registry::Section::GlkBuffer, "glk.buffer"),
        (registry::Section::GlkGrid, "glk.grid"),
        (registry::Section::Map, "map"),
        (registry::Section::Debug, "debug"),
    ];

    let mut out: Vec<(String, Option<ratatui::style::Style>)> = Vec::new();
    for (section, title) in SECTIONS {
        out.push((format!("── {title} ──"), None));
        for row in registry::REGISTRY.iter().filter(|r| r.section == *section) {
            let st = theme.get(row.name).style;
            let fg = st.fg.map(crate::style::color_to_str).unwrap_or_else(|| "default".to_string());
            let bg = st.bg.map(crate::style::color_to_str).unwrap_or_else(|| "default".to_string());
            let mut attrs: Vec<&str> = Vec::new();
            if st.add_modifier.contains(Modifier::BOLD) { attrs.push("bold"); }
            if st.add_modifier.contains(Modifier::ITALIC) { attrs.push("italic"); }
            if st.add_modifier.contains(Modifier::UNDERLINED) { attrs.push("underline"); }
            if st.add_modifier.contains(Modifier::DIM) { attrs.push("dim"); }
            if st.add_modifier.contains(Modifier::REVERSED) { attrs.push("reversed"); }
            let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(",")) };
            out.push((format!("  {}: fg={fg} bg={bg}{attr_str}", row.name), Some(st)));
        }
    }
    out
}
