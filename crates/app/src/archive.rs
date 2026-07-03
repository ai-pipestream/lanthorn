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

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;

use mapper::mapper::Mapper;
use mapper::persist::{from_json, to_json};

use crate::engine::EngineSave;

// ZIP entry names.
const ENTRY_MAP: &str = "map.json";
const ENTRY_META: &str = "meta.json";
const ENTRY_TRANSCRIPT: &str = "transcript.json";
const ENTRY_COMMAND_HISTORY: &str = "command_history.json";
const ENTRY_SCREEN: &str = "screen.json";
const ENTRY_AUX: &str = "aux.dat";
/// Engine tag (the `EngineSave` engine string) the save was written by.
const ENTRY_ENGINE: &str = "engine.txt";

/// The engine tag stamped into archives written before the tag existed (and the
/// only engine that exists today). Used as the default when an archive has no
/// `engine.txt` entry, so legacy saves restore unchanged.
pub const DEFAULT_ENGINE: &str = "zmachine";

/// The inner save-entry extension for an engine tag, matching the raw
/// interchange extensions: Z-machine Quetzal is `qzl`, Glulx is `glksave`. The
/// archive's `game.<ext>` and per-turn `history/turn-NNNN.<ext>` entries use it.
fn save_ext(engine: &str) -> &'static str {
    if engine == "glulx" { "glksave" } else { "qzl" }
}

/// Whether an archive written by `archive_engine` may be restored while the app
/// is running `current_engine`. The `.babelmap` archive records the engine that
/// produced the save (see [`ArchiveContents::engine`]); a save from a different
/// engine is refused so a future Glulx save can't be fed to the Z-machine (or
/// vice-versa). Only `"zmachine"` exists today, so this always allows in 3b-i.
pub fn restore_engine_allowed(archive_engine: &str, current_engine: &str) -> Result<(), String> {
    if archive_engine == current_engine {
        Ok(())
    } else {
        Err(format!(
            "this save was written by the \"{archive_engine}\" engine, but babelmap is running the \"{current_engine}\" engine"
        ))
    }
}
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
    /// Per-line Z-machine style runs, parallel to `lines`. Defaults to empty for
    /// archives written before this field existed (back-compatible load).
    #[serde(default)]
    runs: Vec<Vec<crate::state::StyleRun>>,
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
        zvm::screen::ScreenState {
            upper_window_rows: self.upper_window_rows,
            current_window: self.current_window,
            text_style: self.text_style,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            buffer_mode: self.buffer_mode,
            show_status_requested: self.show_status_requested,
            upper: zvm::screen::UpperWindow {
                cols: self.cols,
                rows: self.rows,
                cells: self
                    .cells
                    .iter()
                    .map(|&(ch, style)| zvm::screen::Cell { ch, style, fg: zvm::screen::ZColour::Default, bg: zvm::screen::ZColour::Default })
                    .collect(),
            },
            ..Default::default()
        }
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
    /// Parallel per-line Z-machine style runs (same length as `transcript`;
    /// empty per line for archives that pre-date this field).
    pub transcript_runs: Vec<Vec<crate::state::StyleRun>>,
    /// Per-turn rewind/replay history (empty for archives without `history/`).
    pub history: Vec<crate::history::TurnRecord>,
    /// Saved Z-machine screen state (None for archives without `screen.json`).
    /// Applied on the host-mediated restore paths so the upper window is restored.
    pub screen: Option<zvm::screen::ScreenState>,
    /// Auxiliary key/value data from the machine (empty for archives without `aux.dat`).
    pub aux: std::collections::BTreeMap<String, Vec<u8>>,
    /// Shell-style command history (empty for archives without `command_history.json`).
    pub command_history: Vec<String>,
    /// The engine that wrote the save (`engine.txt`), defaulting to
    /// [`DEFAULT_ENGINE`] for archives written before the tag existed. The
    /// restore path refuses a save from a different engine (see
    /// [`restore_engine_allowed`]).
    pub engine: String,
}

/// Write a `.babelmap` archive containing the current map and VM save.
///
/// `save` is the engine-tagged game state (from `Engine::save_state`); its
/// `bytes` become `game.qzl` (Z-machine) or `game.glksave` (Glulx), and the
/// `engine` tag becomes `engine.txt`. `screen`
/// is the Z-machine `ScreenState` written to `screen.json` — `Some` only for the
/// Z-machine (Glulx keeps its display inside `save.bytes`). `aux` is the engine's
/// auxiliary key/value table.
pub fn save_archive(
    path: &Path,
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    transcript_runs: &[Vec<crate::state::StyleRun>],
    history: &[crate::history::TurnRecord],
    command_history: &[String],
) -> io::Result<()> {
    save_archive_meta(path, mapper, save, screen, aux, Meta {
        format_version: CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
    }, transcript, transcript_kinds, transcript_runs, history, command_history)
}

/// Write a `.babelmap` archive with explicit metadata (name, turns, saved_at).
///
/// Used by `persist_files::save_named` to attach save slot information. See
/// [`save_archive`] for the `save`/`screen`/`aux` parameters.
pub fn save_archive_meta(
    path: &Path,
    mapper: &Mapper,
    save: &EngineSave,
    screen: Option<&zvm::screen::ScreenState>,
    aux: &BTreeMap<String, Vec<u8>>,
    meta: Meta,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    transcript_runs: &[Vec<crate::state::StyleRun>],
    history: &[crate::history::TurnRecord],
    command_history: &[String],
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

    // game.<ext> — the engine-tagged save bytes (Quetzal): game.qzl for the
    // Z-machine, game.glksave for Glulx, matching the raw interchange extension.
    zip.start_file(format!("game.{}", save_ext(&save.engine)), options)?;
    zip.write_all(&save.bytes)?;

    // engine.txt — the EngineSave tag (which engine produced this save).
    zip.start_file(ENTRY_ENGINE, options)?;
    zip.write_all(save.engine.as_bytes())?;

    // meta.json
    let meta_json =
        serde_json::to_string_pretty(&meta).expect("Meta is always serializable");
    zip.start_file(ENTRY_META, options)?;
    zip.write_all(meta_json.as_bytes())?;

    // transcript.json — persist only the story text the player saw (Story + the
    // player's own Input), dropping Meta/Warning lines (slash-command output,
    // diagnostics) that aren't part of the game's narrative.
    use crate::state::TranscriptKind;
    let mut lines: Vec<String> = Vec::new();
    let mut kinds: Vec<TranscriptKind> = Vec::new();
    let mut runs: Vec<Vec<crate::state::StyleRun>> = Vec::new();
    for (i, (line, &k)) in transcript.iter().zip(transcript_kinds.iter()).enumerate() {
        if matches!(k, TranscriptKind::Story | TranscriptKind::Input) {
            lines.push(line.clone());
            kinds.push(k);
            runs.push(transcript_runs.get(i).cloned().unwrap_or_default());
        }
    }
    let td = TranscriptData { lines, kinds, runs };
    let transcript_json =
        serde_json::to_string_pretty(&td).expect("TranscriptData is always serializable");
    zip.start_file(ENTRY_TRANSCRIPT, options)?;
    zip.write_all(transcript_json.as_bytes())?;

    // command_history.json — the player's submitted command lines (JSON array).
    let cmd_history_json = serde_json::to_string_pretty(command_history)
        .expect("Vec<String> is always serializable");
    zip.start_file(ENTRY_COMMAND_HISTORY, options)?;
    zip.write_all(cmd_history_json.as_bytes())?;

    // screen.json — Z-machine screen state (for host-mediated restore redraw).
    // Z-machine-only: Glulx passes `None` (its display lives inside save.bytes).
    if let Some(scr) = screen {
        let screen_json = serde_json::to_string(&ScreenDto::from_screen(scr))
            .expect("ScreenDto is always serializable");
        zip.start_file(ENTRY_SCREEN, options)?;
        zip.write_all(screen_json.as_bytes())?;
    }

    // aux.dat — engine aux data (only when non-empty).
    if !aux.is_empty() {
        zip.start_file(ENTRY_AUX, options)?;
        zip.write_all(&crate::aux_store::encode_aux(aux))?;
    }

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

        let ext = save_ext(&save.engine);
        for r in history {
            zip.start_file(format!("history/turn-{:04}.{}", r.turn, ext), options)?;
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

    // engine.txt — which engine produced this save; drives the save entry name
    // (game.qzl / game.glksave). Absent → default engine.
    let engine = match zip.by_name(ENTRY_ENGINE) {
        Ok(mut entry) => {
            let mut buf = String::new();
            let _ = entry.read_to_string(&mut buf);
            let t = buf.trim();
            if t.is_empty() { DEFAULT_ENGINE.to_string() } else { t.to_string() }
        }
        Err(_) => DEFAULT_ENGINE.to_string(),
    };

    // game.<ext> — the engine-tagged save bytes.
    let save = {
        let name = format!("game.{}", save_ext(&engine));
        let mut entry = zip.by_name(&name).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}: {e}"))
        })?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        buf
    };

    // transcript.json — optional; older archives omit it, default to empty vecs.
    let (transcript, transcript_kinds, transcript_runs) = match zip.by_name(ENTRY_TRANSCRIPT) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            match serde_json::from_str::<TranscriptData>(&buf) {
                Ok(td) => {
                    // Keep runs length-synced with lines: archives that pre-date
                    // the runs field (or with a mismatched length) get one empty
                    // run vec per line.
                    let runs = if td.runs.len() == td.lines.len() {
                        td.runs
                    } else {
                        vec![Vec::new(); td.lines.len()]
                    };
                    (td.lines, td.kinds, runs)
                }
                Err(_) => (Vec::new(), Vec::new(), Vec::new()),
            }
        }
        Err(_) => (Vec::new(), Vec::new(), Vec::new()),
    };

    // command_history.json — optional; older archives omit it → empty vec.
    let command_history: Vec<String> = match zip.by_name(ENTRY_COMMAND_HISTORY) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            serde_json::from_str(&buf).unwrap_or_default()
        }
        Err(_) => Vec::new(),
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
                    if let Ok(mut z) = zip.by_name(&format!("history/turn-{:04}.{}", e.turn, save_ext(&engine))) {
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

    // aux.dat — optional; absent in pre-aux archives → empty map.
    let aux = match zip.by_name(ENTRY_AUX) {
        Ok(mut entry) => {
            let mut buf = Vec::new();
            let _ = entry.read_to_end(&mut buf);
            crate::aux_store::decode_aux(&buf)
        }
        Err(_) => std::collections::BTreeMap::new(),
    };

    Ok(ArchiveContents { mapper, save, meta, transcript, transcript_kinds, transcript_runs, history, screen, aux, command_history, engine })
}

/// Read ONLY the `meta.json` entry from a save archive — avoids `load_archive`
/// unzipping the map, save image, transcript, history, screen, and aux just to
/// show a save summary. Applies the same `format_version` rejection as
/// `load_archive`, so a future-format archive is reported as an error (and thus
/// skipped by `list_saves`) exactly as today.
pub fn read_archive_meta(path: &Path) -> io::Result<Meta> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let meta: Meta = {
        let mut entry = zip.by_name(ENTRY_META).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_META}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_META}: {e}"))
        })?
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
    Ok(meta)
}

impl ArchiveContents {
    /// The persisted game state as an engine-tagged [`EngineSave`], rebuilt from
    /// the archive's `game.<ext>` bytes + `engine.txt` tag (defaulting to
    /// [`DEFAULT_ENGINE`] for legacy archives). The save-format version is not
    /// stored in the archive — the engine ignores it on restore — so a
    /// placeholder is used. Feed this to `Engine::restore_state`, which refuses a
    /// foreign-engine save via [`restore_engine_allowed`]'s equivalent guard.
    pub fn engine_save(&self) -> EngineSave {
        EngineSave::new(self.engine.clone(), 1, self.save.clone())
    }
}

/// Read raw Quetzal bytes from a save file for an in-game RESTORE.
///
/// If `path` is a `.babelmap` ZIP archive, returns its `game.qzl`/`game.glksave`
/// entry;
/// otherwise returns the file's raw bytes (a plain `.qzl` Quetzal save).
pub fn read_quetzal_from_file(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        for name in ["game.qzl", "game.glksave"] {
            if let Ok(mut entry) = zip.by_name(name) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                return Ok(buf);
            }
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

    /// The Z-machine `EngineSave` for `machine` (Quetzal bytes, `"zmachine"` tag).
    fn zvm_es(machine: &zvm::cpu::exec::Machine) -> EngineSave {
        EngineSave::new(DEFAULT_ENGINE, 1, machine.save_quetzal())
    }

    /// Test helper: write an archive from a Z-machine `machine` (the old call
    /// shape), building the `EngineSave` + screen + aux the production fn now
    /// takes separately.
    #[allow(clippy::too_many_arguments)]
    fn save_archive_m(
        path: &Path,
        mapper: &Mapper,
        machine: &zvm::cpu::exec::Machine,
        transcript: &[String],
        kinds: &[crate::state::TranscriptKind],
        runs: &[Vec<crate::state::StyleRun>],
        history: &[crate::history::TurnRecord],
        cmds: &[String],
    ) -> io::Result<()> {
        save_archive(path, mapper, &zvm_es(machine), Some(&machine.screen), &machine.aux_data,
            transcript, kinds, runs, history, cmds)
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
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save_archive");
        let got = read_quetzal_from_file(&path).expect("read_quetzal_from_file");
        let _ = std::fs::remove_file(&path);

        assert_eq!(got, expected, "game.qzl bytes extracted from the .babelmap");
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

    #[test]
    fn restore_engine_allowed_refuses_foreign_tag() {
        // The running engine restoring its own save is allowed.
        assert!(restore_engine_allowed(DEFAULT_ENGINE, DEFAULT_ENGINE).is_ok());
        // A save written by a different (faked) engine is refused.
        let err = restore_engine_allowed("glulx", DEFAULT_ENGINE)
            .expect_err("a foreign-engine save must be refused");
        assert!(err.contains("glulx") && err.contains(DEFAULT_ENGINE), "message names both engines: {err}");
    }

    #[test]
    fn archive_records_and_loads_engine_tag() {
        let machine = dummy_machine();
        let path = temp_archive_path("engine-tag");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE, "archive records the zmachine engine tag");
        assert!(restore_engine_allowed(&ac.engine, DEFAULT_ENGINE).is_ok());
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
        machine.screen.upper.put(1, 2, 'Z', 2, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        machine.screen.current_window = 1;
        machine.screen.cursor_col = 3;

        let path = temp_archive_path("screen-roundtrip");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
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
    fn save_drops_meta_and_warning_transcript_lines() {
        use crate::state::TranscriptKind;

        let transcript = vec![
            "West of House".to_string(),
            "> open mailbox".to_string(),
            "/help".to_string(),
            "save failed".to_string(),
            "You open the mailbox.".to_string(),
        ];
        let kinds = vec![
            TranscriptKind::Story,
            TranscriptKind::Input,
            TranscriptKind::Meta,
            TranscriptKind::Warning,
            TranscriptKind::Story,
        ];

        let path = temp_archive_path("transcript-filter");
        let machine = dummy_machine();
        save_archive_meta(
            &path,
            &small_mapper(),
            &zvm_es(&machine),
            Some(&machine.screen),
            &machine.aux_data,
            Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new() },
            &transcript,
            &kinds,
            &[],
            &[],
            &[],
        )
        .expect("save_archive_meta");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        // Only Story + Input survive; Meta/Warning are dropped.
        assert_eq!(
            ac.transcript,
            vec!["West of House", "> open mailbox", "You open the mailbox."]
        );
        assert_eq!(
            ac.transcript_kinds,
            vec![TranscriptKind::Story, TranscriptKind::Input, TranscriptKind::Story]
        );
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
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &history, &[])
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
        save_archive_m(&path, &mapper, &dummy_machine(), &[], &[], &[], &[], &[])
            .expect("save_archive without history");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        assert!(ac.history.is_empty(), "archive without history/ → empty history");
    }

    #[test]
    fn command_history_round_trips_in_archive() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let cmds = vec!["look".to_string(), "open mailbox".to_string(), "/help".to_string()];
        let path = temp_archive_path("cmd-history-rt");
        save_archive_m(&path, &small_mapper(), &dummy_machine(), &[], &[], &[], &[], &cmds)
            .expect("save_archive");
        let ac = load_archive(&path).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.command_history, cmds);
    }

    #[test]
    fn archive_without_command_history_loads_empty() {
        // Simulate a pre-feature archive by removing the command_history.json entry.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let path = temp_archive_path("cmd-history-missing");
        save_archive_m(&path, &small_mapper(), &dummy_machine(), &[], &[], &[], &[],
            &["north".to_string()])
            .expect("save_archive");

        // Rewrite the archive dropping the command_history.json entry.
        let stripped = temp_archive_path("cmd-history-stripped");
        {
            let src = std::fs::File::open(&path).unwrap();
            let mut zin = zip::ZipArchive::new(src).unwrap();
            let out = std::fs::File::create(&stripped).unwrap();
            let mut zout = zip::ZipWriter::new(out);
            for i in 0..zin.len() {
                let mut e = zin.by_index(i).unwrap();
                if e.name() == ENTRY_COMMAND_HISTORY {
                    continue;
                }
                let name = e.name().to_string();
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                zout.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                zout.write_all(&buf).unwrap();
            }
            zout.finish().unwrap();
        }
        let ac = load_archive(&stripped).expect("load_archive");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&stripped);
        assert!(ac.command_history.is_empty(), "missing entry → empty command history");
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
        save_archive_m(&path, &mapper, &machine, &[], &[], &[], &[], &[]).expect("save_archive");

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
    // read_archive_meta: cheap meta-only read matches load_archive's meta
    // -------------------------------------------------------------------------
    #[test]
    fn read_archive_meta_matches_load_archive_meta() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }

        let path = temp_archive_path("meta-only");
        let machine = dummy_machine();
        save_archive_meta(
            &path,
            &small_mapper(),
            &zvm_es(&machine),
            Some(&machine.screen),
            &machine.aux_data,
            Meta {
                format_version: CURRENT_FORMAT_VERSION,
                ifid: Some("ZCODE-1-000000-0000".to_string()),
                name: Some("before-troll".to_string()),
                turns: 42,
                saved_at: "2026-06-30T12:00:00Z".to_string(),
            },
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("save_archive_meta");

        let full = load_archive(&path).unwrap().meta;
        let quick = read_archive_meta(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(quick.format_version, full.format_version);
        assert_eq!(quick.ifid, full.ifid);
        assert_eq!(quick.name, full.name);
        assert_eq!(quick.turns, full.turns);
        assert_eq!(quick.saved_at, full.saved_at);
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
            zip.start_file("game.qzl", options).unwrap();
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
            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&[]).unwrap();

            // transcript.json with mixed Story/Meta entries
            let td = TranscriptData {
                lines: vec!["West of House".to_string(), "/help".to_string(), "You are standing...".to_string()],
                kinds: vec![TranscriptKind::Story, TranscriptKind::Meta, TranscriptKind::Story],
                runs: Vec::new(),
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

    #[test]
    fn transcript_data_round_trips_runs() {
        use crate::state::{StyleRun, TranscriptKind};
        let td = TranscriptData {
            lines: vec!["a".into(), "b".into()],
            kinds: vec![TranscriptKind::Story, TranscriptKind::Input],
            runs: vec![vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0 }], vec![]],
        };
        let json = serde_json::to_string(&td).unwrap();
        let back: TranscriptData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.runs, td.runs);
    }

    #[test]
    fn old_transcript_json_loads_with_empty_runs() {
        // JSON without a "runs" field (older archive)
        let json = r#"{"lines":["x"],"kinds":["Story"]}"#;
        let td: TranscriptData = serde_json::from_str(json).unwrap();
        assert!(td.runs.is_empty());
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

            zip.start_file("game.qzl", options).unwrap();
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

    #[test]
    fn archive_round_trips_aux_data() {
        let mut machine = dummy_machine();
        machine.aux_data.insert("hints".to_string(), vec![1, 2, 3]);
        let path = temp_archive_path("aux");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.aux.get("hints").map(|v| v.as_slice()), Some(&[1, 2, 3][..]));
    }

    /// Read a single ZIP entry's bytes, or `None` when the entry is absent.
    fn read_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
        let file = std::fs::File::open(path).ok()?;
        let mut zip = zip::ZipArchive::new(file).ok()?;
        let mut e = zip.by_name(name).ok()?;
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).ok()?;
        Some(buf)
    }

    #[test]
    fn engine_save_round_trips_through_archive() {
        // A zvm-tagged EngineSave writes game.sav == its bytes + engine.txt ==
        // "zmachine"; a Some(screen) writes screen.json; load returns the tag,
        // bytes, and screen.
        let machine = dummy_machine();
        let es = zvm_es(&machine);
        let path = temp_archive_path("engine-save-rt");
        save_archive(&path, &small_mapper(), &es, Some(&machine.screen), &machine.aux_data,
            &[], &[], &[], &[], &[]).expect("save");

        assert_eq!(read_entry(&path, "game.qzl").as_deref(), Some(es.bytes.as_slice()),
            "game.qzl holds the EngineSave bytes");
        assert_eq!(read_entry(&path, ENTRY_ENGINE).as_deref(), Some(DEFAULT_ENGINE.as_bytes()),
            "engine.txt holds the engine tag");
        assert!(read_entry(&path, ENTRY_SCREEN).is_some(), "screen.json written for zvm");

        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE);
        assert_eq!(ac.save, es.bytes, "loaded save bytes match");
        assert!(ac.screen.is_some(), "screen restored");
        assert_eq!(ac.engine_save(), es, "engine_save() reconstructs the EngineSave");
    }

    #[test]
    fn glulx_tagged_save_round_trips_without_screen() {
        // A "glulx"-tagged save (no screen) writes NO screen.json; load reports
        // the glulx tag, the bytes, and screen == None.
        let es = EngineSave::new("glulx", 1, vec![9, 8, 7, 6]);
        let path = temp_archive_path("glulx-no-screen");
        save_archive(&path, &small_mapper(), &es, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[]).expect("save");

        assert!(read_entry(&path, ENTRY_SCREEN).is_none(), "no screen.json for glulx");
        assert_eq!(read_entry(&path, ENTRY_ENGINE).as_deref(), Some(b"glulx".as_slice()));

        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, "glulx");
        assert_eq!(ac.save, vec![9, 8, 7, 6]);
        assert!(ac.screen.is_none(), "glulx archive carries no screen");
    }

    #[test]
    fn inner_save_entry_extension_matches_engine() {
        // The inner save entry is named for the engine's format: game.qzl for the
        // Z-machine, game.glksave for Glulx, and never the old game.sav.
        let zm = EngineSave::new(DEFAULT_ENGINE, 1, vec![1, 2, 3]);
        let zpath = temp_archive_path("entry-ext-zm");
        save_archive(&zpath, &small_mapper(), &zm, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[]).expect("save zm");
        assert!(read_entry(&zpath, "game.qzl").is_some(), "Z-machine save entry is game.qzl");
        assert!(read_entry(&zpath, "game.sav").is_none(), "no legacy game.sav entry");
        assert!(read_entry(&zpath, "game.glksave").is_none());
        assert_eq!(load_archive(&zpath).unwrap().save, vec![1, 2, 3], "zm round-trips");
        let _ = std::fs::remove_file(&zpath);

        let gl = EngineSave::new("glulx", 1, vec![4, 5, 6]);
        let gpath = temp_archive_path("entry-ext-gl");
        save_archive(&gpath, &small_mapper(), &gl, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[]).expect("save gl");
        assert!(read_entry(&gpath, "game.glksave").is_some(), "Glulx save entry is game.glksave");
        assert!(read_entry(&gpath, "game.qzl").is_none());
        assert!(read_entry(&gpath, "game.sav").is_none(), "no legacy game.sav entry");
        assert_eq!(load_archive(&gpath).unwrap().save, vec![4, 5, 6], "glulx round-trips");
        let _ = std::fs::remove_file(&gpath);
    }

    #[test]
    fn archive_without_engine_txt_loads_as_zmachine() {
        // An archive with NO engine.txt must default to the Z-machine: its
        // game.qzl bytes load as EngineSave { "zmachine", <bytes> } + the screen.
        let machine = dummy_machine();
        let quetzal = machine.save_quetzal();
        let path = temp_archive_path("old-no-engine");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            let meta = Meta { format_version: CURRENT_FORMAT_VERSION, ifid: None, name: None, turns: 0, saved_at: String::new() };
            zip.start_file(ENTRY_META, options).unwrap();
            zip.write_all(serde_json::to_string(&meta).unwrap().as_bytes()).unwrap();
            zip.start_file(ENTRY_MAP, options).unwrap();
            zip.write_all(mapper::persist::to_json(&small_mapper()).as_bytes()).unwrap();
            zip.start_file("game.qzl", options).unwrap();
            zip.write_all(&quetzal).unwrap();
            // screen.json present, engine.txt absent (the old format).
            zip.start_file(ENTRY_SCREEN, options).unwrap();
            zip.write_all(serde_json::to_string(&ScreenDto::from_screen(&machine.screen)).unwrap().as_bytes()).unwrap();
            zip.finish().unwrap();
        }

        let ac = load_archive(&path).expect("old archive loads");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, DEFAULT_ENGINE, "absent engine.txt defaults to zmachine");
        assert_eq!(ac.save, quetzal, "raw Quetzal bytes load unchanged");
        assert!(ac.screen.is_some(), "legacy screen.json still loads");
        assert_eq!(ac.engine_save().engine, DEFAULT_ENGINE);
        assert_eq!(ac.engine_save().bytes, quetzal);
    }

    #[test]
    fn archive_without_aux_loads_empty_map() {
        let machine = dummy_machine(); // empty aux_data
        let path = temp_archive_path("noaux");
        save_archive_m(&path, &small_mapper(), &machine, &[], &[], &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.aux.is_empty());
    }
}
