//! SQ-0698 / SQ-0781 — vertical extension of Infocom v6 **side border art**.
//!
//! Three of the graphical v6 titles frame their story window with side artwork
//! that was authored for a 320x200 screen and does not reach the bottom of a
//! modern pane. lanthorn used to make up the difference by *stretching* the
//! flank band vertically (SQ-0511), which elongates the art by whatever the
//! letterbox slack happens to be — measured at 2.2x on Zork Zero and 3.0x on
//! Shogun at a 117x64 terminal, against a horizontal factor of 1.0x. Shogun and
//! Arthur are worse off still: their art genuinely stops short in NATIVE space
//! (Shogun's border ends at native row 336 of 400; Arthur's poles at 379), so
//! the stretch spread an empty band over the bottom of the strip.
//!
//! This module TILES instead, the way Spatterlight's Bocfel does
//! (`terps/bocfel/z6/draw_border.cpp`, header: *"Used by Arthur, Shogun, and
//! Zork Zero"*, rationale *"The original games did not do this, but it looks
//! better with modern screen sizes"*). Read for MECHANISM, not policy — Bocfel
//! never scales border art horizontally either (`draw_to_pixmap_unscaled*`
//! throughout), because it never fits art to a terminal pane.
//!
//! ## Shape of the code
//!
//! A small toolkit of primitives — [`snapshot`], [`stamp`], [`tile_down`],
//! [`erase_below`] — plus a port of Bocfel's [`extend_pillars`], and then one
//! handler per title. The "derive a general tile-vs-stretch discriminator"
//! requirement was dropped deliberately (SQ-0698, 2026-08-11): the reference
//! could not do it either, hard-coding per game *and* per platform. What is
//! derived here is WHICH of the three known layouts a flank is showing
//! ([`recognize`]), from the art's own native extent — and, for Zork Zero, WHERE
//! its pillars are ([`pillar_shaft`]), because lanthorn lets the player choose
//! the rendition and Bocfel does not. Everything else is per title, named, and
//! sourced.
//!
//! Every row coordinate in this file is in lanthorn's v6 **unit space**, which
//! is the art's own pixels doubled *vertically* (`session::V6_ART_SCALE` = 2;
//! the horizontal factor is 1 for a 640-wide EGA or CGA archive, whose pixels
//! are half as wide — SQ-0790). Bocfel's constants are in raw art rows, so each
//! one appears here doubled, and the doubling is called out at every constant.

use image::{Rgba, RgbaImage};

/// One v6 text row in unit space — `InterpreterProfile::v6_font_cell()`'s 16,
/// which is the grain a v6 screen height is rounded to and therefore the most a
/// full-height plate can fall short of the screen it was drawn for. See
/// [`recognize`].
const V6_TEXT_ROW: u32 = 16;

/// The narrowest painted row a **single-piece border** has, in unit columns —
/// the third measurement [`recognize`] needs, and the one SQ-0881 added.
///
/// `top == 0` alone stood in for "single piece" until a flank turned up that
/// starts at row 0 and is *not* one: the MACINTOSH press of Arthur. Its poles
/// run native rows 0..368 of 400 (0..275 of 304 in the monochrome archive),
/// where the Amiga press runs them 11..379 — so a nonzero top had been carrying
/// the whole distinction, by luck of which media had been measured. Arthur's
/// Macintosh flank therefore took `shogun()`, which extends by stamping a second
/// copy of the WHOLE border below the first; that copy carries the top banner,
/// and the player sees a piece of it tiled down the side of the screen.
///
/// Neither existing measurement can separate the two — Arthur's Macintosh flank
/// and Shogun's Amiga flank (0..336 of 400) both start at row 0, both stop short
/// of the bottom, and both are slabs of constant width. What does separate them
/// is how WIDE they are, which is the difference between a decorated panel and a
/// narrow column, and it is not close:
///
/// | flank | measured width |
/// |---|---|
/// | Shogun, Macintosh colour | 60 |
/// | Shogun, Amiga (r295) | 46 |
/// | Shogun, Macintosh monochrome | 44 |
/// | Arthur, Macintosh colour | 12 |
/// | Arthur, Macintosh monochrome | 8 |
/// | Arthur, Amiga (r54) | 6 |
///
/// The cut sits in the middle of a gap nearly four times wide, so it is a
/// threshold in name only. Both ends are pinned in the tests below and swept
/// over the whole corpus by `v6_archive_border_sweep.rs`.
const SINGLE_PIECE_MIN_WIDTH: u32 = 24;

/// Which of the three Infocom v6 side-border layouts a flank is showing.
///
/// Recognised from the art's own native extent rather than from the story's
/// identity: the renderer is handed a screen model and a canvas, and has no
/// path to the release it came from (`ScreenModel` is engine-neutral and is
/// built at 64 sites). The three shapes are distinguishable in two measurements
/// — see [`recognize`], which is pinned over the whole v6 corpus by
/// `v6_side_border_tiling.rs`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BorderArt {
    /// **Arthur** — two narrow poles (pictures 170/171) hanging below the top
    /// banner (54), stopping short of the screen bottom.
    ArthurPoles,
    /// **Shogun** — one single-piece border image (`P_BORDER` = 3 or
    /// `P_BORDER2` = 4, two alternative styles for different scenes) that
    /// carries the whole frame, top edge included, and ends above the screen
    /// bottom.
    ShogunSinglePiece,
    /// **Zork Zero** — architectural pillars, capital to base, painted to the
    /// native screen bottom; only a pane taller than the art needs them
    /// extended.
    ZorkZeroPillars,
}

/// The narrowest and widest painted row of a flank — `(min, max)` opaque column
/// span width over native columns `[x0, x1)`, rows `art.0..art.1`. `None` when
/// nothing there is painted.
///
/// This is the *shape* of a flank reduced to two numbers, and it is what tells a
/// pillar from a slab: a pillar has a banner or a capital wider than the column
/// hanging below it, a slab holds one width from top to bottom.
fn painted_widths(canvas: &RgbaImage, x0: u32, x1: u32, art: (u32, u32)) -> Option<(u32, u32)> {
    let x1 = x1.min(canvas.width());
    let (mut lo, mut hi) = (u32::MAX, 0u32);
    for y in art.0..art.1.min(canvas.height()) {
        let (mut first, mut last) = (None, 0);
        for x in x0..x1 {
            if canvas.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        if let Some(f) = first {
            lo = lo.min(last - f + 1);
            hi = hi.max(last - f + 1);
        }
    }
    (hi > 0).then_some((lo, hi))
}

/// Does this flank repeat an ORNAMENT — two or more full-width bands of the same
/// height, spaced down the column (SQ-0899)?
///
/// [`painted_widths`] reduces a column to its narrowest and widest row and throws
/// away where those rows are. That is enough to tell a pillar from a slab, and not
/// enough to tell a pillar from a POLE: both narrow, and the difference is that a
/// capital happens once at the head of its pillar where an ornament recurs down a
/// pole. Measured as the run-length profile of each column's painted span, on the
/// frame each title draws:
///
/// | flank | span profile, top to bottom | full-width bands |
/// |---|---|---|
/// | Arthur, ProDOS (r63) | `8x4 17x34 8x30 17x34 8x32 17x34 8x216` | 34, 34, 34 |
/// | Arthur, Amiga (r54) | `6x4 11x34 6x30 11x34 6x32 11x34 6x200` | 34, 34, 34 |
/// | Arthur, DOS (r74) | `6x4 11x34 6x30 11x34 6x32 11x34 4x200` | 34, 34, 34 |
/// | Zork Zero, castle (MCGA/EGA/Amiga) | `17x82 3x292 …foot` | 82 |
/// | Zork Zero, CGA castle | 39 runs, widest at 17 for 2–10 rows | none |
/// | Zork Zero, Amiga plate 7 left | `86x44 84x8 86x20 68x2 72x54 …` | 44, 20 |
/// | Zork Zero, Amiga plate 7 right | `86x74 72x8 70x2 70x58 …` | 74 |
///
/// Arthur's ornaments are **thirty-four rows on every press he has**, which is
/// what makes them one repeated drawing rather than a silhouette that happens to
/// touch full width twice. On a 560x384 ProDOS screen his poles reach both edges,
/// so the inset that identifies him everywhere else reads 0 and he was handed Zork
/// Zero's masonry — the ornament tiled down the plain shaft, which is the "banner
/// repeating down the flank" SQ-0899 reports.
///
/// **Equal heights, and not merely two of them, and that is the whole subtlety.**
/// A bare count of full-width bands classifies Zork Zero's own Amiga plate 7 as a
/// pole on its LEFT crop and a pillar on its RIGHT, because that scene border is
/// organic art whose silhouette wanders back to full width at 44 rows and again at
/// 20. One plate is one symmetric drawing and its two crops must classify alike —
/// `v6_archive_border_sweep`'s property 6, SQ-0845 — and that sweep is what caught
/// it across sixty-eight flanks. Two bands of unequal height are a coastline; two
/// of the same height are a repeat.
///
/// The height floor keeps CGA out: Zork Zero's CGA pillar is dithered and its span
/// oscillates to full width for two to ten rows the whole way down its capital, so
/// a floorless reading finds six bands there. A band thinner than a text row is
/// dither, not architecture. This test only ever REMOVES a flank from the pillar
/// recipe, so answering `false` is the conservative side of it.
fn repeats_an_ornament(canvas: &RgbaImage, x0: u32, x1: u32, art: (u32, u32), hi: u32) -> bool {
    let x1 = x1.min(canvas.width());
    let (mut heights, mut run) = (Vec::new(), 0u32);
    for y in art.0..art.1.min(canvas.height()) {
        let (mut first, mut last) = (None, 0);
        for x in x0..x1 {
            if canvas.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        if first.is_some_and(|f| last - f + 1 >= hi) {
            run += 1;
        } else {
            if run >= V6_TEXT_ROW {
                heights.push(run);
            }
            run = 0;
        }
    }
    if run >= V6_TEXT_ROW {
        heights.push(run);
    }
    heights.len() >= 2 && heights.windows(2).all(|w| w[0] == w[1])
}

/// Classify a flank from the shape of its own native columns.
///
/// `art` is `(first, last_exclusive)` opaque native row within columns
/// `[x0, x1)` of `canvas`; `native_h` is the native screen height (400 for every
/// v6 title we carry). `None` when the flank has no art at all — the caller then
/// keeps its existing behaviour.
///
/// Two measurements separate the three, and the ORDER matters because Zork
/// Zero's banner covers its flank columns from row 0 just as Shogun's border
/// does:
///
/// | title            | art rows (measured) | reaches bottom | narrows | starts at 0 |
/// |------------------|---------------------|----------------|---------|-------------|
/// | Arthur (r54 adf) | 11..379             | no             | –       | no          |
/// | Shogun (r295 adf)| 0..336              | no             | –       | yes         |
/// | Shogun (DOS)     | 0..400              | **yes**        | **no**  | yes         |
/// | Zork Zero (r393) | 0..400              | **yes**        | **yes** | yes         |
///
/// **Reaching the bottom is not enough on its own, and that was SQ-0802.**
/// Shogun's DOS art is authored for the full 200-row screen where its Amiga art
/// stops at 168, so `.MG1`, `.EG1`, `.CG1` and the Blorb all satisfy the bottom
/// test and took Zork Zero's masonry recipe — cut at the shaft, a repeat, a foot
/// — applied to a Japanese lacquer frame. The second measurement is [`painted_widths`]:
///
/// * **Shogun's border is a slab.** Measured on `shogun-r322-s890706.z6` at a
///   gameplay frame, both flanks, every rendition: narrowest ÷ widest painted row
///   is **1.00** on `.mg1`, `.eg1` and `Shogun.blb`, 1.00 and **0.96** on the two
///   `.cg1` flanks, and 1.00 on `James Clavell's Shogun.adf` (release 295).
/// * **Zork Zero's is a pillar under a banner.** Same measurement across all five
///   renditions and all three of its scene borders (castle in-game; underground
///   and jungle composed from the archives' own pictures 7/6 and 0x1f3–0x1f6 at
///   the flank width one `TEXT_WINDOW_PIC_LOC` fixes for all three): **0.02–0.56**
///   castle, **0.77–0.81** underground, **0.37–0.80** jungle.
///
/// So every Zork Zero flank we can measure is at or below 0.81 and every Shogun
/// flank at or above 0.96. The cut is at **9/10**, in the gap, and both ends of
/// it are pinned by `v6_side_border_tiling.rs`.
///
/// **"Reaches the bottom" means to within one text row, and that is SQ-0841.**
/// A v6 screen is the archive's picture space rounded UP to a whole cell, so a
/// full-height plate can stop short of the screen by that rounding. Every
/// rendition but one divides exactly and nobody noticed: the standard
/// Macintosh's monochrome archive is 480x300 laid on lanthorn's 8x16 cell, which
/// rounds to a **304**-row screen (SQ-0838 — see `InterpreterProfile::v6_font_cell`
/// for the four pixels of slack it names by hand). Zork Zero's monochrome
/// pillars are painted to row 300 of that 304, so an exact test read them as
/// Shogun's single-piece slab and extended them by MIRRORING the whole column —
/// which is what stamped a second capital, and a length of bare shaft, below
/// their feet.
///
/// **The slack is the WHOLE inset, not the bottom's alone — SQ-0881.**
/// Measuring only the bottom left the top free, and a tolerance with a free end
/// gets met by something it was not sized for. Arthur's Macintosh MONOCHROME
/// plate is drawn at native rows 14..289 of that same 304-row screen: fifteen
/// short at the bottom, one inside the sixteen allowed. It also narrows — a
/// 16-wide banner over a 5-wide pole, ratio 0.31 — so both older measurements
/// agreed on Zork Zero, and the player got Zork Zero's masonry recipe stamped
/// down Arthur's side, capital and all. That is the "piece of the upper frame"
/// tiling down the flank.
///
/// A pillar under a banner SPANS its screen: the banner is painted to the top
/// edge and the pillars stand on the bottom one, and the only thing between the
/// art and the frame is the cell rounding SQ-0841 named. So charge that rounding
/// once, against `top + (native_h - bottom)`, and the two titles separate by a
/// factor of seven. Measured in-game at 165x50 through the pty harness, on the
/// frame each title actually draws:
///
/// | title | rendition | art rows | screen | inset |
/// |---|---|---|---|---|
/// | Zork Zero | Macintosh monochrome | 0..300 | 304 | **4** |
/// | Zork Zero | Macintosh colour, DOS `.MG1`, Amiga | 0..400 | 400 | **0** |
/// | Shogun | DOS `.MG1` | 0..400 | 400 | **0** |
/// | Arthur | Macintosh monochrome | 14..289 | 304 | **29** |
/// | Arthur | Macintosh colour, Amiga (r54) | 11..379 | 400 | **32** |
/// | Arthur | DOS `.MG1`, Blorb | 16..384 | 400 | **32** |
///
/// Shogun's Amiga flank (0..336 of 400, inset 64) fails this as it failed the
/// bottom test, and lands where it always did. The one flank in the corpus that
/// is neither perfectly flush nor inset is `Zork Zero - The Revenge of
/// Megaboz.adf`'s **plate 6, right side**, whose art begins at row **2** where
/// its own left side begins at 0 — two blank rows in the drawing, and the reason
/// this is a tolerance rather than `top == 0`. `v6_archive_border_sweep` asserts
/// a plate's two crops classify alike, and that asymmetry is what it caught.
///
/// Note how narrowly the DOS press escaped under the bottom-only test: 384 + 16
/// is 400, which is not *greater* than 400, so one row of inset stood between it
/// and the same fault. Against the whole inset its margin is sixteen.
pub fn recognize(canvas: &RgbaImage, x0: u32, x1: u32, art: (u32, u32), native_h: u32) -> Option<BorderArt> {
    let (top, bottom) = art;
    if bottom <= top {
        return None;
    }
    let (lo, hi) = painted_widths(canvas, x0, x1, art)?;
    let inset = top + native_h.saturating_sub(bottom);
    // …and the wide part does not RECUR. A capital sits at the head of its pillar;
    // an ornament of one height repeats down a pole, and a column carrying several is
    // Arthur's however flush it sits. See [`repeats_an_ornament`] — SQ-0899.
    if inset <= V6_TEXT_ROW && lo * 10 < hi * 9 && !repeats_an_ornament(canvas, x0, x1, art, hi) {
        return Some(BorderArt::ZorkZeroPillars);
    }
    if top == 0 && hi >= SINGLE_PIECE_MIN_WIDTH {
        return Some(BorderArt::ShogunSinglePiece);
    }
    Some(BorderArt::ArthurPoles)
}

// ── Toolkit ──────────────────────────────────────────────────────────────────

/// Copy rows `[y, y + h)` of `src` into a new image of the same width. Rows past
/// the end of `src` come out transparent, exactly as Bocfel's
/// `copy_rect_from_bitmap` zero-fills them.
pub fn snapshot(src: &RgbaImage, y: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(src.width(), h.max(1));
    for oy in 0..h.min(out.height()) {
        let sy = y + oy;
        if sy >= src.height() {
            break;
        }
        for x in 0..src.width() {
            out.put_pixel(x, oy, *src.get_pixel(x, sy));
        }
    }
    out
}

/// Stamp `strip` into `dst` with its top at row `y`, optionally flipped
/// vertically, clipped to `dst`. Every pixel is copied, transparent ones
/// included: these strips are opaque artwork and a stamp REPLACES what is under
/// it, which is what makes an overlapping tile hide the seam below it.
pub fn stamp(dst: &mut RgbaImage, strip: &RgbaImage, y: u32, flipped: bool) {
    let h = strip.height();
    for sy in 0..h {
        let dy = y + sy;
        if dy >= dst.height() {
            break;
        }
        let src_y = if flipped { h - 1 - sy } else { sy };
        for x in 0..strip.width().min(dst.width()) {
            dst.put_pixel(x, dy, *strip.get_pixel(x, src_y));
        }
    }
}

/// Tile `strip` down `dst` from `start_y` while the stamp's top is at or above
/// `end_y`, stepping `strip.height() - overlap` rows at a time — Bocfel's
/// `tile_section_down`, whose stride is `pillar_height - overlap` so that tiles
/// OVERLAP rather than butt together. When `flip` is set each tile's vertical
/// flip alternates from `initial_parity`; otherwise every tile is drawn with
/// `initial_parity`. Returns the row the next tile would have started at.
///
/// Both devices exist to hide the seam in a repeated pattern. Arthur needs
/// neither (his repeat unit is two lines of a plain texture); Zork Zero's
/// patterned masonry is the case they were written for.
pub fn tile_down(
    dst: &mut RgbaImage,
    strip: &RgbaImage,
    start_y: u32,
    end_y: u32,
    overlap: u32,
    initial_parity: bool,
    flip: bool,
) -> u32 {
    let stride = strip.height().saturating_sub(overlap).max(1);
    let mut parity = initial_parity;
    let mut y = start_y;
    while y <= end_y {
        stamp(dst, strip, y, parity);
        if flip {
            parity = !parity;
        }
        y += stride;
    }
    y
}

/// Clear every row of `dst` from `y` down — Bocfel's `erase_lines_in_bitmap`,
/// used to drop the overshoot of the last whole tile before the foot goes on.
pub fn erase_below(dst: &mut RgbaImage, y: u32) {
    for dy in y..dst.height() {
        for x in 0..dst.width() {
            dst.put_pixel(x, dy, Rgba([0, 0, 0, 0]));
        }
    }
}

/// Bocfel's `extend_pillars()`, ported: **capital → tiled shaft → foot**.
///
/// The art occupies rows `[0, total_height)`; its bottom `foot_height` rows are
/// its base, and rows `[top_cut, top_cut + pillar_height)` are the unit that
/// repeats. Tiling starts at `total_height - foot_height - overlap` (i.e. where
/// the foot was) and runs to `desired_height`, then the foot is stamped at
/// `desired_height - foot_height` with everything below it erased.
///
/// **The ordering caveat is Bocfel's own, and it is not optional:** snapshot the
/// repeat unit BEFORE erasing the foot. For Arthur `top_cut` equals
/// `total_height - foot_height` exactly, so the unit's source rows sit inside
/// the region erased immediately below — copy first, erase second, or the whole
/// extension comes out blank.
///
/// One deliberate divergence: Bocfel nudges Arthur's foot up onto an even row
/// (`if (is_spatterlight_arthur) foot_top -= (foot_top & 1);`) to keep his
/// 2-line texture in phase where it meets the foot. It does not do that here.
/// Bocfel can afford the nudge because its pixmap is clipped to
/// `desired_height` and then scaled as a whole; ours is a band placed at a rect
/// the caller already fixed, so pulling the foot up leaves an unpainted sliver
/// against the pane's bottom edge. A gap at the bottom of the frame is a defect
/// anyone can see; a one-raw-line phase jump inside a vertical texture is not.
#[allow(clippy::too_many_arguments)]
pub fn extend_pillars(
    dst: &mut RgbaImage,
    top_cut: u32,
    foot_height: u32,
    total_height: u32,
    pillar_height: u32,
    overlap: u32,
    flip: bool,
    desired_height: u32,
) {
    if desired_height < total_height || foot_height == 0 || pillar_height == 0 {
        return;
    }
    let section = snapshot(dst, top_cut, pillar_height);
    let foot = snapshot(dst, total_height - foot_height, foot_height);
    erase_below(dst, total_height - foot_height);

    let start_y = total_height.saturating_sub(foot_height + overlap);
    // Bocfel: `bool initial_parity = flip;` — when flipping is on, the FIRST
    // tile is the flipped one and the alternation runs from there.
    tile_down(dst, &section, start_y, desired_height, overlap, flip, flip);

    let foot_top = desired_height.saturating_sub(foot_height);
    erase_below(dst, foot_top);
    stamp(dst, &foot, foot_top, false);
}

// ── Per-title handlers ───────────────────────────────────────────────────────

/// **Arthur** — `draw_arthur_side_images()` in `draw_border.cpp`.
///
/// Bocfel cuts a **2-line** horizontal strip at 90% of the pole art's own height
/// and repeats it plainly (`extend_pillars(place_to_cut, foot_height,
/// total_height, /*pillar=*/2, /*overlap=*/0, /*flip=*/false, …)`), then stamps
/// the original bottom 10% back on as the foot. The poles are a plain vertical
/// texture, so neither overlap nor flip is wanted: the `kBWHint*` seam
/// machinery an earlier reading attributed to Arthur belongs to the Mac B/W
/// *hint* border, and Zork Zero's masonry.
///
/// Doubled into unit space the strip is **4 rows**, and `top_cut` is rounded
/// down to an even row so the doubled texture keeps its phase.
///
/// Bocfel additionally shortens the foot on Amiga (`foot_height -= top_margin;
/// total_height -= top_margin`). That does NOT transfer: `arthur_pic_top_margin`
/// is a Glk window-layout quantity Bocfel introduces because it re-places
/// Arthur's frame itself, quantised to its own cell height. lanthorn never
/// re-places the art — the game draws it and we read the canvas back — so there
/// is no top margin to subtract. Measured on `Arthur - The Quest for
/// Excalibur.adf` (release 54, serial 890606): the poles run native rows
/// 11..379 in flank columns 10..15 (left) and 624..629 (right).
fn arthur(dst: &mut RgbaImage, art_bottom: u32, desired_height: u32) {
    /// Bocfel: `place_to_cut = (int)(hw_screenheight * 0.9)`.
    const CUT_NUM: u32 = 9;
    const CUT_DEN: u32 = 10;
    /// Bocfel's `pillar_height` argument is 2 raw lines; unit space doubles it.
    const UNIT_ROWS: u32 = 4;
    let top_cut = (art_bottom * CUT_NUM / CUT_DEN) & !1;
    let foot_height = art_bottom - top_cut;
    extend_pillars(dst, top_cut, foot_height, art_bottom, UNIT_ROWS, 0, false, desired_height);
}

/// **Shogun** — `shogun_extend_amiga_mac_border()` + `common_extend_border()`.
///
/// Shogun's Amiga and Mac B/W border is a **single image with no separate
/// pillars**, so Bocfel extends it by stamping a second copy of *whichever
/// border is in use* below the first: flipped for `P_BORDER` (3) at
/// `border_height - 2`, plain for `P_BORDER2` (4) at `border_height - 22`
/// (Mac B/W: `border_height` and `border_height - 35`). `P_BORDER` and
/// `P_BORDER2` are two alternative border STYLES for different scenes, not two
/// halves of one border. The flip is not decoration — it is what makes the join
/// read as continuous on a border whose top and bottom differ.
///
/// We cannot see which of the two styles the game chose (the canvas is pixels,
/// not picture numbers), and we do not need to: the flipped `P_BORDER` join is
/// the one that reads continuously for either, and a 2-row overlap is small
/// enough to be invisible if the style was the other one. Bocfel then runs
/// `common_extend_border()`, which repeats everything drawn so far downward
/// until the pane is filled — that is the third step here.
///
/// **Both repeats are cut from `art`, never from `dst`.** This is the ORDER
/// Bocfel works in and it is load-bearing. Its covering rectangle over the
/// status bar — *"the raw border graphics have a decorative top bar that doesn't
/// fit the header text, so the original interpreters cover it with a solid color
/// rectangle"* — is step 4 of `draw_border_common`, drawn AFTER the extension,
/// so the copies it stamps are of the unmasked picture. lanthorn's chrome canvas
/// is the same art with that band already gone: Shogun's status line is two
/// 16px rows the renderer draws as crisp terminal cells (SQ-0500), so the top
/// **32 native rows** of the flank are transparent there while the
/// graphics-only canvas still carries them. Cutting the repeat from the chrome
/// canvas therefore copies a 32-row hole twice — once at the flipped copy's
/// FOOT (its source rows 32..0) and once at the tiled block's HEAD — and the two
/// meet at the join, which is the 64-native-row (94px at a 120x90 terminal) black
/// band between the panels the user reported. Reading `border_height` off the
/// art while reading the PIXELS off the chrome canvas was the mismatch; the
/// gap's size is twice the cleared status band, not the art's unpainted tail,
/// and it sits centred on the join at `2·border_height − overlap` rather than
/// at the art's own bottom. SQ-0698.
///
/// `dst`'s own first copy keeps the chrome canvas's clearing, because that band
/// is where the status CELLS go; the flank crop starts below it in every pane we
/// have measured, so it is only ever the rows the extension adds that need the
/// art.
///
/// Measured on `James Clavell's Shogun.adf` (release 295, serial 890321): the
/// border art is native rows 0..336 across flank columns 0..46 and 594..640, so
/// `border_height` = 336 = the 168-row raw image doubled.
fn shogun(dst: &mut RgbaImage, art: &RgbaImage, border_height: u32, desired_height: u32) {
    /// Bocfel's Amiga offset for `P_BORDER` is 2 raw lines; unit space doubles it.
    const OVERLAP: u32 = 4;
    if border_height == 0 || desired_height <= border_height {
        return;
    }
    let whole = snapshot(art, 0, border_height);
    let second_top = border_height - OVERLAP;
    stamp(dst, &whole, second_top, true);
    let lowest = second_top + border_height;
    if desired_height <= lowest {
        return;
    }
    // `common_extend_border(desired_height, lowest_drawn_pixel, start_copy_from)`
    // — repeat everything from `pillar_top` (0, for Shogun) down to the lowest
    // drawn line, truncating the last copy at the bottom. Composed here from the
    // art rather than snapshotted out of `dst`, for the reason above; the two
    // stamps reproduce rows `[0, lowest)` exactly as the steps above laid them.
    let mut block = RgbaImage::new(dst.width(), lowest);
    stamp(&mut block, &whole, 0, false);
    stamp(&mut block, &whole, second_top, true);
    tile_down(dst, &block, lowest, desired_height, 0, false, false);
}

/// The plain **shaft** of a pillar — `(top, bottom_exclusive)` — as the art
/// itself declares it, or `None` when this flank shows no pillar shape.
///
/// A pillar is a capital, a shaft and a base, and the shaft is the only part of
/// it that repeats. What separates the three in the pixels is WIDTH: the capital
/// and the base flare out, and the shaft between them holds one span for
/// hundreds of rows. So the shaft is the longest run of consecutive rows whose
/// opaque column span (first and last painted column) is identical, and it
/// counts as a pillar only when
///
/// * that span is strictly NARROWER than the flank's widest painted row — there
///   is a capital or a base wider than it, which is what makes the shape a
///   pillar rather than a slab;
/// * something is painted above it and something below it, so there is a
///   capital to cut beneath and a base to stamp back on; and
/// * it is **most of the flank**. A pillar is mostly shaft — the capital and the
///   base are the minority — so a run that holds for less than half the art is
///   not a shaft, it is a coincidence in a textured surface. See below.
///
/// Rows are compared by span rather than by pixel count on purpose: the CGA
/// rendition dithers its masonry, so the number of opaque pixels in a shaft row
/// wobbles from row to row while its edges do not move at all.
///
/// ## The majority test is SQ-0792, and it is what keeps a border SYMMETRIC
///
/// Zork Zero has three scene borders and the derivation above is right for
/// exactly one of them. Measured over the four native archives, with each scene's
/// flanks composed from the archive's own pictures the way `DISPLAY_BORDER` draws
/// them — top strip (5 castle, 7 underground, 6 jungle) at `(0,0)`, then the left
/// pillar `0x1f1`/`0x1f3`/`0x1f5` and the right `0x1f2`/`0x1f4`/`0x1f6` at
/// `y = strip height`, at the flank width one `TEXT_WINDOW_PIC_LOC` picture fixes
/// for all three (86 px; 88 on `zork0.pic`) — the longest constant-span run is:
///
/// | scene       | `zork0.mg1` | `zork0.eg1` | `zork0.cg1` | `zork0.pic` |
/// |-------------|-------------|-------------|-------------|-------------|
/// | castle      | 292 / 292   | 292 / 290   | 280 / 280   | 292 / 292   |
/// | underground | 54 / 146    | 44 / 180    | 54 / 72     | 54 / 76     |
/// | jungle      | 14 / 30     | 12 / 30     | 16 / 14     | 14 / 30     |
///
/// (left flank / right flank, of 400 rows. The composition is trustworthy
/// because the castle reproduces the IN-GAME shaft to the row — `[82, 374)` on
/// `zork0.mg1`, `[102, 382)` on `zork0.cg1`.)
///
/// The castle holds one span for **70–73%** of the flank on every rendition and
/// both flanks. The underground is alternating stone blocks and the jungle is
/// foliage: neither holds one width for long, and the longest run this would
/// otherwise have ACCEPTED is 146 rows — **36%**. So the two families are
/// separated by the gap 36%..70%, and the cut is at **half the flank**, which is
/// in it with margin at both ends and needs no fitting: it is the definition of a
/// pillar, not a number tuned to this corpus.
///
/// **What went wrong without it is worse than a mis-cut, because the two flanks
/// disagreed with each other.** A border is symmetric by construction — one pair
/// of pillar pictures drawn at one `y` — but this runs per flank, so on
/// `zork0.cg1` underground the left flank cut at row 78 and the right at row 296,
/// and on `zork0.mg1` jungle the left derived a 14-row repeat unit while the
/// right fell back to the castle's 284. Six of the eight non-castle flank PAIRS
/// measured got different recipes from each other. With the majority test every
/// castle flank still derives and every other flank falls back, so the underground
/// and the jungle get the castle constants uniformly — which is what SQ-0792
/// predicted in the first place, and which the mirror of SQ-0808 then makes
/// seamless without knowing which scene is on screen.
///
/// Only rows `[0, art_bottom)` are considered — below that is the caller's
/// extension, not the game's art.
pub fn pillar_shaft(dst: &RgbaImage, art_bottom: u32) -> Option<(u32, u32)> {
    let w = dst.width();
    let span = |y: u32| -> Option<(u32, u32)> {
        let mut first = None;
        let mut last = 0;
        for x in 0..w {
            if dst.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        first.map(|f| (f, last))
    };
    let rows: Vec<Option<(u32, u32)>> = (0..art_bottom.min(dst.height())).map(span).collect();
    let widest = rows.iter().flatten().map(|(f, l)| l - f + 1).max()?;
    let (mut best, mut cur) = ((0u32, 0u32), (0u32, 0u32));
    for (y, s) in rows.iter().enumerate() {
        let y = y as u32;
        if y > 0 && *s == rows[y as usize - 1] {
            cur.1 = y + 1;
        } else {
            cur = (y, y + 1);
        }
        if s.is_some() && cur.1 - cur.0 > best.1 - best.0 {
            best = cur;
        }
    }
    let (top, bottom) = best;
    let (first, last) = rows.get(top as usize).copied().flatten()?;
    // …and the run must be MOST of the flank (SQ-0792) — see the table above.
    if last - first + 1 >= widest || top == 0 || bottom >= art_bottom || (bottom - top) * 2 < art_bottom {
        return None;
    }
    Some((top, bottom))
}

/// The **banded** shaft of a pillar — `(top, bottom_exclusive)` — for a column
/// whose shaft is not one constant span because a decorative band interrupts it.
/// SQ-0841.
///
/// [`pillar_shaft`] answers for a shaft that holds ONE opaque span for most of
/// the flank, which is every Zork Zero rendition Infocom shipped for a PC or an
/// Amiga. The standard Macintosh's monochrome plate is not one of them: measured
/// on `stories/Zork Zero Disk.image` (**Zork Zero r296 / s881019**) with
/// `--pictures Pic.data`, at a gameplay frame, both flanks carry a ring in the
/// middle of an otherwise plain shaft —
///
/// | flank | capital | shaft         | ring       | foot     |
/// |-------|---------|---------------|------------|----------|
/// | left  | 0..63   | 63..285 (8..45) | 163..173 (±1 col) | 285..300 |
/// | right | 0..63   | 63..286 (17..53)| 164..173 (±1 col) | 286..300 |
///
/// — so the longest CONSTANT run is only 112 of the flank's 300 rows (37%) and
/// the majority test rejects it, correctly: by that measurement this is not a
/// plain shaft. It is still a shaft, and this says so by scanning for the
/// longest run of rows whose span stays within **one column at each edge** of
/// some reference row's. The ring deviates by exactly one column, which is why
/// one column is the tolerance and not two: at two, `zork0.mg1`'s underground
/// masonry starts declaring shafts again and the flanks stop agreeing, which is
/// the failure SQ-0792 removed.
///
/// The reference is scanned over every row rather than taken from the run's
/// first, because a run anchored to its first row breaks at the ring on the
/// RIGHT flank (its shaft is 17..53, its ring 18..52, and its own taper row is
/// 16..54 — two columns apart from the ring, one from the shaft). Anchoring to
/// the shaft itself finds all 225 rows; anchoring to whatever happens to come
/// first finds 102 and 122 and neither is a majority. Same gates as
/// [`pillar_shaft`] otherwise, majority included.
pub fn banded_shaft(dst: &RgbaImage, art_bottom: u32) -> Option<(u32, u32)> {
    let w = dst.width();
    let span = |y: u32| -> Option<(u32, u32)> {
        let mut first = None;
        let mut last = 0;
        for x in 0..w {
            if dst.get_pixel(x, y)[3] >= 128 {
                first.get_or_insert(x);
                last = x;
            }
        }
        first.map(|f| (f, last))
    };
    let rows: Vec<Option<(u32, u32)>> = (0..art_bottom.min(dst.height())).map(span).collect();
    let widest = rows.iter().flatten().map(|(f, l)| l - f + 1).max()?;
    /// One column at each edge — the ring's own deviation, and no more.
    const SLACK: u32 = 1;
    let near = |s: Option<(u32, u32)>, r: (u32, u32)| {
        matches!(s, Some((f, l)) if f.abs_diff(r.0) <= SLACK && l.abs_diff(r.1) <= SLACK)
    };
    let mut best: Option<(u32, u32, (u32, u32))> = None;
    for (i, r) in rows.iter().enumerate() {
        let Some(reference) = *r else { continue };
        let mut top = i;
        while top > 0 && near(rows[top - 1], reference) {
            top -= 1;
        }
        let mut bottom = i + 1;
        while bottom < rows.len() && near(rows[bottom], reference) {
            bottom += 1;
        }
        let (top, bottom) = (top as u32, bottom as u32);
        if best.is_none_or(|(t, b, _)| bottom - top > b - t) {
            best = Some((top, bottom, reference));
        }
    }
    let (top, bottom, (first, last)) = best?;
    if last - first + 1 >= widest || top == 0 || bottom >= art_bottom || (bottom - top) * 2 < art_bottom {
        return None;
    }
    Some((top, bottom))
}

/// **A banded pillar, repeated at a uniform stride with its foot flush at the
/// bottom** — SQ-0841, and the composition [`extend_pillars`] cannot express.
///
/// [`extend_pillars`] cuts a plain length of shaft, tiles it at a fixed stride
/// until it passes the bottom, and stamps the foot over whatever the last tile
/// overshot. On a featureless shaft that is invisible and it is what Bocfel
/// does. On a shaft with a band in it, two things show:
///
/// 1. **The remainder.** The run ends wherever the fixed stride happens to
///    reach, so the gap between the last band and the foot is whatever is left
///    over — never the gap between two bands.
/// 2. **The mirror.** [`extend_pillars`] alternates each tile's vertical flip
///    (SQ-0808) because a duplicated row hides a seam a translation cannot. On a
///    plain shaft a mirror is indistinguishable from a translation; on a banded
///    one it MOVES the band, by twice its offset from the unit's centre.
///
/// So a banded column is composed the other way round: the repeat unit is the
/// whole of the pillar BELOW its capital — shaft, band and foot together — and
/// `k` further copies are laid at a stride that divides the extension exactly,
/// so the last copy's foot lands on the bottom row and every band is one stride
/// from the next. Each copy overwrites the one above it from its own top down,
/// so only the last copy's foot survives; the rest contribute shaft and bands.
///
/// The rhythm this keeps is the ART's own, at every pane height: the capital-to
/// first-band distance and the last-band-to-foot distance are both exactly what
/// the picture was drawn with, because both come from an unmodified copy of it.
///
/// `top_cut` is where the unit is cut (just below the capital), `art_bottom` the
/// art's own last painted row, `desired_height` the band to fill. `k` is derived
/// from the span, so a taller pane gets MORE bands rather than longer ones.
pub fn extend_banded_pillars(dst: &mut RgbaImage, top_cut: u32, art_bottom: u32, desired_height: u32) {
    if art_bottom <= top_cut || desired_height <= art_bottom {
        return;
    }
    let unit_h = art_bottom - top_cut;
    // What the copies below the first have to cover. `k` is the fewest of them
    // that can, so the stride is as long as the art allows and never longer.
    let extra = desired_height - art_bottom;
    let k = extra.div_ceil(unit_h);
    let unit = snapshot(dst, top_cut, unit_h);
    for i in 1..=k {
        stamp(dst, &unit, top_cut + (i * extra) / k, false);
    }
    // Copy `k` sits at `top_cut + extra`, so its foot ends exactly on
    // `desired_height`. Nothing may follow it.
    erase_below(dst, desired_height);
}

/// **Zork Zero** — `extend_pillars(43, 13, 200, 142, 0, false, false)`, the
/// CASTLE border on the VGA / Amiga / Blorb artwork (`zorkzero.cpp`), with
/// every one of those four numbers **measured off the art** instead.
///
/// Bocfel can hard-code them because it only ever draws one rendition per run,
/// chosen by display mode; lanthorn lets the player switch archives, and Zork
/// Zero's renditions do not agree on where the pillars begin. The banner above
/// them is 34 raw rows on MCGA, **37 on EGA and 39 on CGA**, while the pillars
/// are 166 rows in all three — so a cut pinned to 86 in unit space lands 6 rows
/// high under EGA and 10 under CGA, inside the ring beneath the capital, and
/// every tile boundary then repeats that ring as a horizontal seam. SQ-0799.
///
/// What is measured is [`pillar_shaft`], and the four constants fall out of it:
///
/// | Bocfel (raw)            | derived from the shaft `[top, bottom)`      |
/// |-------------------------|---------------------------------------------|
/// | `place_to_cut` = 43     | `top + INSET`                               |
/// | `foot_height` = 13      | `art_bottom - bottom`                       |
/// | `total_height` = 200    | `art_bottom` (the caller already measured)  |
/// | `pillar_height` = 142   | `bottom - top - 2·INSET`                    |
///
/// **`INSET` is the one number that cannot be derived**, and it is 2 raw rows —
/// Bocfel keeps its repeat unit that far clear of the capital above and the base
/// below, which is the whole of the difference between its constants and the
/// shaft this measures. On the MCGA/Amiga/Blorb art the shaft is unit rows
/// `[82, 374)` of a 400-row flank, and the table above returns **86 / 26 / 400 /
/// 284** — Bocfel's four constants exactly, which is what makes this a
/// derivation of them rather than a replacement for them.
///
/// When the flank shows no pillar shape at all, `pillar_shaft` says so and those
/// constants are used as written. Every *castle* flank measured declares a shaft
/// and no other Zork Zero flank does, so that arm is exactly its other two scene
/// borders (below) — see [`pillar_shaft`] for the measurement that separates
/// them, and for why letting a scene border derive its own recipe was worse than
/// this fallback rather than better (SQ-0792).
///
/// ## Alternate tiles are MIRRORED, and that is SQ-0808
///
/// `flip` is on, which makes every internal join an exact duplicated row: tile
/// *n* ends on strip row 0 and tile *n+1* — drawn the other way up — begins on
/// strip row 0. A translation repeat instead butts the strip's LAST row against
/// its FIRST, and that only disappears when the shaft is uniform down its length.
///
/// Two of Zork Zero's three renditions are uniform and one is not. Mean row
/// luminance down the castle shaft, measured on `zork0-r393-s890714.z6` at a
/// gameplay frame: `zork0.mg1` and `zork0.eg1` hold **54** and **51** from top to
/// bottom, while `zork0.cg1` runs **97 → 82** — its pillar is a *lit* column,
/// shaded darker toward its base, and repeating it reset the shading at every
/// join. Against the art's own steepest internal step of 16.8 (a 16-row
/// low-pass), the extension's join measured **29.3** at band row 654, the second
/// tile boundary. That is the seam the user reported, and it is neither the EGA
/// dither of SQ-0797 nor the mis-cut repeat unit of SQ-0799 — the cut is in the
/// plain shaft on all three renditions and CGA still seams, because the shaft
/// itself is not translation-invariant.
///
/// Bocfel reached the same conclusion per platform and hard-codes it:
/// `extend_pillars(52, 25, 205, 133, 11, /*flip=*/true, false)` for CGA against
/// `(43, 13, 200, 142, 0, false, false)` for VGA/Amiga/Blorb, and it forces the
/// first EGA tile flipped too (`if (is_spatterlight_zork0 && graphics_type ==
/// kGraphicsTypeEGA) initial_parity = true;`). Mirroring unconditionally is the
/// same answer without the dispatch: on a uniform shaft a mirror is
/// indistinguishable from a translation, so the two flat renditions are
/// unaffected, and it needs none of Bocfel's per-platform overlap (11 raw rows on
/// CGA, 9 on EGA) because a duplicated row has nothing left to hide.
///
/// ## Known limitation, deliberate
///
/// Zork Zero has three scene borders and Bocfel gives each its own routine:
/// castle (above), `extend_underground_pillars(73, 54, 200, 74, 38, 37)` and
/// `extend_jungle_pillars(67, 210, 143, 59)`. Which one is on screen is a
/// Z-machine global Bocfel reads and we cannot — `WinNode::Graphics` carries a
/// flattened `RgbaImage`, and picture numbers do not survive the engine boundary.
/// SQ-0792 settled how much of that dispatch the pixels can replace:
///
/// * **Castle against not-castle: derivable, and now derived.** The castle is the
///   only one of the three that holds one span for most of the flank, so
///   [`pillar_shaft`]'s majority test tells them apart on every rendition, and the
///   other two take the constants above — uniformly on BOTH flanks, which is the
///   part that had regressed.
/// * **The jungle's own overlap: moot.** `extend_jungle_pillars` is a plain
///   59-row-overlap repeat with no foot, and the overlap exists to hide the seam
///   that SQ-0808's mirror removes outright. Its `total_height` of 210 raw rows
///   is TALLER than the 200-row screen, so the rows it reads below 200 are
///   clipped from the canvas before we receive it; Bocfel sees them only because
///   its pixmap is unclipped until `flush_bitmap`.
/// * **The underground's stone alternation: genuinely blocked.** Bocfel does not
///   flip that tile, it SWAPS the flanks — on alternate tiles it draws the right
///   pillar's 37-px-wide stones at `x = 0` and the left pillar's at
///   `x = width − 37`, so the two columns trade courses and the masonry stays
///   consistent between them. That is a statement about the pair, applied only to
///   the underground; a mirror inside one flank cannot express it, and applying
///   it to the castle would flip a lit column's shading. Nothing in the pixels
///   says "this is the underground".
///
/// ## Autocorrelation was tried, and the castle is why it cannot replace this
///
/// SQ-0813 asked whether the repeat unit could come from the art alone —
/// autocorrelate a flank down y, take the strongest period as the tile height —
/// which would retire the dispatch above and subsume the constant-span case,
/// since a constant shaft autocorrelates at every lag. Measured over every
/// rendition and all three scene borders it reproduces the castle's derivation on
/// none of them, and the reason is structural: **a pillar shaft has no period.**
/// `zork0.mg1`'s is uniform — its rows are pixel-identical, mean mismatch 0.30 of
/// 255 at every lag from 4 to 20 alike — so the search is maximally confident and
/// maximally uninformative and returns the smallest lag it is offered, 4 against
/// the 284 [`pillar_shaft`] gives. `zork0.cg1`'s is a graded lit column (97 → 82,
/// above) and a gradient is not periodic either, so its best lag scores *worse*
/// than the average lag. Driven through this handler the per-pixel form returns 4
/// on MCGA, Amiga and the Blorb and nothing at all on EGA and CGA.
///
/// The two scenes it was meant to rescue score no better: the underground's course
/// is found at 74 rows at a confidence of 0.75–0.97 where 1.0 is a coin toss, and
/// on `zork0.cg1` the two flanks disagree (76 left, 74 right) — the asymmetry
/// SQ-0792 removed. Accepts and rejects interleave with no gap between them, where
/// SQ-0792's cut had 36%..70%. The statistic measures self-similarity and a plain
/// shaft is more self-similar than patterned masonry, so it is anti-correlated
/// with what it was asked to detect. It also yields only the unit, never the
/// `top_cut` and `foot` that [`extend_pillars`] needs and the shape measurement
/// supplies. `v6_side_border_tiling.rs` pins the corpus numbers so a future
/// statistic that does separate them says so.
fn zork_zero(dst: &mut RgbaImage, art_bottom: u32, desired_height: u32) {
    /// How far Bocfel's repeat unit stays clear of the capital above it and the
    /// base below it: 2 raw rows at each end, doubled into unit space.
    const INSET: u32 = 4;
    /// `extend_pillars(top_cut=43, …)` doubled.
    const TOP_CUT: u32 = 86;
    /// `foot_height=13` doubled.
    const FOOT: u32 = 26;
    /// `pillar_height=142` doubled.
    const UNIT: u32 = 284;
    let (top_cut, foot, unit) = match pillar_shaft(dst, art_bottom) {
        Some((top, bottom)) => (top + INSET, art_bottom - bottom, (bottom - top).saturating_sub(2 * INSET)),
        // A shaft with a BAND in it is not a plain one, and repeating a plain
        // length of it is what left bare shaft below the Macintosh column's
        // feet. SQ-0841 — see [`extend_banded_pillars`].
        None => match banded_shaft(dst, art_bottom) {
            Some((top, _)) => return extend_banded_pillars(dst, top + INSET, art_bottom, desired_height),
            None => (TOP_CUT, FOOT, UNIT),
        },
    };
    if unit == 0 || foot == 0 || art_bottom <= top_cut + foot {
        return;
    }
    let unit = unit.min(art_bottom - foot - top_cut);
    extend_pillars(dst, top_cut, foot, art_bottom, unit, 0, true, desired_height);
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// The opaque row extent `(first, last_exclusive)` of native columns
/// `[x0, x1)` of `canvas`. `(0, 0)` when nothing there is painted.
pub fn art_extent(canvas: &RgbaImage, x0: u32, x1: u32) -> (u32, u32) {
    let x1 = x1.min(canvas.width());
    let mut first = None;
    let mut last = 0;
    for y in 0..canvas.height() {
        if (x0..x1).any(|x| canvas.get_pixel(x, y)[3] >= 128) {
            first.get_or_insert(y);
            last = y + 1;
        }
    }
    match first {
        Some(f) => (f, last),
        None => (0, 0),
    }
}

/// Build the native-space source image for ONE side flank band: columns
/// `[x0, x1)` of `canvas`, rows `[crop_top, crop_top + rows)`, with this title's
/// border art extended downward so the whole band is painted.
///
/// `art` is the flank's opaque extent as [`art_extent`] reports it over the
/// SAME columns — measured on the graphics-only canvas `gfx`, so a status run
/// rasterised into `canvas` cannot be mistaken for border art.
///
/// `gfx` is that graphics-only canvas itself, and it is not merely the
/// classifier's input: `canvas` is the artwork MINUS whatever the renderer draws
/// as terminal cells instead, so a handler that repeats a unit cut from
/// `canvas` repeats the holes those cells left. Only [`shogun`] needs it today
/// (see there) and only [`shogun`] is given it, so the other two are byte-for-byte
/// what they were. That is not luck, and the suite measures it rather than
/// assuming it: Zork Zero's status sits ON its banner art so nothing is cleared
/// from its flank at all, and Arthur's repeat unit is cut at 90% of his poles'
/// own height, far below the status row between his banner and the story.
///
/// `None` when the flank shows no recognised border art, or when the art
/// already covers the band — the caller then keeps whatever it did before.
pub fn flank_source(
    canvas: &RgbaImage,
    gfx: &RgbaImage,
    x0: u32,
    x1: u32,
    art: (u32, u32),
    native_h: u32,
    crop_top: u32,
    rows: u32,
) -> Option<RgbaImage> {
    let kind = recognize(canvas, x0, x1, art, native_h)?;
    let desired = crop_top + rows;
    // Bocfel guards every one of these routines the same way: extend only when
    // the pane is taller than the art (`if (desired_height <= total_height) return;`).
    if desired <= art.1 || rows == 0 || x1 <= x0 {
        return None;
    }
    // Work in ABSOLUTE canvas rows so each title's constants read exactly as
    // they do in the reference, then hand the caller the band's own window.
    let w = x1.min(canvas.width()).saturating_sub(x0);
    if w == 0 {
        return None;
    }
    let mut strip = RgbaImage::new(w, desired);
    for y in 0..native_h.min(canvas.height()).min(desired) {
        for x in 0..w {
            strip.put_pixel(x, y, *canvas.get_pixel(x0 + x, y));
        }
    }
    match kind {
        BorderArt::ArthurPoles => arthur(&mut strip, art.1, desired),
        BorderArt::ShogunSinglePiece => {
            let mut art_strip = RgbaImage::new(w, art.1);
            for y in 0..art.1.min(gfx.height()) {
                for x in 0..w.min(gfx.width().saturating_sub(x0)) {
                    art_strip.put_pixel(x, y, *gfx.get_pixel(x0 + x, y));
                }
            }
            shogun(&mut strip, &art_strip, art.1, desired);
        }
        BorderArt::ZorkZeroPillars => zork_zero(&mut strip, art.1, desired),
    }
    Some(snapshot(&strip, crop_top, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid `w x h` image of one colour, for shape assertions.
    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    /// A flank `w` wide whose rows `[top, bottom)` are painted, `narrow` columns
    /// wide below `waist` and the full width above it — the banner-over-pillar
    /// shape, or a constant-width slab when `narrow == w`.
    fn flank(w: u32, top: u32, bottom: u32, waist: u32, narrow: u32) -> RgbaImage {
        let mut c = RgbaImage::new(w, 400);
        for y in top..bottom {
            let n = if y < waist { w } else { narrow };
            for x in 0..n {
                c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        c
    }

    #[test]
    fn recognize_separates_the_measured_shapes() {
        // Arthur adf r54: poles native 11..379 of 400.
        let a = flank(28, 11, 379, 11, 22);
        assert_eq!(recognize(&a, 0, 28, (11, 379), 400), Some(BorderArt::ArthurPoles));
        // Shogun adf r295: single-piece border native 0..336 of 400.
        let s = flank(46, 0, 336, 0, 46);
        assert_eq!(recognize(&s, 0, 46, (0, 336), 400), Some(BorderArt::ShogunSinglePiece));
        // Zork Zero r393: pillars painted to the native bottom, narrowing from an
        // 86-wide banner to a 48-wide shaft (ratio 0.56).
        let z = flank(86, 0, 400, 68, 48);
        assert_eq!(recognize(&z, 0, 86, (0, 400), 400), Some(BorderArt::ZorkZeroPillars));
        // An unpainted flank is nobody's border.
        assert_eq!(recognize(&z, 0, 86, (0, 0), 400), None);
    }

    /// SQ-0802 — Shogun's DOS art reaches the native screen bottom, so "reaches
    /// the bottom" alone handed it Zork Zero's masonry recipe.
    ///
    /// Falsifiable: drop the width test from [`recognize`] and every case here
    /// comes back `ZorkZeroPillars`.
    #[test]
    fn a_slab_that_reaches_the_bottom_is_not_a_pillar() {
        // shogun.mg1 / .eg1 / Shogun.blb: 46-wide, ratio 1.00.
        let s = flank(46, 0, 400, 0, 46);
        assert_eq!(recognize(&s, 0, 46, (0, 400), 400), Some(BorderArt::ShogunSinglePiece));
        // shogun.cg1's right flank: 57 wide, narrowest painted row 55 — ratio
        // 0.96, the tightest slab measured, and still not a pillar.
        let c = flank(57, 0, 400, 200, 55);
        assert_eq!(recognize(&c, 0, 57, (0, 400), 400), Some(BorderArt::ShogunSinglePiece));
        // …while Zork Zero's widest-waisted flank — the underground border at
        // 70/86, ratio 0.81 — is still pillars. The cut is at 9/10, in the gap.
        let u = flank(86, 0, 400, 78, 70);
        assert_eq!(recognize(&u, 0, 86, (0, 400), 400), Some(BorderArt::ZorkZeroPillars));
    }

    /// A flank on a screen of a stated height: rows `[top, bottom)` painted,
    /// `wide` columns above `waist` and `narrow` below it.
    ///
    /// [`flank`]'s 400-row screen cannot express the standard Macintosh's
    /// monochrome one, which is 304 — and 304 is where every case below lives.
    fn flank_on(w: u32, h: u32, top: u32, bottom: u32, waist: u32, wide: u32, narrow: u32) -> RgbaImage {
        let mut c = RgbaImage::new(w, h);
        for y in top..bottom.min(h) {
            for x in 0..(if y < waist { wide } else { narrow }).min(w) {
                c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
            }
        }
        c
    }

    /// SQ-0881 — an INSET plate that all but reaches the bottom is not a pillar.
    ///
    /// SQ-0841 loosened "reaches the bottom" to one text row so the standard
    /// Macintosh's 480x300 art would still count on its 304-row screen. Arthur's
    /// monochrome plate is inset fifteen rows above that same bottom — one row
    /// inside the tolerance — and narrows like a pillar, so both of the older
    /// measurements agreed on Zork Zero and the player got Zork Zero's capital
    /// stamped down Arthur's side. The art's TOP is what tells them apart.
    ///
    /// Every number here was measured in-game through the pty harness at 165x50,
    /// on the frame each title draws; see [`recognize`]'s table.
    ///
    /// Falsifiable: drop `top == 0` from the pillar branch and the first case
    /// comes back `ZorkZeroPillars`.
    #[test]
    fn an_inset_plate_that_all_but_reaches_the_bottom_is_not_a_pillar() {
        // Arthur, `MAC/ARTHUR FOLDER/PIC.DATA`: art 14..289 of 304, a 16-wide
        // banner over a 5-wide pole. Fifteen short of 304, ratio 0.31 — inside
        // BOTH older tests, and outside this one.
        let a = flank_on(21, 304, 14, 289, 130, 16, 5);
        assert_eq!(recognize(&a, 0, 21, (14, 289), 304), Some(BorderArt::ArthurPoles));
        // Zork Zero, `Pic.data` on the same 304-row screen: SQ-0841's case, and
        // still a pillar — inset 4, the cell rounding and nothing else.
        let z = flank_on(62, 304, 0, 300, 60, 62, 35);
        assert_eq!(recognize(&z, 0, 62, (0, 300), 304), Some(BorderArt::ZorkZeroPillars));
        // `Zork Zero - The Revenge of Megaboz.adf` plate 6's RIGHT flank starts
        // at row 2 where its left starts at 0 — two blank rows in the drawing.
        // A `top == 0` rule would classify one plate's two crops differently,
        // which `v6_archive_border_sweep` forbids and caught.
        let r = flank_on(86, 400, 2, 400, 68, 86, 48);
        assert_eq!(recognize(&r, 0, 86, (2, 400), 400), Some(BorderArt::ZorkZeroPillars));
        // Arthur's DOS press clears the bottom test by exactly one row —
        // 384 + 16 is 400, which is not GREATER than 400 — so it was never
        // misread, and must not start being read differently now.
        let d = flank_on(31, 400, 16, 384, 130, 25, 4);
        assert_eq!(recognize(&d, 0, 31, (16, 384), 400), Some(BorderArt::ArthurPoles));
    }

    #[test]
    fn tile_down_strides_by_height_less_overlap() {
        let mut dst = RgbaImage::new(1, 40);
        let strip = solid(1, 10, [1, 2, 3, 255]);
        // No overlap: stamps at 0, 10, 20, 30 (and 40 is past `end_y`).
        let next = tile_down(&mut dst, &strip, 0, 30, 0, false, false);
        assert_eq!(next, 40, "the next tile would have started at 40");
        assert!((0..40).all(|y| dst.get_pixel(0, y)[3] == 255), "every row painted");
        // Overlap 4 → stride 6.
        let mut dst = RgbaImage::new(1, 40);
        let next = tile_down(&mut dst, &strip, 0, 12, 4, false, false);
        assert_eq!(next, 18, "0, 6, 12 then past the end");
        assert_eq!(next - 12, 6, "stride is height - overlap");
    }

    #[test]
    fn tile_down_alternates_the_flip_only_when_asked() {
        // A strip whose two halves differ, so a flip is visible.
        let mut strip = RgbaImage::new(1, 2);
        strip.put_pixel(0, 0, Rgba([10, 0, 0, 255]));
        strip.put_pixel(0, 1, Rgba([20, 0, 0, 255]));
        let mut dst = RgbaImage::new(1, 4);
        tile_down(&mut dst, &strip, 0, 2, 0, true, true);
        // First tile flipped (initial_parity = true), second unflipped.
        assert_eq!(dst.get_pixel(0, 0)[0], 20);
        assert_eq!(dst.get_pixel(0, 1)[0], 10);
        assert_eq!(dst.get_pixel(0, 2)[0], 10);
        assert_eq!(dst.get_pixel(0, 3)[0], 20);
        // …and with `flip = false` every tile keeps the initial parity.
        let mut dst = RgbaImage::new(1, 4);
        tile_down(&mut dst, &strip, 0, 2, 0, false, false);
        assert_eq!(dst.get_pixel(0, 0)[0], 10);
        assert_eq!(dst.get_pixel(0, 2)[0], 10);
    }

    /// The ordering caveat Bocfel documents: Arthur's `top_cut` sits INSIDE the
    /// foot region erased just below it, so a routine that erases before it
    /// snapshots tiles a blank strip. Falsifiable: a 1-px-wide pole whose
    /// texture is only in its bottom 10%.
    #[test]
    fn extend_pillars_snapshots_the_unit_before_erasing_the_foot() {
        let mut dst = RgbaImage::new(1, 200);
        for y in 0..100 {
            dst.put_pixel(0, y, Rgba([7, 7, 7, 255]));
        }
        // total 100, cut at 90 → the unit's rows ARE the foot's rows.
        extend_pillars(&mut dst, 90, 10, 100, 4, 0, false, 180);
        assert!(
            (90..180).all(|y| dst.get_pixel(0, y)[3] == 255),
            "the shaft between the cut and the foot is painted, not blank"
        );
        assert!(dst.get_pixel(0, 179)[3] == 255, "the foot reaches the bottom");
    }

    #[test]
    fn nothing_is_extended_when_the_art_already_covers_the_band() {
        let canvas = solid(64, 400, [9, 9, 9, 255]);
        assert!(
            flank_source(&canvas, &canvas, 0, 32, (0, 400), 400, 0, 400).is_none(),
            "a band no taller than the art needs no extension"
        );
    }

    #[test]
    fn an_extended_flank_is_painted_to_its_last_row() {
        // Shogun's shape: art to row 336 of a 400-row screen, band wants 700.
        let mut canvas = RgbaImage::new(64, 400);
        for y in 0..336 {
            for x in 0..64 {
                canvas.put_pixel(x, y, Rgba([(y % 251) as u8, 4, 5, 255]));
            }
        }
        let out = flank_source(&canvas, &canvas, 0, 32, (0, 336), 400, 30, 670).expect("extended");
        assert_eq!((out.width(), out.height()), (32, 670));
        for y in 0..out.height() {
            assert!(out.get_pixel(0, y)[3] == 255, "row {y} of the band is painted");
        }
    }

    /// SQ-0698 — **the gap between Shogun's tiled panels**, reported as *"there
    /// is a gap between the tiled shogun side-art pieces"*.
    ///
    /// The chrome canvas is the artwork MINUS whatever the renderer draws as
    /// terminal cells: Shogun's two-row status line is 32 native pixels the top
    /// of its border sits behind, so those rows are cleared there while the
    /// graphics canvas still carries them. Repeating a unit cut from the chrome
    /// canvas copies that hole twice — the flipped copy's foot and the tiled
    /// block's head are both the missing rows — and they meet at the join.
    ///
    /// Measured on `James Clavell's Shogun.adf` (release 295, serial 890321) at
    /// a 120x90 terminal: 64 transparent native rows centred on native row 668
    /// (`2·336 − 4`), which the uniform scale of 1.475 put on screen as a 94px
    /// black band. Falsifiable: cut the two repeats from `dst` again and this
    /// fails with exactly 64 blank rows in the same place.
    #[test]
    fn shoguns_repeats_come_from_the_art_not_the_status_cleared_canvas() {
        const H: u32 = 336;
        const CLEARED: u32 = 32;
        let mut gfx = RgbaImage::new(64, 400);
        for y in 0..H {
            for x in 0..64 {
                gfx.put_pixel(x, y, Rgba([(y % 251) as u8, 4, 5, 255]));
            }
        }
        // …and the chrome canvas the band actually ships, with the status band
        // gone from the top of the flank.
        let mut canvas = gfx.clone();
        for y in 0..CLEARED {
            for x in 0..64 {
                canvas.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
        // A crop that starts below the cleared band, as every measured pane does.
        let out = flank_source(&canvas, &gfx, 0, 32, (0, H), 400, 37, 1025).expect("extended");
        let blank: Vec<u32> =
            (0..out.height()).filter(|&y| (0..out.width()).all(|x| out.get_pixel(x, y)[3] == 0)).collect();
        assert!(
            blank.is_empty(),
            "the extended flank has {} transparent row(s) — first at {:?}, native {:?}; \
             the join sits at native {}",
            blank.len(),
            blank.first(),
            blank.first().map(|y| y + 37),
            2 * H - 4
        );
    }

    /// A synthetic Zork Zero flank: a full-width banner, then a pillar whose
    /// capital ends in a **ring wider than the shaft** — the feature that gives
    /// a mis-placed cut away, because a tile that starts inside it repeats it
    /// down the whole column. Everything below the shaft is the base, and the
    /// pillar is clipped at row 400 exactly as a taller banner clips it on screen.
    fn zork_zero_flank(banner: u32) -> RgbaImage {
        const W: u32 = 86;
        let mut c = RgbaImage::new(W, 400);
        let mut band = |y0: u32, y1: u32, x0: u32, x1: u32, v: u8| {
            for y in y0..y1.min(400) {
                for x in x0..x1 {
                    c.put_pixel(x, y, Rgba([v, v / 2, 9, 255]));
                }
            }
        };
        band(0, banner, 0, W, 200); // banner, full flank width
        band(banner, banner + 10, 4, 78, 150); // capital
        band(banner + 10, banner + 14, 2, 80, 250); // the ring under it
        band(banner + 14, banner + 306, 14, 62, 100); // the plain shaft
        band(banner + 306, banner + 332, 2, 80, 180); // the base
        c
    }

    /// SQ-0799 — the shaft is found from the art, at whatever row the banner
    /// above it happens to end.
    #[test]
    fn the_pillar_shaft_is_measured_from_the_art_not_pinned_to_one_banner_height() {
        // MCGA's 34-row banner doubled, EGA's 37, CGA's 39.
        for (banner, want) in [(68u32, (82u32, 374u32)), (74, (88, 380)), (78, (92, 384))] {
            assert_eq!(
                pillar_shaft(&zork_zero_flank(banner), 400),
                Some(want),
                "a {banner}-row banner puts the shaft at {want:?}"
            );
        }
        // And on the MCGA layout the measurement reproduces Bocfel's own castle
        // constants: cut 86 = 82 + 4, foot 26 = 400 - 374, unit 284 = 292 - 8.
        let (top, bottom) = pillar_shaft(&zork_zero_flank(68), 400).expect("a shaft");
        assert_eq!((top + 4, 400 - bottom, bottom - top - 8), (86, 26, 284));
    }

    /// SQ-0792 — a run that is not MOST of the flank is a coincidence in a
    /// texture, not a pillar shaft, and accepting it gave the two sides of one
    /// symmetric border different repeat units.
    ///
    /// The two cases below are `zork0.mg1`'s underground flanks reduced to their
    /// measured runs: 54 rows on the left and 146 on the right, of 400. Both must
    /// decline. Falsifiable: drop the majority test and they come back
    /// `Some((74, 128))` and `Some((220, 366))` — two different recipes for two
    /// halves of the same border.
    #[test]
    fn a_run_shorter_than_half_the_flank_is_not_a_shaft() {
        /// A flank whose only constant-span run is `[top, top + run)`, narrower
        /// than the banner above it; every other row wobbles by a pixel, exactly
        /// as dithered masonry does.
        fn wobbly(run: u32, top: u32) -> RgbaImage {
            let mut c = RgbaImage::new(86, 400);
            for y in 0..400u32 {
                let n = if (top..top + run).contains(&y) { 60 } else { 40 + (y % 7) };
                for x in 0..n.min(86) {
                    c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
                }
            }
            // A banner wider than anything below it, so the narrowness test passes.
            for y in 0..20 {
                for x in 0..86 {
                    c.put_pixel(x, y, Rgba([9, 9, 9, 255]));
                }
            }
            c
        }
        assert_eq!(pillar_shaft(&wobbly(54, 74), 400), None, "54 of 400 rows is 13%");
        assert_eq!(pillar_shaft(&wobbly(146, 220), 400), None, "146 of 400 rows is 36%");
        // …and the castle's own 292 of 400 (73%) still is one.
        assert_eq!(pillar_shaft(&wobbly(292, 82), 400), Some((82, 374)));
    }

    /// A single-piece border of one constant width — Shogun's DOS renditions,
    /// which reach the native screen bottom and are therefore handed to the Zork
    /// Zero handler (SQ-0802). It declares no shaft, so Bocfel's constants stand.
    #[test]
    fn a_constant_width_slab_declares_no_shaft() {
        let slab = solid(46, 400, [9, 9, 9, 255]);
        assert_eq!(pillar_shaft(&slab, 400), None);
    }

    /// SQ-0799, the defect as reported: *"for cg1 and eg1 we get a horizontal
    /// line on zork0 where we are tiling"*.
    ///
    /// Zork Zero's banner is 34 raw rows on MCGA but 37 on EGA and 39 on CGA,
    /// while its pillars are 166 rows in all three — so a cut pinned to unit row
    /// 86 sits in the plain shaft under MCGA and inside the ring beneath the
    /// capital under the other two, and every tile boundary repeats that ring.
    ///
    /// Falsifiable: put `TOP_CUT`/`FOOT`/`UNIT` back in place of the measurement
    /// and the 74- and 78-row cases fail with wide rows at the tile boundaries,
    /// while the 68-row case still passes — which is exactly why neither SQ-0698
    /// nor SQ-0790 could have seen this.
    #[test]
    fn no_tile_boundary_repeats_the_ring_under_the_capital() {
        const BAND: u32 = 800;
        for banner in [68u32, 74, 78] {
            let canvas = zork_zero_flank(banner);
            let (shaft_top, shaft_bottom) = pillar_shaft(&canvas, 400).expect("a shaft");
            let base = 400 - shaft_bottom;
            let out = flank_source(&canvas, &canvas, 0, 86, (0, 400), 400, 0, BAND).expect("extended");
            // The shaft's own span, read off the art rather than assumed.
            let span = |img: &RgbaImage, y: u32| {
                let (mut f, mut l) = (None, 0);
                for x in 0..img.width() {
                    if img.get_pixel(x, y)[3] >= 128 {
                        f.get_or_insert(x);
                        l = x;
                    }
                }
                f.map(|f| (f, l))
            };
            let want = span(&canvas, shaft_top);
            let wrong: Vec<u32> = (shaft_top..BAND - base).filter(|&y| span(&out, y) != want).collect();
            assert!(
                wrong.is_empty(),
                "a {banner}-row banner leaves {} row(s) of the extended shaft that are not the \
                 shaft's own span {want:?} — first at {:?}, which is the ring under the capital \
                 tiled down the column",
                wrong.len(),
                wrong.first()
            );
        }
    }

    #[test]
    fn arthurs_extension_keeps_his_foot_at_the_bottom() {
        // A pole whose last 20 rows are a distinctive foot.
        let mut canvas = RgbaImage::new(16, 400);
        for y in 0..379 {
            let v = if y >= 359 { 200 } else { 100 };
            for x in 0..16 {
                canvas.put_pixel(x, y, Rgba([v, 0, 0, 255]));
            }
        }
        let out = flank_source(&canvas, &canvas, 0, 8, (11, 379), 400, 0, 600).expect("extended");
        assert_eq!(out.height(), 600);
        assert!(out.get_pixel(0, 599)[3] == 255, "the band's last row is painted");
        assert_eq!(out.get_pixel(0, 599)[0], 200, "and it is the foot, not the shaft");
    }
}
