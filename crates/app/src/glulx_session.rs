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
use std::time::{Duration, Instant};

use gvm::{GError, Machine, Memory, StepResult};

use crate::engine::{Engine, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel, StatusModel, WinNode};
use crate::glk_backend::AppGlk;
use crate::session::{clamp_runs, strip_read_prompt, trim_elems_to_len, InputKind, TranscriptElem, TurnResult};
use zvm::location::LocationMethod;

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
    /// The current room, derived from the last Inform `Subheader` heading and
    /// held sticky across heading-less turns (examine/talk/failed-move).
    last_room: Option<LocationInfo>,
}

/// Wall-clock budget for a single drive (one turn's worth of execution). A
/// well-behaved game reaches an input request in milliseconds; if it runs this
/// long it is assumed to be in a runaway loop (e.g. layout code that cannot
/// converge on a given screen geometry) and the turn is aborted as a recoverable
/// fault so the app survives instead of hard-hanging. Generous, because this is a
/// last-resort backstop — the tree-driven size snapping in `gvm` already prevents
/// the known cause. Set via env `BABELMAP_TURN_BUDGET_MS` for testing.
fn turn_budget() -> Duration {
    std::env::var("BABELMAP_TURN_BUDGET_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(10))
}

/// Step the machine until it pauses for input or quits, returning
/// `(pending_kind, quit)`. Aborts a runaway turn via the wall-clock watchdog.
fn drive(machine: &mut Machine) -> (InputKind, bool) {
    let budget = turn_budget();
    let start = Instant::now();
    let mut steps: u64 = 0;
    loop {
        match machine.step() {
            StepResult::Continue => {
                steps += 1;
                // Checking the clock every step is too costly; sample periodically.
                if steps.is_multiple_of(1_000_000) && start.elapsed() > budget {
                    machine.abort_with_fault(format!(
                        "turn aborted after {:?} / {steps} steps with no input request \
                         (runaway game loop); the app stays interactive",
                        start.elapsed()
                    ));
                    return (InputKind::Line, true);
                }
            }
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
    pub fn new(
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
    ) -> Result<GlulxSession, GError> {
        let mem = Memory::new(image)?;
        let picts = crate::graphics::PictSource::new(pict_blorb);
        let backend = Box::new(AppGlk::with_graphics(cols, rows, char_px, picts));
        let mut machine = Machine::with_glk(mem, backend);
        machine.set_acceleration(acceleration);
        machine.set_graphics(graphics_enabled);
        let (pending, quit) = drive(&mut machine);
        let mut session = GlulxSession {
            machine,
            pending,
            quit,
            screen_cache: blank_screen(),
            aux: BTreeMap::new(),
            aux_dirty: false,
            last_room: None,
        };
        session.refresh_screen();
        session.last_room =
            session.appglk().take_room_heading().map(|n| heading_to_room(&n));
        Ok(session)
    }

    /// Report a new display size to the backend (the host story-pane size).
    pub fn set_screen_size(&mut self, cols: u32, rows: u32) {
        self.appglk().set_screen_size(cols, rows);
    }

    /// React to a change in the host story-pane size: report it to the backend,
    /// relayout the window tree to the new geometry (rescaling graphics canvases),
    /// and — if the game is waiting on input — deliver a Glk Arrange event so it
    /// redraws (e.g. a graphics window repaints its image at the new size). Drives
    /// the game's redraw to its next input request and refreshes the screen cache.
    /// A no-op once the game has quit.
    pub fn resize(&mut self, cols: u32, rows: u32) {
        if self.quit {
            return;
        }
        self.set_screen_size(cols, rows);
        self.machine.rearrange();
        let (pending, quit) = drive(&mut self.machine);
        self.pending = pending;
        self.quit = quit;
        self.refresh_screen();
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
        // Drain the primary window's ordered elements (text runs + inline
        // images). Derive the flat `raw`/`raw_runs` from the Text elements for
        // the existing consumers (banner-strip, location, tests): the
        // concatenation of element text equals what the old text-only drain
        // produced.
        let mut elems = self.appglk().take_transcript_elems();
        let mut raw = String::new();
        let mut raw_runs = Vec::new();
        for e in &elems {
            if let TranscriptElem::Text { text, runs } = e {
                raw.push_str(text);
                raw_runs.extend(runs.iter().copied());
            }
        }
        // Strip the game's trailing read prompt (e.g. a final "\n>") so the app's
        // own bottom input bar is the only ">" shown -- mirroring the Z-machine
        // path. clamp_runs keeps the style chunks aligned with the shortened text,
        // and trim_elems_to_len applies the same shortening to the element list so
        // the ordered elems stay consistent with the flat `transcript`.
        let transcript = strip_read_prompt(&raw).to_owned();
        let kept = transcript.chars().count();
        let transcript_runs = clamp_runs(raw_runs, kept);
        trim_elems_to_len(&mut elems, kept);
        self.refresh_screen();
        if let Some(name) = self.appglk().take_room_heading() {
            self.last_room = Some(heading_to_room(&name));
        }
        let location = self.last_room.clone();
        let location_method = location.as_ref().map(|_| LocationMethod::RoomHeading);
        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit: self.quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            diagnostics,
            fault,
            location_method,
            pending_io: None,
            timed_out: false,
            transcript_elems: elems,
        }
    }
}

/// Build a name-based room snapshot from an Inform room heading. Glulx has no
/// readable object tree, so identity is the synthetic id of the normalized name.
fn heading_to_room(name: &str) -> LocationInfo {
    zvm::ObjectSnapshot {
        number: crate::roomid::synthetic_room_id(name),
        parent: 0,
        name: name.to_string(),
    }
}

/// The empty initial screen snapshot.
fn blank_screen() -> ScreenModel {
    ScreenModel {
        root: WinNode::Blank,
        status: StatusModel::HostManaged,
        bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
        fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
    }
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
        strip_read_prompt(&self.appglk().take_transcript().0).to_owned()
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
        self.last_room.clone()
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

        let mut sess = GlulxSession::new(image_for(body, 2), 80, 24, true, false, (1, 1), None).expect("new");
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
            r.transcript_runs.iter().any(|&(_, bits, _, _)| bits == 0x02),
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
        let mut sess = GlulxSession::new(char_echo_image(), 80, 24, true, false, (1, 1), None).expect("new");
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

    /// Program: open a buffer, split off a proportional (50%) graphics window
    /// below it, fill a rect to create its canvas, request a line, then select
    /// twice so the game stays alive across an Arrange (re-suspending on the
    /// persisted line request). local0 = buffer, local1 = graphics window.
    fn graphics_split_line_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude();
        // window_open(split=buffer, method=Below|Proportional=0x23, size=50,
        // wintype_Graphics=5, rock=0) → local1. Push order is args reversed.
        for v in [Imm(0), Imm(5), Imm(50), Imm(0x23), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (byte offset 4)
        // fill_rect(win=graphics, color=0x00FFFFFF, left=0, top=0, w=4, h=4) to
        // materialize the canvas so relayout has something to resize.
        for v in [Imm(4), Imm(4), Imm(0), Imm(0), Imm(0x00FF_FFFF), LocLoad(4)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xEA), Imm(6), Discard])); // glk_window_fill_rect
        // request_line_event(win=buffer, LINEBUF, maxlen=20, initlen=0).
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        // Three selects: #1 drains the Redraw queued by opening the graphics
        // window; #2 is where new() suspends on the line request; #3 is where the
        // game re-suspends after the resize()-delivered Arrange (proving survival).
        for _ in 0..3 {
            body.extend(enc(0x40, &[Imm(EVENT), Push]));
            body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        }
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    /// Find the first Graphics leaf's canvas pixel size in a screen model.
    fn graphics_canvas_dims(node: &WinNode) -> Option<(u32, u32)> {
        match node {
            WinNode::Graphics(g) => Some((g.canvas.width(), g.canvas.height())),
            WinNode::Pair { first, second, .. } => {
                graphics_canvas_dims(first).or_else(|| graphics_canvas_dims(second))
            }
            _ => None,
        }
    }

    #[test]
    fn resize_rescales_graphics_canvas_and_game_survives_arrange() {
        // char_px = (2, 2): a 50%-height graphics window under an 80x24 screen is
        // 80 cols x 12 rows → 160 x 24 px. Shrinking the pane to 40 cols halves
        // its width to 80 px; the game re-suspends on its line request (not quit).
        let mut sess =
            GlulxSession::new(graphics_split_line_image(), 80, 24, true, true, (2, 2), None)
                .expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line);
        let (w0, h0) = graphics_canvas_dims(&sess.screen().root).expect("a graphics window");
        assert_eq!((w0, h0), (160, 24), "initial canvas from the 80-col virtual screen");

        sess.resize(40, 24);
        assert_eq!(sess.pending_input(), InputKind::Line, "game re-suspended on its line request");
        assert!(!sess.has_quit(), "an Arrange must not end the game");
        let (w1, h1) = graphics_canvas_dims(&sess.screen().root).expect("a graphics window");
        assert_eq!((w1, h1), (80, 24), "canvas tracks the narrower pane after resize");
    }

    #[test]
    fn resize_after_quit_is_a_noop() {
        // A quit session must ignore resize (no drive, no panic).
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None)
            .expect("new");
        let r = sess.submit("go"); // drives to quit
        assert!(r.quit);
        sess.resize(40, 12); // must be a harmless no-op
        assert!(sess.has_quit());
    }

    #[test]
    fn trailing_read_prompt_is_stripped_from_banner_and_turns() {
        use E::*;
        // Print "Hi\n> ", request a line (banner), then after input print
        // "done\n> " and request again (a turn). Both the banner and the turn
        // transcript must drop the trailing prompt so the app's bottom input bar
        // is the only ">" the player sees (matches the Z-machine path).
        let mut body = open_buffer_prelude();
        for c in b"Hi\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (banner)
        for c in b"done\n> " {
            body.extend(streamchar(*c));
        }
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push]));
        }
        body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select (turn)
        body.extend(enc(0x120, &[])); // quit
        let image = image_for(body, 1);

        let mut sess = GlulxSession::new(image, 80, 24, true, false, (1, 1), None).expect("new");
        assert_eq!(sess.take_transcript(), "Hi", "banner drops the trailing prompt");
        let r = sess.submit("x");
        assert_eq!(r.transcript, "done", "turn output drops the trailing prompt");
    }

    #[test]
    fn save_state_is_tagged_and_round_trips_with_guard() {
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None).expect("new");
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
    fn introspect_is_none() {
        let sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None).expect("new");
        assert!(sess.introspect().is_none(), "Glulx introspection is SP4");
    }

    #[test]
    fn heading_to_room_uses_synthetic_id() {
        let r = super::heading_to_room("Studio Apartment");
        assert_eq!(r.name, "Studio Apartment");
        assert_eq!(r.parent, 0);
        assert_eq!(r.number, crate::roomid::synthetic_room_id("Studio Apartment"));
        // Same name → same id (identity is name-based).
        assert_eq!(super::heading_to_room("Studio Apartment").number, r.number);
    }

    #[test]
    fn glulx_state_round_trips_through_babelmap_archive() {
        use std::collections::BTreeMap;
        // A Glulx engine save survives a .babelmap archive round-trip: write its
        // EngineSave (no screen.json), reload, and restore into a FRESH session
        // through Engine::restore_state — state is preserved, no panic.
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None).expect("new");
        let _ = sess.take_transcript(); // drain the banner
        let es = sess.save_state();
        assert_eq!(es.engine, GLULX_ENGINE);

        let mut path = std::env::temp_dir();
        path.push(format!("babelmap-glulx-arch-{}.babelmap", std::process::id()));
        let mapper = mapper::mapper::Mapper::default();
        crate::archive::save_archive(&path, &mapper, &es, None, &BTreeMap::new(),
            &[], &[], &[], &[], &[]).expect("save archive");

        let ac = crate::archive::load_archive(&path).expect("load archive");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.engine, GLULX_ENGINE, "archive records the glulx tag");
        assert!(ac.screen.is_none(), "Glulx archive carries no screen.json");
        assert_eq!(ac.save, es.bytes, "archived bytes are the Glulx save");

        let mut fresh = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None).expect("new");
        let _ = fresh.take_transcript();
        fresh.restore_state(&ac.engine_save()).expect("Glulx restore from archive");
        assert_eq!(fresh.pending_input(), InputKind::Line, "restored input state");
        assert_eq!(fresh.save_state().bytes, es.bytes, "restored Glulx state matches");
    }

    #[test]
    fn glulx_restore_refuses_zmachine_archive() {
        // The foreign-engine guard fires gracefully (no panic) when a zmachine
        // save is offered to a Glulx session.
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, (1, 1), None).expect("new");
        let foreign = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        assert!(matches!(
            sess.restore_state(&foreign),
            Err(EngineError::EngineMismatch { .. })
        ));
    }

    /// End-to-end smoke: a hand-built .ulx plays through GlulxSession and renders
    /// through the generic story-pane renderer with the map pane bolted alongside.
    #[test]
    fn glulx_plays_and_renders_end_to_end() {
        use crate::render::screen::render_story_pane;
        use crate::state::AppState;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;

        // Program: open a buffer, print "Hello", request a line, echo it, quit.
        let image = {
            use E::*;
            let mut body = open_buffer_prelude();
            for c in b"Hello" {
                body.extend(streamchar(*c));
            }
            for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
                body.extend(enc(0x40, &[v, Push]));
            }
            body.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
            body.extend(enc(0x40, &[Imm(EVENT), Push]));
            body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
            body.extend(enc(0x40, &[MemLoad(VAL1), Push])); // len
            body.extend(enc(0x40, &[Imm(LINEBUF), Push])); // addr
            body.extend(enc(0x130, &[Imm(0x84), Imm(2), Discard])); // glk_put_buffer
            body.extend(enc(0x120, &[])); // quit
            image_for(body, 1)
        };

        let mut sess = GlulxSession::new(image, 78, 20, true, false, (1, 1), None).expect("new");

        // Mirror the app loop: drain the banner into the transcript, take a turn.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let banner = sess.take_transcript();
        assert_eq!(banner, "Hello");
        state.push_transcript(&banner);

        let r = sess.submit("there");
        assert_eq!(r.transcript, "there");
        state.push_transcript(&r.transcript);

        // Render the Glulx screen into a story pane (no panic, content present).
        let area = Rect::new(0, 0, 78, 20);
        let mut buf = Buffer::empty(area);
        let _ = render_story_pane(&sess.screen(), false, sess.introspect(), &state, area, &mut buf);

        let dump: String = (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(dump.contains("Hello"), "banner rendered in the story pane");
        assert!(dump.contains("there"), "echoed line rendered in the story pane");
    }
}
