//! `ScottSession` — adapts the `scott` VM (ScottFree-format Adventure
//! International titles) to the engine-neutral [`Engine`] trait, alongside
//! `zvm`'s `GameSession` (`session.rs`) and the Glulx `GlulxSession`.
//!
//! Scott games are line-only (no `read_char`), carry no Glk window tree (the
//! app renders the transcript itself), and have no in-game `@save`/`@restore`
//! suspension protocol — persistence is entirely the host-driven Save State
//! snapshot (`Vm::snapshot`/`Vm::restore`).

use std::any::Any;
use std::collections::BTreeMap;

use crate::engine::{
    BufferWindow, Engine, EngineError, EngineSave, LocationInfo, ScreenModel, StatusModel, WinNode,
};
use crate::session::{InputKind, TurnResult};

/// The engine tag recorded in an `EngineSave` produced by the Scott adapter.
pub const SCOTT_ENGINE: &str = "scott";
/// The save-format version within the `scott` engine.
pub const SCOTT_SAVE_FORMAT: u32 = 1;

/// A running Scott Adams (ScottFree `.dat`) game session.
pub struct ScottSession {
    vm: scott::Vm,
    /// The opening room description from `Vm::new`, drained by the first
    /// `take_transcript` call; empty thereafter (per-turn output flows
    /// through `TurnResult::transcript` instead).
    intro: String,
    aux: BTreeMap<String, Vec<u8>>,
    aux_dirty: bool,
}

impl ScottSession {
    /// Parse a ScottFree `.dat` (UTF-8 text) and start a session.
    pub fn new(bytes: Vec<u8>) -> Result<ScottSession, String> {
        let src = std::str::from_utf8(&bytes)
            .map_err(|_| "Scott .dat is not valid text".to_string())?;
        let db = scott::Database::parse(src).map_err(|e| format!("invalid Scott .dat: {e:?}"))?;
        let mut vm = scott::Vm::new(db);
        let intro = vm.take_output();
        Ok(ScottSession { vm, intro, aux: BTreeMap::new(), aux_dirty: false })
    }

    /// Build a `TurnResult` with the non-Scott fields at their empty default,
    /// mirroring `GameSession::drain_turn`'s field set exactly.
    fn turn(&self, transcript: String, quit: bool) -> TurnResult {
        TurnResult {
            transcript,
            transcript_runs: Vec::new(),
            location: self.snapshot_location(),
            quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: Vec::new(),
            fault: None,
            location_method: None,
            pending_io: None,
            timed_out: false,
            transcript_elems: Vec::new(),
        }
    }

    fn snapshot_location(&self) -> Option<LocationInfo> {
        let r = self.vm.current_room();
        Some(LocationInfo { number: r as u16, parent: 0, name: self.vm.room_name(r).to_string() })
    }
}

impl Engine for ScottSession {
    fn submit(&mut self, command: &str) -> TurnResult {
        self.vm.supply_line(command);
        let _ = self.vm.step();
        let transcript = self.vm.take_output();
        let quit = self.vm.has_quit();
        self.turn(transcript, quit)
    }

    fn submit_key(&mut self, _key: crate::engine::KeyInput) -> Option<TurnResult> {
        // Scott is line-only: it never issues a `read_char`-style request.
        None
    }

    fn take_transcript(&mut self) -> String {
        std::mem::take(&mut self.intro)
    }

    fn pending_input(&self) -> InputKind {
        InputKind::Line
    }

    fn resume_save(&mut self, _wrote_ok: bool) -> TurnResult {
        // Scott has no in-game @save suspension; nothing to resume.
        self.turn(String::new(), self.vm.has_quit())
    }

    fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult {
        // Scott has no in-game @restore suspension; nothing to resume.
        self.turn(String::new(), self.vm.has_quit())
    }

    fn has_quit(&self) -> bool {
        self.vm.has_quit()
    }

    fn screen(&self) -> ScreenModel {
        ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        }
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(SCOTT_ENGINE, SCOTT_SAVE_FORMAT, self.vm.snapshot())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(SCOTT_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: SCOTT_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        self.vm
            .restore(&save.bytes)
            .map_err(|_| EngineError::BadSave("bad Scott snapshot".to_string()))
    }

    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        self.vm
            .restore(bytes)
            .map_err(|_| EngineError::BadSave("bad Scott snapshot".to_string()))
    }

    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.aux
    }

    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) {
        self.aux = data;
        self.aux_dirty = true;
    }

    fn aux_dirty(&self) -> bool {
        self.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.aux_dirty = false;
    }

    fn current_location(&self) -> Option<LocationInfo> {
        self.snapshot_location()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;

    fn dat() -> Vec<u8> {
        include_bytes!("../../scott/tests/tiny_cave.dat").to_vec()
    }

    #[test]
    fn boots_and_reports_location() {
        let mut s = ScottSession::new(dat()).unwrap();
        let loc = s.current_location().expect("loc");
        assert_eq!(loc.number, 1); // tiny_cave's player_room header field is 1
        let intro = s.take_transcript();
        assert!(!intro.is_empty(), "opening room described");
        assert!(s.take_transcript().is_empty(), "intro drains only once");

        // tiny_cave room 1 has a scripted "down" exit (room1 -> room2).
        let r = s.submit("down");
        assert!(r.location.is_some());
        assert_eq!(r.location.unwrap().number, 2);
        assert!(!r.quit);

        let _ = s.screen();
    }

    #[test]
    fn save_restore_roundtrip() {
        let mut s = ScottSession::new(dat()).unwrap();
        let start = s.current_location().unwrap().number;
        let save = s.save_state();

        s.submit("down");
        let moved = s.current_location().unwrap().number;
        assert_ne!(moved, start, "the move actually changed rooms (sanity)");

        s.restore_state(&save).unwrap();
        assert_eq!(s.current_location().unwrap().number, start);
    }

    #[test]
    fn restore_state_rejects_foreign_engine() {
        let mut s = ScottSession::new(dat()).unwrap();
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        let err = s.restore_state(&foreign).unwrap_err();
        assert!(matches!(err, EngineError::EngineMismatch { .. }));
    }
}
