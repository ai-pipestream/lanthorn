//! Pre-game story picker: when a directory is passed at launch instead of a
//! story file, scan it for Z-machine stories and let the user choose one.
//!
//! Titles are resolved cheaply (no game is run): the known-title table keyed by
//! the IFID, falling back to the filename stem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::hints;

/// The VM engine a story runs on (version-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    ZCode,
    Glulx,
}

/// One blorb resource-index entry, string-rendered for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    pub usage: String,      // "Exec" | "Pict" | "Snd " | "Data" …
    pub number: u32,
    pub chunk_type: String, // "ZCOD" | "GLUL" | "PNG " | "OGGV" …
    pub len: usize,
}

/// Best-effort static feature signals. Glulx-unknowable features are `None`/false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features {
    pub sound: bool,
    pub graphics: bool,
    pub colour: Option<bool>, // Z: Some(bit6); Glulx: None (runtime Glk → omit)
    pub hints: bool,          // folded in from StoryAux when the aux resolves
}

/// Eager per-story metadata, derived from bytes `scan_stories` already reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryMeta {
    pub size_bytes: u64,
    pub modified: Option<String>, // "YYYY-MM-DD"
    pub engine: Engine,
    pub format: String,           // "Z-code" | "Glulx" | "Blorb (Z-code)" | "Blorb (Glulx)"
    pub version: Option<String>,  // Z: "3"; Glulx: "3.1.2"
    pub serial: Option<String>,   // Z only
    pub release: Option<u16>,     // Z only
    pub ifid: String,
    pub features: Features,
    pub self_blorb: Option<Vec<ChunkInfo>>, // Some when the story file itself is a blorb
}

/// One selectable story in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryEntry {
    pub path: PathBuf,
    /// Display title: `known_title(ifid)` or the filename stem.
    pub title: String,
    /// The bare filename (e.g. `zork1.z5`), shown beside the title.
    pub filename: String,
    pub meta: StoryMeta,
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

/// True for blorb-container extensions (case-insensitive).
fn is_blorb_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zblorb" | "blorb" | "gblorb" | "blb"))
        .unwrap_or(false)
}

/// Format a `SystemTime` mtime as "YYYY-MM-DD" (UTC, civil-date arithmetic; no
/// chrono dependency). Returns None if the time is before the Unix epoch.
fn format_mtime_ymd(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Z-machine version byte at header offset 0x00.
fn z_version(exec: &[u8]) -> Option<u8> {
    exec.first().copied()
}

/// Z-machine release: big-endian word at header offset 0x02.
fn z_release(exec: &[u8]) -> Option<u16> {
    match (exec.get(0x02), exec.get(0x03)) {
        (Some(&h), Some(&l)) => Some(u16::from_be_bytes([h, l])),
        _ => None,
    }
}

/// Z-machine serial: 6 ASCII bytes at header offset 0x12..0x18.
fn z_serial(exec: &[u8]) -> Option<String> {
    let s = exec.get(0x12..0x18)?;
    Some(String::from_utf8_lossy(s).into_owned())
}

/// Z-machine Flags2: big-endian word at header offset 0x10.
/// bit 3 (0x0008)=graphics, bit 6 (0x0040)=colours, bit 7 (0x0080)=sound.
fn z_flags2(exec: &[u8]) -> u16 {
    match (exec.get(0x10), exec.get(0x11)) {
        (Some(&h), Some(&l)) => u16::from_be_bytes([h, l]),
        _ => 0,
    }
}

/// Glulx version: 16-bit major at 0x04, minor at 0x06, subminor at 0x07 →
/// "major.minor.subminor".
fn glulx_version(exec: &[u8]) -> Option<String> {
    let major = u16::from_be_bytes([*exec.get(0x04)?, *exec.get(0x05)?]);
    let minor = *exec.get(0x06)?;
    let subminor = *exec.get(0x07)?;
    Some(format!("{major}.{minor}.{subminor}"))
}

/// Lazily-resolved, per-highlight data that touches other files/dirs.
pub struct StoryAux {
    /// Sibling/dir-scan blorb resources when the story is NOT itself a blorb.
    /// Carries the source path so the panel can name the file.
    pub assoc_blorb: Option<(PathBuf, Vec<ChunkInfo>)>,
    pub saves: Vec<crate::persist_files::SaveInfo>,
    pub hints_available: bool,
}

/// Resolve the lazy aux for one story. `save_dir` is `user_dir/saves`;
/// `hint_index` is the shared index loaded once at picker start.
pub fn resolve_aux(
    entry: &StoryEntry,
    save_dir: &Path,
    hint_index: &hints::HintIndex,
) -> StoryAux {
    // Only record an ASSOCIATED blorb (a different file); the self-blorb case is
    // already carried in StoryMeta.self_blorb.
    let assoc_blorb = match blorb::resolve_sound_blorb(&entry.path) {
        Some((b, src)) if src != entry.path => Some((src, chunks_of(&b))),
        _ => None,
    };
    let saves = crate::persist_files::list_saves(save_dir, &entry.meta.ifid);
    let hints_available = hint_index.get(&entry.meta.ifid).is_some();
    StoryAux { assoc_blorb, saves, hints_available }
}

/// Convert a parsed blorb's resource index into displayable `ChunkInfo`.
pub fn chunks_of(b: &blorb::Blorb) -> Vec<ChunkInfo> {
    b.resources()
        .iter()
        .map(|r| ChunkInfo {
            usage: String::from_utf8_lossy(&r.usage).into_owned(),
            number: r.number,
            chunk_type: String::from_utf8_lossy(&r.chunk_type).into_owned(),
            len: r.len,
        })
        .collect()
}

/// Eager `Features` for a Z-code exec image, folding in self-blorb resources.
fn z_features(exec: &[u8], self_blorb: Option<&[ChunkInfo]>) -> Features {
    let f2 = z_flags2(exec);
    let mut sound = f2 & 0x0080 != 0;
    let mut graphics = f2 & 0x0008 != 0;
    if let Some(chunks) = self_blorb {
        if chunks.iter().any(|c| c.usage == "Snd ") {
            sound = true;
        }
        if chunks.iter().any(|c| c.usage == "Pict") {
            graphics = true;
        }
    }
    Features { sound, graphics, colour: Some(f2 & 0x0040 != 0), hints: false }
}

/// Eager `Features` for a Glulx story — colour is runtime Glk (None); sound and
/// graphics come from a self-blorb only.
fn glulx_features(self_blorb: Option<&[ChunkInfo]>) -> Features {
    let mut f = Features { sound: false, graphics: false, colour: None, hints: false };
    if let Some(chunks) = self_blorb {
        f.sound = chunks.iter().any(|c| c.usage == "Snd ");
        f.graphics = chunks.iter().any(|c| c.usage == "Pict");
    }
    f
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

        // fs metadata: size + mtime → "YYYY-MM-DD".
        let fs_meta = std::fs::metadata(&path).ok();
        let size_bytes = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = fs_meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(format_mtime_ymd);

        // Self-blorb chunks: only blorb-container files carry a resource index,
        // and extraction (`load_story`) discards it — re-read the raw file for
        // those extensions only, so plain .z* files stay single-read.
        let self_blorb = if is_blorb_ext(&path) {
            std::fs::read(&path).ok().and_then(|raw| {
                if blorb::Blorb::is_blorb(&raw) {
                    blorb::Blorb::parse(raw).ok().map(|b| chunks_of(&b))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let engine = match &loaded {
            crate::hints::LoadedStory::ZCode(_) => Engine::ZCode,
            crate::hints::LoadedStory::Glulx(_) => Engine::Glulx,
        };
        let is_container = self_blorb.is_some();
        let (version, serial, release, features, format) = match engine {
            Engine::ZCode => {
                let version = z_version(&bytes).map(|v| v.to_string());
                let serial = z_serial(&bytes);
                let release = z_release(&bytes);
                let features = z_features(&bytes, self_blorb.as_deref());
                let format = if is_container { "Blorb (Z-code)" } else { "Z-code" };
                (version, serial, release, features, format.to_string())
            }
            Engine::Glulx => {
                let version = glulx_version(&bytes);
                let features = glulx_features(self_blorb.as_deref());
                let format = if is_container { "Blorb (Glulx)" } else { "Glulx" };
                (version, None, None, features, format.to_string())
            }
        };

        let meta = StoryMeta {
            size_bytes,
            modified,
            engine,
            format,
            version,
            serial,
            release,
            ifid,
            features,
            self_blorb,
        };
        out.push(StoryEntry { path, title, filename, meta });
    }
    out.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.filename.cmp(&b.filename))
    });
    out
}

/// Cheap existence flags shown on every list row (panel-independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowBadges {
    pub blorb: bool,
    pub save: bool,
    pub hint: bool,
}

/// True if a same-stem `.blb`/`.blorb`/`.zblorb` sibling of `path` exists.
fn sibling_blorb_exists(path: &Path) -> bool {
    ["blb", "blorb", "zblorb"].iter().any(|ext| {
        let cand = path.with_extension(ext);
        cand != *path && cand.exists()
    })
}

/// Compute a row's artifact badges. `save_names` is the saves-dir listing read
/// once; `hint_index` is loaded once at picker start. No archive reads.
pub fn compute_row_badges(
    entry: &StoryEntry,
    save_names: &HashSet<String>,
    hint_index: &hints::HintIndex,
) -> RowBadges {
    let ifid = &entry.meta.ifid;
    RowBadges {
        blorb: entry.meta.self_blorb.is_some() || sibling_blorb_exists(&entry.path),
        save: save_names.iter().any(|n| n.starts_with(ifid.as_str())),
        hint: hint_index.get(ifid).is_some(),
    }
}

/// Borrowed badge glyphs from the `[symbols]` config, for row rendering.
pub struct BadgeGlyphs<'a> {
    pub zcode: &'a str,
    pub glulx: &'a str,
    pub blorb: &'a str,
    pub save: &'a str,
    pub hint: &'a str,
}

impl<'a> BadgeGlyphs<'a> {
    pub fn from_symbols(s: &'a crate::config::SymbolConfig) -> Self {
        Self {
            zcode: &s.badge_zcode,
            glulx: &s.badge_glulx,
            blorb: &s.badge_blorb,
            save: &s.badge_save,
            hint: &s.badge_hint,
        }
    }
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

    #[test]
    fn z_header_helpers_parse_version_release_serial_flags() {
        let mut b = minimal_v3_story();
        b[0x00] = 3;                       // version
        b[0x02] = 0x00; b[0x03] = 0x58;    // release 88
        b[0x12..0x18].copy_from_slice(b"840726");
        b[0x10] = 0x00; b[0x11] = 0x08 | 0x40 | 0x80; // flags2: graphics|colour|sound

        assert_eq!(z_version(&b), Some(3));
        assert_eq!(z_release(&b), Some(88));
        assert_eq!(z_serial(&b).as_deref(), Some("840726"));
        let f2 = z_flags2(&b);
        assert!(f2 & 0x0008 != 0, "graphics bit");
        assert!(f2 & 0x0040 != 0, "colour bit");
        assert!(f2 & 0x0080 != 0, "sound bit");
    }

    #[test]
    fn glulx_version_formats_major_minor_subminor() {
        let mut b = vec![0u8; 0x40];
        b[0x00..0x04].copy_from_slice(b"Glul");
        b[0x04] = 0x00; b[0x05] = 0x03;    // major = 3
        b[0x06] = 0x01;                    // minor = 1
        b[0x07] = 0x02;                    // subminor = 2
        assert_eq!(glulx_version(&b).as_deref(), Some("3.1.2"));
    }

    #[test]
    fn scan_populates_story_meta_for_v3() {
        let dir = temp_dir("meta");
        let mut b = minimal_v3_story();
        b[0x02] = 0x00; b[0x03] = 0x58;                 // release 88
        b[0x12..0x18].copy_from_slice(b"840726");
        b[0x10] = 0x00; b[0x11] = 0x40;                 // colour bit set
        std::fs::write(dir.join("game.z3"), &b).unwrap();

        let stories = scan_stories(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        let m = &stories[0].meta;
        assert_eq!(m.engine, Engine::ZCode);
        assert_eq!(m.format, "Z-code");
        assert_eq!(m.version.as_deref(), Some("3"));
        assert_eq!(m.release, Some(88));
        assert_eq!(m.serial.as_deref(), Some("840726"));
        assert_eq!(m.features.colour, Some(true));
        assert!(m.size_bytes > 0);
        assert!(m.self_blorb.is_none());
    }

    // Build a StoryEntry with a controllable ifid + self_blorb, on a synthetic path.
    fn entry_with(ifid: &str, path: PathBuf, self_blorb: Option<Vec<ChunkInfo>>) -> StoryEntry {
        StoryEntry {
            path,
            title: "T".into(),
            filename: "t.z5".into(),
            meta: StoryMeta {
                size_bytes: 1, modified: None, engine: Engine::ZCode,
                format: "Z-code".into(), version: Some("5".into()),
                serial: None, release: None, ifid: ifid.into(),
                features: Features::default(), self_blorb,
            },
        }
    }

    #[test]
    fn compute_row_badges_covers_each_signal() {
        let dir = temp_dir("badges");
        // A self-blorb story lights `blorb` with no sibling.
        let e_self = entry_with("IFID-A", dir.join("a.z5"),
            Some(vec![ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 4 }]));
        // A story with a same-stem sibling .blorb lights `blorb`.
        std::fs::write(dir.join("b.z5"), b"x").unwrap();
        std::fs::write(dir.join("b.blorb"), b"x").unwrap();
        let e_sibling = entry_with("IFID-B", dir.join("b.z5"), None);
        // A plain story with nothing.
        let e_bare = entry_with("IFID-C", dir.join("c.z5"), None);

        let mut save_names = HashSet::new();
        save_names.insert("IFID-A.babelmap".to_string());          // default save for A
        save_names.insert("IFID-B-before.babelmap".to_string());   // named save for B

        let hi = hints::load_hint_index(&dir); // empty index (no hints/index.toml)

        let a = compute_row_badges(&e_self, &save_names, &hi);
        let b = compute_row_badges(&e_sibling, &save_names, &hi);
        let c = compute_row_badges(&e_bare, &save_names, &hi);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!((a.blorb, a.save, a.hint), (true, true, false));
        assert_eq!((b.blorb, b.save, b.hint), (true, true, false));
        assert_eq!((c.blorb, c.save, c.hint), (false, false, false));
    }

    // Minimal blorb with one Snd resource so resolve_sound_blorb accepts a sibling.
    fn blorb_with_sound() -> Vec<u8> {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 { v.push(0); }
            v
        }
        let ridx_data_len = 4 + 12;
        let snd_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Snd ");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&(snd_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"OGGV", b"snd"));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn resolve_aux_finds_sibling_blorb_and_saves() {
        let dir = temp_dir("aux");
        std::fs::write(dir.join("g.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("g.blb"), blorb_with_sound()).unwrap();
        let entry = entry_with("IFID-G", dir.join("g.z5"), None);

        let hi = hints::load_hint_index(&dir);
        let aux = resolve_aux(&entry, &dir, &hi); // save_dir=dir (no saves present)
        let _ = std::fs::remove_dir_all(&dir);

        let (src, chunks) = aux.assoc_blorb.expect("sibling blorb resolved");
        assert!(src.ends_with("g.blb"));
        assert!(chunks.iter().any(|c| c.usage == "Snd "));
        assert!(aux.saves.is_empty());
        assert!(!aux.hints_available);
    }
}


