//! Run the real `lanthorn` binary under a pty, answer the terminal queries it
//! asks, feed it keystrokes, and keep every byte it writes back (SQ-0762).
//!
//! Unix only — a pty is. The decoder in [`super::decode`] is portable, and the
//! callers gate this half with `#[cfg(unix)]`.
//!
//! WHY WE MUST PRETEND TO BE KITTY, PROPERLY. lanthorn picks its graphics backend
//! from `Picker::from_query_stdio`, which asks the terminal three questions before
//! the UI starts and falls back to half-blocks when nobody answers. A pty answers
//! nothing by itself, so a naive harness silently measures the half-block path —
//! the wrong backend, and every result it produces is worthless. [`Responder`]
//! answers those queries the way kitty does, and [`Capture::negotiated`] reports
//! what actually happened so a caller can refuse to go on if it is not kitty.

use std::ffi::CStr;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One scripted input step.
#[derive(Clone, Debug)]
pub enum Key {
    /// Literal bytes, exactly as a terminal would deliver them.
    Bytes(Vec<u8>),
    /// Let the app settle (or animate) for a while before the next key.
    Wait(Duration),
}

/// The default foreground this harness answers **OSC 10** with.
///
/// A capture only carries the app→terminal direction, so the reply we type back
/// is not in the bytes the oracle resolves. That makes these two constants the
/// only record of what terminal the app thought it was talking to, and
/// `oracle::resolve` has to be handed them or its picture is of a different
/// terminal than the one lanthorn was answering (SQ-0929).
pub const ANSWERED_FG: [u8; 3] = [0xC0, 0xC0, 0xC0];

/// The default background this harness answers **OSC 11** with.
///
/// It matches the emulator palette's own entry 0, which is what an unpainted cell
/// is filled with — and that agreement is the whole point. While it did not
/// agree, the harness told lanthorn "my background is #000000", lanthorn dutifully
/// painted its chrome cells #000000, and the rasteriser drew every unpainted cell
/// #1D1F21. The result was a black rectangle floating on grey in every v6 capture:
/// a defect that existed nowhere but in the instrument, and that cost a long
/// investigation of the renderer before anyone read this file.
pub const ANSWERED_BG: [u8; 3] = [0x1D, 0x1F, 0x21];

/// An `OSC <n>;rgb:rrrr/gggg/bbbb` reply, in the doubled-hex form terminals use.
fn osc_colour_reply(osc: u8, [r, g, b]: [u8; 3]) -> String {
    format!("\x1b]{osc};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")
}

impl Key {
    /// Parse the ad-hoc key spelling used by the example's `--keys`:
    /// `cr esc tab space bs up down left right home end pgup pgdn f1..f4`,
    /// `wait:MS`, `ctrl+X`, and `text:...` for a literal string.
    pub fn parse(token: &str) -> Result<Key, String> {
        let t = token.trim();
        if let Some(ms) = t.strip_prefix("wait:") {
            let ms: u64 = ms.parse().map_err(|_| format!("bad wait `{t}`"))?;
            return Ok(Key::Wait(Duration::from_millis(ms)));
        }
        if let Some(s) = t.strip_prefix("text:") {
            return Ok(Key::Bytes(s.as_bytes().to_vec()));
        }
        if let Some(c) = t.strip_prefix("ctrl+") {
            let ch = c.chars().next().ok_or_else(|| format!("bad ctrl key `{t}`"))?;
            let b = (ch.to_ascii_uppercase() as u8).wrapping_sub(0x40);
            return Ok(Key::Bytes(vec![b]));
        }
        let bytes: &[u8] = match t.to_ascii_lowercase().as_str() {
            "cr" | "enter" | "return" => b"\r",
            "lf" => b"\n",
            "esc" => b"\x1b",
            "tab" => b"\t",
            "space" | "sp" => b" ",
            "bs" => b"\x7f",
            "up" => b"\x1b[A",
            "down" => b"\x1b[B",
            "right" => b"\x1b[C",
            "left" => b"\x1b[D",
            "home" => b"\x1b[H",
            "end" => b"\x1b[F",
            "pgup" => b"\x1b[5~",
            "pgdn" => b"\x1b[6~",
            "f1" => b"\x1bOP",
            "f2" => b"\x1bOQ",
            "f3" => b"\x1bOR",
            "f4" => b"\x1bOS",
            other => {
                if other.chars().count() == 1 {
                    return Ok(Key::Bytes(t.as_bytes().to_vec()));
                }
                return Err(format!("unknown key `{t}`"));
            }
        };
        Ok(Key::Bytes(bytes.to_vec()))
    }
}

/// What to run and how to drive it.
#[derive(Clone, Debug)]
pub struct Spec {
    pub bin: PathBuf,
    pub story: PathBuf,
    /// A throwaway `--user-dir`, so the run never reads or writes the player's
    /// real `~/.lanthorn` (and never resumes their save into the capture).
    pub user_dir: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// The cell size we report for `CSI 16 t`. Not cosmetic: v6 art is scaled by
    /// pixel and placed by cell, so the whole geometry hangs off this.
    pub cell_w: u16,
    pub cell_h: u16,
    /// Start with the map pane hidden (writes the per-game `show_map = false`
    /// sidecar), which is what gives the story pane the full frame width.
    pub hide_map: bool,
    pub keys: Vec<Key>,
    /// Silence that counts as "the app has finished drawing".
    pub quiet: Duration,
    /// Wait for this much silence after the last key before giving up.
    pub tail: Duration,
    /// Hard ceiling on the whole run.
    pub timeout: Duration,
    pub extra_args: Vec<String>,
    /// Queries (by [`Responder`] name) to answer LATE rather than at once, by
    /// [`Spec::defer_by`]. A real terminal answers when it gets round to it —
    /// after a screenful of graphics, after an alternate-screen switch — and an
    /// app that stops listening before the answer arrives leaves those bytes in
    /// the tty for whoever reads next. Deferring turns that race into a fact
    /// (SQ-0769).
    ///
    /// The lateness applies to the whole BATCH: once a deferred query is seen in
    /// a burst, every reply from that one onwards is held back together. A busy
    /// terminal goes quiet for all of a probe's questions, not for one of them —
    /// and a probe that ends in a DSR only stops listening when the DSR answer
    /// arrives, so answering that on time while holding the rest back would be a
    /// terminal no one has.
    pub defer_queries: Vec<&'static str>,
    /// How late [`Spec::defer_queries`] are answered.
    pub defer_by: Duration,
    /// Answer the kitty capability query (default `true`).
    ///
    /// Turning it off is a DIFFERENT TERMINAL, not a broken one: `ratatui-image`
    /// treats half-blocks as its universal fallback, so lanthorn still runs the
    /// whole v6 pixel path and simply resolves it into `▀` cells. That is a real
    /// thing lanthorn does on a real terminal without graphics — and it is the
    /// only Version 6 output an asciinema cast can carry, because the player
    /// renders cells and SGR and drops kitty's APC escapes on the floor
    /// (SQ-0943).
    ///
    /// Every measuring caller wants this left alone. [`Negotiation`] still
    /// reports what happened, so a capture that turned it off cannot be mistaken
    /// for one that tried for kitty and lost.
    pub answer_kitty: bool,
    /// Run this exact argument list instead of lanthorn's own
    /// `<story> --user-dir … --no-sound`.
    ///
    /// For the CLI clients (`zvm-cli`, `gvm-cli`, `scott-cli`), which take
    /// neither a `--user-dir` nor a map pane, and one of which — `zvm-cli
    /// --machines` — takes no story at all. When this is set, [`Spec::story`] is
    /// only a label and [`Spec::hide_map`] is not acted on, because there is no
    /// per-game sidecar to write for a program that has no per-game anything.
    pub argv: Option<Vec<String>>,
}

impl Spec {
    pub fn new(bin: impl Into<PathBuf>, story: impl Into<PathBuf>, user_dir: impl Into<PathBuf>) -> Spec {
        Spec {
            bin: bin.into(),
            story: story.into(),
            user_dir: user_dir.into(),
            cols: 117,
            rows: 64,
            cell_w: 8,
            cell_h: 18,
            hide_map: true,
            keys: Vec::new(),
            quiet: Duration::from_millis(120),
            tail: Duration::from_millis(900),
            timeout: Duration::from_secs(45),
            extra_args: Vec::new(),
            defer_queries: Vec::new(),
            defer_by: Duration::ZERO,
            answer_kitty: true,
            argv: None,
        }
    }
}

/// One burst of output: bytes that arrived without a [`Spec::quiet`] gap between
/// them. lanthorn writes a frame in one go, so a burst is a frame in practice —
/// and grouping by silence needs no cooperation from the app.
///
/// The gap is measured between READS, not between "the last thing that happened".
/// Sending a keystroke used to reset the same clock the grouping read, so the
/// app's reply — which arrives a few milliseconds after the key — always looked
/// like a continuation of whatever burst came before it, however many seconds
/// earlier that was. Every run therefore collapsed into ONE flush at `at: 0`.
/// Invisible to the decoder, which only wants the grouping for attribution, and
/// fatal to the cast recorder (SQ-0943), for which these timestamps ARE the
/// recording.
#[derive(Clone, Debug)]
pub struct Flush {
    pub at: Duration,
    pub offset: usize,
    pub len: usize,
}

/// Which terminal query was asked, and what we answered.
#[derive(Clone, Debug)]
pub struct Answered {
    pub query: &'static str,
    pub sent: String,
    pub at: Duration,
}

/// Everything the run produced.
pub struct Capture {
    pub bytes: Vec<u8>,
    pub flushes: Vec<Flush>,
    pub answered: Vec<Answered>,
    pub spec: Spec,
    pub duration: Duration,
    pub timed_out: bool,
}

impl Capture {
    /// The graphics protocol actually in force, decided from the stream rather
    /// than from what we hoped: kitty is proven by APC `_G` traffic, and nothing
    /// else proves it.
    pub fn negotiated(&self) -> Negotiation {
        let answered_kitty = self.answered.iter().any(|a| a.query == "kitty graphics support");
        let apc = count_subslices(&self.bytes, b"\x1b_G");
        Negotiation { answered_kitty, apc_commands: apc }
    }
}

/// The protocol verdict. `is_kitty()` is the gate every caller must pass before
/// believing anything else in the capture.
#[derive(Clone, Copy, Debug)]
pub struct Negotiation {
    pub answered_kitty: bool,
    pub apc_commands: usize,
}

impl Negotiation {
    pub fn is_kitty(&self) -> bool {
        self.answered_kitty && self.apc_commands > 0
    }

    pub fn explain(&self) -> String {
        match (self.answered_kitty, self.apc_commands) {
            (false, _) => "NOT KITTY — lanthorn never asked the kitty capability query (or asked it in \
                 a spelling this harness does not answer), so it detected half-blocks"
                .to_string(),
            (true, 0) => "NOT KITTY — the harness answered the capability query, but lanthorn emitted \
                 no APC graphics at all: it fell back to a cell renderer, so this capture measures \
                 the wrong backend"
                .to_string(),
            (true, n) => format!(
                "KITTY — the harness answered the kitty capability query and lanthorn emitted \
                 {n} APC `_G` graphics command(s)"
            ),
        }
    }
}

// ── The pty ───────────────────────────────────────────────────────────────────

struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

fn open_pty(spec: &Spec) -> std::io::Result<Pty> {
    // SAFETY: plain POSIX pty setup; every returned fd is checked and wrapped in
    // an OwnedFd immediately, and `ptsname`'s buffer is copied before any other
    // libc call can reuse it.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if master < 0 {
            return Err(errno());
        }
        let master = OwnedFd::from_raw_fd(master);
        if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
            return Err(errno());
        }
        let name = libc::ptsname(master.as_raw_fd());
        if name.is_null() {
            return Err(errno());
        }
        let name = CStr::from_ptr(name).to_owned();
        let slave = libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if slave < 0 {
            return Err(errno());
        }
        let slave = OwnedFd::from_raw_fd(slave);

        // Size the terminal, in cells AND in pixels: a v6 title asks the host how
        // big the screen is, and a pixel size of zero is not a size a real
        // terminal ever reports.
        let ws = libc::winsize {
            ws_row: spec.rows,
            ws_col: spec.cols,
            ws_xpixel: spec.cols.saturating_mul(spec.cell_w),
            ws_ypixel: spec.rows.saturating_mul(spec.cell_h),
        };
        if libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &ws) != 0 {
            return Err(errno());
        }

        // Local echo off before the child starts: the app turns raw mode on a few
        // milliseconds in, and anything we type before then would otherwise come
        // straight back and pollute the capture with our own keystrokes.
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut tio) == 0 {
            tio.c_lflag &= !(libc::ECHO | libc::ECHONL);
            let _ = libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &tio);
        }
        Ok(Pty { master, slave })
    }
}

fn spawn(spec: &Spec, pty: &Pty) -> std::io::Result<Child> {
    let slave_fd: RawFd = pty.slave.as_raw_fd();
    let mut cmd = Command::new(&spec.bin);
    match &spec.argv {
        // A CLI client's whole command line, verbatim: it has no `--user-dir`
        // and no map pane to hide, so nothing of lanthorn's shape applies.
        Some(argv) => {
            cmd.args(argv);
        }
        None => {
            cmd.arg(&spec.story)
                .arg("--user-dir")
                .arg(&spec.user_dir)
                .arg("--no-sound")
                .args(&spec.extra_args);
        }
    }
    cmd.stdin(Stdio::from(pty.slave.try_clone()?))
        .stdout(Stdio::from(pty.slave.try_clone()?))
        .stderr(Stdio::from(pty.slave.try_clone()?));
    // A kitty-shaped environment, with every "you are actually something else"
    // hint cleared: ratatui-image trusts the IO probe first, but TMUX/WezTerm/
    // Konsole hints would blacklist the very protocol we are here to exercise.
    cmd.env("TERM", "xterm-kitty");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("KITTY_WINDOW_ID", "1");
    for stale in ["TMUX", "TMUX_PANE", "WEZTERM_EXECUTABLE", "KONSOLE_VERSION", "TERM_PROGRAM", "LC_TERMINAL"] {
        cmd.env_remove(stale);
    }
    unsafe {
        cmd.pre_exec(move || {
            // Own a session and make the pty its controlling terminal, or
            // crossterm's /dev/tty open (and therefore raw mode) fails.
            if libc::setsid() < 0 {
                return Err(errno());
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(errno());
            }
            Ok(())
        });
    }
    cmd.spawn()
}

// ── Answering the terminal queries ────────────────────────────────────────────

/// The queries we answer, in the exact spelling ratatui-image and lanthorn send.
/// A query we do NOT answer is not a silent loss: the app's probes all end in a
/// DSR, so an unanswered one costs a timeout and a wrong backend — which is what
/// [`Negotiation`] exists to catch.
struct Responder {
    /// Whether to answer the kitty capability query at all — see
    /// [`Spec::answer_kitty`].
    answer_kitty: bool,
    cell_w: u16,
    cell_h: u16,
    cols: u16,
    rows: u16,
    scanned: usize,
    /// Byte offsets already answered, so the overlapping re-scan below never
    /// types a reply twice — a duplicate reply arrives as phantom keystrokes.
    answered_at: std::collections::HashSet<usize>,
}

impl Responder {
    fn replies(&self) -> Vec<(&'static str, &'static [u8], String)> {
        let (w, h) = (self.cell_w, self.cell_h);
        let mut v = vec![
            // Kitty's own DA1: no `4`, because kitty does not do sixel. Claiming
            // sixel here would let a fallback path look like a success.
            ("primary device attributes", b"\x1b[c".as_slice(), "\x1b[?62;c".to_string()),
            ("cell size in pixels", b"\x1b[16t".as_slice(), format!("\x1b[6;{h};{w}t")),
            (
                "window size in pixels",
                b"\x1b[14t".as_slice(),
                format!("\x1b[4;{};{}t", self.rows * h, self.cols * w),
            ),
            ("window size in cells", b"\x1b[18t".as_slice(), format!("\x1b[8;{};{}t", self.rows, self.cols)),
            (
                "default foreground (OSC 10)",
                b"\x1b]10;?\x07".as_slice(),
                osc_colour_reply(10, ANSWERED_FG),
            ),
            (
                "default background (OSC 11)",
                b"\x1b]11;?\x07".as_slice(),
                osc_colour_reply(11, ANSWERED_BG),
            ),
            ("cursor position report", b"\x1b[6n".as_slice(), "\x1b[1;1R".to_string()),
            ("keyboard protocol flags", b"\x1b[?u".as_slice(), "\x1b[?0u".to_string()),
            // Last, because every probe ends with it and the reply is the app's
            // signal that the whole batch is answered.
            ("device status report", b"\x1b[5n".as_slice(), "\x1b[0n".to_string()),
        ];
        if self.answer_kitty {
            // First, because it is the first thing the app asks and a burst's
            // replies go back in the order its questions arrived.
            v.insert(
                0,
                (
                    "kitty graphics support",
                    b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\".as_slice(),
                    "\x1b_Gi=31;OK\x1b\\".to_string(),
                ),
            );
        }
        v
    }

    /// Scan everything captured so far for queries we have not yet answered, and
    /// return the replies to type back, in the order the app asked.
    ///
    /// The scan restarts a little BEHIND the last one and remembers which offsets
    /// it has already answered: a query is a handful of bytes and a read can land
    /// in the middle of one, and a query missed because it straddled a 64 KiB
    /// boundary is a wrong backend measured in silence.
    fn scan(&mut self, all: &[u8]) -> Vec<(&'static str, String)> {
        const OVERLAP: usize = 64;
        let from = self.scanned.saturating_sub(OVERLAP);
        let mut hits: Vec<(usize, &'static str, String)> = Vec::new();
        for (name, pat, reply) in self.replies() {
            let mut i = from;
            while let Some(off) = find_subslice(&all[i..], pat) {
                let at = i + off;
                if self.answered_at.insert(at) {
                    hits.push((at, name, reply.clone()));
                }
                i = at + pat.len();
            }
        }
        self.scanned = all.len();
        hits.sort_by_key(|(off, _, _)| *off);
        hits.into_iter().map(|(_, n, r)| (n, r)).collect()
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn count_subslices(hay: &[u8], needle: &[u8]) -> usize {
    let (mut i, mut n) = (0usize, 0usize);
    while let Some(off) = find_subslice(&hay[i..], needle) {
        n += 1;
        i += off + needle.len();
    }
    n
}

// ── The run ───────────────────────────────────────────────────────────────────

/// Boot lanthorn under a pty, drive `spec.keys`, and return every byte it wrote.
pub fn run(spec: Spec) -> std::io::Result<Capture> {
    std::fs::create_dir_all(&spec.user_dir)?;
    if spec.hide_map && spec.argv.is_none() {
        // The story pane only gets the whole frame when the map is hidden, and
        // the per-game sidecar sets that BEFORE the first frame — toggling it with
        // a keystroke would put the transition in the capture we are measuring.
        let game_dir = app::storage::game_dir(
            &spec.user_dir.join("saves"),
            &app::storage::story_key_at(&spec.story),
        );
        std::fs::create_dir_all(&game_dir)?;
        std::fs::write(game_dir.join("config.toml"), "show_map = false\n")?;
    }

    let pty = open_pty(&spec)?;
    let mut child = spawn(&spec, &pty)?;
    // The parent must not hold the slave open: with it open, reads on the master
    // block for ever instead of ending when the app exits.
    drop(pty.slave);
    let master = pty.master;

    let start = Instant::now();
    let mut bytes: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut flushes: Vec<Flush> = Vec::new();
    let mut answered: Vec<Answered> = Vec::new();
    let mut responder =
        Responder {
            answer_kitty: spec.answer_kitty,
            cell_w: spec.cell_w,
            cell_h: spec.cell_h,
            cols: spec.cols,
            rows: spec.rows,
            scanned: 0,
            answered_at: std::collections::HashSet::new(),
        };
    let mut keys = spec.keys.clone().into_iter();
    let mut pending_wait: Option<Duration> = None;
    let mut last_byte = Instant::now();
    // Read timing, kept apart from the key pacing above: `last_byte` moves when
    // we TYPE, which is what "has the app gone quiet enough for the next key"
    // wants and what burst grouping must not see. See [`Flush`].
    let mut last_read = Instant::now();
    let mut file = unsafe { std::fs::File::from_raw_fd(libc::dup(master.as_raw_fd())) };
    let mut timed_out = false;
    // Replies held back by `spec.defer_queries`, each with the instant it is due.
    let mut deferred: Vec<(Instant, &'static str, String)> = Vec::new();

    loop {
        if start.elapsed() > spec.timeout {
            timed_out = true;
            break;
        }
        // Late answers first, so a deferred reply lands on time even while the app
        // is quiet and the loop is only ticking the poll timeout.
        let now = Instant::now();
        let mut still_pending = Vec::new();
        for (due, name, reply) in deferred.drain(..) {
            if due <= now {
                let _ = write_all(master.as_raw_fd(), reply.as_bytes());
                answered.push(Answered { query: name, sent: reply, at: start.elapsed() });
            } else {
                still_pending.push((due, name, reply));
            }
        }
        deferred = still_pending;
        match poll_readable(master.as_raw_fd(), Duration::from_millis(20)) {
            Ok(true) => {
                let mut chunk = [0u8; 65536];
                match file.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let now = Instant::now();
                        let gap = now.duration_since(last_read);
                        last_read = now;
                        last_byte = now;
                        let offset = bytes.len();
                        match flushes.last_mut() {
                            Some(f) if gap < spec.quiet => f.len += n,
                            _ => flushes.push(Flush { at: start.elapsed(), offset, len: n }),
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        let mut batch_late = false;
                        for (name, reply) in responder.scan(&bytes) {
                            batch_late |= spec.defer_queries.contains(&name);
                            if batch_late {
                                deferred.push((Instant::now() + spec.defer_by, name, reply));
                                continue;
                            }
                            let _ = write_all(master.as_raw_fd(), reply.as_bytes());
                            answered.push(Answered { query: name, sent: reply, at: start.elapsed() });
                        }
                    }
                    // EIO on Linux is how a pty master reports "the child is gone".
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(false) => {
                let quiet_for = last_byte.elapsed();
                if let Some(w) = pending_wait {
                    if quiet_for >= w {
                        pending_wait = None;
                    }
                    continue;
                }
                if quiet_for < spec.quiet {
                    continue;
                }
                match keys.next() {
                    Some(Key::Bytes(b)) => {
                        write_all(master.as_raw_fd(), &b)?;
                        last_byte = Instant::now();
                    }
                    Some(Key::Wait(d)) => pending_wait = Some(d),
                    // Keys exhausted: give the app a longer silence to finish
                    // whatever the last one started, then stop — but never while a
                    // deferred reply is still owed, or the run would end before the
                    // very lateness it is measuring.
                    None if quiet_for >= spec.tail && deferred.is_empty() => break,
                    None => {}
                }
            }
            Err(e) => return Err(e),
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(Capture { bytes, flushes, answered, spec, duration: start.elapsed(), timed_out })
}

fn poll_readable(fd: RawFd, timeout: Duration) -> std::io::Result<bool> {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one initialised pollfd, count 1, as poll(2) requires.
    let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as libc::c_int) };
    if n < 0 {
        let e = errno();
        if e.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    Ok(n > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}

fn write_all(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    // SAFETY: `fd` stays owned by the caller — dup it so dropping this File does
    // not close the master out from under the run.
    let mut f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
    f.write_all(data)?;
    f.flush()
}

/// Find `target/<profile>/lanthorn` from a binary that cargo built beside it —
/// the trick that lets `cargo run --example` reach the real app without the
/// `CARGO_BIN_EXE_*` that only integration tests get.
pub fn sibling_lanthorn() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LANTHORN_BIN") {
        return Some(PathBuf::from(p));
    }
    sibling_binary("lanthorn")
}

/// Any workspace binary cargo built beside this one — `zvm-cli`, `gvm-cli`,
/// `scott-cli` (SQ-0943). Same trick as [`sibling_lanthorn`], which is now one
/// call of it.
pub fn sibling_binary(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), dir.parent()?.join(name)].into_iter().find(|cand| cand.is_file())
}

/// The repo's `stories/` directory, from this crate's manifest.
pub fn stories_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}
