//! Draw a resolved screen as a picture (SQ-0775).
//!
//! WHY THIS EXISTS. Most of the render quests in this repo end the same way:
//! "the user must go look at it". A capture can prove an image was placed over
//! rows 8..40 and that the cells under it were painted `#001428`, and still not
//! answer the only question that matters — does the frame LOOK right. The person
//! who has to answer it is usually working over ssh with no terminal to run
//! lanthorn in. So this module turns [`super::oracle::Resolved`] into an RGBA
//! canvas and a PNG: a frame that can be looked at instead of reproduced, and a
//! before/after pair that makes a render change reviewable with no terminal at
//! all.
//!
//! WHAT IT IS NOT. It is not a screenshot, and no amount of squinting will make
//! it one. Text is drawn with the repo's own bitmap font
//! ([`app::render::bitfont`], the one the v6 pixel composite uses) — Uni-VGA at
//! 8x16 since SQ-0932, where it used to be an 8x8 master doubled into the cell,
//! so these pictures gained real descenders and a proper stroke weight without
//! anyone asking. There is still no hinting, no ligatures, and bold and italic
//! are synthesized from the roman master rather than being real faces.
//!
//! What it IS honest about is LAYOUT, ART PLACEMENT and COLOUR: where the panes
//! are, where the art landed, what was painted underneath it, which of two
//! overlapping things won. Those are the defects the render quests are about.
//! Judge geometry from this picture; never judge typography from it — and the
//! better the face gets, the more tempting that becomes and the more firmly it
//! stays wrong, because the face here is never the one in your terminal.
//!
//! WHAT IT REFUSES TO HIDE. Every draw is rasterised from its OWN resolved
//! source rect ([`super::oracle::Draw`]), one per resolved placement — never
//! from the aggregated rect. A virtual placement resolves per screen row, and a
//! run that lost its anchor draws the image's FIRST row on every row of the
//! rect (SQ-0772). Sampling per-draw means the picture shows that as the banded
//! smear it is on screen. A rasteriser that drew each image once into its
//! bounding box would produce a clean, plausible, WRONG picture of exactly the
//! bug we most need to see.
//!
//! THE OTHER HONEST LIMIT. A cell the app never painted is drawn in the
//! EMULATOR's default background (palette entry 0), not in whatever the real
//! terminal would use: a capture only sees the app→terminal direction, so the
//! colour the pty answered the OSC 11 probe with is not in the bytes being
//! resolved. Where lanthorn paints its own background — most of a v6 frame —
//! this makes no difference; on a screen it leaves at the terminal default (an
//! Arthur boot prompt, say) the whole frame carries the palette's dark grey
//! instead of the player's own background.
//!
//! DRAW ORDER is the kitty protocol's, not ours (kitty graphics protocol, "Z
//! index"): placements below `-1073741824` draw under the cell backgrounds,
//! placements from there to `-1` draw over the backgrounds but under the text,
//! and `z >= 0` draws over everything. The z used for that bucketing is the one
//! the RENDERER sorts on, which for every virtual placement is the `-1` upstream
//! hardcodes — not the z the client asked for (see [`super::oracle::Draw::z`]).
//! A cell with no explicit background is left as the screen's default fill
//! rather than repainted, so a below-background placement can actually show
//! through one.
//!
//! WITHIN a z the protocol still decides: "if two images with the same z-index
//! overlap then the image with the lower id is considered to have the lower
//! z-index" (kitty graphics protocol, "Controlling displayed image layout"), so
//! the sort in [`render_with`] is `(z, image id, position)` and not `z` alone.
//! There is no "the order the resolver reported" to fall back on — that order
//! comes out of a `HashMap` and is re-randomised on every call, which for two
//! overlapping placements made this picture a coin flip between the right answer
//! and one where a superseded image lands on top and the newer one's transparency
//! was blended into it. Same z AND same id is undefined upstream; the position
//! tail of the key is there so the picture stays a function of the bytes anyway
//! (SQ-0968).

use app::render::bitfont::blit_glyph;
use image::{Rgba, RgbaImage};

use super::decode::Color;
use super::oracle::{Colors, Draw, OracleCell, RasterImage, Resolved};

/// Kitty's boundary between "under the cell background" and "over the background
/// but under the text". Read off the protocol document, not recalled: "Negative
/// z-index values below `INT32_MIN/2` (-1,073,741,824) will be drawn under cells
/// with non-default background colors" (kitty graphics protocol, "Controlling
/// displayed image layout"). Hence the strict `<` at both use sites, and hence
/// [`paint_backgrounds`] filling only the non-default ones.
const BELOW_BACKGROUND: i32 = i32::MIN / 2;

/// The kitty unicode placeholder. Its cells carry an image, not a glyph, and it
/// has no printable form.
const PLACEHOLDER: char = '\u{10EEEE}';

/// Draw the resolved screen: `cols x rows` cells at the capture's own cell size.
///
/// The canvas is `cols * cell_w` by `rows * cell_h` pixels — the pixel geometry
/// the stream was resolved under, so art lands where the arithmetic that placed
/// it says it does.
pub fn render(res: &Resolved) -> RgbaImage {
    render_with(res, &|canvas, ch, px, py, cw, chh, fg| blit_glyph(canvas, ch, px, py, cw, chh, fg, None))
}

/// How one glyph gets drawn into its cell: canvas, char, cell origin, cell size,
/// foreground.
///
/// The indirection exists for exactly one caller — the gallery tool (SQ-0942),
/// which draws with a real outline face because its output is meant to be looked
/// at. [`render`] keeps the bitmap master, so every test and every geometry
/// question is answered by the same deliberately-synthetic picture it always
/// was.
pub type GlyphPainter<'a> = &'a dyn Fn(&mut RgbaImage, char, u32, u32, u32, u32, Rgba<u8>);

/// [`render`], with the glyph painter supplied.
pub fn render_with(res: &Resolved, glyph: GlyphPainter<'_>) -> RgbaImage {
    let width = u32::from(res.cols) * res.cell_w;
    let height = u32::from(res.rows) * res.cell_h;
    let mut canvas = RgbaImage::from_pixel(width.max(1), height.max(1), rgba(res.colors.default_bg()));

    // z first, then the protocol's own tie-break, then position. The order the
    // RESOLVER reported is not a fallback available to us: `resolve_placements`
    // walks `ImageStorage::placements`, a `HashMap`, so its order is a fresh
    // random permutation on every call — two renders of the SAME bytes in the same
    // process disagreed on which of two overlapping placements won, which made
    // this instrument's picture a coin flip and is the whole of SQ-0968.
    let mut draws: Vec<&Draw> = res.draws.iter().collect();
    draws.sort_by_key(|d| (d.z, d.image_id, d.dest_y, d.dest_x, d.src_y, d.src_x));

    for d in draws.iter().filter(|d| d.z < BELOW_BACKGROUND) {
        composite(&mut canvas, d, res);
    }
    paint_backgrounds(&mut canvas, res);
    for d in draws.iter().filter(|d| (BELOW_BACKGROUND..0).contains(&d.z)) {
        composite(&mut canvas, d, res);
    }
    paint_glyphs(&mut canvas, res, glyph);
    for d in draws.iter().filter(|d| d.z >= 0) {
        composite(&mut canvas, d, res);
    }
    canvas
}

/// Two rasters side by side with a divider between them, for a before/after
/// pair. Differing sizes are allowed — the canvas is the taller of the two, and
/// the shorter one sits against the default fill, which is itself a difference
/// worth seeing.
pub fn side_by_side(before: &RgbaImage, after: &RgbaImage) -> RgbaImage {
    const GUTTER: u32 = 8;
    let width = before.width() + GUTTER + after.width();
    let height = before.height().max(after.height());
    let mut out = RgbaImage::from_pixel(width.max(1), height.max(1), Rgba([24, 24, 24, 255]));
    for (x, y, p) in before.enumerate_pixels() {
        out.put_pixel(x, y, *p);
    }
    let dx = before.width() + GUTTER;
    for (x, y, p) in after.enumerate_pixels() {
        out.put_pixel(dx + x, y, *p);
    }
    for y in 0..height {
        for x in before.width()..dx {
            out.put_pixel(x, y, Rgba([200, 40, 40, 255]));
        }
    }
    out
}

/// Fill each cell that has an EXPLICIT background. Cells left at the terminal's
/// default keep the canvas fill, which is what lets a below-background placement
/// show through one.
fn paint_backgrounds(canvas: &mut RgbaImage, res: &Resolved) {
    for row in 0..res.rows {
        for col in 0..res.cols {
            let cell = res.cell(row, col);
            let Some(bg) = cell_bg(&res.colors, cell) else { continue };
            let px = u32::from(col) * res.cell_w;
            let py = u32::from(row) * res.cell_h;
            fill(canvas, px, py, res.cell_w, res.cell_h, rgba(bg));
        }
    }
}

/// Blit every printable glyph over whatever is already there. Transparent
/// background (`None`): the cell fill and any under-text art have already been
/// drawn, and text must not erase them.
fn paint_glyphs(canvas: &mut RgbaImage, res: &Resolved, glyph: GlyphPainter<'_>) {
    for row in 0..res.rows {
        for col in 0..res.cols {
            let cell = res.cell(row, col);
            if matches!(cell.ch, ' ' | '\0' | PLACEHOLDER) {
                continue;
            }
            let px = u32::from(col) * res.cell_w;
            let py = u32::from(row) * res.cell_h;
            glyph(canvas, cell.ch, px, py, res.cell_w, res.cell_h, rgba(cell_fg(&res.colors, cell)));
        }
    }
}

/// The colour this cell's glyph is drawn in, SGR 7 applied.
fn cell_fg(colors: &Colors, cell: &OracleCell) -> [u8; 3] {
    if cell.inverse { colors.bg(cell.bg) } else { colors.fg(cell.fg) }
}

/// The colour to fill this cell with, or `None` to leave the default fill.
/// An inverse cell ALWAYS fills — its background is the foreground colour, which
/// is a thing to paint even when the foreground was never set.
fn cell_bg(colors: &Colors, cell: &OracleCell) -> Option<[u8; 3]> {
    if cell.inverse {
        Some(colors.fg(cell.fg))
    } else if matches!(cell.bg, Color::Default) {
        None
    } else {
        Some(colors.bg(cell.bg))
    }
}

/// Draw one resolved placement, scaling its source rect to its destination size
/// by nearest neighbour and blending on alpha.
///
/// Nearest neighbour, not filtering: this picture is read for geometry, and a
/// filtered edge would make a one-pixel misplacement look like a soft edge
/// rather than a defect. Destination pixels outside the canvas are dropped, the
/// way a rasteriser clips a placement scrolled off the top.
fn composite(canvas: &mut RgbaImage, d: &Draw, res: &Resolved) {
    let Some(img) = res.images.get(&d.image_id) else { return };
    if d.dest_w == 0 || d.dest_h == 0 || d.src_w == 0 || d.src_h == 0 || img.width == 0 {
        return;
    }
    let (cw, ch) = (i64::from(canvas.width()), i64::from(canvas.height()));
    for dy in 0..d.dest_h {
        let y = d.dest_y + i64::from(dy);
        if y < 0 || y >= ch {
            continue;
        }
        let sy = d.src_y + (dy * d.src_h) / d.dest_h;
        if sy >= img.height {
            continue;
        }
        for dx in 0..d.dest_w {
            let x = d.dest_x + i64::from(dx);
            if x < 0 || x >= cw {
                continue;
            }
            let sx = d.src_x + (dx * d.src_w) / d.dest_w;
            if sx >= img.width {
                continue;
            }
            let Some(src) = texel(img, sx, sy) else { continue };
            blend(canvas, x as u32, y as u32, src);
        }
    }
}

/// One RGBA texel, or `None` if the image's data is short of its declared size
/// (a truncated transfer the terminal accepted).
fn texel(img: &RasterImage, x: u32, y: u32) -> Option<[u8; 4]> {
    let i = (y as usize * img.width as usize + x as usize) * 4;
    let px = img.rgba.get(i..i + 4)?;
    Some([px[0], px[1], px[2], px[3]])
}

/// Source-over blend of one texel onto the canvas. lanthorn's art is opaque, so
/// this is usually a straight overwrite; alpha is honoured anyway because the
/// protocol allows it and a half-transparent placement should look like one.
fn blend(canvas: &mut RgbaImage, x: u32, y: u32, src: [u8; 4]) {
    if src[3] == 255 {
        canvas.put_pixel(x, y, Rgba(src));
        return;
    }
    if src[3] == 0 {
        return;
    }
    let a = u32::from(src[3]);
    let dst = canvas.get_pixel(x, y).0;
    let mix = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * (255 - a)) / 255) as u8;
    canvas.put_pixel(
        x,
        y,
        Rgba([mix(src[0], dst[0]), mix(src[1], dst[1]), mix(src[2], dst[2]), 255]),
    );
}

fn fill(canvas: &mut RgbaImage, px: u32, py: u32, w: u32, h: u32, color: Rgba<u8>) {
    for y in py..(py + h).min(canvas.height()) {
        for x in px..(px + w).min(canvas.width()) {
            canvas.put_pixel(x, y, color);
        }
    }
}

fn rgba(c: [u8; 3]) -> Rgba<u8> {
    Rgba([c[0], c[1], c[2], 255])
}
