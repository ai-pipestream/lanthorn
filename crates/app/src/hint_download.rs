//! On-demand InvisiClues hint-file downloader (SQ-0445).
//!
//! The story picker's Shift-H key resolves a [`crate::hints::HintDownload`] for
//! the highlighted game and hands its URL + destination here. Each download runs
//! on its own short-lived thread; results drain (non-blocking) into the picker
//! loop, mirroring how `cover::CoverDecoder` and `fetch_worker::Fetcher` feed the
//! UI without an async runtime.
//!
//! Only a handful of these ever run (one per keypress), so a persistent worker
//! thread would be overkill — a thread per request sharing one result channel is
//! simpler and just as non-blocking.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Cap on a downloaded hint file. Real InvisiClues images are tiny (~30–60 KB);
/// this bounds a misbehaving mirror (or an Internet Archive error page) well
/// clear of any legitimate file.
const MAX_HINT: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// What became of one download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HintDlOutcome {
    /// The file was fetched, validated, and written to `dest`.
    Done,
    /// The download failed (transport error, non–Z-machine payload, or write
    /// error); `dest` was not created.
    Failed(String),
}

/// One completed download, drained by the picker loop.
#[derive(Debug, Clone)]
pub struct HintDlResult {
    /// Where the file was (or would have been) saved, beside the story.
    pub dest: PathBuf,
    /// The story's display title, for the status line.
    pub title: String,
    pub outcome: HintDlOutcome,
}

/// Spawns hint downloads and drains their results without blocking the UI.
pub struct HintDownloader {
    tx: mpsc::Sender<HintDlResult>,
    rx: mpsc::Receiver<HintDlResult>,
    inflight: usize,
}

impl Default for HintDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl HintDownloader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, inflight: 0 }
    }

    /// Begin downloading `url` to `dest` (named after the story `title`). Returns
    /// immediately; the result arrives via [`drain`](Self::drain).
    pub fn start(&mut self, url: String, dest: PathBuf, title: String) {
        self.inflight += 1;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = match fetch_bytes(&url) {
                Ok(bytes) => finalize_download(&bytes, &dest),
                Err(e) => HintDlOutcome::Failed(e),
            };
            let _ = tx.send(HintDlResult { dest, title, outcome });
        });
    }

    /// Non-blocking drain of every download finished since the last call.
    pub fn drain(&mut self) -> Vec<HintDlResult> {
        let done: Vec<_> = self.rx.try_iter().collect();
        self.inflight = self.inflight.saturating_sub(done.len());
        done
    }

    /// True while at least one download is still in flight.
    pub fn busy(&self) -> bool {
        self.inflight > 0
    }
}

/// GET `url` and return its body bytes. ureq follows the Internet Archive's
/// 302 redirect to the nearest capture automatically.
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(user_agent())
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(MAX_HINT)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// Validate `bytes` as a Z-machine story and, if valid, write them to `dest`.
fn finalize_download(bytes: &[u8], dest: &Path) -> HintDlOutcome {
    if !looks_like_zmachine(bytes) {
        return HintDlOutcome::Failed("downloaded file is not a Z-machine story".to_string());
    }
    match std::fs::write(dest, bytes) {
        Ok(()) => HintDlOutcome::Done,
        Err(e) => HintDlOutcome::Failed(format!("write failed: {e}")),
    }
}

/// A raw Z-machine story image starts with a version byte in `1..=8` and carries
/// at least a full 64-byte header. This rejects an HTML error page (e.g. an
/// Internet Archive 404/503, which starts with `<`) or a truncated response.
fn looks_like_zmachine(bytes: &[u8]) -> bool {
    bytes.len() >= 64 && matches!(bytes[0], 1..=8)
}

fn user_agent() -> String {
    format!("babelmap/{} (+https://github.com/sharkusk/babelmap)", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zmachine_v5() -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0] = 5;
        b
    }

    #[test]
    fn looks_like_zmachine_accepts_a_story_and_rejects_junk() {
        assert!(looks_like_zmachine(&zmachine_v5()));
        // An HTML error page.
        assert!(!looks_like_zmachine(b"<!DOCTYPE html><html>404</html>"));
        // Too short to hold a header.
        assert!(!looks_like_zmachine(&[5u8; 10]));
        // Version byte out of range.
        let mut bad = zmachine_v5();
        bad[0] = 0;
        assert!(!looks_like_zmachine(&bad));
    }

    #[test]
    fn finalize_writes_a_valid_story_and_refuses_an_html_page() {
        let dir = std::env::temp_dir().join(format!("bm-hintdl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let good = dir.join("deadlineinv.z5");
        assert_eq!(finalize_download(&zmachine_v5(), &good), HintDlOutcome::Done);
        assert_eq!(std::fs::read(&good).unwrap(), zmachine_v5());

        let bad = dir.join("junk.z5");
        assert!(matches!(finalize_download(b"<html>oops</html>", &bad), HintDlOutcome::Failed(_)));
        assert!(!bad.exists(), "a rejected payload must not create the file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloader_starts_empty_and_not_busy() {
        let mut dl = HintDownloader::new();
        assert!(!dl.busy());
        assert!(dl.drain().is_empty());
    }
}
