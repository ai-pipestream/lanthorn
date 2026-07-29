//! Registry-driven commented `style.toml` template (SQ-0309, Task 6b).
//!
//! [`commented_template`] emits the whole new-schema `style.toml`, grouped by
//! [`Section`] and with every line commented out — a no-op until the user
//! uncomments a line. Each selector's line is reconstructed straight from its
//! [`RegRow`]'s `parent` + `default_delta`, so uncommenting it reproduces
//! exactly the registry default (see `theme::resolve`). This replaces the
//! interactive style editors: [`auto_seed`] writes this template to the user's
//! `style.toml` on first run, and the file is then edited by hand + applied
//! live with `/reload`.

use super::registry::{
    Kind, RegRow, Section, DIAGONAL_CORNERS_DEFAULT, LAYER_CYCLE_DEFAULT, REGISTRY,
};
use crate::style::{color_to_str, personal_style_path};

/// Auto-seed `user_dir/style.toml` with [`commented_template`] if it does not
/// already exist. NEVER overwrites an existing file. Best-effort: a write
/// failure (e.g. a read-only home) is swallowed so startup never crashes.
pub fn auto_seed(user_dir: &std::path::Path) {
    let path = personal_style_path(user_dir);
    if path.exists() {
        return;
    }
    let _ = std::fs::write(&path, commented_template());
}

// A section to emit: (registry `Section`, TOML header, a short blurb comment
// shown above the header). Sections fall into two groups — TEXT (colour/emphasis
// only) and SURFACE (a background + optional border + the text drawn on them) —
// so it's clear where you adjust text vs. where you adjust a bordered box.

/// The roles: the roots everything else derives from, emitted first.
const ROLES_SECTION: (Section, &str, &str) =
    (Section::Roles, "[roles]", "Roles: the 7 roots everything else derives from.");

/// TEXT sections: adjust the foreground colour / emphasis of text, story output,
/// the map, and the debug view. These have no surface (background/border) of
/// their own — that's what the SURFACE sections below are for.
const TEXT_SECTIONS: &[(Section, &str, &str)] = &[
    (
        Section::Elements,
        "[elements]",
        "Elements: app text + host surfaces (status/help bars, upper window,\n# story browser). Every line already equals its default.",
    ),
    (Section::GlkBuffer, "[glk.buffer]", "The 11 Glk styles for text-buffer windows (base: text)."),
    (Section::GlkGrid, "[glk.grid]", "The 11 Glk styles for text-grid windows (base: chrome)."),
    (Section::Map, "[map]", "Map colours + glyph-set presets."),
    (Section::Debug, "[debug]", "Debug inspector disassembly selectors."),
];

/// SURFACE sections: bordered boxes with a background + frame + the text drawn
/// on them. Each is a distinct surface — a tiled pane, a modal, a tooltip.
const SURFACE_SECTIONS: &[(Section, &str, &str)] = &[
    (
        Section::Panel,
        "[panel]",
        "Panel: the tiled panes (story/map/verb/debug frames) — background,\n# border, title, tabs. Story/Glk windows use [glk.*], not this section.",
    ),
    (Section::Dialog, "[dialog]", "Dialog: modal pop-ups — background, its own border, title, buttons, shadow."),
    (Section::Tooltip, "[tooltip]", "Tooltip: hover pop-ups — background and an optional border."),
];

/// Emit the full new-schema `style.toml`, entirely commented out.
pub fn commented_template() -> String {
    let mut out = String::new();

    out.push_str(
        "# babelmap style.toml — auto-seeded. The section headers below are active,\n\
         # but every value line is commented out, so the file stays a no-op until you\n\
         # uncomment lines: each commented value already equals the built-in default,\n\
         # reconstructed straight from the style registry. Edit, uncomment, save, then\n\
         # run reload-style in-app to apply changes live (or turn on config.toml's\n\
         # watch_style to auto-reload on save).\n\
         #\n\
         # Color values accept: a named color (cyan, dark-gray, light-blue, …),\n\
         # palette:N (0-15 from the active scheme), #rrggbb hex, a 256-index (\"17\"),\n\
         # or background / foreground (the scheme's bg/fg).\n\
         \n",
    );
    out.push_str(&format!(
        "version = {}   # style schema version — do not remove; lets babelmap flag an out-of-date file\n\n",
        super::toml_schema::STYLE_SCHEMA_VERSION
    ));
    out.push_str(
        "# scheme = \"tomorrow-night\"   # optional base: built-in name or a Ghostty theme path; omit for terminal colours\n\
         \n",
    );

    let emit = |out: &mut String, section: Section, header: &str, blurb: &str| {
        out.push_str("# ── ");
        out.push_str(blurb);
        out.push('\n');
        out.push_str(header);
        out.push('\n');
        for row in REGISTRY.iter().filter(|r| r.section == section) {
            out.push_str(&row_line(section, row));
            out.push('\n');
        }
        out.push('\n');
    };

    // Roles first (the roots).
    emit(&mut out, ROLES_SECTION.0, ROLES_SECTION.1, ROLES_SECTION.2);

    // TEXT group.
    out.push_str("# ═══ TEXT — foreground colour + emphasis (these selectors have no surface of their own) ═══\n\n");
    for (section, header, blurb) in TEXT_SECTIONS {
        emit(&mut out, *section, header, blurb);
    }

    // SURFACE group.
    out.push_str("# ═══ SURFACES — bordered boxes: a background + frame + the text drawn on them ═══\n\n");
    for (section, header, blurb) in SURFACE_SECTIONS {
        emit(&mut out, *section, header, blurb);
    }

    out.push_str(STATIC_EXAMPLES);

    out
}

/// Reconstruct one registry row as a single commented TOML line.
fn row_line(section: Section, row: &RegRow) -> String {
    // Roles carry no registry delta (their concrete values live in
    // `Roles::from_scheme`), so emit each one's actual scheme-relative default
    // plus a one-line description of what it's for.
    if section == Section::Roles {
        return role_line(row.name);
    }

    // The two map rows whose value `Delta` cannot carry: a list and a bool.
    if row.name == "map.diagonal_corners" {
        return format!(
            "# diagonal_corners = {DIAGONAL_CORNERS_DEFAULT}   \
             # false = plain orthogonal corner exits, for fonts without Unicode 13 (🮠🮡🮢🮣)"
        );
    }

    if row.name == "map.layer_cycle" {
        let list =
            LAYER_CYCLE_DEFAULT.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
        return format!("# layer_cycle = [{list}]");
    }

    let key = toml_key(&strip_section_prefix(section, row.name));

    if row.kind == Kind::Placement {
        let preset = row.default_delta.glyph.as_deref().unwrap_or_default();
        return format!("# {key} = \"{preset}\"{}", enum_hint(row));
    }

    let fields = row_fields(row);
    let line = if fields.is_empty() {
        format!("# {key} = {{}}")
    } else {
        format!("# {key} = {{ {} }}", fields.join(", "))
    };
    format!("{line}{}", enum_hint(row))
}

/// One commented `[roles]` line: the role's scheme-relative default (matching
/// `Roles::from_scheme`) plus a one-line description of its purpose. Kept in
/// sync with the resolver by `uncommented_template_resolves_to_registry_defaults`.
fn role_line(name: &str) -> String {
    let (value, desc) = match name {
        "text" => ("{ fg = \"foreground\" }", "body ink on the page (scheme foreground)"),
        "chrome" => ("{ fg = \"foreground\", bg = \"background\" }", "ink on a UI surface: bars, panels, upper window"),
        "line" => ("{ fg = \"palette:6\" }", "lines, frames, rules, dividers (scheme cyan slot)"),
        "accent" => ("{ fg = \"palette:6\" }", "highlights: links, selection, current room, badges"),
        "muted" => ("{ fg = \"palette:8\" }", "dim / secondary text (scheme bright-black slot)"),
        "alert" => ("{ fg = \"palette:3\" }", "warnings and errors (scheme yellow slot)"),
        "heading" => ("{ fg = \"foreground\", bold = true }", "titles and headers (bold)"),
        _ => return String::new(),
    };
    format!("# {name:<7} = {value:<40}  # {desc}")
}

/// The `# a | b | c` enumeration hint appended to an enumerated row's line
/// (a map glyph-set preset, or a border `style`), or `""` for a non-enumerated
/// row. Value lists are verified against `symbols.rs`'s preset tables and
/// `paneframe::parse_border_style`.
fn enum_hint(row: &RegRow) -> &'static str {
    match row.name {
        "map.box_style" => "   # rounded | thick | double | solid | super-thick | ascii | borderless",
        "map.arrow_set" => "   # filled | line | nerdfont | nf-bold | nf-box | nf-circle | nf-outline",
        "map.portal_icons" => "   # ascii | nerdfont | nerdfont-stairs",
        "map.path_style" | "map.portal_path_style" => "   # light | heavy | dotted",
        // Story-list row badges (SQ-0559). Free-form, not enumerated — any string,
        // so a patched font can use a real icon instead of a bare letter.
        "badge_zcode" => "   # story-list badge: a Z-machine story",
        "badge_glulx" => "   # story-list badge: a Glulx story",
        "badge_blorb" => "   # story-list badge: resources in a Blorb",
        "badge_save" => "   # story-list badge: the story has a save",
        "badge_hint" => "   # story-list badge: hints are installed",
        "badge_hint_available" => "   # story-list badge: hints exist but aren't downloaded",
        _ if row.default_delta.border.is_some() => "   # style: none | single | double | thick | rounded",
        _ => "",
    }
}

/// The `parent = ...` + delta fields for a row's inline table, in the order a
/// user would naturally write them.
fn row_fields(row: &RegRow) -> Vec<String> {
    let d = &row.default_delta;
    let mut fields = Vec::new();
    if let Some(parent) = row.parent {
        fields.push(format!("parent = \"{parent}\""));
    }
    if let Some(fg) = d.fg {
        fields.push(format!("fg = \"{}\"", color_to_str(fg)));
    }
    if let Some(bg) = d.bg {
        fields.push(format!("bg = \"{}\"", color_to_str(bg)));
    }
    if d.bold {
        fields.push("bold = true".to_string());
    }
    if d.italic {
        fields.push("italic = true".to_string());
    }
    if d.underline {
        fields.push("underline = true".to_string());
    }
    if d.reversed {
        fields.push("reversed = true".to_string());
    }
    if d.dim {
        fields.push("dim = true".to_string());
    }
    if let Some(border) = d.border {
        fields.push(format!("style = \"{}\"", crate::render::paneframe::border_style_name(border)));
    }
    if let Some(glyph) = &d.glyph {
        fields.push(format!("glyph = \"{glyph}\""));
    }
    fields
}

/// Strip a section's TOML-header prefix off a full registry selector name,
/// leaving the bare key written under that header (e.g. `panel.border:active`
/// → `border:active`, `glk.buffer.normal` → `normal`).
fn strip_section_prefix(section: Section, name: &str) -> String {
    let prefix = match section {
        Section::Panel => "panel.",
        Section::GlkBuffer => "glk.buffer.",
        Section::GlkGrid => "glk.grid.",
        Section::Map => "map.",
        Section::Debug => "debug.",
        Section::Dialog => "dialog.",
        Section::Tooltip => "tooltip.",
        Section::Roles | Section::Elements | Section::Statusbar => "",
    };
    name.strip_prefix(prefix).unwrap_or(name).to_string()
}

/// Quote a TOML key if it contains `:` or `.` (e.g. `"border:active"`).
fn toml_key(key: &str) -> String {
    if key.contains(':') || key.contains('.') {
        format!("\"{key}\"")
    } else {
        key.to_string()
    }
}

/// Static commented examples with no registry row of their own: a
/// `[[transcript.rule]]` pair and a `[statusbar]` + `[[statusbar.segment]]`
/// block, copied (commented) from the design spec's example.
const STATIC_EXAMPLES: &str = r#"# ── Story-line styling rules: recolour whole transcript lines matching a ────
# ── regex. Rules are tried in order; the first match wins. ──────────────────
# [[transcript.rule]]
# match = "^>.*"                 # your echoed command lines → magenta bold
# fg = "magenta"
# bold = true

# [[transcript.rule]]
# match = "(?i)\\bgrue\\b"       # any line mentioning a "grue" → red (flavour example)
# fg = "red"

# ── Status bar. Omit [statusbar] for the built-in default. ───────────────────
# Placeholders: {location} {score} {moves} {time} {turns} {title} {filter}
# [statusbar]
# border = "none"

# [[statusbar.segment]]
# text  = "{location}"
# align = "left"
# parent = "accent"              # segments may reference a role or set fg/bg directly
# bold  = true

# [[statusbar.segment]]
# text  = "Score: {score}  Moves: {moves}"
# align = "right"

# [[statusbar.segment]]
# text  = "{time}"
# align = "right"
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::GhosttyScheme;
    use crate::theme::resolve::resolve_theme;
    use crate::theme::toml_schema::{self, ParsedStyle};
    use ratatui::style::Color;

    /// Every non-`Statusbar` registry row's local key must appear somewhere in
    /// the generated template (regression: a new row without a template line).
    #[test]
    fn template_covers_every_registry_selector() {
        let template = commented_template();
        for row in REGISTRY.iter().filter(|r| r.section != Section::Statusbar) {
            let leaf = row.name.rsplit('.').next().unwrap();
            assert!(
                template.contains(leaf),
                "registry row {:?} (leaf {leaf:?}) missing from commented_template()",
                row.name
            );
        }
    }

    /// The whole template is comments/blanks only, so it parses as an empty
    /// (all-default) document.
    #[test]
    fn template_parses_clean() {
        let parsed = toml_schema::parse(&commented_template());
        assert!(parsed.is_ok(), "template failed to parse: {parsed:?}");
    }

    /// Uncomment every real TOML line (section headers + `key = value` /
    /// `key = {..}` assignments) while leaving prose/blurb comments alone,
    /// then parse it.
    fn uncomment_toml_lines(template: &str) -> String {
        template
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix("# ") {
                    if rest.starts_with('[') || rest.contains(" = ") {
                        return rest.to_string();
                    }
                }
                line.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A `GhosttyScheme` matching `Roles::terminal_default()`/the resolver's
    /// default test fixture, so `resolve_theme` reproduces registry defaults.
    fn terminal_default_scheme() -> GhosttyScheme {
        let mut scheme = GhosttyScheme { foreground: Color::White, ..GhosttyScheme::default() };
        scheme.palette[3] = Color::Yellow;
        scheme.palette[6] = Color::Cyan;
        scheme.palette[8] = Color::DarkGray;
        scheme
    }

    /// Uncommenting the whole template and resolving it must reproduce the
    /// registry-default theme (spot-checked across sections).
    #[test]
    fn uncommented_template_resolves_to_registry_defaults() {
        let scheme = terminal_default_scheme();
        let uncommented = uncomment_toml_lines(&commented_template());
        let parsed = toml_schema::parse(&uncommented)
            .unwrap_or_else(|e| panic!("uncommented template failed to parse: {e:?}"));

        let seeded = resolve_theme(&scheme, &parsed);
        let default = resolve_theme(&scheme, &ParsedStyle::default());

        // roles: every one's template default must resolve back to the registry
        // default, so the hand-written [roles] block can't drift from the resolver.
        for role in ["text", "chrome", "border", "accent", "muted", "alert", "heading"] {
            assert_eq!(seeded.get(role).style, default.get(role).style, "role {role} drifted");
        }
        // an element
        assert_eq!(seeded.get("transcript_meta").style, default.get("transcript_meta").style);
        assert_eq!(seeded.get("transcript_meta").glyph, default.get("transcript_meta").glyph);
        // panel.border style
        assert_eq!(seeded.get("panel.border").border, default.get("panel.border").border);
        assert_eq!(seeded.get("panel.border").style, default.get("panel.border").style);
        // a glk slot
        assert_eq!(seeded.get("glk.buffer.header").style, default.get("glk.buffer.header").style);
        // a map colour
        assert_eq!(seeded.get("map.connector_distorted").style, default.get("map.connector_distorted").style);
        // a debug tier
        assert_eq!(seeded.get("debug.disasm_data").style, default.get("debug.disasm_data").style);
        assert_eq!(seeded.get("debug.disasm_data").glyph, default.get("debug.disasm_data").glyph);
    }

    /// The repo-root `style.example.toml` must be exactly `commented_template()`'s
    /// output, so the checked-in example can never drift from the generator.
    /// Regenerate it with the generator's output whenever this fails.
    #[test]
    fn style_example_matches_generated_template() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
        assert_eq!(text, commented_template(), "style.example.toml is stale — regenerate it from commented_template()");
    }

    #[test]
    fn auto_seed_writes_when_missing_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "babelmap-template-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = personal_style_path(&dir);

        assert!(!path.exists());
        auto_seed(&dir);
        assert!(path.exists());
        let seeded = std::fs::read_to_string(&path).unwrap();
        assert_eq!(seeded, commented_template());

        // Modify the file, then seed again — must be left byte-unchanged.
        std::fs::write(&path, "# user was here\n").unwrap();
        auto_seed(&dir);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "# user was here\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
