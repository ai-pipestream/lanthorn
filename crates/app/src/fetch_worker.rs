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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::ifdb::{FetchError, FetchOutcome, MetadataSource};
use crate::ifiction::IFiction;
use crate::story_info::{self, FetchedMeta, StoryInfo};

/// A batch of stories to fetch: `(story path, IFID)` pairs. `forced` (`f`)
/// ignores the sidecar cache entirely; unset (`r`) skips a story already at
/// `FETCH_VERSION`.
pub struct FetchOrder {
    pub stories: Vec<(PathBuf, String)>,
    pub forced: bool,
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
    pub fn new(source: Box<dyn MetadataSource>, data_base: PathBuf, delay: Duration) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<FetchOrder>();
        let (res_tx, res_rx) = mpsc::channel::<FetchProgress>();
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_busy = Arc::clone(&busy);
        let worker = thread::spawn(move || {
            while let Ok(order) = req_rx.recv() {
                let total = order.stories.len();
                for (i, (path, ifid)) in order.stories.into_iter().enumerate() {
                    if worker_cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let progress =
                        fetch_one(source.as_ref(), &data_base, path, ifid, order.forced, delay, i, total);
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

/// Handle one story: skip-check, fetch-or-skip, sidecar write, cover write.
/// Sleeps `delay` only when an actual fetch was attempted — a skip costs no
/// request, so it must not sleep.
#[allow(clippy::too_many_arguments)]
fn fetch_one(
    source: &dyn MetadataSource,
    data_base: &Path,
    path: PathBuf,
    ifid: String,
    forced: bool,
    delay: Duration,
    index: usize,
    total: usize,
) -> FetchProgress {
    let game_dir = crate::storage::game_dir(data_base, &crate::storage::story_key(&path));
    let existing = story_info::load(&game_dir, &ifid);

    let (title, outcome) = if !story_info::needs_fetch(existing.as_ref(), forced) {
        (stem_title(&path), Outcome::Skipped)
    } else {
        let result = match source.fetch(&ifid) {
            Ok(FetchOutcome::Found(iff)) => {
                let cover = maybe_fetch_cover(source, &game_dir, &path, &iff);
                let title = iff.title.clone().unwrap_or_else(|| stem_title(&path));
                write_fetched(&game_dir, &ifid, found_meta(&iff, cover));
                (title, Outcome::Fetched)
            }
            Ok(FetchOutcome::NotFound) => {
                write_fetched(&game_dir, &ifid, not_found_meta());
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

    FetchProgress { done: index + 1, total, path, title, outcome }
}

/// Load the sidecar (or start a fresh one), set `.fetched`, and save —
/// preserving any existing `.probe` block (SQ-0276's slot).
fn write_fetched(game_dir: &Path, ifid: &str, fetched: FetchedMeta) {
    let mut info = story_info::load(game_dir, ifid).unwrap_or_else(|| StoryInfo {
        format_version: story_info::FORMAT_VERSION,
        ifid: ifid.to_string(),
        fetched: None,
        probe: None,
    });
    info.fetched = Some(fetched);
    let _ = story_info::save(game_dir, &info);
}

fn found_meta(iff: &IFiction, cover: Option<String>) -> FetchedMeta {
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
        cover: None,
        not_found: true,
    }
}

/// On a `Found` result with a cover URL and no cover already local to the
/// story (a blorb's own `Fspc` frontispiece), fetch it and cache it beside
/// the sidecar as `cover.png`. Best-effort: any failure (no URL, a decode-free
/// local cover already present, a transport error, a write error) simply
/// leaves the field `None` — it never turns a successful metadata fetch into
/// a failed one.
fn maybe_fetch_cover(
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
    std::fs::create_dir_all(game_dir).ok()?;
    std::fs::write(game_dir.join("cover.png"), &bytes).ok()?;
    Some("cover.png".to_string())
}

fn stem_title(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

fn now_rfc3339() -> String {
    jiff::Timestamp::now().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::story_info::ProbeMeta;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;

    /// A unique temp dir per call, safe under parallel test execution (mirrors
    /// `story_info`'s own `tmp()` helper).
    fn tmp() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("bm_fetch_worker_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
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
        /// Artificial per-call delay, used only by the cancel test to open a
        /// deterministic window between "fetch started" and "fetch returned".
        fetch_sleep: Duration,
    }

    impl Fake {
        fn new(responses: HashMap<String, FakeResp>) -> Self {
            Self {
                responses,
                calls: Arc::new(Mutex::new(Vec::new())),
                cover_calls: Arc::new(Mutex::new(Vec::new())),
                cover_bytes: vec![1, 2, 3, 4],
                fetch_sleep: Duration::ZERO,
            }
        }
    }

    impl MetadataSource for Fake {
        fn fetch(&self, ifid: &str) -> Result<FetchOutcome, FetchError> {
            self.calls.lock().unwrap().push(ifid.to_string());
            if !self.fetch_sleep.is_zero() {
                thread::sleep(self.fetch_sleep);
            }
            match self.responses.get(ifid) {
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
            cover: None,
            not_found: false,
        }
    }

    #[test]
    fn r_order_skips_a_story_already_at_fetch_version_without_calling_the_source() {
        let data_base = tmp();
        let path = data_base.join("game.z5");
        let ifid = "ZCODE-1-000001".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));
        story_info::save(&game_dir, &stub_info(&ifid, Some(up_to_date_meta()))).unwrap();

        let fake = Fake::new(HashMap::new());
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid)], forced: false });

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
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));
        story_info::save(&game_dir, &stub_info(&ifid, Some(up_to_date_meta()))).unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            ifid.clone(),
            FakeResp::Found(Box::new(IFiction { title: Some("Refreshed".into()), ..Default::default() })),
        );
        let fake = Fake::new(responses);
        let calls = Arc::clone(&fake.calls);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: true });

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
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));

        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::NotFound);
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: false });

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
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));
        // Pre-existing sidecar with only a probe block (never fetched) — must
        // survive byte-for-byte since a transport error writes nothing.
        let before = stub_info(&ifid, None);
        let mut before = before;
        before.probe = Some(ProbeMeta { probed_at: Some("earlier".into()) });
        story_info::save(&game_dir, &before).unwrap();

        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Err("connection reset".into()));
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: false });

        let progress = wait_for(&fetcher, 1);
        assert_eq!(progress[0].outcome, Outcome::Failed("connection reset".into()));

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert!(reloaded.fetched.is_none(), "a transport error must not write a fetched block");
        assert_eq!(reloaded.probe, before.probe, "the untouched probe block must survive");

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
        let dir1 = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path1));
        let dir2 = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path2));

        let mut responses = HashMap::new();
        for ifid in [&ifid1, &ifid2, &ifid3] {
            responses.insert(
                ifid.clone(),
                FakeResp::Found(Box::new(IFiction { title: Some(format!("Story {ifid}")), ..Default::default() })),
            );
        }
        let mut fake = Fake::new(responses);
        // Generous window: the worker blocks here on story 1 long enough for
        // the test thread to call cancel() before story 2 is ever attempted.
        fake.fetch_sleep = Duration::from_millis(150);
        let calls = Arc::clone(&fake.calls);

        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![
                (path1, ifid1.clone()),
                (path2, ifid2.clone()),
                (path3, ifid3.clone()),
            ],
            forced: true,
        });

        thread::sleep(Duration::from_millis(20)); // comfortably inside story 1's fetch
        fetcher.cancel();

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
        let skip1 = (data_base.join("skip1.z5"), "ZCODE-1-200001".to_string());
        let skip2 = (data_base.join("skip2.z5"), "ZCODE-1-200002".to_string());
        let fetched = (data_base.join("fetch3.z5"), "ZCODE-1-200003".to_string());
        for (path, ifid) in [&skip1, &skip2] {
            let dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(path));
            story_info::save(&dir, &stub_info(ifid, Some(up_to_date_meta()))).unwrap();
        }

        let mut responses = HashMap::new();
        responses.insert(fetched.1.clone(), FakeResp::NotFound);
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder {
            stories: vec![skip1.clone(), skip2.clone(), fetched.clone()],
            forced: false,
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
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));
        let mut seed = stub_info(&ifid, None);
        seed.probe = Some(ProbeMeta { probed_at: Some("2026-01-01".into()) });
        story_info::save(&game_dir, &seed).unwrap();

        let mut responses = HashMap::new();
        responses.insert(
            ifid.clone(),
            FakeResp::Found(Box::new(IFiction { title: Some("Title".into()), ..Default::default() })),
        );
        let fake = Fake::new(responses);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: true });
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
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));

        let iff = IFiction {
            title: Some("Covered".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "abc123".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=abc123&version=1".into()),
            }),
            ..Default::default()
        };
        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Found(Box::new(iff)));
        let fake = Fake::new(responses);
        let cover_calls = Arc::clone(&fake.cover_calls);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: true });
        wait_for(&fetcher, 1);

        assert_eq!(cover_calls.lock().unwrap().len(), 1, "the cover must be fetched exactly once");
        let cover_bytes = std::fs::read(game_dir.join("cover.png")).expect("cover.png must be written");
        assert_eq!(cover_bytes, vec![1, 2, 3, 4]);

        let reloaded = story_info::load(&game_dir, &ifid).unwrap();
        assert_eq!(reloaded.fetched.unwrap().cover.as_deref(), Some("cover.png"));

        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn found_with_an_existing_local_frontispiece_skips_the_cover_fetch() {
        let data_base = tmp();
        let path = data_base.join("game.gblorb");
        std::fs::write(&path, blorb_with_fspc_and_cover()).unwrap();
        let ifid = "ZCODE-1-400002".to_string();
        let game_dir = crate::storage::game_dir(&data_base, &crate::storage::story_key(&path));

        let iff = IFiction {
            title: Some("Already Covered".into()),
            ifdb: Some(crate::ifiction::IfdbExt {
                tuid: "xyz".into(),
                link: None,
                cover_url: Some("https://ifdb.org/coverart?id=xyz&version=1".into()),
            }),
            ..Default::default()
        };
        let mut responses = HashMap::new();
        responses.insert(ifid.clone(), FakeResp::Found(Box::new(iff)));
        let fake = Fake::new(responses);
        let cover_calls = Arc::clone(&fake.cover_calls);
        let fetcher = Fetcher::new(Box::new(fake), data_base.clone(), Duration::ZERO);
        fetcher.request(FetchOrder { stories: vec![(path, ifid.clone())], forced: true });
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
}
