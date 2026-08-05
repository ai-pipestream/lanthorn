//! One-shot query of the terminal's default foreground (OSC 10) and background
//! (OSC 11) colours (SQ-0510).
//!
//! The v6 raster/pixel canvas paints its default ink/page in RGB, but when the
//! active theme leaves fg/bg at "terminal default" there is no known RGB to use —
//! the old code fell back to a hardcoded light grey on black, which is unreadable
//! on a light-background terminal. This module asks the terminal for its actual
//! default colours so the raster canvas can follow the terminal (and thus the
//! theme's "default" intent) everywhere.
//!
//! Safety: the query must never hang, must never leak reply bytes into the
//! app's own input stream, and must never EAT the user's real input. We
//! terminate the query with a Device Status Report (`ESC[5n`) that every
//! responding terminal answers, and drain stdin NON-BLOCKING (`O_NONBLOCK` on
//! the already-raw fd) until that DSR reply arrives — which fully drains the
//! preceding OSC replies with it — giving up after a short silent window.
//! SQ-0642: this used to spawn a worker thread doing blocking `stdin.read()`s;
//! after the timeout the thread stayed parked in `read()` and, on a terminal
//! that never answers, later swallowed up to 64 bytes of the player's first
//! keystrokes before noticing the dead channel. The non-blocking drain leaves
//! nothing behind: when this function returns, nobody is reading stdin.
//! A terminal that answers nothing sent nothing, so there is nothing left to
//! leak; we return `None` for both colours and the caller keeps its fallbacks.

use std::io::Write;
use std::time::{Duration, Instant};

use image::Rgba;

/// Silence window: how long we wait without any reply bytes before giving up.
/// Reset whenever a chunk arrives, so a slow but responsive terminal is not cut
/// off; only a fully silent terminal waits this long once.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Poll interval between non-blocking read attempts while draining the reply.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The terminal's probed default colours. Each is `None` when the terminal did
/// not answer that query (degrade gracefully — the caller keeps its fallback).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TermDefaultColors {
    /// Default foreground (OSC 10) — the raster canvas's default ink.
    pub fg: Option<Rgba<u8>>,
    /// Default background (OSC 11) — the raster canvas's default page.
    pub bg: Option<Rgba<u8>>,
}

/// Query OSC 10 / OSC 11 on the current terminal. Must be called in the pre-UI
/// query window (before the app's own raw-mode/alternate-screen, alongside the
/// image-protocol Picker probe). Never hangs; never leaks reply bytes.
pub fn query_terminal_default_colors() -> TermDefaultColors {
    // OSC 10 (fg) + OSC 11 (bg), each `?`-queried and BEL-terminated, then a DSR
    // that responding terminals answer last — its reply (`ESC[0n`) marks the end
    // of the drain.
    match query_with(b"\x1b]10;?\x07\x1b]11;?\x07\x1b[5n") {
        Some(reply) => parse_osc_colors(&reply),
        None => TermDefaultColors::default(),
    }
}

/// Run any terminal query that ends in a DSR (`ESC[5n`) and return the raw reply.
/// Shared by every pre-UI probe (default colours, SGR-Pixels mouse support) so
/// they all inherit the same safety: raw mode only for the duration, a read that
/// stops at the DSR reply — which drains every earlier reply with it — and a
/// bounded wait so a silent terminal costs one timeout rather than a hang.
///
/// `None` when the terminal cannot be put in raw mode (a piped stdio never
/// answers) or answered nothing at all.
pub(crate) fn query_with(query: &[u8]) -> Option<String> {
    // A non-tty (piped) stdout/stdin never answers; skip to avoid the timeout.
    if crossterm::terminal::enable_raw_mode().is_err() {
        return None;
    }
    let reply = read_query_reply(query);
    let _ = crossterm::terminal::disable_raw_mode();
    (!reply.is_empty()).then_some(reply)
}

/// Write `query` and read the reply, bounded by a timeout. SQ-0642: the drain
/// is a non-blocking poll loop on this thread — no worker thread is ever left
/// parked in a blocking `stdin.read()` to eat the user's first keystrokes on a
/// terminal that never answers.
fn read_query_reply(query: &[u8]) -> String {
    {
        let mut out = std::io::stdout();
        if out.write_all(query).and_then(|_| out.flush()).is_err() {
            return String::new();
        }
    }
    drain_stdin_nonblocking()
}

/// One non-blocking read attempt's outcome, as seen by [`drain_until_dsr`].
#[cfg(any(unix, test))]
enum ReadStep {
    /// `n` bytes were read into the chunk.
    Data(usize),
    /// Nothing available right now (`EWOULDBLOCK`).
    NotReady,
    /// EOF or a hard read error — stop and return what we have.
    Closed,
}

/// The pure drain loop, factored over the read primitive so it is unit-testable
/// without a tty. Reads chunks until the DSR reply (`ESC[0n`) arrives (return
/// the full buffer), the source closes (return what arrived), or a silent
/// [`READ_TIMEOUT`] window passes with no bytes at all (give up → empty, the
/// same as the old worker-thread behaviour). The window resets on every chunk
/// so a slow-but-responsive terminal is not cut off. When this returns, no
/// reader of any kind remains — that is the point (SQ-0642).
#[cfg(any(unix, test))]
fn drain_until_dsr(mut read: impl FnMut(&mut [u8; 64]) -> ReadStep) -> String {
    let mut buf = String::new();
    let mut chunk = [0u8; 64];
    let mut deadline = Instant::now() + READ_TIMEOUT;
    loop {
        match read(&mut chunk) {
            ReadStep::Data(n) => {
                // OSC/DSR replies are pure ASCII, so a chunk boundary never
                // splits a multi-byte char.
                buf.push_str(&String::from_utf8_lossy(&chunk[..n.min(chunk.len())]));
                if buf.contains("\x1b[0n") {
                    // DSR reply seen — every earlier OSC reply is now drained.
                    return buf;
                }
                deadline = Instant::now() + READ_TIMEOUT;
            }
            ReadStep::NotReady => {
                if Instant::now() >= deadline {
                    return String::new(); // silent terminal — give up
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            ReadStep::Closed => return buf,
        }
    }
}

/// Drain the reply from the real stdin fd with `O_NONBLOCK` toggled on for the
/// duration (the fd is already in raw mode — see [`query_with`]). Restores the
/// original fd flags before returning.
#[cfg(unix)]
fn drain_stdin_nonblocking() -> String {
    let fd = libc::STDIN_FILENO;
    // SAFETY: fcntl F_GETFL/F_SETFL on a valid fd; no memory is involved.
    let old = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, old | libc::O_NONBLOCK) } < 0 {
        return String::new(); // cannot go non-blocking — skip rather than risk a hang
    }
    let out = drain_until_dsr(|chunk| {
        // SAFETY: reading at most `chunk.len()` bytes into a valid buffer.
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n > 0 {
            ReadStep::Data(n as usize)
        } else if n == 0 {
            ReadStep::Closed
        } else if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
            ReadStep::NotReady
        } else {
            ReadStep::Closed
        }
    });
    // SAFETY: restore the original flags.
    unsafe { libc::fcntl(fd, libc::F_SETFL, old) };
    out
}

/// Non-Unix (Windows): there is no portable non-blocking console read, and the
/// old blocking worker thread is exactly what swallowed user keystrokes on
/// terminals that never answer (SQ-0642). Skip the drain entirely — the caller
/// degrades to its fallbacks, same as a silent terminal.
#[cfg(not(unix))]
fn drain_stdin_nonblocking() -> String {
    String::new()
}

/// Extract the OSC 10 (fg) and OSC 11 (bg) colours from a raw query reply.
pub fn parse_osc_colors(reply: &str) -> TermDefaultColors {
    TermDefaultColors {
        fg: osc_color(reply, "10"),
        bg: osc_color(reply, "11"),
    }
}

/// Locate an `ESC ] <n> ; rgb:.../.../...` response for OSC number `n` and parse
/// its colour. The payload runs until BEL or ST/ESC. Returns `None` if the
/// response is absent or malformed.
fn osc_color(reply: &str, n: &str) -> Option<Rgba<u8>> {
    let marker = format!("]{n};");
    let start = reply.find(&marker)? + marker.len();
    let rest = &reply[start..];
    let end = rest.find(['\u{7}', '\u{1b}']).unwrap_or(rest.len());
    parse_rgb_spec(&rest[..end])
}

/// Parse an X11 `rgb:R/G/B` colour spec into 8-bit RGBA. Each channel is 1–4 hex
/// digits; handles both `rgb:RRRR/GGGG/BBBB` (16-bit) and `rgb:RR/GG/BB` (8-bit).
fn parse_rgb_spec(spec: &str) -> Option<Rgba<u8>> {
    let hex = spec.trim().strip_prefix("rgb:")?;
    let mut parts = hex.split('/');
    let r = parse_channel(parts.next()?)?;
    let g = parse_channel(parts.next()?)?;
    let b = parse_channel(parts.next()?)?;
    if parts.next().is_some() {
        return None; // more than three components
    }
    Some(Rgba([r, g, b, 255]))
}

/// Scale a 1–4 hex-digit channel to 8-bit per X11 `rgb:` semantics
/// (`value / (16^len - 1) * 255`, rounded).
fn parse_channel(s: &str) -> Option<u8> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    let max = (1u32 << (4 * s.len())) - 1; // 16^len - 1
    Some(((v * 255 + max / 2) / max) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rgb_16bit() {
        // Ghostty/kitty-style 16-bit reply: high byte wins.
        assert_eq!(parse_rgb_spec("rgb:ffff/ffff/ffff"), Some(Rgba([255, 255, 255, 255])));
        assert_eq!(parse_rgb_spec("rgb:0000/0000/0000"), Some(Rgba([0, 0, 0, 255])));
        assert_eq!(parse_rgb_spec("rgb:8080/1234/abcd"), Some(Rgba([0x80, 0x12, 0xab, 255])));
    }

    #[test]
    fn parse_rgb_8bit() {
        assert_eq!(parse_rgb_spec("rgb:ff/00/80"), Some(Rgba([255, 0, 0x80, 255])));
        assert_eq!(parse_rgb_spec("rgb:12/34/56"), Some(Rgba([0x12, 0x34, 0x56, 255])));
    }

    #[test]
    fn parse_rgb_malformed_is_none() {
        assert_eq!(parse_rgb_spec("ffff/ffff/ffff"), None); // no rgb: prefix
        assert_eq!(parse_rgb_spec("rgb:ffff/ffff"), None); // too few components
        assert_eq!(parse_rgb_spec("rgb:ffff/ffff/ffff/ffff"), None); // too many
        assert_eq!(parse_rgb_spec("rgb:gggg/0000/0000"), None); // non-hex
        assert_eq!(parse_rgb_spec("rgb://"), None); // empty channels
        assert_eq!(parse_rgb_spec(""), None);
    }

    #[test]
    fn extract_both_osc_from_reply() {
        // Realistic combined reply: OSC 10 (fg) + OSC 11 (bg) + DSR terminator.
        let reply = "\x1b]10;rgb:e8e8/e8e8/e8e8\x07\x1b]11;rgb:1c1c/1c1c/1c1c\x07\x1b[0n";
        let c = parse_osc_colors(reply);
        assert_eq!(c.fg, Some(Rgba([0xe8, 0xe8, 0xe8, 255])));
        assert_eq!(c.bg, Some(Rgba([0x1c, 0x1c, 0x1c, 255])));
    }

    #[test]
    fn extract_st_terminated_and_partial() {
        // ST (ESC\) terminator instead of BEL, and only the background answered.
        let reply = "\x1b]11;rgb:ffff/ffff/ffff\x1b\\\x1b[0n";
        let c = parse_osc_colors(reply);
        assert_eq!(c.fg, None);
        assert_eq!(c.bg, Some(Rgba([255, 255, 255, 255])));
    }

    #[test]
    fn no_reply_is_none() {
        let c = parse_osc_colors("");
        assert_eq!(c, TermDefaultColors::default());
        assert_eq!(c.fg, None);
        assert_eq!(c.bg, None);
    }

    // ── SQ-0642: the non-blocking drain loop ─────────────────────────────────

    /// Feed a scripted sequence of read outcomes to `drain_until_dsr`.
    fn scripted(steps: Vec<Vec<u8>>) -> impl FnMut(&mut [u8; 64]) -> ReadStep {
        let mut steps = steps.into_iter();
        move |chunk: &mut [u8; 64]| match steps.next() {
            Some(bytes) if bytes.is_empty() => ReadStep::NotReady,
            Some(bytes) => {
                let n = bytes.len().min(chunk.len());
                chunk[..n].copy_from_slice(&bytes[..n]);
                ReadStep::Data(n)
            }
            None => ReadStep::Closed,
        }
    }

    #[test]
    fn drain_stops_at_the_dsr_reply() {
        let reply = b"\x1b]10;rgb:e8/e8/e8\x07\x1b[0n".to_vec();
        let out = drain_until_dsr(scripted(vec![reply.clone()]));
        assert_eq!(out.as_bytes(), &reply[..]);
    }

    #[test]
    fn drain_handles_a_dsr_split_across_chunks_and_notready_gaps() {
        // The DSR arrives byte-split with WouldBlock gaps in between — the
        // window must reset on each chunk instead of timing out mid-reply.
        let out = drain_until_dsr(scripted(vec![
            b"\x1b]11;rgb:1c/1c/1c\x07".to_vec(),
            vec![], // NotReady
            b"\x1b[0".to_vec(),
            vec![], // NotReady
            b"n".to_vec(),
        ]));
        assert!(out.contains("\x1b[0n"));
        assert!(out.contains("]11;"));
    }

    #[test]
    fn drain_gives_up_on_a_silent_source_within_the_window() {
        // A source that never has data (a terminal that never answers): the
        // drain must return empty on THIS thread within roughly the timeout —
        // with no reader left behind to swallow later real input. The old
        // worker-thread version parked a blocking stdin.read() here forever.
        let start = std::time::Instant::now();
        let out = drain_until_dsr(|_| ReadStep::NotReady);
        assert_eq!(out, "");
        let elapsed = start.elapsed();
        assert!(elapsed >= READ_TIMEOUT, "waits out the silence window: {elapsed:?}");
        assert!(elapsed < READ_TIMEOUT * 5, "but does not hang: {elapsed:?}");
    }

    #[test]
    fn drain_returns_partial_data_when_the_source_closes() {
        // EOF mid-reply (piped stdin): keep what arrived rather than hanging.
        let out = drain_until_dsr(scripted(vec![b"\x1b]10;rgb:aa/bb/cc\x07".to_vec()]));
        assert_eq!(out, "\x1b]10;rgb:aa/bb/cc\x07");
    }

    #[test]
    fn drain_never_reads_again_after_returning() {
        // Structural pin for the SQ-0642 symptom: once drain_until_dsr returns,
        // its reader is never invoked again — there is no detached thread to
        // consume the user's first keystrokes later. (The old implementation
        // cannot pass this shape at all: its blocked reader outlived the call.)
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in = calls.clone();
        let out = drain_until_dsr(move |chunk| {
            calls_in.fetch_add(1, Ordering::SeqCst);
            chunk[..4].copy_from_slice(b"\x1b[0n");
            ReadStep::Data(4)
        });
        assert_eq!(out, "\x1b[0n");
        let after_return = calls.load(Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(calls.load(Ordering::SeqCst), after_return, "no reads after return");
    }
}
