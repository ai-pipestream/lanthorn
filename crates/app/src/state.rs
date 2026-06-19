use mapper::direction::Direction;
use mapper::graph::RoomId;

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
        }
    }
}

impl AppState {
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
    /// Formula: scroll = cell - floor(pane / 2)
    /// i.e. scroll.x = cell.0 - pane_w/2, scroll.y = cell.1 - pane_h/2
    /// This places `cell` at the center of the visible pane.
    pub fn recenter_on(&mut self, cell: (i32, i32), pane_w: u16, pane_h: u16) {
        self.scroll = (
            cell.0 - (pane_w / 2) as i32,
            cell.1 - (pane_h / 2) as i32,
        );
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

    /// Return the current input line and clear it.
    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.input)
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
        let mut s = AppState::default();
        // Centering cell (5, 5) in a 20×10 pane:
        // scroll.x = 5 - 20/2 = 5 - 10 = -5
        // scroll.y = 5 - 10/2 = 5 - 5 = 0
        s.recenter_on((5, 5), 20, 10);
        assert_eq!(s.scroll, (-5, 0));
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
}
