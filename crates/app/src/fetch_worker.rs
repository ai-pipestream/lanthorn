//! Background IFDB fetch worker, driven by the `f` (this story, forced) and
//! `r` (whole library, skip already-fetched) keys.
//!
//! `f` is **not** a special case: it is a [`FetchOrder`] of length one with
//! `forced: true`. One worker, one progress channel, one cancel path serve
//! both keys.
//!
//! Mirrors `cover::CoverDecoder`: a long-lived `std::thread` fed by one
//! `mpsc` request channel and drained (non-blocking) from another. No async
//! runtime.
//!
//! The skip decision is the WORKER's, made per story just before fetching —
//! it re-reads the sidecar via `story_info::load`/`needs_fetch` immediately
//! before every fetch, never pre-filtered by the caller. That keeps a `r`
//! sweep correct even if it was cancelled and restarted, or if `f` refreshed
//! a story while the sweep was queued.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::ifdb::{FetchError, FetchOutcome, MetadataSource};
use crate::ifiction::IFiction;
use crate::story_info::{self, FetchedMeta, StoryInfo};

/// One story in an order — which is not the same as one *file* since a disk
/// image can hold six games (SQ-0859): the container's path, which story on it
/// (`None` for a loose file or a single-story image), and its IFID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTarget {
    pub path: PathBuf,
    pub disk_entry: Option<String>,
    pub ifid: String,
    /// The story's game-folder key when the caller already has it (a browser
    /// row does, for free). Without it the worker must open the story file to
    /// work the key out, which a library-wide fetch would then do for every
    /// story, including those already identified.
    pub key: Option<String>,
}

impl FetchTarget {
    /// The target one browser row stands for.
    pub fn row(entry: &crate::picker::StoryEntry) -> FetchTarget {
        FetchTarget {
            path: entry.path.clone(),
            disk_entry: entry.meta.disk_entry.clone(),
            ifid: entry.meta.ifid.clone(),
            key: Some(entry.story_key()),
        }
    }
}

/// A batch of stories to fetch. `forced` (`f`) ignores the sidecar cache
/// entirely; unset (`r`) skips a story already at `FETCH_VERSION`.
pub struct FetchOrder {
    pub stories: Vec<FetchTarget>,
    pub forced: bool,
    /// Manual override (SQ-0371): when set, each story is fetched by this IFDB
    /// page id (tuid) instead of its IFID — for a story the user pointed at an
    /// IFDB page by hand. Implies forced. Used with a single-story order.
    pub id_override: Option<String>,
}

/// What happened to one story in an order.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Fetched and wrote a `FetchedMeta` block (found, or an authoritative
    /// not-found — see [`Outcome::NotFound`] for the latter reported
    /// separately).
    Fetched,
    /// The sidecar was already current; no request was made, no delay paid.
    Skipped,
    /// No request: the story's IFID is listed by an IFDB record already held
    /// in the shared store (or fetched earlier in this run), so its link was
    /// written from that record exactly as if it had been fetched (SQ-1786).
    Linked,
    /// IFDB has no record for this IFID. This IS a completed answer: a
    /// `FetchedMeta` with `not_found: true` was written.
    NotFound,
    /// A transport error. No sidecar block was written, so a later `r` will
    /// retry this story.
    Failed(String),
}

/// One story's result, reported as the worker finishes it. `done`/`total`
/// count every story in the order, skips included, so the progress line
/// never stalls on a mostly-cached library.
#[derive(Debug)]
pub struct FetchProgress {
    pub done: usize,
    pub total: usize,
    pub path: PathBuf,
    /// Which story on `path` this result is for, when the path is a disk image
    /// holding several (SQ-0859) — without it the browser cannot tell which of
    /// six rows off one compilation just finished.
    pub disk_entry: Option<String>,
    pub title: String,
    pub outcome: Outcome,
}

/// Background fetch worker: one long-lived thread, fed `FetchOrder`s and
/// draining `FetchProgress` non-blocking. Cancel is checked BETWEEN stories
/// only, never mid-write, so a cancelled sweep leaves every written sidecar
/// complete and valid.
pub struct Fetcher {
    req_tx: mpsc::Sender<FetchOrder>,
    res_rx: mpsc::Receiver<FetchProgress>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    _worker: thread::JoinHandle<()>,
}

impl Fetcher {
    pub fn new(source: Box<dyn MetadataSource>, roots: crate::data_roots::DataRoots, delay: Duration) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<FetchOrder>();
        let (res_tx, res_rx) = mpsc::channel::<FetchProgress>();
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_busy = Arc::clone(&busy);
        let worker = thread::spawn(move || {
            while let Ok(order) = req_rx.recv() {
                let total = order.stories.len();
                let id_override = order.id_override;
                // IFDB entries fetched during THIS order: a forced refresh asks
                // once per entry, then links the other copies from the result.
                let mut fetched_this_run: HashSet<String> = HashSet::new();
                for (i, target) in order.stories.into_iter().enumerate() {
                    if worker_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let progress = fetch_one(
                        source.as_ref(), &roots, target, order.forced,
                        id_override.as_deref(), delay, i, total,
                        &mut fetched_this_run,
                    );
                    if res_tx.send(progress).is_err() {
                        worker_busy.store(false, Ordering::Relaxed);
                        return;
                    }
                }
                worker_busy.store(false, Ordering::Relaxed);
            }
        });
        Self { req_tx, res_rx, cancel, busy, _worker: worker }
    }

    /// Queue an order. Resets any earlier cancellation — this is a fresh
    /// order, not a continuation.
    pub fn request(&self, order: FetchOrder) {
        self.cancel.store(false, Ordering::Relaxed);
        self.busy.store(true, Ordering::Relaxed);
        let _ = self.req_tx.send(order);
    }

    /// Non-blocking drain of all progress ready so far.
    pub fn drain(&self) -> Vec<FetchProgress> {
        self.res_rx.try_iter().collect()
    }

    /// Ask the worker to stop after the story currently in flight. Checked
    /// between stories only.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }
}

/// The IFDB game id for a Scott Adams `.dat` at `path`, or `None` if it isn't a
/// recognised Scott adventure. A Scott database has no embedded IFID and its
/// computed IFID never resolves on IFDB, so the fetch looks the game up by this
/// id instead. Scott databases are small ASCII text, so the read is capped — a
/// large binary story (Glulx/Z-code) fails the sniff without being slurped whole.
fn scott_ifdb_id(path: &Path, disk_entry: Option<&str>) -> Option<String> {
    // Recognise a Scott story the same way the picker does, so a `.blb` blorb
    // (whose SAAI exec chunk holds the `.dat`) is detected as well as a raw
    // `.dat` — sniffing the raw file text alone misses the binary blorb. The
    // TUID is keyed by filename stem, since a Scott database has no embedded IFID.
    //
    // For a story inside a zip the stem is the ENTRY's (`adv01` in
    // `AdamsGames.zip`), which is the name the table knows; the archive's own
    // stem names nothing, and the eighteen games in that zip fetched as
    // "not on IFDB" until this looked at the right name.
    let (loaded, _) = crate::hints::load_mounted_story_from(path, disk_entry).ok()?;
    if !matches!(loaded, crate::hints::LoadedStory::Scott(_)) {
        return None;
    }
    let named: &Path = disk_entry.map(Path::new).unwrap_or(path);
    let stem = named.file_stem().and_then(|s| s.to_str())?;
    crate::picker::scott_tuid(stem).map(str::to_string)
}

/// Handle one story: skip-check, fetch-or-skip, sidecar write, cover write.
/// Sleeps `delay` only when an actual fetch was attempted — a skip costs no
/// request, so it must not sleep.
#[allow(clippy::too_many_arguments)]
fn fetch_one(
    source: &dyn MetadataSource,
    roots: &crate::data_roots::DataRoots,
    target: FetchTarget,
    forced: bool,
    id_override: Option<&str>,
    delay: Duration,
    index: usize,
    total: usize,
    fetched_this_run: &mut HashSet<String>,
) -> FetchProgress {
    let FetchTarget { path, disk_entry, ifid, key } = target;
    // The story's OWN directory, not the one the image's tiebreak would name:
    // fetching *Leather Goddesses* off `INFOCOM6` must not write its metadata
    // (and its cover) into *Sherlock*'s directory (SQ-0859).
    // Taken from the caller when it has it: working the key out opens the story
    // file, which a sweep over an already-identified library must not do.
    let key = key.unwrap_or_else(|| crate::storage::story_key_at_from(&path, disk_entry.as_deref()));
    let game_dir = crate::storage::game_dir(roots.catalogue(), &key);
    let existing = story_info::load(&game_dir, &ifid);
    // The IFDB page id a previous answer recorded — from a hand-set IFDB URL
    // (SQ-0371), from an IFDB-search download, or from an ordinary by-IFID
    // fetch. Reused, because the *reason* a story has one is usually that IFDB
    // does not index the IFID we compute for it: asking by IFID again answers
    // an authoritative NotFound, and `write_fetched` then replaces a perfectly
    // good record with that not-found block. `f` on *City of Secrets* — Inform 6
    // Glulx, whose Treaty IFID is its file's MD5 and not our `GLULX-…` hash —
    // emptied the row it was pressed on (SQ-1226). The sidecar's identity check
    // is what makes this safe: swap a different game in under the same filename
    // and `existing` is `None`, so nothing is remembered on its behalf.
    let remembered = existing
        .as_ref()
        .and_then(|i| i.fetched.as_ref())
        .and_then(|f| f.ifdb_tuid.clone())
        // The link outlives its record: a copy whose shared entry went missing
        // still knows which page it is (SQ-1723).
        .or_else(|| existing.as_ref().and_then(|_| story_info::linked_tuid(&game_dir)));

    // A manual IFDB-id fetch (SQ-0371) always runs and always fetches by that
    // id; only an IFID-keyed fetch consults the skip cache.
    let forced = forced || id_override.is_some();
    let linked = if id_override.is_none() && story_info::needs_fetch(existing.as_ref(), forced) {
        link_from_store(&game_dir, &ifid, forced, remembered.as_deref(), fetched_this_run)
    } else {
        None
    };
    let (title, outcome) = if let Some(meta) = linked {
        let title = meta.title.clone().unwrap_or_else(|| stem_title(&path));
        write_fetched(roots, &game_dir, &ifid, meta);
        (title, Outcome::Linked)
    } else if !story_info::needs_fetch(existing.as_ref(), forced) {
        (stem_title(&path), Outcome::Skipped)
    } else {
        // A known Scott Adams adventure is fetched by its mapped IFDB id (its own
        // computed IFID never resolves on IFDB). A manual id_override (SQ-0371)
        // still wins, and so does a remembered page id — both name THIS story's
        // page, where the bundled table only knows a filename stem.
        let scott_id = if id_override.is_none() && remembered.is_none() {
            scott_ifdb_id(&path, disk_entry.as_deref())
        } else {
            None
        };
        let fetched = match id_override.or(remembered.as_deref()).or(scott_id.as_deref()) {
            Some(id) => source.fetch_by_id(id),
            None => source.fetch(&ifid),
        };
        let result = match fetched {
            Ok(FetchOutcome::Found(iff)) => {
                let cover = maybe_fetch_cover(source, &game_dir, &path, &iff);
                let title = iff.title.clone().unwrap_or_else(|| stem_title(&path));
                let meta = found_meta(&iff, cover);
                if let Some(tuid) = crate::ifdb_store::shared_tuid(&meta) {
                    fetched_this_run.insert(tuid.to_string());
                    if let Some(cat) = crate::ifdb_store::catalogue_of(&game_dir) {
                        // Best-effort, like the cover: a failure only costs a
                        // later request.
                        let _ = crate::ifdb_store::record_ifids(cat, tuid, &iff.ifids);
                    }
                }
                write_fetched(roots, &game_dir, &ifid, meta);
                (title, Outcome::Fetched)
            }
            Ok(FetchOutcome::NotFound) => {
                write_fetched(roots, &game_dir, &ifid, not_found_meta());
                (stem_title(&path), Outcome::NotFound)
            }
            Err(FetchError::Transport(msg)) => {
                // No sidecar write at all: leave whatever was there, so a
                // later `r` retries it.
                (stem_title(&path), Outcome::Failed(msg))
            }
        };
        thread::sleep(delay);
        result
    };

    FetchProgress { done: index + 1, total, path, disk_entry, title, outcome }
}

/// The stored IFDB record this story can be linked to without a request, or
/// `None` when it needs its own fetch (SQ-1786).
///
/// * An unforced run links only a story with no current record of its own, and
///   only from a record that is itself current: its IFID must be listed by
///   exactly one entry (IFDB duplicates make the map silent, not wrong).
/// * A forced run links only to an entry already fetched in THIS run, so a
///   refresh still asks IFDB once per entry and never serves a stale record.
fn link_from_store(
    game_dir: &Path,
    ifid: &str,
    forced: bool,
    remembered: Option<&str>,
    fetched_this_run: &HashSet<String>,
) -> Option<FetchedMeta> {
    let cat = crate::ifdb_store::catalogue_of(game_dir)?;
    let tuid = if forced {
        let t = remembered.map(str::to_string).or_else(|| crate::ifdb_store::tuid_for_ifid(cat, ifid))?;
        fetched_this_run.contains(&t).then_some(t)?
    } else {
        // A copy that knows its page (a hand link, say) keeps fetching by it.
        if remembered.is_some() {
            return None;
        }
        crate::ifdb_store::tuid_for_ifid(cat, ifid)?
    };
    let meta = crate::ifdb_store::read_record(cat, &tuid)?;
    if meta.not_found || (!forced && meta.fetch_version != story_info::FETCH_VERSION) {
        return None;
    }
    Some(meta)
}

/// After a one-story fetch landed IFDB entry `tuid` in the shared store, link
/// every OTHER copy in `targets` whose IFID that entry's record lists and that
/// has no IFDB link yet, exactly as [`Outcome::Linked`] does inside an order
/// (SQ-1787). No request, no delay. A copy already linked (to this entry or any
/// other) is untouched, as is one whose IFID the entry does not list uniquely.
///
/// Returns the targets it linked, so a host can refresh those rows (or undo
/// them). The TUI's `f` and an embedding host's one-story refresh both call
/// this, so the two cannot drift.
pub fn link_listed_copies(roots: &crate::data_roots::DataRoots, targets: &[FetchTarget], tuid: &str) -> Vec<FetchTarget> {
    let cat = roots.catalogue();
    let Some(meta) = crate::ifdb_store::read_record(cat, tuid).filter(|m| !m.not_found) else {
        return Vec::new();
    };
    let mut linked = Vec::new();
    for t in targets {
        let key = t.key.clone().unwrap_or_else(|| crate::storage::story_key_at_from(&t.path, t.disk_entry.as_deref()));
        let game_dir = crate::storage::game_dir(cat, &key);
        if story_info::linked_tuid(&game_dir).is_some()
            || crate::ifdb_store::tuid_for_ifid(cat, &t.ifid).as_deref() != Some(tuid)
        {
            continue;
        }
        write_fetched(roots, &game_dir, &t.ifid, meta.clone());
        linked.push(FetchTarget { key: Some(key), ..t.clone() });
    }
    linked
}

/// [`link_listed_copies`] for a worker result: only a `Fetched` or `Linked`
/// outcome wrote an entry, so `NotFound` and `Failed` (and `Skipped`) link
/// nothing. `done` is the target that outcome was for; its entry is the one
/// its game folder now links to.
pub fn link_listed_copies_after(
    roots: &crate::data_roots::DataRoots,
    targets: &[FetchTarget],
    done: &FetchTarget,
    outcome: &Outcome,
) -> Vec<FetchTarget> {
    if !matches!(outcome, Outcome::Fetched | Outcome::Linked) {
        return Vec::new();
    }
    let key = done.key.clone().unwrap_or_else(|| crate::storage::story_key_at_from(&done.path, done.disk_entry.as_deref()));
    let Some(tuid) = story_info::linked_tuid(&crate::storage::game_dir(roots.catalogue(), &key)) else {
        return Vec::new();
    };
    link_listed_copies(roots, targets, &tuid)
}

/// Load the sidecar (or start a fresh one), set `.fetched`, and save —
/// preserving any existing `.probe` block (SQ-0276's slot).
///
/// `pub(crate)`: also called directly by `ifdb_search.rs` after an IFDB
/// download (SQ-0474), reusing this writer so both paths produce an
/// identical sidecar shape.
///
/// **The one place a TUID is ever written** (fetch, IFDB-search download, metadata
/// import all end here), so it is also where a game's documents folder is created
/// (SQ-1679) — on the caller's worker thread, after the sidecar is safely saved.
/// A relink to another TUID ensures the new folder and leaves the old one alone.
pub(crate) fn write_fetched(roots: &crate::data_roots::DataRoots, game_dir: &Path, ifid: &str, fetched: FetchedMeta) {
    let mut info = story_info::load(game_dir, ifid).unwrap_or_else(|| StoryInfo {
        format_version: story_info::FORMAT_VERSION,
        ifid: ifid.to_string(),
        fetched: None,
        probe: None,
    });
    let tuid = fetched.ifdb_tuid.clone().filter(|t| !t.is_empty());
    let title = fetched.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
        let key = game_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        key.strip_suffix(".save").unwrap_or(key).to_string()
    });
    info.fetched = Some(fetched);
    let _ = story_info::save(game_dir, &info);
    if let (Some(tuid), true) = (tuid, roots.creates_documents()) {
        let _ = crate::documents::ensure_documents_dir(roots, &tuid, &title);
    }
}

/// `pub(crate)`: also called by `ifdb_search.rs` (SQ-0474) — see
/// [`write_fetched`].
pub(crate) fn found_meta(iff: &IFiction, cover: Option<String>) -> FetchedMeta {
    FetchedMeta {
        scanned_at: now_rfc3339(),
        fetch_version: story_info::FETCH_VERSION,
        source: "ifdb".to_string(),
        title: iff.title.clone(),
        author: iff.author.clone(),
        language: iff.language.clone(),
        first_published: iff.first_published.clone(),
        genre: iff.genre.clone(),
        description: iff.description.clone(),
        ifdb_tuid: iff.ifdb.as_ref().map(|e| e.tuid.clone()),
        ifdb_link: iff.ifdb.as_ref().and_then(|e| e.link.clone()),
        ifdb_rating: iff.ifdb.as_ref().and_then(|e| e.average_rating),
        ifdb_rating_count: iff.ifdb.as_ref().and_then(|e| e.rating_count),
        cover,
        not_found: false,
    }
}

fn not_found_meta() -> FetchedMeta {
    FetchedMeta {
        scanned_at: now_rfc3339(),
        fetch_version: story_info::FETCH_VERSION,
        source: "ifdb".to_string(),
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
        not_found: true,
    }
}

/// On a `Found` result with a cover URL and no cover already local to the
/// story (a blorb's own `Fspc` frontispiece), fetch it and cache it in the
/// shared IFDB store under the record's tuid, in the bytes IFDB served
/// (SQ-1723; a record with no tuid keeps `cover.png` beside the sidecar).
/// Best-effort: any failure (no URL, a decode-free
/// local cover already present, a transport error, a write error) simply
/// leaves the field `None` — it never turns a successful metadata fetch into
/// a failed one.
///
/// `pub(crate)`: also called by `ifdb_search.rs` after an IFDB download
/// (SQ-0474), reusing this exact one-request-max logic rather than
/// duplicating it.
pub(crate) fn maybe_fetch_cover(
    source: &dyn MetadataSource,
    game_dir: &Path,
    path: &Path,
    iff: &IFiction,
) -> Option<String> {
    let url = iff.ifdb.as_ref()?.cover_url.clone()?;
    if crate::cover::load_cover(path, None).is_some() {
        return None; // the story already carries its own frontispiece
    }
    let bytes = source.fetch_cover(&url).ok()?;
    // SQ-0660: only bytes that actually decode as an image are recorded — an
    // HTML error page written as cover.png would be pinned by the sidecar's
    // FETCH_VERSION (no refetch ever), permanently showing no cover. Written
    // temp-then-rename in the same dir so a crash mid-write can't leave a
    // truncated cover.png either.
    crate::cover::decode(&bytes)?;
    // SQ-1723: kept once per IFDB entry, in the bytes IFDB served (so it may
    // be a JPEG), shared by every copy; a copy with no tuid keeps its own.
    let tuid = iff.ifdb.as_ref().map(|e| e.tuid.as_str());
    crate::ifdb_store::write_fetched_cover(game_dir, tuid, &bytes).ok()
}

fn stem_title(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

pub(crate) fn now_rfc3339() -> String {
    jiff::Timestamp::now().to_string()
}

#[cfg(all(test, feature = "t-picker"))]
mod tests {
    use super::*;
    use crate::story_info::ProbeMeta;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    /// Wrap Scott `.dat` bytes in a minimal Blorb with a single `Exec`/`SAAI`
    /// resource — the shape a graphics `.blb` uses.
    fn blb_wrapping_saai(dat: &[u8]) -> Vec<u8> {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = ty.to_vec();
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12; // count + one 12-byte entry
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // resource count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // resource number
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"SAAI", dat));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// A unique temp dir per call, safe under parallel test execution (mirrors
    /// `story_info`'s own `tmp()` helper).
    fn tmp() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("bm_fetch_worker_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scott_ifdb_id_maps_a_known_adventure_and_ignores_non_scott() {
        // A minimal Scott `.dat`; identity comes from the filename stem
        // (`adv01` -> Adventureland), not the trailer number.
        const MINI: &str = "\n32767 1 0 1 2 6 1 0 3 125 0 1\n150 1 0 0 0 0 0 0\n\
             \"\" \"\"\n\"GO\" \"NORTH\"\n0 0 0 0 0 0 \"*limbo\"\n2 0 0 0 0 0 \"*forest clearing\"\n\
             0 0 0 0 0 0 \"*swamp\"\n\"\"\n\"\" 0\n\"*a brass lamp/LAMP/\" 1\n\"action comment\"\n0\n1\n1\n";
        let dir = tmp();
        let dat = dir.join("adv01.dat");
        std::fs::write(&dat, MINI).unwrap();
        assert_eq!(super::scott_ifdb_id(&dat, None).as_deref(), Some("dy4ok8sdlut6ddj7"));

        // The same Scott `.dat` wrapped in a `.blb` blorb (SAAI exec chunk) is
        // detected too, keyed by its filename stem (`circus`).
        let blb = dir.join("circus.blb");
        std::fs::write(&blb, blb_wrapping_saai(MINI.as_bytes())).unwrap();
        assert_eq!(super::scott_ifdb_id(&blb, None).as_deref(), Some("bdnprzz9zomlge4b"));

        // A binary (non-Scott) story yields no Scott id.
        let z = dir.join("story.z5");
        std::fs::write(&z, [3u8, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(super::scott_ifdb_id(&z, None), None);
    }

    /// SQ-1768: the folder a fetch writes into is the folder the picker row
    /// reads. For a `.zblorb` that means keying on the Z-code inside it, which
    /// is what `StoryEntry::story_key` does; the file-name fallback sent the
    /// details somewhere the row never looked. (A `.gblorb` has no release or
    /// serial, so both sides already agree on its file name.)
    #[test]
    fn a_zblorbs_fetch_key_is_the_rows_key() {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = ty.to_vec();
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        // A structurally valid Version 3 header for Zork I r88 / 840726.
        let mut z = vec![0u8; 0x100];
        z[0] = 3;
        z[0x02..0x04].copy_from_slice(&88u16.to_be_bytes());
        for (off, v) in [(0x04, 0x80u16), (0x08, 0xc0), (0x0a, 0x40), (0x0c, 0x44), (0x0e, 0x80)] {
            z[off..off + 2].copy_from_slice(&v.to_be_bytes());
        }
        z[0x12..0x18].copy_from_slice(b"840726");
        let off = 12 + 8 + 4 + 12;
        let mut ridx = 1u32.to_be_bytes().to_vec();
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes());
        ridx.extend_from_slice(&(off as u32).to_be_bytes());
        let mut inner = b"IFRS".to_vec();
        inner.extend_from_slice(&chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&chunk(b"ZCOD", &z));
        let mut file = b"FORM".to_vec();
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);

        let dir = crate::scratch_dir("zblorb-fetch-key");
        let path = dir.join("zork-with-sound.zblorb");
        std::fs::write(&path, file).unwrap();
        let roots = crate::data_roots::DataRoots::single(dir.join("data"));
        let row = crate::picker::resolve_entry(&path, &roots).expect("a launchable row");
        assert_eq!(crate::storage::story_key_at_from(&path, None), row.story_key());
        assert_eq!(row.story_key(), "zork-i-r88-s840726");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[derive(Clone)]
    enum FakeResp {
        Found(Box<IFiction>),
        NotFound,
        Err(String),
    }

    /// A fake `MetadataSource`: canned responses keyed by IFID, plus a call
    /// log that IS the assertion for the skip tests (never touching a real
    /// network).
    struct Fake {
        responses: HashMap<String, FakeResp>,
        calls: Arc<Mutex<Vec<String>>>,
        cover_calls: Arc<Mutex<Vec<String>>>,
        cover_bytes: Vec<u8>,
        /// Set only by the cancel test: every fetch announces itself on
        /// `started` and then parks until the test sends on `release`, so
        /// "cancel arrived mid-fetch" is a fact the test arranges rather than
        /// a race it hopes to win. A 20ms-sleep-then-cancel against a 150ms
        /// fetch lost that race on a loaded macOS runner and fetched story 2.
        fetch_gate: Option<FetchGate>,
    }

    struct FetchGate {
        started: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl Fake {
        fn new(responses: HashMap<String, FakeResp>) -> Self {
            Self {
                responses,
                calls: Arc::new(Mutex::new(Vec::new())),
                cover_calls: Arc::new(Mutex::new(Vec::new())),
                // A real decodable PNG: `maybe_fetch_cover` validates the
                // bytes decode before writing (SQ-0660).
                cover_bytes: real_png_bytes(),
                fetch_gate: None,
            }
        }
    }

    impl MetadataSource for Fake {
        fn fetch(&self, ifid: &str) -> Result<FetchOutcome, FetchError> {
            self.calls.lock().unwrap().push(ifid.to_string());
            if let Some(gate) = &self.fetch_gate {
                let _ = gate.started.send(());
                let _ = gate.release.lock().unwrap().recv();
            }
            match self.responses.get(ifid) {
                Some(FakeResp::Found(f)) => Ok(FetchOutcome::Found(f.clone())),
                Some(FakeResp::NotFound) => Ok(FetchOutcome::NotFound),
                Some(FakeResp::Err(m)) => Err(FetchError::Transport(m.clone())),
                None => Ok(FetchOutcome::NotFound),
            }
        }

        fn fetch_by_id(&self, tuid: &str) -> Result<FetchOutcome, FetchError> {
            // Keyed the same as fetch(): tests register a response under the tuid.
            self.calls.lock().unwrap().push(format!("id:{tuid}"));
            match self.responses.get(tuid) {
                Some(FakeResp::Found(f)) => Ok(FetchOutcome::Found(f.clone())),
                Some(FakeResp::NotFound) => Ok(FetchOutcome::NotFound),
                Some(FakeResp::Err(m)) => Err(FetchError::Transport(m.clone())),
                None => Ok(FetchOutcome::NotFound),
            }
        }

        fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            self.cover_calls.lock().unwrap().push(url.to_string());
            Ok(self.cover_bytes.clone())
        }
    }

    /// Bounded poll (mirrors `cover.rs`'s pattern): collect drained progress
    /// until at least `n` have arrived, or give up after ~2s.
    fn wait_for(fetcher: &Fetcher, n: usize) -> Vec<FetchProgress> {
        let mut got = Vec::new();
        for _ in 0..2000 {
            got.extend(fetcher.drain());
            if got.len() >= n {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        got
    }

    fn stub_info(ifid: &str, fetched: Option<FetchedMeta>) -> StoryInfo {
        StoryInfo { format_version: story_info::FORMAT_VERSION, ifid: ifid.into(), fetched, probe: None }
    }

    fn up_to_date_meta() -> FetchedMeta {
        FetchedMeta {
            scanned_at: "2026-07-16T00:00:00Z".into(),
            fetch_version: story_info::FETCH_VERSION,
            source: "ifdb".into(),
            title: Some("Old Title".into()),
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

    /// A Scott game inside a zip is keyed by the entry's stem, not the zip's.
    #[test]
    fn scott_ifdb_id_inside_a_zip_uses_the_entry_name() {
        const MINI: &str = "\n32767 1 0 1 2 6 1 0 3 125 0 1\n150 1 0 0 0 0 0 0\n\
\"AUTO 0\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\
0 0 0 0 0 0 0\n\"*you are in a room\"\n\"\"\n\"*\"\n\"\"\n0\n0\n0\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n\"\"\n0 0\n0 0\n0 0\n0 0\n0 0\n0 0\n0 0\n0 0\n0 0\n0 0\n\
0\n0\n0\n0\n1\n0\n0\n";
        let dir = crate::scratch_dir("scott-zip-tuid");
        let zip_path = dir.join("AdamsGames.zip");
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["adv01.dat", "notes.txt"] {
                zw.start_file(name, opts).unwrap();
                std::io::Write::write_all(&mut zw, if name.ends_with(".dat") { MINI.as_bytes() } else { b"readme" }).unwrap();
            }
            zw.finish().unwrap();
        }
        assert_eq!(scott_ifdb_id(&zip_path, Some("adv01.dat")).as_deref(), Some("dy4ok8sdlut6ddj7"), "Adventureland, by its entry");
        assert_eq!(scott_ifdb_id(&zip_path, None), None, "the archive's own stem names nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn r_order_skips_a_story_already_at_fetch_version_without_calling_the_source() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-000001".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        story_info::save(&game_dir, &stub_info(&ifid, Some(up_to_date_meta()))).unwrap();

        let fake = Fake::new(HashMap::new());
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid }], forced: false , id_override: None});

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].outcome, Outcome::Skipped);
        assert!(calls.lock().unwrap().is_empty(), "the fake must never be called for an up-to-date story");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn forced_order_calls_the_source_even_for_a_current_found_story() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-000002".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        story_info::save(&game_dir, &stub_info(&ifid, Some(up_to_date_meta()))).unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            ifid.clone(),
            FakeResp::Found(Box::new(IFiction { title: Some("Refreshed".into()), ..Default::default() })),
        );
        let fake = Fake::new(responses);
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: true , id_override: None});

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].outcome, Outcome::Fetched);
        assert_eq!(progress[0].title, "Refreshed");
        assert_eq!(*calls.lock().unwrap(), vec![ifid.clone()], "forced must call the source");

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert_eq!(reloaded.fetched.unwrap().title.as_deref(), Some("Refreshed"));

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn not_found_writes_a_fetched_meta_with_not_found_true() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-000003".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));

        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::NotFound);
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: false , id_override: None});

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress[0].outcome, Outcome::NotFound);

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        let fetched = reloaded.fetched.expect("a not-found answer IS a completed fetch");
        assert!(fetched.not_found);

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn transport_error_writes_no_sidecar_block_so_a_later_sweep_retries() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-000004".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        // Pre-existing sidecar with only a probe block (never fetched) — must
        // survive byte-for-byte since a transport error writes nothing.
        let before = stub_info(&ifid, None);
        let mut before = before;
        before.probe = Some(ProbeMeta { probed_at: Some("earlier".into()) });
        story_info::save(&game_dir, &before).unwrap();

        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Err("connection reset".into()));
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: false , id_override: None});

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress[0].outcome, Outcome::Failed("connection reset".into()));

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert!(reloaded.fetched.is_none(), "a transport error must not write a fetched block");
        assert_eq!(reloaded.probe, before.probe, "the untouched probe block must survive");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn id_override_fetches_by_tuid_and_writes_the_sidecar_under_the_story_ifid() {
        // SQ-0371: a manual IFDB-page fetch looks the game up by the tuid the
        // user supplied, not the story's own IFID (which IFDB didn't index), but
        // still caches under the story's IFID so the identity check holds.
        let data_base = tmp();
        let path = data_base.join("obscure.z5");
        let ifid = "ZCODE-99-999999".to_string(); // not in IFDB
        let tuid = "0dbnusxunq7fw5ro".to_string();
        let mut responses = HashMap::new();
        responses.insert(
            tuid.clone(),
            FakeResp::Found(Box::new(IFiction { title: Some("Found By Hand".into()), ..Default::default() })),
        );
        let fake = Fake::new(responses);
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }],
            forced: false,
            id_override: Some(tuid.clone()),
        });

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress[0].outcome, Outcome::Fetched);
        // Fetched by id, not by ifid.
        assert_eq!(calls.lock().unwrap().as_slice(), &[format!("id:{tuid}")]);

        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&progress[0].path));
        let reloaded = story_info::load(&game_dir, &ifid).expect("sidecar keyed to the story IFID");
        assert_eq!(reloaded.fetched.unwrap().title.as_deref(), Some("Found By Hand"));
        let _ = std::fs::remove_dir_all(&data_base);
    }

    fn found_by(tuid: &str, title: &str) -> FakeResp {
        FakeResp::Found(Box::new(IFiction {
            title: Some(title.into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: tuid.into(),
                link: None,
                cover_url: None,
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        }))
    }

    fn fetch_by_id(fetcher: &Fetcher, path: &Path, ifid: &str, tuid: &str) {
        fetcher.request(FetchOrder {
            stories: vec![FetchTarget { path: path.to_path_buf(), disk_entry: None, key: None, ifid: ifid.into() }],
            forced: true,
            id_override: Some(tuid.into()),
        });
        let progress = wait_for(fetcher, 1);
        assert_eq!(progress[0].outcome, Outcome::Fetched);
    }

    /// SQ-1679: a fetch (or relink) that delivers a TUID creates that game's
    /// documents folder when the user opted in, never touches the previous TUID's
    /// folder on a relink, and creates nothing by default.
    #[test]
    fn a_fetch_that_writes_a_tuid_creates_the_documents_folder_only_when_opted_in() {
        let ifid = "ZCODE-1-000001";
        let mut responses = HashMap::new();
        responses.insert("tuidaaa".to_string(), found_by("tuidaaa", "Zork: The Game?"));
        responses.insert("tuidbbb".to_string(), found_by("tuidbbb", "Other Game"));

        // Opted in.
        let home = tmp();
        let on = crate::data_roots::DataRoots::resolve(
            &home, None, None, &crate::data_roots::DocumentsSettings { dir: None, auto_create: true },
        );
        let path = home.join("game.z5");
        let fetcher = Fetcher::new(Box::new(Fake::new(responses.clone())), on.clone(), Duration::ZERO);
        fetch_by_id(&fetcher, &path, ifid, "tuidaaa");
        let first = on.documents().join("Zork_ The Game_ [tuidaaa]");
        assert!(first.is_dir(), "{:?}", std::fs::read_dir(on.documents()).map(|r| r.flatten().map(|e| e.path()).collect::<Vec<_>>()));
        std::fs::write(first.join("manual.pdf"), b"mine").unwrap();
        // A relink to a different TUID: the new folder appears, the old one is untouched.
        fetch_by_id(&fetcher, &path, ifid, "tuidbbb");
        assert!(on.documents().join("Other Game [tuidbbb]").is_dir());
        assert_eq!(std::fs::read(first.join("manual.pdf")).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(&home);

        // Default config: nothing is created.
        let home = tmp();
        let off = crate::data_roots::DataRoots::resolve(&home, None, None, &Default::default());
        let fetcher = Fetcher::new(Box::new(Fake::new(responses)), off.clone(), Duration::ZERO);
        fetch_by_id(&fetcher, &home.join("game.z5"), ifid, "tuidaaa");
        assert!(!off.documents().exists(), "opt-in: a default config creates no folder");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// SQ-1226: a story only HAS a remembered tuid because something found it
    /// on IFDB by page id — an `f` that went back to the IFID would get the
    /// authoritative NotFound that made the page id necessary in the first
    /// place, and `write_fetched` would replace the record with it. *City of
    /// Secrets* is the reported case: an Inform 6 Glulx game whose Treaty IFID
    /// (its file's MD5) is not the `GLULX-…` hash we compute, so IFDB indexes
    /// nothing we can ask it for.
    #[test]
    fn a_remembered_ifdb_tuid_is_reused_so_a_forced_refetch_cannot_lose_the_metadata() {
        let data_base = tmp();
        let path = data_base.join("CoS.gblorb");
        let ifid = "GLULX-0123456789ABCDEF".to_string();
        let tuid = "0dbnusxunq7fw5ro".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));

        // The sidecar an IFDB-search download leaves behind: found, current,
        // and carrying the page id it was found under.
        let mut meta = up_to_date_meta();
        meta.title = Some("City of Secrets".into());
        meta.ifdb_tuid = Some(tuid.clone());
        story_info::save(&game_dir, &stub_info(&ifid, Some(meta))).unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            tuid.clone(),
            FakeResp::Found(Box::new(IFiction {
                title: Some("City of Secrets".into()),
                ifdb: Some(crate::ifiction::IfdbExt {
                    tuid: tuid.clone(),
                    link: None,
                    cover_url: None,
                    average_rating: None,
                    rating_count: None,
                }),
                ..Default::default()
            })),
        );
        responses.insert(ifid.clone(), FakeResp::NotFound); // IFDB indexes no such IFID
        let fake = Fake::new(responses);
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }],
            forced: true,
            id_override: None,
        });

        let progress = wait_for(&fetcher, 1);
        assert_eq!(
            progress[0].outcome,
            Outcome::Fetched,
            "not the NotFound a by-IFID lookup answers with"
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[format!("id:{tuid}")],
            "asked by the remembered page id, never by the IFID"
        );

        let reloaded = story_info::load(&game_dir, &ifid).and_then(|i| i.fetched).expect("sidecar");
        assert!(!reloaded.not_found, "the found record survives a forced refetch");
        assert_eq!(reloaded.title.as_deref(), Some("City of Secrets"));
        assert_eq!(reloaded.ifdb_tuid.as_deref(), Some(tuid.as_str()), "and so does the page id");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn cancel_mid_order_stops_before_the_next_story_and_keeps_prior_writes() {
        let data_base = tmp();
        let path1 = data_base.join("game1.z5");
        let path2 = data_base.join("game2.z5");
        let path3 = data_base.join("game3.z5");
        let ifid1 = "ZCODE-1-100001".to_string();
        let ifid2 = "ZCODE-1-100002".to_string();
        let ifid3 = "ZCODE-1-100003".to_string();
        let dir1 = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path1));
        let dir2 = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path2));

        let mut responses = HashMap::new();
        for ifid in [&ifid1, &ifid2, &ifid3] {
            responses.insert(
                ifid.clone(),
                FakeResp::Found(Box::new(IFiction { title: Some(format!("Story {ifid}")), ..Default::default() })),
            );
        }
        let mut fake = Fake::new(responses);
        // The worker parks inside story 1's fetch until we say so, so cancel()
        // is guaranteed to land before story 1 returns and story 2 is weighed.
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        fake.fetch_gate = Some(FetchGate { started: started_tx, release: Mutex::new(release_rx) });
        let calls = Arc::clone(&fake.calls);

        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![
                FetchTarget { path: path1, disk_entry: None, key: None, ifid: ifid1.clone() },
                FetchTarget { path: path2, disk_entry: None, key: None, ifid: ifid2.clone() },
                FetchTarget { path: path3, disk_entry: None, key: None, ifid: ifid3.clone() },
            ],
            forced: true,
            id_override: None,
        });

        started_rx.recv_timeout(Duration::from_secs(5)).expect("story 1's fetch never started");
        fetcher.cancel();
        release_tx.send(()).unwrap();

        // Bounded wait for the worker to settle (story 1 finishes, sees
        // cancel, and stops).
        for _ in 0..2000 {
            if !fetcher.busy() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(*calls.lock().unwrap(), vec![ifid1.clone()], "only story 1 should ever be fetched");
        assert!(
            story_info::load(&dir1, &ifid1).and_then(|i| i.fetched).is_some(),
            "story 1's write, made before the cancel, must survive"
        );
        assert!(story_info::load(&dir2, &ifid2).is_none(), "story 2 must never have been touched");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn progress_done_total_count_every_story_including_skips() {
        let data_base = tmp();
        let target = |name: &str, ifid: &str| FetchTarget {
            path: data_base.join(name),
            disk_entry: None, key: None,
            ifid: ifid.to_string(),
        };
        let skip1 = target("skip1.z5", "ZCODE-1-200001");
        let skip2 = target("skip2.z5", "ZCODE-1-200002");
        let fetched = target("fetch3.z5", "ZCODE-1-200003");
        for t in [&skip1, &skip2] {
            let dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&t.path));
            story_info::save(&dir, &stub_info(&t.ifid, Some(up_to_date_meta()))).unwrap();
        }

        let mut responses = HashMap::new();
        responses.insert(fetched.ifid.clone(), FakeResp::NotFound);
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![skip1.clone(), skip2.clone(), fetched.clone()],
            forced: false,
            id_override: None,
        });

        let progress = wait_for(&fetcher, 3);
        assert_eq!(progress.len(), 3);
        assert_eq!(
            progress.iter().map(|p| (p.done, p.total)).collect::<Vec<_>>(),
            vec![(1, 3), (2, 3), (3, 3)],
            "done/total must count every story, skips included"
        );
        assert_eq!(progress[0].outcome, Outcome::Skipped);
        assert_eq!(progress[1].outcome, Outcome::Skipped);
        assert_eq!(progress[2].outcome, Outcome::NotFound);

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn a_fetch_preserves_an_existing_probe_block() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-300001".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));
        let mut seed = stub_info(&ifid, None);
        seed.probe = Some(ProbeMeta { probed_at: Some("2026-01-01".into()) });
        story_info::save(&game_dir, &seed).unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            ifid.clone(),
            FakeResp::Found(Box::new(IFiction { title: Some("Title".into()), ..Default::default() })),
        );
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: true , id_override: None});
        wait_for(&fetcher, 1);

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert!(reloaded.fetched.is_some(), "the fetch must have written a fetched block");
        assert_eq!(reloaded.probe, seed.probe, "an existing probe block must not be clobbered");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn found_with_a_cover_url_and_no_local_frontispiece_writes_cover_png() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        // Not a blorb at all, so `cover::load_cover` returns None: no local
        // frontispiece to defer to.
        std::fs::write(&path, b"not a blorb").unwrap();
        let ifid = "ZCODE-1-400001".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));

        let iff = IFiction {
            title: Some("Covered".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "abc123".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=abc123&version=1".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        };
        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Found(Box::new(iff)));
        let fake = Fake::new(responses);
        let cover_calls = Arc::clone(&fake.cover_calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: true , id_override: None});
        wait_for(&fetcher, 1);

        assert_eq!(cover_calls.lock().unwrap().len(), 1, "the cover must be fetched exactly once");
        let cover_bytes = std::fs::read(data_base.join("ifdb").join("abc123.png"))
            .expect("the cover must be written to the shared store");
        assert_eq!(cover_bytes, real_png_bytes());
        // The temp-then-rename write leaves no .part file behind.
        assert!(
            std::fs::read_dir(&game_dir).unwrap().all(|e| {
                !e.unwrap().file_name().to_string_lossy().contains(".part")
            }),
            "no temp leftovers in the game dir"
        );

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert_eq!(reloaded.fetched.unwrap().cover.as_deref(), Some("abc123.png"));
        assert!(!game_dir.join("cover.png").exists(), "no per-copy cover: it is shared");

        let _ = std::fs::remove_dir_all(&data_base);
    }

    /// SQ-1723: two copies of one game (different story keys, so different
    /// directories) fetched through the worker share ONE record and ONE cover,
    /// kept in the bytes IFDB served (a JPEG here, so `.jpg`, not `.png`).
    #[test]
    fn two_copies_of_one_game_fetch_into_one_shared_record_and_cover() {
        let cat = crate::scratch_dir("fetch-shared");
        let (pa, pb) = (cat.join("a.z5"), cat.join("b.z5"));
        std::fs::write(&pa, b"not a blorb").unwrap();
        std::fs::write(&pb, b"not a blorb").unwrap();
        let iff = |title: &str| IFiction {
            title: Some(title.into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "shared1".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=shared1".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        };
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([5, 6, 7])))
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let roots = crate::data_roots::DataRoots::single(&cat);
        let run = |path: &PathBuf, ifid: &str, title: &str| {
            let mut responses = HashMap::new();
            responses.insert(ifid.to_string(), FakeResp::Found(Box::new(iff(title))));
            // A copy that already knows its page asks by id (SQ-1226).
            responses.insert("shared1".to_string(), FakeResp::Found(Box::new(iff(title))));
            let mut fake = Fake::new(responses);
            fake.cover_bytes = jpeg.clone();
            let fetcher = Fetcher::new(Box::new(fake), roots.clone(), Duration::ZERO);
            fetcher.request(FetchOrder {
                stories: vec![FetchTarget { path: path.clone(), disk_entry: None, key: None, ifid: ifid.into() }],
                forced: true,
                id_override: None,
            });
            wait_for(&fetcher, 1);
        };
        run(&pa, "IFID-A", "First Title");
        run(&pb, "IFID-B", "First Title");

        let dir = |p: &PathBuf| crate::storage::game_dir(&cat, &crate::storage::story_key_at(p));
        assert_eq!(std::fs::read(cat.join("ifdb/shared1.jpg")).unwrap(), jpeg, "original bytes, real extension");
        assert!(!cat.join("ifdb/shared1.png").exists());
        assert!(!dir(&pa).join("cover.png").exists() && !dir(&pb).join("cover.png").exists());

        // A refresh through A is what B sees, without B being fetched again.
        run(&pa, "IFID-A", "Second Title");
        let seen = |p: &PathBuf, ifid: &str| story_info::load(&dir(p), ifid).unwrap().fetched.unwrap();
        assert_eq!(seen(&pb, "IFID-B").title.as_deref(), Some("Second Title"));
        assert_eq!(seen(&pb, "IFID-B").cover.as_deref(), Some("shared1.jpg"));
        let _ = std::fs::remove_dir_all(&cat);
    }

    /// SQ-0660: cover bytes that don't decode as an image (an HTML error page
    /// served with 200) must be written NOWHERE, and the sidecar — which pins
    /// this answer at FETCH_VERSION — must not record a cover.
    #[test]
    fn cover_bytes_that_do_not_decode_are_never_written_or_recorded() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        std::fs::write(&path, b"not a blorb").unwrap();
        let ifid = "ZCODE-1-400003".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));

        let iff = IFiction {
            title: Some("Broken Cover".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "brk".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=brk&version=1".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        };
        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Found(Box::new(iff)));
        let mut fake = Fake::new(responses);
        fake.cover_bytes = b"<html><body>503 Service Unavailable</body></html>".to_vec();
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: true, id_override: None });
        wait_for(&fetcher, 1);

        assert!(!game_dir.join("cover.png").exists(), "junk must never become cover.png");
        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert!(
            reloaded.fetched.unwrap().cover.is_none(),
            "the sidecar must not record a cover it doesn't have"
        );
        if game_dir.exists() {
            assert!(
                std::fs::read_dir(&game_dir).unwrap().all(|e| {
                    !e.unwrap().file_name().to_string_lossy().contains(".part")
                }),
                "no temp leftovers"
            );
        }

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn found_with_an_existing_local_frontispiece_skips_the_cover_fetch() {
        let data_base = tmp();
        let path = data_base.join("game.gblorb");
        std::fs::write(&path, blorb_with_fspc_and_cover()).unwrap();
        let ifid = "ZCODE-1-400002".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key_at(&path));

        let iff = IFiction {
            title: Some("Already Covered".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "xyz".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=xyz&version=1".into()),
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        };
        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Found(Box::new(iff)));
        let fake = Fake::new(responses);
        let cover_calls = Arc::clone(&fake.cover_calls);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&data_base), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![FetchTarget { path, disk_entry: None, key: None, ifid: ifid.clone() }], forced: true , id_override: None});
        wait_for(&fetcher, 1);

        assert!(cover_calls.lock().unwrap().is_empty(), "a story with its own frontispiece needs no fetch");
        assert!(!game_dir.join("cover.png").exists());
        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert!(reloaded.fetched.unwrap().cover.is_none());

        let _ = std::fs::remove_dir_all(&data_base);
    }

    // ── blorb-with-Fspc test fixture (mirrors `blorb::tests::build_blorb_with_top`
    // and `frontispiece_reads_fspc_number`, which are crate-private to `blorb`) ──

    fn iff_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A real (decodable) tiny PNG, so `cover::load_cover`'s decode step
    /// succeeds and reports a local cover.
    fn real_png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn blorb_with_fspc_and_cover() -> Vec<u8> {
        let png = real_png_bytes();
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Pict");
        ridx.extend_from_slice(&7u32.to_be_bytes()); // number
        let ridx_chunk_len = 8 + (4 + 12);
        let fspc_chunk_len = 8 + 4;
        let pict_off = 12 + ridx_chunk_len + fspc_chunk_len;
        ridx.extend_from_slice(&(pict_off as u32).to_be_bytes()); // start
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff_chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&iff_chunk(b"Fspc", &7u32.to_be_bytes()));
        inner.extend_from_slice(&iff_chunk(b"PNG ", &png));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    // ---- SQ-1786: IFID map -------------------------------------------------

    fn entry_record(tuid: &str, title: &str, ifids: &[&str]) -> FakeResp {
        FakeResp::Found(Box::new(IFiction {
            title: Some(title.into()),
            ifids: ifids.iter().map(|s| s.to_string()).collect(),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: tuid.into(),
                link: None,
                cover_url: None,
                average_rating: None,
                rating_count: None,
            }),
            ..Default::default()
        }))
    }

    fn target(dir: &Path, name: &str, ifid: &str) -> FetchTarget {
        FetchTarget { path: dir.join(name), disk_entry: None, ifid: ifid.into(), key: None }
    }

    /// Run one order to completion against a fake and return (progress, calls).
    fn run(
        dir: &Path,
        responses: HashMap<String, FakeResp>,
        stories: Vec<FetchTarget>,
        forced: bool,
    ) -> (Vec<FetchProgress>, Vec<String>) {
        let fake = Fake::new(responses);
        let calls = Arc::clone(&fake.calls);
        let n = stories.len();
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(dir), Duration::ZERO);
        fetcher.request(FetchOrder { stories, forced, id_override: None });
        let progress = wait_for(&fetcher, n);
        assert_eq!(progress.len(), n);
        let calls = calls.lock().unwrap().clone();
        (progress, calls)
    }

    fn dir_of(dir: &Path, name: &str) -> PathBuf {
        crate::storage::game_dir(dir, &crate::storage::story_key_at(&dir.join(name)))
    }

    #[test]
    fn two_copies_listed_in_one_record_cost_one_request() {
        let dir = crate::scratch_dir("ifid-map-two");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        let stories = vec![target(&dir, "game-r1.z5", "ZCODE-1-A"), target(&dir, "game-r2.z5", "ZCODE-2-B")];
        let (progress, calls) = run(&dir, r, stories, false);
        assert_eq!(calls, vec!["ZCODE-1-A".to_string()], "one request for the whole entry");
        assert_eq!(progress[0].outcome, Outcome::Fetched);
        assert_eq!(progress[1].outcome, Outcome::Linked);
        for (name, ifid) in [("game-r1.z5", "ZCODE-1-A"), ("game-r2.z5", "ZCODE-2-B")] {
            let info = story_info::load(&dir_of(&dir, name), ifid).expect("linked");
            assert_eq!(info.fetched.unwrap().ifdb_tuid.as_deref(), Some("tuidaaaa"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_ifid_the_record_does_not_list_gets_its_own_request() {
        let dir = crate::scratch_dir("ifid-map-unlisted");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A"]));
        r.insert("ZCODE-9-Z".to_string(), entry_record("tuidzzzz", "Other", &["ZCODE-9-Z"]));
        let stories = vec![target(&dir, "a.z5", "ZCODE-1-A"), target(&dir, "z.z5", "ZCODE-9-Z")];
        let (progress, calls) = run(&dir, r, stories, false);
        assert_eq!(calls.len(), 2);
        assert!(progress.iter().all(|p| p.outcome == Outcome::Fetched));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- SQ-1787: one-story fetch links the other listed copies ------------

    fn linked_tuid_of(dir: &Path, name: &str) -> Option<String> {
        story_info::linked_tuid(&dir_of(dir, name))
    }

    #[test]
    fn a_one_story_fetch_links_the_other_listed_copy_without_a_request() {
        let dir = crate::scratch_dir("one-story-links");
        let roots = crate::data_roots::DataRoots::single(&dir);
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        let (a, b) = (target(&dir, "game-r1.z5", "ZCODE-1-A"), target(&dir, "game-r2.z5", "ZCODE-2-B"));
        let (progress, calls) = run(&dir, r, vec![a.clone()], true);
        assert_eq!(calls.len(), 1);
        assert_eq!(linked_tuid_of(&dir, "game-r2.z5"), None, "unlinked before");

        let all = vec![a.clone(), b.clone()];
        let linked = link_listed_copies_after(&roots, &all, &a, &progress[0].outcome);
        assert_eq!(linked.iter().map(|t| t.ifid.as_str()).collect::<Vec<_>>(), vec!["ZCODE-2-B"]);
        assert_eq!(linked_tuid_of(&dir, "game-r2.z5").as_deref(), Some("tuidaaaa"));
        assert_eq!(calls.len(), 1, "no extra request");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_already_linked_elsewhere_is_left_alone() {
        let dir = crate::scratch_dir("one-story-keeps");
        let roots = crate::data_roots::DataRoots::single(&dir);
        let mut r = HashMap::new();
        r.insert("ZCODE-2-B".to_string(), entry_record("tuidbbbb", "Elsewhere", &["ZCODE-2-B"]));
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        let (a, b) = (target(&dir, "game-r1.z5", "ZCODE-1-A"), target(&dir, "game-r2.z5", "ZCODE-2-B"));
        // B is fetched first and linked to its own entry.
        let _ = run(&dir, r.clone(), vec![b.clone()], true);
        assert_eq!(linked_tuid_of(&dir, "game-r2.z5").as_deref(), Some("tuidbbbb"));
        let (progress, _) = run(&dir, r, vec![a.clone()], true);
        let linked = link_listed_copies_after(&roots, &[a.clone(), b], &a, &progress[0].outcome);
        assert!(linked.is_empty());
        assert_eq!(linked_tuid_of(&dir, "game-r2.z5").as_deref(), Some("tuidbbbb"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_found_and_failed_link_nothing() {
        let dir = crate::scratch_dir("one-story-nothing");
        let roots = crate::data_roots::DataRoots::single(&dir);
        let (a, b) = (target(&dir, "game-r1.z5", "ZCODE-1-A"), target(&dir, "game-r2.z5", "ZCODE-2-B"));
        let all = vec![a.clone(), b];
        for outcome in [Outcome::NotFound, Outcome::Failed("down".into()), Outcome::Skipped] {
            assert!(link_listed_copies_after(&roots, &all, &a, &outcome).is_empty());
        }
        assert_eq!(linked_tuid_of(&dir, "game-r2.z5"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_ifid_two_entries_list_is_never_linked_from_the_map() {
        let dir = crate::scratch_dir("ifid-map-dup");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "One", &["ZCODE-1-A", "ZCODE-5-X"]));
        r.insert("ZCODE-2-B".to_string(), entry_record("tuidbbbb", "Dup", &["ZCODE-2-B", "ZCODE-5-X"]));
        r.insert("ZCODE-5-X".to_string(), entry_record("tuidaaaa", "One", &["ZCODE-1-A", "ZCODE-5-X"]));
        let stories = vec![
            target(&dir, "a.z5", "ZCODE-1-A"),
            target(&dir, "b.z5", "ZCODE-2-B"),
            target(&dir, "x.z5", "ZCODE-5-X"),
        ];
        let (progress, calls) = run(&dir, r, stories, false);
        assert_eq!(calls, vec!["ZCODE-1-A", "ZCODE-2-B", "ZCODE-5-X"], "the shared IFID asks for itself");
        assert_eq!(progress[2].outcome, Outcome::Fetched);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_forced_run_asks_once_per_entry_not_once_per_copy() {
        let dir = crate::scratch_dir("ifid-map-forced");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        // A refresh asks by the page id the copies already remember.
        r.insert("tuidaaaa".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        let stories = || vec![target(&dir, "a.z5", "ZCODE-1-A"), target(&dir, "b.z5", "ZCODE-2-B")];
        let (_, first) = run(&dir, r.clone(), stories(), false);
        assert_eq!(first.len(), 1);
        let (progress, calls) = run(&dir, r, stories(), true);
        assert_eq!(calls, vec!["id:tuidaaaa".to_string()], "refresh asks once for the entry");
        assert_eq!(progress[0].outcome, Outcome::Fetched);
        assert_eq!(progress[1].outcome, Outcome::Linked);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unforced_run_over_an_identified_library_makes_no_request() {
        let dir = crate::scratch_dir("ifid-map-quiet");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        let stories = || vec![target(&dir, "a.z5", "ZCODE-1-A"), target(&dir, "b.z5", "ZCODE-2-B")];
        let (_, first) = run(&dir, r.clone(), stories(), false);
        assert_eq!(first.len(), 1);
        // The story files do not even exist: nothing is read to decide.
        let (progress, calls) = run(&dir, r, stories(), false);
        assert!(calls.is_empty(), "calls: {calls:?}");
        assert!(progress.iter().all(|p| p.outcome == Outcome::Skipped), "map-linked copy counts as identified");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_manual_relink_leaves_the_ifid_map_alone() {
        let dir = crate::scratch_dir("ifid-map-relink");
        let mut r = HashMap::new();
        r.insert("ZCODE-1-A".to_string(), entry_record("tuidaaaa", "Game", &["ZCODE-1-A", "ZCODE-2-B"]));
        r.insert("tuidcccc".to_string(), entry_record("tuidcccc", "Elsewhere", &["ZCODE-7-Q"]));
        let stories = vec![target(&dir, "a.z5", "ZCODE-1-A"), target(&dir, "b.z5", "ZCODE-2-B")];
        let (_, _) = run(&dir, r.clone(), stories, false);

        let fake = Fake::new(r);
        let fetcher = Fetcher::new(Box::new(fake), crate::data_roots::DataRoots::single(&dir), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![target(&dir, "b.z5", "ZCODE-2-B")],
            forced: false,
            id_override: Some("tuidcccc".into()),
        });
        assert_eq!(wait_for(&fetcher, 1)[0].outcome, Outcome::Fetched);

        let b = story_info::load(&dir_of(&dir, "b.z5"), "ZCODE-2-B").unwrap();
        assert_eq!(b.fetched.unwrap().ifdb_tuid.as_deref(), Some("tuidcccc"), "the copy follows the relink");
        let a = story_info::load(&dir_of(&dir, "a.z5"), "ZCODE-1-A").unwrap();
        assert_eq!(a.fetched.unwrap().ifdb_tuid.as_deref(), Some("tuidaaaa"), "the other copy does not");
        let cat = dir_of(&dir, "b.z5").parent().unwrap().to_path_buf();
        assert_eq!(crate::ifdb_store::tuid_for_ifid(&cat, "ZCODE-2-B").as_deref(), Some("tuidaaaa"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
