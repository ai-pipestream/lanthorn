//! Pre-game story picker: when a directory is passed at launch instead of a
//! story file, scan it for Z-machine stories and let the user choose one.
//!
//! Titles are resolved cheaply (no game is run): the known-title table keyed by
//! the IFID, falling back to the filename stem.

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
    /// Parsed format detail (e.g. "15.4 kHz · 8-bit · mono · 2.2s" for a sound,
    /// "800×600 · 32bpp" for an image). `None` when the resource isn't a
    /// sound/image, or its header couldn't be parsed.
    pub detail: Option<String>,
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
    /// The story's per-game dir (SQ-0284), for the info panel's Saves header.
    pub game_dir: PathBuf,
    /// `.qzl` in-game saves in `game_dir` (SQ-0285).
    pub qzl_saves: Vec<crate::persist_files::SaveInfo>,
    /// Game-managed automatic `.qzl` saves in `game_dir` (SQ-0296): the
    /// `_`-prefixed fixed-name files the player saves list hides.
    pub auto_saves: Vec<crate::persist_files::SaveInfo>,
    /// Sidecar filenames present in `game_dir` (`default.aux`/`default.glkvfs`).
    pub sidecars: Vec<&'static str>,
}

/// Resolve the lazy aux for one story. `data_base` is the storage base
/// (`user_dir/saves` or `--data-dir`); the story's saves live in its per-game
/// dir `<data_base>/<story-key>/` (SQ-0284). `hint_index` is the shared index
/// loaded once at picker start (still keyed by IFID).
pub fn resolve_aux(
    entry: &StoryEntry,
    data_base: &Path,
    hint_index: &hints::HintIndex,
) -> StoryAux {
    // Only record an ASSOCIATED blorb (a different file); the self-blorb case is
    // already carried in StoryMeta.self_blorb.
    let assoc_blorb = match blorb::resolve_sound_blorb(&entry.path) {
        Some((b, src)) if src != entry.path => Some((src, chunks_of(&b))),
        _ => None,
    };
    let game_dir = crate::storage::game_dir(data_base, &crate::storage::story_key(&entry.path));
    let saves = crate::persist_files::list_saves(&game_dir);
    let hints_available = hint_index.get(&entry.meta.ifid).is_some();
    let qzl_saves = crate::persist_files::list_qzl(&game_dir);
    let auto_saves = crate::persist_files::list_qzl_auto(&game_dir);
    let mut sidecars = Vec::new();
    if game_dir.join("default.aux").exists() { sidecars.push("default.aux"); }
    if game_dir.join("default.glkvfs").exists() { sidecars.push("default.glkvfs"); }
    StoryAux { assoc_blorb, saves, hints_available, game_dir, qzl_saves, auto_saves, sidecars }
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
            detail: resource_detail(&r.usage, &r.chunk_type, b.resource_data(r)),
        })
        .collect()
}

// ── Resource format-detail parsing ──────────────────────────────────────
//
// Best-effort header-only parsing of Blorb `Snd `/`Pict` resources, for the
// info panel's Resources listing. Never decodes actual audio/pixel data, and
// is panic-proof on malformed/truncated input: every read is bounds-checked
// and a parse failure simply yields `None`.

/// Dispatch a resource's format detail by usage, or `None` when the usage
/// isn't a sound/image or the payload doesn't parse.
fn resource_detail(usage: &[u8; 4], chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match usage {
        b"Snd " => sound_detail(chunk_type, data),
        b"Pict" => image_detail(chunk_type, data),
        _ => None,
    }
}

/// Big-endian u16 at `off`, bounds-checked.
fn be_u16(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

/// Big-endian u32 at `off`, bounds-checked.
fn be_u32(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Decode an IEEE 80-bit extended-precision float (as used for AIFF sample
/// rates) to its nearest `u32`. Returns 0 on malformed/negative-exponent
/// input rather than panicking.
fn extended80_to_u32(e: &[u8]) -> u32 {
    if e.len() < 10 {
        return 0;
    }
    let exp = ((((e[0] as u16) << 8) | e[1] as u16) & 0x7fff) as i32 - 16383;
    let mant = u64::from_be_bytes([e[2], e[3], e[4], e[5], e[6], e[7], e[8], e[9]]);
    if exp < 0 {
        return 0;
    }
    let shift = 63 - exp;
    if !(0..=63).contains(&shift) {
        return 0;
    }
    (mant >> shift) as u32
}

/// Sound resource format detail, dispatched by chunk type (matches
/// [`blorb::SoundKind`] detection: `FORM` → AIFF/AIFC, `OGGV` → Ogg, `MOD ` →
/// module).
fn sound_detail(chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match chunk_type {
        b"FORM" => aiff_detail(data),
        b"OGGV" => ogg_detail(data),
        b"MOD " => mod_detail(data),
        _ => None,
    }
}

/// AIFF/AIFC sample-rate + bit depth + channels + duration, parsed from the
/// `COMM` subchunk. Blorb strips the outer `FORM` header, so `data` starts
/// with the form type (`AIFF`/`AIFC`) followed by subchunks.
fn aiff_detail(data: &[u8]) -> Option<String> {
    let sig = data.get(0..4)?;
    if sig != b"AIFF" && sig != b"AIFC" {
        return None;
    }
    let mut pos = 4;
    while pos + 8 <= data.len() {
        let id = data.get(pos..pos + 4)?;
        let clen = be_u32(data, pos + 4)? as usize;
        let cs = pos + 8;
        if cs.checked_add(clen)? > data.len() {
            return None;
        }
        if id == b"COMM" {
            if clen < 18 {
                return None;
            }
            let channels = be_u16(data, cs)?;
            let num_frames = be_u32(data, cs + 2)?;
            let sample_size = be_u16(data, cs + 6)?;
            let rate_bytes = data.get(cs + 8..cs + 18)?;
            let rate = extended80_to_u32(rate_bytes);
            let mut parts = vec![format!("{:.1} kHz", rate as f64 / 1000.0)];
            parts.push(format!("{sample_size}-bit"));
            parts.push(match channels {
                1 => "mono".to_string(),
                2 => "stereo".to_string(),
                n => format!("{n}ch"),
            });
            if rate != 0 {
                parts.push(format!("{:.1}s", num_frames as f64 / rate as f64));
            }
            return Some(parts.join(" · "));
        }
        pos = cs + clen + (clen & 1);
    }
    None
}

/// Ogg Vorbis sample rate + channels, found by scanning for the Vorbis
/// identification-header packet (`\x01vorbis`) within the first ~512 bytes.
fn ogg_detail(data: &[u8]) -> Option<String> {
    if data.get(0..4)? != b"OggS" {
        return None;
    }
    let window = data.get(0..data.len().min(512))?;
    let needle = b"\x01vorbis";
    let p = window.windows(needle.len()).position(|w| w == needle)?;
    let channels = *data.get(p + 11)?;
    let rate_bytes = data.get(p + 12..p + 16)?;
    let rate = u32::from_le_bytes([rate_bytes[0], rate_bytes[1], rate_bytes[2], rate_bytes[3]]);
    let ch_word = match channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n}ch"),
    };
    Some(format!("{:.1} kHz · {ch_word}", rate as f64 / 1000.0))
}

/// Amiga ProTracker module channel count, read from the format tag at
/// offset 1080..1084 (present only when the module has 31 instruments).
fn mod_detail(data: &[u8]) -> Option<String> {
    if data.len() < 1084 {
        return None;
    }
    let tag = data.get(1080..1084)?;
    let n: u32 = match tag {
        b"M.K." | b"M!K!" | b"FLT4" | b"4CHN" => 4,
        b"6CHN" => 6,
        b"8CHN" | b"FLT8" => 8,
        _ => {
            let s = std::str::from_utf8(tag).unwrap_or("");
            let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(4)
        }
    };
    Some(format!("{n}ch"))
}

/// Image resource format detail, dispatched by chunk type.
fn image_detail(chunk_type: &[u8; 4], data: &[u8]) -> Option<String> {
    match chunk_type {
        b"PNG " => png_detail(data),
        b"JPEG" => jpeg_detail(data),
        _ => None,
    }
}

/// PNG width × height + bits-per-pixel, parsed from the IHDR chunk (fixed
/// offsets right after the 8-byte PNG signature).
fn png_detail(data: &[u8]) -> Option<String> {
    const SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 26 {
        return None;
    }
    if data.get(0..8)? != SIG {
        return None;
    }
    let width = be_u32(data, 16)?;
    let height = be_u32(data, 20)?;
    let bit_depth = *data.get(24)?;
    let color_type = *data.get(25)?;
    let channels: u32 = match color_type {
        0 => 1, // grayscale
        2 => 3, // RGB
        3 => 1, // palette
        4 => 2, // grayscale + alpha
        6 => 4, // RGBA
        _ => 1,
    };
    let bpp = bit_depth as u32 * channels;
    Some(format!("{width}×{height} · {bpp}bpp"))
}

/// JPEG width × height + precision + component count, parsed by scanning
/// markers for the first SOF (start-of-frame) segment.
fn jpeg_detail(data: &[u8]) -> Option<String> {
    if data.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    while pos < data.len() {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }
        // Skip fill bytes (runs of 0xFF before the real marker byte).
        let mut m_pos = pos;
        while data.get(m_pos) == Some(&0xFF) {
            m_pos += 1;
        }
        let marker = *data.get(m_pos)?;
        let seg_start = m_pos + 1;
        // Markers with no length field: SOI/EOI/RSTn.
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            pos = seg_start;
            continue;
        }
        let len = be_u16(data, seg_start)? as usize;
        if len < 2 {
            return None;
        }
        let body_off = seg_start + 2;
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let precision = *data.get(body_off)?;
            let height = be_u16(data, body_off + 1)?;
            let width = be_u16(data, body_off + 3)?;
            let components = *data.get(body_off + 5)?;
            return Some(format!("{width}×{height} · {precision}-bit · {components}ch"));
        }
        pos = body_off.checked_add(len - 2)?;
    }
    None
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

/// Compute a row's artifact badges. `data_base` is the storage base; the save
/// badge lights when the story's per-game dir `<data_base>/<story-key>/` exists
/// and holds a `.babelmap` or `.qzl` (SQ-0284). `hint_index` (IFID-keyed) is
/// loaded once at picker start. No archive reads.
pub fn compute_row_badges(
    entry: &StoryEntry,
    data_base: &Path,
    hint_index: &hints::HintIndex,
) -> RowBadges {
    let ifid = &entry.meta.ifid;
    let game_dir = crate::storage::game_dir(data_base, &crate::storage::story_key(&entry.path));
    RowBadges {
        blorb: entry.meta.self_blorb.is_some() || sibling_blorb_exists(&entry.path),
        save: game_dir_has_save(&game_dir),
        hint: hint_index.get(ifid).is_some(),
    }
}

/// True if `game_dir` exists and contains at least one `.babelmap` or `.qzl`.
fn game_dir_has_save(game_dir: &Path) -> bool {
    std::fs::read_dir(game_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with(".babelmap") || n.ends_with(".qzl"))
        })
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
            Some(vec![ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 4, detail: None }]));
        // A story with a same-stem sibling .blorb lights `blorb`.
        std::fs::write(dir.join("b.z5"), b"x").unwrap();
        std::fs::write(dir.join("b.blorb"), b"x").unwrap();
        let e_sibling = entry_with("IFID-B", dir.join("b.z5"), None);
        // A plain story with nothing.
        let e_bare = entry_with("IFID-C", dir.join("c.z5"), None);

        // Storage base with per-game dirs keyed by story filename (SQ-0284):
        // A has a default Save State, B a named `.qzl` game save, C nothing.
        let base = dir.join("data");
        let a_dir = crate::storage::game_dir(&base, &crate::storage::story_key(&dir.join("a.z5")));
        let b_dir = crate::storage::game_dir(&base, &crate::storage::story_key(&dir.join("b.z5")));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(a_dir.join("default.babelmap"), b"x").unwrap();
        std::fs::write(b_dir.join("before.qzl"), b"x").unwrap();

        let hi = hints::load_hint_index(&dir); // empty index (no hints/index.toml)

        let a = compute_row_badges(&e_self, &base, &hi);
        let b = compute_row_badges(&e_sibling, &base, &hi);
        let c = compute_row_badges(&e_bare, &base, &hi);
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
        let aux = resolve_aux(&entry, &dir, &hi); // data_base=dir (no per-game saves)
        let _ = std::fs::remove_dir_all(&dir);

        let (src, chunks) = aux.assoc_blorb.expect("sibling blorb resolved");
        assert!(src.ends_with("g.blb"));
        assert!(chunks.iter().any(|c| c.usage == "Snd "));
        assert!(aux.saves.is_empty());
        assert!(!aux.hints_available);
    }

    #[test]
    fn resolve_aux_reports_game_dir_qzl_saves_and_sidecars() {
        let dir = temp_dir("aux-qzl");
        std::fs::write(dir.join("g.z5"), minimal_v3_story()).unwrap();
        let entry = entry_with("IFID-G", dir.join("g.z5"), None);

        // A separate data base so the per-game dir doesn't collide with the
        // story file itself (SQ-0284 keys by filename).
        let base = dir.join("data");
        let game_dir = crate::storage::game_dir(&base, &crate::storage::story_key(&entry.path));
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("default.babelmap"), b"x").unwrap();
        std::fs::write(game_dir.join("quick.qzl"), b"x").unwrap();
        std::fs::write(game_dir.join("_startup.qzl"), b"x").unwrap();
        std::fs::write(game_dir.join("default.aux"), b"x").unwrap();

        let hi = hints::load_hint_index(&dir);
        let aux = resolve_aux(&entry, &base, &hi);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(aux.game_dir, game_dir);
        // default.babelmap has no valid archive, so list_saves skips it here.
        assert_eq!(aux.saves.len(), 0, "notanarchive default.babelmap is skipped by list_saves");
        assert_eq!(aux.qzl_saves.len(), 1);
        assert_eq!(aux.qzl_saves[0].name, "quick");
        assert!(!aux.qzl_saves.iter().any(|s| s.name == "_startup"), "auto save excluded from player list");
        assert_eq!(aux.auto_saves.len(), 1, "auto_saves carries the game-managed underscore save");
        assert_eq!(aux.auto_saves[0].name, "_startup");
        assert_eq!(aux.sidecars, vec!["default.aux"]);
    }

    // ── Resource format-detail parsing ──────────────────────────────────

    /// Encode `rate` as an IEEE 80-bit extended-precision float, the inverse
    /// of `extended80_to_u32`, for building AIFF `COMM` fixtures.
    fn encode_extended80(rate: u32) -> [u8; 10] {
        let bits = 32 - rate.leading_zeros(); // significant bits in `rate`
        let exp = 16383 + (bits as i32 - 1);
        let mantissa = (rate as u64) << (63 - (bits - 1));
        let mut out = [0u8; 10];
        out[0] = (exp >> 8) as u8;
        out[1] = exp as u8;
        out[2..10].copy_from_slice(&mantissa.to_be_bytes());
        out
    }

    /// Build a minimal AIFF `Snd ` payload (post-FORM-header, as blorb stores
    /// it): form type + one `COMM` subchunk.
    fn aiff_fixture(channels: u16, sample_size: u16, num_frames: u32, rate: u32) -> Vec<u8> {
        let mut comm = Vec::new();
        comm.extend_from_slice(&channels.to_be_bytes());
        comm.extend_from_slice(&num_frames.to_be_bytes());
        comm.extend_from_slice(&sample_size.to_be_bytes());
        comm.extend_from_slice(&encode_extended80(rate));
        let mut data = b"AIFF".to_vec();
        data.extend_from_slice(b"COMM");
        data.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        data.extend_from_slice(&comm);
        data
    }

    #[test]
    fn aiff_sound_detail_parses_rate_bit_depth_and_channels() {
        let data = aiff_fixture(1, 8, 16000, 8000);
        let detail = sound_detail(b"FORM", &data).expect("valid AIFF COMM parses");
        assert!(detail.contains("8.0 kHz"), "{detail:?}");
        assert!(detail.contains("8-bit"), "{detail:?}");
        assert!(detail.contains("mono"), "{detail:?}");
        assert!(detail.contains("2.0s"), "{detail:?}");
    }

    #[test]
    fn aiff_sound_detail_rejects_garbage() {
        assert_eq!(sound_detail(b"FORM", b"not aiff at all"), None);
        assert_eq!(sound_detail(b"FORM", b"AIFF"), None); // no COMM subchunk
        assert_eq!(sound_detail(b"FORM", &[]), None);
    }

    #[test]
    fn ogg_sound_detail_parses_rate_and_channels() {
        let mut data = b"OggS".to_vec();
        data.extend_from_slice(&[0u8; 20]); // leading page-header padding
        data.extend_from_slice(b"\x01vorbis");
        data.extend_from_slice(&[0u8; 4]); // vorbis_version (unused)
        data.push(2); // channels: stereo
        data.extend_from_slice(&44_100u32.to_le_bytes());
        let detail = sound_detail(b"OGGV", &data).expect("valid Ogg Vorbis header parses");
        assert!(detail.contains("44.1 kHz"), "{detail:?}");
        assert!(detail.contains("stereo"), "{detail:?}");
    }

    #[test]
    fn ogg_sound_detail_rejects_garbage() {
        assert_eq!(sound_detail(b"OGGV", b"not ogg"), None);
        assert_eq!(sound_detail(b"OGGV", b"OggS"), None); // no vorbis ident packet
    }

    #[test]
    fn mod_sound_detail_reads_channel_tag() {
        let mut data = vec![0u8; 1084];
        data[1080..1084].copy_from_slice(b"M.K.");
        assert_eq!(sound_detail(b"MOD ", &data).as_deref(), Some("4ch"));

        let mut data6 = vec![0u8; 1084];
        data6[1080..1084].copy_from_slice(b"6CHN");
        assert_eq!(sound_detail(b"MOD ", &data6).as_deref(), Some("6ch"));
    }

    #[test]
    fn mod_sound_detail_rejects_too_short() {
        assert_eq!(sound_detail(b"MOD ", &[0u8; 100]), None);
    }

    #[test]
    fn png_image_detail_parses_dimensions_and_bpp() {
        let mut data = vec![0u8; 26];
        data[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        data[8..12].copy_from_slice(&13u32.to_be_bytes()); // IHDR length
        data[12..16].copy_from_slice(b"IHDR");
        data[16..20].copy_from_slice(&800u32.to_be_bytes()); // width
        data[20..24].copy_from_slice(&600u32.to_be_bytes()); // height
        data[24] = 8; // bit depth
        data[25] = 6; // color type: RGBA → 4 channels
        let detail = image_detail(b"PNG ", &data).expect("valid PNG IHDR parses");
        assert!(detail.contains("800×600"), "{detail:?}");
        assert!(detail.contains("32bpp"), "{detail:?}");
    }

    #[test]
    fn png_image_detail_rejects_truncated() {
        assert_eq!(image_detail(b"PNG ", b"\x89PNG\r\n\x1a\n"), None); // signature only
        assert_eq!(image_detail(b"PNG ", b"not a png"), None);
    }

    #[test]
    fn jpeg_image_detail_parses_dimensions_and_components() {
        let mut data = vec![0xFFu8, 0xD8, 0xFF, 0xC0]; // SOI, SOF0 marker
        data.extend_from_slice(&17u16.to_be_bytes()); // segment length
        data.push(8); // precision
        data.extend_from_slice(&100u16.to_be_bytes()); // height
        data.extend_from_slice(&200u16.to_be_bytes()); // width
        data.push(3); // components
        data.extend_from_slice(&[0u8; 9]); // 3 components × 3 bytes each
        let detail = image_detail(b"JPEG", &data).expect("valid JPEG SOF0 parses");
        assert!(detail.contains("200×100"), "{detail:?}");
        assert!(detail.contains("8-bit"), "{detail:?}");
        assert!(detail.contains("3ch"), "{detail:?}");
    }

    #[test]
    fn jpeg_image_detail_rejects_garbage() {
        assert_eq!(image_detail(b"JPEG", b"not a jpeg"), None);
        assert_eq!(image_detail(b"JPEG", &[0xFF, 0xD8]), None); // SOI only, no SOF
    }

    #[test]
    fn resource_detail_dispatches_by_usage_none_for_unknown() {
        let png = {
            let mut data = vec![0u8; 26];
            data[0..8].copy_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
            data[16..20].copy_from_slice(&1u32.to_be_bytes());
            data[20..24].copy_from_slice(&1u32.to_be_bytes());
            data[24] = 8;
            data[25] = 2;
            data
        };
        assert!(resource_detail(b"Pict", b"PNG ", &png).is_some());
        assert_eq!(resource_detail(b"Data", b"PNG ", &png), None);
        assert_eq!(resource_detail(b"Exec", b"ZCOD", b"whatever"), None);
    }
}


