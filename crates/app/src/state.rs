use std::time::{Duration, Instant};

// ── Hint system state ─────────────────────────────────────────────────────────

/// The source driving the open Hints panel.
///
/// `Zcode` wraps a second Z-machine session running the companion Invisiclues
/// (or any hint `.z5`) file.  The enum is a seam for future sources (e.g. UHS).
pub enum HintSource {
    /// A companion Invisiclues / hint program run as a second Z-machine session.
    Zcode(crate::session::GameSession),
}

// GameSession does not implement Debug, so we implement Debug manually for HintSource.
impl std::fmt::Debug for HintSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintSource::Zcode(_) => write!(f, "HintSource::Zcode(<GameSession>)"),
        }
    }
}

/// Transient state for the Hints panel modal.
///
/// Held in `AppState.hints: Option<HintSession>` — `Some` while the panel is
/// open, `None` when closed.  The session is NOT persisted into the `.babelmap`
/// archive; only the per-IFID hint-file association is saved (Task A).
pub struct HintSession {
    /// The active hint source (currently always `Zcode`).
    pub source: HintSource,
    /// The hint program's own output (its scrollback transcript).
    pub transcript: Vec<String>,
    /// Scroll offset within the hint transcript.
    pub scroll: u16,
    /// The hint panel's own input line (typed by the player).
    pub input: String,
    /// Dialog title, e.g. "Invisiclues: Zork I".
    pub label: String,
    /// When true, show the suggestion "This game has its own hints — type HINT".
    pub builtin_hint: bool,
}

// GameSession does not implement Debug, so we implement Debug manually for HintSession.
impl std::fmt::Debug for HintSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintSession")
            .field("source", &self.source)
            .field("transcript", &self.transcript)
            .field("scroll", &self.scroll)
            .field("input", &self.input)
            .field("label", &self.label)
            .field("builtin_hint", &self.builtin_hint)
            .finish()
    }
}

// ── Room panel ────────────────────────────────────────────────────────────────

/// Which display mode the room panel is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPanelMode {
    /// Story/game info view (name, notes, exits, objects for current room).
    Info,
    /// Layout diagnostics view (reuses draw_inspector).
    Diagnostics,
}

/// The currently-open room info/diagnostics panel, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomPanel {
    pub id: mapper::graph::RoomId,
    pub mode: RoomPanelMode,
}

// ── Drag-pan state ────────────────────────────────────────────────────────────

/// Middle-button drag-pan accumulator state.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    /// Terminal cell position of the last drag event.
    pub last: (u16, u16),
    /// Sub-cell accumulator for x (in terminal columns).
    pub acc_x: i32,
    /// Sub-cell accumulator for y (in terminal rows).
    pub acc_y: i32,
}

// ── Verb menu state ───────────────────────────────────────────────────────────

/// Which pane is active in the verb menu modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbMenuPane {
    Verbs,
    Nouns,
    Preps,
}

/// Transient state for the verb/item token-palette modal.
/// `None` in `AppState.verb_menu` = modal closed.
#[derive(Debug, Clone)]
pub struct VerbMenuState {
    /// Which column is currently active.
    pub pane: VerbMenuPane,
    /// Selected index within the Verbs pane.
    pub verb_idx: usize,
    /// Selected index within the Nouns pane.
    pub noun_idx: usize,
    /// Selected index within the Preps pane.
    pub prep_idx: usize,
    /// Noun list built from room words ∪ inventory at menu-open time.
    pub nouns: Vec<String>,
}

impl VerbMenuState {
    /// Return the token that is currently selected (token to append on Pick).
    pub fn selected_token<'a>(&'a self, verbs: &'a [&'static str], preps: &'a [&'static str]) -> &'a str {
        match self.pane {
            VerbMenuPane::Verbs => verbs.get(self.verb_idx).copied().unwrap_or(""),
            VerbMenuPane::Nouns => self.nouns.get(self.noun_idx).map(|s| s.as_str()).unwrap_or(""),
            VerbMenuPane::Preps => preps.get(self.prep_idx).copied().unwrap_or(""),
        }
    }
}

// ── Gallery constants ─────────────────────────────────────────────────────────

/// Category index for box-style gallery column.
pub const GALLERY_CATEGORY_BOX: usize = 0;
/// Category index for arrow-set gallery column.
pub const GALLERY_CATEGORY_ARROWS: usize = 1;
/// Category index for portal-icons gallery column.
pub const GALLERY_CATEGORY_PORTAL: usize = 2;
/// Category index for path-style gallery column.
pub const GALLERY_CATEGORY_PATH: usize = 3;

/// Category names displayed in the gallery left pane.
pub const GALLERY_CATEGORY_NAMES: &[&str] = &["Box style", "Arrows", "Portals", "Path"];

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;

// ── Transcript kind ───────────────────────────────────────────────────────────

/// Category tag for each transcript entry.
///
/// `Story` = game output. `Input` = the player's echoed command. `Meta` =
/// app/slash output. `Warning` = VM diagnostics. The `/filter` view is coarse
/// (story = Story+Input, meta = Meta+Warning); the styling is per-variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptKind {
    Story,
    Input,
    Meta,
    Warning,
}

// ── Transcript filter ─────────────────────────────────────────────────────────

/// Which categories of transcript entries are currently visible.
///
/// `Both` (the default) shows all entries. `Story` shows only game output.
/// `Meta` shows only app-generated output (slash commands, /help, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TranscriptFilter {
    #[default]
    Both,
    Story,
    Meta,
}

// ── Tidy animation ────────────────────────────────────────────────────────────

/// One captured stage of the tidy pipeline, held for playback. `graph` is a clone
/// of the layout as it stood after the named stage ran.
#[derive(Debug, Clone)]
pub struct TidyFrame {
    pub label: String,
    pub graph: MapGraph,
    pub description: String,
    pub stats: mapper::layout::TidyStats,
    pub stage_start: bool,
}

/// Transient playback state for the tidy animation. While this is `Some`, the map
/// pane renders the current frame's graph instead of the live one. Playback holds
/// on the final frame; `Esc` clears it back to the live map.
#[derive(Debug)]
pub struct TidyAnim {
    pub frames: Vec<TidyFrame>,
    pub idx: usize,
    pub playing: bool,
    last_advance: Instant,
}

impl TidyAnim {
    pub fn new(frames: Vec<TidyFrame>) -> Self {
        Self { frames, idx: 0, playing: true, last_advance: Instant::now() }
    }

    pub fn current(&self) -> &TidyFrame {
        &self.frames[self.idx]
    }

    fn at_end(&self) -> bool {
        self.idx + 1 >= self.frames.len()
    }

    /// Step `delta` frames (clamped to range) and pause — manual control overrides playback.
    pub fn step(&mut self, delta: isize) {
        let last = self.frames.len().saturating_sub(1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle play/pause; resuming restarts the dwell clock so the current frame holds full time.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one frame if playing and `dwell` has elapsed since the last advance. Stops (holds)
    /// at the final frame. Returns true if the frame index changed.
    pub fn tick(&mut self, dwell: Duration) -> bool {
        if !self.playing || self.at_end() {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.at_end() {
            self.playing = false;
        }
        true
    }
}

// ── Replay / rewind ───────────────────────────────────────────────────────────

/// Transient state for the rewind/replay modal. While `Some`, the map pane
/// renders the reconstructed snapshot for `idx` instead of the live graph
/// (like `TidyAnim`). `Esc`/`q` clears it back to the live game with no change.
#[derive(Debug)]
pub struct ReplayState {
    /// Selected turn index into `AppState.history`.
    pub idx: usize,
    pub playing: bool,
    last_advance: Instant,
}

impl ReplayState {
    /// Open seeded at the last turn (`last_idx`), paused.
    pub fn new(last_idx: usize) -> Self {
        Self { idx: last_idx, playing: false, last_advance: Instant::now() }
    }

    /// Step `delta` turns (clamped to `[0, len-1]`) and pause.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 { self.idx = 0; self.playing = false; return; }
        let last = (len - 1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle auto-play; resuming restarts the dwell clock.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one turn if playing and `dwell` elapsed; holds at the last turn.
    /// Returns true if `idx` changed.
    pub fn tick(&mut self, dwell: Duration, len: usize) -> bool {
        if !self.playing || len == 0 || self.idx + 1 >= len {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.idx + 1 >= len {
            self.playing = false;
        }
        true
    }
}

// ── Sound pulse ──────────────────────────────────────────────────────────────

/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
#[derive(Debug)]
pub struct SoundPulse {
    pub kind: zvm::cpu::exec::Beep,
    pub started: std::time::Instant,
}

// ── Background tidy job ───────────────────────────────────────────────────────

/// An in-flight background tidy job. The worker thread runs the relayout on a
/// clone of the graph and returns the tidied clone. The run loop polls
/// `handle.is_finished()` each iteration and joins when done.
pub struct TidyJob {
    /// Worker thread handle. Returns the tidied graph clone on success.
    pub handle: std::thread::JoinHandle<mapper::graph::MapGraph>,
    /// The layer being tidied.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn time. Used to detect stale results.
    pub gen: u64,
    /// Instant the job was spawned. Used to compute the pulse phase for the border color.
    pub started: std::time::Instant,
}

impl std::fmt::Debug for TidyJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TidyJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

// ── Saves manager state ───────────────────────────────────────────────────────

/// Transient state for the saves-manager modal.
/// `None` in `AppState.saves` = modal closed.
#[derive(Debug, Clone)]
pub struct SavesState {
    /// All discovered save files for the current story (default first, then named).
    pub entries: Vec<crate::persist_files::SaveInfo>,
    /// Index of the currently-highlighted row.
    pub selected: usize,
}

// ── Gallery state ─────────────────────────────────────────────────────────────

/// Transient state for the symbol gallery modal.
/// `None` in AppState.gallery = closed.
#[derive(Debug, Clone)]
pub struct GalleryState {
    /// Which category column is currently active (0..4).
    pub category_idx: usize,
    /// Selected preset index within each category (indices into preset_names()).
    /// Order: [box, arrows, portal, path]
    pub selections: [usize; 4],
}

impl GalleryState {
    /// Build a SymbolConfig from the current gallery selections.
    pub fn symbol_config(&self) -> crate::config::SymbolConfig {
        use crate::symbols::{Arrows, BoxStyle, PathGlyphs, PortalGlyphs};
        crate::config::SymbolConfig {
            box_style: BoxStyle::preset_names()[self.selections[GALLERY_CATEGORY_BOX]].to_owned(),
            arrow_set: Arrows::preset_names()[self.selections[GALLERY_CATEGORY_ARROWS]].to_owned(),
            portal_icons: PortalGlyphs::preset_names()[self.selections[GALLERY_CATEGORY_PORTAL]].to_owned(),
            path_style: PathGlyphs::preset_names()[self.selections[GALLERY_CATEGORY_PATH]].to_owned(),
            overrides: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Game,
    Map,
}

// ── Prompt sub-mode ───────────────────────────────────────────────────────────

/// Which path field is being edited in the config screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPathField {
    UserDir,
}

/// What triggered the prompt, carrying the target room (and edge direction where
/// applicable).  Used by `apply_action` to know which mapper method to call on
/// Enter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    RenameRoom(RoomId),
    EditNotes(RoomId),
    /// Relabel the edge that exits `RoomId` in the given direction.
    RelabelEdge(RoomId, Direction),
    /// Rename the layer with the given id.
    RenameLayer(LayerId),
    /// Enter a name for a new named save slot (saves-manager sub-mode).
    SaveAs,
    /// Confirm deletion of the named save at this path.
    ConfirmDeleteSave(std::path::PathBuf),
    /// Enter a filename for an exported Quetzal save in the given directory.
    ExportSaveName(std::path::PathBuf),
    /// Edit a config path field (user_dir or colors.scheme) from the config screen.
    ConfigEditPath { field: ConfigPathField },
}

// ── File browser state ────────────────────────────────────────────────────────

/// Mode for the file browser: picking a file to import, or a directory to export into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbMode {
    /// Import: browse and pick a `.qzl`/`.sav` file.
    PickFile,
    /// Export: browse and pick a directory, then enter a filename.
    PickDir,
}

/// One entry in the file browser listing.
#[derive(Debug, Clone)]
pub struct FbEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Transient state for the file-browser modal.
/// `None` in `AppState.file_browser` = modal closed.
#[derive(Debug, Clone)]
pub struct FileBrowserState {
    /// Current working directory shown by the browser.
    pub cwd: std::path::PathBuf,
    /// Sorted entries: `..` (if not root), then dirs, then matching files.
    pub entries: Vec<FbEntry>,
    /// Index of the currently-highlighted row.
    pub selected: usize,
    /// Whether we are picking a file (import) or a directory (export).
    pub mode: FbMode,
    /// Default filename for the export prompt: `<ifid>.qzl`.
    pub export_default_name: String,
}

impl FileBrowserState {
    /// Build a new `FileBrowserState` for `cwd`, reading the filesystem.
    /// Entries: `..` when not at root, then dirs sorted, then `.qzl`/`.sav` files sorted
    /// (PickFile only).  Entries that fail to read are silently omitted.
    pub fn build(cwd: std::path::PathBuf, mode: FbMode, export_default_name: String) -> Self {
        let entries = Self::read_entries(&cwd, mode);
        FileBrowserState { cwd, entries, selected: 0, mode, export_default_name }
    }

    /// (Re)build entries for the current `cwd` and `mode`.
    pub fn refresh(&mut self) {
        self.entries = Self::read_entries(&self.cwd, self.mode);
        self.selected = 0;
    }

    /// Navigate into a subdirectory or parent.
    pub fn cd(&mut self, dir: std::path::PathBuf) {
        self.cwd = dir;
        self.refresh();
    }

    fn read_entries(cwd: &std::path::Path, mode: FbMode) -> Vec<FbEntry> {
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();

        if let Ok(iter) = std::fs::read_dir(cwd) {
            for entry in iter.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                // Skip hidden files (starting with '.') except we add '..' explicitly.
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(name.to_owned());
                } else if mode == FbMode::PickFile {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".qzl") || lower.ends_with(".sav") {
                        files.push(name.to_owned());
                    }
                }
            }
        }

        dirs.sort_unstable();
        files.sort_unstable();

        let mut entries: Vec<FbEntry> = Vec::new();
        // Prepend ".." if not at root.
        if cwd.parent().is_some() {
            entries.push(FbEntry { name: "..".to_owned(), is_dir: true });
        }
        for d in dirs {
            entries.push(FbEntry { name: d, is_dir: true });
        }
        for f in files {
            entries.push(FbEntry { name: f, is_dir: false });
        }
        entries
    }
}

// ── Config screen state ───────────────────────────────────────────────────────

/// Transient state for the config-screen modal.
/// `None` in `AppState.config_screen` = modal closed.
#[derive(Debug, Clone)]
pub struct ConfigScreenState {
    /// A working copy of the config, edited in the modal.
    /// On Save this is copied to `state.config`; on Cancel it is dropped.
    pub working: crate::config::Config,
    /// Index of the currently-selected row.
    pub selected: usize,
}

/// A small text-entry sub-mode overlaid on map focus.  While `AppState::prompt`
/// is `Some`, key events are routed to the prompt buffer rather than to the
/// normal map bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub buffer: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Split,
    TranscriptFull,
    MapFull,
}

/// Zoom levels for the map pane. `Boxes` is the closest/most-detailed view;
/// `Overview` is the most zoomed-out view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    Boxes,
    Compact,
    Overview,
}

/// Map a fine zoom level (0–8) to the three-variant `Zoom` enum used for rendering.
///
/// Fine levels 0–2 → Overview, 3–5 → Compact, 6–8 → Boxes.
/// The fine level slows down zoom transitions so the middle (Compact) level is
/// reachable without accidentally skipping it when scrolling quickly.
pub(crate) fn zoom_from_level(level: u8) -> Zoom {
    match level {
        0..=2 => Zoom::Overview,
        3..=5 => Zoom::Compact,
        _ => Zoom::Boxes,
    }
}

impl Zoom {
    /// Returns (step_w, step_h): the terminal cell stride per map-grid cell.
    ///
    /// The stride is larger than the box size (see `zoom_box_size`), and the
    /// difference is gutter where connectors route. The Boxes-zoom box is 11×5
    /// (a ~2:1 width:height ratio so it looks square given the ~1:2 terminal cell
    /// aspect; both odd so side anchors land on the exact box centre). The stride
    /// adds an 8-col / 6-row gutter for the direction-aware router's clearance and
    /// perpendicular-crossing lanes.
    pub fn steps(self) -> (i32, i32) {
        match self {
            Zoom::Boxes => (19, 11),
            Zoom::Compact => (12, 5),
            Zoom::Overview => (2, 2),
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub focus: Focus,
    pub layout: Layout,
    pub zoom: Zoom,
    /// Fine zoom level (0–8): 0–2 = Overview, 3–5 = Compact, 6–8 = Boxes.
    /// Derived from this level; `zoom_in`/`zoom_out`/`zoom_reset` update both.
    pub zoom_level: u8,
    /// Map scroll offset in grid cells: (x, y).
    pub scroll: (i32, i32),
    pub selected_room: Option<RoomId>,
    pub transcript: Vec<String>,
    /// Parallel kind tag for each entry in `transcript` (always same length).
    pub transcript_kinds: Vec<TranscriptKind>,
    /// Which categories of transcript entries are currently visible.
    pub transcript_filter: TranscriptFilter,
    pub transcript_scroll: u16,
    pub input: String,
    // Reserved for future status-bar messages (not yet displayed).
    #[allow(dead_code)]
    pub status: String,
    /// Transient status message shown on the status line (cleared on next keypress/turn).
    pub status_msg: Option<String>,
    /// Active text-entry prompt, if any.  While set, key events are routed to
    /// the prompt buffer instead of the normal map or game bindings.
    pub prompt: Option<Prompt>,
    /// When true, draw each chained room's alignment code (`R{id}` / `C{id}`) in
    /// its box interior (Boxes zoom only).  Toggled by `Ctrl+A`.
    pub show_alignment: bool,
    /// When true, portal icons additionally show their destination room name (Boxes zoom only).
    /// Toggled by `Ctrl+P`.
    pub show_portal_labels: bool,
    /// Active tidy-animation playback, if any. While `Some`, the map renders the current
    /// captured stage instead of the live graph. Started by `Ctrl+Y`, cleared by `Esc`.
    pub tidy_anim: Option<TidyAnim>,
    /// In-flight background tidy job, if any. The worker runs the relayout on a clone
    /// of the graph and returns the tidied clone. Driven by the run loop (spawn, poll, apply).
    pub tidy_job: Option<TidyJob>,
    /// In-flight one-shot story-border flash, if any. Armed by a beep event; expires after SOUND_PULSE_MS.
    pub sound_pulse: Option<SoundPulse>,
    /// Monotonically increasing generation counter. Bumped each time the real graph is mutated
    /// by an applied turn. Used to detect stale tidy results (job's gen vs current gen).
    pub graph_gen: u64,
    /// Explicit layer override for the map view. `None` means follow the current room's layer.
    pub viewed_layer: Option<LayerId>,
    /// When true, draw the per-room diagnostics inspector overlay over the map pane.
    /// Toggled by the `i` key in map focus.
    /// Deprecated: use `room_panel` for new code. Kept for keyboard-toggle compat.
    pub show_inspector: bool,
    /// Currently-open room panel (info or diagnostics), if any.
    /// Set by mouse clicks and the keyboard inspector toggle; drives `draw_frame`.
    pub room_panel: Option<RoomPanel>,
    /// Middle-button drag-pan state. `Some` while a drag gesture is in progress.
    pub drag: Option<DragState>,
    /// Story-pane text selection (left-drag). `Some` while selecting; the
    /// highlight is shown during the drag and copied on release.
    pub selection: Option<crate::clipboard::Selection>,
    /// Sub-character pan offset in terminal columns/rows, applied on top of `scroll`.
    /// Allows 1-character precision drag panning without changing the cell-unit scroll.
    /// Cleared by `recenter_on`.
    pub char_pan: (i32, i32),
    /// When true, show the hotkey dialog overlay. Opened by the prefix key (Ctrl+K),
    /// closed by the prefix key again or 'q'.
    pub hotkey_dialog: bool,

    /// Resolved glyph set for the map renderer.  Defaults to today's hardcoded glyphs;
    /// overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub symbols: crate::symbols::SymbolSet,

    /// Resolved color scheme.  Defaults to `ColorScheme::terminal_default()` (today's exact
    /// ANSI colors); overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub colors: crate::colors::ColorScheme,

    /// Resolved keymap.  Defaults to `KeyMap::default()` (today's hardcoded bindings);
    /// overwritten at startup via `KeyMap::resolve(&cfg.keymap)` when a config is present.
    pub keymap: crate::keymap::KeyMap,

    /// Hotkey layout: prefix key, direct command set, dialog groups.
    /// Defaults to the built-in layout; overwritten at startup from config.
    pub hotkeys: crate::keymap::HotkeyLayout,

    /// Active symbol gallery modal state. `None` means the gallery is closed.
    pub gallery: Option<GalleryState>,

    /// Active saves-manager modal state. `None` means the modal is closed.
    pub saves: Option<SavesState>,

    /// Set while a game-initiated (v4+) `@save`/`@restore` is awaiting the host's
    /// file I/O. The saves dialog runs in "in-game" mode: its confirm/cancel call
    /// `session.resume_save`/`resume_restore` instead of the Ctrl+S/Ctrl+R path.
    pub ingame_io: Option<crate::session::PendingIo>,

    /// Flag-hop: set by `handle_saves_prompt` after a successful in-game SAVE so
    /// the run loop (where `session`/`mapper`/`last_panes` are in scope) performs
    /// the VM resume + recenter. `Some(true)` = file written. Cleared on resume.
    pub ingame_resume_save: Option<bool>,

    /// Active file-browser modal state. `None` means the browser is closed.
    pub file_browser: Option<FileBrowserState>,

    /// Active verb/item token-palette modal state. `None` means the modal is closed.
    pub verb_menu: Option<VerbMenuState>,

    /// The resolved runtime config. Set at startup; updated on config-screen Save.
    pub config: crate::config::Config,

    /// Active config-screen modal state. `None` means the screen is closed.
    pub config_screen: Option<ConfigScreenState>,

    /// Session turn counter; incremented on each non-empty `SubmitCommand`.
    /// Written into `Meta` on every save (quick-save and named).
    pub turns: u32,

    /// Per-turn rewind/replay history. Filled when `config.record_turn_history`
    /// is on; persisted into the `.babelmap` archive. Empty otherwise.
    pub history: Vec<crate::history::TurnRecord>,

    /// Active rewind/replay modal state. `None` means the modal is closed.
    pub replay: Option<ReplayState>,

    /// Set by apply_action when a saves-manager prompt (SaveAs or ConfirmDeleteSave)
    /// is submitted. The caller (main.rs) reads this to perform the I/O operation,
    /// then clears it. The tuple is (kind, user_input_buffer).
    pub saves_prompt_submitted: Option<(PromptKind, String)>,

    // ── Autocomplete state ────────────────────────────────────────────────────

    /// Cached parser-vocabulary words from the Z-machine dictionary.
    /// Populated once by the run loop after session creation via
    /// `zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem)`.
    /// If empty, autocomplete draws only from room-description words.
    pub dict_words: Vec<String>,
    /// Current list of completion candidates, recomputed whenever `input` changes
    /// while in Game focus. Empty means no suggestions are shown.
    pub suggestions: Vec<String>,
    /// Index into `suggestions` of the currently-highlighted candidate.
    /// `Tab` advances this (cycling); typing resets it to 0.
    pub suggestion_idx: usize,

    // ── Adventure title ───────────────────────────────────────────────────────

    /// Resolved adventure title (override > banner > filename stem).
    /// Set once at startup; used by pane chrome to label the story pane.
    pub title: String,

    /// The current story's IFID (set at session creation). Keys the per-game
    /// style override (`user_dir/styles/<ifid>.toml`). Empty until set.
    pub ifid: String,

    // ── Inventory panel state ─────────────────────────────────────────────────

    /// When true, the inventory strip is shown above the input line.
    pub show_inventory: bool,
    /// Locked player object number once detected by the heuristic. None until
    /// the player moves between two rooms and exactly one object follows.
    pub player_obj: Option<u16>,
    /// Last parsed output from an inventory command (parse fallback when player_obj
    /// is not yet locked).
    pub inventory_fallback: Vec<String>,
    /// The player's previous room (global 0 value from the previous turn).
    pub prev_location: Option<u16>,
    /// Objects whose parent was prev_location at the end of the previous turn.
    pub prev_objects_here: std::collections::BTreeSet<u16>,

    // ── Reset dialog state ────────────────────────────────────────────────────

    /// When true, the reset-confirmation dialog is open.
    pub reset_dialog: bool,
    /// When true, the "Also clear the map" checkbox is checked in the reset dialog.
    pub reset_clear_map: bool,

    // ── Quit dialog state ─────────────────────────────────────────────────────

    /// When true, the "Save before quitting?" confirmation dialog is open.
    pub quit_dialog: bool,

    // ── Launch dialog state ───────────────────────────────────────────────────

    /// When true, the "Resume saved game?" dialog is shown at startup.
    pub launch_dialog: bool,
    /// Stashed restore data shown while the launch dialog is open.
    /// Tuple is (save bytes, transcript lines, transcript kinds).
    pub pending_resume: Option<(Vec<u8>, Vec<String>, Vec<TranscriptKind>, Option<zvm::screen::ScreenState>)>,
    /// When true, room numbers (#id) are shown in Boxes-zoom room boxes.
    pub show_room_numbers: bool,
    /// How the current room was detected (for the map indicator). Retained
    /// across turns; updated when a turn reports a method.
    pub loc_method: Option<zvm::location::LocationMethod>,
    /// The current room's display name (from `TurnResult.location`), retained
    /// across turns. Drives the built-in `transcript:location` story rule.
    pub current_room_name: Option<String>,
    /// Whether the detection-method indicator is shown. Default false.
    pub show_loc_method: bool,
    /// Whether the status/score bar (top row of the story pane) is shown.
    /// Default true; toggled by ToggleStatusBar. Hidden, the row collapses into
    /// the transcript but still pops up briefly for a transient status message.
    pub show_status_bar: bool,

    // ── Hints panel state ─────────────────────────────────────────────────────

    /// Active Hints panel session. `None` means the panel is closed.
    pub hints: Option<HintSession>,

    // ── Search state ──────────────────────────────────────────────────────────

    /// The active search query, if any. `None` means no search is active.
    pub search_query: Option<String>,
    /// Positions (0-based) within the visible-index list of lines that match the query.
    pub search_matches: Vec<usize>,
    /// Index into `search_matches` of the current match.
    pub search_idx: usize,

    // ── Dialog focus state ────────────────────────────────────────────────────

    /// Index of the currently focused button in an open modal dialog. Reset to
    /// a button index when a modal opens; cycled by Tab/Shift-Tab.
    pub dialog_focus: usize,

    // ── Char-input mode ───────────────────────────────────────────────────────

    /// True when the Z-machine is awaiting a single keypress (`read_char`).
    /// Set each frame by the run loop from `session.pending_input()`.
    /// Used by the renderer to hide the bottom input prompt.
    pub char_mode: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            focus: Focus::Game,
            layout: Layout::Split,
            zoom: Zoom::Boxes,
            zoom_level: 7, // default = Boxes (level 7)
            scroll: (0, 0),
            selected_room: None,
            transcript: Vec::new(),
            transcript_kinds: Vec::new(),
            transcript_filter: TranscriptFilter::Both,
            transcript_scroll: 0,
            input: String::new(),
            status: String::new(),
            status_msg: None,
            prompt: None,
            show_alignment: false,
            show_portal_labels: false,
            tidy_anim: None,
            tidy_job: None,
            sound_pulse: None,
            graph_gen: 0,
            viewed_layer: None,
            show_inspector: false,
            room_panel: None,
            drag: None,
            selection: None,
            char_pan: (0, 0),
            hotkey_dialog: false,
            symbols: crate::symbols::SymbolSet::default(),
            colors: crate::colors::ColorScheme::terminal_default(),
            keymap: crate::keymap::KeyMap::default(),
            hotkeys: crate::keymap::HotkeyLayout::default(),
            gallery: None,
            saves: None,
            ingame_io: None,
            ingame_resume_save: None,
            file_browser: None,
            verb_menu: None,
            config: crate::config::Config::default(),
            config_screen: None,
            turns: 0,
            history: Vec::new(),
            replay: None,
            saves_prompt_submitted: None,
            dict_words: Vec::new(),
            suggestions: Vec::new(),
            suggestion_idx: 0,
            title: String::new(),
            ifid: String::new(),
            show_inventory: false,
            player_obj: None,
            inventory_fallback: Vec::new(),
            prev_location: None,
            prev_objects_here: std::collections::BTreeSet::new(),
            reset_dialog: false,
            reset_clear_map: false,
            quit_dialog: false,
            launch_dialog: false,
            pending_resume: None,
            show_room_numbers: false,
            loc_method: None,
            current_room_name: None,
            show_loc_method: false,
            show_status_bar: true,
            hints: None,
            search_query: None,
            search_matches: Vec::new(),
            search_idx: 0,
            dialog_focus: 0,
            char_mode: false,
        }
    }
}

impl AppState {
    /// Return true if any modal, dialog, or overlay is currently open.
    ///
    /// Used to suppress the story input cursor while an overlay is covering the pane.
    pub fn any_overlay_open(&self) -> bool {
        self.gallery.is_some()
            || self.saves.is_some()
            || self.file_browser.is_some()
            || self.config_screen.is_some()
            || self.verb_menu.is_some()
            || self.hotkey_dialog
            || self.room_panel.is_some()
            || self.tidy_anim.is_some()
            || self.prompt.is_some()
            || self.reset_dialog
            || self.quit_dialog
            || self.launch_dialog
            || self.hints.is_some()
            || self.replay.is_some()
    }

    /// Set the explicit layer override. `None` means follow the current room's layer.
    pub fn set_viewed_layer(&mut self, layer: Option<LayerId>) {
        self.viewed_layer = layer;
    }

    /// Return the layer to render the map with.
    /// Priority: `viewed_layer` (if set and still present), else the current room's layer, else `MAIN_LAYER`.
    pub fn active_layer(&self, graph: &MapGraph) -> LayerId {
        use mapper::layer::MAIN_LAYER;
        if let Some(l) = self.viewed_layer {
            if graph.layers().contains_key(&l) {
                return l;
            }
        }
        graph.current().map(|id| graph.layer_of(id)).unwrap_or(MAIN_LAYER)
    }

    /// Toggle focus between Game and Map panes.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Game => Focus::Map,
            Focus::Map => Focus::Game,
        };
    }

    /// Cycle layout: Split → TranscriptFull → MapFull → Split.
    pub fn cycle_layout(&mut self) {
        self.layout = match self.layout {
            Layout::Split => Layout::TranscriptFull,
            Layout::TranscriptFull => Layout::MapFull,
            Layout::MapFull => Layout::Split,
        };
    }

    /// Cycle layout in reverse: Split → MapFull → TranscriptFull → Split.
    pub fn cycle_layout_reverse(&mut self) {
        self.layout = match self.layout {
            Layout::Split => Layout::MapFull,
            Layout::MapFull => Layout::TranscriptFull,
            Layout::TranscriptFull => Layout::Split,
        };
    }

    /// Zoom in one fine step (toward Boxes). Clamps at level 8 (Boxes).
    pub fn zoom_in(&mut self) {
        self.zoom_level = self.zoom_level.saturating_add(1).min(8);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Zoom out one fine step (toward Overview). Clamps at level 0 (Overview).
    pub fn zoom_out(&mut self) {
        self.zoom_level = self.zoom_level.saturating_sub(1);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Reset zoom to the default level (7 = Boxes) and clear char_pan.
    pub fn zoom_reset(&mut self) {
        self.zoom_level = 7;
        self.zoom = Zoom::Boxes;
        self.char_pan = (0, 0);
    }

    /// Pan the map scroll by (dx, dy).
    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.scroll = (self.scroll.0 + dx, self.scroll.1 + dy);
    }

    /// Set scroll so that `cell` is centered in a pane of size `pane_w` × `pane_h`.
    ///
    /// `pane_w` and `pane_h` are in terminal characters; this method converts
    /// them to map-grid cells using the current zoom step before centering,
    /// so that `scroll` stays in cell units (matching `cell_to_screen`).
    ///
    /// For Boxes zoom the non-uniform layout places rooms at roughly
    /// `(BOX_W + MIN_GUTTER)` × `(BOX_H + MIN_GUTTER)` pixels per cell, which
    /// is smaller than `zoom.steps()` (19×11). Using the actual cell footprint
    /// keeps the target room near the pane centre rather than at the top edge.
    pub fn recenter_on(&mut self, cell: (i32, i32), pane_w: u16, pane_h: u16) {
        use crate::render::map::{BOX_W, BOX_H, MIN_GUTTER};
        let (sw, sh) = match self.zoom {
            Zoom::Boxes => (BOX_W + MIN_GUTTER, BOX_H + MIN_GUTTER), // 13 × 7
            _ => self.zoom.steps(),
        };
        let cells_w = (pane_w as i32 / sw).max(1);
        let cells_h = (pane_h as i32 / sh).max(1);
        self.scroll = (cell.0 - cells_w / 2, cell.1 - cells_h / 2);
        // Reset char-granular pan offset when re-centering the view.
        self.char_pan = (0, 0);
    }

    /// Return the indices (into `self.transcript`) of entries that pass the active
    /// `transcript_filter`, in order. `Both` returns all indices; `Story`/`Meta`
    /// return only indices whose kind matches. Defensively tolerates any length
    /// mismatch between `transcript` and `transcript_kinds` by defaulting to `Story`.
    pub fn visible_transcript_indices(&self) -> Vec<usize> {
        (0..self.transcript.len())
            .filter(|&i| {
                let kind = self.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
                match self.transcript_filter {
                    TranscriptFilter::Both => true,
                    TranscriptFilter::Story => matches!(kind, TranscriptKind::Story | TranscriptKind::Input),
                    TranscriptFilter::Meta => matches!(kind, TranscriptKind::Meta | TranscriptKind::Warning),
                }
            })
            .collect()
    }

    /// Split `text` on `'\n'` and append each line to the transcript, tagged as `Story`.
    pub fn push_transcript(&mut self, text: &str) {
        self.push_transcript_kind(text, TranscriptKind::Story);
    }

    /// Split `text` on `'\n'` and append each line to the transcript with the given kind tag.
    pub fn push_transcript_kind(&mut self, text: &str, kind: TranscriptKind) {
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
        }
    }

    /// Set the transient status message (displayed on the status line until cleared).
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
    }

    /// Set the selected room.
    pub fn select_room(&mut self, room: Option<RoomId>) {
        self.selected_room = room;
    }

    /// Append a character to the input line.
    pub fn push_input_char(&mut self, c: char) {
        self.input.push(c);
    }

    /// Remove the last character from the input line, if any.
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Return the current input line and clear it. Also clears autocomplete state.
    pub fn take_input(&mut self) -> String {
        self.suggestions.clear();
        self.suggestion_idx = 0;
        std::mem::take(&mut self.input)
    }

    /// Clear the current autocomplete suggestions.
    pub fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_idx = 0;
    }

    /// Return the partial word the player is currently typing (the last
    /// whitespace-delimited token in `input`).
    pub fn current_partial(&self) -> &str {
        // Find the last space; if none, the whole input is the partial word.
        match self.input.rfind(' ') {
            Some(pos) => &self.input[pos + 1..],
            None => &self.input,
        }
    }

    // ── Search helpers ────────────────────────────────────────────────────────

    /// Run a case-insensitive substring search over the visible transcript lines.
    ///
    /// Fills `search_matches` with the 0-based positions (within the visible list)
    /// of lines that contain `query`. Sets `search_idx` to the last match index
    /// when `start_backward` is true (landing on the most recent match), or the
    /// first (0) when false. Sets `search_query` to the query string regardless of
    /// whether matches were found (so the status line can show "no matches").
    /// Returns the number of matches.
    pub fn run_search(&mut self, query: &str, start_backward: bool) -> usize {
        let query_lower = query.to_lowercase();
        let visible = self.visible_transcript_indices();
        self.search_matches = visible
            .iter()
            .enumerate()
            .filter(|&(_, &raw_idx)| {
                self.transcript[raw_idx].to_lowercase().contains(&query_lower)
            })
            .map(|(pos, _)| pos)
            .collect();
        let count = self.search_matches.len();
        self.search_idx = if start_backward && count > 0 { count - 1 } else { 0 };
        self.search_query = Some(query.to_string());
        count
    }

    /// Advance the current match by one step and return the new match's visible-list position.
    ///
    /// `forward = true` moves toward the end (newer lines); `forward = false` moves
    /// toward the start (older lines). Both directions wrap around. Returns `None` if
    /// there are no matches.
    pub fn search_next(&mut self, forward: bool) -> Option<usize> {
        let count = self.search_matches.len();
        if count == 0 {
            return None;
        }
        if forward {
            self.search_idx = (self.search_idx + 1) % count;
        } else {
            self.search_idx = self.search_idx.checked_sub(1).unwrap_or(count - 1);
        }
        Some(self.search_matches[self.search_idx])
    }

    /// Clear all search state: query, matches, and index.
    pub fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.search_idx = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_history_defaults_empty() {
        let s = AppState::default();
        assert!(s.history.is_empty(), "history starts empty");
    }

    #[test]
    fn replay_state_step_clamps_and_pauses() {
        let mut r = ReplayState::new(4); // start at last idx
        assert_eq!(r.idx, 4);
        r.step(-1, 5);
        assert_eq!(r.idx, 3);
        assert!(!r.playing, "manual step pauses");
        r.step(-10, 5);
        assert_eq!(r.idx, 0, "clamped at 0");
        r.step(10, 5);
        assert_eq!(r.idx, 4, "clamped at len-1");
    }

    #[test]
    fn replay_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.replay = Some(ReplayState::new(0));
        assert!(s.any_overlay_open(), "replay open => any_overlay_open true");
    }

    #[test]
    fn filter_maps_input_with_story_and_warning_with_meta() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("> go north", TranscriptKind::Input);
        s.push_transcript_kind("meta", TranscriptKind::Meta);
        s.push_transcript_kind("warn", TranscriptKind::Warning);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1]); // Story + Input
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![2, 3]); // Meta + Warning
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn current_room_name_defaults_none() {
        let s = AppState::default();
        assert_eq!(s.current_room_name, None);
    }

    #[test]
    fn visible_transcript_indices_respects_filter() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("meta1", TranscriptKind::Meta);
        s.push_transcript("story2");
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2]);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 2]);
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![1]);
    }

    #[test]
    fn transcript_tags_story_and_meta() {
        let mut s = AppState::default();
        s.push_transcript("West of House");
        s.push_transcript_kind("/help line", TranscriptKind::Meta);
        // last entry is Meta, prior is Story
        assert_eq!(s.transcript_kinds.len(), 2);
        assert!(matches!(s.transcript_kinds[0], TranscriptKind::Story));
        assert!(matches!(s.transcript_kinds[1], TranscriptKind::Meta));
    }

    #[test]
    fn sound_pulse_defaults_none_and_holds_kind() {
        use zvm::cpu::exec::Beep;
        let mut s = AppState::default();
        assert!(s.sound_pulse.is_none(), "no pulse by default");
        s.sound_pulse = Some(SoundPulse { kind: Beep::High, started: std::time::Instant::now() });
        assert!(matches!(s.sound_pulse.as_ref().map(|p| p.kind), Some(Beep::High)));
    }

    #[test]
    fn any_overlay_open_reflects_state() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open(), "default AppState must have no overlay open");

        // gallery
        s.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });
        assert!(s.any_overlay_open(), "gallery open => any_overlay_open true");
        s.gallery = None;

        // saves
        s.saves = Some(SavesState { entries: vec![], selected: 0 });
        assert!(s.any_overlay_open(), "saves open => any_overlay_open true");
        s.saves = None;

        // file_browser
        s.file_browser = Some(FileBrowserState::build(
            std::path::PathBuf::from("/tmp"),
            FbMode::PickFile,
            "x.qzl".to_string(),
        ));
        assert!(s.any_overlay_open(), "file_browser open => any_overlay_open true");
        s.file_browser = None;

        // config_screen
        s.config_screen = Some(ConfigScreenState {
            working: crate::config::Config::default(),
            selected: 0,
        });
        assert!(s.any_overlay_open(), "config_screen open => any_overlay_open true");
        s.config_screen = None;

        // verb_menu
        s.verb_menu = Some(VerbMenuState {
            pane: VerbMenuPane::Verbs,
            verb_idx: 0,
            noun_idx: 0,
            prep_idx: 0,
            nouns: vec![],
        });
        assert!(s.any_overlay_open(), "verb_menu open => any_overlay_open true");
        s.verb_menu = None;

        // hotkey_dialog
        s.hotkey_dialog = true;
        assert!(s.any_overlay_open(), "hotkey_dialog true => any_overlay_open true");
        s.hotkey_dialog = false;

        // room_panel
        s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        assert!(s.any_overlay_open(), "room_panel open => any_overlay_open true");
        s.room_panel = None;

        // tidy_anim
        s.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
        }]));
        assert!(s.any_overlay_open(), "tidy_anim active => any_overlay_open true");
        s.tidy_anim = None;

        // prompt
        s.prompt = Some(Prompt { kind: PromptKind::SaveAs, buffer: String::new() });
        assert!(s.any_overlay_open(), "prompt active => any_overlay_open true");
        s.prompt = None;

        // launch_dialog
        s.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.launch_dialog = false;

        assert!(!s.any_overlay_open(), "all cleared => any_overlay_open false again");
    }

    #[test]
    fn cycle_layout_reverse_goes_backwards() {
        let mut s = AppState::default();
        assert!(matches!(s.layout, Layout::Split));
        s.cycle_layout_reverse();
        assert!(matches!(s.layout, Layout::MapFull));
        s.cycle_layout_reverse();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.cycle_layout_reverse();
        assert!(matches!(s.layout, Layout::Split));
    }

    #[test]
    fn focus_layout_zoom_transitions() {
        let mut s = AppState::default();
        assert!(matches!(s.focus, Focus::Game));
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Map));
        s.cycle_layout();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.cycle_layout();
        assert!(matches!(s.layout, Layout::MapFull));
        s.cycle_layout();
        assert!(matches!(s.layout, Layout::Split));
        // zoom clamps — now uses 9-level fine zoom (0-8); starts at level 7 (Boxes).
        // Level 7→6: still Boxes; 6→5: Compact; …; 0: Overview; clamped at 0.
        // Zoom out to level 5 (first Compact level).
        s.zoom_out(); // 7→6: Boxes
        s.zoom_out(); // 6→5: Compact
        assert!(matches!(s.zoom, Zoom::Compact));
        // Zoom out to level 0 (Overview).
        s.zoom_out(); // 5→4: Compact
        s.zoom_out(); // 4→3: Compact
        s.zoom_out(); // 3→2: Overview
        s.zoom_out(); // 2→1: Overview
        s.zoom_out(); // 1→0: Overview
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out(); // clamp at 0
        assert!(matches!(s.zoom, Zoom::Overview)); // clamped
        // Zoom back to Boxes (need 8 zoom_in steps from 0).
        for _ in 0..8 {
            s.zoom_in();
        }
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in(); // clamp at 8
        assert!(matches!(s.zoom, Zoom::Boxes)); // clamped
    }

    #[test]
    fn input_line_and_transcript() {
        let mut s = AppState::default();
        s.push_input_char('g');
        s.push_input_char('o');
        s.backspace();
        assert_eq!(s.input, "g");
        let cmd = s.take_input();
        assert_eq!(cmd, "g");
        assert_eq!(s.input, "");
        s.push_transcript("line1\nline2");
        assert_eq!(s.transcript.len(), 2);
    }

    #[test]
    fn recenter_on_centers_cell() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (5, 5) in a 20×10 character pane:
        // cells_w = 20 / 13 = 1, cells_h = 10 / 7 = 1
        // scroll = (5 - 1/2, 5 - 1/2) = (5 - 0, 5 - 0) = (5, 5)
        s.recenter_on((5, 5), 20, 10);
        assert_eq!(s.scroll, (5, 5));
    }

    #[test]
    fn recenter_on_boxes_larger_pane() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (0, 0) in a 80×24 character pane:
        // cells_w = 80 / 13 = 6, cells_h = 24 / 7 = 3
        // scroll = (0 - 6/2, 0 - 3/2) = (0 - 3, 0 - 1) = (-3, -1)
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.scroll, (-3, -1));
    }

    #[test]
    fn recenter_on_compact_zoom() {
        use crate::state::Zoom;
        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // steps = (12, 5)
        // Centering cell (4, 4) in a 48×20 pane:
        // cells_w = 48 / 12 = 4, cells_h = 20 / 5 = 4
        // scroll = (4 - 4/2, 4 - 4/2) = (4 - 2, 4 - 2) = (2, 2)
        s.recenter_on((4, 4), 48, 20);
        assert_eq!(s.scroll, (2, 2));
    }

    #[test]
    fn pan_accumulates() {
        let mut s = AppState::default();
        s.pan(3, -2);
        s.pan(1, 4);
        assert_eq!(s.scroll, (4, 2));
    }

    #[test]
    fn select_room_roundtrip() {
        let mut s = AppState::default();
        assert_eq!(s.selected_room, None);
        s.select_room(Some(42));
        assert_eq!(s.selected_room, Some(42));
        s.select_room(None);
        assert_eq!(s.selected_room, None);
    }

    #[test]
    fn active_layer_follows_current_then_view_override() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_current(1);
        let l = g.new_layer(Some(0), "B".into());
        let mut s = AppState::default();
        assert_eq!(s.active_layer(&g), 0, "defaults to current room's layer");
        s.set_viewed_layer(Some(l));
        assert_eq!(s.active_layer(&g), l, "explicit view wins");
        s.set_viewed_layer(Some(999)); // stale id (no such layer)
        assert_eq!(s.active_layer(&g), 0, "stale view falls back to current room's layer");
    }

    #[test]
    fn appstate_default_symbols_are_default_set() {
        let st = AppState::default();
        assert_eq!(st.symbols, crate::symbols::SymbolSet::default());
    }

    #[test]
    fn gallery_state_symbol_config_roundtrips() {
        let g = GalleryState {
            category_idx: 0,
            selections: [0, 0, 0, 0], // rounded, filled, ascii, light (the defaults)
        };
        let cfg = g.symbol_config();
        assert_eq!(cfg.box_style, "rounded");
        assert_eq!(cfg.arrow_set, "filled");
        assert_eq!(cfg.portal_icons, "ascii");
        assert_eq!(cfg.path_style, "light");
    }

    // ── FileBrowserState tests ────────────────────────────────────────────────

    /// Create a temporary directory with a unique tag.
    /// Contents: subdir/, save.qzl, notes.txt.
    fn make_test_fb_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("babelmap-fb-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("save.qzl"), b"fake quetzal").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a save").unwrap();
        dir
    }

    #[test]
    fn filebrowser_pickfile_shows_dirs_and_qzl_not_txt() {
        let dir = make_test_fb_dir("pickfile");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile, "x.qzl".to_string());
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "should contain parent link");
        assert!(names.contains(&"subdir"), "should contain subdir");
        assert!(names.contains(&"save.qzl"), "should contain .qzl file");
        assert!(!names.contains(&"notes.txt"), ".txt file must not appear in PickFile mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_pickdir_shows_only_dirs() {
        let dir = make_test_fb_dir("pickdir");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickDir, "x.qzl".to_string());
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "should contain parent link");
        assert!(names.contains(&"subdir"), "should contain subdir");
        assert!(!names.contains(&"save.qzl"), ".qzl file must not appear in PickDir mode");
        assert!(!names.contains(&"notes.txt"), ".txt file must not appear in PickDir mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_dotdot_absent_at_root() {
        // Synthesize a state rooted at "/" (or the filesystem root on this OS).
        let root = std::path::Path::new("/");
        let fb = FileBrowserState::build(root.to_path_buf(), FbMode::PickDir, "x.qzl".to_string());
        let has_dotdot = fb.entries.iter().any(|e| e.name == "..");
        assert!(!has_dotdot, "'..' must not appear when at filesystem root");
    }

    #[test]
    fn filebrowser_cd_into_subdir_and_refresh() {
        let dir = make_test_fb_dir("cd");
        let mut fb = FileBrowserState::build(dir.clone(), FbMode::PickFile, "x.qzl".to_string());
        let subdir = dir.join("subdir");
        fb.cd(subdir.clone());
        assert_eq!(fb.cwd, subdir, "cwd should update after cd");
        assert_eq!(fb.selected, 0, "selection should reset to 0 after cd");
        // subdir is empty (no qzl files), but ".." should be present.
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "subdir should show '..'");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_entries_sorted_dirs_before_files() {
        let dir = make_test_fb_dir("sorted");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile, "x.qzl".to_string());
        // Verify: ".." first, then dirs, then files.
        let mut saw_dir = false;
        let mut saw_file = false;
        for e in &fb.entries {
            if e.is_dir {
                assert!(!saw_file, "dirs should appear before files, but saw a file first");
                saw_dir = true;
            } else {
                saw_file = true;
            }
        }
        assert!(saw_dir, "should have at least one dir");
        assert!(saw_file, "should have at least one file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── zoom_level / fine zoom tests (item 2) ────────────────────────────────

    #[test]
    fn zoom_level_default_is_boxes() {
        let s = AppState::default();
        assert_eq!(s.zoom_level, 7);
        assert!(matches!(s.zoom, Zoom::Boxes));
    }

    #[test]
    fn zoom_in_increments_level_and_clamps() {
        let mut s = AppState::default(); // level 7
        s.zoom_in(); // 7 -> 8
        assert_eq!(s.zoom_level, 8);
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in(); // clamp at 8
        assert_eq!(s.zoom_level, 8);
    }

    #[test]
    fn zoom_out_decrements_level_and_clamps() {
        let mut s = AppState::default(); // level 7
        s.zoom_out(); // 7 -> 6: Boxes
        assert_eq!(s.zoom_level, 6);
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_out(); // 6 -> 5: Compact
        assert_eq!(s.zoom_level, 5);
        assert!(matches!(s.zoom, Zoom::Compact));
        // Go all the way to 0
        for _ in 0..5 {
            s.zoom_out();
        }
        assert_eq!(s.zoom_level, 0);
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out(); // clamp at 0
        assert_eq!(s.zoom_level, 0);
    }

    #[test]
    fn zoom_reset_returns_to_default_level() {
        let mut s = AppState::default();
        // Go to Overview
        for _ in 0..7 {
            s.zoom_out();
        }
        assert!(matches!(s.zoom, Zoom::Overview));
        // Also set char_pan to something non-zero
        s.char_pan = (4, -2);
        // Reset
        s.zoom_reset();
        assert_eq!(s.zoom_level, 7, "zoom_reset must restore level to 7");
        assert!(matches!(s.zoom, Zoom::Boxes), "zoom_reset must restore Zoom::Boxes");
        assert_eq!(s.char_pan, (0, 0), "zoom_reset must clear char_pan");
    }

    #[test]
    fn zoom_from_level_maps_correctly() {
        use super::zoom_from_level;
        assert!(matches!(zoom_from_level(0), Zoom::Overview));
        assert!(matches!(zoom_from_level(1), Zoom::Overview));
        assert!(matches!(zoom_from_level(2), Zoom::Overview));
        assert!(matches!(zoom_from_level(3), Zoom::Compact));
        assert!(matches!(zoom_from_level(4), Zoom::Compact));
        assert!(matches!(zoom_from_level(5), Zoom::Compact));
        assert!(matches!(zoom_from_level(6), Zoom::Boxes));
        assert!(matches!(zoom_from_level(7), Zoom::Boxes));
        assert!(matches!(zoom_from_level(8), Zoom::Boxes));
    }

    // ── char_pan / drag-pan tests (item 1) ───────────────────────────────────

    #[test]
    fn char_pan_default_is_zero() {
        let s = AppState::default();
        assert_eq!(s.char_pan, (0, 0));
    }

    #[test]
    fn recenter_on_clears_char_pan() {
        let mut s = AppState::default();
        s.char_pan = (5, -3);
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.char_pan, (0, 0), "recenter_on must reset char_pan to (0,0)");
    }

    #[test]
    fn reset_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.reset_dialog = true;
        assert!(s.any_overlay_open(), "reset_dialog open => any_overlay_open true");
    }

    #[test]
    fn quit_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.quit_dialog = true;
        assert!(s.any_overlay_open(), "quit_dialog open => any_overlay_open true");
        s.quit_dialog = false;
        assert!(!s.any_overlay_open(), "quit_dialog false => any_overlay_open false");
    }

    #[test]
    fn hints_panel_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());

        // Build a minimal HintSession using the minizork fixture (same approach as
        // the reset test in input.rs). If the fixture is absent we skip.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes).expect("GameSession::new");
        s.hints = Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        });
        assert!(s.any_overlay_open(), "hints open => any_overlay_open true");
        s.hints = None;
        assert!(!s.any_overlay_open(), "hints closed => any_overlay_open false");
    }

    #[test]
    fn run_search_direction_and_next_wrap() {
        let mut s = AppState::default();
        for t in ["alpha", "beta", "alpha again", "gamma", "ALPHA"] { s.push_transcript(t); }
        // matches for "alpha" at visible positions 0, 2, 4 (case-insensitive)
        let n = s.run_search("alpha", true); // start backward → last match
        assert_eq!(n, 3);
        assert_eq!(s.search_matches, vec![0, 2, 4]);
        assert_eq!(s.search_idx, 2); // index into search_matches → position 4
        // n = back
        assert_eq!(s.search_next(false), Some(2)); // now at match position 2
        // forward wraps from 2 → 4 → back to 0
        let _ = s.search_next(true); // → 4
        assert_eq!(s.search_next(true), Some(0)); // wrap to first
        let f = s.run_search("alpha", false); // start forward → first match
        assert_eq!(f, 3);
        assert_eq!(s.search_idx, 0);
        s.clear_search();
        assert!(s.search_query.is_none() && s.search_matches.is_empty());
    }
}
