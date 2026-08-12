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
/// (`honor_game_colours`, `borderless_windows`, `show_map`, `pictures`), kept
/// separate from `style.toml` so the style parser/writer stays a pure style
/// document.
pub fn per_game_config_path(game_dir: &Path) -> PathBuf {
    game_dir.join("config.toml")
}

/// Read one boolean key from the per-game `config.toml`. `None` if the file is
/// absent/unparseable or the key is missing.
fn read_config_bool(game_dir: &Path, key: &str) -> Option<bool> {
    let text = std::fs::read_to_string(per_game_config_path(game_dir)).ok()?;
    text.parse::<toml::Value>().ok()?.get(key).and_then(|v| v.as_bool())
}

/// Read one string key from the per-game `config.toml`, trimmed. `None` if the
/// file is absent/unparseable, the key is missing, or its value is blank.
fn read_config_str(game_dir: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(per_game_config_path(game_dir)).ok()?;
    let v = text.parse::<toml::Value>().ok()?;
    let s = v.get(key)?.as_str()?.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
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

/// Read the per-game `pictures` override — SQ-0734 tier 3, the user naming a
/// native Infocom picture archive (`Pic.data`, `.MG1`/`.EG1`/`.CG1`) and thereby
/// ASSERTING that it belongs to this story. `None` = no override, so the Blorb
/// (tier 1) or the disk image the story was mounted from (tier 2) decides.
///
/// The value is a path: absolute, or relative to the STORY's own directory —
/// "beside the story" is where these archives sit. Resolution and validation
/// live in [`crate::graphics::PictureOverride::resolve`]; this only reads the key.
///
/// There is deliberately no writer: no UI sets this, and the key is
/// hand-written into the sidecar. The writer below nonetheless carries it, so
/// toggling a colour or map preference cannot delete it.
pub fn read_per_game_pictures(game_dir: &Path) -> Option<String> {
    read_config_str(game_dir, "pictures")
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
    // `pictures` (SQ-0734) has no writer of its own — the user hand-writes it —
    // so it is read back and re-emitted here. Without this, toggling any of the
    // three keys above through the UI would silently delete the picture archive
    // the user named, and the game would quietly revert to its Blorb art.
    if let Some(v) = read_per_game_pictures(game_dir) {
        body.push_str(&format!("pictures = {}\n", toml::Value::String(v)));
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
    fn per_game_style_path_is_game_dir_style_toml() {
        let dir = tmp("path");
        assert_eq!(per_game_style_path(&dir), dir.join("style.toml"));
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

    /// SQ-0734: `pictures` is hand-written and has no writer of its own, so the
    /// three keys that DO have writers must carry it through. Without that,
    /// toggling game colours from the UI would silently delete the picture
    /// archive the user chose and quietly revert the game to its Blorb art —
    /// the exact "plausible but wrong" failure the tier policy exists to avoid,
    /// caused by us rather than by a bad guess.
    #[test]
    fn a_hand_written_pictures_key_survives_every_sibling_write() {
        let dir = tmp("pictures");
        assert_eq!(read_per_game_pictures(&dir), None);
        std::fs::write(per_game_config_path(&dir), "pictures = \"FMVPOKER.EG1\"\n").unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()));

        // Each writer rewrites the whole sidecar; each must preserve `pictures`.
        write_per_game_honor(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "honor write");
        write_per_game_borderless(&dir, Some(true)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "borderless write");
        write_per_game_show_map(&dir, Some(false)).unwrap();
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()), "show_map write");

        // Clearing every OTHER key must not delete the file out from under it.
        write_per_game_honor(&dir, None).unwrap();
        write_per_game_borderless(&dir, None).unwrap();
        write_per_game_show_map(&dir, None).unwrap();
        assert!(per_game_config_path(&dir).is_file(), "pictures alone keeps the sidecar alive");
        assert_eq!(read_per_game_pictures(&dir), Some("FMVPOKER.EG1".to_string()));

        // A blank value is not a choice.
        std::fs::write(per_game_config_path(&dir), "pictures = \"  \"\n").unwrap();
        assert_eq!(read_per_game_pictures(&dir), None);
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

}
