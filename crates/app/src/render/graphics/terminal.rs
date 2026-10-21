//! Terminal image protocols for the game pane (SQ-1804): everything in the graphics
//! renderer that names `ratatui-image` or encodes/uploads the kitty, sixel and
//! iTerm2 protocols. Compiled only with the `terminal` feature; `graphics.rs`
//! re-exports it, so every `render::graphics::<name>` path is unchanged. What a
//! non-terminal host draws with (click maps, the cell path, geometry) stays in the
//! parent module.

use super::*;
pub(super) use crate::imaging::{GameImaging, ImageProtocol};
pub(super) use ratatui_image::picker::Picker;
pub(super) use ratatui_image::protocol::Protocol;
pub(super) use ratatui_image::{Image, Resize};

/// Pre-scale `img` for [`Picker::new_protocol`] through [`resize_directional`],
/// and return it with the cell [`Size`] to hand the picker (SQ-0829).
///
/// Hand the pair to `new_protocol` with `Resize::Fit(None)`, which is then a
/// no-op: the returned image is already exactly `size × font_size` pixels, so the
/// crate's own `needs_resize` short-circuits and never resamples. That is the whole
/// point — `Resize::Fit(None)` means "resize with the DEFAULT filter", and the
/// default is `FilterType::Nearest`. Every delegation to it was therefore a Nearest
/// resample chosen by omission rather than on purpose, and `Fit` is overwhelmingly
/// a MINIFICATION: a 1200×1600 cover into a 20-cell panel is a 7× reduction in
/// which Nearest keeps one source row in seven and throws the rest away.
///
/// `upscale` picks between the crate's two aspect-preserving modes — `false` is
/// `Resize::Fit` (clamped to the image's own size, so a picture smaller than the
/// box is not blown up to fill it) and `true` is `Resize::Scale` (fills the box in
/// both directions). Aspect is preserved either way, so both axes always travel the
/// same way and [`resize_directional`]'s per-axis split costs nothing here; what it
/// brings is the direction itself, plus the alpha association a cut-out PNG needs.
///
/// The leftover strip below/right of the fitted picture is transparent padding, laid
/// down top-left — byte for byte what the crate does with a picker that was never
/// given a `background_color`, which is every picker lanthorn builds.
pub fn fit_for_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> (image::DynamicImage, Size) {
    if target.width == 0 || target.height == 0 {
        return (img.clone(), target);
    }
    let g = fit_geometry(picker.font_size(), (img.width(), img.height()), target, upscale);
    let (pw, ph) = g.boxed;
    let (tw, th) = g.pic;
    let scaled = resize_directional(&img.to_rgba8(), tw, th);
    let out = if (tw, th) == (pw, ph) {
        scaled
    } else {
        let mut padded = image::RgbaImage::new(pw, ph);
        image::imageops::replace(&mut padded, &scaled, 0, 0);
        padded
    };
    (image::DynamicImage::ImageRgba8(out), g.cells)
}

/// Where an aspect-preserving fit lands, in cells and in pixels — the whole of
/// [`fit_for_protocol`]'s geometry, kept apart from its resample so the half-blocks
/// arm of [`fitted_protocol`] can reach the same cells by a different route (SQ-0979).
///
/// The two answers cannot be allowed to differ. `Protocol::size()` is what every
/// caller centres against — [`GraphicsRender::render`]'s letterbox, the resource
/// preview's, and the gallery's own `fitted_tile_rect` — so a cell rect that moved
/// when the backend changed would be a layout change wearing a performance fix's
/// clothes. `fitted_cells_match_the_prescale` pins them against each other.
pub(super) struct FitGeometry {
    /// The cell rect the protocol will report.
    pub(super) cells: Size,
    /// The fitted picture, in device pixels.
    pub(super) pic: (u32, u32),
    /// `cells` in device pixels — what `pic` is padded out to, top-left.
    pub(super) boxed: (u32, u32),
}

pub(super) fn fit_geometry(
    fs: ratatui_image::FontSize,
    src: (u32, u32),
    target: Size,
    upscale: bool,
) -> FitGeometry {
    let (fw, fh) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
    let (sw, sh) = (src.0.max(1), src.1.max(1));
    let (bw, bh) = (target.width as u32 * fw, target.height as u32 * fh);
    // Which cell rect the protocol will report: the fit, rounded UP to whole cells.
    let (cw, ch) = if upscale { (bw, bh) } else { (bw.min(sw), bh.min(sh)) };
    let (aw, ah) = fit_proportionally(sw, sh, cw, ch);
    let cells = Size::new(aw.div_ceil(fw) as u16, ah.div_ceil(fh) as u16);
    // Then the pixels inside it. The second fit is not redundant: the ceil above can
    // add up to a cell on each axis, and the crate resamples into that whole box.
    let boxed = (cells.width as u32 * fw, cells.height as u32 * fh);
    FitGeometry { cells, pic: fit_proportionally(sw, sh, boxed.0, boxed.1), boxed }
}

/// The protocol a fitted picture goes on screen as: cover art, gallery tiles, the
/// resource preview and the non-kitty graphics-window blit (SQ-0979).
///
/// One call in place of the [`fit_for_protocol`] + `new_protocol(.., Resize::Fit(None))`
/// pair those four sites each wrote out, because on HALF-BLOCKS that pair resamples
/// twice. `fit_for_protocol` pre-scales to the pane's device pixels, and
/// `Halfblocks::encode` then takes whatever it is handed straight back down to
/// `cols x 2·rows` samples — one per column, two per row, `font_size` thrown away. At
/// a 20x11 gallery tile on a 10x20 font that is a 200x220 intermediate built to reach
/// a 20x22 grid, and up-then-down through two filters is blurrier than one pass down.
/// So half-blocks resamples ONCE, onto the sample grid itself, exactly as the v6
/// composite has since SQ-0973.
///
/// **Only half-blocks.** Kitty, sixel and iTerm2 genuinely encode pixels, so the
/// pre-scale is the right shape for them and is byte for byte where SQ-0829 left it —
/// and `GraphicsRender::render` never reaches here under kitty at all, which places
/// its own canvas. `only_halfblocks_leaves_the_fit_prescale_behind` is that claim.
///
/// The CELL RECT is the same either way: [`fit_geometry`] answers it once, and the
/// half-blocks arm maps the picture onto its grid rather than choosing a new box. So
/// nothing centred against `Protocol::size()` moves.
///
/// `None` when the protocol fails to build — every caller draws nothing, which is what
/// each of them did with the `Err` this replaces.
pub fn fitted_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> Option<Protocol> {
    if picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks
        && target.width > 0
        && target.height > 0
    {
        return halfblocks_fitted_protocol(picker, img, target, upscale);
    }
    let (img, size) = fit_for_protocol(picker, img, target, upscale);
    picker.new_protocol(img, size, Resize::Fit(None)).ok()
}

/// A fitted picture as half-blocks, resampled EXACTLY ONCE (SQ-0979).
///
/// The grid is `cells.width x 2·cells.height` samples, and the picture's own extent on
/// it is the device-pixel fit scaled into that grid — `pic.0 · cols / boxed.0` across
/// and `pic.1 · 2·rows / boxed.1` down. Those two ratios are not the same number: a
/// cell is one sample wide and two tall whatever the font's aspect is, so the mapping
/// is anisotropic and the picture's own aspect is carried by `pic`, which was fitted in
/// square device pixels. The leftover — under a cell's worth on the axis the fit did
/// not bind, so at most one column or two rows here — stays transparent padding laid
/// down top-left, as [`fit_for_protocol`] leaves it, because `Halfblocks::encode`
/// resolves alpha to black and that black margin is what the letterbox already showed.
///
/// [`resize_directional`] does the one resample, so SQ-0829's two guarantees hold on
/// the grid that is actually drawn rather than on device pixels the backend discards:
/// each axis filtered by the direction IT moves (and the two can differ here, where
/// they never could in device space — a photograph reduced into a tile shrinks on both,
/// while a small Scott room picture magnified to fill its window can still shrink
/// vertically onto the sample grid), and a blending pass on associated colour so a
/// cut-out edge is not averaged toward `(0,0,0)`.
pub(super) fn halfblocks_fitted_protocol(
    picker: &Picker,
    img: &image::DynamicImage,
    target: Size,
    upscale: bool,
) -> Option<Protocol> {
    use ratatui_image::protocol::halfblocks::Halfblocks;
    let g = fit_geometry(picker.font_size(), (img.width(), img.height()), target, upscale);
    let (gw, gh) = (u32::from(g.cells.width), u32::from(g.cells.height) * 2);
    let map = |v: u32, from: u32, to: u32| {
        ((f64::from(v) * f64::from(to) / f64::from(from.max(1))).round() as u32).clamp(1, to.max(1))
    };
    let (sx, sy) = (map(g.pic.0, g.boxed.0, gw), map(g.pic.1, g.boxed.1, gh));
    let scaled = resize_directional(&img.to_rgba8(), sx, sy);
    let grid = if (sx, sy) == (gw, gh) {
        scaled
    } else {
        let mut padded = image::RgbaImage::new(gw, gh);
        image::imageops::replace(&mut padded, &scaled, 0, 0);
        padded
    };
    let hb = Halfblocks::new(image::DynamicImage::ImageRgba8(grid), g.cells).ok()?;
    Some(Protocol::Halfblocks(hb))
}


/// The image the v6 raster composite goes to the protocol as, and the fit mode that
/// finishes it — the whole of [`GraphicsRender::encode_v6`]'s resampling decision,
/// kept pure so it can be measured (SQ-0824).
///
/// `Resize::Fit` only ever SHRINKS, so a pane bigger than the composite needs the
/// magnification done here: Nearest, capped at `max_upscale`, after which the
/// protocol's own (also Nearest) fit at most nudges the result onto the cell grid.
/// The ceiling is the BACKEND's, not this function's — [`v6_upscale_cap`] answers
/// it, and `None` means the backend has no encode to budget for (SQ-0964).
///
/// A pane SMALLER than the composite needs no pre-scale at all, and that is the fix.
/// The scale used to be clamped at 1.0, which turned this branch into a full identity
/// copy of the canvas that bought nothing — and then left the actual shrink to the
/// protocol's DEFAULT filter, Nearest, which drops whole rows and columns exactly
/// where Journey's dithered foreground keeps its detail. Naming the area filter makes
/// it one resample, from the best source there is, in the right direction.
pub fn v6_fit_source(
    canvas: &image::RgbaImage,
    box_w: u32,
    box_h: u32,
    lock: Option<f32>,
    max_upscale: Option<f64>,
) -> (image::RgbaImage, Resize) {
    let (cw, ch) = canvas.dimensions();
    // The backend's ceiling on magnification, or none at all (SQ-0964). Whatever it
    // allows, the BOX still decides how big the composite gets: every scale below is
    // derived from `box_w`/`box_h` (the locked one via the same pane, see below), so
    // lifting the ceiling lets the composite reach the pane and never past it.
    let capped = |s: f64| match max_upscale {
        Some(c) => s.min(c),
        None => s,
    };
    // SQ-0936: the LOCKED magnification, when the pane has one. The raster arm used
    // to compute its own free scale here and so never saw `v6_pixel_lock` at all —
    // which is not the "the setting does nothing in raster mode" caveat it looks
    // like, because a title that publishes no primary Buffer falls through to this
    // arm in HYBRID mode too. scopa (three Grids and `erase_window` fills, SQ-0711)
    // and fmvpoker both do, and both measured 0 differing pixels with the lock on
    // and off until this existed.
    //
    // A locked scale is always <= the free one, so the pre-scaled image always fits
    // its box and the protocol's `Fit` (which only ever shrinks) leaves it alone and
    // centres it.
    if let Some(s) = lock.filter(|s| s.is_finite() && *s > 0.0) {
        let s = capped(f64::from(s));
        let (tw, th) = (((cw as f64 * s) as u32).max(1), ((ch as f64 * s) as u32).max(1));
        // Nearest in BOTH directions here, unlike the free path below, and that is
        // the point rather than an oversight: a locked scale puts one art pixel on a
        // whole number of device pixels, so nearest is exact — it duplicates or
        // drops whole pixels and invents no intermediate colour. Area-averaging an
        // exact 1/2 would blend pairs that the original never blended.
        let scaled = image::imageops::resize(canvas, tw, th, image::imageops::FilterType::Nearest);
        return (scaled, Resize::Fit(None));
    }
    let scale = capped((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64));
    if scale < 1.0 {
        return (canvas.clone(), Resize::Fit(Some(image::imageops::FilterType::Triangle)));
    }
    let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
    let scaled =
        image::imageops::resize(canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
    (scaled, Resize::Fit(None))
}

/// The composite as the protocol must HOLD it — padded out to the whole cells it
/// will be placed over — and the picture's own device-pixel rect inside that box
/// (SQ-1081).
///
/// **Kitty scales a virtual placement's image to the cell rectangle it covers.**
/// lanthorn's own transmit says so in as many words (`r=`/`c=` in
/// [`kitty_transmit_virtual`]), `ratatui-image`'s says it by omission, and the
/// placement oracle — a port of Ghostty's core — resolves it that way, which is why
/// `tests/pty_stream/raster.rs` composites by scaling a placement's source rect onto
/// its destination. So wherever the image and the cells it is placed over disagree,
/// the pixels the player sees are the protocol's image RESAMPLED BY THE TERMINAL,
/// through whatever filter that terminal smooths with — and against artwork with no
/// intermediate tones in it (`machine-screenshots/amiga-journey.png`, the Amiga
/// release at the party menu) that is pure loss: every edge in the frame softened to
/// reconstruct detail the source never had. Sixel and iTerm2 draw at their own pixel
/// size instead, so for them the pad is inert — a transparent margin that merely makes
/// the picture centre in the cells the placement already reserved.
///
/// They disagree because [`Picker::new_protocol`] SHORT-CIRCUITS. Its `needs_resize`
/// returns `None` the moment the pre-scaled image already fits, handing the protocol
/// the image untouched under a cell rect rounded UP — so the composite is placed over
/// a box up to a cell taller and wider than itself. Measured on a 640x400 press at
/// the gallery's 16x32 kitty cell:
///
/// ```text
///   pane     s      composite     placed over        terminal stretch
///   82x28    2.00   1280x800   →  80x25 = 1280x800     1.00000   (exact)
///   60x24    1.50    960x600   →  60x19 = 960x608      1.01333   ← every row resampled
///   76x40    1.90   1216x760   →  76x24 = 1216x768     1.01053
/// ```
///
/// That is the whole of "a fractional magnification interpolates every edge", and it
/// is why the 0.3 gallery had to pin its raster shots to a whole `s`: at `s = 2` a
/// 640x400 press lands on `16s x 32s` exactly and the stretch is 1. It is not about
/// the fraction as such — `s = 1.2` on an 8x16 cell reaches 768x480, whole cells on
/// both axes, and is already exact — it is about landing on the CELL GRID, which a
/// whole `s` on a matched cell does by luck and a fractional one essentially never does.
///
/// Padding costs nothing anyone can see: the cell rect is `ceil(pixels / cell)` either
/// way, so the composite occupies the SAME cells and nothing centred against
/// `Protocol::size()` moves. What changes is that the terminal has a 1:1 blit to do
/// instead of a resample. The pad is split evenly around the picture, because this
/// image is a game screen being letterboxed rather than a picture in a box — one more
/// sub-cell of margin on each side, not a whole cell of it below. (Only on the arm
/// this pads; the shrinking one belongs to the crate, which lays its picture top-left.)
///
/// Nothing here touches the SHRINKING arm, which never had the defect: there the
/// protocol still has a resize to do, `Resize::resize` runs, and it pads its own
/// output onto the cell grid already. All this wants from that arm is where the
/// picture will land inside it, which is [`fit_proportionally`] — the same arithmetic
/// the crate's `needs_resize_pixels` performs, reproduced here for exactly this
/// purpose.
pub(super) fn v6_pad_to_cells(
    img: image::RgbaImage,
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
) -> (image::RgbaImage, (u32, u32, u32, u32)) {
    let (fw, fh) = (u32::from(fs.width.max(1)), u32::from(fs.height.max(1)));
    let (iw, ih) = img.dimensions();
    if iw > box_w || ih > box_h {
        let (dw, dh) = fit_proportionally(iw, ih, box_w.min(iw), box_h.min(ih));
        return (img, (0, 0, dw, dh));
    }
    let (pw, ph) = (iw.div_ceil(fw) * fw, ih.div_ceil(fh) * fh);
    if (pw, ph) == (iw, ih) {
        return (img, (0, 0, iw, ih));
    }
    let (ox, oy) = ((pw - iw) / 2, (ph - ih) / 2);
    let mut padded = image::RgbaImage::new(pw, ph);
    image::imageops::replace(&mut padded, &img, i64::from(ox), i64::from(oy));
    (padded, (ox, oy, iw, ih))
}


/// How far this backend may magnify the v6 raster composite before the protocol
/// fits it into the pane: [`MAX_V6_UPSCALE`], or **no ceiling at all** (SQ-0964).
///
/// The cap is a PNG-encode budget and nothing else. Kitty and iTerm2 ship the
/// composite down the wire as encoded pixels every frame it changes, and sixel
/// encodes one too, so every extra factor of magnification is bytes to build and
/// bytes to write — there the ceiling earns its keep and stays exactly where it is.
///
/// Half-blocks encodes nothing. `ratatui-image` resolves the image straight into
/// terminal cells at one pixel per column and two per row, so the budget it is
/// protecting does not exist — while the COST is entirely real, because under
/// `Resize::Fit` (which only ever shrinks) the pre-scale here is what decides how
/// many CELLS the composite occupies. Capped at 2×, a 640×400 canvas reaches a
/// fixed 1280×800 nominal pixels — a fixed number of cells — while shrinking the
/// terminal font goes on giving the pane more of them. So the picture that should
/// have grown sharper as the grid got finer visibly SHRANK instead, worst on the
/// titles that fall through to the composite whatever the mode (scopa and fmvpoker,
/// which publish no primary Buffer — SQ-0711).
///
/// Removing the ceiling does not remove a bound: the free scale is derived from the
/// pane box and the locked one from the same pane's ladder (SQ-0945), so the
/// composite still stops at the pane. It may simply climb the whole way there.
pub fn v6_upscale_cap(picker: &Picker) -> Option<f64> {
    match picker.protocol_type() {
        ratatui_image::picker::ProtocolType::Halfblocks => None,
        _ => Some(MAX_V6_UPSCALE),
    }
}

/// Whether the v6 magnification LOCK has a rung on this backend to snap to at all
/// (SQ-0978).
///
/// `v6_pixel_lock` promises **one art pixel on a whole number of device pixels**, and
/// [`crate::render::v6_layout::locked_scale`] delivers it by quantizing the letterbox
/// factor `s` in device pixels — `pane_dev = cells x picker.font_size()`. That is exact
/// for every backend that ships pixels: kitty, iTerm2 and sixel each put the composite
/// on the screen at the device resolution the pane really has.
///
/// Half-blocks does not have one. `Picker::halfblocks()` — and `from_query_stdio`'s
/// default when a terminal answers no cell size — hardcodes `FontSize::new(10, 20)`
/// whatever the real font is, and the encoder then **throws the font size away**:
/// `Halfblocks::encode` resamples whatever it is handed to exactly `width x 2·height`
/// SAMPLES, one per column and two per row. So the grid the picture actually resolves
/// onto is a property of the CELL grid, and the device pixels `s` was quantized in are
/// a number the picker invented.
///
/// The honest analogue would be to quantize onto that sample grid instead — one art
/// pixel on a whole number of half-block samples, which is `cols` and `2·rows` and no
/// font size at all. **It was measured and it buys nothing**, and the reason is that
/// half-blocks does not magnify: a 640x400 canvas has more unit pixels than a terminal
/// has samples until the pane reaches 640x200 CELLS, so the composite is minified at
/// every size anyone runs, and [`resize_directional`] minifies through `Triangle`.
/// Measured on a 640x400 canvas of 2x2 art pixels in hard black/white stripes:
///
/// ```text
///   sample grid   ratio   samples that are a PURE art-pixel colour
///   640x400        1:1    640 / 640     (Nearest — the target is not below the source)
///   458x288       1.4:1     50 / 458
///   320x200         2:1      0 / 320    ← an EXACT rung: one art pixel per sample
///   160x100         4:1      0 / 160
/// ```
///
/// The 320x200 row is the whole finding. It is the honest ladder's own rung — one art
/// pixel onto exactly one sample — and Triangle still lands every sample on a 25/75
/// blend of two art pixels, because a separable Triangle at ratio 2 has support 2 in
/// source space and reaches across the art pixel's edge. The rung delivers nothing.
///
/// And below that rung there is nothing to reach for either, because free scaling is
/// never worse: at `s >= 10` nominal device pixels the sample grid is at or above the
/// canvas, `resize_directional` picks `Nearest`, and the art comes out pure ALREADY —
/// while the lock could only move `s` DOWN, off that plateau and into Triangle. Below
/// it every `s` blends, rung or no rung. So on half-blocks the lock has no reachable
/// pane size at which it improves a frame, and a measured 17-20% of linear resolution
/// to lose where it acts: at a 120x40 pane a 640x400 canvas free-scales to 120x38
/// cells and the old device-pixel rung cut it to 96x30.
///
/// So the lock is INERT here, and `/dump-terminal` says so in those words rather than
/// reporting a snap that did not happen. Not a ceiling — SQ-0964 removed the one
/// half-blocks used to carry and nothing here puts it back; the free scale still climbs
/// the whole way to the pane.
///
/// The picker is the thing that knows, exactly as it is for [`v6_upscale_cap`].
pub fn v6_pixel_lock_applies(picker: &Picker) -> bool {
    picker.protocol_type() != ratatui_image::picker::ProtocolType::Halfblocks
}

/// Whether this protocol's images are painted ABOVE the terminal's cells, so a
/// dialog written into the cells beneath one is invisible (SQ-1796).
///
/// Sixel and iTerm2 draw a bitmap over the cell grid. Kitty, as this app drives it
/// (`render_kitty_virtual`), places NO bitmap at all: the image is a field of unicode
/// placeholder cells, so a dialog that overwrites those cells simply replaces them
/// and wins; half-blocks is cells outright.
pub fn protocol_paints_over_cells(picker: &Picker) -> bool {
    matches!(
        picker.protocol_type(),
        ratatui_image::picker::ProtocolType::Sixel | ratatui_image::picker::ProtocolType::Iterm2
    )
}

/// Whether this terminal may be sent a DEFLATED kitty transmission (SQ-0997).
///
/// `o=z` is not a hint a terminal may ignore. One that cannot inflate refuses the
/// transmission outright: the image is never stored, and every placement naming
/// it draws nothing — so a graphics window simply has no picture in it, with no
/// error, no fallback, and nothing on screen to suggest anything went wrong.
///
/// That is why SQ-0991 made `ratatui-image` probe before compressing. Lanthorn's
/// OWN transmit — [`kitty_transmit_virtual`], which SQ-0976 taught to compress
/// before the capability existed — stated `o=z` whatever the probe said, so on
/// such a terminal every graphics-window image (Glulx toolbars, Scott room
/// pictures, v6 graphics windows) vanished silently while the chrome ring beside
/// them, encoded by the crate, drew fine. Two encoders, one wire, one answer.
///
/// The picker is the thing that knows, exactly as it is for [`v6_upscale_cap`].
/// An EMPTY capability list means raw, and it has to: that is what
/// `Picker::halfblocks()`, the deprecated `from_fontsize`, the default picker
/// returned when a query fails, tmux without passthrough and the terminal
/// blacklist all leave behind, and none of them is evidence that this terminal
/// can inflate anything. Fail safe — a raw upload is merely slower, an
/// uninflatable one is invisible.
pub fn kitty_compression(picker: &Picker) -> bool {
    picker.capabilities().contains(&ratatui_image::picker::Capability::KittyCompression)
}

/// Whether this terminal can open a POSIX shared memory object we write (SQ-1374).
///
/// The same shape as [`kitty_compression`] and for the same reason: `t=s` is not a
/// hint. A terminal that cannot reach the object refuses the transmission, stores
/// no image, and every placement naming it draws nothing — and "cannot reach"
/// covers the ordinary case of a terminal at the other end of an ssh connection,
/// which is a first-class way to run this app. So it is ASKED, by setting
/// `ratatui-image`'s `kitty_shared_memory_object` (SQ-1382: one option, not a
/// separate probe flag): a one-pixel object created at startup, named in the
/// capability query with `a=q,t=s`, and reported only on `OK`.
///
/// An empty capability list therefore means no, exactly as it does for
/// compression, and so does `kitty_shared_memory = "off"` — that key is honoured
/// by never sending the probe (see `picker_ui::build_cover_picker`), so a user who
/// declined it looks from here like a terminal that cannot do it, which is the
/// same thing for every decision downstream.
pub fn kitty_shared_memory(picker: &Picker) -> bool {
    picker.capabilities().contains(&ratatui_image::picker::Capability::KittySharedMemory)
}

/// How a graphics window's pixels reach this terminal (SQ-1374).
///
/// Two capabilities the terminal answers separately, kept as one value because
/// every decision downstream needs BOTH: shared memory is preferred where it is
/// available, and the deflate answer is still what the fallback needs when a
/// shared memory write fails at runtime. Passing them as two booleans down the
/// same call chain is the shape this codebase has been bitten by (see the
/// refactoring policy in CLAUDE.md) — one of them gets dropped at a call site and
/// the frame that comes out is self-consistent and wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowWire {
    /// The terminal answered the `o=z` probe: a deflated payload will be inflated.
    pub compress: bool,
    /// The terminal answered the `t=s` probe: it can open an object we write.
    pub shared_memory: bool,
}

/// What [`WindowWire`] this picker's terminal is owed.
pub fn window_wire(picker: &Picker) -> WindowWire {
    WindowWire { compress: kitty_compression(picker), shared_memory: kitty_shared_memory(picker) }
}

/// Build a `Protocol::Kitty` directly, under a caller-chosen image id, without
/// the fork-only `Picker::new_protocol_with_id` (SQ-1513).
///
/// Upstream now exposes everything the id-stability path (SQ-0995/SQ-0996)
/// needs: [`Picker::tmux_detected`], [`Picker::capabilities`], and the
/// already-public `Kitty::new` and [`Resize::size_for`]/[`Resize::resize`].
/// This reproduces exactly what the fork's `new_protocol_id` did for the
/// Kitty arm — `size_for` computes the cell area the image resizes into,
/// `resize` performs it (background `None`, matching every picker lanthorn
/// builds — see [`fit_for_protocol`]'s doc), and the result goes to
/// `Kitty::new` under `id` instead of a random draw. Skipping the crate's own
/// `needs_resize` short-circuit costs one redundant resize call when the
/// image is already exactly cell-sized (every caller here pre-fits for
/// exactly that reason — see [`v6_pad_to_cells`]), but produces the same
/// pixels: a `Resize::Fit`/`Nearest` resample at 1:1 is an identity.
///
/// Upstream 12.0.0-rc.0's `Kitty::new` takes `smo` as a plain `bool` — the
/// shared-memory object's name is generated inside the crate now
/// (`generate_shm_name`, a random 16-byte name under the macOS 31-byte limit),
/// so there is no pid to thread through any more (SQ-1510).
pub(super) fn kitty_protocol_with_id(
    picker: &Picker,
    image: image::DynamicImage,
    size: Size,
    resize: Resize,
    id: u32,
) -> Result<Protocol, ratatui_image::errors::Errors> {
    let font_size = picker.font_size();
    let area = resize.size_for(&image, font_size, size);
    let image = resize.resize(&image, font_size, area, None);
    Ok(Protocol::Kitty(ratatui_image::protocol::kitty::Kitty::new(
        image,
        area,
        id,
        picker.tmux_detected(),
        kitty_compression(picker),
        kitty_shared_memory(picker),
    )?))
}


/// The cell rect the v6 composite occupies under HALF-BLOCKS, without building a
/// pixel of it (SQ-0973).
///
/// Same answer [`v6_fit_source`] and the protocol's own `Fit` arrive at together —
/// the composite fitted into the pane's device box at the magnification the pane (or
/// the lock) allows, then rounded up onto the cell grid — reached by arithmetic instead
/// of by a 52 MB intermediate. `v6_halfblocks_grid_matches_the_protocol` pins the two
/// against each other over a pane sweep, in every branch, so the mirror cannot drift.
///
/// No upscale ceiling appears here because half-blocks has none: [`v6_upscale_cap`]
/// answers `None` for it, and this function is reachable only from that arm.
pub fn v6_halfblocks_grid(
    canvas: (u32, u32),
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
    lock: Option<f32>,
) -> Size {
    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    let (fw, fh) = (u32::from(fs.width.max(1)), u32::from(fs.height.max(1)));
    // What `v6_fit_source` hands over: the canvas at the pane's (or the lock's)
    // magnification, truncated exactly as it truncates, and the canvas untouched where
    // it declines to pre-scale at all.
    let (tw, th) = match lock.filter(|s| s.is_finite() && *s > 0.0) {
        // The artwork's own integer ladder (SQ-0945/0936).
        Some(s) => {
            let s = f64::from(s);
            (((cw as f64 * s) as u32).max(1), ((ch as f64 * s) as u32).max(1))
        }
        None => {
            let s = (box_w as f64 / cw as f64).min(box_h as f64 / ch as f64);
            if s < 1.0 {
                (cw, ch)
            } else {
                (((cw as f64 * s) as u32).max(cw), ((ch as f64 * s) as u32).max(ch))
            }
        }
    };
    // Then what `Resize::Fit` does to it, which is the crate's own
    // `fit_area_proportionally(tw, th, min(box_w, tw), min(box_h, th))` — the shrink the
    // pre-scale deliberately leaves to the protocol, and the ONLY thing standing between
    // an over-large locked rung and a composite that overruns its pane. It ROUNDS where
    // the magnification above truncates, and it is an exact identity whenever the
    // pre-scale already fits.
    let ratio = (f64::from(tw.min(box_w)) / f64::from(tw)).min(f64::from(th.min(box_h)) / f64::from(th));
    let (tw, th) = (
        ((f64::from(tw) * ratio).round() as u32).max(1),
        ((f64::from(th) * ratio).round() as u32).max(1),
    );
    Size::new(tw.div_ceil(fw).max(1) as u16, th.div_ceil(fh).max(1) as u16)
}

/// The v6 composite as half-blocks, resampled EXACTLY ONCE (SQ-0973).
///
/// The encoding backends need [`v6_fit_source`]'s pre-scale because `Resize::Fit`
/// never grows: hand it the bare canvas and `needs_resize_pixels`' `min(box, image)`
/// collapses the composite to a native-sized cell footprint. Half-blocks needs none
/// of it, because half-blocks does not encode pixels at all — `Halfblocks::encode`
/// throws away `font_size` and resamples whatever it is given to exactly
/// `width x 2·height` samples, one per column and two per row. The pre-scale was
/// therefore magnifying the canvas to device pixels the backend was about to discard:
/// a 640x400 canvas in a 458x144-cell pane went up to 4580x2862 RGBA (52 MB, Nearest)
/// so that the crate could take it straight back down to 458x288 (Triangle). Two
/// resamples in opposite directions to land BELOW where the canvas started, and
/// Nearest-up-then-Triangle-down is the blur that combination always is.
///
/// So resample once, onto the sample grid itself, through [`resize_directional`] —
/// which picks its filter per axis by the direction that axis moves, and associates
/// alpha before it blends (SQ-0824/0827). The crate's own `resize_exact` then runs at
/// a 1:1 ratio, which is a bit-exact identity, so that filter choice is what reaches
/// the screen rather than being overwritten by the crate's unconditional Triangle.
///
/// This is half-blocks ONLY. Sixel, iTerm2 and kitty keep [`v6_fit_source`] exactly
/// as it was: they ship encoded pixels down the wire, the cap is a real budget for
/// them, and `Fit` plus a pre-scale is the right shape.
pub(super) fn v6_halfblocks_protocol(
    canvas: &image::RgbaImage,
    box_w: u32,
    box_h: u32,
    fs: ratatui_image::FontSize,
    lock: Option<f32>,
) -> Option<Protocol> {
    use ratatui_image::protocol::halfblocks::Halfblocks;
    let cells = v6_halfblocks_grid(canvas.dimensions(), box_w, box_h, fs, lock);
    let grid = resize_directional(canvas, u32::from(cells.width), u32::from(cells.height) * 2);
    let hb = Halfblocks::new(image::DynamicImage::ImageRgba8(grid), cells).ok()?;
    Some(Protocol::Halfblocks(hb))
}

/// A completed v6 raster encode (SQ-0469): the uploaded protocol plus the key it
/// was built for (`gen` + pane cell size) and the native canvas extent (for the
/// click map). Always rendered once present — a stale entry is shown until a
/// fresher encode lands, so the pane never flickers to blank on a change.
pub(super) struct V6Ready {
    pub(super) gen: u64,
    pub(super) area_w: u16,
    pub(super) area_h: u16,
    pub(super) proto: Protocol,
    /// The canvas encoded, and the game's own screen inside it — see
    /// [`V6ClickMap::canvas`] / [`V6ClickMap::screen`]. They differ only for an
    /// extended frame (SQ-1032).
    pub(super) canvas: (u16, u16),
    pub(super) screen: (u16, u16),
    /// Where the composite itself lies inside the cell rect the protocol reports,
    /// in device pixels: `(off_x, off_y, w, h)` (SQ-1081).
    ///
    /// It is not the whole of that rect, because the rect is `ceil`ed onto the cell
    /// grid and the picture is centred in what that leaves — see [`v6_pad_to_cells`].
    /// The click map has to invert through the PICTURE and not through the box, or a
    /// click reads a game pixel up to a cell out. (It used to invert through the box
    /// and be right anyway, on the magnifying arm only, because the terminal was
    /// stretching the picture to fill it. On the shrinking arm, where the protocol
    /// has always padded, it was quietly wrong by the same amount.)
    pub(super) pic: (u32, u32, u32, u32),
    /// The kitty image id this composite was last PLACED as, read back off the
    /// placement (SQ-0753). `None` under a non-kitty protocol, and until the first
    /// [`GraphicsRender::redraw_v6`]. Without it the full-frame composite — the
    /// single largest upload lanthorn makes, 2.8 MB on Journey — can only be
    /// forgotten, never freed.
    pub(super) placed_id: Option<u32>,
    /// SQ-1338: how long this encode's own resize step took — `None` on the
    /// half-blocks arm, which resolves straight onto the cell grid and has no
    /// separate resize to time (see [`GraphicsRender::encode_v6`]).
    pub(super) resize: Option<std::time::Duration>,
    /// SQ-1338: how long the `picker.new_protocol*` call took — deflate and
    /// base64 fused into one step inside `ratatui-image`, which this file
    /// cannot time separately without patching the fork.
    pub(super) encode: std::time::Duration,
}

/// The worker-thread handle for an in-flight v6 raster encode (SQ-0469). The
/// heavy resize + PNG/protocol encode (tens–hundreds of ms) runs off the UI
/// thread and yields a ready-to-render [`V6Ready`]. Coalesced: only one runs at a
/// time, and `poll_v6_job` installs its result (see `spawn_v6_encode`).
pub(super) type V6Job = std::thread::JoinHandle<Option<V6Ready>>;


/// One band encode staged for the background worker (SQ-1188): the finished
/// (sealed) band image, the cache key it will land under, the content hash it
/// answers for, and the kitty id it re-transmits to (SQ-0996).
pub(super) struct PendingBand {
    pub(super) key: BandKey,
    pub(super) band: Rect,
    pub(super) hash: u64,
    pub(super) img: image::DynamicImage,
    pub(super) reuse: Option<u32>,
}

/// One band the worker finished encoding (SQ-1188). `proto` is `None` when the
/// encode failed — the staging is then un-marked so the next frame retries.
pub(super) struct BandEncoded {
    pub(super) key: BandKey,
    pub(super) band: Rect,
    pub(super) hash: u64,
    pub(super) reuse: Option<u32>,
    pub(super) proto: Option<Protocol>,
    /// SQ-1338: how long the `picker.new_protocol*` call took on the worker.
    pub(super) encode: std::time::Duration,
}

/// SQ-1188: whether this backend's band encodes are worth a worker thread.
/// Kitty and iTerm2 pay zlib-L6 + base64 per encode and sixel pays icy_sixel's
/// quantization; half-blocks "encodes" straight into cells with no compression
/// stage at all, so the worker would buy a frame of latency and save nothing —
/// it (and therefore every cell-buffer test harness) stays synchronous.
pub(super) fn band_encode_offthread(picker: &Picker) -> bool {
    !matches!(picker.protocol_type(), ratatui_image::picker::ProtocolType::Halfblocks)
}


/// One unit of graphics-protocol traffic (SQ-0590).
///
/// The pixel path's failure modes are all about WHICH of these happened: an image
/// that vanished is a stale [`Place`](GraphicsOp::Place) with no re-[`Upload`](GraphicsOp::Upload),
/// a slow frame is an `Upload` that should have been a [`Reuse`](GraphicsOp::Reuse),
/// and a leak is an `Upload` with no matching [`Drop`](GraphicsOp::Drop). None of
/// that is visible in the rendered `Buffer` — under the kitty protocol the cells
/// carry only placeholder glyphs — so it has to be recorded as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphicsOp {
    /// Pixels were encoded and handed to the terminal.
    Upload { target: GraphicsTarget, id: Option<u32>, cells: (u16, u16) },
    /// The terminal already had these exact pixels; an existing upload was re-placed
    /// instead of re-sent (the kitty per-window cache, or a fresh band).
    Reuse { target: GraphicsTarget, id: Option<u32> },
    /// An image was placed on screen at a cell rect `(x, y, w, h)`.
    Place { target: GraphicsTarget, at: (u16, u16, u16, u16) },
    /// An upload was released — a kitty `a=d` delete, or a cached band dropped.
    /// The terminal frees the image AND its placements, so anything dropped must
    /// be re-uploaded before it can be seen again (SQ-0587).
    Drop { target: GraphicsTarget },
}

/// What a [`GraphicsOp`] acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GraphicsTarget {
    /// A v6 graphics window, by window number.
    Window(u32),
    /// One chrome-ring band, by its cell rect `(x, y, w, h)`.
    Band(u16, u16, u16, u16),
    /// The v6 RASTER path's full-frame composite (SQ-0747). There is only ever one,
    /// and it covers the whole pane — so a frame that stops drawing it without
    /// dropping it strands an image over everything the next path draws. Journey
    /// boots through this path (`raster x2 · hybrid-ring x27`) before its menu
    /// switches to the ring, and the transition was invisible to the op log until
    /// this target existed.
    Raster,
}

impl GraphicsOp {
    pub fn target(&self) -> GraphicsTarget {
        match *self {
            GraphicsOp::Upload { target, .. }
            | GraphicsOp::Reuse { target, .. }
            | GraphicsOp::Place { target, .. }
            | GraphicsOp::Drop { target } => target,
        }
    }
    pub fn is_upload(&self) -> bool {
        matches!(self, GraphicsOp::Upload { .. })
    }
    pub fn is_place(&self) -> bool {
        matches!(self, GraphicsOp::Place { .. })
    }
}

/// Cap on the retained op log. A v6 frame records a few dozen ops and clears at
/// the top of the next one ([`GraphicsRender::begin_band_log`]); this only bounds
/// the non-v6 paths, which have no frame boundary to clear on.
pub(super) const OPS_MAX: usize = 512;

/// Build a [`Picker`] that speaks the kitty graphics protocol at a fixed cell
/// size, without querying a terminal (SQ-0590).
///
/// Test harnesses otherwise use `Picker::halfblocks()`, which reports 10×20 and
/// draws cells — so every band cache, upload, eviction and placement in this file
/// was unreachable from the suite, and the whole class of v6 graphics regressions
/// could only be found by playing. Pass the cell size the case needs (14×28 is a
/// typical kitty terminal); the pixel path is sensitive to it, since art scales by
/// pixel while text places by cell.
///
/// `from_fontsize` is deprecated upstream in favour of `from_query_stdio`, which
/// needs a real terminal — exactly what a headless test does not have.
pub fn kitty_picker(cell_w: u16, cell_h: u16) -> Picker {
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(cell_w, cell_h));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    picker
}

impl From<&Picker> for GameImaging {
    /// The plain facts core code reads off a picker (SQ-1804). The only
    /// conversion; `AppState::set_game_picker` calls it so the picker and the
    /// value derived from it cannot disagree.
    fn from(picker: &Picker) -> Self {
        use ratatui_image::picker::ProtocolType;
        let f = picker.font_size();
        GameImaging {
            cell_px: (f.width, f.height),
            protocol: match picker.protocol_type() {
                ProtocolType::Halfblocks => ImageProtocol::Halfblocks,
                ProtocolType::Sixel => ImageProtocol::Sixel,
                ProtocolType::Kitty => ImageProtocol::Kitty,
                ProtocolType::Iterm2 => ImageProtocol::Iterm2,
            },
        }
    }
}

/// The kitty image backing one graphics window (SQ-0520/SQ-0995).
pub(super) struct KittyWindowImage {
    /// Canvas version the upload was last reconciled against. Hashing a canvas
    /// costs a pass over every pixel, so it only happens when the game actually
    /// drew something — not on every frame.
    pub(super) version: u64,
    pub(super) w: u16,
    pub(super) h: u16,
    /// The id this window's pixels live under in the terminal, allocated once
    /// when the entry is created and STABLE for as long as the window keeps this
    /// cell size (SQ-0995).
    ///
    /// The id is a PER-CELL value — `kitty_place_rows` writes its low 24 bits into
    /// every placeholder cell's foreground and its high byte into the third
    /// diacritic — so changing it dirties the whole grid. Keying an id on the
    /// canvas's CONTENT, as this did before, therefore made one changed pixel
    /// repaint every cell of the window: measured on golden_baton.blb at 230×64,
    /// a 228×16 window cost 42,207 bytes for a frame whose compressed image was
    /// 2,208. Holding the id still and replacing the data behind it costs the
    /// image and nothing else.
    pub(super) id: u32,
    /// The transmit escape, prepended to the first placeholder row of the next
    /// emitted frame, then dropped — the terminal keeps the image by id after that.
    pub(super) pending_transmit: Option<String>,
    /// Hash of the pixels currently uploaded under [`Self::id`], or `None` while
    /// nothing has been transmitted yet. A repaint that lands on identical pixels
    /// (advent.blb's toolbar redraws itself from scratch to release a button)
    /// re-places the id and sends nothing; `None` also tells
    /// [`GraphicsRender::queue_kitty_deletes`] there is nothing in the terminal to
    /// free.
    pub(super) uploaded: Option<u64>,
}


impl GraphicsRender {
    /// [`Self::render`] for a Glulx design-mode canvas (SQ-1703 P3): the canvas
    /// is STRETCHED to exactly `area` per axis, aspect not honoured — the
    /// whole design frame fills the pane.
    ///
    /// EVERY backend gets a canvas already resampled to the area's device-pixel
    /// box, kitty included. Kitty's `r`/`c` on a unicode-placeholder placement
    /// does not stretch: the protocol says "The image will eventually be fit to
    /// the specified rectangle, its aspect ratio preserved"
    /// (<https://sw.kovidgoyal.net/kitty/graphics-protocol/>, Unicode
    /// placeholders), so a raw 640x58 canvas in a 160x2-cell box is drawn
    /// aspect-fit and centred, short of its window. Handing the terminal a canvas
    /// whose aspect already equals the box's makes that fit a no-op. The other
    /// backends fit by aspect too, so they need the same pre-stretch. The
    /// resample is cached per window ([`Self::stretch_cache`]).
    pub fn render_stretched(&mut self, picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style, buf: &mut Buffer) {
        let kitty = picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty;
        let fresh = !kitty
            && matches!(self.cache.get(&gw.win),
                Some((v, w, h, _)) if *v == gw.version && *w == area.width && *h == area.height);
        if area.width == 0 || area.height == 0 || fresh {
            return self.render(picker, gw, area, letterbox, buf);
        }
        let fs = picker.font_size();
        let (fw, fh) = (fs.width.max(1), fs.height.max(1));
        let key = ((area.width, area.height), (fw, fh));
        let canvas = match self.stretch_cache.get(&gw.win) {
            Some((v, a, f, c)) if *v == gw.version && (*a, *f) == key => c.clone(),
            _ => {
                let c = std::sync::Arc::new(resize_directional(
                    &gw.canvas,
                    area.width as u32 * fw as u32,
                    area.height as u32 * fh as u32,
                ));
                self.stretch_cache.insert(gw.win, (gw.version, key.0, key.1, c.clone()));
                c
            }
        };
        let stretched = GraphicsWindow { canvas, upscale: true, ..gw.clone() };
        self.render(picker, &stretched, area, letterbox, buf)
    }

    pub fn render(&mut self, picker: &Picker, gw: &GraphicsWindow, area: Rect, letterbox: Style, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Letterbox fill behind the fitted canvas.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(letterbox);
                }
            }
        }
        // Kitty terminals: place the canvas ourselves with an explicit r×c grid,
        // so the terminal scales it to exactly the window's cell rect. The
        // ratatui-image placement omits r/c, leaving the on-screen extent up to
        // the terminal's own cell-pixel accounting — on displays where images
        // render at device pixels but cell metrics report logical points
        // (Ghostty/macOS 2×), the image covered only part of the window and
        // mouse clicks mapped to the wrong game pixels. (SQ-0520)
        if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty {
            self.render_kitty_virtual(gw, area, window_wire(picker), buf);
            return;
        }
        let fresh = matches!(self.cache.get(&gw.win),
            Some((v, w, h, _)) if *v == gw.version && *w == area.width && *h == area.height);
        if !fresh {
            let img = image::DynamicImage::ImageRgba8((*gw.canvas).clone());
            // `upscale` blows a small canvas up to fill the window (aspect
            // preserved → crisp pixel art); otherwise the canvas stays at native
            // size, centered. Scott room pictures want the former.
            //
            // Only the NON-kitty backends reach here — kitty placed the canvas
            // itself and returned, above — so this is the sixel/iTerm2/halfblocks
            // resample, and it went through the crate's default Nearest in BOTH
            // directions. A canvas larger than its window (advent.blb's 1104-px
            // toolbar in a narrow pane) was minified by dropping columns (SQ-0829).
            // Half-blocks then does it ONCE, onto its own sample grid (SQ-0979).
            match fitted_protocol(picker, &img, Size::new(area.width, area.height), gw.upscale) {
                Some(p) => { self.cache.insert(gw.win, (gw.version, area.width, area.height, p)); }
                None => return,
            }
        }
        if let Some((_, _, _, proto)) = self.cache.get(&gw.win) {
            let sz = proto.size();
            let w = sz.width.min(area.width);
            let h = sz.height.min(area.height);
            let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
            place_protocol(proto, dest, buf);
        }
    }

    /// Drop cache entries for windows no longer live (evicts on close; bounds growth).
    ///
    /// A dropped kitty entry's uploads are DELETED in the terminal, not merely
    /// forgotten (SQ-0637) — see [`GraphicsRender::queue_kitty_deletes`].
    pub fn retain_live(&mut self, live: &std::collections::HashSet<u32>) {
        self.cache.retain(|win, _| live.contains(win));
        self.stretch_cache.retain(|win, _| live.contains(win));
        self.cell_memo.retain(|win, _| live.contains(win));
        let dead: Vec<u32> = self.kitty_wins.keys().copied().filter(|w| !live.contains(w)).collect();
        for win in dead {
            if let Some(entry) = self.kitty_wins.remove(&win) {
                self.queue_kitty_deletes(&entry, win);
            }
        }
    }

    /// Record a transmit of `pixels` bytes under `id` in [`Self::outstanding`]
    /// (SQ-1201) and mirror the map's live size/sum into `uploads.stranded_*`.
    /// A no-op for `pixels == 0` — a re-place of already-uploaded content, which
    /// leaves whatever the id already holds (or does not) exactly as it was.
    pub(super) fn note_upload_id(&mut self, id: Option<u32>, pixels: u64) {
        if let Some(id) = id {
            if pixels > 0 {
                self.outstanding.insert(id, pixels);
            }
        }
        self.sync_stranded();
    }

    /// Record an `a=d` delete for `id` in [`Self::outstanding`] (SQ-1201):
    /// `uploads.deletes` counts the command regardless, and `uploads.freed_pixels`
    /// is credited only when `id` was still outstanding — a delete for an id this
    /// struct never transmitted, or already freed, is counted but not credited.
    pub(super) fn note_delete_id(&mut self, id: Option<u32>) {
        if let Some(id) = id {
            self.uploads.deletes += 1;
            if let Some(px) = self.outstanding.remove(&id) {
                self.uploads.freed_pixels += px;
            }
        }
        self.sync_stranded();
    }

    /// Mirror [`Self::outstanding`]'s current size/sum into `uploads`. A snapshot,
    /// not a running total — called after every [`Self::note_upload_id`]/
    /// [`Self::note_delete_id`] so `stranded_uploads`/`stranded_pixels` always read
    /// "as of now" rather than something `UploadBytes::add` accumulated.
    pub(super) fn sync_stranded(&mut self) {
        self.uploads.stranded_uploads = self.outstanding.len() as u64;
        self.uploads.stranded_pixels = self.outstanding.values().sum();
    }

    /// Queue an `a=d,d=I` delete for the id an abandoned [`KittyWindowImage`] still
    /// holds in the terminal, and record a [`GraphicsOp::Drop`] for it (SQ-0637).
    /// `d=I` frees the image data AND its placements, so nothing deleted here can be
    /// re-placed — which is correct: the entry that knew this id is gone.
    ///
    /// One id, not a cache generation: since SQ-0995 a window owns exactly one
    /// upload at a given cell size and replaces the data behind it in place. An
    /// entry that never transmitted (`uploaded` is `None`) names nothing the
    /// terminal is holding, so it queues nothing.
    pub(super) fn queue_kitty_deletes(&mut self, entry: &KittyWindowImage, win: u32) {
        use std::fmt::Write as _;
        if entry.uploaded.is_none() {
            return;
        }
        let id = entry.id;
        write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
        self.note_delete_id(Some(id));
        self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Window(win) });
    }

    /// Queue an `a=d,d=I` delete for ONE abandoned `ratatui-image` upload (SQ-0753).
    ///
    /// Everything drawn through a [`Protocol`] — every chrome-ring band, the v6
    /// raster composite — was uploaded by `ratatui-image`, which never deletes:
    /// dropping the struct forgets the id on our side and leaves the pixels in the
    /// terminal for ever. Only the graphics WINDOWS, whose ids this file allocates
    /// itself, were ever freed (SQ-0637). Measured on Journey release 30 over five
    /// keystrokes: 4.1 MB uploaded, 0 bytes freed, because a band that re-encodes
    /// strands its predecessor and the boot raster frame is abandoned wholesale when
    /// the ring takes over. Kitty evicts by LRU and evicts images that are CURRENTLY
    /// PLACED, so an unbounded pile of orphans can blank a live one.
    ///
    /// `None` (a non-kitty protocol, or a cache entry never placed) is a no-op: there
    /// is nothing the terminal is holding that we could name. `d=I` frees the data
    /// and its placements together, which is right — the entry that knew this id is
    /// gone, so nothing can re-place it.
    pub(super) fn queue_protocol_delete(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
        }
        self.note_delete_id(id);
    }

    /// Queue `a=d,d=I` deletes for uploads a SIBLING cache owns (SQ-1190): the
    /// inline-image transcript bands and the picker's cover/gallery tiles are
    /// placed through [`place_protocol`], exactly like a chrome band, but they
    /// live in `InlineImageRender`/`cover.rs`, not here — `queue_protocol_delete`
    /// is private, so this is the seam that lets their eviction share this queue
    /// and [`Self::flush_kitty_deletes`]'s no-flicker sequencing instead of
    /// duplicating either.
    pub(crate) fn queue_external_deletes(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.queue_protocol_delete(Some(id));
        }
    }

    /// Queue a delete for an upload that is being REPLACED IN PLACE — one still
    /// covering the rect its successor is about to take (SQ-0817).
    ///
    /// [`Self::queue_protocol_delete`] is right for an upload nothing on screen
    /// depends on: a closed window, a band evicted from the ring, the composite
    /// abandoned at the raster→ring transition. Freeing those early is free. This one
    /// is the opposite case — the terminal is drawing these pixels right now — so the
    /// escape goes to [`Self::deletes_after_place`] and rides out behind the
    /// placement that covers the rect again.
    pub(super) fn queue_protocol_delete_after_place(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.deletes_after_place, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
                .expect("write to String");
        }
        self.note_delete_id(id);
    }

    /// Flush any queued kitty deletes into `buf` when no graphics window will place
    /// this frame (SQ-0637) — closing the LAST graphics window is exactly the case
    /// that leaks, and it leaves no placement to piggyback on.
    ///
    /// The escapes are prepended to a cell's existing symbol (keeping its glyph, with
    /// the width forced to 1 so the escape's own "width" never shifts the row) — the
    /// same trick `kitty_place_rows` uses for a transmit, so the bytes ride the normal
    /// ratatui diff instead of a racing stdout write. Only a plain single-column ASCII
    /// cell is used: one already carrying an escape (a kitty placeholder row), one
    /// marked `Skip`, or a wide glyph belongs to something whose width/placement must
    /// not be disturbed. If no such cell exists the deletes stay queued for a later
    /// frame — a delete is never dropped, only deferred.
    pub fn flush_kitty_deletes(&mut self, area: Rect, buf: &mut Buffer) {
        use ratatui::buffer::CellDiffOption;
        if self.pending_deletes.is_empty() || area.width == 0 || area.height == 0 {
            return;
        }
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let Some(cell) = buf.cell_mut((x, y)) else { continue };
                let plain = cell.diff_option == CellDiffOption::None
                    && cell.symbol().len() == 1
                    && cell.symbol().is_ascii()
                    && !cell.symbol().starts_with(char::is_control);
                if !plain {
                    continue;
                }
                let symbol = format!("{}{}", self.pending_deletes, cell.symbol());
                cell.set_symbol(&symbol)
                    .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
                self.pending_deletes.clear();
                return;
            }
        }
    }

    /// Render a graphics window's canvas through kitty unicode placeholders
    /// with an EXPLICIT `r×c` grid on the virtual placement (SQ-0520): the
    /// terminal scales the image to exactly the placeholder rect, so the
    /// visible image always fills the window's cells — independent of any
    /// logical-vs-device pixel mismatch — and the cell→game-pixel mouse
    /// mapping in `glk_mouse_target` stays truthful.
    ///
    /// THE ID IS STABLE AND THE DATA BEHIND IT IS REPLACED (SQ-0995). A window
    /// takes one image id when its entry is created and keeps it for as long as
    /// the window keeps that cell size; a changed canvas re-transmits to the SAME
    /// id. That is the whole reason the placeholder design pays off: the id lives
    /// in every cell (low 24 bits in the foreground, high byte in the third
    /// diacritic), so an id that changes dirties every cell of the window and
    /// ratatui's diff emits all of them. Allocating a fresh id per canvas — which
    /// this did until SQ-0995 — meant one changed pixel repainted the whole grid,
    /// and it did so on a CACHE HIT too, since re-placing a previously uploaded
    /// canvas also swaps the id back. Measured on golden_baton.blb at 230×64
    /// cells, a 228×16-cell room picture: 42,207 bytes for a changed frame whose
    /// compressed image was 2,208, and 39,859 bytes for a frame that transmitted
    /// nothing at all. With the id held still, both collapse to the image.
    ///
    /// The protocol licenses it: *"When re-transmitting image data for a specific
    /// id, the existing image and all its placements must be deleted"* — the data
    /// is replaced wholesale, and our `a=T,U=1,r,c` re-creates the window's
    /// anonymous placement in the same command, so the cells never stop resolving.
    /// The old image also stays on screen throughout, because a chunked transmit
    /// commits only on its final chunk: nothing blanks mid-transfer.
    ///
    /// A repaint that lands on identical pixels still costs nothing — advent.blb's
    /// toolbar redraws itself from scratch to press and release a button, bumping
    /// the version each time, and the release hashes equal to what is already
    /// uploaded (SQ-0564's insight, kept; SQ-0564's LRU of alternative uploads is
    /// gone, because an id you might place is an id in the cells).
    pub(super) fn render_kitty_virtual(
        &mut self,
        gw: &GraphicsWindow,
        area: Rect,
        wire: WindowWire,
        buf: &mut Buffer,
    ) {
        // A resize invalidates this window's upload: the placement's r×c grid is
        // baked into the transmission, so what the terminal holds cannot be
        // re-placed at the new size. Start the window over — and DELETE the upload
        // being abandoned, or every resize would strand a generation in the
        // terminal (SQ-0637).
        let previous = self.kitty_wins.remove(&gw.win);
        let mut entry = match previous {
            Some(e) if e.w == area.width && e.h == area.height => e,
            stale => {
                if let Some(stale) = stale {
                    self.queue_kitty_deletes(&stale, gw.win);
                }
                // Private id range, disjoint from ratatui-image's random ids in
                // practice. The top byte stays 0, which is what keeps every id this
                // path allocates nameable by `kitty_place_rows` — it now encodes the
                // byte rather than assuming it (SQ-0772), but the diacritic table
                // tops out at 296 and an id above that could not be placed at all.
                self.next_kitty_id = self.next_kitty_id.wrapping_add(1) & 0x000F_FFFF;
                KittyWindowImage {
                    version: 0,
                    w: area.width,
                    h: area.height,
                    id: 0x00B0_0000 | self.next_kitty_id,
                    pending_transmit: None,
                    uploaded: None,
                }
            }
        };
        if entry.uploaded.is_none() || entry.version != gw.version {
            entry.version = gw.version;
            let hash = canvas_hash(&gw.canvas);
            if entry.uploaded == Some(hash) {
                // The terminal already holds these exact pixels under this id:
                // re-place it and transmit nothing.
                self.note_op(GraphicsOp::Reuse {
                    target: GraphicsTarget::Window(gw.win),
                    id: Some(entry.id),
                });
            } else {
                let (transmit, timing) =
                    kitty_transmit_virtual_wire(&gw.canvas, entry.id, area.height, area.width, wire);
                if let Some(d) = timing.shm {
                    self.encode_timings.window_shm.record(d);
                }
                if let Some(d) = timing.deflate {
                    self.encode_timings.window_deflate.record(d);
                }
                if let Some(d) = timing.base64 {
                    self.encode_timings.window_base64.record(d);
                }
                let measured = measure_transmit(&transmit);
                self.note_upload_id(Some(entry.id), measured.pixels);
                self.uploads.add(measured);
                entry.pending_transmit = Some(transmit);
                entry.uploaded = Some(hash);
                self.note_op(GraphicsOp::Upload {
                    target: GraphicsTarget::Window(gw.win),
                    id: Some(entry.id),
                    cells: (area.width, area.height),
                });
            }
        }
        // Queued deletes (a closed window, or this window's pre-resize generation)
        // ride ahead of this frame's transmit, in the same batch (SQ-0637). They only
        // ever name ids no entry can place any more, so freeing them before the
        // placement below cannot blank anything still on screen.
        let mut batch = std::mem::take(&mut self.pending_deletes);
        batch.push_str(entry.pending_transmit.take().as_deref().unwrap_or(""));
        let transmit = (!batch.is_empty()).then_some(batch);
        kitty_place_rows(entry.id, transmit.as_deref(), area, buf);
        self.note_op(GraphicsOp::Place {
            target: GraphicsTarget::Window(gw.win),
            at: (area.x, area.y, area.width, area.height),
        });
        self.kitty_wins.insert(gw.win, entry);
    }

    /// How many images this window is holding in the terminal (0 before its first
    /// transmit, 1 thereafter) and the id they live under (observability hook,
    /// SQ-0564/SQ-0995). The count is the thing worth asserting: an id that is
    /// replaced in place can never grow it.
    #[cfg(all(test, feature = "t-render"))]
    pub(super) fn kitty_uploads(&self, win: u32) -> Option<(usize, u32)> {
        self.kitty_wins.get(&win).map(|e| (usize::from(e.uploaded.is_some()), e.id))
    }

    /// The kitty delete escapes waiting to be written to the terminal (SQ-0637).
    #[cfg(all(test, feature = "t-render"))]
    pub(super) fn queued_deletes(&self) -> &str {
        &self.pending_deletes
    }

    /// The deletes waiting to ride out BEHIND the placement that supersedes them
    /// (SQ-0817). Separate from [`Self::queued_deletes`] because "nothing was
    /// freed" is only true when both are empty (SQ-0996).
    #[cfg(all(test, feature = "t-render"))]
    pub(super) fn queued_deletes_after_place(&self) -> &str {
        &self.deletes_after_place
    }

    /// What each cache keyed on cell geometry is currently holding, for SQ-0988:
    /// `(non-kitty window protocols, chrome bands, kitty window uploads,
    /// a raster composite?)`. One accessor rather than four because the whole
    /// question is which of them survive an invalidation and which must not.
    #[cfg(all(test, feature = "t-render"))]
    pub(super) fn cell_keyed_cache_sizes(&self) -> (usize, usize, usize, bool) {
        (self.cache.len(), self.chrome_bands.len(), self.kitty_wins.len(), self.v6.is_some())
    }

    /// Resize + encode a native v6 canvas into a terminal image protocol,
    /// upscaled (Nearest → crisp pixel art) to fill `area`'s device pixels with
    /// aspect preserved, capped at whatever this backend's [`v6_upscale_cap`] is.
    /// Pure/self-contained so it can run on a worker thread (SQ-0469). Returns
    /// `None` if the protocol encode fails.
    ///
    /// The backend is threaded no further than this: the `picker` is already the
    /// thing that knows it, and is already here (SQ-0964).
    ///
    /// Half-blocks takes its own arm (SQ-0973) and lands on the same cell rect by a
    /// single resample — see [`v6_halfblocks_protocol`] for why the pre-scale every
    /// other backend needs is pure waste there.
    ///
    /// `reuse` is the kitty image id the composite currently on screen lives under,
    /// and the new encode goes out under it (SQ-0996). The composite covers the
    /// whole pane — 3,680 cells at 117x64 — and the id is written into every one of
    /// them, so an encode under a fresh id repaints the pane in cells on top of
    /// sending the picture. Measured on Journey r83 in raster mode, one changed
    /// frame: 48,742 bytes for a 7,668-byte image. `None` on the first encode after
    /// boot or after [`Self::invalidate_v6`] (nothing to reuse — and the delete that
    /// abandonment queued means there had better not be), and under every backend
    /// with no addressable id, where `placed_id` is never anything else.
    pub(super) fn encode_v6(
        picker: &Picker,
        canvas: &image::RgbaImage,
        gen: u64,
        area: Rect,
        frame: RasterFrame,
        reuse: Option<u32>,
    ) -> Option<V6Ready> {
        let lock = frame.lock;
        let fs = picker.font_size();
        let box_w = area.width as u32 * fs.width.max(1) as u32;
        let box_h = area.height as u32 * fs.height.max(1) as u32;
        let (proto, pic, resize, encode) =
            if picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks {
                // Half-blocks never had the stretch to fix: `Halfblocks::encode` resolves
                // the image onto the cell grid itself, so there is no pixel size for a
                // terminal to disagree with. Its own arm builds the grid exactly, and its
                // picture therefore IS the whole cell rect. No separate resize step to
                // time (SQ-1338): the resample happens inside this one call.
                let t0 = std::time::Instant::now();
                let proto = v6_halfblocks_protocol(canvas, box_w, box_h, fs, lock)?;
                let encode = t0.elapsed();
                let sz = proto.size();
                let box_px = (
                    0,
                    0,
                    u32::from(sz.width) * u32::from(fs.width.max(1)),
                    u32::from(sz.height) * u32::from(fs.height.max(1)),
                );
                (proto, box_px, None, encode)
            } else {
                let t0 = std::time::Instant::now();
                let (img, fit) = v6_fit_source(canvas, box_w, box_h, lock, v6_upscale_cap(picker));
                // Out to whole cells, so the terminal blits the composite 1:1 instead of
                // resampling it into a box up to a cell bigger than itself (SQ-1081).
                let (img, pic) = v6_pad_to_cells(img, box_w, box_h, fs);
                let resize = t0.elapsed();
                let img = image::DynamicImage::ImageRgba8(img);
                let size = Size::new(area.width, area.height);
                let t1 = std::time::Instant::now();
                let proto = match reuse {
                    Some(id) if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty => {
                        kitty_protocol_with_id(picker, img, size, fit, id).ok()?
                    }
                    _ => picker.new_protocol(img, size, fit).ok()?,
                };
                let encode = t1.elapsed();
                (proto, pic, Some(resize), encode)
            };
        Some(V6Ready {
            pic,
            gen,
            area_w: area.width,
            area_h: area.height,
            proto,
            canvas: (canvas.width() as u16, canvas.height() as u16),
            screen: frame.native,
            // The id this composite is already placed under, carried across the
            // re-encode: `redraw_v6` re-confirms it off the placement it writes.
            placed_id: reuse,
            resize,
            encode,
        })
    }

    /// Whether the caller should (re)build the native v6 canvas and spawn an
    /// encode for `(gen, area)` this frame (SQ-0469). False when the last-ready
    /// encode already matches (nothing changed) OR a background encode is already
    /// in flight (coalesced — one at a time; the next frame after it lands
    /// respawns for the current generation). The expensive canvas BUILD is thus
    /// skipped entirely on an unchanged frame — no rebuild, no pixel hash.
    pub fn v6_wants_build(&self, gen: u64, area: Rect) -> bool {
        if area.width == 0 || area.height == 0 {
            return false;
        }
        if self.v6_job.is_some() {
            return false;
        }
        !matches!(&self.v6, Some(r) if r.gen == gen && r.area_w == area.width && r.area_h == area.height)
    }

    /// Spawn the background resize+encode for a freshly built native `canvas`
    /// (SQ-0469). Caller must have checked [`v6_wants_build`]; this replaces any
    /// (already-absent) job. The UI thread never blocks on the encode — the
    /// worker's result is installed by [`poll_v6_job`] — with one exception:
    /// when there is NO last-ready composite at all (first raster frame after
    /// boot or after [`invalidate_v6`]), the encode runs synchronously so the
    /// new screen shows this frame. Redrawing "the previous encode until the
    /// worker lands" is right during a burst of the same screen, but at a
    /// transition there is no honest previous frame — the pane would blank, or
    /// worse flash whatever the raster path last showed (SQ-0578: entering the
    /// rebus flashed the title splash or the on-screen map for a split second).
    pub fn spawn_v6_encode(
        &mut self,
        picker: &Picker,
        canvas: image::RgbaImage,
        gen: u64,
        area: Rect,
        // SQ-1032: the frame, not the bare lock. It carries the canvas height, the
        // magnification that height was derived from, and the GAME's own screen —
        // one subject, so a caller cannot supply the scale and omit the bound a
        // click has to be judged against (CLAUDE.md's refactoring policy).
        frame: RasterFrame,
    ) {
        if area.width == 0 || area.height == 0 || canvas.width() == 0 || canvas.height() == 0 {
            return;
        }
        // The id the composite on screen lives under, so the re-encode replaces its
        // pixels rather than moving to a new id and repainting the pane's cells
        // (SQ-0996). `None` when there is no composite yet, which is the same
        // branch that has to encode synchronously.
        let reuse = self.v6.as_ref().and_then(|r| r.placed_id);
        if self.v6.is_none() {
            self.v6 = Self::encode_v6(picker, &canvas, gen, area, frame, None);
            if let Some(r) = &self.v6 {
                if let Some(d) = r.resize {
                    self.encode_timings.raster_resize.record(d);
                }
                self.encode_timings.raster_encode.record(r.encode);
            }
            return;
        }
        let picker = picker.clone();
        self.v6_job =
            Some(std::thread::spawn(move || Self::encode_v6(&picker, &canvas, gen, area, frame, reuse)));
    }

    /// Drop the last-ready v6 raster composite (and detach any in-flight encode,
    /// discarding its result). Called by the HYBRID band path on every frame it
    /// renders WITHOUT the raster composite: once the pane has shown chrome-band
    /// frames, the cached composite is stale content from another screen, and a
    /// later fall-through to raster (a full-screen picture takeover, SQ-0570)
    /// must not flash it while the new encode is in flight (SQ-0578). The next
    /// raster frame re-encodes synchronously (see [`spawn_v6_encode`]).
    pub fn invalidate_v6(&mut self) {
        // The composite the terminal was last pointed at is being abandoned; record
        // it, so "placed on the previous frame, neither re-placed nor dropped on this
        // one" is answerable for the raster→ring transition too (SQ-0747).
        if let Some(old) = self.v6.take() {
            // …and free it in the terminal, which forgetting it here does not do
            // (SQ-0753). This is the transition Journey makes two frames into its
            // boot, and the abandoned composite is 2.8 MB.
            self.queue_protocol_delete(old.placed_id);
            self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Raster });
        }
        self.v6_job = None;
    }

    /// SQ-0988: the terminal's CELL changed size. Throw away everything fitted
    /// against the old one.
    ///
    /// A resize normally changes the cell GRID, and every cache here is keyed on
    /// a rect in cells, so a resize invalidates them by not matching. A font-size
    /// change is the case that slips through: the pane can keep the same
    /// `width × height` in cells while every one of those cells becomes a
    /// different box in pixels, and a cache keyed in cells sees no change at all.
    ///
    /// What that leaves behind, cache by cache:
    ///
    /// * `cache` — the non-kitty per-window protocol, keyed `(version, cols,
    ///   rows)`. STALE: the protocol holds an image encoded for the old device
    ///   box, and nothing about the window changed to dislodge it.
    /// * `v6` — the raster composite, keyed `(gen, area_w, area_h)` in cells, so
    ///   [`Self::v6_wants_build`] answers "already have it" and the pane keeps
    ///   showing a picture resampled to the old pixel size. STALE, and the most
    ///   visible of the two.
    ///
    /// And four that are already safe — checked, not assumed, because most of
    /// them share the suspect key shape and only some of them are safe for the
    /// same reason:
    ///
    /// * `chrome_bands` — the key IS in cells, but the freshness HASH mixes in
    ///   `(bw, bh)` in device pixels, so a font change makes every band miss and
    ///   re-encode on its own.
    /// * `chrome_scaled` — keyed on the scaled dimensions, likewise device
    ///   pixels.
    /// * `kitty_wins` — keyed `(version, w, h)` in cells, exactly like `cache`,
    ///   and nonetheless immune: [`Self::render_kitty_virtual`] transmits the
    ///   canvas at its NATIVE size with an explicit `r×c` grid and lets the
    ///   terminal scale it to the cell rect (SQ-0520). It never reads
    ///   `picker.font_size()` at all, so there is nothing in that cache fitted
    ///   to a cell, and re-uploading would spend a full canvas to arrive at the
    ///   same pixels.
    /// * `cell_memo` (SQ-1200) — keyed `(version, area, force)` in cells, and
    ///   immune for the same reason as `kitty_wins`: [`classify_graphics_as_cells`]
    ///   reads only `gw.canvas`'s NATIVE pixel dimensions and `area`'s cell
    ///   count, never a device-pixel size or `picker.font_size()`, so its answer
    ///   for an unchanged `(version, area, force)` is unchanged by a font-size
    ///   change too.
    ///
    /// The two device-pixel caches are dropped anyway even though they would
    /// re-encode by themselves: they are cheap to rebuild, and a ring whose
    /// bands survived a font change while the composite behind them did not is a
    /// seam waiting to happen. `kitty_wins` and `cell_memo` are deliberately KEPT.
    ///
    /// Every drop frees its upload in the terminal rather than merely forgetting
    /// it (SQ-0753), which is why this delegates instead of clearing the maps.
    pub fn invalidate_cell_geometry(&mut self) {
        self.cache.clear();
        self.invalidate_v6();
        self.invalidate_chrome_bands();
        self.chrome_scaled = None;
        // The hybrid frame's generation key covers the font size, so a font
        // change would rebuild it anyway — dropped here too so a frame fitted
        // against the old cell can never outlive the band caches it replays
        // (SQ-1187).
        self.hybrid = None;
    }

    /// Poll the background v6 encode: if it finished, install its protocol as the
    /// new last-ready composite and return true (the caller should redraw). The
    /// result is always installed (even if the generation has since advanced) so
    /// the display keeps converging to the latest encoded frame during a burst;
    /// an out-of-date entry is still rendered until the next encode lands, so the
    /// pane never blanks. (SQ-0469)
    pub fn poll_v6_job(&mut self) -> bool {
        // SQ-1188: the chrome-band worker is polled on the same tick — one
        // caller (`AppState::poll_v6_encode_job`), two workers, either can
        // warrant the redraw.
        let bands = self.poll_band_job();
        let done = self.v6_job.as_ref().is_some_and(|j| j.is_finished());
        if !done {
            return bands;
        }
        let job = self.v6_job.take().expect("checked above");
        if let Ok(Some(ready)) = job.join() {
            // SQ-1338: fold the worker's own timings in on this (main) thread — the
            // worker measured them, but only the installer writes them, so the
            // per-frame path never takes a lock for this.
            if let Some(d) = ready.resize {
                self.encode_timings.raster_resize.record(d);
            }
            self.encode_timings.raster_encode.record(ready.encode);
            // The composite being replaced is a whole-pane upload; free it in the
            // terminal rather than letting the assignment orphan it (SQ-0753). A
            // raster-mode game re-encodes on every visible change, so this is the
            // heaviest leak in the app — scopa strands one full frame per move.
            //
            // It is also the composite the terminal is showing RIGHT NOW, and the
            // replacement is a whole pane of image data away, so the delete rides
            // BEHIND the placement that covers the pane again (SQ-0817).
            //
            // …but since SQ-0996 the replacement is usually the SAME id, re-transmitted
            // — and deleting that would free the image this frame is about to place.
            // Written as the comparison rather than as "never delete" because the
            // reuse can be absent (a non-kitty backend, a composite never placed) and
            // then the old rule still applies exactly.
            let reused = ready.placed_id;
            let stale = self.v6.replace(ready);
            let stale_id = stale.and_then(|r| r.placed_id);
            if stale_id != reused {
                self.queue_protocol_delete_after_place(stale_id);
            }
        }
        true
    }

    /// True while a background v6 encode is in flight (SQ-0469).
    pub fn v6_encode_in_flight(&self) -> bool {
        self.v6_job.is_some()
    }

    /// Render the last-ready v6 composite into `area`, centred/letterboxed, and
    /// record the click map (SQ-0469). No-op (leaves the pane blank) until the
    /// first encode has landed. Never re-encodes — the heavy work happened on the
    /// worker.
    pub fn redraw_v6(&mut self, picker: &Picker, area: Rect, buf: &mut Buffer) {
        let Some(ready) = &self.v6 else { return };
        let proto = &ready.proto;
        let sz = proto.size();
        let w = sz.width.min(area.width);
        let ht = sz.height.min(area.height);
        let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - ht) / 2, w, ht);
        let (canvas, screen) = (ready.canvas, ready.screen);
        let pic = ready.pic;
        // Queued deletes ride out on this placement, in the same batch (SQ-0637's
        // rule); deferred back to the queue if this backend has no row to carry them.
        // The supersede-deletes ride BEHIND it instead (SQ-0817) — see
        // [`Self::deletes_after_place`].
        let pending = std::mem::take(&mut self.pending_deletes);
        let after = std::mem::take(&mut self.deletes_after_place);
        let (placed_id, placed_bytes) = place_protocol_with(proto, dest, buf, &pending, &after);
        self.uploads.add(placed_bytes);
        self.note_upload_id(placed_id, placed_bytes.pixels);
        if placed_id.is_none() {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on these either:
            // hand them to the ordinary queue rather than stranding them.
            self.pending_deletes.push_str(&after);
        }
        // Learn the id this composite lives under in the terminal, so whoever
        // abandons it can free it (SQ-0753).
        if let Some(r) = &mut self.v6 {
            r.placed_id = placed_id;
        }
        // Record the letterbox geometry so a click in the pane can be mapped
        // back to a game pixel (Lane M). `dest`'s cells are the box the composite is
        // PLACED over; `pic` is where the composite itself lies inside that box, which
        // is what a click has to invert through (SQ-1081) — the ceil onto the cell grid
        // leaves up to a cell of margin, and the terminal no longer stretches the
        // picture across it.
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1), fs.height.max(1));
        self.note_op(GraphicsOp::Place {
            target: GraphicsTarget::Raster,
            at: (dest.x, dest.y, dest.width, dest.height),
        });
        self.last_v6_map = Some(V6ClickMap {
            pane_x: area.x,
            pane_y: area.y,
            cell_w: cw,
            cell_h: ch,
            img_x: (dest.x - area.x) as f32 * cw as f32 + pic.0 as f32,
            img_y: (dest.y - area.y) as f32 * ch as f32 + pic.1 as f32,
            img_w: pic.2 as f32,
            img_h: pic.3 as f32,
            canvas,
            screen,
            packed_text: Vec::new(),
        });
    }

    /// Drop cached chrome-band protocols whose band rect is not in `live` — called
    /// once per hybrid frame so a resize/layout change can't leave stale band
    /// uploads accumulating.
    /// Drop every cached chrome band, so the next draw re-encodes and re-PLACES all
    /// of them (SQ-0587). The cache's job is to skip re-uploading a band whose pixels
    /// have not changed — which is exactly wrong when the terminal has lost the
    /// placement rather than the band having changed: an overlay covered the pane, the
    /// v6 pixel path stood down while it was up, and on the way back every band is a
    /// cache HIT and nothing is sent. The image is gone from the screen and the cache
    /// believes it is still there.
    /// Start a fresh band log — and op log (SQ-0590) — for this frame.
    pub fn begin_band_log(&mut self) {
        self.band_log.clear();
        self.band_mags.clear();
        self.ops.clear();
        self.band_ground = None;
        self.band_replay = false;
    }

    /// SQ-1187: declare whether this frame REPLAYS an unchanged hybrid frame —
    /// see the field. Set by the hybrid draw half right after the frame gate,
    /// never anywhere else.
    pub fn set_band_replay(&mut self, on: bool) {
        self.band_replay = on;
    }

    /// Declare the ground this frame's chrome bands resolve their transparency
    /// onto — see [`Self::band_ground`]. `None` ships the alpha.
    pub fn set_band_ground(&mut self, ground: Option<image::Rgba<u8>>) {
        self.band_ground = ground;
    }

    /// Hand a finished band image to the encoder, resolving its transparency
    /// first when this frame named a ground (SQ-0944).
    pub(super) fn seal_band(&self, mut img: image::RgbaImage) -> image::DynamicImage {
        if let Some(page) = self.band_ground {
            crate::render::inline_image::flatten_onto(&mut img, page);
        }
        image::DynamicImage::ImageRgba8(img)
    }

    /// Record what magnification one band drew at (SQ-0898) — see [`Self::band_mags`].
    /// `src`/`dst` are the sizes that went into the resample, in native and device
    /// pixels; a zero on either side means there was nothing to measure.
    pub(super) fn note_band_mag(&mut self, band: Rect, fit: BandFit, src: (u32, u32), dst: (u32, u32)) {
        if src.0 == 0 || src.1 == 0 {
            return;
        }
        self.band_mags.push((band, fit, src, dst));
    }

    /// Record one unit of protocol traffic (SQ-0590), bounded by [`OPS_MAX`] so a
    /// path with no frame boundary cannot grow it without limit.
    pub(super) fn note_op(&mut self, op: GraphicsOp) {
        if self.ops.len() >= OPS_MAX {
            self.ops.remove(0);
        }
        self.ops.push(op);
    }

    /// The protocol traffic recorded since the last [`begin_band_log`] (SQ-0590).
    pub fn ops(&self) -> &[GraphicsOp] {
        &self.ops
    }

    /// Every target that was UPLOADED (not merely re-placed) since the last frame
    /// boundary — the question "was the terminal actually sent these pixels?".
    pub fn uploaded_targets(&self) -> std::collections::BTreeSet<GraphicsTarget> {
        self.ops.iter().filter(|o| o.is_upload()).map(|o| o.target()).collect()
    }

    /// Every target PLACED on screen since the last frame boundary — the question
    /// "what should be visible right now?", which a stale cache can answer wrongly
    /// only by omission (SQ-0587).
    pub fn placed_targets(&self) -> std::collections::BTreeSet<GraphicsTarget> {
        self.ops.iter().filter(|o| o.is_place()).map(|o| o.target()).collect()
    }

    pub fn invalidate_chrome_bands(&mut self) {
        let dropped: Vec<_> = self.chrome_bands.drain().map(|(k, (_, _, id))| (k, id)).collect();
        for ((_, x, y, w, h), id) in dropped {
            // Free the upload in the terminal, not merely the struct here (SQ-0753).
            self.queue_protocol_delete(id);
            self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Band(x, y, w, h) });
        }
        // SQ-1188: staged and in-flight encodes answered for entries that no
        // longer exist — cancel them (an in-flight batch is left to finish; its
        // results find their dirty marks gone and are dropped on install).
        self.band_pending.clear();
        self.band_dirty.clear();
        self.band_inflight.clear();
    }

    pub fn retain_chrome_bands(&mut self, live: &std::collections::HashSet<BandKey>) {
        let before: Vec<_> = self.chrome_bands.iter().map(|(k, (_, _, id))| (*k, *id)).collect();
        self.chrome_bands.retain(|k, _| live.contains(k));
        // SQ-0587: dropping a cached band drops its image PROTOCOL, and a graphics
        // protocol releases its placement when it goes (kitty deletes the image).
        // A band that merely survived is a cache HIT, so it would not re-upload and
        // its placement can go with the deleted ones — Arthur loses its header art
        // for a frame whenever the ring's band set changes shape, which is what a
        // restore and its first move do. Cheap and certain: when anything was
        // evicted, evict the rest too, so every surviving band re-encodes and
        // re-places on this frame. Costs one re-encode on the rare frames where the
        // ring's shape changes, and nothing at all on the common path.
        if self.chrome_bands.len() != before.len() {
            self.chrome_bands.clear();
            // Everything that was cached is gone — the ones `retain` evicted and
            // the survivors cleared with them — so every one of them must
            // re-upload before it can be seen again, and every one of them must be
            // freed in the terminal first (SQ-0753).
            for ((_, x, y, w, h), id) in before {
                self.queue_protocol_delete(id);
                self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Band(x, y, w, h) });
            }
        }
    }

    /// The kitty image id a cached chrome band is currently placed as, if any
    /// (observability hook, SQ-0753).
    #[cfg(all(test, feature = "t-render"))]
    pub(super) fn chrome_band_id(&self, key: BandKey) -> Option<u32> {
        self.chrome_bands.get(&key).and_then(|(_, _, id)| *id)
    }

    /// Snapshot the current chrome-band freshness hashes, keyed by band cell rect
    /// (observability hook, SQ-0514). A band whose hash is unchanged across two
    /// frames did NOT re-encode — used to confirm that a change confined to one
    /// band leaves the other bands' uploads fresh.
    pub fn chrome_band_hashes(&self) -> std::collections::HashMap<BandKey, u64> {
        self.chrome_bands.iter().map(|(k, (h, _, _))| (*k, *h)).collect()
    }

    /// The whole native `chrome_canvas` scaled to `sw × sh` device pixels
    /// (Nearest → crisp), memoised and shared across every band of a frame
    /// (SQ-0514). Rebuilt only when the canvas content or the scaled size
    /// changes, so the heavy resize runs at most once per changed frame; each
    /// band then crops its sub-rect from the returned image. The cropped pixels
    /// are byte-identical to resizing the whole canvas per band (it IS the same
    /// resize) — the only change is that the resize is no longer repeated.
    pub(super) fn scaled_chrome(&mut self, chrome_canvas: &image::RgbaImage, s: f32, sw: u32, sh: u32) -> &image::RgbaImage {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        chrome_canvas.as_raw().hash(&mut h);
        s.to_bits().hash(&mut h);
        (sw, sh).hash(&mut h);
        let key = h.finish();
        if !matches!(&self.chrome_scaled, Some((k, _)) if *k == key) {
            let scaled = resize_directional(chrome_canvas, sw, sh);
            self.chrome_scaled = Some((key, scaled));
        }
        &self.chrome_scaled.as_ref().expect("just inserted").1
    }

    /// Draw ONE chrome ring band (Lane H hybrid mode): the crop of the letterbox-
    /// scaled `chrome_canvas` lying under `band`'s device region, placed as a
    /// single image at the band's cell rect. `chrome_canvas` is the native
    /// game-pixel chrome composite; `scale` is the same [`uniform_scale`] the story
    /// viewport was mapped through, so the ring lines up pixel-exactly with the
    /// terminal story region it surrounds. `pane` is the whole v6 pane's cell rect
    /// (the band's coordinate origin). Cached per band on a content+scale hash.
    pub fn draw_chrome_band(
        &mut self,
        picker: &Picker,
        chrome_canvas: &image::RgbaImage,
        scale: &crate::render::v6_layout::Scale,
        pane: Rect,
        band: Rect,
        buf: &mut Buffer,
    ) {
        if band.width == 0 || band.height == 0 || chrome_canvas.width() == 0 || chrome_canvas.height() == 0 {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        // The band's device-pixel region, measured from the pane's top-left pixel.
        let rel_x0 = band.x.saturating_sub(pane.x) as u32 * cw;
        let rel_y0 = band.y.saturating_sub(pane.y) as u32 * ch;
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;
        // The scaled chrome canvas occupies [off_x, off_x + native_w·s) ×
        // [off_y, off_y + native_h·s) in that same pane-relative device space.
        let (nw, nh) = (chrome_canvas.width(), chrome_canvas.height());
        let sw = ((nw as f32 * scale.s).round() as u32).max(1);
        let sh = ((nh as f32 * scale.s).round() as u32).max(1);

        // The band reads scaled-canvas pixels [sx_lo, sx_hi) × [sy_lo, sy_hi)
        // (the crop loop below only touches sx/sy in-bounds of the scaled canvas).
        let sx_lo = (rel_x0 as i64 - scale.off_x as i64).clamp(0, sw as i64);
        let sx_hi = (rel_x0 as i64 + bw as i64 - scale.off_x as i64).clamp(sx_lo, sw as i64);
        let sy_lo = (rel_y0 as i64 - scale.off_y as i64).clamp(0, sh as i64);
        let sy_hi = (rel_y0 as i64 + bh as i64 - scale.off_y as i64).clamp(sy_lo, sh as i64);

        use std::hash::{Hash, Hasher};
        // Hash ONLY the band's own native footprint (not the whole canvas): a
        // change confined to another band's native pixels then leaves this band's
        // hash — and its cached upload — untouched (SQ-0514). The footprint is the
        // native bounding box of the scaled pixels this band samples; the Nearest
        // scaled→native map is monotone, so it covers exactly the native pixels
        // that can alter the band.
        // …and the same footprint is what this band PAINTS, so the dump reports it
        // (SQ-0747): a band's cell rect says where an image lands, never which rows
        // of the game's screen were rasterized into it, and "which canvas rows is
        // this band showing?" is the question a band painting the wrong region of
        // the screen is answered by.
        let mut footprint = None;
        if sx_lo < sx_hi && sy_lo < sy_hi {
            // A minifying letterbox resamples through an area filter whose kernel is
            // as wide as the ratio, so a scaled pixel reads native pixels either side
            // of the one Nearest would have picked. The footprint carries that halo
            // (zero when magnifying, where Nearest is exact) — SQ-0824.
            let halo = scale_halo(scale.s);
            let nx0 = scaled_to_native(sx_lo as u32, nw, sw).saturating_sub(halo);
            let nx1 = (scaled_to_native(sx_hi as u32 - 1, nw, sw) + 1 + halo).min(nw);
            let ny0 = scaled_to_native(sy_lo as u32, nh, sh).saturating_sub(halo);
            let ny1 = (scaled_to_native(sy_hi as u32 - 1, nh, sh) + 1 + halo).min(nh);
            footprint = Some((nx0, ny0, nx1 - nx0, ny1 - ny0));
        }
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the whole-frame generation key has already
        // proven every input to this hash unchanged, so the stored hash IS the
        // hash and no pixel is walked. Any frame that could have moved an input
        // rebuilt the HybridFrame, and that frame carries `band_replay = false`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                match footprint {
                    Some((nx0, ny0, fw, fh)) => {
                        (nx0, nx0 + fw, ny0, ny0 + fh).hash(&mut h);
                        hash_canvas_rows(&mut h, chrome_canvas, nx0, nx0 + fw, ny0, ny0 + fh);
                    }
                    // Fully in the letterbox margin — no native pixels feed it.
                    None => 0u8.hash(&mut h),
                }
                self.band_ground.map(|p| p.0).hash(&mut h);
                scale.s.to_bits().hash(&mut h);
                (scale.off_x, scale.off_y).hash(&mut h);
                (cw, ch).hash(&mut h);
                (rel_x0, rel_y0, bw, bh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: exactly this content is already on its way to the worker
            // — keep placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            // Copy the sub-rect under this band out of the frame-shared scaled
            // chrome into a band-sized image (letterbox area outside the scaled
            // chrome stays transparent). The whole-canvas resize happens at most
            // once per changed frame (cached in `chrome_scaled`), not per band.
            let band_img = {
                let scaled = self.scaled_chrome(chrome_canvas, scale.s, sw, sh);
                // SQ-1188: the in-bounds sx range is one contiguous byte run per
                // row — copy rows, not pixels. `sx = rel_x0 + bx - off_x`, so bx
                // is in-bounds on [off_x - rel_x0, off_x - rel_x0 + sw).
                let lo = (scale.off_x as i64 - rel_x0 as i64).max(0);
                let hi = ((scale.off_x as i64 - rel_x0 as i64) + sw as i64).min(bw as i64);
                let mut data = vec![0u8; bw as usize * bh as usize * 4];
                if lo < hi {
                    let len = (hi - lo) as usize * 4;
                    let sx0 = (rel_x0 as i64 + lo - scale.off_x as i64) as usize;
                    let raw = scaled.as_raw();
                    for by in 0..bh {
                        let sy = rel_y0 as i64 + by as i64 - scale.off_y as i64;
                        if sy < 0 || sy as u32 >= sh {
                            continue;
                        }
                        let src = (sy as usize * sw as usize + sx0) * 4;
                        let dst = (by as usize * bw as usize + lo as usize) * 4;
                        data[dst..dst + len].copy_from_slice(&raw[src..src + len]);
                    }
                }
                image::RgbaImage::from_raw(bw, bh, data).expect("sized above")
            };
            let img = self.seal_band(band_img);
            // Under the id this band is already placed as, when it has one — see
            // [`Self::band_encode`] (SQ-0996); off the main thread when the old
            // upload can cover for it (SQ-1188).
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // The band image is exactly band-sized, so it places at the band's
        // top-left (no centering — the crop is already positioned).
        //
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let dest = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (dest, sz, place_protocol_with(proto, dest, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((dest, sz, (id, _))) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}): {} · proto {}x{} · placed {}x{} at ({},{}) · native {} · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    match footprint {
                        Some((x, y, w, h)) => format!("{w}x{h}@({x},{y})"),
                        None => "— (entirely in the letterbox margin)".to_string(),
                    },
                    resample_note(nw, nh, sw, sh),
                ));
                // The crop's magnification is the whole canvas's, on both axes, and
                // it cannot be anything else: this band copies device pixels 1:1 out
                // of the ONE frame-shared `sw x sh` scaled canvas (SQ-0514). Anything
                // outside the canvas stays transparent, so the letterbox margin is
                // margin here rather than art stretched into it. Recorded from the
                // resize's own sizes so the reading is a measurement, not a promise.
                self.note_band_mag(band, BandFit::Letterbox, (nw, nh), (sw, sh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (dest.x, dest.y, dest.width, dest.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}): NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }

    /// Record the kitty image id a band was just placed as, so the upload can be
    /// freed when the entry is abandoned (SQ-0753) — and so the band's NEXT encode
    /// can go out under the same id (SQ-0996; see [`Self::band_encode`]). A no-op
    /// under a protocol with no addressable id (half-blocks, sixel).
    pub(super) fn remember_band_id(&mut self, key: BandKey, id: Option<u32>) {
        if let Some((_, _, slot)) = self.chrome_bands.get_mut(&key) {
            *slot = id;
        }
    }

    /// Encode one chrome band, UNDER THE ID IT IS ALREADY PLACED AS when it has
    /// one (SQ-0996), and record the upload. `None` if the encode failed.
    ///
    /// A kitty virtual placement writes the image id into EVERY cell of its rect —
    /// low 24 bits as the foreground colour, high byte as the third diacritic — so
    /// the id is a per-cell value, and a band that re-encodes under a fresh id
    /// dirties every one of those cells. `ratatui-image` draws its ids at random
    /// (`rand::random()` per `Protocol`) and this path builds a new `Protocol` on
    /// every content change, so until now a chrome band that changed by one pixel
    /// repainted its whole placeholder rect. Measured on Journey r83 at 117x64
    /// under a pty, the 39x20-cell illustration band: 26,968 bytes for a frame
    /// whose image was 15,136 — the rest was cells.
    ///
    /// Handing the previous id back to the crate replaces the DATA behind an
    /// unchanged placement instead. The placeholder cells come out byte-identical
    /// to the last frame's except the first, which carries the transmit, so
    /// ratatui's diff emits one cell and the picture.
    ///
    /// The id comes from the placement we last WROTE, not from an allocator: it is
    /// read back off the cells (`place_protocol_with`), so it is `None` under
    /// half-blocks and sixel — which have no ids and want none — and `None` before
    /// a band's first place, where the crate's random draw is exactly right. That
    /// also means an EVICTED band gets a fresh id, which it must: eviction queues
    /// an `a=d` for the old one, and a delete riding out on another band's cell
    /// could otherwise be emitted after the re-transmit that revived the id.
    pub(super) fn band_encode(
        &mut self,
        picker: &Picker,
        img: image::DynamicImage,
        key: BandKey,
        band: Rect,
        hash: u64,
        reuse: Option<u32>,
    ) -> Option<()> {
        let size = Size::new(band.width, band.height);
        let t0 = std::time::Instant::now();
        let encoded = match reuse {
            Some(id) if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty => {
                kitty_protocol_with_id(picker, img, size, Resize::Fit(None), id)
            }
            _ => picker.new_protocol(img, size, Resize::Fit(None)),
        };
        let elapsed = t0.elapsed();
        let p = encoded.ok()?;
        self.encode_timings.band_encode.record(elapsed);
        self.band_encodes += 1;
        // The id carries forward with the new protocol, so the placement's cells
        // are the ones already on screen and `remember_band_id` re-confirms it.
        let stale = self.chrome_bands.insert(key, (hash, p, reuse));
        let stale_id = stale.and_then(|(_, _, id)| id);
        // Whatever this key held is being replaced: free it in the terminal before
        // the only record of its id is overwritten (SQ-0753) — UNLESS it is the id
        // we just re-transmitted to, which is the whole point and would delete the
        // image this frame is about to show.
        if stale_id != reuse {
            self.queue_protocol_delete_after_place(stale_id);
        }
        self.note_op(GraphicsOp::Upload {
            target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
            id: reuse,
            cells: (band.width, band.height),
        });
        Some(())
    }

    /// SQ-1188: is an encode for exactly this band content already staged or in
    /// flight? Then the caller skips rebuilding the band image and keeps
    /// placing the old upload until the worker's result lands.
    pub(super) fn band_queued(&self, key: BandKey, hash: u64) -> bool {
        self.band_dirty.get(&key) == Some(&hash)
    }

    /// SQ-1188: hand a changed band to the encoder — the background worker when
    /// this backend's encode is worth a thread AND an older upload is still
    /// placed to keep showing; synchronously otherwise. The three outcomes are
    /// the band log's status words.
    ///
    /// The synchronous cases are deliberate, not leftovers:
    /// * **no cached upload** — a first appearance or a post-resume re-upload
    ///   has no honest previous image to keep placed, so deferring it would
    ///   blank the band for a frame. This is exactly `spawn_v6_encode`'s rule
    ///   for the raster composite's first frame (SQ-0578).
    /// * **half-blocks** — see [`band_encode_offthread`].
    pub(super) fn stage_band_encode(
        &mut self,
        picker: &Picker,
        img: image::DynamicImage,
        key: BandKey,
        band: Rect,
        hash: u64,
    ) -> Option<&'static str> {
        let reuse = self.chrome_bands.get(&key).and_then(|(_, _, id)| *id);
        if !band_encode_offthread(picker) || !self.chrome_bands.contains_key(&key) {
            self.band_dirty.remove(&key);
            return self.band_encode(picker, img, key, band, hash, reuse).map(|()| "encoded");
        }
        self.band_dirty.insert(key, hash);
        self.band_pending.push(PendingBand { key, band, hash, img, reuse });
        Some("encode queued (worker)")
    }

    /// SQ-1188: hand this frame's staged band encodes to one background worker.
    /// Called at the end of the hybrid draw, once per frame. Coalesced exactly
    /// like the raster worker: one batch at a time, and stagings that arrive
    /// while a batch runs are dropped and re-staged by a later frame (their
    /// dirty marks are lifted so the re-stage actually happens — except where
    /// the in-flight batch already carries the same content).
    pub fn spawn_band_jobs(&mut self, picker: &Picker) {
        let pending = std::mem::take(&mut self.band_pending);
        if pending.is_empty() {
            return;
        }
        if self.band_job.is_some() {
            for p in pending {
                if self.band_inflight.get(&p.key) != Some(&p.hash) {
                    self.band_dirty.remove(&p.key);
                }
            }
            return;
        }
        self.band_inflight = pending.iter().map(|p| (p.key, p.hash)).collect();
        let picker = picker.clone();
        self.band_job = Some(std::thread::spawn(move || {
            pending
                .into_iter()
                .map(|p| {
                    let size = Size::new(p.band.width, p.band.height);
                    // The id-reuse discipline rides into the worker unchanged
                    // (SQ-0996): the encode goes out under the id the band is
                    // already placed as, so the placeholder cells stay stable.
                    let t0 = std::time::Instant::now();
                    let proto = match p.reuse {
                        Some(id) if picker.protocol_type() == ratatui_image::picker::ProtocolType::Kitty => {
                            kitty_protocol_with_id(&picker, p.img, size, Resize::Fit(None), id)
                        }
                        _ => picker.new_protocol(p.img, size, Resize::Fit(None)),
                    }
                    .ok();
                    let encode = t0.elapsed();
                    BandEncoded { key: p.key, band: p.band, hash: p.hash, reuse: p.reuse, proto, encode }
                })
                .collect()
        }));
    }

    /// SQ-1188: reap the background band batch. Installs each result whose band
    /// still expects exactly that content — a band that changed again while the
    /// worker ran, or was evicted, drops its result on the floor (its staging
    /// mark is lifted so the next frame re-stages the CURRENT content instead).
    /// Returns true whenever a batch was reaped, so the caller schedules the
    /// redraw that places the new uploads (and re-stages whatever is still
    /// stale) — the raster worker's last-ready shape.
    pub(super) fn poll_band_job(&mut self) -> bool {
        let done = self.band_job.as_ref().is_some_and(|j| j.is_finished());
        if !done {
            return false;
        }
        let job = self.band_job.take().expect("checked above");
        if let Ok(results) = job.join() {
            for r in results {
                if self.band_dirty.get(&r.key) != Some(&r.hash) {
                    continue; // superseded or cancelled — a newer staging owns this band
                }
                self.band_dirty.remove(&r.key);
                let Some(proto) = r.proto else { continue }; // failed encode: retried by the next frame
                let Some(entry) = self.chrome_bands.get_mut(&r.key) else { continue }; // evicted
                let stale_id = entry.2;
                *entry = (r.hash, proto, r.reuse);
                self.band_encodes += 1;
                self.encode_timings.band_encode.record(r.encode);
                if stale_id != r.reuse {
                    self.queue_protocol_delete_after_place(stale_id);
                }
                self.note_op(GraphicsOp::Upload {
                    target: GraphicsTarget::Band(r.key.1, r.key.2, r.key.3, r.key.4),
                    id: r.reuse,
                    cells: (r.band.width, r.band.height),
                });
            }
        }
        self.band_inflight.clear();
        true
    }

    /// True while any band encode is staged or in flight (SQ-1188) — the test
    /// harness's settle probe, mirroring [`Self::v6_encode_in_flight`].
    pub fn band_encode_in_flight(&self) -> bool {
        self.band_job.is_some() || !self.band_pending.is_empty() || !self.band_dirty.is_empty()
    }

    /// SQ-0511: draw ONE side flank band VERTICALLY STRETCHED — sample the native
    /// `crop` sub-rect of `chrome_canvas` (x, y, w, h in game pixels) and resize it
    /// to fill the whole `band` device region (Nearest → crisp). The horizontal
    /// resize factor is the uniform letterbox scale (the caller derives `crop`'s
    /// width from `band.width · cell_w / s`), so columns keep their true width; only
    /// the vertical factor grows, elongating an ornate frame column to span the
    /// reclaimed dead space between the top-anchored chrome and a bottom-anchored
    /// band/menu (enclosed-frame Zork0/Shogun flanks; Journey's picture-column
    /// divider continuing to its menu strip).
    ///
    /// Shares the per-band [`chrome_bands`](Self) cache with [`draw_chrome_band`]
    /// (one entry per band rect), so a band drawn stretched still participates in
    /// [`retain_chrome_bands`]/[`chrome_band_hashes`]. The freshness hash covers ONLY
    /// the crop's own native pixels + its coords + the target size, so a status tick
    /// (whose pixels sit ABOVE the flank crop) leaves the flank's cached upload fresh
    /// (SQ-0514 property preserved), and the crop native rect is folded in so the
    /// stretch factor itself is part of the key.
    ///
    /// `fit` is the caller's own claim about what this band is, and it is a
    /// parameter rather than a constant because that is what SQ-0898's second round
    /// turned on: recorded here as `Fitted` for everybody, it exempted every caller
    /// of this function from the one-magnification gate, including the one that was
    /// wrong. See [`BandFit`].
    pub fn draw_chrome_band_stretched(
        &mut self,
        picker: &Picker,
        chrome_canvas: &image::RgbaImage,
        band: Rect,
        crop: (u32, u32, u32, u32),
        slot: BandSlot,
        fit: BandFit,
        buf: &mut Buffer,
    ) {
        let (cx, cy, cw_n, ch_n) = crop;
        if band.width == 0 || band.height == 0 || cw_n == 0 || ch_n == 0 {
            return;
        }
        let (canvas_w, canvas_h) = (chrome_canvas.width(), chrome_canvas.height());
        if cx >= canvas_w || cy >= canvas_h {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;

        use std::hash::{Hash, Hasher};
        let x1 = (cx + cw_n).min(canvas_w);
        let y1 = (cy + ch_n).min(canvas_h);
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the stored hash is the hash — see
        // `draw_chrome_band`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                // Hash ONLY the crop's native footprint (coords + pixels) plus the target
                // device size — the stretch factor is (bw,bh)/(cw_n,ch_n), so both ends are
                // covered. A change outside this native rect (e.g. the banner's Score/Moves)
                // never alters the hash, keeping the flank's cached upload fresh (SQ-0514).
                (slot as u8).hash(&mut h); // discriminator vs. draw_chrome_band keys on the same map
                (cx, cy, cw_n, ch_n).hash(&mut h);
                hash_canvas_rows(&mut h, chrome_canvas, cx, x1, cy, y1);
                self.band_ground.map(|p| p.0).hash(&mut h);
                (bw, bh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: this content is already on its way to the worker — keep
            // placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            // Copy the native crop (clamped to the canvas) into its own image, then
            // resize it to the band's device box. Transparent native pixels stay
            // transparent, so the theme backdrop shows through gaps in the flank.
            // SQ-1188: rows, not pixels — each crop row is one contiguous byte run.
            let mut data = vec![0u8; cw_n as usize * ch_n as usize * 4];
            let cols = (x1 - cx) as usize * 4;
            let raw = chrome_canvas.as_raw();
            for oy in 0..(y1 - cy) {
                let srow = ((cy + oy) as usize * canvas_w as usize + cx as usize) * 4;
                let drow = oy as usize * cw_n as usize * 4;
                data[drow..drow + cols].copy_from_slice(&raw[srow..srow + cols]);
            }
            let src = image::RgbaImage::from_raw(cw_n, ch_n, data).expect("sized above");
            let stretched = resize_directional(&src, bw, bh);
            let img = self.seal_band(stretched);
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // SQ-0747: a STRETCHED band goes in the band log too. It never did, and that
        // is why every `/dump-windows` capture of Journey's menu listed the right-hand
        // flank and the bottom strip and no left flank at all — the picture column is
        // drawn by this function, at a dest rect derived from the panel rather than
        // from the strip, so the one band an investigation most wanted to see was the
        // one band the dump could not name. Two passes were spent inferring it from
        // the strip rect beside it. The crop rides along for the same reason it does
        // above: it says which rows of the game's screen this image is showing.
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let dest = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (dest, sz, place_protocol_with(proto, dest, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((dest, sz, (id, _))) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, stretched]: {} · proto {}x{} · placed {}x{} at ({},{}) · native {cw_n}x{ch_n}@({cx},{cy}) · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    resample_note(cw_n, ch_n, bw, bh),
                ));
                self.note_band_mag(band, fit, (cw_n, ch_n), (bw, bh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (dest.x, dest.y, dest.width, dest.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}) [{slot:?}, stretched]: NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }

    /// SQ-0698: draw ONE band from a source image the CALLER composed in native
    /// pixels, rather than from a sub-rect of the chrome canvas.
    ///
    /// The side border extension ([`crate::render::v6_border`]) tiles a flank's
    /// art downward past the native screen bottom, so its source does not exist
    /// in the canvas and cannot be named as a crop.
    ///
    /// `dest` is where inside the band, in device pixels relative to the band's
    /// own top-left, `src` belongs — **the letterbox image of `src`'s native box
    /// at the frame's scale**, computed by the caller from the same mapping the
    /// plain crop uses ([`crate::render::screen`]'s `flank_native_box`). The rest
    /// of the band stays transparent, exactly as a crop leaves the letterbox
    /// margin transparent.
    ///
    /// It used to be the whole band, and that was SQ-0898's defect. The caller
    /// clipped the SOURCE to the art it has — a flank cannot read native columns
    /// left of zero, and there are none to its left when the pane is wider than
    /// the scaled screen — and then handed the clipped source to a destination
    /// nobody had clipped, so the resize made up the difference by changing the
    /// magnification. MEASURED on Arthur at a 70x19 pane (`off_x = 6`, scale
    /// 0.855): the pole's 30 native columns were resized into the band's full
    /// 32 device px — 1.067 px per native px against the canvas's 0.855, and
    /// shifted 6 px left of the crop directly above it, which is the corner
    /// fragment the user reported as "not scaled to match". At `off_x = 0` the
    /// clip is a no-op and the same code looks perfect, which is why this
    /// survived a corpus checked at one pane size.
    ///
    /// Shares the per-band cache with the other two draws (one entry per band
    /// rect + slot), so it participates in [`Self::retain_chrome_bands`]. The
    /// freshness hash is the source's own pixels plus the target size, so a band
    /// whose art did not change is not re-encoded.
    pub fn draw_chrome_band_image(
        &mut self,
        picker: &Picker,
        src: &image::RgbaImage,
        band: Rect,
        dest: BandDest,
        slot: BandSlot,
        buf: &mut Buffer,
    ) {
        if band.width == 0 || band.height == 0 || src.width() == 0 || src.height() == 0 {
            return;
        }
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let bw = band.width as u32 * cw;
        let bh = band.height as u32 * ch;
        // Clamped rather than trusted: a destination outside the band would panic
        // the blit, and the caller's rounding is device-pixel arithmetic.
        let (dx, dy) = (dest.0.min(bw), dest.1.min(bh));
        let (dw, dh) = (dest.2.min(bw - dx), dest.3.min(bh - dy));
        if dw == 0 || dh == 0 {
            return;
        }

        use std::hash::{Hash, Hasher};
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let cached_hash = self.chrome_bands.get(&key).map(|(v, _, _)| *v);
        // SQ-1187: on a replay frame the stored hash is the hash — see
        // `draw_chrome_band`.
        let hash = match cached_hash.filter(|_| self.band_replay && !self.band_dirty.contains_key(&key)) {
            Some(v) => v,
            None => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (slot as u8).hash(&mut h);
                (src.width(), src.height()).hash(&mut h);
                src.as_raw().hash(&mut h);
                (bw, bh).hash(&mut h);
                self.band_ground.map(|p| p.0).hash(&mut h);
                (dx, dy, dw, dh).hash(&mut h);
                h.finish()
            }
        };
        let fresh = cached_hash == Some(hash);
        let status = if fresh {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
            "cache HIT"
        } else if self.band_queued(key, hash) {
            // SQ-1188: this content is already on its way to the worker — keep
            // placing the old upload below until the result lands.
            "encode queued (worker)"
        } else {
            let scaled = resize_directional(src, dw, dh);
            // The band is `dest` and transparent everywhere else. When `dest` IS the
            // band — every pane where the letterbox leaves no margin beside this
            // flank — the copy is skipped and the bytes are what they always were.
            let scaled = if (dx, dy, dw, dh) == (0, 0, bw, bh) {
                scaled
            } else {
                let mut band_img = image::RgbaImage::new(bw, bh);
                image::imageops::replace(&mut band_img, &scaled, dx as i64, dy as i64);
                band_img
            };
            let img = self.seal_band(scaled);
            let Some(status) = self.stage_band_encode(picker, img, key, band, hash) else {
                return;
            };
            status
        };
        // Any deletes queued above (this band's own predecessor, or another band's)
        // ride out on this placement, in the same batch (SQ-0637's rule). Restored
        // to the queue when nothing was placed, or when the backend has no
        // placeholder row to carry them — a delete is deferred, never dropped.
        let pending = std::mem::take(&mut self.pending_deletes);
        // …and this band's own predecessor rides BEHIND the placement, because it is
        // still covering the rect that placement is about to take (SQ-0817).
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed = self.chrome_bands.get(&key).map(|(_, proto, _)| {
            let sz = proto.size();
            let at = Rect::new(band.x, band.y, sz.width.min(band.width), sz.height.min(band.height));
            (at, sz, place_protocol_with(proto, at, buf, &pending, &after))
        });
        if !matches!(placed, Some((_, _, (Some(_), _)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        if let Some((_, _, (id, bytes))) = placed {
            self.uploads.add(bytes);
            self.note_upload_id(id, bytes.pixels);
        }
        match placed {
            Some((placed_at, sz, (id, _))) => {
                let (blank, run, run_at) = blank_rows(src);
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, tiled]: {} · proto {}x{} · placed {}x{} at ({},{}) · source {}x{} native px · into {dw}x{dh}px at ({dx},{dy}) of {bw}x{bh} · blank rows {}, longest run {} at {} · {}",
                    band.width, band.height, band.x, band.y,
                    status,
                    sz.width, sz.height, placed_at.width, placed_at.height, placed_at.x, placed_at.y,
                    src.width(), src.height(),
                    blank, run, run_at,
                    resample_note(src.width(), src.height(), dw, dh),
                ));
                self.note_band_mag(band, BandFit::Letterbox, (src.width(), src.height()), (dw, dh));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (placed_at.x, placed_at.y, placed_at.width, placed_at.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}) [{slot:?}, tiled]: NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
    }
}


// ── Delete queues detached from a `GraphicsRender` instance (SQ-1190) ────────
//
// `GraphicsRender` lives on `AppState`, but two other `Protocol` caches place
// uploads through the same `place_protocol` and have no `AppState` to share it
// with: the pre-game picker's cover/tile/preview caches (`cover.rs`,
// `picker_ui.rs`) run their own event loop before any `AppState` exists. The
// two pieces below are `GraphicsRender::queue_protocol_delete` and
// `flush_kitty_deletes`, detached from `self` so a queue with no instance in
// common can still emit the identical bytes and no-flicker sequencing instead
// of duplicating either by hand.

/// The literal escape that frees one kitty image id — `a=d,d=I` deletes the
/// image data and every placement of it at once, exactly what
/// `GraphicsRender::queue_protocol_delete` writes.
pub(crate) fn kitty_delete_escape(id: u32) -> String {
    format!("\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
}

/// [`GraphicsRender::flush_kitty_deletes`], operating on a caller-owned `pending`
/// string instead of `self.pending_deletes`. Kept in sync with that method by
/// hand — the two have no instance to share, so this is the alternative to
/// duplicating the caller-facing queue itself.
pub(super) fn flush_kitty_deletes_into(pending: &mut String, area: Rect, buf: &mut Buffer) {
    use ratatui::buffer::CellDiffOption;
    if pending.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell_mut((x, y)) else { continue };
            let plain = cell.diff_option == CellDiffOption::None
                && cell.symbol().len() == 1
                && cell.symbol().is_ascii()
                && !cell.symbol().starts_with(char::is_control);
            if !plain {
                continue;
            }
            let symbol = format!("{}{}", pending, cell.symbol());
            cell.set_symbol(&symbol)
                .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
            pending.clear();
            return;
        }
    }
}

/// A delete queue for a `Protocol` cache with no `GraphicsRender` to share
/// (SQ-1190) — see the section header above. Same two-step shape as
/// `GraphicsRender`'s own queue: [`Self::queue`] an id an eviction/replacement
/// abandoned, [`Self::flush`] it into a frame's buffer once a plain cell can
/// carry it.
#[derive(Default)]
pub struct KittyDeleteQueue(String);

impl KittyDeleteQueue {
    /// Queue a delete for an upload nothing on screen depends on any more. A
    /// no-op for `None` — a non-kitty protocol, or an entry never placed.
    pub fn queue(&mut self, id: Option<u32>) {
        if let Some(id) = id {
            self.0.push_str(&kitty_delete_escape(id));
        }
    }

    /// Flush queued deletes into `buf`, riding on the first plain cell found in
    /// `area` (never dropped — deferred to a later flush if none is found).
    pub fn flush(&mut self, area: Rect, buf: &mut Buffer) {
        flush_kitty_deletes_into(&mut self.0, area, buf);
    }
}

/// Rows of `src` with no opaque pixel anywhere across them: how many, the
/// longest contiguous run of them, and where that run starts (SQ-0698).
///
/// Reported on every tiled band's log line, because a hole in a tiled flank is
/// invisible in the band's RECT — the placement is the full height either way —
/// and shows up only as a black stripe on the user's screen. The RUN and WHERE
/// are the numbers that matter. A run at row 0 is the band's own top edge, where
/// the renderer clears the rows a chrome text strip draws as crisp cells (a v6
/// text row is 16 native px, so the leading run is small); an INTERIOR run is a
/// hole, and Shogun's tiled pieces were separated by one of 64 native rows.
pub(super) fn blank_rows(src: &image::RgbaImage) -> (u32, u32, u32) {
    let (mut total, mut run, mut longest, mut at) = (0, 0, 0, 0);
    for y in 0..src.height() {
        if (0..src.width()).all(|x| src.get_pixel(x, y)[3] == 0) {
            total += 1;
            run += 1;
            if run > longest {
                longest = run;
                at = y + 1 - run;
            }
        } else {
            run = 0;
        }
    }
    (total, longest, at)
}

// ── Kitty virtual-placement emission (SQ-0520) ────────────────────────────────

/// Kitty's `rowcolumn-diacritics.txt`, complete: the index into this table IS
/// the value the diacritic encodes. A placeholder cell carries up to three of
/// them — image ROW, image COLUMN, and the image id's HIGH BYTE — and every one
/// of those three needs an arbitrary index, so the table has to be whole. It
/// held only the first 140 entries while the emitter used exactly two values
/// (the row, and zero for the other two); that cap silently made a wide
/// placement and a high-byte id inexpressible (SQ-0772).
///
/// Transcribed from `ratatui-image` 11.0.6's `DIACRITICS` and cross-checked
/// entry-for-entry against `qwertty-term-vt` 0.4.0's independent copy — two
/// implementations that agree on all 297 values, rather than one recalled table.
pub(super) const KITTY_DIACRITICS: [char; 297] = [
    '\u{305}', '\u{30D}', '\u{30E}', '\u{310}', '\u{312}', '\u{33D}', '\u{33E}', '\u{33F}',
    '\u{346}', '\u{34A}', '\u{34B}', '\u{34C}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35B}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36A}', '\u{36B}', '\u{36C}', '\u{36D}', '\u{36E}', '\u{36F}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59C}', '\u{59D}', '\u{59E}', '\u{59F}', '\u{5A0}', '\u{5A1}',
    '\u{5A8}', '\u{5A9}', '\u{5AB}', '\u{5AC}', '\u{5AF}', '\u{5C4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}', '\u{658}',
    '\u{659}', '\u{65A}', '\u{65B}', '\u{65D}', '\u{65E}', '\u{6D6}', '\u{6D7}', '\u{6D8}',
    '\u{6D9}', '\u{6DA}', '\u{6DB}', '\u{6DC}', '\u{6DF}', '\u{6E0}', '\u{6E1}', '\u{6E2}',
    '\u{6E4}', '\u{6E7}', '\u{6E8}', '\u{6EB}', '\u{6EC}', '\u{730}', '\u{732}', '\u{733}',
    '\u{735}', '\u{736}', '\u{73A}', '\u{73D}', '\u{73F}', '\u{740}', '\u{741}', '\u{743}',
    '\u{745}', '\u{747}', '\u{749}', '\u{74A}', '\u{7EB}', '\u{7EC}', '\u{7ED}', '\u{7EE}',
    '\u{7EF}', '\u{7F0}', '\u{7F1}', '\u{7F3}', '\u{816}', '\u{817}', '\u{818}', '\u{819}',
    '\u{81B}', '\u{81C}', '\u{81D}', '\u{81E}', '\u{81F}', '\u{820}', '\u{821}', '\u{822}',
    '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82A}', '\u{82B}', '\u{82C}',
    '\u{82D}', '\u{951}', '\u{953}', '\u{954}', '\u{F82}', '\u{F83}', '\u{F86}', '\u{F87}',
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}',
    '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}',
    '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}',
    '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}',
    '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}',
    '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}',
    '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}',
    '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}',
    '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}', '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}',
    '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}',
    '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}', '\u{1D187}',
    '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}', '\u{1D242}', '\u{1D243}',
    '\u{1D244}',
];

/// Plain-Rust base64 (standard alphabet, padded) — only used for kitty image
/// transmission, so no dependency is worth it.
pub(super) fn kitty_b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Deflate a transmit payload for the kitty protocol's `o=z`.
///
/// RFC 1950 (zlib), which is the ONLY compression the protocol defines. Level 6
/// (`Compression::default()`) rather than 1 or 9 — measured on real v6 canvases
/// in SQ-0976, where the three levels are not close:
///
/// | canvas (turns)                | b64 raw | L1 b64 | L6 b64 | L9 b64 | L1 / L6 / L9 ms |
/// |-------------------------------|--------:|-------:|-------:|-------:|-----------------|
/// | Zork Zero r393 win 7, 640x400 | 1365336 |  35068 |   6580 |   6024 | 0.40 / 1.43 / 2.40 |
/// | Shogun r322 win 7, 640x400    | 1365336 |  20276 |   6532 |   6040 | 0.27 / 1.77 / 3.30 |
/// | Journey r83 win 3, 232x304    |  376152 |  27168 |  10884 |  10016 | 0.35 / 1.88 / 5.34 |
/// | Zork Zero r393 win 1, 640x78  |  266240 |   3180 |    960 |    964 | 0.05 / 0.26 / 0.37 |
///
/// L1 leaves three to five times as much on the wire for a millisecond saved;
/// L9 buys 5–8% more for two to four times L6's cost, and on one canvas it is
/// *larger*. Sixteen-colour flat artwork is what deflate is best at, and 1.4–3.3
/// ms on the render worker is nothing beside 1.37 MB of base64.
///
/// Writing into a `Vec` has no failure mode; the `expect` documents that rather
/// than inventing a fallback path no test could reach.
pub(super) fn zlib_deflate(raw: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(raw.len() / 32 + 64),
        flate2::Compression::default(),
    );
    enc.write_all(raw).expect("a zlib encoder writing into a Vec cannot fail");
    enc.finish().expect("a zlib encoder writing into a Vec cannot fail")
}

/// The kitty transmit sequence for `canvas` as image `id`: a VIRTUAL placement
/// (`U=1`) declaring an explicit `r×c` grid, so the terminal scales the image
/// to exactly the placeholder rect (SQ-0520). RGBA, chunked per the protocol's
/// 4096-encoded-byte limit, and zlib-compressed when `compress` says the terminal
/// can inflate it. (No tmux passthrough — matches the app's existing kitty
/// support, which targets direct terminals.)
///
/// **`compress` is a capability, not a preference** (SQ-0997). It comes from
/// [`kitty_compression`], and this function stated `o=z` unconditionally until it
/// did: a terminal that answers the kitty query but not the `o=z` one refuses a
/// deflated transmission, stores no image, and draws nothing for every placement
/// naming it — with no error and nothing on screen to explain the missing picture.
/// See [`kitty_compression`] for why an unasked terminal counts as "cannot".
///
/// **`o=z` is the payload's encoding and nothing else** (SQ-0976). Per the kitty
/// graphics protocol: *"the payload is now compressed using deflate (this occurs
/// prior to base64 encoding)"*, so `f=32` still names the format the terminal
/// finds after inflating, and `s=`/`v=` still name the **uncompressed** image's
/// pixel dimensions — the terminal sizes its buffer from `s*v*4` and the inflated
/// payload must be exactly that long. The `S` key is not involved: the spec
/// requires it only for PNG-plus-compression, where the decompressed length is
/// not implied by the geometry.
///
/// Chunking therefore applies to the COMPRESSED stream, because it is the thing
/// being base64-encoded — *"the pixel data must first be base64 encoded then
/// chunked up into chunks no larger than 4096 bytes"*. 3072 compressed bytes make
/// exactly 4096 base64 characters and satisfy "all chunks except the last must
/// have a size that is a multiple of 4"; continuation chunks carry only `m` and
/// `q`, as the spec demands, so `o=z` is stated once on the first chunk and
/// governs the reassembled whole.
///
/// **The placement is left anonymous (`p=0`, i.e. no `p=` key at all)** (SQ-0995,
/// SQ-1512). `id` is stable across a window's whole life and this command is
/// re-issued whenever the canvas changes; the protocol says *"When re-transmitting
/// image data for a specific id, the existing image and all its placements must be
/// deleted"*, which a conforming terminal honors in full. Ghostty's storage (≤1.3.1)
/// did not: it replaced only the image and left the placement behind, so this file
/// used to name the placement (`p=1`) to force each re-transmit to replace it in
/// the map rather than leaving a stranded duplicate. Ghostty's own fix
/// (ghostty#13723) replaces placements correctly now, and measuring the ≤1.3.1
/// leak directly — 200,000 re-transmits, ~30MB resident growth, no visible artifact,
/// no measurable latency cost — settled that the workaround costs more (a named
/// placement is one more invariant to keep straight) than the leak it prevented on
/// an old release. The placeholder cells still encode placement 0, which resolves
/// to "the first virtual placement of this image" and therefore to the only one,
/// same as before.
// SQ-1338: the production caller now goes straight to `kitty_transmit_virtual_timed`
// so it can fold the timing in; this thin wrapper exists only for the ~8 test
// callers below that don't care about it, so it is test-only rather than a live
// (and therefore dead-code-warned) production entry point.
#[cfg(all(test, feature = "t-render"))]
pub(super) fn kitty_transmit_virtual(canvas: &image::RgbaImage, id: u32, rows: u16, cols: u16, compress: bool) -> String {
    kitty_transmit_virtual_timed(canvas, id, rows, cols, compress).0
}

/// Wall-clock split of one [`kitty_transmit_virtual`] call (SQ-1338). Unlike the
/// raster composite and chrome bands, whose deflate and base64 are fused inside
/// one `ratatui-image` call this file cannot see into, a graphics window deflates
/// its own payload before base64-chunking it — two real steps this file DOES
/// control the boundary of.
pub(super) struct WindowTiming {
    /// `None` when `compress` was false: there was no deflate to time.
    pub(super) deflate: Option<std::time::Duration>,
    /// The whole chunk/base64 loop, timed once — not per chunk. `None` when the
    /// pixels went through shared memory, where there is no payload to encode
    /// (SQ-1374).
    pub(super) base64: Option<std::time::Duration>,
    /// Writing the pixels into a shared memory object — `shm_open` plus a
    /// `write(2)` on Linux, or `shm_open`, `ftruncate` and one copy through a
    /// mapping elsewhere (SQ-1380). `None` on every other route.
    pub(super) shm: Option<std::time::Duration>,
}

/// [`kitty_transmit_virtual`] plus its own timing (SQ-1338). Split out as its own
/// function, rather than widening the public-facing one, so the ~8 existing test
/// callers of [`kitty_transmit_virtual`] need no change; the render path is the
/// only caller that wants the timing.
pub(super) fn kitty_transmit_virtual_timed(
    canvas: &image::RgbaImage,
    id: u32,
    rows: u16,
    cols: u16,
    compress: bool,
) -> (String, WindowTiming) {
    use std::fmt::Write as _;
    let (w, h) = (canvas.width(), canvas.height());
    let deflated;
    let mut deflate_time = None;
    // SQ-0997: `compress` is [`kitty_compression`]'s answer for the picker in
    // force. Raw when it is false — the geometry keys are untouched either way,
    // because `o=z` describes the payload's encoding and nothing about the image.
    let (payload, encoding): (&[u8], &str) = if compress {
        let t0 = std::time::Instant::now();
        deflated = zlib_deflate(canvas.as_raw());
        deflate_time = Some(t0.elapsed());
        (&deflated, "o=z,")
    } else {
        (canvas.as_raw(), "")
    };
    let t1 = std::time::Instant::now();
    let chunks: Vec<&[u8]> = payload.chunks(3072).collect();
    let n = chunks.len();
    let mut out = String::with_capacity(payload.len() / 3 * 4 + n * 24);
    for (i, chunk) in chunks.into_iter().enumerate() {
        let more = u8::from(i + 1 < n);
        if i == 0 {
            write!(
                out,
                "\x1b_Gq=2,i={id},a=T,U=1,f=32,{encoding}t=d,\
                 s={w},v={h},r={rows},c={cols},m={more};"
            )
            .unwrap();
        } else {
            write!(out, "\x1b_Gq=2,m={more};").unwrap();
        }
        out.push_str(&kitty_b64(chunk));
        out.push_str("\x1b\\");
    }
    let base64_time = t1.elapsed();
    (out, WindowTiming { deflate: deflate_time, base64: Some(base64_time), shm: None })
}

/// The name of the shared memory object one graphics-window transmit is handed
/// over in (SQ-1374).
///
/// **Short on purpose, and per-transmit rather than per-image.**
///
/// Short, because macOS caps a POSIX shared memory name at 31 bytes *including*
/// the leading slash (`PSHMNAMLEN`) and refuses anything longer with
/// `ENAMETOOLONG` — measured, and the reason the upstream `t=s` patch transmitted
/// nothing at all on macOS until its names were shortened too. At `u32::MAX` for
/// both numbers this is 26 bytes.
///
/// Per-transmit, because the object is handed over ASYNCHRONOUSLY: the escape
/// rides out on the next flush and the terminal opens the object whenever it gets
/// to it. Naming the object after the window's image id — which is stable across
/// re-transmits, deliberately (SQ-0995) — would let a second transmit truncate the
/// object out from under a terminal still reading the first. A fresh name per
/// transmit cannot race, and the id in the escape is still the stable one, so
/// nothing about the placement changes.
#[cfg(unix)]
pub(super) fn kitty_shm_name(serial: u32) -> String {
    format!("/lnt-{}-{serial}", std::process::id())
}

/// Write `bytes` into a fresh shared memory object called `name`, or `None` if the
/// platform refuses (SQ-1374, SQ-1380).
///
/// **Linux writes through the descriptor.** `write(2)` on a POSIX shared memory
/// object allocates the `tmpfs` pages it touches inside the syscall itself — unlike
/// `ftruncate`, which only names a length and lets `tmpfs` allocate on first touch
/// — so there is nothing to reserve up front and no mapping to make: a graphics
/// window bigger than the remaining space in `/dev/shm` (64 MB by default in a
/// Docker container) answers `write_all` with an ordinary `io::Error` — `ENOSPC`,
/// or a short write — which takes the same failure path as any other: the object
/// is unlinked and this function returns `None`, and the caller already falls back
/// to the wire. `ftruncate` is unnecessary too, since the write itself sets the
/// object's length. `File::from_raw_fd` is the one `unsafe` this path still needs,
/// to hand the descriptor `shm_open` returned to something that can `write_all`;
/// past that point the file owns it and closes it on drop like any other.
///
/// The object still ends up sized to exactly the payload — a terminal rejects one
/// smaller than `s * v * bpp` (Ghostty says "shared memory size too small") — which
/// falls out of `write_all` writing exactly `bytes.len()` bytes to a fresh object.
///
/// **A successful object is not unlinked here.** The protocol makes the terminal
/// responsible: it unlinks the object once it has read it, which is what lets the
/// handover need no synchronisation. An object no terminal ever reads does outlive
/// us — which is exactly why [`kitty_shared_memory`] refuses this route unless the
/// terminal answered a probe by reading one. A FAILED write is unlinked, because
/// a half-written object is one nothing will ever be told about.
#[cfg(target_os = "linux")]
pub(super) fn kitty_shm_write(name: &str, bytes: &[u8]) -> Option<()> {
    use std::io::Write as _;
    use std::os::unix::io::FromRawFd as _;

    if bytes.is_empty() {
        return None;
    }
    let cname = std::ffi::CString::new(name).ok()?;
    // SAFETY: `cname` is a NUL-terminated string, and the returned descriptor is
    // checked against its documented failure value before use.
    let fd = unsafe { libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_TRUNC, 0o600) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is the valid, freshly-opened descriptor checked above, and
    // nothing else holds or closes it — `File` becomes its sole owner and closes
    // it on drop, on every path out of this function.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let written = file.write_all(bytes).is_ok();
    if !written {
        // SAFETY: `cname` is the same NUL-terminated string used to open it.
        unsafe {
            libc::shm_unlink(cname.as_ptr());
        }
        return None;
    }
    Some(())
}

/// Write `bytes` into a fresh shared memory object called `name`, or `None` if the
/// platform refuses (SQ-1374, SQ-1380).
///
/// **Every non-Linux Unix goes in through a mapping instead of `write(2)`**,
/// because macOS does not implement read/write on a shared memory object at all —
/// the descriptor opens, `ftruncate` succeeds, and the first write answers
/// `ENXIO`. A mapping is also how the terminal reads the object back. macOS has no
/// `tmpfs` quota to run out of either: its shared memory objects are ordinary
/// anonymous memory, so `ftruncate` here already reserves what it names and needs
/// no `posix_fallocate`-style forcing the way Linux's `tmpfs` does (see the
/// `target_os = "linux"` twin of this function).
///
/// The object is sized to exactly the payload: a terminal rejects one smaller than
/// `s * v * bpp` (Ghostty says "shared memory size too small"), and one larger
/// wastes a page it will never look at.
///
/// **A successful object is not unlinked here.** The protocol makes the terminal
/// responsible: it unlinks the object once it has read it, which is what lets the
/// handover need no synchronisation. An object no terminal ever reads does outlive
/// us — which is exactly why [`kitty_shared_memory`] refuses this route unless the
/// terminal answered a probe by reading one. A FAILED write is unlinked, because
/// a half-written object is one nothing will ever be told about.
#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn kitty_shm_write(name: &str, bytes: &[u8]) -> Option<()> {
    if bytes.is_empty() {
        return None;
    }
    let cname = std::ffi::CString::new(name).ok()?;
    // SAFETY: `cname` is a NUL-terminated string that outlives every call below;
    // each descriptor and pointer is checked against its documented failure value
    // before use, and both the mapping and the descriptor are released here.
    unsafe {
        let fd = libc::shm_open(cname.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_TRUNC, 0o600);
        if fd < 0 {
            return None;
        }
        let sized = libc::ftruncate(fd, bytes.len() as libc::off_t) == 0;
        let filled = if sized {
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                bytes.len(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                false
            } else {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
                libc::munmap(ptr, bytes.len());
                true
            }
        } else {
            false
        };
        libc::close(fd);
        if !filled {
            libc::shm_unlink(cname.as_ptr());
            return None;
        }
    }
    Some(())
}

/// The kitty transmit sequence for `canvas` as image `id`, handed over through a
/// POSIX shared memory object instead of down the wire (SQ-1374).
///
/// The same virtual placement [`kitty_transmit_virtual_timed`] emits — `U=1` with
/// an explicit `r×c` grid, `f=32` RGBA and the canvas's own `s`/`v` — with two
/// differences, both of them the point:
///
///   * `t=s` instead of `t=d`, and the payload is the base64 of the object's NAME
///     rather than of a megabyte of pixels. One chunk, always: there is nothing to
///     chunk, and `m=0` says so.
///   * No `o=z`. Deflate exists to make the wire smaller and the pixels are not on
///     the wire, so compressing them would cost the deflate and buy nothing — the
///     terminal reads the object with a copy.
///
/// `None` when the object could not be written, and the caller falls back to the
/// inline route. That fallback is load-bearing rather than defensive: a machine can
/// run out of shared memory, and a window that draws nothing is the failure mode
/// this whole path exists to avoid.
#[cfg(unix)]
pub(super) fn kitty_transmit_virtual_shm(
    canvas: &image::RgbaImage,
    id: u32,
    rows: u16,
    cols: u16,
    serial: u32,
) -> Option<(String, std::time::Duration)> {
    use std::fmt::Write as _;
    let (w, h) = (canvas.width(), canvas.height());
    let name = kitty_shm_name(serial);
    let t0 = std::time::Instant::now();
    kitty_shm_write(&name, canvas.as_raw())?;
    let elapsed = t0.elapsed();

    let mut out = String::with_capacity(name.len() * 2 + 80);
    write!(
        out,
        "\x1b_Gq=2,i={id},a=T,U=1,f=32,t=s,\
         s={w},v={h},r={rows},c={cols},m=0;"
    )
    .unwrap();
    out.push_str(&kitty_b64(name.as_bytes()));
    out.push_str("\x1b\\");
    Some((out, elapsed))
}

/// One graphics-window transmit, by whichever route this terminal answered for
/// (SQ-1374).
///
/// Shared memory first where the terminal took that probe; the inline route —
/// deflated or raw, per the same terminal's `o=z` answer — everywhere else, and
/// whenever a shared memory write fails. That fallback is why [`WindowWire`] is one
/// value rather than two parameters: the deflate answer is still needed on the path
/// where shared memory was preferred and did not work.
pub(super) fn kitty_transmit_virtual_wire(
    canvas: &image::RgbaImage,
    id: u32,
    rows: u16,
    cols: u16,
    wire: WindowWire,
) -> (String, WindowTiming) {
    #[cfg(unix)]
    if wire.shared_memory {
        // A serial per transmit, never the image id — see [`kitty_shm_name`].
        // Wrapping is fine and unreachable in practice: it would take four billion
        // window repaints, and the name only has to be unique against objects a
        // terminal has not finished reading yet.
        static SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some((out, elapsed)) = kitty_transmit_virtual_shm(canvas, id, rows, cols, serial) {
            return (out, WindowTiming { deflate: None, base64: None, shm: Some(elapsed) });
        }
    }
    kitty_transmit_virtual_timed(canvas, id, rows, cols, wire.compress)
}


/// One kitty APC chunk's control block (everything up to its first `;`, or its
/// whole body if it has none) and the chunk's total wire length (control block +
/// base64 + the `ESC \` terminator). Shared by [`measure_transmit`] and
/// [`measure_traffic`] so the two ways of reading this file's own emitted bytes
/// agree on where one chunk ends and the next begins.
pub(super) fn kitty_chunks(text: &str) -> Vec<(&str, u64)> {
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    while let Some(rel) = b[i..].windows(3).position(|w| w == b"\x1b_G") {
        let start = i + rel;
        // The chunk runs to its `ESC \`; a truncated one is measured to the end.
        let term = b[start..].windows(2).position(|w| w == b"\x1b\\");
        let end = term.map_or(b.len(), |p| start + p + 2);
        // `content_end` excludes the terminator itself, so a chunk with no `;` —
        // every delete escape, which has no payload to introduce one — does not
        // read its own `ESC \` as part of the last param's value (SQ-1201: that
        // silently broke `i=<id>` parsing on a delete, which has no other
        // separator after it).
        let content_end = term.map_or(b.len(), |p| start + p);
        let head_end = b[start..content_end].iter().position(|&c| c == b';').map_or(content_end, |p| start + p);
        out.push((&text[start + 3..head_end], (end - start) as u64));
        i = end;
        if i >= b.len() {
            break;
        }
    }
    out
}

pub(super) fn kitty_param(params: &str, key: &str) -> Option<u64> {
    params.split(',').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// Whether a chunk's own params (never its neighbours') are a transmit's — `a=T`
/// (transmit and display) or `a=t` (transmit only), the two spellings
/// `kitty_transmit_virtual` and `ratatui-image`'s encoder emit.
///
/// The gate `measure_traffic` needed and `measure_transmit` never did: lanthorn's
/// own kitty capability PROBE (`a=q`, sent once at startup to ask whether the
/// terminal answers at all) transmits a throwaway 1x1 `s=1,v=1` image too — s/v
/// alone is not "this is an upload", it is "this chunk names pixel geometry",
/// and a query names it for the same reason a transmit does. `measure_transmit`
/// was never fed that probe (only its own already-known-good transmit text), so
/// this never manifested there; `measure_traffic` reads the WHOLE wire, probe
/// included, and without this gate counted it as two extra uploads and two
/// falsely-stranded ids (SQ-1201).
pub(super) fn kitty_is_upload_action(params: &str) -> bool {
    params.split(',').any(|kv| kv == "a=T" || kv == "a=t")
}

/// Measure one transmit's cost off its own bytes. See [`UploadBytes`].
///
/// Cheap on purpose: it reads each chunk's control block — the few dozen bytes
/// before the `;` — and the chunk's length, never its payload. A 14 MB upload
/// costs the same to measure as a 200-byte one, which is what lets this sit on the
/// frame path where [`crate::terminal_dump::Traffic`] deliberately would not scan.
///
/// A chunk with no `s`/`v` is a continuation (`m=1` carries no geometry) and adds
/// wire without adding pixels, so a chunked upload is counted once.
///
/// `deletes` is counted here too (an `a=d` chunk, by `,a=d,` appearing in the
/// params — cheap, no allocation) but `freed_pixels`/`stranded_*` are always zero:
/// pairing a delete against the transmit it frees needs to have seen BOTH within
/// one measurement, and every call site that feeds this function a small
/// per-frame fragment (SQ-1005) never has both in the same fragment — the delete
/// for THIS id rides on a placement several frames later. [`measure_traffic`] is
/// the whole-capture sibling that does the pairing.
pub fn measure_transmit(transmit: &str) -> UploadBytes {
    let mut out = UploadBytes::default();
    for (params, wire) in kitty_chunks(transmit) {
        out.wire += wire;
        if params.split(',').any(|kv| kv == "a=d") {
            out.deletes += 1;
            continue;
        }
        if !kitty_is_upload_action(params) {
            continue;
        }
        // `S` is the kitty spec's own "size of the uncompressed data" and only ever
        // accompanies a compressed PNG; for the `f=32` RGBA we and the crate emit,
        // the declared geometry is the same fact and is always present.
        if let Some(size) = kitty_param(params, "S") {
            out.pixels += size;
            out.uploads += 1;
        } else if let (Some(w), Some(h)) = (kitty_param(params, "s"), kitty_param(params, "v")) {
            out.pixels += w * h * 4;
            out.uploads += 1;
        }
    }
    out
}

/// Measure a WHOLE capture's kitty traffic — every transmit and every delete in
/// `text`, paired by `i=<id>` (SQ-1201).
///
/// This is [`measure_transmit`] with the one thing a single small fragment can
/// never show it: a delete's OWN id, matched against a transmit the same text
/// also contains. A transmit sets/overwrites a local `id → pixels` ledger — a
/// re-transmit to an id already held REPLACES it in the terminal, per the kitty
/// spec's own re-transmit rule, so the ledger does too, rather than accumulating
/// both sizes — and a delete removes its id from the ledger, crediting
/// `freed_pixels`. Whatever the ledger still holds when `text` runs out is
/// `stranded_uploads`/`stranded_pixels`: transmitted in this capture, never freed
/// in it.
///
/// A delete naming an id nothing in `text` transmitted (freed by an EARLIER
/// capture not included here, or already freed once and named again) still counts
/// toward `deletes`, just not `freed_pixels` — there is nothing in this text to
/// credit it against.
///
/// A transmit with no `i=` is invisible to the ledger — neither freed nor
/// stranded — rather than assumed safe: `kitty_transmit_virtual` and
/// `ratatui-image`'s own kitty encoder both always state one (every id lanthorn
/// emits is meant to be freed later), so this is unreached on lanthorn's own
/// traffic today, and an id-less transmit still counts toward `pixels`/`uploads`
/// above, just not toward stranding.
///
/// Whole-capture, not per-frame: meant for a caller holding the ENTIRE emitted
/// stream at once (the pty-stream harness), where the cost of one `HashMap` is
/// nothing beside the megabytes of image data already in hand. [`measure_transmit`]
/// stays the frame-path measurer, unchanged, for exactly that reason.
pub fn measure_traffic(text: &str) -> UploadBytes {
    let mut out = UploadBytes::default();
    let mut outstanding: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    for (params, wire) in kitty_chunks(text) {
        out.wire += wire;
        let id = kitty_param(params, "i").map(|v| v as u32);
        if params.split(',').any(|kv| kv == "a=d") {
            out.deletes += 1;
            if let Some(id) = id {
                if let Some(px) = outstanding.remove(&id) {
                    out.freed_pixels += px;
                }
            }
            continue;
        }
        if !kitty_is_upload_action(params) {
            continue;
        }
        let size = kitty_param(params, "S").or_else(|| {
            let (w, h) = (kitty_param(params, "s")?, kitty_param(params, "v")?);
            Some(w * h * 4)
        });
        if let Some(size) = size {
            out.pixels += size;
            out.uploads += 1;
            if let Some(id) = id {
                outstanding.insert(id, size);
            }
        }
    }
    out.stranded_uploads = outstanding.len() as u64;
    out.stranded_pixels = outstanding.values().sum();
    out
}

/// Hash a canvas's pixels, so two canvases that look identical share one upload
/// (SQ-0564). Only ever compared against other hashes from this same function —
/// a collision would place the wrong cached image, which SipHash makes a
/// non-concern at these sizes.
pub(super) fn canvas_hash(canvas: &image::RgbaImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canvas.as_raw().hash(&mut h);
    canvas.dimensions().hash(&mut h);
    h.finish()
}

/// Every placeholder cell is a real buffer cell of forced width 1: the escape it
/// carries prints exactly one column, so ratatui's cursor bookkeeping and the
/// terminal's agree.
pub(super) const PLACEHOLDER_WIDTH: ratatui::buffer::CellDiffOption =
    ratatui::buffer::CellDiffOption::ForcedWidth(std::num::NonZeroU16::MIN);

/// Write ONE row of a kitty virtual placement into `buf`: `width` self-describing
/// placeholder cells starting at `(x0, y)`, each naming its own image row, image
/// column and id high byte, with the id's low 24 bits as the cell's foreground.
///
/// SELF-DESCRIBING IS THE POINT (SQ-0772). The protocol lets a placeholder with no
/// diacritics inherit its row/column/id-high-byte from the cell to its left, and
/// both this app and `ratatui-image` used to emit exactly one anchored cell per row
/// followed by bare continuations. That is legal until a later frame overpaints the
/// row's left edge: the anchor dies, the survivors keep only the foreground's low 24
/// bits, and the run either names an image the terminal does not hold (drawing
/// nothing) or — for lanthorn's own `0x00B0_xxxx` ids, whose high byte is zero —
/// resolves to the art's FIRST row, redrawn on every row and shifted a column right.
/// A cell that carries its own three diacritics cannot be orphaned by anything that
/// happens to its neighbours.
///
/// BUFFER-VISIBLE IS THE OTHER HALF. The old shape squeezed the whole row into the
/// first cell and marked the rest `Skip`, which made the placement invisible to
/// ratatui's damage model: a later frame that simply stopped drawing there produced
/// no diff, so nothing unpainted the placeholders and they outlived the frame that
/// made them (SQ-0763). Real cells diff like any other content — a frame that stops
/// placing writes ordinary spaces over them, and a frame that re-places writes the
/// identical symbols, so the steady state still costs nothing.
///
/// `prefix` (an upload, or queued deletes) rides on the first cell, as before.
/// Columns past the table's 297 entries fall back to bare continuation placeholders:
/// the protocol cannot express a larger explicit index either.
pub(super) fn kitty_place_row(
    buf: &mut Buffer,
    (x0, y): (u16, u16),
    width: u16,
    fg: Color,
    (row_d, extra_d): (char, char),
    prefix: Option<&str>,
) {
    let mut symbol = String::new();
    let mut prefix = prefix;
    for x in 0..width {
        symbol.clear();
        if let Some(seq) = prefix.take() {
            symbol.push_str(seq);
        }
        symbol.push('\u{10EEEE}');
        if let Some(&col_d) = KITTY_DIACRITICS.get(usize::from(x)) {
            symbol.push(row_d);
            symbol.push(col_d);
            symbol.push(extra_d);
        }
        let Some(cell) = buf.cell_mut((x0 + x, y)) else { continue };
        cell.set_symbol(&symbol).set_fg(fg).set_diff_option(PLACEHOLDER_WIDTH);
    }
}

/// Write the placeholder rows for image `id` into `buf` over `area`. `transmit`
/// (the first frame after an encode, plus any queued deletes) rides on the very
/// first cell.
pub(super) fn kitty_place_rows(id: u32, transmit: Option<&str>, area: Rect, buf: &mut Buffer) {
    let [id_hi, id_r, id_g, id_b] = id.to_be_bytes();
    // The third diacritic carries the id's HIGH byte; the foreground carries the
    // other 24 bits. Emitting the byte instead of a hardcoded zero is what makes
    // the placement agree with `kitty_transmit_virtual`'s full 32-bit `i={id}` for
    // any id, not merely for the `0x00B0_xxxx` range this file happens to allocate
    // from (SQ-0772). An id above the table's 296 is unplaceable in this protocol at
    // all; drawing nothing beats pointing the terminal at a truncated id, which names
    // a DIFFERENT image and is exactly the failure this quest is about. Unreachable
    // from the allocation above, which never sets the top byte.
    let Some(&extra_d) = KITTY_DIACRITICS.get(usize::from(id_hi)) else { return };
    let fg = Color::Rgb(id_r, id_g, id_b);
    let rows = area.height.min(KITTY_DIACRITICS.len() as u16);
    let mut transmit = transmit;
    for y in 0..rows {
        kitty_place_row(
            buf,
            (area.left(), area.top() + y),
            area.width,
            fg,
            (KITTY_DIACRITICS[usize::from(y)], extra_d),
            transmit.take(),
        );
    }
}

/// Render a `ratatui-image` protocol at `dest`, then re-lay any kitty virtual
/// placement it just wrote into the same self-describing, buffer-visible cells
/// [`kitty_place_row`] emits (SQ-0772).
///
/// WHY THIS WRAPPER EXISTS AT ALL. lanthorn has two kitty emitters and only owns
/// one. Everything drawn through a [`Protocol`] — the v6 raster composite, every
/// chrome-ring band, inline story art, the picker's cover tiles — is placed by
/// `ratatui-image`, whose kitty backend squeezes each row into its first cell and
/// marks the rest `Skip`, with an explicit "use inherited diacritic values"
/// comment. So a fix confined to our own emitter would fix none of the art the
/// player actually looks at: the Journey capture that opened SQ-0772 has the main
/// 920x575 composite orphaned by a later frame trimming its left edge, and that
/// composite is a `ratatui-image` placement. Re-laying the row afterwards makes
/// both emitters produce the same thing without forking the crate or hand-rolling
/// its encoders.
///
/// It reads back what the protocol wrote rather than being told: a `Protocol`
/// keeps its image id, its transmit sequence and its row diacritics private. The
/// three things needed are all fixed by the kitty protocol, not by the crate's
/// taste — the foreground SGR carrying the id's low 24 bits, the placeholder
/// character, and the (row, column, id-high-byte) diacritic triple after it. A row
/// that does not parse into exactly that shape is left exactly as the protocol
/// wrote it, so a half-block or sixel protocol (no placeholders at all) and any
/// future change to the crate's row format degrade to today's behaviour rather
/// than to garbage.
/// Returns the kitty image id it just placed, when the protocol was a kitty one —
/// `None` for half-blocks, sixel, iterm2 or a failed parse. That id is the only
/// handle we ever get on a `ratatui-image` upload, and without it the image can
/// never be freed in the terminal (SQ-0753): the crate keeps the id private, so
/// reading it back off the placement it just wrote is how we learn to name it.
pub fn place_protocol(proto: &Protocol, dest: Rect, buf: &mut Buffer) -> Option<u32> {
    place_protocol_with(proto, dest, buf, "", "").0
}

/// [`place_protocol`], with `prefix` (queued `a=d` deletes) riding on the very first
/// placeholder cell — ahead of the protocol's own upload, in the SAME output batch —
/// and `suffix` (deletes for uploads this placement SUPERSEDES) on the very last, so
/// they are freed only once the rect they cover has been covered again (SQ-0817).
///
/// That is the rule SQ-0637 set for graphics windows and the reason `pending_deletes`
/// is a queue rather than a direct write: escapes handed straight to stdout interleave
/// unpredictably with ratatui's diff, while a cell's symbol is emitted exactly where
/// the frame puts it. Bands and the raster composite are placed by `ratatui-image`
/// rather than by our own emitter, so they had no way to carry anything until the
/// re-seat gave them one (SQ-0753). Returns `None` — and carries nothing — for a
/// protocol with no placeholder rows, which is every non-kitty backend.
pub(super) fn place_protocol_with(
    proto: &Protocol,
    dest: Rect,
    buf: &mut Buffer,
    prefix: &str,
    suffix: &str,
) -> (Option<u32>, UploadBytes) {
    Image::new(proto).render(dest, buf);
    reseat_kitty_placement(dest, buf, prefix, suffix)
}

/// Rewrite each row of a `ratatui-image` kitty placement at `area` in place, and
/// return the image id the rows name. See [`place_protocol`]; a no-op returning
/// `None` for anything that is not a kitty placement.
pub(super) fn reseat_kitty_placement(
    area: Rect,
    buf: &mut Buffer,
    prefix: &str,
    suffix: &str,
) -> (Option<u32>, UploadBytes) {
    let mut id = None;
    // The image data rides on the placement's very first cell, ahead of that
    // cell's own placeholder — `anchor.prefix`. Measured here because this is
    // the ONE funnel every `ratatui-image` upload passes through (SQ-1005).
    let mut bytes = UploadBytes::default();
    let mut prefix = (!prefix.is_empty()).then_some(prefix);
    // The last cell this re-seat writes — where a `suffix` delete goes, so it is
    // emitted AFTER every byte of the placement it supersedes (SQ-0817).
    let mut last: Option<(u16, u16)> = None;
    for y in area.top()..area.bottom() {
        let Some(anchor) = parse_placement_anchor(buf, area.left(), y) else { continue };
        // The prefix is only ever consumed on a row we could also NAME, so the
        // caller's "did this carry my deletes?" question is answered by the id
        // coming back — never half-answered.
        let this_id = placement_id(anchor.fg, anchor.extra_d);
        id = id.or(this_id);
        bytes.add(measure_transmit(&anchor.prefix));
        let width = placement_row_width(buf, area, y).min(area.width);
        let head = match this_id.and(prefix.take()) {
            Some(p) => format!("{p}{}", anchor.prefix),
            None => anchor.prefix.clone(),
        };
        kitty_place_row(buf, (area.left(), y), width, anchor.fg, (anchor.row_d, anchor.extra_d), Some(&head));
        if width > 0 {
            last = Some((area.left() + width - 1, y));
        }
        // Anything the protocol marked `Skip` past the run we just rewrote (a
        // shorter row than the rect, which `full_width` clamping can produce)
        // would otherwise stay invisible to the diff for ever.
        for x in width..area.width {
            if let Some(c) = buf.cell_mut((area.left() + x, y)) {
                if c.diff_option == ratatui::buffer::CellDiffOption::Skip {
                    c.set_diff_option(ratatui::buffer::CellDiffOption::None);
                }
            }
        }
    }
    // The supersede-deletes go last, appended to the final placeholder cell — after
    // the transmit that rides on the first cell and after every placeholder row, so
    // the rect this delete frees is already covered by its replacement (SQ-0817).
    // Only ever attached to a placement we could NAME, for the same reason the prefix
    // is: the caller reads the returned id as "my escapes went out".
    if let (false, Some(pos), Some(_)) = (suffix.is_empty(), last, id) {
        if let Some(cell) = buf.cell_mut(pos) {
            let symbol = format!("{}{suffix}", cell.symbol());
            cell.set_symbol(&symbol);
        }
    }
    (id, bytes)
}

/// Reassemble a kitty image id from the two halves a placeholder row carries: the
/// low 24 bits ride as the cell foreground, the high byte as the third diacritic
/// (SQ-0753). The inverse of what [`kitty_place_rows`] writes, and of what
/// `ratatui-image` writes — both split the id exactly this way, because the kitty
/// protocol says to.
pub(super) fn placement_id(fg: Color, extra_d: char) -> Option<u32> {
    let Color::Rgb(r, g, b) = fg else { return None };
    let hi = KITTY_DIACRITICS.iter().position(|&c| c == extra_d)?;
    Some(u32::from_be_bytes([u8::try_from(hi).ok()?, r, g, b]))
}

/// The leftmost cell of one row of a `ratatui-image` kitty placement, read back
/// off the buffer it was rendered into.
///
/// Upstream 12.0.0-rc.0 draws a placement into REAL per-cell buffer cells — one
/// placeholder character per cell, with the id colour set as a proper `Style`
/// foreground (`cell.fg`) — rather than the fork's single anchor cell crammed
/// with the whole row's placeholder text plus a hand-rolled cursor-save/restore
/// escape dance (SQ-1510). Only a row's leftmost cell (`x == 0` in the crate's
/// own `render()`) carries the row/column/id-extra diacritics, and only the
/// placement's very first cell overall carries the transmit escape (the
/// crate's `AtomicBool`-gated `make_transmit()` fires once for the whole
/// image, not once per row).
pub(super) struct PlacementAnchor {
    /// Whatever rode ahead of the placeholder character in this cell — the
    /// crate's own transmit escape on the placement's first cell, empty
    /// everywhere else.
    pub(super) prefix: String,
    /// The id's low 24 bits, as the foreground the protocol chose.
    pub(super) fg: Color,
    /// The image row and id-high-byte diacritics, verbatim: the row index is the
    /// protocol's to decide (it slices images across rows) and the high byte is
    /// the only part of the id the foreground cannot carry.
    pub(super) row_d: char,
    pub(super) extra_d: char,
}

pub(super) fn parse_placement_anchor(buf: &Buffer, x: u16, y: u16) -> Option<PlacementAnchor> {
    let cell = buf.cell((x, y))?;
    let symbol = cell.symbol();
    let at = symbol.find('\u{10EEEE}')?;
    let (prefix, tail) = symbol.split_at(at);
    let mut diacritics = tail.chars().skip(1).take_while(|c| KITTY_DIACRITICS.contains(c));
    let row_d = diacritics.next()?;
    let _col_d = diacritics.next()?;
    let extra_d = diacritics.next()?;
    Some(PlacementAnchor { prefix: prefix.to_string(), fg: cell.fg, row_d, extra_d })
}

/// How many of row `y`'s cells, starting at `area.left()`, carry a placeholder —
/// the crate clamps a placement to its own image size, which can fall short of
/// `area.width` (SQ-1510: each cell is real now, so this is a scan rather than
/// counting characters crammed into one string).
pub(super) fn placement_row_width(buf: &Buffer, area: Rect, y: u16) -> u16 {
    (area.left()..area.right())
        .take_while(|&x| buf.cell((x, y)).is_some_and(|c| c.symbol().contains('\u{10EEEE}')))
        .count() as u16
}
