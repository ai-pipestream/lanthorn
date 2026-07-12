//! Per-game storage layout (SQ-0284). Saves and sidecars live in a flat
//! per-game directory `<base>/<story-key>/`, keyed by the story's *filename*
//! (not its IFID). Inside it: `default.aux`, `default.glkvfs`,
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

/// The per-game directory: `<base>/<story-key>/`.
pub fn game_dir(base: &Path, key: &str) -> PathBuf {
    base.join(key)
}

/// The default (auto/singleton) Save-State slot inside a game dir.
pub fn default_state_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.babelmap")
}

/// `default` is reserved for the auto/singleton slot; a user save may not use it.
pub fn is_reserved_slug(slug: &str) -> bool {
    slug == "default"
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
        assert_eq!(game_dir(Path::new("/base"), "Zork1.z5"), PathBuf::from("/base/Zork1.z5"));
    }

    #[test]
    fn default_state_path_is_in_game_dir() {
        assert_eq!(
            default_state_path(Path::new("/base/Zork1.z5")),
            PathBuf::from("/base/Zork1.z5/default.babelmap")
        );
    }

    #[test]
    fn default_is_reserved() {
        assert!(is_reserved_slug("default"));
        assert!(!is_reserved_slug("quicksave"));
    }
}
