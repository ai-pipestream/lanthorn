//! asciinema v2 casts, recorded from the pty harness (SQ-0943).
//!
//! WHY THIS IS SMALL. [`super::driver`] already returns
//! `Capture { bytes, flushes }`, and a [`Flush`](super::driver::Flush) is a
//! timestamped byte range from the REAL binary under a real pty. An asciinema v2
//! cast is a JSON header line followed by `[seconds, "o", data]` events. So the
//! recorder is a serialiser over data the harness already collects, and it needs
//! no rasteriser, no typeface and no decision about typography — the player
//! draws with the reader's own terminal font, which is the one thing the stills
//! in SQ-0942 can never be honest about.
//!
//! THE DECISION THAT MATTERS: DO NOT ANSWER AS KITTY. The harness normally
//! insists on negotiating kitty, because a capture that does not is silently
//! measuring the wrong backend. For a cast that is exactly backwards. The
//! asciinema player renders cells and SGR; it drops kitty's APC graphics on the
//! floor, so recording a kitty session produces a file in which the artwork
//! silently vanishes and lanthorn looks like it draws nothing.
//!
//! Answering as a terminal with no kitty support does NOT cost the artwork.
//! `ratatui-image` treats half-blocks as its universal fallback, so the whole v6
//! pixel path still runs and lands as `▀` with a foreground and a background —
//! glyphs and SGR, which the player supports all the way to 24-bit colour. Zork
//! Zero's frame, Journey's picture column and Arthur's flank all survive, chunky
//! but recognisable. That is a much better cast than plain text, and it is
//! honest: it is what lanthorn really draws on a terminal without graphics.
//!
//! WHAT IT COSTS. A half-block frame is far heavier in bytes than a text frame —
//! every cell can carry two 24-bit colours — so [`CastEntry::max_bytes`] exists
//! and every graphical cast is short on purpose. The text titles and the CLI
//! clients have no such constraint.
//!
//! UTF-8 IS NOT OPTIONAL. A v2 event's data is a JSON string, so it must be
//! valid UTF-8, and a multi-byte character can straddle two flushes — half-block
//! output is nothing BUT multi-byte characters. [`to_cast`] carries an
//! incomplete tail forward to the next event rather than replacing it, because a
//! lossy conversion here would put U+FFFD in the middle of somebody's frame.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::driver::{Capture, Key};

/// Which program a cast records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Program {
    /// The TUI.
    #[default]
    Lanthorn,
    ZvmCli,
    GvmCli,
    ScottCli,
}

impl Program {
    /// The binary's name as cargo built it.
    pub fn binary(self) -> &'static str {
        match self {
            Program::Lanthorn => "lanthorn",
            Program::ZvmCli => "zvm-cli",
            Program::GvmCli => "gvm-cli",
            Program::ScottCli => "scott-cli",
        }
    }

    pub fn is_cli(self) -> bool {
        !matches!(self, Program::Lanthorn)
    }
}

/// One recording, exactly as `examples/casts.toml` spells it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastEntry {
    /// Stable slug; becomes the `.cast` filename.
    pub id: String,
    /// Shown in the player's title bar, and in the proof index.
    pub title: String,
    /// What the recording is for, in a sentence. Not played; read by whoever
    /// places it on a page.
    pub caption: String,
    /// Which binary to drive.
    #[serde(default)]
    pub program: Program,
    /// The medium, relative to the repository root. Empty for a cast of a
    /// program that takes no story — `zvm-cli --machines` is the one.
    #[serde(default)]
    pub media: String,
    /// Arguments. For a CLI cast these are the WHOLE command line after the
    /// binary, with `{media}` standing for the story path; for a lanthorn cast
    /// they are extra arguments beside the story.
    #[serde(default)]
    pub args: Vec<String>,
    /// The key spec, in [`Key::parse`]'s spelling.
    pub keys: String,
    /// Terminal size in cells, `COLSxROWS`.
    pub size: String,
    /// Keep the map pane. lanthorn casts only — and the automapper cast is the
    /// whole reason the flag is here.
    #[serde(default)]
    pub show_map: bool,
    /// The PRNG seed pinned for this cast. lanthorn only; the CLI clients have
    /// no `--user-dir` to pin it in.
    #[serde(default = "default_seed")]
    pub seed: u32,
    /// Seconds of silence the player is allowed to replay before it skips ahead.
    ///
    /// The key spec's waits are wall-clock and generous — a `wait:3000` exists
    /// so a slow medium finishes paging, not because anyone wants three seconds
    /// of nothing. Capping the idle time keeps the recording watchable without
    /// falsifying when anything happened.
    #[serde(default = "default_idle")]
    pub idle_time_limit: f64,
    /// Refuse the cast if it comes out larger than this. Half-block frames are
    /// heavy, and a cast nobody will wait to load is not an asset.
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    /// Text that must appear in the recorded stream, or the cast is discarded.
    ///
    /// Same guard, same reason, as the gallery's (SQ-0942): a cast of a browser,
    /// a boot prompt, or the wrong story off a shared disk is indistinguishable
    /// from a good one until somebody plays it.
    pub expect: Vec<String>,
}

fn default_seed() -> u32 {
    12345
}

fn default_idle() -> f64 {
    2.0
}

fn default_max_bytes() -> usize {
    3 * 1024 * 1024
}

/// The whole cast manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CastManifest {
    pub casts: Vec<CastEntry>,
}

impl CastManifest {
    pub fn parse(text: &str) -> Result<CastManifest, String> {
        let m: CastManifest = toml::from_str(text).map_err(|e| format!("cast manifest: {e}"))?;
        if m.casts.is_empty() {
            return Err("cast manifest: no [[casts]]".into());
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for c in &m.casts {
            c.validate()?;
            if !seen.insert(c.id.as_str()) {
                return Err(format!("cast manifest: duplicate cast id `{}` — ids are filenames", c.id));
            }
        }
        Ok(m)
    }

    /// The committed manifest's path: `crates/app/examples/casts.toml`.
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/casts.toml")
    }

    pub fn load(path: &Path) -> Result<CastManifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cast manifest: reading {}: {e}", path.display()))?;
        CastManifest::parse(&text)
    }
}

impl CastEntry {
    fn validate(&self) -> Result<(), String> {
        let who = &self.id;
        if self.id.is_empty()
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "cast manifest: cast id `{who}` must be lowercase ASCII, digits and dashes — it becomes a filename"
            ));
        }
        for (field, value) in [("title", &self.title), ("caption", &self.caption)] {
            if value.trim().is_empty() {
                return Err(format!("cast manifest: `{who}` has an empty `{field}`"));
            }
        }
        self.size_cells().map(|_| ())?;
        self.keys().map(|_| ())?;
        if self.expect.is_empty() {
            return Err(format!(
                "cast manifest: `{who}` names nothing in `expect` — a cast of a browser or a boot \
                 prompt looks exactly like a good one until somebody plays it"
            ));
        }
        // The tool answers the capability queries and owns the throwaway home;
        // a cast that forced a protocol would be recording something the player
        // cannot replay.
        for owned in ["--image-protocol", "--user-dir"] {
            if self.args.iter().any(|a| a == owned) {
                return Err(format!(
                    "cast manifest: `{who}` passes `{owned}` — the cast tool owns that argument, and \
                     a forced graphics protocol is exactly what an asciinema player cannot show"
                ));
            }
        }
        if self.program.is_cli() && self.show_map {
            return Err(format!("cast manifest: `{who}` is a CLI cast; the CLI clients have no map pane"));
        }
        if self.media.is_empty() && self.args.iter().any(|a| a.contains("{media}")) {
            return Err(format!("cast manifest: `{who}` uses `{{media}}` but names no `media`"));
        }
        if self.media.is_empty() && self.program == Program::Lanthorn {
            return Err(format!("cast manifest: `{who}` is a lanthorn cast with no `media` to play"));
        }
        if self.idle_time_limit <= 0.0 {
            return Err(format!("cast manifest: `{who}` has a non-positive idle_time_limit"));
        }
        Ok(())
    }

    pub fn size_cells(&self) -> Result<(u16, u16), String> {
        let (c, r) = self
            .size
            .split_once('x')
            .ok_or_else(|| format!("cast manifest: `{}` has size `{}`, wanted COLSxROWS", self.id, self.size))?;
        let cols: u16 = c.trim().parse().map_err(|_| format!("cast manifest: `{}`: bad column count", self.id))?;
        let rows: u16 = r.trim().parse().map_err(|_| format!("cast manifest: `{}`: bad row count", self.id))?;
        if cols == 0 || rows == 0 {
            return Err(format!("cast manifest: `{}` has a zero dimension in `{}`", self.id, self.size));
        }
        Ok((cols, rows))
    }

    pub fn keys(&self) -> Result<Vec<Key>, String> {
        self.keys
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| Key::parse(t).map_err(|e| format!("cast manifest: `{}`: {e}", self.id)))
            .collect()
    }

    /// The medium's absolute path, or `None` for a cast that takes no story.
    pub fn media_path(&self) -> Option<PathBuf> {
        if self.media.is_empty() {
            return None;
        }
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(&self.media))
    }

    /// The whole argument list for a CLI cast, `{media}` resolved.
    pub fn cli_argv(&self) -> Vec<String> {
        let media = self.media_path().map(|p| p.display().to_string()).unwrap_or_default();
        self.args.iter().map(|a| a.replace("{media}", &media)).collect()
    }
}

// ── The serialiser ────────────────────────────────────────────────────────────

/// The v2 header's fields, as far as this recorder fills them in.
#[derive(Clone, Debug)]
pub struct Header {
    pub width: u16,
    pub height: u16,
    pub title: String,
    pub idle_time_limit: f64,
    /// Unix seconds. The cast records WHEN it was made, which is half of what
    /// makes a recording a fixture rather than a picture.
    pub timestamp: i64,
    /// The `env` map. Only `TERM` and `SHELL` are conventional; `TERM` is the
    /// one that matters, and it is the terminal the harness PRETENDED to be.
    pub term: String,
    /// A free-text note the player never shows, carried so the file can answer
    /// "why is there no kitty artwork in this recording" on its own.
    pub note: String,
}

/// Serialise a capture as an asciinema v2 cast.
///
/// One event per flush, at the flush's own offset from the start of the run. A
/// flush is bytes that arrived with no `Spec::quiet` gap between them, which for
/// an app that writes a frame in one go IS a frame — so the pacing in the file
/// is the pacing the program actually had, not a reconstruction.
pub fn to_cast(cap: &Capture, header: &Header) -> String {
    use std::fmt::Write as _;

    let mut s = String::with_capacity(cap.bytes.len() * 2);
    let _ = writeln!(
        s,
        "{{\"version\": 2, \"width\": {}, \"height\": {}, \"timestamp\": {}, \
         \"idle_time_limit\": {}, \"title\": {}, \"env\": {{\"TERM\": {}, \"SHELL\": \"/bin/sh\"}}, \
         \"lanthorn\": {{\"build\": {}, \"note\": {}}}}}",
        header.width,
        header.height,
        header.timestamp,
        json_num(header.idle_time_limit),
        json_str(&header.title),
        json_str(&header.term),
        json_str(buildinfo::LONG),
        json_str(&header.note),
    );

    // A multi-byte character split across two flushes must not be closed off
    // with U+FFFD: hold the incomplete tail and prepend it to the next event.
    let mut carry: Vec<u8> = Vec::new();
    for f in &cap.flushes {
        let Some(chunk) = cap.bytes.get(f.offset..f.offset + f.len) else { continue };
        let mut buf = std::mem::take(&mut carry);
        buf.extend_from_slice(chunk);
        let (text, tail) = split_utf8(&buf);
        carry = tail;
        if text.is_empty() {
            continue;
        }
        let _ = writeln!(s, "[{}, \"o\", {}]", json_num(f.at.as_secs_f64()), json_str(&text));
    }
    // Whatever is left is genuinely not UTF-8 — a truncated write at the very
    // end. Emit it lossily rather than dropping it, so the file is not silently
    // shorter than the run.
    if !carry.is_empty() {
        let at = cap.flushes.last().map(|f| f.at.as_secs_f64()).unwrap_or(0.0);
        let _ = writeln!(s, "[{}, \"o\", {}]", json_num(at), json_str(&String::from_utf8_lossy(&carry)));
    }
    s
}

/// Split `buf` into the longest valid UTF-8 prefix and the bytes after it.
///
/// The tail is only ever an INCOMPLETE trailing sequence in practice; anything
/// longer than three bytes is real corruption and is handed back to be dealt
/// with once, at the end.
fn split_utf8(buf: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(buf) {
        Ok(s) => (s.to_string(), Vec::new()),
        Err(e) => {
            let good = e.valid_up_to();
            // SAFETY-equivalent: `valid_up_to` is exactly the length that parses.
            let text = String::from_utf8_lossy(&buf[..good]).into_owned();
            match e.error_len() {
                // An incomplete sequence at the end: hold it for the next flush.
                None => (text, buf[good..].to_vec()),
                // Genuinely invalid bytes in the middle. Drop the offending run
                // and carry on rather than truncating the rest of the frame.
                Some(n) => {
                    let (rest, tail) = split_utf8(&buf[good + n..]);
                    (format!("{text}\u{FFFD}{rest}"), tail)
                }
            }
        }
    }
}

/// `f64` in a form JSON accepts — never `NaN`, never `inf`, always a decimal
/// point so a whole number does not read as an integer.
fn json_num(v: f64) -> String {
    if !v.is_finite() {
        return "0.0".to_string();
    }
    format!("{v:.6}")
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// APC `_G` commands in `bytes` that would put PIXELS on a screen — every
/// graphics command except the capability probe.
///
/// `Negotiation::apc_commands` counts every `ESC _ G` in the stream, and the
/// probe lanthorn sends to ASK whether the terminal does kitty is one of them.
/// It is app→terminal traffic like everything else in a capture, so a cast that
/// correctly drew nothing graphical still reads as "1 APC command" there. That
/// is right for the negotiation verdict and wrong for the only question this
/// tool asks: is there anything in this recording the player will drop?
///
/// The probe is `a=q`; a transmission or a placement is not.
pub fn graphics_commands(bytes: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while let Some(off) = find(&bytes[i..], b"\x1b_G") {
        let start = i + off + 3;
        // The parameter list runs to the payload separator or the terminator,
        // whichever comes first.
        let end = [find(&bytes[start..], b";"), find(&bytes[start..], b"\x1b\\")]
            .into_iter()
            .flatten()
            .min()
            .map(|d| start + d)
            .unwrap_or(bytes.len());
        if find(&bytes[start..end], b"a=q").is_none() {
            n += 1;
        }
        i = start;
    }
    n
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// What every cast this recorder writes has to be able to say for itself.
pub const NO_KITTY_NOTE: &str =
    "Recorded with the kitty capability query UNANSWERED, so lanthorn drew through its half-block \
     fallback. The asciinema player renders cells and SGR and drops kitty's APC graphics, so a kitty \
     recording would show no artwork at all; half-blocks are glyphs and colour, and replay exactly.";

/// A plain index over the finished casts — the proof sheet's equivalent, and the
/// place the "why is there no kitty artwork" answer is written down in prose.
pub fn contact_sheet(rows: &[(String, String, String, usize, f64)], failed: &[String]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "# lanthorn casts\n");
    let _ = writeln!(s, "lanthorn `{}`, {} recording(s).\n", buildinfo::LONG, rows.len());
    let _ = writeln!(s, "{NO_KITTY_NOTE}\n");
    if !failed.is_empty() {
        let _ = writeln!(s, "## {} did not record\n", failed.len());
        for f in failed {
            let _ = writeln!(s, "- {f}");
        }
        s.push('\n');
    }
    let _ = writeln!(s, "| cast | title | seconds | KiB | what it is for |");
    let _ = writeln!(s, "|---|---|---:|---:|---|");
    for (id, title, caption, bytes, secs) in rows {
        let _ = writeln!(
            s,
            "| `{id}.cast` | {title} | {secs:.1} | {} | {} |",
            bytes / 1024,
            caption.replace('\n', " ")
        );
    }
    s
}
