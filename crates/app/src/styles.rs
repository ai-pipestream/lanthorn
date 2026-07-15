//! Per-game style overrides: a `style.toml` (and non-style `config.toml`
//! sidecar) stored in the story's per-game save directory (`game_dir`,
//! `<data_base>/<story-key>.save/`), layered over the global style.toml. Keyed
//! by story filename, co-located with the story's saves/aux/glkvfs (SQ-0346).
//! See docs/superpowers/specs/2026-06-25-per-game-styles-design.md.

use std::path::{Path, PathBuf};

/// The per-game style file path: `<game_dir>/style.toml`.
pub fn per_game_style_path(game_dir: &Path) -> PathBuf {
    game_dir.join("style.toml")
}

/// The per-game NON-style config sidecar: `<game_dir>/config.toml`. Holds
/// per-game overrides that are not part of the style schema
/// (`honor_game_colours`, `borderless_windows`, `show_map`), kept separate from
/// `style.toml` so the style parser/writer stays a pure style document.
pub fn per_game_config_path(game_dir: &Path) -> PathBuf {
    game_dir.join("config.toml")
}

/// Read one boolean key from the per-game `config.toml`. `None` if the file is
/// absent/unparseable or the key is missing.
fn read_config_bool(game_dir: &Path, key: &str) -> Option<bool> {
    let text = std::fs::read_to_string(per_game_config_path(game_dir)).ok()?;
    text.parse::<toml::Value>().ok()?.get(key).and_then(|v| v.as_bool())
}

/// Read the per-game `honor_game_colours` override, if the user set one.
/// `None` = no override (fall back to garglk.ini, then the global config default).
pub fn read_per_game_honor(game_dir: &Path) -> Option<bool> {
    read_config_bool(game_dir, "honor_game_colours")
}

/// Read the per-game `borderless_windows` override, if the user set one. `None`
/// = no override (fall back to the default: honor the Glk border hint). When
/// `Some(true)`, all window splits abut with no reserved gutter (SQ-0341).
pub fn read_per_game_borderless(game_dir: &Path) -> Option<bool> {
    read_config_bool(game_dir, "borderless_windows")
}

/// Read the per-game `show_map` override, if the user set one. `None` = no
/// override (fall back to the default: the map panel is shown). When
/// `Some(false)` the map panel starts hidden for this story (SQ-0304).
pub fn read_per_game_show_map(game_dir: &Path) -> Option<bool> {
    read_config_bool(game_dir, "show_map")
}

/// Write the per-game `config.toml` with the given overrides, omitting a `None`
/// key and deleting the file entirely when all are `None`. Centralised so each
/// key's writer preserves the others' values (SQ-0341). Creates `game_dir` if
/// needed.
fn write_per_game_config(
    game_dir: &Path,
    honor: Option<bool>,
    borderless: Option<bool>,
    show_map: Option<bool>,
) -> std::io::Result<()> {
    let path = per_game_config_path(game_dir);
    let mut body = String::new();
    if let Some(v) = honor {
        body.push_str(&format!("honor_game_colours = {v}\n"));
    }
    if let Some(v) = borderless {
        body.push_str(&format!("borderless_windows = {v}\n"));
    }
    if let Some(v) = show_map {
        body.push_str(&format!("show_map = {v}\n"));
    }
    if body.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, body)
}

/// Persist (or clear) the per-game `honor_game_colours` override, preserving any
/// `borderless_windows` / `show_map` override in the same sidecar. `Some(v)`
/// writes it; `None` clears it (→ fall back to garglk.ini / the global default).
pub fn write_per_game_honor(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    write_per_game_config(game_dir, value, read_per_game_borderless(game_dir), read_per_game_show_map(game_dir))
}

/// Persist (or clear) the per-game `borderless_windows` override, preserving any
/// `honor_game_colours` / `show_map` override in the same sidecar (SQ-0341).
pub fn write_per_game_borderless(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    write_per_game_config(game_dir, read_per_game_honor(game_dir), value, read_per_game_show_map(game_dir))
}

/// Persist (or clear) the per-game `show_map` override, preserving any
/// `honor_game_colours` / `borderless_windows` override in the same sidecar
/// (SQ-0304).
pub fn write_per_game_show_map(game_dir: &Path, value: Option<bool>) -> std::io::Result<()> {
    write_per_game_config(game_dir, read_per_game_honor(game_dir), read_per_game_borderless(game_dir), value)
}

/// Write the live look self-contained to the current game's per-game style file
/// (`<game_dir>/style.toml`), creating `game_dir` if needed. Returns the path
/// written. Does NOT repoint `config.style`; the file is merged over the global
/// style.toml on the next reload.
pub fn save_per_game_style(
    game_dir: &Path,
    colors: &crate::colors::ColorScheme,
    symbols: &crate::symbols::SymbolSet,
) -> std::io::Result<PathBuf> {
    let path = per_game_style_path(game_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::style::write_style_full(&path, colors, symbols)?;
    Ok(path)
}

/// Create the per-game style file (and `game_dir`) if it does not exist,
/// seeded with a title/IFID header. Returns `(path, created)`; never overwrites an
/// existing file (`created = false`).
pub fn scaffold_per_game_style(game_dir: &Path, ifid: &str, title: &str) -> std::io::Result<(PathBuf, bool)> {
    let path = per_game_style_path(game_dir);
    if path.exists() {
        return Ok((path, false));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!(
        "# Per-game style override for: {title}\n\
         # IFID: {ifid}\n\
         # Layers on the global style.toml. See style.example.toml for the full schema.\n\
         # Anything style.toml supports works here (colors, symbols, transcript rules,\n\
         # statusbar) and overrides the global value for this game only.\n\
         \n\
         [colors]\n\
         # \"room:current\" = {{ fg = \"yellow\" }}\n"
    );
    std::fs::write(&path, body)?;
    Ok((path, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("babelmap-pgstyle-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn path_and_scaffold_behaviour() {
        let dir = tmp("scaffold");
        let ifid = "ZCODE-1-ABCDEF-0001";
        let p = per_game_style_path(&dir);
        assert_eq!(p, dir.join("style.toml"));

        // First scaffold creates the file with a header naming the title.
        let (path, created) = scaffold_per_game_style(&dir, ifid, "Zork I").unwrap();
        assert!(created);
        assert_eq!(path, p);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Zork I"), "header names the title");
        assert!(text.contains(ifid), "header names the IFID");
        assert!(text.contains("[colors]"), "seeds an editable [colors] section");

        // Second scaffold does NOT overwrite and reports created=false.
        std::fs::write(&path, "[colors]\n\"room\" = { fg = \"red\" }\n").unwrap();
        let (path2, created2) = scaffold_per_game_style(&dir, ifid, "Zork I").unwrap();
        assert!(!created2);
        assert_eq!(path2, p);
        assert!(std::fs::read_to_string(&path2).unwrap().contains("\"room\""), "existing content preserved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_honor_roundtrips_and_clears() {
        let dir = tmp("honor");
        // Absent → no override.
        assert_eq!(read_per_game_honor(&dir), None);
        // Some(false) persists and reads back.
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(false));
        assert!(per_game_config_path(&dir).is_file());
        // Overwrite with Some(true).
        write_per_game_honor(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(true));
        // None (auto) clears the override.
        write_per_game_honor(&dir, None).unwrap();
        assert_eq!(read_per_game_honor(&dir), None);
        assert!(!per_game_config_path(&dir).exists());
        // Clearing when already absent is a no-op, not an error.
        write_per_game_honor(&dir, None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_borderless_roundtrips_and_coexists_with_honor() {
        let dir = tmp("borderless");
        assert_eq!(read_per_game_borderless(&dir), None);
        // Borderless persists and reads back.
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_borderless(&dir), Some(true));
        // Setting honor must PRESERVE the borderless override (shared sidecar).
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_honor(&dir), Some(false));
        assert_eq!(read_per_game_borderless(&dir), Some(true), "honor write kept borderless");
        // Clearing borderless keeps honor.
        write_per_game_borderless(&dir, None).unwrap();
        assert_eq!(read_per_game_borderless(&dir), None);
        assert_eq!(read_per_game_honor(&dir), Some(false), "borderless clear kept honor");
        // Clearing the last key removes the sidecar.
        write_per_game_honor(&dir, None).unwrap();
        assert!(!per_game_config_path(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_game_show_map_roundtrips_and_coexists_with_others() {
        let dir = tmp("show_map");
        assert_eq!(read_per_game_show_map(&dir), None);
        // show_map persists and reads back.
        write_per_game_show_map(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_show_map(&dir), Some(false));
        // Writing the other two keys must PRESERVE the show_map override.
        write_per_game_honor(&dir, Some(true)).unwrap();
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_show_map(&dir), Some(false), "sibling writes kept show_map");
        assert_eq!(read_per_game_honor(&dir), Some(true));
        assert_eq!(read_per_game_borderless(&dir), Some(true));
        // Clearing show_map keeps the siblings.
        write_per_game_show_map(&dir, None).unwrap();
        assert_eq!(read_per_game_show_map(&dir), None);
        assert_eq!(read_per_game_honor(&dir), Some(true), "show_map clear kept honor");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_per_game_writes_self_contained_and_roundtrips() {
        let dir = tmp("save_pg");
        let colors = crate::colors::ColorScheme::terminal_default();
        let symbols = crate::symbols::SymbolSet::default();
        let path = save_per_game_style(&dir, &colors, &symbols).unwrap();
        assert_eq!(path, per_game_style_path(&dir));
        assert!(path.is_file(), "per-game style file is written");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[colors]"), "file is a self-contained style doc");
        // Self-contained: parses back without error.
        crate::style::parse_style_toml(&text).unwrap();
    }
}
