//! Per-game style overrides: a style.toml keyed by IFID, layered over the global
//! style.toml. See docs/superpowers/specs/2026-06-25-per-game-styles-design.md.

use std::path::{Path, PathBuf};

/// The per-game style file path: `user_dir/styles/<ifid>.toml`.
pub fn per_game_style_path(user_dir: &Path, ifid: &str) -> PathBuf {
    user_dir.join("styles").join(format!("{ifid}.toml"))
}

/// Write the live look self-contained to the current game's per-game style file
/// (`user_dir/styles/<ifid>.toml`), creating `styles/` if needed. Returns the path
/// written. Does NOT repoint `config.style`; the file is merged over the global
/// style.toml on the next reload.
pub fn save_per_game_style(
    user_dir: &Path,
    ifid: &str,
    colors: &crate::colors::ColorScheme,
    symbols: &crate::symbols::SymbolSet,
) -> std::io::Result<PathBuf> {
    let path = per_game_style_path(user_dir, ifid);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::style::write_style_full(&path, colors, symbols)?;
    Ok(path)
}

/// Create the per-game style file (and the `styles/` dir) if it does not exist,
/// seeded with a title/IFID header. Returns `(path, created)`; never overwrites an
/// existing file (`created = false`).
pub fn scaffold_per_game_style(user_dir: &Path, ifid: &str, title: &str) -> std::io::Result<(PathBuf, bool)> {
    let path = per_game_style_path(user_dir, ifid);
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
        let p = per_game_style_path(&dir, ifid);
        assert_eq!(p, dir.join("styles").join(format!("{ifid}.toml")));

        // First scaffold creates the file (+ styles/ dir) with a header naming the title.
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
    fn save_per_game_writes_self_contained_and_roundtrips() {
        let dir = tmp("save_pg");
        let ifid = "ZCODE-1-ABCDEF-0001";
        let colors = crate::colors::ColorScheme::terminal_default();
        let symbols = crate::symbols::SymbolSet::default();
        let path = save_per_game_style(&dir, ifid, &colors, &symbols).unwrap();
        assert_eq!(path, per_game_style_path(&dir, ifid));
        assert!(path.is_file(), "per-game style file is written");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[colors]"), "file is a self-contained style doc");
        // Self-contained: parses back without error.
        crate::style::parse_style_toml(&text).unwrap();
    }
}
