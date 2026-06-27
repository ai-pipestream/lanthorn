//! Pre-game story picker: when a directory is passed at launch instead of a
//! story file, scan it for Z-machine stories and let the user choose one.
//!
//! Titles are resolved cheaply (no game is run): the known-title table keyed by
//! the IFID, falling back to the filename stem.

use std::path::{Path, PathBuf};

/// One selectable story in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryEntry {
    pub path: PathBuf,
    /// Display title: `known_title(ifid)` or the filename stem.
    pub title: String,
    /// The bare filename (e.g. `zork1.z5`), shown beside the title.
    pub filename: String,
}

/// Candidate story-file extensions (matched case-insensitively). `.zblorb` /
/// `.blorb` / zips are handled by `load_story_bytes`; `.dat` covers some
/// Infocom releases.
const STORY_EXTS: &[&str] = &[
    "z3", "z4", "z5", "z7", "z8", "zblorb", "blorb", "zlb", "dat", "ulx", "gblorb",
];

fn has_story_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| STORY_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Scan `dir` (top level, non-recursive) for **launchable** Z-machine stories,
/// resolving a display title for each. Files that don't load or don't parse as
/// a supported story (incl. v6) are silently skipped. Sorted by title
/// (case-insensitive), then filename.
pub fn scan_stories(dir: &Path) -> Vec<StoryEntry> {
    let mut out: Vec<StoryEntry> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !has_story_ext(&path) {
            continue;
        }
        let Ok(loaded) = crate::hints::load_story(&path) else {
            continue;
        };
        // Only list stories babelmap can actually launch: Z-code via the
        // Z-machine loader (accepts v3/4/5/7/8, rejects v6/v1/v2), Glulx via the
        // Glulx loader.
        let bytes = loaded.bytes().to_vec();
        let launchable = match &loaded {
            crate::hints::LoadedStory::ZCode(b) => zvm::memory::Memory::new(b.clone()).is_ok(),
            crate::hints::LoadedStory::Glulx(b) => gvm::Memory::new(b.clone()).is_ok(),
        };
        if !launchable {
            continue;
        }
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let ifid = crate::ifid::compute_ifid(&bytes);
        let title = crate::session::known_title(&ifid)
            .map(|t| t.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&filename)
                    .to_string()
            });
        out.push(StoryEntry { path, title, filename });
    }
    out.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.filename.cmp(&b.filename))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid v3 story bytes (same minimal header as the render tests).
    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        buf[0x18] = 0x00; buf[0x19] = 0x60;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("babelmap-picker-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scan_lists_valid_stories_and_skips_junk() {
        let dir = temp_dir("scan");
        std::fs::write(dir.join("game.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a story").unwrap();   // wrong ext
        std::fs::write(dir.join("broken.z5"), b"garbage").unwrap();       // bad header

        let stories = scan_stories(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "only the valid .z5 is listed");
        assert_eq!(stories[0].filename, "game.z5");
        // No known title for this synthetic IFID → falls back to the stem.
        assert_eq!(stories[0].title, "game");
    }

    #[test]
    fn scan_skips_v6_and_unsupported_versions() {
        let dir = temp_dir("v6");
        let mut v6 = minimal_v3_story();
        v6[0x00] = 6; // graphical v6 — unsupported
        std::fs::write(dir.join("graphic.z6"), &v6).unwrap();
        // .z6 isn't even in STORY_EXTS, and the header would be rejected anyway.
        let mut v6b = minimal_v3_story();
        v6b[0x00] = 6;
        std::fs::write(dir.join("graphic.z5"), &v6b).unwrap(); // v6 bytes, .z5 ext

        let stories = scan_stories(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(stories.is_empty(), "v6 stories are not listed (can't launch)");
    }

    #[test]
    fn scan_sorts_by_title() {
        let dir = temp_dir("sort");
        std::fs::write(dir.join("zebra.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("apple.z5"), minimal_v3_story()).unwrap();
        let stories = scan_stories(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let titles: Vec<&str> = stories.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["apple", "zebra"]);
    }
}


