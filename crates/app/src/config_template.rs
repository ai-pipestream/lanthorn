//! Commented `config.toml` template (SQ-0573).
//!
//! Mirrors what `style.toml` already does (see [`crate::theme::template`]): emit
//! EVERY setting, each with a short comment, and leave the line commented out when
//! it holds the default — so the file is a browsable catalogue of what babelmap can
//! be told to do, and uncommenting a line is a no-op until you change its value.
//!
//! Before this, [`crate::config::write_config`] only wrote keys it had a value for,
//! so a setting nobody had touched simply did not appear. `interpreter_number` was
//! the case that surfaced it: a real, useful knob that was invisible unless you read
//! the source.
//!
//! Two kinds of line, distinguished because only one of them is safe to blanket
//! uncomment:
//!
//! * [`Line::Default`] — the value shown IS the default. Uncommenting reproduces
//!   current behaviour exactly (the `template_default_lines_are_really_the_defaults`
//!   test proves it key by key).
//! * [`Line::Example`] — the default cannot be written down (`None`, or a computed
//!   path), so the value is an illustration. Uncommenting it CHANGES behaviour.
//!
//! Section headers (`[search]`, `[animation]`, `[keymap]`) are emitted UNCOMMENTED,
//! matching style.toml, and the schema `version` stamp is a live key: everything that
//! is actually a *setting* stays commented.
//!
//! Scope: this is the GLOBAL config only (`<user_dir>/config.toml`). A game's own
//! save directory can hold a second, unrelated `config.toml` — the sparse per-game
//! override sidecar in [`crate::styles`], carrying at most `honor_game_colours`,
//! `borderless_windows` and `show_map` as bare uncommented lines, and deleted when
//! empty. It is deliberately NOT templated: there, an absent key means "inherit the
//! global value", so seeding defaults into it would turn every one of them into a
//! per-game override. [`auto_seed`] is only ever called with the user dir.
//!
//! [`auto_seed`] writes the template on first run and never overwrites an existing
//! file, exactly like the style seed. `write_config` still owns runtime edits and
//! stays format-preserving, so a seeded file keeps its comments as settings change.

#[cfg(test)]
use crate::config::Config;
use crate::config::CONFIG_SCHEMA_VERSION;

/// Whether a template line's value reproduces the default or merely illustrates the
/// shape. See the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// The value shown is the default; uncommenting changes nothing.
    Default,
    /// The default is unrepresentable (`None`/computed); the value is an example and
    /// uncommenting it changes behaviour.
    Example,
}

/// One documented setting: TOML key, the literal to show, whether that literal is
/// the real default, and the comment lines above it (no leading `#`).
struct Row {
    key: &'static str,
    value: &'static str,
    line: Line,
    doc: &'static [&'static str],
}

const fn d(key: &'static str, value: &'static str, doc: &'static [&'static str]) -> Row {
    Row { key, value, line: Line::Default, doc }
}
const fn ex(key: &'static str, value: &'static str, doc: &'static [&'static str]) -> Row {
    Row { key, value, line: Line::Example, doc }
}

/// A group of settings under a banner comment. `table` names the TOML table the rows
/// belong to (`Some("[search]")`), or `None` for top-level keys.
struct Group {
    banner: &'static str,
    table: Option<&'static str>,
    rows: &'static [Row],
}

const STARTUP: &[Row] = &[
    ex(
        "user_dir",
        "\"~/.babelmap\"",
        &["Root directory for babelmap data (maps/, saves/, style.toml).", "Default: ~/.babelmap."],
    ),
    ex(
        "default_story_dir",
        "\"~/games/if\"",
        &[
            "Directory (or single story file) opened when babelmap is launched with",
            "no path argument. Unset by default, so a path is required.",
        ],
    ),
    ex(
        "style",
        "\"style.toml\"",
        &[
            "Style-file pointer: a built-in name or a file path. Unset uses",
            "<user_dir>/style.toml when present, else the built-in theme.",
        ],
    ),
    d("watch_style", "false", &["Watch the resolved style file and live-reload it on change."]),
];

const SAVES: &[Row] = &[
    d(
        "auto_load",
        "true",
        &[
            "Restore game state from the archive on startup so play resumes where it",
            "left off. Set false to start fresh while keeping the accumulated map.",
        ],
    ),
    d(
        "auto_save",
        "false",
        &["Save the archive after every turn, on top of the exit-save and Ctrl+S."],
    ),
    d("prompt_save_on_quit", "true", &["When auto_save is off, offer to save on quit."]),
    d("prompt_load_on_launch", "true", &["When auto_load is off, offer to resume a save found on launch."]),
    d(
        "record_turn_history",
        "false",
        &[
            "Record a per-turn rewind/replay history into the archive. Opt-in: it grows",
            "the archive and keeps per-turn blobs in memory.",
        ],
    ),
    d("undo_levels", "16", &["Undo depth: retained in-memory snapshots. 0 disables undo."]),
    d(
        "aux_storage",
        "\"ask\"",
        &["Where v5 auxiliary save data goes: \"ask\" (default), \"archive\", \"global\"."],
    ),
];

const INTERFACE: &[Row] = &[
    d(
        "mouse",
        "true",
        &[
            "Capture the mouse: click-to-select in the browser and map, wheel scrolling,",
            "and Glk mouse input for games that ask for it.",
        ],
    ),
    d("mouse_wheel_invert", "false", &["Invert wheel direction, for terminals reporting \"natural\" scrolling."]),
    d("command_bar", "false", &["Type into a persistent command bar instead of the inline story prompt."]),
    d("command_prefix", "\"/\"", &["The character that routes a line to a slash command."]),
    d("show_status_bar", "true", &["Show the status/score bar across the top of the story pane."]),
    d("show_room_numbers", "false", &["Show room numbers (#id) inside Boxes-zoom room boxes."]),
    d("split_ratio", "50", &["The story pane's share of the story/map split, as a percentage."]),
    d("verb_dock_pct", "32", &["Verb dock width, as a percentage of screen width."]),
    d("inv_dock_pct", "33", &["Inventory dock height cap, as a percentage of screen height."]),
    d(
        "text_margin_x",
        "0",
        &["Blank columns reserved inside each side of the transcript window."],
    ),
    d("text_margin_y", "0", &["Blank rows reserved above and below the transcript text."]),
    d(
        "background_tidy",
        "\"every_room\"",
        &[
            "Automatic map re-tidy when new rooms appear: \"off\", \"every_room\"",
            "(default), \"on_overlap\", \"debounced\".",
        ],
    ),
    d(
        "hint_skip_screen_warning",
        "true",
        &[
            "Auto-skip the InvisiClues \"your screen is only N characters wide…\" banner",
            "and land on the topic menu. Set false to see and dismiss it yourself.",
        ],
    ),
];

const INTERPRETER: &[Row] = &[
    d(
        "honor_game_colours",
        "true",
        &["Honour game-set colours. Set false to use only the configured theme."],
    ),
    d(
        "honor_timed_input",
        "true",
        &["Honour the Z-machine's timed input. Set false to treat every read as untimed."],
    ),
    ex(
        "interpreter_number",
        "6",
        &[
            "Interpreter number advertised in header byte 0x1E. Games branch on it:",
            "Beyond Zork picks character graphics over colour on IBM PC, and several v6",
            "story files were built for one specific machine. ZMSD §11.1.3's values:",
            "",
            "   1  DECSystem-20      7  Commodore 128",
            "   2  Apple IIe         8  Commodore 64",
            "   3  Macintosh         9  Apple IIc",
            "   4  Amiga            10  Apple IIgs",
            "   5  Atari ST         11  Tandy Color",
            "   6  IBM PC",
            "",
            "Unset auto-selects: 6 (IBM PC) for v6, else 1 (DECSystem-20). Override for",
            "a single run with `babelmap --interpreter-number N`.",
        ],
    ),
    d(
        "v6_render",
        "\"hybrid\"",
        &[
            "How v6 graphical games (Zork Zero, Arthur, Journey, Shogun) are drawn:",
            "  \"hybrid\"    — crisp terminal story inside a scaled pixel frame (default)",
            "  \"raster\"    — the whole pane as one pixel image",
            "  \"frameless\" — no frame: full-pane text with a status band, inline pictures",
        ],
    ),
    d(
        "v6_arrow_keys",
        "true",
        &[
            "Forward arrow keys to a v6 story as ZSCII 129-132. Set false to keep them",
            "for scrollback and map panning instead. v1-5 and Glulx always get them.",
        ],
    ),
    ex(
        "virtual_screen_cols",
        "80",
        &[
            "Override the screen size reported to the Z-machine in header bytes $21/$20.",
            "Unset (default) follows the story pane, which is what ZMSD §8.4 asks for.",
            "Set both only to pin a fixed virtual screen.",
        ],
    ),
    ex("virtual_screen_rows", "24", &["The height half of the same override; set it alongside the width."]),
];

const SOUND: &[Row] = &[
    d("enable_sound", "true", &["Play audio for sound_effect: bleeps and Blorb samples."]),
    d("volume", "100", &["Master volume 0-100, combined with the game's own per-sound scale."]),
];

const SEARCH: &[Row] = &[
    d("start_backward", "true", &["A new /search starts backward, from the most recent match."]),
    d("key_back", "\"n\"", &["Key that steps to an older match."]),
    d("key_forward", "\"N\"", &["Key that steps to a newer match."]),
];

const ANIMATION: &[Row] = &[
    d("enabled", "true", &["Master switch. False makes every animation instant."]),
    d(
        "easing",
        "\"ease-out\"",
        &["Easing curve: \"linear\", \"ease-in\", \"ease-out\", \"ease-in-out\"."],
    ),
    d("scroll_ms", "120", &["Smooth-scroll duration in milliseconds. 0 is instant."]),
];

const KEYMAP: &[Row] = &[d(
    "use_defaults",
    "true",
    &[
        "Keep the built-in bindings and layer your overrides on top. False starts",
        "from an empty keymap, so only what you bind below works.",
    ],
)];

const GROUPS: &[Group] = &[
    Group { banner: "Startup and files", table: None, rows: STARTUP },
    Group { banner: "Saving and undo", table: None, rows: SAVES },
    Group { banner: "Interface", table: None, rows: INTERFACE },
    Group { banner: "Interpreter behaviour", table: None, rows: INTERPRETER },
    Group { banner: "Sound", table: None, rows: SOUND },
    Group { banner: "Transcript search", table: Some("search"), rows: SEARCH },
    Group { banner: "Animation", table: Some("animation"), rows: ANIMATION },
    Group { banner: "Key bindings", table: Some("keymap"), rows: KEYMAP },
];

/// Free-form trailing blocks for the open-ended tables, which have no fixed set of
/// keys to enumerate: `[keymap.*]` maps any command name to any key spec, and
/// `[hotkeys]` holds an arbitrary list of groups. Shown as commented examples.
const TRAILER: &str = r#"
# Bind commands to keys. Each entry is command_name = "key-spec"; a value may list
# several specs separated by spaces. Names are the snake_case command names from the
# command registry (see /help).
#
# [keymap.global]
# quit = "ctrl+q"
# toggle_map = "f2 ctrl+m"
#
# [keymap.map]
# zoom_in = "+"
#
# [keymap.anim]
# animate_tidy = "ctrl+t"

# ── Hotkey dialog ─────────────────────────────────────────────────────────────
# prefix       — the key that opens the hotkey dialog
# direct       — commands always available without opening the dialog
# [[hotkeys.group]] — one block per group shown in the dialog
#
# [hotkeys]
# prefix = "ctrl+p"
# direct = ["toggle_map", "quit"]
#
# [[hotkeys.group]]
# title = "Map"
# commands = ["zoom_in", "zoom_out", "retidy"]
"#;

/// Render the whole commented `config.toml`.
///
/// Every line is commented, so the template parses as an EMPTY document and yields
/// [`Config::default()`] — writing it changes nothing until the user edits it.
pub fn commented_template() -> String {
    let mut out = String::new();
    out.push_str("# babelmap config.toml\n");
    out.push_str("#\n");
    out.push_str("# Every setting babelmap reads is listed here. Lines are commented out, and the\n");
    out.push_str("# value shown is the DEFAULT unless the comment says otherwise — so uncommenting\n");
    out.push_str("# a line as-is changes nothing. Edit the value to change behaviour.\n");
    out.push_str("#\n");
    out.push_str("# Colours, glyphs and borders are NOT here: they live in style.toml.\n");
    out.push_str("#\n");
    out.push_str("# PER-GAME overrides live in a second, separate config.toml inside that game's\n");
    out.push_str("# own save directory, and hold only these three keys:\n");
    out.push_str("#\n");
    out.push_str("#   honor_game_colours   borderless_windows   show_map\n");
    out.push_str("#\n");
    out.push_str("# That file is a sparse override layer, not a copy of this one: it carries only\n");
    out.push_str("# the keys that differ, bare and uncommented, and is deleted once nothing is\n");
    out.push_str("# overridden. An absent key there means \"inherit whatever this file says\", so\n");
    out.push_str("# do NOT paste this template into a game directory — every line you uncommented\n");
    out.push_str("# would become a per-game override pinning that value for that story.\n");
    out.push_str("#\n# `version` below is the schema stamp — babelmap manages it; leave it alone.\n");
    out.push_str("# It is also the anchor that keeps settings babelmap writes for you (from the\n");
    out.push_str("# settings screen, say) together at the top rather than scattered.\n");
    out.push_str(&format!("version = {CONFIG_SCHEMA_VERSION}\n"));

    for g in GROUPS {
        out.push_str(&format!("\n# ── {} {}\n", g.banner, "─".repeat(72usize.saturating_sub(g.banner.len() + 6))));
        if let Some(t) = g.table {
            // Real, UNCOMMENTED header — exactly as style.toml does it. It also gives
            // the document structure, so a key `write_config` adds lands in the root
            // table above the sections instead of scattering.
            out.push_str(&format!("[{t}]\n"));
        }
        for (i, row) in g.rows.iter().enumerate() {
            if i > 0 {
                out.push_str("#\n");
            }
            for line in row.doc {
                if line.is_empty() {
                    out.push_str("#\n");
                } else {
                    out.push_str(&format!("# {line}\n"));
                }
            }
            if row.line == Line::Example {
                out.push_str("# (example — the default is unset)\n");
            }
            out.push_str(&format!("# {} = {}\n", row.key, row.value));
        }
    }
    out.push_str(TRAILER);
    out
}

/// Write [`commented_template`] to `user_dir/config.toml` when that file does not
/// exist. NEVER overwrites: an existing config is the user's. Best-effort — a write
/// failure (read-only home) is swallowed so startup cannot crash on it.
pub fn auto_seed(user_dir: &std::path::Path) {
    let path = user_dir.join("config.toml");
    if path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(user_dir);
    let _ = std::fs::write(&path, commented_template());
}

/// A `Config`'s Debug string with the schema stamp normalized. An absent `version`
/// key deliberately reads as 0 ("written before versioning", see the field's docs)
/// while `Config::default()` carries the current stamp, so the stamp is the one
/// difference a commented template is EXPECTED to have.
#[cfg(test)]
fn shape(cfg: &Config) -> String {
    format!("{cfg:?}").replacen(&format!("version: {}", cfg.version), "version: <stamp>", 1)
}

/// Every documented row, for the tests below and for anyone auditing coverage.
#[cfg(test)]
fn all_rows() -> Vec<(&'static str, &'static str, Line)> {
    GROUPS.iter().flat_map(|g| g.rows.iter().map(|r| (r.key, r.value, r.line))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template must parse — and, with every line commented, must describe
    /// exactly the default config. This is what makes it safe to seed on first run.
    #[test]
    fn template_is_valid_toml_and_a_no_op_as_written() {
        let t = commented_template();
        let parsed: toml::Table = toml::from_str(&t).expect("the template parses as TOML");
        // Only the managed schema stamp and the (empty) section headers are live —
        // every actual setting is commented out.
        let mut live: Vec<&str> = parsed.keys().map(String::as_str).collect();
        live.sort_unstable();
        assert_eq!(live, ["animation", "keymap", "search", "version"], "live keys: {parsed:?}");
        for t in ["animation", "keymap", "search"] {
            assert!(
                parsed[t].as_table().is_some_and(|x| x.is_empty()),
                "section [{t}] is a bare header with every setting commented: {:?}",
                parsed[t]
            );
        }
        let cfg: Config = toml::from_str(&t).expect("and deserializes as a Config");
        assert_eq!(
            shape(&cfg),
            shape(&Config::default()),
            "the commented template must yield exactly the default config"
        );
    }

    /// Uncommenting a `Line::Default` row must reproduce the default it claims to
    /// document — otherwise the file lies about what babelmap does. Checked one key
    /// at a time so a failure names the offender.
    #[test]
    fn template_default_lines_are_really_the_defaults() {
        let base = shape(&Config::default());
        for (key, value, line) in all_rows() {
            if line != Line::Default {
                continue;
            }
            // Table rows need their header to land in the right section.
            let table = GROUPS
                .iter()
                .find(|g| g.rows.iter().any(|r| r.key == key))
                .and_then(|g| g.table);
            let doc = match table {
                Some(t) => format!("[{t}]\n{key} = {value}\n"),
                None => format!("{key} = {value}\n"),
            };
            let cfg: Config = toml::from_str(&doc)
                .unwrap_or_else(|e| panic!("template line for `{key}` is not valid TOML/type: {e}\n{doc}"));
            assert_eq!(
                shape(&cfg),
                base,
                "`{key} = {value}` is documented as the default but changes the config"
            );
        }
    }

    /// Anti-drift: every key `Config` loads from the file must appear in the
    /// template, or a new setting silently becomes undiscoverable again — the exact
    /// problem this module exists to fix. The persisted set is read straight out of
    /// `resolve`'s field-by-field merge, so adding a field there without documenting
    /// it here fails this test.
    #[test]
    fn every_persisted_setting_is_documented() {
        let src = include_str!("config.rs");
        let mut persisted: Vec<&str> = src
            .lines()
            .filter_map(|l| {
                let (key, _) = l.trim().strip_prefix("cfg.")?.split_once(" = from_file.")?;
                Some(key)
            })
            .collect();
        persisted.sort_unstable();
        persisted.dedup();
        assert!(persisted.len() > 30, "sanity: found the merge list ({} keys)", persisted.len());

        let documented: Vec<&str> = all_rows().iter().map(|(k, _, _)| *k).collect();
        let trailer = TRAILER;
        // `version` is managed by babelmap (stamped by write_config, never hand-set),
        // and the two open-ended tables are documented as commented example blocks in
        // the trailer rather than as enumerable keys.
        let exempt = ["version"];
        let missing: Vec<&str> = persisted
            .iter()
            .copied()
            .filter(|k| {
                !documented.contains(k)
                    && !exempt.contains(k)
                    && !trailer.contains(&format!("[{k}"))
                    && !GROUPS.iter().any(|g| g.table == Some(*k))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "these settings are loaded from config.toml but not documented in the template: {missing:?}"
        );
    }

    /// The end-to-end shape the reporter hit: seed a fresh config, then let something
    /// save settings (the story browser's "remember this directory?" prompt is enough)
    /// and confirm the file is still the annotated template rather than the old flat
    /// key list. `write_config` used to stamp all ~36 keys unconditionally, and because
    /// an all-comment file parses as trailing trivia they landed ABOVE the comments —
    /// so a brand-new config read as the old format at first glance.
    #[test]
    fn a_settings_save_keeps_the_seeded_template() {
        let dir = std::env::temp_dir().join(format!("bm-cfgsave-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        auto_seed(&dir);
        let path = dir.join("config.toml");
        let seeded = std::fs::read_to_string(&path).unwrap();

        // Exactly what the story-list prompt does: set default_story_dir and save.
        // `user_dir` deliberately left at its default: the real flow only sets it when
        // `--user-dir` was passed, and an unchanged one must not be persisted either.
        let mut cfg = Config::default();
        cfg.default_story_dir = Some(std::path::PathBuf::from("/games/if"));
        crate::config::write_config(&dir, &cfg).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();

        // The documentation survives intact, comment for comment.
        let comments = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).count();
        assert_eq!(comments(&after), comments(&seeded), "every comment line survives the save");
        assert!(after.contains("# ── Interpreter behaviour"), "the group banners survive");
        assert!(after.contains("#    6  IBM PC"), "the interpreter table survives");

        // And only the setting that actually changed went live.
        let parsed: toml::Table = toml::from_str(&after).unwrap();
        let mut live: Vec<&str> = parsed.keys().map(String::as_str).collect();
        live.sort_unstable();
        assert_eq!(
            live,
            ["animation", "default_story_dir", "keymap", "search", "version"],
            "only the changed setting joins the stamp and the section headers: {after}"
        );
        for t in ["animation", "keymap", "search"] {
            assert!(parsed[t].as_table().is_some_and(|x| x.is_empty()), "[{t}] stays a bare header");
        }

        // Re-reading it gives back the config that was saved.
        let reread: Config = toml::from_str(&after).unwrap();
        assert_eq!(reread.default_story_dir, Some(std::path::PathBuf::from("/games/if")));
        assert!(reread.auto_load, "an absent key still means its default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The seed never clobbers a real config.
    #[test]
    fn auto_seed_creates_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("bm-cfgseed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        auto_seed(&dir);
        let path = dir.join("config.toml");
        let seeded = std::fs::read_to_string(&path).expect("seeded");
        assert!(seeded.contains("interpreter_number"), "the template documents interpreter_number");

        std::fs::write(&path, "volume = 40\n").unwrap();
        auto_seed(&dir);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "volume = 40\n", "an existing config is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
