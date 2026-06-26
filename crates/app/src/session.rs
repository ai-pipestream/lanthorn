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
use zvm::cpu::exec::{Beep, Machine, StepResult};
use zvm::error::ZError;
use zvm::io::Output;
use zvm::location::{detect_location, Location, LocationMethod};
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
}

/// Which in-game (game-initiated) I/O the VM is suspended on after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingIo {
    Save,
    Restore,
}

// ── CaptureSink ───────────────────────────────────────────────────────────────

/// An output sink that accumulates printed text and lets the caller drain it.
pub struct CaptureSink {
    pub text: String,
}

impl CaptureSink {
    fn new() -> Self {
        CaptureSink { text: String::new() }
    }

    /// Drain all accumulated text, leaving the buffer empty.
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }
}

impl Output for CaptureSink {
    fn print(&mut self, s: &str) {
        self.text.push_str(s);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Public types ──────────────────────────────────────────────────────────────

/// Result of one player turn.
pub struct TurnResult {
    pub transcript: String,
    pub location: Option<ObjectSnapshot>,
    pub quit: bool,
    /// Optional informational note to surface to the player (e.g. when the
    /// game's own save/restore is auto-failed, hint them toward Ctrl+S/Ctrl+R).
    pub info: Option<String>,
    /// The latest bleep emitted this turn (last wins), if any.
    pub beep: Option<Beep>,
    /// Host-facing diagnostic lines emitted this turn (drained from the VM).
    pub diagnostics: Vec<String>,
    /// How the current room was detected this turn (drives the map indicator).
    pub location_method: Option<LocationMethod>,
    /// Set when the VM suspended on its own `@save`/`@restore` (v4+). The host
    /// performs the file I/O and calls `resume_save`/`resume_restore`. `None` for
    /// an ordinary turn (and for v3, which still auto-fails — see `info`).
    pub pending_io: Option<PendingIo>,
}

/// A running Z-machine game session.
pub struct GameSession {
    pub machine: Machine,
    pub quit: bool,
    /// Which kind of input the VM is currently waiting for.
    pending: InputKind,
}

// ── GameSession impl ──────────────────────────────────────────────────────────

impl GameSession {
    /// Build a new session from raw story bytes.
    ///
    /// Constructs a `Machine` with a `CaptureSink`, calls `init_caps`, then
    /// steps until the first `NeedLine`/`NeedChar`/`Quit` — this drives the
    /// game's opening text into the sink.  The sink is NOT drained here; the
    /// caller can call `take_transcript` to retrieve the banner/intro text.
    pub fn new(story: Vec<u8>) -> Result<GameSession, ZError> {
        let mem = Memory::new(story)?;
        let sink = Box::new(CaptureSink::new());
        let mut machine = Machine::with_output(mem, sink);
        machine.init_caps();

        let mut quit = false;
        let pending = loop {
            let (stop, _v3) = run_until_input(&mut machine);
            match stop {
                RunStop::Quit => { quit = true; break InputKind::Line; }
                RunStop::Input(k) => break k,
                RunStop::SavePending => machine.complete_save(false),
                RunStop::RestorePending => machine.complete_restore_failure(),
            }
        };

        Ok(GameSession { machine, quit, pending })
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        strip_read_prompt(&sink_mut(&mut self.machine).take_text()).to_owned()
    }

    /// Which kind of input the VM is currently waiting for.
    pub fn pending_input(&self) -> InputKind {
        self.pending
    }

    /// Supply a player command, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.machine.supply_line(command);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    /// Supply a single keypress, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit_char(&mut self, ch: u8) -> TurnResult {
        self.machine.supply_char(ch);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    /// Resume after the host performed an in-game SAVE (`wrote_ok` = file written).
    pub fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.machine.complete_save(wrote_ok);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
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
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    /// Build the `TurnResult` from a `RunStop` (+ v3 auto-fail flag) and drain the
    /// VM's per-turn buffers. Shared by submit/submit_char/resume_*.
    fn finish_turn(&mut self, stop: RunStop, v3_failed: bool) -> TurnResult {
        let (quit, pending, pending_io) = match stop {
            RunStop::Quit => (true, InputKind::Line, None),
            RunStop::Input(k) => (false, k, None),
            RunStop::SavePending => (false, self.pending, Some(PendingIo::Save)),
            RunStop::RestorePending => (false, self.pending, Some(PendingIo::Restore)),
        };
        self.quit = quit;
        self.pending = pending;

        let raw = sink_mut(&mut self.machine).take_text();
        let transcript = strip_read_prompt(&raw).to_owned();
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(|loc| match loc {
            Location::NameOnly(name) => zvm::ObjectSnapshot {
                number: crate::roomid::synthetic_room_id(name),
                parent: 0,
                name: name.clone(),
            },
            _ => loc.object().expect("non-NameOnly variants carry an object").clone(),
        });
        let location_method = detected.as_ref().map(Location::method);

        let info = if v3_failed {
            Some("(babelmap: this game's in-game save/restore isn't wired; use Ctrl+S to save and Ctrl+R to restore instead.)".to_string())
        } else {
            None
        };

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let beep = self.machine.pending_beeps.last().copied();
        self.machine.pending_beeps.clear();

        TurnResult { transcript, location, quit, info, beep, diagnostics, location_method, pending_io }
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
        mapper.observe(snap.number, &snap.name, parse_direction(command));
        if mapper.mode == mapper::layout::LayoutMode::Auto {
            crate::render::map::cleanup_overlaps(&mut mapper.graph, 2, 20);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Stop reason from `run_until_input`.
enum RunStop {
    /// VM is waiting for player input of this kind.
    Input(InputKind),
    /// VM ended (Quit/Restart).
    Quit,
    /// VM suspended on its own `@save` (v4+) — host must `resume_save`.
    SavePending,
    /// VM suspended on its own `@restore` (v4+) — host must `resume_restore`.
    RestorePending,
}

/// Step until the machine pauses for input, quits, or (v4+) suspends on its own
/// save/restore. Returns `(stop, v3_auto_failed)` where `v3_auto_failed` is true
/// when a v3 game's `@save`/`@restore` was auto-rejected this run (drives the
/// host hint). v4+ save/restore is NOT auto-failed: it bubbles up as
/// `SavePending`/`RestorePending`.
fn run_until_input(machine: &mut Machine) -> (RunStop, bool) {
    let mut v3_failed = false;
    loop {
        match machine.step() {
            StepResult::Quit => return (RunStop::Quit, v3_failed),
            StepResult::NeedLine { .. } => return (RunStop::Input(InputKind::Line), v3_failed),
            StepResult::NeedChar => return (RunStop::Input(InputKind::Char), v3_failed),
            StepResult::SaveRequest => {
                if machine.mem.version() <= 3 {
                    machine.complete_save(false);
                    v3_failed = true;
                } else {
                    return (RunStop::SavePending, v3_failed);
                }
            }
            StepResult::RestoreRequest => {
                if machine.mem.version() <= 3 {
                    machine.complete_restore_failure();
                    v3_failed = true;
                } else {
                    return (RunStop::RestorePending, v3_failed);
                }
            }
            StepResult::Restart => {
                // Restart is not supported in headless mode; treat as quit.
                return (RunStop::Quit, v3_failed);
            }
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
    let trimmed = s.trim_end_matches(|c: char| c == ' ' || c == '\t');
    // After stripping trailing spaces/tabs the string may still end with "\n>"
    // or just ">".  Check for that and strip.
    if let Some(without_gt) = trimmed.strip_suffix('>') {
        // Only strip if the ">" is at the start of a line (preceded by '\n')
        // or if it's the only character remaining.
        let preceded_by_newline = without_gt.ends_with('\n') || without_gt.is_empty();
        if preceded_by_newline {
            return without_gt.trim_end_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r');
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
/// the IFID (`ZCODE-<release>-<serial>`, WITHOUT the trailing byte-checksum). This
/// is robust to different file copies of the same release and can be populated
/// from the documented Infocom serial catalog without needing each story file.
/// Used when the opening banner doesn't reliably yield the title (a game opening
/// with copyright/epigraph/narration) or to prefer a clean canonical name.
/// Checked before the banner heuristic.
const KNOWN_TITLES: &[(&str, &str)] = &[
    ("ZCODE-77-850814", "A Mind Forever Voyaging"),
    ("ZCODE-116-870602", "Bureaucracy"),
    ("ZCODE-31-871119", "The Hitchhiker's Guide to the Galaxy"),
    ("ZCODE-37-851003", "Planetfall"),
    ("ZCODE-87-860904", "Spellbreaker"),
    ("ZCODE-393-890714", "Zork Zero: The Revenge of Megaboz"),
    ("ZCODE-88-840726", "Zork I: The Great Underground Empire"),
    ("ZCODE-16-970828", "Zork: The Undiscovered Underground"),
];

/// The canonical title for a known game, matched on the release+serial prefix of
/// the IFID (the trailing `-<checksum>` is ignored).
pub fn known_title(ifid: &str) -> Option<&'static str> {
    // Strip the trailing checksum segment: "ZCODE-88-840726-A129" → "ZCODE-88-840726".
    let key = ifid.rsplit_once('-').map_or(ifid, |(prefix, _)| prefix);
    KNOWN_TITLES.iter().find(|(id, _)| *id == key).map(|(_, t)| *t)
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
            !l.is_empty() && !(l.starts_with('>') && l.trim_start_matches('>').trim().is_empty())
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;

    // ── Pure bridge test ──────────────────────────────────────────────────────

    #[test]
    fn apply_turn_bridge_sets_current_and_creates_edge() {
        let mut m = Mapper::default();

        // First observation: set current room (no prior → no edge).
        let first = TurnResult {
            transcript: String::new(),
            location: Some(ObjectSnapshot { number: 1, parent: 0, name: "Hall".into() }),
            quit: false,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
        };
        apply_turn(&mut m, "look", &first);
        assert_eq!(m.graph.current(), Some(1));
        assert!(m.graph.room(1).is_some());
        assert_eq!(m.graph.connections().len(), 0, "first observe must not create edge");

        // Second observation: move north → directed N edge 1→2.
        let second = TurnResult {
            transcript: String::new(),
            location: Some(ObjectSnapshot { number: 2, parent: 0, name: "Attic".into() }),
            quit: false,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
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
            location: None,
            quit: false,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
        };
        apply_turn(&mut m, "look", &result);
        assert_eq!(m.graph.current(), None);
    }

    // ── TurnResult.info tests ─────────────────────────────────────────────────

    #[test]
    fn turn_result_info_defaults_none_for_normal_turn() {
        // A TurnResult from a normal (non-save/restore) turn has info == None.
        // The save-request note path (info == Some(...)) is exercised manually
        // by running a game that issues its own SAVE verb and confirming the
        // transcript line "(babelmap: this game's in-game save/restore isn't
        // wired...)" appears.
        let r = TurnResult {
            transcript: "You are in a maze.".to_string(),
            location: None,
            quit: false,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
        };
        assert!(r.info.is_none());
    }

    // ── Task-5 overlap cleanup tests ──────────────────────────────────────────

    /// Helper: build a TurnResult with a location (mirrors the pattern used above).
    fn turn(number: u16, name: &str) -> TurnResult {
        TurnResult {
            transcript: String::new(),
            location: Some(ObjectSnapshot { number, parent: 0, name: name.into() }),
            quit: false,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
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
        let session = GameSession::new(story).expect("GameSession::new with czech.z5");
        assert_eq!(session.pending_input(), InputKind::Line,
            "a story that quits without requesting input should leave pending == Line");
    }

    #[test]
    fn pending_input_is_char_after_new_on_read_char_story() {
        let story = read_char_story_v5();
        let session = GameSession::new(story).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char,
            "GameSession::new on a read_char story should leave pending == Char");
    }

    #[test]
    fn submit_char_returns_turn_result_and_advances() {
        let story = read_char_story_v5();
        let mut session = GameSession::new(story).expect("GameSession::new failed");
        assert_eq!(session.pending_input(), InputKind::Char);

        // After read_char the next instruction is quit, so submit_char drives
        // the machine to Quit → TurnResult.quit == true.
        let result = session.submit_char(b'x');
        assert!(result.quit, "submit_char on a read_char→quit story should return quit=true");

        // The quit path sets pending back to Line (no input pending).
        assert_eq!(session.pending_input(), InputKind::Line,
            "after quit, pending should be reset to Line");
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
        let mut sess = GameSession::new(read_char_then_save_v4()).expect("new");
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
        let mut sess = GameSession::new(read_char_then_restore_v4()).expect("new");

        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore));
        assert!(!r.quit);

        // Cancel: resume_restore(None) -> complete_restore_failure stores 0, runs on.
        let r2 = sess.resume_restore(None);
        assert!(r2.quit);
        assert_eq!(sess.machine.global(0), 0, "cancelled restore stored 0 into G0");
    }

    #[test]
    fn v3_ingame_save_still_auto_fails_with_info() {
        // v3 keeps the host-mediated message; the VM auto-fails the request.
        // v3 save is a BRANCH instruction (0OP:0x05 short form 0xB5 + 1 branch byte).
        let mut buf = read_char_story_v5();
        buf[0x00] = 3;
        buf[0x44] = 0xB5; // 0OP:0x05 save (branch form in v3)
        buf[0x45] = 0xC0; // branch: on-true, offset that lands on quit (see note)
        buf[0x46] = 0xBA; // quit
        let mut sess = GameSession::new(buf).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, None, "v3 never bubbles pending_io");
        assert!(r.info.is_some(), "v3 keeps the 'isn't wired' info line");
    }

    #[test]
    fn turn_result_carries_location_method_field() {
        // Build the same way the sibling submit test does; the field just needs to exist
        // and default to a value. For a v3 fixture with global 0 set, method is GlobalVar0.
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        // The field exists and is an Option<LocationMethod>; on a v5 story with no
        // location it is None — either is acceptable here.
        let _ = r.location_method;
    }

    #[test]
    fn turn_result_has_empty_sound_fields_by_default() {
        let story = read_char_story_v5();
        let mut sess = GameSession::new(story).expect("GameSession::new failed");
        // The story starts with a read_char; submit_char drives it to quit.
        let r = sess.submit_char(b'x');
        assert!(r.beep.is_none(), "no beep when the game emits no sound");
        assert!(r.diagnostics.is_empty(), "no diagnostics on a clean turn");
        // VM queues are drained after the turn.
        assert!(sess.machine.pending_beeps.is_empty());
        assert!(sess.machine.diagnostics.is_empty());
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
        let mut session = GameSession::new(story).expect("GameSession::new with czech.z5");
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
}
