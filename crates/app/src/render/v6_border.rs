//! SQ-0698 / SQ-0781 — vertical extension of Infocom v6 **side border art**.
//!
//! Three of the graphical v6 titles frame their story window with side artwork
//! that was authored for a 320x200 screen and does not reach the bottom of a
//! modern pane. babelmap used to make up the difference by *stretching* the
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
//! derived here is only WHICH of the three known layouts a flank is showing
//! ([`recognize`]), from the art's own native extent; the constants are per
//! title, named, and sourced.
//!
//! Every row coordinate in this file is in babelmap's v6 **unit space**, which
//! is the art's own pixels doubled (`session::V6_ART_SCALE` = 2). Bocfel's
//! constants are in raw art rows, so each one appears here doubled, and the
//! doubling is called out at every constant.

use image::{Rgba, RgbaImage};

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

/// Classify a flank from the opaque row extent of its own native columns.
///
/// `art` is `(first, last_exclusive)` opaque native row within the flank's
/// columns; `native_h` is the native screen height (400 for every v6 title we
/// carry). `None` when the flank has no art, or when its art already reaches
/// the screen bottom without starting at the top and the shape is not one of
/// the three below — in which case the caller keeps its existing behaviour.
///
/// Two measurements separate the three, and the ORDER matters because Zork
/// Zero's banner covers its flank columns from row 0 just as Shogun's border
/// does:
///
/// | title            | art rows (measured) | reaches bottom | starts at 0 |
/// |------------------|---------------------|----------------|-------------|
/// | Arthur (r54 adf) | 11..379             | no             | no          |
/// | Shogun (r295 adf)| 0..336              | no             | yes         |
/// | Zork Zero (r393) | 0..400              | **yes**        | yes         |
pub fn recognize(art: (u32, u32), native_h: u32) -> Option<BorderArt> {
    let (top, bottom) = art;
    if bottom <= top {
        return None;
    }
    if bottom >= native_h {
        return Some(BorderArt::ZorkZeroPillars);
    }
    if top == 0 {
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
/// Arthur's frame itself, quantised to its own cell height. babelmap never
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
/// Measured on `James Clavell's Shogun.adf` (release 295, serial 890321): the
/// border art is native rows 0..336 across flank columns 0..46 and 594..640, so
/// `border_height` = 336 = the 168-row raw image doubled.
fn shogun(dst: &mut RgbaImage, border_height: u32, desired_height: u32) {
    /// Bocfel's Amiga offset for `P_BORDER` is 2 raw lines; unit space doubles it.
    const OVERLAP: u32 = 4;
    if border_height == 0 || desired_height <= border_height {
        return;
    }
    let whole = snapshot(dst, 0, border_height);
    let second_top = border_height - OVERLAP;
    stamp(dst, &whole, second_top, true);
    let lowest = second_top + border_height;
    if desired_height <= lowest {
        return;
    }
    // `common_extend_border(desired_height, lowest_drawn_pixel, start_copy_from)`
    // — repeat everything from row 0 down to the lowest drawn line, truncating
    // the last copy at the bottom.
    let block = snapshot(dst, 0, lowest);
    tile_down(dst, &block, lowest, desired_height, 0, false, false);
}

/// **Zork Zero** — `extend_pillars(43, 13, 200, 142, 0, false, false)`, the
/// CASTLE border on the VGA / Amiga / Blorb artwork (`zorkzero.cpp`).
///
/// Doubled into unit space: cut at row **86**, repeat a **284**-row unit, keep
/// the bottom **26** rows as the foot, no overlap, no flip. (Bocfel's other
/// platforms differ — EGA `(45, 13, 203, 144, 9, …)`, CGA `(52, 25, 205, 133,
/// 11, flip=true)` — and its EGA/CGA art is not what a Blorb or an Amiga floppy
/// ships us.)
///
/// **Known limitation, deliberate.** Zork Zero has three scene borders and
/// Bocfel gives each its own routine: castle (above), `extend_underground_pillars(73,
/// 54, 200, 74, 38, 37)` — which alternates left/right stone blocks so the
/// masonry stays consistent — and `extend_jungle_pillars(67, 210, 143, 59)`.
/// Which one is on screen is a Z-machine global Bocfel reads and we cannot: the
/// canvas is pixels. The castle constants are used for all three. They degrade
/// gracefully — the cut at row 86 is below the capital in every one of the
/// three, and the bottom 26 rows are the base in every one of the three — but a
/// jungle screen loses its 59-row overlap and may show a seam. SQ-0792.
fn zork_zero(dst: &mut RgbaImage, art_bottom: u32, desired_height: u32) {
    /// `extend_pillars(top_cut=43, …)` doubled.
    const TOP_CUT: u32 = 86;
    /// `foot_height=13` doubled.
    const FOOT: u32 = 26;
    /// `pillar_height=142` doubled.
    const UNIT: u32 = 284;
    if art_bottom <= TOP_CUT + FOOT {
        return;
    }
    let unit = UNIT.min(art_bottom - FOOT - TOP_CUT);
    extend_pillars(dst, TOP_CUT, FOOT, art_bottom, unit, 0, false, desired_height);
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
/// SAME columns — measured on the graphics-only canvas, so a status run
/// rasterised into `canvas` cannot be mistaken for border art.
///
/// `None` when the flank shows no recognised border art, or when the art
/// already covers the band — the caller then keeps whatever it did before.
pub fn flank_source(
    canvas: &RgbaImage,
    x0: u32,
    x1: u32,
    art: (u32, u32),
    native_h: u32,
    crop_top: u32,
    rows: u32,
) -> Option<RgbaImage> {
    let kind = recognize(art, native_h)?;
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
        BorderArt::ShogunSinglePiece => shogun(&mut strip, art.1, desired),
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

    #[test]
    fn recognize_separates_the_three_measured_shapes() {
        // Arthur adf r54: poles native 11..379 of 400.
        assert_eq!(recognize((11, 379), 400), Some(BorderArt::ArthurPoles));
        // Shogun adf r295: single-piece border native 0..336 of 400.
        assert_eq!(recognize((0, 336), 400), Some(BorderArt::ShogunSinglePiece));
        // Zork Zero r393: pillars painted to the native bottom.
        assert_eq!(recognize((0, 400), 400), Some(BorderArt::ZorkZeroPillars));
        // An unpainted flank is nobody's border.
        assert_eq!(recognize((0, 0), 400), None);
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
            flank_source(&canvas, 0, 32, (0, 400), 400, 0, 400).is_none(),
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
        let out = flank_source(&canvas, 0, 32, (0, 336), 400, 30, 670).expect("extended");
        assert_eq!((out.width(), out.height()), (32, 670));
        for y in 0..out.height() {
            assert!(out.get_pixel(0, y)[3] == 255, "row {y} of the band is painted");
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
        let out = flank_source(&canvas, 0, 8, (11, 379), 400, 0, 600).expect("extended");
        assert_eq!(out.height(), 600);
        assert!(out.get_pixel(0, 599)[3] == 255, "the band's last row is painted");
        assert_eq!(out.get_pixel(0, 599)[0], 200, "and it is the foot, not the shaft");
    }
}
