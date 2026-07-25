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
//! Safety: the query must never hang and must never leak reply bytes into the
//! app's own input stream. We terminate the query with a Device Status Report
//! (`ESC[5n`) that every responding terminal answers, read on a worker thread
//! until that DSR reply arrives — which fully drains the preceding OSC replies
//! with it — and bail on a short recv timeout. A terminal that answers nothing
//! (e.g. Windows Terminal) sent nothing, so there is nothing left to leak; we
//! simply return `None` for both colours and the caller keeps its old fallbacks.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use image::Rgba;

/// Per-heartbeat read window. Reset on every worker heartbeat, so a slow but
/// responsive terminal is not cut off; only a fully silent terminal waits this
/// long once before we give up.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

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
    // A non-tty (piped) stdout/stdin never answers; skip to avoid the timeout.
    if crossterm::terminal::enable_raw_mode().is_err() {
        return TermDefaultColors::default();
    }
    let reply = read_query_reply();
    let _ = crossterm::terminal::disable_raw_mode();
    parse_osc_colors(&reply)
}

/// Write the OSC 10/11 + DSR query and read the reply, bounded by a timeout.
fn read_query_reply() -> String {
    // OSC 10 (fg) + OSC 11 (bg), each `?`-queried and BEL-terminated, then a DSR
    // that responding terminals answer last — its reply (`ESC[0n`) marks the end
    // of the drain.
    const QUERY: &[u8] = b"\x1b]10;?\x07\x1b]11;?\x07\x1b[5n";
    {
        let mut out = std::io::stdout();
        if out.write_all(QUERY).and_then(|_| out.flush()).is_err() {
            return String::new();
        }
    }

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = String::new();
        let mut chunk = [0u8; 64];
        let mut stdin = std::io::stdin();
        loop {
            // Heartbeat before each blocking read so a responsive-but-slow
            // terminal keeps the recv window open (mirrors ratatui-image's
            // query loop). A dead channel means the main side gave up.
            if tx.send(None).is_err() {
                return;
            }
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Some(buf));
                    return;
                }
                Ok(n) => {
                    // OSC/DSR replies are pure ASCII, so a chunk boundary never
                    // splits a multi-byte char.
                    buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    if buf.contains("\x1b[0n") {
                        // DSR reply seen — every earlier OSC reply is now drained.
                        let _ = tx.send(Some(buf));
                        return;
                    }
                }
            }
        }
    });

    loop {
        match rx.recv_timeout(READ_TIMEOUT) {
            Ok(Some(buf)) => return buf,
            Ok(None) => continue, // heartbeat: restart the window
            Err(_) => return String::new(), // silent terminal — give up
        }
    }
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
}
