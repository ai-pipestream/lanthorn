//! The **Atari 8-bit** line-art picture format of the four US S.A.G.A.
//! releases whose side B is a token stream rather than family-C bitmaps —
//! *Adventureland*, *Pirate Adventure*, *Mission Impossible* and *Strange
//! Odyssey* (SQ-1525; the per-room index that reaches these records is
//! SQ-1524's).
//!
//! # This is not the Apple II grammar
//!
//! [`crate::saga_atari`]'s `decode_line_art_opening` reads these bytes under
//! [`crate::apple_pictures`]' item-26 grammar, and SQ-1524 measured why that
//! cannot be right: every record here is a stream of **fixed three-byte
//! tokens** whose drawing coordinates never leave **160 x 96**, the Atari's
//! four-colour `GRAPHICS 7` canvas, where the Apple's tokens are one, two or
//! three bytes on 280 x 192. This module reads the format the machine
//! actually draws, and everything below was settled the way
//! `apple_pictures.rs` settled `M3`: by reading the release's **own renderer**
//! off its own boot side as a specimen. No interpreter source was consulted
//! (`docs/internals/clean-room.md`).
//!
//! # The screen, as the renderer builds it
//!
//! The picture is **two** 160 x 96 bitmaps of two-bit pixels, at `$9000` and
//! `$A000`, and a display list of 96 ANTIC mode-E lines whose load-memory-scan
//! address the vertical-blank hook flips between the two **every frame**,
//! loading a different five-byte colour table into the `COLOR0`-`COLOR4`
//! shadows for each. A canvas pixel is therefore a **pair** — its value in
//! the `$A000` bitmap and its value in the `$9000` one — and its colour is
//! the eye's average of two GTIA colours shown on alternate frames. Sixteen
//! pairs are reachable and all sixteen are named by one table in the
//! renderer, [`COLOUR_PAIRS`]; [`PALETTE_A`] and [`PALETTE_B`] are the two
//! colour tables, and [`pair_rgb`] is the average.
//!
//! # Tokens
//!
//! Three bytes each, `[command, p, q]`; the command's **top three bits** are
//! the class and its **low five** an operand. A byte whose top three bits are
//! all clear is the **end of the picture** and stands alone — the loader
//! copies tokens three bytes at a time until it stores one.
//!
//! | class | token | effect |
//! |---|---|---|
//! | `0x00` | `00` | end |
//! | `0x20` | `20 x y` | **clear**: both bitmaps to fill colour *p*'s pair (`x y` unused) |
//! | `0x20` | `21 x y` | **recolour**: re-run the record from its start, redrawing every line whose colour is *p* in colour *q* and nothing else, until this token is reached again (`x y` unused) |
//! | `0x40` | `4n 00 00` | **pause** — an animation delay, no pixels |
//! | `0x60` | `6c p q` | line colour *c*, fill colours *p* and *q*, all indices into [`COLOUR_PAIRS`] |
//! | `0x80` | `80 x y` | **move** the pen to (*x*, *y*) |
//! | `0xA0` | `A? x y` | **line** from the pen to (*x*, *y*) in the line colour; the pen follows (operand ignored) |
//! | `0xC0` | `Cs x y` | **bounded fill** from (*x*, *y*): every pixel that is not the line colour, in pattern *s* |
//! | `0xE0` | `Es x y` | **flood fill** from (*x*, *y*): every pixel of the seed's colour, in pattern *s* |
//!
//! At the start of a record the line colour is [`DEFAULT_LINE_COLOUR`] and
//! the recolour flag is off; **the fill colours, the pen and the bitmaps are
//! not reset**, which is how an object picture draws over its room. A room
//! record opens with `6c p q | 20 x y` — set the colours, clear — and an
//! object record with `80 x y`, exactly as measured.
//!
//! **Fill patterns** *s* = 0..4 name two four-pixel byte patterns built from
//! the fill colours, one for even rows and one for odd (the fill alternates
//! them row by row, and canonicalises which is which by the parity of its
//! top row): 0 solid *p*; 1 a *p*/*q* checkerboard; 2 alternate rows of solid
//! *p* and solid *q*; 3 vertical *q*/*p* stripes; 4 alternate rows of solid
//! *p* and *q*/*p* stripes. The corpus uses exactly these five.
//!
//! **The fill is not a flood fill.** It walks **up** the seed's column to the
//! topmost matching pixel, then sweeps **down** one row at a time: it walks
//! left to the row's span edge, fills right to the span's end, and on the
//! next row starts at the span's left edge — if that pixel matches it walks
//! left again, otherwise it scans right, no further than the previous span's
//! end, for the first match and fills from there without walking left. It
//! stops at the first row with no match, or at row 95. The artwork was
//! authored against exactly that reach, and a true flood fill would spill
//! where the machine does not, so [`LineArtCanvas`] reproduces it step for
//! step.
//!
//! **The line** is a Bresenham walk that always steps along its major axis
//! from the endpoint with the smaller major-axis coordinate (ties broken
//! towards the smaller minor coordinate), tests `error >= 0` after each
//! plot, and plots both endpoints. That precision matters only because a line
//! is usually the boundary of a fill.
//!
//! # What is still approximate
//!
//! - **Colour.** [`pair_rgb`] averages two [`crate::saga_pictures::atari_colour`]
//!   answers; the real machine flickered them at the frame rate.
//! - **Animation.** The pause token is a countdown loop, not a frame count,
//!   so the durations are not carried; [`LineArtCanvas::draw_frames`] hands
//!   back a snapshot at each pause so a host can show the frame the machine
//!   rested on. The darkness card ends with a clear to black after its last
//!   pause, so its final frame is genuinely black on the machine and its
//!   lettering lives in the snapshots.
//! - One comparison in the renderer's fill reads zero-page `$A0` where it
//!   plainly means the immediate `$A0`; the corpus never reaches it, and this
//!   decoder treats it as the immediate.

use crate::saga_pictures::{atari_colour, Painted, PaintedBox, Rgb};

/// The canvas width in pixels: `GRAPHICS 7`'s 160.
pub const CANVAS_WIDTH: usize = 160;

/// The canvas height in pixels: the 96 mode-E lines of the renderer's display
/// list.
pub const CANVAS_HEIGHT: usize = 96;

/// The sixteen colours a token can name, as `(a, b)`: `a` is the two-bit
/// pixel value written to the `$A000` bitmap and `b` the one written to the
/// `$9000` bitmap. Read off the renderer's table; a canvas pixel's value is
/// `a * 4 + b`, which [`LineArtPicture::pixels`] stores and
/// [`LineArtPicture::palette`] resolves.
pub const COLOUR_PAIRS: [(u8, u8); 16] = [
    (3, 3),
    (3, 2),
    (3, 1),
    (3, 0),
    (2, 2),
    (2, 1),
    (2, 0),
    (1, 1),
    (1, 0),
    (0, 0),
    (2, 3),
    (1, 3),
    (0, 3),
    (1, 2),
    (0, 2),
    (0, 1),
];

/// The GTIA colour each pixel value of the `$A000` bitmap shows on its
/// frames, by pixel value 0-3: the renderer's five-byte table for that
/// bitmap read through mode E's register order (value 0 is the background
/// register, values 1-3 are `COLOR0`-`COLOR2`).
pub const PALETTE_A: [u8; 4] = [0x00, 0x32, 0x84, 0xD4];

/// [`PALETTE_A`]'s twin for the `$9000` bitmap.
pub const PALETTE_B: [u8; 4] = [0x00, 0x42, 0xB4, 0xE4];

/// The line colour every record starts with: index 10 of [`COLOUR_PAIRS`].
pub const DEFAULT_LINE_COLOUR: u8 = 10;

/// The colour a pixel pair presents: the average of the two GTIA colours it
/// shows on alternate frames.
#[must_use]
pub fn pair_rgb(a: u8, b: u8) -> Rgb {
    let (ra, ga, ba) = atari_colour(PALETTE_A[usize::from(a & 3)]);
    let (rb, gb, bb) = atari_colour(PALETTE_B[usize::from(b & 3)]);
    let mid = |x: u8, y: u8| ((u16::from(x) + u16::from(y)) / 2) as u8;
    (mid(ra, rb), mid(ga, gb), mid(ba, bb))
}

/// All sixteen pixel values' colours, indexed by `a * 4 + b`.
#[must_use]
pub fn palette() -> [Rgb; 16] {
    let mut out = [(0, 0, 0); 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = pair_rgb((i / 4) as u8, (i % 4) as u8);
    }
    out
}

/// Why a token stream is not a line-art picture ("name it and refuse it").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineArtError {
    /// The stream ran out before an end token.
    Truncated {
        /// Stream length offered.
        len: usize,
    },
    /// A drawing token names a point off the 160 x 96 canvas.
    OffCanvas {
        /// Byte offset of the token.
        at: usize,
        /// The point.
        x: u8,
        /// The point.
        y: u8,
    },
    /// A colour token names an index past [`COLOUR_PAIRS`].
    BadColour {
        /// Byte offset of the token.
        at: usize,
        /// The index.
        index: u8,
    },
    /// A fill token names a pattern the renderer has no table for.
    BadFillStyle {
        /// Byte offset of the token.
        at: usize,
        /// The style.
        style: u8,
    },
}

impl std::fmt::Display for LineArtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { len } => write!(f, "line-art stream of {len} bytes has no end token"),
            Self::OffCanvas { at, x, y } => {
                write!(
                    f,
                    "line-art token at 0x{at:X} draws at ({x}, {y}), off the 160x96 canvas"
                )
            }
            Self::BadColour { at, index } => {
                write!(
                    f,
                    "line-art token at 0x{at:X} names colour {index}, past the sixteen"
                )
            }
            Self::BadFillStyle { at, style } => {
                write!(
                    f,
                    "line-art token at 0x{at:X} names fill pattern {style}, past the five"
                )
            }
        }
    }
}

impl std::error::Error for LineArtError {}

/// One decoded picture: [`CANVAS_WIDTH`] x [`CANVAS_HEIGHT`] pixel values,
/// each `a * 4 + b` for the pair a pixel holds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LineArtPicture {
    /// Always [`CANVAS_WIDTH`].
    pub(crate) width: usize,
    /// Always [`CANVAS_HEIGHT`].
    pub(crate) height: usize,
    /// `width * height` pixel values, each 0-15, row-major from the top-left.
    pub(crate) pixels: Vec<u8>,
    /// The canvas rectangle the record that produced this drew on — the same
    /// fact [`crate::saga_pictures::Picture::painted`] carries, needed for the
    /// same reason: an object picture is composited over the rectangle it
    /// drew and nowhere else. A clear counts as drawing the whole canvas.
    pub(crate) painted: Option<Painted>,
}

impl LineArtPicture {
    /// Always [`CANVAS_WIDTH`].
    pub fn width(&self) -> usize {
        self.width
    }

    /// Always [`CANVAS_HEIGHT`].
    pub fn height(&self) -> usize {
        self.height
    }

    /// `width() * height()` pixel values, each 0-15, row-major from the
    /// top-left; index [`Self::palette`] with one.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The colour of each pixel value.
    pub fn palette(&self) -> [Rgb; 16] {
        palette()
    }

    /// The canvas rectangle the producing record drew on, inclusive, or
    /// `None` if it drew nothing.
    pub fn painted(&self) -> Option<Painted> {
        self.painted
    }

    /// The RGB of the pixel at `(x, y)`, or `None` off the canvas.
    #[must_use]
    pub fn rgb(&self, x: usize, y: usize) -> Option<Rgb> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let v = *self.pixels.get(y * self.width + x)?;
        Some(pair_rgb(v / 4, v % 4))
    }
}

/// The renderer's screen and registers: the two bitmaps, the fill colours
/// and the pen, which persist from one record to the next so that object
/// pictures draw over their room. Make one, [`draw`](Self::draw) the room
/// record, then each object record, then take the [`picture`](Self::picture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineArtCanvas {
    /// The `$A000` bitmap, one pixel value per byte.
    a: Vec<u8>,
    /// The `$9000` bitmap.
    b: Vec<u8>,
    /// The two fill colours, indices into [`COLOUR_PAIRS`].
    fill: (u8, u8),
    /// The pen.
    pen: (u8, u8),
}

impl Default for LineArtCanvas {
    fn default() -> Self {
        Self::new()
    }
}

/// The four pattern bytes a fill uses — `$A000` and `$9000` bitmap bytes for
/// one row parity, then the same for the other — as the renderer lays them
/// out.
struct Fill {
    even_a: u8,
    even_b: u8,
    odd_a: u8,
    odd_b: u8,
}

/// Four two-bit pixel values into one bitmap byte, leftmost first.
const fn byte4(p0: u8, p1: u8, p2: u8, p3: u8) -> u8 {
    (p0 << 6) | (p1 << 4) | (p2 << 2) | p3
}

/// One pixel value in all four positions.
const fn rep4(v: u8) -> u8 {
    byte4(v, v, v, v)
}

/// Pixel `x`'s two bits of a pattern byte.
const fn pattern_pixel(pattern: u8, x: usize) -> u8 {
    (pattern >> (6 - 2 * (x & 3))) & 3
}

impl LineArtCanvas {
    /// A black screen with the fill colours at index 0 and the pen at the
    /// origin — nothing a room record does not overwrite before it draws.
    #[must_use]
    pub fn new() -> Self {
        Self {
            a: vec![0; CANVAS_WIDTH * CANVAS_HEIGHT],
            b: vec![0; CANVAS_WIDTH * CANVAS_HEIGHT],
            fill: (0, 0),
            pen: (0, 0),
        }
    }

    /// What the screen shows now, with `painted` covering the whole canvas.
    #[must_use]
    pub fn picture(&self) -> LineArtPicture {
        self.picture_with(Some(Painted {
            left: 0,
            top: 0,
            right: CANVAS_WIDTH - 1,
            bottom: CANVAS_HEIGHT - 1,
        }))
    }

    fn picture_with(&self, painted: Option<Painted>) -> LineArtPicture {
        let pixels = self
            .a
            .iter()
            .zip(&self.b)
            .map(|(&a, &b)| a * 4 + b)
            .collect();
        LineArtPicture {
            width: CANVAS_WIDTH,
            height: CANVAS_HEIGHT,
            pixels,
            painted,
        }
    }

    /// Play one record over the canvas, to its end token.
    ///
    /// Returns the picture as it stands afterwards, whose `painted` is the
    /// rectangle **this record** drew on. `stream` may run past the record;
    /// reading stops at the end token.
    ///
    /// # Errors
    ///
    /// [`LineArtError`] — the stream is refused, and the canvas is left as
    /// far as it got.
    pub fn draw(&mut self, stream: &[u8]) -> Result<LineArtPicture, LineArtError> {
        let mut frames = Vec::new();
        self.play(stream, false, &mut frames)?;
        Ok(frames.pop().expect("play always pushes the final frame"))
    }

    /// [`draw`](Self::draw), also keeping a snapshot at every pause token —
    /// the frames the machine rested on — with the final picture last. A
    /// record with no pauses yields one frame.
    ///
    /// # Errors
    ///
    /// As [`draw`](Self::draw).
    pub fn draw_frames(&mut self, stream: &[u8]) -> Result<Vec<LineArtPicture>, LineArtError> {
        let mut frames = Vec::new();
        self.play(stream, true, &mut frames)?;
        Ok(frames)
    }

    fn play(
        &mut self,
        stream: &[u8],
        keep_pauses: bool,
        frames: &mut Vec<LineArtPicture>,
    ) -> Result<(), LineArtError> {
        let mut painted = PaintedBox::default();
        let mut line_colour = DEFAULT_LINE_COLOUR;
        // The recolour pass: `Some((from_a, from_b, to_a, to_b))` while
        // re-running the record.
        let mut recolour: Option<(u8, u8, u8, u8)> = None;
        let mut at = 0;
        loop {
            let &cmd = stream
                .get(at)
                .ok_or(LineArtError::Truncated { len: stream.len() })?;
            if cmd & 0xE0 == 0 {
                break;
            }
            let (p, q) = match (stream.get(at + 1), stream.get(at + 2)) {
                (Some(&p), Some(&q)) => (p, q),
                _ => return Err(LineArtError::Truncated { len: stream.len() }),
            };
            let op = cmd & 0x1F;
            match cmd >> 5 {
                // move
                4 => {
                    on_canvas(at, p, q)?;
                    self.pen = (p, q);
                }
                // line
                5 => {
                    on_canvas(at, p, q)?;
                    let (ca, cb) = COLOUR_PAIRS[usize::from(line_colour)];
                    let mut pat = (rep4(ca), rep4(cb));
                    let draw = match recolour {
                        None => true,
                        Some((fa, fb, ta, tb)) if pat == (fa, fb) => {
                            pat = (ta, tb);
                            true
                        }
                        Some(_) => false,
                    };
                    if draw {
                        self.line(self.pen, (p, q), pat, &mut painted);
                    }
                    self.pen = (p, q);
                }
                // colours
                3 => {
                    for index in [op, p, q] {
                        if index > 15 {
                            return Err(LineArtError::BadColour { at, index });
                        }
                    }
                    line_colour = op;
                    self.fill = (p, q);
                }
                // fills
                6 | 7 => {
                    on_canvas(at, p, q)?;
                    if recolour.is_none() {
                        let fill = self
                            .patterns(op)
                            .ok_or(LineArtError::BadFillStyle { at, style: op })?;
                        let boundary = if cmd >> 5 == 6 {
                            let (ca, cb) = COLOUR_PAIRS[usize::from(line_colour)];
                            Some((ca << 2) | cb)
                        } else {
                            None
                        };
                        self.fill((p, q), boundary, fill, &mut painted);
                    }
                }
                // pause
                2 => {
                    if keep_pauses && recolour.is_none() {
                        frames.push(self.picture_with(painted.clone().finish()));
                    }
                }
                // clear, or the recolour marker
                1 => {
                    if op == 0 {
                        if recolour.is_none() {
                            let (ca, cb) = COLOUR_PAIRS[usize::from(self.fill.0)];
                            self.a.fill(ca);
                            self.b.fill(cb);
                            painted.mark(0, 0);
                            painted.mark(CANVAS_WIDTH - 1, CANVAS_HEIGHT - 1);
                        }
                    } else if op == 1 {
                        match recolour {
                            Some(_) => recolour = None,
                            None => {
                                let (fa, fb) = COLOUR_PAIRS[usize::from(self.fill.0)];
                                let (ta, tb) = COLOUR_PAIRS[usize::from(self.fill.1)];
                                recolour = Some((rep4(fa), rep4(fb), rep4(ta), rep4(tb)));
                                // The renderer re-enters its loop at the record's
                                // first token with the line colour as it stands.
                                at = 0;
                                continue;
                            }
                        }
                    }
                }
                _ => {}
            }
            at += 3;
        }
        frames.push(self.picture_with(painted.finish()));
        Ok(())
    }

    /// The two row patterns for fill style `style` from the fill colours,
    /// or `None` for a style the renderer has no table for.
    fn patterns(&self, style: u8) -> Option<Fill> {
        let (c1a, c1b) = COLOUR_PAIRS[usize::from(self.fill.0)];
        let (c2a, c2b) = COLOUR_PAIRS[usize::from(self.fill.1)];
        let stripes_a = byte4(c2a, c1a, c2a, c1a);
        let stripes_b = byte4(c2b, c1b, c2b, c1b);
        Some(match style {
            0 => Fill {
                even_a: rep4(c1a),
                even_b: rep4(c1b),
                odd_a: rep4(c1a),
                odd_b: rep4(c1b),
            },
            1 => Fill {
                even_a: byte4(c1a, c2a, c1a, c2a),
                even_b: byte4(c1b, c2b, c1b, c2b),
                odd_a: stripes_a,
                odd_b: stripes_b,
            },
            2 => Fill {
                even_a: rep4(c1a),
                even_b: rep4(c1b),
                odd_a: rep4(c2a),
                odd_b: rep4(c2b),
            },
            3 => Fill {
                even_a: stripes_a,
                even_b: stripes_b,
                odd_a: stripes_a,
                odd_b: stripes_b,
            },
            4 => Fill {
                even_a: rep4(c1a),
                even_b: rep4(c1b),
                odd_a: stripes_a,
                odd_b: stripes_b,
            },
            _ => return None,
        })
    }

    /// The pair code at `(x, y)`: `a << 2 | b`, as the renderer compares.
    fn code(&self, x: usize, y: usize) -> u8 {
        let i = y * CANVAS_WIDTH + x;
        (self.a[i] << 2) | self.b[i]
    }

    /// Plot pixel `x` of the pattern pair at `(x, y)`.
    fn plot(&mut self, x: usize, y: usize, (pat_a, pat_b): (u8, u8), painted: &mut PaintedBox) {
        let i = y * CANVAS_WIDTH + x;
        self.a[i] = pattern_pixel(pat_a, x);
        self.b[i] = pattern_pixel(pat_b, x);
        painted.mark(x, y);
    }

    /// The bitmap byte holding `(x, y)` — the four pixels from `x & !3`.
    fn byte_at(buf: &[u8], x: usize, y: usize) -> u8 {
        let i = y * CANVAS_WIDTH + (x & !3);
        byte4(buf[i], buf[i + 1], buf[i + 2], buf[i + 3])
    }

    fn set_byte(buf: &mut [u8], x: usize, y: usize, val: u8) {
        let i = y * CANVAS_WIDTH + (x & !3);
        for k in 0..4 {
            buf[i + k] = pattern_pixel(val, k);
        }
    }

    /// The renderer's line, from the pen `from` to `to`, both inclusive.
    fn line(&mut self, from: (u8, u8), to: (u8, u8), pat: (u8, u8), painted: &mut PaintedBox) {
        // The renderer names the new point (x0, y0) and the pen (x1, y1),
        // and its eight-way case split reduces to this: walk the major axis
        // upward from the endpoint with the smaller major coordinate, the
        // minor axis stepping by `sign`, with the pen winning a tie.
        let (x0, y0) = (usize::from(to.0), usize::from(to.1));
        let (x1, y1) = (usize::from(from.0), usize::from(from.1));
        if (x0, y0) == (x1, y1) {
            self.plot(x0, y0, pat, painted);
            return;
        }
        let dx = x0.abs_diff(x1);
        let dy = y0.abs_diff(y1);
        let x_major = dy < dx;
        // (start, sign) per the renderer's case table.
        let (mut x, mut y, sign): (usize, usize, isize) = if x_major {
            if y0 == y1 {
                if x0 >= x1 {
                    (x1, y1, 1)
                } else {
                    (x0, y0, 1)
                }
            } else if y1 < y0 {
                (x1, y1, if x0 >= x1 { 1 } else { -1 })
            } else {
                (x0, y0, if x0 >= x1 { -1 } else { 1 })
            }
        } else if x0 == x1 {
            if y0 >= y1 {
                (x1, y1, 1)
            } else {
                (x1, y1, -1)
            }
        } else if x1 < x0 {
            (x1, y1, if y0 >= y1 { 1 } else { -1 })
        } else {
            (x0, y0, if y1 >= y0 { 1 } else { -1 })
        };
        let step = |v: usize| {
            v.checked_add_signed(sign)
                .expect("the renderer's line stays on the canvas")
        };
        if x_major {
            let (dx, dy) = (dx as isize, dy as isize);
            let mut err = 2 * dy - dx;
            for _ in 0..dx {
                self.plot(x, y, pat, painted);
                if err >= 0 {
                    y += 1;
                    err += 2 * dy - 2 * dx;
                } else {
                    err += 2 * dy;
                }
                x = step(x);
            }
        } else {
            let (dx, dy) = (dx as isize, dy as isize);
            let mut err = 2 * dx - dy;
            for _ in 0..dy {
                self.plot(x, y, pat, painted);
                if err >= 0 {
                    x += 1;
                    err += 2 * dx - 2 * dy;
                } else {
                    err += 2 * dx;
                }
                y = step(y);
            }
        }
        self.plot(x, y, pat, painted);
    }

    /// The renderer's fill from `seed`: bounded by `boundary` (every pixel
    /// that is not that code) when given, else over the seed's own colour.
    fn fill(&mut self, seed: (u8, u8), boundary: Option<u8>, fill: Fill, painted: &mut PaintedBox) {
        let (sx, sy) = (usize::from(seed.0), usize::from(seed.1));
        let seed_code = self.code(sx, sy);
        let seed_byte_a = rep4(seed_code >> 2);
        let seed_byte_b = rep4(seed_code & 3);
        let matches = |canvas: &Self, x: usize, y: usize| match boundary {
            Some(b) => canvas.code(x, y) != b,
            None => canvas.code(x, y) == seed_code,
        };
        // Up the seed's column.
        let mut y = sy;
        while y > 0 && matches(self, sx, y - 1) {
            y -= 1;
        }
        // The row patterns: `cur` for this row, `alt` for the next, swapped
        // per row after the renderer's parity canonicalisation of the top row
        // (a bit-7 test on a wrapping byte difference, kept as is).
        let (mut cur, mut alt) = ((fill.even_a, fill.even_b), (fill.odd_a, fill.odd_b));
        let diff = |lhs: (u8, u8), rhs: (u8, u8)| {
            if lhs.1 != rhs.1 {
                lhs.1.wrapping_sub(rhs.1)
            } else {
                lhs.0.wrapping_sub(rhs.0)
            }
        };
        let d = if y & 1 == 1 {
            diff(cur, alt)
        } else {
            diff(alt, cur)
        };
        if d & 0x80 != 0 {
            std::mem::swap(&mut cur, &mut alt);
        }
        let mut x = sx;
        let mut walk_left = true;
        loop {
            if walk_left {
                while x > 0 && matches(self, x - 1, y) {
                    x -= 1;
                }
            }
            let span_start = x;
            loop {
                if x & 3 == 0
                    && boundary.is_none()
                    && Self::byte_at(&self.b, x, y) == seed_byte_b
                    && Self::byte_at(&self.a, x, y) == seed_byte_a
                {
                    Self::set_byte(&mut self.b, x, y, cur.1);
                    Self::set_byte(&mut self.a, x, y, cur.0);
                    painted.mark(x, y);
                    painted.mark(x + 3, y);
                    x += 4;
                } else {
                    if !matches(self, x, y) {
                        break;
                    }
                    self.plot(x, y, cur, painted);
                    x += 1;
                }
                if x == CANVAS_WIDTH {
                    break;
                }
            }
            let span_end = x;
            std::mem::swap(&mut cur, &mut alt);
            if y == CANVAS_HEIGHT - 1 {
                return;
            }
            y += 1;
            x = span_start;
            if matches(self, x, y) {
                walk_left = true;
                continue;
            }
            loop {
                x += 1;
                if x == span_end {
                    return;
                }
                if matches(self, x, y) {
                    break;
                }
            }
            walk_left = false;
        }
    }
}

/// Refuse a drawing token off the canvas.
fn on_canvas(at: usize, x: u8, y: u8) -> Result<(), LineArtError> {
    if usize::from(x) < CANVAS_WIDTH && usize::from(y) < CANVAS_HEIGHT {
        Ok(())
    } else {
        Err(LineArtError::OffCanvas { at, x, y })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream from `(class | op, p, q)` triples plus the end byte.
    fn stream(tokens: &[(u8, u8, u8)]) -> Vec<u8> {
        let mut out: Vec<u8> = tokens.iter().flat_map(|&(c, p, q)| [c, p, q]).collect();
        out.push(0);
        out
    }

    #[test]
    fn every_pair_in_the_table_is_distinct_and_the_palette_covers_them() {
        let mut seen = std::collections::BTreeSet::new();
        for (a, b) in COLOUR_PAIRS {
            assert!(seen.insert(a * 4 + b));
        }
        assert_eq!(seen.len(), 16);
        assert_eq!(pair_rgb(0, 0), (0, 0, 0), "pair 9 is black on both frames");
        assert_eq!(
            palette()[usize::from(COLOUR_PAIRS[9].0 * 4 + COLOUR_PAIRS[9].1)],
            (0, 0, 0)
        );
    }

    #[test]
    fn a_clear_paints_the_whole_canvas_in_fill_colour_p() {
        let mut canvas = LineArtCanvas::new();
        let pic = canvas
            .draw(&stream(&[(0x6A, 4, 9), (0x20, 16, 0)]))
            .unwrap();
        assert!(
            pic.pixels().iter().all(|&v| v == 2 * 4 + 2),
            "solid pair 4 = (2, 2)"
        );
        let p = pic.painted().unwrap();
        assert_eq!((p.left(), p.top(), p.right(), p.bottom()), (0, 0, 159, 95));
    }

    #[test]
    fn a_line_plots_both_endpoints_in_the_line_colour_and_moves_the_pen() {
        let mut canvas = LineArtCanvas::new();
        // colour 0 = (3, 3); a diagonal from (10, 10) to (13, 12), then a
        // second line that starts where the first ended.
        let pic = canvas
            .draw(&stream(&[
                (0x60, 9, 9),
                (0x80, 10, 10),
                (0xA0, 13, 12),
                (0xA0, 13, 14),
            ]))
            .unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(10, 10), 15);
        assert_eq!(at(13, 12), 15);
        assert_eq!(at(13, 14), 15);
        assert_eq!(
            pic.pixels().iter().filter(|&&v| v == 15).count(),
            4 + 2,
            "four on the first, two more on the second"
        );
        let p = pic.painted().unwrap();
        assert_eq!((p.left(), p.top(), p.right(), p.bottom()), (10, 10, 13, 14));
    }

    #[test]
    fn a_flood_fill_stops_at_a_line_and_a_bounded_fill_stops_at_the_line_colour() {
        // A 20x20 box outlined in colour 0 on black, flood-filled inside with
        // colour 4 solid; the outside stays black and the outline stays.
        let mut canvas = LineArtCanvas::new();
        let pic = canvas
            .draw(&stream(&[
                (0x60, 4, 9),
                (0x80, 40, 40),
                (0xA0, 60, 40),
                (0xA0, 60, 60),
                (0xA0, 40, 60),
                (0xA0, 40, 40),
                (0xE0, 50, 50),
            ]))
            .unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(50, 50), 10, "inside: pair 4 = (2, 2)");
        assert_eq!(at(41, 59), 10, "inside corner");
        assert_eq!(at(40, 40), 15, "the outline survives");
        assert_eq!(at(30, 50), 0, "outside stays black");
        assert_eq!(at(70, 50), 0);
        // The bounded form fills everything that is not the line colour,
        // including the black outside, up to the canvas edge — seeded
        // outside the box it never crosses the outline.
        let mut canvas = LineArtCanvas::new();
        let pic = canvas
            .draw(&stream(&[
                (0x60, 4, 9),
                (0x80, 40, 40),
                (0xA0, 60, 40),
                (0xA0, 60, 60),
                (0xA0, 40, 60),
                (0xA0, 40, 40),
                (0xC0, 10, 50),
            ]))
            .unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(10, 50), 10);
        assert_eq!(at(50, 50), 0, "the inside was never reached");
        assert_eq!(at(40, 40), 15);
    }

    #[test]
    fn the_fill_sweeps_down_from_the_top_of_the_seed_column_and_no_further_up() {
        // An L-shaped region: a black column x=10..19 for all rows, plus a
        // black bar y=0..9 across x=10..59. Seeding at (15, 50) reaches the
        // whole column (up to row 0) but only the bar's rows via the column,
        // and then each row's span to the right — so the bar IS filled; a
        // region reachable only above the top of the seed column is not.
        let mut canvas = LineArtCanvas::new();
        // Paint the canvas with colour 0 first, then cut the region back to
        // black with lines? Simpler: fill colour 0 everywhere, then draw the
        // region in black (colour 9) lines row by row.
        let mut tokens = vec![(0x60, 0, 9), (0x20, 0, 0), (0x69, 9, 9)];
        for y in 0..96u8 {
            tokens.push((0x80, 10, y));
            tokens.push((0xA0, 19, y));
        }
        for y in 0..10u8 {
            tokens.push((0x80, 10, y));
            tokens.push((0xA0, 59, y));
        }
        // An island of black at rows 20..29, x=100..109, joined to the bar by
        // nothing: unreachable.
        for y in 20..30u8 {
            tokens.push((0x80, 100, y));
            tokens.push((0xA0, 109, y));
        }
        tokens.push((0x64, 4, 9));
        tokens.push((0xE0, 15, 50));
        let pic = canvas.draw(&stream(&tokens)).unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(15, 95), 10, "column bottom");
        assert_eq!(at(15, 0), 10, "column top");
        assert_eq!(
            at(55, 5),
            10,
            "the bar, reached by the sweep down from row 0"
        );
        assert_eq!(at(105, 25), 0, "the island is never reached");
        assert_eq!(at(30, 50), 15, "the ground keeps colour 0");
    }

    #[test]
    fn the_recolour_marker_redraws_earlier_lines_of_colour_p_in_colour_q() {
        let mut canvas = LineArtCanvas::new();
        // Two lines in colour 0, one in colour 4, then `6? 0 9` and `21`:
        // the colour-0 lines become black (9), the colour-4 line stays.
        let pic = canvas
            .draw(&stream(&[
                (0x60, 9, 9),
                (0x80, 10, 10),
                (0xA0, 20, 10),
                (0x64, 9, 9),
                (0x80, 10, 20),
                (0xA0, 20, 20),
                (0x60, 0, 9),
                (0x21, 0, 0),
            ]))
            .unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(15, 10), 0, "colour 0 became black");
        assert_eq!(at(15, 20), 10, "colour 4 untouched");
        let p = pic.painted().unwrap();
        assert_eq!((p.top(), p.bottom()), (10, 20));
    }

    #[test]
    fn a_pause_yields_a_frame_and_a_final_clear_is_still_the_last_frame() {
        let mut canvas = LineArtCanvas::new();
        let frames = canvas
            .draw_frames(&stream(&[
                (0x60, 4, 9),
                (0x80, 0, 0),
                (0xA0, 159, 95),
                (0x41, 0, 0),
                (0x69, 9, 9),
                (0x20, 0, 0),
            ]))
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(
            frames[0].pixels().iter().any(|&v| v != 0),
            "the diagonal is on the paused frame"
        );
        assert!(
            frames[1].pixels().iter().all(|&v| v == 0),
            "the final frame is the clear"
        );
        assert_eq!(
            canvas.draw(&stream(&[(0x41, 0, 0)])).unwrap().painted(),
            None
        );
    }

    #[test]
    fn state_carries_between_records_the_way_an_object_draws_over_its_room() {
        let mut canvas = LineArtCanvas::new();
        canvas.draw(&stream(&[(0x64, 4, 9), (0x20, 0, 0)])).unwrap();
        // The object sets no colours: its fill uses the room's (4, 9), its
        // lines the default colour 10, and its painted box is its own.
        let pic = canvas
            .draw(&stream(&[(0x80, 50, 50), (0xA0, 50, 55)]))
            .unwrap();
        let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
        assert_eq!(at(50, 52), COLOUR_PAIRS[10].0 * 4 + COLOUR_PAIRS[10].1);
        assert_eq!(at(0, 0), 10, "the room's clear is still there");
        let p = pic.painted().unwrap();
        assert_eq!((p.left(), p.top(), p.right(), p.bottom()), (50, 50, 50, 55));
    }

    #[test]
    fn malformed_streams_are_named_and_refused() {
        let mut canvas = LineArtCanvas::new();
        assert_eq!(
            canvas.draw(&[0x80, 10]),
            Err(LineArtError::Truncated { len: 2 })
        );
        assert_eq!(
            canvas.draw(&[0x80, 10, 10]),
            Err(LineArtError::Truncated { len: 3 })
        );
        assert_eq!(
            canvas.draw(&[0x80, 160, 10, 0]),
            Err(LineArtError::OffCanvas {
                at: 0,
                x: 160,
                y: 10
            })
        );
        assert_eq!(
            canvas.draw(&[0x80, 10, 96, 0]),
            Err(LineArtError::OffCanvas {
                at: 0,
                x: 10,
                y: 96
            })
        );
        assert_eq!(
            canvas.draw(&[0x70, 0, 0, 0]),
            Err(LineArtError::BadColour { at: 0, index: 16 })
        );
        assert_eq!(
            canvas.draw(&[0x60, 16, 0, 0]),
            Err(LineArtError::BadColour { at: 0, index: 16 })
        );
        assert_eq!(
            canvas.draw(&[0xE5, 0, 0, 0]),
            Err(LineArtError::BadFillStyle { at: 0, style: 5 })
        );
        assert_eq!(
            canvas.draw(&[0x00]).unwrap().painted(),
            None,
            "an empty record is a picture that drew nothing"
        );
    }
}
