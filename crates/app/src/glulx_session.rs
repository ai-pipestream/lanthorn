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

use gvm::glk::{GlkBackend, Rect as GlkRect, WinType};
use gvm::{GError, Machine, Memory, StepResult};

use crate::engine::{Engine, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel, StatusModel, WinNode};
use crate::glk_backend::AppGlk;
use crate::session::{clamp_runs, strip_read_prompt, trim_elems_to_len, FilenameReq, InputKind, PendingIo, TranscriptElem, TurnResult};
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
    /// A game-initiated save/restore awaiting the host's file I/O, bubbled to the
    /// run loop via the next `TurnResult`. Set when a turn's drive stops on an
    /// `@save`/`@restore`; cleared by `resume_save`/`resume_restore`.
    pending_io: Option<PendingIo>,
    /// A game-initiated create_by_prompt awaiting a host filename, bubbled to the
    /// run loop. Set when a turn's drive stops on one; cleared by resume_filename.
    pending_filename: Option<FilenameReq>,
    /// The last screen snapshot (the backend's tree is only reachable mutably, so
    /// `screen()` returns this cache, refreshed after each turn).
    screen_cache: ScreenModel,
    /// Auxiliary persistent data (Glulx aux persistence is a later phase).
    aux: BTreeMap<String, Vec<u8>>,
    aux_dirty: bool,
    /// The current room, derived from the last Inform `Subheader` heading and
    /// held sticky across heading-less turns (examine/talk/failed-move).
    last_room: Option<LocationInfo>,
    /// When false, the game's own trailing `>` read prompt is kept in the
    /// transcript instead of being stripped. Default true. See
    /// [`Engine::set_strip_prompt`].
    strip_prompt: bool,
    /// The per-story persistent store for the game's OWN fixed-name saves
    /// (`create_by_name`). Empty = no store (game-auto saves auto-fail). See
    /// [`drive_auto`].
    game_dir: std::path::PathBuf,
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

/// Why a [`drive`] loop stopped.
enum DriveStop {
    /// The VM is waiting for input of this kind.
    Input(InputKind),
    /// The VM quit (or the runaway watchdog aborted the turn).
    Quit,
    /// The game executed `@save`: the host must write a save file, then
    /// [`GlulxSession::resume_save`].
    Save,
    /// The game executed `@restore`: the host must pick a save file, then
    /// [`GlulxSession::resume_restore`].
    Restore,
    /// The game executed `create_by_prompt`: the host must supply a filename, then
    /// [`GlulxSession::resume_filename`].
    Filename { usage: u32, fmode: u32 },
}

/// Step the machine until it pauses for input, quits, or requests an in-game
/// save/restore. Aborts a runaway turn via the wall-clock watchdog.
fn drive(machine: &mut Machine) -> DriveStop {
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
                    return DriveStop::Quit;
                }
            }
            StepResult::Quit => return DriveStop::Quit,
            StepResult::NeedLine { .. } => return DriveStop::Input(InputKind::Line),
            StepResult::NeedChar { .. } => return DriveStop::Input(InputKind::Char),
            StepResult::SaveRequest => return DriveStop::Save,
            StepResult::RestoreRequest => return DriveStop::Restore,
            StepResult::NeedFilename { usage, fmode } => {
                // SavedGame usage: @save/@restore are host-intercepted (they become
                // Save/Restore requests served by babelmap's own Save State dialogs),
                // so a save-file prompt here is redundant AND would split the turn
                // before @save. Auto-name it and keep driving to the opcode; the
                // synthesized VFS file is unused (the snapshot is host-side). Only the
                // real consumers (Transcript / InputRecord / Data) surface a request.
                if usage & 0x0f == 0x01 {
                    machine.supply_filename(Some(format!("__prompt_{}__", usage & 0x0f)));
                } else {
                    return DriveStop::Filename { usage, fmode };
                }
            }
        }
    }
}

/// Drive to a player-facing stop, transparently servicing the game's OWN
/// (`create_by_name`) `@save`/`@restore` against `game_dir` — no host UI. A
/// game-managed `@save` writes `<game_dir>/<name>.qzl`; a game-managed
/// `@restore` reads it if present, else fails cleanly (so a first run runs the
/// game's init). Only the player's SAVE/RESTORE verb (`create_by_prompt`), or
/// any save when no store is configured (`game_dir` empty), bubbles up as
/// `DriveStop::Save`/`Restore`.
/// Seed the machine's host-managed SavedGame existence index from every
/// `<game_dir>/*.qzl` on disk (raw basename minus the `.qzl` suffix, matching
/// how [`drive_auto`] writes and how the index is keyed), so a `create_by_name`
/// game probing `glk_fileref_does_file_exist` before `@restore` sees its save
/// across launches (SQ-0301). No-op when `game_dir` is empty (the no-store
/// path) or unreadable. Over-seeding player-save `.qzl` names is inert — a game
/// only ever probes names it created.
fn seed_saved_games(machine: &mut Machine, game_dir: &std::path::Path) {
    if game_dir.as_os_str().is_empty() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(game_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("qzl") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as u32;
        machine.seed_saved_game_file(name.to_string(), size);
    }
}

fn drive_auto(machine: &mut Machine, game_dir: &std::path::Path) -> DriveStop {
    loop {
        let stop = drive(machine);
        let restore = match stop {
            DriveStop::Save => false,
            DriveStop::Restore => true,
            other => return other,
        };
        let req = machine.pending_saveload_request().unwrap_or_default();
        // The player's verb (by_prompt), an unknown target, or no store: let the
        // caller decide (host UI in a turn, auto-fail in a non-interactive drive).
        if req.by_prompt || req.name.is_empty() || game_dir.as_os_str().is_empty() {
            return stop;
        }
        let path = game_dir.join(format!("{}.qzl", req.name));
        if restore {
            match std::fs::read(&path) {
                Ok(bytes) if machine.complete_restore_quetzal(&bytes) => {}
                _ => machine.complete_restore_failure(),
            }
        } else {
            let ok = std::fs::create_dir_all(game_dir).is_ok()
                && std::fs::write(&path, machine.save_quetzal()).is_ok();
            machine.complete_save(ok);
        }
        // Keep driving: the op may chain into another game-managed save/restore.
    }
}

/// Drive to an input request or quit, returning `(pending_kind, quit)`. A
/// player `@save`/`@restore` fired during a non-interactive drive (startup,
/// resize, sound-notify) is auto-failed — those paths have no UI to prompt the
/// player, and leaving the VM suspended would wedge the next turn. The game's
/// OWN fixed-name saves are serviced silently by [`drive_auto`] first.
fn drive_settled(machine: &mut Machine, game_dir: &std::path::Path) -> (InputKind, bool) {
    loop {
        match drive_auto(machine, game_dir) {
            DriveStop::Input(k) => return (k, false),
            DriveStop::Quit => return (InputKind::Line, true),
            DriveStop::Save => machine.complete_save(false),
            DriveStop::Restore => machine.complete_restore_failure(),
            DriveStop::Filename { .. } => machine.supply_filename(None),
        }
    }
}

impl GlulxSession {
    /// Build a session from a raw Glulx image, reporting a `cols × rows` display.
    ///
    /// Steps to the first input request / quit (driving the opening text into the
    /// backend); the text is NOT drained here, so `take_transcript` returns the
    /// banner.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
        vfs_bytes: &[u8],
    ) -> Result<GlulxSession, GError> {
        // No persistent store: the game's own fixed-name @save/@restore auto-fail
        // (empty game_dir), matching pre-persistence behavior. Used by tests.
        Self::new_in(
            std::path::PathBuf::new(), image, cols, rows, acceleration,
            graphics_enabled, sound_enabled, char_px, pict_blorb, vfs_bytes,
        )
    }

    /// Like [`GlulxSession::new`] but with a `game_dir` persistent store: the
    /// game's OWN fixed-name saves (`create_by_name` — CM's init cache, undo,
    /// autotesting) are serviced silently against `<game_dir>/<name>.qzl` during
    /// every drive (including boot), with no host UI. Only the player's SAVE/
    /// RESTORE verb (`create_by_prompt`) bubbles up for the app's saves dialog.
    #[allow(clippy::too_many_arguments)]
    pub fn new_in(
        game_dir: std::path::PathBuf,
        image: Vec<u8>,
        cols: u32,
        rows: u32,
        acceleration: bool,
        graphics_enabled: bool,
        sound_enabled: bool,
        char_px: (u32, u32),
        pict_blorb: Option<blorb::Blorb>,
        vfs_bytes: &[u8],
    ) -> Result<GlulxSession, GError> {
        let mem = Memory::new(image)?;
        let picts = crate::graphics::PictSource::new(pict_blorb);
        let backend = Box::new(AppGlk::with_graphics(cols, rows, char_px, picts));
        let mut machine = Machine::with_glk(mem, backend);
        machine.set_acceleration(acceleration);
        machine.set_graphics(graphics_enabled);
        machine.set_sound(sound_enabled);
        // Load the per-story Glk file VFS sidecar BEFORE booting: a Glulx game
        // may read a cache during boot (e.g. CM skips its long init) or write one
        // (leaving vfs_dirty set), so the sidecar must be in place first (SQ-0290).
        machine.load_vfs(vfs_bytes);
        // Reseed host-managed SavedGame slots from disk BEFORE booting: `.qzl`
        // saves live in host files decoupled from the VFS and the machine's
        // existence index is session-transient, so a create_by_name game probing
        // glk_fileref_does_file_exist during init would otherwise never see its
        // own on-disk save across launches (SQ-0301).
        seed_saved_games(&mut machine, &game_dir);
        let (pending, quit) = drive_settled(&mut machine, &game_dir);
        let mut session = GlulxSession {
            machine,
            pending,
            quit,
            pending_io: None,
            pending_filename: None,
            screen_cache: blank_screen(),
            aux: BTreeMap::new(),
            aux_dirty: false,
            last_room: None,
            strip_prompt: true,
            game_dir,
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

    /// Enable/disable Glk sound on the running VM (the Sound gestalt + schannel
    /// dispatch), so a runtime sound toggle reaches games that re-check
    /// `gestalt_Sound` before playing.
    pub fn set_sound(&mut self, on: bool) {
        self.machine.set_sound(on);
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
        let (pending, quit) = drive_settled(&mut self.machine, &self.game_dir);
        self.pending = pending;
        self.quit = quit;
        self.refresh_screen();
    }

    /// Drive one turn's worth of execution, updating `pending`/`quit`/`pending_io`.
    /// On an in-game `@save`/`@restore` the drive stops with `pending_io` set (and
    /// `pending`/`quit` left unchanged, since the game is mid-turn); the run loop
    /// performs the file I/O and calls `resume_save`/`resume_restore`.
    fn drive_turn(&mut self) {
        match drive_auto(&mut self.machine, &self.game_dir) {
            DriveStop::Input(k) => {
                self.pending = k;
                self.quit = false;
                self.pending_io = None;
                self.pending_filename = None;
            }
            DriveStop::Quit => {
                self.quit = true;
                self.pending_io = None;
                self.pending_filename = None;
            }
            DriveStop::Save => self.pending_io = Some(PendingIo::Save),
            DriveStop::Restore => self.pending_io = Some(PendingIo::Restore),
            DriveStop::Filename { usage, fmode } => {
                self.pending_filename = Some(FilenameReq { usage, fmode })
            }
        }
    }

    fn appglk(&mut self) -> &mut AppGlk {
        self.machine
            .backend_mut()
            .as_any_mut()
            .downcast_mut::<AppGlk>()
            .expect("GlulxSession always drives an AppGlk backend")
    }

    fn refresh_screen(&mut self) {
        let mut model = self.appglk().screen_model();
        // Honor the game's Normal buffer-window colours as the pane background/
        // foreground, so a game that styles its text for a light interpreter
        // (e.g. CounterfeitMonkey's black-on-white intro) paints a matching pane
        // instead of leaving white text-islands on the dark theme. Only overrides
        // a channel the game actually set (via glk_stylehint_set); an unset
        // channel stays Default and the theme wins. (SQ-0196)
        let normal = self.machine.style_colour(WinType::TextBuffer, gvm::glk::GlkStyle::Normal);
        if let Some(rgb) = normal.bg {
            model.bg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(rgb));
        }
        if let Some(rgb) = normal.fg {
            model.fg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(rgb));
        }
        self.screen_cache = model;
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
        let transcript = if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw };
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
        let glulx_sound_ops = self.appglk().take_sound_ops();
        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit: self.quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops,
            diagnostics,
            fault,
            location_method,
            pending_io: self.pending_io.take(),
            timed_out: false,
            transcript_elems: elems,
        }
    }

    /// The Glk timer interval the game has requested (via
    /// `glk_request_timer_events`), or `None` when timer events are off. The host
    /// reads this to arm its own clock and calls [`Self::deliver_timer`] once per
    /// interval.
    pub fn timer_interval(&self) -> Option<std::time::Duration> {
        self.machine
            .glk_timer_interval()
            .map(|ms| std::time::Duration::from_millis(ms as u64))
    }

    /// A timer interval elapsed: deliver a Glk `Evtype_Timer` event to the game
    /// and drive it to its next input request, returning the resulting turn. A
    /// no-op turn once the game has quit.
    pub fn deliver_timer(&mut self) -> TurnResult {
        if !self.quit {
            self.machine.deliver_timer();
            let (pending, quit) = drive_settled(&mut self.machine, &self.game_dir);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }

    /// A sound finished: deliver a Glk `Evtype_SoundNotify` to the game and drive
    /// it to its next input request, returning the resulting turn (which carries
    /// any sound ops the game buffered while handling the notify — sound
    /// sequencing). A no-op turn once the game has quit.
    pub fn sound_notify(&mut self, sound: u32, notify: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_sound_notify(sound, notify);
            let (pending, quit) = drive_settled(&mut self.machine, &self.game_dir);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }

    /// The layout rects (in story-pane cells) of every window with an active Glk
    /// mouse request — the windows a terminal click may be diverted into. Empty
    /// when no window is watching for clicks.
    pub fn mouse_windows(&mut self) -> Vec<(u32, WinType, GlkRect)> {
        let layout: Vec<(u32, WinType, GlkRect)> = self.appglk().layout().to_vec();
        layout
            .into_iter()
            .filter(|&(id, _, _)| self.machine.mouse_requested(id))
            .collect()
    }

    /// The layout rects (in story-pane cells) of every window with an active Glk
    /// hyperlink request — the windows a click on a linked transcript cell may be
    /// diverted into. Empty when no window is watching for hyperlink clicks.
    pub fn hyperlink_windows(&mut self) -> Vec<(u32, WinType, GlkRect)> {
        let layout: Vec<(u32, WinType, GlkRect)> = self.appglk().layout().to_vec();
        layout
            .into_iter()
            .filter(|&(id, _, _)| self.machine.hyperlink_requested(id))
            .collect()
    }

    /// The `(width, height)` of one text-grid cell in pixels — used to convert a
    /// click's window-relative cells into pixels for a graphics window.
    pub fn char_pixels(&mut self) -> (u32, u32) {
        self.appglk().char_pixels()
    }

    /// A terminal click landed inside a mouse-watching window: deliver a Glk
    /// `Evtype_MouseInput` event at window-relative `(x, y)` and drive the game to
    /// its next input request. A no-op turn once the game has quit. `x`/`y` are
    /// char col/row for a grid window, pixels for a graphics window.
    pub fn deliver_mouse(&mut self, win: u32, x: u32, y: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_mouse(win, x, y);
            let (pending, quit) = drive_settled(&mut self.machine, &self.game_dir);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }

    /// A click landed on a linked transcript cell inside a hyperlink-watching
    /// window: deliver a Glk `Evtype_Hyperlink` event carrying `link` (the link
    /// value from the cell→link map) and drive the game to its next input
    /// request. A no-op turn once the game has quit. One-shot — the gvm
    /// `deliver_hyperlink` consumes the request, so the game must re-arm.
    pub fn deliver_hyperlink(&mut self, win: u32, link: u32) -> TurnResult {
        if !self.quit {
            self.machine.deliver_hyperlink(win, link);
            let (pending, quit) = drive_settled(&mut self.machine, &self.game_dir);
            self.pending = pending;
            self.quit = quit;
        }
        self.finish_turn()
    }

    /// The bare, standard Glulx-Quetzal bytes for the game's own in-game
    /// `@save` (VM state only — no `GReg`/`Glk ` chunks; resumed via a call
    /// stub). Distinct from [`Engine::save_state`], which is the full host
    /// snapshot behind Save State (`.babelmap`).
    pub fn save_quetzal(&self) -> Vec<u8> {
        self.machine.save_quetzal()
    }
}

/// Decide whether a terminal click at absolute `(col, row)` should be diverted
/// to the game as a Glk mouse-input event, and if so compute its coordinates.
///
/// Returns `(win, val1, val2)` only when no overlay is open and the click lands
/// inside one of the mouse-watching `windows` (as reported by
/// [`GlulxSession::mouse_windows`]). `story = (x, y, w, h)` is the story-pane
/// rect: the Glk screen is sized to exactly the story pane, so a click cell maps
/// to a Glk screen cell by subtracting the pane origin. `val1`/`val2` are then
/// window-relative col/row for a grid window, or pixels (relative cells ×
/// `char_px`) for a graphics window — cell-granular in a TUI, the best a
/// terminal can offer.
pub fn glk_mouse_target(
    overlay_open: bool,
    col: u16,
    row: u16,
    story: (u16, u16, u16, u16),
    windows: &[(u32, WinType, GlkRect)],
    char_px: (u32, u32),
) -> Option<(u32, u32, u32)> {
    if overlay_open {
        return None;
    }
    let (sx0, sy0, sw, sh) = story;
    if col < sx0 || col >= sx0 + sw || row < sy0 || row >= sy0 + sh {
        return None;
    }
    let sx = (col - sx0) as u32;
    let sy = (row - sy0) as u32;
    let (win, wintype, rect) = windows
        .iter()
        .copied()
        .find(|&(_, _, r)| sx >= r.left && sx < r.left + r.width && sy >= r.top && sy < r.top + r.height)?;
    let (rel_x, rel_y) = (sx - rect.left, sy - rect.top);
    let (vx, vy) = if wintype == WinType::Graphics {
        (rel_x * char_px.0, rel_y * char_px.1)
    } else {
        (rel_x, rel_y)
    };
    Some((win, vx, vy))
}

/// Decide which hyperlink-watching window owns a click on a linked transcript
/// cell at absolute `(col, row)`, if any.
///
/// Returns the window id only when no overlay is open and the click lands inside
/// one of the hyperlink-watching `windows` (as reported by
/// [`GlulxSession::hyperlink_windows`]). Same overlay/bounds/origin logic as
/// [`glk_mouse_target`], but a hyperlink event carries the link value from the
/// cell→link map rather than coordinates, so only the window id is returned.
pub fn glk_hyperlink_window(
    overlay_open: bool,
    col: u16,
    row: u16,
    story: (u16, u16, u16, u16),
    windows: &[(u32, WinType, GlkRect)],
) -> Option<u32> {
    if overlay_open {
        return None;
    }
    let (sx0, sy0, sw, sh) = story;
    if col < sx0 || col >= sx0 + sw || row < sy0 || row >= sy0 + sh {
        return None;
    }
    let sx = (col - sx0) as u32;
    let sy = (row - sy0) as u32;
    windows
        .iter()
        .find(|&&(_, _, r)| sx >= r.left && sx < r.left + r.width && sy >= r.top && sy < r.top + r.height)
        .map(|&(win, _, _)| win)
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
            self.drive_turn();
        }
        self.finish_turn()
    }

    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult> {
        let code = key_to_glk(key)?;
        if !self.quit {
            self.machine.supply_char(code);
            self.drive_turn();
        }
        Some(self.finish_turn())
    }

    fn take_transcript(&mut self) -> String {
        self.machine.flush();
        let raw = self.appglk().take_transcript().0;
        if self.strip_prompt { strip_read_prompt(&raw).to_owned() } else { raw }
    }

    fn take_transcript_elems(&mut self) -> Vec<TranscriptElem> {
        // Ordered banner/startup drain: text runs + any inline images the game
        // drew before the first turn (title/cover art). Mirrors `finish_turn`'s
        // trailing-read-prompt handling so the returned elements stay consistent
        // with the flat `take_transcript()` string: the concatenation of the
        // returned `Text` equals `strip_read_prompt(raw)` (or `raw` unchanged
        // when `strip_prompt` is false) — same gating as `take_transcript`.
        self.machine.flush();
        let mut elems = self.appglk().take_transcript_elems();
        let mut raw = String::new();
        for e in &elems {
            if let TranscriptElem::Text { text, .. } = e {
                raw.push_str(text);
            }
        }
        let kept = if self.strip_prompt { strip_read_prompt(&raw).chars().count() } else { raw.chars().count() };
        trim_elems_to_len(&mut elems, kept);
        elems
    }

    fn set_strip_prompt(&mut self, on: bool) {
        self.strip_prompt = on;
    }

    fn pending_input(&self) -> InputKind {
        self.pending
    }

    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        // The host wrote (or failed to write) the save file; deliver the result
        // to the suspended @save and run to the next input request.
        self.machine.complete_save(wrote_ok);
        self.pending_io = None;
        self.drive_turn();
        self.finish_turn()
    }

    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        // `Some(bytes)` = the player picked a save; `None` = cancelled. Corrupt
        // bytes fall back to failure so the game sees a clean failure result.
        // Uses `complete_restore_quetzal` (stub-based, live-state-preserving),
        // matching the bare standard `.qzl` that `@save` now writes — NOT
        // `complete_restore_success`, which expects the full host-snapshot
        // format (Save State only).
        match data {
            Some(bytes) if self.machine.complete_restore_quetzal(bytes) => {}
            _ => self.machine.complete_restore_failure(),
        }
        self.pending_io = None;
        self.drive_turn();
        self.finish_turn()
    }

    fn pending_filename(&self) -> Option<FilenameReq> {
        self.pending_filename
    }

    fn file_names(&self) -> Vec<String> {
        self.machine.file_names()
    }

    fn resume_filename(&mut self, name: Option<String>) -> TurnResult {
        // `Some` = the player chose/entered a name; `None` = cancelled (NULL fileref).
        self.machine.supply_filename(name);
        self.pending_filename = None;
        self.drive_turn();
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

    fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), EngineError> {
        Err(EngineError::BadSave("Glulx has no game-save (.qzl) format".into()))
    }

    fn is_saveload_pending(&self) -> bool {
        self.machine.is_saveload_pending()
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

    fn vfs_bytes(&self) -> Vec<u8> {
        self.machine.vfs_bytes()
    }

    fn load_vfs(&mut self, bytes: &[u8]) {
        self.machine.load_vfs(bytes);
    }

    fn vfs_dirty(&self) -> bool {
        self.machine.vfs_dirty()
    }

    fn clear_vfs_dirty(&mut self) {
        self.machine.clear_vfs_dirty();
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
    fn vfs_delegation_load_and_bytes_roundtrip_through_the_machine() {
        // A trivial quit program yields a valid (settled) session; the VFS
        // methods operate on the machine, not on running game code. `machine.glk`
        // is pub(crate) to gvm, so a session-level test cannot originate a file
        // write from the app crate — the dirty-on-write path is covered at the
        // gvm layer (`vfs_dirty_tracks_mutations`, `machine_vfs_roundtrip`). Here
        // we assert the app-side delegation: `load_vfs` populates the machine VFS
        // and `vfs_bytes` re-encodes it, using gvm's public sidecar codec.
        let mut sess =
            GlulxSession::new(image_for(enc(0x120, &[]), 1), 80, 24, true, false, false, (1, 1), None, &[])
                .expect("new");
        assert!(!sess.vfs_dirty(), "a fresh session's VFS is not dirty");
        assert!(
            gvm::glk::decode_files(&sess.vfs_bytes()).is_empty(),
            "a fresh session has no persisted files"
        );

        let mut files = std::collections::BTreeMap::new();
        files.insert("scores".to_string(), b"42".to_vec());
        sess.load_vfs(&gvm::glk::encode_files(&files));
        // Loading a sidecar is not a game mutation.
        assert!(!sess.vfs_dirty(), "loading the sidecar does not dirty the VFS");

        let out = gvm::glk::decode_files(&sess.vfs_bytes());
        assert_eq!(
            out.get("scores").map(Vec::as_slice),
            Some(b"42".as_slice()),
            "the loaded file survives a vfs_bytes round-trip through the session"
        );
    }

    #[test]
    fn new_loads_vfs_sidecar_before_boot() {
        // SQ-0290: a Glulx game may read/write a Glk file during BOOT (e.g. CM's
        // init cache). GlulxSession::new must load the sidecar into the VM's VFS
        // BEFORE driving the boot, so the running game sees it and any boot-time
        // write persists. Pin that ordering guarantee: a non-empty sidecar passed
        // to new() is present in the session's VFS immediately after construction.
        let mut files = std::collections::BTreeMap::new();
        files.insert("cache".to_string(), b"init-data".to_vec());
        let sidecar = gvm::glk::encode_files(&files);

        let sess = GlulxSession::new(
            simple_line_image(), 80, 24, true, false, false, (1, 1), None, &sidecar,
        )
        .expect("new");

        let out = gvm::glk::decode_files(&sess.vfs_bytes());
        assert_eq!(
            out.get("cache").map(Vec::as_slice),
            Some(b"init-data".as_slice()),
            "the sidecar VFS entry must be loaded into the VM before boot"
        );
    }

    #[test]
    fn new_in_seeds_on_disk_saved_game_slot_before_boot() {
        use E::*;
        // SQ-0301: a create_by_name SavedGame slot's save lives in a host
        // <game_dir>/<name>.qzl file, decoupled from the VFS. The machine's
        // existence index is session-transient, so new_in must reseed it from
        // disk BEFORE booting — else a game probing glk_fileref_does_file_exist
        // during init never sees its own on-disk save across launches. Probe:
        // create_by_name("foo", SavedGame) then streamnum(does_file_exist) — the
        // banner shows "1" only if the on-disk foo.qzl was seeded.
        const NAMEADDR: u32 = 0x420; // free RAM; NAMEADDR+3 is already NUL

        let mut body = open_buffer_prelude(); // buffer window -> loc0
        for (i, &c) in b"foo".iter().enumerate() {
            body.extend(enc(0x4E, &[Imm(NAMEADDR), Imm(i as u32), Imm(c as u32)])); // astoreb
        }
        // fref = glk_fileref_create_by_name(usage=1 SavedGame, NAMEADDR, rock=0) -> loc4.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(NAMEADDR), Push])); // nameptr
        body.extend(enc(0x40, &[Imm(1), Push])); // usage (arg0, topmost)
        body.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(4)]));
        // exists = glk_fileref_does_file_exist(fref) -> loc8.
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0x67), Imm(1), LocStore(8)]));
        body.extend(enc(0x71, &[LocLoad(8)])); // streamnum -> prints "1"/"0" to the window
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit

        let dir = std::env::temp_dir().join(format!("bm-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp game_dir");
        std::fs::write(dir.join("foo.qzl"), b"pretend-save-bytes").expect("write foo.qzl");

        let mut sess = GlulxSession::new_in(
            dir.clone(), image_for(body, 3), 80, 24, true, false, false, (1, 1), None, &[],
        )
        .expect("new_in");
        assert_eq!(sess.pending_input(), InputKind::Line);
        assert_eq!(
            sess.take_transcript(),
            "1",
            "an on-disk .qzl slot is seeded before boot, so does_file_exist reports true across launches",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

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

        let mut sess = GlulxSession::new(image_for(body, 2), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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
            r.transcript_runs.iter().any(|&(_, bits, _, _, _)| bits == 0x02),
            "the Header-styled 'Y' contributes a bold run: {:?}",
            r.transcript_runs
        );
    }

    /// A line-input prompt: request_line_event(LINEBUF, 20) then glk_select.
    fn line_prompt() -> Vec<u8> {
        use E::*;
        let mut b = Vec::new();
        for v in [Imm(0), Imm(20), Imm(LINEBUF), LocLoad(0)] {
            b.extend(enc(0x40, &[v, Push]));
        }
        b.extend(enc(0x130, &[Imm(0xd0), Imm(4), Discard])); // request_line_event
        b.extend(enc(0x40, &[Imm(EVENT), Push]));
        b.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        b
    }

    #[test]
    fn ingame_save_restore_round_trips_through_the_engine() {
        use E::*;
        // RAM scratch for the @save/@restore result descriptors (mode 7 = store to
        // a 4-byte main-memory address; `MemLoad` reuses that mode for stores).
        const SAVE_RES: u32 = 0x410;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save -> mem[SAVE_RES]
        body.extend(line_prompt()); // turn 2 prompt (resume point after a restore)
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore -> mem[RESTORE_RES]
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // Turn 1: the command drives into @save, which bubbles a Save request.
        let r1 = sess.submit("save");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "@save bubbles a Save request");
        assert!(!r1.quit);

        // The run loop captures the snapshot bytes, writes them, then reports success.
        let blob = sess.save_state().bytes;
        let r2 = sess.resume_save(true);
        assert_eq!(r2.pending_io, None, "@save completes and runs on");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "reaches the turn-2 prompt");

        // Turn 2: the command drives into @restore, which bubbles a Restore request.
        let r3 = sess.submit("restore");
        assert_eq!(r3.pending_io, Some(PendingIo::Restore), "@restore bubbles a Restore request");

        // Feeding the saved bytes back restores to the save point (the turn-2
        // prompt) rather than falling through to the trailing quit.
        let r4 = sess.resume_restore(Some(&blob));
        assert!(!r4.quit, "a successful restore returns to the save point, not the quit");
        assert_eq!(r4.pending_io, None);
        assert_eq!(sess.pending_input(), InputKind::Line, "restored back to the turn-2 prompt");
    }

    /// SQ-0283 Task 6: the Glulx in-game save/restore round-trip through the
    /// bare standard `save_quetzal()` bytes (what `save_game_named_bytes` now
    /// writes), NOT the full `save_state()` host snapshot. Also checks that the
    /// live Glk window survives the restore (unlike `restore_state`, which
    /// would replace it from a `Glk ` chunk, `restore_quetzal` never touches
    /// `self.glk` — §1.8.5).
    #[test]
    fn ingame_save_restore_round_trips_via_save_quetzal_and_keeps_the_live_window() {
        use E::*;
        const SAVE_RES: u32 = 0x410;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save -> mem[SAVE_RES]
        body.extend(line_prompt()); // turn 2 prompt (resume point after a restore)
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore -> mem[RESTORE_RES]
        body.extend(enc(0x120, &[])); // quit

        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // A window is open (from open_buffer_prelude) before the save.
        assert!(!matches!(sess.screen().root, WinNode::Blank), "a window is open before @save");

        // Turn 1: the command drives into @save, which bubbles a Save request.
        let r1 = sess.submit("save");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "@save bubbles a Save request");
        assert!(!r1.quit);

        // The host would write these bytes to a .qzl file (save_game_named_bytes).
        let blob = sess.save_quetzal();
        assert!(!blob.is_empty(), "save_quetzal produces bytes");

        let r2 = sess.resume_save(true);
        assert_eq!(r2.pending_io, None, "@save completes and runs on");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "reaches the turn-2 prompt");

        // Turn 2: the command drives into @restore, which bubbles a Restore request.
        let r3 = sess.submit("restore");
        assert_eq!(r3.pending_io, Some(PendingIo::Restore), "@restore bubbles a Restore request");

        // Feeding the save_quetzal bytes back (via resume_restore ->
        // complete_restore_quetzal) restores VM state to the save point (the
        // turn-2 prompt) rather than falling through to the trailing quit.
        let r4 = sess.resume_restore(Some(&blob));
        assert!(!r4.quit, "a successful restore returns to the save point, not the quit");
        assert_eq!(r4.pending_io, None);
        assert_eq!(sess.pending_input(), InputKind::Line, "restored back to the turn-2 prompt");

        // The live Glk window survives the restore (restore_quetzal never
        // touches self.glk) -- the screen tree still has an open window, and
        // the session can keep driving turns through it.
        assert!(!matches!(sess.screen().root, WinNode::Blank), "the live window survives restore_quetzal");
    }

    #[test]
    fn game_managed_save_is_serviced_silently_to_game_dir_without_bubbling() {
        use E::*;
        // A create_by_name (game's OWN, not player-prompted) SavedGame @save must
        // be written silently to <game_dir>/<name>.qzl during the drive — no
        // Save request bubbled to the host UI. This is CM's init-cache path.
        const SAVE_RES: u32 = 0x410;
        const NAMEADDR: u32 = 0x420; // free RAM; the byte after is already NUL
        let dir = std::env::temp_dir().join(format!("bm-gameauto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut body = open_buffer_prelude();
        // Write the C-string "c" at NAMEADDR (astoreb; mem[NAMEADDR+1] is 0 already).
        body.extend(enc(0x4E, &[Imm(NAMEADDR), Imm(0), Imm(b'c' as u32)]));
        // fref = glk_fileref_create_by_name(usage=1 SavedGame, NAMEADDR, rock=0) -> local0.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(NAMEADDR), Push])); // nameptr
        body.extend(enc(0x40, &[Imm(1), Push])); // usage (arg0, topmost)
        body.extend(enc(0x130, &[Imm(0x61), Imm(3), LocStore(0)]));
        // str = glk_stream_open_file(fref, fmode=1 Write, rock=0) -> local1.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode Write
        body.extend(enc(0x40, &[LocLoad(0), Push])); // fref (arg0)
        body.extend(enc(0x130, &[Imm(0x42), Imm(3), LocStore(4)]));
        // @save str -> mem[SAVE_RES]: game-managed, serviced silently at boot.
        body.extend(enc(0x123, &[LocLoad(4), MemLoad(SAVE_RES)]));
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit

        let sess = GlulxSession::new_in(
            dir.clone(), image_for(body, 2), 80, 24, true, false, false, (1, 1), None, &[],
        )
        .expect("new");
        // No Save request reached the host: the boot drive ran straight to the prompt.
        assert_eq!(sess.pending_input(), InputKind::Line, "the game-managed @save did not bubble");
        // The fixed-name slot was written to <dir>/c.qzl.
        let bytes = std::fs::read(dir.join("c.qzl")).expect("game-managed @save wrote c.qzl silently");
        assert!(!bytes.is_empty(), "the saved slot carries the save bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingame_create_by_prompt_bubbles_filename_request_and_resume_continues() {
        use E::*;
        const FREF_RES: u32 = 0x410;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        // glk_fileref_create_by_prompt(usage=0, fmode=1 Write, rock=0) -> mem[FREF_RES].
        // @glk pops args with arg[0] topmost, so push rock, then fmode, then usage.
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode = Write
        body.extend(enc(0x40, &[Imm(0), Push])); // usage (topmost = arg[0])
        body.extend(enc(0x130, &[Imm(0x62), Imm(3), MemLoad(FREF_RES)]));
        body.extend(line_prompt()); // resume point after supply_filename
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

        // The command drives into create_by_prompt, which bubbles a filename request.
        let r1 = sess.submit("script");
        assert_eq!(sess.pending_filename(), Some(FilenameReq { usage: 0, fmode: 1 }));
        assert!(r1.pending_io.is_none(), "a filename request is not a save/restore");
        assert!(!r1.quit);

        // The host supplies a name; execution resumes to the next prompt (proving the
        // @glk stored a value and did not fault or wedge).
        let r2 = sess.resume_filename(Some("transcript".to_string()));
        assert_eq!(sess.pending_filename(), None, "request cleared after supply");
        assert!(!r2.quit);
        assert_eq!(sess.pending_input(), InputKind::Line, "resumes at the turn-2 prompt");
    }

    #[test]
    fn create_by_prompt_savedgame_auto_resolves_into_save_without_a_filename_modal() {
        use E::*;
        const FREF: u32 = 0x410;
        const SAVE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt()); // turn 1 prompt
        // create_by_prompt(usage=SavedGame(1), fmode=Write(1), rock=0) -> mem[FREF].
        body.extend(enc(0x40, &[Imm(0), Push])); // rock
        body.extend(enc(0x40, &[Imm(1), Push])); // fmode = Write
        body.extend(enc(0x40, &[Imm(1), Push])); // usage = SavedGame (topmost)
        body.extend(enc(0x130, &[Imm(0x62), Imm(3), MemLoad(FREF)]));
        body.extend(enc(0x123, &[Imm(0), MemLoad(SAVE_RES)])); // @save (host-intercepted)
        body.extend(line_prompt());
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let r1 = sess.submit("save");
        // SavedGame create_by_prompt must NOT surface a filename request; it auto-resolves
        // in-session so the turn reaches @save and bubbles a Save request in ONE turn.
        assert_eq!(sess.pending_filename(), None, "SavedGame create_by_prompt does not prompt");
        assert_eq!(r1.pending_io, Some(PendingIo::Save), "the same turn reaches @save");
    }

    #[test]
    fn ingame_restore_cancel_completes_as_failure() {
        use E::*;
        const RESTORE_RES: u32 = 0x414;
        let mut body = open_buffer_prelude();
        body.extend(line_prompt());
        body.extend(enc(0x124, &[Imm(0), MemLoad(RESTORE_RES)])); // @restore
        body.extend(enc(0x120, &[])); // quit
        let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");

        let r1 = sess.submit("restore");
        assert_eq!(r1.pending_io, Some(PendingIo::Restore));
        // Cancel (no bytes): @restore fails, execution falls through to quit.
        let r2 = sess.resume_restore(None);
        assert!(r2.quit, "a cancelled restore fails and runs to the trailing quit");
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
        let mut sess = GlulxSession::new(char_echo_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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
            GlulxSession::new(graphics_split_line_image(), 80, 24, true, true, false, (2, 2), None, &[])
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
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[])
            .expect("new");
        let r = sess.submit("go"); // drives to quit
        assert!(r.quit);
        sess.resize(40, 12); // must be a harmless no-op
        assert!(sess.has_quit());
    }

    #[test]
    fn finish_turn_drains_buffered_sound_ops() {
        use crate::session::SchannelOp;
        use gvm::glk::GlkBackend;
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, true, (1, 1), None, &[])
            .expect("new");
        {
            let g = sess.appglk();
            let c = g.schannel_create(0);
            g.schannel_play(c, 7, 1, 0);
        }
        let result = sess.submit(""); // finish_turn drains the buffered op
        assert!(
            result.glulx_sound_ops.iter().any(|op| matches!(op, SchannelOp::Play { snd: 7, .. })),
            "buffered play reached the TurnResult: {:?}",
            result.glulx_sound_ops
        );
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

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert_eq!(sess.take_transcript(), "Hi", "banner drops the trailing prompt");
        let r = sess.submit("x");
        assert_eq!(r.transcript, "done", "turn output drops the trailing prompt");
    }

    #[test]
    fn trailing_read_prompt_kept_when_strip_prompt_is_false() {
        use E::*;
        // strip_prompt gates whether the trailing "> " read prompt is removed
        // (SQ-0264: inline-prompt mode keeps it). With strip_prompt = false both
        // the banner and turn transcripts must retain the game's own ">".
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

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        sess.strip_prompt = false;
        assert_eq!(sess.take_transcript(), "Hi\n> ", "banner keeps the trailing prompt");
        let r = sess.submit("x");
        assert_eq!(r.transcript, "done\n> ", "turn output keeps the trailing prompt");
    }

    #[test]
    fn banner_take_transcript_elems_keeps_startup_image_and_strips_prompt() {
        // A game that draws a startup/cover image before the first turn: the
        // banner "Hi\n> " lands in the primary buffer, then an inline image is
        // drawn (seeded directly, as a resolvable Pict needs a Blorb the harness
        // lacks). `take_transcript_elems` must return the image in order AND strip
        // the trailing read prompt from the Text element — mirroring the
        // string-only `take_transcript` (which returns "Hi").
        use E::*;
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
        let image = image_for(body, 1);

        let mut sess = GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
        };
        sess.appglk().test_push_primary_image(dummy);

        let elems = sess.take_transcript_elems();
        assert_eq!(elems.len(), 2, "one Text element + the startup image");
        match &elems[0] {
            TranscriptElem::Text { text, .. } => {
                assert_eq!(text, "Hi", "trailing read prompt stripped from the Text element")
            }
            _ => panic!("elems[0] must be Text"),
        }
        assert!(
            matches!(&elems[1], TranscriptElem::Image(_)),
            "the startup image survives in order",
        );
    }

    #[test]
    fn save_state_is_tagged_and_round_trips_with_guard() {
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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
        let sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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

        let mut fresh = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        let _ = fresh.take_transcript();
        fresh.restore_state(&ac.engine_save()).expect("Glulx restore from archive");
        assert_eq!(fresh.pending_input(), InputKind::Line, "restored input state");
        assert_eq!(fresh.save_state().bytes, es.bytes, "restored Glulx state matches");
    }

    #[test]
    fn glulx_restore_refuses_zmachine_archive() {
        // The foreign-engine guard fires gracefully (no panic) when a zmachine
        // save is offered to a Glulx session.
        let mut sess = GlulxSession::new(simple_line_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
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

        let mut sess = GlulxSession::new(image, 78, 20, true, false, false, (1, 1), None, &[]).expect("new");

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

    // ── Mouse input (glk_request_mouse_event) ─────────────────────────────────

    /// Program: open a buffer, split a 1-row grid above, arm a mouse request AND
    /// a char request on the grid, then glk_select (suspends on the char). The
    /// grid is window 2 (buffer=1, pair=3).
    fn grid_mouse_watch_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude(); // buffer → local0 (window 1)
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // rock, wintype=grid, size=1, method=above|fixed, split
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (grid = window 2)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd4), Imm(1), Discard])); // request_mouse_event(grid)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event(grid) → select suspends
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    #[test]
    fn mouse_windows_lists_only_watching_windows_and_char_pixels_exposed() {
        let mut sess =
            GlulxSession::new(grid_mouse_watch_image(), 80, 24, true, false, false, (9, 19), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char, "suspends on the grid char request");

        // Only the grid (window 2) watches; the buffer (window 1) does not.
        let windows = sess.mouse_windows();
        assert_eq!(windows.len(), 1, "only the requesting window is listed");
        assert_eq!(windows[0].0, 2, "grid window id");
        assert_eq!(windows[0].1, WinType::TextGrid);
        assert_eq!(windows[0].2, GlkRect { left: 0, top: 0, width: 80, height: 1 }, "grid spans the top row");

        assert_eq!(sess.char_pixels(), (9, 19), "cell pixel size exposed for graphics scaling");
    }

    #[test]
    fn deliver_mouse_resumes_the_game_and_is_one_shot() {
        let mut sess =
            GlulxSession::new(grid_mouse_watch_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert!(!sess.mouse_windows().is_empty(), "armed before the click");

        // The click resumes the suspended select; the game runs to its trailing quit.
        let r = sess.deliver_mouse(2, 5, 0);
        assert!(r.quit, "the game consumed the click and ran to quit");
        assert!(sess.mouse_windows().is_empty(), "mouse request consumed (one-shot)");
    }

    // ── glk_mouse_target coordinate mapping ───────────────────────────────────

    #[test]
    fn glk_mouse_target_grid_is_identity_minus_origin() {
        // Story pane at (3, 2); a grid window filling the top row at rect(0,0,80,1).
        let windows = [(2u32, WinType::TextGrid, GlkRect { left: 0, top: 0, width: 80, height: 1 })];
        // Click at absolute (10, 2) → story cell (7, 0) → grid-relative (7, 0).
        let got = super::glk_mouse_target(false, 10, 2, (3, 2, 80, 24), &windows, (9, 19));
        assert_eq!(got, Some((2, 7, 0)), "grid reports window-relative col/row");
    }

    #[test]
    fn glk_mouse_target_graphics_scales_by_char_pixels() {
        // A graphics window offset within the pane at rect(4, 1, 20, 10).
        let windows = [(5u32, WinType::Graphics, GlkRect { left: 4, top: 1, width: 20, height: 10 })];
        // Story pane at origin (0,0); click at (6, 3) → story cell (6,3) →
        // window-relative (2, 2) → pixels (2×9, 2×19) = (18, 38).
        let got = super::glk_mouse_target(false, 6, 3, (0, 0, 80, 24), &windows, (9, 19));
        assert_eq!(got, Some((5, 18, 38)), "graphics reports pixels (rel cells × char_px)");
    }

    #[test]
    fn glk_mouse_target_declines_outside_and_under_an_overlay() {
        let windows = [(2u32, WinType::TextGrid, GlkRect { left: 0, top: 0, width: 80, height: 1 })];
        // Click below the grid (row 5) but inside the pane → misses every window.
        assert_eq!(
            super::glk_mouse_target(false, 10, 5, (0, 0, 80, 24), &windows, (1, 1)),
            None,
            "a click outside every watching window falls through",
        );
        // Click outside the story pane entirely.
        assert_eq!(
            super::glk_mouse_target(false, 90, 0, (0, 0, 80, 24), &windows, (1, 1)),
            None,
            "a click outside the story pane falls through",
        );
        // Same in-window click, but an overlay is open → declined.
        assert_eq!(
            super::glk_mouse_target(true, 10, 0, (0, 0, 80, 24), &windows, (1, 1)),
            None,
            "an open overlay keeps the click",
        );
    }

    // ── Hyperlink input (glk_request_hyperlink_event) ─────────────────────────

    /// Program: open a buffer, split a 1-row grid above, arm a hyperlink request
    /// AND a char request on the grid, then glk_select (suspends on the char).
    /// The grid is window 2 (buffer=1, pair=3). Mirrors `grid_mouse_watch_image`
    /// with `request_hyperlink_event` (0x102) in place of `request_mouse_event`.
    fn grid_hyperlink_watch_image() -> Vec<u8> {
        use E::*;
        let mut body = open_buffer_prelude(); // buffer → local0 (window 1)
        for v in [Imm(0), Imm(4), Imm(1), Imm(0x12), LocLoad(0)] {
            body.extend(enc(0x40, &[v, Push])); // rock, wintype=grid, size=1, method=above|fixed, split
        }
        body.extend(enc(0x130, &[Imm(0x23), Imm(5), LocStore(4)])); // window_open → local1 (grid = window 2)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0x102), Imm(1), Discard])); // request_hyperlink_event(grid)
        body.extend(enc(0x40, &[LocLoad(4), Push]));
        body.extend(enc(0x130, &[Imm(0xd2), Imm(1), Discard])); // request_char_event(grid) → select suspends
        body.extend(enc(0x40, &[Imm(EVENT), Push]));
        body.extend(enc(0x130, &[Imm(0xc0), Imm(1), Discard])); // glk_select
        body.extend(enc(0x120, &[])); // quit
        image_for(body, 2)
    }

    #[test]
    fn hyperlink_windows_lists_only_watching_windows() {
        let mut sess =
            GlulxSession::new(grid_hyperlink_watch_image(), 80, 24, true, false, false, (9, 19), None, &[]).expect("new");
        assert_eq!(sess.pending_input(), InputKind::Char, "suspends on the grid char request");

        // Only the grid (window 2) watches; the buffer (window 1) does not.
        let windows = sess.hyperlink_windows();
        assert_eq!(windows.len(), 1, "only the requesting window is listed");
        assert_eq!(windows[0].0, 2, "grid window id");
        assert_eq!(windows[0].1, WinType::TextGrid);
        assert_eq!(windows[0].2, GlkRect { left: 0, top: 0, width: 80, height: 1 }, "grid spans the top row");
    }

    #[test]
    fn deliver_hyperlink_resumes_the_game_and_is_one_shot() {
        let mut sess =
            GlulxSession::new(grid_hyperlink_watch_image(), 80, 24, true, false, false, (1, 1), None, &[]).expect("new");
        assert!(!sess.hyperlink_windows().is_empty(), "armed before the click");

        // The click resumes the suspended select; the game runs to its trailing quit.
        let r = sess.deliver_hyperlink(2, 42);
        assert!(r.quit, "the game consumed the link and ran to quit");
        assert!(sess.hyperlink_windows().is_empty(), "hyperlink request consumed (one-shot)");
    }

    // ── glk_hyperlink_window hit test ─────────────────────────────────────────

    #[test]
    fn glk_hyperlink_window_returns_the_owning_window_id() {
        // Story pane at (3, 2); a grid window filling the top row at rect(0,0,80,1).
        let windows = [(2u32, WinType::TextGrid, GlkRect { left: 0, top: 0, width: 80, height: 1 })];
        // Click at absolute (10, 2) → story cell (7, 0) → inside the grid.
        let got = super::glk_hyperlink_window(false, 10, 2, (3, 2, 80, 24), &windows);
        assert_eq!(got, Some(2), "returns the window whose rect contains the cell");
    }

    #[test]
    fn glk_hyperlink_window_declines_outside_overlay_and_empty() {
        let windows = [(2u32, WinType::TextGrid, GlkRect { left: 0, top: 0, width: 80, height: 1 })];
        // Click below the grid (row 5) but inside the pane → misses every window.
        assert_eq!(
            super::glk_hyperlink_window(false, 10, 5, (0, 0, 80, 24), &windows),
            None,
            "a click outside every watching window falls through",
        );
        // Click outside the story pane entirely.
        assert_eq!(
            super::glk_hyperlink_window(false, 90, 0, (0, 0, 80, 24), &windows),
            None,
            "a click outside the story pane falls through",
        );
        // Same in-window click, but an overlay is open → declined.
        assert_eq!(
            super::glk_hyperlink_window(true, 10, 0, (0, 0, 80, 24), &windows),
            None,
            "an open overlay keeps the click",
        );
        // No hyperlink-watching windows at all.
        assert_eq!(
            super::glk_hyperlink_window(false, 10, 0, (0, 0, 80, 24), &[]),
            None,
            "no watching windows → nothing to divert to",
        );
    }
}
