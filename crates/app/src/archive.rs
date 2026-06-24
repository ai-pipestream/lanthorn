//! Single-file archive bundling a story's map + VM save into one `.babelmap` ZIP.
//!
//! # Integration points (for the follow-up wiring task)
//!
//! In `main.rs` / `session.rs`, replace the two separate persist calls:
//!
//!   ```text
//!   // save path: replace save_map + save_game with:
//!   archive::save_archive(&archive_path, &mapper, &machine)?;
//!
//!   // load path: replace load_map + restore_game with:
//!   let ac = archive::load_archive(&archive_path)?;
//!   let mapper = ac.mapper;
//!   machine.restore_quetzal(&ac.save).map_err(|e| ...)?;
//!   ```
//!
//! Archive path convention (mirrors `ifid::map_path`): `<base_dir>/<ifid>.babelmap`
//!
//! The `meta.ifid` field is currently populated by the caller; pass `None` until
//! IFID computation is wired in. `format_version` must equal 1 or `load_archive`
//! returns an error.

use std::io::{self, Read, Write};
use std::path::Path;

use mapper::mapper::Mapper;
use mapper::persist::{from_json, to_json};

// ZIP entry names.
const ENTRY_MAP: &str = "map.json";
const ENTRY_SAVE: &str = "game.sav";
const ENTRY_META: &str = "meta.json";

const CURRENT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Meta {
    pub format_version: u32,
    pub ifid: Option<String>,
    /// Human-readable save name, or None for the default (quick-save) slot.
    #[serde(default)]
    pub name: Option<String>,
    /// Turn counter at save time (app-tracked, 0 for saves written before this field existed).
    #[serde(default)]
    pub turns: u32,
    /// RFC3339 timestamp of when this save was written, empty string for legacy saves.
    #[serde(default)]
    pub saved_at: String,
}

#[derive(Debug)]
pub struct ArchiveContents {
    pub mapper: Mapper,
    pub save: Vec<u8>,
    pub meta: Meta,
}

/// Write a `.babelmap` archive containing the current map and VM save.
///
/// The `machine` save bytes are produced via `Machine::save_quetzal`, the same
/// method used by `persist_files::save_game`. No save logic is duplicated here.
pub fn save_archive(
    path: &Path,
    mapper: &Mapper,
    machine: &zvm::cpu::exec::Machine,
) -> io::Result<()> {
    save_archive_meta(path, mapper, machine, Meta {
        format_version: CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
    })
}

/// Write a `.babelmap` archive with explicit metadata (name, turns, saved_at).
///
/// Used by `persist_files::save_named` to attach save slot information.
pub fn save_archive_meta(
    path: &Path,
    mapper: &Mapper,
    machine: &zvm::cpu::exec::Machine,
    meta: Meta,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // map.json — reuse mapper::persist serialization
    let map_json = to_json(mapper);
    zip.start_file(ENTRY_MAP, options)?;
    zip.write_all(map_json.as_bytes())?;

    // game.sav — same bytes save_game writes
    let save_bytes = machine.save_quetzal();
    zip.start_file(ENTRY_SAVE, options)?;
    zip.write_all(&save_bytes)?;

    // meta.json
    let meta_json =
        serde_json::to_string_pretty(&meta).expect("Meta is always serializable");
    zip.start_file(ENTRY_META, options)?;
    zip.write_all(meta_json.as_bytes())?;

    zip.finish()?;
    Ok(())
}

/// Read a `.babelmap` archive.
///
/// Returns `Err` if the file is missing, corrupt, an entry is absent, or
/// `meta.format_version` is not 1. The caller restores the VM save via:
/// `machine.restore_quetzal(&contents.save)`.
pub fn load_archive(path: &Path) -> io::Result<ArchiveContents> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // meta.json — check version first
    let meta: Meta = {
        let mut entry = zip.by_name(ENTRY_META).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_META}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_META}: {e}")))?
    };

    if meta.format_version != CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected {}",
                meta.format_version, CURRENT_FORMAT_VERSION
            ),
        ));
    }

    // map.json
    let mapper = {
        let mut entry = zip.by_name(ENTRY_MAP).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_MAP}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        from_json(&buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_MAP}: {e}")))?
    };

    // game.sav
    let save = {
        let mut entry = zip.by_name(ENTRY_SAVE).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_SAVE}: {e}"))
        })?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };

    Ok(ArchiveContents { mapper, save, meta })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    fn temp_archive_path(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("babelmap-archive-test-{}-{}.babelmap", tag, std::process::id()));
        p
    }

    fn small_mapper() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m
    }

    // -------------------------------------------------------------------------
    // round-trip: map JSON and save bytes survive a write-read cycle
    // -------------------------------------------------------------------------
    #[test]
    fn round_trip_map_and_save_bytes() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else {
            return; // skip if fixture absent
        };

        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 {
            let _ = machine.step();
        }

        let mapper = small_mapper();
        let expected_map_json = to_json(&mapper);
        let expected_save = machine.save_quetzal();

        let path = temp_archive_path("roundtrip");
        save_archive(&path, &mapper, &machine).expect("save_archive");

        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        // Map round-trips via JSON comparison (same as persist_files tests)
        assert_eq!(to_json(&ac.mapper), expected_map_json, "map JSON must match");

        // Save bytes are byte-identical
        assert_eq!(ac.save, expected_save, "save bytes must be identical");

        // Meta
        assert_eq!(ac.meta.format_version, 1);
        assert!(ac.meta.ifid.is_none());
    }

    // -------------------------------------------------------------------------
    // corrupt ZIP -> Err, not a panic
    // -------------------------------------------------------------------------
    #[test]
    fn corrupt_zip_returns_err() {
        let path = temp_archive_path("corrupt");
        std::fs::write(&path, b"this is not a zip file").unwrap();
        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "corrupt archive must return Err");
    }

    // -------------------------------------------------------------------------
    // valid ZIP but missing a required entry -> Err
    // -------------------------------------------------------------------------
    #[test]
    fn missing_entry_returns_err() {
        use std::io::Write as _;

        let path = temp_archive_path("missing-entry");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write only meta.json; omit map.json and game.sav
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new() };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "archive missing map.json must return Err");
    }

    // -------------------------------------------------------------------------
    // back-compat: old archive (no name/turns/saved_at fields) still loads
    // -------------------------------------------------------------------------
    #[test]
    fn old_archive_without_new_meta_fields_loads_with_defaults() {
        use std::io::Write as _;

        let path = temp_archive_path("backcompat");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write a meta.json with only the original two fields.
            let old_meta_json = r#"{"format_version":1,"ifid":"ZCODE-1-000000-0000"}"#;
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(old_meta_json.as_bytes()).unwrap();

            // map.json: minimal valid mapper JSON
            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            // game.sav: empty bytes (won't be restored in this test)
            zip.start_file(ENTRY_SAVE, options).unwrap();
            zip.write_all(&[]).unwrap();

            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("old archive should load");
        let _ = std::fs::remove_file(&path);

        assert!(ac.meta.name.is_none(), "name defaults to None");
        assert_eq!(ac.meta.turns, 0, "turns defaults to 0");
        assert_eq!(ac.meta.saved_at, "", "saved_at defaults to empty string");
        assert_eq!(ac.meta.ifid.as_deref(), Some("ZCODE-1-000000-0000"));
    }

    // -------------------------------------------------------------------------
    // unknown format_version -> Err
    // -------------------------------------------------------------------------
    #[test]
    fn unknown_format_version_returns_err() {
        use std::io::Write as _;

        let path = temp_archive_path("bad-version");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            let meta = Meta { format_version: 99, ifid: None, name: None, turns: 0, saved_at: String::new() };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let result = load_archive(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err(), "unknown format_version must return Err");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("99"), "error should mention the bad version: {msg}");
    }
}
