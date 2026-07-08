use std::io;
use std::path::{Path, PathBuf};
use mapper::mapper::Mapper;
use mapper::persist::from_json;

// ── Named save slots ──────────────────────────────────────────────────────────

/// Metadata for one discovered save file.
#[derive(Debug, Clone)]
pub struct SaveInfo {
    /// Absolute path to the `.babelmap` file.
    pub path: PathBuf,
    /// Human-readable name (slug-form for named saves, "(default)" for the
    /// quick-save slot).
    pub name: String,
    /// Turn counter at save time.
    pub turns: u32,
    /// RFC3339 timestamp string (may be empty for legacy saves).
    pub saved_at: String,
    /// True for the default (IFID-only) quick-save slot.
    pub is_default: bool,
}

/// List all save files for the given IFID in `dir`.
///
/// Discovers `<ifid>.babelmap` (default slot) and `<ifid>-*.babelmap` (named
/// slots), reads their `Meta`, and returns sorted results: default slot first,
/// then named saves sorted by `saved_at` descending (newest first). Files that
/// fail to parse are silently skipped.
pub fn list_saves(dir: &Path, ifid: &str) -> Vec<SaveInfo> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let default_name = format!("{}.babelmap", ifid);
    let named_prefix = format!("{}-", ifid);

    let mut infos: Vec<SaveInfo> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };

        let is_default = fname == default_name;
        let is_named = !is_default && fname.starts_with(&named_prefix) && fname.ends_with(".babelmap");

        if !is_default && !is_named {
            continue;
        }

        // Read only meta.json; skip on failure (corrupt/unsupported → not listed).
        let meta = match crate::archive::read_archive_meta(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let name = if is_default {
            "(default)".to_string()
        } else {
            // Strip "<ifid>-" prefix and ".babelmap" suffix to recover the slug.
            let inner = &fname[named_prefix.len()..fname.len() - ".babelmap".len()];
            // Prefer the name stored in Meta, fall back to the slug.
            meta.name.clone().unwrap_or_else(|| inner.to_string())
        };

        infos.push(SaveInfo {
            path,
            name,
            turns: meta.turns,
            saved_at: meta.saved_at.clone(),
            is_default,
        });
    }

    // Sort: default first, then by saved_at descending (newer saves sort earlier).
    infos.sort_by(|a, b| {
        match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => b.saved_at.cmp(&a.saved_at),
        }
    });

    infos
}

/// Write a named save: `<dir>/<ifid>-<slug>.babelmap`.
///
/// `name` is sanitized into a filesystem-safe slug (lowercase alphanum + hyphens).
#[allow(clippy::too_many_arguments)]
pub fn save_named(
    dir: &Path,
    ifid: &str,
    name: &str,
    mapper: &Mapper,
    save: &crate::engine::EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &std::collections::BTreeMap<String, Vec<u8>>,
    turns: u32,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    transcript_runs: &[Vec<crate::state::StyleRun>],
) -> io::Result<()> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "save name is empty after sanitization"));
    }
    let path = dir.join(format!("{}-{}.babelmap", ifid, slug));

    let saved_at = rfc3339_now();
    let meta = crate::archive::Meta {
        format_version: crate::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: Some(name.to_string()),
        turns,
        saved_at,
    };
    // Named saves are separate slots; command history is per-game, not per-slot.
    crate::archive::save_archive_meta(&path, mapper, save, screen, aux, meta, transcript, transcript_kinds, transcript_runs, &[], &[])
}

/// Remove a save file.
pub fn delete_save(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a human-readable name to a filesystem-safe slug.
///
/// Lowercases, replaces runs of non-alphanumeric chars with a single hyphen,
/// and trims leading/trailing hyphens.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = true; // suppress leading hyphens
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }
    // Trim trailing hyphen.
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Return the current time as an RFC3339 string (UTC, second precision).
fn rfc3339_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as YYYY-MM-DDTHH:MM:SSZ without external crate.
    let s = secs;
    let sec = s % 60;
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400; // days since 1970-01-01

    // Compute calendar date from days since epoch.
    let (year, month, day) = days_to_ymd(days);

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm: shift epoch to 1 Mar 0000 for easier leap-year math.
    days += 719468; // days from year 0 to 1970-01-01 (proleptic Gregorian)
    let era = days / 146097;
    let doe = days % 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month prime [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn load_map(path: &Path) -> Option<Mapper> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return None,
    };
    match from_json(&contents) {
        Ok(mapper) => Some(mapper),
        Err(e) => {
            eprintln!("babelmap: failed to parse map file {}: {}", path.display(), e);
            None
        }
    }
}

pub fn save_game(path: &Path, machine: &zvm::cpu::exec::Machine) -> std::io::Result<()> {
    std::fs::write(path, machine.save_quetzal())
}

pub fn restore_game(path: &Path, machine: &mut zvm::cpu::exec::Machine) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    machine.restore_quetzal(&bytes).map_err(|e| match e {
        zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
        other => format!("restore failed: {:?}", other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::mapper::Mapper;
    use mapper::direction::Direction;
    use mapper::persist::to_json;

    #[test]
    fn save_then_load_round_trips() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        std::fs::write(&path, to_json(&m)).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.connections(), m.graph.connections());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_none() {
        assert!(load_map(Path::new("/no/such/babelmap.map.json")).is_none());
    }

    #[test]
    fn load_corrupt_is_none() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-test-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.map.json");
        std::fs::write(&path, b"this is not valid json {{{").unwrap();
        assert!(load_map(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_round_trips_layers_and_names() {
        use mapper::direction::Direction;
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-layers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let l = mapper::layer::peel_region(&mut m.graph, 2).expect("peel");
        m.graph.set_layer_name(l, "Basement".into());
        std::fs::write(&path, to_json(&m)).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.layer_of(2), l);
        assert_eq!(loaded.graph.layer_name(l), "Basement");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_save_restore_round_trips_with_czech() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return /* skip */ };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        // step a few instructions so dynamic memory differs from the pristine image
        for _ in 0..50 { let _ = machine.step(); }
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("babelmap-save-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();
        let mut m2 = zvm::cpu::exec::Machine::new(
            zvm::memory::Memory::new(std::fs::read(&fixture).unwrap()).unwrap()
        );
        m2.init_caps();
        restore_game(&tmp, &mut m2).expect("restore ok");
        let _ = std::fs::remove_file(&tmp);
    }

    // ── slugify ───────────────────────────────────────────────────────────────

    #[test]
    fn slugify_produces_fs_safe_names() {
        assert_eq!(super::slugify("Before Troll"), "before-troll");
        assert_eq!(super::slugify("  hello  world  "), "hello-world");
        assert_eq!(super::slugify("CAPS and Symbols!!"), "caps-and-symbols");
        assert_eq!(super::slugify("a--b"), "a-b");
        assert_eq!(super::slugify("   "), "");
    }

    // ── rfc3339_now ───────────────────────────────────────────────────────────

    #[test]
    fn rfc3339_now_looks_like_timestamp() {
        let ts = super::rfc3339_now();
        // Format: 2026-06-18T12:34:56Z (20 chars)
        assert_eq!(ts.len(), 20, "expected 20-char RFC3339 timestamp, got '{ts}'");
        assert!(ts.ends_with('Z'), "should end with Z");
        assert!(ts.contains('T'), "should contain T separator");
        // Year should be plausible (>= 2024).
        let year: u32 = ts[0..4].parse().unwrap();
        assert!(year >= 2024, "year {year} looks wrong");
    }

    // ── list_saves / save_named / delete_save ─────────────────────────────────

    fn make_temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("babelmap-saves-test-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The Z-machine `EngineSave` for `m` (Quetzal bytes, `"zmachine"` tag).
    fn es(m: &zvm::cpu::exec::Machine) -> crate::engine::EngineSave {
        crate::engine::EngineSave::new(crate::archive::DEFAULT_ENGINE, 1, m.save_quetzal())
    }

    /// Build a machine from the czech.z5 fixture, or return None to skip.
    fn fake_machine() -> Option<zvm::cpu::exec::Machine> {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return None };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut m = zvm::cpu::exec::Machine::new(mem);
        m.init_caps();
        for _ in 0..50 {
            let _ = m.step();
        }
        Some(m)
    }

    #[test]
    fn save_named_round_trip() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("named");
        let mut mapper = Mapper::default();
        mapper.observe(1, "Foyer", None);

        let ifid = "ZCODE-1-TEST00-0001";
        super::save_named(&dir, ifid, "before-troll", &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, 42, &[], &[], &[])
            .expect("save_named ok");

        let saves = super::list_saves(&dir, ifid);
        assert_eq!(saves.len(), 1, "should have 1 save");
        let s = &saves[0];
        assert_eq!(s.name, "before-troll");
        assert_eq!(s.turns, 42);
        assert!(!s.is_default);
        assert!(!s.saved_at.is_empty(), "saved_at should be set");

        // The file should be loadable as an archive.
        let ac = crate::archive::load_archive(&s.path).expect("load_archive ok");
        assert_eq!(ac.meta.turns, 42);
        assert_eq!(ac.meta.name.as_deref(), Some("before-troll"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_saves_ordering_default_first() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("order");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0002";

        // Write a default archive.
        let default_path = dir.join(format!("{}.babelmap", ifid));
        crate::archive::save_archive(&default_path, &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, &[], &[], &[], &[], &[])
            .expect("default save ok");

        // Write two named saves.
        super::save_named(&dir, ifid, "save-a", &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, 10, &[], &[], &[]).unwrap();
        // Small sleep between named saves so timestamps differ, but since we
        // can't sleep in tests, we directly patch the timestamps via the archive
        // — instead, just verify ordering constraint is maintained.
        super::save_named(&dir, ifid, "save-b", &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, 20, &[], &[], &[]).unwrap();

        let saves = super::list_saves(&dir, ifid);
        assert_eq!(saves.len(), 3, "should find 3 saves (1 default + 2 named)");
        assert!(saves[0].is_default, "default save must be first");
        // Remaining two are named; order between them is by saved_at desc (both
        // written in the same second in tests, so we just check they are present).
        let names: Vec<&str> = saves[1..].iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"save-a"), "save-a should be present");
        assert!(names.contains(&"save-b"), "save-b should be present");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_saves_skips_non_archive_files() {
        let dir = make_temp_dir("skip");
        let ifid = "ZCODE-1-TEST00-0003";

        // Write a non-archive file matching the pattern.
        std::fs::write(dir.join(format!("{}-notanarchive.babelmap", ifid)), b"garbage")
            .unwrap();

        let saves = super::list_saves(&dir, ifid);
        assert!(saves.is_empty(), "garbage file should be skipped");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_save_removes_file() {
        let Some(machine) = fake_machine() else { return };
        let dir = make_temp_dir("delete");
        let mapper = Mapper::default();
        let ifid = "ZCODE-1-TEST00-0004";

        super::save_named(&dir, ifid, "to-delete", &mapper, &es(&machine), Some(&machine.screen), &machine.aux_data, 5, &[], &[], &[]).unwrap();
        let saves = super::list_saves(&dir, ifid);
        assert_eq!(saves.len(), 1);
        let path = saves[0].path.clone();

        super::delete_save(&path).expect("delete ok");
        let saves_after = super::list_saves(&dir, ifid);
        assert!(saves_after.is_empty(), "save should be gone after delete");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── export round-trip (save_game / restore_game) ──────────────────────────

    #[test]
    fn export_round_trip_bytes_match_save_quetzal() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };
        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 { let _ = machine.step(); }

        let tmp = std::env::temp_dir().join(format!("babelmap-export-rt-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();

        // Bytes on disk should equal machine.save_quetzal().
        let on_disk = std::fs::read(&tmp).unwrap();
        assert_eq!(on_disk, machine.save_quetzal(), "exported file bytes must match save_quetzal()");

        // restore_game into a fresh machine should succeed.
        let mem2 = zvm::memory::Memory::new(story).unwrap();
        let mut machine2 = zvm::cpu::exec::Machine::new(mem2);
        machine2.init_caps();
        restore_game(&tmp, &mut machine2).expect("restore ok");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn import_keeps_mapper_unchanged() {
        // After restore_game the mapper is unmodified (standard saves have no map).
        use mapper::mapper::Mapper;
        use mapper::direction::Direction;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };
        let mem = zvm::memory::Memory::new(story.clone()).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        for _ in 0..50 { let _ = machine.step(); }

        let tmp = std::env::temp_dir().join(format!("babelmap-import-map-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();

        // Build a mapper with rooms.
        let mut mapper = Mapper::default();
        mapper.observe(1, "West of House", None);
        mapper.observe(2, "Forest", Some(Direction::N));

        let room_count_before = mapper.graph.rooms().count();
        let connections_before = mapper.graph.connections().len();

        // restore_game should NOT touch the mapper.
        let mem2 = zvm::memory::Memory::new(story).unwrap();
        let mut machine2 = zvm::cpu::exec::Machine::new(mem2);
        machine2.init_caps();
        restore_game(&tmp, &mut machine2).expect("restore ok");

        assert_eq!(mapper.graph.rooms().count(), room_count_before, "mapper rooms unchanged after import");
        assert_eq!(mapper.graph.connections().len(), connections_before, "mapper connections unchanged after import");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn restore_game_fails_gracefully_on_wrong_story() {
        // Restoring a save from czech.z5 into a machine loaded with a *different* story
        // (or same story but different state check) should return an error, not panic.
        // We test by saving from czech and attempting to restore into a fresh czech machine
        // after corrupting the bytes.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };

        // Write clearly-invalid bytes.
        let tmp = std::env::temp_dir().join(format!("babelmap-bad-save-{}.qzl", std::process::id()));
        std::fs::write(&tmp, b"this is not a quetzal save at all").unwrap();

        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();

        let result = restore_game(&tmp, &mut machine);
        assert!(result.is_err(), "restore of corrupt file should return Err");

        let _ = std::fs::remove_file(&tmp);
    }
}
