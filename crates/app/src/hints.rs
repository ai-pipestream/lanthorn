use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

// ── Name pattern matching ──────────────────────────────────────────────────────

/// Returns true if `file_name` looks like a hint file.
///
/// A hint file must:
/// - have a `.z3`, `.z5`, or `.z8` extension, AND
/// - contain one of the keywords `hint`, `clue`, or `invisiclues` in its stem
///   (case-insensitive).
///
/// The extension alone (e.g. `zork1.z5`) does NOT match.
pub fn hint_name_matches(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let has_ext = lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8");
    if !has_ext {
        return false;
    }
    // Strip the extension to check only the stem.
    let stem = &lower[..lower.rfind('.').unwrap_or(lower.len())];
    stem.contains("hint") || stem.contains("clue") || stem.contains("invisiclues")
}

// ── Built-in HINT detection ───────────────────────────────────────────────────

/// Returns true if the story's dictionary contains `hint` or `hints`
/// (case-insensitive).  This is a heuristic: a dictionary entry strongly
/// suggests the story has a built-in hint command, surfaced as a suggestion
/// (never an auto-action).
pub fn story_supports_hint<I: IntoIterator<Item = String>>(dictionary: I) -> bool {
    for word in dictionary {
        let lower = word.to_ascii_lowercase();
        if lower == "hint" || lower == "hints" {
            return true;
        }
    }
    false
}

// ── Per-IFID hint index ───────────────────────────────────────────────────────

/// In-memory map of IFID → hint file path, loaded from `dir/hints/index.toml`.
pub struct HintIndex {
    map: HashMap<String, PathBuf>,
}

impl HintIndex {
    /// Look up the hint file associated with the given IFID.
    pub fn get(&self, ifid: &str) -> Option<PathBuf> {
        self.map.get(ifid).cloned()
    }
}

/// Load the hint index from `dir/hints/index.toml`.
///
/// Returns an empty index if the file does not exist or cannot be parsed.
pub fn load_hint_index(dir: &Path) -> HintIndex {
    let path = dir.join("hints").join("index.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HintIndex { map: HashMap::new() },
    };
    let table: toml::Table = match toml::from_str(&text) {
        Ok(t) => t,
        Err(_) => return HintIndex { map: HashMap::new() },
    };
    let mut map = HashMap::new();
    for (key, val) in table {
        if let toml::Value::String(s) = val {
            map.insert(key, PathBuf::from(s));
        }
    }
    HintIndex { map }
}

/// Persist a hint-file association for `ifid` to `dir/hints/index.toml`.
///
/// Creates the `dir/hints/` directory if absent.  Merges into any existing
/// entries (does not overwrite unrelated IFIDs).
pub fn save_hint_assoc(dir: &Path, ifid: &str, path: &Path) -> io::Result<()> {
    let hints_dir = dir.join("hints");
    std::fs::create_dir_all(&hints_dir)?;
    let index_path = hints_dir.join("index.toml");

    // Load existing document (format-preserving) or start fresh.
    let existing = std::fs::read_to_string(&index_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = existing.parse().unwrap_or_default();

    doc[ifid] = toml_edit::value(path.to_string_lossy().as_ref());

    std::fs::write(&index_path, doc.to_string())
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// The outcome of hint-source resolution.
#[derive(Debug, PartialEq)]
pub enum HintResolution {
    /// A hint file was found at this path.
    File(PathBuf),
    /// No hint file was found automatically — ask the user to choose one.
    AskUser,
    /// (Reserved for future use — e.g. when a `None` branch is needed.)
    None,
}

/// Resolve a hint source for the given story.
///
/// Discovery order:
/// 1. Remembered: the per-IFID association from `index`.
/// 2. Sibling files: any file in the same directory as `story_path` whose
///    name matches `hint_name_matches`.
/// 3. Else: `AskUser` (caller should open the file browser).
pub fn resolve_hint_source(story_path: &Path, ifid: &str, index: &HintIndex) -> HintResolution {
    // Step 1: remembered association.
    if let Some(remembered) = index.get(ifid) {
        if remembered.exists() {
            return HintResolution::File(remembered);
        }
    }

    // Step 2: sibling files in the story's directory.
    if let Some(dir) = story_path.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path == story_path {
                    continue; // skip the story itself
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if hint_name_matches(name) {
                    return HintResolution::File(path);
                }
            }
        }
    }

    HintResolution::AskUser
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_name_matches_patterns() {
        assert!(hint_name_matches("zork1.invisiclues.z5"));
        assert!(hint_name_matches("MyGame-hints.z5"));
        assert!(hint_name_matches("clues.z3"));
        assert!(!hint_name_matches("zork1.z5"));     // the story itself
        assert!(!hint_name_matches("hints.txt"));    // wrong extension
    }

    #[test]
    fn story_supports_hint_detects_dictionary_word() {
        assert!(story_supports_hint(["look", "hint", "take"].map(String::from)));
        assert!(!story_supports_hint(["look", "take"].map(String::from)));
    }

    #[test]
    fn hint_index_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-hintidx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save_hint_assoc(&dir, "ZCODE-1", std::path::Path::new("/x/h.z5")).unwrap();
        let idx = load_hint_index(&dir);
        assert_eq!(idx.get("ZCODE-1"), Some(std::path::PathBuf::from("/x/h.z5")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_finds_sibling_then_asks() {
        // Set up a temp dir with a story file and a sibling hints file.
        let dir = std::env::temp_dir().join(format!("bm-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let story = dir.join("story.z5");
        let hints = dir.join("story.hints.z5");
        std::fs::write(&story, b"fake story").unwrap();
        std::fs::write(&hints, b"fake hints").unwrap();

        let empty_index = HintIndex { map: HashMap::new() };

        // With sibling hints file present: should return File(hints).
        let result = resolve_hint_source(&story, "ZCODE-TEST", &empty_index);
        assert_eq!(result, HintResolution::File(hints));

        // Without any hint sibling: should return AskUser.
        let no_hints_dir = std::env::temp_dir().join(format!("bm-resolve-nosibling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&no_hints_dir);
        std::fs::create_dir_all(&no_hints_dir).unwrap();
        let story2 = no_hints_dir.join("story.z5");
        std::fs::write(&story2, b"fake story").unwrap();

        let result2 = resolve_hint_source(&story2, "ZCODE-TEST", &empty_index);
        assert_eq!(result2, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&no_hints_dir);
    }
}
