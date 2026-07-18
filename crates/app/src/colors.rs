//! Color scheme support: Ghostty theme parsing and per-element color resolution.
//!
//! # Overview
//!
//! - [`GhosttyScheme`] holds the raw colors parsed from a Ghostty theme file.
//! - [`ColorScheme`] holds the resolved per-element colors used by the renderer.
//! - [`ColorScheme::terminal_default`] reproduces the hardcoded colors in the current renderer.
//! - [`ColorScheme::from_ghostty`] maps a parsed scheme onto UI elements.
//! - `resolve_base` is the live entry point: resolves a scheme name/path to a
//!   `(ColorScheme, GhosttyScheme, warnings)` triple used by `style::resolve`.

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

use crate::render::dialog::DialogPlacement;
use crate::render::paneframe::{BorderStyle, PaneGlyphs, PaneSides};

/// A compiled user transcript-styling rule: a regex matched whole-line against
/// Story text, plus the `Style` patched over the base `transcript` style on a
/// match. `PartialEq` compares the source `pattern` and `style` only — the
/// compiled `Regex` has no `PartialEq`, and two rules with the same pattern are
/// equal by construction.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub pattern: String,
    pub regex: Regex,
    pub style: Style,
}

impl PartialEq for CompiledRule {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.style == other.style
    }
}

/// One per-Glk-style theme colour slot: an optional foreground / background
/// applied between the game's stylehint and the per-app-element base (SQ-0331).
/// `None` = inherit (fall through to the element). `Default` = both `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlkStyleColour {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

/// Which alignment cluster a status-bar segment belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    /// The lowercase config name for this alignment.
    pub fn as_str(&self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}

/// One resolved status-bar segment: a text template, its cluster, and the style
/// patched over the base `statusbar` style.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSegment {
    pub text: String,
    pub align: Align,
    pub style: Style,
}

/// The ordered list of status-bar segments. `Default` is the built-in layout that
/// reproduces today's bar (location left; score/moves or clock right; filter right).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBarLayout {
    pub segments: Vec<StatusSegment>,
}

impl Default for StatusBarLayout {
    fn default() -> Self {
        let seg = |text: &str, align: Align| StatusSegment {
            text: text.to_string(),
            align,
            style: Style::default(),
        };
        StatusBarLayout {
            segments: vec![
                seg("{location}", Align::Left),
                seg("Score: {score}  Moves: {moves}", Align::Right),
                seg("{time}", Align::Right),
                seg(" {filter}", Align::Right),
            ],
        }
    }
}

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
    /// A "shared path" connector — one that collapses several same-pair compass
    /// directions into one line. Deliberately BRIGHTER than `connector`; its line,
    /// arrowheads, and secondary markers all use this color.
    pub shared_path: Style,
    /// Status bar (top of transcript pane).
    pub status_bar: Style,
    /// Top-right notification toasts (SQ-0176).
    pub notification: Style,
    /// Transcript text (body of transcript pane).
    pub transcript: Style,
    /// Autocomplete suggestion line.
    pub suggestion: Style,
    /// Live input line: the typed command text (patched over the transcript style).
    pub input_text: Style,
    /// Live input line: the leading prompt character (e.g. `>`).
    pub input_prompt: Style,
    /// Transcript scrollbar (track + thumb).
    pub scrollbar: Style,
    /// Progress bar shown in the map pane while the tidy animation builds off-thread.
    pub tidy_progress: Style,
    /// Gutter marker drawn beside META (app/slash) transcript output.
    pub meta_marker: Style,
    /// Glk hyperlink text (fg colour; an underline is layered on at render time).
    pub hyperlink: Style,
    /// Focused-pane border.
    pub focused_border: Style,
    /// Help bar (bottom row).
    pub help_bar: Style,
    /// Map pane border color.
    pub map_border: Style,
    /// Story pane border color.
    pub story_border: Style,
    /// Story pane title (centered in border).
    pub story_title: Style,
    /// Inventory dock panel: border + item-list text.
    pub inventory_dock: Style,
    /// Story-picker info panel body + border.
    pub story_info: Style,
    /// Story-picker info panel title (story name).
    pub story_info_title: Style,
    /// Story-picker info panel field labels.
    pub story_info_label: Style,
    /// Story-picker info panel field values.
    pub story_info_value: Style,
    /// Story-picker info panel blurb text (the fetched IFDB description).
    pub story_info_blurb: Style,
    pub story_info_link: Style,
    /// Story-picker info panel cover-art letterbox fill (behind/around the
    /// scaled frontispiece image).
    pub story_info_cover: Style,
    /// In-game Glulx graphics-window letterbox fill (behind/around the
    /// fitted canvas image).
    pub graphics: Style,
    /// Inline-image band letterbox fill (behind/around images rendered
    /// inline in text-buffer windows).
    pub inline_image: Style,
    /// Story-picker row badge cluster (type badge + artifact letters); fg + bg.
    pub story_badge: Style,
    /// Story-picker column header row (inactive sort column).
    pub story_header: Style,
    /// Story-picker column header row: the active sort column (shows its
    /// direction arrow).
    pub story_header_active: Style,
    /// Story-picker row: author column text.
    pub story_author: Style,
    /// Story-picker row: year column text.
    pub story_year: Style,
    /// Story-picker row: "(no metadata yet)" placeholder shown in the author
    /// column when a story has no fetched/embedded author.
    pub story_no_metadata: Style,
    /// Story-picker cover-gallery tile caption (title under an unselected cover).
    pub story_tile: Style,
    /// Story-picker cover-gallery tile caption for the selected cover.
    pub story_tile_selected: Style,
    /// Map layer tab (inactive).
    pub map_layer_tab: Style,
    /// Map layer tab (active).
    pub map_layer_tab_active: Style,
    /// Status header style.
    pub status_header: Style,
    /// Input line style.
    pub input_line: Style,
    /// Resolved border style for the map pane.
    pub map_border_style: BorderStyle,
    /// Resolved border style for the story pane.
    pub story_border_style: BorderStyle,
    /// Resolved border style for the status header.
    pub status_header_style: BorderStyle,
    /// Resolved border style for the input line.
    pub input_line_style: BorderStyle,
    /// Resolved border style for the auto-complete suggestion popup.
    pub suggestion_line_style: BorderStyle,
    /// Border style for notification toasts (default `Single`). (SQ-0176)
    pub notification_style: BorderStyle,
    /// Dialog frame background/foreground style.
    pub dialog: Style,
    /// Dialog title text style.
    pub dialog_title: Style,
    /// Leader-panel hotkey letter style.
    pub hotkey_key: Style,
    /// Dialog button (normal) style.
    pub dialog_button: Style,
    /// Dialog button (active/focused) style.
    pub dialog_button_active: Style,
    /// Dialog drop-shadow style.
    pub dialog_shadow: Style,
    /// Resolved border style for the dialog box.
    pub dialog_box_style: BorderStyle,
    /// Whether the dialog drop-shadow is enabled.
    pub dialog_shadow_on: bool,
    /// Where centered modals are anchored on screen (default `Center`).
    pub dialog_placement: DialogPlacement,
    /// Cells of gap from the anchored edge(s); ignored for `Center` (default `0`).
    pub dialog_margin: u16,
    /// Upper (virtual) window content style.
    /// Background/foreground for a split-screen room panel (the Scott top window),
    /// distinct from `transcript` so the two regions read apart. Selector:
    /// `room_panel`.
    pub room_panel: Style,
    pub upper_window: Style,
    /// Upper (virtual) window border style.
    pub upper_window_border: Style,
    /// Resolved border style for the upper (virtual) window frame.
    pub virtual_window_border: BorderStyle,
    /// Per-side border styles (default = all of the matching base `*_style`).
    pub map_border_sides: PaneSides,
    pub story_border_sides: PaneSides,
    pub status_header_sides: PaneSides,
    pub input_line_sides: PaneSides,
    pub suggestion_line_sides: PaneSides,
    pub notification_sides: PaneSides,
    pub upper_window_border_sides: PaneSides,
    /// Per-side/corner glyph overrides for each bordered pane element.
    pub map_border_glyphs: PaneGlyphs,
    pub story_border_glyphs: PaneGlyphs,
    pub status_header_glyphs: PaneGlyphs,
    pub input_line_glyphs: PaneGlyphs,
    pub suggestion_line_glyphs: PaneGlyphs,
    pub notification_glyphs: PaneGlyphs,
    pub upper_window_border_glyphs: PaneGlyphs,
    pub dialog_glyphs: PaneGlyphs,
    /// Whether the story title / map layer-tab header strip is shown.
    pub story_header_on: bool,
    pub map_header_on: bool,
    /// Border pulse color for the high-pitched bleep (sound_effect #1).
    pub sound_beep_high: Style,
    /// Border pulse color for the low-pitched bleep (sound_effect #2).
    pub sound_beep_low: Style,
    /// Room-detection-method indicator (map corner).
    pub loc_indicator: Style,
    /// Player input echo text.
    pub transcript_input: Style,
    /// Meta (app/slash) text.
    pub transcript_meta: Style,
    /// VM warning text.
    pub transcript_warning: Style,
    /// VM crash / fault trace lines in the transcript.
    pub transcript_crash: Style,
    /// Built-in story rule: room-name / location header line.
    pub transcript_location: Style,
    /// Built-in story rule: bracketed system line.
    pub transcript_system: Style,
    /// Gutter marker style for warning lines.
    pub warning_marker: Style,
    /// Compiled user story-styling rules, in evaluation order.
    pub transcript_rules: Vec<CompiledRule>,
    /// The status-bar segment layout (default reproduces today's bar).
    pub statusbar_layout: StatusBarLayout,
    /// The 16-colour ANSI palette carried from the terminal theme (indices 0-15).
    /// Used by `resolve_zcolour` to map `ZColour::Standard(2..=9)` through the
    /// user's colour scheme rather than the Z-machine's raw ANSI numbers.
    pub palette: [Color; 16],
    /// Per-Glk-style theme colours, resolved between the game's stylehint and the
    /// per-app-element base (SQ-0331). Row 0 = text-buffer windows (base
    /// `transcript`); row 1 = text-grid windows (base `upper_window`). Indexed by
    /// the Glk style class (0=Normal .. 10=User2). A slot's `None` channel falls
    /// through to the element, so a `Default` (all-`None`) array leaves the
    /// Z-machine (all-Normal) render byte-identical. Seeded here (buffer Input←
    /// `input_text`, buffer Subheader←`transcript_location`); the SQ-0319
    /// garglk.ini importer will populate the rest. Not yet editable via style.toml
    /// (deferred to the SQ-0309 style redesign).
    pub glk_styles: [[GlkStyleColour; 11]; 2],
}

/// Build the seed `glk_styles` array (SQ-0331): buffer Input(8) ← `input_text`,
/// buffer Subheader(4) ← `transcript_location`; every other slot inherits its
/// element (left `None`), so Normal is definitionally the element and the
/// Z-machine render stays byte-identical.
fn seed_glk_styles(input_text: Style, transcript_location: Style) -> [[GlkStyleColour; 11]; 2] {
    let mut styles = [[GlkStyleColour::default(); 11]; 2];
    styles[0][8] = GlkStyleColour { fg: input_text.fg, bg: input_text.bg };
    styles[0][4] = GlkStyleColour { fg: transcript_location.fg, bg: transcript_location.bg };
    styles
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
            shared_path: Style::new().fg(Color::LightCyan),
            status_bar: Style::new().add_modifier(Modifier::REVERSED),
            notification: Style::new().fg(Color::Black).bg(Color::Cyan),
            transcript: Style::new().fg(Color::White),
            suggestion: Style::new().fg(Color::DarkGray),
            input_text: Style::new(),
            input_prompt: Style::new(),
            scrollbar: Style::new().fg(Color::DarkGray),
            tidy_progress: Style::new().fg(Color::Cyan),
            meta_marker: Style::new().fg(Color::DarkGray),
            hyperlink: Style::new().fg(Color::Cyan),
            focused_border: Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            help_bar: Style::new().add_modifier(Modifier::REVERSED),
            map_border: Style::new().fg(Color::Cyan),
            story_border: Style::new().fg(Color::Cyan),
            story_title: Style::new().fg(Color::White),
            inventory_dock: Style::new().fg(Color::Cyan),
            story_info: Style::new().fg(Color::Cyan),
            story_info_title: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            story_info_label: Style::new().fg(Color::DarkGray),
            story_info_value: Style::new().fg(Color::White),
            story_info_blurb: Style::new().fg(Color::Gray).add_modifier(Modifier::ITALIC),
            story_info_link: Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            story_info_cover: Style::new().bg(Color::Black),
            graphics: Style::new().bg(Color::Black),
            inline_image: Style::new().bg(Color::Black),
            story_badge: Style::new().fg(Color::Blue),
            story_header: Style::new().fg(Color::DarkGray),
            story_header_active: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            story_author: Style::new().fg(Color::White),
            story_year: Style::new().fg(Color::White),
            story_no_metadata: Style::new().fg(Color::DarkGray),
            story_tile: Style::new().fg(Color::White),
            story_tile_selected: Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            // The shown layer reads brighter than the others: inactive tabs are
            // dimmed, the active one is the bold accent colour. Both themeable.
            map_layer_tab: Style::new().fg(Color::DarkGray),
            map_layer_tab_active: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            status_header: Style::new(),
            input_line: Style::new(),
            map_border_style: BorderStyle::None,
            story_border_style: BorderStyle::None,
            status_header_style: BorderStyle::None,
            input_line_style: BorderStyle::None,
            suggestion_line_style: BorderStyle::None,
            notification_style: BorderStyle::Single,
            dialog: Style::new().fg(Color::White).bg(Color::Black),
            dialog_title: Style::new().fg(Color::Cyan),
            hotkey_key: Style::new().fg(Color::Cyan),
            dialog_button: Style::new().add_modifier(Modifier::REVERSED),
            dialog_button_active: Style::new().fg(Color::Black).bg(Color::Cyan),
            dialog_shadow: Style::new().bg(Color::DarkGray),
            dialog_box_style: BorderStyle::None,
            dialog_shadow_on: false,
            dialog_placement: DialogPlacement::Center,
            dialog_margin: 0,
            room_panel: Style::new().fg(Color::White).bg(Color::Blue),
            upper_window: Style::new(),
            upper_window_border: Style::new().fg(Color::Cyan),
            virtual_window_border: BorderStyle::Single,
            map_border_sides: PaneSides::all(BorderStyle::None),
            story_border_sides: PaneSides::all(BorderStyle::None),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            suggestion_line_sides: PaneSides::all(BorderStyle::None),
            notification_sides: PaneSides::all(BorderStyle::Single),
            upper_window_border_sides: PaneSides::all(BorderStyle::Single),
            map_border_glyphs: PaneGlyphs::default(),
            story_border_glyphs: PaneGlyphs::default(),
            status_header_glyphs: PaneGlyphs::default(),
            input_line_glyphs: PaneGlyphs::default(),
            suggestion_line_glyphs: PaneGlyphs::default(),
            notification_glyphs: PaneGlyphs::default(),
            upper_window_border_glyphs: PaneGlyphs::default(),
            dialog_glyphs: PaneGlyphs::default(),
            story_header_on: true,
            map_header_on: true,
            sound_beep_high: Style::new().fg(Color::Rgb(255, 180, 40)),
            sound_beep_low: Style::new().fg(Color::Rgb(60, 140, 220)),
            loc_indicator: Style::new().fg(Color::DarkGray),
            transcript_input: Style::new().fg(Color::Cyan),
            transcript_meta: Style::new().fg(Color::DarkGray),
            transcript_warning: Style::new().fg(Color::Yellow),
            transcript_crash: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            transcript_location: Style::new().add_modifier(Modifier::BOLD),
            transcript_system: Style::new().fg(Color::DarkGray),
            warning_marker: Style::new().fg(Color::Yellow),
            transcript_rules: Vec::new(),
            statusbar_layout: StatusBarLayout::default(),
            palette: [
                Color::Black,      // 0  Z Standard(2) black
                Color::Red,        // 1  Standard(3) red
                Color::Green,      // 2  Standard(4) green
                Color::Yellow,     // 3  Standard(5) yellow
                Color::Blue,       // 4  Standard(6) blue
                Color::Magenta,    // 5  Standard(7) magenta
                Color::Cyan,       // 6  Standard(8) cyan
                Color::Gray,       // 7  Standard(9) white (ANSI 7)
                Color::DarkGray,   // 8  bright black
                Color::LightRed,   // 9
                Color::LightGreen, // 10
                Color::LightYellow,// 11
                Color::LightBlue,  // 12
                Color::LightMagenta,//13
                Color::LightCyan,  // 14
                Color::White,      // 15 bright white
            ],
            glk_styles: seed_glk_styles(Style::new(), Style::new().add_modifier(Modifier::BOLD)),
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
        let shared_path_fg = resolve_element("shared_path", scheme.palette[14]); // bright cyan slot
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
            shared_path: Style::new().fg(shared_path_fg),
            status_bar,
            notification: Style::new().fg(bg).bg(scheme.palette[6]),
            transcript: Style::new().fg(transcript_fg),
            suggestion: Style::new().fg(suggestion_fg),
            input_text: Style::new(),
            input_prompt: Style::new(),
            scrollbar: Style::new().fg(suggestion_fg),
            tidy_progress: Style::new().fg(scheme.palette[6]),
            meta_marker: Style::new().fg(suggestion_fg),
            hyperlink: Style::new().fg(scheme.palette[6]),
            focused_border: Style::new()
                .fg(focused_border_fg)
                .add_modifier(Modifier::BOLD),
            help_bar,
            map_border: Style::new().fg(scheme.palette[6]),
            story_border: Style::new().fg(scheme.palette[6]),
            story_title: Style::new().fg(fg),
            inventory_dock: Style::new().fg(scheme.palette[6]),
            story_info: Style::new().fg(scheme.palette[6]),
            story_info_title: Style::new().fg(fg).add_modifier(Modifier::BOLD),
            story_info_label: Style::new().fg(fg).add_modifier(Modifier::DIM),
            story_info_value: Style::new().fg(fg),
            story_info_blurb: Style::new().fg(fg).add_modifier(Modifier::ITALIC),
            story_info_link: Style::new().fg(fg).add_modifier(Modifier::UNDERLINED),
            story_info_cover: Style::new().bg(bg),
            graphics: Style::new().bg(bg),
            inline_image: Style::new().bg(bg),
            story_badge: Style::new().fg(scheme.palette[4]),
            story_header: Style::new().fg(fg).add_modifier(Modifier::DIM),
            story_header_active: Style::new().fg(scheme.palette[6]).add_modifier(Modifier::BOLD),
            story_author: Style::new().fg(fg),
            story_year: Style::new().fg(fg),
            story_no_metadata: Style::new().fg(fg).add_modifier(Modifier::DIM),
            story_tile: Style::new().fg(fg),
            story_tile_selected: Style::new()
                .fg(bg)
                .bg(scheme.palette[6])
                .add_modifier(Modifier::BOLD),
            map_layer_tab: Style::new().fg(fg).add_modifier(Modifier::DIM),
            map_layer_tab_active: Style::new().fg(scheme.palette[6]).add_modifier(Modifier::BOLD),
            status_header: Style::new(),
            input_line: Style::new(),
            map_border_style: BorderStyle::None,
            story_border_style: BorderStyle::None,
            status_header_style: BorderStyle::None,
            input_line_style: BorderStyle::None,
            suggestion_line_style: BorderStyle::None,
            notification_style: BorderStyle::Single,
            dialog: Style::new().fg(fg).bg(bg),
            dialog_title: Style::new().fg(scheme.palette[6]),
            hotkey_key: Style::new().fg(scheme.palette[6]),
            dialog_button: Style::new().fg(fg).add_modifier(Modifier::REVERSED),
            dialog_button_active: Style::new().fg(bg).bg(scheme.palette[6]),
            dialog_shadow: Style::new().bg(scheme.palette[8]),
            dialog_box_style: BorderStyle::None,
            dialog_shadow_on: false,
            dialog_placement: DialogPlacement::Center,
            dialog_margin: 0,
            room_panel: Style::new().fg(fg).bg(scheme.palette[4]),
            upper_window: Style::new().fg(fg),
            upper_window_border: Style::new().fg(scheme.palette[6]),
            virtual_window_border: BorderStyle::Single,
            map_border_sides: PaneSides::all(BorderStyle::None),
            story_border_sides: PaneSides::all(BorderStyle::None),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            suggestion_line_sides: PaneSides::all(BorderStyle::None),
            notification_sides: PaneSides::all(BorderStyle::Single),
            upper_window_border_sides: PaneSides::all(BorderStyle::Single),
            map_border_glyphs: PaneGlyphs::default(),
            story_border_glyphs: PaneGlyphs::default(),
            status_header_glyphs: PaneGlyphs::default(),
            input_line_glyphs: PaneGlyphs::default(),
            suggestion_line_glyphs: PaneGlyphs::default(),
            notification_glyphs: PaneGlyphs::default(),
            upper_window_border_glyphs: PaneGlyphs::default(),
            dialog_glyphs: PaneGlyphs::default(),
            story_header_on: true,
            map_header_on: true,
            sound_beep_high: Style::new().fg(Color::Rgb(255, 180, 40)),
            sound_beep_low: Style::new().fg(Color::Rgb(60, 140, 220)),
            loc_indicator: Style::new().fg(scheme.palette[8]),
            transcript_input: Style::new().fg(scheme.palette[6]),
            transcript_meta: Style::new().fg(scheme.palette[8]),
            transcript_warning: Style::new().fg(scheme.palette[3]),
            transcript_crash: Style::new().fg(scheme.palette[1]).add_modifier(Modifier::BOLD),
            transcript_location: Style::new().add_modifier(Modifier::BOLD),
            transcript_system: Style::new().fg(scheme.palette[8]),
            warning_marker: Style::new().fg(scheme.palette[3]),
            transcript_rules: Vec::new(),
            statusbar_layout: StatusBarLayout::default(),
            palette: scheme.palette,
            glk_styles: seed_glk_styles(Style::new(), Style::new().add_modifier(Modifier::BOLD)),
        }
    }

    /// Resolve the style for one Story line: first matching user rule wins, else
    /// the built-in location rule (line matches `room_name`), else the built-in
    /// system rule (whole line bracketed), else the base `transcript` style.
    /// A match patches its style over `transcript` (overriding only set fields).
    pub fn resolve_story_style(&self, line: &str, room_name: Option<&str>) -> Style {
        for rule in &self.transcript_rules {
            if rule.regex.is_match(line) {
                return self.transcript.patch(rule.style);
            }
        }
        if let Some(name) = room_name {
            if zvm::location::status_name_matches(line, name) {
                return self.transcript.patch(self.transcript_location);
            }
        }
        let t = line.trim();
        if t.len() >= 2 && t.starts_with('[') && t.ends_with(']') {
            return self.transcript.patch(self.transcript_system);
        }
        self.transcript
    }

}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::terminal_default()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolve a scheme name/path to a `(ColorScheme, GhosttyScheme, warnings)` triple.
///
/// - `scheme == None` → returns `(terminal_default(), GhosttyScheme::default(), [])`
/// - A known built-in name or a file path → parses the Ghostty theme and returns
///   `(ColorScheme::from_ghostty(&gs, &empty), gs, [])`.
/// - Parse/read failure → returns `(terminal_default(), GhosttyScheme::default(), [warning])`.
///
/// The caller is responsible for applying element overrides on top of the returned
/// `ColorScheme` if needed.
pub(crate) fn resolve_base(
    scheme: Option<&str>,
    dir: &Path,
) -> (ColorScheme, GhosttyScheme, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();

    let name = match scheme {
        None => return (ColorScheme::terminal_default(), GhosttyScheme::default(), warnings),
        Some(n) => n,
    };

    let gs = match builtin_scheme_text(name) {
        Some(text) => match GhosttyScheme::parse(text) {
            Ok(gs) => gs,
            Err(e) => {
                warnings.push(format!(
                    "built-in scheme '{}' failed to parse: {}; using terminal defaults",
                    name, e
                ));
                return (ColorScheme::terminal_default(), GhosttyScheme::default(), warnings);
            }
        },
        None => {
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
                        return (
                            ColorScheme::terminal_default(),
                            GhosttyScheme::default(),
                            warnings,
                        );
                    }
                },
                Err(e) => {
                    warnings.push(format!(
                        "could not read scheme file '{}': {}; using terminal defaults",
                        path.display(),
                        e
                    ));
                    return (
                        ColorScheme::terminal_default(),
                        GhosttyScheme::default(),
                        warnings,
                    );
                }
            }
        }
    };

    let empty_overrides = std::collections::BTreeMap::new();
    let cs = ColorScheme::from_ghostty(&gs, &empty_overrides);
    (cs, gs, warnings)
}

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
pub fn expand_path(s: &str, base_dir: &Path) -> std::path::PathBuf {
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
        "default" => return Some(Color::Reset),
        // Explicit "unset" sentinel written by `style::write_style_full` for a colour
        // that is None. Resolves to no colour (patches nothing) so an unset field
        // stays unset — distinct from "default"/"reset" (explicit terminal default).
        "none" => return None,
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_glk_styles_copies_input_and_subheader_leaves_rest_none() {
        let styles = seed_glk_styles(
            Style::new().fg(Color::Cyan).bg(Color::Black),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        );
        // Buffer (row 0): Input(8) ← input_text fg/bg; Subheader(4) ← transcript_location.
        assert_eq!(styles[0][8], GlkStyleColour { fg: Some(Color::Cyan), bg: Some(Color::Black) });
        assert_eq!(styles[0][4], GlkStyleColour { fg: Some(Color::Green), bg: None });
        // Normal(0) is always None → definitionally the element (byte-identical Z-machine).
        assert_eq!(styles[0][0], GlkStyleColour::default());
        // Every other buffer slot and the entire grid row inherit (None).
        for i in [1usize, 2, 3, 5, 6, 7, 9, 10] {
            assert_eq!(styles[0][i], GlkStyleColour::default(), "buffer slot {i} inherits");
        }
        for (i, slot) in styles[1].iter().enumerate() {
            assert_eq!(*slot, GlkStyleColour::default(), "grid slot {i} inherits (row 1 unseeded)");
        }
    }

    #[test]
    fn terminal_default_glk_styles_are_all_none() {
        // input_text / transcript_location carry no concrete colour in the terminal
        // default, so the seeds resolve to None and Normal stays the element.
        let cs = ColorScheme::terminal_default();
        for row in 0..2 {
            for i in 0..11 {
                assert_eq!(cs.glk_styles[row][i], GlkStyleColour::default(), "row {row} slot {i}");
            }
        }
    }

    #[test]
    fn border_sides_default_to_all_of_base_and_headers_on() {
        use crate::render::paneframe::{PaneSides, BorderStyle};
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.map_border_sides, PaneSides::all(cs.map_border_style));
        assert_eq!(cs.story_border_sides, PaneSides::all(cs.story_border_style));
        assert_eq!(cs.status_header_sides, PaneSides::all(cs.status_header_style));
        assert_eq!(cs.input_line_sides, PaneSides::all(cs.input_line_style));
        assert_eq!(cs.suggestion_line_sides, PaneSides::all(cs.suggestion_line_style));
        assert_eq!(cs.notification_sides, PaneSides::all(cs.notification_style));
        assert_eq!(cs.notification_style, BorderStyle::Single, "toasts default to a single border");
        assert_eq!(cs.upper_window_border_sides, PaneSides::all(cs.virtual_window_border));
        assert!(cs.story_header_on);
        assert!(cs.map_header_on);
        // None base → all sides None.
        assert_eq!(cs.map_border_sides, PaneSides::all(BorderStyle::None));
    }

    #[test]
    fn statusbar_layout_default_reproduces_today() {
        let l = StatusBarLayout::default();
        // location (left), Score/Moves (right), time (right), filter (right).
        assert_eq!(l.segments.len(), 4);
        assert_eq!(l.segments[0].text, "{location}");
        assert!(matches!(l.segments[0].align, Align::Left));
        assert_eq!(l.segments[1].text, "Score: {score}  Moves: {moves}");
        assert!(matches!(l.segments[1].align, Align::Right));
        assert_eq!(l.segments[2].text, "{time}");
        assert_eq!(l.segments[3].text, " {filter}");
        // All built-in segments carry no per-segment override (render in base style).
        assert!(l.segments.iter().all(|s| s.style == Style::default()));
        // ColorScheme carries the default layout.
        assert_eq!(ColorScheme::terminal_default().statusbar_layout, StatusBarLayout::default());
    }

    #[test]
    fn terminal_default_transcript_category_styles() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.transcript_input.fg, Some(Color::Cyan));
        assert_eq!(cs.transcript_meta.fg, Some(Color::DarkGray));
        assert_eq!(cs.transcript_warning.fg, Some(Color::Yellow));
        assert!(cs.transcript_location.add_modifier.contains(Modifier::BOLD));
        assert_eq!(cs.transcript_location.fg, None); // bold-only, inherits base fg
        assert_eq!(cs.transcript_system.fg, Some(Color::DarkGray));
        assert_eq!(cs.warning_marker.fg, Some(Color::Yellow));
        assert!(cs.transcript_rules.is_empty());
    }

    #[test]
    fn resolve_story_style_precedence_and_patch() {
        use ratatui::style::{Color, Modifier};
        let mut cs = ColorScheme::terminal_default(); // transcript fg = White
        // A user rule that only sets bold (no fg) → patch keeps base fg.
        cs.transcript_rules.push(CompiledRule {
            pattern: "^>".into(),
            regex: regex::Regex::new("^>").unwrap(),
            style: Style::new().add_modifier(Modifier::BOLD),
        });

        // 1. User rule wins, patch semantics: bold added, base White fg kept.
        let s = cs.resolve_story_style("> go north", Some("West of House"));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert_eq!(s.fg, Some(Color::White));

        // 2. Built-in location: line equals room name → bold (transcript_location).
        let loc = cs.resolve_story_style("West of House", Some("West of House"));
        assert!(loc.add_modifier.contains(Modifier::BOLD));

        // 2b. Boundary guard: "Hall" line vs room "Hallway" must NOT match location.
        let no_loc = cs.resolve_story_style("Hall", Some("Hallway"));
        assert!(!no_loc.add_modifier.contains(Modifier::BOLD));
        assert_eq!(no_loc, cs.transcript); // falls through to base

        // 3. Built-in system: bracketed line → transcript_system (DarkGray).
        let sys = cs.resolve_story_style("[Your score just went up by ten points.]", None);
        assert_eq!(sys.fg, Some(Color::DarkGray));

        // 4. No match → base transcript.
        assert_eq!(cs.resolve_story_style("plain prose", None), cs.transcript);

        // 5. None room name → location never matches.
        assert_eq!(cs.resolve_story_style("West of House", None), cs.transcript);
    }

    #[test]
    fn compiled_rule_eq_ignores_regex_object() {
        let a = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
        let b = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
        assert_eq!(a, b);
    }

    #[test]
    fn parse_color_value_accepts_named_colors() {
        let scheme = GhosttyScheme::default(); // or a minimal scheme
        assert_eq!(parse_color_value("red", &scheme), Some(Color::Red));
        assert_eq!(parse_color_value("bright-blue", &scheme), Some(Color::LightBlue));
        assert_eq!(parse_color_value("white", &scheme), Some(Color::White));
    }

    #[test]
    fn parse_color_value_maps_default_and_reset_to_reset() {
        let gs = GhosttyScheme::default();
        assert_eq!(parse_color_value("default", &gs), Some(Color::Reset));
        assert_eq!(parse_color_value("reset", &gs), Some(Color::Reset));
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

    #[test]
    fn sound_beep_defaults_are_amber_and_cyan_blue() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.sound_beep_high.fg, Some(Color::Rgb(255, 180, 40)));
        assert_eq!(cs.sound_beep_low.fg, Some(Color::Rgb(60, 140, 220)));
    }

    #[test]
    fn loc_indicator_default_is_dim() {
        let cs = ColorScheme::terminal_default();
        assert_eq!(cs.loc_indicator.fg, Some(Color::DarkGray));
    }

    #[test]
    fn terminal_default_palette_maps_standard_colours_concretely() {
        use zvm::screen::ZColour;
        use ratatui::style::Color;
        let s = ColorScheme::terminal_default();
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(2), &s), Color::Black, "black");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(3), &s), Color::Red, "red");
        assert_eq!(crate::render::resolve_zcolour(ZColour::Standard(9), &s), Color::Gray, "white(9)->ANSI white");
    }

}
