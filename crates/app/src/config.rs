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
///   "ctrl+s" = "save-state"
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
pub(crate) fn default_badge_hint_available() -> String { "h".into() }
pub(crate) fn default_diagonal_corners() -> bool { true }
pub(crate) fn default_portal_path_style() -> String { "dotted".into() }

/// The resolved map glyph configuration, built from style.toml's `[map]`
/// section by `style::finalize_symbols`.  All fields default to the preset
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
    /// Line-art preset for the up/down/in/out portal connectors, chosen
    /// separately from the cardinal `path_style` (default "dotted" — the
    /// ┊/┄ connectors the map has always drawn).
    #[serde(default = "default_portal_path_style")]
    pub portal_path_style: String,
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
    /// Row "a hint is available to download" artifact badge glyph (default "h").
    #[serde(default = "default_badge_hint_available")]
    pub badge_hint_available: String,
    /// Draw a diagonal stub out of a room corner for ne/nw/se/sw exits (SQ-0314).
    /// Default true. Set false for a terminal/font without Unicode 13 Legacy
    /// Computing coverage: the map falls back to the corner arrow plus a purely
    /// orthogonal path (the pre-SQ-0314 look).
    #[serde(default = "default_diagonal_corners")]
    pub diagonal_corners: bool,
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
            portal_path_style: default_portal_path_style(),
            badge_zcode: default_badge_zcode(),
            badge_glulx: default_badge_glulx(),
            badge_blorb: default_badge_blorb(),
            badge_save: default_badge_save(),
            badge_hint: default_badge_hint(),
            badge_hint_available: default_badge_hint_available(),
            diagonal_corners: default_diagonal_corners(),
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
#[command(
    name = "babelmap",
    version = buildinfo::LONG,
    about = "Interactive-fiction interpreter (Z-machine, Glulx, Scott Adams) with live automapping",
    // Show `babelmap <version>` at the top of --help (clap omits it by default).
    help_template = "{before-help}{name} {version}\n{about-with-newline}\n{usage-heading} {usage}\n\n{all-args}{after-help}"
)]
pub struct Cli {
    /// Path to a story file (.z3/.z5/.z8 etc.) or a directory to browse. When
    /// omitted, falls back to the `default_story_dir` config setting.
    pub story: Option<PathBuf>,

    /// Override the babelmap home directory (default: ~/.babelmap)
    #[arg(long, value_name = "PATH")]
    pub user_dir: Option<PathBuf>,

    /// Override the storage base for saves/sidecars (default: <user_dir>/saves).
    /// Files land in `<data_dir>/<story-filename>/`.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// Path to a non-default config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Disable Glulx accelerated-function interception (debug; default: enabled)
    #[arg(long)]
    pub no_accel: bool,

    /// Disable sound for this run (bleeps + sampled audio); the border still
    /// flashes as the accessibility cue. Overrides the config's `enable_sound`.
    #[arg(long)]
    pub no_sound: bool,

    /// Force the terminal image protocol for cover art (default: auto-detect).
    #[arg(long, value_enum, default_value_t = ImageProtocol::Auto)]
    pub image_protocol: ImageProtocol,

    /// Disable all image rendering (in-game graphics + story-picker cover art).
    #[arg(long)]
    pub no_images: bool,

    /// Interpreter number to advertise in the story header (0x1E).
    ///
    /// Games branch on it: Beyond Zork picks character graphics over colour on
    /// IBM PC, and several v6 story files were built for one specific machine.
    /// The values are ZMSD §11.1.3's table:
    ///
    ///   1  DECSystem-20      7  Commodore 128
    ///   2  Apple IIe         8  Commodore 64
    ///   3  Macintosh         9  Apple IIc
    ///   4  Amiga            10  Apple IIgs
    ///   5  Atari ST         11  Tandy Color
    ///   6  IBM PC
    ///
    /// Overrides `interpreter_number` in config.toml. With neither set, babelmap
    /// auto-selects per Frotz's rule: 6 (IBM PC) for v6, else 1 (DECSystem-20).
    #[arg(long, value_name = "N", verbatim_doc_comment)]
    pub interpreter_number: Option<u8>,

    /// Debug trace sections to enable from boot: comma list of screen,map,hostio,v6
    /// (or `all`/`none`). Output goes to <user_dir>/trace.log. (trace feature)
    #[arg(long, value_name = "LIST")]
    pub trace: Option<String>,

    /// Trace execution from boot into the debug disassembly cache and auto-open
    /// the inspector. Loads/saves a per-story executed-PC sidecar so coverage
    /// (blue lines) persists across runs. (SQ-0449)
    #[arg(long)]
    pub debug: bool,
}

/// Terminal image protocol for cover art. `Auto` detects the best available
/// (falling back to half-blocks); the rest force a specific mode for testing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ImageProtocol {
    Auto,
    Halfblocks,
    Kitty,
    Sixel,
    Iterm2,
}

fn default_image_protocol() -> ImageProtocol {
    ImageProtocol::Auto
}

fn default_images() -> bool { true }

// ── Hotkeys config ────────────────────────────────────────────────────────────

/// One group of commands shown together in the hotkey dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeyGroupConfig {
    pub title: String,
    pub commands: Vec<String>,
}

/// The `[hotkeys]` section of config.toml.
/// `prefix` overrides the dialog-open key (default: Ctrl+P).
/// `direct` overrides which commands are always available (bypass dialog).
/// `group` overrides the command groups shown in the dialog.
#[derive(Debug, Default, Deserialize, Clone)]
pub struct HotkeysConfig {
    /// Override the dialog-prefix key spec string (e.g. "ctrl+p").
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

/// Fallback screen size used only before the story pane has been measured (the
/// engine boots before the first frame) and by the Glulx factory, whose real
/// size arrives one poll later from `poll_glulx_resize`. ZMSD §8.4 asks for "at
/// least 60 characters wide by 14 lines deep"; 80×24 is the classic terminal.
pub const FALLBACK_SCREEN_COLS: u16 = 80;
pub const FALLBACK_SCREEN_ROWS: u16 = 24;
pub(crate) fn default_split_ratio() -> u16 { 50 }
pub(crate) fn default_verb_dock_pct() -> u16 { 32 }
pub(crate) fn default_inv_dock_pct() -> u16 { 33 }
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

/// Fallback for [`Config::config_file`] when a `Config` is built without [`resolve`]
/// (tests, `Config::default()`): the default home's config.toml.
fn default_config_file() -> PathBuf {
    default_user_dir().join("config.toml")
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

/// How many device pixels the host reports per "pixel" a Glulx game asks for
/// (SQ-0593).
///
/// TOML: `glk_pixel_scale = "auto"` (default) or an integer like `1` or `2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlkPixelScale {
    /// Derive it from the terminal's cell height (see [`GlkPixelScale::resolve`]).
    #[default]
    Auto,
    /// Report the cell size divided by exactly this, whatever the terminal says.
    Fixed(u32),
}

/// Accepts `"auto"` or a bare integer. A derived `untagged` impl cannot do this — an
/// untagged unit variant matches null, not the string `"auto"` — so the two shapes are
/// spelled out.
impl<'de> serde::Deserialize<'de> for GlkPixelScale {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(u32),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Ok(GlkPixelScale::Fixed(n)),
            Raw::Str(s) if s.eq_ignore_ascii_case("auto") => Ok(GlkPixelScale::Auto),
            // A quoted number is a natural thing to write and costs nothing to accept.
            Raw::Str(s) => s.trim().parse::<u32>().map(GlkPixelScale::Fixed).map_err(|_| {
                serde::de::Error::custom(format!(
                    "glk_pixel_scale must be \"auto\" or a positive integer, got {s:?}"
                ))
            }),
        }
    }
}

/// A conventional terminal cell height in device pixels, and the yardstick `Auto`
/// measures against. It is what an unscaled 1x display reports for a normal font, so
/// `Auto` resolves to 1 there and changes nothing.
pub const REFERENCE_CELL_PX: u32 = 14;

impl GlkPixelScale {
    /// The cell size to hand a Glulx game, given what the terminal reports.
    ///
    /// A Glk game sizes its graphics windows in PIXELS, and those requests are
    /// constants its author picked against a conventional screen — advent.blb asks for
    /// a 36px toolbar. The row count that buys is `ceil(36 / cell_height)`, so the
    /// terminal's cell size decides how much of the screen the game's artwork occupies:
    /// a fixed strip at 40px whatever the font, which on a scaled display or with a
    /// large font is a shrinking fraction of a growing screen.
    ///
    /// `Auto` normalises that away by reporting a cell of exactly
    /// [`REFERENCE_CELL_PX`] tall, with the width scaled to preserve the terminal's own
    /// aspect ratio. Every terminal then presents the same pixel space, so a game's
    /// layout is identical everywhere and its artwork scales WITH the text rather than
    /// against it — 3 rows at any font size, `3 × cell_height` on screen.
    ///
    /// Deliberately not keyed off the display's DPI: a large font on an unscaled
    /// display produces the identical complaint while reporting no unusual DPI, and
    /// reading the real scale would need a separate platform path for each Linux
    /// compositor, macOS and Windows. It is also deliberately not an integer divisor,
    /// which was the first attempt — `round(cell / 14)` puts a cliff at 21px, where a
    /// one-pixel font change doubled the toolbar (40px → 84px). 21px is exactly a 1.5x
    /// scaled 14px cell, i.e. a common Windows and GNOME default, so that cliff sat in
    /// the middle of real configurations.
    ///
    /// `Fixed(n)` divides by `n` instead, an escape hatch for a game that dislikes the
    /// normalisation; `Fixed(1)` reports the terminal's cell unchanged, the behaviour
    /// before any of this existed. Never returns 0 on either axis.
    pub fn apply(self, cell: (u32, u32)) -> (u32, u32) {
        match self {
            GlkPixelScale::Fixed(n) => {
                let n = n.max(1);
                ((cell.0 / n).max(1), (cell.1 / n).max(1))
            }
            GlkPixelScale::Auto => {
                let h = cell.1.max(1);
                let w = ((cell.0 as f32 * REFERENCE_CELL_PX as f32 / h as f32).round() as u32).max(1);
                (w, REFERENCE_CELL_PX)
            }
        }
    }
}

/// How the v6 graphical story pane is rendered.
///
/// TOML: `v6_render = "hybrid"` (default), `"raster"`, or `"frameless"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum V6RenderMode {
    /// Chrome (frame + status) as a scaled pixel ring around a terminal story
    /// viewport with crisp text (default).
    #[default]
    Hybrid,
    /// The whole pane rasterized into one pixel image (feature-limited).
    Raster,
    /// No decorative frame at all: the story runs as a normal full-pane terminal
    /// transcript (native scrollback, full-size text), the game's chrome/status
    /// text collapses to a compact terminal status band across the top, and
    /// inline story pictures still render via the transcript image path. The
    /// decorative frame art and full-screen graphics windows are simply not
    /// drawn. With `--no-images` this equals the classic cell fallback. (SQ-0461)
    Frameless,
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

/// Current `config.toml` schema version. Bump when a config change means an
/// older hand-written file may behave unexpectedly. `write_config` stamps this
/// as `version = N`; a future babelmap can compare a file's `version` against
/// this to flag an out-of-date config. A file with no `version` reads as 0.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// User preferences loaded from TOML.  Every field has a default so a missing
/// config file (or a file with only some fields) is always valid.
///
/// ADDING A PERSISTED FIELD — touch ALL of these or it silently half-works:
///   1. this struct (with a `#[serde(default …)]`);
///   2. `impl Default for Config` AND the test-builder `Config { … }` literal
///      further down (both are full literals — a new field must be listed in
///      each or the crate won't compile, which is the good case);
///   3. `resolve`'s field-by-field merge from `from_file` — MISS THIS and a
///      value in the file is ignored (default always wins on load);
///   4. `write_config`'s `doc["…"] = …` — MISS THIS and a settings-panel edit
///      is never written, so it reverts to the default on the next launch.
///
/// Steps 3 and 4 fail SILENTLY (they still compile); the round-trip test
/// `write_config_persists_panel_editable_scalars_round_trip` guards the class.
/// Runtime-only fields (`#[serde(skip)]`, e.g. `acceleration`) skip 3 and 4.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    /// Schema stamp (see [`CONFIG_SCHEMA_VERSION`]). A file written before
    /// versioning has no `version` key and reads as 0.
    #[serde(default)]
    pub version: u32,
    /// Root directory for babelmap data (maps, saves, exports).
    /// Sub-directories: maps/ — where per-story map files live.
    #[serde(default = "default_user_dir")]
    pub user_dir: PathBuf,
    /// Directory (or story file) opened when babelmap is launched with no path
    /// argument. `None` (default) means a path is required on the command line.
    #[serde(default)]
    pub default_story_dir: Option<PathBuf>,
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
    /// When true, capture the mouse (click-to-select in the story browser and
    /// map, wheel scrolling, and Glk mouse input to games that request it).
    /// Default true (SQ-0298): in-app mouse support is on out of the box. Set
    /// `mouse = false` to disable it — mouse capture puts the terminal in
    /// any-motion reporting mode (every movement drives a redraw) and overrides
    /// the terminal's native text selection.
    #[serde(default = "default_true")]
    pub mouse: bool,
    /// When true, edit story commands in a persistent command bar instead of
    /// the inline story-text prompt. Default false: the inline prompt.
    #[serde(default)]
    pub command_bar: bool,
    /// When true (default) and auto_save is off, prompt the user to save on quit.
    #[serde(default = "default_true")]
    pub prompt_save_on_quit: bool,
    /// When true (default) and auto_load is off, prompt the user to resume a found save on launch.
    #[serde(default = "default_true")]
    pub prompt_load_on_launch: bool,
    /// When true, record a per-turn rewind/replay history (Quetzal save + map
    /// snapshots) into the `.babelmap` archive. Default false (opt-in: it grows
    /// the archive and keeps per-turn blobs in memory).
    #[serde(default)]
    pub record_turn_history: bool,
    /// When true (default), auto-skip the InvisiClues "your screen is only N
    /// characters wide…" banner that izm hint files print at startup, landing the
    /// player straight on the topic menu. Set false to see the banner and dismiss
    /// it yourself.
    #[serde(default = "default_true")]
    pub hint_skip_screen_warning: bool,
    /// Controls automatic background re-tidy when new rooms are discovered.
    /// Default: EveryRoom (re-tidy on each turn that finds a new room).
    #[serde(default)]
    pub background_tidy: BackgroundTidy,
    /// Where to persist v5 auxiliary save data. Default: Ask.
    #[serde(default)]
    pub aux_storage: AuxStorage,
    /// How the v6 graphical story pane is rendered. Default: Hybrid.
    #[serde(default)]
    pub v6_render: V6RenderMode,
    /// Divisor applied to the terminal's reported cell size before a Glulx game
    /// sees it, so a game's fixed pixel sizes keep their proportions on a scaled
    /// display or with a large font (SQ-0593). Default: Auto.
    #[serde(default)]
    pub glk_pixel_scale: GlkPixelScale,
    /// When true (default), forward arrow keypresses to a v6 story as ZSCII
    /// cursor codes (129-132). Some v6 games bind arrows to movement; set false
    /// (or `--no-v6-arrows`) to withhold them so arrows drive app-side
    /// scrollback/map panning instead. Only affects v6 — v1-5/Glulx stories
    /// always get arrows regardless of this setting.
    #[serde(default = "default_true")]
    pub v6_arrow_keys: bool,
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
    /// Show the status/score bar (top row of the story pane). Default true.
    /// The v3 status line (location/score/moves) is only meaningful for v3
    /// games; for v4+ (which draw their own upper-window status) it reads
    /// garbage globals, so this can be toggled off (ToggleStatusBar).
    #[serde(default = "default_true")]
    pub show_status_bar: bool,
    /// Search configuration: start direction, nav keys.
    #[serde(default)]
    pub search: SearchConfig,
    /// OVERRIDE for the screen width (in characters) reported to the Z-machine in
    /// header byte $21. Unset (the default) means "follow the story pane": ZMSD
    /// §8.4 requires the interpreter to "write the current height (in lines) and
    /// width (in characters) into bytes $20 and $21", and it "may change the exact
    /// dimensions whenever it likes", so babelmap reports the pane's real measured
    /// size and re-reports it on every terminal resize. Set this key only to pin a
    /// fixed virtual screen (e.g. to reproduce a game's original 80-column
    /// layout); a pinned width no longer matches what the transcript wraps at, so
    /// the upper window will be drawn centred inside the pane. (SQ-0532/A-F1)
    #[serde(default)]
    pub virtual_screen_cols: Option<u16>,
    /// OVERRIDE for the screen height (in lines) reported in header byte $20.
    /// Unset (the default) follows the story pane — see `virtual_screen_cols`.
    #[serde(default)]
    pub virtual_screen_rows: Option<u16>,
    /// Story pane's share of the story/map Split, as a percentage (default 50).
    #[serde(default = "default_split_ratio")]
    pub split_ratio: u16,
    /// Verb dock width as a percentage of screen width (default 32, ≈ the old
    /// fixed 26-of-80 columns).
    #[serde(default = "default_verb_dock_pct")]
    pub verb_dock_pct: u16,
    /// Inventory dock height cap as a percentage of screen height (default 33,
    /// ≈ the old fixed 1/3 cap).
    #[serde(default = "default_inv_dock_pct")]
    pub inv_dock_pct: u16,
    /// Inner margin reserved inside the text-buffer (transcript) window, in
    /// character cells: `text_margin_x` blank columns on each side,
    /// `text_margin_y` blank rows top and bottom. Default 0. Populated from
    /// garglk `tmarginx`/`tmarginy` (converted px→cells) when a garglk config
    /// is imported (SQ-0344).
    #[serde(default)]
    pub text_margin_x: u16,
    #[serde(default)]
    pub text_margin_y: u16,
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
    /// character-graphics instead of colour. The full ZMSD §11.1.3 value table is
    /// on `Cli::interpreter_number`, which overrides this for one run.
    #[serde(default)]
    pub interpreter_number: Option<u8>,
    /// The config file this `Config` was READ from, so a later save goes back to the
    /// same file rather than to `user_dir` (SQ-0574). Set by [`resolve`]; never
    /// persisted, and not part of the file's schema.
    #[serde(skip, default = "default_config_file")]
    pub config_file: PathBuf,
    /// Set when the config file exists but does not parse: the TOML error, carried so
    /// startup can say which line broke instead of running on defaults in silence
    /// (SQ-0580). A single typo costs the user the WHOLE file — TOML is parsed as one
    /// document, so there is no partial load to fall back to. Never persisted, and not
    /// part of the file's schema.
    #[serde(skip)]
    pub config_error: Option<String>,
    /// True when `interpreter_number` came from `--interpreter-number` rather than
    /// the config file. A one-run CLI override must not be baked into config.toml by
    /// a later settings write, so [`write_config`] leaves the file's own value alone
    /// when this is set. Never persisted itself.
    #[serde(skip)]
    pub interpreter_number_from_cli: bool,
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
    /// Cover-art image protocol. Runtime-only (set from --image-protocol);
    /// not persisted or user-facing.
    #[serde(skip, default = "default_image_protocol")]
    pub image_protocol: ImageProtocol,
    /// Whether image rendering (in-game graphics + cover art) is enabled.
    /// Runtime-only (set from --no-images); not persisted.
    #[serde(skip, default = "default_images")]
    pub images: bool,
    /// Active debug-trace sections. Runtime-only (from --trace / /trace); not persisted.
    #[serde(skip)]
    pub trace: crate::trace::TraceSections,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            user_dir: default_user_dir(),
            default_story_dir: None,
            auto_load: true,
            auto_save: false,
            mouse_wheel_invert: false,
            mouse: true,
            command_bar: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_turn_history: false,
            hint_skip_screen_warning: true,
            background_tidy: BackgroundTidy::EveryRoom,
            aux_storage: AuxStorage::Ask,
            v6_render: V6RenderMode::Hybrid,
            glk_pixel_scale: GlkPixelScale::Auto,
            v6_arrow_keys: true,
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: None,
            watch_style: false,
            undo_levels: default_undo_levels(),
            command_prefix: default_command_prefix(),
            show_room_numbers: false,
            show_status_bar: true,
            search: SearchConfig::default(),
            virtual_screen_cols: None,
            virtual_screen_rows: None,
            split_ratio: default_split_ratio(),
            verb_dock_pct: default_verb_dock_pct(),
            inv_dock_pct: default_inv_dock_pct(),
            text_margin_x: 0,
            text_margin_y: 0,
            animation: AnimationConfig::default(),
            honor_game_colours: default_honor_game_colours(),
            honor_timed_input: default_honor_timed_input(),
            config_file: default_config_file(),
            config_error: None,
            interpreter_number: None,
            interpreter_number_from_cli: false,
            enable_sound: default_enable_sound(),
            volume: default_volume(),
            acceleration: default_acceleration(),
            image_protocol: default_image_protocol(),
            images: default_images(),
            trace: crate::trace::TraceSections::default(),
        }
    }
}

/// The config file path `resolve` reads from, most specific first: the `--config`
/// override, else `--user-dir`'s `config.toml`, else the default home's.
///
/// SQ-0574: `--user-dir` used to be ignored here, so it relocated every WRITE
/// (`write_config` and the template seed both took `cfg.user_dir`) while reads still
/// came from the default home — `babelmap --user-dir /tmp/x` seeded and saved
/// `/tmp/x/config.toml` and then loaded `~/.babelmap/config.toml`, silently
/// discarding everything it had just written.
///
/// Deliberately driven by the CLI alone: the `user_dir` KEY inside the file names the
/// data root (maps, saves, exports) and cannot also name the file it was read from
/// without being circular. `--user-dir` moves both; the key moves only the data.
pub fn config_path(cli: &Cli) -> std::path::PathBuf {
    match &cli.config {
        Some(p) => p.clone(),
        None => cli.user_dir.clone().unwrap_or_else(default_user_dir).join("config.toml"),
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
    let mut cfg = Config { config_file: config_path.clone(), ..Config::default() };

    // Layer in the config file if it exists.
    if let Ok(text) = std::fs::read_to_string(&config_path) {
        let parsed = toml::from_str::<Config>(&text);
        // A file that exists but doesn't parse used to be dropped in silence, so one
        // stray character reverted every setting to its default with nothing said —
        // and the next settings save then overwrote the user's file (SQ-0580). Keep
        // the error for startup to show; `write_config_at` refuses to clobber.
        if let Err(e) = &parsed {
            cfg.config_error = Some(e.to_string());
        }
        if let Ok(from_file) = parsed {
            // NOTE: this is a field-by-field merge — every persisted field must
            // be copied here or the file's value is ignored on load. See the
            // checklist on `struct Config`. (Also mirror it in `write_config`.)
            // Carry the file's own version stamp (0 if the file predates
            // versioning) so a future check can flag an out-of-date config.
            cfg.version = from_file.version;
            cfg.user_dir = from_file.user_dir;
            cfg.default_story_dir = from_file.default_story_dir;
            cfg.auto_load = from_file.auto_load;
            cfg.auto_save = from_file.auto_save;
            cfg.mouse_wheel_invert = from_file.mouse_wheel_invert;
            cfg.mouse = from_file.mouse;
            cfg.command_bar = from_file.command_bar;
            cfg.prompt_save_on_quit = from_file.prompt_save_on_quit;
            cfg.prompt_load_on_launch = from_file.prompt_load_on_launch;
            cfg.record_turn_history = from_file.record_turn_history;
            cfg.hint_skip_screen_warning = from_file.hint_skip_screen_warning;
            cfg.background_tidy = from_file.background_tidy;
            cfg.aux_storage = from_file.aux_storage;
            cfg.v6_render = from_file.v6_render;
            cfg.glk_pixel_scale = from_file.glk_pixel_scale;
            cfg.v6_arrow_keys = from_file.v6_arrow_keys;
            cfg.keymap = from_file.keymap;
            cfg.hotkeys = from_file.hotkeys;
            cfg.style = from_file.style;
            cfg.watch_style = from_file.watch_style;
            cfg.undo_levels = from_file.undo_levels;
            cfg.command_prefix = from_file.command_prefix;
            cfg.show_room_numbers = from_file.show_room_numbers;
            cfg.show_status_bar = from_file.show_status_bar;
            cfg.honor_game_colours = from_file.honor_game_colours;
            cfg.honor_timed_input = from_file.honor_timed_input;
            cfg.interpreter_number = from_file.interpreter_number;
            cfg.enable_sound = from_file.enable_sound;
            cfg.volume = from_file.volume;
            cfg.search = from_file.search;
            cfg.virtual_screen_cols = from_file.virtual_screen_cols;
            cfg.virtual_screen_rows = from_file.virtual_screen_rows;
            cfg.split_ratio = from_file.split_ratio;
            cfg.verb_dock_pct = from_file.verb_dock_pct;
            cfg.inv_dock_pct = from_file.inv_dock_pct;
            cfg.text_margin_x = from_file.text_margin_x;
            cfg.text_margin_y = from_file.text_margin_y;
            cfg.animation = from_file.animation;
        }
        // A malformed file leaves every field at its default — TOML is parsed as one
        // document, so there is no half-loaded config to salvage.
    }

    // CLI overrides beat the file.
    if let Some(dir) = &cli.user_dir {
        cfg.user_dir = dir.clone();
    }

    cfg.acceleration = !cli.no_accel;
    cfg.image_protocol = cli.image_protocol;
    cfg.images = !cli.no_images;
    // One-way override: `--no-sound` forces sound off for this run, but its
    // absence must not turn on a config that persisted `enable_sound = false`.
    if cli.no_sound {
        cfg.enable_sound = false;
    }

    // `--interpreter-number N` beats the file's `interpreter_number`; absent, the
    // file's value (or the auto rule, when it too is unset) stands.
    if let Some(n) = cli.interpreter_number {
        cfg.interpreter_number = Some(n);
        cfg.interpreter_number_from_cli = true;
    }

    if let Some(list) = &cli.trace {
        let (sections, unknown) = crate::trace::TraceSections::parse(list);
        cfg.trace = sections;
        for u in unknown {
            eprintln!("warning: unknown --trace section '{u}' (valid: screen, map, hostio, v6, all, none)");
        }
    }

    cfg
}

// ── Write helpers ─────────────────────────────────────────────────────────────

/// Set `key` only when it is worth persisting (SQ-0573).
///
/// A setting at its default belongs in the seeded commented template, not as a live
/// key: `write_config` used to stamp all ~36 keys unconditionally, so the first
/// settings save — the story browser's "remember this directory?" prompt is enough —
/// appended the whole flat key list to a freshly seeded config.toml, pinning every
/// default and burying the comments (they land BELOW the inserted keys, since an
/// all-comment file parses as trailing trivia).
///
/// So: add a key only when its value differs from the default, but ALWAYS update one
/// the file already has. That second half matters twice over — it keeps a setting the
/// user flipped back to its default from silently reverting on the next launch, and
/// it means nothing is ever removed, so a comment the user wrote above their own key
/// (which toml_edit attaches to that key as decor) is never dropped.
fn put(doc: &mut toml_edit::DocumentMut, key: &str, value: toml_edit::Value, is_default: bool) {
    if !is_default || doc.contains_key(key) {
        doc[key] = toml_edit::Item::Value(value);
    }
}

/// [`put`] for a key inside a table.
fn put_in(tbl: &mut toml_edit::Item, key: &str, value: toml_edit::Value, is_default: bool) {
    let present = tbl.get(key).is_some();
    if !is_default || present {
        tbl[key] = toml_edit::Item::Value(value);
    }
}

/// Save `cfg` back to the file it was loaded from ([`Config::config_file`]).
///
/// Production code should use this rather than [`write_config`]: writing to
/// `cfg.user_dir` meant a `--user-dir` (or a `user_dir` key) sent the save somewhere
/// `resolve` would never read it back from (SQ-0574).
pub fn write_config_file(cfg: &Config) -> std::io::Result<()> {
    write_config_at(&cfg.config_file, cfg)
}

/// Write the functional config fields (and the `style` pointer) to `dir/config.toml`
/// using toml_edit (format-preserving). Creates the file and parent directory if absent.
/// Does NOT emit `[colors]`/`[symbols]` — those now live in the style file.
/// Preserves all other content (comments, `[keymap]`, `[hotkeys]`, any visual sections, etc.).
///
/// NOTE: every persisted field needs a `doc["…"] = …` line below — a field
/// that's missing here is never written, so a settings-panel edit silently
/// reverts on the next launch. See the checklist on `struct Config` (and mirror
/// any new field in `resolve`).
pub fn write_config(dir: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    write_config_at(&dir.join("config.toml"), cfg)
}

/// [`write_config`] to an exact file path — the form [`write_config_file`] uses so a
/// save lands on the file `resolve` read (SQ-0574). `write_config(dir, …)` remains for
/// callers that genuinely mean "this directory's config.toml", chiefly tests.
pub fn write_config_at(config_path: &std::path::Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    // A file that doesn't parse is NOT a blank slate to build on: `unwrap_or_default`
    // here meant the first settings save (the story browser's "remember this
    // directory?" prompt is enough) replaced the user's entire file — every key and
    // every comment — with a fresh doc, destroying the very text they need to see to
    // fix the typo. Refuse instead, and say why (SQ-0580). An absent or empty file
    // parses fine, so this only ever fires on real syntax errors.
    let mut doc: toml_edit::DocumentMut = match existing.parse() {
        Ok(doc) => doc,
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} is not valid TOML ({e}) — refusing to overwrite it. Fix the file, \
                     or move it aside and babelmap will seed a fresh one.",
                    config_path.display(),
                ),
            ));
        }
    };
    // Defaults to compare against, so a setting nobody changed is not written out.
    let def = Config::default();

    // Top-level scalar fields. Always stamp the current schema version — writing
    // the file brings it up to the format this build produces.
    doc["version"] = toml_edit::value(CONFIG_SCHEMA_VERSION as i64);
    put(&mut doc, "user_dir", cfg.user_dir.to_string_lossy().as_ref().into(), cfg.user_dir == def.user_dir);
    match &cfg.default_story_dir {
        Some(p) => { doc["default_story_dir"] = toml_edit::value(p.to_string_lossy().as_ref()); }
        None => { doc.remove("default_story_dir"); }
    }
    put(&mut doc, "auto_load", cfg.auto_load.into(), cfg.auto_load == def.auto_load);
    put(&mut doc, "auto_save", cfg.auto_save.into(), cfg.auto_save == def.auto_save);
    put(&mut doc, "mouse_wheel_invert", cfg.mouse_wheel_invert.into(), cfg.mouse_wheel_invert == def.mouse_wheel_invert);
    put(&mut doc, "mouse", cfg.mouse.into(), cfg.mouse == def.mouse);
    put(&mut doc, "command_bar", cfg.command_bar.into(), cfg.command_bar == def.command_bar);
    put(&mut doc, "prompt_save_on_quit", cfg.prompt_save_on_quit.into(), cfg.prompt_save_on_quit == def.prompt_save_on_quit);
    put(&mut doc, "prompt_load_on_launch", cfg.prompt_load_on_launch.into(), cfg.prompt_load_on_launch == def.prompt_load_on_launch);
    let bg_str = match cfg.background_tidy {
        BackgroundTidy::Off => "off",
        BackgroundTidy::EveryRoom => "every_room",
        BackgroundTidy::OnOverlap => "on_overlap",
        BackgroundTidy::Debounced => "debounced",
    };
    put(&mut doc, "background_tidy", bg_str.into(), cfg.background_tidy == def.background_tidy);
    let aux_str = match cfg.aux_storage {
        AuxStorage::Ask => "ask",
        AuxStorage::Archive => "archive",
        AuxStorage::Global => "global",
    };
    put(&mut doc, "aux_storage", aux_str.into(), cfg.aux_storage == def.aux_storage);
    let v6_str = match cfg.v6_render {
        V6RenderMode::Hybrid => "hybrid",
        V6RenderMode::Raster => "raster",
        V6RenderMode::Frameless => "frameless",
    };
    put(&mut doc, "v6_render", v6_str.into(), cfg.v6_render == def.v6_render);
    let scale_val: toml_edit::Value = match cfg.glk_pixel_scale {
        GlkPixelScale::Auto => "auto".into(),
        GlkPixelScale::Fixed(n) => (n as i64).into(),
    };
    put(&mut doc, "glk_pixel_scale", scale_val, cfg.glk_pixel_scale == def.glk_pixel_scale);
    put(&mut doc, "v6_arrow_keys", cfg.v6_arrow_keys.into(), cfg.v6_arrow_keys == def.v6_arrow_keys);
    put(&mut doc, "show_room_numbers", cfg.show_room_numbers.into(), cfg.show_room_numbers == def.show_room_numbers);
    put(&mut doc, "show_status_bar", cfg.show_status_bar.into(), cfg.show_status_bar == def.show_status_bar);
    put(&mut doc, "hint_skip_screen_warning", cfg.hint_skip_screen_warning.into(), cfg.hint_skip_screen_warning == def.hint_skip_screen_warning);
    put(&mut doc, "watch_style", cfg.watch_style.into(), cfg.watch_style == def.watch_style);
    put(&mut doc, "record_turn_history", cfg.record_turn_history.into(), cfg.record_turn_history == def.record_turn_history);
    put(&mut doc, "honor_game_colours", cfg.honor_game_colours.into(), cfg.honor_game_colours == def.honor_game_colours);
    put(&mut doc, "honor_timed_input", cfg.honor_timed_input.into(), cfg.honor_timed_input == def.honor_timed_input);
    put(&mut doc, "enable_sound", cfg.enable_sound.into(), cfg.enable_sound == def.enable_sound);
    put(&mut doc, "volume", (cfg.volume as i64).into(), cfg.volume == def.volume);
    put(&mut doc, "undo_levels", (cfg.undo_levels as i64).into(), cfg.undo_levels == def.undo_levels);
    // A `--interpreter-number` override is for THIS run only: writing it here would
    // silently make it permanent the next time anything saves settings, so leave the
    // file's own value (or its absence) untouched in that case.
    if let Some(n) = cfg.interpreter_number.filter(|_| !cfg.interpreter_number_from_cli) {
        doc["interpreter_number"] = toml_edit::value(n as i64);
    }
    // Written only when the user pinned one: an absent key means "follow the
    // story pane" (ZMSD §8.4), and emitting the measured size would silently turn
    // this session's terminal into a permanent override. (SQ-0532/A-F1)
    match cfg.virtual_screen_cols {
        Some(n) => { doc["virtual_screen_cols"] = toml_edit::value(i64::from(n)); }
        None => { doc.remove("virtual_screen_cols"); }
    }
    match cfg.virtual_screen_rows {
        Some(n) => { doc["virtual_screen_rows"] = toml_edit::value(i64::from(n)); }
        None => { doc.remove("virtual_screen_rows"); }
    }
    put(&mut doc, "split_ratio", i64::from(cfg.split_ratio).into(), cfg.split_ratio == def.split_ratio);
    put(&mut doc, "verb_dock_pct", i64::from(cfg.verb_dock_pct).into(), cfg.verb_dock_pct == def.verb_dock_pct);
    put(&mut doc, "inv_dock_pct", i64::from(cfg.inv_dock_pct).into(), cfg.inv_dock_pct == def.inv_dock_pct);
    put(&mut doc, "text_margin_x", i64::from(cfg.text_margin_x).into(), cfg.text_margin_x == def.text_margin_x);
    put(&mut doc, "text_margin_y", i64::from(cfg.text_margin_y).into(), cfg.text_margin_y == def.text_margin_y);

    // style pointer — the only visual key written to config.toml. The actual
    // colors/symbols live in the style file ([colors]/[symbols] are no longer
    // emitted here). Visual override sections, if present, are preserved as-is.
    match &cfg.style {
        Some(s) => { doc["style"] = toml_edit::value(s.as_str()); }
        None => { doc.remove("style"); }
    }

    // [search] table — only materialized once something in it is non-default, so a
    // seeded config keeps the commented block instead of gaining an all-defaults table.
    if doc.contains_key("search")
        || cfg.search.start_backward != def.search.start_backward
        || cfg.search.key_back != def.search.key_back
        || cfg.search.key_forward != def.search.key_forward
    {
        let tbl = doc["search"].or_insert(toml_edit::table());
        put_in(tbl, "start_backward", cfg.search.start_backward.into(), cfg.search.start_backward == def.search.start_backward);
        put_in(tbl, "key_back", cfg.search.key_back.to_string().into(), cfg.search.key_back == def.search.key_back);
        put_in(tbl, "key_forward", cfg.search.key_forward.to_string().into(), cfg.search.key_forward == def.search.key_forward);
    }

    // [animation] table — same rule as [search].
    if doc.contains_key("animation")
        || cfg.animation.enabled != def.animation.enabled
        || cfg.animation.easing != def.animation.easing
        || cfg.animation.scroll_ms != def.animation.scroll_ms
    {
        let tbl = doc["animation"].or_insert(toml_edit::table());
        put_in(tbl, "enabled", cfg.animation.enabled.into(), cfg.animation.enabled == def.animation.enabled);
        put_in(tbl, "easing", crate::anim::easing_token(cfg.animation.easing).into(), cfg.animation.easing == def.animation.easing);
        put_in(tbl, "scroll_ms", (cfg.animation.scroll_ms as i64).into(), cfg.animation.scroll_ms == def.animation.scroll_ms);
    }

    std::fs::write(config_path, doc.to_string())
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
        assert!(!config_has_style_sections("style = \"s.toml\"\n"));
    }

    #[test]
    fn virtual_screen_defaults_to_following_the_pane() {
        let cfg = Config::default();
        // ZMSD §8.4 wants the REAL screen size in $20/$21, so an unset key means
        // "measure the story pane", not a fixed 80x24 guess. (SQ-0532/A-F1)
        assert_eq!(cfg.virtual_screen_cols, None);
        assert_eq!(cfg.virtual_screen_rows, None);
    }

    #[test]
    fn virtual_screen_parses_from_toml() {
        let cfg: Config = toml::from_str("virtual_screen_cols = 64\nvirtual_screen_rows = 20").unwrap();
        assert_eq!(cfg.virtual_screen_cols, Some(64));
        assert_eq!(cfg.virtual_screen_rows, Some(20));
    }

    #[test]
    fn pane_size_pcts_default_and_parse() {
        let d = Config::default();
        assert_eq!(d.split_ratio, 50);
        assert_eq!(d.verb_dock_pct, 32);
        assert_eq!(d.inv_dock_pct, 33);

        let cfg: Config = toml::from_str("split_ratio = 70\nverb_dock_pct = 40\ninv_dock_pct = 25\n").unwrap();
        assert_eq!(cfg.split_ratio, 70);
        assert_eq!(cfg.verb_dock_pct, 40);
        assert_eq!(cfg.inv_dock_pct, 25);
    }

    #[test]
    fn config_show_room_numbers_default_false_and_round_trips() {
        assert!(!Config::default().show_room_numbers);
        let cfg: Config = toml::from_str("show_room_numbers = true\n").unwrap();
        assert!(cfg.show_room_numbers);
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
    fn default_story_dir_defaults_none_parses_and_round_trips() {
        // Absent by default.
        assert!(Config::default().default_story_dir.is_none());
        let empty: Config = toml::from_str("").unwrap();
        assert!(empty.default_story_dir.is_none());
        // Parsed from the file when present.
        let cfg: Config = toml::from_str(r#"default_story_dir = "/tmp/stories""#).unwrap();
        assert_eq!(cfg.default_story_dir, Some(PathBuf::from("/tmp/stories")));
        // write_config persists it, and a Some value survives the round trip.
        let dir = std::env::temp_dir().join("babelmap-dsd-test");
        let _ = std::fs::remove_dir_all(&dir);
        write_config(&dir, &cfg).unwrap();
        let written = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&written).unwrap();
        assert_eq!(back.default_story_dir, Some(PathBuf::from("/tmp/stories")));
        // None removes the key rather than writing an empty value.
        let mut none_cfg = cfg.clone();
        none_cfg.default_story_dir = None;
        write_config(&dir, &none_cfg).unwrap();
        let doc: toml_edit::DocumentMut =
            std::fs::read_to_string(dir.join("config.toml")).unwrap().parse().unwrap();
        assert!(doc.get("default_story_dir").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_override_beats_file() {
        let cfg_path = write_temp_config("cli_override", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: Some(PathBuf::from("/tmp/from-cli")),
            data_dir: None,
            config: Some(cfg_path),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };

        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-cli"));
    }

    #[test]
    fn missing_config_file_resolves_to_defaults() {
        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir.file_name().unwrap(), ".babelmap");
    }

    #[test]
    fn file_value_beats_default_when_no_cli_override() {
        let cfg_path = write_temp_config("file_beats_default", r#"user_dir = "/tmp/from-file""#);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-file"));
    }

    #[test]
    fn stale_use_default_map_key_is_ignored() {
        let cfg: crate::config::Config = toml::from_str("use_default_map = true").unwrap();
        let _ = cfg; // unknown key ignored, no panic
    }

    #[test]
    fn keymap_config_parses_context_sections() {
        let toml = r#"
[keymap]
use_defaults = false
[keymap.global]
"ctrl+s" = "save-state"
[keymap.map]
"left" = "pan-map -1 0"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(!cfg.keymap.use_defaults);
        assert_eq!(cfg.keymap.global.get("ctrl+s").map(String::as_str), Some("save-state"));
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

    /// What advent.blb's toolbar needs (SQ-0593). `Auto` reports a cell of exactly
    /// the reference height at every font size, so the game's fixed 36px request always
    /// buys the same 3 rows and its artwork scales WITH the text.
    #[test]
    fn glk_pixel_scale_auto_normalises_every_cell_to_the_reference() {
        use GlkPixelScale::*;
        // A conventional 1x cell is reported unchanged — auto is a no-op there.
        assert_eq!(Auto.apply((7, 14)), (7, 14));
        // A 2x-scaled cell reports the same pixel space as the 1x one.
        assert_eq!(Auto.apply((14, 28)), (7, 14), "a 2x display sees what 1x sees");
        // ...and so does every other scale, including fractional ones. This is the
        // whole point of dropping the integer divisor: `round(cell/14)` sent 21px
        // (a 1.5x-scaled 14px cell, the Windows/GNOME default) to a divisor of 2,
        // which doubled the toolbar against its 20px neighbour.
        assert_eq!(Auto.apply((10, 21)), (7, 14), "1.5x — the cliff the divisor rule had");
        assert_eq!(Auto.apply((11, 20)), (8, 14), "and its neighbour lands beside it");
        assert_eq!(Auto.apply((21, 42)), (7, 14), "3x");
        // A SMALL cell is normalised up rather than left alone, which is the change
        // in behaviour: artwork stays proportional to the text instead of holding a
        // fixed pixel size on a tiny font.
        assert_eq!(Auto.apply((4, 8)), (7, 14));
        // Aspect ratio is preserved, not assumed 1:2.
        assert_eq!(Auto.apply((10, 20)), (7, 14));
        assert_eq!(Auto.apply((20, 20)), (14, 14), "a square cell stays square");
        assert_eq!(Auto.apply((0, 0)), (1, 14), "nonsense input still yields usable pixels");

        // Fixed is a plain divisor, and 1 restores the terminal's own cell.
        assert_eq!(Fixed(1).apply((14, 28)), (14, 28), "the author's literal pixels");
        assert_eq!(Fixed(2).apply((14, 28)), (7, 14));
        assert_eq!(Fixed(0).apply((14, 28)), (14, 28), "0 must not divide by zero");
        assert_eq!(Fixed(99).apply((14, 28)), (1, 1), "never reports a zero-size cell");
    }

    /// The toolbar row count that falls out of it: advent asks for 36px, and the point
    /// of normalising is that the answer stops depending on the font.
    #[test]
    fn glk_pixel_scale_auto_gives_the_same_row_count_at_every_font_size() {
        for cell_h in [8u32, 10, 12, 14, 17, 20, 21, 24, 28, 34, 42] {
            let (_, seen) = GlkPixelScale::Auto.apply((cell_h / 2, cell_h));
            let rows = 36u32.div_ceil(seen);
            assert_eq!(
                rows, 3,
                "a {cell_h}px cell must still give advent's 36px toolbar 3 rows (saw {seen}px)"
            );
        }
    }

    #[test]
    fn glk_pixel_scale_defaults_to_auto_and_round_trips() {
        assert_eq!(Config::default().glk_pixel_scale, GlkPixelScale::Auto);
        let c: Config = toml::from_str("glk_pixel_scale = 2").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Fixed(2));
        let c: Config = toml::from_str("glk_pixel_scale = \"auto\"").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Auto);
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.glk_pixel_scale, GlkPixelScale::Auto, "absent is auto");
    }

    #[test]
    fn v6_render_defaults_to_hybrid() {
        assert_eq!(Config::default().v6_render, V6RenderMode::Hybrid);
    }

    #[test]
    fn v6_render_parses_raster_from_toml() {
        let c: Config = toml::from_str("v6_render = \"raster\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Raster);
    }

    #[test]
    fn v6_render_absent_is_hybrid() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Hybrid);
    }

    #[test]
    fn v6_render_parses_frameless_from_toml() {
        let c: Config = toml::from_str("v6_render = \"frameless\"").unwrap();
        assert_eq!(c.v6_render, V6RenderMode::Frameless);
    }

    #[test]
    fn v6_render_frameless_round_trips_through_writer() {
        let dir = std::env::temp_dir().join(format!("babelmap-v6-frameless-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::default();
        cfg.v6_render = V6RenderMode::Frameless;
        write_config(&dir, &cfg).unwrap();
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.v6_render, V6RenderMode::Frameless, "frameless must survive save→load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_config_round_trips_scalars_and_preserves_keymap() {
        let dir = std::env::temp_dir().join(format!("babelmap_write_config_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Write initial config with a [keymap] section and a comment.
        let initial = "# babelmap config\n[keymap]\nzoom_in = \"z\"\n";
        std::fs::write(dir.join("config.toml"), initial).unwrap();

        let cfg = Config {
            version: CONFIG_SCHEMA_VERSION,
            user_dir: dir.clone(),
            default_story_dir: None,
            auto_load: false,
            auto_save: true,
            mouse_wheel_invert: false,
            mouse: true,
            command_bar: false,
            prompt_save_on_quit: true,
            prompt_load_on_launch: true,
            record_turn_history: false,
            hint_skip_screen_warning: true,
            background_tidy: BackgroundTidy::OnOverlap,
            aux_storage: AuxStorage::Ask,
            v6_render: V6RenderMode::Hybrid,
            glk_pixel_scale: GlkPixelScale::Auto,
            v6_arrow_keys: true,
            keymap: KeymapConfig::default(),
            hotkeys: HotkeysConfig::default(),
            style: Some("neon".into()),
            watch_style: false,
            undo_levels: 16,
            command_prefix: '/',
            show_room_numbers: false,
            show_status_bar: true,
            honor_game_colours: true,
            honor_timed_input: true,
            config_file: default_config_file(),
            config_error: None,
            interpreter_number: None,
            interpreter_number_from_cli: false,
            enable_sound: true,
            volume: 100,
            search: SearchConfig::default(),
            virtual_screen_cols: None,
            virtual_screen_rows: None,
            split_ratio: 70,
            verb_dock_pct: 40,
            inv_dock_pct: 25,
            text_margin_x: 0,
            text_margin_y: 0,
            animation: AnimationConfig::default(),
            acceleration: true,
            image_protocol: ImageProtocol::Auto,
            images: true,
            trace: crate::trace::TraceSections::default(),
        };
        write_config(&dir, &cfg).unwrap();

        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();

        // Scalars are set.
        assert_eq!(doc["auto_load"].as_bool(), Some(false));
        assert_eq!(doc["auto_save"].as_bool(), Some(true));
        assert_eq!(doc["background_tidy"].as_str(), Some("on_overlap"));
        assert_eq!(doc["split_ratio"].as_integer(), Some(70));
        assert_eq!(doc["verb_dock_pct"].as_integer(), Some(40));
        assert_eq!(doc["inv_dock_pct"].as_integer(), Some(25));
        // SQ-0573: `mouse` is at its DEFAULT and the pre-existing file did not carry
        // it, so it is deliberately not written — a default belongs in the commented
        // template, not as a live key. `user_dir` here is the test's temp dir, so it
        // differs from the default and IS written.
        assert!(doc.get("mouse").is_none(), "a default absent from the file stays absent");
        assert_eq!(doc["user_dir"].as_str(), Some(dir.to_string_lossy().as_ref()));
        // Style pointer is written; visual sections are NOT.
        assert_eq!(doc["style"].as_str(), Some("neon"));
        assert!(!content.contains("[colors]"));
        assert!(!content.contains("[symbols]"));
        // Keymap is preserved.
        assert_eq!(doc["keymap"]["zoom_in"].as_str(), Some("z"));
        // Comment is in the raw text.
        assert!(content.contains("# babelmap config"), "comment must be preserved");

        // SQ-0573, the other half: a key the file ALREADY has is always rewritten, even
        // when it now holds the default — otherwise flipping a setting back to its
        // default would leave the old value in the file and silently revert on the next
        // launch. Nothing is removed, so a comment the user wrote above their own key
        // (toml_edit attaches it to that key) survives too.
        let with_key = format!("# mine\nauto_load = false\n{initial}");
        std::fs::write(dir.join("config.toml"), &with_key).unwrap();
        let mut back_to_default = cfg.clone();
        back_to_default.auto_load = true; // the default
        write_config(&dir, &back_to_default).unwrap();
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["auto_load"].as_bool(), Some(true), "a present key is updated to its default");
        assert!(content.contains("# mine"), "the user's own comment above it survives");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_config_creates_file_and_dir_when_missing() {
        // Settings-save must create config.toml (and its parent) from scratch.
        let dir = std::env::temp_dir()
            .join(format!("babelmap_write_config_new_{}", std::process::id()))
            .join("nested");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.exists());
        let mut cfg = Config::default();
        cfg.user_dir = dir.clone();
        write_config(&dir, &cfg).unwrap();
        assert!(dir.join("config.toml").exists(), "config.toml must be created when missing");
        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(!content.contains("[colors]"), "config.toml must not carry style sections");
        assert!(!content.contains("[symbols]"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn write_config_persists_panel_editable_scalars_round_trip() {
        // Regression: undo_levels / watch_style / record_turn_history /
        // hint_skip_screen_warning are settings-panel-editable but were absent
        // from write_config, so a saved edit reverted to the default on restart.
        // Round-trip NON-default values through the writer and a fresh parse.
        let dir = std::env::temp_dir().join(format!("babelmap-cfg-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.undo_levels = 3; // default 16
        cfg.watch_style = true; // default false
        cfg.record_turn_history = true; // default false
        cfg.hint_skip_screen_warning = false; // default true
        write_config(&dir, &cfg).unwrap();

        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.undo_levels, 3, "undo_levels must survive save→load");
        assert!(parsed.watch_style, "watch_style must survive save→load");
        assert!(parsed.record_turn_history, "record_turn_history must survive save→load");
        assert!(!parsed.hint_skip_screen_warning, "hint_skip_screen_warning must survive save→load");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_reads_style_pointer() {
        let cfg: Config = toml::from_str("style = \"neon\"\n").unwrap();
        assert_eq!(cfg.style.as_deref(), Some("neon"));
    }

    #[test]
    fn mouse_capture_defaults_off_and_opts_in_from_file() {
        // Absent from the file → mouse capture stays off (the responsive default).
        let default: Config = toml::from_str("").unwrap();
        assert!(default.mouse, "mouse capture defaults on");
        // Explicit opt-in is honored.
        let on: Config = toml::from_str("mouse = true\n").unwrap();
        assert!(on.mouse, "mouse = true must enable capture");
    }

    #[test]
    fn command_bar_defaults_off_and_opts_in_from_file() {
        // Absent from the file → command bar stays off (inline prompt is the default).
        let default: Config = toml::from_str("").unwrap();
        assert!(!default.command_bar, "command_bar must default off");
        // Explicit opt-in is honored.
        let on: Config = toml::from_str("command_bar = true\n").unwrap();
        assert!(on.command_bar, "command_bar = true must enable the command bar");
    }

    #[test]
    fn command_bar_round_trips_through_toml() {
        let dir = std::env::temp_dir().join(format!("babelmap_command_bar_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut cfg = Config::default();
        cfg.user_dir = dir.clone();
        cfg.command_bar = true;
        write_config(&dir, &cfg).unwrap();

        let content = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = content.parse().unwrap();
        assert_eq!(doc["command_bar"].as_bool(), Some(true));

        let reparsed: Config = toml::from_str(&content).unwrap();
        assert!(reparsed.command_bar, "command_bar = true must round-trip");

        let _ = std::fs::remove_dir_all(&dir);
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
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: true,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let cfg = resolve(&cli);
        assert!(!cfg.acceleration);
    }

    #[test]
    fn no_sound_flag_disables_enable_sound() {
        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        // Absent flag: sound stays on (config default).
        assert!(resolve(&base).enable_sound);
        // Flag present: sound forced off for this run.
        let muted = Cli { no_sound: true, ..base };
        assert!(!resolve(&muted).enable_sound);
    }

    #[test]
    fn v6_arrow_keys_persists_from_config_file() {
        // Config-only setting (the --no-v6-arrows CLI flag was retired): a
        // persisted `v6_arrow_keys = false` must survive resolve; absent, the
        // default keeps arrows forwarded.
        let dir = std::env::temp_dir().join(format!("bm-v6arrows-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "v6_arrow_keys = false\n").unwrap();
        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        assert!(!resolve(&base).v6_arrow_keys, "persisted false must hold");
        let absent = Cli { config: Some(dir.join("missing.toml")), ..base };
        assert!(resolve(&absent).v6_arrow_keys, "default keeps arrows forwarded");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_parses_trace_flag() {
        let mut cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        cli.trace = Some("screen,map".to_string());
        let cfg = resolve(&cli);
        assert!(cfg.trace.screen && cfg.trace.map && !cfg.trace.hostio);
    }

    #[test]
    fn images_defaults_true_and_no_images_disables() {
        assert!(Config::default().images);

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(PathBuf::from("/nonexistent/path/config.toml")),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: true,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let cfg = resolve(&cli);
        assert!(!cfg.images);
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

    /// SQ-0574: `--user-dir` must move the config READ as well as the writes. It used
    /// to be ignored by `config_path`, so `babelmap --user-dir /tmp/x` seeded and saved
    /// `/tmp/x/config.toml` while still loading `~/.babelmap/config.toml` — every
    /// setting it wrote was silently discarded on the next launch.
    #[test]
    fn user_dir_moves_the_config_file_and_a_save_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-userdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: Some(dir.clone()),
            data_dir: None,
            config: None,
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        // The read path follows --user-dir, and the resolved config remembers it.
        assert_eq!(config_path(&cli), dir.join("config.toml"));
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.config_file, dir.join("config.toml"));

        // A save lands there — not in the default home — and loads back.
        cfg.volume = 42;
        write_config_file(&cfg).unwrap();
        assert!(dir.join("config.toml").is_file(), "the save went to the --user-dir");
        assert_eq!(resolve(&cli).volume, 42, "and the next launch reads it back");

        // --config still wins over --user-dir for the file's location.
        let elsewhere = dir.join("other.toml");
        std::fs::write(&elsewhere, "volume = 7\n").unwrap();
        let pinned = Cli { config: Some(elsewhere.clone()), ..cli };
        assert_eq!(config_path(&pinned), elsewhere);
        let pinned_cfg = resolve(&pinned);
        assert_eq!(pinned_cfg.volume, 7);
        assert_eq!(pinned_cfg.config_file, elsewhere);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `user_dir` KEY inside the file names the data root; it must NOT redirect
    /// where the file itself is saved, or a save would land somewhere `resolve` never
    /// reads (the second half of SQ-0574, and the reason `write_config_file` exists).
    #[test]
    fn a_user_dir_key_in_the_file_does_not_move_the_file() {
        let home = std::env::temp_dir().join(format!("bm-udkey-{}", std::process::id()));
        let data = home.join("data-root");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // A TOML *literal* string (single quotes) — no escape processing. A basic
        // double-quoted string would read the `\U` of a Windows `C:\Users\…` temp
        // path as TOML's 8-hex-digit unicode escape and fail to parse the file at
        // all, and `resolve` silently falls back to defaults on a parse error.
        std::fs::write(home.join("config.toml"), format!("user_dir = '{}'\n", data.display())).unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(home.join("config.toml")),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let mut cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, data, "the key still names the data root");
        assert_eq!(cfg.config_file, home.join("config.toml"), "but not the config's own location");

        cfg.volume = 33;
        write_config_file(&cfg).unwrap();
        assert!(!data.join("config.toml").exists(), "nothing is written into the data root");
        assert_eq!(resolve(&cli).volume, 33, "the save is where resolve reads");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// SQ-0580: a config file that doesn't parse costs the user every setting in it —
    /// TOML is one document, so there is no partial load. That much is unavoidable;
    /// doing it in silence is not. `resolve` must report the parse error so startup can
    /// tell the user which line broke instead of quietly running on defaults.
    #[test]
    fn a_malformed_config_is_reported_rather_than_swallowed() {
        let dir = std::env::temp_dir().join(format!("bm-badcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // `volume` would take effect if the file parsed; the unterminated string on the
        // next line means none of it does.
        std::fs::write(&path, "volume = 42\nstyle = \"neon\n").unwrap();

        let cli = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(path.clone()),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        let cfg = resolve(&cli);
        assert!(cfg.config_error.is_some(), "the parse failure is reported");
        assert_eq!(cfg.volume, Config::default().volume, "and nothing from the file loaded");

        // A file that parses leaves the field clear — the error must not be sticky.
        std::fs::write(&path, "volume = 42\n").unwrap();
        let good = resolve(&cli);
        assert_eq!(good.config_error, None, "a valid file reports no error");
        assert_eq!(good.volume, 42);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SQ-0580, the destructive half: `write_config_at` parsed the existing file into a
    /// toml_edit doc and fell back to an EMPTY doc when that failed, so the first
    /// settings save after a typo replaced the user's whole config — keys, comments and
    /// all — with a handful of defaults. Refuse the write and keep the file byte-exact.
    #[test]
    fn a_save_refuses_to_clobber_a_malformed_config() {
        let dir = std::env::temp_dir().join(format!("bm-badsave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let original = "# my carefully commented config\nvolume = 42\nstyle = \"neon\n";
        std::fs::write(&path, original).unwrap();

        let mut cfg = Config { config_file: path.clone(), ..Config::default() };
        cfg.volume = 33;
        let err = write_config_file(&cfg).expect_err("a malformed file is not overwritten");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("not valid TOML"), "and it says why: {err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "the user's file survives untouched, comments included",
        );

        // Once the file is well-formed again, saving works normally.
        std::fs::write(&path, "# my carefully commented config\nvolume = 42\n").unwrap();
        write_config_file(&cfg).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# my carefully commented config"), "comments preserved: {after}");
        assert!(after.contains("volume = 33"), "and the new value landed: {after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `--interpreter-number N` overrides the config file for ONE run, and must not
    /// be written back: a later settings save would otherwise make a throwaway
    /// experiment permanent (and pin one machine for every story, defeating the
    /// per-version auto rule). The header values themselves are ZMSD §11.1.3's table
    /// (1 DECSystem-20 … 6 IBM PC … 11 Tandy Color).
    #[test]
    fn interpreter_number_cli_overrides_file_without_persisting() {
        let dir = std::env::temp_dir().join(format!("bm-interp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(&cfg_path, "interpreter_number = 4\n").unwrap();

        let base = Cli {
            story: Some(PathBuf::from("foo.z5")),
            user_dir: None,
            data_dir: None,
            config: Some(cfg_path.clone()),
            no_accel: false,
            no_sound: false,
            image_protocol: ImageProtocol::Auto,
            no_images: false,
            interpreter_number: None,
            trace: None,
            debug: false,
        };
        // No flag: the file's Amiga (4) stands, and it is provenance-clean.
        let from_file = resolve(&base);
        assert_eq!(from_file.interpreter_number, Some(4), "the file's value loads");
        assert!(!from_file.interpreter_number_from_cli);

        // Flag present: IBM PC (6) wins for this run, marked as CLI-sourced.
        let overridden = resolve(&Cli { interpreter_number: Some(6), ..base });
        assert_eq!(overridden.interpreter_number, Some(6), "the CLI beats the file");
        assert!(overridden.interpreter_number_from_cli);

        // Writing settings must leave the FILE on 4, not bake in the run's 6.
        write_config(&dir, &overridden).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        let reread: Config = toml::from_str(&back).unwrap();
        assert_eq!(
            reread.interpreter_number,
            Some(4),
            "a one-run --interpreter-number must not be persisted; file now: {back}"
        );

        // A value that came from the file DOES persist (the settings panel path).
        write_config(&dir, &from_file).unwrap();
        let back = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(toml::from_str::<Config>(&back).unwrap().interpreter_number, Some(4));
        let _ = std::fs::remove_dir_all(&dir);
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
