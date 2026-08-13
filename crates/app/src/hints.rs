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

/// Returns true if `name` (case-insensitive) contains an Infocom Solid Gold
/// release/serial marker of the form `-r<digits>-s<digits>` (e.g.
/// `-r52-s871125`). Hand-parsed (no regex): find `-r`, require ≥1 ascii digit,
/// then `-s`, then ≥1 ascii digit. Returns true on the first such occurrence.
pub fn has_release_serial(name: &str) -> bool {
    let b = name.to_ascii_lowercase().into_bytes();
    let n = b.len();
    let mut i = 0;
    while i + 1 < n {
        if b[i] == b'-' && b[i + 1] == b'r' {
            let mut j = i + 2;
            let rstart = j;
            while j < n && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > rstart && j + 1 < n && b[j] == b'-' && b[j + 1] == b's' {
                let mut k = j + 2;
                let sstart = k;
                while k < n && b[k].is_ascii_digit() {
                    k += 1;
                }
                if k > sstart {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Returns true if `file_name` looks like an InvisiClues / hint image by name.
///
/// It must have a `.z3/.z5/.z8` extension AND its stem (lowercased, extension
/// stripped like [`hint_stem`]) either contains one of `hint`, `clue`,
/// `invisiclues`, or ends with the SLAG suffix `inv`, or ends with
/// `_hints`/`-hints`.
///
/// Unlike [`hint_name_matches`], this also recognises the IF-Archive SLAG
/// naming (`deadlineinv.z5`, `stuga_hints.z5`) which carries no keyword.
pub fn is_invisiclues_name(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let has_ext = lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8");
    if !has_ext {
        return false;
    }
    let stem = hint_stem(file_name);
    stem.contains("hint")
        || stem.contains("clue")
        || stem.contains("invisiclues")
        || stem.ends_with("inv")
        || stem.ends_with("_hints")
        || stem.ends_with("-hints")
        // waitingforgo `<abbrev>izm.z5` carries no keyword and no `inv` suffix;
        // recognise it only by the curated table (not a bare `izm` suffix, which
        // could false-positive on an unrelated filename).
        || curated_hint_key(&stem).is_some()
}

/// Returns true if `file_name` is a hint *sidecar* — an InvisiClues/hint image
/// that is NOT itself a full Solid Gold game.
///
/// Infocom Solid Gold releases like `zork1-invclues-r52-s871125.z5` carry a
/// `-r<digits>-s<digits>` marker and are full games (with built-in clues), not
/// standalone hint files, so they are excluded here.
pub fn is_hint_sidecar(file_name: &str) -> bool {
    is_invisiclues_name(file_name) && !has_release_serial(file_name)
}

/// Curated `(hint-stem, game-key)` table for the IF-Archive SLAG InvisiClues
/// collection, whose abbreviated names don't stem-match their games.
///
/// The hint-stem is the lowercased file stem without extension; the game-key is
/// a lowercased keyword present in the game's title/filename.
const SLAG_HINTS: &[(&str, &str)] = &[
    ("deadlineinv", "deadline"),
    ("enchaninv", "enchanter"),
    ("hhgginv", "hitchhiker"),
    ("hollywoodinv", "hollywood"),
    ("lgopinv", "leather"),
    ("lurkinginv", "lurking"),
    ("planetinv", "planetfall"),
    ("sorcinv", "sorcerer"),
    ("spellbinv", "spellbreaker"),
    ("starcrossinv", "starcross"),
    ("stationinv", "stationfall"),
    ("stuga_hints", "stuga"),
    ("suspendedinv", "suspended"),
    ("trininv", "trinity"),
    ("wishbrinv", "wishbringer"),
    ("zork1inv", "zork1"),
    ("zork2inv", "zork2"),
    ("zork3inv", "zork3"),
    ("ztuuinv", "ztuu"),
];

/// Curated `(hint-stem, game-key)` table for the waitingforgo *InvisiClues* set
/// (the `<abbrev>izm.z5` naming). The site is defunct, so downloads come from
/// the Internet Archive; the table also lets a locally-present `*izm.z5` file be
/// detected/hidden/badged like a SLAG sidecar. Multi-game "Collection" images
/// (fant1izm, scifizm, …) are intentionally excluded — they map to no one game.
const IZM_HINTS: &[(&str, &str)] = &[
    ("zork1izm", "zork1"),
    ("zork2izm", "zork2"),
    ("zork3izm", "zork3"),
    // Compound-word keys use the full concatenated game name, so a stray word
    // in an unrelated title (e.g. "Brain Guzzlers from Beyond") can't match.
    ("bzorkizm", "beyondzork"),
    ("zork0izm", "zork0"),
    ("zuuizm", "ztuu"),
    ("wishbizm", "wishbringer"),
    ("enchizm", "enchanter"),
    ("sorcrizm", "sorcerer"),
    ("spellizm", "spellbreaker"),
    ("trntyizm", "trinity"),
    ("starcizm", "starcross"),
    ("spendizm", "suspended"),
    ("plntfizm", "planetfall"),
    ("hitchizm", "hitchhiker"),
    ("amfvizm", "amfv"),
    ("statnizm", "stationfall"),
    ("deadlizm", "deadline"),
    ("witnizm", "witness"),
    ("spectizm", "suspect"),
    ("ballyizm", "ballyhoo"),
    ("moonmizm", "moonmist"),
    ("infdlizm", "infidel"),
    ("seastizm", "seastalker"),
    ("cutthizm", "cutthroats"),
    ("hollyizm", "hollywood"),
    ("shognizm", "shogun"),
    ("leathizm", "leather"),
    ("bureaizm", "bureaucracy"),
    ("nordizm", "nordandbert"),
    ("lurkizm", "lurking"),
    ("plundizm", "plunderedhearts"),
    ("bordrizm", "borderzone"),
    ("sherlizm", "sherlock"),
    ("journizm", "journey"),
    ("arthrizm", "arthur"),
];

/// Canonical titles (from `known_titles.tsv`, keyed by IFID) whose text does not
/// spell the hint catalogs' key for them.
///
/// Every catalog files Zork I as `zork1`, and every title of it writes the
/// number as a roman numeral; `Zork: The Undiscovered Underground` is filed as
/// `ztuu`, an abbreviation that appears nowhere in its name. No substring rule
/// can bridge either gap, so the mapping is listed.
///
/// Matched on the WHOLE normalised canonical title, never a substring — the TSV
/// also names `Mini-Zork I` and `Zork I Demo`, which are different games and
/// must fall through rather than collide with Zork I's clues.
const TITLE_KEYS: &[(&str, &str)] = &[
    ("zorkithegreatundergroundempire", "zork1"),
    ("zorkiithewizardoffrobozz", "zork2"),
    ("zorkiiithedungeonmaster", "zork3"),
    ("zorkzerotherevengeofmegaboz", "zork0"),
    ("zorktheundiscoveredunderground", "ztuu"),
];

/// The catalog key for a story's **identity** — nothing here reads a filename.
///
/// The IFID names the exact build (release + serial), `known_titles.tsv` names
/// the game that build is, and [`TITLE_KEYS`] names the catalog key for that
/// game. `None` when the IFID is unknown or the title spells its own key (the
/// common case, which substring matching already handles).
fn identity_hint_key(ifid: &str) -> Option<&'static str> {
    let norm = normalize_ident(crate::session::known_title(ifid)?);
    TITLE_KEYS.iter().find(|(t, _)| *t == norm).map(|(_, k)| *k)
}

/// The canonical title for a story's identity, normalised for key matching.
///
/// Empty when the IFID names no game we know. Unlike the caller's displayed
/// title this cannot have come from a container's filename, so it is the first
/// thing key matching consults (SQ-0767).
fn identity_ident(ifid: &str) -> String {
    crate::session::known_title(ifid).map(normalize_ident).unwrap_or_default()
}

/// Lowercase a string keeping only ASCII alphanumerics — so `"Beyond Zork"`,
/// `"beyond_zork"`, and `"beyondzork"` all normalise to `"beyondzork"`. Matching
/// game keys against the normalised form lets a multi-word canonical name match
/// while a stray word inside an unrelated title/filename (e.g. the "Beyond" in
/// "Brain Guzzlers from Beyond") cannot.
pub(crate) fn normalize_ident(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The curated game-key for a hint-file `stem`, consulting both the SLAG and the
/// izm tables (SLAG first). `None` when neither table names the stem.
fn curated_hint_key(stem: &str) -> Option<&'static str> {
    SLAG_HINTS
        .iter()
        .chain(IZM_HINTS.iter())
        .find(|(s, _)| *s == stem)
        .map(|(_, key)| *key)
}

/// The lowercased game keyword a hint file belongs to, or `None`.
///
/// First consults the curated tables ([`SLAG_HINTS`] / [`IZM_HINTS`]) by exact
/// stem (so `enchaninv` → `enchanter`, which a naïve strip would get wrong); else derives
/// the base by stripping a trailing hint marker (longest first), returning it
/// when it is ≥3 chars (so `zork1_hints` → `zork1`).
pub fn hint_game_key(file_name: &str) -> Option<String> {
    let stem = hint_stem(file_name);
    if let Some(key) = curated_hint_key(&stem) {
        return Some(key.to_string());
    }
    // Longest markers first so `-invisiclues` isn't shortened to `inv` etc.
    const MARKERS: &[&str] = &[
        "-invisiclues",
        "-hints",
        "_hints",
        "-clues",
        "-hint",
        "_hint",
        "-clue",
        "-inv",
        "_inv",
        "inv",
    ];
    for m in MARKERS {
        if let Some(base) = stem.strip_suffix(m) {
            return (base.len() >= 3).then(|| base.to_string());
        }
    }
    None
}

/// Returns true if the hint file `hint_file_name` is associated with the story
/// identified by `story_stem_or_title` (a filename stem OR a title).
///
/// True when [`hint_game_key`] yields a key of length ≥3 that the lowercased
/// `story_stem_or_title` contains.
pub fn hint_matches_story(hint_file_name: &str, story_stem_or_title: &str) -> bool {
    match hint_game_key(hint_file_name) {
        Some(key) if key.len() >= 3 => {
            let k = normalize_ident(&key);
            !k.is_empty() && normalize_ident(story_stem_or_title).contains(&k)
        }
        _ => false,
    }
}

/// Returns true if the hint file `hint_file_name` belongs to the story with this
/// `ifid` — the same question as [`hint_matches_story`], asked of the story's
/// identity instead of a name (SQ-0767).
///
/// A local `zork1inv.z5` sitting beside `Zork I - The Great Underground
/// Empire.adf` is that floppy's InvisiClues, and no comparison of the two
/// filenames can say so.
pub fn hint_matches_identity(hint_file_name: &str, ifid: &str) -> bool {
    let Some(key) = hint_game_key(hint_file_name) else { return false };
    let k = normalize_ident(&key);
    if k.len() < 3 {
        return false;
    }
    match identity_hint_key(ifid) {
        Some(ik) => normalize_ident(ik) == k,
        None => identity_ident(ifid).contains(&k),
    }
}

/// A downloadable InvisiClues hint file for a story: where to fetch it and what
/// to name the file saved next to the story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintDownload {
    /// Filename to save beside the story, e.g. `deadlineinv.z5`.
    pub filename: String,
    /// Fully-qualified download URL.
    pub url: String,
}

fn slag_url(stem: &str) -> String {
    format!("https://ifarchive.org/if-archive/solutions/slag/{stem}.z5")
}

/// The waitingforgo site is defunct, so izm files come from a fixed Internet
/// Archive snapshot. The `id_` suffix serves the raw bytes (no archive chrome);
/// the request 302-redirects to the nearest capture, which ureq follows.
fn izm_url(stem: &str) -> String {
    format!("https://web.archive.org/web/20161027165356id_/http://www.waitingforgo.com/invisiclues/{stem}.z5")
}

/// Find a downloadable InvisiClues hint file for a story, matched by the game
/// key its **identity** resolves to — falling back to the filename stem and the
/// displayed title only when the identity names no game we know.
///
/// The medium is not the story (SQ-0767): a disk image is named for its box
/// (`Zork I - The Great Underground Empire.adf`), so its filename never
/// contains `zork1` and neither does the title derived from it. `ifid` carries
/// the mounted story's release and serial, which name the build regardless of
/// what the file on disk is called, so it is consulted first.
///
/// SLAG (live IF Archive) is preferred; the izm set (Internet Archive) is the
/// fallback for games SLAG doesn't cover. Returns `None` when no catalog entry
/// matches. A key must be ≥3 chars to match (guards against spurious hits).
pub fn hint_download_for(ifid: &str, game_stem: &str, game_title: &str) -> Option<HintDownload> {
    let identity_key = identity_hint_key(ifid);
    let canonical = identity_ident(ifid);
    let stem = normalize_ident(game_stem);
    let title = normalize_ident(game_title);
    let matches = |key: &str| {
        let k = normalize_ident(key);
        if k.len() < 3 {
            return false;
        }
        match identity_key {
            // The identity names its catalog key outright — authoritative, and
            // exclusive: no other key can be right for this build.
            Some(ik) => normalize_ident(ik) == k,
            // Else the identity-resolved canonical title, then — last resort —
            // the filename stem and the displayed title, which for a story
            // mounted out of a container are the CONTAINER's, not the game's.
            None => canonical.contains(&k) || stem.contains(&k) || title.contains(&k),
        }
    };
    if let Some((s, _)) = SLAG_HINTS.iter().find(|(_, k)| matches(k)) {
        return Some(HintDownload { filename: format!("{s}.z5"), url: slag_url(s) });
    }
    if let Some((s, _)) = IZM_HINTS.iter().find(|(_, k)| matches(k)) {
        return Some(HintDownload { filename: format!("{s}.z5"), url: izm_url(s) });
    }
    None
}

/// True if `text` is the InvisiClues narrow-screen boot banner. The izm hint
/// files print it when the advertised screen width (a single header byte, so
/// ≤255) is below their longest menu-item name, which can reach 512 chars — so
/// it fires for any real terminal. Matched on the stable phrase (not the width
/// number) so the hint boot can auto-skip it. See `hint_opening` in main.rs.
pub fn is_narrow_screen_warning(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("your screen is only") && t.contains("characters wide")
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
///
/// A bare `starts_with` would let story `zork` match `zork2_hints.z5`, so the
/// prefix must be followed by end-of-stem or a separator (not another
/// alphanumeric): the hint name is the story name plus a hint suffix, never a
/// longer word that merely begins the same way.
fn stem_matches_story(story_stem: &str, candidate_name: &str) -> bool {
    if story_stem.is_empty() {
        return false;
    }
    let story = story_stem.to_ascii_lowercase();
    let cand = hint_stem(candidate_name);
    match cand.strip_prefix(&story) {
        Some(rest) => rest.chars().next().is_none_or(|c| !c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// Rank hint candidate names by story-stem preference and return the chosen one.
///
/// Tiers:
/// 1. any candidate the story owns — its stem starts with `story_stem`
///    ([`stem_matches_story`]) or its curated/derived game key matches the story
///    ([`hint_matches_story`]) → the first such after a stable name sort
///    (deterministic regardless of readdir order);
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
    if let Some(m) = names
        .iter()
        .find(|n| stem_matches_story(story_stem, n) || hint_matches_story(n, story_stem))
    {
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
/// 2. Sibling files: any `.z3/.z5/.z8` whose name is a hint sidecar
///    ([`is_hint_sidecar`]) in the same directory as `story_path` — this finds
///    SLAG files like `deadlineinv.z5` while skipping full Solid Gold games
///    like `zork1-invclues-r52-s871125.z5`.
/// 3. Sibling ZIP: any `.zip` in the same directory that contains a hint-sidecar
///    entry; returns `ZipEntry` so the caller can extract the bytes with
///    `read_zip_entry`.
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
                // Step 2: collect hint sidecars (ranked below); skips full
                // Solid Gold games that carry a release/serial marker.
                if is_hint_sidecar(name) {
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

/// Return the best-ranked hint-sidecar entry in `zip_path`, preferring an entry
/// the story owns (stem prefix or game-key match).
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
        if is_hint_sidecar(basename) {
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

/// Which release medium a story was mounted out of, when it was one at all.
///
/// Re-exported rather than declared here (SQ-0839): the medium implies a machine
/// implies an interpreter number, and `zvm-cli` needs that same conclusion
/// without depending on `app`. `blorb` is where these filesystems are
/// recognised and the only crate both front-ends share, so it owns the type and
/// the mapping — see [`blorb::medium`]. Every existing `app::hints::DiskImage`
/// spelling keeps working.
pub use blorb::medium::DiskImage;

/// Read a story file's executable bytes, transparently unwrapping a ZIP whose
/// first `.z3/.z5/.z8` entry is the story, or a release disk image — Amiga or
/// Macintosh — whose filesystem holds one. Does not classify the engine — see
/// [`load_story`] / [`extract_story`].
///
/// The flag says the bytes were mounted out of a disk image rather than read as
/// a plain file, and which kind, so callers can name the container the story
/// came off (SQ-0737).
fn read_story_file(path: &Path) -> io::Result<(Vec<u8>, Option<DiskImage>)> {
    let raw = std::fs::read(path)?;
    // SQ-0719: an original Amiga release floppy. Mount it and take the file
    // whose CONTENT is a story — AmigaDOS has no extensions, and the disk's
    // `Story.data` convention is a tiebreak, not a guarantee.
    if blorb::adf::Adf::looks_like_adf(&raw) {
        let adf = blorb::adf::Adf::mount(raw)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
        return match adf.story() {
            Some((_, bytes)) => Ok((bytes, Some(DiskImage::Adf))),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no story file on the disk image {} ({} files; is this the boot disk?)",
                    path.display(),
                    adf.files().len()
                ),
            )),
        };
    }
    // SQ-0837: a Macintosh release floppy, the same idea one filesystem over.
    if blorb::hfs::Hfs::looks_like_hfs(&raw) {
        let hfs = blorb::hfs::Hfs::mount(raw)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
        return match hfs.story() {
            Some((_, bytes)) => Ok((bytes, Some(DiskImage::Hfs))),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no story file on the disk image {} ({} files on {}; is this the boot disk?)",
                    path.display(),
                    hfs.files().len(),
                    hfs.volume_name()
                ),
            )),
        };
    }
    if raw.starts_with(ZIP_MAGIC) {
        // It's a ZIP — find the first story entry.
        let pred = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".z3") || lower.ends_with(".z5") || lower.ends_with(".z8")
        };
        match read_zip_entry(path, pred)? {
            Some(bytes) => Ok((bytes, None)),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no .z3/.z5/.z8 entry found in zip: {}", path.display()),
            )),
        }
    } else {
        Ok((raw, None))
    }
}

/// Load a story from `path`, classified by engine ([`LoadedStory`]).
///
/// Unwraps a ZIP (first `.z*` entry) and a Blorb container; a raw `.ulx`
/// (`Glul` magic) is recognised as Glulx, and any other raw bytes are treated
/// as Z-code (preserving the historical default).
pub fn load_story(path: &Path) -> io::Result<LoadedStory> {
    Ok(load_mounted_story(path)?.0)
}

/// As [`load_story`], plus which **release disk image** the story was mounted
/// out of, if it was one rather than a plain file (SQ-0737, SQ-0837).
///
/// The answer is the mount's own — an image is recognised by its own filesystem,
/// never by its name — so a disk image with any extension is reported as one and
/// a mis-named ordinary story file is not.
pub fn load_mounted_story(path: &Path) -> io::Result<(LoadedStory, Option<DiskImage>)> {
    let (bytes, disk_image) = read_story_file(path)?;
    Ok((extract_story(bytes)?, disk_image))
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
        Ok((blorb::ExecKind::Scott, data)) => {
            // SQ-0629: gate the SAAI payload behind the same content sniff the
            // raw-`.dat` path above uses — a hostile blorb must not reach
            // scott's loader with arbitrary bytes just by claiming an SAAI
            // exec chunk.
            if !std::str::from_utf8(data).is_ok_and(scott::looks_like_scott) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Blorb SAAI executable does not look like a Scott Adams database",
                ));
            }
            Ok(LoadedStory::Scott(data.to_vec()))
        }
        Err(e) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Blorb has no executable: {e:?}"),
        )),
    }
}

/// Cap on one extracted ZIP entry (SQ-0660). Story files here are Z-code
/// images, whose format tops out at 512 KiB; 4 MiB is generous headroom while
/// still stopping a hostile zip from inflating a multi-GB entry into memory.
const MAX_ZIP_ENTRY: u64 = 4 * 1024 * 1024;

/// Return the bytes of the first ZIP entry whose name satisfies `pred`.
///
/// Returns `Ok(None)` if no entry matches.  Returns `Err` if the file cannot
/// be opened, is not a valid ZIP, or the matched entry inflates beyond
/// [`MAX_ZIP_ENTRY`].
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
            let name = entry.name().to_string();
            // Capped read (one byte past the cap detects overflow without ever
            // buffering the excess): the entry's declared size is attacker
            // data, so the *inflated* stream itself is what gets bounded.
            let mut buf = Vec::new();
            (&mut entry).take(MAX_ZIP_ENTRY + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > MAX_ZIP_ENTRY {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("zip entry '{name}' inflates beyond {MAX_ZIP_ENTRY} bytes"),
                ));
            }
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
    fn has_release_serial_detects_solid_gold_marker() {
        assert!(has_release_serial("zork1-invclues-r52-s871125.z5"));
        assert!(has_release_serial("deadline-r27-s851006.z3"));
        assert!(!has_release_serial("deadlineinv.z5"));
        assert!(!has_release_serial("zork1_hints.z5"));
    }

    #[test]
    fn is_hint_sidecar_recognizes_slag_and_excludes_solid_gold() {
        // SLAG + user hint files are sidecars.
        assert!(is_hint_sidecar("deadlineinv.z5"));
        assert!(is_hint_sidecar("stuga_hints.z5"));
        assert!(is_hint_sidecar("zork1-invisiclues.z5"));
        assert!(is_hint_sidecar("zork1_hints.z5"));
        // The story itself is not a sidecar.
        assert!(!is_hint_sidecar("zork1.z5"));
        // Solid Gold full games (with release/serial) are not sidecars.
        assert!(!is_hint_sidecar("zork1-invclues-r52-s871125.z5"));
        assert!(!is_hint_sidecar("enchanter-r29-s860820.z3"));
    }

    #[test]
    fn hint_game_key_curated_and_derived() {
        assert_eq!(hint_game_key("deadlineinv.z5").as_deref(), Some("deadline"));
        assert_eq!(hint_game_key("hhgginv.z5").as_deref(), Some("hitchhiker"));
        assert_eq!(hint_game_key("lgopinv.z5").as_deref(), Some("leather"));
        // Curated must beat the (wrong) derived `enchan`.
        assert_eq!(hint_game_key("enchaninv.z5").as_deref(), Some("enchanter"));
        assert_eq!(hint_game_key("zork1_hints.z5").as_deref(), Some("zork1"));
        assert_eq!(hint_game_key("stuga_hints.z5").as_deref(), Some("stuga"));
    }

    #[test]
    fn hint_matches_story_associates_by_key() {
        assert!(hint_matches_story(
            "hhgginv.z5",
            "The Hitchhiker's Guide to the Galaxy"
        ));
        assert!(hint_matches_story("hhgginv.z5", "hitchhiker-r59-s851108"));
        assert!(hint_matches_story("deadlineinv.z5", "deadline-r27-s851006"));
        assert!(!hint_matches_story("deadlineinv.z5", "zork1-r88-s840726"));
    }

    /// The seven SLAG games added beyond the Phase-1 table must resolve.
    #[test]
    fn slag_new_entries_resolve() {
        for (file, key) in [
            ("suspendedinv.z5", "suspended"),
            ("trininv.z5", "trinity"),
            ("wishbrinv.z5", "wishbringer"),
            ("zork1inv.z5", "zork1"),
            ("zork2inv.z5", "zork2"),
            ("zork3inv.z5", "zork3"),
            ("ztuuinv.z5", "ztuu"),
        ] {
            assert_eq!(hint_game_key(file).as_deref(), Some(key), "{file}");
            assert!(is_hint_sidecar(file), "{file} must be a sidecar");
        }
    }

    /// A locally-present `*izm.z5` file (no `inv` suffix, no keyword) is only
    /// recognised via the curated izm table — and then hides/associates like SLAG.
    #[test]
    fn izm_local_files_are_detected_and_keyed() {
        assert!(is_invisiclues_name("deadlizm.z5"));
        assert!(is_hint_sidecar("deadlizm.z5"));
        assert_eq!(hint_game_key("deadlizm.z5").as_deref(), Some("deadline"));
        assert_eq!(hint_game_key("witnizm.z5").as_deref(), Some("witness"));
        assert_eq!(hint_game_key("bzorkizm.z5").as_deref(), Some("beyondzork"));
        assert!(hint_matches_story("witnizm.z5", "The Witness"));
        // A bare `izm` suffix that isn't in the table is NOT a hint file.
        assert!(!is_invisiclues_name("mechanizm.z5"));
    }

    /// An empty IFID means "identity says nothing", so every case here exercises
    /// the filename/title fallback — the common path for the many stories that
    /// are only ever bare files, which must keep working (SQ-0767).
    #[test]
    fn hint_download_prefers_slag_then_izm() {
        // A SLAG-covered game: prefer the live IF Archive file.
        let d = hint_download_for("", "deadline", "Deadline").expect("deadline has a hint");
        assert_eq!(d.filename, "deadlineinv.z5");
        assert!(d.url.contains("ifarchive.org/if-archive/solutions/slag/deadlineinv.z5"), "{}", d.url);

        // A game only the izm set covers: fall back to the Internet Archive.
        let w = hint_download_for("", "witness", "The Witness").expect("witness has an izm hint");
        assert_eq!(w.filename, "witnizm.z5");
        assert!(w.url.contains("web.archive.org"), "{}", w.url);
        assert!(w.url.ends_with("witnizm.z5"), "{}", w.url);

        // Match on title when the stem is opaque.
        assert!(hint_download_for("", "hhgg", "The Hitchhiker's Guide to the Galaxy").is_some());

        // A game with no hint anywhere.
        assert!(hint_download_for("", "adventure", "Colossal Cave").is_none());
    }

    /// Beyond Zork keys on "beyond", never bare "zork", so it must not collide
    /// with zork1/2/3 (and vice-versa).
    #[test]
    fn hint_download_zork_variants_dont_collide() {
        assert_eq!(hint_download_for("", "zork1", "Zork I").unwrap().filename, "zork1inv.z5");
        assert_eq!(hint_download_for("", "beyondzork", "Beyond Zork").unwrap().filename, "bzorkizm.z5");
        // Canonical multi-word/underscored names still match via normalisation.
        assert_eq!(hint_download_for("", "beyond_zork", "").unwrap().filename, "bzorkizm.z5");
        assert_eq!(hint_download_for("", "", "Beyond Zork").unwrap().filename, "bzorkizm.z5");
        assert_eq!(hint_download_for("", "zork0", "Zork Zero").unwrap().filename, "zork0izm.z5");
    }

    /// Regression: a stray common word in a title must not match a compound-word
    /// game key. "Brain Guzzlers from Beyond" contains "beyond" but is not
    /// Beyond Zork, so it gets no hint (badge stays dark).
    #[test]
    fn hint_download_rejects_stray_word_match() {
        assert!(hint_download_for("", "Brain_Guzzlers_from_Beyond!.gblorb", "Brain Guzzlers from Beyond!").is_none());
        assert!(!hint_matches_story("bzorkizm.z5", "Brain Guzzlers from Beyond!"));
    }

    // ── SQ-0767: identity, not filename ─────────────────────────────────────

    /// The four Amiga floppies the bug was reported on, by the release+serial
    /// their disks actually carry (pinned in `tests/real_media_releases.rs`).
    /// The container is named for the box, so its stem and the title derived
    /// from it contain no catalog key — only the IFID can say which game it is.
    #[test]
    fn a_disk_image_finds_its_invisiclues_by_identity_not_filename() {
        let cases = [
            ("ZCODE-88-840726-A129", "Zork I - The Great Underground Empire", "zork1inv.z5"),
            ("ZCODE-48-840904-D899", "Zork II - The Wizard of Frobozz", "zork2inv.z5"),
            ("ZCODE-17-840727-2E7A", "Zork III - The Dungeon Master", "zork3inv.z5"),
            ("ZCODE-16-970828-1185", "Zork - The Undiscovered Underground", "ztuuinv.z5"),
            ("ZCODE-366-890323-C5CD", "Zork Zero - The Revenge of Megaboz", "zork0izm.z5"),
        ];
        for (ifid, stem, want) in cases {
            // The title the picker shows for a container is the canonical one,
            // which spells the number the way the box does — also no key.
            let title = stem.replace(" - ", ": ");
            assert!(
                !normalize_ident(stem).contains("zork1") && !normalize_ident(&title).contains("ztuu"),
                "the premise: no catalog key is in the container's name ({stem})"
            );
            let dl = hint_download_for(ifid, stem, &title)
                .unwrap_or_else(|| panic!("{stem}: identity {ifid} must find its InvisiClues"));
            assert_eq!(dl.filename, want, "{stem}");
        }
    }

    /// The identity is authoritative and exclusive: knowing a build is Zork II
    /// must not let Zork I's or Beyond Zork's clues match it.
    #[test]
    fn an_identified_story_matches_only_its_own_key() {
        let dl = hint_download_for("ZCODE-48-840904-D899", "zork1", "Beyond Zork").unwrap();
        assert_eq!(dl.filename, "zork2inv.z5", "identity beats a misleading stem AND title");
    }

    /// A local sidecar sitting beside a disk image is associated by identity —
    /// comparing the two filenames cannot do it.
    #[test]
    fn a_local_sidecar_is_associated_with_a_disk_image_by_identity() {
        assert!(hint_matches_identity("zork1inv.z5", "ZCODE-88-840726-A129"));
        assert!(hint_matches_identity("zork3izm.z5", "ZCODE-17-840727-2E7A"));
        // Beyond Zork's title spells its own key, so the canonical title path
        // (no TITLE_KEYS row needed) carries it.
        assert!(hint_matches_identity("bzorkizm.z5", "ZCODE-57-871221-C5AD"));
        // Wrong game, and an unknown identity, both stay unmatched.
        assert!(!hint_matches_identity("zork2inv.z5", "ZCODE-88-840726-A129"));
        assert!(!hint_matches_identity("zork1inv.z5", "ZCODE-1-000000-0000"));
    }

    /// `Mini-Zork I` and `Zork I Demo` are separate entries in the title table
    /// and separate games; matching the WHOLE canonical title keeps Zork I's
    /// clues off them.
    #[test]
    fn title_keys_match_whole_titles_so_near_namesakes_dont_collide() {
        assert_eq!(identity_hint_key("ZCODE-88-840726-A129"), Some("zork1"));
        assert_eq!(identity_hint_key("ZCODE-34-871124-0000"), None, "Mini-Zork I");
        assert_eq!(identity_hint_key("ZCODE-15-840330-0000"), None, "Zork I Demo");
    }

    #[test]
    fn narrow_screen_warning_matches_any_width() {
        // The real banner, at two different advertised widths.
        assert!(is_narrow_screen_warning(
            "WARNING: Your screen is only 80 characters wide. The names of some menu items contain up to 512 characters"
        ));
        assert!(is_narrow_screen_warning("your screen is only 255 characters wide."));
        // Ordinary clue/menu text must not match.
        assert!(!is_narrow_screen_warning("Type the number of the topic you want a hint for."));
        assert!(!is_narrow_screen_warning(""));
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

    /// SQ-0660: a zip entry that inflates beyond the cap must error instead of
    /// allocating unbounded memory; a small entry still extracts.
    #[test]
    fn read_zip_entry_caps_a_huge_inflated_entry() {
        use std::io::Write as _;

        let base = std::env::temp_dir().join(format!("bm-zipcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // A deflated entry of MAX_ZIP_ENTRY + 128 KiB of zeros: tiny on disk,
        // huge inflated — exactly the zip-bomb shape the cap exists for.
        let zip_path = base.join("bomb.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("huge.z5", opts).unwrap();
            let chunk = vec![0u8; 64 * 1024];
            for _ in 0..(MAX_ZIP_ENTRY / chunk.len() as u64 + 2) {
                zw.write_all(&chunk).unwrap();
            }
            zw.finish().unwrap();
        }
        let err = read_zip_entry(&zip_path, |n| n.ends_with(".z5"))
            .expect_err("an over-cap entry must error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("huge.z5"), "error names the entry: {err}");

        // A normal-sized entry is unaffected.
        let ok_path = base.join("ok.zip");
        {
            let file = std::fs::File::create(&ok_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zw.start_file("small.z5", opts).unwrap();
            zw.write_all(&[5u8; 128]).unwrap();
            zw.finish().unwrap();
        }
        let bytes = read_zip_entry(&ok_path, |n| n.ends_with(".z5")).unwrap().unwrap();
        assert_eq!(bytes, vec![5u8; 128]);

        let _ = std::fs::remove_dir_all(&base);
    }

    /// SQ-0629: a blorb claiming a Scott Adams (`SAAI`) executable only loads
    /// when its payload passes the same `looks_like_scott` sniff the raw-`.dat`
    /// path uses — arbitrary bytes must never reach scott's loader.
    #[test]
    fn extract_story_gates_a_blorb_saai_payload_behind_the_scott_sniff() {
        // Binary junk behind an SAAI exec chunk: rejected.
        let hostile = make_blorb(b"SAAI", &[0xFF, 0x00, 0xC3, 0x28, 0x9A, 0x01]);
        let err = extract_story(hostile).expect_err("non-Scott SAAI payload must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Text that fails the header sniff behind SAAI: also rejected.
        let not_scott = make_blorb(b"SAAI", b"just some readme text, not a database");
        assert!(extract_story(not_scott).is_err());

        // A genuine Scott header (12 sane ints) still loads.
        const MINI: &str = "\n32767 1 0 1 2 6 1 0 3 125 0 1\n150 1 0 0 0 0 0 0\n";
        let genuine = make_blorb(b"SAAI", MINI.as_bytes());
        assert_eq!(
            extract_story(genuine).unwrap(),
            LoadedStory::Scott(MINI.as_bytes().to_vec())
        );
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
    fn resolve_slag_hint_via_game_key() {
        // Curated SLAG hint (no keyword, no stem prefix) resolves via game key.
        let dir = scratch_dir(
            "resolve-slag",
            &["deadline-r27-s851006.z3", "deadlineinv.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("deadline-r27-s851006.z3"), "IFID", &empty);
        assert_eq!(r, HintResolution::File(dir.join("deadlineinv.z5")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_skips_solid_gold_sibling() {
        // A Solid Gold full game (with -rNN-sNNNNNN) is not a hint sidecar, so
        // it is never offered as another story's hints.
        let dir = scratch_dir(
            "resolve-solidgold",
            &["story.z5", "zork1-invclues-r52-s871125.z5"],
        );
        let empty = HintIndex { map: HashMap::new() };

        let r = resolve_hint_source(&dir.join("story.z5"), "IFID", &empty);
        assert_eq!(r, HintResolution::AskUser);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stem_matches_story_is_case_insensitive_prefix() {
        assert!(stem_matches_story("zork1", "zork1_hints.z5"));
        assert!(stem_matches_story("zork1", "Zork1.hints.z5"));
        assert!(stem_matches_story("zork1", "ZORK1-invisiclues.z5"));
        assert!(!stem_matches_story("zork1", "zork2_hints.z5"));
    }

    #[test]
    fn stem_matches_story_requires_a_word_boundary() {
        // A bare prefix must not cross-wire: story `zork` ≠ `zork2_hints`.
        assert!(!stem_matches_story("zork", "zork2_hints.z5"));
        assert!(!stem_matches_story("zork1", "zork10_hints.z5"));
        // Exact stem, or prefix + separator, does match.
        assert!(stem_matches_story("zork", "zork_hints.z5"));
        assert!(stem_matches_story("zork", "zork.hints.z5"));
        assert!(stem_matches_story("zork", "zork-invisiclues.z5"));
        // An empty story stem never matches (pathological path).
        assert!(!stem_matches_story("", "invisiclues.z5"));
    }
}
