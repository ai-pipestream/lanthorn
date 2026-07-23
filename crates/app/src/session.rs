// GameSession — drives one VM turn, captures transcript output, mapper bridge.
//
// Transcript capture approach: we use a custom `CaptureSink` (rather than
// reusing `zvm::io::BufferOutput`) because `BufferOutput` has no drain/clear
// method.  `CaptureSink` implements `zvm::io::Output` and exposes `take_text`
// to drain accumulated text between turns.  After construction the sink is
// accessed by downcasting `machine.out` via the `as_any()` supertrait —
// `machine.out` is `pub`, so no zvm visibility change is required.
//
// zvm change made for this module: added Output::as_any_mut (+ BufferOutput/StdoutOutput impls) to allow mutable downcast to CaptureSink.

use std::any::Any;

use mapper::direction::parse_direction;
use mapper::mapper::Mapper;
use zvm::cpu::exec::{Machine, PictureEvent, SoundEvent, StepResult};
use zvm::error::ZError;
use zvm::io::{Output, TextAttrs};
use zvm::location::{detect_location, Location, LocationMethod};
use zvm::screen::ZColour;
use zvm::ObjectSnapshot;

use crate::state::ParaFmt;
use zvm::memory::Memory;

// ── InputKind ─────────────────────────────────────────────────────────────────

/// Which kind of input the VM is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Waiting for a full line of text (`read` / `sread` opcode).
    Line,
    /// Waiting for a single keypress (`read_char` opcode).
    Char,
    /// Waiting on a non-input Glk event only — a timer, mouse, or hyperlink
    /// (`glk_select` with no line/char request; Glulx, Glk §4.4). The game
    /// requested no typed input, so the host shows no prompt/cursor and delivers
    /// the event (a timer tick on its clock, a mouse/hyperlink event on a click)
    /// rather than a keystroke. Never produced by the Z-machine engine.
    Event,
}

/// Which in-game (game-initiated) I/O the VM is suspended on after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingIo {
    Save,
    Restore,
}

/// A game-initiated Glk `create_by_prompt` awaiting a host-supplied filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilenameReq {
    /// Glk fileusage.
    pub usage: u32,
    /// Glk filemode (Read `0x02`, Write `0x01`, ReadWrite `0x03`, WriteAppend `0x05`).
    pub fmode: u32,
}

// ── CaptureSink ───────────────────────────────────────────────────────────────

/// An output sink that accumulates printed text and lets the caller drain it.
///
/// `runs` records one `(char_count, text_style_bits, fg, bg, link, para)` chunk
/// per `print`/`print_styled`/`print_attr` call, in lockstep with the appended
/// text, so callers can reconstruct which spans carried Z-machine emphasis and
/// colour. `link` is the Glk hyperlink value (always 0 on the Z-machine path);
/// `para` is the paragraph layout format (always [`ParaFmt::default`] on the
/// Z-machine path — the Glulx buffer path is the only source of non-default
/// layout, carried via [`crate::glk_backend::AppGlk::take_transcript_elems`]).
/// One captured `(char_count, text_style_bits, fg, bg, link, para, ...)` chunk.
type CaptureRun = (usize, u8, ZColour, ZColour, u32, ParaFmt, u8);

pub struct CaptureSink {
    pub text: String,
    pub runs: Vec<CaptureRun>,
}

impl CaptureSink {
    fn new() -> Self {
        CaptureSink { text: String::new(), runs: Vec::new() }
    }

    /// Drain accumulated text and style runs together, leaving both empty.
    pub fn take_styled(&mut self) -> (String, Vec<CaptureRun>) {
        (std::mem::take(&mut self.text), std::mem::take(&mut self.runs))
    }

    /// Drain all accumulated text, leaving the buffer empty.
    pub fn take_text(&mut self) -> String {
        self.take_styled().0
    }
}

impl Output for CaptureSink {
    fn print(&mut self, s: &str) {
        self.runs.push((s.chars().count(), 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0));
        self.text.push_str(s);
    }
    fn print_styled(&mut self, s: &str, style: u8) {
        self.runs.push((s.chars().count(), style, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0));
        self.text.push_str(s);
    }
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.runs.push((s.chars().count(), attrs.style, attrs.fg, attrs.bg, 0, ParaFmt::default(), 0));
        self.text.push_str(s);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Trim a `(char_count, bits, fg, bg)` chunk list so its total char-count equals
/// `char_len` (used after `strip_read_prompt` shortens the captured text by a
/// trailing prompt). Chunks past the limit are dropped; the boundary chunk is
/// truncated. A list shorter than `char_len` is returned unchanged (the missing
/// tail is treated as plain by `push_transcript_runs`).
pub(crate) fn clamp_runs(
    runs: Vec<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)>,
    char_len: usize,
) -> Vec<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)> {
    let mut out = Vec::with_capacity(runs.len());
    let mut total = 0usize;
    for (c, b, fg, bg, link, para, gs) in runs {
        if total >= char_len {
            break;
        }
        let take = c.min(char_len - total);
        out.push((take, b, fg, bg, link, para, gs));
        total += take;
    }
    out
}

/// Interleave v6 window-0 inline pictures into a turn's styled text as ordered
/// [`TranscriptElem`]s. Each picture carries the absolute win0 output-char
/// offset it was drawn at (`PictureEvent::out_chars`); `base` is the count at
/// the start of this turn's text, so `abs - base` is the picture's position
/// within `text`. Offsets snap DOWN to the start of their line (v6 games draw
/// inline art at the text cursor, i.e. at line starts — snapping keeps a
/// mid-line offset from splitting a paragraph in two, since
/// `push_transcript_runs` starts a new transcript line per `Text` element).
/// The line separator consumed by a split is dropped from the emitted text
/// (the element boundary itself is the break) and its style-chunk char is
/// consumed in lockstep.
fn interleave_story_pics(
    text: &str,
    runs: &[(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)],
    pics: Vec<(u64, crate::inline_image::InlineImage)>,
    base: u64,
) -> Vec<TranscriptElem> {
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    // Clamp into this turn's text, then snap to the owning line's start.
    let mut inserts: Vec<(usize, crate::inline_image::InlineImage)> = pics
        .into_iter()
        .map(|(abs, img)| {
            let mut off = (abs.saturating_sub(base) as usize).min(total);
            while off > 0 && chars[off - 1] != '\n' {
                off -= 1;
            }
            (off, img)
        })
        .collect();
    inserts.sort_by_key(|(o, _)| *o); // stable: equal offsets keep draw order

    // Lockstep style-chunk consumption: `take(n)` returns the chunks covering
    // the next `n` chars, splitting the boundary chunk as needed.
    let mut run_iter = runs.iter().copied();
    let mut pending: Option<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)> = run_iter.next();
    let mut take = |n: usize| -> Vec<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)> {
        let mut out = Vec::new();
        let mut left = n;
        while left > 0 {
            match pending {
                Some(mut r) => {
                    if r.0 <= left {
                        left -= r.0;
                        out.push(r);
                        pending = run_iter.next();
                    } else {
                        let mut head = r;
                        head.0 = left;
                        out.push(head);
                        r.0 -= left;
                        left = 0;
                        pending = Some(r);
                    }
                }
                None => break,
            }
        }
        out
    };

    let mut elems = Vec::new();
    let mut pos = 0usize;
    for (off, img) in inserts {
        if off > pos {
            // Text up to the split, excluding the '\n' the split lands after —
            // the element boundary IS the line break.
            let end = off - 1; // chars[off-1] == '\n' (or off == total edge below)
            let (chunk_end, drop_sep) = if chars[off - 1] == '\n' { (end, true) } else { (off, false) };
            if chunk_end > pos {
                let chunk: String = chars[pos..chunk_end].iter().collect();
                let chunk_runs = take(chunk_end - pos);
                elems.push(TranscriptElem::Text { text: chunk, runs: chunk_runs });
            }
            if drop_sep {
                let _ = take(1); // consume the dropped separator's style char
            }
            pos = off;
        }
        elems.push(TranscriptElem::Image(img));
    }
    if pos < total {
        let tail: String = chars[pos..].iter().collect();
        let tail_runs = take(total - pos);
        elems.push(TranscriptElem::Text { text: tail, runs: tail_runs });
    }
    elems
}

// ── Public types ──────────────────────────────────────────────────────────────

/// One ordered piece of a turn's buffer output: a text run (with its style
/// chunks) or an inline image. Preserves emission order so images land between
/// the right lines.
pub enum TranscriptElem {
    Text { text: String, runs: Vec<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)> },
    Image(crate::inline_image::InlineImage),
}

/// Trim trailing `Text` elements of `elems` so the total char-count of their
/// text equals `keep` — the element-list counterpart to `strip_read_prompt`
/// shortening the flat text by a trailing read prompt. Walks from the end,
/// clearing whole `Text` elements and truncating the boundary one (its `text`
/// AND its `runs`, via `clamp_runs`) so the concatenation of element text stays
/// exactly equal to the stripped flat `raw`. `Image` elements carry no text, so
/// a strip that reaches across one still lands on the preceding text.
pub(crate) fn trim_elems_to_len(elems: &mut [TranscriptElem], keep: usize) {
    let total: usize = elems
        .iter()
        .map(|e| match e {
            TranscriptElem::Text { text, .. } => text.chars().count(),
            TranscriptElem::Image(_) => 0,
        })
        .sum();
    if total <= keep {
        return;
    }
    let mut remove = total - keep;
    for e in elems.iter_mut().rev() {
        if remove == 0 {
            break;
        }
        if let TranscriptElem::Text { text, runs } = e {
            let n = text.chars().count();
            if n <= remove {
                remove -= n;
                text.clear();
                runs.clear();
            } else {
                let keep_here = n - remove;
                let byte = text
                    .char_indices()
                    .nth(keep_here)
                    .map(|(i, _)| i)
                    .unwrap_or(text.len());
                text.truncate(byte);
                *runs = clamp_runs(std::mem::take(runs), keep_here);
                remove = 0;
            }
        }
    }
}

/// One buffered Glk sound-channel operation, emitted by `AppGlk` during a turn
/// and drained into `TurnResult.glulx_sound_ops` for `AppState` to play. Channel
/// *state* (refs, rocks, volume) lives in `AppGlk`; only the playback-affecting
/// operations travel here. `Play.volume` snapshots the channel's current Glk
/// volume so the player (which cannot see `AppGlk`) can compute gain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchannelOp {
    /// `paused` snapshots the channel's pause state (Glk 0.7.3 §8.3): a sound
    /// played on a channel paused while empty must start paused, and release
    /// only on `unpause`.
    Play { chan: u32, snd: u32, repeats: u32, notify: u32, volume: u32, paused: bool },
    Stop { chan: u32 },
    SetVolume { chan: u32, vol: u32 },
    Destroy { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_pause` — pause playback, keeping position.
    Pause { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_unpause` — resume a paused channel.
    Unpause { chan: u32 },
    /// Glk 0.7.3 Sound2 `glk_schannel_set_volume_ext` — a volume change ramped
    /// over `duration_ms` (0 = immediate). When `notify != 0` the host fires an
    /// `evtype_VolumeNotify(val2 = notify)` once the ramp completes.
    SetVolumeExt { chan: u32, vol: u32, duration_ms: u32, notify: u32 },
}

/// Result of one player turn.
pub struct TurnResult {
    pub transcript: String,
    /// Text-style chunks for `transcript`: a `(char_count, bits, fg, bg, link, para)`
    /// list covering every char of `transcript`, fed to `push_transcript_runs`. All
    /// chunks carry bits 0, default colours and default `para` when the turn emitted
    /// no styling (the Z-machine path never sets a non-default `para`).
    pub transcript_runs: Vec<(usize, u8, ZColour, ZColour, u32, ParaFmt, u8)>,
    pub location: Option<ObjectSnapshot>,
    pub quit: bool,
    /// The game cleared the screen this turn — a Z-machine `erase_window`
    /// (lower / all, ZMSD §8.7.3) or a Glulx `glk_window_clear` on the primary
    /// buffer (e.g. a help-menu takeover / Inform 7 menu redraw). The host pins
    /// this turn's output to a fresh screen (scrollback preserved) so stale text
    /// does not bleed through — matching a retained-mode interpreter like Lectrote.
    pub erase_lower: bool,
    /// Optional one-line note to surface to the player (general-purpose; currently unused — no producer sets it).
    pub info: Option<String>,
    /// Sound events emitted this turn (drained from the VM), in order.
    pub sounds: Vec<SoundEvent>,
    /// Glk sound-channel operations emitted this turn (Glulx only; empty for the
    /// Z-machine, which uses `sounds`). Played by `AppState::play_glulx_sound_ops`.
    pub glulx_sound_ops: Vec<SchannelOp>,
    /// Host-facing diagnostic lines emitted this turn (drained from the VM).
    pub diagnostics: Vec<String>,
    /// How the current room was detected this turn (drives the map indicator).
    pub location_method: Option<LocationMethod>,
    /// Set when the game's own `@save`/`@restore` (any version) suspends the VM for host-mediated file I/O; `None` otherwise.
    pub pending_io: Option<PendingIo>,
    /// Set when this turn came from `abort_timed_input` (the pending read was
    /// completed as timed-out, either directly or because `run_timed_interrupt`'s
    /// routine aborted the read). `false` for every other turn.
    pub timed_out: bool,
    /// Pre-formatted crash stack-trace lines when the VM faulted this turn.
    pub fault: Option<Vec<String>>,
    /// v6 `draw_picture`/`erase_picture` events emitted this turn (drained from
    /// the VM), in order. Empty for v1–5/v7/v8 and for the Glulx path (which
    /// composites its own graphics windows). `GameSession::drain_turn` also
    /// applies each event to `GameSession::pictures_canvas` as it drains them
    /// (mirrors `sounds`, but the Z-machine path additionally rasterizes here
    /// rather than leaving that to the app layer, since a v6 window's canvas
    /// must be self-contained on the session for the Task 4 screen adapter).
    pub pictures: Vec<PictureEvent>,
    /// Ordered buffer output for this turn (text runs + inline images). Empty
    /// for the Z-machine path (no images); the Glulx path fills it and the run
    /// loop pushes from it. When empty, the loop falls back to `transcript` +
    /// `transcript_runs`.
    pub transcript_elems: Vec<TranscriptElem>,
}

/// A running Z-machine game session.
pub struct GameSession {
    pub machine: Machine,
    pub quit: bool,
    /// Which kind of input the VM is currently waiting for.
    pending: InputKind,
    /// When false, the game's own trailing `>` read prompt is kept in the
    /// transcript instead of being stripped. Default true. See
    /// [`Engine::set_strip_prompt`].
    strip_prompt: bool,
    /// Lazily-built, memoized disassembly cache (routine-discovery boundaries).
    /// `RefCell` because the Debugger read-path is `&self`; consistent with the
    /// existing `mem_fault` interior-mutability pattern.
    disasm_cache: std::cell::RefCell<Option<zvm::cpu::disasm_cache::DisasmCache>>,
    /// PC at which the disasm cache was last runtime-confirmed; the per-turn
    /// fold is skipped while the VM is parked at the same PC (nav/scroll calls).
    last_confirmed_pc: std::cell::Cell<Option<u32>>,
    /// v6 Pict resolver (self-blorb/sidecar), set via [`set_pict_source`]
    /// (`None` for non-v6 stories, or when set before construction hasn't
    /// happened yet). Kept on the session — rather than only on `AppState` —
    /// so `drain_turn` can rasterize `pending_pictures` into `pictures_canvas`
    /// without the app layer reaching in. (Plan 1b Task 2)
    ///
    /// [`set_pict_source`]: GameSession::set_pict_source
    pict_source: Option<crate::graphics::PictSource>,
    /// Per-v6-window pixel canvas, keyed by window number (1–7; window 0 is
    /// the main text window and never gets a canvas). Populated by
    /// `drain_turn` from `Machine::pending_pictures`; read by the Task 4
    /// screen adapter to build the layered composite.
    pub pictures_canvas: std::collections::HashMap<u8, crate::graphics::Canvas>,
    /// v6 window-0 inline pictures (drop-caps, room icons) awaiting transcript
    /// interleaving: the absolute win0 output-char offset each was drawn at
    /// (`PictureEvent::out_chars`), plus the prepared float image. Drained into
    /// ordered `TranscriptElem`s so each picture anchors to its paragraph.
    story_pics: Vec<(u64, crate::inline_image::InlineImage)>,
    /// `Machine::v6_win0_out_chars` at the last transcript drain — an event's
    /// offset within the current turn's text is `out_chars - this`.
    v6_win0_chars_seen: u64,
}

// ── GameSession impl ──────────────────────────────────────────────────────────

impl GameSession {
    /// Build a new session from raw story bytes.
    ///
    /// Constructs a `Machine` with a `CaptureSink`, calls `init_caps`, then
    /// steps until the first `NeedLine`/`NeedChar`/`Quit` — this drives the
    /// game's opening text into the sink.  The sink is NOT drained here; the
    /// caller can call `take_transcript` to retrieve the banner/intro text.
    pub fn new(story: Vec<u8>, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>) -> Result<GameSession, ZError> {
        Self::new_with_trace(story, honor_game_colours, sound_available, interpreter_number, false, Vec::new(), None)
    }

    /// Like [`new`](Self::new) but enables execution tracing BEFORE the VM runs to
    /// its first input prompt, so the boot/initialisation code — the whole reason
    /// `--debug` exists (a mid-game `/debug` can never see it) — is captured into
    /// the cumulative coverage set. (SQ-0449)
    ///
    /// `picture_dims` is the v6 Pict dimension table (`(number, width, height)`),
    /// resolved app-side from a self-blorb/sidecar Blorb — empty for non-v6
    /// stories. It MUST be injected before the boot run below: `picture_data` is
    /// called during boot, which happens inside this very function (Phase 0
    /// boot-tracing lesson), so `set_picture_dims` runs right after
    /// `set_sound_available`, before `init_caps()`/the boot loop.
    pub fn new_with_trace(story: Vec<u8>, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>, trace_from_boot: bool, picture_dims: Vec<(u16, u16, u16)>, v6_screen_px: Option<(u16, u16)>) -> Result<GameSession, ZError> {
        let mem = Memory::new(story)?;
        let sink = Box::new(CaptureSink::new());
        let mut machine = Machine::with_output(mem, sink);
        machine.set_honor_game_colours(honor_game_colours);
        machine.set_sound_available(sound_available);
        machine.set_picture_dims(picture_dims);
        machine.set_interpreter_number(interpreter_number);
        machine.init_caps();
        // v6: advertise the game's NATIVE picture resolution (the Blorb `Reso`
        // standard window, default 320×200) as the screen size BEFORE the boot
        // loop below. A v6 game lays out its windows AND its hardcoded pixel art
        // during boot against the reported screen dims; reporting the terminal's
        // cell×8 size instead made the windows stretch while the art stayed at
        // its native coords (SQ-0186). init_caps seeded the v1–5 default; this
        // overrides it for v6 only, before the game can read it.
        if machine.mem.version() == 6 {
            let (w, h) = v6_screen_px.unwrap_or((320, 200));
            let cols = (w / zvm::screen::V6_FONT_WIDTH).clamp(1, 255) as u8;
            let rows = (h / zvm::screen::V6_FONT_HEIGHT).clamp(1, 255) as u8;
            machine.set_screen_dims(rows, cols);
        }
        // Trace from the very first instruction when requested, so the opening
        // run below records boot PCs into `ever_exec_pcs`. Also capture screen
        // opcodes from boot — a v6 game does its whole window/margin/picture
        // layout during boot, so `--trace screen` would otherwise miss it.
        machine.trace_exec = trace_from_boot;
        machine.trace_screen = trace_from_boot;

        let mut quit = false;
        let pending = loop {
            let stop = run_until_input(&mut machine);
            match stop {
                RunStop::Quit => { quit = true; break InputKind::Line; }
                RunStop::Input(k) => break k,
                RunStop::SavePending => machine.complete_save(false),
                RunStop::RestorePending => machine.complete_restore_failure(),
            }
        };

        Ok(GameSession {
            machine, quit, pending, strip_prompt: true,
            disasm_cache: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
        })
    }

    /// Set the v6 Pict resolver used to rasterize `draw_picture`/`erase_picture`
    /// events into `pictures_canvas`. Call once, right after construction —
    /// `drain_turn` reads it on every turn (see the `pict_source` field doc).
    pub fn set_pict_source(&mut self, src: Option<crate::graphics::PictSource>) {
        self.pict_source = src;
    }

    /// Drain and apply any `draw_picture`/`erase_picture` events the VM queued
    /// during boot (`Machine::pending_pictures`, populated inside
    /// `new_with_trace` before this method can ever run). A v6 game like
    /// Zork0 draws its opening art during boot, before the first turn — call
    /// this once, right after `set_pict_source`, so the very first `screen()`
    /// (rendered before the player types anything) already reflects those
    /// boot draws instead of showing a blank graphics window until the first
    /// turn's `drain_turn` happens to pick them up (Plan 1b Task 5 gap).
    pub fn flush_boot_pictures(&mut self) {
        self.drain_pictures();
    }

    /// Encode each rasterized v6 window canvas (`pictures_canvas`) to PNG bytes,
    /// keyed by window number, for Lane P host Save State persistence. Ordered by
    /// each canvas's draw-order stamp (`z_seq`) ASCENDING — the same order the v6
    /// compositor paints them — so `load_pictures_png` can reproduce the relative
    /// z-order (later-drawn windows on top) from the blob order alone, without
    /// storing the raw stamps. Empty for non-v6 stories / before any graphics are
    /// drawn. Pass the result to `archive::save_archive_meta_pics`. PNG is
    /// lossless for RGBA, so a save → restore round-trip reproduces the canvases
    /// byte-for-byte.
    pub fn pictures_png(&self) -> Vec<(u8, Vec<u8>)> {
        let mut keys: Vec<u8> = self.pictures_canvas.keys().copied().collect();
        keys.sort_by_key(|k| (self.pictures_canvas[k].z_seq, *k));
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let canvas = &self.pictures_canvas[&k];
            let mut bytes = Vec::new();
            if image::DynamicImage::ImageRgba8((*canvas.img).clone())
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .is_ok()
            {
                out.push((k, bytes));
            }
        }
        out
    }

    /// Rebuild `pictures_canvas` from persisted per-window PNG blobs
    /// (`archive::ArchiveContents::pictures`) after a host Save State restore, so
    /// a v6 story's graphics windows redraw identically without replaying draw
    /// events (Lane P). Replaces the current canvases. `blobs` are expected in
    /// paint order (as `pictures_png` emits them and the archive preserves);
    /// fresh z-order stamps are assigned sequentially so the ORIGINAL relative
    /// z-order (later-drawn windows on top) is reproduced.
    pub fn load_pictures_png(&mut self, blobs: &[(u8, Vec<u8>)]) {
        self.pictures_canvas.clear();
        for (win, png) in blobs {
            let Ok(img) = image::load_from_memory(png) else { continue };
            let rgba = img.to_rgba8();
            let mut canvas = crate::graphics::Canvas::new(rgba.width(), rgba.height());
            canvas.img = std::sync::Arc::new(rgba);
            canvas.version = canvas.version.wrapping_add(1);
            canvas.z_seq = crate::graphics::next_draw_seq();
            self.pictures_canvas.insert(*win, canvas);
        }
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        let raw = sink_mut(&mut self.machine).take_text();
        // Keep the win0 char-offset base in sync with the drained sink, so any
        // later inline-picture interleave measures against the right origin.
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw }
    }

    /// Whether the game's trailing `>` read prompt is stripped from transcripts.
    pub fn strip_prompt(&self) -> bool {
        self.strip_prompt
    }

    /// Which kind of input the VM is currently waiting for.
    pub fn pending_input(&self) -> InputKind {
        self.pending
    }

    #[cfg(test)]
    fn interpreter_number_for_test(&self) -> u8 {
        self.machine.mem.read_byte(0x1E)
    }

    /// Supply a player command, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.submit_line_with_terminator(command, 13)
    }

    /// Supply a player command terminated by an explicit ZSCII terminator (v5+
    /// terminating-characters table), step until the next input request or Quit,
    /// and return the turn result. `submit` is this with terminator 13 (Enter).
    pub fn submit_line_with_terminator(&mut self, command: &str, terminator: u8) -> TurnResult {
        self.machine.supply_line(command, terminator);
        self.advance_after_input(false)
    }

    /// v5+: does `ch` terminate a line read per the game's terminating-characters
    /// table? Thin wrapper over [`Machine::is_terminator`].
    pub fn is_terminator(&self, ch: u16) -> bool {
        self.machine.is_terminator(ch)
    }

    /// Supply a single keypress, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit_char(&mut self, ch: u8) -> TurnResult {
        self.machine.supply_char(ch);
        self.advance_after_input(false)
    }

    /// While a timed read/read_char is pending, `(time_tenths, packed_routine)`
    /// — the interval to poll for and the interrupt routine to run on timeout.
    /// `None` for an untimed read or when no read is pending.
    pub fn pending_timeout(&self) -> Option<(u16, u16)> {
        self.machine.pending_timeout()
    }

    /// Run the pending read's interrupt routine once. If the routine aborts the
    /// read, completes it via `abort_timed_input` (steps to the next input,
    /// `timed_out == true`); otherwise the read is still pending, and the
    /// returned `TurnResult` carries only the routine's drained output
    /// (`pending`/`quit` unchanged, `timed_out == false`).
    pub fn run_timed_interrupt(&mut self) -> TurnResult {
        let out = self.machine.run_timed_interrupt();
        if out.aborted {
            self.abort_timed_input("")
        } else {
            self.collect_turn()
        }
    }

    /// Run a sampled sound's finish-routine (v5+) to completion and drain any
    /// output it produced. The return value is ignored (ZMSD §9.4 — it does not
    /// abort anything). Does not step a pending read forward.
    pub fn run_sound_finish(&mut self, routine: u16) -> TurnResult {
        self.machine.run_routine(routine);
        self.collect_turn()
    }

    /// Complete the pending read as timed-out: `read_char` delivers ZSCII 0;
    /// `read` writes the partial `typed` line with terminator 0. Steps to the
    /// next input request and returns a `TurnResult` with `timed_out == true`.
    pub fn abort_timed_input(&mut self, typed: &str) -> TurnResult {
        self.machine.abort_timed_input(typed);
        self.advance_after_input(true)
    }

    /// Resume after the host performed an in-game SAVE (`wrote_ok` = file written).
    pub fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.machine.complete_save(wrote_ok);
        let stop = run_until_input(&mut self.machine);
        self.finish_turn(stop)
    }

    /// Resume after the host performed an in-game RESTORE. `Some(bytes)` =
    /// the user picked a save (Quetzal); `None` = cancelled. On corrupt bytes we
    /// fall back to failure so the game sees a clean "Failed.".
    pub fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        match data {
            Some(bytes) => {
                if self.machine.complete_restore_success(bytes).is_err() {
                    self.machine.complete_restore_failure();
                }
            }
            None => self.machine.complete_restore_failure(),
        }
        let stop = run_until_input(&mut self.machine);
        self.finish_turn(stop)
    }

    /// Build the `TurnResult` from a `RunStop` and drain the VM's per-turn
    /// buffers. Shared by submit/submit_char/resume_*.
    fn finish_turn(&mut self, stop: RunStop) -> TurnResult {
        let (quit, pending, pending_io) = match stop {
            RunStop::Quit => (true, InputKind::Line, None),
            RunStop::Input(k) => (false, k, None),
            RunStop::SavePending => (false, self.pending, Some(PendingIo::Save)),
            RunStop::RestorePending => (false, self.pending, Some(PendingIo::Restore)),
        };
        self.quit = quit;
        self.pending = pending;
        self.drain_turn(quit, pending_io, false)
    }

    /// Step the VM to the next input request (or Quit) and build the
    /// `TurnResult` — the shared tail of `submit`/`submit_char`/
    /// `abort_timed_input` once input has been supplied to the VM. `timed_out`
    /// is `true` only for the `abort_timed_input` caller.
    fn advance_after_input(&mut self, timed_out: bool) -> TurnResult {
        let stop = run_until_input(&mut self.machine);
        let mut result = self.finish_turn(stop);
        result.timed_out = timed_out;
        result
    }

    /// Drain the VM's per-turn output into a `TurnResult` without stepping —
    /// used after a timed-interrupt routine ran but did not abort the read: the
    /// read is still pending, so `quit`/`pending` are left as-is and
    /// `timed_out` stays `false`.
    fn collect_turn(&mut self) -> TurnResult {
        self.drain_turn(self.quit, None, false)
    }

    /// Drain the VM's per-turn buffers (transcript, location, diagnostics, sounds,
    /// erase_lower) into a `TurnResult`, given the already-resolved
    /// `quit`/`pending_io`/`timed_out` state. Shared by
    /// `finish_turn` (after stepping to the next input) and `collect_turn`
    /// (mid-read, after a timed-interrupt routine that did not abort).
    fn drain_turn(
        &mut self,
        quit: bool,
        pending_io: Option<PendingIo>,
        timed_out: bool,
    ) -> TurnResult {
        let win0_base = self.v6_win0_chars_seen;
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let transcript_runs = clamp_runs(raw_runs, transcript.chars().count());
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(location_to_snapshot);
        let location_method = detected.as_ref().map(Location::method);

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        let sounds = std::mem::take(&mut self.machine.pending_sounds);
        let erase_lower = std::mem::take(&mut self.machine.screen.erase_lower_requested);
        let pictures = self.drain_pictures();
        // Window-0 inline pictures interleave into this turn's text as ordered
        // elements; empty for turns without them (the app then uses the flat
        // transcript path unchanged).
        let transcript_elems = if self.story_pics.is_empty() {
            Vec::new()
        } else {
            interleave_story_pics(&transcript, &transcript_runs, std::mem::take(&mut self.story_pics), win0_base)
        };

        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit,
            erase_lower,
            info: None,
            sounds,
            glulx_sound_ops: Vec::new(),
            diagnostics,
            fault,
            location_method,
            pending_io,
            timed_out,
            pictures,
            transcript_elems,
        }
    }

    /// Drain `Machine::pending_pictures`, applying each event to
    /// `pictures_canvas` as it's drained (resolving/rasterizing via
    /// `pict_source`), and return the drained events for `TurnResult` — mirrors
    /// `pending_sounds`, except the rasterization happens here rather than in
    /// the app layer (Task 2 decision: canvas store + Pict source both live on
    /// `GameSession` so the Task 4 screen adapter can read `pictures_canvas`
    /// without reaching into `AppState`). A no-op drain for non-v6 stories,
    /// which never push a `PictureEvent`.
    fn drain_pictures(&mut self) -> Vec<PictureEvent> {
        let events = std::mem::take(&mut self.machine.pending_pictures);
        for ev in &events {
            self.apply_picture_event(ev);
        }
        events
    }

    /// Apply one `PictureEvent` to `pictures_canvas`. The event's `(y, x)` are
    /// the spec's 1-based window-relative pixel coords (zero already resolved to
    /// the window cursor by the engine); the canvas is 0-based, so both drop by
    /// one. The canvas is sized to the window's own pixel box and all plotting
    /// is CLIPPED to it — ZMSD §8: "all text and graphics plotting is always
    /// clipped to the current window". (The pre-Rect-support canvas grew to fit
    /// out-of-window draws; those coords were garbage from failed `picture_data`
    /// placement queries, not real game intent.) Erase paints the *picture's
    /// own* footprint (ZMSD §15), falling back to the whole window when the
    /// Pict's dims can't be resolved. Silently no-ops when the story has no v6
    /// window state, the window index is out of range, or (draw only) the
    /// picture fails to resolve.
    fn apply_picture_event(&mut self, ev: &PictureEvent) {
        // number 0 + erase = a v6 `erase_window`'s canvas-clear, riding the
        // ordered picture queue (so "erase, then draw the borders" replays in
        // order). Drop the whole canvas: Shogun's title splash must actually
        // vanish when the game erases window 7 before drawing the menu frame.
        if ev.erase && ev.number == 0 {
            self.pictures_canvas.remove(&ev.window);
            return;
        }
        let Some(v6) = self.machine.screen.v6.as_ref() else { return };
        let Some(w) = v6.windows.get(ev.window as usize) else { return };
        // Window 0 is the main scrolling text window: its pictures are INLINE
        // story content (Zork Zero's drop-caps and room icons, drawn at the
        // text cursor with a margin set for the text to flow beside them).
        // They anchor to the transcript at their output-char position rather
        // than painting a window canvas — the raster/hybrid renderers float
        // them beside the text they belong to, and they scroll with it.
        if ev.window == 0 {
            if ev.erase {
                return; // no canvas to erase; a win0 erase_picture is a no-op here
            }
            if let Some(img) = self.pict_source.as_mut().and_then(|s| s.image(ev.number as u32)) {
                let float = crate::inline_image::InlineImage {
                    pixels: std::sync::Arc::new(img.to_rgba8()),
                    align: crate::inline_image::ImageAlign::MarginLeft,
                    scaled: None,
                    margin_px: ev.margin_after.map(|m| m as u32),
                };
                self.story_pics.push((ev.out_chars, float));
            }
            return;
        }
        // Clamp the pixel-canvas backing store so a hostile / buggy story that
        // sets window_size(w, 0xFFFF, 0xFFFF) then draws/erases can't force a
        // ~17 GB RgbaImage allocation (an OOM abort). CANVAS_PX_CAP (4096) far
        // exceeds any real v6 screen (~640 px) yet bounds worst-case storage to
        // ~64 MB — mirroring the grid-cell cap on the engine side (Phase 1a).
        const CANVAS_PX_CAP: u32 = 4096;
        let (pw, ph) = (
            (w.x_size.max(1) as u32).min(CANVAS_PX_CAP),
            (w.y_size.max(1) as u32).min(CANVAS_PX_CAP),
        );
        // 1-based window-relative → 0-based canvas coords.
        let dx = (ev.x.max(1) as i32) - 1;
        let dy = (ev.y.max(1) as i32) - 1;
        let canvas = self.pictures_canvas.entry(ev.window)
            .or_insert_with(|| crate::graphics::Canvas::new(pw, ph));
        // Track the window's current box without wiping earlier draws: grow
        // preserves content; a shrunken window only tightens the clip below
        // (window_size "does not change the current display", ZMSD §15).
        canvas.grow_to(pw, ph);
        if ev.erase {
            let dims = self.pict_source.as_mut().and_then(|s| s.dims(ev.number as u32));
            let (ew, eh) = dims.unwrap_or((pw, ph));
            // Clip the erase to the window box.
            let ew = ew.min(pw.saturating_sub(dx.max(0) as u32));
            let eh = eh.min(ph.saturating_sub(dy.max(0) as u32));
            canvas.erase_rect(dx, dy, ew, eh);
        } else if let Some(img) = self.pict_source.as_mut().and_then(|s| s.image(ev.number as u32)) {
            canvas.draw_image_clipped(&img, dx, dy, (pw, ph));
            canvas.z_seq = crate::graphics::next_draw_seq();
        }
    }

    /// Build the v6 z-ordered layered [`ScreenModel`] from `screen.v6`'s 8-window
    /// table plus `pictures_canvas` (Plan 1b Task 2). Called from `Engine::screen`
    /// when the story has v6 window state; the v1–5 `screen_model_from_machine`
    /// path is untouched and stays byte-identical for non-v6 stories.
    ///
    /// Per window 0..8, skipped when `x_size == 0 || y_size == 0`: absolute cell
    /// rect = `(x_coord/FW, y_coord/FH, grid.cols, grid.rows)` — the grid was
    /// already cell-sized at `window_size` time (Phase 1a), so only the position
    /// needs dividing by the font cell size. Window 0 is the scrolling main
    /// window (`Buffer{primary:true}`, drawn from `state.transcript`); windows
    /// 1–7 become `Grid` leaves built from their own char grid (mirrors
    /// `screen_model_from_machine`'s `UpperWindow`→`GridWindow` mapping). Any
    /// window with an entry in `pictures_canvas` ALSO gets a `Graphics` leaf at
    /// the same rect. z-order (list order): graphics entries first (background),
    /// then text windows by ascending window number — `render_node`'s `Layered`
    /// arm (Task 3) paints text over graphics, cell-text-wins.
    fn v6_screen_model(&self) -> ScreenModel {
        use zvm::screen::{V6_FONT_HEIGHT, V6_FONT_WIDTH};
        let screen = &self.machine.screen;
        let v6 = screen.v6.as_ref().expect("caller checked screen.v6.is_some()");

        // Z-order: ALL graphics first (background), then ALL text on top — the
        // v6 decorative frame (Zork0's window 7 border) sits BEHIND the page
        // text, never over it. Within each band, ascending window number
        // (window 1+ overlays paint after window 0). The pixel compositor and the
        // Phase 1b cell fallback both honour this order.
        // Graphics carry their global draw-order stamp so the composite can be
        // sorted by DRAW ORDER (later draw on top), not window number — the frame
        // background (drawn first) sits behind the overlays the game paints after
        // it (compass, room illustration). (SQ-0186)
        let mut graphics_entries: Vec<(u64, PositionedWindow)> = Vec::new();
        let mut text_entries = Vec::new();
        for (i, win) in v6.windows.iter().enumerate() {
            if win.x_size == 0 || win.y_size == 0 {
                continue;
            }
            // ZMSD §8.8.1: window coords are 1-based ((1,1) = screen top-left);
            // the composite raster is 0-based, so positions drop by one here.
            let x_px = win.x_coord.saturating_sub(1);
            let y_px = win.y_coord.saturating_sub(1);
            let x = x_px / V6_FONT_WIDTH;
            let y = y_px / V6_FONT_HEIGHT;
            let (cols, rows) = (win.grid.cols, win.grid.rows);

            if let Some(canvas) = self.pictures_canvas.get(&(i as u8)) {
                graphics_entries.push((canvas.z_seq, PositionedWindow {
                    x,
                    y,
                    w: cols,
                    h: rows,
                    x_px,
                    y_px,
                    w_px: win.x_size,
                    h_px: win.y_size,
                    left_margin: win.left_margin,
                    right_margin: win.right_margin,
                    node: WinNode::Graphics(GraphicsWindow {
                        win: i as u32,
                        canvas: canvas.arc(),
                        version: canvas.version,
                        upscale: false,
                    }),
                }));
            }

            // Window 0 is normally the scrolling transcript Buffer — but with
            // its wrapping attribute CLEARED it is in positioned paint mode
            // (menu screens: Zork Zero's InvisiClues clears bit 0 and paints
            // topics via set_cursor), and its pixel runs render like any grid
            // window's.
            let node = if i == 0 && win.attributes & 1 != 0 {
                WinNode::Buffer(BufferWindow {
                    primary: true,
                    bg: (win.bg != ZColour::Default).then(|| crate::state::pack_zcolour(win.bg)),
                    fg: (win.fg != ZColour::Default).then(|| crate::state::pack_zcolour(win.fg)),
                    ..Default::default()
                })
            } else {
                WinNode::Grid(GridWindow {
                    cols,
                    rows,
                    cells: win
                        .grid
                        .cells
                        .iter()
                        .map(|c| GridCell {
                            ch: c.ch,
                            style: c.style,
                            fg: crate::state::pack_zcolour(c.fg),
                            bg: crate::state::pack_zcolour(c.bg),
                            link: 0, // Z-machine grid cells carry no Glk hyperlink
                            glk_style: 0, // Z-machine is always Normal
                        })
                        .collect(),
                    active_rows: rows,
                    // The v6 window cursor is stored in 1-based PIXELS (ZMSD
                    // §8.8.3.2); the cell renderer wants 1-based cells.
                    cursor: (
                        (win.y_cursor.max(1) - 1) / V6_FONT_HEIGHT + 1,
                        (win.x_cursor.max(1) - 1) / V6_FONT_WIDTH + 1,
                    ),
                    cursor_active: v6.current == i as u8,
                    border: BorderPref::Unspecified,
                    bg: (win.bg != ZColour::Default).then(|| crate::state::pack_zcolour(win.bg)),
                    fg: (win.fg != ZColour::Default).then(|| crate::state::pack_zcolour(win.fg)),
                    reverse: false,
                    // Exact pixel-positioned runs for the pixel raster (the
                    // cells above stay the cell-mode fallback).
                    px_texts: win
                        .texts
                        .iter()
                        .map(|t| crate::engine::PxText {
                            y: t.y,
                            x: t.x,
                            text: t.text.clone(),
                            style: t.style,
                            fg: crate::state::pack_zcolour(t.fg),
                            bg: crate::state::pack_zcolour(t.bg),
                        })
                        .collect(),
                })
            };
            text_entries.push(PositionedWindow {
                x,
                y,
                w: cols,
                h: rows,
                x_px,
                y_px,
                w_px: win.x_size,
                h_px: win.y_size,
                left_margin: win.left_margin,
                right_margin: win.right_margin,
                node,
            });
        }

        // Sort graphics by draw order (stable: equal stamps keep window order),
        // then drop the stamps — later-drawn windows now composite on top.
        graphics_entries.sort_by_key(|(seq, _)| *seq);
        let mut graphics_entries: Vec<PositionedWindow> =
            graphics_entries.into_iter().map(|(_, pw)| pw).collect();

        // content_size: the max right/bottom cell extent actually covered by a
        // window, or (when no window survived the size-0 skip) the header's
        // whole-screen char dims (0x21 cols / 0x20 rows) — either way nonzero,
        // so the v6 model always leaves the simple/degenerate render path.
        let mut max_x = 0u16;
        let mut max_y = 0u16;
        for pw in graphics_entries.iter().chain(text_entries.iter()) {
            max_x = max_x.max(pw.x + pw.w);
            max_y = max_y.max(pw.y + pw.h);
        }
        let content_size = if max_x == 0 || max_y == 0 {
            (
                self.machine.mem.read_byte(0x21) as u16,
                self.machine.mem.read_byte(0x20) as u16,
            )
        } else {
            (max_x, max_y)
        };

        graphics_entries.extend(text_entries);
        ScreenModel {
            root: WinNode::Layered(graphics_entries),
            status: status_model_from_machine(&self.machine),
            bg: crate::state::pack_zcolour(screen.current_bg),
            fg: crate::state::pack_zcolour(screen.current_fg),
            content_size,
        }
    }
}

/// Convert a detected `Location` into the `ObjectSnapshot` used as a room id.
/// `NameOnly` (no backing object) gets a stable synthetic id from its name;
/// every other variant carries a real object. Shared by per-turn draining and
/// the startup seed so both assign the same room id.
fn location_to_snapshot(loc: &Location) -> zvm::ObjectSnapshot {
    match loc {
        Location::NameOnly(name) => zvm::ObjectSnapshot {
            number: crate::roomid::synthetic_room_id(name),
            parent: 0,
            name: name.clone(),
        },
        _ => loc.object().expect("non-NameOnly variants carry an object").clone(),
    }
}

// ── Mapper bridge ─────────────────────────────────────────────────────────────

/// Pure bridge: observe the new location (if any) into the mapper.
///
/// Calls `mapper.observe(snap.number, &snap.name, parse_direction(command))`.
/// In Auto mode, runs a light overlap cleanup (radius 2, max 20 passes) after each
/// observation so the live map never shows an illegal connector overlap.
/// No-op when `result.location` is `None`.
pub fn apply_turn(mapper: &mut Mapper, command: &str, result: &TurnResult) {
    if let Some(snap) = &result.location {
        // Suppress an unvalidated NameOnly location until the map holds a real,
        // object-backed room. A NameOnly before any room is a pre-game
        // banner/menu/character-sheet — e.g. BeyondZork's VT220 setup shows the
        // player's name ("Frank Booth") in a status-line-shaped character sheet.
        // Because NameOnly is gated while the map is empty, the first room to
        // populate it is always object-backed; thereafter NameOnly still works
        // as a legitimate mid-game fallback.
        if result.location_method == Some(LocationMethod::NameOnly)
            && mapper.graph.rooms().next().is_none()
        {
            return;
        }
        if is_death_relocation(&result.transcript) {
            // The game printed a death banner this turn and resurrected the player
            // into a room that is NOT reachable by the command they typed (e.g. a
            // grue kills you in the dark and drops you in the Forest). Record it as
            // an involuntary relocation so no false directional edge is minted from
            // the room you died in to the resurrection room. (SQ-0259)
            mapper.observe_relocation(snap.number, &snap.name);
        } else {
            mapper.observe(snap.number, &snap.name, parse_direction(command));
        }
        // Overlap cleanup is NOT done here: it is map-layout work and must never run
        // on the interpreter thread. On a geometry change the run loop schedules a
        // background cleanup (or full tidy) job — see `finish_command_turn` and
        // `cleanup_overlaps_layer_silent`. (SQ-0379)
    }
}

/// True when this turn's output carries a death/end banner — the interpreter
/// convention of a `*** … ***` line (Inform's `*** You have died ***`, Infocom's
/// spaced `****  You have died  ****`). On such a turn a game may resurrect the
/// player into a room unrelated to the typed command, so the resulting room change
/// must be recorded as an involuntary relocation rather than a walked passage.
///
/// Kept deliberately tight — an asterisk-delimited banner line containing a death
/// word — so it never fires on ordinary room text that merely mentions the dead
/// (and it ignores the winning banner `*** You have won ***`, which changes no
/// room). Custom death banners without "died"/"dead" are a known gap. (SQ-0259)
fn is_death_relocation(transcript: &str) -> bool {
    transcript.lines().any(|line| {
        let t = line.trim();
        if t.len() < 4 || !t.starts_with("**") || !t.ends_with("**") {
            return false;
        }
        let lower = t.to_ascii_lowercase();
        lower.contains("died") || lower.contains("dead")
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Stop reason from `run_until_input`.
enum RunStop {
    /// VM is waiting for player input of this kind.
    Input(InputKind),
    /// VM ended (Quit/Restart).
    Quit,
    /// VM suspended on its own `@save` — host must `resume_save`.
    SavePending,
    /// VM suspended on its own `@restore` — host must `resume_restore`.
    RestorePending,
}

/// Step until the machine pauses for input, quits, or suspends on its own
/// `@save`/`@restore`. In-game save/restore bubbles up as `SavePending`/
/// `RestorePending` for the host to service (all versions, v3 included).
fn run_until_input(machine: &mut Machine) -> RunStop {
    loop {
        match machine.step() {
            StepResult::Quit => return RunStop::Quit,
            StepResult::Fault => return RunStop::Quit,
            StepResult::NeedLine { .. } => return RunStop::Input(InputKind::Line),
            StepResult::NeedChar => return RunStop::Input(InputKind::Char),
            StepResult::SaveRequest => return RunStop::SavePending,
            StepResult::RestoreRequest => return RunStop::RestorePending,
            StepResult::Restart => return RunStop::Quit, // not supported headless; treat as quit
            StepResult::Continue => {}
        }
    }
}

/// Strip a trailing interactive read prompt from captured Z-machine output.
///
/// Infocom-style games print a bare ">" (possibly preceded by whitespace or a
/// newline, possibly followed by a space) as the last thing before issuing a
/// read/sread opcode.  When that output is captured we want to remove it so the
/// app's own fixed bottom input line is the only ">" the player sees.
///
/// The rule: trim trailing ASCII whitespace; if the result ends with ">" AND
/// that ">" is preceded by a newline or is the only character, remove it and
/// trim trailing whitespace again.  Any ">" that appears mid-sentence (e.g.
/// inside a score display like "score > 10") is unaffected because it will not
/// be the last non-whitespace character after a newline.
pub(crate) fn strip_read_prompt(s: &str) -> &str {
    let trimmed = s.trim_end_matches([' ', '\t']);
    // After stripping trailing spaces/tabs the string may still end with "\n>"
    // or just ">".  Check for that and strip.
    if let Some(without_gt) = trimmed.strip_suffix('>') {
        // Only strip if the ">" is at the start of a line (preceded by '\n')
        // or if it's the only character remaining.
        let preceded_by_newline = without_gt.ends_with('\n') || without_gt.is_empty();
        if preceded_by_newline {
            return without_gt.trim_end_matches([' ', '\t', '\n', '\r']);
        }
    }
    trimmed
}

/// Downcast `machine.out` to `&mut CaptureSink`.
///
/// Panics if the machine was not built with a `CaptureSink` (should never
/// happen within this module since `GameSession::new` always installs one).
fn sink_mut(machine: &mut Machine) -> &mut CaptureSink {
    machine
        .out
        .as_any_mut()
        .downcast_mut::<CaptureSink>()
        .expect("GameSession machine must have a CaptureSink output")
}

// ── Adventure-title helpers ───────────────────────────────────────────────────

/// Canonical titles for well-known games, keyed by the release+serial prefix of
/// the IFID (`ZCODE-<release>-<serial>`, WITHOUT the trailing byte-checksum),
/// bundled in `known_titles.tsv` (`include_str!`d at build time). The key is
/// robust to different file copies of the same release. Used to prefer a clean
/// canonical name over the opening-banner heuristic, and by the story picker.
fn known_titles() -> &'static std::collections::HashMap<&'static str, &'static str> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<std::collections::HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("known_titles.tsv")
            .lines()
            .filter_map(|line| {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                line.split_once('\t').map(|(k, v)| (k.trim(), v.trim()))
            })
            .collect()
    })
}

/// The canonical title for a known game, matched on the release+serial prefix of
/// the IFID (the trailing `-<checksum>` is ignored).
pub fn known_title(ifid: &str) -> Option<&'static str> {
    // Strip the trailing checksum segment: "ZCODE-88-840726-A129" → "ZCODE-88-840726".
    let key = ifid.rsplit_once('-').map_or(ifid, |(prefix, _)| prefix);
    known_titles().get(key).copied()
}

/// Extract the adventure title from the opening banner by anchoring on the
/// Infocom-style boilerplate: the title is the non-blank line immediately ABOVE
/// the first line that looks like copyright / "interactive fiction|fantasy" /
/// "Serial number" / trademark text. Returns the trimmed title (capped at 40
/// chars), or `None` when the banner opens with boilerplate (no title above it)
/// or has no such anchor (e.g. an epigraph or story narration) — the caller then
/// falls back to the filename. This avoids grabbing copyright/quote/narration
/// lines as the title.
pub fn title_from_banner(intro_text: &str) -> Option<String> {
    let lines: Vec<&str> = intro_text
        .lines()
        .map(str::trim)
        .filter(|l| {
            !(l.is_empty() || l.starts_with('>') && l.trim_start_matches('>').trim().is_empty())
        })
        .collect();

    let is_anchor = |l: &str| {
        let lower = l.to_lowercase();
        lower.contains("copyright")
            || lower.contains("interactive fiction")
            || lower.contains("interactive fantasy")
            || lower.contains("serial number")
            || lower.contains("trademark")
    };

    let anchor = lines.iter().position(|l| is_anchor(l))?;
    if anchor == 0 {
        return None; // banner opens with boilerplate; no title line above it
    }
    Some(lines[anchor - 1].chars().take(40).collect())
}

/// Resolve the adventure title using a three-tier priority:
/// 1. `override_name` if provided.
/// 2. `banner` (a captured first-banner-line) if provided.
/// 3. The story file's stem (filename without extension).
pub fn resolve_title(
    override_name: Option<&str>,
    ifid: &str,
    banner: Option<&str>,
    story_path: &std::path::Path,
) -> String {
    if let Some(name) = override_name {
        return name.to_owned();
    }
    // Known-game lookup table wins over the banner heuristic (it is exact, and
    // covers games whose banner doesn't yield the title).
    if let Some(t) = known_title(ifid) {
        return t.to_owned();
    }
    if let Some(b) = banner {
        return b.to_owned();
    }
    story_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned()
}

// ── Engine adapter (zvm) ────────────────────────────────────────────────────
//
// `GameSession` implements the engine-neutral `Engine` trait so the app can
// drive the Z-machine through the abstraction. The adapter is a thin wrapper:
// the turn methods delegate to the inherent methods (dot-syntax method calls
// resolve to the inherent impl, which takes precedence over the trait), the
// relocated key→ZSCII mapping lives in `key_input_to_zscii`, and `screen()`
// mirrors the Z-machine screen into the neutral `ScreenModel`.

use crate::engine::{
    BorderPref, BufferWindow, Debugger, DisasmProvenance, Engine, EngineError, EngineSave, GraphicsWindow,
    GridCell, GridWindow, Introspect, KeyInput, LocationInfo, PositionedWindow, ScreenModel, Split,
    StatusField, StatusModel, WinNode,
};

/// The engine tag recorded in an `EngineSave` produced by the Z-machine adapter.
pub const ZMACHINE_ENGINE: &str = "zmachine";
/// The save-format version within the `zmachine` engine (Quetzal).
const ZMACHINE_SAVE_FORMAT: u32 = 1;

impl GameSession {
    /// Build the disasm cache on first use, memoize it, and run `f` against it.
    fn with_disasm_cache<R>(&self, f: impl FnOnce(&zvm::cpu::disasm_cache::DisasmCache) -> R) -> R {
        {
            let mut slot = self.disasm_cache.borrow_mut();
            if slot.is_none() {
                let mut cache = zvm::cpu::disasm_cache::DisasmCache::build(&self.machine.mem);
                // Fold the ENTIRE cumulative "ever executed" set ONCE at build
                // time (covers loaded-sidecar seed + all boot/turn PCs) so those
                // regions decode as real code (soft→rd re-decode). The per-turn
                // `exec_pcs` fold in `fold_confirmations` handles later turns; this
                // stays O(build), never O(turn·|ever|). (SQ-0449)
                let mem = &self.machine.mem;
                for &pc in &self.machine.ever_exec_pcs {
                    cache.confirm_pc(mem, pc);
                }
                self.machine.mem.take_mem_fault();
                *slot = Some(cache);
            }
        } // drop borrow_mut before confirmation / the shared borrow
        // Runtime confirmation, once per turn (skip while parked at same PC).
        if self.last_confirmed_pc.get() != Some(self.machine.state.pc) {
            self.confirm_disasm();
        }
        let slot = self.disasm_cache.borrow();
        f(slot.as_ref().unwrap())
    }

    /// Fold runtime-confirmed boundaries (call-stack func_addrs, parked PC, and
    /// last turn's executed PCs) into the cache. No-op if the cache isn't built.
    fn fold_confirmations(&self) {
        let mut slot = self.disasm_cache.borrow_mut();
        let Some(cache) = slot.as_mut() else { return }; // don't build just to confirm
        let mem = &self.machine.mem;
        for f in &self.machine.state.frames {
            cache.confirm_routine(mem, f.func_addr);
        }
        cache.confirm_pc(mem, self.machine.state.pc);
        // When parked at an input prompt, `state.pc` points PAST the read to the
        // code that consumes the input; confirm the read instruction itself too, so
        // it renders as a real op instead of being eaten by a stale tiling. This is
        // independent of `trace_exec` (the read may have executed before tracing was
        // on — e.g. during startup, for the first prompt). (SQ read-pc fix)
        if let Some(read_pc) = self.machine.pending_read_pc() {
            cache.confirm_pc(mem, read_pc);
        }
        for &pc in &self.machine.exec_pcs {
            cache.confirm_pc(mem, pc);
        }
        // Draining a fault isn't needed here (confirm reads via decode which may
        // latch a fault) — drain to be safe, matching the other debug read paths.
        self.machine.mem.take_mem_fault();
    }

    /// Public entry for the per-turn confirmation fold (also callable in tests).
    ///
    /// Only marks the per-turn gate when the cache actually exists, so calling
    /// this before the cache is built (a bare public call) does not poison the
    /// gate and skip the first real fold.
    pub fn confirm_disasm(&self) {
        let built = self.disasm_cache.borrow().is_some();
        if built {
            self.fold_confirmations();
            self.last_confirmed_pc.set(Some(self.machine.state.pc));
        }
    }

    /// object entry base address -> object number.
    fn object_addr_map(&self) -> std::collections::HashMap<u32, u16> {
        let mem = &self.machine.mem;
        zvm::object_tree_view(&self.machine)
            .iter()
            .map(|s| (zvm::objects::object_entry_addr(mem, s.number), s.number))
            .collect()
    }

    /// dictionary entry base address -> decoded word.
    fn dict_addr_map(&self) -> std::collections::HashMap<u32, String> {
        let mem = &self.machine.mem;
        let d = zvm::dictionary::load(mem); // pub fields: base, count, entry_length
        (0..d.count as u32)
            .filter_map(|i| {
                let addr = d.base + i * d.entry_length as u32;
                let (w, _) = zvm::text::decode::decode_string(mem, addr);
                let w = w.trim().to_string();
                (!w.is_empty()).then_some((addr, w))
            })
            .collect()
    }

    /// Insert a ` [tag]` annotation right after each resolvable `@0x{6hex}` memory
    /// operand in a formatted disassembly line (object wins over dictionary). The
    /// scan is byte-safe (`@0x` + hex digits are all ASCII); insertions are applied
    /// right-to-left so earlier byte positions stay valid.
    fn annotate_refs(
        &self,
        line: &str,
        objs: &std::collections::HashMap<u32, u16>,
        dict: &std::collections::HashMap<u32, String>,
    ) -> String {
        let mut inserts: Vec<(usize, String)> = Vec::new();
        let mut i = 0;
        while i + 9 <= line.len() {
            if line.get(i..i + 3) == Some("@0x") {
                if let Some(hex) = line.get(i + 3..i + 9) {
                    if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                        if let Ok(a) = u32::from_str_radix(hex, 16) {
                            if let Some(n) = objs.get(&a) {
                                inserts.push((i + 9, format!(" [obj#{n}]")));
                            } else if let Some(w) = dict.get(&a) {
                                inserts.push((i + 9, format!(" [{w}]")));
                            }
                            i += 9;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
        let mut s = line.to_string();
        for (pos, text) in inserts.into_iter().rev() {
            s.insert_str(pos, &text);
        }
        s
    }

    /// Map a neutral [`KeyInput`] to a ZSCII input byte (the logic relocated from
    /// the app's former `key_to_zscii`). Returns `None` for keys with no ZSCII
    /// meaning (non-ASCII printables, unhandled specials), so the caller leaves
    /// the turn untouched — matching the old "skip unmapped key" behavior exactly.
    ///
    /// Arrow keys and function keys are mapped to ZSCII cursor/function codes
    /// (ZMSD §3.8): Up=129, Down=130, Left=131, Right=132, F1–F4=133–136.
    /// These match zvm-cli's `decode_escape_seq` in `crates/zvm-cli/src/screen.rs`.
    fn key_input_to_zscii(key: KeyInput) -> Option<u8> {
        match key {
            KeyInput::Enter => Some(13),
            KeyInput::Backspace => Some(8),
            KeyInput::Escape => Some(27),
            KeyInput::Up    => Some(129),
            KeyInput::Down  => Some(130),
            KeyInput::Left  => Some(131),
            KeyInput::Right => Some(132),
            KeyInput::Func(n) => Some(132u8.saturating_add(n)),
            KeyInput::Char(c) if c.is_ascii() => Some(c as u8),
            _ => None,
        }
    }

    /// While a Z-machine *line* read is active, decide whether a special key the
    /// player pressed is one the game listed as a line terminator (v5+ table).
    /// Only arrow keys and function keys are candidate terminators; Enter (13)
    /// flows through the normal submit path, and all other keys are never
    /// terminators. Returns the ZSCII terminator code to submit with, or `None`
    /// to leave the key to its normal app behavior.
    pub fn line_key_terminator(&self, ki: &KeyInput) -> Option<u8> {
        match ki {
            KeyInput::Up | KeyInput::Down | KeyInput::Left | KeyInput::Right | KeyInput::Func(_) => {}
            _ => return None,
        }
        let z = Self::key_input_to_zscii(*ki)?;
        self.is_terminator(z as u16).then_some(z)
    }
}

/// Mirror a Z-machine's screen into the neutral [`ScreenModel`].
///
/// The upper window becomes a [`GridWindow`] (logical size + cells + cursor +
/// active-window flag); the lower window is a buffer placeholder (the app owns
/// the transcript). The status is the v3 automatic status line (`Classic`) for
/// v1–3, or `HostManaged` for v4+ (whose globals are not a status line). Shared
/// by the engine adapter and the render-equivalence tests.
pub fn screen_model_from_machine(machine: &Machine) -> ScreenModel {
    let screen = &machine.screen;
    let src = &screen.upper;
    let grid = GridWindow {
        cols: src.cols,
        rows: src.rows,
        cells: src
            .cells
            .iter()
            .map(|c| GridCell {
                ch: c.ch,
                style: c.style,
                fg: crate::state::pack_zcolour(c.fg),
                bg: crate::state::pack_zcolour(c.bg),
                link: 0, // Z-machine grid cells carry no Glk hyperlink
                glk_style: 0, // Z-machine is always Normal
            })
            .collect(),
        // `upper.rows` equals `upper_window_rows` normally, but grows when a game
        // draws in the upper window below the split (e.g. LostPig's HELP menu
        // splits to 7 rows then prints 5 items at rows 6–10). Render/reserve the
        // full grown height so nothing is clipped.
        active_rows: screen.upper.rows,
        cursor: (screen.cursor_row, screen.cursor_col),
        cursor_active: screen.current_window == 1,
        // The Z-machine has no Glk border concept — leave it to the theme (SQ-0286).
        border: BorderPref::Unspecified,
        // The Z-machine simple path carries no per-window colour override; the
        // page colour comes from the model bg/fg below, so draw_grid stays
        // byte-identical (bg=None → theme). (SQ-0328)
        bg: None,
        fg: None,
        // Z-machine grid reverse is per-cell (style bits), not a window-level Glk
        // ReverseColor, so no window-level reverse fill here. (SQ-0403)
        reverse: false,
        px_texts: Vec::new(),
    };
    ScreenModel {
        root: WinNode::Pair {
            vertical: true,
            split: Split { fixed: screen.upper.rows },
            // The Z-machine has no Glk border; its status box is drawn by the simple path.
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Grid(grid)),
            second: Box::new(WinNode::Buffer(BufferWindow::default())),
        },
        status: status_model_from_machine(machine),
        bg: crate::state::pack_zcolour(screen.current_bg),
        fg: crate::state::pack_zcolour(screen.current_fg),
        // Z-machine layout has no snap margin (simple path); the composite never
        // clamps it. (SQ-0303)
        content_size: (0, 0),
    }
}

/// Build the neutral [`StatusModel`] from a Z-machine's screen state: a
/// `Classic` automatic status line (location + score/turns or clock) for v1–3,
/// or `HostManaged` for v4+ (whose globals are not a status line). Shared by
/// the engine adapter and the render-equivalence tests.
pub fn status_model_from_machine(machine: &Machine) -> StatusModel {
    if machine.mem.version() <= 3 {
        let sl = machine.status_line();
        let right = match sl.right {
            zvm::screen::StatusRight::ScoreTurns { score, turns } => {
                StatusField::ScoreTurns { score, turns }
            }
            zvm::screen::StatusRight::Time { hours, minutes } => {
                StatusField::Time { hours, minutes }
            }
        };
        StatusModel::Classic { location: sl.location, right }
    } else {
        StatusModel::HostManaged
    }
}

impl Engine for GameSession {
    fn submit(&mut self, command: &str) -> TurnResult {
        if self.machine.trace_exec { self.machine.exec_pcs.clear(); }
        // A turn executes new code, so its freshly-recorded boundaries must be
        // folded afterward EVEN IF it returns to the same parked PC (every
        // look/examine returns to the same input prompt). Reopen the per-turn
        // confirmation gate so the next disassemble re-folds. (read-pc follow-up)
        self.last_confirmed_pc.set(None);
        // Dot syntax resolves to the inherent `GameSession::submit` (inherent
        // methods take precedence over trait methods), so this is not recursive.
        self.submit(command)
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        if self.machine.trace_exec { self.machine.exec_pcs.clear(); }
        self.last_confirmed_pc.set(None); // reopen the confirmation gate each turn
        let byte = GameSession::key_input_to_zscii(key)?;
        Some(self.submit_char(byte))
    }

    fn set_mouse(&mut self, y_px: u16, x_px: u16) {
        // Primary button (bit 0) — a host left-click. The VM records the coords
        // and writes the header extension table (ZMSD §11); a following
        // `read_mouse` reports them.
        self.machine.set_mouse(y_px, x_px, 0b1);
    }

    fn take_transcript(&mut self) -> String {
        self.take_transcript()
    }

    fn take_transcript_elems(&mut self) -> Vec<TranscriptElem> {
        // Non-empty only when v6 window-0 inline pictures are pending (Zork
        // Zero's boot drop-cap): interleave them into the sink text as ordered
        // elements. Every other story returns empty → the flat path is used.
        if self.story_pics.is_empty() {
            return Vec::new();
        }
        let base = self.v6_win0_chars_seen;
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        self.v6_win0_chars_seen = self.machine.v6_win0_out_chars;
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let runs = clamp_runs(raw_runs, transcript.chars().count());
        interleave_story_pics(&transcript, &runs, std::mem::take(&mut self.story_pics), base)
    }

    fn set_strip_prompt(&mut self, on: bool) {
        self.strip_prompt = on;
    }

    fn pending_input(&self) -> InputKind {
        self.pending
    }

    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.resume_save(wrote_ok)
    }

    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        self.resume_restore(data)
    }

    fn has_quit(&self) -> bool {
        self.quit
    }

    fn screen(&self) -> ScreenModel {
        if self.machine.screen.v6.is_some() {
            self.v6_screen_model()
        } else {
            screen_model_from_machine(&self.machine)
        }
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(ZMACHINE_ENGINE, ZMACHINE_SAVE_FORMAT, self.machine.save_quetzal())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(ZMACHINE_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: ZMACHINE_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        self.machine
            .restore_file(&save.bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        // A Save State is snapshotted at an input prompt; its PC points AT the
        // read/read_char instruction (save_pc rewinds it), so run forward to
        // re-execute that read — re-arming the pending input on the freshly
        // restored buffers. Without this the VM would be parked past the read
        // with a stale buffer, and the next line would replay the pre-save
        // command (mirrors `resume_restore` for the game `@restore` path).
        let stop = run_until_input(&mut self.machine);
        self.pending = match stop {
            RunStop::Input(k) => k,
            RunStop::Quit => InputKind::Line,
            // A well-formed Save State resumes into a read; the save/restore
            // arms are unreachable here (no @save/@restore mid-snapshot).
            RunStop::SavePending | RunStop::RestorePending => self.pending,
        };
        Ok(())
    }

    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        self.machine.complete_restore_success(bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        // complete_restore_success lands mid-way through the game's save verb
        // (just past the @save descriptor), not at a read. Run forward to the
        // next read so the machine is re-armed at a clean prompt — otherwise the
        // first typed command is dropped while the save-verb tail runs (mirrors
        // resume_restore for the in-game @restore path). The save-verb tail
        // output (e.g. "Ok.") is redundant with the host's "[Game restored]"
        // message, so drain and discard it.
        let stop = run_until_input(&mut self.machine);
        let _ = self.take_transcript();
        match stop {
            RunStop::Input(k) => self.pending = k,
            RunStop::Quit => self.quit = true,
            RunStop::SavePending | RunStop::RestorePending => {}
        }
        Ok(())
    }

    fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
        &self.machine.aux_data
    }

    fn set_aux_data(&mut self, data: std::collections::BTreeMap<String, Vec<u8>>) {
        self.machine.aux_data = data;
    }

    fn aux_dirty(&self) -> bool {
        self.machine.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.machine.aux_dirty = false;
    }

    fn current_location(&self) -> Option<LocationInfo> {
        // Version-aware detection (same as a turn), NOT the v3-only global-0 read:
        // v4+ games have no location global, so `zvm::current_location` returns
        // None at boot, leaving the starting room off the map until the first turn.
        detect_location(&self.machine).as_ref().map(location_to_snapshot)
    }

    fn set_trace_screen(&mut self, on: bool) {
        self.machine.trace_screen = on;
    }

    fn take_screen_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.machine.screen_trace)
    }

    fn set_debug_trace(&mut self, on: bool) {
        self.machine.trace_exec = on;
        // Only the per-turn set is cleared when tracing stops; the cumulative
        // `ever_exec_pcs` (permanent colour + persisted coverage) is preserved.
        if !on { self.machine.exec_pcs.clear(); }
    }

    fn seed_executed_pcs(&mut self, pcs: &std::collections::HashSet<u32>) {
        self.machine.seed_executed(pcs.iter().copied());
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn introspect(&self) -> Option<&dyn Introspect> {
        Some(self)
    }

    fn debugger(&self) -> Option<&dyn Debugger> {
        Some(self)
    }
}

impl Introspect for GameSession {
    fn vocabulary(&self) -> Vec<String> {
        zvm::dictionary::load(&self.machine.mem).words(&self.machine.mem)
    }

    fn contents(&self, container: u16) -> Vec<String> {
        crate::inventory::list_inventory(&self.machine.mem, container)
    }

    fn room_objects(&self, room: u16) -> Vec<String> {
        crate::render::room_info::list_room_objects(&self.machine.mem, room)
    }

    fn children_of(&self, parent: u16) -> std::collections::BTreeSet<u16> {
        let max_obj = zvm::object_tree_view(&self.machine)
            .into_iter()
            .map(|s| s.number)
            .max()
            .unwrap_or(0);
        (1..=max_obj)
            .filter(|&o| zvm::objects::get_parent(&self.machine.mem, o) == parent)
            .collect()
    }

    fn player_object(&self) -> Option<u16> {
        zvm::find_player_object(&self.machine)
    }
}

impl Debugger for GameSession {
    fn pc(&self) -> u32 {
        self.machine.state.pc
    }

    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        let mut out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Full));
        // Annotate `@0x……` memory-reference operands with their referent: a
        // clickable ` [obj#N]` for an object entry base, an informational ` [word]`
        // for a dictionary entry base. Build both reverse maps once per call.
        let objs = self.object_addr_map();
        let dict = self.dict_addr_map();
        for line in &mut out {
            *line = self.annotate_refs(line, &objs, &dict);
        }
        // The disassembler can walk past code into data; an out-of-range read
        // latches a fault into Memory's fault cell that the CPU drains each step.
        // Discard it here so this read-only inspection never leaks a phantom fault
        // that would halt the VM on its next instruction. Between turns there is no
        // legitimately-pending fault (the VM consumes its own at step end), so
        // discarding is safe.
        self.machine.mem.take_mem_fault();
        out
    }

    fn disassemble_tiered(&self, addr: u32, lines: usize) -> Vec<(String, DisasmProvenance)> {
        use zvm::cpu::disasm_cache::CacheFmt;
        // Full-form rows carry the same `[obj#N]`/`[word]` annotations as
        // `disassemble`; provenance is display-format-independent, so a caller in
        // basic/raw mode pairs these provenance tags with its own text lines.
        let mut out =
            self.with_disasm_cache(|c| c.disassemble_tiered(&self.machine.mem, addr, lines, CacheFmt::Full));
        let objs = self.object_addr_map();
        let dict = self.dict_addr_map();
        for (line, _prov) in &mut out {
            *line = self.annotate_refs(line, &objs, &dict);
        }
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out.into_iter().map(|(s, p)| (s, p.into())).collect()
    }

    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        let out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Raw));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String> {
        use zvm::cpu::disasm_cache::CacheFmt;
        // Basic form: plain mnemonic disassembly with NO annotations (the
        // `[obj#N]`/`[word]` reference-following stays exclusive to `disassemble`).
        let out = self.with_disasm_cache(|c| c.disassemble(&self.machine.mem, addr, lines, CacheFmt::Basic));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn next_instr(&self, addr: u32) -> u32 {
        let out = self.with_disasm_cache(|c| c.next_addr(addr));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn prev_instr(&self, addr: u32) -> u32 {
        let out = self.with_disasm_cache(|c| c.prev_addr(addr));
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn describe_line(&self, addr: u32) -> Option<Vec<String>> {
        let version = self.machine.mem.version();
        let instr = zvm::cpu::decode::decode(&self.machine.mem, addr, version);
        let unpack = zvm::cpu::disasm::Unpack::from_mem(&self.machine.mem);
        let lines = zvm::cpu::disasm::describe_instruction(&instr, version, &unpack);
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        Some(lines)
    }

    fn executed_pcs(&self) -> std::collections::HashSet<u32> {
        self.machine.exec_pcs.clone()
    }

    fn ever_executed_pcs(&self) -> std::collections::HashSet<u32> {
        self.machine.ever_exec_pcs.clone()
    }

    fn stack_lines(&self) -> Vec<String> {
        let st = &self.machine.state;
        if st.frames.is_empty() {
            return vec!["(no frames)".to_string()];
        }
        let mut out = Vec::with_capacity(st.frames.len());
        for (i, f) in st.frames.iter().enumerate() {
            out.push(format!(
                "#{i}  fn@{:06x}  ret={:06x}  args={}",
                f.func_addr, f.return_pc, f.arg_count
            ));
        }
        out
    }

    fn eval_stack_lines(&self) -> Vec<String> {
        let st = &self.machine.state;
        if st.eval_stack.is_empty() {
            return vec!["(empty)".to_string()];
        }
        let bases: std::collections::HashSet<usize> =
            st.frames.iter().map(|f| f.eval_base).collect();
        st.eval_stack.iter().enumerate().rev().map(|(i, v)| {
            let b = if bases.contains(&i) { "  <- frame base" } else { "" };
            format!("[{i:>3}] {:04x}  ({}){}", v, *v as i16, b)
        }).collect()
    }

    fn locals_lines(&self) -> Vec<String> {
        match self.machine.state.frames.last() {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(none)".to_string()],
            Some(f) => f.locals.iter().enumerate()
                .map(|(i, w)| format!("local{i} = {:04x}  ({})", w, w))
                .collect(),
        }
    }

    fn globals_lines(&self) -> Vec<String> {
        let out: Vec<String> =
            (0u8..240).map(|n| format!("g{:02x} = {:04x}", n, self.machine.global(n))).collect();
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn object_tree_lines(&self) -> Vec<String> {
        // A real tree: DFS over the child/sibling links so each object renders
        // directly under its parent. (Numeric order + per-object indent, which
        // this replaces, does NOT nest children under their parents.)
        let mem = &self.machine.mem;
        let numbers: Vec<u16> = zvm::object_tree_view(&self.machine)
            .iter().map(|s| s.number).collect();
        let out = build_object_tree(
            &numbers,
            |o| zvm::objects::get_parent(mem, o),
            |o| zvm::objects::get_child(mem, o),
            |o| zvm::objects::get_sibling(mem, o),
            |o| zvm::objects::short_name(mem, o),
            |o| zvm::objects::object_entry_addr(mem, o),
        );
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn dictionary_lines(&self) -> Vec<String> {
        // Each row leads with its entry byte address as a clickable `@0x……`
        // Memory-jump token (debug inspector), then the decoded word.
        let out = zvm::dictionary::load(&self.machine.mem)
            .entries(&self.machine.mem)
            .into_iter()
            .map(|(addr, word)| format!("@0x{:06x} {}", addr, word))
            .collect();
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String> {
        let bytes = self.machine.mem.raw_bytes();
        let len = bytes.len() as u32;
        let mut out = Vec::with_capacity(rows);
        let mut a = addr.min(len);
        for _ in 0..rows {
            if a >= len { break; }
            let end = (a + 16).min(len);
            let row = &bytes[a as usize..end as usize];
            let hex: String = row.iter().map(|b| format!("{:02x} ", b)).collect();
            // VM-correct char column: basic ASCII printable range is a direct
            // identity mapping (same result zscii_to_char would give); the
            // 155-223 ZSCII extended range goes through the story's custom
            // Unicode table if it has one, else zvm's default ZSCII table
            // (mirrors decode_string's own zscii lookup in text/decode.rs).
            // Everything else (control bytes, unassigned ZSCII) is undecodable
            // as a single glyph → '.'.
            let ascii: String = row.iter()
                .map(|&b| match b {
                    0x20..=0x7e => b as char,
                    155..=223 => self.machine.mem.unicode_char(b as u16)
                        .unwrap_or_else(|| zvm::text::decode::zscii_to_char(b as u16)),
                    _ => '.',
                })
                .collect();
            out.push(format!("{:06x}  {:<48}{}", a, hex, ascii));
            a = end;
        }
        out
    }

    fn memory_len(&self) -> u32 {
        self.machine.mem.len() as u32
    }

    fn object_detail(&self, obj: u16) -> Vec<String> {
        let mem = &self.machine.mem;
        let attr_count: u8 = if mem.version() <= 3 { 32 } else { 48 };
        let attrs: Vec<u8> = (0..attr_count).filter(|&a| zvm::objects::get_attr(mem, obj, a)).collect();
        let mut out = Vec::new();
        if attrs.is_empty() {
            out.push("attrs: (none)".to_string());
        } else {
            let list: Vec<String> = attrs.iter().map(|a| a.to_string()).collect();
            out.push(format!("attrs: {}", list.join(", ")));
        }
        // Walk the property table. Properties are stored strictly descending
        // and there are at most 63, so a valid walk is short. A corrupt object
        // (e.g. one the table-bound heuristic mis-identified) could otherwise
        // make get_next_prop cycle or not descend — guard both ways so the
        // debugger never hangs expanding a bad object.
        let mut prop = zvm::objects::get_next_prop(mem, obj, 0);
        for _ in 0..64 {
            if prop == 0 { break; }
            let addr = zvm::objects::get_prop_addr(mem, obj, prop);
            let len = zvm::objects::get_prop_len(mem, addr);
            let bytes: Vec<String> = (0..len as u32)
                .map(|i| format!("{:02x}", mem.read_byte(addr as u32 + i)))
                .collect();
            out.push(format!("  prop {}: {}", prop, bytes.join(" ")));
            let next = zvm::objects::get_next_prop(mem, obj, prop);
            if next >= prop { break; } // must strictly descend; else corrupt
            prop = next;
        }
        self.machine.mem.take_mem_fault(); // never leak a debug-read fault into the VM
        out
    }

    fn frame_locals(&self, idx: usize) -> Vec<String> {
        match self.machine.state.frames.get(idx) {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(no locals)".to_string()],
            Some(f) => f.locals.iter().enumerate()
                .map(|(i, w)| format!("local{i} = 0x{:04x}  ({})", w, *w as i16))
                .collect(),
        }
    }

    fn var_value(&self, var: u8) -> Option<u16> {
        let st = &self.machine.state;
        match var {
            0 => st.eval_stack.last().copied(), // peek the top; never pops
            1..=15 => st.frames.last()?.locals.get((var - 1) as usize).copied(),
            n => Some(self.machine.global(n - 16)),
        }
    }
}

/// Render the object hierarchy as indented `[N] name` lines in **tree order**:
/// a depth-first walk from each root (parent 0, ascending) down each object's
/// child chain (child, then that child's siblings), so every object sits
/// directly beneath its parent. Pure over the link/name lookups so it is
/// unit-testable without a `Machine`. Guards against malformed data: a `seen`
/// set breaks parent/child/sibling cycles, and any object never reached from a
/// root (a broken link) is still appended (at its parent-chain depth) so
/// nothing silently disappears.
fn build_object_tree(
    numbers: &[u16],
    parent: impl Fn(u16) -> u16,
    child: impl Fn(u16) -> u16,
    sibling: impl Fn(u16) -> u16,
    name: impl Fn(u16) -> String,
    addr: impl Fn(u16) -> u32,
) -> Vec<String> {
    let mut out = Vec::with_capacity(numbers.len());
    let mut seen = std::collections::HashSet::new();
    // Roots pushed in reverse so ascending roots emit first (stack pops LIFO).
    let mut stack: Vec<(u16, usize)> = numbers.iter().rev()
        .filter(|&&o| parent(o) == 0)
        .map(|&o| (o, 0usize))
        .collect();
    while let Some((obj, depth)) = stack.pop() {
        if obj == 0 || depth > 64 || !seen.insert(obj) {
            continue;
        }
        out.push(format!("@0x{:06x} {}[{}] {}", addr(obj), "  ".repeat(depth), obj, name(obj)));
        // Collect this object's child chain, then push reversed so the first
        // child is visited first. `!kids.contains` + `!seen` guard cycles.
        let mut kids = Vec::new();
        let mut c = child(obj);
        while c != 0 && !seen.contains(&c) && !kids.contains(&c) {
            kids.push(c);
            c = sibling(c);
        }
        for &k in kids.iter().rev() {
            stack.push((k, depth + 1));
        }
    }
    // Safety net: objects unreachable from any root still appear, at their
    // parent-chain depth, in ascending number order.
    for &o in numbers {
        if seen.insert(o) {
            let mut depth = 0usize;
            let mut p = parent(o);
            while p != 0 && depth < 64 {
                depth += 1;
                p = parent(p);
            }
            out.push(format!("@0x{:06x} {}[{}] {}", addr(o), "  ".repeat(depth), o, name(o)));
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;

    // ── Object tree ordering ──────────────────────────────────────────────────

    #[test]
    fn build_object_tree_walks_children_under_their_parent() {
        // Two roots (1, 2). 1's children are 3 then 5 (siblings); 3 has child 4;
        // 2 has child 6. A numeric-order+indent render would emit 1,2,3,4,5,6 —
        // this DFS must nest each child directly under its parent instead.
        use std::collections::HashMap;
        let parent: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 1), (4, 3), (5, 1), (6, 2)].into();
        let child: HashMap<u16, u16> = [(1, 3), (2, 6), (3, 4), (4, 0), (5, 0), (6, 0)].into();
        let sibling: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 5), (4, 0), (5, 0), (6, 0)].into();
        let lines = build_object_tree(
            &[1, 2, 3, 4, 5, 6],
            |o| parent[&o], |o| child[&o], |o| sibling[&o], |o| format!("o{o}"),
            |o| 0x100 + o as u32,
        );
        assert_eq!(lines, vec![
            "@0x000101 [1] o1".to_string(),
            "@0x000103   [3] o3".to_string(),
            "@0x000104     [4] o4".to_string(),
            "@0x000105   [5] o5".to_string(),
            "@0x000102 [2] o2".to_string(),
            "@0x000106   [6] o6".to_string(),
        ]);
    }

    #[test]
    fn build_object_tree_appends_objects_unreachable_from_a_root() {
        // Object 3 claims parent 2, but 2 has no child pointing back — a broken
        // link. It must still appear (at its parent-chain depth), never vanish.
        use std::collections::HashMap;
        let parent: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 2)].into();
        let child: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 0)].into();
        let sibling: HashMap<u16, u16> = [(1, 0), (2, 0), (3, 0)].into();
        let lines = build_object_tree(
            &[1, 2, 3],
            |o| parent[&o], |o| child[&o], |o| sibling[&o], |o| format!("o{o}"),
            |o| 0x200 + o as u32,
        );
        assert_eq!(lines, vec![
            "@0x000201 [1] o1".to_string(),
            "@0x000202 [2] o2".to_string(),
            "@0x000203   [3] o3".to_string(), // appended, depth 1 (parent 2)
        ]);
    }

    // ── CaptureSink style-run capture ─────────────────────────────────────────

    #[test]
    fn capture_sink_records_style_runs() {
        use zvm::io::Output;
        use zvm::screen::ZColour;
        let mut s = CaptureSink::new();
        s.print("ab");
        s.print_styled("CD", 0x02);
        let (text, runs) = s.take_styled();
        assert_eq!(text, "abCD");
        assert_eq!(runs, vec![
            (2, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0),
            (2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0),
        ]);
    }

    #[test]
    fn interleave_story_pics_splits_at_line_starts_and_keeps_runs_synced() {
        use crate::inline_image::{ImageAlign, InlineImage};
        let img = InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(8, 8)),
            align: ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(56),
        };
        let text = "first line\nsecond line";
        // One style chunk covering everything (bold), to verify run splitting.
        let runs = vec![(text.chars().count(), 2u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0u8)];
        // Drawn at abs offset base+15 — mid-"second line" — must SNAP to that
        // line's start (offset 11), splitting cleanly at the line boundary.
        let elems = interleave_story_pics(text, &runs, vec![(115, img)], 100);
        assert_eq!(elems.len(), 3, "Text, Image, Text");
        let TranscriptElem::Text { text: t0, runs: r0 } = &elems[0] else { panic!("elem 0 is Text") };
        assert_eq!(t0, "first line", "separator dropped — element boundary is the break");
        assert_eq!(r0.iter().map(|r| r.0).sum::<usize>(), 10, "runs cover exactly the chunk");
        assert!(matches!(&elems[1], TranscriptElem::Image(i) if i.margin_px == Some(56)));
        let TranscriptElem::Text { text: t2, runs: r2 } = &elems[2] else { panic!("elem 2 is Text") };
        assert_eq!(t2, "second line");
        assert_eq!(r2.iter().map(|r| r.0).sum::<usize>(), 11, "tail runs cover the tail (separator char consumed)");
    }

    #[test]
    fn interleave_story_pics_at_start_needs_no_split() {
        use crate::inline_image::{ImageAlign, InlineImage};
        let img = InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(8, 8)),
            align: ImageAlign::MarginLeft,
            scaled: None,
            margin_px: None,
        };
        let elems = interleave_story_pics("story text", &[], vec![(0, img)], 0);
        assert_eq!(elems.len(), 2, "Image then Text");
        assert!(matches!(&elems[0], TranscriptElem::Image(_)));
        assert!(matches!(&elems[1], TranscriptElem::Text { text, .. } if text == "story text"));
    }

    #[test]
    fn clamp_runs_trims_to_char_len() {
        use zvm::screen::ZColour;
        // strip_read_prompt removed 3 trailing chars ("\n> " etc.) → clamp.
        let runs = vec![
            (2, 0u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0),
            (5, 0x02u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0),
        ];
        assert_eq!(clamp_runs(runs, 4), vec![
            (2, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0),
            (2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0),
        ]);
    }

    fn dummy_inline_image() -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(2, 2)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None ,
        }
    }

    #[test]
    fn trim_elems_strips_trailing_prompt_from_last_text() {
        use zvm::screen::ZColour;
        // raw ends in "\n> " — strip_read_prompt shortens it; the LAST Text
        // element (and its runs) must be trimmed to match the flat stripped text.
        let raw = "You see a rock.\n> ";
        let kept = strip_read_prompt(raw).chars().count();
        let mut elems = vec![TranscriptElem::Text {
            text: raw.to_string(),
            runs: vec![(raw.chars().count(), 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0)],
        }];
        trim_elems_to_len(&mut elems, kept);
        let TranscriptElem::Text { text, runs } = &elems[0] else { panic!("expected Text") };
        assert_eq!(text, "You see a rock.");
        assert_eq!(runs.iter().map(|r| r.0).sum::<usize>(), kept);
    }

    #[test]
    fn trim_elems_reaches_across_image_to_reach_length() {
        use zvm::screen::ZColour;
        // Text("foo\n"), Image, Text(">") — flat text "foo\n>" strips to "foo".
        // The trim clears the trailing ">" element and reaches back past the
        // image to trim the "\n" off "foo\n".
        let mut elems = vec![
            TranscriptElem::Text { text: "foo\n".into(), runs: vec![(4, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0)] },
            TranscriptElem::Image(dummy_inline_image()),
            TranscriptElem::Text { text: ">".into(), runs: vec![(1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0)] },
        ];
        trim_elems_to_len(&mut elems, 3);
        let TranscriptElem::Text { text, .. } = &elems[0] else { panic!("expected Text") };
        assert_eq!(text, "foo");
        assert!(matches!(&elems[1], TranscriptElem::Image(_)));
        let TranscriptElem::Text { text, .. } = &elems[2] else { panic!("expected Text") };
        assert_eq!(text, "");
    }

    // ── Pure bridge test ──────────────────────────────────────────────────────

    #[test]
    fn apply_turn_bridge_sets_current_and_creates_edge() {
        let mut m = Mapper::default();

        // First observation: set current room (no prior → no edge).
        let first = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 1, parent: 0, name: "Hall".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        apply_turn(&mut m, "look", &first);
        assert_eq!(m.graph.current(), Some(1));
        assert!(m.graph.room(1).is_some());
        assert_eq!(m.graph.connections().len(), 0, "first observe must not create edge");

        // Second observation: move north → directed N edge 1→2.
        let second = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 2, parent: 0, name: "Attic".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        apply_turn(&mut m, "north", &second);
        assert!(m.graph.room(2).is_some());
        assert_eq!(m.graph.current(), Some(2));

        let conns = m.graph.connections();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].origin, 1);
        assert_eq!(conns[0].dir, Direction::N);
        assert_eq!(conns[0].dest, 2);
    }

    #[test]
    fn apply_turn_noop_when_location_none() {
        let mut m = Mapper::default();
        let result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: None,
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        apply_turn(&mut m, "look", &result);
        assert_eq!(m.graph.current(), None);
    }

    #[test]
    fn is_death_relocation_matches_infocom_and_inform_banners_only() {
        // Infocom's spaced banner (verified against a real Zork I grue death).
        assert!(is_death_relocation(
            "Oh, no! A lurking grue slithered into the room and devoured you!\n \n   ****  You have died  **** \n\nForest\n"
        ));
        // Inform's tight banner.
        assert!(is_death_relocation("*** You have died ***"));
        // The winning banner changes no room — must NOT be treated as a relocation.
        assert!(!is_death_relocation("*** You have won ***"));
        // The pitch-black warning (a legit move) has no banner — must NOT match.
        assert!(!is_death_relocation(
            "It is pitch black. You are likely to be eaten by a grue."
        ));
        // Ordinary room prose mentioning the dead must NOT match.
        assert!(!is_death_relocation("A dead body lies in the corner of the crypt."));
    }

    #[test]
    fn apply_turn_death_records_relocation_not_a_directional_edge() {
        // A typed "north" that triggers a grue death + resurrection into Forest must
        // NOT mint a false N-edge Cellar→Forest. (SQ-0259)
        let mk = |num: u16, name: &str, transcript: &str| TurnResult {
            transcript: transcript.into(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: num, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        let mut m = Mapper::default();
        apply_turn(&mut m, "", &mk(1, "Living Room", "Living Room\n"));
        apply_turn(&mut m, "down", &mk(2, "Cellar", "You have moved into a dark place.\n"));
        let edges_before = m.graph.connections().len();
        // The fatal move: resurrection room arrives on the same turn as the banner.
        apply_turn(&mut m, "north", &mk(3, "Forest", "   ****  You have died  **** \n\nForest\n"));
        assert_eq!(m.graph.current(), Some(3), "player is now in the resurrection room");
        assert_eq!(
            m.graph.connections().len(),
            edges_before,
            "the death move must not add any edge (no false Cellar→Forest passage)"
        );
        assert!(
            !m.graph.connections().iter().any(|c| c.origin == 2 && c.dest == 3),
            "no edge from the room we died in to the resurrection room"
        );
    }

    #[test]
    fn apply_turn_gates_nameonly_until_first_real_room() {
        // BeyondZork VT220 setup shows the player's name ("Frank Booth") in a
        // status-line-shaped character sheet → NameOnly. It must NOT seed the
        // map before real play establishes an object-backed room.
        let mk = |method: Option<LocationMethod>, num: u16, name: &str| TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: num, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: method,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };

        let mut m = Mapper::default();

        // 1. Pre-game NameOnly on an empty map → suppressed.
        apply_turn(&mut m, "", &mk(Some(LocationMethod::NameOnly), 111, "Frank Booth"));
        assert_eq!(m.graph.rooms().count(), 0, "NameOnly must not seed an empty map");
        assert_eq!(m.graph.current(), None);

        // 2. Real play: an object-backed room is observed.
        apply_turn(&mut m, "", &mk(Some(LocationMethod::PlayerParent), 48, "Hilltop"));
        assert_eq!(m.graph.current(), Some(48));
        assert_eq!(m.graph.rooms().count(), 1);

        // 3. NameOnly is now trusted as a mid-game fallback (map non-empty).
        apply_turn(&mut m, "north", &mk(Some(LocationMethod::NameOnly), 222, "Foggy Place"));
        assert_eq!(m.graph.current(), Some(222));
        assert_eq!(m.graph.rooms().count(), 2);
    }

    #[test]
    fn apply_turn_observes_roomheading_on_empty_map() {
        // Glulx rooms use RoomHeading (never NameOnly) precisely so the
        // NameOnly-empty-graph gate does NOT suppress the first Glulx room —
        // a Glulx game never produces an object-backed room to un-gate it.
        let mut m = Mapper::default();
        let result = TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number: 333, parent: 0, name: "Orbiting Boony".into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: Some(LocationMethod::RoomHeading),
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        apply_turn(&mut m, "", &result);
        assert_eq!(m.graph.current(), Some(333));
        assert_eq!(m.graph.rooms().count(), 1);
    }

    // ── TurnResult.info tests ─────────────────────────────────────────────────

    #[test]
    fn turn_result_info_defaults_none_for_normal_turn() {
        // A TurnResult from a normal turn has info == None by default.
        let r = TurnResult {
            transcript: "You are in a maze.".to_string(),
            transcript_runs: Vec::new(),
            location: None,
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        };
        assert!(r.info.is_none());
    }

    // ── Task-5 overlap cleanup tests ──────────────────────────────────────────

    /// Helper: build a TurnResult with a location (mirrors the pattern used above).
    fn turn(number: u16, name: &str) -> TurnResult {
        TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: Some(ObjectSnapshot { number, parent: 0, name: name.into() }),
            quit: false,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        }
    }

    #[test]
    fn auto_mode_background_cleanup_keeps_map_free_of_illegal_overlaps() {
        // Drive a small loop (E, N, W, S toward start) that — under incremental
        // placement — can produce a routing overlap.  `apply_turn` no longer cleans
        // overlaps inline (that is background map work now); running the background
        // cleanup the run loop schedules must leave zero illegal overlaps.
        let mut m = Mapper::default(); // Auto mode by default

        apply_turn(&mut m, "look",  &turn(1, "Start"));
        apply_turn(&mut m, "east",  &turn(2, "East Room"));
        apply_turn(&mut m, "north", &turn(3, "North East Room"));
        apply_turn(&mut m, "west",  &turn(4, "North Room"));
        apply_turn(&mut m, "south", &turn(1, "Start")); // back to start — closes the loop

        crate::tidy::cleanup_overlaps_layer_silent(&mut m.graph, mapper::layer::MAIN_LAYER);

        let (illegal, _) = crate::render::map::render_overlap_stats(&m.graph);
        assert_eq!(illegal, 0, "background cleanup must leave zero illegal overlaps");
    }

    #[test]
    fn manual_mode_does_not_move_previously_placed_rooms() {
        use mapper::layout::LayoutMode;

        let mut m = Mapper::default();
        // Place two rooms in Auto mode so they get positions.
        apply_turn(&mut m, "look",  &turn(1, "Hall"));
        apply_turn(&mut m, "north", &turn(2, "Attic"));

        // Record positions before switching to Manual.
        let pos1_before = m.graph.room(1).unwrap().pos;
        let pos2_before = m.graph.room(2).unwrap().pos;

        // Switch to Manual: cleanup must NOT run on subsequent apply_turn calls.
        m.set_mode(LayoutMode::Manual);

        // Observe a new room; this must not move the already-placed rooms.
        apply_turn(&mut m, "east", &turn(3, "East Room"));

        assert_eq!(m.graph.room(1).unwrap().pos, pos1_before, "room 1 must not move in Manual mode");
        assert_eq!(m.graph.room(2).unwrap().pos, pos2_before, "room 2 must not move in Manual mode");
    }

    // ── Task 7: InputKind / submit_char tests ─────────────────────────────────

    /// Build a minimal v5 story whose program is: read_char (store→G0), quit.
    ///
    /// GameSession::new will step until the first NeedChar, so pending_input()
    /// must be `Char`.  Calling submit_char advances past the read_char and
    /// hits the quit opcode, returning a TurnResult.
    fn read_char_story_v5() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        // Version 5
        buf[0x00] = 5;
        // high_mem_base = 0x0400
        buf[0x04] = 0x04; buf[0x05] = 0x00;
        // initial_pc = 0x0040
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        // object_table = 0x0100
        buf[0x0A] = 0x01; buf[0x0B] = 0x00;
        // global_vars = 0x0300
        buf[0x0C] = 0x03; buf[0x0D] = 0x00;
        // static_mem_base = 0x0400 → dynamic memory 0x0000–0x03FF
        buf[0x0E] = 0x04; buf[0x0F] = 0x00;
        // abbrev_table = 0x0060
        buf[0x18] = 0x00; buf[0x19] = 0x60;

        // Program at 0x0040:
        //   read_char (VAR opcode 0xF6)
        //     type byte 0x7F: small-const(01), omit(11), omit(11), omit(11)
        //     operand: 1 (keyboard device)
        //     store: 0x10 (G0)
        //   quit (0xBA)
        buf[0x0040] = 0xF6; // VAR read_char
        buf[0x0041] = 0x7F; // type: small(01), omit(11), omit(11), omit(11)
        buf[0x0042] = 1;    // operand: device=1
        buf[0x0043] = 0x10; // store → G0
        buf[0x0044] = 0xBA; // quit

        buf
    }

    /// Timed variant of `read_char_story_v5`: `read_char(device=1, time=5,
    /// routine=packed(0x0050))` -> G0, then `quit`. `routine_body` is placed at
    /// 0x0050 (0 locals) so the caller can make it `rtrue` (abort) or do a
    /// side-effect + `rfalse` (continue). Packed routine address = 0x0050/4 =
    /// 0x0014 (v5 packed multiplier is 4).
    fn timed_read_char_story_v5(routine_body: &[u8]) -> Vec<u8> {
        let mut buf = read_char_story_v5();
        // Program at 0x0040:
        //   read_char (VAR opcode 0xF6)
        //     type byte 0x53: small(01)=device, small(01)=time, large(00)=routine, omit(11)
        //     operands: device=1, time=5, routine=packed(0x0050)=0x0014
        //     store: 0x10 (G0)
        //   quit (0xBA)
        buf[0x0040] = 0xF6; // VAR read_char
        buf[0x0041] = 0x53; // types: small, small, large, omit
        buf[0x0042] = 1;    // device = 1 (keyboard)
        buf[0x0043] = 5;    // time = 5 (tenths of a second)
        buf[0x0044] = 0x00;
        buf[0x0045] = 0x14; // routine packed addr = 0x0050 / 4
        buf[0x0046] = 0x10; // store → G0
        buf[0x0047] = 0xBA; // quit

        // Routine at 0x0050: header byte = 0 locals, then routine_body.
        buf[0x0050] = 0x00;
        for (i, b) in routine_body.iter().enumerate() {
            buf[0x0051 + i] = *b;
        }
        buf
    }

    #[test]
    fn pending_input_is_line_after_new_on_quitting_story() {
        // czech.z5 quits without ever requesting input; the quit path in
        // run_until_input returns InputKind::Line, so pending_input() == Line.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read czech.z5");
        let session = GameSession::new(story, true, false, None).expect("GameSession::new with czech.z5");
        assert_eq!(session.pending_input(), InputKind::Line,
            "a story that quits without requesting input should leave pending == Line");
    }

    #[test]
    fn v5_start_room_detected_at_boot() {
        // Regression: v4+ games have no location global, so `current_location`
        // must use version-aware detection at boot. With the old global-0 read it
        // returned None for v5 and the starting room stayed off the map until the
        // first turn. Skips when the (git-ignored) story is absent.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/zork1-invclues-r52-s871125.z5");
        if !path.exists() {
            return; // story absent — skip
        }
        let story = std::fs::read(&path).expect("read zork1 r52");
        let session = GameSession::new(story, false, false, None).expect("GameSession::new");
        let loc = session.current_location().expect("v5 starting room must be detected at boot");
        assert!(loc.name.starts_with("West"), "expected West of House, got {:?}", loc.name);
    }

    #[test]
    fn new_with_trace_captures_boot_pcs() {
        // --debug (SQ-0449) traces from the first boot instruction, so the
        // cumulative set is non-empty even before any player turn — capturing the
        // boot/init code a mid-game /debug can never see.
        let story = read_char_story_v5();
        let traced = GameSession::new_with_trace(story.clone(), false, false, None, true, Vec::new(), None)
            .expect("traced session");
        assert!(!traced.machine.ever_exec_pcs.is_empty(),
            "boot PCs must be captured when tracing from boot");
        // Without tracing, the cumulative set stays empty until a traced turn runs.
        let untraced = GameSession::new(story, false, false, None).expect("untraced session");
        assert!(untraced.machine.ever_exec_pcs.is_empty(),
            "no capture without --debug");
    }

    /// Build a minimal v6 story whose "main" routine (header 0x06/0x07, a packed
    /// routine address per ZMSD §5.5) is `quit` with 0 locals. Just enough for
    /// `Machine::with_output`'s v6 arm (which calls `main` via `call_routine`
    /// before the boot loop runs) to construct without faulting.
    fn v6_boot_stub_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 6; // version
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        // header 0x06/0x07 = main's packed address. routines_offset (0x28/0x29)
        // is 0, so unpack_routine(p) = 4*p; routine at 0x0100 -> packed 0x0040.
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dictionary = 0x0080 (empty: word-sep=0, entry-size=4, entry-count=0)
        buf[0x08] = 0x00; buf[0x09] = 0x80;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100 (unused by this stub)
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        // main routine at 0x0100: 0 locals, then `quit` (0OP:0x0A, opcode byte 0xBA).
        buf[0x0100] = 0; // local count
        buf[0x0101] = 0xBA; // quit
        buf
    }

    #[test]
    fn v6_session_injects_picture_dims_before_boot() {
        // The v6 picture-dimension table must be set on `Machine` BEFORE the
        // boot run (picture_data is called during boot, which happens inside
        // new_with_trace itself — the Phase 0 boot-tracing lesson), so it must
        // be visible on the constructed session even for a story that quits
        // immediately in its main routine.
        let dims = vec![(5u16, 100u16, 60u16), (9u16, 20u16, 30u16)];
        let session = GameSession::new_with_trace(v6_boot_stub_story(), false, false, None, false, dims.clone(), None)
            .expect("v6 session");
        assert_eq!(session.machine.picture_dims, dims);
    }

    /// Variant of `read_char_story_v5`: after the read_char completes, instead
    /// of `quit` the program executes a `loadw` with an out-of-bounds address
    /// (array=0xFFFF, index=0xFFFF), which faults the VM mid-turn (ZMSD memory
    /// fault). Mirrors zvm's own `loadw_out_of_bounds_faults_with_trace` test.
    fn faulting_read_char_story_v5() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        // loadw (2OP:0x0F), variable-form encoding with two Large operands:
        // array=0xFFFF index=0xFFFF -> addr = 0xFFFF + 2*0xFFFF, far past this
        // 0x0800-byte story's memory.
        buf[0x0044] = 0xCF; // variable form, bit5=0 -> 2OP, opcode=0x0F (loadw)
        buf[0x0045] = 0x0F; // type byte: large, large, omitted, omitted
        buf[0x0046] = 0xFF; buf[0x0047] = 0xFF; // operand a (array) = 0xFFFF
        buf[0x0048] = 0xFF; buf[0x0049] = 0xFF; // operand b (index) = 0xFFFF
        buf[0x004A] = 0x00; // store var 0x00 = push onto stack
        buf
    }

    #[test]
    fn turn_result_carries_fault_trace_when_vm_faults() {
        // End-to-end: submit a turn whose VM step faults mid-execution and
        // confirm the drained TurnResult.fault carries the formatted trace.
        let mut sess = GameSession::new(faulting_read_char_story_v5(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(sess.pending_input(), InputKind::Char);

        let turn_result = sess.submit_char(b'x');
        assert!(turn_result.quit, "a faulted VM halts (routed through RunStop::Quit)");
        let lines = turn_result.fault.expect("TurnResult.fault must be Some after a VM fault");
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert!(lines[1].starts_with("memory fault: read16 @"), "fault line: {}", lines[1]);
    }

    #[test]
    fn pending_input_is_char_after_new_on_read_char_story() {
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char,
            "GameSession::new on a read_char story should leave pending == Char");
    }

    #[test]
    fn game_session_take_screen_trace_drains_when_enabled() {
        // No handy fixture issues screen opcodes on turn one, so this asserts
        // the drain plumbing directly: set_trace_screen wires to the machine's
        // flag, and take_screen_trace drains screen_trace exactly once.
        let mut s = GameSession::new(read_char_story_v5(), true, false, None)
            .expect("GameSession::new failed");
        s.set_trace_screen(true);
        assert!(s.machine.trace_screen, "set_trace_screen(true) reaches the machine");
        s.machine.screen_trace.push("@set_colour(fg=std5, bg=std2)".to_string());
        let lines = s.take_screen_trace();
        assert!(lines.iter().any(|l| l.starts_with("@")), "{lines:?}");
        assert!(s.take_screen_trace().is_empty(), "second drain is empty");
    }

    #[test]
    fn session_surfaces_timeout_and_aborts_via_run_timed_interrupt() {
        // Interrupt routine: rtrue (0xB0) -> aborts the pending read_char.
        let bytes = timed_read_char_story_v5(&[0xB0]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_input(), InputKind::Char);
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)), "time+packed routine surfaced");

        let tr = s.run_timed_interrupt();
        assert!(tr.timed_out, "routine returned true -> the read was aborted");
        // abort_timed_input completes the read_char (stores 0) and the story
        // immediately hits quit.
        assert!(tr.quit, "story quits right after the aborted read_char");
        assert_eq!(s.pending_timeout(), None, "no read pending once the story has quit");
    }

    #[test]
    fn session_run_timed_interrupt_continues_when_routine_returns_false() {
        // Interrupt routine: inc G1 (0x95, 0x11), then rfalse (0xB1) -> the read
        // stays pending; the host is expected to keep waiting.
        let bytes = timed_read_char_story_v5(&[0x95, 0x11, 0xB1]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)));
        let g_before = s.machine.global(1);

        let tr = s.run_timed_interrupt();
        assert!(!tr.timed_out, "routine returned false -> read still pending");
        assert!(!tr.quit, "read_char has not been completed yet");
        assert_eq!(s.pending_input(), InputKind::Char, "read_char is still the pending input");
        assert_eq!(s.pending_timeout(), Some((5, 0x0014)), "timer stays armed for the next tick");
        assert_eq!(s.machine.global(1), g_before.wrapping_add(1), "routine side effect applied");
    }

    #[test]
    fn abort_timed_input_marks_timed_out_and_advances() {
        // Directly abort a timed read_char (bypassing run_timed_interrupt) and
        // confirm the TurnResult is flagged and the VM advances past the read.
        let bytes = timed_read_char_story_v5(&[0xB0]);
        let mut s = GameSession::new(bytes, true, false, None).expect("GameSession::new");
        assert_eq!(s.pending_input(), InputKind::Char);

        let tr = s.abort_timed_input("");
        assert!(tr.timed_out, "abort_timed_input always marks timed_out");
        assert!(tr.quit, "story quits right after the aborted read_char");
    }

    #[test]
    fn run_sound_finish_returns_turn_result() {
        // Reuse the char-mode fixture: run_sound_finish drives run_routine then
        // collects a TurnResult without stepping the read forward. Passing a 0
        // (bad/no routine) still returns a well-formed TurnResult (no panic).
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        let r = sess.run_sound_finish(0);
        assert!(r.sounds.is_empty(), "no new sounds from a finish callback");
        assert!(!r.quit, "a no-op finish routine does not quit");
    }

    #[test]
    fn new_applies_interpreter_override() {
        // read_char_story_v5 is a v5 story; default would be 1, override to 4.
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, Some(4)).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 4, "override advertised");
    }

    #[test]
    fn new_default_interpreter_is_dec20() {
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, false, None).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 1, "v5 default = DEC-20 (1)");
    }

    #[test]
    fn new_session_forwards_sound_available() {
        // GameSession::new must forward sound_available to Machine::set_sound_available
        // (mirrors honor_game_colours), so the game sees the capability from turn 1.
        let session_on = GameSession::new(read_char_story_v5(), true, true, None).expect("GameSession::new");
        assert!(session_on.machine.sound_available, "sound_available(true) must forward to the Machine");

        let session_off = GameSession::new(read_char_story_v5(), true, false, None).expect("GameSession::new");
        assert!(!session_off.machine.sound_available, "sound_available(false) must forward to the Machine");
    }

    #[test]
    fn submit_char_returns_turn_result_and_advances() {
        let story = read_char_story_v5();
        let mut session = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char);

        // After read_char the next instruction is quit, so submit_char drives
        // the machine to Quit → TurnResult.quit == true.
        let result = session.submit_char(b'x');
        assert!(result.quit, "submit_char on a read_char→quit story should return quit=true");

        // The quit path sets pending back to Line (no input pending).
        assert_eq!(session.pending_input(), InputKind::Line,
            "after quit, pending should be reset to Line");
    }

    // ── Engine adapter (zvm) tests ─────────────────────────────────────────────

    /// Build the v3 variant of the read_char story (so screen() yields a v3
    /// automatic status line).
    fn read_char_story_v3() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 3;
        buf
    }

    #[test]
    fn key_input_to_zscii_matches_legacy_mapping() {
        // Core text keys: unchanged from original mapping.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Enter), Some(13));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Backspace), Some(8));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Escape), Some(27));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('y')), Some(b'y'));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('x')), Some(120));
        // Non-ASCII printable chars carry no ZSCII byte (skip the turn).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('\u{00E9}')), None);
        // Arrow keys now map to ZSCII cursor codes (ZMSD §3.8) so read_char works.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Up),    Some(129));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(1)), Some(133));
    }

    #[test]
    fn engine_submit_key_drives_turn_for_mapped_key() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);
        // 'x' → read_char → quit.
        let r = sess.submit_key(KeyInput::Char('x'));
        assert!(r.is_some(), "a mapped key produces a turn");
        assert!(r.unwrap().quit);
    }

    #[test]
    fn engine_submit_key_is_noop_for_unmapped_key() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        // Home has no ZSCII meaning: no turn runs, the VM stays waiting.
        assert!(sess.submit_key(KeyInput::Home).is_none());
        assert_eq!(sess.pending_input(), InputKind::Char, "VM untouched by an unmapped key");
    }

    #[test]
    fn zmachine_take_transcript_elems_is_empty() {
        // The Z-machine has no inline images, so it keeps the trait DEFAULT:
        // `take_transcript_elems` returns empty (draining nothing), and callers
        // fall back to the flat `take_transcript` string path. This guarantees
        // the banner/startup dispatch is byte-identical to the pre-feature path.
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert!(
            sess.take_transcript_elems().is_empty(),
            "zvm uses the default empty elems; the string path stays authoritative",
        );
        // The default elems method drained nothing: the banner string is identical
        // to a fresh session that never called take_transcript_elems.
        let mut fresh = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert_eq!(
            sess.take_transcript(),
            fresh.take_transcript(),
            "take_transcript_elems must not consume the banner for the Z-machine",
        );
    }

    #[test]
    fn take_transcript_respects_strip_prompt_flag() {
        // strip_prompt gates whether the game's trailing "> " read prompt is
        // removed from the transcript (SQ-0264: inline-prompt mode keeps it).
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let _ = sess.take_transcript(); // drain the banner

        sess.strip_prompt = false;
        sink_mut(&mut sess.machine).print("You are in a room.\n>");
        assert_eq!(
            sess.take_transcript(),
            "You are in a room.\n>",
            "strip_prompt=false keeps the game's trailing '>'"
        );

        sess.strip_prompt = true;
        sink_mut(&mut sess.machine).print("You are in a room.\n>");
        assert_eq!(
            sess.take_transcript(),
            "You are in a room.",
            "strip_prompt=true (default) strips the trailing '>'"
        );
    }

    #[test]
    fn engine_screen_v3_is_classic_status() {
        let sess = GameSession::new(read_char_story_v3(), true, false, None).expect("new v3");
        let model = sess.screen();
        match model.status {
            StatusModel::Classic { right, .. } => {
                // Default flags (bit 1 = 0) → score/turns form.
                assert!(matches!(right, StatusField::ScoreTurns { .. }));
            }
            other => panic!("v3 must yield a Classic status, got {other:?}"),
        }
        // The tree still carries a grid (the upper window) over a buffer.
        assert!(model.grid().is_some(), "screen tree exposes a grid node");
    }

    #[test]
    fn engine_screen_v5_is_host_managed_and_mirrors_upper_grid() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new v5");
        // Paint the upper window directly and confirm screen() mirrors it exactly.
        sess.machine.screen.upper.resize(2, 5);
        sess.machine.screen.upper.put(1, 1, 'H', 2, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default); // bold
        sess.machine.screen.upper.put(1, 2, 'I', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        sess.machine.screen.upper_window_rows = 2;
        sess.machine.screen.cursor_row = 1;
        sess.machine.screen.cursor_col = 3;
        sess.machine.screen.current_window = 1;

        let model = sess.screen();
        assert_eq!(model.status, StatusModel::HostManaged, "v4+ has no automatic status");
        let g = model.grid().expect("grid node");
        assert_eq!((g.cols, g.rows), (5, 2));
        assert_eq!(g.active_rows, 2);
        assert_eq!(g.cell(1, 1).ch, 'H');
        assert_eq!(g.cell(1, 1).style, 2);
        assert_eq!(g.cell(1, 2).ch, 'I');
        assert_eq!(g.cursor, (1, 3));
        assert!(g.cursor_active, "current_window == 1 marks the grid active");
    }

    #[test]
    fn engine_save_state_round_trips_and_is_tagged() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let save = sess.save_state();
        assert_eq!(save.engine, ZMACHINE_ENGINE);
        assert!(!save.bytes.is_empty(), "Quetzal save is non-empty");

        // Advance the VM, then restore the captured state.
        let _ = sess.submit_key(KeyInput::Char('x'));
        sess.restore_state(&save).expect("same-engine restore succeeds");

        // A foreign-engine save is refused.
        let foreign = EngineSave::new("glulx", 1, save.bytes.clone());
        match sess.restore_state(&foreign) {
            Err(EngineError::EngineMismatch { expected, found }) => {
                assert_eq!(expected, ZMACHINE_ENGINE);
                assert_eq!(found, "glulx");
            }
            other => panic!("foreign-engine restore must be refused, got {other:?}"),
        }
    }

    #[test]
    fn engine_introspect_wraps_existing_logic() {
        let sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        let intro = sess.introspect().expect("zvm exposes introspection");
        // vocabulary == today's dictionary load.
        let vocab = intro.vocabulary();
        let expected = zvm::dictionary::load(&sess.machine.mem).words(&sess.machine.mem);
        assert_eq!(vocab, expected);
        // player_object == today's find_player_object.
        assert_eq!(intro.player_object(), zvm::find_player_object(&sess.machine));
    }

    #[test]
    fn engine_aux_data_accessors_round_trip() {
        let mut sess = GameSession::new(read_char_story_v5(), true, false, None).expect("new");
        assert!(sess.aux_data().is_empty());
        let mut table = std::collections::BTreeMap::new();
        table.insert("k".to_string(), vec![1u8, 2, 3]);
        sess.set_aux_data(table.clone());
        assert_eq!(sess.aux_data(), &table);
        sess.machine.aux_dirty = true;
        assert!(sess.aux_dirty());
        sess.clear_aux_dirty();
        assert!(!sess.aux_dirty());
    }

    // ── In-game save/restore plumbing (v4) ─────────────────────────────────────
    //
    // read_char_story_v5 lays out: 0x40 read_char->G0 (4 bytes), 0x44 quit.
    // We re-stamp it to v4 and overwrite the quit at 0x44 with the save/restore
    // opcode so the FIRST keypress drives read_char -> the opcode.
    fn read_char_then_save_v4() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 4;    // version 4 (0OP save/restore store form lives here)
        buf[0x44] = 0xB5; // 0OP:0x05 save (store form) -> G0
        buf[0x45] = 0x10; // store byte: global 0
        buf[0x46] = 0xBA; // quit
        buf
    }

    fn read_char_then_restore_v4() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        buf[0x00] = 4;
        buf[0x44] = 0xB6; // 0OP:0x06 restore (store form) -> G0
        buf[0x45] = 0x10; // store byte: global 0
        buf[0x46] = 0xBA; // quit
        buf
    }

    #[test]
    fn ingame_save_yields_pending_io_and_resume_continues() {
        let mut sess = GameSession::new(read_char_then_save_v4(), true, false, None).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);

        // The keypress drives read_char -> @save, which suspends with pending_io.
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        assert!(!r.quit, "a save-pending turn is not a quit");
        assert!(r.info.is_none(), "v4+ in-game save shows no 'isn't wired' info line");

        // Host wrote the file OK: resume stores 1 into G0 and runs to quit.
        let r2 = sess.resume_save(true);
        assert!(r2.quit, "resume_save continues the VM to the quit opcode");
        assert_eq!(sess.machine.global(0), 1, "complete_save(true) stored 1 into G0");
    }

    #[test]
    fn ingame_restore_yields_pending_io_and_cancel_fails_cleanly() {
        let mut sess = GameSession::new(read_char_then_restore_v4(), true, false, None).expect("new");

        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore));
        assert!(!r.quit);

        // Cancel: resume_restore(None) -> complete_restore_failure stores 0, runs on.
        let r2 = sess.resume_restore(None);
        assert!(r2.quit);
        assert_eq!(sess.machine.global(0), 0, "cancelled restore stored 0 into G0");
    }

    #[test]
    fn v3_ingame_save_and_restore_bubble_pending_io() {
        // v3 @save/@restore are BRANCH instructions (0OP:0x05/0x06 = 0xB5/0xB6 +
        // 1 branch byte). After the standard-PC fix they bubble pending_io like v4+.
        let mut save_buf = read_char_story_v5();
        save_buf[0x00] = 3;              // version 3 (branch form)
        save_buf[0x44] = 0xB5;           // 0OP:0x05 save (branch form)
        save_buf[0x45] = 0x80 | 0x40 | 2; // branch on-true, short form, offset 2 -> quit at 0x46
        save_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(save_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save), "v3 in-game save now bubbles pending_io");
        assert!(r.info.is_none(), "no 'isn't wired' info line for v3 anymore");
        let r2 = sess.resume_save(true);
        assert!(r2.quit, "resume_save completes the branch and runs to quit");

        let mut restore_buf = read_char_story_v5();
        restore_buf[0x00] = 3;
        restore_buf[0x44] = 0xB6;           // 0OP:0x06 restore (branch form)
        restore_buf[0x45] = 0x80 | 0x40 | 2; // branch byte (unused on cancel)
        restore_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(restore_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "v3 in-game restore now bubbles pending_io");
        let r2 = sess.resume_restore(None);
        assert!(r2.quit, "cancelled v3 restore falls through to quit");
    }

    #[test]
    fn turn_result_carries_location_method_field() {
        // Build the same way the sibling submit test does; the field just needs to exist
        // and default to a value. For a v3 fixture with global 0 set, method is GlobalVar0.
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        // The field exists and is an Option<LocationMethod>; on a v5 story with no
        // location it is None — either is acceptable here.
        let _ = r.location_method;
    }

    #[test]
    fn turn_result_has_empty_sound_fields_by_default() {
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story, true, false, None).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        assert!(r.sounds.is_empty(), "no sounds when the game emits no sound");
        assert!(r.diagnostics.is_empty(), "no diagnostics on a clean turn");
        // VM queues are drained after the turn.
        assert!(sess.machine.pending_sounds.is_empty());
        assert!(sess.machine.diagnostics.is_empty());
    }

    // ── Plan 1b Task 2: pending_pictures → per-window canvases ────────────────

    /// A minimal v6 story buffer: version 6, header 0x06/0x07 (main's packed
    /// address) left at 0 so `Machine::with_output`'s v6 boot path
    /// (`call_routine` on the unpacked address) reads byte 0 (the version byte,
    /// 6) as a harmless in-range "locals count" — this test never steps the VM,
    /// so the routine is never actually executed. Mirrors the header layout of
    /// `inventory.rs`'s `sample_story_v3` shim (zvm's own `tests_support` is
    /// crate-private).
    fn minimal_v6_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x1000];
        buf[0x00] = 6;                       // version = 6
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00; buf[0x07] = 0x00; // main's packed addr = 0
        buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary = 0x0200
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        buf
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate (mirrors
    /// `graphics.rs`'s private test helper of the same shape).
    fn png_bytes_2x2_red() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn drain_turn_applies_pending_draw_picture_to_the_window_canvas() {
        use zvm::screen::{V6Windows, ZWindow};

        // A v6 machine with window 7 sized 64x48px, current window = 7, and one
        // pending draw_picture(number=1, window=7, x=2, y=3) event — as if
        // `exec_ext(0x05, ...)` had just run (Task 1/Plan 1a).
        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None });

        // Construct the session directly (bypassing the constructor's boot
        // loop, which this synthetic story can't usefully run) with a Pict
        // source that resolves resource #1 to the red 2x2 PNG.
        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true,
            disasm_cache: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        assert!(sess.pictures_canvas.is_empty(), "no canvas before the turn is drained");
        let result = sess.drain_turn(false, None, false);

        assert_eq!(result.pictures, vec![PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None }],
            "the drained event is carried on TurnResult (mirrors pending_sounds)");
        assert!(sess.machine.pending_pictures.is_empty(), "the VM queue is drained after the turn");

        let canvas = sess.pictures_canvas.get(&7).expect("a canvas was created for window 7");
        assert_eq!(canvas.img.dimensions(), (64, 48), "canvas sized from the v6 window's pixel dims");
        assert_ne!(canvas.img.get_pixel(2, 3).0, [0, 0, 0, 0], "the picture was drawn (non-blank at its origin)");
        assert_eq!(canvas.img.get_pixel(2, 3).0, [0xFF, 0x00, 0x00, 0xFF], "the drawn pixel is the source PNG's red");
        // Outside the drawn 2x2 picture the canvas stays at its transparent default.
        assert_eq!(canvas.img.get_pixel(0, 0).0, [0, 0, 0, 0], "untouched region stays transparent");
    }

    #[test]
    fn drain_turn_applies_pending_erase_picture_to_the_window_canvas() {
        use zvm::screen::{V6Windows, ZWindow};

        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 64, y_size: 48, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        // Draw, then erase the same picture — the erase must clear back to
        // transparent over the picture's own footprint (2x2, ZMSD §15).
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: false, out_chars: 0, margin_after: None });
        machine.pending_pictures.push(PictureEvent { number: 1, window: 7, x: 2, y: 3, erase: true, out_chars: 0, margin_after: None });

        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes_2x2_red());
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true,
            disasm_cache: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
        };
        sess.set_pict_source(Some(crate::graphics::PictSource::new(Some(blorb))));

        let result = sess.drain_turn(false, None, false);
        assert_eq!(result.pictures.len(), 2);
        let canvas = sess.pictures_canvas.get(&7).expect("a canvas was created for window 7");
        assert_eq!(canvas.img.get_pixel(2, 3).0, [0, 0, 0, 0], "erased back to transparent");
    }

    // ── Plan 1b Task 4: v6 layered screen-model adapter ───────────────────────

    #[test]
    fn v6_screen_returns_layered_model_graphics_first_then_text_by_window_number() {
        use crate::engine::GraphicsWindow;
        use zvm::screen::{V6Windows, ZWindow};

        let mem = Memory::new(minimal_v6_story()).expect("minimal v6 story");
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));

        let mut windows: [ZWindow; 8] = Default::default();
        // Window coords are the spec's 1-based pixels ((1,1) = top-left).
        // Window 0: the main scrolling window, at (0, 1) cell, 80x20 cells.
        // attributes 15 = the boot default (wrapping on → transcript Buffer;
        // a cleared wrapping bit would mean positioned paint mode → Grid).
        windows[0] = ZWindow { x_coord: 1, y_coord: 9, x_size: 640, y_size: 160, attributes: 15, ..Default::default() };
        windows[0].grid.resize(20, 80);
        // Window 1: a one-row status strip along the top, at (0, 0) cell, 80x1 cells.
        windows[1] = ZWindow { x_coord: 1, y_coord: 1, x_size: 640, y_size: 8, ..Default::default() };
        windows[1].grid.resize(1, 80);
        // Window 7: a small picture window at (2, 1) cell, 8x6 cells.
        windows[7] = ZWindow { x_coord: 17, y_coord: 9, x_size: 64, y_size: 48, ..Default::default() };
        windows[7].grid.resize(6, 8);
        machine.screen.v6 = Some(V6Windows { windows, current: 1 });

        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true,
            disasm_cache: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
        };
        // Window 7 has a rendered picture (a canvas sized to its pixel dims).
        sess.pictures_canvas.insert(7, crate::graphics::Canvas::new(64, 48));

        let model = sess.screen();
        assert_ne!(model.content_size, (0, 0), "v6 always reports a nonzero content size");
        assert_eq!(model.content_size, (80, 21), "max right/bottom cell extent across the windows");

        let items = match model.root {
            WinNode::Layered(items) => items,
            other => panic!("expected WinNode::Layered, got {other:?}"),
        };

        // z-order: graphics entries first (window 7's picture), then text
        // windows by ascending window number (0, then 1, then 7's own grid).
        assert_eq!(items.len(), 4, "graphics(7) + buffer(0) + grid(1) + grid(7)");

        let g7 = &items[0];
        assert_eq!((g7.x, g7.y, g7.w, g7.h), (2, 1, 8, 6), "window 7's absolute cell rect (pixel / 8)");
        match &g7.node {
            WinNode::Graphics(GraphicsWindow { win, .. }) => assert_eq!(*win, 7),
            other => panic!("expected window 7's Graphics leaf first (background), got {other:?}"),
        }

        let w0 = &items[1];
        assert_eq!((w0.x, w0.y, w0.w, w0.h), (0, 1, 80, 20), "window 0's absolute cell rect (pixel / 8)");
        match &w0.node {
            WinNode::Buffer(b) => assert!(b.primary, "window 0 is the primary scrolling buffer"),
            other => panic!("expected window 0's Buffer leaf, got {other:?}"),
        }

        let w1 = &items[2];
        assert_eq!((w1.x, w1.y, w1.w, w1.h), (0, 0, 80, 1), "window 1's absolute cell rect (pixel / 8)");
        match &w1.node {
            WinNode::Grid(g) => assert_eq!((g.cols, g.rows), (80, 1)),
            other => panic!("expected window 1's Grid leaf, got {other:?}"),
        }

        let w7 = &items[3];
        assert_eq!((w7.x, w7.y, w7.w, w7.h), (2, 1, 8, 6), "window 7's own (blank) text grid, same rect as its Graphics leaf");
        match &w7.node {
            WinNode::Grid(g) => assert_eq!((g.cols, g.rows), (8, 6)),
            other => panic!("expected window 7's Grid leaf, got {other:?}"),
        }
    }

    #[test]
    fn v6_picture_canvas_clamps_hostile_window_size() {
        use zvm::cpu::exec::PictureEvent;
        use zvm::screen::{V6Windows, ZWindow};
        // A window sized to the pixel max must not force a ~17 GB canvas alloc.
        let mem = Memory::new(minimal_v6_story()).unwrap();
        let mut machine = Machine::with_output(mem, Box::new(CaptureSink::new()));
        let mut windows: [ZWindow; 8] = Default::default();
        windows[7] = ZWindow { x_size: 0xFFFF, y_size: 0xFFFF, ..Default::default() };
        machine.screen.v6 = Some(V6Windows { windows, current: 7 });
        let mut sess = GameSession {
            machine, quit: false, pending: InputKind::Line, strip_prompt: true,
            disasm_cache: std::cell::RefCell::new(None),
            last_confirmed_pc: std::cell::Cell::new(None),
            pict_source: None,
            pictures_canvas: std::collections::HashMap::new(),
            story_pics: Vec::new(),
            v6_win0_chars_seen: 0,
        };
        // The erase path allocates the canvas even without a resolved image.
        // (number != 0: a real erase_picture — number 0 is the erase_window
        // canvas-clear sentinel, which removes the canvas instead.)
        sess.apply_picture_event(&PictureEvent { number: 5, window: 7, x: 0, y: 0, erase: true, out_chars: 0, margin_after: None });
        let c = sess.pictures_canvas.get(&7).expect("erase allocated a canvas");
        assert!(c.img.width() <= 4096 && c.img.height() <= 4096,
            "canvas clamped, got {}x{}", c.img.width(), c.img.height());
    }

    // Fixture-gated: in-game SAVE then RESTORE on Bureaucracy (v4) must leave the
    // upper-window status grid non-empty (the redraw this whole feature is about).
    // NOTE/GAP: this drives the SESSION resume API, not the app event loop, and it
    // depends on reaching @save by typing into the game. If the input sequence does
    // not reach @save within the probe budget, the test skips (no false failure).
    #[test]
    fn bureaucracy_ingame_restore_redraws_status_grid() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/bureaucr.z4");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture).expect("read bureaucr.z4");
        let mut sess = GameSession::new(story, true, false, None).expect("new bureaucr.z4");

        // Probe: type SAVE-ish commands until the VM suspends on @save.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["save", "yes", "save", "y", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // pretend the host wrote the file
                break;
            }
            if r.quit { break; }
        }
        let Some(blob) = blob else {
            // Could not reach @save with this probe sequence — document the gap.
            eprintln!("bureaucr.z4: did not reach @save via the probe; skipping redraw assertion");
            return;
        };

        // Now drive a RESTORE and feed the captured blob back.
        let mut restored = false;
        for cmd in ["restore", "yes", "restore", "y", "restore"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Restore) {
                let _ = sess.resume_restore(Some(&blob));
                restored = true;
                break;
            }
            if r.quit { break; }
        }
        if !restored {
            eprintln!("bureaucr.z4: did not reach @restore via the probe; skipping redraw assertion");
            return;
        }

        // The resumed game redrew its own status line into the upper window.
        let any_drawn = sess.machine.screen.upper.cells.iter().any(|c| c.ch != ' ');
        assert!(any_drawn, "after in-game RESTORE the upper-window grid must be non-empty (redraw)");
    }

    // Real v3 game: an in-game @save then @restore must round-trip through the
    // standard branch-form path. Oracle: replaying the same command after a
    // restore reproduces the pre-restore transcript exactly.
    #[test]
    fn minizork_v3_ingame_save_restore_round_trips() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() {
            panic!("minizork.z3 fixture missing at {} — this smoke test must run", fixture.display());
        }
        let story = std::fs::read(&fixture).expect("read minizork.z3");
        let mut sess = GameSession::new(story, true, false, None).expect("new minizork.z3");

        // Reach a stable prompt, then @save via the game's save verb.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["open mailbox", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // host "wrote" the file; @save returns success
                break;
            }
            assert!(!r.quit, "unexpected quit before reaching @save");
        }
        let blob = blob.expect("minizork reached @save via 'save'");

        // Probe command on the post-save branch.
        let t1 = sess.submit("north").transcript;

        // Restore via the game's @restore, supplying the captured blob.
        let r = sess.submit("restore");
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "'restore' reaches @restore");
        sess.resume_restore(Some(&blob));

        // Same probe after restore must reproduce the same transcript.
        let t2 = sess.submit("north").transcript;
        assert_eq!(t2, t1, "post-restore continuation matches the pre-restore continuation");
    }

    // SQ-0233 probe: saves-manager load of a game `.qzl` (host-initiated) goes
    // through restore_game_save (complete_restore_success), NOT resume_restore.
    // Verify the next typed command runs (not dropped / not the pre-save one).
    #[test]
    fn game_save_restore_via_manager_accepts_next_command() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() { panic!("minizork.z3 missing"); }
        let story = std::fs::read(&fixture).expect("read minizork.z3");

        // Producer: reach @save, capture the descriptor-PC game-save blob.
        let mut prod = GameSession::new(story.clone(), true, false, None).expect("new");
        let mut blob = None;
        for cmd in ["open mailbox", "save"] {
            let r = prod.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(prod.machine.save_quetzal());
                let _ = prod.resume_save(true);
                break;
            }
        }
        let blob = blob.expect("reached @save");

        // Consumer: fresh session, restore via the host game-save path.
        let mut sess = GameSession::new(story, true, false, None).expect("new");
        sess.restore_game_save(&blob).expect("restore game save");
        let t = sess.submit("north").transcript;
        assert!(t.contains("North of House"),
            "after saves-manager game-save restore, typed 'north' must run (got {t:?})");
    }

    // Real v3 game, real `.qzl` FILE: extends the test above by exercising the
    // on-disk game-save format end to end. `persist_files::save_game_named` writes
    // the descriptor-PC blob to a real `.qzl` file; a FRESH session's machine is
    // then restored from that file via `persist_files::restore_game` (Task 1's
    // descriptor-completion path) — not `resume_restore` — so the actual
    // file-format restore function is what's under test.
    // Oracle (SQ-0158): `play(prefix).probe()` == `restore(qzl file).probe()`.
    #[test]
    fn minizork_v3_qzl_file_round_trips_end_to_end() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture.exists() {
            panic!("minizork.z3 fixture missing at {} — this smoke test must run", fixture.display());
        }
        let story = std::fs::read(&fixture).expect("read minizork.z3");
        let mut sess = GameSession::new(story.clone(), true, false, None).expect("new minizork.z3");

        let dir = std::env::temp_dir().join(format!("babelmap-task5-qzl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // Reach a stable prompt, then @save via the game's save verb. Capture the
        // descriptor-PC blob AND write it to a real `.qzl` file at the same paused
        // moment, before resume_save continues execution and mutates the machine.
        let mut blob: Option<Vec<u8>> = None;
        let mut qzl_path: Option<std::path::PathBuf> = None;
        for cmd in ["open mailbox", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                qzl_path = Some(
                    crate::persist_files::save_game_named(&dir, "task5", &sess.machine)
                        .expect("save_game_named writes the .qzl file"),
                );
                let _ = sess.resume_save(true); // host "wrote" the file; @save returns success
                break;
            }
            assert!(!r.quit, "unexpected quit before reaching @save");
        }
        let blob = blob.expect("minizork reached @save via 'save'");
        let qzl_path = qzl_path.expect("save_game_named ran");
        assert!(qzl_path.to_string_lossy().ends_with(".qzl"), "game save is a .qzl file");

        let bytes_from_disk = std::fs::read(&qzl_path).expect("read the .qzl file back");
        assert_eq!(bytes_from_disk, blob, ".qzl file bytes match the captured save_quetzal() blob");

        // Reference leg: play(prefix).probe() — continue the SAME session past the save.
        let t1 = sess.submit("north").transcript;
        assert!(t1.contains("North of House"), "probe must reveal real room state, got: {t1:?}");

        // Restore leg: a FRESH session's machine, restored straight from the real
        // `.qzl` file via persist_files::restore_game.
        let mut sess2 = GameSession::new(story, true, false, None).expect("new minizork.z3 (fresh)");
        crate::persist_files::restore_game(&qzl_path, &mut sess2.machine)
            .expect("restore_game completes the .qzl descriptor");
        // Run forward to the next input request (mirrors resume_restore's own
        // run_until_input) and sync the session's pending/quit bookkeeping.
        let stop = run_until_input(&mut sess2.machine);
        let _ = sess2.finish_turn(stop); // drains stray intro/restore text, not asserted

        // restore(qzl file).probe() — same probe command on the restored session.
        let t2 = sess2.submit("north").transcript;
        assert_eq!(t2, t1, "restore(qzl file).probe() must equal play(prefix).probe()");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── czech.z5 smoke test ───────────────────────────────────────────────────
    //
    // czech.z5 is an auto-running opcode test suite: it runs to `Quit` without
    // ever requesting input, so `session.quit` will be `true` after `new`.
    // We verify that the session was built successfully and produced output.

    #[test]
    fn czech_smoke_initial_transcript_nonempty() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read czech.z5");
        let mut session = GameSession::new(story, true, false, None).expect("GameSession::new with czech.z5");
        // czech is an automated test suite that runs to completion (quit=true is normal).
        let transcript = session.take_transcript();
        assert!(!transcript.is_empty(), "czech should produce output before quitting");
    }

    // ── Task 4: first_banner_line + resolve_title tests ──────────────────────

    #[test]
    fn title_from_banner_anchors_on_boilerplate() {
        // Title is the line above the copyright/boilerplate anchor.
        assert_eq!(title_from_banner("\n\nZORK I: The Great Underground Empire\nCopyright...\n> ").as_deref(),
                   Some("ZORK I: The Great Underground Empire"));
        // "interactive fiction" / "interactive fantasy" also anchor.
        assert_eq!(title_from_banner("SPELLBREAKER\nAn Interactive Fantasy\nCopyright (c) 1985").as_deref(),
                   Some("SPELLBREAKER"));
        // Banner opens WITH boilerplate (no title above) → None (caller → filename).
        assert_eq!(title_from_banner("Copyright (C) 1987 Infocom, Inc.\nType RESTORE...").as_deref(), None);
        // No anchor (epigraph / narration) → None.
        assert_eq!(title_from_banner("\"Tomorrow never yet\nOn any human being rose or set.").as_deref(), None);
        assert_eq!(title_from_banner("\n\n").as_deref(), None);
    }

    #[test]
    fn known_title_looks_up_table() {
        assert_eq!(known_title("ZCODE-116-870602-FC65"), Some("Bureaucracy"));
        assert_eq!(known_title("ZCODE-77-850814-5031"), Some("A Mind Forever Voyaging"));
        assert_eq!(known_title("ZCODE-0-000000-0000"), None);
        // Entries added when the table moved to the bundled known_titles.tsv.
        assert_eq!(known_title("ZCODE-27-831005-X"), Some("Deadline"));
        assert_eq!(known_title("ZCODE-48-840904-X"), Some("Zork II: The Wizard of Frobozz"));
        assert_eq!(known_title("ZCODE-29-860820-X"), Some("Enchanter"));
        // Alternate releases (not the copy we own) resolve from the full catalog.
        assert_eq!(known_title("ZCODE-23-820428-X"), Some("Zork I: The Great Underground Empire"));
        assert_eq!(known_title("ZCODE-15-840612-X"), Some("Seastalker"));
        // v6 reference entries resolve even though babelmap can't launch them yet.
        assert_eq!(known_title("ZCODE-296-881019-X"), Some("Zork Zero: The Revenge of Megaboz"));
    }

    #[test]
    fn known_titles_file_parses_without_dupes() {
        let table = known_titles();
        assert!(table.len() >= 30, "bundled table has the verified entries: {}", table.len());
        // Keys are unique IFID prefixes (HashMap would silently dedupe; assert the
        // line count matches the entry count so a duplicate prefix is caught).
        let lines = include_str!("known_titles.tsv")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert_eq!(lines, table.len(), "no duplicate IFID prefixes in known_titles.tsv");
    }

    #[test]
    fn resolve_title_override_then_table_then_banner_then_filename() {
        use std::path::Path;
        // override wins over everything.
        assert_eq!(resolve_title(Some("My Game"), "ZCODE-116-870602-FC65", Some("X"), Path::new("/x/zork1.z3")), "My Game");
        // table wins over the banner heuristic (e.g. Bureaucracy, whose banner is just copyright).
        assert_eq!(resolve_title(None, "ZCODE-116-870602-FC65", None, Path::new("/x/bureaucr.z4")), "Bureaucracy");
        // unknown IFID → banner heuristic.
        assert_eq!(resolve_title(None, "UNKNOWN", Some("ZORK I"), Path::new("/x/zork1.z3")), "ZORK I");
        // unknown IFID + no banner title → filename.
        assert_eq!(resolve_title(None, "UNKNOWN", None, Path::new("/x/zork1.z3")), "zork1");
    }

    // ── strip_read_prompt unit tests ──────────────────────────────────────────

    #[test]
    fn strip_prompt_removes_trailing_gt_on_own_line() {
        // Typical Infocom pattern: text followed by newline and bare ">".
        assert_eq!(
            strip_read_prompt("You are in a room.\n\n>"),
            "You are in a room."
        );
    }

    #[test]
    fn strip_prompt_removes_trailing_gt_with_trailing_space() {
        // Some games emit "> " (with a space after).
        assert_eq!(
            strip_read_prompt("You are in a room.\n> "),
            "You are in a room."
        );
    }

    #[test]
    fn strip_prompt_does_not_remove_mid_text_gt() {
        // A ">" that is NOT the last non-whitespace token on its own line must
        // be preserved — e.g. a score comparison or a quoted string.
        let s = "Your score is > 10.\nYou are here.";
        assert_eq!(strip_read_prompt(s), s);
    }

    #[test]
    fn strip_prompt_does_not_remove_gt_mid_line() {
        // ">" at the end of the last line but inline (no preceding newline).
        let s = "Go east, then go >";
        assert_eq!(strip_read_prompt(s), s);
    }

    #[test]
    fn strip_prompt_empty_input_unchanged() {
        assert_eq!(strip_read_prompt(""), "");
    }

    #[test]
    fn strip_prompt_sole_gt_removed() {
        // Edge case: the entire captured block is just ">".
        assert_eq!(strip_read_prompt(">"), "");
    }

    #[test]
    fn strip_prompt_gt_with_only_whitespace_before() {
        // "\n>" with no preceding text.
        assert_eq!(strip_read_prompt("\n>"), "");
    }

    #[test]
    fn strip_prompt_no_trailing_prompt_unchanged() {
        let s = "You are in a maze of twisty passages, all alike.";
        assert_eq!(strip_read_prompt(s), s);
    }

    // ── key_input_to_zscii: arrow and function keys (Bug B) ──────────────────

    #[test]
    fn key_input_to_zscii_arrows_map_to_zscii_codes() {
        use crate::engine::KeyInput;
        // Arrow keys → ZSCII cursor codes (ZMSD §3.8), matching zvm-cli decode_escape_seq.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Up),    Some(129));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Down),  Some(130));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Left),  Some(131));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Right), Some(132));
        // Function keys F1-F4 → ZSCII 133-136.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(1)), Some(133));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Func(4)), Some(136));
    }

    #[test]
    fn key_input_to_zscii_existing_keys_unchanged() {
        use crate::engine::KeyInput;
        // Pre-existing mappings must not change.
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Enter),     Some(13));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Backspace), Some(8));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Escape),    Some(27));
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('A')), Some(65));
        // Non-ascii char → None (existing behaviour).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Char('\u{00E9}')), None);
        // Tab → None (not a game key).
        assert_eq!(GameSession::key_input_to_zscii(KeyInput::Tab), None);
    }

    // ── SQ-0188: line terminator keys ─────────────────────────────────────────

    /// A v5 story whose header 0x2E points at a terminating-characters table
    /// listing ZSCII 129 (cursor-up), so `is_terminator(129)` is true. Mirrors the
    /// zvm fixture in `terminating_chars_table_is_honoured`.
    fn story_v5_with_up_terminator() -> Vec<u8> {
        let mut buf = read_char_story_v5();
        let tbl: u16 = 0x0090; // dynamic memory, below static base 0x0400
        buf[0x2E] = (tbl >> 8) as u8;
        buf[0x2F] = (tbl & 0xFF) as u8;
        buf[tbl as usize] = 0x81;     // 129 = cursor up
        buf[tbl as usize + 1] = 0x00; // table terminator
        buf
    }

    #[test]
    fn line_key_terminator_maps_listed_key() {
        use crate::engine::KeyInput;
        let s = GameSession::new(story_v5_with_up_terminator(), true, false, None)
            .expect("GameSession::new");
        // Up (129) is listed in the game's table → submit with that terminator.
        assert_eq!(s.line_key_terminator(&KeyInput::Up), Some(129));
        // Down (130) is a candidate but NOT listed → None (keeps app behavior).
        assert_eq!(s.line_key_terminator(&KeyInput::Down), None);
    }

    #[test]
    fn line_key_terminator_none_without_table() {
        use crate::engine::KeyInput;
        // No terminating-characters table → arrows/F-keys are never terminators.
        let s = GameSession::new(read_char_story_v5(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(s.line_key_terminator(&KeyInput::Up), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Func(1)), None);
    }

    #[test]
    fn line_key_terminator_rejects_non_candidate_keys() {
        use crate::engine::KeyInput;
        // Even with a table present, only arrows + F-keys are candidates; Enter
        // flows through the normal submit path and other keys never terminate.
        let s = GameSession::new(story_v5_with_up_terminator(), true, false, None)
            .expect("GameSession::new");
        assert_eq!(s.line_key_terminator(&KeyInput::Char('x')), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Enter), None);
        assert_eq!(s.line_key_terminator(&KeyInput::Backspace), None);
    }
}

#[cfg(test)]
mod debugger_impl_tests {
    use super::*;
    use crate::engine::Engine;

    // minizork.z3 is a real game with a populated dictionary and object table
    // (unlike the synthetic read_char_story_v5 fixture, which has neither), so
    // it exercises every Debugger method meaningfully. It's the same fixture
    // zvm's own dictionary/objects/location tests use for this reason.
    fn zvm_session() -> Option<GameSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None; // fixture absent — skip
        }
        let story = std::fs::read(&fixture_path).expect("read minizork.z3");
        Some(GameSession::new(story, true, false, None).expect("GameSession::new with minizork.z3"))
    }

    #[test]
    fn parked_read_renders_as_a_read_op_before_the_pc() {
        // At an input prompt, `step()` advances state.pc PAST the read, so `pc()`
        // is the code that consumes the input — NOT the read. fold_confirmations
        // confirms the parked read instruction (pending_read_pc) so it still renders
        // as a real read op immediately before the PC, instead of being eaten by a
        // stale tiling and shown as some other opcode. (read-pc disasm fix)
        let Some(session) = zvm_session() else { return };
        assert_eq!(session.pending_input(), InputKind::Line,
            "GameSession::new runs minizork to its first line prompt");
        let dbg = session.debugger().expect("z-machine debugger");
        let pc = dbg.pc();
        let read_addr = dbg.prev_instr(pc);
        let row = &dbg.disassemble(read_addr, 1)[0];
        assert!(row.contains("read"), "parked read renders as a read op, got {row:?}");
        assert_eq!(dbg.next_instr(read_addr), pc, "the read is the instruction immediately before the parked PC");
    }

    #[test]
    fn a_turn_reopens_the_confirmation_gate_at_the_same_prompt() {
        // The per-turn confirmation gate is keyed on the parked PC, but a
        // look/examine returns to the SAME input prompt — its freshly-executed
        // boundaries (which correct false routine headers) must still be folded, so
        // a turn must reopen the gate rather than skip confirmation. (read-pc follow-up)
        let Some(mut session) = zvm_session() else { return };
        session.set_debug_trace(true);
        let pc = session.machine.state.pc;
        let _ = session.debugger().unwrap().disassemble(pc, 1); // builds + confirms, closes the gate
        assert_eq!(session.last_confirmed_pc.get(), Some(pc), "confirm closes the gate on the parked PC");
        let _ = Engine::submit(&mut session, "look");
        assert_eq!(session.last_confirmed_pc.get(), None, "a turn must reopen the confirmation gate");
    }

    // A read-only debug inspection must never leave a latched memory fault in the
    // shared VM `Memory`: the disassembler can walk past code into data, and an
    // OOB read latches into the fault cell the CPU drains each step — so a phantom
    // fault would halt the *game* on its next instruction (the "crash only when
    // /debug is open" bug).
    #[test]
    fn debugger_reads_do_not_leak_a_memory_fault_into_the_vm() {
        let Some(s) = zvm_session() else { return };
        let end = s.machine.mem.len() as u32;
        // Latch a fault the way an out-of-range disassembly read would.
        let _ = s.machine.mem.read_word(end + 100);
        assert!(s.machine.mem.take_mem_fault().is_some(), "sanity: OOB read latches a fault");
        let _ = s.machine.mem.read_word(end + 100); // re-latch (the check above drained it)
        // Any Debugger read must leave the fault cell clean.
        let pc = s.machine.state.pc;
        let dbg = s.debugger().expect("zvm has a debugger");
        let _ = dbg.disassemble(pc, 8);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "a debug read left a phantom fault that would halt the VM on its next step"
        );
        // prev_instr does far more boundary probing (a decode-chain sweep over
        // a whole window) — verify it doesn't leak a fault either.
        let _ = s.machine.mem.read_word(end + 100); // re-latch
        let _ = dbg.prev_instr(pc);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "prev_instr left a phantom fault that would halt the VM on its next step"
        );
        // object_detail reads attributes + property bytes — verify it drains too.
        let _ = s.machine.mem.read_word(end + 100); // re-latch
        let _ = dbg.object_detail(1);
        assert!(
            s.machine.mem.take_mem_fault().is_none(),
            "object_detail left a phantom fault that would halt the VM on its next step"
        );
    }

    #[test]
    fn object_addr_map_maps_object_one_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let obj1_addr = zvm::objects::object_entry_addr(&s.machine.mem, 1);
        assert_eq!(objs.get(&obj1_addr), Some(&1));
    }

    #[test]
    fn dict_addr_map_maps_an_entry_to_its_word() {
        let Some(s) = zvm_session() else { return };
        let dict = s.dict_addr_map();
        assert!(!dict.is_empty(), "minizork has a populated dictionary");
        // Every mapped entry decodes to a non-empty word.
        assert!(dict.values().all(|w| !w.is_empty()));
    }

    #[test]
    fn annotate_refs_appends_obj_tag_for_object_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        let obj1_addr = zvm::objects::object_entry_addr(&s.machine.mem, 1);
        let line = format!("004a2f  loadw @0x{obj1_addr:06x}, #00");
        let out = s.annotate_refs(&line, &objs, &dict);
        assert!(out.contains(&format!("@0x{obj1_addr:06x} [obj#1]")), "got: {out}");
    }

    #[test]
    fn annotate_refs_appends_word_tag_for_dictionary_entry_address() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        // Pick a dictionary entry whose address is not also an object entry base.
        let (&addr, word) = dict
            .iter()
            .find(|(a, _)| !objs.contains_key(a))
            .expect("some dict entry is not an object entry");
        let line = format!("004a2f  storeb @0x{addr:06x}, #01");
        let out = s.annotate_refs(&line, &objs, &dict);
        assert!(out.contains(&format!("@0x{addr:06x} [{word}]")), "got: {out}");
    }

    #[test]
    fn annotate_refs_leaves_non_matching_reference_unchanged() {
        let Some(s) = zvm_session() else { return };
        let objs = s.object_addr_map();
        let dict = s.dict_addr_map();
        // 0xffffff is neither an object nor a dictionary entry base.
        let line = "004a2f  loadw @0xffffff, #00".to_string();
        let out = s.annotate_refs(&line, &objs, &dict);
        assert_eq!(out, line);
    }

    // ── DisasmCache integration (SQ-0418, Task 6) ──────────────────────────
    // The five disassembly Debugger methods now route through GameSession's
    // lazily-built, memoized DisasmCache. These assert integration stability
    // and nav consistency through the &dyn Debugger surface; the cache's own
    // classification/format guarantees are unit-tested in zvm.

    fn is_six_hex(s: &str) -> bool {
        s.len() >= 6 && s.as_bytes()[..6].iter().all(|b| b.is_ascii_hexdigit())
    }

    #[test]
    fn disassemble_routes_through_cache_and_formats_a_real_line() {
        let Some(s) = zvm_session() else { return };
        let pc = Debugger::pc(&s);
        let line = s.disassemble(pc, 1);
        assert_eq!(line.len(), 1, "one requested line -> one line");
        assert!(!line[0].is_empty(), "line is non-empty");
        assert!(is_six_hex(&line[0]), "line begins with a 6-hex address: {:?}", line[0]);
        assert!(&line[0][6..8] == "  ", "6-hex address followed by two spaces: {:?}", line[0]);
    }

    #[test]
    fn nav_boundary_round_trip_and_monotonicity() {
        let Some(s) = zvm_session() else { return };
        let b = Debugger::pc(&s);
        let n = s.next_instr(b);
        let back = s.prev_instr(n);
        // `n` is a real unit boundary produced by next_instr, so stepping
        // forward from prev_instr(n) returns to n.
        assert_eq!(s.next_instr(back), n, "boundary round-trip holds");
        assert!(s.next_instr(n) >= n, "next_instr is non-decreasing");
        assert!(s.prev_instr(n) <= n, "prev_instr is non-increasing");
    }

    #[test]
    fn prev_instr_clamps_without_stalling() {
        let Some(s) = zvm_session() else { return };
        let mut a = Debugger::pc(&s);
        for _ in 0..500 {
            a = s.prev_instr(a);
        }
        // Reached the region-start clamp: stable fixpoint, no panic/hang.
        assert_eq!(s.prev_instr(a), a, "prev_instr is stable at the region-start clamp");
    }

    #[test]
    fn disassemble_window_is_bounded_and_has_no_empty_lines() {
        let Some(s) = zvm_session() else { return };
        let out = s.disassemble(Debugger::pc(&s), 200);
        assert!(out.len() <= 200, "never returns more lines than requested");
        assert!(out.iter().all(|l| !l.is_empty()), "no empty lines");
    }

    #[test]
    fn all_three_modes_agree_on_the_address_prefix() {
        let Some(s) = zvm_session() else { return };
        let pc = Debugger::pc(&s);
        let full = s.disassemble(pc, 1);
        let basic = s.disassemble_basic(pc, 1);
        let raw = s.disassemble_raw(pc, 1);
        assert_eq!(full.len(), 1);
        assert_eq!(basic.len(), 1);
        assert_eq!(raw.len(), 1);
        let addr6 = &full[0][..6];
        assert!(is_six_hex(&full[0]));
        assert_eq!(&basic[0][..6], addr6, "basic shares the address prefix");
        assert_eq!(&raw[0][..6], addr6, "raw shares the address prefix");
        assert_eq!(&raw[0][6..7], ":", "raw's distinct prefix is a colon after the address: {:?}", raw[0]);
    }

    #[test]
    fn zvm_exposes_a_debugger() {
        let Some(s) = zvm_session() else { return };
        let d = s.debugger().expect("zvm has a debugger");
        assert_eq!(d.pc(), s.machine.state.pc);
        assert_eq!(d.globals_lines().len(), 240);
        assert!(!d.dictionary_lines().is_empty());
        assert!(!d.object_tree_lines().is_empty());
        assert_eq!(d.memory_len(), s.machine.mem.len() as u32);
        let hex = d.memory_hex(0, 2);
        assert_eq!(hex.len(), 2);
        assert!(hex[0].starts_with("000000"));
    }

    // ── Runtime-confirmation fold (SQ-0418, Task 9) ────────────────────────
    // Executed/parked PCs and call-stack func_addrs are folded into the cache
    // once per turn so regions the VM really runs self-heal to Instr boundaries.

    #[test]
    fn parked_pc_becomes_an_instr_boundary_after_confirmation() {
        let Some(s) = zvm_session() else { return };
        let p = Debugger::pc(&s);
        // A disassemble read builds the cache then folds the parked PC in.
        let line = s.disassemble(p, 1);
        // p is now the start of an Instr unit: stepping to the next unit and back
        // lands exactly on p (prev/next are unit-boundary ops).
        assert_eq!(s.prev_instr(s.next_instr(p)), p, "parked pc is a unit boundary after confirmation");
        // The first disassembled line is addressed exactly at p.
        assert_eq!(line.len(), 1);
        assert!(line[0].starts_with(&format!("{p:06x}")), "first line starts at p: {:?}", line[0]);
    }

    #[test]
    fn frame_func_addrs_are_promoted_to_routine_headers() {
        let Some(s) = zvm_session() else { return };
        let _ = s.disassemble(Debugger::pc(&s), 1); // build cache + fold the call stack in
        for f in &s.machine.state.frames {
            // Only func_addrs inside the code region get a header; disassembling
            // at one now shows a RoutineHeader unit line ("; routine").
            let hdr = s.disassemble(f.func_addr, 1);
            if hdr.is_empty() {
                continue; // outside the tiled code region
            }
            assert!(
                hdr[0].contains("; routine"),
                "func_addr {:06x} did not become a routine header: {:?}",
                f.func_addr, hdr[0]
            );
        }
    }

    #[test]
    fn confirmation_is_idempotent_and_stable() {
        let Some(s) = zvm_session() else { return };
        let _ = s.disassemble(Debugger::pc(&s), 1); // build cache + first fold
        s.confirm_disasm();
        let first = s.disassemble(Debugger::pc(&s), 50);
        s.confirm_disasm();
        let second = s.disassemble(Debugger::pc(&s), 50);
        assert_eq!(first, second, "confirmation must not oscillate the disasm window");
    }
}
