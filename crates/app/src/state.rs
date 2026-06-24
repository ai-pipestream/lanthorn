use std::time::{Duration, Instant};

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;

// ── Tidy animation ────────────────────────────────────────────────────────────

/// One captured stage of the tidy pipeline, held for playback. `graph` is a clone
/// of the layout as it stood after the named stage ran.
#[derive(Debug, Clone)]
pub struct TidyFrame {
    pub label: String,
    pub graph: MapGraph,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Game,
    Map,
}

// ── Prompt sub-mode ───────────────────────────────────────────────────────────

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
    /// Map scroll offset in grid cells: (x, y).
    pub scroll: (i32, i32),
    pub selected_room: Option<RoomId>,
    pub transcript: Vec<String>,
    pub transcript_scroll: u16,
    pub input: String,
    // Reserved for future status-bar messages (not yet displayed).
    #[allow(dead_code)]
    pub status: String,
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
    /// Explicit layer override for the map view. `None` means follow the current room's layer.
    pub viewed_layer: Option<LayerId>,

    /// Resolved glyph set for the map renderer.  Defaults to today's hardcoded glyphs;
    /// overwritten at startup via `SymbolSet::resolve(&cfg.symbols)` when a config is present.
    pub symbols: crate::symbols::SymbolSet,

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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            focus: Focus::Game,
            layout: Layout::Split,
            zoom: Zoom::Boxes,
            scroll: (0, 0),
            selected_room: None,
            transcript: Vec::new(),
            transcript_scroll: 0,
            input: String::new(),
            status: String::new(),
            prompt: None,
            show_alignment: false,
            show_portal_labels: false,
            tidy_anim: None,
            viewed_layer: None,
            symbols: crate::symbols::SymbolSet::default(),
            dict_words: Vec::new(),
            suggestions: Vec::new(),
            suggestion_idx: 0,
        }
    }
}

impl AppState {
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

    /// Zoom in toward Boxes (more detail). Clamps at Boxes.
    pub fn zoom_in(&mut self) {
        self.zoom = match self.zoom {
            Zoom::Overview => Zoom::Compact,
            Zoom::Compact => Zoom::Boxes,
            Zoom::Boxes => Zoom::Boxes, // already at max detail
        };
    }

    /// Zoom out toward Overview (less detail). Clamps at Overview.
    pub fn zoom_out(&mut self) {
        self.zoom = match self.zoom {
            Zoom::Boxes => Zoom::Compact,
            Zoom::Compact => Zoom::Overview,
            Zoom::Overview => Zoom::Overview, // already at min detail
        };
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
    pub fn recenter_on(&mut self, cell: (i32, i32), pane_w: u16, pane_h: u16) {
        let (sw, sh) = self.zoom.steps();
        let cells_w = (pane_w as i32 / sw).max(1);
        let cells_h = (pane_h as i32 / sh).max(1);
        self.scroll = (cell.0 - cells_w / 2, cell.1 - cells_h / 2);
    }

    /// Split `text` on `'\n'` and append each line to the transcript.
    pub fn push_transcript(&mut self, text: &str) {
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // zoom clamps
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Compact));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview)); // clamped
        s.zoom_in();
        s.zoom_in();
        s.zoom_in();
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
        let mut s = AppState::default(); // Boxes zoom (step 18×6)
        // Centering cell (5, 5) in a 20×10 character pane:
        // cells_w = 20 / 18 = 1, cells_h = 10 / 6 = 1
        // scroll = (5 - 1/2, 5 - 1/2) = (5 - 0, 5 - 0) = (5, 5)
        s.recenter_on((5, 5), 20, 10);
        assert_eq!(s.scroll, (5, 5));
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
}
