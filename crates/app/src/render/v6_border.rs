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
//! derived here is WHICH of the three known layouts a flank is showing
//! ([`recognize`]), from the art's own native extent — and, for Zork Zero, WHERE
//! its pillars are ([`pillar_shaft`]), because babelmap lets the player choose
//! the rendition and Bocfel does not. Everything else is per title, named, and
//! sourced.
//!
//! Every row coordinate in this file is in babelmap's v6 **unit space**, which
//! is the art's own pixels doubled *vertically* (`session::V6_ART_SCALE` = 2;
//! the horizontal factor is 1 for a 640-wide EGA or CGA archive, whose pixels
//! are half as wide — SQ-0790). Bocfel's constants are in raw art rows, so each
//! one appears here doubled, and the doubling is called out at every constant.

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
pub fn recognize(canvas: &RgbaImage, x0: u32, x1: u32, art: (u32, u32), native_h: u32) -> Option<BorderArt> {
    let (top, bottom) = art;
    if bottom <= top {
        return None;
    }
    let (lo, hi) = painted_widths(canvas, x0, x1, art)?;
    if bottom >= native_h && lo * 10 < hi * 9 {
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
/// **Both repeats are cut from `art`, never from `dst`.** This is the ORDER
/// Bocfel works in and it is load-bearing. Its covering rectangle over the
/// status bar — *"the raw border graphics have a decorative top bar that doesn't
/// fit the header text, so the original interpreters cover it with a solid color
/// rectangle"* — is step 4 of `draw_border_common`, drawn AFTER the extension,
/// so the copies it stamps are of the unmasked picture. babelmap's chrome canvas
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
///   pillar rather than a slab; and
/// * something is painted above it and something below it, so there is a
///   capital to cut beneath and a base to stamp back on.
///
/// Rows are compared by span rather than by pixel count on purpose: the CGA
/// rendition dithers its masonry, so the number of opaque pixels in a shaft row
/// wobbles from row to row while its edges do not move at all.
///
/// Measured over Zork Zero's five renditions, the castle shaft is 280 to 292 of
/// the flank's 400 rows. Its other two scene borders declare nothing usable —
/// 14 to 146 rows, or no run at all — which is why [`zork_zero`] still needs the
/// literal constants (SQ-0792).
///
/// Only rows `[0, art_bottom)` are considered — below that is the caller's
/// extension, not the game's art.
fn pillar_shaft(dst: &RgbaImage, art_bottom: u32) -> Option<(u32, u32)> {
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
    if last - first + 1 >= widest || top == 0 || bottom >= art_bottom {
        return None;
    }
    Some((top, bottom))
}

/// **Zork Zero** — `extend_pillars(43, 13, 200, 142, 0, false, false)`, the
/// CASTLE border on the VGA / Amiga / Blorb artwork (`zorkzero.cpp`), with
/// every one of those four numbers **measured off the art** instead.
///
/// Bocfel can hard-code them because it only ever draws one rendition per run,
/// chosen by display mode; babelmap lets the player switch archives, and Zork
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
/// constants are used as written. Every *castle* flank measured declares a shaft,
/// so that arm is Zork Zero's other two scene borders (below), whose stonework
/// has no plain span to find.
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
/// castle (above), `extend_underground_pillars(73, 54, 200, 74, 38, 37)` — which
/// alternates left/right stone blocks so the masonry stays consistent — and
/// `extend_jungle_pillars(67, 210, 143, 59)`. Which one is on screen is a
/// Z-machine global Bocfel reads and we cannot: the canvas is pixels. Composing
/// the other two borders from the archives' own pictures shows what they do get
/// (SQ-0792): the underground and jungle art declares no usable shaft — the
/// longest constant-span run is 14 to 146 rows against the castle's 280 to 292 —
/// so those flanks fall to the castle constants above, which is what the quest
/// predicted and what Bocfel's own dispatch exists to avoid. The mirror helps
/// them as much as it helps CGA, and it is why neither of Bocfel's scene-specific
/// overlaps (37 rows underground, 59 in the jungle) is reproduced here, but the
/// repeat unit is still the castle's.
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
        None => (TOP_CUT, FOOT, UNIT),
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
