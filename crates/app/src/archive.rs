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
//! IFID computation is wired in. `load_archive` rejects only archives whose
//! `format_version` is GREATER than `CURRENT_FORMAT_VERSION`; older versions load
//! (history is read only when a `history/` index is present), so v1 archives load
//! with empty history.

use std::io::{self, Read, Write};
use std::path::Path;

use mapper::mapper::Mapper;
use mapper::persist::{from_json, to_json};

// ZIP entry names.
const ENTRY_MAP: &str = "map.json";
const ENTRY_SAVE: &str = "game.sav";
const ENTRY_META: &str = "meta.json";
const ENTRY_TRANSCRIPT: &str = "transcript.json";
const ENTRY_SCREEN: &str = "screen.json";
const HISTORY_INDEX: &str = "history/index.json";

pub const CURRENT_FORMAT_VERSION: u32 = 2;

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

/// Transcript payload written to `transcript.json` inside the archive.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TranscriptData {
    lines: Vec<String>,
    kinds: Vec<crate::state::TranscriptKind>,
}

/// Z-machine screen state written to `screen.json` (zvm has no serde, so we
/// mirror the public fields here). Restored on the host-mediated restore paths
/// (Ctrl+R / auto-load) so a once-split game's upper window shows after restore.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ScreenDto {
    upper_window_rows: u16,
    current_window: u8,
    text_style: u8,
    cursor_row: u16,
    cursor_col: u16,
    buffer_mode: bool,
    show_status_requested: bool,
    cols: u16,
    rows: u16,
    cells: Vec<(char, u8)>, // upper-window grid (ch, style) in row-major order
}

impl ScreenDto {
    fn from_screen(s: &zvm::screen::ScreenState) -> Self {
        ScreenDto {
            upper_window_rows: s.upper_window_rows,
            current_window: s.current_window,
            text_style: s.text_style,
            cursor_row: s.cursor_row,
            cursor_col: s.cursor_col,
            buffer_mode: s.buffer_mode,
            show_status_requested: s.show_status_requested,
            cols: s.upper.cols,
            rows: s.upper.rows,
            cells: s.upper.cells.iter().map(|c| (c.ch, c.style)).collect(),
        }
    }

    fn to_screen(&self) -> zvm::screen::ScreenState {
        let mut s = zvm::screen::ScreenState::default();
        s.upper_window_rows = self.upper_window_rows;
        s.current_window = self.current_window;
        s.text_style = self.text_style;
        s.cursor_row = self.cursor_row;
        s.cursor_col = self.cursor_col;
        s.buffer_mode = self.buffer_mode;
        s.show_status_requested = self.show_status_requested;
        s.upper.cols = self.cols;
        s.upper.rows = self.rows;
        s.upper.cells = self
            .cells
            .iter()
            .map(|&(ch, style)| zvm::screen::Cell { ch, style })
            .collect();
        s
    }
}

/// One row of `history/index.json`: per-turn metadata + ordering. The bytes,
/// map JSON, and transcript live in sibling `turn-NNNN.*` entries.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct HistoryIndexEntry {
    turn: u32,
    command: String,
    has_map: bool,
}

#[derive(Debug)]
pub struct ArchiveContents {
    pub mapper: Mapper,
    pub save: Vec<u8>,
    pub meta: Meta,
    /// Console transcript lines (may be empty for archives that pre-date this field).
    pub transcript: Vec<String>,
    /// Parallel kind tag for each transcript entry (same length as `transcript`).
    pub transcript_kinds: Vec<crate::state::TranscriptKind>,
    /// Per-turn rewind/replay history (empty for archives without `history/`).
    pub history: Vec<crate::history::TurnRecord>,
    /// Saved Z-machine screen state (None for archives without `screen.json`).
    /// Applied on the host-mediated restore paths so the upper window is restored.
    pub screen: Option<zvm::screen::ScreenState>,
}

/// Write a `.babelmap` archive containing the current map and VM save.
///
/// The `machine` save bytes are produced via `Machine::save_quetzal`, the same
/// method used by `persist_files::save_game`. No save logic is duplicated here.
pub fn save_archive(
    path: &Path,
    mapper: &Mapper,
    machine: &zvm::cpu::exec::Machine,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    history: &[crate::history::TurnRecord],
) -> io::Result<()> {
    save_archive_meta(path, mapper, machine, Meta {
        format_version: CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
    }, transcript, transcript_kinds, history)
}

/// Write a `.babelmap` archive with explicit metadata (name, turns, saved_at).
///
/// Used by `persist_files::save_named` to attach save slot information.
pub fn save_archive_meta(
    path: &Path,
    mapper: &Mapper,
    machine: &zvm::cpu::exec::Machine,
    meta: Meta,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    history: &[crate::history::TurnRecord],
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

    // transcript.json
    let td = TranscriptData {
        lines: transcript.to_vec(),
        kinds: transcript_kinds.to_vec(),
    };
    let transcript_json =
        serde_json::to_string_pretty(&td).expect("TranscriptData is always serializable");
    zip.start_file(ENTRY_TRANSCRIPT, options)?;
    zip.write_all(transcript_json.as_bytes())?;

    // screen.json — Z-machine screen state (for host-mediated restore redraw).
    let screen_json = serde_json::to_string(&ScreenDto::from_screen(&machine.screen))
        .expect("ScreenDto is always serializable");
    zip.start_file(ENTRY_SCREEN, options)?;
    zip.write_all(screen_json.as_bytes())?;

    // history/ — per-turn rewind/replay records (only when non-empty).
    if !history.is_empty() {
        let index: Vec<HistoryIndexEntry> = history
            .iter()
            .map(|r| HistoryIndexEntry {
                turn: r.turn,
                command: r.command.clone(),
                has_map: r.map_snapshot.is_some(),
            })
            .collect();
        let index_json =
            serde_json::to_string_pretty(&index).expect("history index serializable");
        zip.start_file(HISTORY_INDEX, options)?;
        zip.write_all(index_json.as_bytes())?;

        for r in history {
            zip.start_file(format!("history/turn-{:04}.sav", r.turn), options)?;
            zip.write_all(&r.save)?;
            if let Some(map) = &r.map_snapshot {
                zip.start_file(format!("history/turn-{:04}.map.json", r.turn), options)?;
                zip.write_all(map.as_bytes())?;
            }
            zip.start_file(format!("history/turn-{:04}.txt", r.turn), options)?;
            zip.write_all(r.transcript.as_bytes())?;
        }
    }

    zip.finish()?;
    Ok(())
}

/// Read a `.babelmap` archive.
///
/// Returns `Err` if the file is missing, corrupt, an entry is absent, or
/// `meta.format_version` is greater than `CURRENT_FORMAT_VERSION`. The caller
/// restores the VM save via `machine.restore_file(&contents.save)`.
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

    if meta.format_version > CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected <= {}",
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

    // transcript.json — optional; older archives omit it, default to empty vecs.
    let (transcript, transcript_kinds) = match zip.by_name(ENTRY_TRANSCRIPT) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            match serde_json::from_str::<TranscriptData>(&buf) {
                Ok(td) => (td.lines, td.kinds),
                Err(_) => (Vec::new(), Vec::new()),
            }
        }
        Err(_) => (Vec::new(), Vec::new()),
    };

    // history/ — optional; absent in archives that pre-date this feature.
    // Read the index first (releasing its borrow before the per-turn reads).
    let history_index: Option<Vec<HistoryIndexEntry>> = match zip.by_name(HISTORY_INDEX) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            Some(serde_json::from_str(&buf).unwrap_or_default())
        }
        Err(_) => None,
    };
    let history: Vec<crate::history::TurnRecord> = match history_index {
        Some(index) => {
            let mut out = Vec::with_capacity(index.len());
            for e in index {
                let save = {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.sav", e.turn)) {
                        let _ = z.read_to_end(&mut b);
                    }
                    b
                };
                let map_snapshot = if e.has_map {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.map.json", e.turn)) {
                        let _ = z.read_to_end(&mut b);
                    }
                    String::from_utf8(b).ok()
                } else {
                    None
                };
                let transcript = {
                    let mut b = Vec::new();
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.txt", e.turn)) {
                        let _ = z.read_to_end(&mut b);
                    }
                    String::from_utf8(b).unwrap_or_default()
                };
                out.push(crate::history::TurnRecord {
                    turn: e.turn,
                    command: e.command,
                    save,
                    map_snapshot,
                    transcript,
                });
            }
            out
        }
        None => Vec::new(),
    };

    // screen.json — saved Z-machine screen state (absent in pre-screen archives).
    let screen = {
        let mut b = Vec::new();
        if let Ok(mut z) = zip.by_name(ENTRY_SCREEN) {
            if z.read_to_end(&mut b).is_ok() {
                serde_json::from_slice::<ScreenDto>(&b).ok().map(|d| d.to_screen())
            } else {
                None
            }
        } else {
            None
        }
    };

    Ok(ArchiveContents { mapper, save, meta, transcript, transcript_kinds, history, screen })
}

/// Read raw Quetzal bytes from a save file for an in-game RESTORE.
///
/// If `path` is a `.babelmap` ZIP archive, returns its `game.sav` entry;
/// otherwise returns the file's raw bytes (a plain `.qzl` Quetzal save).
pub fn read_quetzal_from_file(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        if let Ok(mut entry) = zip.by_name(ENTRY_SAVE) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Ok(bytes)
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

    #[test]
    fn read_quetzal_extracts_game_sav_from_babelmap() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let machine = dummy_machine();
        let expected = machine.save_quetzal();

        let path = temp_archive_path("qzl-from-babelmap");
        save_archive(&path, &small_mapper(), &machine, &[], &[], &[]).expect("save_archive");
        let got = read_quetzal_from_file(&path).expect("read_quetzal_from_file");
        let _ = std::fs::remove_file(&path);

        assert_eq!(got, expected, "game.sav bytes extracted from the .babelmap");
    }

    #[test]
    fn read_quetzal_returns_raw_bytes_for_plain_qzl() {
        // A non-zip file (a plain .qzl) returns its raw bytes unchanged.
        let path = temp_archive_path("plain-qzl");
        std::fs::write(&path, b"FORM\x00\x00fake-quetzal").unwrap();
        let got = read_quetzal_from_file(&path).expect("read raw");
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, b"FORM\x00\x00fake-quetzal");
    }

    fn dummy_machine() -> zvm::cpu::exec::Machine {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let story = std::fs::read(&fixture).expect("czech.z5 fixture for archive tests");
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut m = zvm::cpu::exec::Machine::new(mem);
        m.init_caps();
        m
    }

    #[test]
    fn screen_state_round_trips_through_archive() {
        let mut machine = dummy_machine();
        machine.screen.upper_window_rows = 1;
        machine.screen.upper.resize(1, 6);
        machine.screen.upper.put(1, 2, 'Z', 2);
        machine.screen.current_window = 1;
        machine.screen.cursor_col = 3;

        let path = temp_archive_path("screen-roundtrip");
        save_archive(&path, &small_mapper(), &machine, &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        let scr = ac.screen.expect("screen.json present and restored");
        assert_eq!(scr.upper_window_rows, 1, "split height round-trips");
        assert_eq!(scr.current_window, 1, "current window round-trips");
        assert_eq!(scr.cursor_col, 3, "cursor round-trips");
        assert_eq!(scr.upper.cell(1, 2).ch, 'Z', "grid glyph round-trips");
        assert_eq!(scr.upper.cell(1, 2).style, 2, "grid style round-trips");
    }

    #[test]
    fn history_round_trips_in_archive() {
        use crate::history::TurnRecord;

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        let mapper = small_mapper();
        let map_json = mapper::persist::to_json(&mapper);
        let history = vec![
            TurnRecord { turn: 1, command: "look".into(), save: vec![1, 2, 3],
                map_snapshot: Some(map_json.clone()), transcript: "West of House".into() },
            TurnRecord { turn: 2, command: "wait".into(), save: vec![4, 5, 6, 7],
                map_snapshot: None, transcript: "Time passes.".into() },
        ];

        let path = temp_archive_path("history-rt");
        save_archive(&path, &mapper, &dummy_machine(), &[], &[], &history)
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        assert_eq!(ac.history.len(), 2);
        assert_eq!(ac.history[0].turn, 1);
        assert_eq!(ac.history[0].command, "look");
        assert_eq!(ac.history[0].save, vec![1, 2, 3], "save bytes byte-identical");
        assert_eq!(ac.history[0].map_snapshot.as_deref(), Some(map_json.as_str()));
        assert_eq!(ac.history[0].transcript, "West of House");
        assert_eq!(ac.history[1].save, vec![4, 5, 6, 7]);
        assert!(ac.history[1].map_snapshot.is_none(), "no-change turn has no map");
        assert_eq!(ac.history[1].transcript, "Time passes.");
    }

    #[test]
    fn v1_archive_loads_with_empty_history() {
        // An archive with no history/ entries (e.g. written before this feature)
        // loads with an empty history and unchanged behavior.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let mapper = small_mapper();
        let path = temp_archive_path("history-v1");
        save_archive(&path, &mapper, &dummy_machine(), &[], &[], &[])
            .expect("save_archive without history");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        assert!(ac.history.is_empty(), "archive without history/ → empty history");
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
        save_archive(&path, &mapper, &machine, &[], &[], &[]).expect("save_archive");

        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        // Map round-trips via JSON comparison (same as persist_files tests)
        assert_eq!(to_json(&ac.mapper), expected_map_json, "map JSON must match");

        // Save bytes are byte-identical
        assert_eq!(ac.save, expected_save, "save bytes must be identical");

        // Meta
        assert_eq!(ac.meta.format_version, CURRENT_FORMAT_VERSION);
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
    // transcript round-trip: lines + kinds survive write-read cycle
    // -------------------------------------------------------------------------
    #[test]
    fn transcript_round_trip() {
        use crate::state::TranscriptKind;
        use std::io::Write as _;

        let path = temp_archive_path("transcript-rt");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            // meta.json
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new() };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();

            // map.json
            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            // game.sav
            zip.start_file(ENTRY_SAVE, options).unwrap();
            zip.write_all(&[]).unwrap();

            // transcript.json with mixed Story/Meta entries
            let td = TranscriptData {
                lines: vec!["West of House".to_string(), "/help".to_string(), "You are standing...".to_string()],
                kinds: vec![TranscriptKind::Story, TranscriptKind::Meta, TranscriptKind::Story],
            };
            let transcript_json = serde_json::to_string(&td).unwrap();
            zip.start_file(ENTRY_TRANSCRIPT, options).unwrap();
            zip.write_all(transcript_json.as_bytes()).unwrap();

            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);

        assert_eq!(ac.transcript, vec!["West of House", "/help", "You are standing..."]);
        assert_eq!(ac.transcript_kinds, vec![TranscriptKind::Story, TranscriptKind::Meta, TranscriptKind::Story]);
        assert_eq!(ac.transcript.len(), ac.transcript_kinds.len(), "vecs must be equal length");
    }

    // -------------------------------------------------------------------------
    // missing transcript entry -> empty vecs (graceful default for old archives)
    // -------------------------------------------------------------------------
    #[test]
    fn missing_transcript_defaults_to_empty() {
        use std::io::Write as _;

        let path = temp_archive_path("transcript-missing");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();

            // Write an archive with no transcript.json entry.
            let meta = Meta { format_version: 1, ifid: None, name: None, turns: 0, saved_at: String::new() };
            let meta_json = serde_json::to_string(&meta).unwrap();
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(meta_json.as_bytes()).unwrap();

            let mapper = Mapper::default();
            let map_json = mapper::persist::to_json(&mapper);
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(map_json.as_bytes()).unwrap();

            zip.start_file(ENTRY_SAVE, options).unwrap();
            zip.write_all(&[]).unwrap();

            // No ENTRY_TRANSCRIPT written.
            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("archive without transcript must load");
        let _ = std::fs::remove_file(&path);

        assert!(ac.transcript.is_empty(), "transcript must default to empty");
        assert!(ac.transcript_kinds.is_empty(), "transcript_kinds must default to empty");
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
