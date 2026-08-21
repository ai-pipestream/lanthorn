//! SQ-0829: every art-scaling site in the app answers to ONE policy — a filter
//! chosen by the direction the axis moves, on associated (premultiplied) colour.
//!
//! `resize_directional` (SQ-0824 for the direction, SQ-0827 for the alpha) had a
//! single caller: the v6 chrome band. Everywhere else art was scaled by whichever
//! filter that site's author had reached for, and the three that moved in both
//! directions were each wrong in one of them:
//!
//! * **`Canvas::draw_image`** — Glulx's `glk_image_draw_scaled`, Triangle at every
//!   size. The game names both axes, so this one call blows a title card up AND
//!   squeezes a card down; magnified pixel art came back blurred, and a stencilled
//!   picture (fmvpoker's deck is cut out on colour 1) bled the `(0, 0, 0)` behind
//!   its transparent pixels into every edge.
//! * **`fit_preserving_aspect`** — inline transcript pictures, Triangle at every
//!   size. Zork Zero's drop caps and room icons are cut-out PNGs, and frameless
//!   mode asks for a deliberate whole 2×/3× enlargement "for pixel-art crispness"
//!   which Triangle then took straight back off.
//! * **`Resize::Fit(None)`** — cover art, gallery tiles, the resource preview and
//!   the non-kitty graphics-window blit. `None` means "the crate's default filter",
//!   and the crate's default is Nearest, so a 1200×1600 jacket reduced into a
//!   twenty-cell panel kept one row in seven and discarded the rest.
//!
//! Everything here is judged against an ideal computed IN THIS FILE from the source
//! pixels — an area average where an axis shrinks, replication where it grows —
//! never against whatever the helper returns. A test that asks the implementation
//! what the answer is passes for a broken implementation, which is the specific trap
//! on this quest: three of these sites already had tests, and all three passed while
//! the filter was wrong.
//!
//! FALSIFY: restore `FilterType::Triangle` in `Canvas::draw_image` and in
//! `fit_preserving_aspect`, or hand `img.clone()` and the raw cell rect straight to
//! `new_protocol(.., Resize::Fit(None))` in place of `fit_for_protocol`. Each
//! reverted site fails its own case with the symptom that motivated it — a colour
//! count that explodes on magnification, an RMS against the area average that jumps
//! by an order of magnitude on minification, a black fringe on a cut-out edge.
//!
//! Those four `Resize::Fit(None)` sites now make one call, `fitted_protocol`, and on
//! every backend that ENCODES pixels it is exactly the pair this file measures —
//! `fit_for_protocol` then an identity `Fit`, which is why the cases below still drive
//! `fit_for_protocol` through a kitty picker. Half-blocks encodes nothing and takes a
//! different route: it resolves the image straight into cells at one sample per column
//! and two per row, so the device-pixel pre-scale was an intermediate built to be
//! thrown away, and there the reduction is ONE pass onto that sample grid (SQ-0979).
//! The policy is unchanged — `resize_directional` does the pass either way — and the
//! half-blocks arm is measured against this same ideal in `graphics.rs`'s
//! `resample_tests`, including SQ-0829's own 1104-px toolbar.
//!
//! No `honor_game_colours` axis here, deliberately: not one of these paths consults
//! a game colour or a theme. They resample artwork, and the ground an inline picture
//! is later flattened onto (`page_for`/`flatten_onto`, which IS mode-dependent) is
//! untouched by this quest and stays pinned by the v6 suites.

use std::collections::HashSet;
use std::path::PathBuf;

use app::graphics::Canvas;
use app::render::graphics::{fit_for_protocol, kitty_picker};
use app::render::inline_image::fit_preserving_aspect;
use image::{DynamicImage, Rgba, RgbaImage};

// ---------------------------------------------------------------------------
// The independent ideal. Hand-rolled here rather than imported, so a change to
// the implementation's own arithmetic cannot quietly move the target.
// ---------------------------------------------------------------------------

/// The area-weighted average of the source rectangle each output pixel covers —
/// the right answer for an axis that SHRINKS, and the thing "resample once, in the
/// right direction" is trying to be.
fn area_average(src: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let (fx, fy) = (sw as f64 / tw as f64, sh as f64 / th as f64);
    let mut out = RgbaImage::new(tw, th);
    for y in 0..th {
        let (y0, y1) = (y as f64 * fy, (y as f64 + 1.0) * fy);
        for x in 0..tw {
            let (x0, x1) = (x as f64 * fx, (x as f64 + 1.0) * fx);
            let (mut acc, mut wsum) = ([0f64; 4], 0f64);
            for sy in (y0.floor() as u32)..(y1.ceil() as u32).min(sh) {
                let wy = (y1.min(sy as f64 + 1.0) - y0.max(sy as f64)).max(0.0);
                for sx in (x0.floor() as u32)..(x1.ceil() as u32).min(sw) {
                    let w = wy * (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)).max(0.0);
                    if w <= 0.0 {
                        continue;
                    }
                    let p = src.get_pixel(sx, sy).0;
                    (0..4).for_each(|c| acc[c] += p[c] as f64 * w);
                    wsum += w;
                }
            }
            let mut px = [0u8; 4];
            (0..4).for_each(|c| px[c] = (acc[c] / wsum).round().clamp(0.0, 255.0) as u8);
            out.put_pixel(x, y, Rgba(px));
        }
    }
    out
}

/// Whole-pixel replication along x — the right answer for an axis that GROWS.
fn nearest_x(src: &RgbaImage, tw: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let mut out = RgbaImage::new(tw, sh);
    for y in 0..sh {
        for x in 0..tw {
            let sx = (((x as f64 + 0.5) * sw as f64 / tw as f64).floor() as u32).min(sw - 1);
            out.put_pixel(x, y, *src.get_pixel(sx, y));
        }
    }
    out
}

fn transpose(src: &RgbaImage) -> RgbaImage {
    let (w, h) = src.dimensions();
    let mut out = RgbaImage::new(h, w);
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(y, x, *src.get_pixel(x, y));
        }
    }
    out
}

/// The per-axis single-resample ideal: area average where the axis shrinks,
/// replication where it grows.
fn ideal(src: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let mid = if tw < sw { area_average(src, tw, sh) } else { nearest_x(src, tw) };
    let t = transpose(&mid);
    let t = if th < sh { area_average(&t, th, tw) } else { nearest_x(&t, th) };
    transpose(&t)
}

fn rms(a: &RgbaImage, b: &RgbaImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "RMS wants matching geometry");
    let (mut s, mut n) = (0f64, 0f64);
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        for c in 0..3 {
            let d = pa.0[c] as f64 - pb.0[c] as f64;
            s += d * d;
            n += 1.0;
        }
    }
    (s / n).sqrt()
}

fn colours(img: &RgbaImage) -> HashSet<[u8; 4]> {
    img.pixels().map(|p| p.0).collect()
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

/// Four-ink pixel art in the shape the corpus's artwork actually takes: broad flat
/// regions joined by checkerboard-dithered transition bands (the shadow gradients
/// Journey's canyon is built from) with hard one-pixel edges cutting across (the
/// foreground rocks). Deliberately the same construction the SQ-0824 unit cases use,
/// because it is the one measured to separate an area filter from a decimating one
/// by an order of magnitude — a plate that is dither everywhere separates nothing,
/// since no filter can reconstruct noise at every frequency at once.
fn pixel_art(w: u32, h: u32) -> RgbaImage {
    let inks = [
        Rgba([0x20, 0x18, 0x10, 0xff]),
        Rgba([0xc8, 0x70, 0x28, 0xff]),
        Rgba([0x48, 0x38, 0x60, 0xff]),
        Rgba([0xf0, 0xe0, 0xa0, 0xff]),
    ];
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f64 * 4.0 / h.max(1) as f64;
            let band = (t.floor() as usize).min(3);
            let frac = t - t.floor();
            let ink = if frac > 0.85 && band < 3 && (x + y) % 2 == 0 {
                inks[band + 1]
            } else if x % 37 == 0 || y % 41 == 0 || (x / 8 + y / 8) % 11 == 0 {
                inks[(band + 2) % inks.len()]
            } else {
                inks[band]
            };
            img.put_pixel(x, y, ink);
        }
    }
    img
}

/// A stencil: opaque white on the left, fully transparent — and therefore carrying
/// `(0, 0, 0)` under its zero alpha — on the right. This is what a cut-out card or
/// a drop cap looks like at its edge, and the pixel that proves whether a blending
/// pass averaged coverage or averaged black.
fn cut_out(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let px = if x < w / 2 {
                Rgba([0xff, 0xff, 0xff, 0xff])
            } else {
                Rgba([0, 0, 0, 0])
            };
            img.put_pixel(x, y, px);
        }
    }
    img
}

/// The darkest colour any pixel with visible coverage came back as. A cut-out of
/// pure white can only produce white; anything darker is the transparent
/// neighbour's `(0, 0, 0)` having been averaged in.
fn darkest_visible(img: &RgbaImage) -> u8 {
    img.pixels().filter(|p| p.0[3] > 0).map(|p| p.0[..3].iter().copied().min().unwrap_or(255)).min().unwrap_or(255)
}

// ---------------------------------------------------------------------------
// Site 1 — `Canvas::draw_image`, i.e. Glulx `glk_image_draw_scaled`.
// ---------------------------------------------------------------------------

/// The canvas region the picture was drawn into, read back out.
fn drawn(canvas: &Canvas, w: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(x, y, *canvas.img.get_pixel(x, y));
        }
    }
    out
}

#[test]
fn glulx_draw_scaled_magnifies_by_replication_and_invents_no_colour() {
    let src = pixel_art(48, 48);
    let mut canvas = Canvas::new(160, 160);
    canvas.draw_image(&DynamicImage::ImageRgba8(src.clone()), 0, 0, Some((144, 144)));
    let got = drawn(&canvas, 144, 144);

    // Growing 3× is pure replication, so the ideal is met EXACTLY, not merely
    // approached — there is no rounding for a filter to disagree about.
    assert_eq!(got.as_raw(), ideal(&src, 144, 144).as_raw(), "a magnifying draw_scaled must replicate whole pixels");
    assert_eq!(
        colours(&got).len(),
        colours(&src).len(),
        "a magnified picture must arrive with the palette it left with; new colours are blur"
    );
}

#[test]
fn glulx_draw_scaled_minifies_by_area_average_not_by_dropping_rows() {
    let src = pixel_art(96, 96);
    let mut canvas = Canvas::new(128, 128);
    canvas.draw_image(&DynamicImage::ImageRgba8(src.clone()), 0, 0, Some((32, 32)));
    let got = drawn(&canvas, 32, 32);

    let target = ideal(&src, 32, 32);
    let dropped = image::imageops::resize(&src, 32, 32, image::imageops::FilterType::Nearest);
    let (near, mine) = (rms(&dropped, &target), rms(&got, &target));
    assert!(
        mine < 8.0 && mine * 3.0 < near,
        "shrinking draw_scaled scores {mine:.2} against the area-average ideal where a \
         decimating resample scores {near:.2}. A tent filter cannot BE the box filter at a \
         3x reduction, so the bar is the one that matters: stay well under 8 and beat \
         row-dropping by more than 3x, which is the aliasing the report was about."
    );
}

#[test]
fn glulx_draw_scaled_never_averages_a_transparent_neighbour_into_black() {
    let src = cut_out(64, 64);
    let mut canvas = Canvas::new(64, 64);
    canvas.draw_image(&DynamicImage::ImageRgba8(src), 0, 0, Some((24, 24)));
    let got = drawn(&canvas, 24, 24);

    assert!(
        darkest_visible(&got) >= 250,
        "a cut-out of pure white came back with a pixel at {} — the (0,0,0) under the \
         transparent side was averaged in, which composites as a dark fringe",
        darkest_visible(&got)
    );
}

// ---------------------------------------------------------------------------
// Site 2 — `fit_preserving_aspect`, the inline transcript picture.
// ---------------------------------------------------------------------------

#[test]
fn inline_picture_magnified_to_its_cell_box_stays_crisp() {
    let src = pixel_art(24, 24);
    // Frameless mode's deliberate integer enlargement: 3× into a box of its own shape.
    let got = fit_preserving_aspect(&src, 72, 72);

    assert_eq!(got.as_raw(), ideal(&src, 72, 72).as_raw(), "an enlarged inline picture must replicate");
    assert_eq!(
        colours(&got).len(),
        colours(&src).len(),
        "the whole point of the integer factor is that the palette survives it"
    );
}

#[test]
fn inline_picture_shrunk_to_its_cell_box_keeps_every_row() {
    let src = pixel_art(120, 90);
    let got = fit_preserving_aspect(&src, 40, 30);

    let target = ideal(&src, 40, 30);
    let dropped = image::imageops::resize(&src, 40, 30, image::imageops::FilterType::Nearest);
    let (near, mine) = (rms(&dropped, &target), rms(&got, &target));
    assert!(
        mine < 8.0 && mine * 3.0 < near,
        "the fitted picture scores {mine:.2} against the area-average ideal, row-dropping {near:.2}"
    );
}

#[test]
fn inline_cut_out_icon_keeps_its_edge_out_of_the_dark() {
    // Zork Zero's room icons and drop caps, in miniature: white artwork with a
    // transparent surround, shrunk to fit the band's cell box.
    let src = cut_out(80, 80);
    let got = fit_preserving_aspect(&src, 30, 30);
    assert!(
        darkest_visible(&got) >= 250,
        "the icon's cut edge came back at {} — black bled out from under the alpha",
        darkest_visible(&got)
    );
}

#[test]
fn inline_fit_still_letterboxes_rather_than_stretching() {
    // SQ-0704's guarantee, re-pinned because the resample under it moved: a 40×40
    // icon in a 40×48 box keeps its shape and the leftover stays transparent.
    let src = pixel_art(40, 40);
    let got = fit_preserving_aspect(&src, 40, 48);
    assert_eq!(got.dimensions(), (40, 48));
    assert_eq!(got.get_pixel(0, 0).0[3], 0, "the top margin is padding, not picture");
    assert_eq!(got.get_pixel(0, 47).0[3], 0, "the bottom margin is padding, not picture");
}

// ---------------------------------------------------------------------------
// Site 3 — `fit_for_protocol`: cover art, gallery tiles, resource preview and the
// non-kitty graphics-window blit, all of which delegated to `Resize::Fit(None)`.
// ---------------------------------------------------------------------------

#[test]
fn a_fitted_cover_is_area_averaged_not_decimated() {
    let picker = kitty_picker(8, 16);
    // A jacket scan into a picker panel: 240×320 native, twenty cells by ten.
    let src = pixel_art(240, 320);
    let (got, size) = fit_for_protocol(&picker, &DynamicImage::ImageRgba8(src.clone()), cells(20, 10), false);
    let got = got.to_rgba8();

    // The cell rect, computed by hand rather than by re-running the helper's own
    // arithmetic: the box is 160×160 px, the jacket's aspect binds on height, so the
    // picture lands at 120×160 px = 15 × 10 cells.
    assert_eq!((size.width, size.height), (15, 10), "fitted cell rect");
    assert_eq!(got.dimensions(), (120, 160), "the fit fills its cell box exactly");

    let target = ideal(&src, 120, 160);
    let dropped = image::imageops::resize(&src, 120, 160, image::imageops::FilterType::Nearest);
    let (near, mine) = (rms(&dropped, &target), rms(&got, &target));
    assert!(
        mine < 8.0 && mine * 3.0 < near,
        "the fitted cover scores {mine:.2} against the area-average ideal, row-dropping {near:.2}"
    );
}

/// The alpha half is a PRECONDITION here, not a repair: `Resize::Fit(None)` never
/// blended, so it never bled black either. Introducing an area filter at these sites
/// is exactly what let SQ-0827's defect in at the v6 flank, so the wrong alternative
/// this case falsifies is the obvious cheap fix — `Resize::Fit(Some(Triangle))`,
/// which buys the direction and loses the edge.
#[test]
fn a_fitted_picture_with_an_alpha_edge_keeps_its_colour() {
    let picker = kitty_picker(8, 16);
    let src = cut_out(160, 160);
    let (got, _) = fit_for_protocol(&picker, &DynamicImage::ImageRgba8(src), cells(6, 3), false);
    let got = got.to_rgba8();
    assert!(
        darkest_visible(&got) >= 250,
        "the preview's cut edge came back at {} — the transparent side's black was averaged in",
        darkest_visible(&got)
    );
}

#[test]
fn a_fitted_picture_hands_the_protocol_an_identity() {
    // The contract that makes this safe to bolt in front of `new_protocol`: the
    // returned image is already exactly the returned cell rect, so the crate's own
    // `Resize::Fit` finds nothing to do and there is no SECOND, Nearest resample.
    let picker = kitty_picker(8, 16);
    let src = pixel_art(240, 320);
    let (fitted, size) = fit_for_protocol(&picker, &DynamicImage::ImageRgba8(src), cells(20, 10), false);
    let proto = picker
        .new_protocol(fitted, size, ratatui_image::Resize::Fit(None))
        .expect("protocol builds");
    assert_eq!(proto.size(), size, "the protocol reports the rect it was pre-scaled to");
}

#[test]
fn a_small_picture_is_not_blown_up_unless_the_window_asked() {
    let picker = kitty_picker(8, 16);
    let src = pixel_art(32, 32);
    // `Fit` (a cover, a preview): leave it at native size, centred by the caller.
    let (fit, _) = fit_for_protocol(&picker, &DynamicImage::ImageRgba8(src.clone()), cells(20, 10), false);
    assert_eq!(fit.to_rgba8().dimensions(), (32, 32), "Fit never magnifies");
    // `Scale` (a Scott room picture in its own window): fill the box.
    let (scaled, _) = fit_for_protocol(&picker, &DynamicImage::ImageRgba8(src.clone()), cells(20, 10), true);
    let scaled = scaled.to_rgba8();
    assert_eq!(scaled.dimensions(), (160, 160), "Scale fills the 160×160 box, aspect preserved");
    assert_eq!(
        colours(&scaled).len(),
        colours(&src).len(),
        "and it fills it by replication — an upscaled room picture must stay crisp"
    );
}

// ---------------------------------------------------------------------------
// Real artwork. Skips vacuously without the gitignored fixture.
// ---------------------------------------------------------------------------

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A picture out of a real Glulx blorb — fmvpoker's deck is the corpus's stencilled
/// art (cut out on colour 1, SQ-0801) and it is the game that draws scaled.
fn a_real_glulx_picture() -> Option<RgbaImage> {
    let bytes = std::fs::read(stories_dir().join("fmvpoker.blb")).ok()?;
    let b = blorb::Blorb::parse(bytes).ok()?;
    for n in 0..64u32 {
        if let Some((_ty, data)) = b.resource(b"Pict", n) {
            if let Ok(img) = image::load_from_memory(data) {
                if img.width() >= 16 && img.height() >= 16 {
                    return Some(img.to_rgba8());
                }
            }
        }
    }
    None
}

#[test]
fn real_glulx_art_survives_a_whole_number_enlargement() {
    let Some(src) = a_real_glulx_picture() else {
        println!("SKIP: stories/fmvpoker.blb absent");
        return;
    };
    let (w, h) = src.dimensions();
    let mut canvas = Canvas::new(w * 2, h * 2);
    canvas.draw_image(&DynamicImage::ImageRgba8(src.clone()), 0, 0, Some((w * 2, h * 2)));
    let got = drawn(&canvas, w * 2, h * 2);
    assert_eq!(
        colours(&got).len(),
        colours(&src).len(),
        "doubling {w}×{h} of real artwork minted new colours — that is the blur"
    );
    assert_eq!(got.as_raw(), ideal(&src, w * 2, h * 2).as_raw(), "a 2× draw_scaled is replication");
}

/// Ratatui's `Size` under a name that reads at the call sites above.
fn cells(cols: u16, rows: u16) -> ratatui::layout::Size {
    ratatui::layout::Size::new(cols, rows)
}

