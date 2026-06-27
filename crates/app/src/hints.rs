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
    /// A hint entry was found inside a ZIP at `zip_path`; `entry` is its name.
    ///
    /// The caller should use `read_zip_entry` to extract the bytes.
    ZipEntry {
        zip_path: PathBuf,
        entry: String,
    },
    /// No hint file was found automatically — ask the user to choose one.
    AskUser,
    /// (Reserved for future use — e.g. when a `None` branch is needed.)
    None,
}

/// Resolve a hint source for the given story.
///
/// Discovery order:
/// 1. Remembered: the per-IFID association from `index`.
/// 2. Sibling files: any `.z3/.z5/.z8` whose name matches `hint_name_matches`
///    in the same directory as `story_path`.
/// 3. Sibling ZIP: any `.zip` in the same directory that contains an entry
///    whose name matches `hint_name_matches`; returns `ZipEntry` so the caller
///    can extract the bytes with `read_zip_entry`.
/// 4. Else: `AskUser` (caller should open the file browser).
pub fn resolve_hint_source(story_path: &Path, ifid: &str, index: &HintIndex) -> HintResolution {
    // Step 1: remembered association.
    if let Some(remembered) = index.get(ifid) {
        if remembered.exists() {
            return HintResolution::File(remembered);
        }
    }

    // Steps 2 + 3: scan siblings, collecting zip files for step 3.
    if let Some(dir) = story_path.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut zips: Vec<PathBuf> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path == story_path {
                    continue; // skip the story itself
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                // Step 2: plain hint file.
                if hint_name_matches(name) {
                    return HintResolution::File(path);
                }
                // Collect ZIPs for step 3.
                if name.to_ascii_lowercase().ends_with(".zip") {
                    zips.push(path);
                }
            }
            // Step 3: look inside sibling ZIPs for a hint entry.
            for zip_path in zips {
                if let Ok(Some(entry_name)) = find_hint_entry_in_zip(&zip_path) {
                    return HintResolution::ZipEntry { zip_path, entry: entry_name };
                }
            }
        }
    }

    HintResolution::AskUser
}

/// Return the name of the first entry in `zip_path` that matches
/// `hint_name_matches`, or `None` if none matches.
fn find_hint_entry_in_zip(zip_path: &Path) -> io::Result<Option<String>> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        // Only the bare filename portion needs to match the pattern.
        let basename = name.rsplit('/').next().unwrap_or(&name);
        if hint_name_matches(basename) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

// ── Zip helpers ───────────────────────────────────────────────────────────────

/// ZIP magic bytes (local file header signature).
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Load story bytes from `path`.
///
/// - If the file begins with the ZIP magic (`PK\x03\x04`), opens it as a ZIP
///   and returns the bytes of the first entry whose name ends in `.z3`, `.z5`,
///   or `.z8`.
/// - Otherwise returns the raw file bytes.
///
/// Returns `Err` if the file cannot be read, or if the path looks like a ZIP
/// but contains no story entry.
pub fn load_story_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    let bytes = if raw.starts_with(ZIP_MAGIC) {
        // It's a ZIP — find the first story entry.
        let pred = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8")
        };
        match read_zip_entry(path, pred)? {
            Some(bytes) => bytes,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no .z3/.z5/.z8 entry found in zip: {}", path.display()),
                ))
            }
        }
    } else {
        raw
    };
    extract_story(bytes)
}

/// If `bytes` is a Blorb, return its Z-code executable; reject Glulx with a
/// clear error; otherwise pass the bytes through unchanged (a raw story file).
pub fn extract_story(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid Blorb: {e:?}")))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::Glulx, _)) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Glulx story files are not yet supported".to_string(),
        )),
        Err(e) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Blorb has no executable: {e:?}"),
        )),
    }
}

/// Return the bytes of the first ZIP entry whose name satisfies `pred`.
///
/// Returns `Ok(None)` if no entry matches.  Returns `Err` if the file cannot
/// be opened or is not a valid ZIP.
pub fn read_zip_entry(
    zip_path: &Path,
    pred: impl Fn(&str) -> bool,
) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;

    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if pred(entry.name()) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
    }
    Ok(None)
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
    fn load_story_bytes_handles_raw_and_zip() {
        use std::io::Write as _;

        let base = std::env::temp_dir().join(format!("bm-lsb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // Some bytes that look like a valid Z-machine story (just need to be distinct).
        let story_bytes: Vec<u8> = vec![5, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4];

        // --- raw path: a plain .z5 file, no zip magic ---
        let raw_path = base.join("game.z5");
        std::fs::write(&raw_path, &story_bytes).unwrap();
        let loaded_raw = load_story_bytes(&raw_path).expect("raw load");
        assert_eq!(loaded_raw, story_bytes, "raw bytes must be returned as-is");

        // --- zip path: pack the same bytes as "game.z5" inside a zip ---
        let zip_path = base.join("game.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file("game.z5", opts).unwrap();
            zw.write_all(&story_bytes).unwrap();
            zw.finish().unwrap();
        }
        let loaded_zip = load_story_bytes(&zip_path).expect("zip load");
        assert_eq!(loaded_zip, story_bytes, "zip entry bytes must match the original");

        let _ = std::fs::remove_dir_all(&base);
    }

    // Build a minimal Blorb wrapping a single Exec chunk of the given type.
    // Mirrors the blorb crate's builder shape: FORM/IFRS + RIdx + Exec/0 chunk.
    fn make_blorb(exec_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        // RIdx has one entry; the Exec chunk follows it.
        let ridx_data_len = 4 + 12;
        let exec_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&(exec_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(exec_type, payload));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn load_story_bytes_extracts_zblorb_executable() {
        let base = std::env::temp_dir().join(format!("bm-zblorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let zcode = b"ZCODE-PAYLOAD";
        let path = base.join("game.zblorb");
        std::fs::write(&path, make_blorb(b"ZCOD", zcode)).unwrap();
        let out = load_story_bytes(&path).expect("zblorb load");
        assert_eq!(out, zcode);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn load_story_bytes_rejects_glulx_blorb() {
        let base = std::env::temp_dir().join(format!("bm-gblorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let path = base.join("game.gblorb");
        std::fs::write(&path, make_blorb(b"GLUL", b"GLULPAYLOAD")).unwrap();
        let err = load_story_bytes(&path).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("glulx"));

        let _ = std::fs::remove_dir_all(&base);
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
