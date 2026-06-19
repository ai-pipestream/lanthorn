// GameSession — drives one VM turn, captures transcript output, mapper bridge.
//
// Transcript capture approach: we use a custom `CaptureSink` (rather than
// reusing `zvm::io::BufferOutput`) because `BufferOutput` has no drain/clear
// method.  `CaptureSink` implements `zvm::io::Output` and exposes `take_text`
// to drain accumulated text between turns.  After construction the sink is
// accessed by downcasting `machine.out` via the `as_any()` supertrait —
// `machine.out` is `pub`, so no zvm visibility change is required.
//
// zvm pub changes: NONE.

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

        let quit = run_until_input(&mut machine);

        Ok(GameSession { machine, quit })
    }

    /// Drain the transcript accumulated since the last drain (intro or last turn).
    pub fn take_transcript(&mut self) -> String {
        sink_mut(&mut self.machine).take_text()
    }

    /// Supply a player command, step until the next input request or Quit,
    /// and return the turn result.
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.machine.supply_line(command);
        let quit = run_until_input(&mut self.machine);
        self.quit = quit;

        let transcript = sink_mut(&mut self.machine).take_text();
        let location = current_location(&self.machine);

        TurnResult { transcript, location, quit }
    }
}

// ── Mapper bridge ─────────────────────────────────────────────────────────────

/// Pure bridge: observe the new location (if any) into the mapper.
///
/// Calls `mapper.observe(snap.number, &snap.name, parse_direction(command))`.
/// No-op when `result.location` is `None`.
pub fn apply_turn(mapper: &mut Mapper, command: &str, result: &TurnResult) {
    if let Some(snap) = &result.location {
        mapper.observe(snap.number, &snap.name, parse_direction(command));
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Step the machine until it pauses for input or quits.
/// Returns `true` if the machine quit.
fn run_until_input(machine: &mut Machine) -> bool {
    loop {
        match machine.step() {
            StepResult::Quit => return true,
            StepResult::NeedLine { .. } | StepResult::NeedChar => return false,
            StepResult::SaveRequest => {
                // Auto-fail saves during headless operation.
                machine.complete_save(false);
            }
            StepResult::RestoreRequest => {
                machine.complete_restore_failure();
            }
            StepResult::Restart => {
                // Restart is not supported in headless mode; treat as quit.
                return true;
            }
            StepResult::Continue => {}
        }
    }
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
        };
        apply_turn(&mut m, "look", &result);
        assert_eq!(m.graph.current(), None);
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
}
