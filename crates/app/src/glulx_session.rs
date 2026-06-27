//! `GlulxSession`: an [`Engine`] over a `gvm` Glulx machine + the app
//! [`AppGlk`] backend.
//!
//! The Z-machine's synchronous turn model fits `gvm` directly: a turn delivers
//! input to the pending Glk request, then **drives the gvm step loop** (like
//! `gvm-cli`'s `drive`) until the next `glk_select` request or Quit, draining the
//! [`AppGlk`] backend's output into the [`TurnResult`].
//!
//! Automapping/play-aids for Glulx are a later phase (SP4): [`introspect`] and
//! [`current_location`] return `None`, so the map pane and the play-aids stay
//! quiet. Glulx saves are tagged `"glulx"`; the 3b-i foreign-engine restore guard
//! prevents cross-loading a Z-machine save (and vice-versa).
//!
//! [`introspect`]: Engine::introspect
//! [`current_location`]: Engine::current_location

use std::any::Any;
use std::collections::BTreeMap;

use gvm::{GError, Machine, Memory, StepResult};

use crate::engine::{Engine, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel, StatusModel, WinNode};
use crate::glk_backend::AppGlk;
use crate::session::{InputKind, TurnResult};

/// The engine tag recorded in an `EngineSave` produced by the Glulx adapter.
pub const GLULX_ENGINE: &str = "glulx";
/// The save-format version within the `glulx` engine (gvm snapshot).
const GLULX_SAVE_FORMAT: u32 = 1;

// ── key → Glk keycode ──────────────────────────────────────────────────────────

/// Map a neutral [`KeyInput`] to a Glk character-input code (`keycode_*` from
/// `glk.h`, or a Latin-1/Unicode code point for a character).
///
/// Returns `None` for keys with no Glk meaning (Insert, F-keys past 12), so the
/// caller leaves the turn untouched — mirroring the Z-machine adapter's
/// "skip unmapped key" behavior.
pub fn key_to_glk(key: KeyInput) -> Option<u32> {
    use gvm::glk::keycode as kc;
    Some(match key {
        KeyInput::Char(c) => c as u32,
        KeyInput::Enter => kc::RETURN,
        KeyInput::Backspace | KeyInput::Delete => kc::DELETE,
        KeyInput::Tab => kc::TAB,
        KeyInput::Escape => kc::ESCAPE,
        KeyInput::Up => kc::UP,
        KeyInput::Down => kc::DOWN,
        KeyInput::Left => kc::LEFT,
        KeyInput::Right => kc::RIGHT,
        KeyInput::Home => kc::HOME,
        KeyInput::End => kc::END,
        KeyInput::PageUp => kc::PAGE_UP,
        KeyInput::PageDown => kc::PAGE_DOWN,
        // keycode_Func1 is the highest-valued F-key code; FuncN = Func1 - (N-1).
        KeyInput::Func(n) if (1..=12).contains(&n) => kc::FUNC1 - (n as u32 - 1),
        KeyInput::Func(_) | KeyInput::Insert => return None,
    })
}

// ── the session ────────────────────────────────────────────────────────────────

/// A running Glulx game session.
pub struct GlulxSession {
    machine: Machine,
    /// Which kind of input the VM is currently waiting for.
    pending: InputKind,
    /// Whether the game has ended.
    quit: bool,
    /// The last screen snapshot (the backend's tree is only reachable mutably, so
    /// `screen()` returns this cache, refreshed after each turn).
    screen_cache: ScreenModel,
    /// Auxiliary persistent data (Glulx aux persistence is a later phase).
    aux: BTreeMap<String, Vec<u8>>,
    aux_dirty: bool,
}

/// Step the machine until it pauses for input or quits, returning
/// `(pending_kind, quit)`.
fn drive(machine: &mut Machine) -> (InputKind, bool) {
    loop {
        match machine.step() {
            StepResult::Continue => {}
            StepResult::Quit => return (InputKind::Line, true),
            StepResult::NeedLine { .. } => return (InputKind::Line, false),
            StepResult::NeedChar { .. } => return (InputKind::Char, false),
        }
    }
}

impl GlulxSession {
    /// Build a session from a raw Glulx image, reporting a `cols × rows` display.
    ///
    /// Steps to the first input request / quit (driving the opening text into the
    /// backend); the text is NOT drained here, so `take_transcript` returns the
    /// banner.
    pub fn new(image: Vec<u8>, cols: u32, rows: u32) -> Result<GlulxSession, GError> {
        let mem = Memory::new(image)?;
        let backend = Box::new(AppGlk::new(cols, rows));
        let mut machine = Machine::with_glk(mem, backend);
        let (pending, quit) = drive(&mut machine);
        let mut session = GlulxSession {
            machine,
            pending,
            quit,
            screen_cache: blank_screen(),
            aux: BTreeMap::new(),
            aux_dirty: false,
        };
        session.refresh_screen();
        Ok(session)
    }

    /// Report a new display size to the backend (the host story-pane size).
    pub fn set_screen_size(&mut self, cols: u32, rows: u32) {
        self.appglk().set_screen_size(cols, rows);
    }

    fn appglk(&mut self) -> &mut AppGlk {
        self.machine
            .backend_mut()
            .as_any_mut()
            .downcast_mut::<AppGlk>()
            .expect("GlulxSession always drives an AppGlk backend")
    }

    fn refresh_screen(&mut self) {
        self.screen_cache = self.appglk().screen_model();
    }

    /// Drain the primary window output + refresh the screen, building the turn
    /// result. Shared by `submit`/`submit_key`/`resume_*`.
    fn finish_turn(&mut self) -> TurnResult {
        self.machine.flush();
        let (transcript, transcript_runs) = self.appglk().take_transcript();
        self.refresh_screen();
        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        TurnResult {
            transcript,
            transcript_runs,
            location: None,
            quit: self.quit,
            info: None,
            beep: None,
            diagnostics,
            location_method: None,
            pending_io: None,
        }
    }
}

/// The empty initial screen snapshot.
fn blank_screen() -> ScreenModel {
    ScreenModel { root: WinNode::Blank, status: StatusModel::HostManaged }
}

impl Engine for GlulxSession {
    fn submit(&mut self, command: &str) -> TurnResult {
        if !self.quit {
            self.machine.supply_line(command);
            let (pending, quit) = drive(&mut self.machine);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        let code = key_to_glk(key)?;
        if !self.quit {
            self.machine.supply_char(code);
            let (pending, quit) = drive(&mut self.machine);
            self.pending = pending;
            self.quit = quit;
        }
        Some(self.finish_turn())
    }

    fn take_transcript(&mut self) -> String {
        self.machine.flush();
        self.appglk().take_transcript().0
    }

    fn pending_input(&self) -> InputKind {
        self.pending
    }

    fn resume_save(&mut self, _wrote_ok: bool) -> TurnResult {
        // Glulx in-game @save via Glk file streams is a later phase; never
        // bubbles pending_io, so this is not reached in practice.
        self.finish_turn()
    }

    fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult {
        self.finish_turn()
    }

    fn has_quit(&self) -> bool {
        self.quit
    }

    fn screen(&self) -> ScreenModel {
        self.screen_cache.clone()
    }

    fn save_state(&self) -> EngineSave {
        EngineSave::new(GLULX_ENGINE, GLULX_SAVE_FORMAT, self.machine.save_state())
    }

    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError> {
        if !save.is_engine(GLULX_ENGINE) {
            return Err(EngineError::EngineMismatch {
                expected: GLULX_ENGINE.to_string(),
                found: save.engine.clone(),
            });
        }
        self.machine
            .restore_state(&save.bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))?;
        self.refresh_screen();
        Ok(())
    }

    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.aux
    }

    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) {
        self.aux = data;
    }

    fn aux_dirty(&self) -> bool {
        self.aux_dirty
    }

    fn clear_aux_dirty(&mut self) {
        self.aux_dirty = false;
    }

    fn current_location(&self) -> Option<LocationInfo> {
        None // Glulx automapping is SP4.
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    // introspect() / debugger() use the trait defaults (None).
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::WinNode;
    use gvm::glk::keycode;

    // ── A tiny Glulx instruction encoder (mirrors gvm-cli's test encoder) ──────

    #[derive(Clone, Copy)]
    enum E {
        Imm(u32),
        LocLoad(u8),
        LocStore(u8),
        MemLoad(u32),
        Push,
        Discard,
    }
    fn emode(e: E) -> u8 {
        match e {
            E::Imm(_) => 3,
            E::LocLoad(_) | E::LocStore(_) => 9,
            E::MemLoad(_) => 7,
            E::Push => 8,
            E::Discard => 0,
        }
    }
    fn edata(e: E) -> Vec<u8> {
        match e {
            E::Imm(v) | E::MemLoad(v) => v.to_be_bytes().to_vec(),
            E::LocLoad(o) | E::LocStore(o) => vec![o],
            E::Push | E::Discard => vec![],
        }
    }
    fn enc(op: u32, args: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        if op <= 0x7f {
            out.push(op as u8);
        } else {
            out.extend_from_slice(&((op | 0x8000) as u16).to_be_bytes());
        }
        let mut modes = vec![0u8; args.len().div_ceil(2)];
        for (i, &a) in args.iter().enumerate() {
            let m = emode(a);
            if i % 2 == 0 {
                modes[i / 2] |= m;
            } else {
                modes[i / 2] |= m << 4;
            }
        }
        out.extend_from_slice(&modes);
        for &a in args {
            out.extend(edata(a));
        }
        out
    }

    // RAM layout (RAMSTART 0x400, ENDMEM 0x500): the event struct, the
    // line-input length word (event.val1), and the line buffer.
    const EVENT: u32 = 0x400;
    const VAL1: u32 = 0x408;
    const LINEBUF: u32 = 0x480;

    /// Wrap `body` as a start function with `nlocals` 4-byte locals into a
    /// runnable image (RAMSTART 0x400, ENDMEM 0x500 → RAM [0x400, 0x500) holds
    /// the event struct + line buffer; code lives in ROM [0x24, 0x400)).
    fn image_for(body: Vec<u8>, nlocals: u8) -> Vec<u8> {
        let mut func = vec![0xC1u8, 0x04, nlocals, 0x00, 0x00]; // type C1; nlocals 4-byte
        func.extend(body);
        let (ramstart, endmem) = (0x400u32, 0x500u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes());
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    /// Open a TextBuffer (id → local0) and make it current.
    fn open_buffer_prelude() -> Vec<u8> {
        use E::*;
        let mut b = enc(0x149, &[Imm(2), Imm(0)]); // setiosys glk
        for v in [Imm(0), Imm(3), Imm(0), Imm(0), Imm(0)] {
            b.extend(enc(0x40, &[v, Push])); // rock, wintype=3, size, method, split
        }
        b.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(0)])); // window_open → local0
        b.extend(enc(0x40, &[LocLoad(0), Push]));
        b.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(local0)
        b
    }

    fn streamchar(c: u8) -> Vec<u8> {
        vec![0x70, 0x01, c] // streamchar (opcode 0x70; mode C8 small const)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn key_to_glk_maps_special_keys_and_chars() {
        assert_eq!(key_to_glk(KeyInput::Char('z')), Some(b'z' as u32));
        assert_eq!(key_to_glk(KeyInput::Enter), Some(keycode::RETURN));
        assert_eq!(key_to_glk(KeyInput::Backspace), Some(keycode::DELETE));
        assert_eq!(key_to_glk(KeyInput::Delete), Some(keycode::DELETE));
        assert_eq!(key_to_glk(KeyInput::Tab), Some(keycode::TAB));
        assert_eq!(key_to_glk(KeyInput::Escape), Some(keycode::ESCAPE));
        assert_eq!(key_to_glk(KeyInput::Up), Some(keycode::UP));
        assert_eq!(key_to_glk(KeyInput::Down), Some(keycode::DOWN));
        assert_eq!(key_to_glk(KeyInput::Left), Some(keycode::LEFT));
        assert_eq!(key_to_glk(KeyInput::Right), Some(keycode::RIGHT));
        assert_eq!(key_to_glk(KeyInput::Home), Some(keycode::HOME));
        assert_eq!(key_to_glk(KeyInput::End), Some(keycode::END));
        assert_eq!(key_to_glk(KeyInput::PageUp), Some(keycode::PAGE_UP));
        assert_eq!(key_to_glk(KeyInput::PageDown), Some(keycode::PAGE_DOWN));
        assert_eq!(key_to_glk(KeyInput::Func(1)), Some(keycode::FUNC1));
        assert_eq!(key_to_glk(KeyInput::Func(12)), Some(keycode::FUNC12));
        // Keys with no Glk meaning are skipped.
        assert_eq!(key_to_glk(KeyInput::Insert), None);
        assert_eq!(key_to_glk(KeyInput::Func(13)), None);
    }

    #[test]
    fn submit_echoes_line_with_runs_and_screen_is_two_window_tree() {
        use E::*;
        // Build the program inline (clearer than the helper).
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // grid → local1
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0x2f), Imm(1), Discard])); // set_window(buffer)
        body.extend(streamchar(b'O'));
        body.extend(streamchar(b'K'));
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        // Header-styled 'Y'.
        body.extend(enc(0x40, &[Imm(3), Push]));
        body.extend(enc(0x130, &[Imm(0x86), Imm(1), Discard])); // set_style(Header)
        body.extend(streamchar(b'Y'));
        body.extend(enc(0x40, &[Imm(0), Push]));
        body.extend(enc(0x130, &[Imm(0x86), Imm(1), Discard])); // set_style(Normal)
        // Echo the typed line: put_buffer(0x180, len=mem[0x108]).
        body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // len
        body.extend(enc(0x40, &[Imm(LINEBUF), Push])); // addr
        body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 2), 80, 24).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line);
        // Banner drained.
        assert_eq!(sess.take_transcript(), "OK");
        // screen() is a grid-over-buffer pair.
        let model = sess.screen();
        match &model.root {
            WinNode::Pair { vertical, first, second, .. } => {
                assert!(*vertical);
                assert!(matches!(**first, WinNode::Grid(_)));
                assert!(matches!(**second, WinNode::Buffer(_)));
            }
            other => panic!("expected a grid-over-buffer Pair, got {other:?}"),
        }
        assert!(model.grid().is_some());

        // Submit a line → "Y" (bold) then the echoed "hi"; the program quits.
        let r = sess.submit("hi");
        assert_eq!(r.transcript, "Yhi");
        assert!(r.quit);
        assert!(
            r.transcript_runs.iter().any(|&(_, bits)| bits == 0x02),
            "the Header-styled 'Y' contributes a bold run: {:?}",
            r.transcript_runs
        );
    }

    /// Program: open a buffer, request a char, echo it as a char, quit.
    fn char_echo_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        body.extend(enc(0x40, &[LocLoad(0), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // the key code (val1)
        body.extend(enc(0x130, &[Imm(0x80), Imm(1), Discard])); // glk_put_char(key)
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    #[test]
    fn submit_key_delivers_char_and_skips_unmapped() {
        let mut sess = GlulxSession::new(char_echo_image(), 80, 24).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char);
        // An unmapped key (Insert) leaves the VM untouched.
        assert!(sess.submit_key(KeyInput::Insert).is_none());
        assert_eq!(sess.pending_input(), InputKind::Char, "VM untouched by unmapped key");
        // A printable key is delivered, echoed, and drives to quit.
        let r = sess.submit_key(KeyInput::Char('Z')).expect("mapped key produces a turn");
        assert_eq!(r.transcript, "Z");
        assert!(r.quit);
    }

    /// Program: open a buffer, request a line, quit on input.
    fn simple_line_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 1)
    }

    #[test]
    fn save_state_is_tagged_and_round_trips_with_guard() {
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24).expect("new");
        let save = sess.save_state();
        assert_eq!(save.engine, GLULX_ENGINE);
        assert!(!save.bytes.is_empty(), "gvm snapshot is non-empty");
        // Same-engine restore succeeds.
        sess.restore_state(&save).expect("same-engine restore");
        // A foreign-engine save is refused.
        let foreign = EngineSave::new("zmachine", 1, save.bytes.clone());
        match sess.restore_state(&foreign) {
            Err(EngineError::EngineMismatch { expected, found }) => {
                assert_eq!(expected, GLULX_ENGINE);
                assert_eq!(found, "zmachine");
            }
            other => panic!("foreign restore must be refused, got {other:?}"),
        }
    }

    #[test]
    fn introspect_and_location_are_none() {
        let sess = GlulxSession::new(simple_line_image(), 80, 24).expect("new");
        assert!(sess.introspect().is_none(), "Glulx introspection is SP4");
        assert!(sess.current_location().is_none(), "Glulx automapping is SP4");
    }
}
