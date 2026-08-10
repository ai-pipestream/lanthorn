//! Decode a captured terminal byte stream into named escape sequences and the
//! screen those sequences would build (SQ-0762).
//!
//! This half is pure and portable — it takes bytes and gives back a model, so it
//! builds and unit-tests on Windows even though the pty that *produces* the bytes
//! does not. See [`super::driver`] for the capture side.
//!
//! WHY A SCREEN MODEL AND NOT A LIST OF ESCAPES. babelmap places kitty images
//! through VIRTUAL placements: the transmit (`a=T,U=1`) declares an `r`×`c` grid
//! but says nothing about WHERE on screen it lands — the position comes from the
//! U+10EEEE placeholder cells printed afterwards, and their screen coordinates
//! only exist relative to whatever `CUP` preceded them. So the one question this
//! harness was built to answer — *is that row covered by an image, or is it a
//! background colour painted into cells?* — cannot be answered by reading escapes
//! in isolation. It needs a cursor, a current SGR, and a grid. This is that grid.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use unicode_width::UnicodeWidthChar;

/// The kitty Unicode placeholder character. A run of these, with the image id in
/// the foreground colour, IS the placement rect.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// A colour as the stream expressed it — kept in the form it arrived in so the
/// report can say `#2b1f5e` where the app sent a truecolour SGR and `idx(4)`
/// where it sent a palette index. Collapsing both to RGB would invent numbers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, PartialOrd, Ord)]
pub enum Color {
    /// No explicit colour — the terminal's own default (SGR 39/49).
    #[default]
    Default,
    /// A palette index (SGR 30-37/40-47/90-97/100-107, or 38;5;n / 48;5;n).
    Idx(u8),
    /// A truecolour triple (38;2;r;g;b / 48;2;r;g;b).
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn label(&self) -> String {
        match *self {
            Color::Default => "default".into(),
            Color::Idx(i) => format!("idx({i})"),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

/// The SGR state a cell was printed under.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    pub fn label(&self) -> String {
        let mut s = format!("fg {} bg {}", self.fg.label(), self.bg.label());
        for (on, name) in
            [(self.bold, "bold"), (self.dim, "dim"), (self.underline, "underline"), (self.reverse, "reverse")]
        {
            if on {
                s.push(' ');
                s.push_str(name);
            }
        }
        s
    }
}

/// One screen cell as the stream left it.
#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub ch: char,
    /// Zero-width marks that followed `ch` — for a placeholder these are kitty's
    /// row/column diacritics, which carry the image's own row index.
    pub marks: Vec<char>,
    pub style: Style,
    /// False when nothing was ever printed here (the cell is whatever the
    /// terminal had; babelmap never touched it in this capture).
    pub written: bool,
    /// Which flush wrote it last — lets a report say "this frame painted these
    /// rows" instead of only "the screen ends up like this".
    pub flush: usize,
}

impl Cell {
    pub fn is_placeholder(&self) -> bool {
        self.ch == PLACEHOLDER
    }

    /// The kitty image id this placeholder belongs to. babelmap encodes it in the
    /// foreground colour (`ESC[38;2;r;g;b`), which is the protocol's own scheme.
    pub fn image_id(&self) -> Option<u32> {
        match (self.is_placeholder(), self.style.fg) {
            (true, Color::Rgb(r, g, b)) => {
                Some((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b))
            }
            _ => None,
        }
    }
}

/// One kitty APC graphics command, with its continuation chunks folded in.
#[derive(Clone, Debug)]
pub struct ApcCmd {
    pub flush: usize,
    /// The raw `key=value,...` control block of the first chunk.
    pub params: String,
    /// Total base64 payload bytes across this command and its `m=1` chunks.
    pub payload_len: usize,
    /// How many APC sequences the command was split over.
    pub chunks: usize,
}

impl ApcCmd {
    fn get(&self, key: &str) -> Option<&str> {
        self.params.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
    }

    /// A one-line human reading of the command: the action spelled out, the id,
    /// the source pixel size and the cell grid it declares.
    pub fn describe(&self) -> String {
        let action = match self.get("a") {
            Some("T") => "transmit+place",
            Some("t") => "transmit",
            Some("p") => "place",
            Some("d") => "delete",
            Some("q") => "query",
            Some("f") => "frame",
            Some("a") => "animate",
            Some("c") => "compose",
            Some(other) => other,
            None => "continuation",
        };
        let mut s = format!("{action:<14}");
        if let Some(i) = self.get("i") {
            let _ = write!(s, " id={i}");
        }
        if let Some(p) = self.get("p") {
            let _ = write!(s, " placement={p}");
        }
        if let Some(f) = self.get("f") {
            let name = match f {
                "32" => "RGBA",
                "24" => "RGB",
                "100" => "PNG",
                other => other,
            };
            let _ = write!(s, " fmt={name}");
        }
        if let (Some(w), Some(h)) = (self.get("s"), self.get("v")) {
            let _ = write!(s, " src={w}x{h}px");
        }
        if let (Some(c), Some(r)) = (self.get("c"), self.get("r")) {
            let _ = write!(s, " grid={c}c x {r}r");
        }
        if let (Some(x), Some(y)) = (self.get("x"), self.get("y")) {
            let _ = write!(s, " srcorigin=({x},{y})");
        }
        if self.get("U") == Some("1") {
            s.push_str(" virtual");
        }
        if let Some(d) = self.get("d") {
            let what = match d {
                "i" | "I" => "by id",
                "a" | "A" => "all",
                "c" | "C" => "at cursor",
                "p" | "P" => "at cell",
                other => other,
            };
            let _ = write!(s, " delete={what}");
        }
        if self.payload_len > 0 {
            let _ = write!(s, "  payload={}B in {} chunk(s)", self.payload_len, self.chunks);
        }
        s
    }
}

/// A decoded virtual placement: every cell carrying one image id.
#[derive(Clone, Debug)]
pub struct Placement {
    pub image_id: u32,
    pub top: u16,
    pub bottom: u16,
    pub left: u16,
    pub right: u16,
    pub cells: usize,
    /// `row -> (first col, last col)`, so a ragged placement is visible as one.
    pub rows: BTreeMap<u16, (u16, u16)>,
}

impl Placement {
    pub fn describe(&self) -> String {
        format!(
            "image {} rows {}..{} cols {}..{}  ({} cols x {} rows, {} cells)",
            self.image_id,
            self.top,
            self.bottom,
            self.left,
            self.right,
            self.right - self.left + 1,
            self.bottom - self.top + 1,
            self.cells,
        )
    }
}

/// A run of adjacent cells on one row sharing a background.
#[derive(Clone, Copy, Debug)]
pub struct BgRun {
    pub row: u16,
    pub col0: u16,
    pub col1: u16,
    pub bg: Color,
    /// True when every cell in the run is a kitty placeholder — i.e. an image
    /// covers it and the background is whatever shows through, not paint.
    pub all_placeholder: bool,
}

/// What each parser state is waiting for. A deliberately small subset: enough for
/// what ratatui + crossterm actually emit, with everything else recorded as
/// unknown rather than silently dropped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
    Osc,
    Apc,
    /// A DCS/SOS/PM string we only need to skip to its ST.
    StringSkip,
}

/// The decoded stream: a screen model plus everything notable that built it.
pub struct Term {
    pub cols: u16,
    pub rows: u16,
    cells: Vec<Cell>,
    cur_row: u16,
    cur_col: u16,
    /// DEC deferred wrap: the cursor sits ON the last column with a pending wrap.
    wrap_pending: bool,
    saved: (u16, u16),
    style: Style,
    saved_style: Style,
    flush: usize,

    state: State,
    buf: Vec<u8>,
    utf8: Vec<u8>,
    utf8_need: usize,

    pub apc: Vec<ApcCmd>,
    pub osc: Vec<(usize, String)>,
    pub modes: Vec<(usize, String)>,
    pub erases: Vec<(usize, String)>,
    pub unknown: Vec<(usize, String)>,
    pub cup_count: usize,
    pub sgr_count: usize,
    pub printed_cells: usize,
    /// Printed-cell counts per flush, so a report can name the frames that drew.
    pub flush_prints: Vec<usize>,
}

impl Term {
    pub fn new(cols: u16, rows: u16) -> Self {
        Term {
            cols,
            rows,
            cells: vec![Cell::default(); cols as usize * rows as usize],
            cur_row: 0,
            cur_col: 0,
            wrap_pending: false,
            saved: (0, 0),
            style: Style::default(),
            saved_style: Style::default(),
            flush: 0,
            state: State::Ground,
            buf: Vec::new(),
            utf8: Vec::new(),
            utf8_need: 0,
            apc: Vec::new(),
            osc: Vec::new(),
            modes: Vec::new(),
            erases: Vec::new(),
            unknown: Vec::new(),
            cup_count: 0,
            sgr_count: 0,
            printed_cells: 0,
            flush_prints: vec![0],
        }
    }

    /// Start attributing writes to flush `n` (the capture's own grouping of the
    /// byte stream into output bursts).
    pub fn set_flush(&mut self, n: usize) {
        self.flush = n;
        while self.flush_prints.len() <= n {
            self.flush_prints.push(0);
        }
    }

    pub fn cell(&self, row: u16, col: u16) -> &Cell {
        &self.cells[row as usize * self.cols as usize + col as usize]
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    fn byte(&mut self, b: u8) {
        match self.state {
            State::Ground => self.ground(b),
            State::Esc => {
                self.state = State::Ground;
                match b {
                    b'[' => {
                        self.buf.clear();
                        self.state = State::Csi;
                    }
                    b']' => {
                        self.buf.clear();
                        self.state = State::Osc;
                    }
                    b'_' => {
                        self.buf.clear();
                        self.state = State::Apc;
                    }
                    b'P' | b'X' | b'^' => {
                        self.buf.clear();
                        self.state = State::StringSkip;
                    }
                    b'7' => {
                        self.saved = (self.cur_row, self.cur_col);
                        self.saved_style = self.style;
                    }
                    b'8' => {
                        (self.cur_row, self.cur_col) = self.saved;
                        self.style = self.saved_style;
                        self.wrap_pending = false;
                    }
                    b'\\' => {}
                    // Charset selects and the like: two-byte, second byte skipped
                    // by simply treating it as ground next round. Recorded so the
                    // report never claims to have understood more than it did.
                    other => {
                        let f = self.flush;
                        self.unknown.push((f, format!("ESC {}", esc_char(other))));
                    }
                }
            }
            State::Csi => {
                self.buf.push(b);
                if (0x40..=0x7e).contains(&b) {
                    let seq = std::mem::take(&mut self.buf);
                    self.csi(&seq);
                    self.state = State::Ground;
                }
            }
            State::Osc | State::Apc | State::StringSkip => {
                self.buf.push(b);
                let ends_st = self.buf.ends_with(b"\x1b\\");
                let ends_bel = b == 0x07;
                if ends_st || ends_bel {
                    let mut seq = std::mem::take(&mut self.buf);
                    seq.truncate(seq.len() - if ends_st { 2 } else { 1 });
                    let text = String::from_utf8_lossy(&seq).into_owned();
                    match self.state {
                        State::Apc => self.apc(&text),
                        State::Osc => self.osc.push((self.flush, text)),
                        _ => {}
                    }
                    self.state = State::Ground;
                }
            }
        }
    }

    fn ground(&mut self, b: u8) {
        if self.utf8_need > 0 {
            self.utf8.push(b);
            self.utf8_need -= 1;
            if self.utf8_need == 0 {
                let s = String::from_utf8_lossy(&std::mem::take(&mut self.utf8)).into_owned();
                for ch in s.chars() {
                    self.print(ch);
                }
            }
            return;
        }
        match b {
            0x1b => self.state = State::Esc,
            b'\r' => {
                self.cur_col = 0;
                self.wrap_pending = false;
            }
            b'\n' => {
                self.newline();
            }
            0x08 => {
                self.cur_col = self.cur_col.saturating_sub(1);
                self.wrap_pending = false;
            }
            b'\t' => {
                self.cur_col = ((self.cur_col / 8) + 1) * 8;
                self.cur_col = self.cur_col.min(self.cols.saturating_sub(1));
                self.wrap_pending = false;
            }
            0x00..=0x1f | 0x7f => {}
            0x20..=0x7e => self.print(b as char),
            _ => {
                // UTF-8 lead byte: collect the continuation bytes before printing.
                let need = match b {
                    0xc0..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf7 => 3,
                    _ => 0,
                };
                if need == 0 {
                    return;
                }
                self.utf8.clear();
                self.utf8.push(b);
                self.utf8_need = need;
            }
        }
    }

    fn newline(&mut self) {
        self.wrap_pending = false;
        if self.cur_row + 1 < self.rows {
            self.cur_row += 1;
        }
        // A scroll at the bottom is not modelled: babelmap draws with absolute
        // cursor moves inside the alternate screen and never relies on scrolling,
        // so pinning the row is closer to the truth than shifting the whole grid.
    }

    fn print(&mut self, ch: char) {
        let w = ch.width().unwrap_or(0);
        if w == 0 {
            // A combining mark belongs to the cell before the cursor — which is
            // how kitty's row/column diacritics ride on a placeholder.
            let col = if self.wrap_pending { self.cur_col } else { self.cur_col.saturating_sub(1) };
            if let Some(c) = self.cell_mut(self.cur_row, col) {
                c.marks.push(ch);
            }
            return;
        }
        if self.wrap_pending {
            self.cur_col = 0;
            self.newline();
            self.wrap_pending = false;
        }
        let (row, col) = (self.cur_row, self.cur_col);
        let style = self.style;
        let flush = self.flush;
        if let Some(c) = self.cell_mut(row, col) {
            c.ch = ch;
            c.marks.clear();
            c.style = style;
            c.written = true;
            c.flush = flush;
        }
        self.printed_cells += 1;
        if let Some(n) = self.flush_prints.get_mut(flush) {
            *n += 1;
        }
        // A double-width glyph eats the next cell too; mark it so a background
        // run does not report a hole where the trailing half sits.
        if w == 2 {
            if let Some(c) = self.cell_mut(row, col + 1) {
                c.ch = ' ';
                c.style = style;
                c.written = true;
                c.flush = flush;
            }
        }
        let next = self.cur_col as usize + w;
        if next >= self.cols as usize {
            self.cur_col = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.cur_col = next as u16;
        }
    }

    fn cell_mut(&mut self, row: u16, col: u16) -> Option<&mut Cell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let i = row as usize * self.cols as usize + col as usize;
        self.cells.get_mut(i)
    }

    fn csi(&mut self, seq: &[u8]) {
        let final_byte = *seq.last().unwrap_or(&b'?');
        let body = &seq[..seq.len() - 1];
        let private = body.first().is_some_and(|c| matches!(c, b'?' | b'>' | b'<' | b'='));
        let text = String::from_utf8_lossy(if private { &body[1..] } else { body }).into_owned();
        let nums: Vec<u32> = text
            .split(';')
            .map(|p| p.split(':').next().unwrap_or("").parse::<u32>().unwrap_or(0))
            .collect();
        let n = |i: usize, dflt: u32| -> u32 {
            match nums.get(i) {
                Some(0) | None => dflt,
                Some(v) => *v,
            }
        };
        self.wrap_pending = false;
        match (private, final_byte) {
            (false, b'H') | (false, b'f') => {
                self.cur_row = (n(0, 1) as u16).saturating_sub(1).min(self.rows - 1);
                self.cur_col = (n(1, 1) as u16).saturating_sub(1).min(self.cols - 1);
                self.cup_count += 1;
            }
            (false, b'A') => self.cur_row = self.cur_row.saturating_sub(n(0, 1) as u16),
            (false, b'B') => self.cur_row = (self.cur_row + n(0, 1) as u16).min(self.rows - 1),
            (false, b'C') => self.cur_col = (self.cur_col + n(0, 1) as u16).min(self.cols - 1),
            (false, b'D') => self.cur_col = self.cur_col.saturating_sub(n(0, 1) as u16),
            (false, b'E') => {
                self.cur_col = 0;
                self.cur_row = (self.cur_row + n(0, 1) as u16).min(self.rows - 1);
            }
            (false, b'F') => {
                self.cur_col = 0;
                self.cur_row = self.cur_row.saturating_sub(n(0, 1) as u16);
            }
            (false, b'G') => self.cur_col = (n(0, 1) as u16).saturating_sub(1).min(self.cols - 1),
            (false, b'd') => self.cur_row = (n(0, 1) as u16).saturating_sub(1).min(self.rows - 1),
            (false, b's') => {
                self.saved = (self.cur_row, self.cur_col);
                self.saved_style = self.style;
            }
            (false, b'u') => {
                (self.cur_row, self.cur_col) = self.saved;
                self.style = self.saved_style;
            }
            (false, b'm') => {
                self.sgr(&text);
                self.sgr_count += 1;
            }
            (false, b'J') => {
                let what = match nums.first().copied().unwrap_or(0) {
                    0 => "to end of screen",
                    1 => "to start of screen",
                    _ => "whole screen",
                };
                self.erase_screen(nums.first().copied().unwrap_or(0));
                self.erases.push((self.flush, format!("erase display {what}")));
            }
            (false, b'K') => {
                let mode = nums.first().copied().unwrap_or(0);
                let what = match mode {
                    0 => "to end of line",
                    1 => "to start of line",
                    _ => "whole line",
                };
                self.erase_line(mode);
                self.erases.push((self.flush, format!("erase line {what} (row {})", self.cur_row)));
            }
            (true, b'h') | (true, b'l') => {
                let on = final_byte == b'h';
                self.modes.push((self.flush, format!("{} mode ?{text}", if on { "set" } else { "reset" })));
            }
            // Cursor visibility, DA, DSR, window ops: no screen effect worth
            // modelling, but they are named rather than dropped.
            (_, b'n') | (_, b'c') | (_, b't') | (_, b'q') => {
                self.unknown.push((self.flush, format!("CSI {text}{}", esc_char(final_byte))));
            }
            _ => self.unknown.push((self.flush, format!("CSI {text}{}", esc_char(final_byte)))),
        }
    }

    fn erase_screen(&mut self, mode: u32) {
        let len = self.cells.len();
        let cur = self.cur_row as usize * self.cols as usize + self.cur_col as usize;
        let (from, to) = match mode {
            0 => (cur, len),
            1 => (0, cur + 1),
            _ => (0, len),
        };
        let (style, flush) = (self.style, self.flush);
        for c in &mut self.cells[from.min(len)..to.min(len)] {
            *c = Cell { ch: ' ', marks: Vec::new(), style, written: true, flush };
        }
    }

    fn erase_line(&mut self, mode: u32) {
        let len = self.cells.len();
        let row = self.cur_row as usize * self.cols as usize;
        let (from, to) = match mode {
            0 => (row + self.cur_col as usize, row + self.cols as usize),
            1 => (row, row + self.cur_col as usize + 1),
            _ => (row, row + self.cols as usize),
        };
        let (style, flush) = (self.style, self.flush);
        for c in &mut self.cells[from.min(len)..to.min(len)] {
            *c = Cell { ch: ' ', marks: Vec::new(), style, written: true, flush };
        }
    }

    fn sgr(&mut self, text: &str) {
        // Sub-parameter colon form (38:2::r:g:b) is normalised to semicolons so
        // one walker handles both spellings.
        let parts: Vec<&str> = text.split([';', ':']).collect();
        let mut i = 0;
        if text.is_empty() {
            self.style = Style::default();
            return;
        }
        while i < parts.len() {
            let p: u32 = parts[i].parse().unwrap_or(0);
            match p {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                2 => self.style.dim = true,
                4 => self.style.underline = true,
                7 => self.style.reverse = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                24 => self.style.underline = false,
                27 => self.style.reverse = false,
                30..=37 => self.style.fg = Color::Idx((p - 30) as u8),
                39 => self.style.fg = Color::Default,
                40..=47 => self.style.bg = Color::Idx((p - 40) as u8),
                49 => self.style.bg = Color::Default,
                90..=97 => self.style.fg = Color::Idx((p - 90 + 8) as u8),
                100..=107 => self.style.bg = Color::Idx((p - 100 + 8) as u8),
                38 | 48 => {
                    let want_fg = p == 38;
                    let kind: u32 = parts.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    let take = |k: usize| -> u8 {
                        parts.get(k).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0) as u8
                    };
                    if kind == 2 {
                        // Both `38;2;r;g;b` and the colon form with an empty
                        // colour-space slot (`38:2::r:g:b`) occur in the wild.
                        let base = if parts.get(i + 2).is_some_and(|s| s.is_empty()) { i + 3 } else { i + 2 };
                        let c = Color::Rgb(take(base), take(base + 1), take(base + 2));
                        if want_fg {
                            self.style.fg = c;
                        } else {
                            self.style.bg = c;
                        }
                        i = base + 2;
                    } else if kind == 5 {
                        let c = Color::Idx(take(i + 2));
                        if want_fg {
                            self.style.fg = c;
                        } else {
                            self.style.bg = c;
                        }
                        i += 2;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn apc(&mut self, text: &str) {
        let Some(rest) = text.strip_prefix('G') else {
            self.unknown.push((self.flush, format!("APC {text}")));
            return;
        };
        let (params, payload) = match rest.split_once(';') {
            Some((p, d)) => (p.to_string(), d.len()),
            None => (rest.to_string(), 0),
        };
        // A continuation chunk carries no action, only `m=`; fold it into the
        // command it continues so the report reads as one upload, not nineteen.
        let is_continuation = !params.split(',').any(|kv| kv.starts_with("a="));
        if is_continuation {
            if let Some(last) = self.apc.last_mut() {
                last.payload_len += payload;
                last.chunks += 1;
                return;
            }
        }
        self.apc.push(ApcCmd { flush: self.flush, params, payload_len: payload, chunks: 1 });
    }

    // ── Readings of the finished screen ──────────────────────────────────────

    /// Every virtual placement on the final screen, by image id.
    pub fn placements(&self) -> Vec<Placement> {
        let mut by_id: BTreeMap<u32, Placement> = BTreeMap::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let c = self.cell(row, col);
                let Some(id) = c.image_id() else { continue };
                let p = by_id.entry(id).or_insert(Placement {
                    image_id: id,
                    top: row,
                    bottom: row,
                    left: col,
                    right: col,
                    cells: 0,
                    rows: BTreeMap::new(),
                });
                p.top = p.top.min(row);
                p.bottom = p.bottom.max(row);
                p.left = p.left.min(col);
                p.right = p.right.max(col);
                p.cells += 1;
                let e = p.rows.entry(row).or_insert((col, col));
                e.0 = e.0.min(col);
                e.1 = e.1.max(col);
            }
        }
        by_id.into_values().collect()
    }

    /// Background runs on `row`, restricted to `cols`, merging adjacent cells
    /// that share a background. Unwritten cells count as `Default`.
    pub fn bg_runs(&self, row: u16, cols: std::ops::Range<u16>) -> Vec<BgRun> {
        let mut out: Vec<BgRun> = Vec::new();
        for col in cols {
            if col >= self.cols {
                break;
            }
            let c = self.cell(row, col);
            let bg = if c.written { c.style.bg } else { Color::Default };
            let ph = c.is_placeholder();
            match out.last_mut() {
                Some(r) if r.bg == bg && r.all_placeholder == ph && r.col1 + 1 == col => r.col1 = col,
                _ => out.push(BgRun { row, col0: col, col1: col, bg, all_placeholder: ph }),
            }
        }
        out
    }

    /// The row's glyphs over `cols`, with placeholders shown as `▒` so an image
    /// region is legible as one rather than as a wall of unprintable runes.
    pub fn row_text(&self, row: u16, cols: std::ops::Range<u16>) -> String {
        cols.filter(|c| *c < self.cols)
            .map(|col| {
                let c = self.cell(row, col);
                if c.is_placeholder() {
                    '▒'
                } else if !c.written {
                    '·'
                } else {
                    c.ch
                }
            })
            .collect()
    }
}

fn esc_char(b: u8) -> String {
    if (0x20..0x7f).contains(&b) {
        (b as char).to_string()
    } else {
        format!("\\x{b:02x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placement rect and a painted background are DIFFERENT BUGS, and telling
    /// them apart is the whole reason this decoder exists (SQ-0762). Two synthetic
    /// streams that look identical to a geometry dump — same rows covered, same
    /// colour on screen — must decode differently here.
    #[test]
    fn a_placement_rect_and_a_painted_background_decode_differently() {
        // Stream A: an image placed over rows 0..1 via kitty placeholders.
        let mut a = Term::new(10, 4);
        a.feed(b"\x1b_Gq=2,i=99,a=T,U=1,f=32,t=d,s=80,v=36,r=2,c=4,m=0;AAAA\x1b\\");
        for row in 0..2u16 {
            a.feed(format!("\x1b[{};1H\x1b[38;2;0;0;99m", row + 1).as_bytes());
            a.feed("\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{10EEEE}\u{10EEEE}".as_bytes());
        }
        let pa = a.placements();
        assert_eq!(pa.len(), 1, "one image");
        assert_eq!((pa[0].image_id, pa[0].top, pa[0].bottom, pa[0].left, pa[0].right), (99, 0, 1, 0, 3));
        assert_eq!(a.apc.len(), 1);
        assert!(a.apc[0].describe().starts_with("transmit+place"), "{}", a.apc[0].describe());

        // Stream B: no image at all — the same four columns filled with spaces
        // under a background SGR.
        let mut b = Term::new(10, 4);
        for row in 0..2u16 {
            b.feed(format!("\x1b[{};1H\x1b[48;2;40;30;90m    ", row + 1).as_bytes());
        }
        assert!(b.placements().is_empty(), "a painted fill is not a placement");
        let runs = b.bg_runs(0, 0..4);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].bg, Color::Rgb(40, 30, 90));
        assert!(!runs[0].all_placeholder, "these cells are paint, not an image");
    }

    /// The proof target's shape in miniature: an image covering the top rows and
    /// a background fill running PAST it. The decoder has to attribute each row
    /// to the right mechanism, because that is what names the fix.
    #[test]
    fn a_fill_that_overruns_its_placement_is_attributable_row_by_row() {
        let mut t = Term::new(8, 6);
        for row in 0..3u16 {
            t.feed(format!("\x1b[{};1H\x1b[48;2;40;30;90m\x1b[38;2;0;0;7m", row + 1).as_bytes());
            t.feed("\u{10EEEE}\u{305}\u{305}\u{10EEEE}\u{10EEEE}\u{10EEEE}".as_bytes());
        }
        for row in 3..6u16 {
            t.feed(format!("\x1b[{};1H\x1b[48;2;40;30;90m\x1b[39m    ", row + 1).as_bytes());
        }
        let p = &t.placements()[0];
        assert_eq!((p.top, p.bottom), (0, 2), "the image stops at row 2");
        for row in 0..6u16 {
            let runs = t.bg_runs(row, 0..4);
            assert_eq!(runs.len(), 1, "row {row} is one background run");
            assert_eq!(runs[0].bg, Color::Rgb(40, 30, 90), "row {row} carries the fill colour");
            assert_eq!(
                runs[0].all_placeholder,
                row < 3,
                "row {row}: rows 0..2 are covered by the image, rows 3..5 are painted cells"
            );
        }
    }

    #[test]
    fn sgr_truecolour_and_palette_forms_both_decode() {
        let mut t = Term::new(4, 1);
        t.feed(b"\x1b[1;1H\x1b[38;2;1;2;3;48;5;9mX");
        assert_eq!(t.cell(0, 0).style.fg, Color::Rgb(1, 2, 3));
        assert_eq!(t.cell(0, 0).style.bg, Color::Idx(9));
        t.feed(b"\x1b[0m\x1b[48:2::9:8:7mY");
        assert_eq!(t.cell(0, 1).style.bg, Color::Rgb(9, 8, 7));
        assert_eq!(t.cell(0, 1).style.fg, Color::Default);
    }

    #[test]
    fn an_apc_upload_folds_its_continuation_chunks() {
        let mut t = Term::new(4, 1);
        t.feed(b"\x1b_Gq=2,i=7,a=T,U=1,f=32,t=d,s=8,v=8,r=1,c=1,m=1;AAAA\x1b\\");
        t.feed(b"\x1b_Gq=2,m=1;BBBB\x1b\\");
        t.feed(b"\x1b_Gq=2,m=0;CC\x1b\\");
        assert_eq!(t.apc.len(), 1, "one upload, not three");
        assert_eq!(t.apc[0].chunks, 3);
        assert_eq!(t.apc[0].payload_len, 10);
    }
}
