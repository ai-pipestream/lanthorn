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
use zvm::cpu::exec::{Machine, SoundEvent, StepResult};
use zvm::error::ZError;
use zvm::io::{Output, TextAttrs};
use zvm::location::{detect_location, Location, LocationMethod};
use zvm::screen::ZColour;
use zvm::ObjectSnapshot;
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
/// `runs` records one `(char_count, text_style_bits, fg, bg, link)` chunk per
/// `print`/`print_styled`/`print_attr` call, in lockstep with the appended
/// text, so callers can reconstruct which spans carried Z-machine emphasis and
/// colour. `link` is the Glk hyperlink value (always 0 on the Z-machine path).
pub struct CaptureSink {
    pub text: String,
    pub runs: Vec<(usize, u8, ZColour, ZColour, u32)>,
}

impl CaptureSink {
    fn new() -> Self {
        CaptureSink { text: String::new(), runs: Vec::new() }
    }

    /// Drain accumulated text and style runs together, leaving both empty.
    pub fn take_styled(&mut self) -> (String, Vec<(usize, u8, ZColour, ZColour, u32)>) {
        (std::mem::take(&mut self.text), std::mem::take(&mut self.runs))
    }

    /// Drain all accumulated text, leaving the buffer empty.
    pub fn take_text(&mut self) -> String {
        self.take_styled().0
    }
}

impl Output for CaptureSink {
    fn print(&mut self, s: &str) {
        self.runs.push((s.chars().count(), 0, ZColour::Default, ZColour::Default, 0));
        self.text.push_str(s);
    }
    fn print_styled(&mut self, s: &str, style: u8) {
        self.runs.push((s.chars().count(), style, ZColour::Default, ZColour::Default, 0));
        self.text.push_str(s);
    }
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.runs.push((s.chars().count(), attrs.style, attrs.fg, attrs.bg, 0));
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
    runs: Vec<(usize, u8, ZColour, ZColour, u32)>,
    char_len: usize,
) -> Vec<(usize, u8, ZColour, ZColour, u32)> {
    let mut out = Vec::with_capacity(runs.len());
    let mut total = 0usize;
    for (c, b, fg, bg, link) in runs {
        if total >= char_len {
            break;
        }
        let take = c.min(char_len - total);
        out.push((take, b, fg, bg, link));
        total += take;
    }
    out
}

// ── Public types ──────────────────────────────────────────────────────────────

/// One ordered piece of a turn's buffer output: a text run (with its style
/// chunks) or an inline image. Preserves emission order so images land between
/// the right lines.
pub enum TranscriptElem {
    Text { text: String, runs: Vec<(usize, u8, ZColour, ZColour, u32)> },
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
    /// Z-machine text-style chunks for `transcript`: a `(char_count, bits, fg, bg)` list
    /// covering every char of `transcript`, fed to `push_transcript_runs`. All
    /// chunks carry bits 0 and default colours when the turn emitted no styling.
    pub transcript_runs: Vec<(usize, u8, ZColour, ZColour, u32)>,
    pub location: Option<ObjectSnapshot>,
    pub quit: bool,
    /// The game issued `erase_window` (lower / all) this turn (ZMSD §8.7.3) — a
    /// screen clear, e.g. a help-menu takeover. The host clears the transcript
    /// before appending this turn's output so stale text does not bleed through
    /// (matching a retained-mode interpreter like Lectrote).
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
        let mem = Memory::new(story)?;
        let sink = Box::new(CaptureSink::new());
        let mut machine = Machine::with_output(mem, sink);
        machine.set_honor_game_colours(honor_game_colours);
        machine.set_sound_available(sound_available);
        machine.set_interpreter_number(interpreter_number);
        machine.init_caps();

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

        Ok(GameSession { machine, quit, pending, strip_prompt: true })
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        let raw = sink_mut(&mut self.machine).take_text();
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
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
        let transcript_runs = clamp_runs(raw_runs, transcript.chars().count());
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(location_to_snapshot);
        let location_method = detected.as_ref().map(Location::method);

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        let sounds = std::mem::take(&mut self.machine.pending_sounds);
        let erase_lower = std::mem::take(&mut self.machine.screen.erase_lower_requested);

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
            transcript_elems: Vec::new(),
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
        if mapper.mode == mapper::layout::LayoutMode::Auto {
            crate::render::map::cleanup_overlaps(&mut mapper.graph, 2, 20);
        }
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
    BorderPref, BufferWindow, Engine, EngineError, EngineSave, GridCell, GridWindow, Introspect,
    KeyInput, LocationInfo, ScreenModel, Split, StatusField, StatusModel, WinNode,
};

/// The engine tag recorded in an `EngineSave` produced by the Z-machine adapter.
pub const ZMACHINE_ENGINE: &str = "zmachine";
/// The save-format version within the `zmachine` engine (Quetzal).
const ZMACHINE_SAVE_FORMAT: u32 = 1;

impl GameSession {
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
        // Dot syntax resolves to the inherent `GameSession::submit` (inherent
        // methods take precedence over trait methods), so this is not recursive.
        self.submit(command)
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        let byte = GameSession::key_input_to_zscii(key)?;
        Some(self.submit_char(byte))
    }

    fn take_transcript(&mut self) -> String {
        self.take_transcript()
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
        screen_model_from_machine(&self.machine)
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

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn introspect(&self) -> Option<&dyn Introspect> {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;

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
            (2, 0, ZColour::Default, ZColour::Default, 0),
            (2, 0x02, ZColour::Default, ZColour::Default, 0),
        ]);
    }

    #[test]
    fn clamp_runs_trims_to_char_len() {
        use zvm::screen::ZColour;
        // strip_read_prompt removed 3 trailing chars ("\n> " etc.) → clamp.
        let runs = vec![
            (2, 0u8, ZColour::Default, ZColour::Default, 0u32),
            (5, 0x02u8, ZColour::Default, ZColour::Default, 0u32),
        ];
        assert_eq!(clamp_runs(runs, 4), vec![
            (2, 0, ZColour::Default, ZColour::Default, 0),
            (2, 0x02, ZColour::Default, ZColour::Default, 0),
        ]);
    }

    fn dummy_inline_image() -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(2, 2)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
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
            runs: vec![(raw.chars().count(), 0, ZColour::Default, ZColour::Default, 0)],
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
            TranscriptElem::Text { text: "foo\n".into(), runs: vec![(4, 0, ZColour::Default, ZColour::Default, 0)] },
            TranscriptElem::Image(dummy_inline_image()),
            TranscriptElem::Text { text: ">".into(), runs: vec![(1, 0, ZColour::Default, ZColour::Default, 0)] },
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
            transcript_elems: Vec::new(),
        }
    }

    #[test]
    fn auto_mode_cleanup_keeps_map_free_of_illegal_overlaps() {
        // Drive a small loop (E, N, W, S toward start) that — under incremental
        // placement — can produce a routing overlap.  After the sequence,
        // render_overlap_stats must report zero illegal overlaps.
        let mut m = Mapper::default(); // Auto mode by default

        apply_turn(&mut m, "look",  &turn(1, "Start"));
        apply_turn(&mut m, "east",  &turn(2, "East Room"));
        apply_turn(&mut m, "north", &turn(3, "North East Room"));
        apply_turn(&mut m, "west",  &turn(4, "North Room"));
        apply_turn(&mut m, "south", &turn(1, "Start")); // back to start — closes the loop

        let (illegal, _) = crate::render::map::render_overlap_stats(&m.graph);
        assert_eq!(illegal, 0, "Auto mode cleanup must leave zero illegal overlaps");
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
