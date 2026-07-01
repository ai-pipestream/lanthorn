//! The app's [`gvm::glk::GlkBackend`] implementation ([`AppGlk`]).
//!
//! A running Glulx game drives Glk display calls (window open/close/arrange,
//! `put_text`, `grid_put`/`grid_clear`, …); `AppGlk` records them and projects
//! them onto the engine-neutral [`ScreenModel`] window tree (the same tree the
//! Z-machine produces), so the one generic renderer draws both engines.
//!
//! Glk styles map to the same text-style bits the transcript runs use
//! ([`glk_style_bits`]), so emphasis renders for free. The **primary** text-
//! buffer window (the first one opened) is the one whose output the app mirrors
//! into `state.transcript` (search / persistence / styling); its new text is
//! drained via [`AppGlk::take_transcript`]. Extra buffer windows carry their
//! inline content in the [`BufferWindow`] node.

use std::any::Any;
use std::collections::BTreeMap;

use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, WinType};

use crate::engine::{
    BufferWindow, GridCell, GridWindow, ScreenModel, Split, StatusModel, WinNode,
};
use crate::state::StyleRun;

// ── Glk style → text-style bits ────────────────────────────────────────────────

/// Map a Glk style class to the neutral text-style bitset used by the transcript
/// runs (1 = reverse, 2 = bold, 4 = italic, 8 = fixed-pitch).
pub fn glk_style_bits(style: GlkStyle) -> u8 {
    match style {
        GlkStyle::Emphasized => 0x02,   // bold
        GlkStyle::Header => 0x02,       // bold
        GlkStyle::Subheader => 0x02,    // bold
        GlkStyle::Input => 0x02,        // bold
        GlkStyle::Alert => 0x03,        // bold + reverse
        GlkStyle::Preformatted => 0x08, // fixed-pitch
        GlkStyle::Normal
        | GlkStyle::Note
        | GlkStyle::BlockQuote
        | GlkStyle::User1
        | GlkStyle::User2 => 0,
    }
}

// ── Per-window record ──────────────────────────────────────────────────────────

/// A text-grid window's cell buffer (cells keyed by 0-based `(row, col)`).
#[derive(Default)]
struct GridBuf {
    width: u32,
    height: u32,
    cells: BTreeMap<(u32, u32), (char, u8)>,
}

/// A text-buffer window's styled output log.
#[derive(Default)]
struct BufBuf {
    /// Every `put_text` run in order, as `(style-bits, text)`.
    log: Vec<(u8, String)>,
    /// Number of leading log entries already drained by `take_transcript`.
    drained: usize,
    /// Scrollback offset for an inline (non-primary) buffer window.
    scroll: u16,
}

// ── The backend ────────────────────────────────────────────────────────────────

/// The app Glk display backend (see the module docs).
pub struct AppGlk {
    /// Reported display size (the story-pane size the game lays windows out in).
    cols: u32,
    rows: u32,
    /// The latest resolved leaf-window layout `(id, type, rect)`.
    layout: Vec<(u32, WinType, GlkRect)>,
    grids: BTreeMap<u32, GridBuf>,
    buffers: BTreeMap<u32, BufBuf>,
    /// The primary buffer window id (the first text-buffer opened), if any.
    primary: Option<u32>,
}

impl Default for AppGlk {
    fn default() -> Self {
        AppGlk::new(80, 24)
    }
}

impl AppGlk {
    /// A backend reporting a `cols × rows` display.
    pub fn new(cols: u32, rows: u32) -> AppGlk {
        AppGlk {
            cols,
            rows,
            layout: Vec::new(),
            grids: BTreeMap::new(),
            buffers: BTreeMap::new(),
            primary: None,
        }
    }

    /// Update the reported display size (the host story-pane size each frame).
    pub fn set_screen_size(&mut self, cols: u32, rows: u32) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
    }

    /// The primary text-buffer window id, if one is open.
    pub fn primary(&self) -> Option<u32> {
        self.primary
    }

    /// Drain the primary window's text printed since the last drain, as
    /// `(text, (char_count, bits, fg, bg) chunks)` for `push_transcript_runs`.
    /// Glulx colour is not yet threaded through AppGlk, so fg/bg default to
    /// `ZColour::Default` for now.
    pub fn take_transcript(&mut self) -> (String, Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)>) {
        let Some(pid) = self.primary else {
            return (String::new(), Vec::new());
        };
        let Some(buf) = self.buffers.get_mut(&pid) else {
            return (String::new(), Vec::new());
        };
        let mut text = String::new();
        let mut chunks: Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)> = Vec::new();
        for (bits, s) in &buf.log[buf.drained..] {
            let n = s.chars().count();
            if n == 0 {
                continue;
            }
            chunks.push((n, *bits, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default));
            text.push_str(s);
        }
        buf.drained = buf.log.len();
        (text, chunks)
    }

    /// Project the recorded Glk state onto the neutral [`ScreenModel`].
    pub fn screen_model(&self) -> ScreenModel {
        // Build a (rect, node) pair for each laid-out leaf window, then assemble
        // the guillotine tree from the rects.
        let mut leaves: Vec<(GlkRect, WinNode)> = Vec::new();
        for &(id, ty, rect) in &self.layout {
            let node = match ty {
                WinType::TextGrid => WinNode::Grid(self.grid_node(id, rect)),
                WinType::TextBuffer => WinNode::Buffer(self.buffer_node(id)),
                WinType::Pair => continue, // pair windows are never in the layout
            };
            leaves.push((rect, node));
        }
        let root = assemble(&leaves);
        ScreenModel { root, status: StatusModel::HostManaged }
    }

    fn grid_node(&self, id: u32, rect: GlkRect) -> GridWindow {
        let g = self.grids.get(&id);
        let cols = g.map(|g| g.width).unwrap_or(rect.width).max(rect.width) as u16;
        let rows = g.map(|g| g.height).unwrap_or(rect.height).max(rect.height) as u16;
        let mut cells = vec![GridCell::default(); cols as usize * rows as usize];
        if let Some(g) = g {
            for (&(r, c), &(ch, bits)) in &g.cells {
                if r < rows as u32 && c < cols as u32 {
                    cells[r as usize * cols as usize + c as usize] = GridCell { ch, style: bits, fg: 0, bg: 0 };
                }
            }
        }
        GridWindow {
            cols,
            rows,
            cells,
            active_rows: rows,
            cursor: (1, 1),
            cursor_active: false,
        }
    }

    fn buffer_node(&self, id: u32) -> BufferWindow {
        if self.primary == Some(id) {
            // The primary buffer is mirrored by the app transcript; carry no
            // inline content (the renderer draws it via the transcript path).
            return BufferWindow { primary: true, ..Default::default() };
        }
        let buf = self.buffers.get(&id);
        let (lines, runs) = buf.map(|b| log_to_lines(&b.log)).unwrap_or_default();
        let scroll = buf.map(|b| b.scroll).unwrap_or(0);
        BufferWindow { lines, runs, scroll, primary: false }
    }
}

// ── Tree assembly + log → lines helpers ────────────────────────────────────────

/// Assemble a guillotine window tree from laid-out leaves `(rect, node)`.
///
/// Glk layouts are always recursive guillotine splits, so the leaf rects admit a
/// clean horizontal or vertical cut at each level. Picks the smallest cut for
/// determinism; falls back to the first leaf if no clean cut exists.
fn assemble(leaves: &[(GlkRect, WinNode)]) -> WinNode {
    match leaves.len() {
        0 => return WinNode::Blank,
        1 => return leaves[0].1.clone(),
        _ => {}
    }
    let region = bounding_box(leaves);

    // Try a horizontal cut (stacked top/bottom → vertical Pair).
    let mut tops: Vec<u32> = leaves.iter().map(|(r, _)| r.top).filter(|&t| t > region.top).collect();
    tops.sort_unstable();
    tops.dedup();
    for &cut in &tops {
        let top: Vec<(GlkRect, WinNode)> = leaves
            .iter()
            .filter(|(r, _)| r.top + r.height <= cut)
            .cloned()
            .collect();
        let bottom: Vec<(GlkRect, WinNode)> = leaves
            .iter()
            .filter(|(r, _)| r.top >= cut)
            .cloned()
            .collect();
        if !top.is_empty() && top.len() + bottom.len() == leaves.len() && !bottom.is_empty() {
            return WinNode::Pair {
                vertical: true,
                split: Split { fixed: (cut - region.top) as u16 },
                first: Box::new(assemble(&top)),
                second: Box::new(assemble(&bottom)),
            };
        }
    }

    // Try a vertical cut (side-by-side left/right → horizontal Pair).
    let mut lefts: Vec<u32> = leaves.iter().map(|(r, _)| r.left).filter(|&l| l > region.left).collect();
    lefts.sort_unstable();
    lefts.dedup();
    for &cut in &lefts {
        let left: Vec<(GlkRect, WinNode)> = leaves
            .iter()
            .filter(|(r, _)| r.left + r.width <= cut)
            .cloned()
            .collect();
        let right: Vec<(GlkRect, WinNode)> = leaves
            .iter()
            .filter(|(r, _)| r.left >= cut)
            .cloned()
            .collect();
        if !left.is_empty() && left.len() + right.len() == leaves.len() && !right.is_empty() {
            return WinNode::Pair {
                vertical: false,
                split: Split { fixed: (cut - region.left) as u16 },
                first: Box::new(assemble(&left)),
                second: Box::new(assemble(&right)),
            };
        }
    }

    // No clean guillotine cut (shouldn't happen for Glk): show the first leaf.
    leaves[0].1.clone()
}

/// The bounding rectangle of a set of leaves.
fn bounding_box(leaves: &[(GlkRect, WinNode)]) -> GlkRect {
    let left = leaves.iter().map(|(r, _)| r.left).min().unwrap_or(0);
    let top = leaves.iter().map(|(r, _)| r.top).min().unwrap_or(0);
    let right = leaves.iter().map(|(r, _)| r.left + r.width).max().unwrap_or(0);
    let bottom = leaves.iter().map(|(r, _)| r.top + r.height).max().unwrap_or(0);
    GlkRect { left, top, width: right - left, height: bottom - top }
}

/// Split a buffer window's styled log into `(lines, per-line runs)`, merging
/// adjacent same-style chars into one [`StyleRun`].
fn log_to_lines(log: &[(u8, String)]) -> (Vec<String>, Vec<Vec<StyleRun>>) {
    let mut lines: Vec<String> = vec![String::new()];
    let mut runs: Vec<Vec<StyleRun>> = vec![Vec::new()];
    for (bits, text) in log {
        for ch in text.chars() {
            if ch == '\n' {
                lines.push(String::new());
                runs.push(Vec::new());
                continue;
            }
            let li = lines.len() - 1;
            let col = lines[li].chars().count();
            lines[li].push(ch);
            if *bits != 0 {
                let r = &mut runs[li];
                match r.last_mut() {
                    Some(last) if last.bits == *bits && last.end == col => last.end = col + 1,
                    _ => r.push(StyleRun { start: col, end: col + 1, bits: *bits, fg: 0, bg: 0 }),
                }
            }
        }
    }
    (lines, runs)
}

// ── GlkBackend impl ────────────────────────────────────────────────────────────

impl GlkBackend for AppGlk {
    fn screen_size(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    fn window_open(&mut self, id: u32, wintype: WinType) {
        match wintype {
            WinType::TextGrid => {
                self.grids.entry(id).or_default();
            }
            WinType::TextBuffer => {
                self.buffers.entry(id).or_default();
                if self.primary.is_none() {
                    self.primary = Some(id);
                }
            }
            WinType::Pair => {}
        }
    }

    fn window_close(&mut self, id: u32) {
        self.grids.remove(&id);
        self.buffers.remove(&id);
        self.layout.retain(|&(wid, _, _)| wid != id);
        if self.primary == Some(id) {
            self.primary = None;
        }
    }

    fn window_layout(&mut self, wins: &[(u32, WinType, GlkRect)]) {
        self.layout = wins.to_vec();
        for &(id, ty, rect) in wins {
            if ty == WinType::TextGrid {
                let g = self.grids.entry(id).or_default();
                g.width = rect.width;
                g.height = rect.height;
            }
        }
    }

    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        let buf = self.buffers.entry(win).or_default();
        buf.log.push((glk_style_bits(style), s.to_string()));
    }

    fn grid_put(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, s: &str) {
        let bits = glk_style_bits(style);
        let g = self.grids.entry(win).or_default();
        for (i, ch) in s.chars().enumerate() {
            g.cells.insert((y, x + i as u32), (ch, bits));
        }
    }

    fn grid_clear(&mut self, win: u32) {
        if let Some(g) = self.grids.get_mut(&win) {
            g.cells.clear();
        }
    }

    fn window_clear(&mut self, win: u32) {
        if let Some(b) = self.buffers.get_mut(&win) {
            b.log.clear();
            b.drained = 0;
        }
    }

    fn flush(&mut self) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: u32, top: u32, width: u32, height: u32) -> GlkRect {
        GlkRect { left, top, width, height }
    }

    #[test]
    fn glk_styles_map_to_bits() {
        assert_eq!(glk_style_bits(GlkStyle::Normal), 0);
        assert_eq!(glk_style_bits(GlkStyle::Emphasized), 0x02);
        assert_eq!(glk_style_bits(GlkStyle::Header), 0x02);
        assert_eq!(glk_style_bits(GlkStyle::Alert), 0x03);
        assert_eq!(glk_style_bits(GlkStyle::Preformatted), 0x08);
    }

    #[test]
    fn grid_over_buffer_builds_pair_tree() {
        // A 1-row TextGrid (id 2) stacked above an 80x23 TextBuffer (id 1).
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextGrid);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 1, 80, 23)),
            (2, WinType::TextGrid, rect(0, 0, 80, 1)),
        ]);
        let model = glk.screen_model();
        match &model.root {
            WinNode::Pair { vertical, split, first, second } => {
                assert!(*vertical, "grid-above-buffer is a vertical stack");
                assert_eq!(split.fixed, 1, "the 1-row grid is the fixed first child");
                assert!(matches!(**first, WinNode::Grid(_)), "top child is the grid");
                assert!(matches!(**second, WinNode::Buffer(_)), "bottom child is the buffer");
            }
            other => panic!("expected a Pair, got {other:?}"),
        }
        // The buffer is the primary (mirrored by the transcript).
        assert_eq!(glk.primary(), Some(1));
        assert!(model.grid().is_some(), "the tree exposes a grid node");
    }

    #[test]
    fn three_window_split_nests() {
        // Grid (id 3, top row) over a left/right buffer split (ids 1, 2).
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextBuffer);
        glk.window_open(3, WinType::TextGrid);
        glk.window_layout(&[
            (3, WinType::TextGrid, rect(0, 0, 80, 1)),
            (1, WinType::TextBuffer, rect(0, 1, 40, 23)),
            (2, WinType::TextBuffer, rect(40, 1, 40, 23)),
        ]);
        let model = glk.screen_model();
        // Top-level: vertical pair (grid over the rest).
        let WinNode::Pair { vertical, first, second, .. } = &model.root else {
            panic!("expected a top-level Pair");
        };
        assert!(*vertical);
        assert!(matches!(**first, WinNode::Grid(_)));
        // The lower region is a horizontal (side-by-side) pair of two buffers.
        let WinNode::Pair { vertical: v2, first: f2, second: s2, .. } = &**second else {
            panic!("expected a nested Pair for the two buffers");
        };
        assert!(!*v2, "two side-by-side buffers form a horizontal pair");
        assert!(matches!(**f2, WinNode::Buffer(_)));
        assert!(matches!(**s2, WinNode::Buffer(_)));
    }

    #[test]
    fn put_text_styles_inline_buffer() {
        // Two buffers: id 1 is primary (drained), id 2 is inline.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_open(2, WinType::TextBuffer);
        glk.window_layout(&[
            (1, WinType::TextBuffer, rect(0, 0, 40, 24)),
            (2, WinType::TextBuffer, rect(40, 0, 40, 24)),
        ]);
        glk.put_text(2, GlkStyle::Normal, "ab");
        glk.put_text(2, GlkStyle::Header, "CD");
        glk.put_text(2, GlkStyle::Normal, "\nx");

        let model = glk.screen_model();
        // Find the inline (non-primary) buffer node.
        fn find_buffers(n: &WinNode, out: &mut Vec<BufferWindow>) {
            match n {
                WinNode::Buffer(b) => out.push(b.clone()),
                WinNode::Pair { first, second, .. } => {
                    find_buffers(first, out);
                    find_buffers(second, out);
                }
                _ => {}
            }
        }
        let mut bufs = Vec::new();
        find_buffers(&model.root, &mut bufs);
        let inline = bufs.iter().find(|b| !b.primary).expect("an inline buffer exists");
        assert_eq!(inline.lines, vec!["abCD".to_string(), "x".to_string()]);
        // "CD" (cols 2..4) is bold (Header → 0x02), merged into one run.
        assert_eq!(inline.runs[0], vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0 }]);
        assert!(inline.runs[1].is_empty());
    }

    #[test]
    fn primary_text_is_drainable() {
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        glk.window_layout(&[(1, WinType::TextBuffer, rect(0, 0, 80, 24))]);
        glk.put_text(1, GlkStyle::Normal, "You are here. ");
        glk.put_text(1, GlkStyle::Emphasized, "Look!");
        let (text, chunks) = glk.take_transcript();
        assert_eq!(text, "You are here. Look!");
        assert_eq!(chunks, vec![
            (14, 0u8, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default),
            (5, 0x02u8, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default),
        ]);
        // A second drain returns only new text.
        glk.put_text(1, GlkStyle::Normal, " More.");
        let (text2, _) = glk.take_transcript();
        assert_eq!(text2, " More.");
        // The primary buffer node carries no inline content.
        let model = glk.screen_model();
        if let WinNode::Buffer(b) = &model.root {
            assert!(b.primary && b.lines.is_empty());
        } else {
            panic!("single buffer is the root");
        }
    }

    #[test]
    fn grid_put_and_clear_update_cells() {
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextGrid);
        glk.window_layout(&[(1, WinType::TextGrid, rect(0, 0, 10, 2))]);
        glk.grid_put(1, 2, 0, GlkStyle::Header, "Hi");
        let model = glk.screen_model();
        let g = model.grid().expect("grid node");
        assert_eq!((g.cols, g.rows), (10, 2));
        // 1-based (row 1, col 3) holds 'H' bold; col 4 holds 'i'.
        assert_eq!(g.cell(1, 3).ch, 'H');
        assert_eq!(g.cell(1, 3).style, 0x02);
        assert_eq!(g.cell(1, 4).ch, 'i');
        // Clear empties the cells.
        glk.grid_clear(1);
        let g2 = glk.screen_model();
        assert_eq!(g2.grid().unwrap().cell(1, 3).ch, ' ');
    }
}
