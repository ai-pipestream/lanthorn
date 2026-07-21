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

/// The stem of a hint file's name, i.e. the raw file name minus a trailing
/// `.z3/.z5/.z8` extension, lowercased.
///
/// Unlike `Path::file_stem`, this strips only the story extension, so a compound
/// name like `zork1.hints.z5` yields `zork1.hints` (not `zork1`).
fn hint_stem(file_name: &str) -> String {
    let lower = file_name.to_ascii_lowercase();
    for ext in [".z3", ".z5", ".z8"] {
        if let Some(s) = lower.strip_suffix(ext) {
            return s.to_string();
        }
    }
    lower
}

/// Returns true when the hint-file `candidate_name`'s stem starts with the
/// story's stem (both compared case-insensitively).
///
/// So story `zork1` matches `zork1_hints.z5`, `zork1.hints.z5`, and
/// `zork1-invisiclues.z5`, but not `zork2_hints.z5`.
fn stem_matches_story(story_stem: &str, candidate_name: &str) -> bool {
    hint_stem(candidate_name).starts_with(&story_stem.to_ascii_lowercase())
}

/// Rank hint candidate names by story-stem preference and return the chosen one.
///
/// Tiers:
/// 1. any candidate whose stem starts with `story_stem` → the first such after a
///    stable name sort (deterministic regardless of readdir order);
/// 2. else if exactly one candidate exists → that lone (generic) candidate;
/// 3. else (multiple candidates, none story-specific) → `None` (ambiguous).
///
/// Returns `None` for an empty list too; callers distinguish empty from
/// ambiguous by checking whether the input was empty.
fn pick_hint_candidate(story_stem: &str, mut names: Vec<String>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    names.sort();
    if let Some(m) = names.iter().find(|n| stem_matches_story(story_stem, n)) {
        return Some(m.clone());
    }
    if names.len() == 1 {
        return names.into_iter().next();
    }
    None
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
        // The story's own stem drives story-aware matching (`zork1.z5` → `zork1`).
        let story_stem = story_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut hint_files: Vec<PathBuf> = Vec::new();
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
                // Step 2: collect plain hint files (ranked below).
                if hint_name_matches(name) {
                    hint_files.push(path.clone());
                }
                // Collect ZIPs for step 3.
                if name.to_ascii_lowercase().ends_with(".zip") {
                    zips.push(path);
                }
            }

            // Step 2: rank the sibling hint files. Prefer a story-stem match,
            // then a lone generic; multiple generics with no story match are
            // ambiguous — ask the user rather than guess.
            if !hint_files.is_empty() {
                let names: Vec<String> = hint_files
                    .iter()
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
                    .collect();
                if let Some(chosen) = pick_hint_candidate(&story_stem, names) {
                    for path in &hint_files {
                        if path.file_name().and_then(|n| n.to_str()) == Some(chosen.as_str()) {
                            return HintResolution::File(path.clone());
                        }
                    }
                }
                // Ambiguous (multiple, none story-specific): don't fall through
                // to the ZIP guess.
                return HintResolution::AskUser;
            }

            // Step 3: look inside sibling ZIPs for a hint entry, story-aware.
            zips.sort();
            for zip_path in zips {
                if let Ok(Some(entry_name)) = find_hint_entry_in_zip(&zip_path, &story_stem) {
                    return HintResolution::ZipEntry { zip_path, entry: entry_name };
                }
            }
        }
    }

    HintResolution::AskUser
}

/// Return the best-ranked entry in `zip_path` matching `hint_name_matches`,
/// preferring an entry whose stem starts with `story_stem`.
///
/// Applies the same tiers as [`pick_hint_candidate`]: a story-stem match wins;
/// a lone generic entry is used; multiple generics with no story match are
/// ambiguous and yield `None`.
fn find_hint_entry_in_zip(zip_path: &Path, story_stem: &str) -> io::Result<Option<String>> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut matches: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let name = entry.name().to_string();
        // Only the bare filename portion needs to match the pattern.
        let basename = name.rsplit('/').next().unwrap_or(&name);
        if hint_name_matches(basename) {
            matches.push(name);
        }
    }
    // Rank by the bare filename, but return the full entry path.
    let basename_of = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    let basenames: Vec<String> = matches.iter().map(|n| basename_of(n)).collect();
    match pick_hint_candidate(story_stem, basenames) {
        Some(chosen) => Ok(matches.into_iter().find(|n| basename_of(n) == chosen)),
        None => Ok(None),
    }
}

// ── Zip helpers ───────────────────────────────────────────────────────────────

/// ZIP magic bytes (local file header signature).
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// A story image classified by the VM engine that runs it.
///
/// `extract_story` returns this so session creation can route a Z-code image to
/// the Z-machine (`GameSession`) and a Glulx image to `GlulxSession`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadedStory {
    /// A Z-machine story image (`.z*` / `ZCOD` Blorb).
    ZCode(Vec<u8>),
    /// A Glulx story image (`.ulx` / `GLUL` Blorb / `.gblorb`).
    Glulx(Vec<u8>),
    /// A Scott Adams story database (`.dat`).
    Scott(Vec<u8>),
}

impl LoadedStory {
    /// The raw executable bytes, regardless of engine.
    pub fn bytes(&self) -> &[u8] {
        match self {
            LoadedStory::ZCode(b) | LoadedStory::Glulx(b) | LoadedStory::Scott(b) => b,
        }
    }
    /// Consume into the raw executable bytes, regardless of engine.
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            LoadedStory::ZCode(b) | LoadedStory::Glulx(b) | LoadedStory::Scott(b) => b,
        }
    }
}

/// Read a story file's executable bytes, transparently unwrapping a ZIP whose
/// first `.z3/.z5/.z8` entry is the story. Does not classify the engine — see
/// [`load_story`] / [`extract_story`].
fn read_story_file(path: &Path) -> io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    if raw.starts_with(ZIP_MAGIC) {
        // It's a ZIP — find the first story entry.
        let pred = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8")
        };
        match read_zip_entry(path, pred)? {
            Some(bytes) => Ok(bytes),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no .z3/.z5/.z8 entry found in zip: {}", path.display()),
            )),
        }
    } else {
        Ok(raw)
    }
}

/// Load a story from `path`, classified by engine ([`LoadedStory`]).
///
/// Unwraps a ZIP (first `.z*` entry) and a Blorb container; a raw `.ulx`
/// (`Glul` magic) is recognised as Glulx, and any other raw bytes are treated
/// as Z-code (preserving the historical default).
pub fn load_story(path: &Path) -> io::Result<LoadedStory> {
    extract_story(read_story_file(path)?)
}

/// Load story bytes from `path`, restricted to **Z-code** images.
///
/// Convenience wrapper over [`load_story`] for the Z-machine-only call sites
/// (the hint companion VM, the picker's legacy path). A Glulx image is rejected
/// with a clear error so those sites behave exactly as before.
pub fn load_story_bytes(path: &Path) -> io::Result<Vec<u8>> {
    match load_story(path)? {
        LoadedStory::ZCode(b) => Ok(b),
        LoadedStory::Glulx(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Glulx story files are not supported on this path".to_string(),
        )),
        LoadedStory::Scott(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Scott Adams story files are not supported on this path".to_string(),
        )),
    }
}

/// Classify `bytes` into a [`LoadedStory`].
///
/// A Blorb yields its embedded executable's kind; a raw image starting with the
/// `Glul` magic is Glulx; anything else is treated as a raw Z-code story (the
/// historical pass-through). Never errors for a non-Blorb input.
pub fn extract_story(bytes: Vec<u8>) -> io::Result<LoadedStory> {
    if !blorb::Blorb::is_blorb(&bytes) {
        // Raw image: distinguish Glulx by its `Glul` magic; a Scott Adams `.dat`
        // is content-sniffed (it has no fixed magic); default to Z-code.
        if bytes.starts_with(b"Glul") {
            return Ok(LoadedStory::Glulx(bytes));
        }
        if let Ok(s) = std::str::from_utf8(&bytes) {
            if scott::looks_like_scott(s) {
                return Ok(LoadedStory::Scott(bytes));
            }
        }
        return Ok(LoadedStory::ZCode(bytes));
    }
    let b = blorb::Blorb::parse(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid Blorb: {e:?}")))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(LoadedStory::ZCode(data.to_vec())),
        Ok((blorb::ExecKind::Glulx, data)) => Ok(LoadedStory::Glulx(data.to_vec())),
        Ok((blorb::ExecKind::Scott, data)) => Ok(LoadedStory::Scott(data.to_vec())),
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
    fn extract_story_classifies_engine() {
        // ZCOD Blorb → ZCode(payload).
        let zblorb = make_blorb(b"ZCOD", b"ZCODE");
        assert_eq!(extract_story(zblorb).unwrap(), LoadedStory::ZCode(b"ZCODE".to_vec()));
        // GLUL Blorb → Glulx(payload).
        let gblorb = make_blorb(b"GLUL", b"GLULX");
        assert_eq!(extract_story(gblorb).unwrap(), LoadedStory::Glulx(b"GLULX".to_vec()));
        // Raw Z-code (version byte, no Glul magic) → ZCode pass-through.
        let raw_z: Vec<u8> = vec![5, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(extract_story(raw_z.clone()).unwrap(), LoadedStory::ZCode(raw_z));
        // Raw .ulx (Glul magic) → Glulx pass-through.
        let mut raw_ulx = b"Glul".to_vec();
        raw_ulx.extend_from_slice(&[0, 3, 1, 2]);
        assert_eq!(extract_story(raw_ulx.clone()).unwrap(), LoadedStory::Glulx(raw_ulx));
    }

    #[test]
    fn load_story_routes_gblorb_and_ulx() {
        let base = std::env::temp_dir().join(format!("bm-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // A .gblorb (GLUL) routes to Glulx.
        let gpath = base.join("game.gblorb");
        std::fs::write(&gpath, make_blorb(b"GLUL", b"GLULPAYLOAD")).unwrap();
        assert_eq!(load_story(&gpath).unwrap(), LoadedStory::Glulx(b"GLULPAYLOAD".to_vec()));

        // A raw .ulx routes to Glulx by its magic.
        let upath = base.join("game.ulx");
        let mut ulx = b"Glul".to_vec();
        ulx.extend_from_slice(&[0, 3, 1, 2, 9, 9]);
        std::fs::write(&upath, &ulx).unwrap();
        assert_eq!(load_story(&upath).unwrap(), LoadedStory::Glulx(ulx));

        // A .zblorb (ZCOD) still routes to Z-code.
        let zpath = base.join("game.zblorb");
        std::fs::write(&zpath, make_blorb(b"ZCOD", b"ZP")).unwrap();
        assert_eq!(load_story(&zpath).unwrap(), LoadedStory::ZCode(b"ZP".to_vec()));

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
    fn detects_scott_dat() {
        let dat = include_bytes!("../../scott/tests/tiny_cave.dat").to_vec();
        match extract_story(dat).unwrap() {
            LoadedStory::Scott(_) => {}
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn zcode_still_defaults() {
        assert!(matches!(
            extract_story(vec![3, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
            LoadedStory::ZCode(_)
        ));
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

    // Create a fresh temp dir with the given files (empty contents), returning it.
    fn scratch_dir(tag: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn resolve_is_story_aware_in_multi_story_dir() {
        let dir = scratch_dir(
            "resolve-multistory",
            &["zork1.z5", "zork1_hints.z5", "zork2.z5", "zork2-invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r1 = resolve_hint_source(&dir.join("zork1.z5"), "IFID-1", &empty);
        assert_eq!(r1, HintResolution::File(dir.join("zork1_hints.z5")));

        let r2 = resolve_hint_source(&dir.join("zork2.z5"), "IFID-2", &empty);
        assert_eq!(r2, HintResolution::File(dir.join("zork2-invisiclues.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_uses_lone_generic() {
        let dir = scratch_dir("resolve-lonegeneric", &["story.z5", "invisiclues.z5"]);
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("invisiclues.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_ambiguous_generics_asks_user() {
        let dir = scratch_dir("resolve-ambiguous", &["story.z5", "hintsA.z5", "hintsB.z5"]);
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_story_stem_beats_generic() {
        let dir = scratch_dir(
            "resolve-stembeats",
            &["zork1.z5", "zork1_hints.z5", "invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("zork1.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("zork1_hints.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_multistory_is_deterministic() {
        let dir = scratch_dir(
            "resolve-determinism",
            &["zork1.z5", "zork1_hints.z5", "zork2.z5", "zork2-invisiclues.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let expected = HintResolution::File(dir.join("zork1_hints.z5"));
        for _ in 0..8 {
            let r = resolve_hint_source(&dir.join("zork1.z5"), "IFID-1", &empty);
            assert_eq!(r, expected);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stem_matches_story_is_case_insensitive_prefix() {
        assert!(stem_matches_story("zork1", "zork1_hints.z5"));
        assert!(stem_matches_story("zork1", "Zork1.hints.z5"));
        assert!(stem_matches_story("zork1", "ZORK1-invisiclues.z5"));
        assert!(!stem_matches_story("zork1", "zork2_hints.z5"));
    }
}
