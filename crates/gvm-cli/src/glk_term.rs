// Terminal Glk backend for gvm-cli (phase 3a-1: output).
//
// Reuses the zvm-cli screen-model approach: static TextGrid windows are drawn at
// their resolved rects via absolute cursor addressing, the main TextBuffer window
// scrolls in an ANSI scroll-region confined to its row band, and inter-window
// separators (the `winmethod_Border` hint) are drawn as box-drawing rules in the
// gutter cells gvm reserves for them. Glk styles map to SGR. When stdout is not a
// TTY the backend degrades to plain text streaming (all geometry/border chrome is
// suppressed) so piped output stays byte-identical.
//
// KNOWN LIMITATION (SQ-0327): an ANSI scroll region (DECSTBM) is always
// FULL-WIDTH, so a line-oriented terminal cannot scroll two side-by-side buffer
// columns independently. ABOVE/BELOW (stacked) splits are honoured exactly — the
// scrolling buffer takes the rows above/below its static neighbours. But for a
// LEFT/RIGHT split the buffer can only be confined to its ROW band at full width:
// it spans the whole terminal width and the side grid/graphics window (redrawn on
// top) is overwritten as the buffer scrolls. This is documented rather than hacked
// around with a broken sub-column scroll region.

use std::io::{self, IsTerminal, Write};

use gvm::glk::{GlkBackend, GlkStyle, Rect, StyleAttrs, StyleColour, WinTree, WinType};

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

/// SGR set-codes for a Glk style class with its rendered Weight/Oblique
/// stylehints layered on top (no trailing reset). A set hint overrides the
/// class default — Weight 1 → bold, any other value → not bold ("lighter" has
/// no terminal rendering, so it maps to plain); Oblique 1 → italic, other →
/// upright. An unset hint keeps the class's intrinsic look (SQ-0317).
fn sgr_set(style: GlkStyle, attrs: StyleAttrs) -> String {
    // Class-intrinsic (bold, italic, reverse) look.
    let (mut bold, mut italic, reverse) = match style {
        GlkStyle::Emphasized => (false, true, false),
        GlkStyle::Header | GlkStyle::Subheader | GlkStyle::Input => (true, false, false),
        GlkStyle::Alert => (true, false, true),
        _ => (false, false, false), // Normal/Preformatted/Note/… plain
    };
    match attrs.weight {
        Some(1) => bold = true,
        Some(_) => bold = false,
        None => {}
    }
    match attrs.oblique {
        Some(1) => italic = true,
        Some(_) => italic = false,
        None => {}
    }
    let mut s = String::new();
    if bold {
        s.push_str("\x1b[1m");
    }
    if italic {
        s.push_str("\x1b[3m");
    }
    if reverse {
        s.push_str("\x1b[7m");
    }
    s
}

/// Split a 24-bit `0xRRGGBB` colour into `(r, g, b)`.
pub(crate) fn rgb24(v: u32) -> (u8, u8, u8) {
    (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
}

// ── page-background OSC helpers ─────────────────────────────────────────────

/// OSC 11: set the terminal's default background to `#rrggbb`.
pub fn osc_set_bg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x07")
}

/// OSC 111: reset the terminal's default background to the user's default.
pub fn osc_reset_bg() -> &'static str {
    "\x1b]111\x07"
}

/// DECSCUSR: force a steady block cursor (SQ-0281 — a box rather than the
/// terminal's default underline).
pub fn cursor_steady_block() -> &'static str {
    "\x1b[2 q"
}

/// DECSCUSR: restore the terminal's default cursor shape.
pub fn cursor_reset() -> &'static str {
    "\x1b[0 q"
}

/// The escape to emit for a page-bg transition from `prev` to `cur` (both are
/// already honor-resolved: `None` = no game bg / default). `None` return = no
/// change.
pub fn page_bg_escape(cur: Option<(u8, u8, u8)>, prev: Option<(u8, u8, u8)>) -> Option<String> {
    if cur == prev {
        return None;
    }
    Some(match cur {
        Some(rgb) => osc_set_bg(rgb),
        None => osc_reset_bg().to_string(),
    })
}

/// Opening SGR for a style + resolved colour and attribute hints. Style
/// attributes always apply; the game's fg/bg/reverse colour is added only when
/// `honor` is true (the `--no-game-colours` gate), emitted as 24-bit truecolor
/// so no fidelity is lost. Returns `""` when nothing needs setting.
fn sgr_open(style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, honor: bool) -> String {
    let mut s = sgr_set(style, attrs);
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

/// Wrap `s` in SGR for `style` + `colour` + `attrs` when on a TTY and something is set.
fn style_wrap(s: &str, style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, honor: bool, tty: bool) -> String {
    let open = sgr_open(style, colour, attrs, honor);
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

/// One status-grid cell: character + Glk style + resolved colour + rendered
/// Weight/Oblique attribute hints.
type GridCell = (char, GlkStyle, StyleColour, StyleAttrs);

/// An empty (space, unstyled) grid cell.
const BLANK_CELL: GridCell =
    (' ', GlkStyle::Normal, StyleColour { fg: None, bg: None, reverse: false }, StyleAttrs { weight: None, oblique: None, indent: None, para_indent: None, justify: None });

/// Box-drawing characters for the inter-window separator rules and graphics
/// placeholder outlines.
const H_RULE: char = '─';
const V_RULE: char = '│';

/// A tracked text-grid window: its resolved rect and cell buffer. Every grid
/// leaf is drawn at its own rect (not collapsed to a single pinned status line).
struct GridWin {
    id: u32,
    rect: Rect,
    cells: Vec<Vec<GridCell>>,
}

impl GridWin {
    /// Grow the cell buffer to at least `height × width`.
    fn ensure(&mut self, height: u32, width: u32) {
        if (self.cells.len() as u32) < height {
            self.cells.resize(height as usize, Vec::new());
        }
        for row in &mut self.cells {
            if (row.len() as u32) < width {
                row.resize(width as usize, BLANK_CELL);
            }
        }
    }
}

/// Append one grid's cells to `out`, addressed absolutely at its rect. A grid
/// that spans the full terminal width keeps the historical clear-line rendering
/// (`\x1b[2K` then the trimmed row), so the common status-line-on-top case is
/// byte-identical; a narrower grid (a Left/Right column) writes its cells at its
/// own width only, so it never clears into a neighbouring window.
fn append_grid(out: &mut String, g: &GridWin, term_cols: u32, honor: bool, tty: bool) {
    let full_width = g.rect.left == 0 && g.rect.width == term_cols;
    for (r, row) in g.cells.iter().enumerate() {
        let screen_row = g.rect.top + r as u32 + 1;
        let screen_col = g.rect.left + 1;
        out.push_str(&format!("\x1b[{screen_row};{screen_col}H"));
        let mut line = String::new();
        for c in 0..g.rect.width as usize {
            let (ch, st, col, at) = row.get(c).copied().unwrap_or(BLANK_CELL);
            line.push_str(&style_wrap(&ch.to_string(), st, col, at, honor, tty));
        }
        if full_width {
            out.push_str("\x1b[2K"); // clear the whole line, then the trimmed row
            out.push_str(line.trim_end());
        } else {
            out.push_str(&line); // fixed-width column: no line clear
        }
    }
}

/// Append the inter-window separator rules for a window tree to `out`. Each
/// bordered pair draws a rule in the gutter cell gvm reserves for it: a
/// horizontal rule at row `top + split` for a stacked pair, a vertical rule at
/// column `left + split` for a side-by-side pair. `NoBorder` pairs draw nothing.
fn append_borders(out: &mut String, tree: &WinTree) {
    if let WinTree::Pair { vertical, border, split, rect, first, second, .. } = tree {
        if *border {
            if *vertical {
                // Horizontal rule across the reserved gutter row.
                let row = rect.top + split + 1;
                out.push_str(&format!("\x1b[{};{}H", row, rect.left + 1));
                out.extend(std::iter::repeat(H_RULE).take(rect.width as usize));
            } else {
                // Vertical rule down the reserved gutter column.
                let col = rect.left + split + 1;
                for r in 0..rect.height {
                    out.push_str(&format!("\x1b[{};{}H", rect.top + r + 1, col));
                    out.push(V_RULE);
                }
            }
        }
        append_borders(out, first);
        append_borders(out, second);
    }
}

/// Append a simple bordered placeholder box for a graphics window at its rect
/// (gvm-cli has no image protocol). The interior is cleared to spaces so stale
/// content within the rect doesn't show through; nothing outside the rect is
/// touched.
fn append_graphics(out: &mut String, rect: Rect) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    for r in 0..rect.height {
        out.push_str(&format!("\x1b[{};{}H", rect.top + r + 1, rect.left + 1));
        let mut line = String::new();
        for c in 0..rect.width {
            let edge_row = r == 0 || r == rect.height - 1;
            let edge_col = c == 0 || c == rect.width - 1;
            line.push(if edge_row { H_RULE } else if edge_col { V_RULE } else { ' ' });
        }
        out.push_str(&line);
    }
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
    /// Rendered Weight/Oblique hints of the chars in `pending_word`; flushed
    /// like the style.
    pending_word_attrs: StyleAttrs,
    /// Whether to render the game's stylehint colours (`--no-game-colours` off).
    honor: bool,
    /// Every tracked TextGrid window with its resolved rect + cell buffer. Each
    /// is drawn at its own rect (grids no longer collapse to one status line).
    grids: Vec<GridWin>,
    /// Graphics windows as `(id, rect)`, drawn as bordered placeholder boxes.
    graphics: Vec<(u32, Rect)>,
    /// The live window tree (from `window_tree`), used to draw inter-window
    /// separators in the gutters gvm reserves. `None` until the first tree, or
    /// when no root window exists.
    tree: Option<WinTree>,
    /// Whether the ANSI scroll region is currently in effect.
    region_set: bool,
    /// The `(top, bottom)` bounds of the scroll region as last emitted. Used to
    /// avoid re-emitting `enter_region` (which parks the cursor at the bottom-left)
    /// on an unchanged re-layout, which would yank the cursor to column 0 mid-line.
    region_bounds: (u32, u32),
    /// Whether the screen has been initialized (cleared) once.
    started: bool,
    /// A command just read via line input, awaiting deferred echo resolution on
    /// the next buffer output (SQ-0282). `None` when no input echo is pending.
    pending_echo: Option<String>,
    /// Emit terminal-detection diagnostics to stderr (env `BABELMAP_DEBUG_TERM`).
    debug: bool,
    /// The story Blorb, when one was loaded, retained to serve `Data` resource
    /// streams (`glk_stream_open_resource`). `None` for a plain `.ulx` — the
    /// resource open then fails, per spec.
    data_blorb: Option<blorb::Blorb>,
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
            pending_word_attrs: StyleAttrs::default(),
            honor: true,
            grids: Vec::new(),
            graphics: Vec::new(),
            tree: None,
            region_set: false,
            region_bounds: (0, 0),
            started: false,
            pending_echo: None,
            debug,
            data_blorb: None,
        }
    }

    /// Enable or disable rendering of the game's stylehint colours. When off,
    /// only style attributes (bold/italic/reverse) are emitted — the terminal's
    /// own palette shows through, matching zvm-cli's `--no-game-colours`.
    pub fn set_honor_colours(&mut self, on: bool) {
        self.honor = on;
    }

    /// Retain the story Blorb so `glk_stream_open_resource` can read its `Data`
    /// chunks. `None` for a plain `.ulx` (resource opens then fail).
    pub fn set_data_blorb(&mut self, blorb: Option<blorb::Blorb>) {
        self.data_blorb = blorb;
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
            pending_word_attrs: StyleAttrs::default(),
            honor: true,
            grids: Vec::new(),
            graphics: Vec::new(),
            tree: None,
            region_set: false,
            region_bounds: (0, 0),
            started: false,
            pending_echo: None,
            debug: false,
            data_blorb: None,
        }
    }

    /// Index of the tracked grid for `id`, creating an empty entry if none.
    fn grid_index(&mut self, id: u32) -> usize {
        if let Some(i) = self.grids.iter().position(|g| g.id == id) {
            i
        } else {
            self.grids.push(GridWin { id, rect: Rect::default(), cells: Vec::new() });
            self.grids.len() - 1
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
        let text =
            style_wrap(&self.pending_word, style, self.pending_word_colour, self.pending_word_attrs, self.honor, self.is_tty);
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
        let text = style_wrap(&result, style, self.pending_word_colour, self.pending_word_attrs, self.honor, self.is_tty);
        let _ = self.out.write_all(text.as_bytes());
    }

    /// Flush buffered output to the display **without** tearing down the scroll
    /// region (used before reading input, so the prompt is visible mid-run).
    pub fn flush_out(&mut self) {
        // A line input that produced no reprinting buffer output before the next
        // prompt still needs its command echoed, or it would vanish (SQ-0282).
        if let Some(cmd) = self.pending_echo.take() {
            self.emit_library_echo(&cmd);
        }
        // Emit any word still sitting in the pending buffer before blocking on
        // input — without this, the last word before a prompt would be invisible.
        if !self.pending_word.is_empty() {
            self.flush_pending_word();
        }
        let _ = self.out.flush();
    }

    /// Arm a deferred echo of `cmd` (a just-read line-input command) to be resolved
    /// on the next buffer output (SQ-0282). No-op when stdout is not a TTY (piped
    /// output carries no echo, keeping it byte-identical).
    pub fn arm_input_echo(&mut self, cmd: String) {
        if self.is_tty {
            self.pending_echo = Some(cmd);
        }
    }

    /// Echo `cmd` in the Input style followed by a newline — the library echo a
    /// game expects when it does not reprint the command itself.
    fn emit_library_echo(&mut self, cmd: &str) {
        let text = style_wrap(cmd, GlkStyle::Input, StyleColour::default(), StyleAttrs::default(), self.honor, self.is_tty);
        let _ = self.out.write_all(text.as_bytes());
        let _ = self.out.write_all(b"\n");
        self.current_col = 0;
    }

    /// Redraw all static chrome (TTY only): every grid at its rect, the graphics
    /// placeholders, and the inter-window separator rules — wrapped in a single
    /// cursor save/restore so the scrolling buffer's cursor is never disturbed.
    fn redraw_chrome(&mut self) {
        if !self.is_tty {
            return;
        }
        let mut out = String::new();
        out.push_str("\x1b7"); // DECSC save cursor
        for g in &self.grids {
            append_grid(&mut out, g, self.cols, self.honor, true);
        }
        for &(_, rect) in &self.graphics {
            append_graphics(&mut out, rect);
        }
        if let Some(tree) = &self.tree {
            append_borders(&mut out, tree);
        }
        out.push_str("\x1b8"); // DECRC restore cursor
        let _ = self.out.write_all(out.as_bytes());
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

    fn data_resource(&mut self, num: u32) -> Option<(Vec<u8>, bool)> {
        let (ty, bytes) = self.data_blorb.as_ref()?.resource(b"Data", num)?;
        Some((bytes.to_vec(), ty == b"TEXT"))
    }

    // `default_style_colours` (SQ-0315) deliberately stays at the trait default
    // (None): the CLI renders on the terminal's own palette, which it cannot
    // read portably, and the only colours it ever paints itself (the OSC-11
    // page background) come from game-set stylehints — which glk_style_measure
    // already answers from the model, taking precedence over any backend
    // report. None is the honest "I don't know".

    fn window_layout(&mut self, wins: &[(u32, WinType, Rect, Option<bool>)]) {
        if self.debug {
            for (id, ty, r, _border) in wins {
                let kind = if *ty == WinType::TextGrid { "grid" } else if *ty == WinType::TextBuffer { "buffer" } else { "other" };
                eprintln!("[term] layout: win={id} {kind} w={} h={} (left={} top={})", r.width, r.height, r.left, r.top);
            }
        }
        // Register every TextGrid at its rect (growing its cell buffer), and drop
        // grids that have closed. Rendering happens in redraw_chrome / grid_put.
        let mut live: Vec<u32> = Vec::new();
        for &(id, ty, rect, _) in wins {
            if ty == WinType::TextGrid {
                let idx = self.grid_index(id);
                self.grids[idx].rect = rect;
                self.grids[idx].ensure(rect.height, rect.width);
                live.push(id);
            }
        }
        self.grids.retain(|g| live.contains(&g.id));
        // Register graphics windows for their placeholder boxes.
        self.graphics = wins
            .iter()
            .filter(|(_, ty, _, _)| *ty == WinType::Graphics)
            .map(|&(id, _, rect, _)| (id, rect))
            .collect();

        if !self.is_tty {
            return; // piped: no geometry chrome, output stays byte-identical
        }

        // Clear the screen once, on the first layout that establishes any chrome.
        let has_chrome = !self.grids.is_empty() || !self.graphics.is_empty();
        if !self.started && has_chrome {
            let _ = self.out.write_all(b"\x1b[2J\x1b[H"); // clear screen, home
            self.started = true;
        }

        // Confine the scrolling TextBuffer to its ROW band via an ANSI scroll
        // region. DECSTBM is FULL-WIDTH, so this honours ABOVE/BELOW (stacked)
        // geometry exactly, but a LEFT/RIGHT split falls back to the buffer's full
        // width (the documented limitation — a terminal cannot scroll a sub-column
        // independently). The largest buffer is treated as the primary one.
        let buffer = wins
            .iter()
            .filter(|(_, ty, _, _)| *ty == WinType::TextBuffer)
            .max_by_key(|(_, _, r, _)| r.width * r.height);
        if let Some(&(_, _, rect, _)) = buffer {
            let top = rect.top + 1;
            let bottom = rect.top + rect.height;
            // A full-screen buffer needs no region (natural whole-screen scroll),
            // which also keeps the buffer-only case byte-identical.
            let full_screen = top == 1 && bottom >= self.rows;
            if !full_screen && rect.height > 0 {
                let bounds = (top, bottom);
                // Only (re)enter the scroll region when its bounds actually change.
                // enter_region parks the cursor at the region's bottom-left, so
                // re-emitting it on an unchanged re-layout would yank the cursor to
                // column 0 mid-line — which is why the first prompt landed at the
                // start of the line (CM re-lays-out its windows right before input).
                if self.region_bounds != bounds {
                    let _ = self.out.write_all(enter_region(top, bottom).as_bytes());
                    self.region_bounds = bounds;
                    self.current_col = 0; // cursor now parked at the region's bottom-left
                }
                self.region_set = true;
            }
        }
    }

    fn window_tree(&mut self, tree: Option<WinTree>) {
        self.tree = tree;
        // The tree carries the borders + true rects; redraw the static chrome so
        // separators and any repositioned grids appear (TTY only).
        self.redraw_chrome();
    }

    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        self.put_text_attr(win, style, StyleColour::default(), StyleAttrs::default(), 0, s);
    }

    fn put_text_attr(&mut self, _win: u32, style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, _link: u32, s: &str) {
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

        // Deferred input echo (SQ-0282): the first buffer output after a line input
        // resolves it. If the game reprints the command itself in style_Input (e.g.
        // Counterfeit Monkey) let that stand; otherwise (e.g. sensory, which relies
        // on library echo gvm doesn't implement) echo the command here so it isn't
        // lost. Cleared either way so it fires only once per input.
        if let Some(cmd) = self.pending_echo.take() {
            if style != GlkStyle::Input {
                self.emit_library_echo(&cmd);
            }
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
                    if (self.pending_word_style != style
                        || self.pending_word_colour != colour
                        || self.pending_word_attrs != attrs)
                        && !self.pending_word.is_empty()
                    {
                        self.flush_pending_word();
                    }
                    self.pending_word_style = style;
                    self.pending_word_colour = colour;
                    self.pending_word_attrs = attrs;
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
        self.grid_put_attr(win, x, y, style, StyleColour::default(), StyleAttrs::default(), 0, s);
    }

    fn grid_put_attr(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, colour: StyleColour, attrs: StyleAttrs, _link: u32, s: &str) {
        if self.debug {
            eprintln!("[term] grid_put: win={win} x={x} y={y} len={}", s.chars().count());
        }
        let idx = self.grid_index(win);
        self.grids[idx].ensure(y + 1, x + s.chars().count() as u32);
        let cells = &mut self.grids[idx].cells;
        for (i, ch) in s.chars().enumerate() {
            let row = y as usize;
            let col = x as usize + i;
            if row < cells.len() && col < cells[row].len() {
                cells[row][col] = (ch, style, colour, attrs);
            }
        }
        self.redraw_chrome();
    }

    fn grid_clear(&mut self, win: u32) {
        if let Some(g) = self.grids.iter_mut().find(|g| g.id == win) {
            for row in &mut g.cells {
                for cell in row.iter_mut() {
                    *cell = BLANK_CELL;
                }
            }
        }
        self.redraw_chrome();
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
            self.region_bounds = (0, 0);
        }
        // Cursor is now at the start of a new line; reset column tracking.
        self.current_col = 0;
        let _ = self.out.flush();
    }

    /// The system timezone's UTC offset (seconds east of Greenwich) at the given
    /// instant, for the Glk `_local` date/time selectors — resolved per instant
    /// (DST-correct at any time), thread-safe, and cross-platform via `jiff`.
    /// Out-of-range instants → `None` → the selectors fall back to UTC.
    fn local_utc_offset_seconds(&self, epoch_seconds: i64) -> Option<i32> {
        let ts = jiff::Timestamp::from_second(epoch_seconds).ok()?;
        Some(jiff::tz::TimeZone::system().to_offset(ts).seconds())
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
        let noat = StyleAttrs::default();
        assert_eq!(sgr_set(GlkStyle::Normal, noat), "");
        assert_eq!(sgr_set(GlkStyle::Header, noat), "\x1b[1m");
        assert_eq!(sgr_set(GlkStyle::Emphasized, noat), "\x1b[3m");
        assert_eq!(sgr_set(GlkStyle::Alert, noat), "\x1b[1m\x1b[7m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, none, noat, true, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(style_wrap("hi", GlkStyle::Header, none, noat, true, false), "hi");
        assert_eq!(style_wrap("hi", GlkStyle::Normal, none, noat, true, true), "hi");
    }

    #[test]
    fn sgr_layers_weight_and_oblique_hints_over_class_defaults() {
        // SQ-0317: a set hint overrides the class look; an unset hint keeps it.
        let bold = StyleAttrs { weight: Some(1), ..Default::default() };
        let unbold = StyleAttrs { weight: Some(0), ..Default::default() };
        let italic = StyleAttrs { oblique: Some(1), ..Default::default() };
        let upright = StyleAttrs { oblique: Some(0), ..Default::default() };
        assert_eq!(sgr_set(GlkStyle::Normal, bold), "\x1b[1m", "weight hint adds bold");
        assert_eq!(sgr_set(GlkStyle::Normal, italic), "\x1b[3m", "oblique hint adds italic");
        assert_eq!(sgr_set(GlkStyle::Header, unbold), "", "weight 0 strips class bold");
        assert_eq!(sgr_set(GlkStyle::Emphasized, upright), "", "oblique 0 strips class italic");
        // "Lighter" (-1 as u32) has no terminal rendering -> plain.
        assert_eq!(sgr_set(GlkStyle::Header, StyleAttrs { weight: Some(u32::MAX), ..Default::default() }), "");
        // Hints layer with the untouched channel: bold hint + Emphasized's italic.
        assert_eq!(sgr_set(GlkStyle::Emphasized, bold), "\x1b[1m\x1b[3m");
    }

    #[test]
    fn stylehint_colour_emits_truecolor_sgr() {
        let fg_bg = StyleColour { fg: Some(0x00FF_8040), bg: Some(0x0011_2233), reverse: false };
        // fg -> 38;2;r;g;b, bg -> 48;2;r;g;b, honoured, wrapped for a TTY.
        assert_eq!(
            style_wrap("x", GlkStyle::Normal, fg_bg, StyleAttrs::default(), true, true),
            "\x1b[38;2;255;128;64m\x1b[48;2;17;34;51mx\x1b[0m"
        );
        // --no-game-colours (honor=false) drops the colour entirely.
        assert_eq!(style_wrap("x", GlkStyle::Normal, fg_bg, StyleAttrs::default(), false, true), "x");
        // reverse hint emits SGR 7 ahead of any colour.
        let rev = StyleColour { fg: None, bg: None, reverse: true };
        assert_eq!(style_wrap("x", GlkStyle::Normal, rev, StyleAttrs::default(), true, true), "\x1b[7mx\x1b[0m");
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
        let grid = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }, Some(true));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 1, width: 80, height: 23 }, Some(true));
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

    // ── page-background OSC helpers ───────────────────────────────────────────

    #[test]
    fn osc_set_bg_formats_hex() {
        assert_eq!(super::osc_set_bg((0x12, 0x34, 0x56)), "\x1b]11;#123456\x07");
        assert_eq!(super::osc_set_bg((0, 0, 0)), "\x1b]11;#000000\x07");
    }

    #[test]
    fn page_bg_escape_emits_only_on_change() {
        assert_eq!(super::page_bg_escape(None, None), None);
        assert_eq!(super::page_bg_escape(Some((1, 2, 3)), None), Some("\x1b]11;#010203\x07".into()));
        assert_eq!(super::page_bg_escape(Some((1, 2, 3)), Some((1, 2, 3))), None);
        assert_eq!(super::page_bg_escape(Some((9, 9, 9)), Some((1, 2, 3))), Some("\x1b]11;#090909\x07".into()));
        assert_eq!(super::page_bg_escape(None, Some((1, 2, 3))), Some("\x1b]111\x07".into()));
    }

    // ── deferred input echo (SQ-0282) ─────────────────────────────────────────

    #[test]
    fn deferred_echo_emitted_when_game_does_not_reprint() {
        // sensory case: the turn's first output is Normal-styled, not a command
        // reprint, so the backend must echo the command itself (library echo).
        let (mut b, buf) = backend(true);
        b.arm_input_echo("look".into());
        b.put_text_attr(1, GlkStyle::Normal, StyleColour::default(), StyleAttrs::default(), 0, "You see nothing.");
        b.flush_out();
        let out = out_string(&buf);
        assert!(out.contains("look"), "library echo emitted: {out:?}");
        assert!(out.contains("You see nothing."), "game output still rendered: {out:?}");
    }

    #[test]
    fn deferred_echo_suppressed_when_game_reprints_in_input_style() {
        // CM case: the game reprints the command in Input style, so the backend
        // must NOT add a second copy.
        let (mut b, buf) = backend(true);
        b.arm_input_echo("look".into());
        b.put_text_attr(1, GlkStyle::Input, StyleColour::default(), StyleAttrs::default(), 0, "look");
        b.flush_out();
        let out = out_string(&buf);
        assert_eq!(out.matches("look").count(), 1, "no duplicate command echo: {out:?}");
    }

    #[test]
    fn deferred_echo_falls_back_on_flush_when_no_buffer_output() {
        // A turn that produces no buffer output before the next prompt must still
        // echo the command (via flush_out) or it would vanish.
        let (mut b, buf) = backend(true);
        b.arm_input_echo("wait".into());
        b.flush_out();
        assert!(out_string(&buf).contains("wait"), "command echoed on flush");
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

    // ── window geometry + borders (SQ-0327) ───────────────────────────────────

    // Helpers to build leaf/pair nodes tersely.
    fn leaf(id: u32, ty: WinType, left: u32, top: u32, width: u32, height: u32) -> WinTree {
        WinTree::Leaf { id, wintype: ty, rect: Rect { left, top, width, height }, bg: None, fg: None }
    }

    #[test]
    fn stacked_split_draws_horizontal_rule_and_confines_buffer_band() {
        // Grid (h=1, top=0) above a buffer (top=2, h=22) with a reserved gutter
        // row at screen row 2. The scroll region confines the buffer to its band
        // and a horizontal rule sits in the gutter.
        let (mut b, buf) = backend(true);
        let grid = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }, Some(true));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 2, width: 80, height: 22 }, Some(true));
        b.window_layout(&[buffer, grid]);
        let tree = WinTree::Pair {
            vertical: true,
            border: true,
            split: 1,
            rect: Rect { left: 0, top: 0, width: 80, height: 24 },
            key_bg: None,
            key_fg: None,
            first: Box::new(leaf(2, WinType::TextGrid, 0, 0, 80, 1)),
            second: Box::new(leaf(1, WinType::TextBuffer, 0, 2, 80, 22)),
        };
        b.window_tree(Some(tree));
        let out = out_string(&buf);
        assert!(out.contains("\x1b[3;24r"), "buffer band confined below the gutter: {out:?}");
        assert!(out.contains(&format!("\x1b[2;1H{}", H_RULE)), "horizontal rule in the gutter row: {out:?}");
    }

    #[test]
    fn left_right_split_draws_vertical_rule_and_buffer_stays_full_width() {
        // Grid column (w=20) left of a buffer column (left=21, w=59), a reserved
        // gutter column at screen col 21. A terminal cannot scroll a sub-column,
        // so the buffer keeps the full-width fallback: NO scroll region is set.
        let (mut b, buf) = backend(true);
        let grid = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 20, height: 24 }, Some(true));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 21, top: 0, width: 59, height: 24 }, Some(true));
        b.window_layout(&[buffer, grid]);
        let tree = WinTree::Pair {
            vertical: false,
            border: true,
            split: 20,
            rect: Rect { left: 0, top: 0, width: 80, height: 24 },
            key_bg: None,
            key_fg: None,
            first: Box::new(leaf(2, WinType::TextGrid, 0, 0, 20, 24)),
            second: Box::new(leaf(1, WinType::TextBuffer, 21, 0, 59, 24)),
        };
        b.window_tree(Some(tree));
        let out = out_string(&buf);
        assert!(!b.region_set, "left/right: no sub-column scroll region (full-width fallback)");
        assert_eq!(b.region_bounds, (0, 0), "no scroll-region bounds recorded for a full-width buffer");
        assert!(out.contains(&format!("\x1b[1;21H{}", V_RULE)), "vertical rule down the gutter column: {out:?}");
    }

    #[test]
    fn noborder_split_draws_no_rule() {
        // A NoBorder stacked split reserves no gutter and must draw no separator.
        let (mut b, buf) = backend(true);
        let grid = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }, Some(false));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 1, width: 80, height: 23 }, Some(false));
        b.window_layout(&[buffer, grid]);
        let tree = WinTree::Pair {
            vertical: true,
            border: false,
            split: 1,
            rect: Rect { left: 0, top: 0, width: 80, height: 24 },
            key_bg: None,
            key_fg: None,
            first: Box::new(leaf(2, WinType::TextGrid, 0, 0, 80, 1)),
            second: Box::new(leaf(1, WinType::TextBuffer, 0, 1, 80, 23)),
        };
        b.window_tree(Some(tree));
        let out = out_string(&buf);
        assert!(!out.contains(H_RULE), "NoBorder draws no horizontal rule: {out:?}");
        assert!(!out.contains(V_RULE), "NoBorder draws no vertical rule: {out:?}");
    }

    #[test]
    fn second_grid_renders_at_its_own_rect() {
        // Two grids (a top status line and a bottom bar) must both be drawn at
        // their rects — not collapsed to a single pinned grid.
        let (mut b, buf) = backend(true);
        let top = (2u32, WinType::TextGrid, Rect { left: 0, top: 0, width: 80, height: 1 }, Some(true));
        let bot = (3u32, WinType::TextGrid, Rect { left: 0, top: 23, width: 80, height: 1 }, Some(true));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 1, width: 80, height: 22 }, Some(true));
        b.window_layout(&[buffer, top, bot]);
        b.grid_put(2, 0, 0, GlkStyle::Normal, "TOP");
        b.grid_put(3, 0, 0, GlkStyle::Normal, "BOTTOM");
        let out = out_string(&buf);
        assert!(out.contains("\x1b[1;1H\x1b[2KTOP"), "top grid at row 1: {out:?}");
        assert!(out.contains("\x1b[24;1H\x1b[2KBOTTOM"), "bottom grid at row 24: {out:?}");
    }

    #[test]
    fn graphics_window_draws_a_placeholder_box() {
        // A graphics window has no image protocol here, so it gets a bordered
        // placeholder box positioned at its rect.
        let (mut b, buf) = backend(true);
        let gfx = (2u32, WinType::Graphics, Rect { left: 0, top: 0, width: 10, height: 3 }, Some(true));
        let buffer = (1u32, WinType::TextBuffer, Rect { left: 0, top: 3, width: 80, height: 21 }, Some(true));
        b.window_layout(&[buffer, gfx]);
        b.window_tree(None); // trigger a chrome redraw
        let out = out_string(&buf);
        // Top edge is a run of horizontal rules at the window's top-left.
        assert!(out.contains(&format!("\x1b[1;1H{}", H_RULE.to_string().repeat(10))), "graphics box top edge: {out:?}");
    }
}
