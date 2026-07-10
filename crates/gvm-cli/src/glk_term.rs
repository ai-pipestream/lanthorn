// Terminal Glk backend for gvm-cli (phase 3a-1: output).
//
// Reuses the zvm-cli screen-model approach: the status TextGrid window is pinned
// at the top of the terminal via an ANSI scroll-region + cursor addressing, and
// the main TextBuffer window scrolls in the region below. Glk styles map to SGR.
// When stdout is not a TTY the backend degrades to plain text streaming (the
// pinned grid is suppressed) so piped output stays clean.

use std::io::{self, IsTerminal, Write};

use gvm::glk::{GlkBackend, GlkStyle, Rect, StyleColour, WinType};

// ── word-wrap helper ──────────────────────────────────────────────────────────

/// Soft-wrap `text` at `cols` columns, using `current_col` as the starting
/// column position. Returns `(wrapped_text, new_col)`. Explicit `\n` in `text`
/// always resets the column to 0. When `cols` is 0 the text is returned
/// unchanged (no wrap — used when stdout is not a TTY, to keep piped output
/// byte-identical).
///
/// This function is retained for its own unit tests; production wrapping is
/// now handled by the stateful pending-word buffer in `put_text`.
#[cfg(test)]
pub fn soft_wrap(text: &str, cols: u32, current_col: u32) -> (String, u32) {
    if cols == 0 {
        // Compute new_col consistently even without wrapping.
        let mut col = current_col;
        for ch in text.chars() {
            if ch == '\n' {
                col = 0;
            } else {
                col = col.saturating_add(1);
            }
        }
        return (text.to_string(), col);
    }
    let mut out = String::with_capacity(text.len());
    let mut col = current_col;
    for word in text.split_inclusive(&[' ', '\n'][..]) {
        let is_nl = word.ends_with('\n');
        let clean = if is_nl { &word[..word.len() - 1] } else { word };
        let wlen = clean.chars().count() as u32;
        if col > 0 && col.saturating_add(wlen) > cols {
            out.push('\n');
            let trimmed = clean.trim_start();
            out.push_str(trimmed);
            col = trimmed.chars().count() as u32;
        } else {
            out.push_str(clean);
            col = col.saturating_add(wlen);
        }
        if is_nl {
            out.push('\n');
            col = 0;
        }
    }
    (out, col)
}

// ── SGR helpers ───────────────────────────────────────────────────────────────

/// SGR set-codes for a Glk style class (no trailing reset).
fn sgr_set(style: GlkStyle) -> &'static str {
    match style {
        GlkStyle::Emphasized => "\x1b[3m",   // italic
        GlkStyle::Header => "\x1b[1m",        // bold
        GlkStyle::Subheader => "\x1b[1m",     // bold
        GlkStyle::Alert => "\x1b[1m\x1b[7m",  // bold + reverse
        GlkStyle::Input => "\x1b[1m",         // bold
        _ => "",                              // Normal/Preformatted/Note/… plain
    }
}

/// Split a 24-bit `0xRRGGBB` colour into `(r, g, b)`.
fn rgb24(v: u32) -> (u8, u8, u8) {
    (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
}

/// Opening SGR for a style + resolved colour. Style attributes always apply;
/// the game's fg/bg/reverse colour is added only when `honor` is true (the
/// `--no-game-colours` gate), emitted as 24-bit truecolor so no fidelity is
/// lost. Returns `""` when nothing needs setting.
fn sgr_open(style: GlkStyle, colour: StyleColour, honor: bool) -> String {
    let mut s = String::from(sgr_set(style));
    if honor {
        if colour.reverse {
            s.push_str("\x1b[7m");
        }
        if let Some(fg) = colour.fg {
            let (r, g, b) = rgb24(fg);
            s.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
        }
        if let Some(bg) = colour.bg {
            let (r, g, b) = rgb24(bg);
            s.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
        }
    }
    s
}

/// Wrap `s` in SGR for `style` + `colour` when on a TTY and something is set.
fn style_wrap(s: &str, style: GlkStyle, colour: StyleColour, honor: bool, tty: bool) -> String {
    let open = sgr_open(style, colour, honor);
    if tty && !open.is_empty() {
        format!("{open}{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Set the scroll region to rows `[top, bottom]` and park the cursor at its
/// bottom-left (so subsequent buffer output scrolls inside it).
fn enter_region(top: u32, bottom: u32) -> String {
    format!("\x1b[{top};{bottom}r\x1b[{bottom};1H")
}

/// Reset the scroll region to the whole screen.
fn leave_region() -> String {
    "\x1b[r".to_string()
}

/// Render the pinned grid rows as ANSI: save cursor, redraw each grid row at its
/// absolute position (cleared), then restore the cursor.
fn render_grid(cells: &[Vec<(char, GlkStyle, StyleColour)>], width: u32, honor: bool, tty: bool) -> String {
    let mut out = String::new();
    out.push_str("\x1b7"); // DECSC save cursor
    for (r, row) in cells.iter().enumerate() {
        out.push_str(&format!("\x1b[{};1H\x1b[2K", r + 1)); // move to row, clear line
        let mut line = String::new();
        for c in 0..width as usize {
            let (ch, st, col) = row.get(c).copied().unwrap_or((' ', GlkStyle::Normal, StyleColour::default()));
            line.push_str(&style_wrap(&ch.to_string(), st, col, honor, tty));
        }
        out.push_str(line.trim_end());
    }
    out.push_str("\x1b8"); // DECRC restore cursor
    out
}

// ── Detect terminal size ──────────────────────────────────────────────────────

/// Convert a raw terminal size to a usable `(cols, rows)`, falling back to
/// 80×24 when either dimension is zero. `crossterm::terminal::size()` can
/// return `Ok((0, 0))` on some PTY implementations (e.g. macOS `script`);
/// a zero `cols` would make `soft_wrap` treat the output as "no wrap", which
/// silently disables word-wrap even on a real TTY.
fn coerce_size(cols: u16, rows: u16) -> (u32, u32) {
    if cols > 0 && rows > 0 { (cols as u32, rows as u32) } else { (80, 24) }
}

/// Detect the terminal size via crossterm. Falls back to 80×24 on error or
/// when the reported size is zero. Returns `(cols, rows)`.
fn detect_size() -> (u32, u32) {
    match crossterm::terminal::size() {
        Ok((cols, rows)) => coerce_size(cols, rows),
        Err(_) => (80, 24),
    }
}

// ── TerminalBackend ───────────────────────────────────────────────────────────

/// A terminal display backend.
pub struct TerminalBackend {
    out: Box<dyn Write>,
    is_tty: bool,
    cols: u32,
    rows: u32,
    /// Current column position in the buffer window (for soft word-wrap).
    /// Reset to 0 after each explicit or soft newline, and on flush.
    current_col: u32,
    /// Pending word: chars accumulated since the last space or newline (TTY
    /// only). Flushed — with a leading newline if the word doesn't fit on the
    /// current line — when a space, newline, or explicit flush arrives.
    pending_word: String,
    /// Style of the chars in `pending_word`. On a mid-word style change the
    /// accumulated portion is flushed with the old style before switching.
    pending_word_style: GlkStyle,
    /// Resolved colour of the chars in `pending_word`; flushed like the style.
    pending_word_colour: StyleColour,
    /// Whether to render the game's stylehint colours (`--no-game-colours` off).
    honor: bool,
    /// The tracked status grid window id (first TextGrid opened), if any.
    grid_win: Option<u32>,
    grid_rect: Rect,
    /// Grid cell buffer `[row][col] -> (char, style, colour)`.
    grid_cells: Vec<Vec<(char, GlkStyle, StyleColour)>>,
    /// Whether the ANSI scroll region is currently in effect.
    region_set: bool,
    /// Whether the screen has been initialized (cleared) once.
    started: bool,
    /// Emit terminal-detection diagnostics to stderr (env `BABELMAP_DEBUG_TERM`).
    debug: bool,
}

impl TerminalBackend {
    /// Build a backend writing to stdout, detecting TTY + terminal size.
    pub fn new() -> Self {
        let is_tty = io::stdout().is_terminal();
        let (cols, rows) = detect_size();
        let debug = std::env::var_os("BABELMAP_DEBUG_TERM").is_some();
        if debug {
            eprintln!("[term] new: is_tty={is_tty} cols={cols} rows={rows}");
        }
        TerminalBackend {
            out: Box::new(io::stdout()),
            is_tty,
            cols,
            rows,
            current_col: 0,
            pending_word: String::new(),
            pending_word_style: GlkStyle::Normal,
            pending_word_colour: StyleColour::default(),
            honor: true,
            grid_win: None,
            grid_rect: Rect::default(),
            grid_cells: Vec::new(),
            region_set: false,
            started: false,
            debug,
        }
    }

    /// Enable or disable rendering of the game's stylehint colours. When off,
    /// only style attributes (bold/italic/reverse) are emitted — the terminal's
    /// own palette shows through, matching zvm-cli's `--no-game-colours`.
    pub fn set_honor_colours(&mut self, on: bool) {
        self.honor = on;
    }

    /// Build a backend over an explicit writer (for tests).
    #[cfg(test)]
    fn with_writer(out: Box<dyn Write>, is_tty: bool, cols: u32, rows: u32) -> Self {
        TerminalBackend {
            out,
            is_tty,
            cols,
            rows,
            current_col: 0,
            pending_word: String::new(),
            pending_word_style: GlkStyle::Normal,
            pending_word_colour: StyleColour::default(),
            honor: true,
            grid_win: None,
            grid_rect: Rect::default(),
            grid_cells: Vec::new(),
            region_set: false,
            started: false,
            debug: false,
        }
    }

    /// Grow the grid cell buffer to at least `height × width`.
    fn ensure_grid(&mut self, height: u32, width: u32) {
        if (self.grid_cells.len() as u32) < height {
            self.grid_cells.resize(height as usize, Vec::new());
        }
        for row in &mut self.grid_cells {
            if (row.len() as u32) < width {
                row.resize(width as usize, (' ', GlkStyle::Normal, StyleColour::default()));
            }
        }
    }

    /// Emit the pending word (if any) to the output, inserting a newline before
    /// it when the word would overflow the current line. Updates `current_col`.
    fn flush_pending_word(&mut self) {
        if self.pending_word.is_empty() {
            return;
        }
        let style = self.pending_word_style;
        let wlen = self.pending_word.chars().count() as u32;
        if self.current_col > 0 && self.current_col + wlen > self.cols {
            let _ = self.out.write_all(b"\n");
            self.current_col = 0;
        }
        let text = style_wrap(&self.pending_word, style, self.pending_word_colour, self.honor, self.is_tty);
        let _ = self.out.write_all(text.as_bytes());
        self.current_col += wlen;
        self.pending_word.clear();
    }

    /// Called when `pending_word` has grown to `>= cols` characters and can
    /// never fit on a single line. Hard-breaks the accumulated word at the
    /// column limit (character wrap) so accumulation cannot grow unboundedly.
    fn flush_overlong_pending(&mut self) {
        let style = self.pending_word_style;
        let word = std::mem::take(&mut self.pending_word);
        // Move to a new line first if the word won't fit from the current column.
        let wlen = word.chars().count() as u32;
        if self.current_col > 0 && self.current_col + wlen > self.cols {
            let _ = self.out.write_all(b"\n");
            self.current_col = 0;
        }
        // Emit chars one at a time, inserting '\n' each time we reach the limit.
        let mut result = String::with_capacity(word.len() + 4);
        for ch in word.chars() {
            if self.current_col >= self.cols {
                result.push('\n');
                self.current_col = 0;
            }
            result.push(ch);
            self.current_col += 1;
        }
        let text = style_wrap(&result, style, self.pending_word_colour, self.honor, self.is_tty);
        let _ = self.out.write_all(text.as_bytes());
    }

    /// Flush buffered output to the display **without** tearing down the scroll
    /// region (used before reading input, so the prompt is visible mid-run).
    pub fn flush_out(&mut self) {
        // Emit any word still sitting in the pending buffer before blocking on
        // input — without this, the last word before a prompt would be invisible.
        if !self.pending_word.is_empty() {
            self.flush_pending_word();
        }
        let _ = self.out.flush();
    }

    /// Redraw the pinned grid (TTY only).
    fn redraw_grid(&mut self) {
        if !self.is_tty {
            return;
        }
        let s = render_grid(&self.grid_cells, self.grid_rect.width, self.honor, true);
        let _ = self.out.write_all(s.as_bytes());
    }

    /// Update the terminal size. Returns `true` if the size actually changed.
    /// The caller should call [`gvm::exec::Machine::notify_resize`] afterward
    /// to push an `evtype_Arrange` event and re-layout windows.
    pub fn update_size(&mut self, cols: u32, rows: u32) -> bool {
        if self.cols == cols && self.rows == rows {
            return false;
        }
        self.cols = cols;
        self.rows = rows;
        true
    }
}

impl Default for TerminalBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl GlkBackend for TerminalBackend {
    fn screen_size(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    fn window_layout(&mut self, wins: &[(u32, WinType, Rect)]) {
        if self.debug {
            for (id, ty, r) in wins {
                let kind = if *ty == WinType::TextGrid { "grid" } else if *ty == WinType::TextBuffer { "buffer" } else { "other" };
                eprintln!("[term] layout: win={id} {kind} w={} h={} (left={} top={})", r.width, r.height, r.left, r.top);
            }
        }
        // Track the first TextGrid as the pinned status window.
        let grid = wins.iter().find(|(_, ty, _)| *ty == WinType::TextGrid);
        if let Some(&(id, _, rect)) = grid {
            self.grid_win = Some(id);
            self.grid_rect = rect;
            self.ensure_grid(rect.height, rect.width);
            if self.is_tty && rect.height > 0 {
                if !self.started {
                    let _ = self.out.write_all(b"\x1b[2J\x1b[H"); // clear screen, home
                    self.started = true;
                }
                let top = rect.height + 1;
                let _ = self.out.write_all(enter_region(top, self.rows).as_bytes());
                self.region_set = true;
            }
        }
    }

    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        self.put_text_attr(win, style, StyleColour::default(), 0, s);
    }

    fn put_text_attr(&mut self, _win: u32, style: GlkStyle, colour: StyleColour, _link: u32, s: &str) {
        // When piped (not a TTY), pass through byte-identical — no buffering, no
        // wrap. `cols == 0` is the non-TTY sentinel for the soft_wrap helper and
        // is logged below but never used in the new char-by-char path.
        let wrap_cols = if self.is_tty { self.cols } else { 0 };
        if self.debug {
            eprintln!(
                "[term] put_text: is_tty={} cols={} wrap_cols={} col={} len={}",
                self.is_tty, self.cols, wrap_cols, self.current_col, s.chars().count()
            );
        }
        if !self.is_tty {
            let _ = self.out.write_all(s.as_bytes());
            return;
        }

        // TTY: buffer chars into a pending word. Whole words are placed at a
        // time, so breaks always happen at space boundaries even when the game
        // sends one character per call (as Glulx games do via glk_put_char).
        for ch in s.chars() {
            match ch {
                '\n' => {
                    self.flush_pending_word();
                    let _ = self.out.write_all(b"\n");
                    self.current_col = 0;
                }
                ' ' => {
                    self.flush_pending_word();
                    // Drop a trailing space that would push past the right margin
                    // (same normalisation as the previous soft_wrap helper).
                    if self.current_col < self.cols {
                        let _ = self.out.write_all(b" ");
                        self.current_col += 1;
                    }
                }
                _ => {
                    // On a mid-word style or colour change (rare) flush the old
                    // portion first so each run gets its own SGR wrap.
                    if (self.pending_word_style != style || self.pending_word_colour != colour)
                        && !self.pending_word.is_empty()
                    {
                        self.flush_pending_word();
                    }
                    self.pending_word_style = style;
                    self.pending_word_colour = colour;
                    self.pending_word.push(ch);
                    // Overlong-word guard: if the accumulated word has reached the
                    // column width it can never fit on one line — hard-break it
                    // now so the buffer cannot grow without bound.
                    if self.pending_word.chars().count() as u32 >= self.cols {
                        self.flush_overlong_pending();
                    }
                }
            }
        }
    }

    fn grid_put(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, s: &str) {
        self.grid_put_attr(win, x, y, style, StyleColour::default(), 0, s);
    }

    fn grid_put_attr(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, colour: StyleColour, _link: u32, s: &str) {
        if self.debug {
            eprintln!("[term] grid_put: win={win} x={x} y={y} len={}", s.chars().count());
        }
        if self.grid_win.is_none() {
            self.grid_win = Some(win);
        }
        self.ensure_grid(y + 1, x + s.chars().count() as u32);
        for (i, ch) in s.chars().enumerate() {
            let row = y as usize;
            let col = x as usize + i;
            if row < self.grid_cells.len() && col < self.grid_cells[row].len() {
                self.grid_cells[row][col] = (ch, style, colour);
            }
        }
        self.redraw_grid();
    }

    fn grid_clear(&mut self, _win: u32) {
        for row in &mut self.grid_cells {
            for cell in row.iter_mut() {
                *cell = (' ', GlkStyle::Normal, StyleColour::default());
            }
        }
        self.redraw_grid();
    }

    fn flush(&mut self) {
        // Emit any word still pending in the char-stream buffer before tearing
        // down the scroll region, so text that precedes glk_select is visible.
        if !self.pending_word.is_empty() {
            self.flush_pending_word();
        }
        if self.region_set {
            let _ = self.out.write_all(leave_region().as_bytes());
            let _ = write!(self.out, "\x1b[{};1H", self.rows);
            self.region_set = false;
        }
        // Cursor is now at the start of a new line; reset column tracking.
        self.current_col = 0;
        let _ = self.out.flush();
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A `Write` that appends into a shared buffer.
    struct SharedWriter(Rc<RefCell<Vec<u8>>>);
    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn backend(tty: bool) -> (TerminalBackend, Rc<RefCell<Vec<u8>>>) {
        let buf = Rc::new(RefCell::new(Vec::new()));
        let b = TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), tty, 80, 24);
        (b, buf)
    }

    fn out_string(buf: &Rc<RefCell<Vec<u8>>>) -> String {
        String::from_utf8(buf.borrow().clone()).unwrap()
    }

    // ── soft_wrap unit tests ──────────────────────────────────────────────────

    #[test]
    fn soft_wrap_no_wrap_when_enough_space() {
        let (out, col) = soft_wrap("hello world", 80, 0);
        assert_eq!(out, "hello world");
        assert_eq!(col, 11);
    }

    #[test]
    fn soft_wrap_wraps_at_column_boundary() {
        // "hello" at col 78 → 78+5=83 > 80 → wrap before "hello"
        let (out, col) = soft_wrap("hello", 80, 78);
        assert_eq!(out, "\nhello");
        assert_eq!(col, 5);
    }

    #[test]
    fn soft_wrap_no_wrap_at_line_start() {
        // Even a very long word at col 0 is not wrapped (avoids infinite loop).
        let long = "a".repeat(120);
        let (out, col) = soft_wrap(&long, 80, 0);
        assert_eq!(out, long);
        assert_eq!(col, 120);
    }

    #[test]
    fn soft_wrap_honors_explicit_newlines() {
        let (out, col) = soft_wrap("foo\nbar", 80, 70);
        assert_eq!(out, "foo\nbar");
        assert_eq!(col, 3);
    }

    #[test]
    fn soft_wrap_trims_space_that_triggers_wrap() {
        // A space token at col 80 triggers a wrap; the space is trimmed.
        // " " at col 80: 80+1=81>80 → wrap, trim(" ")="" → col=0
        // "world" at col 0: 0+5=5≤80 → emit, col=5
        let (out, col) = soft_wrap(" world", 80, 80);
        assert_eq!(out, "\nworld");
        assert_eq!(col, 5);
    }

    #[test]
    fn soft_wrap_disabled_when_cols_zero() {
        let text = "a very long line that would normally wrap at 80";
        let (out, col) = soft_wrap(text, 0, 75);
        assert_eq!(out, text);
        // col tracks chars even without wrapping
        assert_eq!(col, 75 + text.chars().count() as u32);
    }

    // ── coerce_size regression (root-cause guard) ─────────────────────────────
    //
    // crossterm::terminal::size() returns Ok((0, 0)) on some PTY implementations
    // (confirmed: macOS `script` creates a PTY where size() → Ok((0, 0))). A zero
    // cols value reaches soft_wrap as the "no wrap" sentinel, silently disabling
    // word-wrap even when is_tty=true. coerce_size() must catch the (0,0) case.
    #[test]
    fn coerce_size_falls_back_on_zero_cols() {
        assert_eq!(coerce_size(0, 0), (80, 24), "zero cols/rows must fall back");
        assert_eq!(coerce_size(0, 24), (80, 24), "zero cols alone must fall back");
        assert_eq!(coerce_size(80, 0), (80, 24), "zero rows alone must fall back");
    }

    #[test]
    fn coerce_size_passes_through_valid_size() {
        assert_eq!(coerce_size(120, 40), (120, 40));
        assert_eq!(coerce_size(1, 1), (1, 1));
    }

    #[test]
    fn update_size_returns_changed() {
        let (mut b, _) = backend(false);
        assert!(b.update_size(100, 30), "size changed from 80x24");
        assert!(!b.update_size(100, 30), "size unchanged");
    }

    // ── SGR and streaming tests (from original) ────────────────────────────────

    #[test]
    fn sgr_maps_styles() {
        let none = StyleColour::default();
        assert_eq!(sgr_set(GlkStyle::Normal), "");
        assert_eq!(sgr_set(GlkStyle::Header), "\x1b[1m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, none, true, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, none, true, false), "hi");
        assert_eq!(style_wrap("hi", GlkStyle::Normal, none, true, true), "hi");
    }

    #[test]
    fn stylehint_colour_emits_truecolor_sgr() {
        let fg_bg = StyleColour { fg: Some(0x00FF_8040), bg: Some(0x0011_2233), reverse: false };
        // fg -> 38;2;r;g;b, bg -> 48;2;r;g;b, honoured, wrapped for a TTY.
        assert_eq!(
            style_wrap("x", GlkStyle::Normal, fg_bg, true, true),
            "\x1b[38;2;255;128;64m\x1b[48;2;17;34;51mx\x1b[0m"
        );
        // --no-game-colours (honor=false) drops the colour entirely.
        assert_eq!(style_wrap("x", GlkStyle::Normal, fg_bg, false, true), "x");
        // reverse hint emits SGR 7 ahead of any colour.
        let rev = StyleColour { fg: None, bg: None, reverse: true };
        assert_eq!(style_wrap("x", GlkStyle::Normal, rev, true, true), "\x1b[7mx\x1b[0m");
    }

    #[test]
    fn non_tty_streams_plain_buffer_text() {
        let (mut b, buf) = backend(false);
        b.put_text(1, GlkStyle::Normal, "Hello");
        b.put_text(1, GlkStyle::Header, " World"); // style dropped when piped
        b.flush();
        assert_eq!(out_string(&buf), "Hello World");
        assert!(!out_string(&buf).contains('\x1b'), "piped output carries no ANSI");
    }

    #[test]
    fn tty_buffer_text_word_wraps_at_cols() {
        // Regression (user report 2026-06-30): TextBuffer output must soft-wrap
        // at the terminal width on a TTY. All glk_put_char/string for a buffer
        // window funnel through put_text, so this covers both.
        let (mut b, buf) = backend(true); // tty, 80 cols
        let line = "word ".repeat(30); // 150 chars with spaces -> must wrap at 80
        b.put_text(1, GlkStyle::Normal, &line);
        let out = out_string(&buf);
        assert!(out.contains('\n'), "buffer text must wrap at 80 cols on a TTY: {out:?}");
        assert!(
            out.lines().all(|l| l.chars().count() <= 80),
            "no wrapped line exceeds the column width: {out:?}"
        );
    }

    #[test]
    fn tty_buffer_text_wraps_char_by_char() {
        // glk_put_char emits one char per put_text call; current_col must persist
        // across calls so a long char-by-char line still wraps.
        let (mut b, buf) = backend(true);
        for ch in "abcdefghij ".repeat(10).chars() {
            // 110 chars
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        let out = out_string(&buf);
        assert!(out.contains('\n'), "char-by-char buffer output must still wrap: {out:?}");
    }

    #[test]
    fn tty_pins_grid_and_sets_scroll_region() {
        let (mut b, buf) = backend(true);
        // Layout: a 1-row TextGrid (id 2) above an 80x23 TextBuffer (id 1).
        let grid = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 });
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 1, width: 80, height: 23 });
        b.window_layout(&[buffer, grid]);
        b.grid_put(2, 0, 0, GlkStyle::Normal, "Score: 10");
        b.put_text(1, GlkStyle::Normal, "You are in a room.");
        b.flush();
        let out = out_string(&buf);
        assert!(out.contains("\x1b[2;24r"), "scroll region below the 1-row grid: {out:?}");
        assert!(out.contains("Score: 10"), "grid text rendered: {out:?}");
        assert!(out.contains("You are in a room."), "buffer text rendered");
        assert!(out.contains("\x1b[r"), "region reset on flush");
    }

    #[test]
    fn tty_styles_buffer_output() {
        let (mut b, buf) = backend(true);
        b.put_text(1, GlkStyle::Emphasized, "ahem");
        // "ahem" is buffered until flush_out() since it has no trailing space.
        b.flush_out();
        assert_eq!(out_string(&buf), "\x1b[3mahem\x1b[0m");
    }

    #[test]
    fn tty_wraps_long_buffer_line() {
        // Backend with 20-col width. Long text should wrap.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b = TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 20, 24);
        b.put_text(1, GlkStyle::Normal, "hello world foo bar baz");
        // "baz" has no trailing space so it sits in the pending buffer until
        // flush_out() is called (mimicking what happens before glk_select).
        b.flush_out();
        let out = String::from_utf8(buf.borrow().clone()).unwrap();
        // Should contain at least one soft newline
        assert!(out.contains('\n'), "long line wrapped: {out:?}");
        // The text (without soft newlines) should be preserved
        let text: String = out.chars().filter(|&c| c != '\n').collect();
        assert_eq!(text, "hello world foo bar baz");
    }

    #[test]
    fn non_tty_does_not_wrap() {
        // Piped backend should not insert soft newlines.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b = TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), false, 20, 24);
        b.put_text(1, GlkStyle::Normal, "hello world foo bar baz");
        let out = String::from_utf8(buf.borrow().clone()).unwrap();
        assert!(!out.contains('\n'), "piped output must not be wrapped: {out:?}");
    }

    // ── char-stream word-wrap tests (TDD for the Glulx glk_put_char fix) ──────

    #[test]
    fn char_stream_wraps_at_word_boundary_not_mid_word() {
        // Glulx games emit one char per put_text call. With cols=10,
        // character-based wrapping would split "world" (chars 7-11 straddle col
        // 10). Word-based wrapping must keep the whole word intact on one line.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b =
            TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 10, 24);
        for ch in "hello world foo".chars() {
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        // "foo" has no trailing space — flush it explicitly, as happens before
        // an input prompt.
        b.flush_out();
        let out = String::from_utf8(buf.borrow().clone()).unwrap();
        // Every output line must fit within cols.
        for line in out.lines() {
            assert!(
                line.chars().count() <= 10,
                "line exceeds cols=10: {line:?} (full output: {out:?})"
            );
        }
        // Every word must appear intact on some output line (no mid-word split).
        for word in &["hello", "world", "foo"] {
            assert!(
                out.lines().any(|l| l.contains(word)),
                "word {word:?} must appear intact on one line: {out:?}"
            );
        }
    }

    #[test]
    fn overlong_word_hard_breaks() {
        // A single word longer than cols must hard-break rather than hang or
        // produce lines that exceed the column width.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b =
            TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 10, 24);
        // "superlongword" is 13 chars which exceeds cols=10.
        for ch in "superlongword".chars() {
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        b.flush_out();
        let out = String::from_utf8(buf.borrow().clone()).unwrap();
        for line in out.lines() {
            assert!(
                line.chars().count() <= 10,
                "hard-break: line exceeds cols=10: {line:?} (full: {out:?})"
            );
        }
        // All characters must appear in the output — none dropped.
        let text: String = out.chars().filter(|&c| c != '\n').collect();
        assert_eq!(text, "superlongword", "all chars preserved after hard-break: {out:?}");
    }

    #[test]
    fn explicit_newlines_honored_in_char_stream() {
        // Explicit '\\n' in the char stream must be honoured and reset the
        // column counter, regardless of column position.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b =
            TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 20, 24);
        for ch in "foo\nbar".chars() {
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        b.flush_out(); // flush "bar" (no trailing space)
        let out = String::from_utf8(buf.borrow().clone()).unwrap();
        assert!(out.contains("foo\nbar"), "explicit newline preserved: {out:?}");
    }

    #[test]
    fn flush_out_emits_pending_word() {
        // A word buffered with no trailing space must be invisible until
        // flush_out() is called (matching what happens before glk_select).
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b =
            TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 20, 24);
        for ch in "foo".chars() {
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        // "foo" has no trailing space — must still be buffered.
        assert!(
            !out_string(&buf).contains("foo"),
            "pending word must not be emitted before flush_out: {:?}",
            out_string(&buf)
        );
        b.flush_out();
        assert!(
            out_string(&buf).contains("foo"),
            "pending word must appear after flush_out: {:?}",
            out_string(&buf)
        );
    }

    #[test]
    fn piped_char_by_char_unchanged() {
        // Non-TTY output must be byte-identical even when chars arrive one at a
        // time — no buffering, no newlines inserted.
        let (mut b, buf) = backend(false);
        for ch in "hello world foo bar baz".chars() {
            b.put_text(1, GlkStyle::Normal, &ch.to_string());
        }
        assert_eq!(out_string(&buf), "hello world foo bar baz");
        assert!(!out_string(&buf).contains('\n'), "piped char-by-char: no newlines inserted");
    }
}
