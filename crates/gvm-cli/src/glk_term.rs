// Terminal Glk backend for gvm-cli (phase 3a-1: output).
//
// Reuses the zvm-cli screen-model approach: the status TextGrid window is pinned
// at the top of the terminal via an ANSI scroll-region + cursor addressing, and
// the main TextBuffer window scrolls in the region below. Glk styles map to SGR.
// When stdout is not a TTY the backend degrades to plain text streaming (the
// pinned grid is suppressed) so piped output stays clean.

use std::io::{self, IsTerminal, Write};

use gvm::glk::{GlkBackend, GlkStyle, Rect, WinType};

// ── word-wrap helper ──────────────────────────────────────────────────────────

/// Soft-wrap `text` at `cols` columns, using `current_col` as the starting
/// column position. Returns `(wrapped_text, new_col)`. Explicit `\n` in `text`
/// always resets the column to 0. When `cols` is 0 the text is returned
/// unchanged (no wrap — used when stdout is not a TTY, to keep piped output
/// byte-identical).
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

/// Wrap `s` in SGR for `style` when on a TTY and the style is non-plain.
fn style_wrap(s: &str, style: GlkStyle, tty: bool) -> String {
    let set = sgr_set(style);
    if tty && !set.is_empty() {
        format!("{set}{s}\x1b[0m")
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
fn render_grid(cells: &[Vec<(char, GlkStyle)>], width: u32, tty: bool) -> String {
    let mut out = String::new();
    out.push_str("\x1b7"); // DECSC save cursor
    for (r, row) in cells.iter().enumerate() {
        out.push_str(&format!("\x1b[{};1H\x1b[2K", r + 1)); // move to row, clear line
        let mut line = String::new();
        for c in 0..width as usize {
            let (ch, st) = row.get(c).copied().unwrap_or((' ', GlkStyle::Normal));
            line.push_str(&style_wrap(&ch.to_string(), st, tty));
        }
        out.push_str(line.trim_end());
    }
    out.push_str("\x1b8"); // DECRC restore cursor
    out
}

// ── Detect terminal size ──────────────────────────────────────────────────────

/// Detect the terminal size via crossterm. Falls back to 80×24 on error (e.g.
/// stdout is piped). Returns `(cols, rows)`.
fn detect_size() -> (u32, u32) {
    match crossterm::terminal::size() {
        Ok((cols, rows)) => (cols as u32, rows as u32),
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
    /// The tracked status grid window id (first TextGrid opened), if any.
    grid_win: Option<u32>,
    grid_rect: Rect,
    /// Grid cell buffer `[row][col] -> (char, style)`.
    grid_cells: Vec<Vec<(char, GlkStyle)>>,
    /// Whether the ANSI scroll region is currently in effect.
    region_set: bool,
    /// Whether the screen has been initialized (cleared) once.
    started: bool,
}

impl TerminalBackend {
    /// Build a backend writing to stdout, detecting TTY + terminal size.
    pub fn new() -> Self {
        let is_tty = io::stdout().is_terminal();
        let (cols, rows) = detect_size();
        TerminalBackend {
            out: Box::new(io::stdout()),
            is_tty,
            cols,
            rows,
            current_col: 0,
            grid_win: None,
            grid_rect: Rect::default(),
            grid_cells: Vec::new(),
            region_set: false,
            started: false,
        }
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
            grid_win: None,
            grid_rect: Rect::default(),
            grid_cells: Vec::new(),
            region_set: false,
            started: false,
        }
    }

    /// Grow the grid cell buffer to at least `height × width`.
    fn ensure_grid(&mut self, height: u32, width: u32) {
        if (self.grid_cells.len() as u32) < height {
            self.grid_cells.resize(height as usize, Vec::new());
        }
        for row in &mut self.grid_cells {
            if (row.len() as u32) < width {
                row.resize(width as usize, (' ', GlkStyle::Normal));
            }
        }
    }

    /// Flush buffered output to the display **without** tearing down the scroll
    /// region (used before reading input, so the prompt is visible mid-run).
    pub fn flush_out(&mut self) {
        let _ = self.out.flush();
    }

    /// Redraw the pinned grid (TTY only).
    fn redraw_grid(&mut self) {
        if !self.is_tty {
            return;
        }
        let s = render_grid(&self.grid_cells, self.grid_rect.width, true);
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

    fn put_text(&mut self, _win: u32, style: GlkStyle, s: &str) {
        // Soft-wrap text at the terminal column width when on a TTY.  Grid
        // windows use `grid_put` (never `put_text`), so wrapping here is safe
        // for all TextBuffer output.  When piped (is_tty = false) wrap cols is
        // 0 which leaves the text unchanged, preserving byte-identical piped
        // output.
        let wrap_cols = if self.is_tty { self.cols } else { 0 };
        let (wrapped, new_col) = soft_wrap(s, wrap_cols, self.current_col);
        self.current_col = new_col;
        let text = style_wrap(&wrapped, style, self.is_tty);
        let _ = self.out.write_all(text.as_bytes());
    }

    fn grid_put(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, s: &str) {
        if self.grid_win.is_none() {
            self.grid_win = Some(win);
        }
        self.ensure_grid(y + 1, x + s.chars().count() as u32);
        for (i, ch) in s.chars().enumerate() {
            let row = y as usize;
            let col = x as usize + i;
            if row < self.grid_cells.len() && col < self.grid_cells[row].len() {
                self.grid_cells[row][col] = (ch, style);
            }
        }
        self.redraw_grid();
    }

    fn grid_clear(&mut self, _win: u32) {
        for row in &mut self.grid_cells {
            for cell in row.iter_mut() {
                *cell = (' ', GlkStyle::Normal);
            }
        }
        self.redraw_grid();
    }

    fn flush(&mut self) {
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

    #[test]
    fn update_size_returns_changed() {
        let (mut b, _) = backend(false);
        assert!(b.update_size(100, 30), "size changed from 80x24");
        assert!(!b.update_size(100, 30), "size unchanged");
    }

    // ── SGR and streaming tests (from original) ────────────────────────────────

    #[test]
    fn sgr_maps_styles() {
        assert_eq!(sgr_set(GlkStyle::Normal), "");
        assert_eq!(sgr_set(GlkStyle::Header), "\x1b[1m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, false), "hi");
        assert_eq!(style_wrap("hi", GlkStyle::Normal, true), "hi");
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
        assert_eq!(out_string(&buf), "\x1b[3mahem\x1b[0m");
    }

    #[test]
    fn tty_wraps_long_buffer_line() {
        // Backend with 20-col width. Long text should wrap.
        let buf = Rc::new(RefCell::new(Vec::new()));
        let mut b = TerminalBackend::with_writer(Box::new(SharedWriter(buf.clone())), true, 20, 24);
        b.put_text(1, GlkStyle::Normal, "hello world foo bar baz");
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
}
