use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;

// ── Keymap config ─────────────────────────────────────────────────────────────

/// The [keymap] section of config.toml.  Maps snake_case command names to
/// comma-separated key-spec strings.  Absent commands keep their defaults.
#[derive(Debug, Default, Deserialize)]
pub struct KeymapConfig {
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

// ── Symbol config ─────────────────────────────────────────────────────────────

fn default_box_style() -> String { "rounded".into() }
fn default_arrow_set() -> String { "filled".into() }
fn default_portal_icons() -> String { "ascii".into() }
fn default_path_style() -> String { "light".into() }

/// The [symbols] section of config.toml.  All fields default to the preset
/// names that match today's hardcoded glyphs, so an absent section is a no-op.
#[derive(Debug, Deserialize)]
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
            overrides: BTreeMap::new(),
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
}

// ── Config ────────────────────────────────────────────────────────────────────

fn default_user_dir() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".babelmap")
}

/// User preferences loaded from TOML.  Every field has a default so a missing
/// config file (or a file with only some fields) is always valid.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Root directory for babelmap data (maps, saves, exports).
    /// Sub-directories: maps/ — where per-story map files live.
    #[serde(default = "default_user_dir")]
    pub user_dir: PathBuf,
    /// Map symbol configuration: presets + per-glyph overrides.
    #[serde(default)]
    pub symbols: SymbolConfig,
    /// Keymap overrides: command_name → key-spec string(s).
    #[serde(default)]
    pub keymap: KeymapConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            user_dir: default_user_dir(),
            symbols: SymbolConfig::default(),
            keymap: KeymapConfig::default(),
        }
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
    let config_path = match &cli.config {
        Some(p) => p.clone(),
        None => {
            // Default location: derive from the default user_dir, not from the
            // CLI user_dir override, so the file location is stable.
            default_user_dir().join("config.toml")
        }
    };

    // Start from defaults.
    let mut cfg = Config::default();

    // Layer in the config file if it exists.
    if let Ok(text) = std::fs::read_to_string(&config_path) {
        if let Ok(from_file) = toml::from_str::<Config>(&text) {
            cfg.user_dir = from_file.user_dir;
            cfg.symbols = from_file.symbols;
            cfg.keymap = from_file.keymap;
        }
        // If the file exists but is malformed, silently keep defaults.
        // Production code could warn here; for now, YAGNI.
    }

    // CLI overrides beat the file.
    if let Some(dir) = &cli.user_dir {
        cfg.user_dir = dir.clone();
    }

    cfg
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
        };
        let cfg = resolve(&cli);
        assert_eq!(cfg.user_dir, PathBuf::from("/tmp/from-file"));
    }
}
