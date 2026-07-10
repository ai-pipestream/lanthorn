//! Cover-art decoding + terminal-protocol caching for the story picker.
//!
//! `load_cover` pulls a blorb's `Fspc` frontispiece image and decodes it;
//! `CoverState` holds a bounded LRU cache of decoded images and lazily builds
//! (and caches) a `ratatui-image` protocol scaled to the panel's cover region
//! for the currently-selected story. `CoverDecoder` owns a background worker
//! thread that runs `load_cover` off the main loop so scrolling never stalls.
//! Every failure resolves to `None` — the picker simply shows no cover.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use ratatui::layout::{Rect, Size};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

/// Decode PNG/JPEG bytes into a `DynamicImage`. `None` on any decode failure.
pub fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    image::load_from_memory(bytes).ok()
}

/// Read `path`; if it is a blorb declaring an `Fspc` frontispiece, fetch and
/// decode that Pict. `None` when the file isn't a blorb, has no frontispiece,
/// the referenced Pict is missing, or the image doesn't decode.
pub fn load_cover(path: &Path) -> Option<image::DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    if !blorb::Blorb::is_blorb(&bytes) {
        return None;
    }
    let b = blorb::Blorb::parse(bytes).ok()?;
    let n = b.frontispiece()?;
    let (_ty, data) = b.resource(b"Pict", n)?;
    decode(data)
}

/// Cache capacity: how many decoded covers `CoverState` retains before the
/// least-recently-inserted one is evicted.
const CAP: usize = 32;

/// Selection-scoped cover state: a bounded LRU map of decoded images (one entry
/// per visited story; `None` records a coverless story so it isn't re-decoded)
/// plus a protocol cached by `(path, cols, rows)` so it is rebuilt only when the
/// selected story or the cover region's size changes.
#[derive(Default)]
pub struct CoverState {
    decoded: HashMap<PathBuf, Option<image::DynamicImage>>,
    order: VecDeque<PathBuf>,
    proto: Option<(PathBuf, u16, u16, Protocol)>,
}

impl CoverState {
    /// True when `path` has already been decoded (`Some` or `None`) — skip the
    /// re-read/decode. A cached `None` (coverless story) still counts.
    pub fn has(&self, path: &Path) -> bool {
        self.decoded.contains_key(path)
    }

    /// Record the decode result for `path` (`Some(img)` or a coverless `None`)
    /// in the LRU cache, evicting the oldest entry once capacity is exceeded.
    ///
    /// Re-inserting an existing path refreshes its recency without duplicating it
    /// in `order` (so `order` stays 1:1 with `decoded` — no leak, no premature
    /// eviction of a live key). A replaced image also drops any stale built
    /// protocol for that path so `protocol()` rebuilds from the new image.
    pub fn insert(&mut self, path: PathBuf, img: Option<image::DynamicImage>) {
        if self.decoded.insert(path.clone(), img).is_some() {
            // Existing key: move it to most-recent, and invalidate a stale raster.
            self.order.retain(|p| p != &path);
            if matches!(&self.proto, Some((p, _, _, _)) if *p == path) {
                self.proto = None;
            }
        }
        self.order.push_back(path);
        while self.decoded.len() > CAP {
            match self.order.pop_front() {
                Some(oldest) => { self.decoded.remove(&oldest); }
                None => break,
            }
        }
    }

    /// Build-or-reuse a protocol for `path`'s cover, fitted (aspect-preserved)
    /// into `area`. `None` when `path` has no decoded cover or the build fails.
    ///
    /// While `animating` is true and a protocol for `path` is already cached
    /// (at any size), that stale raster is reused rather than rebuilt — the
    /// panel width changes every frame during a slide, and re-resizing the
    /// image on each tick is expensive for no visible benefit mid-motion. The
    /// geometry catches up on the next non-animating (settled) frame.
    pub fn protocol(
        &mut self,
        picker: &Picker,
        path: &Path,
        area: Rect,
        animating: bool,
    ) -> Option<&Protocol> {
        let img = self.decoded.get(path).and_then(|o| o.as_ref())?;
        let cached_for_path = matches!(&self.proto, Some((p, _, _, _)) if p == path);
        if animating && cached_for_path {
            return self.proto.as_ref().map(|(_, _, _, p)| p);
        }
        let fresh = matches!(
            &self.proto,
            Some((p, w, h, _)) if p == path && *w == area.width && *h == area.height
        );
        if !fresh {
            let built = picker
                .new_protocol(img.clone(), Size::new(area.width, area.height), Resize::Fit(None))
                .ok()?;
            self.proto = Some((path.to_path_buf(), area.width, area.height, built));
        }
        self.proto.as_ref().map(|(_, _, _, p)| p)
    }
}

/// Background cover decoder: owns one long-lived worker thread that runs
/// `load_cover` off the main loop. Requests are queued on `req_tx`; decoded
/// results are drained (non-blocking) from `res_rx`. The worker exits cleanly
/// when the `CoverDecoder` is dropped (dropping `req_tx` makes the worker's
/// `recv()` err, ending its loop).
pub struct CoverDecoder {
    req_tx: std::sync::mpsc::Sender<PathBuf>,
    res_rx: std::sync::mpsc::Receiver<(PathBuf, Option<image::DynamicImage>)>,
    _worker: std::thread::JoinHandle<()>,
}

impl CoverDecoder {
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<PathBuf>();
        let (res_tx, res_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while let Ok(path) = req_rx.recv() {
                // ends when req_tx drops (picker exits)
                let img = load_cover(&path);
                if res_tx.send((path, img)).is_err() {
                    break;
                }
            }
        });
        Self { req_tx, res_rx, _worker: worker }
    }

    /// Queue `path` for background decoding. Silently dropped if the worker has
    /// already exited.
    pub fn request(&self, path: PathBuf) {
        let _ = self.req_tx.send(path);
    }

    /// Non-blocking drain of all decoded results ready so far.
    pub fn drain(&self) -> impl Iterator<Item = (PathBuf, Option<image::DynamicImage>)> + '_ {
        self.res_rx.try_iter()
    }
}

impl Default for CoverDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure debounce predicate: request a cover only when it isn't already cached,
/// isn't already in flight, and the selection has been settled at least
/// `debounce` (avoids decoding covers you scroll straight past).
pub fn should_request_cover(
    cached: bool,
    requested: bool,
    since_change: std::time::Duration,
    debounce: std::time::Duration,
) -> bool {
    !cached && !requested && since_change >= debounce
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decode_accepts_png_rejects_garbage() {
        assert!(decode(&png_bytes()).is_some());
        assert!(decode(b"not an image").is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn cover_state_caches_by_path_and_builds_protocol() {
        let mut st = CoverState::default();
        let path = Path::new("game.gblorb");
        assert!(!st.has(path));

        st.insert(path.to_path_buf(), decode(&png_bytes()));
        assert!(st.has(path));

        // halfblocks() needs no terminal query — deterministic in tests.
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = ratatui::layout::Rect::new(0, 0, 10, 5);
        assert!(st.protocol(&picker, path, area, false).is_some());

        // A different path has no cover until inserted.
        let other = Path::new("other.gblorb");
        assert!(!st.has(other));
        assert!(st.protocol(&picker, other, area, false).is_none());
    }

    #[test]
    fn insert_evicts_oldest_beyond_cap() {
        let mut st = CoverState::default();
        // Insert CAP + 3 distinct paths; the 3 oldest must be evicted.
        let paths: Vec<PathBuf> =
            (0..CAP + 3).map(|i| PathBuf::from(format!("game{i}.gblorb"))).collect();
        for p in &paths {
            st.insert(p.clone(), decode(&png_bytes()));
        }
        assert_eq!(st.decoded.len(), CAP, "cache is bounded to CAP");
        // Oldest 3 evicted.
        for p in &paths[..3] {
            assert!(!st.has(p), "oldest entries should be evicted");
        }
        // Newest present (the just-inserted current is never evicted).
        for p in &paths[3..] {
            assert!(st.has(p), "recent entries should remain cached");
        }
    }

    #[test]
    fn reinsert_refreshes_recency_without_corrupting_lru() {
        let mut st = CoverState::default();
        // Fill exactly to CAP.
        let paths: Vec<PathBuf> =
            (0..CAP).map(|i| PathBuf::from(format!("game{i}.gblorb"))).collect();
        for p in &paths {
            st.insert(p.clone(), decode(&png_bytes()));
        }
        // Re-insert the OLDEST (game0) — it must move to most-recent, and `order`
        // must not gain a duplicate (else a later eviction would drop a live key
        // and `order`/`decoded` would diverge).
        st.insert(paths[0].clone(), decode(&png_bytes()));
        assert_eq!(st.decoded.len(), CAP, "re-insert must not change the count");
        assert_eq!(st.order.len(), CAP, "order must stay 1:1 with decoded (no dup)");

        // Now insert one NEW path: the oldest survivor (game1, since game0 was
        // refreshed) is evicted — NOT the just-refreshed game0.
        st.insert(PathBuf::from("new.gblorb"), decode(&png_bytes()));
        assert_eq!(st.decoded.len(), CAP);
        assert!(st.has(&paths[0]), "refreshed key must survive eviction");
        assert!(!st.has(&paths[1]), "the genuine oldest must be evicted");
        assert!(st.has(Path::new("new.gblorb")));
    }

    #[test]
    fn none_is_cached_and_not_redecoded() {
        let mut st = CoverState::default();
        let path = Path::new("coverless.z5");
        st.insert(path.to_path_buf(), None);
        // A cached `None` still counts as "known" so it isn't re-requested.
        assert!(st.has(path));
        // ...and yields no protocol without touching the disk.
        let picker = ratatui_image::picker::Picker::halfblocks();
        let area = ratatui::layout::Rect::new(0, 0, 10, 5);
        assert!(st.protocol(&picker, path, area, false).is_none());
    }

    /// A large (300x300) solid PNG — big enough that `Resize::Fit` actually
    /// scales it down differently for different target areas (unlike the tiny
    /// 2x2 `png_bytes()` fixture, which is already smaller than any halfblocks
    /// cell box and so never changes fitted size regardless of area).
    fn large_png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(300, 300, image::Rgb([0, 255, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn protocol_reuses_cached_raster_when_animating() {
        let mut st = CoverState::default();
        let path = Path::new("game.gblorb");
        st.insert(path.to_path_buf(), decode(&large_png_bytes()));

        let picker = ratatui_image::picker::Picker::halfblocks();

        let size_a = st
            .protocol(&picker, path, Rect::new(0, 0, 10, 6), false)
            .unwrap()
            .size();

        // Different area, but animating: reuse the stale raster, no rebuild.
        let size_animating = st
            .protocol(&picker, path, Rect::new(0, 0, 20, 10), true)
            .unwrap()
            .size();
        assert_eq!(size_animating, size_a, "animating reuse should not resize");

        // Same area, not animating: settle by rebuilding at the new size.
        let size_settled = st
            .protocol(&picker, path, Rect::new(0, 0, 20, 10), false)
            .unwrap()
            .size();
        assert_ne!(size_settled, size_a, "settled frame should rebuild at the new area");
    }

    #[test]
    fn decoder_round_trips_a_non_blorb_as_none() {
        // A real file that isn't a blorb: `load_cover` returns `None`, so the
        // worker delivers `(path, None)` — exercises spawn → request → decode →
        // deliver with no cover fixture needed.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("babelmap-cover-test-{}.txt", std::process::id()));
        std::fs::write(&path, b"not a blorb").unwrap();

        let d = CoverDecoder::new();
        d.request(path.clone());

        let mut got = None;
        // Bounded poll: worker is near-instant, but don't spin forever.
        for _ in 0..1000 {
            if let Some(res) = d.drain().next() {
                got = Some(res);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = std::fs::remove_file(&path);

        let (rp, img) = got.expect("worker should deliver a result");
        assert_eq!(rp, path);
        assert!(img.is_none(), "a non-blorb has no cover");
    }

    #[test]
    fn should_request_cover_truth_table() {
        let zero = Duration::ZERO;
        let debounce = Duration::from_millis(100);
        let past = Duration::from_millis(150);

        // Not cached, not requested, debounce elapsed → request.
        assert!(should_request_cover(false, false, past, debounce));
        // Time gate: not yet debounced → hold off.
        assert!(!should_request_cover(false, false, zero, debounce));
        // Already cached → never request.
        assert!(!should_request_cover(true, false, past, debounce));
        // Already in flight → never re-request.
        assert!(!should_request_cover(false, true, past, debounce));
        // Boundary: exactly at the debounce is enough (>=).
        assert!(should_request_cover(false, false, debounce, debounce));
    }
}
