//! Engine abstraction (Glulx 3b-i).
//!
//! The app talks to a running game through the engine-neutral [`Engine`] trait
//! and a small family of app-owned, engine-agnostic types ([`KeyInput`],
//! [`ScreenModel`], [`Introspect`], the reserved [`Debugger`], and the
//! engine-tagged [`EngineSave`]).  `zvm`'s `GameSession` implements `Engine`
//! (see `session.rs`); a future `gvm` (Glulx) session will slot in beside it.
//!
//! These types deliberately carry **no** `Glk` / `Glulx` / `Z-machine` specifics
//! in their public surface: a `GridWindow` is a grid of style-bit cells, a
//! status line is a location plus a score/turns or clock field, an object
//! handle is an opaque `u16`.  Each engine adapts its own world into them.

use std::any::Any;
use std::collections::BTreeMap;

use crate::session::{FilenameReq, InputKind, TurnResult};

// ── Neutral key input ───────────────────────────────────────────────────────

/// A neutral, terminal-agnostic key press.
///
/// The app maps a crossterm `KeyEvent` into this with [`key_event_to_input`];
/// each engine converts it into its own input encoding (the `zvm` adapter maps
/// it to ZSCII; a Glk adapter would map it to Glk keycodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    /// Function key F1..=F12 (carries the digit, e.g. `Func(1)` for F1).
    Func(u8),
}

/// Map a crossterm `KeyEvent` to a neutral [`KeyInput`].
///
/// Returns `None` for keys with no neutral representation (media keys, modifier
/// presses, etc.).  Modifiers are not encoded here — the caller decides whether
/// a Ctrl/Alt combo is app routing or game input before forwarding.
pub fn key_event_to_input(key: crossterm::event::KeyEvent) -> Option<KeyInput> {
    use crossterm::event::KeyCode;
    Some(match key.code {
        KeyCode::Char(c) => KeyInput::Char(c),
        KeyCode::Enter => KeyInput::Enter,
        KeyCode::Backspace => KeyInput::Backspace,
        KeyCode::Tab => KeyInput::Tab,
        KeyCode::Esc => KeyInput::Escape,
        KeyCode::Up => KeyInput::Up,
        KeyCode::Down => KeyInput::Down,
        KeyCode::Left => KeyInput::Left,
        KeyCode::Right => KeyInput::Right,
        KeyCode::Home => KeyInput::Home,
        KeyCode::End => KeyInput::End,
        KeyCode::PageUp => KeyInput::PageUp,
        KeyCode::PageDown => KeyInput::PageDown,
        KeyCode::Delete => KeyInput::Delete,
        KeyCode::Insert => KeyInput::Insert,
        KeyCode::F(n) => KeyInput::Func(n),
        _ => return None,
    })
}

// ── Neutral screen model (window tree) ──────────────────────────────────────

/// One styled character cell in a [`GridWindow`].
///
/// `style` is a neutral text-style bitset following the common interactive-
/// fiction convention (bit 1 = reverse, 2 = bold, 4 = italic, 8 = fixed-pitch).
#[derive(Debug, Clone, Copy)]
pub struct GridCell {
    pub ch: char,
    pub style: u8,
    /// Packed foreground colour (see `crate::state::pack_zcolour`); 0 = Default.
    pub fg: u32,
    /// Packed background colour; 0 = Default.
    pub bg: u32,
    /// Glk hyperlink value stamped on this cell (0 = not a link). (SQ-0258)
    pub link: u32,
    /// Glk style class (0=Normal .. 10=User2) for the theme's per-style colour
    /// slot (SQ-0331). Z-machine grid cells are always Normal (0).
    pub glk_style: u8,
}

impl Default for GridCell {
    fn default() -> Self {
        GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }
    }
}

/// A text-grid window: fixed-size positioned cells with a cursor (a status line
/// or a Glk text-grid).  The renderer applies a viewport over the logical grid
/// and auto-follows the cursor.
/// A grid window's border-presence preference (SQ-0286). Only a Glulx window
/// split carries an explicit preference; the Z-machine, the default constructor,
/// and a parentless Glulx root leave it `Unspecified` so the theme decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderPref {
    /// No border preference (Z-machine, or a parentless Glulx root): the theme decides.
    #[default]
    Unspecified,
    /// A Glulx split explicitly requested a border (`winmethod_Border`). Presence forced on.
    Border,
    /// A Glulx split requested `winmethod_NoBorder`. Presence forced off.
    NoBorder,
}

#[derive(Debug, Clone, Default)]
pub struct GridWindow {
    /// Logical grid width in columns.
    pub cols: u16,
    /// Logical grid height in rows (allocation height).
    pub rows: u16,
    /// `rows * cols` cells in row-major order.
    pub cells: Vec<GridCell>,
    /// Active row count to render (e.g. the Z-machine `upper_window_rows`); may
    /// be less than `rows`.
    pub active_rows: u16,
    /// 1-based cursor (row, col).
    pub cursor: (u16, u16),
    /// True when this grid is the engine's currently selected output window
    /// (drives whether the cursor is shown while awaiting a keypress).
    pub cursor_active: bool,
    /// The game's border-presence preference (SQ-0286). `Unspecified` (the
    /// default) lets the theme decide; a Glulx split forces `Border`/`NoBorder`.
    pub border: BorderPref,
    /// This window's own Normal-style background colour (packed RGB
    /// `0x00RRGGBB`), or `None` if the game set none (the host uses its theme).
    pub bg: Option<u32>,
    /// This window's own Normal-style foreground colour (packed RGB), or `None`.
    pub fg: Option<u32>,
    /// The grid's Normal-style ReverseColor flag: when the game reversed the grid
    /// styles with no explicit colours (Counterfeit Monkey's menu), the empty-cell
    /// fill is drawn reversed too, so the whole window matches. (SQ-0403)
    pub reverse: bool,
}

impl GridWindow {
    /// Resize to `rows` × `cols`, clearing all cells.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![GridCell::default(); rows as usize * cols as usize];
    }

    /// Cell at 1-based (`row`, `col`), or a blank default when out of bounds.
    pub fn cell(&self, row: u16, col: u16) -> GridCell {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return GridCell::default();
        }
        let idx = ((row - 1) as usize) * self.cols as usize + (col - 1) as usize;
        self.cells.get(idx).copied().unwrap_or_default()
    }

    /// Write `ch`/`style` at 1-based (`row`, `col`).  Out-of-bounds is a no-op.
    pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8) {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return;
        }
        let idx = ((row - 1) as usize) * self.cols as usize + (col - 1) as usize;
        if let Some(c) = self.cells.get_mut(idx) {
            *c = GridCell { ch, style, fg: 0, bg: 0, link: 0, glk_style: 0 };
        }
    }
}

/// A text-buffer window: the scrolling, wrapped, styled lower window.
///
/// For the Z-machine (and a Glulx game's **primary** window) the app keeps its
/// own transcript buffer, so [`primary`](Self::primary) is set and `lines`/`runs`
/// stay empty — the renderer draws this window from `state.transcript` (keeping
/// search / persistence / styling). A Glulx layout's **extra** buffer windows
/// set `primary = false` and carry their inline content in `lines`/`runs`/`scroll`.
#[derive(Debug, Clone, Default)]
pub struct BufferWindow {
    /// Accumulated logical lines (split on `\n`) for an inline (non-primary)
    /// buffer window. Empty for the primary window.
    pub lines: Vec<String>,
    /// Per-line style runs, parallel to [`lines`](Self::lines).
    pub runs: Vec<Vec<crate::state::StyleRun>>,
    /// Per-line Glk paragraph layout, parallel to [`lines`](Self::lines) (SQ-0330).
    pub para: Vec<crate::state::ParaFmt>,
    /// Optional inline image parallel to `lines` (always same length). `Some`
    /// marks a line that renders as an image band instead of text.
    pub images: Vec<Option<crate::inline_image::InlineImage>>,
    /// Scrollback offset (0 = newest at bottom).
    pub scroll: u16,
    /// True when this is the primary window whose content the app mirrors into
    /// `state.transcript`; the renderer then draws it via the transcript path.
    pub primary: bool,
    /// This window's own Normal-style background colour (packed RGB
    /// `0x00RRGGBB`), or `None` if the game set none (the host uses its theme).
    pub bg: Option<u32>,
    /// This window's own Normal-style foreground colour (packed RGB), or `None`.
    pub fg: Option<u32>,
    /// True for a chrome panel (e.g. the Scott room panel) drawn with the themed
    /// `room_panel` colour instead of the transcript colour, so the top and bottom
    /// of a split read as distinct regions. A game-set `bg` still wins.
    pub panel: bool,
}

/// How a [`WinNode::Pair`] divides its space.
#[derive(Debug, Clone, Copy, Default)]
pub struct Split {
    /// Size (rows or cols, per `vertical`) given to the first child.
    pub fixed: u16,
}

/// A graphics-window leaf: a snapshot of the window's canvas for rendering.
#[derive(Debug, Clone)]
pub struct GraphicsWindow {
    pub win: u32,
    pub canvas: std::sync::Arc<image::RgbaImage>,
    pub version: u64,
    /// Scale the canvas up to fill the window (preserving aspect), rather than
    /// centering it at native size. Set for small pixel-art canvases like Scott
    /// Adams room pictures (256×96); Glulx keeps native-size centering.
    pub upscale: bool,
}

/// A node in the engine-neutral window tree.
#[derive(Debug, Clone)]
pub enum WinNode {
    /// A split of two child windows.
    Pair {
        vertical: bool,
        split: Split,
        /// The split's `winmethod_Border` hint (true = a separator between the
        /// children); rendered in T4.
        border: bool,
        /// The KEY (new) window's Normal-style background colour (packed RGB),
        /// or `None` if unset — the colour the between-siblings separator adopts.
        key_bg: Option<u32>,
        /// The KEY window's Normal-style foreground colour (packed RGB), or `None`.
        key_fg: Option<u32>,
        first: Box<WinNode>,
        second: Box<WinNode>,
    },
    /// A text-grid window.
    Grid(GridWindow),
    /// A text-buffer window.
    Buffer(BufferWindow),
    /// A pixel-canvas graphics window.
    Graphics(GraphicsWindow),
    /// An empty placeholder.
    Blank,
}

/// The right-hand field of a classic (v3-style) status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusField {
    ScoreTurns { score: i16, turns: u16 },
    Time { hours: u8, minutes: u8 },
}

/// The status the app draws above the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusModel {
    /// A classic automatic status line (location + score/turns or clock); the
    /// app renders it through its configurable status-bar layout.
    Classic { location: String, right: StatusField },
    /// The engine has no automatic status line; the app shows its own
    /// (detected room + turn counter) info instead.
    HostManaged,
}

/// The whole screen as an engine-neutral window tree plus the status the app
/// draws as chrome.
#[derive(Debug, Clone)]
pub struct ScreenModel {
    /// The window tree.  In 3b-i this is the degenerate `Pair { Grid, Buffer }`.
    pub root: WinNode,
    /// The status line the app draws above the transcript.
    pub status: StatusModel,
    /// The game's current background colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to paint the story pane.
    pub bg: u32,
    /// The game's current foreground colour, packed (see `crate::state::pack_zcolour`).
    /// `pack_zcolour(ZColour::Default)` when unset; used to colour the live input line.
    pub fg: u32,
    /// The extent (cols, rows) gvm's window tree actually covers; may be smaller
    /// than the story pane because gvm snaps proportional splits to whole cells and
    /// leaves a blank margin (SQ-0303). The generic multi-window composite clamps to
    /// this so the margin stays blank rather than stretching the last window. `(0, 0)`
    /// means unknown (the simple/Z-machine paths, which have no snap margin) → the
    /// composite falls back to the full pane.
    pub content_size: (u16, u16),
}

impl ScreenModel {
    /// Borrow the first [`GridWindow`] in the tree (the upper/status grid), if any.
    pub fn grid(&self) -> Option<&GridWindow> {
        fn find(node: &WinNode) -> Option<&GridWindow> {
            match node {
                WinNode::Grid(g) => Some(g),
                WinNode::Pair { first, second, .. } => find(first).or_else(|| find(second)),
                _ => None,
            }
        }
        find(&self.root)
    }
}

// ── Introspection capability ────────────────────────────────────────────────

/// Read-only introspection into the game world that drives the play-aids
/// (autocomplete vocabulary, inventory strip, room inspector, inventory
/// tracking).  An engine without introspection (e.g. an Inform-7 Glulx game
/// before symbol support exists) returns `None` from [`Engine::introspect`] and
/// the aids degrade gracefully.
///
/// Object handles are opaque `u16` identifiers; their meaning is engine-defined.
pub trait Introspect {
    /// The parser vocabulary (used to seed autocomplete at startup).
    fn vocabulary(&self) -> Vec<String>;
    /// The display names of the direct children of `container` (the inventory
    /// strip passes the player object here).
    fn contents(&self, container: u16) -> Vec<String>;
    /// The objects located directly in `room`, formatted for the inspector.
    fn room_objects(&self, room: u16) -> Vec<String>;
    /// The object handles whose parent is `parent` (drives inventory tracking).
    fn children_of(&self, parent: u16) -> std::collections::BTreeSet<u16>;
    /// The player object, if it can be identified.
    fn player_object(&self) -> Option<u16>;
}

// ── Debugger capability ──────────────────────────────────────────────────────

/// Read-only debug inspection of a running engine. All methods return
/// pre-formatted lines so the app render code stays engine-neutral (mirrors
/// `Engine::window_dump`). Z-machine implements this; other engines return
/// `None` from `Engine::debugger` for now. (Inspect-only; a stepper is a
/// future increment that will add `&mut` control methods.)
pub trait Debugger {
    /// Instruction pointer the VM is parked at (for "jump to PC").
    fn pc(&self) -> u32;
    /// Disassemble `lines` instructions starting at `addr`, one string per line.
    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Raw disassembly: instruction bytes + decoded structure with NO lookups
    /// (no mnemonic name, operand-role sigils, variable naming, or packed-address
    /// unpacking) — a diagnostic view to catch bugs in the translation layer.
    fn disassemble_raw(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Basic disassembly: plain mnemonic form — named mnemonics, `#hex`/named-
    /// variable operands, and computed branch targets, but NO reference-following
    /// (no operand-role sigils, packed-address unpacking, `VarRef`, or annotations).
    fn disassemble_basic(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Address of the instruction after the one at `addr` (clamped to memory);
    /// lets the panel advance the disassembly view by whole instructions.
    fn next_instr(&self, addr: u32) -> u32;
    /// Start address of the instruction before `addr` (for backward scrolling).
    fn prev_instr(&self, addr: u32) -> u32;
    /// Human-readable help lines for the instruction at `addr` (what the opcode
    /// does, its operand roles, store/branch) — for the hover tooltip. Returns
    /// `None` if the engine has no descriptions or `addr` isn't an instruction.
    fn describe_line(&self, _addr: u32) -> Option<Vec<String>> {
        None
    }
    /// The set of instruction start-PCs executed during the last command turn
    /// (empty until a turn runs with tracing on).
    fn executed_pcs(&self) -> std::collections::HashSet<u32>;
    /// Call stack, one or more lines per frame, innermost last.
    fn stack_lines(&self) -> Vec<String>;
    /// Evaluation/value stack, top first, marking frame-base boundaries.
    fn eval_stack_lines(&self) -> Vec<String>;
    /// Locals of the innermost frame.
    fn locals_lines(&self) -> Vec<String>;
    /// Global variables, formatted.
    fn globals_lines(&self) -> Vec<String>;
    /// The object tree, indented.
    fn object_tree_lines(&self) -> Vec<String>;
    /// Dictionary words.
    fn dictionary_lines(&self) -> Vec<String>;
    /// Hex+ASCII dump: `rows` rows of 16 bytes from `addr`.
    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String>;
    /// Total addressable memory length (so the panel can clamp scroll).
    fn memory_len(&self) -> u32;
    /// Detail lines for object `obj`: its set attributes, then its property
    /// table (number → hex bytes) — shown inline when the Objects tree entry
    /// is expanded.
    fn object_detail(&self, obj: u16) -> Vec<String>;
    /// Detail lines for call-stack frame `idx`: its locals (`localN = 0x…… (N)`),
    /// shown inline when the Call Stack frame entry is expanded.
    fn frame_locals(&self, idx: usize) -> Vec<String>;
    /// Current value of Z-machine variable `var` (0 = top of the eval stack,
    /// 1..=15 = locals of the innermost frame, 16..=255 = globals). `None` when
    /// unavailable (no frame, empty stack, or no such local). Read-only peek —
    /// never pops. Lets the Memory jump box dereference a variable to an address.
    fn var_value(&self, var: u8) -> Option<u16>;
}

// ── Engine-tagged save ──────────────────────────────────────────────────────

/// The location/room currency shared between the engine and the mapper.
pub type LocationInfo = zvm::ObjectSnapshot;

/// A persisted game state, tagged with the engine that produced it.
///
/// The archive records [`engine`](Self::engine) so a restore can refuse a save
/// written by a different engine.  `bytes` is the engine-defined save blob
/// (Quetzal for `zvm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineSave {
    /// The engine tag (e.g. `"zmachine"`).
    pub engine: String,
    /// The save format version within that engine.
    pub format_version: u32,
    /// The engine-defined save bytes.
    pub bytes: Vec<u8>,
}

impl EngineSave {
    /// Build a save tagged for `engine`.
    pub fn new(engine: impl Into<String>, format_version: u32, bytes: Vec<u8>) -> Self {
        EngineSave { engine: engine.into(), format_version, bytes }
    }

    /// True when this save was produced by `engine`.
    pub fn is_engine(&self, engine: &str) -> bool {
        self.engine == engine
    }
}

/// An engine operation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// A save written by a different engine was offered for restore.
    EngineMismatch { expected: String, found: String },
    /// The save bytes were rejected by the engine (corrupt / wrong story).
    BadSave(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::EngineMismatch { expected, found } => write!(
                f,
                "save engine mismatch: expected {expected}, found {found}"
            ),
            EngineError::BadSave(msg) => write!(f, "bad save: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

// ── The Engine trait ────────────────────────────────────────────────────────

/// The app-facing handle to a running game, independent of the underlying VM.
///
/// The app holds a `Box<dyn Engine>` where it once held a concrete
/// `GameSession`.  The `zvm` adapter lives in `session.rs`.
pub trait Engine {
    // ── turn cycle ──
    /// Supply a player command and run to the next input request / quit.
    fn submit(&mut self, command: &str) -> TurnResult;
    /// Supply a single keypress.  Returns `None` when the key has no input
    /// meaning for this engine (e.g. an arrow key under the Z-machine), in
    /// which case the caller leaves the turn untouched.
    fn submit_key(&mut self, key: KeyInput) -> Option<TurnResult>;
    /// Drain the transcript accumulated since the last drain.
    fn take_transcript(&mut self) -> String;
    /// Drain the accumulated transcript as ordered elements (text runs + inline
    /// images) — the element counterpart to `take_transcript`, mirroring the
    /// per-turn `TurnResult::transcript_elems`. The DEFAULT returns empty, meaning
    /// "no ordered elements; use the flat `take_transcript` string path" — the
    /// Z-machine has no inline images, so it keeps the default and drains nothing
    /// here. `GlulxSession` overrides it so banner/startup images survive.
    fn take_transcript_elems(&mut self) -> Vec<crate::session::TranscriptElem> {
        Vec::new()
    }
    /// When false, the game's own trailing `>` read prompt is preserved in the
    /// transcript (inline-prompt mode) instead of being stripped for the app's
    /// dedicated input bar. Default true.
    fn set_strip_prompt(&mut self, _on: bool) {}
    /// Which kind of input the VM is currently waiting for.
    fn pending_input(&self) -> InputKind;
    /// Resume after the host performed an in-game SAVE.
    fn resume_save(&mut self, wrote_ok: bool) -> TurnResult;
    /// Resume after the host performed an in-game RESTORE.
    fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult;
    /// The pending `create_by_prompt` filename request, if the VM suspended on one
    /// this turn. Default `None` (only the Glulx engine issues these).
    fn pending_filename(&self) -> Option<FilenameReq> {
        None
    }
    /// The user-visible VFS filenames, for a `create_by_prompt` read picker.
    /// Default empty (engines without a Glk VFS).
    fn file_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Resume after the host chose a filename (or cancelled with `None`) for a
    /// `create_by_prompt`. Only valid for engines that produce filename requests;
    /// the default panics because the run loop only calls this when
    /// [`Engine::pending_filename`] returned `Some`.
    fn resume_filename(&mut self, _name: Option<String>) -> TurnResult {
        unreachable!("resume_filename is only valid for engines that issue filename requests (Glulx)")
    }
    /// Whether the game has ended.
    fn has_quit(&self) -> bool;

    // ── screen ──
    /// The current screen as a neutral window tree + status.
    fn screen(&self) -> ScreenModel;

    /// A diagnostic dump of the live window layout, one line per entry, for the
    /// `/dump-windows` command. The default gives a one-line Z-machine summary
    /// (the grid dims + the buffer); engines with a real Glk window tree (Glulx)
    /// override this to print the full indented tree with per-window colours.
    fn window_dump(&self) -> Vec<String> {
        let model = self.screen();
        let (gc, gr) = model.grid().map(|g| (g.cols, g.active_rows)).unwrap_or((0, 0));
        vec![format!("Window layout: Grid {}x{} over Buffer (Z-machine simple path)", gc, gr)]
    }

    /// Enable/disable the `screen` trace on this engine's VM (default: no-op for
    /// engines without a Glk/screen model, e.g. Scott). (trace feature)
    fn set_trace_screen(&mut self, _on: bool) {}

    /// Drain any accumulated `screen`-trace lines (display instructions the story
    /// issued this turn). Default empty; zvm/gvm sessions override. (trace feature)
    fn take_screen_trace(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Enable/disable per-turn execution tracing for the debug inspector.
    fn set_debug_trace(&mut self, _on: bool) {}

    // ── persistence (engine-tagged) ──
    /// Capture the game state as an engine-tagged save.
    fn save_state(&self) -> EngineSave;
    /// Restore from an engine-tagged save.  Refuses a foreign-engine save.
    fn restore_state(&mut self, save: &EngineSave) -> Result<(), EngineError>;
    /// Restore a bare standard Quetzal *game* save (`.qzl`) by completing the save
    /// instruction's descriptor (v3 branch true / v4+ store 2). Z-machine only.
    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError>;
    /// Whether a game-initiated `@save`/`@restore` is currently suspended,
    /// awaiting host file I/O. Hosts must skip any unconditional host-snapshot
    /// trigger (e.g. exit auto-save) while this is true — snapshotting mid-
    /// suspension would capture an un-popped Glulx `@save` call stub, corrupting
    /// the stack on a later Save State restore. Default `false` (only Glulx's
    /// stub-based `@save` has this hazard; the Z-machine's descriptor-based
    /// `@save` does not).
    fn is_saveload_pending(&self) -> bool {
        false
    }

    // ── auxiliary persistent data (neutral byte map) ──
    /// The engine's auxiliary persistent data table.
    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>>;
    /// Replace the auxiliary persistent data table.
    fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>);
    /// Whether the auxiliary data changed since the last clear.
    fn aux_dirty(&self) -> bool;
    /// Clear the auxiliary-data dirty flag.
    fn clear_aux_dirty(&mut self);

    // ── Glk file VFS (Glulx only; default no-ops for the Z-machine) ──
    /// Encode the Glk file VFS as a disk sidecar blob (empty for engines
    /// without a Glk VFS).
    fn vfs_bytes(&self) -> Vec<u8> { Vec::new() }
    /// Replace the Glk file VFS from a disk sidecar blob (no-op if unsupported).
    fn load_vfs(&mut self, _bytes: &[u8]) {}
    /// Whether the Glk file VFS changed since the last clear.
    fn vfs_dirty(&self) -> bool { false }
    /// Clear the Glk file VFS dirty flag.
    fn clear_vfs_dirty(&mut self) {}

    // ── mapping ──
    /// The player's current location, for the mapper.
    fn current_location(&self) -> Option<LocationInfo>;

    // ── capabilities / escape hatch ──
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Introspection capability, when the engine has one.
    fn introspect(&self) -> Option<&dyn Introspect> {
        None
    }
    /// Debug-inspection capability, when the engine has one.
    fn debugger(&self) -> Option<&dyn Debugger> {
        None
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn key_event_to_input_maps_named_keys() {
        assert_eq!(key_event_to_input(key(KeyCode::Enter)), Some(KeyInput::Enter));
        assert_eq!(key_event_to_input(key(KeyCode::Backspace)), Some(KeyInput::Backspace));
        assert_eq!(key_event_to_input(key(KeyCode::Esc)), Some(KeyInput::Escape));
        assert_eq!(key_event_to_input(key(KeyCode::Up)), Some(KeyInput::Up));
        assert_eq!(key_event_to_input(key(KeyCode::Char('y'))), Some(KeyInput::Char('y')));
        assert_eq!(key_event_to_input(key(KeyCode::F(3))), Some(KeyInput::Func(3)));
        // A key with no neutral form maps to None.
        assert_eq!(key_event_to_input(key(KeyCode::CapsLock)), None);
    }

    #[test]
    fn screen_model_builds_and_finds_grid() {
        let mut grid = GridWindow::default();
        grid.resize(1, 5);
        grid.put(1, 1, 'H', 0);
        grid.put(1, 2, 'I', 2); // bold
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let g = model.grid().expect("tree has a grid");
        assert_eq!(g.cell(1, 1).ch, 'H');
        assert_eq!(g.cell(1, 2).ch, 'I');
        assert_eq!(g.cell(1, 2).style, 2);
        // Out-of-bounds is a blank default.
        assert_eq!(g.cell(9, 9).ch, ' ');
    }

    #[test]
    fn engine_save_round_trips_its_tag() {
        let save = EngineSave::new("zmachine", 1, vec![1, 2, 3]);
        assert_eq!(save.engine, "zmachine");
        assert_eq!(save.format_version, 1);
        assert_eq!(save.bytes, vec![1, 2, 3]);
        assert!(save.is_engine("zmachine"));
        assert!(!save.is_engine("glulx"));
    }

    #[test]
    fn engine_mismatch_error_displays() {
        let e = EngineError::EngineMismatch {
            expected: "zmachine".into(),
            found: "glulx".into(),
        };
        assert!(e.to_string().contains("zmachine"));
        assert!(e.to_string().contains("glulx"));
    }
}

#[cfg(test)]
mod debugger_trait_tests {
    use super::*;

    struct Dummy;
    impl Debugger for Dummy {
        fn pc(&self) -> u32 { 0x4a2f }
        fn disassemble(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f  add".into()] }
        fn disassemble_raw(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f: 54 05 03 05   2OP:0x14".into()] }
        fn disassemble_basic(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f  loadw #0abc".into()] }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn prev_instr(&self, a: u32) -> u32 { a.saturating_sub(4) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn eval_stack_lines(&self) -> Vec<String> { vec!["(empty)".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { vec!["g00=0000".into()] }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        fn memory_hex(&self, _a: u32, _r: usize) -> Vec<String> { vec!["000000  00".into()] }
        fn memory_len(&self) -> u32 { 0x10000 }
        fn object_detail(&self, _obj: u16) -> Vec<String> { vec!["attrs: (none)".into()] }
        fn frame_locals(&self, _idx: usize) -> Vec<String> { vec!["local0 = 0x0001  (1)".into()] }
        fn var_value(&self, _var: u8) -> Option<u16> { None }
    }

    #[test]
    fn debugger_object_is_usable() {
        let d = Dummy;
        let dyn_d: &dyn Debugger = &d;
        assert_eq!(dyn_d.pc(), 0x4a2f);
        assert_eq!(dyn_d.next_instr(0x4a2f), 0x4a33);
        assert!(!dyn_d.disassemble(0, 4).is_empty());
    }
}
