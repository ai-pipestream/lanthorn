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
    BufferWindow, Engine, EngineError, EngineSave, LocationInfo, ScreenModel, Split, StatusModel,
    WinNode,
};
use crate::session::{InputKind, PendingIo, TurnResult};

/// The engine tag recorded in an `EngineSave` produced by the Scott adapter.
pub const SCOTT_ENGINE: &str = "scott";
/// The save-format version within the `scott` engine.
pub const SCOTT_SAVE_FORMAT: u32 = 1;

/// The canonical Scott Adams input prompt, shown before each command. ScottFree
/// prints it from its input routine (`scott.c`: `Output("\nTell me what to do ? ")`),
/// so it belongs to the host/input layer here (not the VM, which stays input-agnostic).
/// Scott used this phrase, never the Infocom-style `>`.
const PROMPT: &str = "\nTell me what to do ? ";

/// Build the top room-panel buffer from a `Vm::room_block()` string: one logical
/// line per `\n`, with the per-line style/paragraph/image tracks filled parallel
/// (the inline-buffer renderer indexes them by line). `primary: false` so the app
/// draws it inline rather than mirroring it into the transcript.
fn room_panel(block: &str) -> BufferWindow {
    let lines: Vec<String> = block.split('\n').map(str::to_string).collect();
    let n = lines.len();
    BufferWindow {
        lines,
        runs: vec![Vec::new(); n],
        para: vec![crate::state::ParaFmt::default(); n],
        images: vec![None; n],
        primary: false,
        panel: true,
        ..Default::default()
    }
}

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
        let mut intro = vm.take_output();
        if !vm.has_quit() {
            intro.push_str(PROMPT);
        }
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
        // The game ran the SAVE GAME action (opcode 71): bubble the same Save
        // request the Z-machine/Glulx engines raise for `@save`, so the app's
        // Save State file I/O runs. The prompt is withheld and returns via
        // `resume_save` once the host has written the snapshot.
        if !quit && self.vm.take_save_request() {
            let mut result = self.turn(transcript, quit);
            result.pending_io = Some(PendingIo::Save);
            return result;
        }
        let mut transcript = transcript;
        if !quit {
            transcript.push_str(PROMPT);
        }
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
        // The SAVE GAME action ran the whole turn synchronously; the host has now
        // performed the Save State write. Nothing in the VM to resume — just
        // return to the command prompt (withheld by `submit` for this turn).
        let quit = self.vm.has_quit();
        let transcript = if quit { String::new() } else { PROMPT.to_string() };
        self.turn(transcript, quit)
    }

    fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult {
        // Scott has no in-game @restore suspension; nothing to resume.
        self.turn(String::new(), self.vm.has_quit())
    }

    fn has_quit(&self) -> bool {
        self.vm.has_quit()
    }

    fn screen(&self) -> ScreenModel {
        // The classic Scott split: a persistent top panel showing the current
        // room block (redrawn every frame from live VM state), above the scrolling
        // command transcript. The panel is a non-primary buffer carrying its own
        // lines; the primary buffer below is the transcript the app mirrors.
        let panel = room_panel(&self.vm.room_block());
        let rows = panel.lines.len() as u16;
        let root = WinNode::Pair {
            vertical: true,
            split: Split { fixed: rows },
            border: true,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Buffer(panel)),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        };
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) }
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

    /// The top room-panel text (first buffer of the split), joined for matching.
    fn panel_text(model: &ScreenModel) -> String {
        match &model.root {
            WinNode::Pair { first, .. } => match &**first {
                WinNode::Buffer(b) => b.lines.join("\n"),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    #[test]
    fn boots_and_shows_room_panel() {
        let mut s = ScottSession::new(dat()).unwrap();
        assert_eq!(s.current_location().expect("loc").number, 1);

        // The transcript opens with only the prompt — the room lives in the panel.
        let intro = s.take_transcript();
        assert!(intro.contains("Tell me what to do ?"), "intro carries the Scott prompt");
        assert!(!intro.contains('>'), "Scott never uses the '>' prompt");
        assert!(s.take_transcript().is_empty(), "intro drains only once");

        // The top panel shows the room block: description, exits, and items.
        let panel = panel_text(&s.screen());
        assert!(panel.contains("sunlit forest clearing"), "room in panel: {panel:?}");
        assert!(panel.contains("Obvious exits:"), "exits in panel: {panel:?}");
        assert!(panel.contains("brass lamp"), "items in panel: {panel:?}");

        // Take the lamp (so room 2 is lit), then descend: the panel follows the
        // player, and each turn's transcript ends with the prompt.
        s.submit("take lamp");
        let r = s.submit("down");
        assert_eq!(r.location.expect("loc").number, 2);
        assert!(!r.quit);
        assert!(r.transcript.contains("Tell me what to do ?"), "each turn ends with the prompt");
        assert!(panel_text(&s.screen()).contains("damp, dark cave"), "panel follows the player");
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
    fn a_normal_turn_raises_no_save_request_and_resume_returns_to_prompt() {
        // Only the SAVE GAME action (opcode 71) bubbles a Save request; an
        // ordinary command must not. resume_save (called by the host after it
        // writes the snapshot) returns cleanly to the command prompt.
        let mut s = ScottSession::new(dat()).unwrap();
        let r = s.submit("down");
        assert_eq!(r.pending_io, None, "a plain move does not request a save");

        let resumed = s.resume_save(true);
        assert!(
            resumed.transcript.contains("Tell me what to do ?"),
            "resume_save returns to the Scott prompt: {:?}",
            resumed.transcript
        );
        assert_eq!(resumed.pending_io, None);
    }

    #[test]
    fn restore_state_rejects_foreign_engine() {
        let mut s = ScottSession::new(dat()).unwrap();
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        let err = s.restore_state(&foreign).unwrap_err();
        assert!(matches!(err, EngineError::EngineMismatch { .. }));
    }
}
