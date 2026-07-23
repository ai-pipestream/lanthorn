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
use std::sync::Arc;

use crate::engine::{
    BufferWindow, Debugger, Engine, EngineError, EngineSave, GraphicsWindow, LocationInfo,
    ScreenModel, Split, StatusModel, WinNode,
};
use crate::graphics::PictSource;
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

/// Terminal rows reserved for the room-picture band in a graphics (`.blb`) game.
/// The renderer scales the picture (typically 256×96) to fit this band.
const PICTURE_ROWS: u16 = 16;

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
    /// Blorb `Pict` resources for a graphics (`.blb`) game; empty for a plain
    /// `.dat`. The SAGA/Mysterious Adventures graphic versions ship the room
    /// pictures here (SQ-0402).
    picts: PictSource,
    /// The decoded picture to show for the current room, and the picture number
    /// it was resolved from — recomputed only when the number changes so the same
    /// image isn't re-uploaded to the terminal every frame.
    current_canvas: Option<Arc<image::RgbaImage>>,
    current_pic_num: Option<u16>,
    pic_version: u64,
}

impl ScottSession {
    /// Parse a ScottFree `.dat` (UTF-8 text) and start a session. `pict_blorb` is
    /// the game's own Blorb when it is a `.blb` graphics container (carrying the
    /// room `Pict` images); `None` for a plain text `.dat`.
    pub fn new(bytes: Vec<u8>, pict_blorb: Option<blorb::Blorb>) -> Result<ScottSession, String> {
        ScottSession::new_with_trace(bytes, pict_blorb, false)
    }

    /// Like [`ScottSession::new`], but starts the VM with fired-action tracing
    /// on (the `--debug` boot path) so the opening occurrence pass — run inside
    /// `Vm::new_with_trace`, before any host code can toggle tracing — is
    /// captured as coverage from the first frame.
    pub fn new_with_trace(
        bytes: Vec<u8>,
        pict_blorb: Option<blorb::Blorb>,
        trace: bool,
    ) -> Result<ScottSession, String> {
        let src = std::str::from_utf8(&bytes)
            .map_err(|_| "Scott .dat is not valid text".to_string())?;
        let db = scott::Database::parse(src).map_err(|e| format!("invalid Scott .dat: {e:?}"))?;
        let mut vm = scott::Vm::new_with_trace(db, trace);
        let mut intro = vm.take_output();
        if !vm.has_quit() {
            intro.push_str(PROMPT);
        }
        let mut s = ScottSession {
            vm,
            intro,
            aux: BTreeMap::new(),
            aux_dirty: false,
            picts: PictSource::new(pict_blorb),
            current_canvas: None,
            current_pic_num: None,
            pic_version: 0,
        };
        s.refresh_picture();
        Ok(s)
    }

    /// Recompute the current room's decoded picture. Cheap when the picture
    /// number is unchanged (early-out). A dark room shows no picture; a room
    /// whose number has no `Pict` resource also shows none.
    fn refresh_picture(&mut self) {
        let want = if self.vm.is_dark() { None } else { self.vm.current_picture() };
        if want == self.current_pic_num {
            return;
        }
        self.current_pic_num = want;
        self.current_canvas = want
            .and_then(|n| self.picts.image(n as u32))
            .map(|dynimg| Arc::new(dynimg.to_rgba8()));
        self.pic_version += 1;
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
            pictures: Vec::new(),
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
        self.refresh_picture();
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
        let text = WinNode::Pair {
            vertical: true,
            split: Split { fixed: rows },
            border: true,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Buffer(panel)),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        };
        // A graphics (`.blb`) game shows the current room's picture in a band
        // above the room panel; the renderer scales the canvas into the band.
        let root = match &self.current_canvas {
            Some(canvas) => WinNode::Pair {
                vertical: true,
                split: Split { fixed: PICTURE_ROWS },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Graphics(GraphicsWindow {
                    win: 1,
                    canvas: Arc::clone(canvas),
                    version: self.pic_version,
                    upscale: true,
                })),
                second: Box::new(text),
            },
            None => text,
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
        let r = self
            .vm
            .restore(&save.bytes)
            .map_err(|_| EngineError::BadSave("bad Scott snapshot".to_string()));
        self.refresh_picture();
        r
    }

    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        let r = self
            .vm
            .restore(bytes)
            .map_err(|_| EngineError::BadSave("bad Scott snapshot".to_string()));
        self.refresh_picture();
        r
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

    fn set_debug_trace(&mut self, on: bool) {
        // Turning the inspector on enables fired-action tracing; off stops it.
        // The cumulative `ever_fired` set (permanent colour + persisted coverage)
        // is preserved either way — only the per-turn set stops updating.
        self.vm.set_trace_fired(on);
    }

    fn seed_executed_pcs(&mut self, pcs: &std::collections::HashSet<u32>) {
        // The PC-set sidecar stores Scott action indices as `u32`s (SQ-0449).
        self.vm.seed_ever_fired(pcs);
    }

    fn debugger(&self) -> Option<&dyn Debugger> {
        Some(&self.vm)
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
    use crate::debug_panel::{DebugPanelState, Section};
    use crate::engine::Engine;

    fn dat() -> Vec<u8> {
        include_bytes!("../../scott/tests/tiny_cave.dat").to_vec()
    }

    #[test]
    fn debug_inspector_wires_scott_sections_and_tracks_fired_actions() {
        // End-to-end through the real session + panel: open the inspector, adopt
        // the Scott layout, refresh, and confirm the sections are populated and
        // Call Stack / Eval Stack / Memory are hidden. Then play a turn with
        // tracing on and confirm coverage accrues.
        let mut s = ScottSession::new_with_trace(dat(), None, true).unwrap();
        assert!(s.debugger().is_some(), "Scott exposes a debugger");

        let mut panel = {
            let dbg = s.debugger().expect("debugger");
            let mut p = DebugPanelState::new(dbg.pc());
            p.apply_engine_layout(dbg);
            p.refresh(dbg);
            p
        };

        // The Scott layout hides the register-machine sections and relabels its
        // three windows; every window still has at least one tab.
        let all: Vec<Section> = panel.tabs.iter().flat_map(|w| w.iter().copied()).collect();
        assert!(!all.contains(&Section::CallStack) && !all.contains(&Section::Memory));
        assert!(panel.tabs.iter().all(|w| !w.is_empty()));
        assert_eq!(panel.tab_label(Section::Disasm), "Actions");

        // Sections carry real content pulled from the loaded game.
        assert!(!panel.snapshot.disasm.is_empty(), "Actions list non-empty");
        assert!(panel.snapshot.globals.iter().any(|l| l.starts_with("Room:")), "State shown");
        assert!(panel.snapshot.objects.iter().any(|l| l.contains("lamp")), "Items shown");
        assert!(!panel.snapshot.dict.is_empty(), "Vocab shown");
        assert!(panel.snapshot.locals.iter().any(|l| l == "Rooms:"), "World shown");

        // Play a turn that fires a table action (moving down runs an occurrence),
        // then refresh: both the last-turn and cumulative fired sets grow.
        s.submit("down");
        {
            let dbg = s.debugger().expect("debugger");
            panel.refresh(dbg);
        }
        assert!(!panel.snapshot.executed.is_empty(), "an action fired this turn");
        assert!(!panel.snapshot.executed_ever.is_empty(), "and was recorded cumulatively");
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
        let mut s = ScottSession::new(dat(), None).unwrap();
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
        let mut s = ScottSession::new(dat(), None).unwrap();
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
        let mut s = ScottSession::new(dat(), None).unwrap();
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

    /// A minimal 2×2 PNG for a blorb `Pict` resource.
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn blb_game_shows_a_room_picture_band() {
        // A graphics (.blb) game: the start room's Pict renders as a Graphics
        // window above the room panel. tiny_cave.dat starts in (lit) room 1, and
        // picture number == room number, so Pict resource 1 is the start picture.
        let blorb = crate::graphics::test_blorb_with_pict(1, &tiny_png());
        let s = ScottSession::new(dat(), Some(blorb)).unwrap();
        match s.screen().root {
            WinNode::Pair { first, .. } => {
                assert!(matches!(*first, WinNode::Graphics(_)), "picture band on top");
            }
            other => panic!("expected a graphics band on top, got {other:?}"),
        }
    }

    #[test]
    fn plain_dat_has_no_picture_band() {
        // Without a picture blorb the layout is unchanged: room panel over
        // transcript, no graphics window.
        let s = ScottSession::new(dat(), None).unwrap();
        match s.screen().root {
            WinNode::Pair { first, .. } => {
                assert!(matches!(*first, WinNode::Buffer(_)), "no graphics band");
            }
            other => panic!("expected the plain room/transcript pair, got {other:?}"),
        }
    }

    #[test]
    fn in_game_save_bytes_round_trip_via_restore_game_save() {
        // The in-game SAVE opcode writes `save_state().bytes` as the game-save
        // payload; the in-game restore feeds them back through
        // `restore_game_save`. They must round-trip (regression: the payload was
        // empty for Scott, so restore hit "bad Scott snapshot").
        let mut s = ScottSession::new(dat(), None).unwrap();
        let start = s.current_location().unwrap().number;
        let bytes = s.save_state().bytes;
        assert!(!bytes.is_empty(), "the in-game save payload is the VM snapshot");

        s.submit("down");
        assert_ne!(s.current_location().unwrap().number, start, "moved before restore");

        s.restore_game_save(&bytes).expect("restore accepts the snapshot");
        assert_eq!(s.current_location().unwrap().number, start, "restored to the saved room");
    }

    #[test]
    fn restore_state_rejects_foreign_engine() {
        let mut s = ScottSession::new(dat(), None).unwrap();
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        let err = s.restore_state(&foreign).unwrap_err();
        assert!(matches!(err, EngineError::EngineMismatch { .. }));
    }
}
