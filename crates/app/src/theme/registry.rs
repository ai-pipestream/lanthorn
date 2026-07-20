//! The selector registry: one declarative row per themeable selector.
//!
//! This is a *pure data table* — no behaviour is wired to it here. The resolver,
//! TOML parser, template generator and export path (later SQ-0309 tasks) all read
//! [`REGISTRY`] so that every themeable element is declared exactly once.
//!
//! Each [`RegRow`] carries the selector's `name`, its template [`Section`], its
//! [`Kind`], its `parent` (a role name or another selector), and the
//! `default_delta` layered on that parent. Values are transcribed from the design
//! spec `docs/design/2026-07-14-styling-role-redesign.md` (§1–§4b) — that document
//! is authoritative for every parent/delta.

use ratatui::style::Color;

/// Which template section a selector belongs to (drives grouping in the seed
/// template / `style.example.toml`). Every row has exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The 7 role roots (§1).
    Roles,
    /// App chrome/text elements (§2).
    Elements,
    /// Shared panel chrome (§2a/§2b).
    Panel,
    /// The 11 Glk styles for text-buffer windows (§3).
    GlkBuffer,
    /// The 11 Glk styles for text-grid windows (§3).
    GlkGrid,
    /// Map-domain colours + glyph-set presets (§4).
    Map,
    /// Debug inspector disassembly selectors (§4b).
    Debug,
    /// Status-bar segments (dynamic `[[statusbar.segment]]`; no fixed rows here).
    Statusbar,
}

/// What a selector primarily controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A plain text/colour style (fg/bg + modifiers).
    Style,
    /// A border/frame or a glyph carried in a border (dividers, terminators).
    BorderGlyphs,
    /// A named glyph-set preset (e.g. map `box_style = "rounded"`).
    Placement,
}

/// A partial style layered on a selector's parent: optional fg/bg, the terminal
/// modifier flags, and optional glyph(s).
///
/// - `glyph` is a single glyph a selector carries (gutter marks, tab dividers,
///   terminator caps) or, for a [`Kind::Placement`] preset row, the preset name.
/// - `glyphs` is a small named-glyph slot list for selectors that own a set
///   (e.g. box/arrow slots on `map.room`/`map.connector`). Empty for the defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Delta {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reversed: bool,
    pub dim: bool,
    pub glyph: Option<&'static str>,
    pub glyphs: &'static [(&'static str, &'static str)],
    pub border: Option<&'static str>,
}

impl Delta {
    /// The zero delta: same as the parent, no glyph.
    pub const EMPTY: Delta = Delta {
        fg: None,
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        reversed: false,
        dim: false,
        glyph: None,
        glyphs: &[],
        border: None,
    };
}

/// One registry row: a themeable selector and how it derives from its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegRow {
    pub name: &'static str,
    pub section: Section,
    pub kind: Kind,
    pub parent: Option<&'static str>,
    pub default_delta: Delta,
}

/// The 7 role root names (§1). Roles are the parents everything else derives
/// from; their concrete colours come from the base scheme + `[roles]`, not this
/// table, so their rows carry `parent = None` and an empty delta.
pub const ROLE_NAMES: [&str; 7] =
    ["text", "chrome", "border", "accent", "muted", "alert", "heading"];

/// The default per-layer edge colour cycle for `map.layer_cycle` (§4). This is
/// the sole list-valued selector, so its value lives here rather than in a
/// [`Delta`]; the `map.layer_cycle` registry row exists only for completeness.
pub const LAYER_CYCLE_DEFAULT: [&str; 5] = ["cyan", "green", "magenta", "yellow", "blue"];

/// Build a row concisely.
const fn row(
    name: &'static str,
    section: Section,
    kind: Kind,
    parent: Option<&'static str>,
    default_delta: Delta,
) -> RegRow {
    RegRow { name, section, kind, parent, default_delta }
}

/// A delta carrying only modifier flags (colours/glyph inherited from parent).
const fn mods(bold: bool, italic: bool, underline: bool, reversed: bool) -> Delta {
    Delta { bold, italic, underline, reversed, ..Delta::EMPTY }
}

/// A delta setting only a foreground colour.
const fn fg(c: Color) -> Delta {
    Delta { fg: Some(c), ..Delta::EMPTY }
}

/// A delta carrying only a single glyph (e.g. gutter mark, tab divider, or a
/// [`Kind::Placement`] preset name).
const fn glyph(g: &'static str) -> Delta {
    Delta { glyph: Some(g), ..Delta::EMPTY }
}

/// A delta setting only the border style (colour/glyph inherited from parent).
const fn border(name: &'static str) -> Delta {
    Delta { border: Some(name), ..Delta::EMPTY }
}

pub const REGISTRY: &[RegRow] = &[
    // ── §1 roles ──────────────────────────────────────────────────────────────
    // Roots: colours come from the base scheme + `[roles]`, so no parent/delta.
    row("text", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("chrome", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("border", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("accent", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("muted", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("alert", Section::Roles, Kind::Style, None, Delta::EMPTY),
    row("heading", Section::Roles, Kind::Style, None, Delta::EMPTY),
    // ── §2 elements ───────────────────────────────────────────────────────────
    row("transcript", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("status_bar", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("help_bar", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("upper_window", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("story_info", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    // heading on chrome: heading fg with the chrome bg (black).
    row("status_header", Section::Elements, Kind::Style, Some("heading"), Delta { bg: Some(Color::Black), ..Delta::EMPTY }),
    row("story_title", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    row("input_line", Section::Elements, Kind::Style, Some("border"), Delta::EMPTY),
    row("suggestion_line", Section::Elements, Kind::Style, Some("border"), Delta::EMPTY),
    row("scrollbar", Section::Elements, Kind::Style, Some("border"), Delta::EMPTY),
    row("transcript_location", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("story_badge", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("hyperlink", Section::Elements, Kind::Style, Some("accent"), mods(false, false, true, false)),
    row("story_info_label", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("suggestion", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    // transcript_meta / transcript_warning carry a gutter glyph (§2/§4 glyph decision).
    row("transcript_meta", Section::Elements, Kind::Style, Some("muted"), glyph("▏")),
    row("transcript_warning", Section::Elements, Kind::Style, Some("alert"), glyph("!")),
    row("transcript_crash", Section::Elements, Kind::Style, Some("alert"), mods(true, false, false, false)),
    // ── §2a/§2b panel.* (shared panel chrome) ────────────────────────────────
    row("panel.background", Section::Panel, Kind::Style, Some("chrome"), Delta::EMPTY),
    // border rows: default border style = "single" (unified panel frame, §2a).
    // `:active` = single + bold (today's cyan+bold).
    row("panel.border", Section::Panel, Kind::BorderGlyphs, Some("border"), border("single")),
    row("panel.border:active", Section::Panel, Kind::BorderGlyphs, Some("border"),
        Delta { border: Some("single"), bold: true, ..Delta::EMPTY }),
    row("panel.title", Section::Panel, Kind::Style, Some("heading"), Delta::EMPTY),
    row("panel.tab", Section::Panel, Kind::Style, Some("muted"), Delta::EMPTY),
    row("panel.tab:active", Section::Panel, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("panel.tab_divider", Section::Panel, Kind::BorderGlyphs, Some("border"), glyph("│")),
    // terminator glyphs default to the single-border caps ┤/├ (spec §2b).
    row("panel.terminator_left", Section::Panel, Kind::BorderGlyphs, Some("border"), glyph("┤")),
    row("panel.terminator_right", Section::Panel, Kind::BorderGlyphs, Some("border"), glyph("├")),
    // ── §3 glk.buffer.* (buffer base = text) ─────────────────────────────────
    row("glk.buffer.normal", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.emphasized", Section::GlkBuffer, Kind::Style, Some("text"), mods(false, true, false, false)),
    row("glk.buffer.preformatted", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.header", Section::GlkBuffer, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.buffer.subheader", Section::GlkBuffer, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.buffer.alert", Section::GlkBuffer, Kind::Style, Some("alert"), mods(true, false, false, false)),
    row("glk.buffer.note", Section::GlkBuffer, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("glk.buffer.blockquote", Section::GlkBuffer, Kind::Style, Some("muted"), Delta::EMPTY),
    row("glk.buffer.input", Section::GlkBuffer, Kind::Style, Some("accent"), Delta::EMPTY),
    row("glk.buffer.user1", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    row("glk.buffer.user2", Section::GlkBuffer, Kind::Style, Some("text"), Delta::EMPTY),
    // ── §3 glk.grid.* (grid base = chrome) ───────────────────────────────────
    row("glk.grid.normal", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.emphasized", Section::GlkGrid, Kind::Style, Some("chrome"), mods(false, true, false, false)),
    row("glk.grid.preformatted", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.header", Section::GlkGrid, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.grid.subheader", Section::GlkGrid, Kind::Style, Some("heading"), mods(true, false, false, false)),
    row("glk.grid.alert", Section::GlkGrid, Kind::Style, Some("alert"), mods(true, false, false, false)),
    row("glk.grid.note", Section::GlkGrid, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("glk.grid.blockquote", Section::GlkGrid, Kind::Style, Some("muted"), Delta::EMPTY),
    row("glk.grid.input", Section::GlkGrid, Kind::Style, Some("accent"), Delta::EMPTY),
    row("glk.grid.user1", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("glk.grid.user2", Section::GlkGrid, Kind::Style, Some("chrome"), Delta::EMPTY),
    // ── §4 map.* colours (hybrid (c): own tokens; current/selected → accent) ──
    // map.background: own token, defaults to terminal background (resolver-supplied).
    row("map.background", Section::Map, Kind::Style, None, Delta::EMPTY),
    row("map.room", Section::Map, Kind::Style, None, fg(Color::White)),
    row("map.room_current", Section::Map, Kind::Style, Some("accent"), Delta::EMPTY),
    row("map.room_selected", Section::Map, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("map.connector", Section::Map, Kind::Style, None, fg(Color::Cyan)),
    row("map.connector_distorted", Section::Map, Kind::Style, None, fg(Color::Magenta)),
    row("map.connector_portal", Section::Map, Kind::Style, None, fg(Color::Cyan)),
    row("map.shared_path", Section::Map, Kind::Style, None, fg(Color::LightCyan)),
    row("map.loc_indicator", Section::Map, Kind::Style, Some("muted"), Delta::EMPTY),
    // layer_cycle: sole list-valued selector; value in LAYER_CYCLE_DEFAULT.
    row("map.layer_cycle", Section::Map, Kind::Style, None, Delta::EMPTY),
    // Glyph-set presets (the old [symbols] section, merged in): preset name in `glyph`.
    row("map.box_style", Section::Map, Kind::Placement, None, glyph("rounded")),
    row("map.arrow_set", Section::Map, Kind::Placement, None, glyph("filled")),
    row("map.portal_icons", Section::Map, Kind::Placement, None, glyph("ascii")),
    row("map.path_style", Section::Map, Kind::Placement, None, glyph("light")),
    row("map.portal_path_style", Section::Map, Kind::Placement, None, glyph("dotted")),
    // ── §4b debug.* (disasm-only; each tier carries a gutter glyph) ───────────
    row("debug.pc", Section::Debug, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("debug.tooltip", Section::Debug, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("debug.disasm_executed", Section::Debug, Kind::Style, Some("accent"), glyph("|")),
    row("debug.disasm_rd", Section::Debug, Kind::Style, Some("text"), glyph(" ")),
    row("debug.disasm_soft", Section::Debug, Kind::Style, Some("muted"), glyph(" ")),
    row("debug.disasm_data", Section::Debug, Kind::Style, Some("muted"), Delta { italic: true, glyph: Some(" "), ..Delta::EMPTY }),
    // ── §2 elements (expansion, Task 5.0): every remaining ColorScheme field ──
    row("more_prompt", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("tidy_progress", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("meta_marker", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("inventory_dock", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("story_info_title", Section::Elements, Kind::Style, Some("heading"), Delta::EMPTY),
    row("story_info_value", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_info_blurb", Section::Elements, Kind::Style, Some("muted"), mods(false, true, false, false)),
    row("story_info_link", Section::Elements, Kind::Style, Some("accent"), mods(false, false, true, false)),
    row("story_info_cover", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("graphics", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("inline_image", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("story_header", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("story_header_active", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, false)),
    row("story_author", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_year", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_no_metadata", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("story_tile", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("story_tile_selected", Section::Elements, Kind::Style, Some("accent"), mods(true, false, false, true)),
    row("notification", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    row("dialog", Section::Elements, Kind::Style, Some("chrome"), Delta::EMPTY),
    row("dialog_title", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("dialog_button", Section::Elements, Kind::Style, Some("chrome"), mods(false, false, false, true)),
    row("dialog_button_active", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
    // dialog_shadow: the one explicit-colour row — keeps a distinctive dark-gray bg.
    row("dialog_shadow", Section::Elements, Kind::Style, Some("muted"), Delta { bg: Some(Color::DarkGray), ..Delta::EMPTY }),
    row("hotkey_key", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("sound_beep_high", Section::Elements, Kind::Style, Some("alert"), Delta::EMPTY),
    row("sound_beep_low", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("transcript_input", Section::Elements, Kind::Style, Some("accent"), Delta::EMPTY),
    row("transcript_system", Section::Elements, Kind::Style, Some("muted"), Delta::EMPTY),
    row("warning_marker", Section::Elements, Kind::Style, Some("alert"), Delta::EMPTY),
    row("input_text", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("input_prompt", Section::Elements, Kind::Style, Some("text"), Delta::EMPTY),
    row("upper_window_border", Section::Elements, Kind::Style, Some("border"), Delta::EMPTY),
    row("room_panel", Section::Elements, Kind::Style, Some("accent"), mods(false, false, false, true)),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The full set of selector names the spec (§1–§4b) requires, copied out
    /// explicitly so the test fails loudly if the registry drifts.
    const EXPECTED: &[&str] = &[
        // §1 roles
        "text",
        "chrome",
        "border",
        "accent",
        "muted",
        "alert",
        "heading",
        // §2 elements
        "transcript",
        "status_bar",
        "help_bar",
        "upper_window",
        "story_info",
        "status_header",
        "story_title",
        "input_line",
        "suggestion_line",
        "scrollbar",
        "transcript_location",
        "story_badge",
        "hyperlink",
        "story_info_label",
        "suggestion",
        "transcript_meta",
        "transcript_warning",
        "transcript_crash",
        // §2a/§2b panel.*
        "panel.background",
        "panel.border",
        "panel.border:active",
        "panel.title",
        "panel.tab",
        "panel.tab:active",
        "panel.tab_divider",
        "panel.terminator_left",
        "panel.terminator_right",
        // §3 glk.buffer.*
        "glk.buffer.normal",
        "glk.buffer.emphasized",
        "glk.buffer.preformatted",
        "glk.buffer.header",
        "glk.buffer.subheader",
        "glk.buffer.alert",
        "glk.buffer.note",
        "glk.buffer.blockquote",
        "glk.buffer.input",
        "glk.buffer.user1",
        "glk.buffer.user2",
        // §3 glk.grid.*
        "glk.grid.normal",
        "glk.grid.emphasized",
        "glk.grid.preformatted",
        "glk.grid.header",
        "glk.grid.subheader",
        "glk.grid.alert",
        "glk.grid.note",
        "glk.grid.blockquote",
        "glk.grid.input",
        "glk.grid.user1",
        "glk.grid.user2",
        // §4 map.*
        "map.background",
        "map.room",
        "map.room_current",
        "map.room_selected",
        "map.connector",
        "map.connector_distorted",
        "map.connector_portal",
        "map.shared_path",
        "map.loc_indicator",
        "map.layer_cycle",
        "map.box_style",
        "map.arrow_set",
        "map.portal_icons",
        "map.path_style",
        "map.portal_path_style",
        // §4b debug.*
        "debug.pc",
        "debug.tooltip",
        "debug.disasm_executed",
        "debug.disasm_rd",
        "debug.disasm_soft",
        "debug.disasm_data",
        // §2 elements (expansion, Task 5.0)
        "more_prompt",
        "tidy_progress",
        "meta_marker",
        "inventory_dock",
        "story_info_title",
        "story_info_value",
        "story_info_blurb",
        "story_info_link",
        "story_info_cover",
        "graphics",
        "inline_image",
        "story_header",
        "story_header_active",
        "story_author",
        "story_year",
        "story_no_metadata",
        "story_tile",
        "story_tile_selected",
        "notification",
        "dialog",
        "dialog_title",
        "dialog_button",
        "dialog_button_active",
        "dialog_shadow",
        "hotkey_key",
        "sound_beep_high",
        "sound_beep_low",
        "transcript_input",
        "transcript_system",
        "warning_marker",
        "input_text",
        "input_prompt",
        "upper_window_border",
        "room_panel",
    ];

    #[test]
    fn expanded_elements_resolve_from_roles() {
        use crate::theme::resolve::{resolve, Decls, Roles};
        use ratatui::style::Modifier;

        let roles = Roles::terminal_default();
        let theme = resolve(&roles, &Decls::new(), &Decls::new(), &Decls::new());

        // story_info_blurb: muted + italic — fg snaps to muted, ITALIC preserved.
        let blurb = theme.get("story_info_blurb").style;
        assert_eq!(blurb.fg, roles.muted.fg);
        assert!(blurb.add_modifier.contains(Modifier::ITALIC));

        // sound_beep_high: plain alert derivation (orange -> yellow accepted).
        assert_eq!(theme.get("sound_beep_high").style.fg, theme.get("alert").style.fg);

        // dialog_shadow: the one explicit-colour row — keeps an explicit dark-gray bg.
        assert_eq!(theme.get("dialog_shadow").style.bg, Some(Color::DarkGray));

        // story_tile_selected: accent + reversed + bold.
        let tile_selected = theme.get("story_tile_selected").style;
        assert!(tile_selected.add_modifier.contains(Modifier::REVERSED | Modifier::BOLD));
    }

    #[test]
    fn registry_is_complete_and_unique() {
        // (b) names are unique
        let mut seen = HashSet::new();
        for r in REGISTRY {
            assert!(seen.insert(r.name), "duplicate registry name: {}", r.name);
        }

        // (a) the registry names are exactly the expected set
        let expected: HashSet<&str> = EXPECTED.iter().copied().collect();
        let missing: Vec<&str> = expected.difference(&seen).copied().collect();
        let extra: Vec<&str> = seen.difference(&expected).copied().collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "registry mismatch — missing: {missing:?}, unexpected: {extra:?}"
        );

        // (c) every non-None parent names a role or another registry row
        let roles: HashSet<&str> = ROLE_NAMES.iter().copied().collect();
        for r in REGISTRY {
            if let Some(p) = r.parent {
                assert!(
                    roles.contains(p) || seen.contains(p),
                    "row {} has parent {p:?} that is neither a role nor a registry selector",
                    r.name
                );
            }
        }
    }
}
