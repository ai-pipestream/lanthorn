use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Deserializer};

use crate::anim::Easing;

// ── Keymap config ─────────────────────────────────────────────────────────────

/// The `[keymap]` section of config.toml.
///
/// `use_defaults = true` (the default) layers user bindings on top of the
/// built-in defaults. Set `use_defaults = false` for a clean-slate keymap.
///
/// Per-context override tables map key-spec strings to command strings:
///
///   `[keymap]`
///   use_defaults = true
///   [keymap.global]
///   "ctrl+s" = "save-game"
///   [keymap.map]
///   "left" = "pan-map -1 0"
///   [keymap.anim]
///   "l" = "anim-step forward"
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub use_defaults: bool,
    pub global: std::collections::BTreeMap<String, String>,
    pub map: std::collections::BTreeMap<String, String>,
    pub anim: std::collections::BTreeMap<String, String>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            use_defaults: true,
            global: Default::default(),
            map: Default::default(),
            anim: Default::default(),
        }
    }
}

// ── Symbol config ─────────────────────────────────────────────────────────────

pub(crate) fn default_box_style() -> String { "rounded".into() }
pub(crate) fn default_arrow_set() -> String { "filled".into() }
pub(crate) fn default_portal_icons() -> String { "ascii".into() }
pub(crate) fn default_path_style() -> String { "light".into() }
pub(crate) fn default_badge_zcode() -> String { "Z".into() }
pub(crate) fn default_badge_glulx() -> String { "G".into() }
pub(crate) fn default_badge_blorb() -> String { "B".into() }
pub(crate) fn default_badge_save() -> String { "S".into() }
pub(crate) fn default_badge_hint() -> String { "H".into() }

/// The `[symbols]` section of config.toml.  All fields default to the preset
/// names that match today's hardcoded glyphs, so an absent section is a no-op.
#[derive(Debug, Deserialize, Clone)]
pub struct SymbolConfig {
    /// Room outline style preset name.
    #[serde(default = "default_box_style")]
    pub box_style: String,
    /// Arrow glyph set preset name.
    #[serde(default = "default_arrow_set")]
    pub arrow_set: String,
    /// Portal icon preset name.
    #[serde(default = "default_portal_icons")]
    pub portal_icons: String,
    /// Path line-art preset name.
    #[serde(default = "default_path_style")]
    pub path_style: String,
    /// Row story-type badge glyph for Z-code stories (default "Z").
    #[serde(default = "default_badge_zcode")]
    pub badge_zcode: String,
    /// Row story-type badge glyph for Glulx stories (default "G").
    #[serde(default = "default_badge_glulx")]
    pub badge_glulx: String,
    /// Row "a blorb exists" artifact badge glyph (default "B").
    #[serde(default = "default_badge_blorb")]
    pub badge_blorb: String,
    /// Row "a save exists" artifact badge glyph (default "S").
    #[serde(default = "default_badge_save")]
    pub badge_save: String,
    /// Row "a hint file exists" artifact badge glyph (default "H").
    #[serde(default = "default_badge_hint")]
    pub badge_hint: String,
    /// Per-slot overrides (slot key → single-char value).
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

impl Default for SymbolConfig {
    fn default() -> Self {
        Self {
            box_style: default_box_style(),
            arrow_set: default_arrow_set(),
            portal_icons: default_portal_icons(),
            path_style: default_path_style(),
            badge_zcode: default_badge_zcode(),
            badge_glulx: default_badge_glulx(),
            badge_blorb: default_badge_blorb(),
            badge_save: default_badge_save(),
            badge_hint: default_badge_hint(),
            overrides: BTreeMap::new(),
        }
    }
}

// ── Search config ─────────────────────────────────────────────────────────────

fn default_start_backward() -> bool { true }
fn default_key_back() -> char { 'n' }
fn default_key_forward() -> char { 'N' }

/// Deserialize a single-char string field, defaulting to 'n' on empty.
/// Used for key_back and key_forward (first char of the string).
fn deserialize_char_key_back<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('n'))
}

fn deserialize_char_key_forward<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('N'))
}

/// The `[search]` section of config.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct SearchConfig {
    /// When true (default), a new /search starts backward from the bottom (most recent match).
    #[serde(default = "default_start_backward")]
    pub start_backward: bool,
    /// Key to navigate backward (toward older lines). Default 'n'.
    #[serde(default = "default_key_back", deserialize_with = "deserialize_char_key_back")]
    pub key_back: char,
    /// Key to navigate forward (toward newer lines). Default 'N'.
    #[serde(default = "default_key_forward", deserialize_with = "deserialize_char_key_forward")]
    pub key_forward: char,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            start_backward: default_start_backward(),
            key_back: default_key_back(),
            key_forward: default_key_forward(),
        }
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

/// babelmap: a Z-machine interpreter with live automapping.
#[derive(Parser, Debug)]
#[command(name = "babelmap", about = "Z-machine interpreter with live automapping")]
pub struct Cli {
    /// Path to the story file (.z3/.z5/.z8 etc.)
    pub story: PathBuf,

    /// Override the babelmap home directory (default: ~/.babelmap)
    #[arg(long, value_name = "PATH")]
    pub user_dir: Option<PathBuf>,

    /// Path to a non-default config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Disable Glulx accelerated-function interception (debug; default: enabled)
    #[arg(long)]
    pub no_accel: bool,
}

// ── Hotkeys config ────────────────────────────────────────────────────────────

/// One group of commands shown together in the hotkey dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeyGroupConfig {
    pub title: String,
    pub commands: Vec<String>,
}

/// The `[hotkeys]` section of config.toml.
/// `prefix` overrides the dialog-open key (default: Ctrl+K).
/// `direct` overrides which commands are always available (bypass dialog).
/// `group` overrides the command groups shown in the dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeysConfig {
    /// Override the dialog-prefix key spec string (e.g. "ctrl+k").
    pub prefix: Option<String>,
    /// Override the set of always-available commands (by snake_case name).
    pub direct: Option<Vec<String>>,
    /// Override the command groups in the dialog.
    #[serde(default)]
    pub group: Vec<HotkeyGroupConfig>,
}

// ── Config ────────────────────────────────────────────────────────────────────

fn default_command_prefix() -> char { '/' }
fn default_undo_levels() -> usize { 16 }

fn default_virtual_screen_cols() -> u16 { 80 }
fn default_virtual_screen_rows() -> u16 { 24 }
fn default_honor_game_colours() -> bool { true }
fn default_acceleration() -> bool { true }
fn default_honor_timed_input() -> bool { true }
fn default_enable_sound() -> bool { true }
fn default_volume() -> u8 { 100 }

/// Deserialize a single-char string field into a `char`.  Takes the first
/// Unicode scalar value of the string; falls back to `/` on an empty string.
fn deserialize_char_from_str<'de, D>(d: D) -> Result<char, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(s.chars().next().unwrap_or('/'))
}

fn default_user_dir() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".babelmap")
}

fn default_true() -> bool { true }

// ── Background-tidy mode ──────────────────────────────────────────────────────

/// Controls when the map is automatically re-tidied after new rooms are discovered.
///
/// TOML: `background_tidy = "every_room"` (default), `"off"`, `"on_overlap"`, `"debounced"`.
///
/// NOTE: the default (`EveryRoom`) changes today's behavior — a full relayout runs on
/// each turn that discovers a new room. Set `background_tidy = "off"` to keep the
/// manual-only tidy behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTidy {
    /// Never auto-tidy; only manual Retidy / AnimateTidy.
    Off,
    /// Re-tidy whenever a turn discovers a new room (default).
    #[default]
    EveryRoom,
    /// Re-tidy only when incremental placement caused an overlap or distorted edge.
    OnOverlap,
    /// Re-tidy once every K new rooms (`BG_TIDY_DEBOUNCE`).
    Debounced,
}

/// Number of new rooms that must accumulate before a `Debounced` background tidy fires.
pub const BG_TIDY_DEBOUNCE: u32 = 5;

/// Where to persist v5 auxiliary save data (the `save/restore table` opcodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuxStorage {
    /// Ask the user on first use, then store the choice in config.
    #[default]
    Ask,
    /// Inside each `.babelmap` save archive.
    Archive,
    /// In one per-game file in the save directory (shared across playthroughs).
    Global,
}

// ── Animation config ──────────────────────────────────────────────────────────

fn default_scroll_ms() -> u64 { 120 }
fn default_easing() -> Easing { Easing::EaseOut }

/// Deserialize an easing token string (e.g. "ease-out") into an [`Easing`].
/// Unknown tokens fall back to `EaseOut` (via `parse_easing`).
fn deserialize_easing<'de, D>(d: D) -> Result<Easing, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    Ok(crate::anim::parse_easing(&s))
}

/// The `[animation]` section of config.toml. Controls the shared TUI animation
/// engine. With `enabled = false` (or `scroll_ms = 0`) every animation is
/// instant, exactly reproducing the pre-animation behavior.
#[derive(Debug, Deserialize, Clone)]
pub struct AnimationConfig {
    /// Master switch (default true). When false, every animation is instant.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Easing curve token (default "ease-out").
    #[serde(default = "default_easing", deserialize_with = "deserialize_easing")]
    pub easing: Easing,
    /// Smooth-scroll duration in milliseconds (default 120). Zero = instant.
    #[serde(default = "default_scroll_ms")]
    pub scroll_ms: u64,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            easing: Easing::EaseOut,
            scroll_ms: 120,
        }
    }
}

/// User preferences loaded from TOML.  Every field has a default so a missing
/// config file (or a file with only some fields) is always valid.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Root directory for babelmap data (maps, saves, exports).
    /// Sub-directories: maps/ — where per-story map files live.
    #[serde(default = "default_user_dir")]
    pub user_dir: PathBuf,
    /// When false (default), a story with no .babelmap archive starts with an
    /// empty map (fog-of-war, self-contained).  When true, fall back to the
    /// legacy shared `<ifid>.map.json` so pre-accumulated maps are still visible.
    #[serde(default)]
    pub use_default_map: bool,
    /// When true (default), restore the game state from the archive on startup so
    /// play resumes where it left off. Set false to start a fresh playthrough while
    /// retaining the accumulated map.
    #[serde(default = "default_true")]
    pub auto_load: bool,
    /// When true, save the archive after every game turn (in addition to the
    /// exit-save and Ctrl+S quick-save). Default false.
    #[serde(default)]
    pub auto_save: bool,
    /// When true, invert mouse-wheel scroll direction (for terminals reporting
    /// "natural" scrolling). Default false = conventional direction.
    #[serde(default)]
    pub mouse_wheel_invert: bool,
    /// When true (default) and auto_save is off, prompt the user to save on quit.
    #[serde(default = "default_true")]
    pub prompt_save_on_quit: bool,
    /// When true (default) and auto_load is off, prompt the user to resume a found save on launch.
    #[serde(default = "default_true")]
    pub prompt_load_on_launch: bool,
    /// When true (default), record command history across sessions. Set false to disable.
    #[serde(default = "default_true")]
    pub record_history: bool,
    /// When true, record a per-turn rewind/replay history (Quetzal save + map
    /// snapshots) into the `.babelmap` archive. Default false (opt-in: it grows
    /// the archive and keeps per-turn blobs in memory).
    #[serde(default)]
    pub record_turn_history: bool,
    /// Controls automatic background re-tidy when new rooms are discovered.
    /// Default: EveryRoom (re-tidy on each turn that finds a new room).
    #[serde(default)]
    pub background_tidy: BackgroundTidy,
    /// Where to persist v5 auxiliary save data. Default: Ask.
    #[serde(default)]
    pub aux_storage: AuxStorage,
    /// Keymap overrides: command_name → key-spec string(s).
    #[serde(default)]
    pub keymap: KeymapConfig,
    /// Hotkey dialog configuration: prefix key, direct commands, dialog groups.
    #[serde(default)]
    pub hotkeys: HotkeysConfig,
    /// Style-file pointer: a built-in name, a file path, or absent (use
    /// `user_dir/style.toml` if present, else the built-in default).
    #[serde(default)]
    pub style: Option<String>,
    /// Watch the resolved style.toml and live-reload it on change (default false).
    #[serde(default)]
    pub watch_style: bool,
    /// Undo depth: max retained in-memory undo snapshots (default 16; 0 disables).
    #[serde(default = "default_undo_levels")]
    pub undo_levels: usize,
    /// The prefix character that triggers slash-command routing (default: '/').
    /// Stored as a single-character string in TOML: command_prefix = "/".
    #[serde(default = "default_command_prefix", deserialize_with = "deserialize_char_from_str")]
    pub command_prefix: char,
    /// When true, room numbers (#id) are shown in Boxes-zoom room boxes.
    /// Default false (hidden); toggled at runtime by ToggleRoomNumbers.
    #[serde(default)]
    pub show_room_numbers: bool,
    /// Show the room-detection-method indicator in the map corner. Default false.
    #[serde(default)]
    pub show_loc_method: bool,
    /// Show the status/score bar (top row of the story pane). Default true.
    /// The v3 status line (location/score/moves) is only meaningful for v3
    /// games; for v4+ (which draw their own upper-window status) it reads
    /// garbage globals, so this can be toggled off (ToggleStatusBar).
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
    /// Search configuration: start direction, nav keys.
    #[serde(default)]
    pub search: SearchConfig,
    /// Virtual screen width reported to the Z-machine (chars). Default 80.
    #[serde(default = "default_virtual_screen_cols")]
    pub virtual_screen_cols: u16,
    /// Virtual screen height reported to the Z-machine (lines). Default 24.
    #[serde(default = "default_virtual_screen_rows")]
    pub virtual_screen_rows: u16,
    /// Animation engine settings: enable switch, easing curve, scroll duration.
    #[serde(default)]
    pub animation: AnimationConfig,
    /// When true (default), honor game-set colours in the transcript and upper
    /// window. Set false to use only the configured color scheme.
    #[serde(default = "default_honor_game_colours")]
    pub honor_game_colours: bool,
    /// When true (default), honor the Z-machine's timed-input (`read`/`read_char`
    /// `time`+`routine` operands). Set false to treat all reads as untimed.
    #[serde(default = "default_honor_timed_input")]
    pub honor_timed_input: bool,
    /// Interpreter number to advertise (header 0x1E). `None` = auto (Frotz's rule:
    /// 1 for v1-5, 6 for v6). Set to override, e.g. 6 for BeyondZork's IBM PC
    /// character-graphics instead of colour.
    #[serde(default)]
    pub interpreter_number: Option<u8>,
    /// When true (default), play audio for `sound_effect` (bleeps + Blorb samples).
    #[serde(default = "default_enable_sound")]
    pub enable_sound: bool,
    /// Master audio volume 0..=100 (default 100). Combined with the game's per-sound
    /// Z-scale volume.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// Whether Glulx accel interception is active. Runtime-only (set from the
    /// --no-accel CLI flag); intentionally not persisted or user-facing.
    #[serde(skip, default = "default_acceleration")]
    pub acceleration: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            user_dir: default_user_dir(),
            use_default_map: false,
            auto_load: true,
            auto_save: false,
            mouse_wheel_invert: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_history: true,
            record_turn_history: false,
            background_tidy: BackgroundTidy::EveryRoom,
            aux_storage: AuxStorage::Ask,
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: None,
            watch_style: false,
            undo_levels: default_undo_levels(),
            command_prefix: default_command_prefix(),
            show_room_numbers: false,
            show_loc_method: false,
            show_status_bar: true,
            search: SearchConfig::default(),
            virtual_screen_cols: default_virtual_screen_cols(),
            virtual_screen_rows: default_virtual_screen_rows(),
            animation: AnimationConfig::default(),
            honor_game_colours: default_honor_game_colours(),
            honor_timed_input: default_honor_timed_input(),
            interpreter_number: None,
            enable_sound: default_enable_sound(),
            volume: default_volume(),
            acceleration: default_acceleration(),
        }
    }
}

/// The config file path `resolve` reads from: the `--config` override, else the
/// default `user_dir/config.toml`.
pub fn config_path(cli: &Cli) -> std::path::PathBuf {
    match &cli.config {
        Some(p) => p.clone(),
        None => default_user_dir().join("config.toml"),
    }
}

/// True if a raw config.toml still contains a top-level `[colors]` or `[symbols]`
/// table. Those style sections moved to style.toml and are no longer read; the
/// caller warns once so users can migrate.
pub fn config_has_style_sections(raw: &str) -> bool {
    match raw.parse::<toml::Value>() {
        Ok(toml::Value::Table(t)) => t.contains_key("colors") || t.contains_key("symbols"),
        _ => false,
    }
}

// ── Load order ────────────────────────────────────────────────────────────────

/// Resolve configuration with precedence: defaults < config file < CLI flags.
///
/// A missing config file is silently ignored (not an error).
/// Returns the merged Config.  The Cli is returned by the caller via
/// `Cli::parse()` before calling this; pass a reference here.
pub fn resolve(cli: &Cli) -> Config {
    // Determine which config file to read.
    let config_path = config_path(cli);

    // Start from defaults.
    let mut cfg = Config::default();

    // Layer in the config file if it exists.
    if let Ok(text) = std::fs::read_to_string(&config_path) {
        if let Ok(from_file) = toml::from_str::<Config>(&text) {
            cfg.user_dir = from_file.user_dir;
            cfg.use_default_map = from_file.use_default_map;
            cfg.auto_load = from_file.auto_load;
            cfg.auto_save = from_file.auto_save;
            cfg.mouse_wheel_invert = from_file.mouse_wheel_invert;
            cfg.prompt_save_on_quit = from_file.prompt_save_on_quit;
            cfg.prompt_load_on_launch = from_file.prompt_load_on_launch;
            cfg.record_history = from_file.record_history;
            cfg.record_turn_history = from_file.record_turn_history;
            cfg.background_tidy = from_file.background_tidy;
            cfg.aux_storage = from_file.aux_storage;
            cfg.keymap = from_file.keymap;
            cfg.hotkeys = from_file.hotkeys;
            cfg.style = from_file.style;
            cfg.watch_style = from_file.watch_style;
            cfg.undo_levels = from_file.undo_levels;
            cfg.command_prefix = from_file.command_prefix;
            cfg.show_room_numbers = from_file.show_room_numbers;
            cfg.show_loc_method = from_file.show_loc_method;
            cfg.show_status_bar = from_file.show_status_bar;
            cfg.honor_game_colours = from_file.honor_game_colours;
            cfg.honor_timed_input = from_file.honor_timed_input;
            cfg.interpreter_number = from_file.interpreter_number;
            cfg.enable_sound = from_file.enable_sound;
            cfg.volume = from_file.volume;
            cfg.search = from_file.search;
            cfg.virtual_screen_cols = from_file.virtual_screen_cols;
            cfg.virtual_screen_rows = from_file.virtual_screen_rows;
            cfg.animation = from_file.animation;
        }
        // If the file exists but is malformed, silently keep defaults.
        // Production code could warn here; for now, YAGNI.
    }

    // CLI overrides beat the file.
    if let Some(dir) = &cli.user_dir {
        cfg.user_dir = dir.clone();
    }

    cfg.acceleration = !cli.no_accel;

    cfg
}

// ── Write helpers ─────────────────────────────────────────────────────────────

/// Write the functional config fields (and the `style` pointer) to `dir/config.toml`
/// using toml_edit (format-preserving). Creates the file and parent directory if absent.
/// Does NOT emit `[colors]`/`[symbols]` — those now live in the style file.
/// Preserves all other content (comments, `[keymap]`, `[hotkeys]`, any visual sections, etc.).
pub fn write_config(dir: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let config_path = dir.join("config.toml");

    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = existing.parse().unwrap_or_default();

    // Top-level scalar fields.
    doc["user_dir"] = toml_edit::value(cfg.user_dir.to_string_lossy().as_ref());
    doc["use_default_map"] = toml_edit::value(cfg.use_default_map);
    doc["auto_load"] = toml_edit::value(cfg.auto_load);
    doc["auto_save"] = toml_edit::value(cfg.auto_save);
    doc["mouse_wheel_invert"] = toml_edit::value(cfg.mouse_wheel_invert);
    doc["prompt_save_on_quit"] = toml_edit::value(cfg.prompt_save_on_quit);
    doc["prompt_load_on_launch"] = toml_edit::value(cfg.prompt_load_on_launch);
    doc["record_history"] = toml_edit::value(cfg.record_history);
    let bg_str = match cfg.background_tidy {
        BackgroundTidy::Off => "off",
        BackgroundTidy::EveryRoom => "every_room",
        BackgroundTidy::OnOverlap => "on_overlap",
        BackgroundTidy::Debounced => "debounced",
    };
    doc["background_tidy"] = toml_edit::value(bg_str);
    let aux_str = match cfg.aux_storage {
        AuxStorage::Ask => "ask",
        AuxStorage::Archive => "archive",
        AuxStorage::Global => "global",
    };
    doc["aux_storage"] = toml_edit::value(aux_str);
    doc["show_room_numbers"] = toml_edit::value(cfg.show_room_numbers);
    doc["show_loc_method"] = toml_edit::value(cfg.show_loc_method);
    doc["show_status_bar"] = toml_edit::value(cfg.show_status_bar);
    doc["honor_game_colours"] = toml_edit::value(cfg.honor_game_colours);
    doc["honor_timed_input"] = toml_edit::value(cfg.honor_timed_input);
    doc["enable_sound"] = toml_edit::value(cfg.enable_sound);
    doc["volume"] = toml_edit::value(cfg.volume as i64);
    if let Some(n) = cfg.interpreter_number {
        doc["interpreter_number"] = toml_edit::value(n as i64);
    }
    doc["virtual_screen_cols"] = toml_edit::value(i64::from(cfg.virtual_screen_cols));
    doc["virtual_screen_rows"] = toml_edit::value(i64::from(cfg.virtual_screen_rows));

    // style pointer — the only visual key written to config.toml. The actual
    // colors/symbols live in the style file ([colors]/[symbols] are no longer
    // emitted here). Visual override sections, if present, are preserved as-is.
    match &cfg.style {
        Some(s) => { doc["style"] = toml_edit::value(s.as_str()); }
        None => { doc.remove("style"); }
    }

    // [search] table.
    {
        let tbl = doc["search"].or_insert(toml_edit::table());
        tbl["start_backward"] = toml_edit::value(cfg.search.start_backward);
        tbl["key_back"] = toml_edit::value(cfg.search.key_back.to_string());
        tbl["key_forward"] = toml_edit::value(cfg.search.key_forward.to_string());
    }

    // [animation] table.
    {
        let tbl = doc["animation"].or_insert(toml_edit::table());
        tbl["enabled"] = toml_edit::value(cfg.animation.enabled);
        tbl["easing"] = toml_edit::value(crate::anim::easing_token(cfg.animation.easing));
        tbl["scroll_ms"] = toml_edit::value(cfg.animation.scroll_ms as i64);
    }

    std::fs::write(&config_path, doc.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_levels_defaults_to_16() {
        assert_eq!(Config::default().undo_levels, 16);
    }

    #[test]
    fn record_turn_history_defaults_false_and_round_trips() {
        assert!(!Config::default().record_turn_history);
        let cfg: Config = toml::from_str("record_turn_history = true\n").unwrap();
        assert!(cfg.record_turn_history);
    }

    #[test]
    fn watch_style_defaults_false_and_detector_works() {
        let c = Config::default();
        assert!(!c.watch_style);
        assert!(config_has_style_sections("[colors]\n\"room\" = { fg = \"red\" }\n"));
        assert!(config_has_style_sections("[symbols]\nbox_style = \"thick\"\n"));
        assert!(!config_has_style_sections("style = \"s.toml\"\nuse_default_map = true\n"));
    }

    #[test]
    fn virtual_screen_defaults_80x24() {
        let cfg = Config::default();
        assert_eq!(cfg.virtual_screen_cols, 80);
        assert_eq!(cfg.virtual_screen_rows, 24);
    }

    #[test]
    fn virtual_screen_parses_from_toml() {
        let cfg: Config = toml::from_str("virtual_screen_cols = 64\nvirtual_screen_rows = 20").unwrap();
        assert_eq!(cfg.virtual_screen_cols, 64);
        assert_eq!(cfg.virtual_screen_rows, 20);
    }

    #[test]
    fn config_show_room_numbers_default_false_and_round_trips() {
        assert!(!Config::default().show_room_numbers);
        let cfg: Config = toml::from_str("show_room_numbers = true\n").unwrap();
        assert!(cfg.show_room_numbers);
    }

    #[test]
    fn config_show_loc_method_default_false_and_round_trips() {
        assert!(!Config::default().show_loc_method);
        let cfg: Config = toml::from_str("show_loc_method = true\n").unwrap();
        assert!(cfg.show_loc_method);
    }

    #[test]
    fn config_show_status_bar_default_true_and_round_trips() {
        assert!(Config::default().show_status_bar);
        let cfg: Config = toml::from_str("show_status_bar = false\n").unwrap();
        assert!(!cfg.show_status_bar);
    }

    #[test]
    fn config_reads_command_prefix() {
        let cfg: Config = toml::from_str("command_prefix = \";\"\n").unwrap();
        assert_eq!(cfg.command_prefix, ';');
        assert_eq!(Config::default().command_prefix, '/');
    }
    use std::io::Write;

    /// Write a temp config file and return its path.  Uses a unique filename
    /// derived from the test function name to avoid collisions in parallel runs.
    fn write_temp_config(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("babelmap_test_{}.toml", name));
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", contents).unwrap();
        path
    }

    #[test]
    fn default_config_has_babelmap_dir() {
        let cfg = Config::default();
        // The default user_dir must end with ".babelmap".
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".babelmap");
    }

    #[test]
    fn parse_toml_populates_user_dir() {
        let toml = r#"user_dir = "/tmp/mydata""#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/mydata"));
    }

    #[test]
    fn unspecified_fields_fall_back_to_defaults() {
        // An empty TOML file should give us the same user_dir as Config::default().
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".babelmap");
    }

    #[test]
    fn cli_override_beats_file() {
        let cfg_path = write_temp_config("cli_override", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: PathBuf::from("foo.z5"),
            user_dir: Some(PathBuf::from("/tmp/from-cli")),
            config: Some(cfg_path),
            no_accel: false,
        };

        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-cli"));
    }

    #[test]
    fn missing_config_file_resolves_to_defaults() {
        let cli = Cli {
            story: PathBuf::from("foo.z5"),
            user_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: false,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".babelmap");
    }

    #[test]
    fn file_value_beats_default_when_no_cli_override() {
        let cfg_path = write_temp_config("file_beats_default", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: PathBuf::from("foo.z5"),
            user_dir: None,
            config: Some(cfg_path),
            no_accel: false,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-file"));
    }

    #[test]
    fn use_default_map_is_false_by_default() {
        let cfg = Config::default();
        assert!(!cfg.use_default_map, "use_default_map must default to false");
    }

    #[test]
    fn use_default_map_parses_from_toml() {
        let toml = "use_default_map = true";
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.use_default_map);
    }

    #[test]
    fn use_default_map_omitted_stays_false() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(!cfg.use_default_map);
    }

    #[test]
    fn keymap_config_parses_context_sections() {
        let toml = r#"
[keymap]
use_defaults = false
[keymap.global]
"ctrl+s" = "save-game"
[keymap.map]
"left" = "pan-map -1 0"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(!cfg.keymap.use_defaults);
        assert_eq!(cfg.keymap.global.get("ctrl+s").map(String::as_str), Some("save-game"));
        assert_eq!(cfg.keymap.map.get("left").map(String::as_str), Some("pan-map -1 0"));
        // Default keeps use_defaults true.
        assert!(Config::default().keymap.use_defaults);
    }

    #[test]
    fn auto_load_defaults_true() {
        let cfg = Config::default();
        assert!(cfg.auto_load, "auto_load must default to true");
    }

    #[test]
    fn auto_save_defaults_false() {
        let cfg = Config::default();
        assert!(!cfg.auto_save, "auto_save must default to false");
    }

    #[test]
    fn background_tidy_defaults_every_room() {
        let cfg = Config::default();
        assert_eq!(cfg.background_tidy, BackgroundTidy::EveryRoom);
    }

    #[test]
    fn auto_load_parses_false_from_toml() {
        let cfg: Config = toml::from_str("auto_load = false").unwrap();
        assert!(!cfg.auto_load);
    }

    #[test]
    fn auto_save_parses_true_from_toml() {
        let cfg: Config = toml::from_str("auto_save = true").unwrap();
        assert!(cfg.auto_save);
    }

    #[test]
    fn background_tidy_parses_on_overlap_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"on_overlap\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::OnOverlap);
    }

    #[test]
    fn background_tidy_parses_off_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"off\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::Off);
    }

    #[test]
    fn background_tidy_parses_debounced_from_toml() {
        let cfg: Config = toml::from_str("background_tidy = \"debounced\"").unwrap();
        assert_eq!(cfg.background_tidy, BackgroundTidy::Debounced);
    }

    #[test]
    fn aux_storage_defaults_to_ask() {
        assert_eq!(Config::default().aux_storage, AuxStorage::Ask);
    }

    #[test]
    fn aux_storage_parses_variants_from_toml() {
        let c: Config = toml::from_str("aux_storage = \"archive\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Archive);
        let c: Config = toml::from_str("aux_storage = \"global\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Global);
    }

    #[test]
    fn write_config_round_trips_scalars_and_preserves_keymap() {
        let dir = std::env::temp_dir().join(format!("babelmap_write_config_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Write initial config with a [keymap] section and a comment.
        let initial = "# babelmap config\n[keymap]\nzoom_in = \"z\"\n";
        std::fs::write(dir.join("config.toml"), initial).unwrap();

        let cfg = Config {
            user_dir: dir.clone(),
            use_default_map: true,
            auto_load: false,
            auto_save: true,
            mouse_wheel_invert: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_history: false,
            record_turn_history: false,
            background_tidy: BackgroundTidy::OnOverlap,
            aux_storage: AuxStorage::Ask,
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: Some("neon".into()),
            watch_style: false,
            undo_levels: 16,
            command_prefix: '/',
            show_room_numbers: false,
            show_loc_method: false,
            show_status_bar: true,
            honor_game_colours: true,
            honor_timed_input: true,
            interpreter_number: None,
            enable_sound: true,
            volume: 100,
            search: SearchConfig::default(),
            virtual_screen_cols: 80,
            virtual_screen_rows: 24,
            animation: AnimationConfig::default(),
            acceleration: true,
        };
        write_config(&dir, &cfg).unwrap();

        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();

        // Scalars are set.
        assert_eq!(doc["use_default_map"].as_bool(), Some(true));
        assert_eq!(doc["auto_load"].as_bool(), Some(false));
        assert_eq!(doc["auto_save"].as_bool(), Some(true));
        assert_eq!(doc["record_history"].as_bool(), Some(false));
        assert_eq!(doc["background_tidy"].as_str(), Some("on_overlap"));
        // Style pointer is written; visual sections are NOT.
        assert_eq!(doc["style"].as_str(), Some("neon"));
        assert!(!content.contains("[colors]"));
        assert!(!content.contains("[symbols]"));
        // Keymap is preserved.
        assert_eq!(doc["keymap"]["zoom_in"].as_str(), Some("z"));
        // Comment is in the raw text.
        assert!(content.contains("# babelmap config"), "comment must be preserved");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_reads_style_pointer() {
        let cfg: Config = toml::from_str("style = \"neon\"\n").unwrap();
        assert_eq!(cfg.style.as_deref(), Some("neon"));
    }

    #[test]
    fn prompt_flags_default_true_and_round_trip() {
        assert!(Config::default().prompt_save_on_quit);
        assert!(Config::default().prompt_load_on_launch);
        // Setting one to false parses correctly, other keeps default true.
        let cfg: Config = toml::from_str("prompt_save_on_quit = false\n").unwrap();
        assert!(!cfg.prompt_save_on_quit);
        assert!(cfg.prompt_load_on_launch);
    }

    #[test]
    fn search_config_defaults_and_round_trip() {
        let d = Config::default();
        assert!(d.search.start_backward);
        assert_eq!(d.search.key_back, 'n');
        assert_eq!(d.search.key_forward, 'N');
        let cfg: Config = toml::from_str("[search]\nstart_backward = false\nkey_forward = \"j\"\n").unwrap();
        assert!(!cfg.search.start_backward);
        assert_eq!(cfg.search.key_forward, 'j');
        assert_eq!(cfg.search.key_back, 'n'); // default kept
    }

    #[test]
    fn write_config_does_not_emit_style_sections() {
        let dir = std::env::temp_dir().join(format!(
            "babelmap_write_config_no_style_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // seed a config with functional + a [keymap] to confirm preservation
        std::fs::write(dir.join("config.toml"), "auto_save = true\n[keymap]\nquit = \"q\"\n").unwrap();
        let mut cfg = Config::default();
        cfg.auto_save = true;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(!text.contains("[colors]"));
        assert!(!text.contains("[symbols]"));
        assert!(text.contains("[keymap]")); // functional sections preserved

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn animation_config_defaults() {
        let c = Config::default();
        assert!(c.animation.enabled);
        assert_eq!(c.animation.easing, Easing::EaseOut);
        assert_eq!(c.animation.scroll_ms, 120);
    }

    #[test]
    fn animation_config_absent_uses_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.animation.enabled);
        assert_eq!(cfg.animation.easing, Easing::EaseOut);
        assert_eq!(cfg.animation.scroll_ms, 120);
    }

    #[test]
    fn animation_config_parses_table() {
        let cfg: Config = toml::from_str(
            "[animation]\nenabled = false\neasing = \"linear\"\nscroll_ms = 200\n",
        )
        .unwrap();
        assert!(!cfg.animation.enabled);
        assert_eq!(cfg.animation.easing, Easing::Linear);
        assert_eq!(cfg.animation.scroll_ms, 200);
    }

    #[test]
    fn animation_config_unknown_easing_falls_back_to_ease_out() {
        let cfg: Config = toml::from_str("[animation]\neasing = \"wobble\"\n").unwrap();
        assert_eq!(cfg.animation.easing, Easing::EaseOut);
    }

    #[test]
    fn write_config_round_trips_animation() {
        let dir = std::env::temp_dir().join(format!(
            "babelmap_write_config_anim_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::default();
        cfg.animation = AnimationConfig {
            enabled: false,
            easing: Easing::EaseInOut,
            scroll_ms: 250,
        };
        write_config(&dir, &cfg).unwrap();
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["animation"]["enabled"].as_bool(), Some(false));
        assert_eq!(doc["animation"]["easing"].as_str(), Some("ease-in-out"));
        assert_eq!(doc["animation"]["scroll_ms"].as_integer(), Some(250));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn honor_game_colours_defaults_true() {
        let c = Config::default();
        assert!(c.honor_game_colours);
        // round-trips through TOML: absent key keeps the default true
        let back: Config = toml::from_str("").unwrap();
        assert!(back.honor_game_colours);
        // explicit false overrides the default
        let off: Config = toml::from_str("honor_game_colours = false\n").unwrap();
        assert!(!off.honor_game_colours);
    }

    #[test]
    fn acceleration_defaults_true_and_no_accel_disables() {
        assert!(Config::default().acceleration);

        let cli = Cli {
            story: PathBuf::from("foo.z5"),
            user_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: true,
        };
        let cfg = resolve(&cli);
        assert!(!cfg.acceleration);
    }

    #[test]
    fn honor_timed_input_defaults_true() {
        let c = Config::default();
        assert!(c.honor_timed_input);
        // round-trips through TOML: absent key keeps the default true
        let back: Config = toml::from_str("").unwrap();
        assert!(back.honor_timed_input);
        // explicit false overrides the default
        let off: Config = toml::from_str("honor_timed_input = false\n").unwrap();
        assert!(!off.honor_timed_input);
    }

    #[test]
    fn enable_sound_defaults_true() {
        assert!(Config::default().enable_sound);
        let back: Config = toml::from_str("").unwrap();
        assert!(back.enable_sound, "absent key keeps default true");
        let off: Config = toml::from_str("enable_sound = false\n").unwrap();
        assert!(!off.enable_sound);
    }

    #[test]
    fn volume_defaults_100_and_roundtrips() {
        assert_eq!(Config::default().volume, 100);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.volume, 100, "absent key keeps default 100");
        let set: Config = toml::from_str("volume = 40\n").unwrap();
        assert_eq!(set.volume, 40);
    }

    #[test]
    fn interpreter_number_defaults_none_and_parses_override() {
        // Default and absent key → None (auto).
        assert_eq!(Config::default().interpreter_number, None);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.interpreter_number, None, "absent key keeps None");
        // Explicit override parses.
        let over: Config = toml::from_str("interpreter_number = 6\n").unwrap();
        assert_eq!(over.interpreter_number, Some(6), "explicit override parses");
    }

    #[test]
    fn shipped_keymap_example_parses() {
        let toml = r#"
[keymap]
use_defaults = true
[keymap.map]
"+" = "zoom-map in"
"c" = "center-map"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let (km, warns) = crate::keymap::KeyMap::resolve(&cfg.keymap);
        assert!(warns.is_empty());
        let c: crate::keymap::KeySpec = "c".parse().unwrap();
        assert_eq!(km.lookup(&c, crate::keymap::Context::Map), Some("center-map"));
    }

    #[test]
    fn symbol_config_badge_glyph_defaults() {
        let s = SymbolConfig::default();
        assert_eq!(s.badge_zcode, "Z");
        assert_eq!(s.badge_glulx, "G");
        assert_eq!(s.badge_blorb, "B");
        assert_eq!(s.badge_save, "S");
        assert_eq!(s.badge_hint, "H");
    }

    #[test]
    fn symbol_config_badge_glyph_override_and_absent_default() {
        // Overriding one field parses; the others keep their defaults.
        let toml = r#"
            badge_blorb = "◆"
        "#;
        let s: SymbolConfig = toml::from_str(toml).unwrap();
        assert_eq!(s.badge_blorb, "◆");
        assert_eq!(s.badge_zcode, "Z");
        assert_eq!(s.badge_hint, "H");
    }
}
