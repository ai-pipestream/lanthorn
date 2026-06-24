//! Color scheme support: Ghostty theme parsing and per-element color resolution.
//!
//! # Overview
//!
//! - [`GhosttyScheme`] holds the raw colors parsed from a Ghostty theme file.
//! - [`ColorScheme`] holds the resolved per-element colors used by the renderer.
//! - [`ColorScheme::terminal_default`] reproduces the hardcoded colors in the current renderer.
//! - [`ColorScheme::from_ghostty`] maps a parsed scheme onto UI elements.
//! - [`ColorScheme::resolve`] is the top-level entry point: reads config, finds the scheme,
//!   applies overrides, and returns the final scheme plus any diagnostic warnings.

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};

use crate::config::ColorsConfig;

// ── Built-in theme texts ──────────────────────────────────────────────────────

const BUILTIN_MONO: &str = include_str!("colors/mono.ghostty");
const BUILTIN_HIGH_CONTRAST: &str = include_str!("colors/high-contrast.ghostty");
const BUILTIN_TOMORROW_NIGHT: &str = include_str!("colors/tomorrow-night.ghostty");

// ── GhosttyScheme ─────────────────────────────────────────────────────────────

/// The raw colors loaded from a Ghostty theme file.
///
/// Ghostty theme files use `key = value` syntax (one per line).  Relevant keys:
/// `palette = N=#rrggbb` (or `rrggbb`), `background`, `foreground`,
/// `cursor-color`, `selection-background`, `selection-foreground`.
/// All other keys are silently ignored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GhosttyScheme {
    /// The 16-color ANSI palette.  Entries that were not specified in the file
    /// default to `Color::Reset`.
    pub palette: [Color; 16],
    pub background: Color,
    pub foreground: Color,
    pub cursor: Option<Color>,
    pub selection_bg: Option<Color>,
    pub selection_fg: Option<Color>,
}

impl GhosttyScheme {
    /// Parse a Ghostty theme file text.  Returns `Err` only when `background`
    /// or `foreground` is missing from the file (they are required for a
    /// meaningful scheme).  All other parsing errors on individual lines are
    /// silently skipped.
    pub fn parse(text: &str) -> Result<GhosttyScheme, String> {
        let mut palette: [Color; 16] = [Color::Reset; 16];
        let mut background: Option<Color> = None;
        let mut foreground: Option<Color> = None;
        let mut cursor: Option<Color> = None;
        let mut selection_bg: Option<Color> = None;
        let mut selection_fg: Option<Color> = None;

        for line in text.lines() {
            // Strip comments and whitespace.
            let line = match line.find('#') {
                // '#' is only a comment when it is at the start of the value
                // OR the whole line.  In Ghostty palette lines the '#' is part
                // of the hex color, so only strip a leading standalone '#'.
                Some(_) if line.trim_start().starts_with('#') => continue,
                _ => line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "palette" => {
                    // Format: N=#rrggbb or N=rrggbb
                    if let Some((idx_s, hex_s)) = value.split_once('=') {
                        let idx_s = idx_s.trim();
                        let hex_s = hex_s.trim();
                        if let Ok(idx) = idx_s.parse::<usize>() {
                            if idx < 16 {
                                if let Some(c) = parse_hex_color(hex_s) {
                                    palette[idx] = c;
                                }
                            }
                        }
                    }
                }
                "background" => {
                    if let Some(c) = parse_hex_color(value) {
                        background = Some(c);
                    }
                }
                "foreground" => {
                    if let Some(c) = parse_hex_color(value) {
                        foreground = Some(c);
                    }
                }
                "cursor-color" => {
                    cursor = parse_hex_color(value);
                }
                "selection-background" => {
                    selection_bg = parse_hex_color(value);
                }
                "selection-foreground" => {
                    selection_fg = parse_hex_color(value);
                }
                _ => {} // ignore unknown keys
            }
        }

        let background = background.ok_or_else(|| "missing 'background' key".to_string())?;
        let foreground = foreground.ok_or_else(|| "missing 'foreground' key".to_string())?;

        Ok(GhosttyScheme {
            palette,
            background,
            foreground,
            cursor,
            selection_bg,
            selection_fg,
        })
    }
}

// ── ColorScheme ───────────────────────────────────────────────────────────────

/// Per-element resolved colors for the renderer.
///
/// Each field is a [`Style`] ready to apply with ratatui.  The renderer should
/// use these instead of its hardcoded color constants once the renderer-wiring
/// track connects `AppState.colors` to the render functions.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    /// Normal (unvisited / unselected) room.
    pub room_normal: Style,
    /// The current room (player is here): rendered with REVERSED video.
    pub room_current: Style,
    /// The selected (cursor-highlighted) room.
    pub room_selected: Style,
    /// Normal connector line (non-distorted).
    pub connector: Style,
    /// Distorted / one-way connector line.
    pub connector_distorted: Style,
    /// Portal (Up/Down/In/Out) connector line.
    pub portal_connector: Style,
    /// Status bar (top of transcript pane).
    pub status_bar: Style,
    /// Transcript text (body of transcript pane).
    pub transcript: Style,
    /// Autocomplete suggestion line.
    pub suggestion: Style,
    /// Focused-pane border.
    pub focused_border: Style,
    /// Help bar (bottom row).
    pub help_bar: Style,
}

impl ColorScheme {
    /// Reproduce the exact colors hardcoded in today's renderer constants.
    ///
    /// Matches:
    /// - `render/map.rs`: `CURRENT_STYLE`, `SELECTED_STYLE`, `NORMAL_STYLE`, `CONNECTOR_STYLE`,
    ///   plus the inline `Cyan`/`Magenta` colors and the portal-connector `Cyan`.
    /// - `render/transcript.rs`: `STATUS_STYLE`, `NORMAL_STYLE`, and the `DarkGray` suggestion.
    /// - `main.rs`: `focused_border` (`Cyan + BOLD`) and `help_style` (`REVERSED`).
    pub fn terminal_default() -> ColorScheme {
        ColorScheme {
            room_normal: Style::new().fg(Color::White).bg(Color::Reset),
            room_current: Style::new()
                .add_modifier(Modifier::REVERSED)
                .fg(Color::White)
                .bg(Color::Reset),
            room_selected: Style::new().fg(Color::Yellow).bg(Color::Reset),
            connector: Style::new().fg(Color::Cyan),
            connector_distorted: Style::new().fg(Color::Magenta),
            portal_connector: Style::new().fg(Color::Cyan),
            status_bar: Style::new().add_modifier(Modifier::REVERSED),
            transcript: Style::new().fg(Color::White),
            suggestion: Style::new().fg(Color::DarkGray),
            focused_border: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            help_bar: Style::new().add_modifier(Modifier::REVERSED),
        }
    }

    /// Build a `ColorScheme` from a parsed `GhosttyScheme` and optional per-element overrides.
    ///
    /// # Default element→role mapping
    ///
    /// | Element             | Ghostty role          |
    /// |---------------------|-----------------------|
    /// | `room_normal`       | `foreground`          |
    /// | `room_current`      | `reversed(fg, bg)`    |
    /// | `room_selected`     | `palette[3]`          |
    /// | `connector`         | `palette[6]`          |
    /// | `connector_distorted` | `palette[5]`        |
    /// | `portal_connector`  | `palette[6]`          |
    /// | `status_bar`        | `reversed(fg, bg)`    |
    /// | `transcript`        | `foreground`          |
    /// | `suggestion`        | `palette[8]`          |
    /// | `focused_border`    | `palette[6] + bold`   |
    /// | `help_bar`          | `reversed(fg, bg)`    |
    ///
    /// Overrides in `elements` map element names to color values (parsed by
    /// [`parse_color_value`]) and beat the default mapping.
    pub fn from_ghostty(
        scheme: &GhosttyScheme,
        overrides: &BTreeMap<String, String>,
    ) -> ColorScheme {
        let fg = scheme.foreground;
        let bg = scheme.background;

        // Helper: look up element override or fall back to the default color.
        let resolve_element = |name: &str, default: Color| -> Color {
            overrides
                .get(name)
                .and_then(|v| parse_color_value(v, scheme))
                .unwrap_or(default)
        };

        let room_normal_fg = resolve_element("room_normal", fg);
        let connector_fg = resolve_element("connector", scheme.palette[6]);
        let room_selected_fg = resolve_element("room_selected", scheme.palette[3]);
        let connector_distorted_fg =
            resolve_element("connector_distorted", scheme.palette[5]);
        let portal_connector_fg = resolve_element("portal_connector", scheme.palette[6]);
        let transcript_fg = resolve_element("transcript", fg);
        let suggestion_fg = resolve_element("suggestion", scheme.palette[8]);
        let focused_border_fg = resolve_element("focused_border", scheme.palette[6]);

        // REVERSED elements use fg/bg from the scheme; overrides on these elements
        // replace the fg component of the reversed style.
        let status_bar_fg = overrides
            .get("status_bar")
            .and_then(|v| parse_color_value(v, scheme));
        let help_bar_fg = overrides
            .get("help_bar")
            .and_then(|v| parse_color_value(v, scheme));
        let room_current_fg = overrides
            .get("room_current")
            .and_then(|v| parse_color_value(v, scheme));

        let status_bar = if let Some(c) = status_bar_fg {
            Style::new().fg(c).bg(bg).add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
                .fg(fg)
                .bg(bg)
                .add_modifier(Modifier::REVERSED)
        };

        let help_bar = if let Some(c) = help_bar_fg {
            Style::new().fg(c).bg(bg).add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
                .fg(fg)
                .bg(bg)
                .add_modifier(Modifier::REVERSED)
        };

        let room_current = if let Some(c) = room_current_fg {
            Style::new()
                .fg(c)
                .bg(bg)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
                .fg(fg)
                .bg(bg)
                .add_modifier(Modifier::REVERSED)
        };

        ColorScheme {
            room_normal: Style::new().fg(room_normal_fg).bg(bg),
            room_current,
            room_selected: Style::new().fg(room_selected_fg).bg(bg),
            connector: Style::new().fg(connector_fg),
            connector_distorted: Style::new().fg(connector_distorted_fg),
            portal_connector: Style::new().fg(portal_connector_fg),
            status_bar,
            transcript: Style::new().fg(transcript_fg),
            suggestion: Style::new().fg(suggestion_fg),
            focused_border: Style::new()
                .fg(focused_border_fg)
                .add_modifier(Modifier::BOLD),
            help_bar,
        }
    }

    /// Resolve the final `ColorScheme` from config and the user-data directory.
    ///
    /// Resolution order:
    /// 1. `cfg.scheme == None` → [`terminal_default`], then element overrides applied.
    /// 2. A known built-in name → that scheme's embedded text.
    /// 3. Any other string → treated as a file path (`~` expanded; relative to `dir`).
    /// 4. Parse failure or missing file → warning appended, fall back to [`terminal_default`].
    ///
    /// Element overrides from `cfg.elements` are always applied on top (even for `terminal_default`).
    pub fn resolve(cfg: &ColorsConfig, dir: &Path) -> (ColorScheme, Vec<String>) {
        let mut warnings: Vec<String> = Vec::new();

        let scheme_name = cfg.scheme.as_deref();

        let base = match scheme_name {
            None => {
                // No scheme: use terminal defaults, then apply any element overrides.
                if cfg.elements.is_empty() {
                    return (ColorScheme::terminal_default(), warnings);
                }
                // Apply overrides on top of terminal_default by building a pseudo-scheme
                // from ANSI named colors then running from_ghostty.
                // Simpler: just apply element overrides directly to terminal_default.
                let mut cs = ColorScheme::terminal_default();
                apply_terminal_overrides(&mut cs, &cfg.elements);
                return (cs, warnings);
            }
            Some(name) => {
                match builtin_scheme_text(name) {
                    Some(text) => match GhosttyScheme::parse(text) {
                        Ok(gs) => gs,
                        Err(e) => {
                            warnings.push(format!(
                                "built-in scheme '{}' failed to parse: {}; using terminal defaults",
                                name, e
                            ));
                            let mut cs = ColorScheme::terminal_default();
                            apply_terminal_overrides(&mut cs, &cfg.elements);
                            return (cs, warnings);
                        }
                    },
                    None => {
                        // Not a built-in: treat as a file path.
                        let path = expand_path(name, dir);
                        match std::fs::read_to_string(&path) {
                            Ok(text) => match GhosttyScheme::parse(&text) {
                                Ok(gs) => gs,
                                Err(e) => {
                                    warnings.push(format!(
                                        "scheme file '{}' failed to parse: {}; using terminal defaults",
                                        path.display(),
                                        e
                                    ));
                                    let mut cs = ColorScheme::terminal_default();
                                    apply_terminal_overrides(&mut cs, &cfg.elements);
                                    return (cs, warnings);
                                }
                            },
                            Err(e) => {
                                warnings.push(format!(
                                    "could not read scheme file '{}': {}; using terminal defaults",
                                    path.display(),
                                    e
                                ));
                                let mut cs = ColorScheme::terminal_default();
                                apply_terminal_overrides(&mut cs, &cfg.elements);
                                return (cs, warnings);
                            }
                        }
                    }
                }
            }
        };

        let cs = ColorScheme::from_ghostty(&base, &cfg.elements);
        (cs, warnings)
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Return the embedded Ghostty theme text for a known built-in name, or `None`.
fn builtin_scheme_text(name: &str) -> Option<&'static str> {
    match name {
        "mono" => Some(BUILTIN_MONO),
        "high-contrast" => Some(BUILTIN_HIGH_CONTRAST),
        "tomorrow-night" => Some(BUILTIN_TOMORROW_NIGHT),
        _ => None,
    }
}

/// Expand `~` in a path string and resolve relative paths against `base_dir`.
fn expand_path(s: &str, base_dir: &Path) -> std::path::PathBuf {
    let expanded = if s.starts_with('~') {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(s.trim_start_matches("~/").trim_start_matches('~'))
    } else {
        std::path::PathBuf::from(s)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        base_dir.join(expanded)
    }
}

/// Parse a hex color string (`#rrggbb` or `rrggbb`) into `Color::Rgb`.
/// Returns `None` on invalid input.
pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

/// Parse a color value from a `[colors.elements]` entry.
///
/// Accepted formats:
/// - `palette:N`  — index 0-15 into the scheme's palette (requires a scheme)
/// - `background` / `foreground` — the scheme's bg/fg (requires a scheme)
/// - A named ratatui color (`cyan`, `yellow`, …) — case-insensitive
/// - A decimal 256-index (`"17"`)
/// - A hex color (`#5fafd7` or `5fafd7`)
///
/// Returns `None` if the value cannot be parsed.
pub fn parse_color_value(value: &str, scheme: &GhosttyScheme) -> Option<Color> {
    let v = value.trim();

    // palette:N
    if let Some(rest) = v.strip_prefix("palette:") {
        if let Ok(idx) = rest.trim().parse::<usize>() {
            if idx < 16 {
                return Some(scheme.palette[idx]);
            }
        }
        return None;
    }

    // scheme-relative roles
    match v {
        "background" => return Some(scheme.background),
        "foreground" => return Some(scheme.foreground),
        _ => {}
    }

    // ratatui named colors (case-insensitive)
    if let Some(c) = parse_named_color(v) {
        return Some(c);
    }

    // 256-index
    if let Ok(idx) = v.parse::<u8>() {
        return Some(Color::Indexed(idx));
    }

    // hex
    parse_hex_color(v)
}

/// Parse a ratatui named color (case-insensitive).
///
/// Accepts the standard ANSI names (`black`, `red`, … `white`) and their
/// `bright-*` / `light-*` / `dark-*` variants. `bright-black` and `dark-black`
/// both map to `DarkGray`.
pub fn parse_named_color(s: &str) -> Option<Color> {
    match s.to_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "white" => Some(Color::White),
        "reset" => Some(Color::Reset),
        // dark- variants
        "dark-gray" | "dark-grey" | "darkgray" | "dark_gray" | "darkgrey" | "dark_grey"
        | "bright-black" | "dark-black" => Some(Color::DarkGray),
        // light- / bright- variants
        "light-red" | "lightred" | "light_red" | "bright-red" => Some(Color::LightRed),
        "light-green" | "lightgreen" | "light_green" | "bright-green" => Some(Color::LightGreen),
        "light-yellow" | "lightyellow" | "light_yellow" | "bright-yellow" => {
            Some(Color::LightYellow)
        }
        "light-blue" | "lightblue" | "light_blue" | "bright-blue" => Some(Color::LightBlue),
        "light-magenta" | "lightmagenta" | "light_magenta" | "bright-magenta" => {
            Some(Color::LightMagenta)
        }
        "light-cyan" | "lightcyan" | "light_cyan" | "bright-cyan" => Some(Color::LightCyan),
        "light-white" | "bright-white" => Some(Color::White),
        "light-black" | "bright-gray" | "bright-grey" | "light-gray" | "light-grey" => {
            Some(Color::Gray)
        }
        _ => None,
    }
}

/// Apply per-element overrides to an existing `ColorScheme` using terminal default
/// colors as context (no Ghostty scheme available).  Accepts only named colors,
/// 256-index, and hex values — `palette:N` / `background` / `foreground` have no
/// meaning without a scheme and are ignored with no error.
fn apply_terminal_overrides(cs: &mut ColorScheme, elements: &BTreeMap<String, String>) {
    // Build a dummy scheme so we can reuse parse_color_value; palette/bg/fg are ANSI defaults.
    let dummy = dummy_ansi_scheme();
    for (element, value) in elements {
        if let Some(c) = parse_color_value(value, &dummy) {
            match element.as_str() {
                "room_normal" => cs.room_normal = cs.room_normal.fg(c),
                "room_current" => cs.room_current = cs.room_current.fg(c),
                "room_selected" => cs.room_selected = cs.room_selected.fg(c),
                "connector" => cs.connector = cs.connector.fg(c),
                "connector_distorted" => cs.connector_distorted = cs.connector_distorted.fg(c),
                "portal_connector" => cs.portal_connector = cs.portal_connector.fg(c),
                "status_bar" => cs.status_bar = cs.status_bar.fg(c),
                "transcript" => cs.transcript = cs.transcript.fg(c),
                "suggestion" => cs.suggestion = cs.suggestion.fg(c),
                "focused_border" => cs.focused_border = cs.focused_border.fg(c),
                "help_bar" => cs.help_bar = cs.help_bar.fg(c),
                _ => {} // unknown element names are silently ignored
            }
        }
    }
}

/// A dummy `GhosttyScheme` using approximate ANSI named colors, used when
/// resolving element overrides in terminal-default mode (no file scheme).
fn dummy_ansi_scheme() -> GhosttyScheme {
    use Color::*;
    GhosttyScheme {
        palette: [
            Black, Red, Green, Yellow, Blue, Magenta, Cyan, White,
            DarkGray, LightRed, LightGreen, LightYellow, LightBlue, LightMagenta, LightCyan, White,
        ],
        background: Black,
        foreground: White,
        cursor: None,
        selection_bg: None,
        selection_fg: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_color_value_accepts_named_colors() {
        let scheme = GhosttyScheme::default(); // or a minimal scheme
        assert_eq!(parse_color_value("red", &scheme), Some(Color::Red));
        assert_eq!(parse_color_value("bright-blue", &scheme), Some(Color::LightBlue));
        assert_eq!(parse_color_value("white", &scheme), Some(Color::White));
    }

    // ── GhosttyScheme::parse ──────────────────────────────────────────────────

    const SAMPLE_THEME: &str = r#"
palette = 0=#1d1f21
palette = 1=#cc6666
palette = 6=#70c0ba
palette = 8=#373b41
palette = 15=#ffffff
background = 1d1f21
foreground = c5c8c6
cursor-color = c5c8c6
selection-background = 373b41
selection-foreground = c5c8c6
unknown-key = ignored
"#;

    #[test]
    fn parse_palette_entry() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.palette[1], Color::Rgb(0xcc, 0x66, 0x66));
        assert_eq!(gs.palette[6], Color::Rgb(0x70, 0xc0, 0xba));
        assert_eq!(gs.palette[15], Color::Rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn parse_background_foreground() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.background, Color::Rgb(0x1d, 0x1f, 0x21));
        assert_eq!(gs.foreground, Color::Rgb(0xc5, 0xc8, 0xc6));
    }

    #[test]
    fn parse_optional_fields() {
        let gs = GhosttyScheme::parse(SAMPLE_THEME).unwrap();
        assert_eq!(gs.cursor, Some(Color::Rgb(0xc5, 0xc8, 0xc6)));
        assert_eq!(gs.selection_bg, Some(Color::Rgb(0x37, 0x3b, 0x41)));
        assert_eq!(gs.selection_fg, Some(Color::Rgb(0xc5, 0xc8, 0xc6)));
    }

    #[test]
    fn parse_missing_optional_fields_are_none() {
        let text = "background = 000000\nforeground = ffffff\n";
        let gs = GhosttyScheme::parse(text).unwrap();
        assert!(gs.cursor.is_none());
        assert!(gs.selection_bg.is_none());
        assert!(gs.selection_fg.is_none());
    }

    #[test]
    fn parse_malformed_lines_are_skipped() {
        // The theme has a malformed palette line and an invalid hex; both should be ignored.
        let text = "background = 000000\nforeground = ffffff\npalette = notanumber=#ff0000\npalette = 3=zzzzzz\n";
        let gs = GhosttyScheme::parse(text).unwrap();
        // Malformed entries leave the slot at Reset.
        assert_eq!(gs.palette[3], Color::Reset);
    }

    #[test]
    fn parse_missing_background_is_error() {
        let text = "foreground = ffffff\n";
        assert!(GhosttyScheme::parse(text).is_err());
    }

    #[test]
    fn parse_missing_foreground_is_error() {
        let text = "background = 000000\n";
        assert!(GhosttyScheme::parse(text).is_err());
    }

    #[test]
    fn parse_hex_with_and_without_hash() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Color::Rgb(0xff, 0, 0)));
        assert_eq!(parse_hex_color("ff0000"), Some(Color::Rgb(0xff, 0, 0)));
        assert_eq!(parse_hex_color("zzzzzz"), None);
    }

    // ── ColorScheme::terminal_default ────────────────────────────────────────

    #[test]
    fn terminal_default_connector_is_cyan() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.connector, Style::new().fg(Color::Cyan));
    }

    #[test]
    fn terminal_default_distorted_is_magenta() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.connector_distorted, Style::new().fg(Color::Magenta));
    }

    #[test]
    fn terminal_default_selected_is_yellow() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.room_selected, Style::new().fg(Color::Yellow).bg(Color::Reset));
    }

    #[test]
    fn terminal_default_suggestion_is_darkgray() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.suggestion, Style::new().fg(Color::DarkGray));
    }

    #[test]
    fn terminal_default_focused_border_is_cyan_bold() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(
            cs.focused_border,
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn terminal_default_status_bar_is_reversed() {
        let cs = ColorScheme::terminal_default();
        assert!(cs.status_bar.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn terminal_default_help_bar_is_reversed() {
        let cs = ColorScheme::terminal_default();
        assert!(cs.help_bar.add_modifier.contains(Modifier::REVERSED));
    }

    // ── ColorScheme::from_ghostty ─────────────────────────────────────────────

    fn sample_scheme() -> GhosttyScheme {
        GhosttyScheme::parse(SAMPLE_THEME).unwrap()
    }

    #[test]
    fn from_ghostty_connector_maps_to_palette6() {
        let gs = sample_scheme();
        let cs = ColorScheme::from_ghostty(&gs, &BTreeMap::new());
        assert_eq!(cs.connector, Style::new().fg(gs.palette[6]));
    }

    #[test]
    fn from_ghostty_distorted_maps_to_palette5() {
        let gs = sample_scheme();
        let cs = ColorScheme::from_ghostty(&gs, &BTreeMap::new());
        assert_eq!(cs.connector_distorted, Style::new().fg(gs.palette[5]));
    }

    #[test]
    fn from_ghostty_selected_maps_to_palette3() {
        let gs = sample_scheme();
        let cs = ColorScheme::from_ghostty(&gs, &BTreeMap::new());
        let expected_fg = gs.palette[3];
        assert_eq!(cs.room_selected, Style::new().fg(expected_fg).bg(gs.background));
    }

    #[test]
    fn element_override_hex_beats_mapping() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        overrides.insert("room_selected".to_string(), "#ff0000".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.room_selected.fg, Some(Color::Rgb(0xff, 0, 0)));
    }

    #[test]
    fn element_override_palette_ref_beats_mapping() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        // Override connector to use palette[1] instead of palette[6].
        overrides.insert("connector".to_string(), "palette:1".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.connector, Style::new().fg(gs.palette[1]));
    }

    #[test]
    fn element_override_named_color_works() {
        let gs = sample_scheme();
        let mut overrides = BTreeMap::new();
        overrides.insert("connector".to_string(), "cyan".to_string());
        let cs = ColorScheme::from_ghostty(&gs, &overrides);
        assert_eq!(cs.connector, Style::new().fg(Color::Cyan));
    }

    // ── ColorScheme::resolve ──────────────────────────────────────────────────

    #[test]
    fn resolve_no_scheme_gives_terminal_default() {
        let cfg = ColorsConfig::default();
        let dir = std::path::Path::new("/tmp");
        let (cs, warnings) = ColorScheme::resolve(&cfg, dir);
        assert_eq!(cs, ColorScheme::terminal_default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn resolve_builtin_tomorrow_night() {
        let cfg = ColorsConfig {
            scheme: Some("tomorrow-night".to_string()),
            elements: BTreeMap::new(),
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(warnings.is_empty());
        // Tomorrow Night palette[6] = #70c0ba.
        let expected_connector_fg = Color::Rgb(0x70, 0xc0, 0xba);
        assert_eq!(cs.connector.fg, Some(expected_connector_fg));
    }

    #[test]
    fn resolve_builtin_mono() {
        let cfg = ColorsConfig {
            scheme: Some("mono".to_string()),
            elements: BTreeMap::new(),
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);
        // Just check it resolved without errors and connector has some color.
        let _ = cs.connector;
    }

    #[test]
    fn resolve_bad_path_warns_and_falls_back() {
        let cfg = ColorsConfig {
            scheme: Some("/nonexistent/path/theme.ghostty".to_string()),
            elements: BTreeMap::new(),
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(!warnings.is_empty(), "expected a warning for missing file");
        assert_eq!(cs, ColorScheme::terminal_default());
    }

    #[test]
    fn resolve_file_path_loads_scheme() {
        // Write a temp Ghostty theme file and resolve it.
        let path = std::env::temp_dir().join("babelmap_test_colors_resolve.ghostty");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "background = 1d1f21").unwrap();
            writeln!(f, "foreground = c5c8c6").unwrap();
            writeln!(f, "palette = 6=#aabbcc").unwrap();
        }
        let cfg = ColorsConfig {
            scheme: Some(path.to_string_lossy().to_string()),
            elements: BTreeMap::new(),
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);
        assert_eq!(cs.connector.fg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn resolve_no_scheme_with_element_override() {
        let mut elements = BTreeMap::new();
        elements.insert("connector".to_string(), "#aabbcc".to_string());
        let cfg = ColorsConfig {
            scheme: None,
            elements,
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(warnings.is_empty());
        assert_eq!(cs.connector.fg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn resolve_bad_parse_warns_and_falls_back() {
        // Write a malformed theme file (no background/foreground).
        let path = std::env::temp_dir().join("babelmap_test_colors_bad_parse.ghostty");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "palette = 6=#aabbcc").unwrap();
        }
        let cfg = ColorsConfig {
            scheme: Some(path.to_string_lossy().to_string()),
            elements: BTreeMap::new(),
        };
        let (cs, warnings) = ColorScheme::resolve(&cfg, std::path::Path::new("/tmp"));
        assert!(!warnings.is_empty(), "expected a warning for bad parse");
        assert_eq!(cs, ColorScheme::terminal_default());
    }
}
