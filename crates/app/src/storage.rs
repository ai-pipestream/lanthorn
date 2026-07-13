//! Per-game storage layout (SQ-0284). Saves and sidecars live in a flat
//! per-game directory `<base>/<story-key>.save/`, keyed by the story's
//! *filename* (not its IFID). The `.save` suffix keeps the directory from
//! colliding with the story file when `base` is the story's own directory.
//! Inside it: `default.aux`, `default.glkvfs`,
//! `default.babelmap` (auto/singleton), plus `<slug>.babelmap` / `<slug>.qzl`
//! (named). IFID is retained elsewhere for title/hint lookup only.

use std::path::{Path, PathBuf};

/// Per-game directory name: the story file's basename (incl. extension),
/// sanitized to a filesystem-safe token. Empty -> "game".
pub fn story_key(story_path: &Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
}

/// The per-game directory: `<base>/<story-key>.save/`. The `.save` suffix
/// keeps the directory from colliding with the story file itself when `base`
/// is the story's own directory (e.g. `Zork1.z5` vs `Zork1.z5.save/`).
pub fn game_dir(base: &Path, key: &str) -> PathBuf {
    base.join(format!("{key}.save"))
}

/// The default (auto/singleton) Save-State slot inside a game dir.
pub fn default_state_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.babelmap")
}

/// `default` is reserved for the auto/singleton slot; a user save may not use it.
pub fn is_reserved_slug(slug: &str) -> bool {
    slug == "default"
}

/// Delete the game's AUTO persistent data so the next boot starts from scratch:
/// the Glk VFS cache (`default.glkvfs`), the Z-machine aux sidecar (`default.aux`),
/// and the auto/singleton Save State (`default.babelmap`). The player's named
/// Save States (`<slug>.babelmap`) and in-game saves (`<slug>.qzl` / `_*.qzl`)
/// are left untouched — only the three reserved `default.*` files go. A missing
/// file is not an error.
pub fn delete_auto_persistent(game_dir: &Path) {
    for name in ["default.glkvfs", "default.aux", "default.babelmap"] {
        let path = game_dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("warning: could not delete {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn key_keeps_ext_and_sanitizes() {
        assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
        assert_ne!(story_key(Path::new("/g/z.z5")), story_key(Path::new("/g/z.gblorb")));
        assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
        assert_eq!(story_key(Path::new("")), "game");
    }

    #[test]
    fn game_dir_joins() {
        assert_eq!(game_dir(Path::new("/base"), "Zork1.z5"), PathBuf::from("/base/Zork1.z5.save"));
    }

    /// The per-game dir name must always end in `.save` — that's what keeps
    /// it from colliding with a story file of the same name.
    #[test]
    fn game_dir_name_ends_with_dot_save() {
        let dir = game_dir(Path::new("/base"), "Advent.gblorb");
        assert!(
            dir.file_name().unwrap().to_str().unwrap().ends_with(".save"),
            "got {dir:?}"
        );
    }

    /// Regression for SQ-0284: when `base` is the story's own directory (the
    /// CLI default) and a file already exists at `<base>/<story-key>`,
    /// `create_dir_all(game_dir(..))` must still succeed because the `.save`
    /// suffix makes the two paths distinct.
    #[test]
    fn game_dir_does_not_collide_with_same_named_story_file() {
        let tmp = std::env::temp_dir().join(format!("babelmap-storage-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.gblorb");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.gblorb

        let dir = game_dir(&tmp, &story_key(&story_path));
        assert_eq!(dir, tmp.join("game.gblorb.save"));
        std::fs::create_dir_all(&dir).expect("must not collide with the story file");
        assert!(dir.is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn default_state_path_is_in_game_dir() {
        assert_eq!(
            default_state_path(Path::new("/base/Zork1.z5.save")),
            PathBuf::from("/base/Zork1.z5.save/default.babelmap")
        );
    }

    #[test]
    fn default_is_reserved() {
        assert!(is_reserved_slug("default"));
        assert!(!is_reserved_slug("quicksave"));
    }

    /// `delete_auto_persistent` removes exactly the three reserved `default.*`
    /// auto files and keeps the player's named Save States and in-game saves.
    #[test]
    fn delete_auto_persistent_removes_only_defaults() {
        let tmp = std::env::temp_dir()
            .join(format!("babelmap-delete-auto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // The three auto sidecars that must go.
        for f in ["default.glkvfs", "default.aux", "default.babelmap"] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }
        // Player data that must survive.
        for f in ["myslot.babelmap", "quick.qzl", "_autosave.qzl"] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        delete_auto_persistent(&tmp);

        for f in ["default.glkvfs", "default.aux", "default.babelmap"] {
            assert!(!tmp.join(f).exists(), "{f} should be deleted");
        }
        for f in ["myslot.babelmap", "quick.qzl", "_autosave.qzl"] {
            assert!(tmp.join(f).exists(), "{f} should be kept");
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A missing file (or a fresh game dir) is not an error.
    #[test]
    fn delete_auto_persistent_ignores_missing() {
        let tmp = std::env::temp_dir()
            .join(format!("babelmap-delete-auto-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        delete_auto_persistent(&tmp); // no default.* present -> no panic
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
