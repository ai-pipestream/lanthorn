//! Where a story's saves and sidecars live.
//!
//! Both save-capable hosts answer the same three questions — what do I call this
//! game's directory, where is it, and what does a filename the player typed at a
//! save prompt resolve to — and both answered them with the same code, comments
//! included.
//!
//! That mattered more than the duplication usually does, because one of the
//! answers is a bug fix. The per-game directory used to be `base/<story name>`,
//! which collides with the story file itself whenever `base` is the story's own
//! directory: `mkdir` on an existing filename fails, so saving was impossible in
//! the most ordinary layout there is. The `.save` suffix (SQ-0284/0294) is what
//! separates them, and it was living in two places at once (SQ-0618).
//!
//! `scott-cli` has no part in this: Scott has no save protocol at all.

use std::path::{Path, PathBuf};

/// A story file's basename, sanitized into a directory-name token.
///
/// The **extension is kept**, deliberately: `Zork1.z5` and `Zork1.gblorb` are
/// different games as far as saves are concerned, and dropping it would let them
/// share a directory. Anything outside `[A-Za-z0-9._-]` becomes `_`, so a title
/// with spaces, quotes or a slash cannot escape the directory it names. An empty
/// result falls back to `game`.
pub fn story_key(story_path: &Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
}

/// The directory holding this story's saves and sidecars.
///
/// `data_dir` is the `--data-dir` override; without it the story's own directory
/// is used, which is what makes the `.save` suffix load-bearing rather than
/// decorative — see the module docs. A story path with no parent (a bare
/// filename) resolves against the current directory.
pub fn game_dir(story_path: &Path, data_dir: Option<&str>) -> PathBuf {
    let base = data_dir.map(PathBuf::from).unwrap_or_else(|| {
        story_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    base.join(format!("{}.save", story_key(story_path)))
}

/// Resolve what the player typed at a save/restore filename prompt.
///
/// A bare name lands in `game_dir` with a `.qzl` extension, so `quick` means
/// this game's quick save and not a file in whatever directory the shell
/// happened to be in. Anything carrying a path separator is honoured verbatim,
/// which is the escape hatch for saving somewhere else on purpose.
pub fn resolve_save_input(input: &str, game_dir: &Path) -> PathBuf {
    let t = input.trim();
    if t.contains('/') || t.contains('\\') {
        PathBuf::from(t)
    } else {
        let name = if t.ends_with(".qzl") { t.to_string() } else { format!("{t}.qzl") };
        game_dir.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_key_keeps_the_extension_and_sanitizes_the_rest() {
        assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
        assert_ne!(
            story_key(Path::new("/g/Zork1.z5")),
            story_key(Path::new("/g/Zork1.gblorb")),
            "two formats of the same title must not share a save directory"
        );
        assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
        assert_eq!(story_key(Path::new("")), "game");
    }

    #[test]
    fn a_hostile_filename_cannot_escape_the_directory_it_names() {
        // Only the basename is considered, and separators are not in the allowed
        // set, so nothing here can climb out.
        assert_eq!(story_key(Path::new("../../etc/passwd")), "passwd");
        assert!(!story_key(Path::new("/g/a/b.z5")).contains('/'));
        assert!(!story_key(Path::new(r"C:\g\b.z5")).contains('\\'));
    }

    #[test]
    fn the_game_dir_does_not_collide_with_the_story_file_itself() {
        // SQ-0284/0294. The default base IS the story's own directory, so
        // without the `.save` suffix `mkdir` would be asked to create a
        // directory where the story file already is, and fail.
        let dir = game_dir(Path::new("/games/zork1.z5"), None);
        assert_eq!(dir, PathBuf::from("/games/zork1.z5.save"));
        assert_ne!(dir, PathBuf::from("/games/zork1.z5"), "must not be the story file");
    }

    #[test]
    fn the_game_dir_can_actually_be_created_beside_the_story_file() {
        // The path comparison above is not the whole guarantee: what failed
        // before SQ-0294 was `mkdir` itself, refusing to create a directory
        // where a file of that name already exists. So do it for real.
        let tmp = std::env::temp_dir()
            .join(format!("cli-host-storage-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.z5");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.z5

        let dir = game_dir(&story_path, None);
        assert_eq!(dir, tmp.join("game.z5.save"));
        std::fs::create_dir_all(&dir).expect("must not collide with the story file");
        assert!(dir.is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn data_dir_overrides_the_storys_own_directory() {
        let dir = game_dir(Path::new("/games/zork1.z5"), Some("/var/saves"));
        assert_eq!(dir, PathBuf::from("/var/saves/zork1.z5.save"));
    }

    #[test]
    fn a_bare_story_filename_resolves_against_the_current_directory() {
        assert_eq!(game_dir(Path::new("zork1.z5"), None), PathBuf::from("./zork1.z5.save"));
    }

    #[test]
    fn a_bare_save_name_lands_in_the_game_dir_a_path_does_not() {
        let gd = Path::new("/data/Zork1.z5.save");
        assert_eq!(resolve_save_input("quick", gd), PathBuf::from("/data/Zork1.z5.save/quick.qzl"));
        assert_eq!(
            resolve_save_input("quick.qzl", gd),
            PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "an extension the player typed is not doubled"
        );
        assert_eq!(resolve_save_input("/tmp/foo.qzl", gd), PathBuf::from("/tmp/foo.qzl"));
        // A RELATIVE path is the case that actually pins the rule: `Path::join`
        // with an absolute path replaces the whole thing, so the absolute case
        // above passes either way and cannot tell a working escape hatch from a
        // broken one.
        assert_eq!(
            resolve_save_input("saves/foo.qzl", gd),
            PathBuf::from("saves/foo.qzl"),
            "a path the player typed is honoured, not reparented into the game dir"
        );
        assert_eq!(resolve_save_input("  quick  ", gd), PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "trimmed, because the prompt line carries whatever was typed");
    }
}
