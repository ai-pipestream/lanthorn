//! The new `style.toml` schema parser (SQ-0309, Task 1.2): raw TOML text →
//! [`ParsedStyle`], a struct of raw, unresolved strings.
//!
//! This does NOT resolve colours, parents, or the [`super::registry`] table —
//! that is the next task. Every value here is kept exactly as it appears in the
//! file (colours as strings, parents as strings, presets as strings).

use std::collections::BTreeMap;

/// A raw, unresolved style declaration straight from a TOML inline table. Every
/// field is optional and kept as the literal text/flag from the file — colours are
/// NOT resolved to `Color` here (Task 1.2 does that against the base scheme).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawDelta {
    /// `parent = "..."` — re-roots this selector onto a role or another selector.
    pub parent: Option<String>,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub dim: Option<bool>,
    pub reversed: Option<bool>,
    /// A single glyph the selector carries (gutter mark, tab divider, terminator
    /// cap) OR — for a map preset key like `box_style = "rounded"` — the preset name.
    pub glyph: Option<String>,
    /// Named glyph-slot overrides: `glyphs = { tl = "+", h = "-" }` on map selectors.
    pub glyphs: BTreeMap<String, String>,
    /// A list value — only `map.layer_cycle = ["cyan", ...]` uses this.
    pub list: Option<Vec<String>>,
    /// Border keys (carried raw; applied later): `style`, `style_<side>`, `header`, `shadow`.
    pub style: Option<String>,
    pub style_top: Option<String>,
    pub style_bottom: Option<String>,
    pub style_left: Option<String>,
    pub style_right: Option<String>,
    pub header: Option<bool>,
    pub shadow: Option<bool>,
}

/// A raw `[[transcript.rule]]`: a regex `match` plus style keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawRule {
    pub pattern: String, // the `match` key (skip a rule whose match is missing/empty)
    pub delta: RawDelta, // fg/bg/mods (reuse the same field parser)
}

/// A raw `[[statusbar.segment]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSegment {
    pub text: String,  // `text` key (default "")
    pub align: String, // `align` key: "left"|"center"|"right" (default "left")
    pub delta: RawDelta, // parent/fg/bg/mods
}

/// A raw `[statusbar]` block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawStatusBar {
    pub border: Option<String>,
    pub border_fg: Option<String>,
    pub segments: Vec<RawSegment>,
}

/// The whole parsed style document, all raw. `decls` is keyed by FULL registry
/// selector name (see prefixing rule below); `roles` is keyed by bare role name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedStyle {
    pub scheme: Option<String>,            // top-level `scheme = "..."`
    pub roles: BTreeMap<String, RawDelta>, // [roles]: keys "text","accent",...
    pub decls: BTreeMap<String, RawDelta>, // elements+panel+glk.*+map+debug, FULL selector names
    pub transcript_rules: Vec<RawRule>,
    pub statusbar: RawStatusBar,
}

/// Parse `style.toml` text. Returns the raw document, or a list of human-readable
/// error strings (at minimum: one entry for a TOML syntax error).
pub fn parse(text: &str) -> Result<ParsedStyle, Vec<String>> {
    let root: toml::Value = text
        .parse()
        .map_err(|e| vec![format!("TOML parse error: {e}")])?;
    let root = root
        .as_table()
        .ok_or_else(|| vec!["style.toml: expected a table at the document root".to_string()])?;

    let mut parsed = ParsedStyle {
        scheme: root.get("scheme").and_then(toml::Value::as_str).map(String::from),
        ..ParsedStyle::default()
    };

    if let Some(toml::Value::Table(roles)) = root.get("roles") {
        for (key, val) in roles {
            if let toml::Value::Table(t) = val {
                parsed.roles.insert(key.clone(), delta_from_table(t));
            }
        }
    }

    if let Some(toml::Value::Table(elements)) = root.get("elements") {
        for (key, val) in elements {
            if let toml::Value::Table(t) = val {
                parsed.decls.insert(key.clone(), delta_from_table(t));
            }
        }
    }

    if let Some(toml::Value::Table(panel)) = root.get("panel") {
        for (key, val) in panel {
            if let toml::Value::Table(t) = val {
                parsed.decls.insert(format!("panel.{key}"), delta_from_table(t));
            }
        }
    }

    if let Some(toml::Value::Table(glk)) = root.get("glk") {
        for sub in ["buffer", "grid"] {
            if let Some(toml::Value::Table(sub_table)) = glk.get(sub) {
                for (key, val) in sub_table {
                    if let toml::Value::Table(t) = val {
                        parsed.decls.insert(format!("glk.{sub}.{key}"), delta_from_table(t));
                    }
                }
            }
        }
    }

    if let Some(toml::Value::Table(map)) = root.get("map") {
        for (key, val) in map {
            let full_key = format!("map.{key}");
            match val {
                toml::Value::Table(t) => {
                    parsed.decls.insert(full_key, delta_from_table(t));
                }
                toml::Value::String(s) => {
                    parsed.decls.insert(
                        full_key,
                        RawDelta { glyph: Some(s.clone()), ..RawDelta::default() },
                    );
                }
                toml::Value::Array(items) => {
                    let list: Vec<String> =
                        items.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                    parsed.decls.insert(
                        full_key,
                        RawDelta { list: Some(list), ..RawDelta::default() },
                    );
                }
                _ => {}
            }
        }
    }

    if let Some(toml::Value::Table(debug)) = root.get("debug") {
        for (key, val) in debug {
            if let toml::Value::Table(t) = val {
                parsed.decls.insert(format!("debug.{key}"), delta_from_table(t));
            }
        }
    }

    if let Some(toml::Value::Table(tr_table)) = root.get("transcript") {
        if let Some(toml::Value::Array(rules)) = tr_table.get("rule") {
            for item in rules {
                if let toml::Value::Table(rt) = item {
                    let pattern =
                        rt.get("match").and_then(toml::Value::as_str).unwrap_or("").to_string();
                    if pattern.is_empty() {
                        continue; // a rule with no `match` is skipped
                    }
                    let delta = delta_from_table(rt);
                    parsed.transcript_rules.push(RawRule { pattern, delta });
                }
            }
        }
    }

    if let Some(toml::Value::Table(sb)) = root.get("statusbar") {
        parsed.statusbar.border = sb.get("border").and_then(toml::Value::as_str).map(String::from);
        parsed.statusbar.border_fg =
            sb.get("border_fg").and_then(toml::Value::as_str).map(String::from);
        if let Some(toml::Value::Array(segs)) = sb.get("segment") {
            for item in segs {
                if let toml::Value::Table(st) = item {
                    let text = st.get("text").and_then(toml::Value::as_str).unwrap_or("").to_string();
                    let align =
                        st.get("align").and_then(toml::Value::as_str).unwrap_or("left").to_string();
                    let delta = delta_from_table(st);
                    parsed.statusbar.segments.push(RawSegment { text, align, delta });
                }
            }
        }
    }

    Ok(parsed)
}

/// Parse a [`RawDelta`] from a TOML inline table (field-by-field). Unknown keys
/// are ignored (forward-compat), same as the old parser.
fn delta_from_table(t: &toml::value::Table) -> RawDelta {
    let mut glyphs = BTreeMap::new();
    if let Some(toml::Value::Table(g)) = t.get("glyphs") {
        for (key, val) in g {
            if let Some(s) = val.as_str() {
                glyphs.insert(key.clone(), s.to_string());
            }
        }
    }

    RawDelta {
        parent: t.get("parent").and_then(toml::Value::as_str).map(String::from),
        fg: t.get("fg").and_then(toml::Value::as_str).map(String::from),
        bg: t.get("bg").and_then(toml::Value::as_str).map(String::from),
        bold: t.get("bold").and_then(toml::Value::as_bool),
        italic: t.get("italic").and_then(toml::Value::as_bool),
        underline: t.get("underline").and_then(toml::Value::as_bool),
        dim: t.get("dim").and_then(toml::Value::as_bool),
        reversed: t.get("reversed").and_then(toml::Value::as_bool),
        glyph: t.get("glyph").and_then(toml::Value::as_str).map(String::from),
        glyphs,
        list: None,
        style: t.get("style").and_then(toml::Value::as_str).map(String::from),
        style_top: t.get("style_top").and_then(toml::Value::as_str).map(String::from),
        style_bottom: t.get("style_bottom").and_then(toml::Value::as_str).map(String::from),
        style_left: t.get("style_left").and_then(toml::Value::as_str).map(String::from),
        style_right: t.get("style_right").and_then(toml::Value::as_str).map(String::from),
        header: t.get("header").and_then(toml::Value::as_bool),
        shadow: t.get("shadow").and_then(toml::Value::as_bool),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
scheme = "tomorrow-night"     # optional base: built-in name or a Ghostty theme path; omit for terminal colours

# ── Roles: the 7 roots everything derives from ───────────────────────────────
[roles]
text    = { fg = "white",     bg = "background" }  # body ink on the page
chrome  = { fg = "white",     bg = "black" }       # ink on a UI surface (bars/panels/upper window)
border  = { fg = "cyan" }                          # lines / frames
accent  = { fg = "cyan" }                          # highlight: links, selection, current room, badges
muted   = { fg = "dark-gray" }                     # dim / secondary
alert   = { fg = "yellow" }                        # warning / error
heading = { fg = "white",     bold = true }        # titles / headers

# ── Panel border: ONE frame for every panel we draw, with focus states. ──────
# ── (Story/VM windows have their own borders — see [glk.*], not here.) ────────
[panel]
background       = { parent = "chrome" }                               # standard panel body fill (all panels)
border           = { parent = "border", style = "single" }             # unfocused frame
"border:active"  = { parent = "border", style = "single", bold = true } # focused frame (today's cyan+bold)
title            = { parent = "heading" }                              # a plain panel title
tab              = { parent = "muted" }                                # inactive tab
"tab:active"     = { parent = "accent", bold = true }                  # active/selected tab
tab_divider      = { glyph = "│", parent = "border" }                  # between tabs
terminator_left  = { parent = "border" }        # cap where the strip meets the frame; glyph defaults to border style (┤)
terminator_right = { parent = "border" }        # …├ ; set glyph = "…" to override

# ── Elements: parent + delta. Shown in full; every line == its default, so a ──
# ── minimal theme may delete this whole section and look identical. ───────────
[elements]
transcript           = { parent = "text" }
status_bar           = { parent = "chrome", reversed = true }
help_bar             = { parent = "chrome", reversed = true }
upper_window         = { parent = "chrome" }
story_info           = { parent = "chrome" }
status_header        = { parent = "heading", bg = "black" }
story_title          = { parent = "heading" }
input_line           = { parent = "border" }        # add style = "single" to box it
suggestion_line      = { parent = "border" }        # add style = "single" to box the popup
dialog               = { bg = "black", shadow = true }   # frame = panel.border:active (modal); shadow/title/buttons are dialog-specific
scrollbar            = { parent = "border" }
transcript_location  = { parent = "accent" }
story_badge          = { parent = "accent", reversed = true }
hyperlink            = { parent = "accent", underline = true }
# NB: panel frames (story/map/verb/debug/dialog) come from [panel] (§2a); the
# upper-window frame is a WINDOW border (game domain), not set here; map/debug
# selectors live in [map]/[debug].
story_info_label     = { parent = "muted" }
suggestion           = { parent = "muted" }
transcript_meta      = { parent = "muted", glyph = "▏" }   # glyph = the meta gutter mark (was symbols "gutter.meta")
transcript_warning   = { parent = "alert", glyph = "!" }   # glyph = the warning gutter mark
transcript_crash     = { parent = "alert", bold = true }

# ── The 11 Glk styles × buffer/grid. Overrides only; unset = §3 canonical ────
# ── default. Buffer base = text, grid base = chrome. Window background of a ───
# ── window type = its `normal.bg` here (§3b). ────────────────────────────────
[glk.buffer]
normal       = { parent = "text" }
emphasized   = { parent = "text",    italic = true }
preformatted = { parent = "text" }                 # mono is a TUI no-op — colour only
header       = { parent = "heading", bold = true }
subheader    = { parent = "heading", bold = true }
alert        = { parent = "alert",   bold = true }
note         = { parent = "muted",   italic = true }
blockquote   = { parent = "muted" }
input        = { parent = "accent" }               # echoed player input
user1        = { parent = "text" }
user2        = { parent = "text" }

[glk.grid]
normal       = { parent = "chrome" }               # status/upper-window background lives here
emphasized   = { parent = "chrome",  italic = true }
preformatted = { parent = "chrome" }
header       = { parent = "heading", bold = true }
subheader    = { parent = "heading", bold = true }
alert        = { parent = "alert",   bold = true }
note         = { parent = "muted" }
blockquote   = { parent = "muted" }
input        = { parent = "accent" }
user1        = { parent = "chrome" }
user2        = { parent = "chrome" }

# ── Map palette — hybrid (c): own tokens for fills/edges, but current/selected ─
# ── reference the shared `accent` so highlight colour stays consistent. ───────
# ── (map.* model still an open question §4 — this shows the recommended shape.) ─
[map]
background            = { bg = "background" }        # overrides panel.background for the map canvas (frame = panel.border)
room                 = { fg = "white", glyphs = { tl = "+" } }             # box glyphs: add glyphs = { tl = "+", h = "-" } to override individual slots
room_current         = { parent = "accent" }
room_selected        = { parent = "accent", reversed = true }
connector            = { fg = "cyan" }              # arrow glyphs: add glyphs = { north = "^", up = "↑" } to override
connector_distorted  = { fg = "magenta" }
connector_portal     = { fg = "cyan" }
shared_path          = { fg = "light-cyan" }
layer_cycle          = ["cyan", "green", "magenta", "yellow", "blue"]  # per-layer edge colour cycle
loc_indicator        = { parent = "muted" }
# ── Map-wide glyph SET presets (the old [symbols] section, merged in). Per-glyph─
# ── overrides attach to the selectors above via a `glyphs` sub-map, not a table.─
box_style            = "rounded"    # room boxes: rounded | thick | double | solid | super-thick | ascii | borderless
arrow_set            = "filled"     # cardinal connector arrows: filled | line | nerdfont | nf-bold | nf-box | nf-circle | nf-outline
portal_icons         = "ascii"      # up/down/in/out endpoint icons: ascii | nerdfont | nerdfont-stairs
path_style           = "light"      # cardinal (N/S/E/W) connector line: light | heavy | dotted
portal_path_style    = "dotted"     # up/down/in/out connector line — styled separately from cardinal paths
# NB: the map's layer tabs render through [panel] tab/tab:active (§2b), not here.

# ── Debug inspector (§4b): host UI. Uses standard [panel] chrome for body / ───
# ── frame / tabs; debug.* holds ONLY the disassembly-specific selectors. ──────
# ── Confidence tiers are the SQ-0428 disasm colouring folded into the registry.─
[debug]
pc                  = { parent = "accent", reversed = true }   # line at the current PC
tooltip             = { parent = "chrome" }
# Confidence tiers (SQ-0428): each styles the disasm line AND sets its gutter mark
# glyph. disasm_executed's glyph IS the executed gutter mark (was exec_mark).
disasm_executed     = { parent = "accent", glyph = "|" }
disasm_rd           = { parent = "text",   glyph = " " }
disasm_soft         = { parent = "muted",  glyph = " " }       # e.g. glyph = "?" to mark guesses
disasm_data         = { parent = "muted",  glyph = " ", italic = true }

# ── Story-line styling rules ─────────────────────────────────────────────────
# Each rule recolours whole transcript lines that match a regex. Rules are tried
# in order; the first match wins, ahead of the built-in location/system rules.
# `match` is a regex (`(?i)` = case-insensitive, `\b` = word boundary); the other
# keys are the same style keys as any selector (fg/bg/bold/italic/…).
[[transcript.rule]]
match = "^>.*"                 # your echoed command lines → magenta bold
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"       # any line mentioning a "grue" → red (flavour example)
fg = "red"

# ── Status bar (carried over). Omit [statusbar] for the built-in default. ─────
# Placeholders: {location} {score} {moves} {time} {turns} {title} {filter}
[statusbar]
border = "none"

[[statusbar.segment]]
text  = "{location}"
align = "left"
parent = "accent"              # segments may reference a role or set fg/bg directly
bold  = true

[[statusbar.segment]]
text  = "Score: {score}  Moves: {moves}"
align = "right"

[[statusbar.segment]]
text  = "{time}"
align = "right"
"#;

    #[test]
    fn parses_the_spec_example() {
        let p = parse(FIXTURE).expect("fixture should parse");

        assert_eq!(p.scheme, Some("tomorrow-night".to_string()));
        assert_eq!(p.roles["accent"].fg, Some("cyan".to_string()));

        assert_eq!(p.decls["panel.border:active"].bold, Some(true));
        assert_eq!(p.decls["panel.border:active"].style, Some("single".to_string()));

        assert_eq!(p.decls["glk.buffer.header"].bold, Some(true));
        assert_eq!(p.decls["glk.buffer.header"].parent, Some("heading".to_string()));

        assert_eq!(p.decls["glk.buffer.emphasized"].italic, Some(true));

        assert_eq!(p.decls["map.box_style"].glyph, Some("rounded".to_string()));

        assert_eq!(
            p.decls["map.layer_cycle"].list,
            Some(vec![
                "cyan".to_string(),
                "green".to_string(),
                "magenta".to_string(),
                "yellow".to_string(),
                "blue".to_string(),
            ])
        );

        assert!(p.decls.contains_key("map.room"));
        assert_eq!(p.decls["map.room"].glyphs["tl"], "+");

        assert_eq!(p.decls["debug.disasm_executed"].glyph, Some("|".to_string()));

        assert_eq!(p.decls["transcript_meta"].glyph, Some("▏".to_string()));

        assert_eq!(p.transcript_rules.len(), 2);
        assert_eq!(p.transcript_rules[0].pattern, "^>.*");
        assert_eq!(p.transcript_rules[0].delta.fg, Some("magenta".to_string()));

        assert_eq!(p.statusbar.border, Some("none".to_string()));
        assert_eq!(p.statusbar.segments.len(), 3);
        assert_eq!(p.statusbar.segments[0].text, "{location}");
        assert_eq!(p.statusbar.segments[0].delta.parent, Some("accent".to_string()));
    }

    #[test]
    fn returns_err_on_bad_toml() {
        assert!(parse("= = =").is_err());
    }
}
