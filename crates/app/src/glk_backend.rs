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

use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, StyleColour, WinType};

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

/// Resolve a Glk style class + its stylehint colour into the app's neutral
/// `(style-bits, packed-fg, packed-bg)`. The reverse hint sets bit `0x01`;
/// 24-bit RGB is carried losslessly via
/// [`ZColour::True24`](zvm::screen::ZColour::True24). Packed colours use
/// [`crate::state::pack_zcolour`] (`0` = `ZColour::Default`).
///
/// Colour is recorded unconditionally — the `honor_game_colours` gate is applied
/// at *render* time by `cell_style`/`draw_str_runs`, exactly like the Z-machine,
/// so toggling it (F2) recolours already-drawn output too.
fn resolve_glk_colour(style: GlkStyle, colour: StyleColour) -> (u8, u32, u32) {
    let mut bits = glk_style_bits(style);
    if colour.reverse {
        bits |= 0x01;
    }
    let pack = |o: Option<u32>| {
        o.map(|v| crate::state::pack_zcolour(zvm::screen::ZColour::True24(v))).unwrap_or(0)
    };
    (bits, pack(colour.fg), pack(colour.bg))
}

// ── Per-window record ──────────────────────────────────────────────────────────

/// A text-grid window's cell buffer (cells keyed by 0-based `(row, col)`).
#[derive(Default)]
struct GridBuf {
    width: u32,
    height: u32,
    /// `(row, col) -> (char, style-bits, packed-fg, packed-bg)`.
    cells: BTreeMap<(u32, u32), (char, u8, u32, u32)>,
}

/// One entry in a text-buffer window's ordered output log.
enum BufElem {
    /// A run of printed text with its style bits and packed colours.
    Text { bits: u8, fg: u32, bg: u32, text: String },
    /// An image drawn into this buffer window (Glk `glk_image_draw`).
    Image(crate::inline_image::InlineImage),
}

/// A text-buffer window's ordered output log (text runs + inline images).
#[derive(Default)]
struct BufBuf {
    log: Vec<BufElem>,
    /// Number of leading log entries already drained by `take_transcript*`.
    drained: usize,
    /// Scrollback offset for an inline (non-primary) buffer window.
    scroll: u16,
}

// ── The backend ────────────────────────────────────────────────────────────────

/// A live Glk sound channel's app-side state.
struct SoundChannel {
    rock: u32,
    /// Glk volume (0x10000 = full); snapshotted into each `Play` op.
    volume: u32,
}

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
    /// Accumulator for the current run of `Subheader` text in the primary
    /// window (the Inform 7 room heading, captured char-by-char).
    heading_acc: String,
    /// The last completed `Subheader` line seen since the previous drain — the
    /// current room heading (`None` if this turn printed none).
    last_heading: Option<String>,
    /// Whether the primary window's output stream is at the start of a line.
    /// A room heading is a `Subheader` run that BEGINS here; `Subheader` runs
    /// beginning mid-line are inline hyperlinks (e.g. Superluminal's command
    /// hints), not rooms.
    at_line_start: bool,
    /// Whether `heading_acc` is an active heading run (a `Subheader` run that
    /// began at line start and has not yet been terminated).
    in_heading: bool,
    /// Graphics-window pixel canvases, keyed by window id.
    graphics: std::collections::BTreeMap<u32, crate::graphics::Canvas>,
    /// The `(width, height)` of one text-grid cell in pixels, for pixel↔cell
    /// layout of graphics windows.
    char_px: (u32, u32),
    /// Resolves + caches Blorb `Pict` resources for `graphics_draw_image`.
    picts: crate::graphics::PictSource,
    /// Live sound channels, keyed by Glk channel ref (BTree for stable iterate).
    schannels: BTreeMap<u32, SoundChannel>,
    /// Next channel ref to hand out (pre-incremented; first create → 1).
    next_schannel: u32,
    /// Buffered per-turn sound operations, drained by `take_sound_ops`.
    sound_ops: Vec<crate::session::SchannelOp>,
}

impl Default for AppGlk {
    fn default() -> Self {
        AppGlk::new(80, 24)
    }
}

impl AppGlk {
    /// A backend reporting a `cols × rows` display. Stylehint colour is always
    /// recorded; the `honor_game_colours` gate is applied at render time.
    pub fn new(cols: u32, rows: u32) -> AppGlk {
        AppGlk::with_graphics(cols, rows, (1, 1), crate::graphics::PictSource::new(None))
    }

    /// A backend also carrying the char-cell pixel size and a `Pict` source,
    /// needed for graphics windows.
    pub fn with_graphics(
        cols: u32,
        rows: u32,
        char_px: (u32, u32),
        picts: crate::graphics::PictSource,
    ) -> AppGlk {
        AppGlk {
            cols,
            rows,
            layout: Vec::new(),
            grids: BTreeMap::new(),
            buffers: BTreeMap::new(),
            primary: None,
            heading_acc: String::new(),
            last_heading: None,
            at_line_start: true,
            in_heading: false,
            graphics: BTreeMap::new(),
            char_px,
            picts,
            schannels: BTreeMap::new(),
            next_schannel: 0,
            sound_ops: Vec::new(),
        }
    }

    /// The pixel size of a graphics window `win` as laid out, from `layout` ×
    /// `char_px`. Does not borrow `self.graphics`, so it can be called while
    /// `self.graphics.entry(..)` is held.
    fn canvas_size(&self, win: u32) -> (u32, u32) {
        let cells = self
            .layout
            .iter()
            .find(|&&(id, _, _)| id == win)
            .map(|&(_, _, r)| (r.width, r.height))
            .unwrap_or((1, 1));
        (cells.0 * self.char_px.0, cells.1 * self.char_px.1)
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
    /// fg/bg carry the resolved stylehint colour (24-bit via `ZColour::True24`).
    pub fn take_transcript(&mut self) -> (String, Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)>) {
        let Some(pid) = self.primary else {
            return (String::new(), Vec::new());
        };
        let Some(buf) = self.buffers.get_mut(&pid) else {
            return (String::new(), Vec::new());
        };
        let mut text = String::new();
        let mut chunks: Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)> = Vec::new();
        for elem in &buf.log[buf.drained..] {
            let BufElem::Text { bits, fg, bg, text: s } = elem else { continue };
            let n = s.chars().count();
            if n == 0 {
                continue;
            }
            chunks.push((n, *bits, crate::state::unpack_zcolour(*fg), crate::state::unpack_zcolour(*bg)));
            text.push_str(s);
        }
        buf.drained = buf.log.len();
        (text, chunks)
    }

    /// Drain the primary window's undrained log into ordered transcript
    /// elements (consecutive text runs coalesced; images preserved in place).
    pub fn take_transcript_elems(&mut self) -> Vec<crate::session::TranscriptElem> {
        use crate::session::TranscriptElem;
        let Some(pid) = self.primary else { return Vec::new() };
        let Some(buf) = self.buffers.get_mut(&pid) else { return Vec::new() };
        let mut out: Vec<TranscriptElem> = Vec::new();
        // Accumulate consecutive Text runs into one element, matching the
        // char-count chunk shape `push_transcript_runs` expects:
        // (char_count, bits, fg, bg).
        let mut cur_text = String::new();
        let mut cur_runs: Vec<(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)> = Vec::new();
        let flush = |out: &mut Vec<TranscriptElem>, text: &mut String, runs: &mut Vec<_>| {
            if !text.is_empty() {
                out.push(TranscriptElem::Text { text: std::mem::take(text), runs: std::mem::take(runs) });
            } else {
                runs.clear();
            }
        };
        for elem in &buf.log[buf.drained..] {
            match elem {
                BufElem::Text { bits, fg, bg, text } => {
                    let n = text.chars().count();
                    if n > 0 {
                        // Convert packed u32 colours back to ZColour to match
                        // the chunk type push_transcript_runs consumes.
                        let (f, b) = (crate::state::unpack_zcolour(*fg), crate::state::unpack_zcolour(*bg));
                        cur_runs.push((n, *bits, f, b));
                        cur_text.push_str(text);
                    }
                }
                BufElem::Image(img) => {
                    flush(&mut out, &mut cur_text, &mut cur_runs);
                    out.push(TranscriptElem::Image(img.clone()));
                }
            }
        }
        flush(&mut out, &mut cur_text, &mut cur_runs);
        buf.drained = buf.log.len();
        out
    }

    /// Drain the sound operations buffered this turn (see [`crate::session::SchannelOp`]).
    pub fn take_sound_ops(&mut self) -> Vec<crate::session::SchannelOp> {
        std::mem::take(&mut self.sound_ops)
    }

    /// Test seam: append an inline image to the primary buffer's undrained log,
    /// as a game's `glk_image_draw` into the buffer window would (a resolvable
    /// Pict needs a Blorb the unit harness lacks). Lets a `GlulxSession` test
    /// exercise the banner/startup image path without a Blorb.
    #[cfg(test)]
    pub(crate) fn test_push_primary_image(&mut self, img: crate::inline_image::InlineImage) {
        if let Some(pid) = self.primary {
            if let Some(buf) = self.buffers.get_mut(&pid) {
                buf.log.push(BufElem::Image(img));
            }
        }
    }

    /// Feed one primary-window output run into the room-heading detector.
    ///
    /// The Inform 7 room heading is a `Subheader` run printed on its OWN line, so
    /// a heading is a `Subheader` run that begins at line start (tracked by
    /// `at_line_start`). A `Subheader` run that begins mid-line is an inline
    /// hyperlink — e.g. Superluminal Vagrant Twin renders its "credits"/"land"
    /// command hints as mid-line `Subheader` — and is ignored. A heading run ends
    /// at the next newline or when the style leaves `Subheader`; the LAST heading
    /// in a turn wins, so a banner title printed earlier on its own line is
    /// overwritten by the real room heading.
    fn capture_heading(&mut self, style: GlkStyle, s: &str) {
        let is_sub = style == GlkStyle::Subheader;
        for ch in s.chars() {
            if ch == '\n' {
                if self.in_heading {
                    self.finalize_heading();
                    self.in_heading = false;
                }
                self.at_line_start = true;
                continue;
            }
            if is_sub {
                if self.at_line_start && !self.in_heading {
                    self.in_heading = true; // a heading begins only at line start
                }
                if self.in_heading {
                    self.heading_acc.push(ch);
                }
                // A `Subheader` run that began mid-line is an inline link → ignore.
            } else if self.in_heading {
                // Non-`Subheader` text on the heading's line ends the heading run.
                self.finalize_heading();
                self.in_heading = false;
            }
            self.at_line_start = false;
        }
    }

    /// Promote the accumulated `Subheader` text (if any, trimmed non-empty) to
    /// the last-heading slot and clear the accumulator.
    fn finalize_heading(&mut self) {
        let line = self.heading_acc.trim().to_string();
        self.heading_acc.clear();
        if !line.is_empty() {
            self.last_heading = Some(line);
        }
    }

    /// Return and clear the last `Subheader` room heading captured since the
    /// previous call. Drained once per turn, alongside `take_transcript`.
    pub fn take_room_heading(&mut self) -> Option<String> {
        if self.in_heading {
            self.finalize_heading(); // flush a heading with no trailing separator yet
            self.in_heading = false;
        }
        self.last_heading.take()
    }

    /// Project the recorded Glk state onto the neutral [`ScreenModel`].
    pub fn screen_model(&self) -> ScreenModel {
        // Build a (rect, node) pair for each laid-out leaf window, then assemble
        // the guillotine tree from the rects.
        let mut leaves: Vec<(GlkRect, WinNode)> = Vec::new();
        for &(id, ty, rect) in &self.layout {
            // A zero-area window (e.g. a collapsed/hidden graphics window a game
            // keeps around) is invisible, and a degenerate rect sitting on a
            // boundary defeats the guillotine reconstruction in `assemble` —
            // making it drop every other window. Skip it entirely.
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            let node = match ty {
                WinType::TextGrid => WinNode::Grid(self.grid_node(id, rect)),
                WinType::TextBuffer => WinNode::Buffer(self.buffer_node(id)),
                WinType::Pair => continue, // pair windows are never in the layout
                WinType::Graphics => {
                    let c = self.graphics.get(&id);
                    WinNode::Graphics(crate::engine::GraphicsWindow {
                        win: id,
                        canvas: c.map(|c| c.arc()).unwrap_or_else(|| std::sync::Arc::new(image::RgbaImage::new(1, 1))),
                        version: c.map(|c| c.version).unwrap_or(0),
                    })
                }
            };
            leaves.push((rect, node));
        }
        let root = assemble(&leaves);
        ScreenModel {
            root,
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
        }
    }

    fn grid_node(&self, id: u32, rect: GlkRect) -> GridWindow {
        let g = self.grids.get(&id);
        let cols = g.map(|g| g.width).unwrap_or(rect.width).max(rect.width) as u16;
        let rows = g.map(|g| g.height).unwrap_or(rect.height).max(rect.height) as u16;
        let mut cells = vec![GridCell::default(); cols as usize * rows as usize];
        if let Some(g) = g {
            for (&(r, c), &(ch, bits, fg, bg)) in &g.cells {
                if r < rows as u32 && c < cols as u32 {
                    cells[r as usize * cols as usize + c as usize] = GridCell { ch, style: bits, fg, bg };
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
        let (lines, runs, images) = buf.map(|b| log_to_lines(&b.log)).unwrap_or_default();
        let scroll = buf.map(|b| b.scroll).unwrap_or(0);
        BufferWindow { lines, runs, images, scroll, primary: false }
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

/// Split a buffer window's styled log into `(lines, per-line runs, per-line
/// image)`, merging adjacent same-style chars into one [`StyleRun`]. The three
/// vecs are always kept the same length: an image occupies its own logical
/// line (a fresh line is started before it if the current line has content,
/// and a fresh line is always started after it).
fn log_to_lines(
    log: &[BufElem],
) -> (Vec<String>, Vec<Vec<StyleRun>>, Vec<Option<crate::inline_image::InlineImage>>) {
    let mut lines: Vec<String> = vec![String::new()];
    let mut runs: Vec<Vec<StyleRun>> = vec![Vec::new()];
    let mut images: Vec<Option<crate::inline_image::InlineImage>> = vec![None];
    for elem in log {
        match elem {
            BufElem::Text { bits, fg, bg, text } => {
                for ch in text.chars() {
                    if ch == '\n' {
                        lines.push(String::new());
                        runs.push(Vec::new());
                        images.push(None);
                        continue;
                    }
                    let li = lines.len() - 1;
                    let col = lines[li].chars().count();
                    lines[li].push(ch);
                    // A run is emitted whenever any styling is active (bits or a colour).
                    if *bits != 0 || *fg != 0 || *bg != 0 {
                        let r = &mut runs[li];
                        match r.last_mut() {
                            Some(last)
                                if last.bits == *bits
                                    && last.fg == *fg
                                    && last.bg == *bg
                                    && last.end == col =>
                            {
                                last.end = col + 1
                            }
                            _ => r.push(StyleRun { start: col, end: col + 1, bits: *bits, fg: *fg, bg: *bg }),
                        }
                    }
                }
            }
            BufElem::Image(img) => {
                // An image occupies its own logical line: start a fresh line
                // before it if the current line already has content.
                if !lines.last().map(|l| l.is_empty()).unwrap_or(true) {
                    lines.push(String::new());
                    runs.push(Vec::new());
                    images.push(None);
                }
                if let Some(last) = images.last_mut() {
                    *last = Some(img.clone());
                }
                // Always start a fresh line after the image.
                lines.push(String::new());
                runs.push(Vec::new());
                images.push(None);
            }
        }
    }
    (lines, runs, images)
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
            WinType::Graphics => {}
        }
    }

    fn window_close(&mut self, id: u32) {
        self.grids.remove(&id);
        self.buffers.remove(&id);
        self.graphics.remove(&id);
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
        for &(id, ty, rect) in wins {
            if ty == WinType::Graphics {
                let (cw, ch) = (rect.width * self.char_px.0, rect.height * self.char_px.1);
                if let Some(c) = self.graphics.get_mut(&id) {
                    c.resize(cw, ch);
                }
            }
        }
    }

    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        self.put_text_attr(win, style, StyleColour::default(), s);
    }

    fn put_text_attr(&mut self, win: u32, style: GlkStyle, colour: StyleColour, s: &str) {
        if Some(win) == self.primary {
            self.capture_heading(style, s);
        }
        let (bits, fg, bg) = resolve_glk_colour(style, colour);
        let buf = self.buffers.entry(win).or_default();
        buf.log.push(BufElem::Text { bits, fg, bg, text: s.to_owned() });
    }

    fn grid_put(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, s: &str) {
        self.grid_put_attr(win, x, y, style, StyleColour::default(), s);
    }

    fn grid_put_attr(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, colour: StyleColour, s: &str) {
        let (bits, fg, bg) = resolve_glk_colour(style, colour);
        let g = self.grids.entry(win).or_default();
        for (i, ch) in s.chars().enumerate() {
            g.cells.insert((y, x + i as u32), (ch, bits, fg, bg));
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
        if let Some(c) = self.graphics.get_mut(&win) {
            let (w, h) = (c.img.width(), c.img.height());
            c.erase_rect(0, 0, w, h);
        }
        // A cleared primary window puts the cursor back at line start, so a
        // heading printed at the top of the fresh window is a valid line-start
        // heading. Reset the detector; discard any partial heading run whose
        // text was just wiped.
        if Some(win) == self.primary {
            self.at_line_start = true;
            self.in_heading = false;
            self.heading_acc.clear();
        }
    }

    fn flush(&mut self) {}

    fn char_pixels(&self) -> (u32, u32) {
        self.char_px
    }

    fn image_info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.picts.info(resnum)
    }

    fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .fill_rect(color, left, top, w, h);
    }

    fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .erase_rect(left, top, w, h);
    }

    fn graphics_set_background(&mut self, win: u32, color: u32) {
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .set_background(color);
    }

    fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) -> bool {
        // Buffer-window target: `x` is really the Glk imagealign flag; the image
        // flows inline with the window's text rather than onto a pixel canvas.
        if self.buffers.contains_key(&win) {
            let Some(src) = self.picts.image(resnum) else { return false };
            let img = crate::inline_image::InlineImage {
                pixels: std::sync::Arc::new(src.to_rgba8()),
                align: crate::inline_image::ImageAlign::from_glk(x as u32),
                scaled: scale,
            };
            if let Some(buf) = self.buffers.get_mut(&win) {
                buf.log.push(BufElem::Image(img));
            }
            return true;
        }
        // Graphics-window target: existing canvas path.
        let Some(src) = self.picts.image(resnum) else { return false };
        let (cw, ch) = self.canvas_size(win);
        self.graphics
            .entry(win)
            .or_insert_with(|| crate::graphics::Canvas::new(cw, ch))
            .draw_image(&src, x, y, scale);
        true
    }

    fn schannel_create(&mut self, rock: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannels.insert(id, SoundChannel { rock, volume: 0x10000 });
        id
    }
    fn schannel_destroy(&mut self, chan: u32) {
        self.schannels.remove(&chan);
        self.sound_ops.push(crate::session::SchannelOp::Destroy { chan });
    }
    fn schannel_iterate(&mut self, chan: u32) -> (u32, u32) {
        let next = if chan == 0 {
            self.schannels.keys().next().copied()
        } else {
            self.schannels.range((chan + 1)..).next().map(|(k, _)| *k)
        };
        match next {
            Some(id) => (id, self.schannels.get(&id).map(|c| c.rock).unwrap_or(0)),
            None => (0, 0),
        }
    }
    fn schannel_get_rock(&mut self, chan: u32) -> u32 {
        self.schannels.get(&chan).map(|c| c.rock).unwrap_or(0)
    }
    fn schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32 {
        let volume = self.schannels.get(&chan).map(|c| c.volume).unwrap_or(0x10000);
        self.sound_ops.push(crate::session::SchannelOp::Play { chan, snd, repeats, notify, volume });
        1
    }
    fn schannel_stop(&mut self, chan: u32) {
        self.sound_ops.push(crate::session::SchannelOp::Stop { chan });
    }
    fn schannel_set_volume(&mut self, chan: u32, vol: u32) {
        if let Some(c) = self.schannels.get_mut(&chan) {
            c.volume = vol;
        }
        self.sound_ops.push(crate::session::SchannelOp::SetVolume { chan, vol });
    }

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
    fn non_primary_buffer_carries_inline_image() {
        // A resolvable Pict needs a Blorb, which the test harness lacks (see
        // `take_transcript_elems_coalesces_text_and_keeps_image_between`), so
        // seed the non-primary buffer's log directly: Text, Image, Text.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // primary
        glk.window_open(2, WinType::TextBuffer); // non-primary
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
        };
        let log = &mut glk.buffers.get_mut(&2).unwrap().log;
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, text: "a\n".into() });
        log.push(BufElem::Image(dummy));
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, text: "b".into() });

        let bw = glk.buffer_node(2);
        assert_eq!(bw.lines.len(), bw.runs.len());
        assert_eq!(bw.lines.len(), bw.images.len());
        let img_idx = bw.images.iter().position(|o| o.is_some()).expect("image line present");
        assert_eq!(bw.lines[img_idx], "", "image occupies its own logical line");
        assert!(img_idx > 0 && img_idx < bw.lines.len() - 1, "image line sits between a and b");
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

    #[test]
    fn resolve_glk_colour_packs_24bit_and_reverse() {
        use zvm::screen::ZColour;
        // fg/bg become packed True24; the reverse hint sets bit 0x01.
        let sc = StyleColour { fg: Some(0x00AA_BBCC), bg: Some(0x0011_2233), reverse: true };
        let (bits, fg, bg) = resolve_glk_colour(GlkStyle::Normal, sc);
        assert_eq!(bits, 0x01);
        assert_eq!(fg, crate::state::pack_zcolour(ZColour::True24(0x00AA_BBCC)));
        assert_eq!(bg, crate::state::pack_zcolour(ZColour::True24(0x0011_2233)));
        // No hints set: only the style-class bits, no colour. (The honor gate is
        // applied at render time, not here.)
        assert_eq!(resolve_glk_colour(GlkStyle::Header, StyleColour::default()), (0x02, 0, 0));
    }

    #[test]
    fn buffer_colour_flows_to_transcript() {
        use zvm::screen::ZColour;
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        let red = StyleColour { fg: Some(0x00FF_0000), bg: None, reverse: false };
        glk.put_text_attr(1, GlkStyle::Normal, red, "hi");
        let (text, chunks) = glk.take_transcript();
        assert_eq!(text, "hi");
        assert_eq!(chunks.len(), 1);
        let (n, _bits, fg, bg) = chunks[0];
        assert_eq!(n, 2);
        assert_eq!(fg, ZColour::True24(0x00FF_0000), "24-bit fg carried losslessly");
        assert_eq!(bg, ZColour::Default);
    }

    #[test]
    fn grid_colour_flows_to_cells() {
        use zvm::screen::ZColour;
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextGrid);
        glk.window_layout(&[(1, WinType::TextGrid, rect(0, 0, 10, 2))]);
        let blue_on_white = StyleColour { fg: Some(0x0000_00FF), bg: Some(0x00FF_FFFF), reverse: false };
        glk.grid_put_attr(1, 0, 0, GlkStyle::Normal, blue_on_white, "X");
        let cell = glk.screen_model().grid().unwrap().cell(1, 1);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, crate::state::pack_zcolour(ZColour::True24(0x0000_00FF)));
        assert_eq!(cell.bg, crate::state::pack_zcolour(ZColour::True24(0x00FF_FFFF)));
    }

    #[test]
    fn appglk_graphics_fill_composites_into_canvas() {
        let mut g = AppGlk::with_graphics(80, 24, (2, 2), crate::graphics::PictSource::new(None));
        // Simulate a laid-out graphics window id=1 occupying 4x4 cells → 8x8 px.
        g.window_open(1, gvm::glk::WinType::Graphics);
        g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 4, height: 4 })]);
        g.graphics_fill_rect(1, 0x00FF_0000, 0, 0, 8, 8);
        let canvas = g.graphics.get(&1).unwrap();
        assert_eq!(canvas.img.dimensions(), (8, 8));
        assert_eq!(canvas.img.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF]);
    }

    #[test]
    fn screen_model_emits_graphics_leaf() {
        let mut g = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(None));
        g.window_open(1, gvm::glk::WinType::Graphics);
        g.window_layout(&[(1, gvm::glk::WinType::Graphics, gvm::glk::Rect { left: 0, top: 0, width: 10, height: 4 })]);
        g.graphics_fill_rect(1, 0x00FF00, 0, 0, 10, 4);
        let model = g.screen_model();
        // The tree's single leaf is a Graphics node for window 1.
        fn find_graphics(n: &crate::engine::WinNode) -> bool {
            match n {
                crate::engine::WinNode::Graphics(_) => true,
                crate::engine::WinNode::Pair { first, second, .. } => find_graphics(first) || find_graphics(second),
                _ => false,
            }
        }
        assert!(find_graphics(&model.root), "graphics window should appear as a Graphics leaf");
    }

    /// Regression (CounterfeitMonkey layout): a game keeps a zero-height
    /// graphics window (win 4) around alongside a real graphics window (win 6,
    /// the image), a status grid, and the text buffer. The degenerate rect must
    /// not defeat the guillotine reconstruction — the real graphics window and
    /// the buffer must both survive.
    #[test]
    fn screen_model_survives_zero_area_window() {
        use gvm::glk::{Rect, WinType};
        let mut g = AppGlk::with_graphics(80, 24, (9, 19), crate::graphics::PictSource::new(None));
        g.window_open(1, WinType::TextBuffer);
        g.window_open(2, WinType::TextGrid);
        g.window_open(4, WinType::Graphics);
        g.window_open(6, WinType::Graphics);
        g.window_layout(&[
            (1, WinType::TextBuffer, Rect { left: 40, top: 1, width: 40, height: 23 }),
            (2, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }),
            (4, WinType::Graphics, Rect { left: 0, top: 24, width: 80, height: 0 }), // collapsed
            (6, WinType::Graphics, Rect { left: 0, top: 1, width: 40, height: 23 }), // the image
        ]);
        g.graphics_fill_rect(6, 0x00FF00, 0, 0, 40, 23);
        let model = g.screen_model();

        fn collect(n: &crate::engine::WinNode, out: &mut Vec<(&'static str, u32)>) {
            match n {
                crate::engine::WinNode::Graphics(gw) => out.push(("graphics", gw.win)),
                crate::engine::WinNode::Buffer(_) => out.push(("buffer", 0)),
                crate::engine::WinNode::Grid(_) => out.push(("grid", 0)),
                crate::engine::WinNode::Pair { first, second, .. } => {
                    collect(first, out);
                    collect(second, out);
                }
                crate::engine::WinNode::Blank => {}
            }
        }
        let mut leaves = Vec::new();
        collect(&model.root, &mut leaves);
        assert!(
            leaves.iter().any(|&(k, w)| k == "graphics" && w == 6),
            "the real graphics window (win 6) must survive; got {leaves:?}"
        );
        assert!(leaves.iter().any(|&(k, _)| k == "buffer"), "the text buffer must survive; got {leaves:?}");
        assert!(leaves.iter().any(|&(k, _)| k == "grid"), "the status grid must survive; got {leaves:?}");
    }

    #[test]
    fn image_draw_to_buffer_window_records_image_elem() {
        // No Blorb is registered (`PictSource::new(None)`), so the draw is a
        // silent no-op — this asserts the routing path (surrounding Text order
        // survives a buffer-targeted `graphics_draw_image`), not image presence.
        // Resolvable-image coverage comes via Task 5's `glulx_session` test.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer); // primary buffer
        glk.put_text(1, GlkStyle::Normal, "before\n");
        glk.graphics_draw_image(1, /*resnum*/ 0, /*imagealign*/ 1, 0, None);
        glk.put_text(1, GlkStyle::Normal, "after");
        let elems = glk.take_transcript_elems();
        let kinds: Vec<&str> = elems
            .iter()
            .map(|e| match e {
                crate::session::TranscriptElem::Text { .. } => "T",
                crate::session::TranscriptElem::Image(_) => "I",
            })
            .collect();
        assert_eq!(kinds.first().copied(), Some("T"));
        assert_eq!(kinds.last().copied(), Some("T"));
    }

    #[test]
    fn take_transcript_elems_coalesces_text_and_keeps_image_between() {
        // Seed the primary buffer log directly (a resolvable Pict needs a Blorb,
        // which the test harness lacks): Text, Text (different style bits), Image,
        // Text. take_transcript_elems must coalesce the two leading Text runs into
        // ONE element carrying TWO run-chunks, keep the Image between, and flush
        // the trailing Text as a final element.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        let pid = glk.primary.expect("primary open");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(3, 3)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
        };
        let log = &mut glk.buffers.get_mut(&pid).unwrap().log;
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, text: "foo".into() });
        log.push(BufElem::Text { bits: 0x02, fg: 0, bg: 0, text: "bar".into() });
        log.push(BufElem::Image(dummy));
        log.push(BufElem::Text { bits: 0, fg: 0, bg: 0, text: "baz".into() });

        let elems = glk.take_transcript_elems();
        assert_eq!(elems.len(), 3, "coalesced Text, Image, trailing Text");
        match &elems[0] {
            crate::session::TranscriptElem::Text { text, runs } => {
                assert_eq!(text, "foobar", "the two leading runs coalesce into one element");
                assert_eq!(runs.len(), 2, "each source run kept as its own chunk, in order");
                assert_eq!(runs[0], (3, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default));
                assert_eq!(runs[1].0, 3);
                assert_eq!(runs[1].1, 0x02, "second chunk carries the different style bits");
            }
            _ => panic!("elems[0] must be Text"),
        }
        assert!(matches!(&elems[1], crate::session::TranscriptElem::Image(_)), "image sits between");
        match &elems[2] {
            crate::session::TranscriptElem::Text { text, .. } => assert_eq!(text, "baz"),
            _ => panic!("trailing Text must flush as a final element"),
        }
    }

    #[test]
    fn image_draw_to_graphics_window_still_hits_canvas() {
        // A graphics-window draw must NOT push a buffer image elem; it updates
        // a Canvas via the existing graphics path.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(5, WinType::Graphics);
        glk.graphics_draw_image(5, 0, 10, 10, None);
        // No primary buffer is open → elems empty.
        assert!(glk.take_transcript_elems().is_empty());
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn graphics_draw_image_reports_false_when_pict_missing() {
        // SQ-0175 part A: with no Blorb registered, `resnum` never resolves,
        // so the backend must report false rather than always claiming success.
        let mut glk = AppGlk::new(80, 24);
        glk.window_open(1, WinType::TextBuffer);
        assert!(!glk.graphics_draw_image(1, 0, 1, 0, None), "buffer window, missing image");

        glk.window_open(5, WinType::Graphics);
        assert!(!glk.graphics_draw_image(5, 0, 10, 10, None), "graphics window, missing image");
    }

    #[test]
    fn graphics_draw_image_reports_true_when_pict_resolves() {
        // A resnum backed by a real, decodable Pict in the Blorb must report
        // true on both the buffer-window and graphics-window draw paths.
        let blorb = crate::graphics::test_blorb_with_pict(1, &png_bytes());
        let mut glk = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(Some(blorb)));
        glk.window_open(1, WinType::TextBuffer);
        assert!(glk.graphics_draw_image(1, /*resnum*/ 1, /*imagealign*/ 1, 0, None), "buffer window, resolvable image");

        let blorb2 = crate::graphics::test_blorb_with_pict(1, &png_bytes());
        let mut glk2 = AppGlk::with_graphics(80, 24, (1, 1), crate::graphics::PictSource::new(Some(blorb2)));
        glk2.window_open(5, WinType::Graphics);
        assert!(glk2.graphics_draw_image(5, /*resnum*/ 1, 10, 10, None), "graphics window, resolvable image");
    }

    #[test]
    fn appglk_schannel_create_allocates_refs_and_rocks() {
        use gvm::glk::GlkBackend;
        let mut g = AppGlk::new(80, 24);
        let a = g.schannel_create(11);
        let b = g.schannel_create(22);
        assert_ne!(a, 0, "a created channel has a nonzero ref");
        assert_ne!(a, b, "distinct channels get distinct refs");
        assert_eq!(g.schannel_get_rock(a), 11);
        assert_eq!(g.schannel_get_rock(b), 22);
        assert_eq!(g.schannel_get_rock(9999), 0, "unknown channel → rock 0");
        // iterate: 0 → first, then next, then 0 at the end.
        let (first, first_rock) = g.schannel_iterate(0);
        assert_eq!((first, first_rock), (a, 11));
        let (second, second_rock) = g.schannel_iterate(first);
        assert_eq!((second, second_rock), (b, 22));
        assert_eq!(g.schannel_iterate(second), (0, 0), "past the end → (0,0)");
    }

    #[test]
    fn appglk_schannel_ops_buffer_in_order_with_volume_snapshot() {
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0);
        g.schannel_set_volume(c, 0x8000);          // half volume
        g.schannel_play(c, 5, 3, 9);               // play_ext(chan, snd, repeats, notify)
        g.schannel_stop(c);
        g.schannel_destroy(c);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![
            SchannelOp::SetVolume { chan: c, vol: 0x8000 },
            SchannelOp::Play { chan: c, snd: 5, repeats: 3, notify: 9, volume: 0x8000 },
            SchannelOp::Stop { chan: c },
            SchannelOp::Destroy { chan: c },
        ]);
        assert!(g.take_sound_ops().is_empty(), "draining clears the buffer");
        assert_eq!(g.schannel_get_rock(c), 0, "destroy removed the channel");
    }

    #[test]
    fn appglk_play_snapshots_default_full_volume() {
        use gvm::glk::GlkBackend;
        use crate::session::SchannelOp;
        let mut g = AppGlk::new(80, 24);
        let c = g.schannel_create(0); // no set_volume → default 0x10000 (Glk full)
        g.schannel_play(c, 1, 1, 0);
        let ops = g.take_sound_ops();
        assert_eq!(ops, vec![SchannelOp::Play { chan: c, snd: 1, repeats: 1, notify: 0, volume: 0x10000 }]);
    }
}

#[cfg(test)]
mod heading_tests {
    use super::*;
    use gvm::glk::{GlkBackend, GlkStyle, Rect as GlkRect, WinType};

    fn primary_backend() -> AppGlk {
        let mut b = AppGlk::new(80, 24);
        b.window_open(1, WinType::TextBuffer);
        b.window_layout(&[(1, WinType::TextBuffer, GlkRect { left: 0, top: 0, width: 80, height: 24 })]);
        b
    }

    // Feed a run via the colourless trait entry (delegates to put_text_attr).
    fn put(b: &mut AppGlk, style: GlkStyle, s: &str) {
        b.put_text(1, style, s);
    }

    #[test]
    fn subheader_line_is_the_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Studio Apartment\n");
        put(&mut b, GlkStyle::Normal, "You climb out of bed.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Studio Apartment"));
        // Drained: a second call with no new heading is None.
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn last_subheader_wins_over_banner_title() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Subheader, "Coloratura");
        put(&mut b, GlkStyle::Normal, " by lynnea glasser\n");
        put(&mut b, GlkStyle::Subheader, "Inside the Cellarium");
        put(&mut b, GlkStyle::Normal, "A white structure.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Inside the Cellarium"));
    }

    #[test]
    fn emphasized_and_header_are_not_headings() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Header, "Superluminal Vagrant Twin\n");
        put(&mut b, GlkStyle::Emphasized, "Knock.");
        put(&mut b, GlkStyle::Normal, "Prose.\n");
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn menu_only_normal_text_has_no_heading() {
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "1) Yes\n2) No\n");
        assert_eq!(b.take_room_heading(), None);
    }

    #[test]
    fn heading_char_by_char_runs_accumulate() {
        // Games often emit one glk_put_char per character.
        let mut b = primary_backend();
        for ch in "War Chest".chars() {
            put(&mut b, GlkStyle::Subheader, &ch.to_string());
        }
        put(&mut b, GlkStyle::Normal, "\nThe battle.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("War Chest"));
    }

    #[test]
    fn mid_line_subheader_is_an_inline_link_not_a_room() {
        // Superluminal Vagrant Twin renders command hints ("credits", "land") as
        // Subheader mid-line, before and after the real room heading. Only the
        // line-start heading counts; a trailing inline link must NOT overwrite it.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "(Type ");
        put(&mut b, GlkStyle::Subheader, "credits"); // inline link, mid-line
        put(&mut b, GlkStyle::Normal, " to learn who made this.)\n\n");
        put(&mut b, GlkStyle::Subheader, "Orbiting Boony"); // room heading, line start
        put(&mut b, GlkStyle::Normal, "\nA grey world. You going to ");
        put(&mut b, GlkStyle::Subheader, "land"); // inline link, mid-line
        put(&mut b, GlkStyle::Normal, " soon?\n\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Orbiting Boony"));
    }

    #[test]
    fn window_clear_resets_line_start_for_next_heading() {
        // A game that clears the screen mid-line and then prints the room title
        // with no leading newline: the clear returns the cursor to line start, so
        // the heading must still be recognized.
        let mut b = primary_backend();
        put(&mut b, GlkStyle::Normal, "loading"); // leaves at_line_start = false
        b.window_clear(1);
        put(&mut b, GlkStyle::Subheader, "Grand Hall"); // top of cleared window
        put(&mut b, GlkStyle::Normal, "\nA vast chamber.\n");
        assert_eq!(b.take_room_heading().as_deref(), Some("Grand Hall"));
    }
}
