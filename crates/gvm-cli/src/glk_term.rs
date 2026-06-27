// Terminal Glk backend for gvm-cli (phase 3a-1: output).
//
// Reuses the zvm-cli screen-model approach: the status TextGrid window is pinned
// at the top of the terminal via an ANSI scroll-region + cursor addressing, and
// the main TextBuffer window scrolls in the region below. Glk styles map to SGR.
// When stdout is not a TTY the backend degrades to plain text streaming (the
// pinned grid is suppressed) so piped output stays clean.

use std::env;
use std::io::{self, IsTerminal, Write};

use gvm::glk::{GlkBackend, GlkStyle, Rect, WinType};

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

/// A terminal display backend.
pub struct TerminalBackend {
    out: Box<dyn Write>,
    is_tty: bool,
    cols: u32,
    rows: u32,
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
        let cols = env::var("COLUMNS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
        let rows = env::var("LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
        TerminalBackend {
            out: Box::new(io::stdout()),
            is_tty,
            cols,
            rows,
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

    /// Redraw the pinned grid (TTY only).
    fn redraw_grid(&mut self) {
        if !self.is_tty {
            return;
        }
        let s = render_grid(&self.grid_cells, self.grid_rect.width, true);
        let _ = self.out.write_all(s.as_bytes());
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
        let text = style_wrap(s, style, self.is_tty);
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
}
