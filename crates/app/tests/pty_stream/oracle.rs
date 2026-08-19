//! A second, independent opinion on the same bytes (SQ-0764).
//!
//! WHY THIS EXISTS. [`super::decode`] is our own hand-rolled terminal: a few
//! hundred lines of cursor, SGR and placeholder bookkeeping written by the same
//! hands as the code it audits. When it says "an image covers those cells" and
//! the screen disagrees, there is no way to tell which of the two is lying — the
//! harness and the subject share an author, and a shared misreading of the kitty
//! protocol is invisible to both. So this module runs the identical byte stream
//! through `qwertty-term-vt`, Ghostty's terminal core ported to Rust, and reports
//! what a REAL emulator resolved. Two decoders that agree are evidence; one
//! decoder is an opinion.
//!
//! The two are not redundant. Ours knows things the emulator cannot: which flush
//! wrote a cell, what the APC control blocks said, whether an upload was ever
//! sent at all. The emulator knows the one thing ours only guesses at: whether a
//! run of `U+10EEEE` cells actually RESOLVES — whether the image id those cells
//! encode names an image the terminal holds, and whether the run's diacritics let
//! it find its own row and column. Ours reports placeholder cells; this one
//! reports pixels that would land.
//!
//! THE DIFFERENCE THAT PAID FOR THE CRATE. lanthorn paints a virtual placement
//! one run per row, and only the run's LEAD cell carries the diacritic triple
//! (image row index, image column index, the image id's high byte). Every cell
//! after it is a bare `U+10EEEE` leaning on the protocol's continuation rule: a
//! placeholder with no diacritics extends the run to its left. Overpaint that
//! lead cell — a later frame drawing a border, a divider, a space — and the rest
//! of the run loses its anchor. The remaining cells still carry the foreground
//! colour, so they still name an image id, but only its LOW 24 BITS: the high
//! byte lived in the third diacritic and died with the lead cell. If the real id
//! has a non-zero high byte the lookup misses and a real terminal draws NOTHING
//! there. If the high byte happens to be zero — which is lanthorn's own id range,
//! `0x00B0_xxxx` — the lookup succeeds but the orphaned run now claims image row
//! 0 and column 0, so every row of the art redraws the art's first row. Our
//! decoder sees placeholder cells and reports an image in both cases. This one
//! does not. `pty_oracle.rs` pins exactly that.
//!
//! PORTABLE ON PURPOSE. No `cfg(unix)` anywhere below. The pty that *produces*
//! bytes is unix-only, but resolving bytes is arithmetic, and the hand-authored
//! half of `pty_oracle.rs` must run on Windows like any other unit test.
//!
//! WHAT THE CRATE'S SHAPES DO AND DO NOT CARRY (all measured, not assumed):
//!   * a virtual placement resolves to one `RenderImagePlacement` PER ROW — a
//!     40x21-cell image is 21 entries — so a rect only exists after aggregating;
//!   * `RenderImagePlacement` carries no placement id, no virtual flag, and
//!     hardcodes `z = -1` for virtual placements (upstream draws them under
//!     text). The transmit's authored `z=` survives only in
//!     `ImageStorage::placements`, which is where [`ImageRect::z`] reads it;
//!   * deriving a cell width from `dest_width.div_ceil(cell_w)` UNDER-REPORTS
//!     whenever aspect-fit shrinks the image inside its cell box, so the virtual
//!     path counts cells with [`unicode::placement_iterator`] instead, and only
//!     pin placements — which have no placeholder cells to count — fall back to
//!     pixel arithmetic;
//!   * the two decoders name images differently. Ours reads the placeholder's
//!     foreground colour, which is the LOW 24 BITS; this one reports the full
//!     32-bit `i=` value. `full = low24 | (high << 24)`, so [`disagreements`]
//!     matches on the low 24 bits and nothing else.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use qwertty_term_vt::kitty::storage::Location;
use qwertty_term_vt::kitty::{TerminalGeometry, resolve_placements, resolve_window, unicode};
use qwertty_term_vt::point::Tag;
use qwertty_term_vt::snapshot::{SnapshotColor, SnapshotWindow};
use qwertty_term_vt::stream::{Stream, TerminalHandler};
use qwertty_term_vt::terminal::{Options, Terminal};

use super::decode::{self, Color};

/// The low 24 bits an `ESC[38;2;r;g;b` foreground can carry — the only part of
/// an image id both decoders can always see.
pub const ID_MASK: u32 = 0x00ff_ffff;

/// How a resolved rect got its cells: walked back off `U+10EEEE` placeholder
/// cells (`U=1`), or anchored to a screen cell by the transmit itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Origin {
    /// A `U=1` virtual placement, positioned by placeholder cells.
    Virtual,
    /// A placement anchored to a tracked screen pin.
    Pin,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Virtual => "virtual",
            Origin::Pin => "pin",
        }
    }
}

/// One cell as a real terminal resolved it.
#[derive(Clone, Debug, Default)]
pub struct OracleCell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    /// The FULL 32-bit id of the image whose pixels land on this cell, if any.
    /// `None` means a renderer would draw no image here — a different statement
    /// from "no placeholder was printed here" (see the module docs).
    pub image_id: Option<u32>,
    /// Which PIXEL ROW of that image lands here — the resolved run's `source_y`.
    ///
    /// The one reading that tells a correct placement from a corrupt one. A run
    /// that lost its row diacritic still resolves (when the id's high byte is
    /// zero) and still covers the same cells, so a rect comparison cannot see the
    /// damage: every screen row simply redraws the image's FIRST row. The source
    /// row can see it — down a healthy placement this climbs, and down a broken
    /// one it is 0 all the way (SQ-0772).
    pub source_y: Option<u32>,
    /// SGR 7. Carried rather than folded into `fg`/`bg` because [`disagreements`]
    /// compares this cell's colours against our own decoder's, which does not
    /// swap either — pre-swapping here would invent a divergence out of an
    /// attribute both models read the same way. The rasteriser swaps; the
    /// comparison does not.
    pub inverse: bool,
}

/// Every rendering placement of one image, aggregated from its per-row entries
/// into a single cell rect.
#[derive(Clone, Debug)]
pub struct ImageRect {
    /// The full 32-bit image id. Our decoder's ids are this masked to 24 bits.
    pub image_id: u32,
    pub top: u16,
    pub bottom: u16,
    pub left: u16,
    pub right: u16,
    /// Cells actually covered — fewer than the bounding box when the rows are
    /// ragged, which is what a half-overpainted placement looks like.
    pub cells: usize,
    /// The z the transmit ASKED for, read back off the stored placement.
    /// `RenderImagePlacement::z` cannot supply it: that field reports the -1
    /// upstream draws every virtual placement at, whatever the client wrote.
    /// When one image carries several placements at different z, the lowest.
    pub z: i32,
    pub origin: Origin,
}

impl ImageRect {
    pub fn describe(&self) -> String {
        format!(
            "image {:#010x} rows {}..{} cols {}..{}  ({} cols x {} rows, {} cells, z={}, {})",
            self.image_id,
            self.top,
            self.bottom,
            self.left,
            self.right,
            self.right - self.left + 1,
            self.bottom - self.top + 1,
            self.cells,
            self.z,
            self.origin.label(),
        )
    }
}

/// One resolved placement's PIXEL geometry: which pixels of which image land
/// where on the screen.
///
/// [`ImageRect`] answers a different question — which CELLS an image covers —
/// and answers it aggregated, one entry per image. A rasteriser cannot use that:
/// a virtual placement resolves one entry per screen row, each with its own
/// source row, and collapsing them loses exactly the reading that tells a
/// healthy placement from one redrawing its first row (see [`OracleCell::source_y`]).
/// So the draws stay one-per-resolved-placement, unaggregated, and a picture
/// drawn from them shows that corruption instead of smoothing it over.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    pub image_id: u32,
    /// The z a RENDERER orders by — NOT [`ImageRect::z`], which reports the z the
    /// client asked for. Upstream hardcodes `-1` for every virtual placement, and
    /// a picture that wants to look like the screen has to obey the number the
    /// renderer actually sorts on, not the one the client wished for.
    pub z: i32,
    /// Top-left destination pixel. Signed: a placement whose top has scrolled
    /// above the window has a negative row, and a rasteriser clips it the way a
    /// GPU would.
    pub dest_x: i64,
    pub dest_y: i64,
    pub dest_w: u32,
    pub dest_h: u32,
    /// The source rectangle within the image, already clamped to its bounds.
    pub src_x: u32,
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
}

/// One image the terminal holds, decoded to tightly-packed RGBA. lanthorn
/// transmits `f=32` (raw RGBA) so this is usually a straight copy, but the
/// decode goes through the crate's own converter, which handles the gray/RGB
/// formats too — the rasteriser must not care which format arrived.
#[derive(Clone, Debug)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The numbers a [`Color`] needs before it can become pixels: the terminal's
/// live 256-entry palette (OSC 4 changes it) and the defaults an unstyled cell
/// falls back to (OSC 10/11 change those).
///
/// When the app never set a dynamic default — lanthorn does not — the fallback
/// is the palette's own black and light grey rather than an invented pair, so
/// every colour in a rendered picture traces back to something the stream said.
#[derive(Clone)]
pub struct Colors {
    palette: [[u8; 3]; 256],
    default_fg: [u8; 3],
    default_bg: [u8; 3],
}

impl Colors {
    fn from_window(win: &SnapshotWindow) -> Colors {
        let mut palette = [[0u8; 3]; 256];
        for (dst, src) in palette.iter_mut().zip(win.palette.iter()) {
            *dst = [src.r, src.g, src.b];
        }
        Colors {
            default_fg: win.default_fg.map_or(palette[7], |c| [c.r, c.g, c.b]),
            default_bg: win.default_bg.map_or(palette[0], |c| [c.r, c.g, c.b]),
            palette,
        }
    }

    /// The colour to draw a glyph in. `Default` resolves to the terminal's
    /// default foreground.
    pub fn fg(&self, c: Color) -> [u8; 3] {
        self.resolve(c, self.default_fg)
    }

    /// The colour to fill a cell with. `Default` resolves to the terminal's
    /// default background.
    pub fn bg(&self, c: Color) -> [u8; 3] {
        self.resolve(c, self.default_bg)
    }

    /// The default background, which is also the colour the screen starts as.
    pub fn default_bg(&self) -> [u8; 3] {
        self.default_bg
    }

    fn resolve(&self, c: Color, default: [u8; 3]) -> [u8; 3] {
        match c {
            Color::Default => default,
            Color::Idx(i) => self.palette[i as usize],
            Color::Rgb(r, g, b) => [r, g, b],
        }
    }
}

/// A real terminal's reading of a byte stream: the grid it built and the images
/// it would actually draw on it.
pub struct Resolved {
    pub cols: u16,
    pub rows: u16,
    /// Pixels per cell, as answered to `CSI 16 t` — the units every pixel number
    /// below is in.
    pub cell_w: u32,
    pub cell_h: u32,
    cells: Vec<OracleCell>,
    pub placements: Vec<ImageRect>,
    /// Every placement's pixel geometry, for [`super::raster`]. Unaggregated and
    /// in no particular order; a renderer sorts them by z.
    pub draws: Vec<Draw>,
    /// The pixels of every image a `draws` entry names, by image id.
    pub images: BTreeMap<u32, RasterImage>,
    /// The palette and defaults the cell colours above are indices into.
    pub colors: Colors,
}

impl Resolved {
    pub fn cell(&self, row: u16, col: u16) -> &OracleCell {
        &self.cells[row as usize * self.cols as usize + col as usize]
    }

    /// The resolved placements, one line each, for a report.
    pub fn describe_placements(&self) -> String {
        let mut s = String::new();
        if self.placements.is_empty() {
            let _ = writeln!(s, "  (none — a real terminal would draw no image on this screen)");
        }
        for p in &self.placements {
            let _ = writeln!(s, "  {}", p.describe());
        }
        s
    }
}

/// The screen geometry the resolution needs, gathered so the helpers below stay
/// readable rather than taking eight scalars each.
#[derive(Clone, Copy)]
struct Grid {
    cols: u16,
    rows: u16,
    cell_w: u32,
    cell_h: u32,
    /// Absolute screen row of the window's top row, so a pin's absolute row can
    /// be expressed the way a renderer sees it.
    window_top: usize,
}

/// Feed `bytes` to a real terminal emulator and read back the grid and the
/// images it resolved.
///
/// `cell_w`/`cell_h` are the pixel size of one cell — the same numbers the pty
/// driver answers `CSI 16 t` with. They matter: kitty's aspect-fit maths is in
/// pixels, and a placement whose destination rounds to zero pixels is one a
/// renderer skips.
///
/// The ACTIVE screen is read, so lanthorn's `?1049` alternate screen needs no
/// special handling — but a capture that outlived the app's exit from the
/// alternate screen resolves against the primary screen, which is empty. That is
/// the truth about those bytes, not a fault in this function.
pub fn resolve(bytes: &[u8], cols: u16, rows: u16, cell_w: u32, cell_h: u32) -> Resolved {
    let mut t = Terminal::new(Options { cols, rows, max_scrollback: 10_000, ..Default::default() });
    // Pixel geometry is not a constructor argument; these are public fields and
    // the kitty model reads them directly.
    t.width_px = u32::from(cols) * cell_w;
    t.height_px = u32::from(rows) * cell_h;

    // One-shot feeding is equivalent to chunking (the parser is a state machine
    // carried across calls) and measurably faster on a multi-megabyte capture.
    let mut stream = Stream::new(TerminalHandler::new(t));
    stream.feed(bytes);
    let t = stream.handler.terminal;

    // The glyph grid. `snapshot_window` hands back owned rows, so nothing below
    // has to outlive a borrow of the page list.
    let win = t.snapshot_window(0);
    let mut cells = vec![OracleCell::default(); cols as usize * rows as usize];
    for (row, srow) in win.window.iter().enumerate().take(rows as usize) {
        for (col, scell) in srow.cells.iter().enumerate().take(cols as usize) {
            cells[row * cols as usize + col] = OracleCell {
                ch: scell.ch,
                fg: color(scell.style.fg),
                bg: color(scell.style.bg),
                image_id: None,
                source_y: None,
                inverse: scell.style.inverse,
            };
        }
    }
    let colors = Colors::from_window(&win);

    let grid = Grid { cols, rows, cell_w, cell_h, window_top: win.window_top };
    let placements = resolve_rects(&t, grid, &mut cells);
    let (draws, images) = capture_draws(&t, grid);
    Resolved { cols, rows, cell_w, cell_h, cells, placements, draws, images, colors }
}

/// The same placements again, kept as pixels instead of cells (SQ-0775).
///
/// A second resolution rather than a second return value out of [`resolve_rects`]
/// on purpose: that function's whole subject is the join between the placeholder
/// walk and the resolved list, and it is hard enough to follow without a second
/// bookkeeping job threaded through it. Re-resolving costs one more pass over a
/// placement list that is tens of entries long; the byte feed above it dominates
/// by orders of magnitude.
fn capture_draws(t: &Terminal, grid: Grid) -> (Vec<Draw>, BTreeMap<u32, RasterImage>) {
    let geo = TerminalGeometry::new(t.cols, t.rows, t.width_px, t.height_px);
    let screen = t.screen();
    let (placements, images, _live) = resolve_window(&screen.kitty_images, &screen.pages, &geo, 0);

    let draws = placements
        .iter()
        .map(|p| Draw {
            image_id: p.image_id,
            z: p.z,
            dest_x: i64::from(p.grid_col) * i64::from(grid.cell_w) + i64::from(p.cell_offset_x),
            dest_y: i64::from(p.grid_row) * i64::from(grid.cell_h) + i64::from(p.cell_offset_y),
            dest_w: p.dest_width,
            dest_h: p.dest_height,
            src_x: p.source_x,
            src_y: p.source_y,
            src_w: p.source_width,
            src_h: p.source_height,
        })
        .collect();

    let images = images
        .into_iter()
        .map(|i| (i.id, RasterImage { width: i.width, height: i.height, rgba: i.rgba.to_vec() }))
        .collect();
    (draws, images)
}

/// Work out which cells an image actually lands on, and aggregate the per-row
/// entries into one rect per image.
///
/// The join is the interesting part. [`resolve_placements`] is the authority on
/// WHICH runs render — it does the image lookup, the aspect-fit maths and the
/// off-window culling — but it reports each run's position in cells and its size
/// in PIXELS, and pixels round the wrong way (see the module docs). The
/// placeholder walk is the authority on how many CELLS each run spans, but will
/// happily hand back a run naming an image the terminal never received. So a
/// walked run is kept only when a resolved placement sits at the same
/// `(image id, col, row)`; whatever resolved placements are left over are
/// anchored to a pin rather than to placeholder cells, and only those fall back
/// to pixel arithmetic for their extent.
fn resolve_rects(t: &Terminal, grid: Grid, cells: &mut [OracleCell]) -> Vec<ImageRect> {
    let geo = TerminalGeometry::new(t.cols, t.rows, t.width_px, t.height_px);
    let screen = t.screen();
    let resolved = resolve_placements(&screen.kitty_images, &screen.pages, &geo, 0);

    // The authored z and the virtual/pin origin, per image id. `placements` is a
    // HashMap, so fold it into ordered state rather than reading it in whatever
    // order it happens to hash into.
    let mut authored: BTreeMap<u32, i32> = BTreeMap::new();
    for (key, p) in &screen.kitty_images.placements {
        let e = authored.entry(key.image_id).or_insert(p.z);
        *e = (*e).min(p.z);
    }

    // Which resolved entries sit at each (image, col, row) — a list of indices,
    // not a flag: nothing in the protocol forbids two runs landing on one cell.
    let mut pending: BTreeMap<(u32, u32, i32), Vec<usize>> = BTreeMap::new();
    for (i, p) in resolved.iter().enumerate() {
        pending.entry((p.image_id, p.grid_col, p.grid_row)).or_default().push(i);
    }

    let mut acc: BTreeMap<(u32, Origin), Acc> = BTreeMap::new();

    // ── virtual: the placeholder walk, filtered by what actually resolves ─────
    // The walk starts at the top of the whole page list rather than at the
    // window's top pin: reaching the window's top needs `Pin::up`, which is the
    // same unsafe walk with more arithmetic to get wrong. Rows above the window
    // fall out of the row-range filter below instead.
    let top_left = screen.pages.get_top_left(Tag::Screen);
    // SAFETY: `top_left` addresses a live page in `screen.pages`, which `t` owns
    // and which outlives this iterator; no limit pin is passed.
    let mut it = unsafe { unicode::placement_iterator(top_left, None) };
    // SAFETY (each `next`): the pins the iterator walks belong to the same live
    // page list, untouched for the duration of this loop.
    while let Some(run) = unsafe { it.next() } {
        let Some(point) = screen.pages.point_from_pin(Tag::Screen, run.pin) else { continue };
        let row = i64::from(point.coord.y) - grid.window_top as i64;
        if row < 0 || row >= i64::from(grid.rows) {
            continue;
        }
        let col = u32::from(point.coord.x);
        // No resolved placement here means the terminal walked these cells and
        // declined to draw: an unknown image id, or a destination that rounds
        // away. Those cells are placeholders on screen and pixels nowhere.
        let Some(idx) = pending.get_mut(&(run.image_id, col, row as i32)).and_then(Vec::pop) else {
            continue;
        };
        let source_y = resolved[idx].source_y;
        let row = row as u16;
        let z = authored.get(&run.image_id).copied().unwrap_or(0);
        let e = acc.entry((run.image_id, Origin::Virtual)).or_insert_with(|| Acc::new(z));
        for i in 0..run.width {
            let Ok(c) = u16::try_from(col + i) else { break };
            if c >= grid.cols {
                break;
            }
            let cell = &mut cells[row as usize * grid.cols as usize + c as usize];
            cell.image_id = Some(run.image_id);
            cell.source_y = Some(source_y);
            e.add(row, c);
        }
    }

    // ── pin: whatever resolved without a placeholder run to explain it ───────
    for (&(image_id, col, grid_row), idxs) in &pending {
        for &i in idxs {
            let p = &resolved[i];
            // A pin placement has no placeholder cells to count, so its extent
            // comes from the cell grid the transmit declared, falling back to the
            // destination pixels when it declared none. `div_ceil` can
            // over-report by a cell against an aspect-fit shrink; there is
            // nothing better to read, and the fallback is documented as such.
            let stored = screen
                .kitty_images
                .placements
                .iter()
                .filter(|(k, v)| k.image_id == image_id && matches!(v.location, Location::Pin(_)))
                .map(|(_, v)| (v.columns, v.rows))
                .next()
                .unwrap_or((0, 0));
            let w = if stored.0 > 0 { stored.0 } else { p.dest_width.div_ceil(grid.cell_w.max(1)) };
            let h = if stored.1 > 0 { stored.1 } else { p.dest_height.div_ceil(grid.cell_h.max(1)) };
            let z = authored.get(&image_id).copied().unwrap_or(p.z);
            let e = acc.entry((image_id, Origin::Pin)).or_insert_with(|| Acc::new(z));
            for dy in 0..h {
                let y = i64::from(grid_row) + i64::from(dy);
                if y < 0 || y >= i64::from(grid.rows) {
                    continue;
                }
                for dx in 0..w {
                    let Ok(x) = u16::try_from(col + dx) else { break };
                    if x >= grid.cols {
                        break;
                    }
                    let y = y as u16;
                    cells[y as usize * grid.cols as usize + x as usize].image_id = Some(image_id);
                    e.add(y, x);
                }
            }
        }
    }

    acc.into_iter()
        .map(|((image_id, origin), a)| ImageRect {
            image_id,
            top: a.top,
            bottom: a.bottom,
            left: a.left,
            right: a.right,
            cells: a.cells,
            z: a.z,
            origin,
        })
        .collect()
}

/// A rect being grown one covered cell at a time.
struct Acc {
    top: u16,
    bottom: u16,
    left: u16,
    right: u16,
    cells: usize,
    z: i32,
}

impl Acc {
    fn new(z: i32) -> Acc {
        Acc { top: u16::MAX, bottom: 0, left: u16::MAX, right: 0, cells: 0, z }
    }

    fn add(&mut self, row: u16, col: u16) {
        self.top = self.top.min(row);
        self.bottom = self.bottom.max(row);
        self.left = self.left.min(col);
        self.right = self.right.max(col);
        self.cells += 1;
    }
}

/// Every place our decoder and a real terminal read the same bytes differently.
/// An empty vec is full agreement.
///
/// Two axes, because they are the two the harness exists to tell apart:
/// BACKGROUND (was a colour painted into these cells?) and IMAGE COVER (would a
/// renderer put image pixels here?). Differences are collapsed into per-row
/// column runs — a whole-screen divergence has to stay readable, and a run says
/// as much as ten thousand cells would.
///
/// Image ids are compared on their LOW 24 BITS only: ours cannot see the high
/// byte, which lives in a diacritic our model does not decode (module docs).
pub fn disagreements(term: &decode::Term, res: &Resolved) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if (term.cols, term.rows) != (res.cols, res.rows) {
        out.push(format!(
            "grid size: ours {}x{}, oracle {}x{} — comparing the overlap only",
            term.cols, term.rows, res.cols, res.rows
        ));
    }
    let cols = term.cols.min(res.cols);
    let rows = term.rows.min(res.rows);

    for row in 0..rows {
        let bg = |col: u16| -> (String, String) {
            let c = term.cell(row, col);
            let ours = if c.written { c.style.bg } else { Color::Default };
            (ours.label(), res.cell(row, col).bg.label())
        };
        runs(cols, &bg, |c0, c1, ours, theirs| {
            out.push(format!(
                "background row {row} cols {c0}..{c1}: ours {ours}, oracle {theirs}"
            ));
        });

        let img = |col: u16| -> (String, String) {
            let ours = term.cell(row, col).image_id();
            let theirs = res.cell(row, col).image_id.map(|id| id & ID_MASK);
            (id_label(ours), id_label(theirs))
        };
        runs(cols, &img, |c0, c1, ours, theirs| {
            out.push(format!(
                "image cover row {row} cols {c0}..{c1}: ours {ours}, oracle {theirs}"
            ));
        });
    }
    out
}

fn id_label(id: Option<u32>) -> String {
    match id {
        Some(id) => format!("image {id:#08x}"),
        None => "none".to_string(),
    }
}

/// Walk `0..cols`, calling `emit` once per maximal run of columns where `read`
/// reports two different readings that stay the same across the run.
fn runs(
    cols: u16,
    read: &dyn Fn(u16) -> (String, String),
    mut emit: impl FnMut(u16, u16, &str, &str),
) {
    let mut open: Option<(u16, u16, String, String)> = None;
    for col in 0..cols {
        let (ours, theirs) = read(col);
        let differ = ours != theirs;
        match &mut open {
            Some(run) if differ && run.2 == ours && run.3 == theirs => run.1 = col,
            _ => {
                if let Some((c0, c1, a, b)) = open.take() {
                    emit(c0, c1, &a, &b);
                }
                if differ {
                    open = Some((col, col, ours, theirs));
                }
            }
        }
    }
    if let Some((c0, c1, a, b)) = open.take() {
        emit(c0, c1, &a, &b);
    }
}

fn color(c: SnapshotColor) -> Color {
    match c {
        SnapshotColor::Default => Color::Default,
        SnapshotColor::Palette(i) => Color::Idx(i),
        SnapshotColor::Rgb { r, g, b } => Color::Rgb(r, g, b),
    }
}
