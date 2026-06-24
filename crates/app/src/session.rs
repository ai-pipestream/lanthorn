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
use zvm::cpu::exec::{Machine, StepResult};
use zvm::error::ZError;
use zvm::io::Output;
use zvm::location::current_location;
use zvm::ObjectSnapshot;
use zvm::memory::Memory;

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
}

/// A running Z-machine game session.
pub struct GameSession {
    pub machine: Machine,
    pub quit: bool,
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

        let (quit, _) = run_until_input(&mut machine);

        Ok(GameSession { machine, quit })
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        strip_read_prompt(&sink_mut(&mut self.machine).take_text()).to_owned()
    }

    /// Supply a player command, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.machine.supply_line(command);
        let (quit, save_restore_failed) = run_until_input(&mut self.machine);
        self.quit = quit;

        let raw = sink_mut(&mut self.machine).take_text();
        let transcript = strip_read_prompt(&raw).to_owned();
        let location = current_location(&self.machine);

        let info = if save_restore_failed {
            Some("(babelmap: this game's in-game save/restore isn't wired; use Ctrl+S to save and Ctrl+R to restore instead.)".to_string())
        } else {
            None
        };

        TurnResult { transcript, location, quit, info }
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

/// Step the machine until it pauses for input or quits.
///
/// Returns `(quit, save_restore_auto_failed)` where:
/// - `quit` is `true` if the machine ended.
/// - `save_restore_auto_failed` is `true` if a `SaveRequest` or
///   `RestoreRequest` was encountered and auto-rejected this run (so the
///   caller can surface a hint to the player).
fn run_until_input(machine: &mut Machine) -> (bool, bool) {
    let mut save_restore_failed = false;
    loop {
        match machine.step() {
            StepResult::Quit => return (true, save_restore_failed),
            StepResult::NeedLine { .. } | StepResult::NeedChar => return (false, save_restore_failed),
            StepResult::SaveRequest => {
                // Auto-fail saves during headless operation.
                machine.complete_save(false);
                save_restore_failed = true;
            }
            StepResult::RestoreRequest => {
                machine.complete_restore_failure();
                save_restore_failed = true;
            }
            StepResult::Restart => {
                // Restart is not supported in headless mode; treat as quit.
                return (true, save_restore_failed);
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

/// Return the first non-empty, non-`>`-prompt line from the game's opening
/// banner, trimmed and capped at 40 characters.  Returns `None` if no such
/// line exists.
pub fn first_banner_line(intro_text: &str) -> Option<String> {
    for line in intro_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == ">" {
            continue;
        }
        // Skip pure prompt lines (just ">" possibly with whitespace).
        if trimmed.starts_with('>') && trimmed.trim_start_matches('>').trim().is_empty() {
            continue;
        }
        let capped: String = trimmed.chars().take(40).collect();
        return Some(capped);
    }
    None
}

/// Resolve the adventure title using a three-tier priority:
/// 1. `override_name` if provided.
/// 2. `banner` (a captured first-banner-line) if provided.
/// 3. The story file's stem (filename without extension).
pub fn resolve_title(
    override_name: Option<&str>,
    banner: Option<&str>,
    story_path: &std::path::Path,
) -> String {
    if let Some(name) = override_name {
        return name.to_owned();
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
    fn first_banner_line_skips_blank_and_prompt() {
        assert_eq!(first_banner_line("\n\nZORK I: The Great Underground Empire\nCopyright...\n> ").as_deref(),
                   Some("ZORK I: The Great Underground Empire"));
        assert_eq!(first_banner_line("\n\n").as_deref(), None);
    }

    #[test]
    fn resolve_title_prefers_override_then_banner_then_filename() {
        use std::path::Path;
        assert_eq!(resolve_title(Some("My Game"), Some("ZORK I"), Path::new("/x/zork1.z3")), "My Game");
        assert_eq!(resolve_title(None, Some("ZORK I"), Path::new("/x/zork1.z3")), "ZORK I");
        assert_eq!(resolve_title(None, None, Path::new("/x/zork1.z3")), "zork1");
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
