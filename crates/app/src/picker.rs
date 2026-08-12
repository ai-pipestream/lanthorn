//! Pre-game story picker: when a directory is passed at launch instead of a
//! story file, scan it for Z-machine stories and let the user choose one.
//!
//! Metadata (title, author, …) is resolved cheaply (no game is run) by
//! precedence, per field: a blorb's own `IFmd` chunk, then a fetched IFDB
//! sidecar, then (title only) the known-title table keyed by the IFID, then
//! the filename stem. See `resolve`.

use std::path::{Path, PathBuf};

use crate::hints;

/// The VM engine a story runs on (version-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    ZCode,
    Glulx,
    Scott,
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
#[derive(Debug, Clone, PartialEq)]
pub struct StoryMeta {
    /// The size of the file on disk — the container, when the story lives in one.
    pub size_bytes: u64,
    /// The size of the story image babelmap actually runs, after mounting the
    /// container (SQ-0771). Equal to `size_bytes` for a plain `.z*`/`.ulx`/`.dat`;
    /// smaller for every container, and *unrelated* to it for an Amiga floppy —
    /// a `.adf` is 880 KB whatever it holds, so the container's length says
    /// nothing about the game. Reported for every container kind (`.adf`, blorb,
    /// zip), not just the disk image.
    pub story_bytes: u64,
    pub modified: Option<String>, // "YYYY-MM-DD"
    pub engine: Engine,
    pub format: String,           // "Z-code" | "Glulx" | "Blorb (Z-code)" | "Blorb (Glulx)"
    pub version: Option<String>,  // Z: "3"; Glulx: "3.1.2"
    pub serial: Option<String>,   // Z only
    pub release: Option<u16>,     // Z only
    pub ifid: String,
    pub features: Features,
    pub self_blorb: Option<Vec<ChunkInfo>>, // Some when the story file itself is a blorb
    /// The story was mounted out of an Amiga release floppy rather than read as
    /// a plain file, so the TYPE column names that container: `Z6 (ADF)`
    /// (SQ-0737). Decided by the mount, from the disk's own boot block — never
    /// from the filename.
    pub disk_image: bool,
    /// Resolved per `resolve`'s precedence: IFmd > fetched sidecar. No TSV/stem
    /// source for these (title-only), so absent means genuinely unknown.
    pub author: Option<String>,
    pub year: Option<String>, // from iFiction's `first_published`
    pub genre: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    /// The story's IFDB page URL, present only once fetched (no IFmd equivalent).
    pub ifdb_link: Option<String>,
    /// IFDB's community average rating, 1–5 (SQ-0529). Fetched-only, like the
    /// link. `None` covers both "never fetched" and "IFDB has no ratings for
    /// it", and the RATE column renders both as blank — never as `0.0`.
    pub ifdb_rating: Option<f32>,
    /// The number of ratings behind `ifdb_rating`; the rating sort's tiebreak.
    pub ifdb_rating_count: Option<u32>,
    /// A fetch ran but IFDB had no record for this IFID — so the panel offers a
    /// manual IFDB search link instead of a dead end (SQ-0371).
    pub fetch_not_found: bool,
}

/// One selectable story in the picker.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryEntry {
    pub path: PathBuf,
    /// Display title: `known_title(ifid)` or the filename stem.
    pub title: String,
    /// The bare filename (e.g. `zork1.z5`), shown beside the title.
    pub filename: String,
    pub meta: StoryMeta,
    /// An InvisiClues/hint sidecar detected beside this game and associated with
    /// it during the scan (SQ-0443). The sidecar entry is hidden from the list;
    /// its presence lights the hint badge and names the file in the info panel.
    pub hint_sidecar: Option<std::path::PathBuf>,
}

/// Candidate story-file extensions (matched case-insensitively). `.zblorb` /
/// `.blorb` / zips are handled by `load_story_bytes`; `.dat` covers some
/// Infocom releases; `.adf` is an Amiga release floppy, whose story
/// `load_story` mounts out of the disk image (SQ-0719).
const STORY_EXTS: &[&str] = &[
    "z3", "z4", "z5", "z6", "z7", "z8", "zblorb", "blorb", "zlb", "dat", "ulx", "gblorb", "blb",
    "adf",
];

pub(crate) fn has_story_ext(path: &Path) -> bool {
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
    let assoc_blorb = match blorb::resolve_resource_blorb(&entry.path) {
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

/// Per-field metadata resolution, produced once by [`resolve`] and read
/// verbatim by everything downstream (list, sort, info panel).
#[derive(Debug, Clone, PartialEq)]
struct Resolved {
    title: String,
    author: Option<String>,
    year: Option<String>,
    genre: Option<String>,
    language: Option<String>,
    description: Option<String>,
    ifdb_link: Option<String>,
    ifdb_rating: Option<f32>,
    ifdb_rating_count: Option<u32>,
    fetch_not_found: bool,
}

/// The publication year from a Treaty of Babel `<firstpublished>`, which is
/// `YYYY` or `YYYY-MM-DD` (iFiction allows the full ISO date). Keep just the
/// leading four-digit year, so the value both sorts numerically and fits the
/// narrow YEAR column; anything without a 4-digit lead is dropped as unusable.
fn leading_year(s: &str) -> Option<String> {
    let y: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    (y.len() == 4).then_some(y)
}

/// A bundled Scott-format entry: canonical title plus, where known, its IFDB id
/// and — for the homebrew games with no IFDB record — author and one-line
/// description, so the browser can show those without a fetch.
struct ScottEntry {
    title: &'static str,
    tuid: Option<&'static str>,
    author: Option<&'static str>,
    description: Option<&'static str>,
}

/// Canonical metadata for Scott-format ("ScottFree") `.dat`/`.blb` adventures,
/// keyed by the lowercase filename stem, bundled in `scott_titles.tsv`
/// (`include_str!`d at build time). Keyed by filename rather than the `.dat`
/// trailer's adventure number because that number is not unique across the
/// ScottFree corpus (Brian Howarth's Mysterious Adventures reuse 1-11;
/// Questprobe titles have none).
fn scott_titles() -> &'static std::collections::HashMap<&'static str, ScottEntry> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<std::collections::HashMap<&'static str, ScottEntry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("scott_titles.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                // <filename-stem>\t<title>[\t<ifdb-tuid>[\t<author>[\t<description>]]]
                let mut cols = line.splitn(5, '\t');
                let stem = cols.next()?.trim();
                let title = cols.next()?.trim();
                let tuid = cols.next().map(str::trim).filter(|c| !c.is_empty());
                let author = cols.next().map(str::trim).filter(|c| !c.is_empty());
                let description = cols.next().map(str::trim).filter(|c| !c.is_empty());
                (!stem.is_empty() && !title.is_empty())
                    .then_some((stem, ScottEntry { title, tuid, author, description }))
            })
            .collect()
    })
}

/// Look up a bundled Scott entry by filename stem (matched case-insensitively).
fn scott_entry(stem: &str) -> Option<&'static ScottEntry> {
    scott_titles().get(stem.to_ascii_lowercase().as_str())
}

/// The canonical title for a known Scott-format game, keyed by filename stem
/// (matched case-insensitively).
pub fn scott_title(stem: &str) -> Option<&'static str> {
    scott_entry(stem).map(|e| e.title)
}

/// The IFDB game id (TUID) for a known Scott-format game, keyed by filename stem
/// (matched case-insensitively), if we have one. A Scott `.dat`'s computed IFID
/// never resolves on IFDB, so the metadata fetch looks the game up by this id.
pub fn scott_tuid(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.tuid)
}

/// The bundled author for a Scott-format game (filename stem, case-insensitive),
/// present only for the homebrew games that have no IFDB record to fetch it from.
pub fn scott_author(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.author)
}

/// The bundled one-line description for a Scott-format game (filename stem,
/// case-insensitive), present only for the homebrew games with no IFDB record.
pub fn scott_description(stem: &str) -> Option<&'static str> {
    scott_entry(stem).and_then(|e| e.description)
}

/// Resolve a display title for a Scott-format `.dat` story from its filename
/// stem. `None` when the filename isn't a known Scott game (caller falls back to
/// the filename stem).
pub fn scott_story_title(path: &Path) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    scott_title(stem).map(str::to_string)
}

/// SPEC "Precedence": per field, independently, first non-empty wins —
/// `ifmd` (the file's own `IFmd` chunk) > `fetched` (an IFDB sidecar) > the
/// bundled `scott_titles.tsv` (`tsv_title`/`tsv_author`/`tsv_description`, only
/// populated for Scott-format games) > `stem` (the filename, title only). The
/// TSV author/description feed the homebrew Scott games that have no IFDB record
/// to fetch from; a real fetch still outranks them.
///
/// Pure so the whole table is testable without touching a filesystem.
fn resolve(
    ifmd: Option<&crate::ifiction::IFiction>,
    fetched: Option<&crate::story_info::FetchedMeta>,
    tsv_title: Option<&str>,
    tsv_author: Option<&str>,
    tsv_description: Option<&str>,
    stem: &str,
) -> Resolved {
    let title = ifmd
        .and_then(|i| i.title.clone())
        .or_else(|| fetched.and_then(|f| f.title.clone()))
        .or_else(|| tsv_title.map(str::to_string))
        .unwrap_or_else(|| stem.to_string());
    let author = ifmd
        .and_then(|i| i.author.clone())
        .or_else(|| fetched.and_then(|f| f.author.clone()))
        .or_else(|| tsv_author.map(str::to_string));
    let year = ifmd
        .and_then(|i| i.first_published.clone())
        .or_else(|| fetched.and_then(|f| f.first_published.clone()))
        .and_then(|s| leading_year(&s));
    let genre = ifmd
        .and_then(|i| i.genre.clone())
        .or_else(|| fetched.and_then(|f| f.genre.clone()));
    let language = ifmd
        .and_then(|i| i.language.clone())
        .or_else(|| fetched.and_then(|f| f.language.clone()));
    let description = ifmd
        .and_then(|i| i.description.clone())
        .or_else(|| fetched.and_then(|f| f.description.clone()))
        .or_else(|| tsv_description.map(str::to_string));
    // IFDB-only: the page link and the community rating exist solely in a
    // fetched block — an IFmd chunk has no equivalent for either.
    let ifdb_link = fetched.and_then(|f| f.ifdb_link.clone());
    let ifdb_rating = fetched.and_then(|f| f.ifdb_rating);
    let ifdb_rating_count = fetched.and_then(|f| f.ifdb_rating_count);
    let fetch_not_found = fetched.map(|f| f.not_found).unwrap_or(false);
    Resolved {
        title, author, year, genre, language, description, ifdb_link, ifdb_rating,
        ifdb_rating_count, fetch_not_found,
    }
}

/// Scan `dir` (top level, non-recursive) for **launchable** Z-machine stories,
/// resolving a display title for each. Files that don't load or don't parse as
/// a supported story are silently skipped (v6 is supported since SQ-0186).
/// Sorted by title (case-insensitive), then filename.
///
/// `data_base` is the storage base (as passed to `ensure_aux`/`compute_row_badges`),
/// used to locate each story's per-game `info.json` sidecar (SQ-0348's fetched
/// metadata) for precedence resolution.
pub fn scan_stories(dir: &Path, data_base: &Path) -> Vec<StoryEntry> {
    let mut out: Vec<StoryEntry> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !has_story_ext(&path) {
            continue;
        }
        if let Some(entry) = resolve_entry(&path, data_base) {
            out.push(entry);
        }
    }
    associate_hint_sidecars(&mut out);
    sort_stories(&mut out, Sort { key: SortKey::Title, desc: false });
    out
}

/// Second pass over a freshly-scanned list: attach each detected InvisiClues/
/// hint sidecar to the game it belongs to and hide the sidecar's own row.
///
/// A sidecar ([`hints::is_hint_sidecar`]) is matched to a game when its
/// curated/derived game key is contained in the game's filename stem OR its
/// title. Every game keeps at most one sidecar (first after a stable filename
/// sort). Sidecars matched to some present game are removed from `out`; a lone
/// sidecar with no matching game stays listed. O(games × sidecars) — the list
/// is small and built once.
fn associate_hint_sidecars(out: &mut Vec<StoryEntry>) {
    // Split into sidecar and game indices.
    let mut sidecar_idxs: Vec<usize> = Vec::new();
    let mut game_idxs: Vec<usize> = Vec::new();
    for (i, e) in out.iter().enumerate() {
        if hints::is_hint_sidecar(&e.filename) {
            sidecar_idxs.push(i);
        } else {
            game_idxs.push(i);
        }
    }
    // Stable candidate order (by filename) so association is deterministic.
    sidecar_idxs.sort_by(|&a, &b| out[a].filename.cmp(&out[b].filename));

    let mut matched: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for &g in &game_idxs {
        let stem = out[g]
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let title = out[g].title.clone();
        let chosen = sidecar_idxs.iter().copied().find(|&s| {
            hints::hint_matches_story(&out[s].filename, &stem)
                || hints::hint_matches_story(&out[s].filename, &title)
        });
        if let Some(s) = chosen {
            out[g].hint_sidecar = Some(out[s].path.clone());
            matched.insert(out[s].path.clone());
        }
    }
    // Hide the sidecars that were associated with some present game.
    out.retain(|e| !matched.contains(&e.path));
}

/// Resolve one story file into a [`StoryEntry`], re-reading its bytes and its
/// (possibly just-updated) IFDB sidecar. `None` if the file doesn't load or
/// isn't launchable. Shared by `scan_stories` (the initial directory scan) and
/// the picker's fetch-progress handler (SQ-0348), which re-resolves a single
/// story right after its sidecar is (re)written so a completed fetch's title/
/// author/year land in the list without a full re-scan.
pub fn resolve_entry(path: &Path, data_base: &Path) -> Option<StoryEntry> {
    let (loaded, disk_image) = crate::hints::load_mounted_story(path).ok()?;
    // Only list stories babelmap can actually launch: Z-code via the
    // Z-machine loader (accepts v3/4/5/7/8, rejects v6/v1/v2), Glulx via the
    // Glulx loader, Scott Adams via the Scott database parser.
    let bytes = loaded.bytes().to_vec();
    let launchable = match &loaded {
        crate::hints::LoadedStory::ZCode(b) => zvm::memory::Memory::new(b.clone()).is_ok(),
        crate::hints::LoadedStory::Glulx(b) => gvm::Memory::new(b.clone()).is_ok(),
        crate::hints::LoadedStory::Scott(b) => {
            std::str::from_utf8(b).ok().map(|s| scott::Database::parse(s).is_ok()).unwrap_or(false)
        }
    };
    if !launchable {
        return None;
    }
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let ifid = crate::ifid::compute_ifid(&bytes);

    // fs metadata: size + mtime → "YYYY-MM-DD". `size_bytes` measures the file
    // on disk; `story_bytes` measures what was mounted out of it (SQ-0771).
    let fs_meta = std::fs::metadata(path).ok();
    let size_bytes = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let story_bytes = bytes.len() as u64;
    let modified = fs_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(format_mtime_ymd);

    // Self-blorb chunks: only blorb-container files carry a resource index,
    // and extraction (`load_story`) discards it — re-read the raw file for
    // those extensions only, so plain .z* files stay single-read. The same
    // parse yields the `IFmd` chunk (if any) for precedence resolution below.
    let mut ifmd: Option<crate::ifiction::IFiction> = None;
    let self_blorb = if is_blorb_ext(path) {
        std::fs::read(path).ok().and_then(|raw| {
            if blorb::Blorb::is_blorb(&raw) {
                blorb::Blorb::parse(raw).ok().map(|b| {
                    if let Some(xml) = b.metadata() {
                        ifmd = crate::ifiction::parse(xml).ok();
                    }
                    chunks_of(&b)
                })
            } else {
                None
            }
        })
    } else {
        None
    };

    // Fetched IFDB sidecar: absent (never fetched, unreadable, malformed,
    // wrong IFID) is simply no metadata, never a scan error.
    let game_dir = crate::storage::game_dir(data_base, &crate::storage::story_key(path));
    let fetched = crate::story_info::load(&game_dir, &ifid).and_then(|info| info.fetched);
    // Scott stories have no IFID-keyed table; resolve their title (and, for the
    // homebrew games with no IFDB record, author/description) from the filename
    // stem via the bundled filename->metadata table instead.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&filename);
    let is_scott = matches!(loaded, crate::hints::LoadedStory::Scott(_));
    let scott_tsv_title = is_scott.then(|| scott_title(stem)).flatten();
    let tsv_title = scott_tsv_title.or_else(|| crate::session::known_title(&ifid));
    let tsv_author = is_scott.then(|| scott_author(stem)).flatten();
    let tsv_description = is_scott.then(|| scott_description(stem)).flatten();
    let resolved = resolve(
        ifmd.as_ref(),
        fetched.as_ref(),
        tsv_title,
        tsv_author,
        tsv_description,
        stem,
    );
    let title = resolved.title;

    let engine = match &loaded {
        crate::hints::LoadedStory::ZCode(_) => Engine::ZCode,
        crate::hints::LoadedStory::Glulx(_) => Engine::Glulx,
        crate::hints::LoadedStory::Scott(_) => Engine::Scott,
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
        // Scott Adams databases carry no version/serial/release. The graphic
        // (SAGA/Mysterious Adventures) versions ship in a blorb (`.blb`); a plain
        // `.dat` does not.
        Engine::Scott => {
            let format = if is_container { "Blorb (Scott Adams)" } else { "Scott Adams" };
            (None, None, None, Features::default(), format.to_string())
        }
    };

    let meta = StoryMeta {
        size_bytes,
        story_bytes,
        modified,
        engine,
        format,
        version,
        serial,
        release,
        ifid,
        features,
        self_blorb,
        disk_image,
        author: resolved.author,
        year: resolved.year,
        genre: resolved.genre,
        language: resolved.language,
        description: resolved.description,
        ifdb_link: resolved.ifdb_link,
        ifdb_rating: resolved.ifdb_rating,
        ifdb_rating_count: resolved.ifdb_rating_count,
        fetch_not_found: resolved.fetch_not_found,
    };
    Some(StoryEntry { path: path.to_path_buf(), title, filename, meta, hint_sidecar: None })
}

/// Column a story list can be sorted by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Title,
    Author,
    Year,
    Rating,
    Type,
}

/// A sort column plus direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub desc: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { key: SortKey::Title, desc: false }
    }
}

/// A lowercased title sort key with a leading English article dropped, so
/// "The Lurking Horror" files under L and "A Mind Forever Voyaging" under M —
/// standard bibliographic ordering (SQ-0373). Only strips an article that is
/// followed by more text (a story literally titled "The" keeps it).
fn bibliographic_key(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            let rest = rest.trim_start();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    lower
}

/// Order `stories` in place by `sort`. Blanks (no author / no year, a
/// non-numeric year, or no IFDB rating) always sort last, in both ascending and descending
/// order — only the non-blank comparison reverses with `desc`. Filename is
/// the tie-break in every case.
pub fn sort_stories(stories: &mut [StoryEntry], sort: Sort) {
    use std::cmp::Ordering;

    /// Compares two `(is_blank, value)` keys: blank entries always sort last,
    /// non-blank entries compare by `value` (reversed when `desc`).
    fn cmp_blank_last<T: Ord>(
        a_blank: bool,
        a_val: &T,
        b_blank: bool,
        b_val: &T,
        desc: bool,
    ) -> Ordering {
        match (a_blank, b_blank) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                let ord = a_val.cmp(b_val);
                if desc { ord.reverse() } else { ord }
            }
        }
    }

    fn title_key(e: &StoryEntry) -> (bool, String) {
        let t = bibliographic_key(&e.title);
        (t.is_empty(), t)
    }

    fn author_key(e: &StoryEntry) -> (bool, String) {
        // Case-insensitive, like the title sort: a plain byte sort would file
        // every capitalised author ahead of every lowercase one ("Zarf" before
        // "adam cadre"), which reads as broken in a name list.
        let a = e.meta.author.clone().unwrap_or_default();
        (a.is_empty(), a.to_lowercase())
    }

    fn year_key(e: &StoryEntry) -> (bool, i64) {
        match e.meta.year.as_deref().and_then(|s| s.trim().parse::<i64>().ok()) {
            Some(n) => (false, n),
            None => (true, 0),
        }
    }

    /// IFDB's average rating as tenths, so it sorts through the same `Ord`
    /// path as every other key (`f32` is not `Ord`). Unrated — including a
    /// story that has simply never been fetched — is blank, so it lands last
    /// in both directions. The rating count is the tiebreak: between two 4.6s,
    /// the one 200 people rated outranks the one 3 people did.
    fn rating_key(e: &StoryEntry) -> (bool, (u32, u32)) {
        match e.meta.ifdb_rating {
            Some(r) if r.is_finite() && r > 0.0 => (
                false,
                ((r * 10.0).round().max(0.0) as u32, e.meta.ifdb_rating_count.unwrap_or(0)),
            ),
            _ => (true, (0, 0)),
        }
    }

    /// Groups rows by engine (Z-code, then Glulx, then Scott), and within an
    /// engine by version. Each dotted version component is zero-padded to a
    /// fixed width so a plain string compare orders numerically (Z3 < Z5 < Z8,
    /// Glulx 3.1.2 < 3.1.11). Every story has an engine, so the key is never
    /// blank.
    fn type_key(e: &StoryEntry) -> String {
        let rank = match e.meta.engine {
            Engine::ZCode => 0,
            Engine::Glulx => 1,
            Engine::Scott => 2,
        };
        let version: String = e
            .meta
            .version
            .as_deref()
            .unwrap_or("")
            .split('.')
            .map(|part| format!("{part:0>4}"))
            .collect::<Vec<_>>()
            .join(".");
        format!("{rank} {version}")
    }

    stories.sort_by(|a, b| {
        let ord = match sort.key {
            SortKey::Title => {
                let (a_blank, a_val) = title_key(a);
                let (b_blank, b_val) = title_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Author => {
                let (a_blank, a_val) = author_key(a);
                let (b_blank, b_val) = author_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Year => {
                let (a_blank, a_val) = year_key(a);
                let (b_blank, b_val) = year_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Rating => {
                let (a_blank, a_val) = rating_key(a);
                let (b_blank, b_val) = rating_key(b);
                cmp_blank_last(a_blank, &a_val, b_blank, &b_val, sort.desc)
            }
            SortKey::Type => {
                cmp_blank_last(false, &type_key(a), false, &type_key(b), sort.desc)
            }
        };
        ord.then_with(|| a.filename.cmp(&b.filename))
    });
}

/// Reorder `stories` by `sort`, keeping the selection on the same story — by
/// path, never by index. Three things reorder the picker's list (changing the
/// sort key, toggling direction, and an `r` sweep landing new titles under a
/// cursor the user isn't touching), and every one of them must not silently
/// move the cursor to a different game. Returns the new index of the
/// previously-selected story (or `0` if it's gone, e.g. an empty list).
pub fn resort_preserving_selection(stories: &mut [StoryEntry], selected: usize, sort: Sort) -> usize {
    let keep = stories.get(selected).map(|e| e.path.clone());
    sort_stories(stories, sort);
    keep.and_then(|p| stories.iter().position(|e| e.path == p)).unwrap_or(0)
}

/// A row's hint state, driving which (if any) hint glyph the row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HintBadge {
    /// No hint locally and none available to download.
    #[default]
    None,
    /// No local hint, but a matching InvisiClues can be downloaded (`H`) —
    /// shown as the lowercase available-hint glyph.
    Available,
    /// A hint file is present locally (a sidecar or a remembered association) —
    /// shown as the uppercase hint glyph.
    Present,
}

/// Cheap existence flags shown on every list row (panel-independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowBadges {
    pub blorb: bool,
    pub save: bool,
    pub hint: HintBadge,
}

/// True if `path` has an associated resource blorb — an exact same-stem
/// `.blb`/`.blorb`/`.zblorb` sibling, or (like the info panel's resource
/// resolution) an unambiguous stem-prefix match in the same directory, e.g.
/// `Lurking.blb` for `lurkinghorror-r219-s870912.z3`. Filename-only, so the
/// per-row `(blorb)` tag stays cheap (no blorb parsing).
fn sibling_blorb_exists(path: &Path) -> bool {
    blorb::sibling_blorb_by_name(path).is_some()
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
    let hint = if hint_index.get(ifid).is_some() || entry.hint_sidecar.is_some() {
        HintBadge::Present
    } else {
        // No local hint — light the lowercase glyph if one is downloadable.
        let stem = entry.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if hints::hint_download_for(stem, &entry.title).is_some() {
            HintBadge::Available
        } else {
            HintBadge::None
        }
    };
    RowBadges {
        blorb: entry.meta.self_blorb.is_some() || sibling_blorb_exists(&entry.path),
        save: game_dir_has_save(&game_dir),
        hint,
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
    pub hint_available: &'a str,
}

impl<'a> BadgeGlyphs<'a> {
    pub fn from_symbols(s: &'a crate::config::SymbolConfig) -> Self {
        Self {
            zcode: &s.badge_zcode,
            glulx: &s.badge_glulx,
            blorb: &s.badge_blorb,
            save: &s.badge_save,
            hint: &s.badge_hint,
            hint_available: &s.badge_hint_available,
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

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "only the valid .z5 is listed");
        assert_eq!(stories[0].filename, "game.z5");
        // No known title for this synthetic IFID → falls back to the stem.
        assert_eq!(stories[0].title, "game");
    }

    /// An Amiga release floppy is a listable story file (SQ-0719) — the picker
    /// offers it and `load_story` mounts the game out of it.
    #[test]
    fn disk_images_are_listed_as_stories() {
        assert!(has_story_ext(Path::new("Zork Zero.adf")));
        assert!(has_story_ext(Path::new("DISK1.ADF")), "matched case-insensitively");
    }

    #[test]
    fn scan_lists_v6_but_skips_unsupported_versions() {
        let dir = temp_dir("v6");
        // v6 is supported since SQ-0186 (it boots) — a v6 story with the real
        // `.z6` extension IS now listed (the extension is in STORY_EXTS and the
        // header parses).
        let mut v6 = minimal_v3_story();
        v6[0x00] = 6;
        std::fs::write(dir.join("graphic.z6"), &v6).unwrap();
        // v1/v2 remain unsupported (parse_header rejects them) → skipped.
        let mut v1 = minimal_v3_story();
        v1[0x00] = 1;
        std::fs::write(dir.join("old.z5"), &v1).unwrap();

        let stories = scan_stories(&dir, &dir);
        let names: Vec<String> = stories.iter().map(|s| s.filename.clone()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(names.iter().any(|n| n == "graphic.z6"), "v6 .z6 story is listed (supported): {names:?}");
        assert!(!names.iter().any(|n| n == "old.z5"), "v1 remains unsupported → skipped: {names:?}");
    }

    #[test]
    fn scan_sorts_by_title() {
        let dir = temp_dir("sort");
        std::fs::write(dir.join("zebra.z5"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("apple.z5"), minimal_v3_story()).unwrap();
        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        let titles: Vec<&str> = stories.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["apple", "zebra"]);
    }

    /// Builds a bare-bones `StoryEntry` for `sort_stories` tests: only
    /// title/filename/author/year vary, everything else is a placeholder.
    fn story(title: &str, filename: &str, author: Option<&str>, year: Option<&str>) -> StoryEntry {
        StoryEntry {
            path: PathBuf::from(filename),
            title: title.to_string(),
            filename: filename.to_string(),
            meta: StoryMeta {
                size_bytes: 0, story_bytes: 0,
                modified: None,
                engine: Engine::ZCode,
                format: "Z-code".to_string(),
                version: None,
                serial: None,
                release: None,
                ifid: String::new(),
                features: Features::default(),
                self_blorb: None,
                disk_image: false,
                author: author.map(|s| s.to_string()),
                year: year.map(|s| s.to_string()),
                genre: None,
                language: None,
                description: None, ifdb_link: None, ifdb_rating: None,
                ifdb_rating_count: None, fetch_not_found: false,
            },
            hint_sidecar: None,
        }
    }

    fn titles_of(stories: &[StoryEntry]) -> Vec<&str> {
        stories.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn sort_stories_title_ascending_case_insensitive() {
        let mut stories = vec![
            story("Zebra", "z.z5", None, None),
            story("apple", "a.z5", None, None),
            story("Mango", "m.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        assert_eq!(titles_of(&stories), vec!["apple", "Mango", "Zebra"]);
    }

    #[test]
    fn sort_stories_by_type_groups_by_engine_then_version() {
        // Type sort groups Z-code (ordered by version) < Glulx < Scott,
        // independent of title order.
        let typed = |title: &str, engine: Engine, version: Option<&str>| {
            let mut e = story(title, &format!("{title}.dat"), None, None);
            e.meta.engine = engine;
            e.meta.version = version.map(str::to_string);
            e
        };
        let mut stories = vec![
            typed("scott", Engine::Scott, None),
            typed("z8", Engine::ZCode, Some("8")),
            typed("glulx", Engine::Glulx, Some("3.1.2")),
            typed("z3", Engine::ZCode, Some("3")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: false });
        assert_eq!(titles_of(&stories), vec!["z3", "z8", "glulx", "scott"]);

        sort_stories(&mut stories, Sort { key: SortKey::Type, desc: true });
        assert_eq!(titles_of(&stories), vec!["scott", "glulx", "z8", "z3"]);
    }

    #[test]
    fn sort_stories_title_ignores_leading_articles() {
        // SQ-0373: bibliographic ordering. "The Lurking Horror" files under L,
        // "A Mind Forever Voyaging" under M — but the full title still displays.
        let mut stories = vec![
            story("The Lurking Horror", "lh.z3", None, None),
            story("A Mind Forever Voyaging", "amfv.z4", None, None),
            story("Bureaucracy", "bur.z3", None, None),
            story("An Act of Murder", "aom.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        assert_eq!(
            titles_of(&stories),
            vec!["An Act of Murder", "Bureaucracy", "The Lurking Horror", "A Mind Forever Voyaging"],
        );
    }

    #[test]
    fn bibliographic_key_strips_only_a_real_leading_article() {
        assert_eq!(super::bibliographic_key("The Lurking Horror"), "lurking horror");
        assert_eq!(super::bibliographic_key("A Mind Forever Voyaging"), "mind forever voyaging");
        assert_eq!(super::bibliographic_key("An Act of Murder"), "act of murder");
        // "Theatre" starts with "the" but isn't the article "the ".
        assert_eq!(super::bibliographic_key("Theatre"), "theatre");
        // A story literally titled "The" keeps it (nothing follows the article).
        assert_eq!(super::bibliographic_key("The"), "the");
    }

    #[test]
    fn sort_stories_title_descending() {
        let mut stories = vec![
            story("Zebra", "z.z5", None, None),
            story("apple", "a.z5", None, None),
            story("Mango", "m.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: true });
        assert_eq!(titles_of(&stories), vec!["Zebra", "Mango", "apple"]);
    }

    #[test]
    fn sort_stories_title_filename_tiebreak() {
        let mut stories = vec![
            story("Same", "b.z5", None, None),
            story("Same", "a.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let filenames: Vec<&str> = stories.iter().map(|s| s.filename.as_str()).collect();
        assert_eq!(filenames, vec!["a.z5", "b.z5"]);
    }

    #[test]
    fn sort_stories_author_blanks_last_ascending() {
        // A naive sort_by_key on the raw (possibly-empty) string would put the
        // blank author first ("" < "Adams"). It must sort LAST instead.
        let mut stories = vec![
            story("Unfetched", "u.z5", None, None),
            story("Hitchhiker", "h.z5", Some("Adams"), None),
            story("Zork", "z.z5", Some("Blank, Marc"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: false });
        assert_eq!(titles_of(&stories), vec!["Hitchhiker", "Zork", "Unfetched"]);
    }

    #[test]
    fn sort_stories_author_blanks_last_descending() {
        // Blanks sort last in BOTH directions — descending must not flip the
        // blank entry to the front just because the whole tuple got reversed.
        let mut stories = vec![
            story("Unfetched", "u.z5", None, None),
            story("Hitchhiker", "h.z5", Some("Adams"), None),
            story("Zork", "z.z5", Some("Blank, Marc"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: true });
        assert_eq!(titles_of(&stories), vec!["Zork", "Hitchhiker", "Unfetched"]);
    }

    #[test]
    fn sort_stories_author_case_insensitive() {
        // Byte order puts capitals before lowercase (all uppercase < any
        // lowercase), so a case-sensitive sort would file "Zarf" ahead of
        // "adam cadre". The list sorts by name, not by ASCII code.
        let mut stories = vec![
            story("Spider", "s.z5", Some("Zarf"), None),
            story("Photopia", "p.z5", Some("adam cadre"), None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Author, desc: false });
        assert_eq!(titles_of(&stories), vec!["Photopia", "Spider"]);
    }

    #[test]
    fn sort_stories_year_numeric_not_lexical() {
        // Lexical comparison would put "1980" after "1998" is fine, but would
        // put "700" before "80" — assert numeric ordering explicitly.
        let mut stories = vec![
            story("B", "b.z5", None, Some("1998")),
            story("A", "a.z5", None, Some("1980")),
            story("C", "c.z5", None, Some("700")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: false });
        assert_eq!(titles_of(&stories), vec!["C", "A", "B"]);
    }

    #[test]
    fn sort_stories_year_blank_and_non_numeric_last_both_directions() {
        let mut stories = vec![
            story("NoYear", "n.z5", None, None),
            story("BadYear", "x.z5", None, Some("circa 1990")),
            story("Old", "o.z5", None, Some("1980")),
            story("New", "y.z5", None, Some("1998")),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: false });
        // Blanks/non-numeric sort last; among themselves order is stable per
        // the filename tie-break ("n.z5" < "x.z5").
        assert_eq!(titles_of(&stories), vec!["Old", "New", "NoYear", "BadYear"]);

        sort_stories(&mut stories, Sort { key: SortKey::Year, desc: true });
        assert_eq!(titles_of(&stories), vec!["New", "Old", "NoYear", "BadYear"]);
    }

    /// `story()` plus an IFDB rating — SQ-0529's sort key.
    fn rated(title: &str, filename: &str, rating: Option<f32>, count: Option<u32>) -> StoryEntry {
        let mut e = story(title, filename, None, None);
        e.meta.ifdb_rating = rating;
        e.meta.ifdb_rating_count = count;
        e
    }

    /// Ratings are `f32`, so a naive sort would not even compile against `Ord`;
    /// the key goes through tenths. Check the ordering is numeric, not lexical
    /// ("10" vs "9" is the classic trap even though IFDB caps at 5).
    #[test]
    fn sort_stories_rating_orders_numerically() {
        let mut stories = vec![
            rated("Mid", "m.z5", Some(3.8), Some(226)),
            rated("Best", "b.z5", Some(4.6), Some(50)),
            rated("Worst", "w.z5", Some(1.2), Some(9)),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: false });
        assert_eq!(titles_of(&stories), vec!["Worst", "Mid", "Best"]);

        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(titles_of(&stories), vec!["Best", "Mid", "Worst"]);
    }

    /// SPEC (SQ-0529): unrated stories sort LAST — in both directions. A story
    /// that has simply never been fetched is unrated too, and the two are
    /// indistinguishable here by design; neither may masquerade as a 0.0 and
    /// lead the descending list.
    #[test]
    fn sort_stories_rating_unrated_last_both_directions() {
        let mut stories = vec![
            rated("Unfetched", "u.z5", None, None),
            rated("Loved", "l.z5", Some(4.6), Some(50)),
            rated("Panned", "p.z5", Some(1.2), Some(9)),
            rated("Unrated", "z.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: false });
        assert_eq!(titles_of(&stories), vec!["Panned", "Loved", "Unfetched", "Unrated"]);

        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(
            titles_of(&stories), vec!["Loved", "Panned", "Unfetched", "Unrated"],
            "descending flips the rated rows only — the unrated tail stays put"
        );
    }

    /// Two identical averages are broken by how many people rated them, so a
    /// 4.6 from 200 voters outranks a 4.6 from three. Without the tiebreak the
    /// pair would fall through to the filename, which is meaningless here.
    #[test]
    fn sort_stories_rating_ties_break_on_the_rating_count() {
        let mut stories = vec![
            rated("Fluke", "a.z5", Some(4.6), Some(3)),
            rated("Classic", "z.z5", Some(4.6), Some(200)),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Rating, desc: true });
        assert_eq!(
            titles_of(&stories), vec!["Classic", "Fluke"],
            "the well-rated 4.6 leads, despite losing the filename tie-break"
        );
    }

    #[test]
    fn sort_stories_default_is_title_ascending() {
        let default = Sort::default();
        assert_eq!(default.key, SortKey::Title);
        assert!(!default.desc);
    }

    // ── resort_preserving_selection: THE highest-value property in the quest ───
    //
    // Selection is an index. Reordering the list under it (a sort-key change,
    // a direction toggle, or a background fetch sweep rewriting titles) must
    // never silently move the cursor to a different story.

    #[test]
    fn resort_preserving_selection_survives_a_sort_key_change() {
        // Chosen so the selected story lands at a DIFFERENT index under the new
        // sort (title-order index 2, author-order index 1) — a naive
        // index-clamping "helper" would silently land on the wrong story here.
        let mut stories = vec![
            story("Anchorhead", "a.z5", Some("Zed"), None),
            story("Curses", "c.z5", Some("Amy"), None),
            story("Zebra", "z.z5", Some("Cara"), None),
        ];
        // Title-ascending: Anchorhead(0), Curses(1), Zebra(2) — select "Zebra".
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let selected = stories.iter().position(|e| e.title == "Zebra").unwrap();
        assert_eq!(selected, 2);

        // Switch to Author-ascending: Amy(Curses,0), Cara(Zebra,1), Zed(Anchorhead,2).
        let new_idx = resort_preserving_selection(
            &mut stories,
            selected,
            Sort { key: SortKey::Author, desc: false },
        );
        assert_eq!(new_idx, 1, "Zebra must land at its new author-sorted index");
        assert_eq!(stories[new_idx].title, "Zebra", "selection must still point at Zebra");
        assert_eq!(stories[new_idx].path, PathBuf::from("z.z5"));
    }

    #[test]
    fn resort_preserving_selection_survives_a_direction_toggle() {
        // Four items (even count) so reversing genuinely moves every index,
        // including the selected one — with three items the middle entry's
        // index is unchanged by a reversal, which would hide an index-based bug.
        let mut stories = vec![
            story("Anchorhead", "a.z5", None, None),
            story("Bogus", "b.z5", None, None),
            story("Curses", "c.z5", None, None),
            story("Zebra", "z.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        let selected = 0; // "Anchorhead"
        assert_eq!(stories[selected].title, "Anchorhead");

        let new_idx = resort_preserving_selection(
            &mut stories,
            selected,
            Sort { key: SortKey::Title, desc: true },
        );
        assert_eq!(new_idx, 3, "descending reverses the list, moving index 0 to the end");
        assert_eq!(stories[new_idx].title, "Anchorhead");
        assert_eq!(stories[new_idx].path, PathBuf::from("a.z5"));
    }

    #[test]
    fn resort_preserving_selection_survives_a_sweep_rewriting_titles() {
        // Simulates an `r` sweep landing new (fetched) titles mid-flight: the
        // selected story's title changes to something that now sorts
        // elsewhere, while the cursor stays untouched by the user.
        let mut stories = vec![
            story("zork2-r63-s860811", "b.z5", None, None), // stem title, not yet fetched
            story("Anchorhead", "a.z5", None, None),
            story("Curses", "c.z5", None, None),
        ];
        sort_stories(&mut stories, Sort { key: SortKey::Title, desc: false });
        // Alphabetically: Anchorhead(0), Curses(1), zork2-r63-s860811(2) (case-fold
        // puts the lowercase stem after the capitalized titles).
        let selected = stories.iter().position(|e| e.path == *"b.z5").unwrap();
        assert_eq!(selected, 2);

        // The sweep just fetched this story's real title — one that now sorts
        // FIRST, so a naive index-clamp would land on the wrong (unrelated) story.
        stories[selected].title = "AAA Zork II".to_string();

        let new_idx = resort_preserving_selection(&mut stories, selected, Sort::default());
        assert_eq!(new_idx, 0, "the rewritten title now sorts first");
        assert_eq!(stories[new_idx].path, PathBuf::from("b.z5"), "selection follows the story by path");
        assert_eq!(stories[new_idx].title, "AAA Zork II");
    }

    #[test]
    fn resort_preserving_selection_defaults_to_zero_when_the_story_is_gone() {
        let mut stories = vec![story("Anchorhead", "a.z5", None, None)];
        let new_idx = resort_preserving_selection(&mut stories, 5, Sort::default());
        assert_eq!(new_idx, 0);
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

        let stories = scan_stories(&dir, &dir);
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
        assert_eq!(m.story_bytes, m.size_bytes, "a bare story file IS its story");
        assert!(m.self_blorb.is_none());
    }

    /// A self-contained blorb around a story: `Blorb` FORM wrapper + resource
    /// index + one `ZCOD` Exec chunk. Deliberately larger than the story it
    /// holds, so `story_bytes` and `size_bytes` cannot coincide.
    fn blorb_with_exec(story: &[u8]) -> Vec<u8> {
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
        let ridx_data_len = 4 + 12;
        let exec_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&(exec_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"ZCOD", story));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// SQ-0771: a container's byte length is the container's, never the game's.
    /// `size_bytes` measures the file on disk and `story_bytes` measures what
    /// was mounted out of it — for a blorb (and for a zip, and for the `.adf`
    /// the bug was reported on) those are different numbers.
    #[test]
    fn a_container_reports_the_mounted_storys_size_beside_its_own() {
        let dir = temp_dir("story-bytes");
        let story = minimal_v3_story();
        std::fs::write(dir.join("bare.z3"), &story).unwrap();
        std::fs::write(dir.join("wrapped.zblorb"), blorb_with_exec(&story)).unwrap();

        let bare = resolve_entry(&dir.join("bare.z3"), &dir).expect("bare story resolves");
        let blorb = resolve_entry(&dir.join("wrapped.zblorb"), &dir).expect("blorb resolves");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(bare.meta.story_bytes, story.len() as u64);
        assert_eq!(bare.meta.size_bytes, story.len() as u64);
        // The container is bigger than the game, and it is the game the field
        // has to report.
        assert_eq!(blorb.meta.story_bytes, story.len() as u64, "the mounted story's size");
        assert!(
            blorb.meta.size_bytes > blorb.meta.story_bytes,
            "the blorb file is larger than its Exec chunk: {} vs {}",
            blorb.meta.size_bytes,
            blorb.meta.story_bytes
        );
    }

    /// End to end on real media (skips vacuously — `stories/` is gitignored):
    /// every Amiga floppy in the story directory is the same 880 KB whatever it
    /// holds, so its container length says nothing about the game; `story_bytes`
    /// must be the mounted image's own length (SQ-0771).
    #[test]
    fn a_real_disk_image_reports_the_mounted_storys_size() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // no story media here — skip
        };
        let data_base =
            std::env::temp_dir().join(format!("babelmap-adf-size-{}", std::process::id()));
        let mut saw_adf = false;
        for e in entries.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext != "adf" {
                continue;
            }
            let Some(entry) = resolve_entry(&path, &data_base) else {
                continue; // not launchable — the picker wouldn't list it either
            };
            saw_adf = true;
            let mounted = crate::hints::load_story(&path).expect("the floppy mounts").into_bytes();
            assert_eq!(
                entry.meta.story_bytes,
                mounted.len() as u64,
                "{}: story_bytes is the mounted image's length",
                path.display()
            );
            assert!(
                entry.meta.story_bytes < entry.meta.size_bytes,
                "{}: pre-fix this reported the {}-byte floppy as the story's size",
                path.display(),
                entry.meta.size_bytes
            );
        }
        let _ = std::fs::remove_dir_all(&data_base);
        let _ = saw_adf; // no `.adf` present is a vacuous skip, not a failure
    }

    // ── `resolve` precedence (pure, no filesystem) ─────────────────────────

    use crate::ifiction::IFiction;
    use crate::story_info::FetchedMeta;

    /// A fetch that ran to completion but found nothing worth reporting: every
    /// field absent, `not_found: false` (callers override per-test).
    fn fetched_stub() -> FetchedMeta {
        FetchedMeta {
            scanned_at: "2026-07-16T00:00:00Z".into(),
            fetch_version: crate::story_info::FETCH_VERSION,
            source: "ifdb".into(),
            title: None,
            author: None,
            language: None,
            first_published: None,
            genre: None,
            description: None,
            ifdb_tuid: None,
            ifdb_link: None,
            ifdb_rating: None,
            ifdb_rating_count: None,
            cover: None,
            not_found: false,
        }
    }

    /// SPEC "Precedence". Resolution happens ONCE, here — everything downstream
    /// reads plain fields and never asks where a value came from.
    #[test]
    fn ifmd_outranks_a_fetched_sidecar_field_by_field() {
        let ifmd = IFiction { title: Some("From IFmd".into()), author: None, ..Default::default() };
        let fetched = FetchedMeta { title: Some("From IFDB".into()), author: Some("From IFDB".into()), ..fetched_stub() };
        let r = resolve(Some(&ifmd), Some(&fetched), None, None, None, "stem");
        assert_eq!(r.title, "From IFmd", "the file's own metadata wins");
        assert_eq!(r.author.as_deref(), Some("From IFDB"), "but IFDB fills the gap IFmd left");
    }

    #[test]
    fn tsv_then_stem_when_nothing_else_has_a_title() {
        assert_eq!(resolve(None, None, Some("From TSV"), None, None, "stem").title, "From TSV");
        assert_eq!(resolve(None, None, None, None, None, "stem").title, "stem");
    }

    #[test]
    fn a_not_found_block_contributes_nothing_but_is_not_an_error() {
        let nf = FetchedMeta { not_found: true, title: None, ..fetched_stub() };
        assert_eq!(
            resolve(None, Some(&nf), Some("From TSV"), None, None, "stem").title,
            "From TSV"
        );
    }

    #[test]
    fn tsv_author_and_description_fill_gaps_but_a_fetch_still_wins() {
        // Homebrew Scott games have only the bundled TSV author/description.
        let r = resolve(None, None, Some("Marooned"), Some("Kim Watt"), Some("A desc."), "stem");
        assert_eq!(r.author.as_deref(), Some("Kim Watt"));
        assert_eq!(r.description.as_deref(), Some("A desc."));
        // A real IFDB fetch outranks the TSV fallback, field by field.
        let fetched = FetchedMeta {
            author: Some("From IFDB".into()),
            description: Some("From IFDB".into()),
            ..fetched_stub()
        };
        let r = resolve(None, Some(&fetched), None, Some("Kim Watt"), Some("A desc."), "stem");
        assert_eq!(r.author.as_deref(), Some("From IFDB"));
        assert_eq!(r.description.as_deref(), Some("From IFDB"));
    }

    /// The rating is IFDB-only: a blorb's IFmd chunk has no equivalent, so it
    /// comes from a fetched block or not at all (SQ-0529). A story with rich
    /// local metadata and no sidecar still has no rating — and the resolver
    /// must leave it None rather than default it.
    #[test]
    fn the_ifdb_rating_comes_only_from_a_fetched_block() {
        let ifmd = IFiction { title: Some("Local".into()), ..Default::default() };
        let r = resolve(Some(&ifmd), None, None, None, None, "stem");
        assert_eq!(r.ifdb_rating, None, "an IFmd chunk carries no community rating");
        assert_eq!(r.ifdb_rating_count, None);

        let fetched = FetchedMeta {
            ifdb_rating: Some(3.818_584),
            ifdb_rating_count: Some(226),
            ..fetched_stub()
        };
        let r = resolve(Some(&ifmd), Some(&fetched), None, None, None, "stem");
        assert_eq!(r.ifdb_rating, Some(3.818_584), "IFmd wins the title but has no rating to win");
        assert_eq!(r.ifdb_rating_count, Some(226));
    }

    #[test]
    fn leading_year_takes_the_year_from_a_bare_or_iso_firstpublished() {
        assert_eq!(leading_year("1984"), Some("1984".to_string()));
        // iFiction allows a full ISO date; the YEAR column and numeric sort
        // want just the year, not "1984-06-01".
        assert_eq!(leading_year("1984-06-01"), Some("1984".to_string()));
        assert_eq!(leading_year("  1980 "), Some("1980".to_string()));
        // Nothing usable → dropped, so it sorts/displays as "unknown", not "0".
        assert_eq!(leading_year("forthcoming"), None);
        assert_eq!(leading_year("198"), None, "a 3-digit lead is not a year");
    }

    // ── `scan_stories` integration: sidecar resolution end-to-end ──────────

    #[test]
    fn scan_resolves_title_from_a_fetched_sidecar() {
        let dir = temp_dir("sidecar-fetched");
        let bytes = minimal_v3_story();
        std::fs::write(dir.join("game.z5"), &bytes).unwrap();
        let ifid = crate::ifid::compute_ifid(&bytes);

        let data_base = dir.join("data");
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&dir.join("game.z5")));
        let info = crate::story_info::StoryInfo {
            format_version: crate::story_info::FORMAT_VERSION,
            ifid: ifid.clone(),
            fetched: Some(FetchedMeta { title: Some("Fetched Title".into()), ..fetched_stub() }),
            probe: None,
        };
        crate::story_info::save(&game_dir, &info).unwrap();

        let stories = scan_stories(&dir, &data_base);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].title, "Fetched Title");
    }

    #[test]
    fn scan_falls_back_past_a_wrong_ifid_sidecar() {
        let dir = temp_dir("sidecar-wrong-ifid");
        let bytes = minimal_v3_story();
        std::fs::write(dir.join("game.z5"), &bytes).unwrap();

        let data_base = dir.join("data");
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&dir.join("game.z5")));
        let info = crate::story_info::StoryInfo {
            format_version: crate::story_info::FORMAT_VERSION,
            ifid: "WRONG-IFID".into(), // doesn't match the story's real IFID
            fetched: Some(FetchedMeta { title: Some("Should Not Appear".into()), ..fetched_stub() }),
            probe: None,
        };
        crate::story_info::save(&game_dir, &info).unwrap();

        let stories = scan_stories(&dir, &data_base);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].title, "game", "wrong-IFID sidecar ignored entirely; falls to the stem");
    }

    // Build a StoryEntry with a controllable ifid + self_blorb, on a synthetic path.
    fn entry_with(ifid: &str, path: PathBuf, self_blorb: Option<Vec<ChunkInfo>>) -> StoryEntry {
        StoryEntry {
            path,
            title: "T".into(),
            filename: "t.z5".into(),
            meta: StoryMeta {
                size_bytes: 1, story_bytes: 1, modified: None, engine: Engine::ZCode,
                format: "Z-code".into(), version: Some("5".into()),
                serial: None, release: None, ifid: ifid.into(),
                features: Features::default(), self_blorb, disk_image: false,
                author: None, year: None, genre: None, language: None, description: None,
                ifdb_link: None, ifdb_rating: None, ifdb_rating_count: None,
                fetch_not_found: false,
            },
            hint_sidecar: None,
        }
    }

    #[test]
    fn scan_associates_and_hides_hint_sidecar() {
        let dir = temp_dir("hint-sidecar");
        std::fs::write(dir.join("zork1.z3"), minimal_v3_story()).unwrap();
        std::fs::write(dir.join("zork1_hints.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        // (a) the game is listed; (b) the sidecar is NOT listed.
        assert_eq!(stories.len(), 1, "only the game is listed, sidecar hidden");
        assert_eq!(stories[0].filename, "zork1.z3");
        // (c) the game entry points at the hidden sidecar file.
        assert_eq!(
            stories[0].hint_sidecar.as_deref(),
            Some(dir.join("zork1_hints.z5").as_path())
        );
    }

    #[test]
    fn scan_keeps_a_lone_hint_sidecar_listed() {
        // A hint sidecar with no matching game is not orphaned — it stays listed.
        let dir = temp_dir("lone-sidecar");
        std::fs::write(dir.join("deadlineinv.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "lone sidecar stays listed");
        assert_eq!(stories[0].filename, "deadlineinv.z5");
        assert!(stories[0].hint_sidecar.is_none());
    }

    #[test]
    fn scan_does_not_hide_a_solid_gold_game() {
        // A Solid Gold `*-invclues-rNN-sNNN.z5` carries a release/serial, so it is
        // NOT a hint sidecar and must stay listed as a normal game.
        let dir = temp_dir("solid-gold");
        std::fs::write(dir.join("zork1-invclues-r52-s871125.z5"), minimal_v3_story()).unwrap();

        let stories = scan_stories(&dir, &dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(stories.len(), 1, "Solid Gold game is not dropped");
        assert_eq!(stories[0].filename, "zork1-invclues-r52-s871125.z5");
        assert!(stories[0].hint_sidecar.is_none());
    }

    #[test]
    fn compute_row_badges_lights_hint_from_sidecar() {
        // With an empty index, a detected sidecar alone lights the hint badge.
        let dir = temp_dir("badge-sidecar");
        let mut e = entry_with("IFID-H", dir.join("zork1.z3"), None);
        e.hint_sidecar = Some(dir.join("zork1_hints.z5"));
        let base = dir.join("data");
        let hi = hints::load_hint_index(&dir); // empty index

        let b = compute_row_badges(&e, &base, &hi);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(b.hint, HintBadge::Present, "sidecar presence lights the present-hint badge with an empty index");
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

        assert_eq!((a.blorb, a.save, a.hint), (true, true, HintBadge::None));
        assert_eq!((b.blorb, b.save, b.hint), (true, true, HintBadge::None));
        assert_eq!((c.blorb, c.save, c.hint), (false, false, HintBadge::None));
    }

    /// A game with no local hint but a matching downloadable InvisiClues lights
    /// the lowercase available-hint badge; a game with neither stays None.
    #[test]
    fn compute_row_badges_marks_downloadable_hint_available() {
        let dir = temp_dir("badge-available");
        let base = dir.join("data");
        let hi = hints::load_hint_index(&dir); // empty index

        // "deadline" matches the SLAG catalog → Available (no local file).
        let e_dl = entry_with("IFID-DL", dir.join("deadline.z3"), None);
        assert_eq!(compute_row_badges(&e_dl, &base, &hi).hint, HintBadge::Available);

        // A game no catalog covers stays None.
        let e_none = entry_with("IFID-N", dir.join("colossal.z5"), None);
        assert_eq!(compute_row_badges(&e_none, &base, &hi).hint, HintBadge::None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Minimal blorb with one Snd resource so resolve_resource_blorb accepts a sibling.
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

    #[test]
    fn scott_title_lookup_by_filename() {
        assert_eq!(scott_title("adv01"), Some("Adventureland"));
        assert_eq!(scott_title("adv13"), Some("The Sorcerer of Claymorgue Castle"));
        // Distinct games that share the "14" number resolve by filename.
        assert_eq!(scott_title("adv14a"), Some("Return to Pirate's Isle"));
        assert_eq!(scott_title("adv14b"), Some("Buckaroo Banzai"));
        // Howarth's Mysterious Adventures reuse numbers 1-11 but key by name.
        assert_eq!(scott_title("1_baton"), Some("The Golden Baton"));
        assert_eq!(scott_title("b_waxworks"), Some("Waxworks"));
        // Lookup is case-insensitive (the readme uses uppercase stems).
        assert_eq!(scott_title("ADV01"), Some("Adventureland"));
        assert_eq!(scott_title("nope"), None);

        // scott_story_title keys off the path's filename stem.
        assert_eq!(scott_story_title(Path::new("adv01.dat")).as_deref(), Some("Adventureland"));
        assert_eq!(scott_story_title(Path::new("quest1.dat")).as_deref(), Some("The Hulk"));
        // Unknown filename -> None (caller falls back to the filename stem).
        assert_eq!(scott_story_title(Path::new("mygame.dat")), None);

        // Homebrew games carry a bundled author + description; IFDB games and
        // unknown stems have neither.
        assert_eq!(scott_author("marooned"), Some("Kim Watt"));
        assert_eq!(scott_description("miner"), Some("Collect four lost treasures in a mine."));
        assert_eq!(scott_author("bond"), None); // author genuinely unknown
        assert!(scott_description("bond").is_some());
        assert_eq!(scott_author("adv01"), None);
        assert_eq!(scott_description("adv01"), None);
        assert_eq!(scott_author("nope"), None);
    }

    #[test]
    fn scott_tuid_lookup_where_known() {
        assert_eq!(scott_tuid("adv01"), Some("dy4ok8sdlut6ddj7")); // Adventureland
        assert_eq!(scott_tuid("adv13"), Some("11tnb08k1jov4hyl")); // Sorcerer of Claymorgue
        assert_eq!(scott_tuid("quest1"), Some("4blbm63qfki4kf2p")); // The Hulk (Questprobe)
        // The `.dat` and graphics `.blb` repackaging of a Mysterious Adventure
        // are the same game, so they share one IFDB id.
        assert_eq!(scott_tuid("1_baton"), Some("v148gq1vx7leo8al"));
        assert_eq!(scott_tuid("golden_baton"), Some("v148gq1vx7leo8al"));
        // The sampler and the homebrew games have a title but no IFDB entry.
        assert_eq!(scott_title("sampler1"), Some("Adventureland (Sampler)"));
        assert_eq!(scott_tuid("sampler1"), None);
        assert_eq!(scott_title("bond"), Some("James Bond Adventure"));
        assert_eq!(scott_tuid("bond"), None);
        assert_eq!(scott_tuid("nope"), None);
        // Rows not known to lack an IFDB entry carry both a title and a TUID.
        const NO_TUID: &[&str] = &[
            "sampler1", "miner", "bond", "burglar", "romulan", "secret", "gamma", "marooned",
            "conquest",
        ];
        for (stem, entry) in scott_titles() {
            assert!(!entry.title.is_empty(), "title for {stem}");
            if !NO_TUID.contains(stem) {
                assert!(entry.tuid.is_some(), "IFDB id for {stem}");
            }
        }
    }

    #[test]
    fn scott_titles_file_parses_without_dupes() {
        let table = scott_titles();
        let lines = include_str!("scott_titles.tsv")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert_eq!(lines, table.len(), "no duplicate filename stems in scott_titles.tsv");
    }
}


