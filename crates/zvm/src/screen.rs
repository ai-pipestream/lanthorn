// Screen model — ZMSD §7, §8, §11.
//
// `ScreenState` tracks window layout and text attributes the host needs to
// render.  `StatusLine` is the v3 status bar computed on demand from globals.
// `StreamState` manages output-stream routing including stream-3 memory
// redirection.
//
// Stream-3 can nest up to 16 deep (ZMSD §7.1.2.5).  Each frame holds a
// table base address; the first word of the table is the byte-count written.

use crate::memory::Memory;
use crate::objects;

// ---------------------------------------------------------------------------
// Status line
// ---------------------------------------------------------------------------

/// The right-hand portion of a v3 status line (ZMSD §8.2.3.1).
/// Flags1 bit 1: 0 = score/turns, 1 = time (hours:minutes).
#[derive(Debug, PartialEq)]
pub enum StatusRight {
    ScoreTurns { score: i16, turns: u16 },
    Time { hours: u8, minutes: u8 },
}

/// A fully computed v3 status line (location name + right field).
#[derive(Debug, PartialEq)]
pub struct StatusLine {
    pub location: String,
    pub right: StatusRight,
}

// ---------------------------------------------------------------------------
// Screen state (window model)
// ---------------------------------------------------------------------------

/// A Z-machine colour channel value (logical, pre-reverse-swap).
///
/// Transient display state — NOT serialised into Quetzal saves (like
/// `current_font`). The host resolves `Default` to the terminal/scheme
/// default, `Standard(2..=9)` to the scheme palette, `Standard(10..=12)` to
/// fixed grey RGB, `True` to an exact 15-bit RGB colour (Z-machine
/// `set_true_colour`), and `True24` to an exact 24-bit `0xRRGGBB` colour (used
/// by the Glulx host, whose Glk stylehint colours are 24-bit — carried at full
/// fidelity rather than downsampled to 15-bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum ZColour {
    #[default]
    Default,
    Standard(u8),
    True(u16),
    True24(u32),
}

/// Expand a 15-bit RGB (0bbbbbgggggrrrrr) to 8-bit `(r, g, b)`. Shared by the
/// CLI (SGR) and app (ratatui) renderers so the expansion is defined once.
pub fn rgb15_to_888(v: u16) -> (u8, u8, u8) {
    let exp = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
    (exp(v & 0x1F), exp((v >> 5) & 0x1F), exp((v >> 10) & 0x1F))
}

/// Fixed RGB for the v6 greys (Standard 10/11/12). Defined once here so both
/// renderers agree. Any other value falls back to dark grey (12).
pub fn grey_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        10 => (0xB0, 0xB0, 0xB0),
        11 => (0x80, 0x80, 0x80),
        _ => (0x50, 0x50, 0x50), // 12
    }
}

/// One character cell in the upper window.
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}
impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', style: 0, fg: ZColour::Default, bg: ZColour::Default }
    }
}

/// Upper (status) window character grid.
#[derive(Debug, Default, Clone)]
pub struct UpperWindow {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}
impl UpperWindow {
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![Cell::default(); rows as usize * cols as usize];
    }
    pub fn clear(&mut self) {
        for c in &mut self.cells {
            *c = Cell::default();
        }
    }
    /// Grow the grid to at least `new_rows` rows, preserving existing content.
    /// No-op when the grid is already tall enough. Used when a game draws in the
    /// upper window at rows beyond the current split height (Frotz keeps such
    /// writes on screen instead of clipping them to the split).
    pub fn grow_rows(&mut self, new_rows: u16) {
        if new_rows <= self.rows {
            return;
        }
        // Cells are row-major; appending blank cells adds new rows at the bottom
        // without disturbing existing rows.
        self.cells
            .resize(new_rows as usize * self.cols as usize, Cell::default());
        self.rows = new_rows;
    }
    /// Scroll the grid vertically by whole rows (used by `scroll_window`,
    /// EXT:0x14, quantized to the character grid): positive shifts content
    /// forward/up (drops the top `rows`, appends blank rows at the bottom);
    /// negative shifts backward/down (drops the bottom `rows`, inserts blank
    /// rows at the top). `rows` at or beyond the grid's extent clears it.
    pub fn scroll_rows(&mut self, rows: i16) {
        if rows == 0 || self.rows == 0 {
            return;
        }
        let total = self.rows as usize;
        let cols = self.cols as usize;
        let n = (rows.unsigned_abs() as usize).min(total);
        if n == total {
            self.clear();
            return;
        }
        if rows > 0 {
            self.cells.drain(0..n * cols);
            self.cells.resize(total * cols, Cell::default());
        } else {
            self.cells.truncate((total - n) * cols);
            self.cells.splice(0..0, vec![Cell::default(); n * cols]);
        }
    }
    fn idx(&self, row: u16, col: u16) -> Option<usize> {
        if row == 0 || col == 0 || row > self.rows || col > self.cols {
            return None;
        }
        Some(((row - 1) as usize) * self.cols as usize + (col - 1) as usize)
    }
    pub fn cell(&self, row: u16, col: u16) -> Cell {
        self.idx(row, col)
            .and_then(|i| self.cells.get(i).copied())
            .unwrap_or_default()
    }
    pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8, fg: ZColour, bg: ZColour) {
        if let Some(i) = self.idx(row, col) {
            if let Some(c) = self.cells.get_mut(i) {
                *c = Cell { ch, style, fg, bg };
            }
        }
    }
}

/// One v6 window; its fields ARE the ZMSD window-property array (index =
/// property number, ZMSD 1.1 §8.8.3.2).
#[derive(Debug, Clone, Default)]
pub struct ZWindow {
    pub y_coord: u16,          // prop 0  (pixels)
    pub x_coord: u16,          // prop 1
    pub y_size: u16,           // prop 2  (height, pixels)
    pub x_size: u16,           // prop 3  (width, pixels)
    /// Cursor in UNITS (pixels), 1-based within the window (ZMSD §8.8.3.2 —
    /// window props are measured in units, so `get_wind_prop` 4/5 read these
    /// verbatim). The char-cell the grid writes at derives as `(px-1)/font + 1`.
    pub y_cursor: u16,         // prop 4  (pixels)
    pub x_cursor: u16,         // prop 5  (pixels)
    pub left_margin: u16,      // prop 6
    pub right_margin: u16,     // prop 7
    pub interrupt_routine: u16,// prop 8
    pub interrupt_countdown: u16, // prop 9
    pub text_style: u16,       // prop 10
    pub colour_data: u16,      // prop 11 (high byte bg, low byte fg — ZMSD)
    pub font_number: u16,      // prop 12
    pub font_size: u16,        // prop 13 (high byte height, low byte width)
    pub attributes: u16,       // prop 14 (bit0 wrap, bit1 scroll, bit2 copy-to-transcript, bit3 buffered)
    pub line_count: u16,       // prop 15
    /// Character grid for this window (grid windows 1–7). Window 0 scrolls (buffered),
    /// its text goes to the transcript stream, not a grid.
    pub grid: UpperWindow,
    pub fg: ZColour,
    pub bg: ZColour,
    /// Pixel-positioned text runs (grid windows 1–7): each print records the
    /// exact 1-based pixel position it painted at, so a pixel-faithful raster
    /// can draw text where the game put it (e.g. Zork Zero's status text at
    /// rows 6/14, ON the banner ribbons) instead of snapping to the char grid.
    /// The char grid above remains the cell-mode fallback.
    pub texts: Vec<V6Text>,
}

/// One pixel-positioned text run in a v6 grid window: `(y, x)` are the 1-based
/// **screen-absolute** pixel coords of the run's first glyph's top-left,
/// captured at paint time. v6 text is PAINT — once drawn, pixels stay where
/// they were put regardless of later `move_window`/`window_size` calls
/// ("window_size does not change the current display", ZMSD §15; Shogun
/// shrinks its menu window to a 1-px caret AFTER printing the menu items).
/// A run is only removed or trimmed by later paint over the same pixels
/// ([`V6Windows::paint_run`]) or an erase ([`V6Windows::erase_screen_rect`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6Text {
    pub y: u16,
    pub x: u16,
    pub text: String,
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}

impl V6Text {
    /// Pixel width of this run (fixed-cell font).
    fn px_w(&self) -> u32 {
        self.text.chars().count() as u32 * V6_FONT_WIDTH as u32
    }
}

impl ZWindow {
    /// Read property `n` (0–15, ZMSD 1.1 §8.8.3.2). Out-of-range → 0.
    pub fn get_prop(&self, n: u16) -> u16 {
        match n {
            0 => self.y_coord,
            1 => self.x_coord,
            2 => self.y_size,
            3 => self.x_size,
            4 => self.y_cursor,
            5 => self.x_cursor,
            6 => self.left_margin,
            7 => self.right_margin,
            8 => self.interrupt_routine,
            9 => self.interrupt_countdown,
            10 => self.text_style,
            11 => self.colour_data,
            12 => self.font_number,
            13 => self.font_size,
            14 => self.attributes,
            15 => self.line_count,
            _ => 0,
        }
    }
    /// Write property `n` (0–15, ZMSD 1.1 §8.8.3.2). Out-of-range → ignored.
    pub fn put_prop(&mut self, n: u16, v: u16) {
        match n {
            0 => self.y_coord = v,
            1 => self.x_coord = v,
            2 => self.y_size = v,
            3 => self.x_size = v,
            4 => self.y_cursor = v,
            5 => self.x_cursor = v,
            6 => self.left_margin = v,
            7 => self.right_margin = v,
            8 => self.interrupt_routine = v,
            9 => self.interrupt_countdown = v,
            10 => self.text_style = v,
            11 => self.colour_data = v,
            12 => self.font_number = v,
            13 => self.font_size = v,
            14 => self.attributes = v,
            15 => self.line_count = v,
            _ => {}
        }
    }

    /// Scroll this grid window's content by `pixels` (ZMSD 1.1 §15
    /// `scroll_window`: "Scrolls the given window by the given number of
    /// pixels (a negative value scrolls backwards, i.e., down) writing in
    /// blank (background colour) pixels in the new lines."). Shifts each
    /// pixel-positioned text run's `y` by `-pixels` (dropping runs that land
    /// fully outside the window's visible height `[1, y_size]`), and shifts
    /// the cell-grid fallback by whole rows (`pixels / V6_FONT_HEIGHT`,
    /// truncated toward zero).
    pub fn scroll_pixels(&mut self, pixels: i16) {
        // Runs are screen-absolute: the scroll region is this window's CURRENT
        // screen rect; runs shift within it and drop when they leave it.
        let top = self.y_coord.max(1) as i32;
        let bottom_edge = top + self.y_size.max(1) as i32 - 1;
        let delta = pixels as i32;
        self.texts.retain_mut(|t| {
            let new_y = t.y as i32 - delta;
            let bottom = new_y + V6_FONT_HEIGHT as i32 - 1;
            if bottom < top || new_y > bottom_edge {
                false
            } else {
                t.y = new_y.clamp(1, u16::MAX as i32) as u16;
                true
            }
        });
        let rows = pixels / V6_FONT_HEIGHT as i16;
        self.grid.scroll_rows(rows);
    }
}

/// The v6 8-window table (ZMSD §8.4): windows 0–7, addressed in pixels.
#[derive(Debug, Clone, Default)]
pub struct V6Windows {
    pub windows: [ZWindow; 8],
    pub current: u8, // 0–7
}

/// Trim `run` against the screen rect `(top, left)..(top+h, left+w)` in pixels:
/// drop it entirely, keep it, or split it into up-to-two remnants. A glyph is
/// erased when its 8×8 cell intersects the rect at all (paint replaces whole
/// glyphs; sub-glyph residue can't be represented as text).
fn trim_run_against_rect(run: V6Text, top: i32, left: i32, h: i32, w: i32) -> Vec<V6Text> {
    let ry = run.y as i32;
    // Vertical band overlap?
    if ry + V6_FONT_HEIGHT as i32 <= top || ry >= top + h {
        return vec![run];
    }
    let fw = V6_FONT_WIDTH as i32;
    let rx = run.x as i32;
    let n = run.text.chars().count() as i32;
    if rx + n * fw <= left || rx >= left + w {
        return vec![run];
    }
    // Glyph i covers [rx + i*fw, rx + (i+1)*fw); erased iff it intersects
    // [left, left+w). Chars form one contiguous erased span, leaving at most a
    // left and a right remnant.
    let first_erased = ((left - rx).div_euclid(fw)).max(0); // first glyph whose cell intersects
    let last_erased = (((left + w - 1) - rx).div_euclid(fw)).min(n - 1);
    if first_erased > last_erased {
        return vec![run];
    }
    let chars: Vec<char> = run.text.chars().collect();
    let mut out = Vec::new();
    if first_erased > 0 {
        out.push(V6Text {
            y: run.y,
            x: run.x,
            text: chars[..first_erased as usize].iter().collect(),
            style: run.style,
            fg: run.fg,
            bg: run.bg,
        });
    }
    if (last_erased as usize) + 1 < chars.len() {
        out.push(V6Text {
            y: run.y,
            x: (rx + (last_erased + 1) * fw) as u16,
            text: chars[last_erased as usize + 1..].iter().collect(),
            style: run.style,
            fg: run.fg,
            bg: run.bg,
        });
    }
    out
}

impl V6Windows {
    /// Paint one text run: erase whatever earlier runs its pixels cover (in
    /// EVERY window — the screen is one shared raster), then store it on
    /// window `win`. This is what keeps overprinted status lines legible:
    /// Shogun re-prints its location/score at the same pixel cursor each turn
    /// and relies on the new glyphs replacing the old ones.
    ///
    /// A glyph only erases underneath where it deposits OPAQUE pixels: any
    /// glyph over an opaque background paints its whole cell, but a SPACE on a
    /// transparent background paints nothing — Shogun pads its status fields
    /// with such spaces, and erasing under them would eat the neighbouring
    /// labels. (Non-space ink on transparent bg is approximated as covering
    /// its cell: latest-wins per cell, since a text-run model can't
    /// overstrike.)
    pub fn paint_run(&mut self, win: usize, run: V6Text) {
        if run.text.is_empty() {
            return;
        }
        // Inherited colours (Default / Standard "current"/"default") are
        // transparent; a real chosen colour paints an opaque block.
        let bg_opaque = !matches!(run.bg, ZColour::Default | ZColour::Standard(0) | ZColour::Standard(1));
        // A run that is ENTIRELY blanks is a CLEARING run: the game printing
        // spaces to wipe a region. Zork Zero blanks the old, LONGER location
        // name ("Banquet Hall") with such runs before repainting the shorter
        // "Great Hall" — those blanks must erase the covered glyphs, or the old
        // tail survives as "Great Hall" + a stale "ll" ("Great Hallll", SQ-0498).
        // A space WITHIN a mixed run stays non-erasing: those are field-padding
        // gaps (Shogun pads its status fields with spaces) and erasing under
        // them would eat a neighbouring label painted in the same row.
        let clearing = run.text.chars().all(|c| c == ' ');
        let fw = V6_FONT_WIDTH as i32;
        let mut seg_start: Option<i32> = None; // char index of current erasing segment
        let chars: Vec<char> = run.text.chars().collect();
        for i in 0..=chars.len() {
            let erases = i < chars.len() && (bg_opaque || clearing || chars[i] != ' ');
            match (erases, seg_start) {
                (true, None) => seg_start = Some(i as i32),
                (false, Some(s)) => {
                    self.erase_screen_rect(
                        run.y as i32,
                        run.x as i32 + s * fw,
                        V6_FONT_HEIGHT as i32,
                        (i as i32 - s) * fw,
                    );
                    seg_start = None;
                }
                _ => {}
            }
        }
        if let Some(w) = self.windows.get_mut(win) {
            w.texts.push(run);
        }
    }

    /// Erase a screen-absolute pixel rect: every stored run (any window) loses
    /// the glyphs the rect covers. Backs both `paint_run` and `erase_window`
    /// (which erases the target window's CURRENT screen rect — Shogun erases
    /// its 1-px caret window without disturbing the menu items painted around
    /// it earlier).
    pub fn erase_screen_rect(&mut self, top: i32, left: i32, h: i32, w: i32) {
        if h <= 0 || w <= 0 {
            return;
        }
        for win in self.windows.iter_mut() {
            if win.texts.iter().any(|t| {
                let ty = t.y as i32;
                let tx = t.x as i32;
                ty + (V6_FONT_HEIGHT as i32) > top
                    && ty < top + h
                    && tx + (t.px_w() as i32) > left
                    && tx < left + w
            }) {
                let old = std::mem::take(&mut win.texts);
                win.texts =
                    old.into_iter().flat_map(|t| trim_run_against_rect(t, top, left, h, w)).collect();
            }
        }
    }
}

/// Structured screen model the host (TUI etc.) reads to render.
///
/// For v3 the host derives the status line by calling `Machine::status_line()`.
/// For v4+ the host reads `upper_window_rows`, `current_window`, `text_style`,
/// and `cursor` to manage windows.
#[derive(Debug, Clone)]
pub struct ScreenState {
    /// Number of rows in the upper (status) window; 0 means no upper window.
    pub upper_window_rows: u16,
    /// Currently selected window: 0 = lower, 1 = upper.
    pub current_window: u8,
    /// Current text-style bitmask (ZMSD §8.7.2):
    ///   value 1 = reverse video, 2 = bold, 4 = italic, 8 = fixed-pitch (ZMSD §8.7.2).
    pub text_style: u8,
    /// Cursor position in the upper window (1-based row, col).
    pub cursor_row: u16,
    pub cursor_col: u16,
    /// Whether output should be buffered (lower window).
    pub buffer_mode: bool,
    /// Whether `show_status` (v3 0OP:0x0C) was requested since last read.
    pub show_status_requested: bool,
    /// Whether the lower window should be cleared (set by `erase_window` 0/-1/-2;
    /// ZMSD §8.7.3). The engine does not model the scrolling lower window's
    /// contents, so it records the request here for the host to drain and act on.
    pub erase_lower_requested: bool,
    /// Upper window character grid (v4+).
    pub upper: UpperWindow,
    /// Active font number (ZMSD §16): 1 = normal (default), 3 = character-graphics.
    /// This is transient display state — NOT serialised into Quetzal saves.
    pub current_font: u8,
    /// Current logical foreground/background colour (ZMSD §8.3). Transient
    /// display state — NOT serialised into Quetzal saves.
    pub current_fg: ZColour,
    pub current_bg: ZColour,
    /// The v6 8-window table; `Some` only when the loaded story is v6
    /// (v1–5/v7/v8 keep the classic 2-window model above and this stays `None`).
    pub v6: Option<V6Windows>,
}

impl Default for ScreenState {
    fn default() -> Self {
        ScreenState {
            upper_window_rows: 0,
            current_window: 0,
            text_style: 0,
            cursor_row: 0,
            cursor_col: 0,
            // ZMSD §8.7.2.5: the lower window is buffered (word-wrapped) by
            // default; a game turns buffering off explicitly via buffer_mode 0.
            buffer_mode: true,
            show_status_requested: false,
            erase_lower_requested: false,
            upper: UpperWindow::default(),
            current_font: 1,
            current_fg: ZColour::Default,
            current_bg: ZColour::Default,
            v6: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Output stream state
// ---------------------------------------------------------------------------

/// One frame of nested stream-3 redirection.
struct Stream3Frame {
    /// Base address of the table in dynamic memory.
    table_addr: u32,
    /// Bytes written so far into this frame (accumulated before we flush).
    buf: Vec<u8>,
    /// Resolved box width in pixels for v6's optional 3rd `output_stream`
    /// operand (ZMSD §15 `output_stream`: "In Version 6, a width field may
    /// optionally be given: text will then be justified as if it were in the
    /// window with that number (if width is zero or positive) or a box
    /// -width pixels wide (if negative)."). `None` means the operand was
    /// omitted — text is stored verbatim, unwrapped (pre-existing behaviour).
    width_px: Option<u16>,
}

/// Manages all four Z-machine output streams plus the selected input stream.
///
/// Streams 1 (screen) and 2 (transcript) are on/off flags; only stream 1
/// defaults to on.  Stream 3 redirects text to a memory table and can nest.
/// Stream 4 (command log) is flag-only.  The input stream (`input_stream`
/// opcode) is recorded here too; the engine drives all input through the host,
/// so this field only remembers the game's selection.
pub struct StreamState {
    /// Stream 1 (screen) active.
    pub stream1: bool,
    /// Stream 2 (transcript) active.
    pub stream2: bool,
    /// Stream 4 (command log) active.
    pub stream4: bool,
    /// Selected input stream: 0 = keyboard (default), 1 = command file.
    /// Recorded for the host; the engine never reads input from a file itself.
    pub input_stream: u8,
    /// Stack of active stream-3 frames (nested up to 16).
    stream3_stack: Vec<Stream3Frame>,
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    pub fn new() -> Self {
        StreamState {
            stream1: true,
            stream2: false,
            stream4: false,
            input_stream: 0,
            stream3_stack: Vec::new(),
        }
    }

    /// True when stream 3 is active (text goes to memory, not screen).
    pub fn stream3_active(&self) -> bool {
        !self.stream3_stack.is_empty()
    }

    /// Select (push) stream 3 with a table at `table_addr` (ZMSD §7.1.2.5).
    /// `width_px` is the resolved box width in pixels for v6's optional 3rd
    /// operand (the caller resolves a window-number-vs-negative-pixel-width
    /// operand into pixels before calling, since that needs the v6 window
    /// table which lives outside `StreamState`); `None` when the operand was
    /// omitted.
    pub fn push_stream3(&mut self, table_addr: u32, width_px: Option<u16>) {
        if self.stream3_stack.len() < 16 {
            self.stream3_stack.push(Stream3Frame { table_addr, buf: Vec::new(), width_px });
        }
    }

    /// Deselect (pop) stream 3: write accumulated bytes into memory, update
    /// the length word, and return.
    ///
    /// The table layout: word at `table_addr` = byte count; bytes follow from
    /// `table_addr + 2`. When a v6 width was given, the text is word-wrapped
    /// to that width first (ZMSD §15 `output_stream`, quoted on
    /// `Stream3Frame::width_px`) — "Then the table will contain not ordinary
    /// text but formatted text: see print_form." Wrapping happens here, at
    /// close, on the whole accumulated buffer (splitting on ASCII spaces)
    /// rather than incrementally per printed word the way Frotz's
    /// `memory_word` does it; nothing yet consumes the formatted-text table
    /// (`print_form` is a stub — SQ-0457), so a faithful approximation is
    /// enough to make the header math and stored bytes sane.
    pub fn pop_stream3(&mut self, mem: &mut Memory) {
        if let Some(frame) = self.stream3_stack.pop() {
            let (bytes, total_width) = match frame.width_px {
                Some(w) => wrap_stream3_text(&frame.buf, w),
                None => {
                    let w = frame.buf.len() as u32 * V6_FONT_WIDTH as u32;
                    (frame.buf, w)
                }
            };
            let n = bytes.len() as u16;
            mem.write_word(frame.table_addr, n);
            for (i, &b) in bytes.iter().enumerate() {
                mem.write_byte(frame.table_addr + 2 + i as u32, b);
            }
            // ZMSD §7.1.2.1: in v6, deselecting stream 3 stores "the total
            // width of printing (in units)" in header word $30. Infocom games
            // MEASURE string widths this way — Shogun prints its status
            // fields to stream 3 and reads $30 back to right-align them; an
            // unwritten $30 collapses that math to garbage columns.
            if mem.version() == 6 {
                mem.write_word(0x30, total_width.min(u16::MAX as u32) as u16);
            }
        }
    }

    /// Append raw ZSCII bytes to the current stream-3 buffer (ZMSD §7.1.2.5:
    /// each output character is stored as a single byte, not UTF-8). Callers
    /// must convert chars to ZSCII themselves (`Memory::zscii_from_unicode`) —
    /// `StreamState` has no access to the story's custom Unicode table.
    pub fn write_stream3_bytes(&mut self, bytes: &[u8]) {
        if let Some(frame) = self.stream3_stack.last_mut() {
            frame.buf.extend_from_slice(bytes);
        }
    }
}

/// Word-wrap a stream-3 buffer to `width_px` pixels (fixed-width v6 font,
/// `V6_FONT_WIDTH` per glyph), replacing the space at each wrap point with a
/// ZSCII 13 newline (ZMSD §7.1.2.2.1: "Newlines are written to output stream
/// 3 as ZSCII 13") — mirrors Frotz's `memory_word`/`memory_close`
/// (`redirect.c`): a word that would overflow the current line drops its
/// leading space and starts a fresh line instead. Existing embedded ZSCII 13
/// bytes are treated as hard breaks: they end the current line without being
/// counted as printable width, and line-width accounting restarts after them.
/// Returns the rewritten bytes and the total width (sum of every completed
/// line's pixel width, hard-broken or wrapped) for header $30.
fn wrap_stream3_text(buf: &[u8], width_px: u16) -> (Vec<u8>, u32) {
    let fw = V6_FONT_WIDTH as u32;
    let mut out = Vec::with_capacity(buf.len());
    let mut total: u32 = 0;
    for segment in buf.split(|&b| b == 13) {
        let mut line_width: u32 = 0;
        let mut first = true;
        for word in segment.split(|&b| b == b' ') {
            if first {
                first = false;
                line_width = word.len() as u32 * fw;
                out.extend_from_slice(word);
                continue;
            }
            let candidate = line_width + fw + word.len() as u32 * fw;
            if line_width > 0 && candidate > width_px as u32 {
                total += line_width;
                out.push(13);
                line_width = word.len() as u32 * fw;
                out.extend_from_slice(word);
            } else {
                out.push(b' ');
                out.extend_from_slice(word);
                line_width = candidate;
            }
        }
        total += line_width;
        out.push(13); // restore the hard break consumed by `split`
    }
    out.pop(); // the loop always adds one trailing 13 too many
    (out, total)
}

// ---------------------------------------------------------------------------
// Header capability bits (ZMSD §11.1)
// ---------------------------------------------------------------------------

/// Default interpreter number (header 0x1E) per Frotz's rule (ux_init.c): IBM PC
/// (6) for v6 story files, DECSystem-20 (1) otherwise. v6 is rejected at load,
/// so in practice every loaded game defaults to 1.
pub fn default_interpreter_number(version: u8) -> u8 {
    if version == 6 { 6 } else { 1 }
}

/// Set interpreter capability bits in the story header at machine startup.
///
/// We advertise a basic text interpreter:
///   - Flags1: clear graphics (bit 1 v3 is status-line kind — leave alone),
///     clear colour (bit 0 in v5+), clear pictures. Sound (bit 5 in v4+) is
///     capability-driven — see `advertise_sound`.
///     Set fixed-pitch available (bit 4 in v3 Flags2? — actually interpreter
///     sets this in response to Flags2; for simplicity we just clear "no
///     status line" bit).
///   - Flags2: we clear bits we don't support (pictures=0, undo=0, mouse=0,
///     colour=0, menubar=0) by masking. Sound (bit 7) is capability-driven —
///     see `advertise_sound`. Leave transcript (bit 0) and fixed-pitch (bit 1)
///     as-is (game controls those).
///   - 0x1E: interpreter number — override, else Frotz's default (6 for v6, else 1).
///   - 0x1F: interpreter version — 'A' (ASCII 0x41), standard v1.1 era.
///   - 0x32/0x33: standard revision number (1.1 → 1, 1).
///
/// Only modifies bytes inside dynamic memory (below static_mem_base); if the
/// header region is read-only (static_mem_base ≤ 0x40) we skip silently.
pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool, sound_available: bool, interpreter_number: Option<u8>) {
    let version = mem.version();

    // Guard: only write if the header sits in dynamic memory.
    // All story files should have static_mem_base > 0x40 (ZMSD §1.1), but be safe.
    // We check individual addresses before each write via the fact that
    // `write_byte` debug-asserts the range; to avoid panics we only call it
    // if we know memory is writable.  In practice all well-formed stories have
    // dynamic memory covering the header, so this is always fine.

    // Flags1 (byte 0x01): interpreter-writable bits.
    let f1 = mem.read_byte(0x01);
    let new_f1 = if version <= 3 {
        // v3 Flags1 bits (ZMSD §11.1.1):
        //   bit 1: time game (0 = score/turns, set by game — don't touch)
        //   bit 4: status line not available — clear (we support it)
        //   bit 5: screen-splitting available — set
        //   bit 6: variable-pitch font default — clear (use fixed)
        f1 & !((1 << 4) | (1 << 6))   // clear "status line not available" + variable-pitch default
          | (1 << 5)      // screen-splitting available
    } else {
        // v4+ Flags1 bits (ZMSD §11.1.3):
        //   bit 0: colour available — clear (no colour)
        //   bit 1: picture display available — clear
        //   bit 2: boldface available — set (rendered via SGR / style spans)
        //   bit 3: italic available — set (rendered via SGR / style spans)
        //   bit 4: fixed-space font available — set
        //   bit 5: sound effects available — handled separately (advertise_sound)
        //   bit 7: timed keyboard available — clear
        f1 & !((1 << 1) | (1 << 7))  // clear unsupported (colour, sound handled separately)
          | (1 << 2) | (1 << 3) | (1 << 4)  // bold, italic, fixed-space font available
    };
    mem.write_byte(0x01, new_f1);

    // Flags2 (word 0x10–0x11, interpreter clears bits it doesn't support).
    // ZMSD §11.1.4: bits 3–5, 7 are interpreter-writable (clear = not supported).
    //   bit 3: pictures — clear
    //   bit 4: undo available — set for v5+ (save_undo/restore_undo implemented);
    //          pre-v5 has no undo opcodes, so leave it clear.
    //   bit 5: mouse — clear
    //   bit 7: sound effects — handled separately (advertise_sound)
    let f2 = mem.read_word(0x10);
    let mut new_f2 = f2 & !((1 << 3) | (1 << 5));
    if version >= 5 {
        new_f2 |= 1 << 4; // undo available
    } else {
        new_f2 &= !(1 << 4); // pre-v5: no undo
    }
    mem.write_word(0x10, new_f2);

    // Interpreter number (0x1E): explicit override, else Frotz's default
    // (6 for v6, else 1 = DEC-20). `version` was read at the top of this fn.
    let interp = interpreter_number.unwrap_or_else(|| default_interpreter_number(version));
    mem.write_byte(0x1E, interp);

    // Interpreter version (0x1F): b'A' = 0x41.
    mem.write_byte(0x1F, b'A');

    // Standard revision (0x32 = major, 0x33 = minor): 1.1 — the only published
    // Z-Machine Standards Document revision (ZMSD 1.1); no "1.2" exists.
    mem.write_byte(0x32, 1);
    mem.write_byte(0x33, 1);

    // Screen dimensions (ZMSD §11.1). Without these the header keeps the story
    // file's defaults (usually 0), and size-sensitive games (notably Bureaucracy)
    // read "0 lines", print "[Screen too small.]" and abort on the first turn.
    // Seed a generous default; the host refines it to the real pane size via
    // `write_screen_dims` once known (and on resize).
    write_screen_dims(mem, DEFAULT_SCREEN_ROWS, DEFAULT_SCREEN_COLS);

    // Default colours (ZMSD §8.3.2/§8.3.3). Bytes $2C (default background) and
    // $2D (default foreground) exist in V5+. §8.3.3: a colour-capable interpreter
    // "should ... write its default background and foreground colours into bytes
    // $2c and $2d"; §8.3.2: a non-colour interpreter should "write colours 2 and
    // 9 (black and white) ... into the default background and foreground". Both
    // cases are satisfied by black-background / white-foreground, our default
    // presentation. Infocom's own V6 games ship $2C/$2D = 0 (an invalid "current"
    // sentinel); games that read the header defaults to build a colour scheme —
    // Beyond Zork (V5) among them — compute garbage colour numbers from 0/0 and
    // their set_colour calls get ignored, leaving the game monochrome. Seeding
    // valid numbers here makes such games colour correctly.
    if version >= 5 {
        mem.write_byte(0x2C, 2); // default background = black
        mem.write_byte(0x2D, 9); // default foreground = white
    }

    advertise_colour(mem, honor_game_colours);
    advertise_sound(mem, sound_available);
}

/// Set or clear the Flags1 "colour available" bit (bit 0). No-op for v3, which
/// has no colour capability bit. Re-applied on every header init and whenever
/// the host toggles `honor_game_colours`.
pub fn advertise_colour(mem: &mut Memory, on: bool) {
    if mem.version() < 4 {
        return;
    }
    let f1 = mem.read_byte(0x01);
    let f1 = if on { f1 | 1 } else { f1 & !1 };
    mem.write_byte(0x01, f1);

    // Flags2 bit 6 (word 0x10) is the game's "wants colours" request bit. When
    // colour is off, clear it so a game doesn't proceed believing colour was
    // granted; when on, leave the game's request untouched. Render gates colour
    // regardless, so this is strict-correctness hygiene (ZMSD §11.1.4).
    if !on {
        let f2 = mem.read_word(0x10);
        mem.write_word(0x10, f2 & !(1 << 6));
    }
}

/// Set or clear the sound-effects capability bits: Flags1 bit 5 (v4+ ONLY —
/// in v3 that bit means "screen-splitting available", a different capability,
/// and is left untouched) and Flags2 bit 7 (all versions). Re-applied on
/// every header init and whenever the host toggles `sound_available`.
pub fn advertise_sound(mem: &mut Memory, on: bool) {
    if mem.version() >= 4 {
        let f1 = mem.read_byte(0x01);
        let f1 = if on { f1 | (1 << 5) } else { f1 & !(1 << 5) };
        mem.write_byte(0x01, f1);
    }
    let f2 = mem.read_word(0x10);
    let f2 = if on { f2 | (1 << 7) } else { f2 & !(1 << 7) };
    mem.write_word(0x10, f2);
}

/// Default screen size seeded at header init, before the host reports the real
/// pane size. Generous enough that size-sensitive v4+ games run.
pub const DEFAULT_SCREEN_ROWS: u8 = 24;
pub const DEFAULT_SCREEN_COLS: u8 = 80;

/// v6 font cell size in pixels. Reference interpreters present Infocom v6 on a
/// **non-square 8×16 cell** — the Amiga/DOS profile Frotz uses for every v6 game
/// (`src/dos/bcinit.c` mode table `{0x12, 640, 400, 8, 16}`; `restart_header`
/// seeds `h_font_width=8, h_font_height=16`). 8 wide × 16 tall over a 640×400
/// screen gives the authentic **80 cols × 25 rows** that makes text read at the
/// period-screenshot size relative to the 2×-scaled 320×200 art (SQ-0479). v6
/// addresses everything in pixels; the app quantizes to character cells by
/// dividing X by WIDTH and Y by HEIGHT.
pub const V6_FONT_WIDTH: u16 = 8;
pub const V6_FONT_HEIGHT: u16 = 16;

/// Write the screen-dimension header fields for the loaded story's version.
///
/// v4+: byte 0x20 = height in lines, byte 0x21 = width in chars (ZMSD §11.1).
/// v5+: also word 0x22 = width in units, word 0x24 = height in units, and font
/// size bytes 0x26/0x27 = 1 (one unit per char cell, since we render a fixed
/// character grid). `rows`/`cols` of 0 are clamped to 1 to avoid a zero size.
pub fn write_screen_dims(mem: &mut Memory, rows: u8, cols: u8) {
    let version = mem.version();
    if version < 4 {
        return; // v1-3 have no settable screen-size header fields.
    }
    let rows = rows.max(1);
    let cols = cols.max(1);
    if version == 6 {
        mem.write_byte(0x20, rows);
        mem.write_byte(0x21, cols);
        // ZMSD §8.4.3: word $22 = screen width in units, word $24 = screen
        // height in units. v6 units are pixels, so width = cols·8 (640) and
        // height = rows·16 (400) — the ·16 is why a non-square cell needs this
        // path (a square cell made $24 latently correct).
        mem.write_word(0x22, cols as u16 * V6_FONT_WIDTH); // screen width, pixels
        mem.write_word(0x24, rows as u16 * V6_FONT_HEIGHT); // screen height, pixels
        // ZMSD §11.1 header table (verified against the spec): byte $26 = "Font
        // width in V5, or font HEIGHT in V6"; byte $27 = "Font height in V5, or
        // font WIDTH in V6" — the famous V5↔V6 swap (§8.1.1: "in Version 6 the
        // width and height are stored the other way round"). So in V6:
        // $26 = HEIGHT (16), $27 = WIDTH (8). Latent while square; load-bearing now.
        mem.write_byte(0x26, V6_FONT_HEIGHT as u8);
        mem.write_byte(0x27, V6_FONT_WIDTH as u8);
        return;
    }
    mem.write_byte(0x20, rows);
    mem.write_byte(0x21, cols);
    if version >= 5 {
        mem.write_word(0x22, cols as u16); // screen width in units
        mem.write_word(0x24, rows as u16); // screen height in units
        mem.write_byte(0x26, 1); // font width in units
        mem.write_byte(0x27, 1); // font height in units
    }
}

// ---------------------------------------------------------------------------
// Status-line computation (v3)
// ---------------------------------------------------------------------------

/// Compute the current v3 status line from memory globals and header.
///
/// G0 (global var 0) = location object number.
/// G1 = score (signed) or hours (unsigned).
/// G2 = turns or minutes.
/// Flags1 bit 1: 0 = score/turns, 1 = time.
pub fn compute_status_line(mem: &Memory) -> StatusLine {
    let gbase = mem.global_vars() as u32;
    let loc_obj = mem.read_word(gbase);
    let g1 = mem.read_word(gbase + 2);
    let g2 = mem.read_word(gbase + 4);

    let location = if loc_obj == 0 {
        String::new()
    } else {
        objects::short_name(mem, loc_obj)
    };

    // Flags1 bit 1 selects time mode.
    let flags1 = mem.read_byte(0x01);
    let time_mode = (flags1 & (1 << 1)) != 0;

    let right = if time_mode {
        StatusRight::Time { hours: g1 as u8, minutes: g2 as u8 }
    } else {
        StatusRight::ScoreTurns { score: g1 as i16, turns: g2 }
    };

    StatusLine { location, right }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a minimal v3 story with one object whose short name is "West of House".
    /// Object 1 is placed at the v3 entries base.
    /// G0 = 1 (location object), G1, G2 = supplied.
    fn build_v3_status_story(g1: u16, g2: u16, time_mode: bool) -> Vec<u8> {
        let mut buf = sample_story(3);

        // Object table is at 0x0100 (set by sample_story).
        // v3 property-defaults: 31 words = 62 bytes → entries at 0x013E.
        let obj1_entry: usize = 0x013E;
        let prop_tbl: u16 = 0x0200;

        // Object 1 entry (9 bytes): no attrs, no tree, prop_tbl pointer.
        for i in 0..7 { buf[obj1_entry + i] = 0; }
        buf[obj1_entry + 7] = (prop_tbl >> 8) as u8;
        buf[obj1_entry + 8] = (prop_tbl & 0xFF) as u8;

        // Property table: short name = "west" (2 Z-words).
        let name = encode_word("west", 3); // 4 bytes
        assert_eq!(name.len(), 4);
        buf[prop_tbl as usize] = 2; // 2 name-words
        buf[prop_tbl as usize + 1..prop_tbl as usize + 5].copy_from_slice(&name);
        buf[prop_tbl as usize + 5] = 0x00; // sentinel

        // Set G0=1, G1=g1, G2=g2 in global vars table (0x0300).
        let gbase: usize = 0x0300;
        buf[gbase]     = 0; buf[gbase + 1] = 1;  // G0 = 1
        buf[gbase + 2] = (g1 >> 8) as u8; buf[gbase + 3] = (g1 & 0xFF) as u8;
        buf[gbase + 4] = (g2 >> 8) as u8; buf[gbase + 5] = (g2 & 0xFF) as u8;

        // Flags1: bit 1 controls time mode.
        if time_mode {
            buf[0x01] |= 1 << 1;
        } else {
            buf[0x01] &= !(1 << 1);
        }

        buf
    }

    // ── (a) v3 status line: score/turns mode ─────────────────────────────────

    #[test]
    fn v3_status_line_score_turns() {
        let buf = build_v3_status_story(42u16, 7, false);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(
            sl.location.starts_with("west"),
            "location should start with 'west', got {:?}", sl.location
        );
        assert_eq!(sl.right, StatusRight::ScoreTurns { score: 42, turns: 7 });
    }

    // ── (b) v3 status line: time mode ────────────────────────────────────────

    #[test]
    fn v3_status_line_time_mode() {
        let buf = build_v3_status_story(10, 30, true);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(sl.location.starts_with("west"), "location should start with 'west'");
        assert_eq!(sl.right, StatusRight::Time { hours: 10, minutes: 30 });
    }

    // ── (c) header capability bits ───────────────────────────────────────────

    #[test]
    fn header_caps_v3_clears_no_status_line() {
        let mut mem = Memory::new(sample_story(3)).unwrap();
        // Set "status line not available" bit before init.
        let f1 = mem.read_byte(0x01) | (1 << 4);
        mem.write_byte(0x01, f1);
        init_header_caps(&mut mem, false, false, None);
        // Bit 4 should be cleared.
        assert_eq!(mem.read_byte(0x01) & (1 << 4), 0, "bit 4 (no status line) should be clear");
        // Screen-splitting available (bit 5) should be set.
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "bit 5 (screen-split) should be set");
    }

    #[test]
    fn header_caps_v5_clears_unsupported_bits() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        let f1 = mem.read_byte(0x01);
        // Colour (bit 0) should be clear.
        assert_eq!(f1 & (1 << 0), 0, "colour bit should be clear");
        // Pictures (bit 1) should be clear.
        assert_eq!(f1 & (1 << 1), 0, "pictures bit should be clear");
        // Fixed-space font (bit 4) should be set.
        assert_ne!(f1 & (1 << 4), 0, "fixed-space font bit should be set");
        // Interpreter number set.
        assert_eq!(mem.read_byte(0x1E), 1, "interpreter number defaults to DEC-20 (1)");
        assert_eq!(mem.read_byte(0x1F), b'A', "interpreter version = 'A'");
    }

    #[test]
    fn header_caps_v4_seeds_nonzero_screen_dims() {
        // Regression: without seeded screen dims the header keeps 0, and v4 games
        // such as Bureaucracy abort with "[Screen too small.]" on the first turn.
        let mut mem = Memory::new(sample_story(4)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x20), DEFAULT_SCREEN_ROWS, "screen height (lines) seeded");
        assert_eq!(mem.read_byte(0x21), DEFAULT_SCREEN_COLS, "screen width (chars) seeded");
        assert_ne!(mem.read_byte(0x20), 0, "height must not be zero");
        assert_ne!(mem.read_byte(0x21), 0, "width must not be zero");
    }

    #[test]
    fn header_caps_v5_seeds_unit_words_and_font_size() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x20), DEFAULT_SCREEN_ROWS);
        assert_eq!(mem.read_byte(0x21), DEFAULT_SCREEN_COLS);
        assert_eq!(mem.read_word(0x22), DEFAULT_SCREEN_COLS as u16, "width in units");
        assert_eq!(mem.read_word(0x24), DEFAULT_SCREEN_ROWS as u16, "height in units");
        assert_eq!(mem.read_byte(0x26), 1, "font width = 1 unit");
        assert_eq!(mem.read_byte(0x27), 1, "font height = 1 unit");
    }

    #[test]
    fn write_screen_dims_is_noop_for_v3() {
        // v1-3 use bytes 0x20+ for other header data; never clobber them.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        let before = mem.read_byte(0x20);
        write_screen_dims(&mut mem, 30, 60);
        assert_eq!(mem.read_byte(0x20), before, "v3 header byte 0x20 must be untouched");
    }

    #[test]
    fn write_screen_dims_clamps_zero_to_one() {
        let mut mem = Memory::new(sample_story(4)).unwrap();
        write_screen_dims(&mut mem, 0, 0);
        assert_eq!(mem.read_byte(0x20), 1, "zero rows clamped to 1");
        assert_eq!(mem.read_byte(0x21), 1, "zero cols clamped to 1");
    }

    #[test]
    fn v6_advertises_pixel_screen_and_font() {
        let mut m = Memory::new(sample_story(6)).unwrap();
        write_screen_dims(&mut m, 24, 80);
        assert_eq!(m.read_byte(0x20), 24, "rows");
        assert_eq!(m.read_byte(0x21), 80, "cols");
        assert_eq!(m.read_word(0x22), 80 * V6_FONT_WIDTH, "screen width in pixels");
        assert_eq!(m.read_word(0x24), 24 * V6_FONT_HEIGHT, "screen height in pixels");
        // ZMSD §11.1/§8.1.1: in V6 the font-size bytes are the swap of V5 —
        // $26 = font HEIGHT (16), $27 = font WIDTH (8). Non-square now exercises it.
        assert_eq!(m.read_byte(0x26), V6_FONT_HEIGHT as u8, "$26 = font height in V6");
        assert_eq!(m.read_byte(0x27), V6_FONT_WIDTH as u8, "$27 = font width in V6");
        assert_eq!((m.read_byte(0x26), m.read_byte(0x27)), (16, 8), "8×16 non-square cell");
    }

    #[test]
    fn header_caps_v3_clears_variable_pitch_default() {
        // We render fixed-pitch; Flags1 v3 bit 6 (variable-pitch default) must be
        // explicitly cleared rather than inheriting the story file's value.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        let f1 = mem.read_byte(0x01) | (1 << 6); // pre-set variable-pitch default
        mem.write_byte(0x01, f1);
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x01) & (1 << 6), 0, "bit 6 (variable-pitch) should be clear");
    }

    #[test]
    fn header_caps_writes_standard_revision_1_1() {
        // ZMSD 1.1 is the only published standard revision; advertise major=1,
        // minor=1 (bytes 0x32/0x33), not a non-existent "1.2".
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x32), 1, "standard revision major = 1");
        assert_eq!(mem.read_byte(0x33), 1, "standard revision minor = 1");
    }

    #[test]
    fn header_caps_v5_advertises_styles_and_undo() {
        // Bold/italic are rendered (SGR / style spans) and multi-level undo
        // (save_undo/restore_undo, EXT:0x09/0x0A) is implemented, so the header
        // must advertise them or games skip the features at startup.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        let f1 = mem.read_byte(0x01);
        assert_ne!(f1 & (1 << 2), 0, "Flags1 bit 2 (bold available) should be set");
        assert_ne!(f1 & (1 << 3), 0, "Flags1 bit 3 (italic available) should be set");
        let f2 = mem.read_word(0x10);
        assert_ne!(f2 & (1 << 4), 0, "Flags2 bit 4 (undo available) should be set");
    }

    #[test]
    fn header_caps_flags2_clears_pictures() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        // Pre-set pictures (bit 3) in Flags2.
        let f2 = mem.read_word(0x10) | (1 << 3);
        mem.write_word(0x10, f2);
        init_header_caps(&mut mem, false, false, None);
        let f2_after = mem.read_word(0x10);
        assert_eq!(f2_after & (1 << 3), 0, "pictures bit in Flags2 should be clear");
    }

    #[test]
    fn sound_bit_tracks_sound_available_flag_v5() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x01) & (1 << 5), 0, "Flags1 sound bit clear when sound_available=false");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "Flags2 sound bit clear when sound_available=false");

        init_header_caps(&mut mem, false, true, None);
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "Flags1 sound bit set when sound_available=true");
        assert_ne!(mem.read_word(0x10) & (1 << 7), 0, "Flags2 sound bit set when sound_available=true");

        advertise_sound(&mut mem, false);
        assert_eq!(mem.read_byte(0x01) & (1 << 5), 0, "advertise_sound(false) clears Flags1 bit again");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "advertise_sound(false) clears Flags2 bit again");
    }

    #[test]
    fn sound_bit_v3_flags1_untouched_but_flags2_tracks() {
        let mut mem = Memory::new(sample_story(3)).unwrap();
        init_header_caps(&mut mem, false, true, None);
        // v3 Flags1 bit 5 means "screen-splitting available", NOT sound — must
        // stay set regardless of sound_available (it's set unconditionally by
        // init_header_caps for v3, see header_caps_v3_clears_no_status_line).
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "v3 Flags1 bit5 (screen-split) stays set");
        assert_ne!(mem.read_word(0x10) & (1 << 7), 0, "v3 Flags2 sound bit set when sound_available=true");

        init_header_caps(&mut mem, false, false, None);
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "v3 Flags1 bit5 (screen-split) still set");
        assert_eq!(mem.read_word(0x10) & (1 << 7), 0, "v3 Flags2 sound bit clear when sound_available=false");
    }

    #[test]
    fn colour_bit_tracks_honor_flag() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x01) & 1, 0, "colour bit clear when honor=false");
        init_header_caps(&mut mem, true, false, None);
        assert_eq!(mem.read_byte(0x01) & 1, 1, "colour bit set when honor=true");
        advertise_colour(&mut mem, false);
        assert_eq!(mem.read_byte(0x01) & 1, 0, "advertise_colour clears it again");
    }

    #[test]
    fn default_colours_seeded_in_header_v5plus() {
        // ZMSD §8.3.2/§8.3.3: the interpreter writes default bg/fg into $2C/$2D.
        // Infocom stories ship 0/0 (invalid "current"); we overwrite with black
        // (2) bg / white (9) fg so games that read the header defaults compute
        // valid colour numbers.
        for v in [5u8, 6, 7, 8] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            mem.write_byte(0x2C, 0); // simulate Infocom's 0/0
            mem.write_byte(0x2D, 0);
            init_header_caps(&mut mem, true, false, None);
            assert_eq!(mem.read_byte(0x2C), 2, "v{v} default background = black(2)");
            assert_eq!(mem.read_byte(0x2D), 9, "v{v} default foreground = white(9)");
        }
    }

    #[test]
    fn default_colours_not_written_pre_v5() {
        // $2C/$2D are not colour-default bytes before V5; leave them alone.
        let mut mem = Memory::new(sample_story(3)).unwrap();
        mem.write_byte(0x2C, 0x11);
        mem.write_byte(0x2D, 0x22);
        init_header_caps(&mut mem, true, false, None);
        assert_eq!(mem.read_byte(0x2C), 0x11, "v3 $2C untouched");
        assert_eq!(mem.read_byte(0x2D), 0x22, "v3 $2D untouched");
    }

    #[test]
    fn flags2_colour_request_bit_cleared_when_colour_off() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        // Game requests colours (Flags2 bit 6).
        let f2 = mem.read_word(0x10) | (1 << 6);
        mem.write_word(0x10, f2);
        // Honour OFF: the request bit is cleared (colour not granted).
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_word(0x10) & (1 << 6), 0, "bit 6 cleared when colour off");
        // Honour ON: the game's request bit is left untouched.
        let f2 = mem.read_word(0x10) | (1 << 6);
        mem.write_word(0x10, f2);
        init_header_caps(&mut mem, true, false, None);
        assert_eq!(mem.read_word(0x10) & (1 << 6), 1 << 6, "bit 6 preserved when colour on");
    }

    #[test]
    fn default_interpreter_number_follows_frotz_rule() {
        // Frotz: DEC-20 (1) for non-v6, IBM PC (6) for v6.
        assert_eq!(default_interpreter_number(3), 1);
        assert_eq!(default_interpreter_number(5), 1);
        assert_eq!(default_interpreter_number(8), 1);
        assert_eq!(default_interpreter_number(6), 6);
    }

    #[test]
    fn init_header_caps_default_interpreter_is_dec20_for_v5() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, None);
        assert_eq!(mem.read_byte(0x1E), 1, "v5 default interpreter = DEC-20 (1)");
    }

    #[test]
    fn init_header_caps_interpreter_override_wins() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, false, Some(6));
        assert_eq!(mem.read_byte(0x1E), 6, "override forces IBM PC (6)");
    }

    #[test]
    fn v6_screen_state_has_window_table() {
        let m = crate::cpu::exec::Machine::new(Memory::new(sample_story(6)).unwrap());
        let v6 = m.screen.v6.as_ref().expect("v6 story has a window table");
        assert_eq!(v6.windows.len(), 8);
        assert_eq!(v6.current, 0);
    }

    #[test]
    fn non_v6_has_no_window_table() {
        let m = crate::cpu::exec::Machine::new(Memory::new(sample_story(5)).unwrap());
        assert!(m.screen.v6.is_none(), "v5 keeps the classic 2-window model");
    }

    // ── Task 6: get_prop / put_prop over the ZMSD property array ────────────

    #[test]
    fn zwindow_prop_round_trip_all_16() {
        let mut w = ZWindow::default();
        for n in 0..16u16 {
            w.put_prop(n, 1000 + n);
        }
        for n in 0..16u16 {
            assert_eq!(w.get_prop(n), 1000 + n, "prop {n} round-trips");
        }
    }

    #[test]
    fn zwindow_prop_out_of_range_get_is_zero_and_put_is_ignored() {
        // prop 0, untouched by an out-of-range write below
        let mut w = ZWindow { y_coord: 42, ..Default::default() };
        assert_eq!(w.get_prop(16), 0, "prop 16+ not modeled here — reads 0");
        assert_eq!(w.get_prop(255), 0);
        w.put_prop(16, 999); // ignored — must not alias into any real field
        w.put_prop(255, 999);
        assert_eq!(w.get_prop(0), 42, "out-of-range put left prop 0 untouched");
    }

    #[test]
    fn zwindow_prop_indices_match_zmsd_1_1_8_8_3_2() {
        // Direct field <-> index mapping, verified against ZMSD 1.1 §8.8.3.2.
        let w = ZWindow {
            y_coord: 1, x_coord: 2, y_size: 3, x_size: 4,
            y_cursor: 5, x_cursor: 6, left_margin: 7, right_margin: 8,
            interrupt_routine: 9, interrupt_countdown: 10, text_style: 11, colour_data: 12,
            font_number: 13, font_size: 14, attributes: 15, line_count: 16,
            ..Default::default()
        };
        let expected = [1u16, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        for (n, exp) in expected.into_iter().enumerate() {
            assert_eq!(w.get_prop(n as u16), exp, "prop {n}");
        }
    }

    // ── (d) ScreenState defaults ──────────────────────────────────────────────

    #[test]
    fn screen_state_defaults() {
        let s = ScreenState::default();
        assert_eq!(s.upper_window_rows, 0);
        assert_eq!(s.current_window, 0);
        assert_eq!(s.text_style, 0);
        // The lower window is buffered (word-wrapped) by default (ZMSD §8.7.2.5).
        assert!(s.buffer_mode, "buffer_mode defaults to on (buffered)");
        assert_eq!(s.current_font, 1, "default font is 1 (normal)");
    }

    // ── (f) UpperWindow: resize, put, cell, clear ───────────────────────────

    #[test]
    fn upper_window_resize_put_and_cell() {
        let mut w = UpperWindow::default();
        w.resize(2, 4);
        assert_eq!(w.rows, 2);
        assert_eq!(w.cols, 4);
        assert_eq!(w.cell(1, 1).ch, ' ');
        w.put(2, 3, 'X', 0b0001, ZColour::Default, ZColour::Default);
        assert_eq!(w.cell(2, 3).ch, 'X');
        assert_eq!(w.cell(2, 3).style, 0b0001);
        w.put(9, 9, 'Z', 0, ZColour::Default, ZColour::Default); // out of range -> ignored, no panic
        w.clear();
        assert_eq!(w.cell(2, 3).ch, ' ');
    }

    // ── Lane Z: scroll_window (EXT:0x14) helpers ─────────────────────────────

    #[test]
    fn upper_window_scroll_rows_up_shifts_content_and_blanks_bottom() {
        let mut w = UpperWindow::default();
        w.resize(3, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.put(3, 1, 'C', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(1); // positive: scroll forward/up
        assert_eq!(w.cell(1, 1).ch, 'B', "row 2 moved up to row 1");
        assert_eq!(w.cell(2, 1).ch, 'C', "row 3 moved up to row 2");
        assert_eq!(w.cell(3, 1).ch, ' ', "new bottom row is blank");
    }

    #[test]
    fn upper_window_scroll_rows_down_shifts_content_and_blanks_top() {
        let mut w = UpperWindow::default();
        w.resize(3, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.put(3, 1, 'C', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(-1); // negative: scroll backward/down
        assert_eq!(w.cell(1, 1).ch, ' ', "new top row is blank");
        assert_eq!(w.cell(2, 1).ch, 'A', "row 1 moved down to row 2");
        assert_eq!(w.cell(3, 1).ch, 'B', "row 2 moved down to row 3");
    }

    #[test]
    fn upper_window_scroll_rows_beyond_extent_clears() {
        let mut w = UpperWindow::default();
        w.resize(2, 2);
        w.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.scroll_rows(5);
        assert_eq!(w.cell(1, 1).ch, ' ');
        assert_eq!(w.cell(2, 1).ch, ' ');
    }

    #[test]
    fn zwindow_scroll_pixels_shifts_text_runs_and_drops_out_of_range() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text { y: 9, x: 1, text: "far".into(), style: 0, fg: ZColour::Default, bg: ZColour::Default });
        w.texts.push(V6Text { y: 1, x: 1, text: "near".into(), style: 0, fg: ZColour::Default, bg: ZColour::Default });
        // Scroll forward by 32px (two 16px lines):
        //   y=9  -> new_y=-23, bottom=-23+16-1=-8 < 1 -> fully above, dropped.
        //   y=1  -> new_y=-31, bottom=-31+16-1=-16 < 1 -> fully above, dropped.
        w.scroll_pixels(32);
        assert!(w.texts.is_empty(), "both runs fully scrolled above the window");
    }

    #[test]
    fn zwindow_scroll_pixels_keeps_run_still_partially_visible() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text { y: 9, x: 1, text: "keep".into(), style: 0, fg: ZColour::Default, bg: ZColour::Default });
        // Scroll forward by 8px: y=9 -> 1, bottom=1+16-1=16 >= 1, still kept.
        w.scroll_pixels(8);
        assert_eq!(w.texts.len(), 1, "run still overlapping the window is kept");
        assert_eq!(w.texts[0].y, 1, "kept run shifted by -pixels");
    }

    #[test]
    fn zwindow_scroll_pixels_negative_scrolls_down() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.texts.push(V6Text { y: 5, x: 1, text: "a".into(), style: 0, fg: ZColour::Default, bg: ZColour::Default });
        w.scroll_pixels(-3);
        assert_eq!(w.texts[0].y, 8, "negative pixels shift y downward (y - (-3) = y+3)");
    }

    #[test]
    fn zwindow_scroll_pixels_also_shifts_cell_grid_by_whole_rows() {
        let mut w = ZWindow { y_size: 24, ..Default::default() };
        w.grid.resize(3, 2);
        w.grid.put(1, 1, 'A', 0, ZColour::Default, ZColour::Default);
        w.grid.put(2, 1, 'B', 0, ZColour::Default, ZColour::Default);
        w.scroll_pixels(V6_FONT_HEIGHT as i16); // exactly one row
        assert_eq!(w.grid.cell(1, 1).ch, 'B', "grid shifted one row up");
    }

    // ── (e) StreamState: stream-3 push/pop/write ─────────────────────────────

    #[test]
    fn stream3_push_write_pop() {
        let buf = sample_story(5);
        // Reserve a table at 0x0050 (within dynamic memory, safely away from header).
        let table_addr: u32 = 0x0050;

        let mut mem = Memory::new(buf.clone()).unwrap();
        let mut ss = StreamState::new();

        assert!(!ss.stream3_active());
        ss.push_stream3(table_addr, None);
        assert!(ss.stream3_active());

        ss.write_stream3_bytes(b"Hello");
        ss.pop_stream3(&mut mem);

        assert!(!ss.stream3_active());

        // Check table: word at table_addr = 5 (length), then "Hello".
        assert_eq!(mem.read_word(table_addr), 5, "length word should be 5");
        assert_eq!(mem.read_byte(table_addr + 2), b'H');
        assert_eq!(mem.read_byte(table_addr + 3), b'e');
        assert_eq!(mem.read_byte(table_addr + 4), b'l');
        assert_eq!(mem.read_byte(table_addr + 5), b'l');
        assert_eq!(mem.read_byte(table_addr + 6), b'o');
    }

    #[test]
    fn stream3_write_bytes_stores_single_byte_per_char() {
        // A high ZSCII char (e.g. 195 = 'û') must be stored as ONE byte, not
        // multi-byte UTF-8 (SQ-0240).
        let buf = sample_story(5);
        let table_addr: u32 = 0x0050;

        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, None);
        ss.write_stream3_bytes(&[195]);
        ss.pop_stream3(&mut mem);

        assert_eq!(mem.read_word(table_addr), 1, "length word should be 1");
        assert_eq!(mem.read_byte(table_addr + 2), 195);
    }

    #[test]
    fn zcolour_defaults_and_cell_carries_colour() {
        assert_eq!(ZColour::default(), ZColour::Default);
        let c = Cell::default();
        assert_eq!(c.fg, ZColour::Default);
        assert_eq!(c.bg, ZColour::Default);

        let mut w = UpperWindow::default();
        w.resize(1, 4);
        w.put(1, 1, 'X', 0x01, ZColour::Standard(3), ZColour::Standard(6));
        let cell = w.cell(1, 1);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.style, 0x01);
        assert_eq!(cell.fg, ZColour::Standard(3));
        assert_eq!(cell.bg, ZColour::Standard(6));
    }

    #[test]
    fn rgb15_expansion_and_greys() {
        assert_eq!(rgb15_to_888(0x7FFF), (255, 255, 255));
        assert_eq!(rgb15_to_888(0x001F), (255, 0, 0)); // red = low 5 bits
        assert_eq!(grey_rgb(11), (0x80, 0x80, 0x80));
    }

    #[test]
    fn stream3_nested() {
        let buf = sample_story(5);
        let table1: u32 = 0x0050;
        let table2: u32 = 0x0060;

        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table1, None);
        ss.write_stream3_bytes(b"ab");
        ss.push_stream3(table2, None);
        ss.write_stream3_bytes(b"cd");
        ss.pop_stream3(&mut mem); // finalise table2
        ss.write_stream3_bytes(b"ef");
        ss.pop_stream3(&mut mem); // finalise table1

        // table2: "cd" (2 bytes)
        assert_eq!(mem.read_word(table2), 2);
        assert_eq!(mem.read_byte(table2 + 2), b'c');
        assert_eq!(mem.read_byte(table2 + 3), b'd');

        // table1: "ab" + "ef" = "abef" (4 bytes)
        assert_eq!(mem.read_word(table1), 4);
        assert_eq!(mem.read_byte(table1 + 2), b'a');
        assert_eq!(mem.read_byte(table1 + 3), b'b');
        assert_eq!(mem.read_byte(table1 + 4), b'e');
        assert_eq!(mem.read_byte(table1 + 5), b'f');
    }

    // ── (f) v6 output_stream 3 width operand: word-wrap on close ─────────────
    // ZMSD §15 output_stream: "In Version 6, a width field may optionally be
    // given: text will then be justified as if it were in the window with
    // that number (if width is zero or positive) or a box -width pixels wide
    // (if negative). Then the table will contain not ordinary text but
    // formatted text: see print_form."

    #[test]
    fn stream3_width_wraps_overflowing_word_onto_new_line() {
        // "AAAA BBBB" at a 40px box (V6_FONT_WIDTH=8 -> 5 chars) doesn't fit
        // "AAAA BBBB" (72px) on one line; the wrap point replaces the space
        // with ZSCII 13 and drops it from the width tally (Frotz
        // redirect.c:memory_word skips the leading space of the overflowing
        // word).
        let buf = sample_story(6);
        let table_addr: u32 = 0x0050;
        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, Some(40));
        ss.write_stream3_bytes(b"AAAA BBBB");
        ss.pop_stream3(&mut mem);

        let n = mem.read_word(table_addr);
        assert_eq!(n, 9, "byte count unchanged: the space becomes a newline byte");
        let bytes: Vec<u8> = (0..n).map(|i| mem.read_byte(table_addr + 2 + i as u32)).collect();
        assert_eq!(bytes, b"AAAA\rBBBB", "wrap point replaces the space with ZSCII 13");
        // Total width = 4 chars + 4 chars (the newline isn't printable width).
        assert_eq!(mem.read_word(0x30), 8 * V6_FONT_WIDTH, "header $30 excludes the wrap newline");
    }

    #[test]
    fn stream3_width_no_wrap_when_text_fits() {
        // Text that fits within the box on one line is untouched, and the
        // total width matches the simple char-count case (no formatting
        // actually needed).
        let buf = sample_story(6);
        let table_addr: u32 = 0x0050;
        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table_addr, Some(200));
        ss.write_stream3_bytes(b"Score:");
        ss.pop_stream3(&mut mem);

        assert_eq!(mem.read_word(table_addr), 6);
        assert_eq!(mem.read_word(0x30), 6 * V6_FONT_WIDTH);
    }
}
