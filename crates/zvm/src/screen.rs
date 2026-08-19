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

impl ZColour {
    /// This channel's colour as a 15-bit true-colour value (ZMSD §8.3.7), given
    /// the interpreter's own default colour number for the channel.
    ///
    /// Standard numbers map through the §8.3.1 table; `Default` resolves to the
    /// interpreter default the header publishes in $2C/$2D (which is what the
    /// player actually sees); `True` is already a 15-bit value; `True24` is a
    /// 24-bit host colour rounded down to 15 bits — §8.8.3.2.8 anticipates
    /// exactly that ("the value shown may be a 15-bit rounding of a more precise
    /// colour").
    ///
    /// There is no `-4` (transparent) answer here because the model has no
    /// transparent state: §8.3.6 lets an interpreter without transparency
    /// "ignore any attempt to select colour 15", and this one does.
    pub fn true_value(self, interpreter_default: u8) -> u16 {
        match self {
            ZColour::Default => standard_true_colour(interpreter_default).unwrap_or(0),
            ZColour::Standard(n) => standard_true_colour(n).unwrap_or(0),
            ZColour::True(v) => v & 0x7FFF,
            ZColour::True24(rgb) => {
                let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
                (((b >> 3) << 10) | ((g >> 3) << 5) | (r >> 3)) as u16
            }
        }
    }
}

/// Expand a 15-bit RGB (0bbbbbgggggrrrrr) to 8-bit `(r, g, b)`. Shared by the
/// CLI (SGR) and app (ratatui) renderers so the expansion is defined once.
pub fn rgb15_to_888(v: u16) -> (u8, u8, u8) {
    let exp = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
    (exp(v & 0x1F), exp((v >> 5) & 0x1F), exp((v >> 10) & 0x1F))
}

/// RGB for the v6 greys (Standard 10/11/12). Defined once here so both
/// renderers agree. Any other value falls back to dark grey (12).
///
/// ZMSD §8.3.1 gives the true-colour values for these three entries —
/// 10 = light grey ($5AD6), 11 = medium grey ($4631), 12 = dark grey ($2D6B) —
/// so they are just [`rgb15_to_888`] of the spec table, not an invented ramp.
/// Under [`Palette::Amiga`] they come from Infocom's Amiga table instead
/// ([`amiga_true_colour`]), which is the one place the greys genuinely differ.
pub fn grey_rgb(n: u8) -> (u8, u8, u8) {
    // Anything outside the three grey numbers reads as dark grey, as it always
    // has — the callers guard on 10..=12, so this is belt and braces.
    let n = if matches!(n, 10 | 11) { n } else { 12 };
    rgb15_to_888(standard_true_colour(n).unwrap_or(0x2D6B))
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
    /// Resize the grid **preserving** whatever cells survive the new extent
    /// (growing adds blank rows/cols, shrinking truncates).
    ///
    /// ZMSD §15 `split_window`: "In Version 3 (only) the upper window should be
    /// cleared after the split" — so from Version 4 on a re-split must leave the
    /// existing upper-window contents on screen. [`resize`] (which reallocates
    /// blank) is the Version 3 behaviour.
    pub fn resize_preserving(&mut self, rows: u16, cols: u16) {
        if cols == self.cols {
            // Row-major with an unchanged stride: truncate/extend in place.
            self.cells.resize(rows as usize * cols as usize, Cell::default());
            self.rows = rows;
            return;
        }
        let mut next = vec![Cell::default(); rows as usize * cols as usize];
        for r in 0..rows.min(self.rows) as usize {
            for c in 0..cols.min(self.cols) as usize {
                next[r * cols as usize + c] = self.cells[r * self.cols as usize + c];
            }
        }
        self.cells = next;
        self.rows = rows;
        self.cols = cols;
    }
    /// [`resize_preserving`](Self::resize_preserving), but a WIDENING continues
    /// each row's trailing appearance (style + colours, blanked to a space) into
    /// the columns that appear — instead of leaving them at the interpreter
    /// default. (SQ-0679)
    ///
    /// This is for a width change the HOST forces on the game (the screen grew
    /// under it, `refit_upper_window_width`), never for one the game asked for.
    /// A v4/v5 status line is painted once — a run of reverse-video spaces the
    /// game fills at whatever width byte $21 held when it laid out — and the
    /// fields are then updated in place, so nothing ever repaints the columns a
    /// later widen adds. Defaulting them punched an unstyled hole in the game's
    /// band from its old right edge to the new one: the reverse-video bar
    /// stopped short of its own box. Continuing the row's own trailing cell is
    /// the only extension that cannot introduce an appearance the game did not
    /// already have on that row.
    ///
    /// Not an erase, so ZMSD §8.7.3.4 ("Even if the text style is Reverse Video
    /// the new blank space should not have reversed colours") does not apply —
    /// that rule governs `erase_window`/`erase_line`, where the GAME asked for
    /// blank space. Here the game asked for nothing at all.
    pub fn resize_continuing_row_style(&mut self, rows: u16, cols: u16) {
        let old_cols = self.cols;
        // The appearance each surviving row ends in, captured before the move.
        let tail: Vec<Cell> = if cols > old_cols && old_cols > 0 {
            (0..rows.min(self.rows))
                .map(|r| Cell { ch: ' ', ..self.cell(r + 1, old_cols) })
                .collect()
        } else {
            Vec::new()
        };
        self.resize_preserving(rows, cols);
        for (r, t) in tail.iter().enumerate() {
            for c in old_cols..cols {
                self.cells[r * cols as usize + c as usize] = *t;
            }
        }
    }
    pub fn clear(&mut self) {
        self.clear_to(ZColour::Default);
    }
    /// Blank every cell to `bg`.
    ///
    /// ZMSD §8.7.3.2: a window is erased "to background colour", and §8.7.3.4
    /// adds "Even if the text style is Reverse Video the new blank space should
    /// not have reversed colours" — hence style 0 (no reverse bit) on the blank.
    pub fn clear_to(&mut self, bg: ZColour) {
        for c in &mut self.cells {
            *c = Cell { ch: ' ', style: 0, fg: ZColour::Default, bg };
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
    /// Flowing PROSE currently displayed in this window, as logical lines
    /// (SQ-0585). Only a wrap+scroll window that is not the one the game reads
    /// input through fills this: a v6 game may run several scrolling text windows
    /// at once — advent.z6's `style` opens one across the top and keeps playing in
    /// another below — and their streams must not be spliced into one transcript.
    ///
    /// This is LIVE SCREEN STATE, not history: no scrollback, bounded to
    /// [`PROSE_MAX_LINES`], and cleared by `erase_window` exactly as `texts` is.
    /// The window the game reads input through streams to the host transcript as
    /// before and leaves this empty.
    pub prose: Vec<String>,
    /// Where on the screen the prose this window has streamed to the host
    /// transcript is currently sitting (SQ-0697), as runs in the same
    /// screen-absolute space as [`ZWindow::texts`].
    ///
    /// ZMSD §15 is explicit that `move_window`/`window_size` "do not change the
    /// current display": text already printed stays as pixels where it was
    /// drawn. A scrolling window is no exception — Shogun prints its whole title
    /// header while window 0 is the full 640x400 screen, then moves window 0 down
    /// to a 548x64 box beside its menu and prints "You may choose to:" there. On a
    /// real interpreter the header stays painted up top; we stream both halves
    /// into one transcript, so they end up adjacent.
    ///
    /// So the stream is shadowed here as it goes out, and
    /// [`ZWindow::retire_streamed`] hands it over to `texts` — real paint — the
    /// moment the window's box changes. This is the ONLY use: nothing renders
    /// from this buffer directly, and while the window stays put the host
    /// transcript remains the single source of the prose.
    ///
    /// Bounded by the window itself: [`ZWindow::prose_new_line`] scrolls the runs
    /// up with the text and drops whatever leaves the top, so a window that never
    /// moves holds at most a screenful. `erase_window` empties it, exactly as it
    /// empties `texts` and `prose` — the pixels are gone, so their record is too.
    pub streamed: Vec<V6Text>,
    /// Prose this window streamed that has since been FROZEN in place (SQ-0697):
    /// the window was moved or resized, and ZMSD §15 keeps what was already
    /// printed exactly where it was drawn.
    ///
    /// Kept apart from [`ZWindow::texts`] rather than folded into it, even though
    /// both are paint in the same absolute space: `texts` is the window's live
    /// painted layer and several renderers reason about it as such (its rows are
    /// the window's own status/menu bars), while this is a layer the window has
    /// LEFT BEHIND, at coordinates it no longer covers. An erase trims and clears
    /// the two together — the pixels are shared, so their fates are.
    pub retired: Vec<V6Text>,
    /// Screen-absolute `(y, x)` in pixels where the FIRST glyph streamed since
    /// this was last cleared landed (SQ-0804). Set by
    /// [`ZWindow::record_streamed`] and cleared by
    /// [`ZWindow::clear_stream_origin`]; nothing in the VM reads it.
    ///
    /// It answers one question the printed TEXT cannot: did this burst of output
    /// continue the line the last one left the pen on, or did the game reposition
    /// first? A `read_char` echoes nothing (ZMSD §10.7), so a keypress turn
    /// inherits no newline and the host must decide for itself whether to open a
    /// transcript line for the turn's output — and games that redraw a menu
    /// `set_cursor` back to the top with no newline in sight, so the text alone is
    /// not decidable. Compare this against the cursor the window HAD before the
    /// turn and the answer is exact.
    ///
    /// Live per-burst state, not history: transient, not archived, and
    /// meaningless outside the window between a clear and the read that follows.
    pub stream_origin: Option<(u16, u16)>,
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

/// ZMSD §8.8.3.2.6: "A line count of -999 means 'never print [MORE]'."
/// Also the floor §8.8.3.2.2.3 clamps to ("A line count is never decremented
/// below -999"), so once a window reaches the sentinel it stays there.
pub const NEVER_MORE: i16 = -999;

impl ZWindow {
    /// ZMSD §8.8.3.1 attribute 0 ("wrapping").
    pub fn wrapping(&self) -> bool {
        self.attributes & 0b0001 != 0
    }
    /// A flowing-PROSE window: wrapping AND scrolling, the pair that sends a v6
    /// window's output down the stream-1 text path rather than painting it at
    /// pixel coordinates (see `cpu::exec`'s print routing).
    pub fn prose_window(&self) -> bool {
        self.attributes & 0b0011 == 0b0011
    }
    /// ZMSD §8.8.3.1 attribute 2: "text copied to output stream 2 (the transcript,
    /// if selected)". A game that runs more than one prose window marks the one
    /// carrying the narrative with it — advent.z6 sets it on the window the player
    /// types into and clears it on the display window it opens above (SQ-0585).
    pub fn copy_to_transcript(&self) -> bool {
        self.attributes & 0b0100 != 0
    }
    /// Append streamed prose to this window's live line buffer (SQ-0585), starting
    /// a new logical line at each `\n` and dropping the oldest lines past
    /// [`PROSE_MAX_LINES`]. Wrapping is the host's job — it knows the font and the
    /// pane — so lines are stored logically, exactly as the host transcript keeps
    /// them.
    ///
    /// `at_col` is a COLUMN this window's own `set_cursor` declared for `s`
    /// (SQ-0729), in characters from the window's left margin — `None` when the
    /// text simply flowed on from the last print. A v6 prose window is still a
    /// pixel surface, and a game may place runs across it: fmvpoker prints its five
    /// menu labels into one at x = 1, 178, 372, 454 and 557, and dropping those
    /// columns butted the runs against each other as
    /// `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT`.
    ///
    /// The line is PADDED OUT TO the column, not indented BY it: the run has to
    /// land at the column the game named, not that far past wherever the previous
    /// run happened to end. A column already behind the line's end cannot be
    /// reached by appending, so it is ignored — a line buffer can only move right,
    /// and only the streaming shadow ([`ZWindow::record_streamed`]) can express a
    /// true backwards jump.
    ///
    /// `at_row` is that same `set_cursor`'s ROW, in text lines from the window's
    /// top, and is honoured the same way: the buffer is PADDED OUT TO it with blank
    /// lines, and a row already behind the buffer's end is ignored. fmvpoker needs
    /// both halves, and needed the row half the moment its story window became a
    /// canvas: it prints its menu bar and its CONTINUE button at `set_cursor(row=80,
    /// …)`, five text lines down its 156px bottom panel, and prints its running
    /// totals into WINDOW 0 at absolute y = 247 and 265 — which land in that panel's
    /// first two lines. Stacking the panel's own prose from its top edge put the two
    /// on the same rows, so the game's own layout collided with itself.
    pub fn push_prose(&mut self, s: &str, at_col: Option<usize>, at_row: Option<usize>) {
        // `row` is a 0-based line index, so the buffer needs `row + 1` lines for
        // the run to land ON it. A buffer already that long has moved past the
        // declaration and keeps its own last line.
        if let Some(row) = at_row {
            while self.prose.len() <= row {
                self.prose.push(String::new());
            }
        }
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 || self.prose.is_empty() {
                self.prose.push(String::new());
            }
            if let Some(last) = self.prose.last_mut() {
                // Only the first fragment sits at the declared column; everything
                // after a '\n' starts a line of its own at the left margin.
                if i == 0 {
                    if let Some(col) = at_col {
                        let have = last.chars().count();
                        last.extend(std::iter::repeat_n(' ', col.saturating_sub(have)));
                    }
                }
                last.push_str(part);
            }
        }
        if self.prose.len() > PROSE_MAX_LINES {
            let drop = self.prose.len() - PROSE_MAX_LINES;
            self.prose.drain(..drop);
        }
    }
    /// ZMSD §8.8.3.1 attribute 1 ("scrolling").
    pub fn scrolling(&self) -> bool {
        self.attributes & 0b0010 != 0
    }
    /// ZMSD §8.8.3.1 attribute 2 ("text copied to output stream 2 (the
    /// transcript, if selected)").
    pub fn scripting(&self) -> bool {
        self.attributes & 0b0100 != 0
    }
    /// ZMSD §8.8.3.1 attribute 3 ("buffered printing").
    pub fn buffered(&self) -> bool {
        self.attributes & 0b1000 != 0
    }

    /// Property 15 read as the signed number the spec talks about (window
    /// properties are "standard Z-machine numbers", i.e. signed 16-bit).
    pub fn line_count_signed(&self) -> i16 {
        self.line_count as i16
    }

    /// How many lines this window prints before "[MORE]" falls due — its
    /// height in text lines less one, matching frotz's `screen_new_line`
    /// threshold (`above + below - 1`). Degenerate (zero-height) windows
    /// report 1 rather than 0 so the count never starts already-due.
    pub fn more_interval(&self) -> i16 {
        let lines = (self.y_size / V6_FONT_HEIGHT) as i32;
        (lines - 1).clamp(1, i16::MAX as i32) as i16
    }

    /// One new-line happened in this window: ZMSD §8.8.3.2.2 "the line count
    /// is decremented on each new-line", §8.8.3.2.2.3 "A line count is never
    /// decremented below -999". The sentinel is sticky — a window the game
    /// parked at -999 to suppress "[MORE]" (§8.8.3.2.6) stays there.
    pub fn tick_line_count(&mut self) {
        let lc = self.line_count_signed();
        if lc == NEVER_MORE {
            return;
        }
        self.line_count = lc.saturating_sub(1).max(NEVER_MORE) as u16;
    }

    /// Reload the line count to a full window's worth of lines. Frotz does the
    /// equivalent (`line_count = 0`, counting the other way) for all eight
    /// windows whenever a keystroke actually arrives — see
    /// `console_read_input`/`console_read_key` — which is what stops the count
    /// drifting down to the -999 floor over a long game.
    pub fn reload_line_count(&mut self) {
        self.line_count = self.more_interval() as u16;
    }

    /// One new-line in the *scrolling prose* regime (v6 window 0, or an Inform
    /// v6 library's wrap+scroll main window): the cursor returns to the left
    /// margin and drops a line, except on the bottom line where the window
    /// scrolls under a stationary cursor. Mirrors frotz `screen_new_line`
    /// (`if (y_cursor + 2 * font_height - 1 > y_size) scroll else y_cursor +=
    /// font_height`), and ticks the line count (§8.8.3.2.2).
    ///
    /// The *paint* regime deliberately does not use this: painted text keeps
    /// running past the bottom of its window (runs are screen-absolute), so
    /// clamping there would move glyphs the games expect to stay put.
    pub fn prose_new_line(&mut self) {
        self.x_cursor = self.left_margin.saturating_add(1);
        if self.scrolling() {
            self.tick_line_count();
        }
        let fh = V6_FONT_HEIGHT as u32;
        if self.y_cursor as u32 + 2 * fh - 1 <= self.y_size as u32 {
            self.y_cursor += V6_FONT_HEIGHT;
        } else {
            // The window scrolled under a stationary cursor, so everything
            // already on screen moved up one line and the top line left the
            // window (SQ-0697). Move the shadow with it — a record that claimed
            // the old rows would freeze the prose in places it no longer is.
            let top = self.y_coord;
            for t in self.streamed.iter_mut() {
                t.y = t.y.saturating_sub(V6_FONT_HEIGHT);
            }
            self.streamed.retain(|t| t.y >= top);
        }
    }

    /// The screen-absolute `(y, x)` the window's cursor is at right now, in the
    /// same space [`ZWindow::stream_origin`] records — so the two compare directly.
    pub fn pen(&self) -> (u16, u16) {
        (
            self.y_coord.saturating_add(self.y_cursor).saturating_sub(1),
            self.x_coord.saturating_add(self.x_cursor).saturating_sub(1),
        )
    }

    /// Forget where the last burst of prose started (SQ-0804), so the next glyph
    /// this window streams records a fresh [`ZWindow::stream_origin`].
    pub fn clear_stream_origin(&mut self) {
        self.stream_origin = None;
    }

    /// Shadow one streamed glyph at the window cursor's current screen position
    /// (SQ-0697) — see [`ZWindow::streamed`]. Extends the run in progress when the
    /// glyph continues it in the same style at the next cell; starts a new one
    /// otherwise (a new line, a `set_cursor` jump, a style or colour change).
    pub fn record_streamed(&mut self, ch: char, style: u8, fg: ZColour, bg: ZColour) {
        // Window coords and cursors are both 1-based, so the absolute position is
        // origin + cursor - 1 (ZMSD §8.8.1/§8.8.3.2).
        let x = self.x_coord.saturating_add(self.x_cursor).saturating_sub(1);
        let y = self.y_coord.saturating_add(self.y_cursor).saturating_sub(1);
        // Where this burst of prose STARTED (SQ-0804) — see `stream_origin`.
        self.stream_origin.get_or_insert((y, x));
        if let Some(last) = self.streamed.last_mut() {
            let end = last.x as u32 + last.px_w();
            if last.y == y
                && end == x as u32
                && last.style == style
                && last.fg == fg
                && last.bg == bg
            {
                last.text.push(ch);
                return;
            }
        }
        // Insurance only: the scroll in `prose_new_line` already bounds this to a
        // screenful for any window that stays put, and a window that never
        // scrolls is one whose prose fits. A story that defeats both still can't
        // grow the buffer without bound.
        if self.streamed.len() >= STREAMED_MAX_RUNS {
            self.streamed.drain(..self.streamed.len() - STREAMED_MAX_RUNS / 2);
        }
        self.streamed.push(V6Text { y, x, text: ch.to_string(), style, fg, bg });
    }

    /// Hand the shadowed prose over to real paint (SQ-0697): the window's box is
    /// about to become `(x, y, w, h)`, and what it already printed does not move
    /// with it (ZMSD §15, "does not change the current display").
    ///
    /// Only the prose the NEW box no longer covers is frozen. That distinction is
    /// the whole safety of this feature. Arthur resizes and moves window 0 around
    /// its narration on almost every turn — the box changes, but it still covers
    /// the text, which is therefore still the window's own live content and still
    /// belongs to the streaming transcript. Shogun's title header is the other
    /// case: window 0 drops from the full 640x400 screen to a 548x64 box at the
    /// bottom, leaving nine lines of header stranded at rows the window no longer
    /// reaches. Freezing on the box CHANGING rather than on the box LEAVING froze
    /// Arthur's prose seven turns running and stacked its stale prompts up as paint.
    ///
    /// Returns `true` when anything was frozen, which is the host's cue to restart
    /// its transcript at the window's new origin.
    pub fn retire_streamed(&mut self, x: u16, y: u16, w: u16, h: u16) -> bool {
        if self.streamed.is_empty() {
            return false;
        }
        let (left, top) = (x.max(1) as i32, y.max(1) as i32);
        let (right, bottom) = (left + w as i32, top + h as i32);
        let mut kept = Vec::with_capacity(self.streamed.len());
        let mut froze = false;
        for run in std::mem::take(&mut self.streamed) {
            let rx = run.x.max(1) as i32;
            let ry = run.y.max(1) as i32;
            let covered = rx >= left
                && ry >= top
                && rx + run.px_w() as i32 <= right
                && ry + V6_FONT_HEIGHT as i32 <= bottom;
            if covered {
                kept.push(run);
            } else {
                self.retired.push(run);
                froze = true;
            }
        }
        self.streamed = kept;
        froze
    }

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
    ///
    /// 16/17 fall in that ignored range on purpose: §8.8.3.2 ends "The true
    /// foreground and true background properties must not be written by
    /// put_wind_prop." They are read-derived from the window's channels in the
    /// `get_wind_prop` arm instead.
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
            // Every layer that records where glyphs are SITTING: what the window
            // is showing now, the prose it left frozen behind when it moved
            // (SQ-0697), and the prose it is still streaming (SQ-0729 — the
            // shadow of the live stream, which is the same pixels seen from the
            // other side). The erase covers pixels, and does not care which layer
            // put them there: fmvpoker erases its bottom window and reprints the
            // running total there, and a shadow that kept the old figure would
            // have "990" standing on the tail of "1000".
            for layer in [&mut win.texts, &mut win.retired, &mut win.streamed] {
                if layer.iter().any(|t| {
                    let ty = t.y as i32;
                    let tx = t.x as i32;
                    ty + (V6_FONT_HEIGHT as i32) > top
                        && ty < top + h
                        && tx + (t.px_w() as i32) > left
                        && tx < left + w
                }) {
                    let old = std::mem::take(layer);
                    *layer =
                        old.into_iter().flat_map(|t| trim_run_against_rect(t, top, left, h, w)).collect();
                }
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
    /// The v6 window the game last asked for INPUT through, when that window was a
    /// flowing-prose one (SQ-0585). It is the game's main text window by definition
    /// — the one the player types into — so its output is what the host mirrors as
    /// the transcript. Any OTHER prose window is a display panel, and its text goes
    /// to that window's own `prose` buffer instead of being spliced into the same
    /// stream. `0` until the first input request, which is right for boot: window 0
    /// is the classic main window, and text printed before any read (the banner)
    /// belongs to the transcript.
    ///
    /// Lives on `ScreenState`, not `Machine`, for the same reason as `current_fg`/
    /// `current_bg`: it is an input to what the screen must show, and archiving the
    /// whole `ScreenState` (SQ-0749) is what carries it through a host Save State —
    /// a restore taken mid-read through a secondary panel used to come back with
    /// this at 0, so the panel's typed-input echo went dark until the next read
    /// re-established it.
    pub v6_input_window: u8,
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
            v6_input_window: 0,
        }
    }
}

/// Is ZMSD §8.3's **Amiga rule** in force for this story? (SQ-0740)
///
/// §8.3 gives each Version 6 window its own foreground/background pair, and then
/// carves out one machine:
///
/// > "Note that a Version 6 interpreter going under the Amiga interpreter number
/// > must use the same pair of colours for all windows when running Infocom's
/// > games. If either is changed, then the interpreter must change the colour of
/// > all text on the screen to match. This simulates the Amiga hardware, which
/// > used two logical colours for text and switched palette to change their
/// > physical colour. This behaviour should not occur when running non-Infocom
/// > games, and modern games should never expect it. An interpreter that does
/// > not wish to handle this behaviour at all should avoid using the Amiga
/// > interpreter number when running Infocom's Version 6 games."
///
/// The test is [`machine_rule`] asked of
/// [`MachineProfile::global_colour_pens`](crate::interpreter::MachineProfile::global_colour_pens),
/// which the Amiga row sets and no other does — Version 6, colours available,
/// and `$1E` naming the machine, every term read back out of the HEADER so the
/// rule survives a `@restart`, a `@restore` and a host Save State without anybody
/// carrying it.
///
/// **"When running Infocom's games."** §8.3.1.1 asks the same question of the
/// palette knob — an interpreter may substitute its own colour values "if and
/// only if they can detect they are running an original Infocom story file" —
/// and gives no mechanism. lanthorn answers both the same way, because the same
/// thing answers them: interpreter 4 is only ever advertised by
/// [`InterpreterProfile::Amiga`](../../app/interpreter/enum.InterpreterProfile.html),
/// which is selected by an Amiga release floppy, by a native Amiga `Pic.data`
/// archive, or by the player naming the number outright — and Infocom is the only
/// publisher who ever shipped a Version 6 story on Amiga media. The third route
/// is the player asking for an Amiga, which is the standard's own framing: the
/// escape hatch it offers is to "avoid using the Amiga interpreter number", so
/// choosing it *is* the opt-in.
pub fn amiga_global_colour_pair(mem: &Memory) -> bool {
    machine_rule(mem, |m| m.global_colour_pens)
}

/// The shared shape of every per-machine v6 screen rule: Version 6, colours
/// available, and the machine header `$1E` names claims the rule (SQ-0872).
///
/// Each term is read back out of the HEADER rather than held as a field, which
/// is what makes the rules survive a `@restart`, a Quetzal `@restore` and a host
/// Save State without anybody carrying them:
///
/// - **Version 6** — every rule here is scoped to a Version 6 screen, and below
///   v6 there is one screen pair anyway.
/// - **The machine** ($1E) — the byte the story reads to decide what it is
///   running on, so it is exactly the condition the standard names. `claims`
///   picks which member of [`crate::interpreter::MachineProfile`] is being asked
///   about; a number no row models answers `false`, never a substitute.
/// - **Colours available** (Flags 1 bit 0, §8.3.2/§8.3.3) — with
///   `honor_game_colours` off lanthorn declares itself colourless, the host theme
///   owns the screen, and there is no pair for the windows to share.
fn machine_rule(mem: &Memory, claims: fn(&crate::interpreter::MachineProfile) -> bool) -> bool {
    mem.version() == 6
        && mem.read_byte(0x01) & 0x01 != 0
        && crate::interpreter::machine_of(mem).is_some_and(claims)
}

/// ZMSD §11.1.3's interpreter number for the Amiga, from the machine table.
pub use crate::interpreter::AMIGA_INTERPRETER_NUMBER;

/// The pair of pens the whole screen is painted with under
/// [`amiga_global_colour_pair`], as `(foreground, background)`; `None` when the
/// rule is not in force. (SQ-0740)
///
/// This is the pair the machine BOOTS with, and — because Infocom's window-0 gate
/// means Journey's only `set_colour` never lands — the pair Journey is played on:
/// header bytes `$2D`/`$2C`, which under the Amiga row carry `DEF_FORE 9` (white)
/// over `DEF_BACK 12` (**dark** grey `$444`), read out of the release floppies'
/// own interpreters rather than out of `amiga/yzip.h` — see
/// [`crate::interpreter::AMIGA_DEFAULT_BACKGROUND`] for the disassembly (SQ-0822).
///
/// **Why the host needs it, and why §8.3.3 alone was not enough.** Those two
/// bytes are what §8.3.3 tells the *story* about the interpreter's defaults, and
/// lanthorn wrote them faithfully — but nothing ever PAINTED them, so a v6 window
/// left at [`ZColour::Default`] rendered in the host terminal's theme and an Amiga
/// looked exactly like an IBM PC on screen. On the real machine the two registers
/// are not advice, they are the screen: every pixel no picture and no `set_colour`
/// claimed is the background pen. Returning the pair here lets the host paint the
/// page it has been advertising all along.
///
/// A window-0 `set_colour` still wins over this wherever the game made one — it
/// moves the pens, and the model carries the moved pair on the window itself
/// (Zork Zero's black-on-light-grey page). This is the ground beneath that.
pub fn amiga_screen_pair(mem: &Memory) -> Option<(ZColour, ZColour)> {
    amiga_global_colour_pair(mem).then(|| crate::interpreter::header_pair(mem))
}

/// The MACHINE's own screen pair for a Version 6 frame, `(foreground,
/// background)` — the ground every window that names no colour of its own is read
/// on — or `None` on a machine that has no such thing (SQ-0846, SQ-0872).
///
/// Two machines answer, and they answer for the same reason: their §8.3.3 default
/// colours are not advice about a terminal, they are the screen.
/// [`MachineProfile::v6_screen_page`](crate::interpreter::MachineProfile::v6_screen_page)
/// is the flag, and both pairs arrive here the same way — through header bytes
/// `$2D`/`$2C`, which is where §8.3.3 already had the interpreter publishing them
/// to the story.
///
/// - The **Amiga** (interpreter 4), for §8.3's own reason: one pair of pens for
///   the whole screen, shared and unmoving. [`amiga_screen_pair`] carries the full
///   rule, because on that machine the pens also govern `set_colour`; this is the
///   strictly weaker half, and the Amiga answers both.
/// - The **Macintosh** (interpreter 3), for a plainer one: a white page under
///   black ink was what a Mac window WAS, and `mac/xzip.lst` states it outright
///   (`SetColor := (zWHITE*256) + zBLACK; { Mac defaults: white under black }`).
///   Nothing about the pens is claimed — a Mac `set_colour` behaves exactly as it
///   did — only the ground beneath a window that asked for nothing.
///
/// **This is the function that used to be two.** The Amiga's half lived here and
/// the Macintosh's in `app::session::machine_screen_pair`, gated on a constant
/// `blorb` happened to carry — one concept in two crates, and the reason `zvm-cli`
/// could see one machine's rule and not the other's (SQ-0872). Which machines
/// answer is now a column of the table rather than a chain of `if`s, so adding a
/// third is a `true` and not a new function.
///
/// **This is what SQ-0846 was reported as**, on `stories/Zork Zero Disk.image`
/// (release 296, serial 881019): the status banner's location and score text came
/// out grey on the white artwork and read as missing. Zork Zero on the Macintosh
/// **never calls `set_colour` at all**, so with nothing painting `$2C`/`$2D` the
/// ink fell all the way through to the host theme's grey, while the white it sat
/// on was the game's own two-colour plate. That is SQ-0740's Amiga finding
/// exactly, one machine later.
///
/// The colour bit (`$01` bit 0) gates every arm, which is what makes
/// `honor_game_colours = false` a no-op here: a colourless interpreter is never
/// given the machine's pair to publish, and the header then carries zvm's own
/// §8.3.2 seed, which is nobody's machine.
pub fn machine_screen_pair(mem: &Memory) -> Option<(ZColour, ZColour)> {
    machine_rule(mem, |m| m.v6_screen_page).then(|| crate::interpreter::header_pair(mem))
}

impl ScreenState {
    /// Apply a `set_colour` under [`amiga_global_colour_pair`]: move the
    /// machine's two text "pens" and repaint the screen through them. (SQ-0740)
    ///
    /// `fg`/`bg` are the decoded channel requests (`None` = "leave this channel
    /// alone", the opcode's 0 sentinel). `fg_under_cursor`/`bg_under_cursor` flag
    /// the §8.3.1 colour **-1**, "the colour of the pixel under the cursor".
    ///
    /// **The window-0 gate, and why it beats the letter of §8.3.** Infocom's own
    /// Amiga interpreter (`amiga/yzip3.c`) says the two text colours "are now
    /// 'global', meaning they *can't* be changed for a single word on the screen,
    /// or for a certain window", and then states the rule outright: "we allow text
    /// colors to be changed only in window 0, and ignore requests in other windows
    /// (except for the special case of bg = -1)". §8.3 does not mention that gate.
    /// It is nonetheless what this implements, for two reasons that point the same
    /// way:
    ///
    /// - §8.3's stated purpose is to *"simulate the Amiga hardware"*. A reading of
    ///   it that makes lanthorn diverge from that hardware defeats the rule's own
    ///   reason for existing, and Infocom's shipped interpreter is the better
    ///   authority on how Infocom's own games looked on it.
    /// - It is what the machine actually did. `Journey - The Quest Begins.adf`
    ///   (release 30, serial 890322) makes exactly one `set_colour(9, 2)` — white
    ///   ink, black page — and makes it on **window 3**. Applying it globally
    ///   paints Journey black; contemporary Amiga walkthrough material shows the
    ///   game on its *default* pair instead, light grey page with white text,
    ///   which is `DEF_BACK 11` / `DEF_FORE 9` from Infocom's released
    ///   `amiga/yzip.h`. The real machine ignored the call, exactly as yzip3.c
    ///   says it would.
    ///
    /// If a later reader is tempted to "fix" this back to the letter of the
    /// standard: that is the change, and this is why it was not made.
    ///
    /// **Why -1 is not a pen move.** Infocom's carve-out for `bg = -1` is not an
    /// exception to the sharing rule, it is a different request altogether —
    /// "-1" names no colour,
    /// so there is nothing to load into a register; it asks for the glyph to be
    /// drawn *over what is already there*. Zork Zero prints its banner labels
    /// under `COLOR 2 -1` precisely so the ribbon art shows through them, and
    /// loading a real background there would paint opaque boxes over the art. So
    /// a -1 channel stays a per-window paint request, exactly as it is on every
    /// other machine, and leaves the pens where they were.
    ///
    /// **What "change the colour of all text on the screen" rewrites.** Every
    /// glyph the v6 screen model holds — the character grids, the pixel-positioned
    /// runs (`texts`), the prose a window has streamed (`streamed`) and the prose
    /// it has left frozen behind it (`retired`) — takes the new pens, along with
    /// every window's own pair and the current pair the prose stream tags its runs
    /// from. The colours are REWRITTEN rather than resolved late on purpose: on
    /// the Amiga this is a hardware repaint with no Z-machine event behind it, so
    /// after it happens the model must simply *be* the new screen. Every consumer
    /// — the render path, `/dump-windows`, a host Save State — then sees one
    /// truth without knowing this rule exists.
    ///
    /// **Except a transparent background**, which is not repainted because it was
    /// never painted: a glyph whose stored background is [`ZColour::Default`] put
    /// nothing behind itself (that is how -1 and an inherited channel render), so
    /// there is no pen-0 pixel on the screen for a register move to reach. Its
    /// foreground still follows, which is the whole of the reported symptom — the
    /// prose that stayed white while the game had asked for black.
    ///
    /// The same rule governs a WINDOW's own background, and it has to: a window's
    /// `bg` is the page the renderer fills its box with, and Journey never gives
    /// windows 0–2 one — they are transparent over the illustration behind them.
    /// Loading black into all four anyway paints the frame's own picture out of
    /// existence (measured on `Journey - The Quest Begins.adf`, release 30, serial
    /// 890322: a 115×61 hybrid frame collapsed from 730 distinct cell styles to 8).
    /// Nothing was ever drawn in pen 0 there, so there is nothing for the pen to
    /// change — one rule, applied to glyphs and to the pages under them alike.
    pub fn set_amiga_colour_pair(
        &mut self,
        win: u8,
        fg: Option<ZColour>,
        bg: Option<ZColour>,
        fg_under_cursor: bool,
        bg_under_cursor: bool,
    ) {
        if self.v6.is_none() {
            return;
        }
        // 0. **Infocom's window-0 gate.** `amiga/yzip3.c`, above `set_color`:
        //    "We allow text colors to be changed only in window 0, and ignore
        //    requests in other windows (except for the special case of bg = -1)."
        //    A request from any other window is dropped whole — it moves no pen and
        //    does not reach the window that made it. See the doc comment for why
        //    Infocom's interpreter outranks the letter of §8.3 here.
        if win != 0 && !bg_under_cursor {
            return;
        }
        // 1. The window that asked takes the request whole, exactly as it would on
        //    any other machine — including a -1 channel, which is how it says
        //    "draw over what is already here" and is the one thing the sharing
        //    rule must not take away from it.
        let mut mirror = None;
        if let Some(v6) = self.v6.as_mut() {
            let current = v6.current;
            if let Some(w) = v6.windows.get_mut(win as usize) {
                if let Some(c) = fg {
                    w.fg = c;
                }
                if let Some(c) = bg {
                    w.bg = c;
                }
                w.colour_data = crate::cpu::exec::pack_colour_data(w.fg, w.bg);
                if win == current {
                    mirror = Some((w.fg, w.bg));
                }
            }
        }
        // 2. §8.3: whatever the request moved the PENS to is shared by every
        //    window, and the text already on the screen changes with it. A -1
        //    channel names no colour, so it moves no pen (see the doc comment) —
        //    and neither does a request from a window other than 0, which reaches
        //    here only through Infocom's `bg = -1` exception and is that window's
        //    own transparency, not a register move.
        let pen_fg = if fg_under_cursor || win != 0 { None } else { fg };
        let pen_bg = if bg_under_cursor || win != 0 { None } else { bg };
        if pen_fg.is_some() || pen_bg.is_some() {
            self.repaint_amiga_pens(pen_fg, pen_bg);
        }
        // 3. The prose stream tags its runs from the current pair, which follows
        //    the current window exactly as it does on every other machine.
        if let Some((fg, bg)) = mirror {
            self.current_fg = fg;
            self.current_bg = bg;
        }
    }

    /// The repaint half of [`Self::set_amiga_colour_pair`]: load the pens that
    /// MOVED and change every glyph already on the screen to match.
    ///
    /// Per channel, because a `set_colour` moves one, the other or both: the
    /// opcode's 0 sentinel means "leave this channel alone", and a channel left
    /// alone is a pen that has not moved and so has nothing to repaint. Reading a
    /// stationary channel back off the current pair instead is what silently
    /// dragged Zork Zero's light-grey page to transparent — the pair follows the
    /// current WINDOW, which is not where the pens live.
    fn repaint_amiga_pens(&mut self, fg: Option<ZColour>, bg: Option<ZColour>) {
        let Some(v6) = self.v6.as_mut() else { return };
        for w in v6.windows.iter_mut() {
            if let Some(fg) = fg {
                w.fg = fg;
            }
            if let (Some(bg), false) = (bg, matches!(w.bg, ZColour::Default)) {
                w.bg = bg;
            }
            w.colour_data = crate::cpu::exec::pack_colour_data(w.fg, w.bg);
            for c in w.grid.cells.iter_mut() {
                if let Some(fg) = fg {
                    c.fg = fg;
                }
                if let (Some(bg), false) = (bg, matches!(c.bg, ZColour::Default)) {
                    c.bg = bg;
                }
            }
            for t in w.texts.iter_mut().chain(w.streamed.iter_mut()).chain(w.retired.iter_mut()) {
                if let Some(fg) = fg {
                    t.fg = fg;
                }
                if let (Some(bg), false) = (bg, matches!(t.bg, ZColour::Default)) {
                    t.bg = bg;
                }
            }
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
    /// Everything routed to stream 2 while it was selected (ZMSD §7.1.2:
    /// stream 2 is "the game transcript"). Writing it to a FILE is a host
    /// concern the app does not implement (§7.6.5 lets an interpreter decline
    /// external files, and `output_stream 2` warns the player); the model
    /// still has to route text here so the routing is correct the day a file
    /// sink exists — in particular the v6 per-window "copy to stream 2"
    /// attribute (§8.8.3.1 attribute 2), which decides *which* windows'
    /// text a transcript would contain.
    stream2_buf: String,
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
            stream2_buf: String::new(),
        }
    }

    /// Append `s` to the transcript sink (see [`StreamState::stream2_buf`]).
    /// Callers gate this on stream 2 being selected AND — in v6 — on the
    /// printing window carrying attribute 2.
    pub fn write_stream2(&mut self, s: &str) {
        self.stream2_buf.push_str(s);
    }

    /// The transcript text accumulated so far.
    pub fn stream2_text(&self) -> &str {
        &self.stream2_buf
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
/// The bit meanings are ZMSD §11.1's "Flags 1" / "Flags 2" tables; the per-bit
/// reasoning lives beside each mask below. In outline:
///   - Flags1 (v1–3): clear "status line not available" and "variable-pitch
///     font default"; set "screen-splitting available". Bit 1 is the game's
///     status-line kind — left alone.
///   - Flags1 (v4+): advertise bold, italic, fixed-space and timed keyboard
///     input; advertise pictures for v6. Colour (bit 0) and sound (bit 5) are
///     capability-driven — see `advertise_colour` / `advertise_sound`.
///   - Flags2: these are the GAME's requests, so we only clear what we cannot
///     honour — menus (bit 8, `make_menu` is a stub). Font 3 / pictures (bit 3)
///     and mouse (bit 5) are provided and stay as the game left them; undo
///     (bit 4) is advertised for v5+; colour (bit 6) and sound (bit 7) are
///     capability-driven. Transcript (bit 0) and fixed-pitch (bit 1) are the
///     game's own state.
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
        // v4+ Flags1 bits (ZMSD §11.1, "Flags 1" Version 4+ table):
        //   bit 0: "Colours available?" (V5) — handled separately (advertise_colour)
        //   bit 1: "Picture displaying available?" (V6) — set for v6: pictures are
        //          implemented end to end (draw_picture/picture_data/erase_picture
        //          over the blorb Pict resources). Clear below v6, where the bit
        //          has no meaning.
        //   bit 2: "Boldface available?" — set (rendered via SGR / style spans)
        //   bit 3: "Italic available?" — set (rendered via SGR / style spans)
        //   bit 4: "Fixed-space style available?" — set
        //   bit 5: "Sound effects available?" (V6) — handled separately (advertise_sound)
        //   bit 7: "Timed keyboard input available?" — set: timed `read` and
        //          `read_char` (the time/routine operands) are implemented.
        let base = f1 | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 7);
        if version == 6 { base | (1 << 1) } else { base & !(1 << 1) }
    };
    mem.write_byte(0x01, new_f1);

    // Flags2 (word 0x10–0x11). ZMSD §11.1 "Flags 2": bits 3–8 are the GAME's
    // requests ("If set, game wants to use …"); the interpreter clears the ones
    // it cannot honour and otherwise leaves the request standing.
    //   bit 3: V5 = character-graphics font wanted, V6 = "game wants to use
    //          pictures". PRESERVE either way — §8.1.5.1 says only "an
    //          interpreter which cannot provide the character graphics font
    //          should clear bit 3", and we render font 3 (font3_translate) and
    //          v6 pictures both.
    //   bit 4: "game wants to use the UNDO opcodes" — set for v5+ (save_undo/
    //          restore_undo implemented); pre-v5 has no undo opcodes, so clear.
    //   bit 5: "game wants to use a mouse" — PRESERVE: mouse input is
    //          implemented (read_mouse / mouse_window, `Machine::set_mouse`,
    //          and the host delivers clicks).
    //   bit 6: "game wants to use colours" — handled separately (advertise_colour).
    //   bit 7: "game wants to use sound effects" — handled separately (advertise_sound).
    //   bit 8: "game wants to use menus" — CLEAR: `make_menu` (EXT:0x1B) is a
    //          stub that always branches false, so menus are not available.
    let f2 = mem.read_word(0x10);
    let mut new_f2 = f2 & !(1 << 8);
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

    // Interpreter version (0x1F). `b'A'` = 0x41 is the default and has NO
    // PROVENANCE: it arrived in this function's first commit beside `$1E`'s
    // "6, a common neutral value", which SQ-0872 has since replaced with a
    // sourced machine table, and it was never revisited. A story can PRINT this
    // byte — Shogun r295 renders it as a decimal, so 'A' shows as 65 — so
    // `set_interpreter_version` exists to override it while SQ-0885 works out
    // what each machine actually wrote.
    mem.write_byte(0x1F, interpreter_version().unwrap_or(b'A'));

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
    write_default_colours(mem, DEFAULT_BG_COLOUR, DEFAULT_FG_COLOUR);

    advertise_colour(mem, honor_game_colours);
    advertise_sound(mem, sound_available);
}

/// The palette a standard colour NUMBER resolves to actual colour through.
///
/// ZMSD §8.3.1.1 makes this an interpreter choice, not a fixed law: "The
/// equivalences between the colour numbers and true colours are *recommended*.
/// The interpreter may allow the user to change the mapping, but the given
/// values should be the default." A colour number is a name for "whatever this
/// machine shows for red", so a host claiming to *be* a particular machine owes
/// the game that machine's colours.
///
/// [`Palette::Standard`] is the §8.3.1 table verbatim and the default — it is
/// what every lanthorn session has always used. [`Palette::Amiga`] is the
/// sibling, for the Amiga interpreter profile (SQ-0719).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Palette {
    /// ZMSD §8.3.1's recommended true-colour table.
    #[default]
    Standard,
    /// The palette Infocom's own Amiga interpreter loaded.
    Amiga,
}

/// The active palette, as a raw discriminant for [`ACTIVE_PALETTE`].
const PALETTE_STANDARD: u8 = 0;
const PALETTE_AMIGA: u8 = 1;

/// The process-wide active palette.
///
/// Deliberately global rather than threaded through: the palette is a property
/// of *the machine lanthorn is pretending to be*, and there is exactly one of
/// those per run. Every consumer — the VM's own `true_value` (window properties
/// 17/18), the terminal cell renderer, the v6 pixel renderer and the CLI's SGR
/// path — must agree on it or one game colour would look like two different
/// colours on the same screen, so a single source beats four parameters
/// threaded through four unrelated call chains. Set once at boot from the
/// interpreter profile (`app::interpreter`), and re-asserted on every story
/// boot so a picker→play loop cannot carry one story's machine into the next.
static ACTIVE_PALETTE: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(PALETTE_STANDARD);

/// The interpreter version byte for header `$1F`, or `NO_VERSION` for "unset"
/// — a `u16` so the sentinel can sit outside the byte's own range.
const NO_VERSION: u16 = 0x100;
static INTERPRETER_VERSION: core::sync::atomic::AtomicU16 =
    core::sync::atomic::AtomicU16::new(NO_VERSION);

/// Override the interpreter version written into header `$1F`, process-wide.
///
/// Global for the same reason [`set_palette`] is, and it is the same KIND of
/// fact: the byte is a property of the machine lanthorn is pretending to be, and
/// there is exactly one of those per run. It cannot be a session parameter
/// because `GameSession`'s constructor runs the story to its first input, so the
/// header has to be right before construction returns — and threading a
/// twelfth positional argument through an eleven-argument constructor and its
/// fifteen call sites to carry a debugging knob is a worse trade than this.
///
/// `None` restores the default, which is what every ordinary run uses.
///
/// # Why it is worth overriding (SQ-0885)
///
/// The default `b'A'` has no provenance — see [`init_header_caps`] — and the
/// byte is one a story can PRINT. *Shogun* release 295 renders it as a decimal,
/// so `'A'` (65) makes its Amiga banner read "version 6.65" where the original
/// read "version 6.8". Whether a story also BRANCHES on it is unknown and is
/// exactly what this exists to find out: set it, run the game, watch.
pub fn set_interpreter_version(v: Option<u8>) {
    let raw = v.map_or(NO_VERSION, u16::from);
    INTERPRETER_VERSION.store(raw, core::sync::atomic::Ordering::Relaxed);
}

/// The interpreter version override, or `None` when no one has set one.
pub fn interpreter_version() -> Option<u8> {
    match INTERPRETER_VERSION.load(core::sync::atomic::Ordering::Relaxed) {
        NO_VERSION => None,
        v => Some(v as u8),
    }
}

/// Select the palette standard colour numbers resolve through, process-wide.
pub fn set_palette(p: Palette) {
    let v = match p {
        Palette::Standard => PALETTE_STANDARD,
        Palette::Amiga => PALETTE_AMIGA,
    };
    ACTIVE_PALETTE.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// The palette standard colour numbers currently resolve through.
pub fn palette() -> Palette {
    match ACTIVE_PALETTE.load(core::sync::atomic::Ordering::Relaxed) {
        PALETTE_AMIGA => Palette::Amiga,
        _ => Palette::Standard,
    }
}

/// Interpreter default background colour written to header $2C when the host
/// never says otherwise: 2 = black (ZMSD §8.3.1).
pub const DEFAULT_BG_COLOUR: u8 = 2;
/// Interpreter default foreground colour written to header $2D when the host
/// never says otherwise: 9 = white (ZMSD §8.3.1).
pub const DEFAULT_FG_COLOUR: u8 = 9;

/// Clamp a host-supplied default colour to a standard colour number a machine
/// of this `version` could actually have as its own default.
///
/// ZMSD §8.3.1 defines 2..=9 as the true colour names; 0/1 are the "current"/
/// "default" sentinels, 13–14 reserved and 15 transparent — none of which are
/// meaningful as *the interpreter's own* default, so they fall back.
///
/// The greys 10–12 exist "only in Version 6", and there they are perfectly
/// meaningful defaults: Infocom's own Amiga Version 6 interpreter booted with
/// `DEF_BACK 12` — dark grey — as its default page (SQ-0822). So they
/// are accepted for Version 6 and rejected everywhere else, which is exactly
/// what the spec's "only in Version 6" says. (SQ-0719; before that nothing ever
/// offered a grey here, so no existing session's answer moves.)
pub(crate) fn clamp_default_colour(c: u8, fallback: u8, version: u8) -> u8 {
    let ok = (2..=9).contains(&c) || (version == 6 && (10..=12).contains(&c));
    if ok { c } else { fallback }
}

/// Write the interpreter's default background/foreground colours into header
/// bytes $2C and $2D (V5+ only; those bytes have no meaning before V5).
///
/// ZMSD §8.3.3: "If the interpreter can produce colours, it should set bit 0 of
/// 'Flags 1' in the header, and write its default background and foreground
/// colours into bytes $2c and $2d of the header." (§8.3.2 asks a non-colour
/// interpreter for 2 and 9 "either way round", which the 2/9 default satisfies.)
/// Values outside 2..=9 fall back to [`DEFAULT_BG_COLOUR`]/[`DEFAULT_FG_COLOUR`].
pub fn write_default_colours(mem: &mut Memory, bg: u8, fg: u8) {
    if mem.version() < 5 {
        return;
    }
    let bg = clamp_default_colour(bg, DEFAULT_BG_COLOUR, mem.version());
    let fg = clamp_default_colour(fg, DEFAULT_FG_COLOUR, mem.version());
    mem.write_byte(0x2C, bg);
    mem.write_byte(0x2D, fg);
    write_header_ext_colours(mem, bg, fg);
}

/// The true-colour equivalent of standard colour number `n` (2..=12), as a
/// 15-bit RGB value. `None` for the sentinels (0 current, 1 default, -1
/// pixel-under-cursor), the reserved 13/14 and 15 (transparent, which §8.3.7
/// gives the special value -4 rather than an RGB triple).
///
/// Resolved through the [`palette`] the host has selected: [`Palette::Standard`]
/// (the default) is the spec table below verbatim; [`Palette::Amiga`] is the
/// palette Infocom's own Amiga interpreter loaded, which §8.3.1.1 explicitly
/// permits an interpreter to substitute.
pub fn standard_true_colour(n: u8) -> Option<u16> {
    match palette() {
        Palette::Standard => zmsd_true_colour(n),
        Palette::Amiga => amiga_true_colour(n),
    }
}

/// The Amiga palette for standard colour numbers 2..=12, as 15-bit RGB.
///
/// Source: the `colortable[]` compiled into the Amiga Version 6 interpreter on
/// Infocom's own **release floppies** — the one program that ever painted these
/// colours — read through `colormap[]`, which maps a Z-machine colour number to a
/// table slot. The Amiga's `SetRGB4` takes 4 bits per channel, so the raw entries
/// are `0x0RGB`; they are widened to the Z-machine's 5-bit channels by bit
/// replication (`n << 1 | n >> 3`), the expansion that keeps `$F` at full
/// intensity and `$0` at zero.
///
/// ```text
/// colour        slot            raw     → 15-bit
/// 2  black      colortable[2]  $0000     $0000
/// 3  red        colortable[4]  $0E00     $001D
/// 4  green      colortable[3]  $00C0     $0320
/// 5  yellow     colortable[5]  $0FD0     $037F
/// 6  blue       colortable[0]  $005A     $5540
/// 7  magenta    colortable[6]  $0F0F     $7C1F
/// 8  cyan       colortable[7]  $00EE     $77A0
/// 9  white      colortable[1]  $0FFF     $7FFF
/// 10 light grey colortable[8]  $0AAA     $56B5
/// 11 medium     colortable[9]  $x777     $39CE
/// 12 dark grey  colortable[10] $0444     $2108
/// ```
///
/// (Slot 9 is written `0x7777`; `SetRGB4` uses only the low 12 bits, so the stray
/// high nibble never reached the hardware.)
///
/// **The floppies outrank `amiga/yzip1.c`** (SQ-0822). The leaked development
/// source gives slot 5 as `0x0EE0`, and it is the ONE entry where the source and
/// the shipped program disagree: the byte string
/// `00 5A 0F FF 00 00 00 C0 0E 00 0F D0 0F 0F 00 EE 0A AA 77 77 04 44` appears,
/// identically and once, in every Amiga Version 6 interpreter in `stories/` —
/// `Arthur` (release 54 floppy), `Journey` (release 30), `Zork Zero` (release 366)
/// and `Shogun` (release 295) — with `0F D0` where the source has `0E E0`. Ten
/// entries match the source exactly, which is what makes the eleventh a shipped
/// correction rather than a mis-transcription.
///
/// Five of the eleven entries — black, red, magenta, cyan, white — come out
/// bit-for-bit identical to ZMSD §8.3.1's table, which is a strong sign the
/// standard's "recommended" values were themselves read off an Amiga. Green,
/// blue, yellow and the three greys are where the two genuinely differ.
pub fn amiga_true_colour(n: u8) -> Option<u16> {
    Some(match n {
        2 => 0x0000,  // black       $000
        3 => 0x001D,  // red         $E00
        4 => 0x0320,  // green       $0C0
        5 => 0x037F,  // yellow      $FD0
        6 => 0x5540,  // blue        $05A
        7 => 0x7C1F,  // magenta     $F0F
        8 => 0x77A0,  // cyan        $0EE
        9 => 0x7FFF,  // white       $FFF
        10 => 0x56B5, // light grey  $AAA [V6 only]
        11 => 0x39CE, // medium grey $777 [V6 only]
        12 => 0x2108, // dark grey   $444 [V6 only]
        _ => return None,
    })
}

/// ZMSD §8.3.1's true-colour table for standard colour numbers 2..=12, as
/// 15-bit RGB. The spec's own table, transcribed verbatim; §8.3.1.1 calls these
/// equivalences "recommended" and the interpreter default, and they are
/// lanthorn's default too ([`Palette::Standard`]).
pub fn zmsd_true_colour(n: u8) -> Option<u16> {
    Some(match n {
        2 => 0x0000,  // black
        3 => 0x001D,  // red
        4 => 0x0340,  // green
        5 => 0x03BD,  // yellow
        6 => 0x59A0,  // blue
        7 => 0x7C1F,  // magenta
        8 => 0x77A0,  // cyan
        9 => 0x7FFF,  // white
        10 => 0x5AD6, // light grey  [V6 only]
        11 => 0x4631, // medium grey [V6 only]
        12 => 0x2D6B, // dark grey   [V6 only]
        _ => return None,
    })
}

/// Publish the interpreter's side of the header extension table (ZMSD §11.1.7.3):
/// word 4 = Flags 3, word 5 = true default FOREGROUND, word 6 = true default
/// BACKGROUND (note the fg-before-bg order — the reverse of $2C/$2D).
///
/// All three are marked "Int" and "Rst" in the §11.1.7.3 table, i.e. written by
/// the interpreter and re-stamped on restart/restore, which is why this rides
/// along with every `write_default_colours`.
///
/// Flags 3 is cleared outright: §11.1.7.4 — "The bits in Flags 3 are set by the
/// game to request use of a feature. If the interpreter cannot provide a
/// feature, it must clear the relevant bit" — and §11.1.7.4.1 — "All unused bits
/// in Flags 3 must be cleared by the interpreter." Its only defined bit is 0
/// ("game wants to use transparency"), which we do not provide (§8.3.6 lets a
/// non-transparent interpreter ignore colour 15), so every bit goes to 0.
///
/// Writes are skipped for any word past the table's length, per §11.1.7.2: "If
/// the interpreter needs to write a word which is beyond the length of the
/// extension table, or the extension table doesn't exist at all, then the result
/// is that nothing happens."
fn write_header_ext_colours(mem: &mut Memory, bg: u8, fg: u8) {
    let ext = mem.read_word(0x36) as u32;
    if ext == 0 {
        return;
    }
    let count = mem.read_word(ext); // word 0 = number of further words
    if count >= 4 {
        mem.write_word(ext + 8, 0); // word 4: Flags 3 — no features provided
    }
    if count >= 5 {
        let true_fg = standard_true_colour(fg).unwrap_or(0x7FFF);
        mem.write_word(ext + 10, true_fg); // word 5: true default foreground
    }
    if count >= 6 {
        let true_bg = standard_true_colour(bg).unwrap_or(0x0000);
        mem.write_word(ext + 12, true_bg); // word 6: true default background
    }
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

/// Upper bound on any character-grid dimension a story operand can request
/// (`split_window`, EXT `window_size`). A hostile/buggy story passing 0xFFFF
/// would otherwise force a rows×cols cell allocation in the hundreds of
/// megabytes — an OOM abort, where the VM promises graceful faults. 1024
/// far exceeds any real terminal (a 4K screen at 8 px/cell is ~480×270
/// cells) yet caps worst-case storage at ~1M cells per window.
pub const GRID_CELL_CAP: u16 = 1024;

/// Cap on [`ZWindow::prose`] (SQ-0585). A secondary prose window shows what is on
/// screen and nothing more — the tallest v6 screen is 400px, 25 text rows, and a
/// game that prints past its window's bottom without erasing has scrolled the
/// earlier lines off. Twice the tallest screen leaves room for that overshoot while
/// keeping a runaway printer from growing the buffer without bound.
pub const PROSE_MAX_LINES: usize = 50;

/// Cap on [`ZWindow::streamed`] (SQ-0697). The scroll in
/// [`ZWindow::prose_new_line`] already bounds the shadow to a screenful for any
/// window that stays put; this is the backstop for one that never scrolls. A run
/// per glyph on the tallest v6 screen (25 rows of 80 cells) is 2000, so four
/// times that is far past anything real while still bounded.
pub const STREAMED_MAX_RUNS: usize = 8000;

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
/// Does this story keep a clock rather than a score on the status line?
///
/// ZMSD §8.2.1: "In Versions 1 and 2, all games are 'score games'. In Version 3,
/// if bit 1 of 'Flags 1' is clear then the game is a 'score game'; if it is set,
/// then the game is a 'time game'." Flags 1 bit 1 only carries that meaning from
/// v3 on, so it must not be consulted below it. (Belt and braces today: the
/// header parser refuses to load a v1/v2 story at all — see
/// [`crate::header::parse_header`] — so the guard only matters if that ever
/// changes.)
fn is_time_game(version: u8, flags1: u8) -> bool {
    version >= 3 && (flags1 & (1 << 1)) != 0
}

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

    let time_mode = is_time_game(mem.version(), mem.read_byte(0x01));

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

    /// SQ-0804: `stream_origin` is where the FIRST glyph of a burst landed — set
    /// once and then left alone until it is cleared, so the host can compare it
    /// against the cursor the window had before the burst.
    #[test]
    fn stream_origin_records_the_first_glyph_of_a_burst_only() {
        let mut w = ZWindow { x_coord: 5, y_coord: 9, x_cursor: 3, y_cursor: 1, ..Default::default() };
        assert_eq!(w.stream_origin, None, "a window that has printed nothing has no origin");
        assert_eq!(w.pen(), (9, 7), "the pen is origin + cursor - 1, screen-absolute (§8.8.1)");

        w.record_streamed('a', 0, ZColour::Default, ZColour::Default);
        assert_eq!(w.stream_origin, Some((9, 7)), "the first glyph lands at the pen");

        // A second glyph, wherever it goes, must not move the origin.
        w.x_cursor += V6_FONT_WIDTH;
        w.record_streamed('b', 0, ZColour::Default, ZColour::Default);
        w.y_cursor += V6_FONT_HEIGHT;
        w.record_streamed('c', 0, ZColour::Default, ZColour::Default);
        assert_eq!(w.stream_origin, Some((9, 7)), "…and only the first");

        w.clear_stream_origin();
        assert_eq!(w.stream_origin, None, "cleared, ready for the next burst");
        w.record_streamed('d', 0, ZColour::Default, ZColour::Default);
        assert_eq!(w.stream_origin, Some(w.pen()), "the next burst records its own start");
    }

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

    // ── (b2) v1/v2 are always score games (§8.2.1) ───────────────────────────

    #[test]
    fn v1_v2_status_line_is_always_score() {
        // §8.2.1: "In Versions 1 and 2, all games are 'score games'" — the
        // Flags 1 bit 1 "time game" bit must not be consulted below v3.
        // (Tested on the predicate: `parse_header` refuses v1/v2 story files
        // outright, so no Memory can be built at those versions.)
        let flags1_with_time_bit = 1u8 << 1;
        assert!(!is_time_game(1, flags1_with_time_bit), "v1 is always a score game");
        assert!(!is_time_game(2, flags1_with_time_bit), "v2 is always a score game");
        assert!(is_time_game(3, flags1_with_time_bit), "v3 honours the bit");
        assert!(!is_time_game(3, 0), "v3 without the bit is a score game");
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
    fn header_caps_flags2_preserves_font3_and_picture_request() {
        // ZMSD §8.1.5.1: "In Version 5 (only), an interpreter which cannot
        // provide the character graphics font should clear bit 3 of 'Flags 2'."
        // We CAN provide font 3, so the game's request must survive. In V6 the
        // same bit is "game wants to use pictures" (§11.1) — also provided, so
        // also preserved. (This test previously pinned the opposite.)
        for v in [5u8, 6] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            let f2 = mem.read_word(0x10) | (1 << 3);
            mem.write_word(0x10, f2);
            init_header_caps(&mut mem, false, false, None);
            assert_ne!(
                mem.read_word(0x10) & (1 << 3),
                0,
                "v{v}: Flags2 bit 3 (font 3 / pictures wanted) must be preserved"
            );
        }
    }

    #[test]
    fn header_caps_flags1_advertises_timed_input_and_v6_pictures() {
        // ZMSD §11.1 "Flags 1" (Version 4+): bit 1 "Picture displaying
        // available?" (Version 6), bit 7 "Timed keyboard input available?".
        // Timed `read`/`read_char` and v6 pictures are both implemented, so both
        // must be advertised — they used to be cleared unconditionally.
        for v in [4u8, 5, 6, 7, 8] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            init_header_caps(&mut mem, false, false, None);
            let f1 = mem.read_byte(0x01);
            assert_ne!(f1 & (1 << 7), 0, "v{v}: Flags1 bit 7 (timed input) must be set");
            if v == 6 {
                assert_ne!(f1 & (1 << 1), 0, "v6: Flags1 bit 1 (pictures available) must be set");
            } else {
                assert_eq!(f1 & (1 << 1), 0, "v{v}: Flags1 bit 1 is a v6-only capability");
            }
        }
    }

    #[test]
    fn header_caps_flags2_preserves_mouse_request_and_clears_menus() {
        // ZMSD §11.1 "Flags 2": bit 5 "If set, game wants to use a mouse" —
        // preserved, mouse input is implemented (read_mouse / Machine::set_mouse
        // and the host delivers clicks). Bit 8 "If set, game wants to use menus"
        // — cleared, `make_menu` is a stub that always branches false.
        for v in [5u8, 6] {
            let mut mem = Memory::new(sample_story(v)).unwrap();
            mem.write_word(0x10, mem.read_word(0x10) | (1 << 5) | (1 << 8));
            init_header_caps(&mut mem, false, false, None);
            let f2 = mem.read_word(0x10);
            assert_ne!(f2 & (1 << 5), 0, "v{v}: Flags2 bit 5 (mouse wanted) must be preserved");
            assert_eq!(f2 & (1 << 8), 0, "v{v}: Flags2 bit 8 (menus wanted) must be cleared");
        }
    }

    #[test]
    fn header_caps_flags2_leaves_unrequested_bits_unset() {
        // The interpreter only ever CLEARS a game request it cannot honour; it
        // never invents one. With the game asking for nothing, bits 3 and 5 stay
        // clear (bit 4, UNDO, is the interpreter's own advertisement and is set).
        let mut mem = Memory::new(sample_story(5)).unwrap();
        mem.write_word(0x10, 0);
        init_header_caps(&mut mem, false, false, None);
        let f2 = mem.read_word(0x10);
        assert_eq!(f2 & (1 << 3), 0, "bit 3 not requested, not invented");
        assert_eq!(f2 & (1 << 5), 0, "bit 5 not requested, not invented");
        assert_ne!(f2 & (1 << 4), 0, "bit 4 (undo available) is ours to set");
    }

    #[test]
    fn write_default_colours_clamps_and_skips_pre_v5() {
        // ZMSD §8.3.3: the interpreter writes ITS default background ($2C) and
        // foreground ($2D). §8.3.1 only names 2..=9 as real colours, so anything
        // else falls back to black-on-white.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        write_default_colours(&mut mem, 6, 5);
        assert_eq!((mem.read_byte(0x2C), mem.read_byte(0x2D)), (6, 5), "valid pair lands as given");
        for bad in [0u8, 1, 10, 12, 15, 200] {
            write_default_colours(&mut mem, bad, bad);
            assert_eq!(
                (mem.read_byte(0x2C), mem.read_byte(0x2D)),
                (DEFAULT_BG_COLOUR, DEFAULT_FG_COLOUR),
                "colour {bad} is not a standard colour number — falls back to 2/9"
            );
        }
        // $2C/$2D are not colour bytes before V5.
        let mut mem3 = Memory::new(sample_story(3)).unwrap();
        mem3.write_byte(0x2C, 0x11);
        mem3.write_byte(0x2D, 0x22);
        write_default_colours(&mut mem3, 6, 5);
        assert_eq!((mem3.read_byte(0x2C), mem3.read_byte(0x2D)), (0x11, 0x22), "v3 untouched");
    }

    #[test]
    fn default_colours_publish_the_header_extension_words() {
        // ZMSD §11.1.7.3: word 4 = Flags 3, word 5 = true default FOREGROUND,
        // word 6 = true default BACKGROUND — all three marked "Int"/"Rst", so
        // the interpreter writes them alongside $2C/$2D.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        let ext: u32 = 0x0180; // dynamic memory
        mem.write_word(0x36, ext as u16);
        mem.write_word(ext, 6); // 6 further words
        mem.write_word(ext + 8, 0x0001); // game asked for transparency
        write_default_colours(&mut mem, 6, 5); // bg = blue, fg = yellow

        assert_eq!(mem.read_word(ext + 8), 0, "Flags 3 cleared — we provide none of its features");
        assert_eq!(
            mem.read_word(ext + 10),
            0x03BD,
            "word 5 = true default foreground (yellow, §8.3.1)"
        );
        assert_eq!(
            mem.read_word(ext + 12),
            0x59A0,
            "word 6 = true default background (blue, §8.3.1)"
        );
    }

    #[test]
    fn header_extension_writes_stop_at_the_table_length() {
        // ZMSD §11.1.7.2: writing past the table's length must do nothing.
        let mut mem = Memory::new(sample_story(5)).unwrap();
        let ext: u32 = 0x0180;
        mem.write_word(0x36, ext as u16);
        mem.write_word(ext, 4); // only 4 further words: Flags 3 is the last one
        mem.write_word(ext + 10, 0xDEAD);
        mem.write_word(ext + 12, 0xBEEF);
        write_default_colours(&mut mem, 6, 5);
        assert_eq!(mem.read_word(ext + 8), 0, "word 4 is in range and gets cleared");
        assert_eq!(mem.read_word(ext + 10), 0xDEAD, "word 5 out of range → untouched");
        assert_eq!(mem.read_word(ext + 12), 0xBEEF, "word 6 out of range → untouched");

        // No table at all → nothing happens (and no panic).
        let mut bare = Memory::new(sample_story(5)).unwrap();
        bare.write_word(0x36, 0);
        write_default_colours(&mut bare, 6, 5);
        assert_eq!(bare.read_byte(0x2C), 6, "the $2C/$2D half still lands");
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

    /// SQ-0885: `$1F` is overridable, and `None` restores the default.
    ///
    /// The byte has no provenance (see [`init_header_caps`]) and a story can
    /// PRINT it — Shogun r295 renders it as a decimal, so the default `'A'` (65)
    /// makes its Amiga banner read "version 6.65" against the real machine's
    /// "6.8". This is the knob that lets that be tried.
    ///
    /// The override is process-wide, so this restores it before returning; under
    /// `cargo test` (one process for the binary) a leak would reach every later
    /// case, and under nextest (a process per test) it would not — a difference
    /// that must not decide whether the suite passes.
    #[test]
    fn the_interpreter_version_byte_is_overridable() {
        let restore = interpreter_version();
        let byte_after = |v: Option<u8>| {
            set_interpreter_version(v);
            let mut mem = Memory::new(sample_story(5)).unwrap();
            init_header_caps(&mut mem, true, false, None);
            mem.read_byte(0x1F)
        };
        assert_eq!(byte_after(None), b'A', "the default, unchanged");
        assert_eq!(byte_after(Some(8)), 8, "the Amiga's own, per Shogun's banner");
        assert_eq!(byte_after(Some(0)), 0, "zero is a value, not 'unset'");
        assert_eq!(byte_after(None), b'A', "…and None restores the default");
        set_interpreter_version(restore);
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
        // ZMSD §8.3.1 fixes the true-colour value of each grey; expanding those
        // 15-bit values is what `grey_rgb` must return (it used to return an
        // invented #B0/#80/#50 ramp).
        assert_eq!(grey_rgb(10), rgb15_to_888(0x5AD6), "10 = light grey ($5AD6)");
        assert_eq!(grey_rgb(11), rgb15_to_888(0x4631), "11 = medium grey ($4631)");
        assert_eq!(grey_rgb(12), rgb15_to_888(0x2D6B), "12 = dark grey ($2D6B)");
        assert_eq!(grey_rgb(11), (0x8C, 0x8C, 0x8C));
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

    /// SQ-0679: when the HOST widens the grid, the columns that appear continue
    /// the appearance their row already ended in — so a status bar the game
    /// painted as a run of reverse-video spaces reaches the new right edge
    /// instead of stopping at the old one. Shrinking is still plain truncation,
    /// and a row that ended in default cells is byte-identical to before.
    #[test]
    fn widening_continues_each_rows_trailing_appearance() {
        let mut u = UpperWindow::default();
        u.resize(2, 4);
        // Row 1: a reverse-video bar with text in it, the whole row.
        for (c, ch) in " Hi ".chars().enumerate() {
            u.cells[c] = Cell { ch, style: 0x01, fg: ZColour::Default, bg: ZColour::Standard(4) };
        }
        // Row 2 is left entirely default.
        u.resize_continuing_row_style(2, 7);

        assert_eq!(u.cols, 7);
        assert_eq!((1..=4).map(|c| u.cell(1, c).ch).collect::<String>(), " Hi ", "old columns verbatim");
        for c in 5..=7 {
            let cell = u.cell(1, c);
            assert_eq!(cell.ch, ' ', "a grown column is blank space, never a copied glyph");
            assert_eq!(cell.style, 0x01, "…carrying the row's trailing style (col {c})");
            assert!(matches!(cell.bg, ZColour::Standard(4)), "…and its colours (col {c})");
        }
        for c in 5..=7 {
            let cell = u.cell(2, c);
            assert_eq!(cell.style, 0, "a default row grows default (col {c})");
            assert!(matches!(cell.bg, ZColour::Default));
        }

        // A shrink truncates and continues nothing.
        u.resize_continuing_row_style(2, 2);
        assert_eq!(u.cols, 2);
        assert_eq!((1..=2).map(|c| u.cell(1, c).ch).collect::<String>(), " H");
    }

    // ── ZMSD §8.3, the Amiga rule (SQ-0740) ──────────────────────────────────

    /// A story header describing `version` on interpreter `interp`, with colour
    /// either advertised or withdrawn (Flags 1 bit 0, §8.3.2/§8.3.3).
    fn header_for(version: u8, interp: u8, colour: bool) -> Memory {
        let mut mem = Memory::new(sample_story(version)).unwrap();
        mem.write_byte(0x1E, interp);
        advertise_colour(&mut mem, colour);
        mem
    }

    #[test]
    fn the_amiga_rule_is_read_out_of_the_header_and_nothing_else() {
        // §8.3 scopes it to "a Version 6 interpreter going under the Amiga
        // interpreter number", and §11.1.3 numbers the Amiga 4.
        assert!(amiga_global_colour_pair(&header_for(6, 4, true)), "v6 on interpreter 4");
        // Every other machine keeps the per-window model §8.3 gives it. 6 is the
        // IBM PC — the number the whole existing v6 corpus runs under, so this
        // row is the pin that says the corpus cannot move.
        for interp in [1u8, 2, 3, 5, 6, 7, 8, 9, 10, 11] {
            assert!(
                !amiga_global_colour_pair(&header_for(6, interp, true)),
                "v6 on interpreter {interp} keeps one pair per window",
            );
        }
        // Below Version 6 there is one screen pair anyway; the rule names v6.
        for v in [3u8, 4, 5, 7, 8] {
            assert!(!amiga_global_colour_pair(&header_for(v, 4, true)), "version {v}");
        }
        // …and with `honor_game_colours` off lanthorn declares itself colourless
        // (§8.3.2), so the host theme owns the screen and there is no pair to
        // share. The Amiga rule must not reach past that switch.
        assert!(
            !amiga_global_colour_pair(&header_for(6, 4, false)),
            "colours withdrawn: the theme owns the screen, Amiga or not",
        );
    }

    /// The pair the host PAINTS with, as opposed to the pair §8.3.3 advertises to
    /// the story: on the Amiga they are the same two bytes, and before SQ-0740
    /// lanthorn wrote them and then painted the terminal's colours instead.
    #[test]
    fn the_amiga_screen_pair_is_the_headers_own_default_colours() {
        let mut mem = header_for(6, 4, true);
        // What `InterpreterProfile::Amiga` publishes: `DEF_BACK 12` (dark grey)
        // and `DEF_FORE 9` (white), read out of the release floppies' own Amiga
        // interpreters (SQ-0822).
        write_default_colours(&mut mem, 12, 9);
        assert_eq!(
            amiga_screen_pair(&mem),
            Some((ZColour::Standard(9), ZColour::Standard(12))),
            "(foreground, background), straight off $2D/$2C",
        );
        // Every machine that is not an Amiga has no such thing — each window
        // carries its own pair and the host theme owns everything else.
        let mut ibm = header_for(6, 6, true);
        write_default_colours(&mut ibm, 12, 9);
        assert_eq!(amiga_screen_pair(&ibm), None, "interpreter 6 publishes no screen pair");
        // …and neither does a colourless interpreter, Amiga or not.
        let mut off = header_for(6, 4, false);
        write_default_colours(&mut off, 12, 9);
        assert_eq!(amiga_screen_pair(&off), None, "colours withdrawn: nothing to paint with");
    }

    /// A v6 screen with something already drawn on it: window 1 holds a grid cell
    /// and a painted run in an explicit pair, window 2 holds a run drawn over
    /// whatever was underneath it (a `-1`/inherited background), and window 3 is a
    /// window the game has never coloured.
    fn screen_with_text_on_it() -> ScreenState {
        let mut s = ScreenState { v6: Some(V6Windows::default()), ..Default::default() };
        let v6 = s.v6.as_mut().unwrap();
        let w1 = &mut v6.windows[1];
        w1.fg = ZColour::Standard(9);
        w1.bg = ZColour::Standard(2);
        w1.grid.resize(1, 1);
        w1.grid.cells[0] = Cell {
            ch: 'A',
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Standard(2),
        };
        w1.texts.push(V6Text {
            y: 1,
            x: 1,
            text: "banner".into(),
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Standard(2),
        });
        w1.streamed.push(V6Text {
            y: 9,
            x: 1,
            text: "prose".into(),
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Standard(2),
        });
        w1.retired.push(V6Text {
            y: 17,
            x: 1,
            text: "frozen".into(),
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Standard(2),
        });
        v6.windows[2].texts.push(V6Text {
            y: 1,
            x: 1,
            text: "over the art".into(),
            style: 0,
            fg: ZColour::Standard(9),
            bg: ZColour::Default,
        });
        s
    }

    /// The heart of §8.3: "If either is changed, then the interpreter must change
    /// the colour of ALL TEXT ON THE SCREEN to match."
    ///
    /// So this prints first and changes the colour second, and asserts the text
    /// that was already there moved. A test that only checked newly printed text
    /// would pass without the rule implemented at all.
    ///
    /// FALSIFY by deleting the `w.texts`/`streamed`/`retired`/`grid` loops from
    /// `repaint_amiga_pens`: every "already on the screen" assertion below fails
    /// with the reported symptom — glyphs still white (standard 9) after the game
    /// asked for black (standard 2).
    #[test]
    fn changing_either_colour_repaints_the_text_already_on_the_screen() {
        let mut s = screen_with_text_on_it();
        // …and the change is made from window 0, which owns none of that text.
        s.set_amiga_colour_pair(
            0,
            Some(ZColour::Standard(2)),
            Some(ZColour::Standard(10)),
            false,
            false,
        );

        let v6 = s.v6.as_ref().unwrap();
        let w1 = &v6.windows[1];
        assert_eq!(w1.grid.cells[0].fg, ZColour::Standard(2), "the grid cell's ink follows the pen");
        assert_eq!(w1.grid.cells[0].bg, ZColour::Standard(10), "…and its page");
        for (what, run) in
            [("texts", &w1.texts[0]), ("streamed", &w1.streamed[0]), ("retired", &w1.retired[0])]
        {
            assert_eq!(run.fg, ZColour::Standard(2), "{what}: already-painted ink follows the pen");
            assert_eq!(run.bg, ZColour::Standard(10), "{what}: and its page");
        }
        // "The same pair of colours for all windows" — including windows that did
        // not ask and windows that hold nothing.
        for (i, w) in v6.windows.iter().enumerate() {
            assert_eq!(w.fg, ZColour::Standard(2), "window {i} shares the foreground");
        }
        // A run drawn over what was underneath it painted no background at all, so
        // a pen carrying a new background has nothing there to change. Its INK
        // still follows — that is the reported symptom, prose left white while the
        // game had asked for black.
        let over_art = &v6.windows[2].texts[0];
        assert_eq!(over_art.fg, ZColour::Standard(2), "ink follows even over artwork");
        assert_eq!(over_art.bg, ZColour::Default, "…but a transparent background stays transparent");
        assert_eq!(v6.windows[3].bg, ZColour::Default, "an uncoloured window gains no opaque page");
    }

    /// The `0` sentinel means "leave this channel alone" (§8.3), and a channel
    /// left alone is a pen that has not moved — so it repaints nothing.
    #[test]
    fn a_channel_the_game_left_alone_moves_no_pen() {
        let mut s = screen_with_text_on_it();
        s.set_amiga_colour_pair(0, Some(ZColour::Standard(2)), None, false, false);
        let w1 = &s.v6.as_ref().unwrap().windows[1];
        assert_eq!(w1.texts[0].fg, ZColour::Standard(2), "the channel that moved repaints");
        assert_eq!(w1.texts[0].bg, ZColour::Standard(2), "the one that did not is untouched");
    }

    /// Colour **-1** is "the colour of the pixel under the cursor" (§8.3.1): it
    /// names no colour, so there is nothing to load into a pen. Infocom's own
    /// Amiga interpreter carves it out of the window-0 gate for exactly that reason
    /// (`amiga/yzip3.c`), and Zork Zero depends on it — it prints its banner
    /// labels over the ribbon artwork under `COLOR 2 -1`.
    ///
    /// So the request reaches the window that made it, and stops there: a window
    /// other than 0 moves no pen, whatever channels it names.
    #[test]
    fn the_pixel_under_the_cursor_is_a_paint_request_not_a_pen() {
        let mut s = screen_with_text_on_it();
        // Window 1 asks for black on whatever is beneath it.
        s.set_amiga_colour_pair(
            1,
            Some(ZColour::Standard(2)),
            Some(ZColour::Default),
            false,
            true,
        );
        let v6 = s.v6.as_ref().unwrap();
        assert_eq!(v6.windows[1].bg, ZColour::Default, "the asking window draws over the art");
        assert_eq!(v6.windows[1].fg, ZColour::Standard(2), "…in the ink it asked for");
        // No pen moved, so nothing already on the screen changed — not its page,
        // and not its ink either.
        assert_eq!(
            v6.windows[1].texts[0].bg,
            ZColour::Standard(2),
            "a -1 request repaints no existing background",
        );
        assert_eq!(
            v6.windows[1].texts[0].fg,
            ZColour::Standard(9),
            "…and a window that is not window 0 moves no ink pen either",
        );
        assert_eq!(v6.windows[0].fg, ZColour::Default, "no other window hears about it at all");
    }

    /// Infocom's window-0 gate: "We allow text colors to be changed only in window
    /// 0, and ignore requests in other windows (except for the special case of
    /// bg = -1)" (`amiga/yzip3.c`). A plain request from any other window is
    /// dropped whole — it does not move the pens AND it does not reach the window
    /// that made it.
    ///
    /// This is what makes `Journey - The Quest Begins.adf` (release 30, serial
    /// 890322) play on the Amiga's own light-grey default rather than the black
    /// page its single `set_colour(9, 2)` — issued on window 3 — asks for.
    ///
    /// FALSIFY by deleting the `win != 0` early return: window 3 takes the pair and
    /// every other window follows it.
    #[test]
    fn a_request_from_a_window_other_than_0_is_dropped_whole() {
        let mut s = screen_with_text_on_it();
        s.set_amiga_colour_pair(
            3,
            Some(ZColour::Standard(9)),
            Some(ZColour::Standard(2)),
            false,
            false,
        );
        let v6 = s.v6.as_ref().unwrap();
        assert_eq!(v6.windows[3].fg, ZColour::Default, "the asking window is not coloured");
        assert_eq!(v6.windows[3].bg, ZColour::Default, "…on either channel");
        assert_eq!(v6.windows[0].fg, ZColour::Default, "no pen moved");
        assert_eq!(
            v6.windows[1].texts[0].fg,
            ZColour::Standard(9),
            "nothing already on the screen was repainted",
        );
        assert_eq!(s.current_fg, ZColour::Default, "and the prose stream's pair is untouched");
    }

    /// Nothing here may touch a machine that is not an Amiga: the entry point is
    /// only ever reached through [`amiga_global_colour_pair`], and a v6 screen
    /// with no window table is left entirely alone.
    #[test]
    fn a_screen_with_no_v6_window_table_is_untouched() {
        let mut s = ScreenState::default();
        s.set_amiga_colour_pair(0, Some(ZColour::Standard(2)), Some(ZColour::Standard(10)), false, false);
        assert_eq!(s.current_fg, ZColour::Default);
        assert_eq!(s.current_bg, ZColour::Default);
    }
}
