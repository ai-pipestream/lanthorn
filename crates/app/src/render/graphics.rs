//! Renders `WinNode::Graphics` canvases via ratatui-image, caching the built
//! protocol per (window, canvas version, area size).

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::engine::GraphicsWindow;

/// Two RGBA samples are "the same colour" within a small tolerance (anti-alias slack).
fn close(a: image::Rgba<u8>, b: image::Rgba<u8>) -> bool {
    (0..4).all(|i| a[i].abs_diff(b[i]) <= 8)
}

/// The native source pixel a scaled-canvas pixel `sp` samples under the `image`
/// crate's Nearest resize from `np` native pixels to `sp_max` scaled pixels:
/// `floor((sp + 0.5) · np / sp_max)`, clamped into `[0, np)`. Used only to bound
/// a band's native footprint for its freshness hash (SQ-0514); it mirrors, and
/// never replaces, the crate's own resize.
fn scaled_to_native(sp: u32, np: u32, sp_max: u32) -> u32 {
    let v = ((sp as f32 + 0.5) * np as f32 / sp_max.max(1) as f32).floor() as i64;
    v.clamp(0, np as i64 - 1) as u32
}

/// Resample `src` to `tw × th`, picking the filter per axis by the DIRECTION that
/// axis moves in (SQ-0824).
///
/// Nearest is what keeps pixel art crisp when an axis GROWS: it replicates whole
/// source pixels and invents no colours — a 1.48× magnification of Journey's canyon
/// plate comes back with the same 14 colours it went in with, where Triangle returns
/// 1636. It is exactly the wrong filter when an axis SHRINKS, because the same rule
/// that replicates a pixel on the way up DROPS one on the way down: at the smallest
/// pane swept, 54 of the plate's 222 columns and 56 of its 254 rows were never
/// sampled at all, and a dithered foreground is precisely where "some pixels survive
/// and their neighbours don't" reads as noise.
///
/// Measured against the per-axis ideal (an area average where the axis shrinks,
/// replication where it grows), on that same plate: Nearest scores an RMS of
/// 9.9–10.7 on a minification, Triangle 0.4–1.6. CatmullRom (2.1–2.6) and Lanczos3
/// (3.8–4.1) over-sharpen dithered art — they push adjacent-pixel contrast ABOVE the
/// ideal rather than fusing the dither — and Gaussian (2.4–3.5) over-blurs. Triangle,
/// whose kernel `image` widens to the resampling ratio, is the area filter here.
///
/// The axes go in separate passes because a band can grow on one and shrink on the
/// other — that is what [`GraphicsRender::draw_chrome_band_stretched`] exists for —
/// while `image` takes one filter for the pair. A pass at a 1:1 ratio is a bit-exact
/// identity under either filter, so the ordinary uniform case still costs one resize.
///
/// A BLENDING pass runs on ASSOCIATED (premultiplied) colour, and that is not a
/// refinement (SQ-0827). `image` filters the four channels independently, so where an
/// opaque pixel meets a transparent one it averages the transparent pixel's RGB —
/// which is `(0,0,0)`, a colour nothing on screen ever had — into its neighbour, and
/// drops the alpha to match. Composited, the pair reads as a dark fringe one pixel
/// wide: a Zork Zero flank whose art ends in the story page abutting clear canvas
/// emitted `(38,38,38,57)` at the seam, which over that page draws
/// `38·57/255 + 173·198/255 = 142` against the page's own 173 — the reported line down
/// both edges of the story pane. Nearest never blended, so the defect arrived with the
/// area filter above and not with anything that touched it.
///
/// Associating first makes the average one of light-with-nothing rather than
/// light-with-black, and the seam comes back out as page. It costs nothing where it is
/// not needed: at `a = 255` both directions round-trip exactly (`(255v+127)/255 = v`),
/// so a fully opaque plate — Journey's canyon, every RMS figure above — is bit-identical
/// either way, and a magnifying pass skips the conversion entirely.
///
/// # What it guarantees, to any caller
///
/// This is deliberately GENERAL, not v6-private: it takes an image and a target size and
/// assumes nothing about unit space, chrome bands or the Z-machine. Two guarantees, and
/// they are the two the rest of the app's resamples do not have (SQ-0829, which unifies
/// the seven other call sites onto this one — `render/inline_image.rs`, `render/screen.rs`,
/// `graphics.rs`, `picker_ui.rs` and `cover.rs` each picked their own filter and none
/// premultiplies):
///
/// * **Direction.** Each axis is filtered by the way it moves — replication when it grows
///   (pixel art stays crisp and gains no colours) and an area average when it shrinks
///   (nothing is dropped). A band that grows on one axis and shrinks on the other gets
///   both, in separate passes.
/// * **Alpha.** A blending pass runs on associated colour, so a transparent neighbour
///   contributes its coverage and not its `(0,0,0)`.
///
/// An axis that does not move is a bit-exact identity, so the common uniform case still
/// costs one resize.
pub(crate) fn resize_directional(src: &image::RgbaImage, tw: u32, th: u32) -> image::RgbaImage {
    use image::imageops::FilterType;
    let pick = |t: u32, s: u32| if t < s { FilterType::Triangle } else { FilterType::Nearest };
    let (sw, sh) = src.dimensions();
    let (fx, fy) = (pick(tw, sw), pick(th, sh));
    // Only a filter that AVERAGES neighbours can smear a transparent pixel's colour
    // into an opaque one; Nearest picks one source pixel whole.
    let blends = fx == FilterType::Triangle || fy == FilterType::Triangle;
    let associated;
    let src = if blends {
        associated = associate_alpha(src);
        &associated
    } else {
        src
    };
    let mut out = if fx == fy {
        image::imageops::resize(src, tw, th, fx)
    } else {
        let mid = image::imageops::resize(src, tw, sh, fx);
        image::imageops::resize(&mid, tw, th, fy)
    };
    if blends {
        unassociate_alpha(&mut out);
    }
    out
}

/// Straight (unassociated) RGBA → premultiplied, for [`resize_directional`].
fn associate_alpha(src: &image::RgbaImage) -> image::RgbaImage {
    let mut out = src.clone();
    for p in out.pixels_mut() {
        let a = u32::from(p.0[3]);
        for c in 0..3 {
            p.0[c] = ((u32::from(p.0[c]) * a + 127) / 255) as u8;
        }
    }
    out
}

/// Premultiplied RGBA → straight, undoing [`associate_alpha`] after the resample.
///
/// Triangle's weights are non-negative and sum to one, so no channel can come back
/// above its own alpha and the division cannot overflow; the `min` is belt and braces
/// against a future filter with negative lobes.
fn unassociate_alpha(img: &mut image::RgbaImage) {
    for p in img.pixels_mut() {
        let a = u32::from(p.0[3]);
        if a == 0 {
            p.0 = [0, 0, 0, 0];
            continue;
        }
        for c in 0..3 {
            p.0[c] = ((u32::from(p.0[c]) * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// The image the v6 raster composite goes to the protocol as, and the fit mode that
/// finishes it — the whole of [`GraphicsRender::encode_v6`]'s resampling decision,
/// kept pure so it can be measured (SQ-0824).
///
/// `Resize::Fit` only ever SHRINKS, so a pane bigger than the composite needs the
/// magnification done here: Nearest, capped at [`MAX_V6_UPSCALE`], after which the
/// protocol's own (also Nearest) fit at most nudges the result onto the cell grid.
///
/// A pane SMALLER than the composite needs no pre-scale at all, and that is the fix.
/// The scale used to be clamped at 1.0, which turned this branch into a full identity
/// copy of the canvas that bought nothing — and then left the actual shrink to the
/// protocol's DEFAULT filter, Nearest, which drops whole rows and columns exactly
/// where Journey's dithered foreground keeps its detail. Naming the area filter makes
/// it one resample, from the best source there is, in the right direction.
fn v6_fit_source(canvas: &image::RgbaImage, box_w: u32, box_h: u32) -> (image::RgbaImage, Resize) {
    let (cw, ch) = canvas.dimensions();
    let scale = ((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64)).min(MAX_V6_UPSCALE);
    if scale < 1.0 {
        return (canvas.clone(), Resize::Fit(Some(image::imageops::FilterType::Triangle)));
    }
    let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
    let scaled =
        image::imageops::resize(canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
    (scaled, Resize::Fit(None))
}

/// How [`resize_directional`] will treat a resample, for the band log and
/// `/dump-windows` (SQ-0824). Which filter a band went through is not inferable from
/// its cell rect — the direction depends on the band's own native extent against its
/// device box, and a band that magnifies sits beside one that shrinks — and "is this
/// art being minified?" is the question a report of aliasing in fine detail is
/// answered by.
fn resample_note(sw: u32, sh: u32, tw: u32, th: u32) -> String {
    let axis = |t: u32, s: u32| if t < s { "area" } else { "nearest" };
    format!("resample {sw}x{sh}->{tw}x{th} x:{} y:{}", axis(tw, sw), axis(th, sh))
}

/// How many native pixels beyond a band's own footprint can alter its scaled pixels
/// (SQ-0824). Nearest samples exactly one native pixel per scaled pixel, so a
/// magnifying (or 1:1) letterbox has no halo at all; a minifying one goes through
/// Triangle, whose kernel `image` widens to the resampling ratio, so a scaled pixel
/// averages roughly `1/s` native pixels either side of its centre. The band freshness
/// hash in [`GraphicsRender::draw_chrome_band`] covers the footprint plus this halo,
/// or a change just outside a band's own native rect could alter its image without
/// altering its key.
fn scale_halo(s: f32) -> u32 {
    if s >= 1.0 || !s.is_finite() || s <= 0.0 {
        0
    } else {
        (1.0 / s).ceil() as u32 + 1
    }
}

/// Render a graphics window directly as per-cell background colours when it is a
/// solid fill or a thin strip — the shape games use for chrome: panel dividers,
/// colour bars, backgrounds (e.g. Kerkerkruip draws its rules as 1×N / N×1 solid
/// graphics windows). Returns `true` when it painted the window this way.
///
/// Why not the image protocol: a thin 1-cell strip rendered as a kitty/sixel image
/// sits on a separate compositing layer that doesn't align to the character grid
/// and clobbers adjacent text. Sampling the canvas into cell backgrounds is exact,
/// grid-aligned, cheap, and needs no image-capable terminal. A detailed (non-thin,
/// non-uniform) canvas returns `false` so the caller falls back to the protocol.
/// `force` (v6 layered composite): skip the thin/uniform gate and always paint
/// every cell as the average of its opaque pixels (transparent cells left
/// untouched). This gives a low-res but grid-aligned, letterbox-free composite
/// for overlapping v6 background windows — the image-protocol path would paint a
/// solid grey letterbox over each mostly-empty canvas and clobber the layers
/// beneath. Non-v6 (Glulx) callers pass `false` to keep the detailed-image path.
pub fn render_graphics_as_cells(gw: &GraphicsWindow, area: Rect, buf: &mut Buffer, force: bool) -> bool {
    if area.width == 0 || area.height == 0 {
        return false;
    }
    let (cw, ch) = (gw.canvas.width(), gw.canvas.height());
    if cw == 0 || ch == 0 {
        return false;
    }
    // A window with no opaque pixel anywhere is blank — the game opened it but
    // never painted it (narco frames its story with graphics windows it leaves
    // empty). Report it HANDLED (painting nothing) so it does NOT fall through to
    // the image protocol, which would garble a transparent image into stray
    // chars/lines over the neighbouring windows. The scan short-circuits on the
    // first opaque pixel, so a real image pays almost nothing. (SQ-0338)
    if !gw.canvas.pixels().any(|p| p[3] >= 128) {
        return true;
    }
    // A cell's colour is the AVERAGE of the OPAQUE pixels in its canvas region, or
    // `None` if the region has none. Scanning the whole region (not just the centre
    // pixel) is essential: games draw their rules as 1–2px lines that rarely sit at
    // a cell's centre — a centre sample would miss them and render nothing. Any
    // opaque pixel in the cell surfaces the line's colour. (SQ-0332)
    let cell_color = |cx: u16, cy: u16| -> Option<image::Rgba<u8>> {
        let px0 = cx as u32 * cw / area.width as u32;
        let px1 = (((cx as u32 + 1) * cw / area.width as u32).max(px0 + 1)).min(cw);
        let py0 = cy as u32 * ch / area.height as u32;
        let py1 = (((cy as u32 + 1) * ch / area.height as u32).max(py0 + 1)).min(ch);
        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
        for py in py0..py1 {
            for px in px0..px1 {
                let p = gw.canvas.get_pixel(px, py);
                if p[3] >= 128 {
                    r += p[0] as u64;
                    g += p[1] as u64;
                    b += p[2] as u64;
                    n += 1;
                }
            }
        }
        (n > 0).then(|| image::Rgba([(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]))
    };
    // Handle a thin strip (a rule/divider) or a solid uniform fill as cells;
    // otherwise leave it to the image protocol. The uniform scan short-circuits on
    // the first differing/transparent cell, so a detailed image bails fast.
    //
    // A thin window is a RULE only when every opaque cell agrees on one colour
    // (gaps allowed — a partial-length rule). A DETAILED thin canvas — advent.blb's
    // clickable toolbar lands at 2 cells tall — is an image, not a divider:
    // averaging it into ─ glyphs shredded it into "two thin strips of pixels"
    // while the real icons never reached the image protocol. (SQ-0520)
    let thin = area.width.min(area.height) <= 2;
    let first = cell_color(0, 0);
    let uniform = first.is_some()
        && (0..area.height).all(|cy| (0..area.width).all(|cx| cell_color(cx, cy).is_some_and(|c| close(c, first.unwrap()))));
    let rule_like = thin && {
        // All opaque cells close to the first opaque cell's colour.
        let mut reference: Option<image::Rgba<u8>> = None;
        (0..area.height).all(|cy| {
            (0..area.width).all(|cx| match cell_color(cx, cy) {
                None => true,
                Some(c) => match reference {
                    None => {
                        reference = Some(c);
                        true
                    }
                    Some(r) => close(c, r),
                },
            })
        })
    };
    if !(force || rule_like || uniform) {
        return false;
    }
    // A window ≤2 cells in one dimension IS a rule/divider (Kerkerkruip's panel
    // borders). Draw it as a thin line GLYPH (fg = the rule colour, background
    // untouched) so it reads like a real rule at any width — not a full-cell colour
    // block that looks far thicker than a pixel interpreter's 1–2px bar. Like a
    // pixel interpreter, a white rule on a matching page then stays invisibly
    // subtle. Only larger (background) fills paint the whole cell. (SQ-0332)
    let line_glyph = if thin {
        // Vertical rule (tall & narrow) → │, horizontal rule → ─.
        Some(if area.height >= area.width { "\u{2502}" } else { "\u{2500}" })
    } else {
        None
    };
    for cy in 0..area.height {
        for cx in 0..area.width {
            let Some(p) = cell_color(cx, cy) else {
                continue; // no opaque pixels here → leave the underlying cell
            };
            if let Some(c) = buf.cell_mut((area.x + cx, area.y + cy)) {
                let fg = Color::Rgb(p[0], p[1], p[2]);
                match line_glyph {
                    Some(g) => {
                        // Preserve the underlying background; only the glyph + fg change.
                        let mut s = Style::default().fg(fg);
                        if let Some(bg) = c.style().bg {
                            s = s.bg(bg);
                        }
                        c.set_symbol(g).set_style(s);
                    }
                    None => {
                        c.set_symbol(" ").set_style(Style::default().bg(fg));
                    }
                }
            }
        }
    }
    true
}

/// The geometry needed to invert the v6 letterbox mapping: given a terminal
/// cell click, recover the game-pixel coordinate under the pointer. Recorded by
/// the last v6 draw path (single-canvas [`GraphicsRender::draw_v6_canvas`] or the
/// hybrid chrome ring) so [`map_click`](GraphicsRender::map_click) can be a pure
/// inverse of the forward scale-and-centre placement.
///
/// All fields are in the terminal's device-pixel space, measured relative to the
/// v6 pane's top-left cell (`pane_x`, `pane_y`). The drawn game image occupies the
/// device-pixel rect (`img_x`, `img_y`, `img_w`, `img_h`) inside that pane, and
/// maps a `native_w × native_h` game-pixel canvas across it, aspect-preserved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V6ClickMap {
    pub pane_x: u16,
    pub pane_y: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub img_x: f32,
    pub img_y: f32,
    pub img_w: f32,
    pub img_h: f32,
    pub native_w: u16,
    pub native_h: u16,
    /// SQ-0550: the rows of a TEXT strip that is drawn with one terminal row per
    /// GAME row, as `(top terminal row, row count, first game row)`.
    ///
    /// [`draw_chrome_text_strip`](crate::render::screen) does not place its runs
    /// through the letterbox scale — it buckets them by game row and packs those
    /// buckets onto CONSECUTIVE terminal rows from the strip's top (SQ-0543), so
    /// the menu reads as a compact block instead of inheriting the scale's row
    /// gaps. Inside this span the linear inverse below is therefore wrong, and
    /// wrong by a growing amount: at a scale of 1.725 the letterbox spreads game
    /// rows 1.53 terminal rows apart while the strip draws them 1 apart, so by the
    /// fifth menu row the click lands two game rows high. `None` when no such
    /// strip is on screen (every path but the hybrid Menu plan).
    pub text_rows: Option<(u16, u16, u16)>,
}

impl V6ClickMap {
    /// Map a terminal cell click at `(col, row)` to a 1-based game pixel
    /// `(x, y)`, or `None` when the cell lies outside the drawn game image
    /// (the letterbox margin, or another pane). This is the exact inverse of the
    /// forward letterbox: the forward map places game pixel `p` (0-based) at
    /// device offset `img_origin + (p + ½)·(img_size/native)`; inverting, a click
    /// at device position `d` recovers `p = ⌊(d − img_origin)/img_size · native⌋`.
    /// The click's device position is taken at the clicked cell's centre, giving
    /// a subcell estimate finer than the cell grid.
    pub fn map_click(&self, col: u16, row: u16) -> Option<(u16, u16)> {
        if self.img_w <= 0.0 || self.img_h <= 0.0 || self.native_w == 0 || self.native_h == 0 {
            return None;
        }
        if col < self.pane_x || row < self.pane_y {
            return None;
        }
        // Device-pixel position of the clicked cell's centre, relative to pane.
        let dx = (col - self.pane_x) as f32 * self.cell_w as f32 + self.cell_w as f32 / 2.0;
        let dy = (row - self.pane_y) as f32 * self.cell_h as f32 + self.cell_h as f32 / 2.0;
        let fx = (dx - self.img_x) / self.img_w;
        if !(0.0..1.0).contains(&fx) {
            return None;
        }
        let gx = (fx * self.native_w as f32).floor() as u16 + 1;
        // A consecutively-packed text strip (see `text_rows`) inverts by row index,
        // not by the letterbox: the clicked row's game row is its offset from the
        // strip's top, and the game pixel is that row's VERTICAL MIDDLE, so the whole
        // terminal row selects the menu line the player sees on it. The strip's own
        // rows are drawn, so they need no `fy` range check.
        let strip = self.text_rows.filter(|&(top, count, _)| row >= top && row < top.saturating_add(count));
        let gy = match strip {
            Some((top, _, first_game)) => (first_game as u32 + (row - top) as u32) * 16 + 8,
            None => {
                let fy = (dy - self.img_y) / self.img_h;
                if !(0.0..1.0).contains(&fy) {
                    return None;
                }
                (fy * self.native_h as f32).floor() as u32 + 1
            }
        };
        Some((gx.min(self.native_w), (gy.min(self.native_h as u32)) as u16))
    }
}

/// Largest integer upscale the v6 raster composite is encoded at (SQ-0469). A
/// native 320×200 game therefore encodes at most 1280×800 instead of the full
/// pane device resolution (which could be ~1920×1200 = 9 MB and cost hundreds of
/// ms to resize+PNG-encode). The protocol still fits/centres the smaller image in
/// the pane, so the only visible effect is that pixel art stops growing past this
/// multiple of its native size — crisp Nearest scaling either way.
///
/// SQ-0479 doubled the native canvas (320×200 → 640×400), so 2× here reaches the
/// SAME 1280×800 output ceiling the old 4× cap gave over the 320×200 canvas — the
/// encoded-pixel budget is unchanged, not quadrupled.
const MAX_V6_UPSCALE: f64 = 2.0;

/// A completed v6 raster encode (SQ-0469): the uploaded protocol plus the key it
/// was built for (`gen` + pane cell size) and the native canvas extent (for the
/// click map). Always rendered once present — a stale entry is shown until a
/// fresher encode lands, so the pane never flickers to blank on a change.
struct V6Ready {
    gen: u64,
    area_w: u16,
    area_h: u16,
    proto: Protocol,
    native_w: u16,
    native_h: u16,
    /// The kitty image id this composite was last PLACED as, read back off the
    /// placement (SQ-0753). `None` under a non-kitty protocol, and until the first
    /// [`GraphicsRender::redraw_v6`]. Without it the full-frame composite — the
    /// single largest upload babelmap makes, 2.8 MB on Journey — can only be
    /// forgotten, never freed.
    placed_id: Option<u32>,
}

/// The worker-thread handle for an in-flight v6 raster encode (SQ-0469). The
/// heavy resize + PNG/protocol encode (tens–hundreds of ms) runs off the UI
/// thread and yields a ready-to-render [`V6Ready`]. Coalesced: only one runs at a
/// time, and `poll_v6_job` installs its result (see `spawn_v6_encode`).
type V6Job = std::thread::JoinHandle<Option<V6Ready>>;

/// Which of a frame's draws a cached chrome band belongs to (SQ-0755).
///
/// A band's cache key is the cell rect it is drawn at — but one rect can legitimately
/// be drawn twice on a single frame. Journey's right flank IS its border column, so the
/// flank's own art and the divider extension replicated down the reclaimed gap land on
/// exactly the same cells. With the rect alone as the key each overwrote the other's
/// entry, every frame, so neither was ever a cache hit and both re-encoded forever.
/// Skipping one of the draws is not the answer: they carry different pixels — the flank
/// the column's true native extent, the extension one native row replicated past where
/// the canvas ends — and dropping either loses ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BandSlot {
    /// The band's own artwork, at the letterbox scale or stretched to a flank.
    Art = 0,
    /// A flank border column replicated down the gap between the art and the menu.
    DividerExtension = 1,
}

/// A chrome-band cache key: its [`BandSlot`] plus the cell rect it is drawn at.
pub type BandKey = (u8, u16, u16, u16, u16);

#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
    /// Letterbox geometry recorded by the most recent v6 draw, for inverting a
    /// terminal click back to a game pixel (Lane M mouse input). `None` until a
    /// v6 frame has been drawn.
    pub last_v6_map: Option<V6ClickMap>,
    /// Last-ready v6 pixel composite (Phase 1c / SQ-0469), keyed on a change
    /// generation + area rather than a full-buffer pixel hash. Built off-thread.
    v6: Option<V6Ready>,
    /// The in-flight background encode, if any (SQ-0469).
    v6_job: Option<V6Job>,
    /// Per-band cache for the v6 HYBRID chrome ring (Lane H): one uploaded
    /// protocol per band cell rect, keyed on the band rect with a stored
    /// hash of ONLY that band's own native sub-rect (plus scale/offset/cell/rect)
    /// so a change confined to one band leaves the other bands' uploads fresh
    /// (SQ-0514). Pruned each frame to the live band set by
    /// [`GraphicsRender::retain_chrome_bands`].
    /// Keyed on the rect a band is DRAWN at, plus a [`BandSlot`]: one rect can carry
    /// two different images on one frame — a flank's own art and the divider extension
    /// replicated over it — and keying on the rect alone made each overwrite the
    /// other's cache entry, so both re-encoded on every frame forever (SQ-0755).
    /// The third element is the kitty image id the band was last PLACED as, read
    /// back off the placement (SQ-0753) — `None` under any non-kitty protocol, and
    /// until the band's first place. It is the only handle we have on a
    /// `ratatui-image` upload, and it is what lets an abandoned band be freed in the
    /// terminal instead of merely forgotten here.
    chrome_bands: std::collections::HashMap<BandKey, (u64, Protocol, Option<u32>)>,
    /// What happened to each chrome band on the last v6 frame, for `/dump-windows`
    /// (SQ-0587). Whether a band was a cache hit, whether it encoded, and what size
    /// the protocol reported — the questions that decide whether a missing image is a
    /// stale placement, a failed upload, or a band that was never drawn at all. The
    /// answers cannot be inferred from the geometry, which is why this exists.
    pub band_log: Vec<String>,
    /// How many chrome bands have been ENCODED (uploaded) since launch (SQ-0587).
    /// The band log only shows the latest frame; this shows whether an upload ever
    /// happened across an event like a restore. Dump it either side: if the number
    /// has not moved, every band was a cache hit and the terminal was sent nothing.
    pub band_encodes: u64,
    /// The whole native chrome canvas scaled to device pixels, shared across all
    /// bands of a frame so the expensive Nearest resize runs at most ONCE per
    /// changed frame instead of once per band (SQ-0514). Keyed on the canvas
    /// content + scale + scaled dimensions; each band crops its sub-rect from it,
    /// so band output stays byte-identical to a per-band whole-canvas resize.
    chrome_scaled: Option<(u64, image::RgbaImage)>,
    /// Kitty-protocol graphics windows: one transmitted image per window,
    /// placed with an EXPLICIT r×c grid so the terminal scales the canvas to
    /// exactly the window's cell rect (SQ-0520 — see `render_kitty_virtual`).
    kitty_wins: std::collections::HashMap<u32, KittyWindowImage>,
    /// Monotonic id source for `kitty_wins` (offset into a private id range).
    next_kitty_id: u32,
    /// Kitty `a=d` delete escapes for uploads abandoned WHOLESALE — a window that
    /// closed ([`GraphicsRender::retain_live`]) or a window whose area changed, which
    /// restarts its cache (SQ-0637). Dropping the [`KittyWindowImage`] only forgets
    /// the ids on OUR side; the terminal keeps every transmitted generation (up to
    /// [`KITTY_CACHE`] per window) until it is told to free them, so a closed window
    /// or a sequence of resizes leaked terminal image memory until the terminal's own
    /// quota forced an eviction. The escapes are queued rather than written here
    /// because they must ride the SAME output batch as the rest of our kitty traffic
    /// (buffer cell symbols, not a direct stdout write that would interleave with
    /// ratatui's diff): they are flushed by the next placement, or by
    /// [`GraphicsRender::flush_kitty_deletes`] when no window places again.
    pending_deletes: String,
    /// Kitty `a=d` deletes for an upload that is still COVERING ITS RECT — a chrome
    /// band re-encoded into its own cache slot, the raster composite superseded by
    /// the next encode (SQ-0817). These cannot ride ahead of the frame's traffic the
    /// way [`Self::pending_deletes`] does: the id being freed is the one the terminal
    /// is showing at that very rect, and its replacement is up to 618 KB of image data
    /// away. Freeing it first leaves the cells with nothing to draw for the length of
    /// that transfer — which is exactly the flicker Zork Zero's compass, its map and
    /// Arthur's Merlin composite all showed, once per frame the game changed.
    ///
    /// So they ride as a SUFFIX on the placement that supersedes them, in the same
    /// batch: transmit new, place new, free old. The rect is covered throughout, and
    /// the memory is still freed on the same frame — deferring by a frame would work
    /// too, but there is no reason to hold 618 KB longer than the width of one
    /// placement.
    deletes_after_place: String,
    /// Machine-readable record of the protocol traffic this frame produced
    /// (SQ-0590) — see [`GraphicsOp`]. `band_log` above is the human dump for
    /// `/dump-windows`; this is the same events in a form a test can assert on,
    /// which is what lets the pixel path be exercised headlessly at all.
    ops: Vec<GraphicsOp>,
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
const OPS_MAX: usize = 512;

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

/// How many distinct canvases one graphics window keeps uploaded in the terminal
/// (SQ-0564). Bounds the terminal's image memory — at advent.blb's toolbar size
/// (1104×36 RGBA ≈ 155 KiB) this is ~1.2 MiB per window — while still covering a
/// game that cycles through a handful of fixed frames.
const KITTY_CACHE: usize = 8;

/// The kitty images backing one graphics window (SQ-0520/SQ-0564).
struct KittyWindowImage {
    /// Canvas version the cache was last reconciled against. Hashing a canvas
    /// costs a pass over every pixel, so it only happens when the game actually
    /// drew something — not on every frame.
    version: u64,
    w: u16,
    h: u16,
    /// The id currently placed on screen; 0 before the first upload.
    id: u32,
    /// The transmit escape (plus any eviction deletes), prepended to the first
    /// placeholder row of the next emitted frame, then dropped — the terminal
    /// keeps the image by id after that.
    pending_transmit: Option<String>,
    /// Content hash → uploaded id for every canvas transmitted at this size,
    /// least-recently-placed FIRST. A game that flips between a few fixed
    /// canvases re-places an id it already sent instead of re-uploading:
    /// advent.blb's toolbar alternates between the resting bar and one
    /// pressed-button variant per press, so releasing a button used to re-send
    /// the whole canvas for a picture the terminal already had.
    sent: Vec<(u64, u32)>,
}

impl std::fmt::Debug for GraphicsRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphicsRender").field("cached", &self.cache.len()).finish()
    }
}

impl GraphicsRender {
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
            self.render_kitty_virtual(gw, area, buf);
            return;
        }
        let fresh = matches!(self.cache.get(&gw.win),
            Some((v, w, h, _)) if *v == gw.version && *w == area.width && *h == area.height);
        if !fresh {
            let img = image::DynamicImage::ImageRgba8((*gw.canvas).clone());
            // `Scale` upscales a small canvas to fill the window (aspect
            // preserved, Nearest filter → crisp pixel art); `Fit` leaves it at
            // native size, centered. Scott room pictures want the former.
            let resize = if gw.upscale { Resize::Scale(None) } else { Resize::Fit(None) };
            match picker.new_protocol(img, Size::new(area.width, area.height), resize) {
                Ok(p) => { self.cache.insert(gw.win, (gw.version, area.width, area.height, p)); }
                Err(_) => return,
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
        let dead: Vec<u32> = self.kitty_wins.keys().copied().filter(|w| !live.contains(w)).collect();
        for win in dead {
            if let Some(entry) = self.kitty_wins.remove(&win) {
                self.queue_kitty_deletes(&entry, win);
            }
        }
    }

    /// Queue `a=d,d=I` deletes for every id an abandoned [`KittyWindowImage`] still
    /// holds in the terminal, and record one [`GraphicsOp::Drop`] per id (SQ-0637).
    /// `d=I` frees the image data AND its placements, so nothing deleted here can be
    /// re-placed — which is correct: the entry that knew these ids is gone.
    fn queue_kitty_deletes(&mut self, entry: &KittyWindowImage, win: u32) {
        use std::fmt::Write as _;
        for &(_, id) in &entry.sent {
            write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
            self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Window(win) });
        }
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
    fn queue_protocol_delete(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.pending_deletes, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\").expect("write to String");
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
    fn queue_protocol_delete_after_place(&mut self, id: Option<u32>) {
        use std::fmt::Write as _;
        if let Some(id) = id {
            write!(self.deletes_after_place, "\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
                .expect("write to String");
        }
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
    /// Each distinct canvas is transmitted once per area size and kept in the
    /// terminal (up to [`KITTY_CACHE`]), keyed by a hash of its pixels: a game
    /// that returns to a canvas it has drawn before re-places that id and sends
    /// no image data at all. Placeholder cells are re-written every frame
    /// (identical content, so the terminal diff drops them when unchanged).
    ///
    /// Keying on CONTENT rather than on the canvas version is what makes a
    /// button press cheap. advent.blb's toolbar redraws itself from scratch to
    /// show a button pressed and again to release it, bumping the version each
    /// time; the release repaints the resting bar pixel-for-pixel, so it costs a
    /// re-place instead of a second full upload. (SQ-0564)
    fn render_kitty_virtual(&mut self, gw: &GraphicsWindow, area: Rect, buf: &mut Buffer) {
        use std::fmt::Write as _;
        // A resize invalidates every upload for this window: the placement's r×c
        // grid is baked into the transmission, so nothing cached at the old size
        // can be re-placed. Start the window's cache over — and DELETE the uploads
        // being abandoned, or every resize would strand a whole cache generation in
        // the terminal (SQ-0637).
        let previous = self.kitty_wins.remove(&gw.win);
        let mut entry = match previous {
            Some(e) if e.w == area.width && e.h == area.height => e,
            stale => {
                if let Some(stale) = stale {
                    self.queue_kitty_deletes(&stale, gw.win);
                }
                KittyWindowImage {
                    version: 0,
                    w: area.width,
                    h: area.height,
                    id: 0,
                    pending_transmit: None,
                    sent: Vec::new(),
                }
            }
        };
        if entry.id == 0 || entry.version != gw.version {
            entry.version = gw.version;
            let hash = canvas_hash(&gw.canvas);
            match entry.sent.iter().position(|&(h, _)| h == hash) {
                // The terminal already has these exact pixels: re-place that id,
                // transmit nothing, and mark it most-recently-placed.
                Some(pos) => {
                    let sent = entry.sent.remove(pos);
                    entry.id = sent.1;
                    entry.sent.push(sent);
                    self.note_op(GraphicsOp::Reuse {
                        target: GraphicsTarget::Window(gw.win),
                        id: Some(entry.id),
                    });
                }
                None => {
                    // Private id range, disjoint from ratatui-image's random ids in
                    // practice. The top byte stays 0, which is what keeps every id
                    // this path allocates nameable by `kitty_place_rows` — it now
                    // encodes the byte rather than assuming it (SQ-0772), but the
                    // diacritic table tops out at 296 and an id above that could not
                    // be placed at all.
                    self.next_kitty_id = self.next_kitty_id.wrapping_add(1) & 0x000F_FFFF;
                    let id = 0x00B0_0000 | self.next_kitty_id;
                    let mut transmit = String::new();
                    // Past the cap, free the least-recently-placed upload. Never
                    // the id being placed nor the one still on screen — both sit
                    // at the most-recent end, so no frame is ever blanked while
                    // its replacement is still being parsed. d=I frees data +
                    // placements.
                    while entry.sent.len() >= KITTY_CACHE {
                        let (_, dead) = entry.sent.remove(0);
                        write!(transmit, "\x1b_Gq=2,a=d,d=I,i={dead}\x1b\\").unwrap();
                        self.note_op(GraphicsOp::Drop { target: GraphicsTarget::Window(gw.win) });
                    }
                    transmit.push_str(&kitty_transmit_virtual(&gw.canvas, id, area.height, area.width));
                    entry.pending_transmit = Some(transmit);
                    entry.id = id;
                    entry.sent.push((hash, id));
                    self.note_op(GraphicsOp::Upload {
                        target: GraphicsTarget::Window(gw.win),
                        id: Some(id),
                        cells: (area.width, area.height),
                    });
                }
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

    /// How many distinct canvases this window currently has uploaded, and whether
    /// the last reconcile transmitted one (observability hook, SQ-0564).
    #[cfg(test)]
    fn kitty_uploads(&self, win: u32) -> Option<(usize, u32)> {
        self.kitty_wins.get(&win).map(|e| (e.sent.len(), e.id))
    }

    /// The kitty delete escapes waiting to be written to the terminal (SQ-0637).
    #[cfg(test)]
    fn queued_deletes(&self) -> &str {
        &self.pending_deletes
    }

    /// Resize + encode a native v6 canvas into a terminal image protocol,
    /// upscaled (Nearest → crisp pixel art) to fill `area`'s device pixels with
    /// aspect preserved, capped at [`MAX_V6_UPSCALE`]. Pure/self-contained so it
    /// can run on a worker thread (SQ-0469). Returns `None` if the protocol
    /// encode fails.
    fn encode_v6(picker: &Picker, canvas: &image::RgbaImage, gen: u64, area: Rect) -> Option<V6Ready> {
        let fs = picker.font_size();
        let box_w = area.width as u32 * fs.width.max(1) as u32;
        let box_h = area.height as u32 * fs.height.max(1) as u32;
        let (img, fit) = v6_fit_source(canvas, box_w, box_h);
        let img = image::DynamicImage::ImageRgba8(img);
        match picker.new_protocol(img, Size::new(area.width, area.height), fit) {
            Ok(proto) => Some(V6Ready {
                gen,
                area_w: area.width,
                area_h: area.height,
                proto,
                native_w: canvas.width() as u16,
                native_h: canvas.height() as u16,
                placed_id: None,
            }),
            Err(_) => None,
        }
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
    pub fn spawn_v6_encode(&mut self, picker: &Picker, canvas: image::RgbaImage, gen: u64, area: Rect) {
        if area.width == 0 || area.height == 0 || canvas.width() == 0 || canvas.height() == 0 {
            return;
        }
        if self.v6.is_none() {
            self.v6 = Self::encode_v6(picker, &canvas, gen, area);
            return;
        }
        let picker = picker.clone();
        self.v6_job = Some(std::thread::spawn(move || Self::encode_v6(&picker, &canvas, gen, area)));
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

    /// Poll the background v6 encode: if it finished, install its protocol as the
    /// new last-ready composite and return true (the caller should redraw). The
    /// result is always installed (even if the generation has since advanced) so
    /// the display keeps converging to the latest encoded frame during a burst;
    /// an out-of-date entry is still rendered until the next encode lands, so the
    /// pane never blanks. (SQ-0469)
    pub fn poll_v6_job(&mut self) -> bool {
        let done = self.v6_job.as_ref().is_some_and(|j| j.is_finished());
        if !done {
            return false;
        }
        let job = self.v6_job.take().expect("checked above");
        if let Ok(Some(ready)) = job.join() {
            // The composite being replaced is a whole-pane upload; free it in the
            // terminal rather than letting the assignment orphan it (SQ-0753). A
            // raster-mode game re-encodes on every visible change, so this is the
            // heaviest leak in the app — scopa strands one full frame per move.
            //
            // It is also the composite the terminal is showing RIGHT NOW, and the
            // replacement is a whole pane of image data away, so the delete rides
            // BEHIND the placement that covers the pane again (SQ-0817).
            let stale = self.v6.replace(ready);
            self.queue_protocol_delete_after_place(stale.and_then(|r| r.placed_id));
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
        let (native_w, native_h) = (ready.native_w, ready.native_h);
        // Queued deletes ride out on this placement, in the same batch (SQ-0637's
        // rule); deferred back to the queue if this backend has no row to carry them.
        // The supersede-deletes ride BEHIND it instead (SQ-0817) — see
        // [`Self::deletes_after_place`].
        let pending = std::mem::take(&mut self.pending_deletes);
        let after = std::mem::take(&mut self.deletes_after_place);
        let placed_id = place_protocol_with(proto, dest, buf, &pending, &after);
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
        // back to a game pixel (Lane M). The image fills `dest`'s cells
        // aspect-preserved, so the click map's image rect is `dest` in device
        // pixels and its native extent is the canvas dimensions.
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
            img_x: (dest.x - area.x) as f32 * cw as f32,
            img_y: (dest.y - area.y) as f32 * ch as f32,
            img_w: dest.width as f32 * cw as f32,
            img_h: dest.height as f32 * ch as f32,
            native_w,
            native_h,
            text_rows: None,
        });
    }

    /// Record the letterbox click map for the HYBRID draw path (Lane H), where the
    /// game image is drawn as a chrome ring around a terminal viewport rather than
    /// one canvas. `scale` is the same [`uniform_scale`](crate::render::v6_layout::uniform_scale)
    /// the chrome bands were placed through, so the recovered game pixel matches
    /// what the player sees. `pane` is the whole v6 pane's cell rect; `native` is
    /// the chrome canvas's game-pixel extent; `cell_px` is the font cell size.
    /// `text_rows` carries the interactive text strip's consecutive-row mapping when
    /// one is on screen — see [`V6ClickMap::text_rows`].
    pub fn record_hybrid_click_map(
        &mut self,
        pane: Rect,
        scale: &crate::render::v6_layout::Scale,
        native: (u16, u16),
        cell_px: (u16, u16),
        text_rows: Option<(u16, u16, u16)>,
    ) {
        let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
        self.last_v6_map = Some(V6ClickMap {
            pane_x: pane.x,
            pane_y: pane.y,
            cell_w: cw,
            cell_h: ch,
            img_x: scale.off_x as f32,
            img_y: scale.off_y as f32,
            img_w: native.0 as f32 * scale.s,
            img_h: native.1 as f32 * scale.s,
            native_w: native.0,
            native_h: native.1,
            text_rows,
        });
    }

    /// Record the click map for the v6 CELL path — frameless mode (SQ-0461), and
    /// the same fallback taken on a terminal with no image protocol.
    ///
    /// That path draws no game image at all: it re-lays the v6 screen out as
    /// ordinary terminal text filling the whole pane. There is therefore no
    /// letterbox to invert — but the pane still stands for the game's screen, so
    /// a click is mapped by proportion: the pane's full cell rect covers the
    /// native `(width, height)` game-pixel canvas, and a click at a given
    /// fraction across/down the pane yields the game pixel at the same fraction.
    ///
    /// Without this, clicks in frameless mode were simply dead — the raster and
    /// hybrid paths each recorded a map and this one recorded none, so
    /// `map_click` returned a stale (or missing) geometry while games that ask
    /// for mouse input (the capability bit is advertised) got nothing.
    /// (SQ-0532/A-F4)
    pub fn record_frameless_click_map(&mut self, pane: Rect, native: (u16, u16), cell_px: (u16, u16)) {
        let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
        self.last_v6_map = Some(V6ClickMap {
            pane_x: pane.x,
            pane_y: pane.y,
            cell_w: cw,
            cell_h: ch,
            img_x: 0.0,
            img_y: 0.0,
            img_w: pane.width as f32 * cw as f32,
            img_h: pane.height as f32 * ch as f32,
            native_w: native.0,
            native_h: native.1,
            text_rows: None,
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
        self.ops.clear();
    }

    /// Record one unit of protocol traffic (SQ-0590), bounded by [`OPS_MAX`] so a
    /// path with no frame boundary cannot grow it without limit.
    fn note_op(&mut self, op: GraphicsOp) {
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
    #[cfg(test)]
    fn chrome_band_id(&self, key: BandKey) -> Option<u32> {
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
    fn scaled_chrome(&mut self, chrome_canvas: &image::RgbaImage, s: f32, sw: u32, sh: u32) -> &image::RgbaImage {
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
        let mut h = std::collections::hash_map::DefaultHasher::new();
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
            (nx0, nx1, ny0, ny1).hash(&mut h);
            for ny in ny0..ny1 {
                for nx in nx0..nx1 {
                    chrome_canvas.get_pixel(nx, ny).0.hash(&mut h);
                }
            }
        } else {
            // Fully in the letterbox margin — no native pixels feed it.
            0u8.hash(&mut h);
        }
        scale.s.to_bits().hash(&mut h);
        (scale.off_x, scale.off_y).hash(&mut h);
        (cw, ch).hash(&mut h);
        (rel_x0, rel_y0, bw, bh).hash(&mut h);
        let hash = h.finish();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _, _)) if *v == hash);
        if !fresh {
            // Copy the sub-rect under this band out of the frame-shared scaled
            // chrome into a band-sized image (letterbox area outside the scaled
            // chrome stays transparent). The whole-canvas resize happens at most
            // once per changed frame (cached in `chrome_scaled`), not per band.
            let band_img = {
                let scaled = self.scaled_chrome(chrome_canvas, scale.s, sw, sh);
                let mut band_img = image::RgbaImage::new(bw, bh);
                for by in 0..bh {
                    let sy = rel_y0 as i64 + by as i64 - scale.off_y as i64;
                    if sy < 0 || sy as u32 >= sh {
                        continue;
                    }
                    for bx in 0..bw {
                        let sx = rel_x0 as i64 + bx as i64 - scale.off_x as i64;
                        if sx < 0 || sx as u32 >= sw {
                            continue;
                        }
                        band_img.put_pixel(bx, by, *scaled.get_pixel(sx as u32, sy as u32));
                    }
                }
                band_img
            };
            let img = image::DynamicImage::ImageRgba8(band_img);
            match picker.new_protocol(img, Size::new(band.width, band.height), Resize::Fit(None)) {
                Ok(p) => {
                    self.band_encodes += 1;
                    // Whatever this key held is being replaced: free it in the
                    // terminal before the only record of its id is overwritten
                    // (SQ-0753).
                    let stale = self.chrome_bands.insert(key, (hash, p, None));
                    self.queue_protocol_delete_after_place(stale.and_then(|(_, _, id)| id));
                    self.note_op(GraphicsOp::Upload {
                        target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                        id: None,
                        cells: (band.width, band.height),
                    });
                }
                Err(_) => return,
            }
        } else {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
        }
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
        if !matches!(placed, Some((_, _, Some(_)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        match placed {
            Some((dest, sz, id)) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}): {} · proto {}x{} · placed {}x{} at ({},{}) · native {} · {}",
                    band.width, band.height, band.x, band.y,
                    if fresh { "cache HIT" } else { "encoded" },
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    match footprint {
                        Some((x, y, w, h)) => format!("{w}x{h}@({x},{y})"),
                        None => "— (entirely in the letterbox margin)".to_string(),
                    },
                    resample_note(nw, nh, sw, sh),
                ));
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
    /// freed when the entry is abandoned (SQ-0753). A no-op under a protocol with
    /// no addressable id (half-blocks, sixel).
    fn remember_band_id(&mut self, key: BandKey, id: Option<u32>) {
        if let Some((_, _, slot)) = self.chrome_bands.get_mut(&key) {
            *slot = id;
        }
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
    pub fn draw_chrome_band_stretched(
        &mut self,
        picker: &Picker,
        chrome_canvas: &image::RgbaImage,
        band: Rect,
        crop: (u32, u32, u32, u32),
        slot: BandSlot,
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
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Hash ONLY the crop's native footprint (coords + pixels) plus the target
        // device size — the stretch factor is (bw,bh)/(cw_n,ch_n), so both ends are
        // covered. A change outside this native rect (e.g. the banner's Score/Moves)
        // never alters the hash, keeping the flank's cached upload fresh (SQ-0514).
        (slot as u8).hash(&mut h); // discriminator vs. draw_chrome_band keys on the same map
        (cx, cy, cw_n, ch_n).hash(&mut h);
        let x1 = (cx + cw_n).min(canvas_w);
        let y1 = (cy + ch_n).min(canvas_h);
        for ny in cy..y1 {
            for nx in cx..x1 {
                chrome_canvas.get_pixel(nx, ny).0.hash(&mut h);
            }
        }
        (bw, bh).hash(&mut h);
        let hash = h.finish();
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _, _)) if *v == hash);
        if !fresh {
            // Copy the native crop (clamped to the canvas) into its own image, then
            // resize it to the band's device box. Transparent native pixels stay
            // transparent, so the theme backdrop shows through gaps in the flank.
            let mut src = image::RgbaImage::new(cw_n, ch_n);
            for oy in 0..ch_n {
                let ny = cy + oy;
                if ny >= canvas_h {
                    break;
                }
                for ox in 0..cw_n {
                    let nx = cx + ox;
                    if nx >= canvas_w {
                        break;
                    }
                    src.put_pixel(ox, oy, *chrome_canvas.get_pixel(nx, ny));
                }
            }
            let stretched = resize_directional(&src, bw, bh);
            let img = image::DynamicImage::ImageRgba8(stretched);
            match picker.new_protocol(img, Size::new(band.width, band.height), Resize::Fit(None)) {
                Ok(p) => {
                    self.band_encodes += 1;
                    let stale = self.chrome_bands.insert(key, (hash, p, None));
                    self.queue_protocol_delete_after_place(stale.and_then(|(_, _, id)| id));
                    self.note_op(GraphicsOp::Upload {
                        target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                        id: None,
                        cells: (band.width, band.height),
                    });
                }
                Err(_) => return,
            }
        } else {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
        }
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
        if !matches!(placed, Some((_, _, Some(_)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        match placed {
            Some((dest, sz, id)) => {
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, stretched]: {} · proto {}x{} · placed {}x{} at ({},{}) · native {cw_n}x{ch_n}@({cx},{cy}) · {}",
                    band.width, band.height, band.x, band.y,
                    if fresh { "cache HIT" } else { "encoded" },
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    resample_note(cw_n, ch_n, bw, bh),
                ));
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
    /// in the canvas and cannot be named as a crop. `src` is resized to the
    /// band's device box exactly as [`Self::draw_chrome_band_stretched`] resizes
    /// a crop — and because the caller sizes `src` at the uniform letterbox
    /// scale, that resize is 1:1 in aspect: the flank keeps the same horizontal
    /// factor as the top plate above it, which is the user's rule for a title
    /// that has top artwork, and plain scaling for one that does not.
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

        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (slot as u8).hash(&mut h);
        (src.width(), src.height()).hash(&mut h);
        src.as_raw().hash(&mut h);
        (bw, bh).hash(&mut h);
        let hash = h.finish();
        let key = (slot as u8, band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _, _)) if *v == hash);
        if !fresh {
            let scaled = resize_directional(src, bw, bh);
            let img = image::DynamicImage::ImageRgba8(scaled);
            match picker.new_protocol(img, Size::new(band.width, band.height), Resize::Fit(None)) {
                Ok(p) => {
                    self.band_encodes += 1;
                    let stale = self.chrome_bands.insert(key, (hash, p, None));
                    self.queue_protocol_delete_after_place(stale.and_then(|(_, _, id)| id));
                    self.note_op(GraphicsOp::Upload {
                        target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                        id: None,
                        cells: (band.width, band.height),
                    });
                }
                Err(_) => return,
            }
        } else {
            self.note_op(GraphicsOp::Reuse {
                target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                id: None,
            });
        }
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
        if !matches!(placed, Some((_, _, Some(_)))) {
            self.pending_deletes = pending;
            // Nothing was placed, so nothing on screen depends on the supersede
            // deletes either — hand them to the ordinary queue rather than strand them.
            self.pending_deletes.push_str(&after);
        }
        match placed {
            Some((dest, sz, id)) => {
                let (blank, run, run_at) = blank_rows(src);
                self.band_log.push(format!(
                    "band {}x{}@({},{}) [{slot:?}, tiled]: {} · proto {}x{} · placed {}x{} at ({},{}) · source {}x{} native px · blank rows {}, longest run {} at {} · {}",
                    band.width, band.height, band.x, band.y,
                    if fresh { "cache HIT" } else { "encoded" },
                    sz.width, sz.height, dest.width, dest.height, dest.x, dest.y,
                    src.width(), src.height(),
                    blank, run, run_at,
                    resample_note(src.width(), src.height(), bw, bh),
                ));
                self.remember_band_id(key, id);
                self.note_op(GraphicsOp::Place {
                    target: GraphicsTarget::Band(key.1, key.2, key.3, key.4),
                    at: (dest.x, dest.y, dest.width, dest.height),
                });
            }
            None => self.band_log.push(format!(
                "band {}x{}@({},{}) [{slot:?}, tiled]: NO PROTOCOL (encode failed)",
                band.width, band.height, band.x, band.y
            )),
        }
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
fn blank_rows(src: &image::RgbaImage) -> (u32, u32, u32) {
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
const KITTY_DIACRITICS: [char; 297] = [
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
fn kitty_b64(data: &[u8]) -> String {
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

/// The kitty transmit sequence for `canvas` as image `id`: a VIRTUAL placement
/// (`U=1`) declaring an explicit `r×c` grid, so the terminal scales the image
/// to exactly the placeholder rect (SQ-0520). RGBA chunked per the protocol's
/// 4096-encoded-byte limit. (No tmux passthrough — matches the app's existing
/// kitty support, which targets direct terminals.)
fn kitty_transmit_virtual(canvas: &image::RgbaImage, id: u32, rows: u16, cols: u16) -> String {
    use std::fmt::Write as _;
    let (w, h) = (canvas.width(), canvas.height());
    let bytes = canvas.as_raw();
    let chunks: Vec<&[u8]> = bytes.chunks(3072).collect();
    let n = chunks.len();
    let mut out = String::with_capacity(bytes.len() / 3 * 4 + n * 24);
    for (i, chunk) in chunks.into_iter().enumerate() {
        let more = u8::from(i + 1 < n);
        if i == 0 {
            write!(out, "\x1b_Gq=2,i={id},a=T,U=1,f=32,t=d,s={w},v={h},r={rows},c={cols},m={more};").unwrap();
        } else {
            write!(out, "\x1b_Gq=2,m={more};").unwrap();
        }
        out.push_str(&kitty_b64(chunk));
        out.push_str("\x1b\\");
    }
    out
}

/// Hash a canvas's pixels, so two canvases that look identical share one upload
/// (SQ-0564). Only ever compared against other hashes from this same function —
/// a collision would place the wrong cached image, which SipHash makes a
/// non-concern at these sizes.
fn canvas_hash(canvas: &image::RgbaImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canvas.as_raw().hash(&mut h);
    canvas.dimensions().hash(&mut h);
    h.finish()
}

/// Every placeholder cell is a real buffer cell of forced width 1: the escape it
/// carries prints exactly one column, so ratatui's cursor bookkeeping and the
/// terminal's agree.
const PLACEHOLDER_WIDTH: ratatui::buffer::CellDiffOption =
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
/// nothing) or — for babelmap's own `0x00B0_xxxx` ids, whose high byte is zero —
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
fn kitty_place_row(
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
fn kitty_place_rows(id: u32, transmit: Option<&str>, area: Rect, buf: &mut Buffer) {
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
/// WHY THIS WRAPPER EXISTS AT ALL. babelmap has two kitty emitters and only owns
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
    place_protocol_with(proto, dest, buf, "", "")
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
fn place_protocol_with(
    proto: &Protocol,
    dest: Rect,
    buf: &mut Buffer,
    prefix: &str,
    suffix: &str,
) -> Option<u32> {
    Image::new(proto).render(dest, buf);
    reseat_kitty_placement(dest, buf, prefix, suffix)
}

/// Rewrite each row of a `ratatui-image` kitty placement at `area` in place, and
/// return the image id the rows name. See [`place_protocol`]; a no-op returning
/// `None` for anything that is not a kitty placement.
fn reseat_kitty_placement(area: Rect, buf: &mut Buffer, prefix: &str, suffix: &str) -> Option<u32> {
    let mut id = None;
    let mut prefix = (!prefix.is_empty()).then_some(prefix);
    // The last cell this re-seat writes — where a `suffix` delete goes, so it is
    // emitted AFTER every byte of the placement it supersedes (SQ-0817).
    let mut last: Option<(u16, u16)> = None;
    for y in area.top()..area.bottom() {
        let Some(symbol) = buf.cell((area.left(), y)).map(|c| c.symbol().to_string()) else { continue };
        let Some(row) = parse_placement_row(&symbol) else { continue };
        // The prefix is only ever consumed on a row we could also NAME, so the
        // caller's "did this carry my deletes?" question is answered by the id
        // coming back — never half-answered.
        let this_id = placement_id(row.fg, row.extra_d);
        id = id.or(this_id);
        let width = row.cells.min(area.width);
        let head = match this_id.and(prefix.take()) {
            Some(p) => format!("{p}{}", row.prefix),
            None => row.prefix.to_string(),
        };
        kitty_place_row(buf, (area.left(), y), width, row.fg, (row.row_d, row.extra_d), Some(&head));
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
    id
}

/// Reassemble a kitty image id from the two halves a placeholder row carries: the
/// low 24 bits ride as the cell foreground, the high byte as the third diacritic
/// (SQ-0753). The inverse of what [`kitty_place_rows`] writes, and of what
/// `ratatui-image` writes — both split the id exactly this way, because the kitty
/// protocol says to.
fn placement_id(fg: Color, extra_d: char) -> Option<u32> {
    let Color::Rgb(r, g, b) = fg else { return None };
    let hi = KITTY_DIACRITICS.iter().position(|&c| c == extra_d)?;
    Some(u32::from_be_bytes([u8::try_from(hi).ok()?, r, g, b]))
}

/// One `ratatui-image` placeholder row, read back off the cell it was written to.
struct PlacementRow<'a> {
    /// Everything before the row's own escapes — the image upload, when this is
    /// the row that carries it. Passed through untouched.
    prefix: &'a str,
    /// The id's low 24 bits, as the foreground the protocol chose.
    fg: Color,
    /// The image row and id-high-byte diacritics, verbatim: the row index is the
    /// protocol's to decide (it slices images across rows) and the high byte is
    /// the only part of the id the foreground cannot carry.
    row_d: char,
    extra_d: char,
    /// Placeholder cells in the row.
    cells: u16,
}

fn parse_placement_row(symbol: &str) -> Option<PlacementRow<'_>> {
    let at = symbol.find('\u{10EEEE}')?;
    let (head, tail) = symbol.split_at(at);
    // `ESC[38;2;r;g;bm` immediately before the first placeholder is the id colour.
    let sgr = head.rfind("\x1b[38;2;")?;
    let rgb = head.get(sgr + 7..)?.strip_suffix('m')?;
    let mut parts = rgb.split(';');
    let mut byte = || parts.next()?.parse::<u8>().ok();
    let fg = Color::Rgb(byte()?, byte()?, byte()?);
    if parts.next().is_some() {
        return None;
    }
    // The protocol's own cursor-save sits between the upload and the id colour.
    let prefix = &head[..head[..sgr].rfind("\x1b[s")?];

    let mut diacritics = tail.chars().skip(1).take_while(|c| KITTY_DIACRITICS.contains(c));
    let row_d = diacritics.next()?;
    let _col_d = diacritics.next()?;
    let extra_d = diacritics.next()?;
    let cells = u16::try_from(tail.chars().filter(|&c| c == '\u{10EEEE}').count()).ok()?;
    Some(PlacementRow { prefix, fg, row_d, extra_d, cells })
}

/// SQ-0824: the resampler picks its filter by direction, so a pane smaller than the
/// artwork stops dropping the rows and columns a dithered picture keeps its detail in.
///
/// Judged against the per-axis single-resample ideal — an area average where an axis
/// shrinks, replication where it grows — which is the thing "resample once, from the
/// best source, with the right filter" is trying to be. Nearest cannot come close on a
/// minification: it is off by an RMS of ~10 on a dithered plate where the directional
/// resampler is off by ~1, and that gap IS the reported aliasing.
///
/// FALSIFY by restoring `image::imageops::resize(src, tw, th, FilterType::Nearest)` as
/// the body of `resize_directional`: every minifying case fails on its RMS bound.
#[cfg(test)]
mod resample_tests {
    use super::resize_directional;
    use image::{Rgba, RgbaImage};

    /// A plate in the shape of the artwork this quest is about: a four-ink palette laid
    /// down as broad flat regions, joined by checkerboard-dithered transition bands
    /// (the shadow gradients Journey's canyon is built from), with hard one-pixel edges
    /// cutting across (the foreground rocks). Synthetic, so the case runs on a machine
    /// without the gitignored story — the shipped floppy's real plate is measured by
    /// `v6_art_resample.rs`, which skips when the fixture is absent.
    fn dithered_plate(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        let inks = [
            Rgba([0x20, 0x18, 0x10, 0xff]),
            Rgba([0xc8, 0x70, 0x28, 0xff]),
            Rgba([0x48, 0x38, 0x60, 0xff]),
            Rgba([0xf0, 0xe0, 0xa0, 0xff]),
        ];
        for y in 0..h {
            for x in 0..w {
                // Four horizontal regions; the middle third of each boundary dithers
                // between the two inks either side of it rather than stepping.
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

    /// The area-weighted average — the correct answer for a shrinking axis.
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

    /// Nearest replication along x — the correct answer for a growing axis.
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

    fn ideal(src: &RgbaImage, tw: u32, th: u32) -> RgbaImage {
        let (sw, sh) = src.dimensions();
        let mid = if tw < sw { area_average(src, tw, sh) } else { nearest_x(src, tw) };
        let t = transpose(&mid);
        let t = if th < sh { area_average(&t, th, tw) } else { nearest_x(&t, th) };
        transpose(&t)
    }

    fn rms(a: &RgbaImage, b: &RgbaImage) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions());
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

    /// Every direction regime the three art paths can put a resample in, at the ratios
    /// the pane sweep actually produces on Journey's 222×254 canyon plate. The bound is
    /// 2.0 everywhere; Nearest measures 9.9–10.7 on each of the shrinking cases.
    #[test]
    fn resampling_tracks_the_single_resample_ideal_in_every_direction() {
        let src = dithered_plate(222, 254);
        for (tw, th, regime) in [
            (168u32, 198u32, "both axes shrink, hard"),
            (200, 234, "both axes shrink"),
            (212, 244, "both axes shrink, barely"),
            (212, 256, "x shrinks while y grows"),
            (217, 259, "x shrinks barely while y grows"),
            (224, 270, "both axes grow, barely"),
            (328, 378, "both axes grow"),
            (222, 254, "no change at all"),
        ] {
            let got = resize_directional(&src, tw, th);
            assert_eq!(got.dimensions(), (tw, th), "222x254 -> {tw}x{th} ({regime})");
            let ideal = ideal(&src, tw, th);
            let err = rms(&got, &ideal);
            let nearest =
                rms(&image::imageops::resize(&src, tw, th, image::imageops::FilterType::Nearest), &ideal);
            if tw < 222 || th < 254 {
                assert!(
                    err < 4.0 && err * 2.0 < nearest,
                    "222x254 -> {tw}x{th} ({regime}): RMS {err:.3} against the per-axis \
                     single-resample ideal, where a plain Nearest resample scores {nearest:.3}. \
                     A filter chosen by direction must stay under 4 AND beat Nearest by more \
                     than 2x — Nearest on a shrinking axis IS the reported aliasing."
                );
            } else {
                assert_eq!(
                    err, 0.0,
                    "222x254 -> {tw}x{th} ({regime}): a resample that only magnifies must BE \
                     the ideal, pixel for pixel — that is the crisp look at native size and \
                     above, and it is not negotiable"
                );
            }
        }
    }

    /// The other half of the rule, and the one that must NOT regress: magnification is
    /// still bit-exact pixel replication, so art at or above its native size keeps the
    /// crisp look `MAX_V6_UPSCALE` and the corpus tests exist to protect. A smoothing
    /// filter would show up here instantly — Triangle turns this plate's four inks into
    /// hundreds of blends.
    #[test]
    fn magnification_invents_no_colours() {
        let src = dithered_plate(222, 254);
        let inks = |img: &RgbaImage| {
            img.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>().len()
        };
        for (tw, th) in [(444u32, 508u32), (328, 378), (224, 270), (222, 508)] {
            let got = resize_directional(&src, tw, th);
            assert_eq!(
                inks(&got),
                inks(&src),
                "222x254 -> {tw}x{th} magnifies, so every pixel must be a replicated \
                 source pixel — a growing axis is exactly what Nearest is for"
            );
        }
    }

    /// SQ-0824, second pass: **the `V6_ART_SCALE` pre-double is not a resample the final
    /// one compounds with — it composes away exactly.**
    ///
    /// The premise under investigation was that v6 art is "pre-scaled 2x and then scaled
    /// again off the pre-scale", so the picture the player sees has been through two
    /// samplers. It has been through two *calls*, and that is not the same thing. The
    /// doubling is `image`'s Nearest at an integer ratio, which is pure replication:
    /// output pixel `i` takes source `floor(i/2)`. A second Nearest then takes
    /// `floor((o+0.5)·2N/T)` of that, and `floor(floor(2u)/2) = floor(u)` for every real
    /// `u ≥ 0` — so the pair IS `floor((o+0.5)·N/T)`, the single Nearest resample straight
    /// from the artwork's own resolution. Not approximately: bit for bit, which is what
    /// this case asserts, in both directions and at the ratios the pane sweep produces.
    ///
    /// The consequence is worth stating plainly, because it decides a fix: sampling the
    /// native artwork instead of the unit-space replica **cannot change a single pixel**
    /// anywhere the final resample magnifies — which is every pane at or above ~80
    /// columns, including the 166x44 the defect was reported at. Whatever is wrong there
    /// is not double sampling. (Journey's plate at 166x44 is one art pixel per 3.69 device
    /// pixels, so Nearest emits it as runs of 3 and 4 — the unevenness is the non-integer
    /// magnification itself, and it survives any change of source.)
    ///
    /// Where the two DO differ is the direction decision above, and only there: a target
    /// between the artwork's size and its double magnifies from native while it minifies
    /// from the replica, so the same target picks Nearest one way and the area filter the
    /// other. That is the `(168, 198)` row below, and it is a policy question — this
    /// quest's own shrink win — not an arithmetic one.
    #[test]
    fn the_art_scale_predouble_composes_away_under_nearest() {
        use image::imageops::FilterType;
        let native = dithered_plate(111, 127);
        // Exactly what `session::v6_scaled_art` does at `V6_ART_SCALE`.
        let doubled = image::imageops::resize(&native, 222, 254, FilterType::Nearest);
        for (tw, th) in
            [(444u32, 508u32), (456, 522), (410, 468), (328, 378), (224, 270), (222, 254), (168, 198)]
        {
            assert_eq!(
                image::imageops::resize(&doubled, tw, th, FilterType::Nearest).as_raw(),
                image::imageops::resize(&native, tw, th, FilterType::Nearest).as_raw(),
                "111x127 doubled to 222x254 and then Nearest-resampled to {tw}x{th} must be \
                 BIT-IDENTICAL to one Nearest resample from the native artwork — an integer \
                 replication composes with the sampler that follows it, so the pre-double is \
                 not a second sampling and cannot be what distorts the picture"
            );
        }
        // …and therefore the shipped resampler is a single resample from native wherever
        // it magnifies, which is the regime the defect was reported in.
        for (tw, th) in [(444u32, 508u32), (456, 522), (410, 468), (328, 378), (224, 270)] {
            assert_eq!(
                resize_directional(&doubled, tw, th).as_raw(),
                image::imageops::resize(&native, tw, th, FilterType::Nearest).as_raw(),
                "a magnifying {tw}x{th} band already samples the native artwork exactly once"
            );
        }
    }

    /// The two-pass form leans on a 1:1 pass being a true identity, or a band that grows
    /// on one axis and shrinks on the other would be resampled twice over on the axis
    /// that did not move.
    #[test]
    fn an_unmoved_axis_is_untouched() {
        let src = dithered_plate(64, 64);
        assert_eq!(resize_directional(&src, 64, 64).as_raw(), src.as_raw(), "1:1 is identity");
        let narrowed = resize_directional(&src, 48, 64);
        let twice = resize_directional(&narrowed, 48, 64);
        assert_eq!(twice.as_raw(), narrowed.as_raw(), "an unmoved axis re-resamples to itself");
    }

    /// A minifying letterbox reads native pixels either side of the one Nearest would
    /// have picked, so the band freshness hash has to cover a halo; a magnifying one
    /// does not, and must not pay for one.
    #[test]
    fn only_a_shrinking_letterbox_carries_a_halo() {
        assert_eq!(super::scale_halo(2.0), 0, "magnifying: Nearest samples one pixel");
        assert_eq!(super::scale_halo(1.0), 0, "1:1: Nearest samples one pixel");
        assert_eq!(super::scale_halo(0.5), 3, "half size: a kernel two native pixels wide");
        assert_eq!(super::scale_halo(0.0), 0, "degenerate scales are not a panic");
    }

    /// The RASTER composite's half of the same rule, measured through the protocol's own
    /// `Resize` rather than around it — a pane smaller than the composite must land on
    /// the area-averaged ideal, and one bigger must still be exact pixel replication.
    ///
    /// FALSIFY by restoring `.clamp(1.0, MAX_V6_UPSCALE)` and `Resize::Fit(None)` in
    /// `v6_fit_source`: the shrinking cases fail on their RMS bound, because the pane
    /// then gets an identity copy of the canvas followed by a Nearest shrink.
    #[test]
    fn the_raster_composite_takes_one_resample_in_the_right_direction() {
        use ratatui_image::FontSize;
        // Journey's composite: its 320x200 screen at the uniform `V6_ART_SCALE`.
        let canvas = dithered_plate(640, 400);
        let fs = FontSize::new(8, 18);
        for (cols, rows) in [(60u16, 24u16), (70, 30), (76, 28), (100, 40), (160, 60)] {
            let (box_w, box_h) = (cols as u32 * 8, rows as u32 * 18);
            let (src, fit) = super::v6_fit_source(&canvas, box_w, box_h);
            let dyn_src = image::DynamicImage::ImageRgba8(src.clone());
            let cells = fit.size_for(&dyn_src, fs, ratatui::layout::Size::new(cols, rows));
            let got = fit.resize(&dyn_src, fs, cells, None).to_rgba8();
            // The protocol pads its output out to the cell grid with a TRANSPARENT
            // background; the composite itself is opaque, so the drawn extent is
            // exactly the opaque part. Measuring the padding would drown the signal.
            let dw = (0..got.width()).filter(|&x| got.get_pixel(x, 0).0[3] != 0).count() as u32;
            let dh = (0..got.height()).filter(|&y| got.get_pixel(0, y).0[3] != 0).count() as u32;
            let drawn = image::imageops::crop_imm(&got, 0, 0, dw, dh).to_image();
            let ideal = ideal(&canvas, dw, dh);
            let err = rms(&drawn, &ideal);
            assert!(dw > 0 && dh > 0, "raster pane {cols}x{rows}: nothing opaque was drawn");
            if dw < canvas.width() {
                let nearest = rms(
                    &image::imageops::resize(&canvas, dw, dh, image::imageops::FilterType::Nearest),
                    &ideal,
                );
                assert!(
                    err < 6.0 && err * 2.0 < nearest,
                    "raster pane {cols}x{rows}: the composite reached {dw}x{dh} with an RMS of \
                     {err:.3} against the single-resample ideal, where the Nearest shrink it \
                     used to get scores {nearest:.3}. A pane smaller than the composite must be \
                     ONE area-filtered shrink, not an identity copy followed by a Nearest one."
                );
            } else {
                assert_eq!(
                    err, 0.0,
                    "raster pane {cols}x{rows}: at or above native size the composite must be \
                     replicated pixel for pixel"
                );
            }
        }
    }

    // ── SQ-0827: the seam where art ends and the canvas is clear ────────────────

    /// A flank in the shape Zork Zero's is: a column of art, then the story page it
    /// abuts, then CLEAR canvas past the crop's inner edge. The alpha step is the whole
    /// specimen — the art either side of it is flat, so anything dark that comes out of
    /// the resample was invented by the filter.
    fn flank_with_a_clear_edge(w: u32, h: u32, art_to: u32, page_to: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = if x < art_to {
                    Rgba([0x44, 0x00, 0x00, 0xff]) // the pillar
                } else if x < page_to {
                    Rgba([0xad, 0xad, 0xad, 0xff]) // the game's page, opaque
                } else {
                    Rgba([0, 0, 0, 0]) // clear: the cell background shows through
                };
                img.put_pixel(x, y, p);
            }
        }
        img
    }

    /// What a terminal puts on screen for one emitted pixel: straight-alpha compositing
    /// over the cell background behind the band.
    fn over(px: Rgba<u8>, bg: [u8; 3]) -> [u8; 3] {
        let a = f64::from(px.0[3]) / 255.0;
        let mut out = [0u8; 3];
        for c in 0..3 {
            out[c] = (f64::from(px.0[c]) * a + f64::from(bg[c]) * (1.0 - a)).round() as u8;
        }
        out
    }

    /// SQ-0827, the reported symptom: a one-pixel darker column down the story pane's
    /// edge. Where the flank's opaque page meets clear canvas, no emitted pixel may
    /// composite darker than the page it is part of — the band is drawn OVER a cell
    /// flooded with that same page, so a correct seam is invisible.
    ///
    /// FALSIFY by dropping the `associate_alpha`/`unassociate_alpha` pair from
    /// `resize_directional`: the seam pixel comes back as `(38,38,38,57)`, which over
    /// the page draws 142 against 173, and this fails naming that column.
    #[test]
    fn a_shrinking_band_leaves_no_dark_fringe_where_its_art_meets_clear_canvas() {
        const PAGE: [u8; 3] = [0xad, 0xad, 0xad];
        // 95 native columns into an 84px band is Zork Zero's own ratio on the Amiga
        // floppy at an 83-column terminal; the rest sweep the regime either side of it.
        for (nw, bw) in [(95u32, 84u32), (94, 70), (95, 91), (128, 64), (100, 99)] {
            let src = flank_with_a_clear_edge(nw, 40, nw * 3 / 4, nw - 7);
            let got = resize_directional(&src, bw, 34);
            let y = got.height() / 2;
            for x in 0..got.width() {
                let px = *got.get_pixel(x, y);
                let seen = over(px, PAGE);
                assert!(
                    // Only a PARTLY TRANSPARENT pixel is judged: an opaque one that
                    // reads darker is the pillar itself, which is meant to be dark.
                    // The one level of slack is the 8-bit premultiply round trip — the
                    // page's own 173 comes back as 172 — against the 31 levels the
                    // reported line was worth.
                    px.0[3] == 0xff || (0..3).all(|c| seen[c] + 1 >= PAGE[c]),
                    "{nw}->{bw}: emitted pixel x={x} is {:?}, which over the page behind the \
                     band draws {seen:?} against the page's own {PAGE:?} — a partly \
                     transparent pixel whose colour was averaged with the (0,0,0) of clear \
                     canvas IS the one-pixel dark line down the story pane's edge",
                    px.0
                );
            }
        }
    }

    /// …and the same specimen at the same ratios under a GROWING axis, which takes the
    /// Nearest arm and must not pay for the conversion at all: every output pixel is a
    /// source pixel, alpha included.
    #[test]
    fn a_growing_band_still_replicates_whole_pixels_across_an_alpha_edge() {
        let src = flank_with_a_clear_edge(84, 20, 60, 77);
        let got = resize_directional(&src, 95, 30);
        let inks: std::collections::HashSet<_> = got.pixels().map(|p| p.0).collect();
        assert_eq!(
            inks,
            src.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>(),
            "a magnifying pass invents no colour, so an alpha edge stays a step"
        );
    }

    /// The reason the conversion is free where it is not needed, and the reason every
    /// RMS figure in this module is untouched by it: on FULLY OPAQUE art it is the
    /// identity, bit for bit. Journey's canyon plate is exactly that.
    #[test]
    fn associating_a_fully_opaque_plate_is_the_identity() {
        let src = dithered_plate(222, 254);
        assert!(src.pixels().all(|p| p.0[3] == 0xff), "the specimen must be opaque to prove this");
        assert_eq!(super::associate_alpha(&src).as_raw(), src.as_raw(), "premultiply by 1");
        let mut back = src.clone();
        super::unassociate_alpha(&mut back);
        assert_eq!(back.as_raw(), src.as_raw(), "and divide by 1");
        // Both axes shrink: the single-pass Triangle arm.
        for (tw, th) in [(200u32, 234u32), (168, 198)] {
            assert_eq!(
                resize_directional(&src, tw, th).as_raw(),
                image::imageops::resize(&src, tw, th, image::imageops::FilterType::Triangle)
                    .as_raw(),
                "222x254 -> {tw}x{th}: opaque art resamples exactly as it did before SQ-0827"
            );
        }
        // …and the mixed arm, whose two passes are equally untouched.
        let mid = image::imageops::resize(&src, 212, 254, image::imageops::FilterType::Triangle);
        assert_eq!(
            resize_directional(&src, 212, 256).as_raw(),
            image::imageops::resize(&mid, 212, 256, image::imageops::FilterType::Nearest).as_raw(),
            "222x254 -> 212x256 shrinks on x and grows on y, and is unchanged too"
        );
    }

    /// The band log names the direction, because a cell rect never could.
    #[test]
    fn the_band_log_names_the_direction() {
        assert_eq!(super::resample_note(222, 254, 200, 234), "resample 222x254->200x234 x:area y:area");
        assert_eq!(
            super::resample_note(222, 254, 212, 256),
            "resample 222x254->212x256 x:area y:nearest"
        );
        assert_eq!(
            super::resample_note(222, 254, 328, 378),
            "resample 222x254->328x378 x:nearest y:nearest"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 100×100 native image drawn 1:1 (scale 1) into a pane at the origin with
    // 10×10-pixel cells — one cell == 10 game pixels.
    fn unit_map() -> V6ClickMap {
        V6ClickMap {
            pane_x: 0,
            pane_y: 0,
            cell_w: 10,
            cell_h: 10,
            img_x: 0.0,
            img_y: 0.0,
            img_w: 100.0,
            img_h: 100.0,
            native_w: 100,
            native_h: 100,
            text_rows: None,
        }
    }

    #[test]
    fn map_click_inverts_letterbox_at_cell_centre() {
        let m = unit_map();
        // Cell (0,0) centre is device px (5,5) -> game px floor(5/100*100)+1 = 6.
        assert_eq!(m.map_click(0, 0), Some((6, 6)));
        // Cell (3,4) centre is device px (35,45) -> game px (36, 46).
        assert_eq!(m.map_click(3, 4), Some((36, 46)));
        // Last in-image cell (9,9): centre (95,95) -> (96, 96), within native.
        assert_eq!(m.map_click(9, 9), Some((96, 96)));
    }

    #[test]
    fn map_click_rejects_clicks_outside_the_image() {
        let m = unit_map();
        // Cell 10 starts at device px 100 == native width -> outside (letterbox).
        assert_eq!(m.map_click(10, 0), None);
        assert_eq!(m.map_click(0, 10), None);
    }

    #[test]
    fn map_click_honours_pane_and_letterbox_offset() {
        // Pane origin at cell (4,2); image centred with an 8px/4px letterbox
        // offset inside a 5×5-pixel cell grid; native 40×40 scaled 2×.
        let m = V6ClickMap {
            pane_x: 4,
            pane_y: 2,
            cell_w: 5,
            cell_h: 5,
            img_x: 8.0,
            img_y: 4.0,
            img_w: 80.0, // 40 native * 2
            img_h: 80.0,
            text_rows: None,
            native_w: 40,
            native_h: 40,
        };
        // A click left of the pane origin is rejected outright.
        assert_eq!(m.map_click(3, 2), None);
        // Cell (6,4) rel to pane is (2,2): centre device px (12.5, 12.5);
        // (12.5-8)/80 * 40 = 2.25 -> floor+1 = 3 in x; (12.5-4)/80*40=4.25 -> 5 in y.
        assert_eq!(m.map_click(6, 4), Some((3, 5)));
        // A click in the top-left letterbox margin (before img_x/img_y) → None.
        assert_eq!(m.map_click(4, 2), None);
    }

    fn window(win: u32) -> GraphicsWindow {
        GraphicsWindow {
            win,
            canvas: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
            version: 1,
            upscale: false,
        }
    }

    fn populate(gr: &mut GraphicsRender, picker: &Picker, wins: &[u32]) {
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        for &win in wins {
            gr.render(picker, &window(win), area, Style::default(), &mut buf);
        }
    }

    #[test]
    fn retain_live_drops_closed_windows() {
        // halfblocks() needs no terminal query — deterministic in tests.
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        populate(&mut gr, &picker, &[1, 2]);
        assert_eq!(gr.cache.len(), 2);

        gr.retain_live(&std::collections::HashSet::from([1]));
        assert_eq!(gr.cache.len(), 1);
        assert!(gr.cache.contains_key(&1));
    }

    fn solid(win: u32, wpx: u32, hpx: u32, rgba: [u8; 4]) -> GraphicsWindow {
        GraphicsWindow {
            win,
            canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(wpx, hpx, image::Rgba(rgba))),
            version: 1,
            upscale: false,
        }
    }

    #[test]
    fn thin_divider_renders_as_line_glyph_in_its_colour() {
        // A 1×3-cell divider (solid red canvas) renders as a │ rule in red — thin,
        // not a full-cell block.
        let gw = solid(1, 9, 57, [156, 31, 0, 255]);
        let area = Rect::new(2, 0, 1, 3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin → cells");
        for cy in 0..3 {
            assert_eq!(buf.cell((2, cy)).unwrap().symbol(), "\u{2502}", "cell (2,{cy}) is a │ rule");
            assert_eq!(buf.cell((2, cy)).unwrap().style().fg, Some(Color::Rgb(156, 31, 0)), "rule colour on fg");
        }
    }

    #[test]
    fn thin_sparse_rule_renders_as_line_glyph() {
        // Kerkerkruip's real case: a 1px-tall rule at the TOP of a 1-cell-tall
        // (19px) window. A centre sample would miss it; the region scan catches the
        // opaque line. Because it's SPARSE (a pixel-thin rule), it renders as a thin
        // ─ glyph in the rule colour (fg), NOT a full-cell block — matching a pixel
        // interpreter's thin bar.
        let mut img = image::RgbaImage::new(90, 19); // 10 cells × 1 cell, transparent
        for x in 0..90 {
            img.put_pixel(x, 0, image::Rgba([200, 40, 60, 255])); // top row only
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin strip → cells");
        let cell = buf.cell((5, 0)).unwrap();
        assert_eq!(cell.symbol(), "\u{2500}", "sparse horizontal rule → ─ glyph");
        assert_eq!(cell.style().fg, Some(Color::Rgb(200, 40, 60)), "rule colour on the glyph fg");
    }

    #[test]
    fn thin_vertical_sparse_rule_renders_vertical_glyph() {
        // A 2px-wide vertical rule in a 1-cell-wide × 3-tall window → │ glyph.
        let mut img = image::RgbaImage::new(9, 57); // 1 cell × 3 cells, transparent
        for y in 0..57 {
            img.put_pixel(3, y, image::Rgba([255, 255, 255, 255]));
            img.put_pixel(4, y, image::Rgba([255, 255, 255, 255]));
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 1, 3);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false));
        assert_eq!(buf.cell((0, 1)).unwrap().symbol(), "\u{2502}", "sparse vertical rule → │ glyph");
    }

    #[test]
    fn kitty_graphics_place_with_explicit_grid_and_transmit_once() {
        // SQ-0520: on a kitty terminal a graphics window transmits its canvas as
        // a virtual placement with an EXPLICIT r×c grid (the terminal scales the
        // image to exactly the window's cells) and re-transmits only when the
        // canvas version or area changes — a replaced transmit deletes its
        // predecessor's image id.
        let mut img = image::RgbaImage::new(1104, 36);
        for (x, _y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, 40, 200, 255]);
        }
        let gw = GraphicsWindow { win: 7, canvas: std::sync::Arc::new(img), version: 3, upscale: false };
        // from_fontsize is deprecated in favor of a live stdio query, which a
        // headless test can't do — the fixed 8×18 mirrors the SQ-0520 report.
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        let area = Rect::new(0, 0, 138, 2);
        let mut gr = GraphicsRender::default();

        let mut buf = Buffer::empty(area);
        gr.render(&picker, &gw, area, Style::default(), &mut buf);
        let first = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(first.contains(",r=2,c=138,"), "transmit declares the explicit cell grid");
        assert!(first.contains("a=T,U=1,f=32"), "virtual placement transmit present");
        assert!(first.contains('\u{10EEEE}'), "placeholder run present");
        assert!(!first.contains("a=d"), "first transmit deletes nothing");
        assert!(buf.cell((0, 1)).unwrap().symbol().contains('\u{10EEEE}'), "second row placed");

        // Same version + area: no re-transmit, placeholders only.
        let mut buf2 = Buffer::empty(area);
        gr.render(&picker, &gw, area, Style::default(), &mut buf2);
        let second = buf2.cell((0, 0)).unwrap().symbol().to_string();
        assert!(!second.contains("a=T"), "unchanged canvas is not re-transmitted");
        assert!(second.contains('\u{10EEEE}'), "placeholders still placed every frame");

        // Version bump with UNCHANGED pixels (a game that repaints its whole
        // window every turn): nothing is re-uploaded and nothing is deleted —
        // the same id is simply re-placed. (SQ-0564)
        let gw2 = GraphicsWindow { win: 7, canvas: gw.canvas.clone(), version: 4, upscale: false };
        let mut buf3 = Buffer::empty(area);
        gr.render(&picker, &gw2, area, Style::default(), &mut buf3);
        let third = buf3.cell((0, 0)).unwrap().symbol().to_string();
        assert!(!third.contains("a=T"), "a repaint of identical pixels is not re-uploaded");
        assert!(!third.contains("a=d"), "and frees nothing — the image is still wanted");
        assert_eq!(gr.kitty_uploads(7), Some((1, id_of(&first))), "still one upload, still placed");
    }

    /// Recover the image id from a transmit escape (`i=<id>`), so a test can assert
    /// WHICH cached upload a frame placed.
    fn id_of(transmit: &str) -> u32 {
        let after = transmit.split("i=").nth(1).expect("transmit carries an id");
        after
            .split(|c: char| !c.is_ascii_digit())
            .find(|s| !s.is_empty())
            .expect("digits after i=")
            .parse()
            .expect("numeric id")
    }

    /// SQ-0564: the toolbar-press pattern. A game that alternates between two
    /// canvases — a resting bar and a pressed-button variant — uploads each ONCE
    /// and thereafter only re-places. Before this, every press and every release
    /// re-sent the whole canvas (155 KiB for advent.blb's toolbar, twice per
    /// click), which is the churn behind the toolbar's black frames.
    #[test]
    fn alternating_canvases_are_uploaded_once_each_then_re_placed() {
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        let area = Rect::new(0, 0, 138, 2);
        let mut gr = GraphicsRender::default();

        let resting = std::sync::Arc::new(image::RgbaImage::from_pixel(
            1104, 36, image::Rgba([220, 220, 220, 255]),
        ));
        // The same bar with one button drawn pressed: a few differing pixels.
        let pressed = std::sync::Arc::new({
            let mut img = (*resting).clone();
            for x in 14..26 {
                img.put_pixel(x, 2, image::Rgba([154, 154, 154, 255]));
            }
            img
        });

        // Each canvas the game paints bumps the version, exactly as advent's
        // toolbar does when it repaints itself from scratch.
        let frame = |gr: &mut GraphicsRender, canvas: &std::sync::Arc<image::RgbaImage>, version: u64| {
            let gw = GraphicsWindow { win: 2, canvas: canvas.clone(), version, upscale: false };
            let mut buf = Buffer::empty(area);
            gr.render(&picker, &gw, area, Style::default(), &mut buf);
            buf.cell((0, 0)).unwrap().symbol().to_string()
        };

        let f1 = frame(&mut gr, &resting, 1);
        assert!(f1.contains("a=T"), "the resting bar uploads once");
        let resting_id = id_of(&f1);

        let f2 = frame(&mut gr, &pressed, 2);
        assert!(f2.contains("a=T"), "the pressed variant is a new picture — upload it");
        let pressed_id = id_of(&f2);
        assert_ne!(pressed_id, resting_id, "distinct canvases get distinct ids");

        // Release: back to the resting bar. THIS is the frame that used to re-send
        // the entire canvas.
        let f3 = frame(&mut gr, &resting, 3);
        assert!(!f3.contains("a=T"), "releasing re-places the resting upload, sends no pixels");
        assert!(!f3.contains("a=d"), "and frees nothing — both canvases are still wanted");
        assert_eq!(gr.kitty_uploads(2), Some((2, resting_id)), "two uploads; resting one placed");

        // Press again: likewise free.
        let f4 = frame(&mut gr, &pressed, 4);
        assert!(!f4.contains("a=T"), "pressing again re-places the pressed upload");
        assert_eq!(gr.kitty_uploads(2), Some((2, pressed_id)), "still two uploads");
    }

    /// The cache is bounded: past [`KITTY_CACHE`] distinct canvases the
    /// least-recently-placed upload is freed, so a long animation can't grow the
    /// terminal's image memory without limit. The freed id is never the one on
    /// screen — that would blank the window mid-parse.
    #[test]
    fn kitty_cache_evicts_the_least_recently_placed_upload() {
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
        let area = Rect::new(0, 0, 10, 2);
        let mut gr = GraphicsRender::default();
        let mut ids = Vec::new();
        for i in 0..=KITTY_CACHE as u32 {
            // A distinct canvas per frame (differing in one pixel's red channel).
            let mut img = image::RgbaImage::from_pixel(80, 36, image::Rgba([10, 20, 30, 255]));
            img.put_pixel(0, 0, image::Rgba([i as u8, 0, 0, 255]));
            let gw = GraphicsWindow {
                win: 4,
                canvas: std::sync::Arc::new(img),
                version: i as u64 + 1,
                upscale: false,
            };
            let mut buf = Buffer::empty(area);
            gr.render(&picker, &gw, area, Style::default(), &mut buf);
            let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
            assert!(sym.contains("a=T"), "frame {i} is a new picture → uploaded");
            ids.push(id_of(&sym));
            if i < KITTY_CACHE as u32 {
                assert!(!sym.contains("a=d"), "frame {i} is within the cap → frees nothing");
            } else {
                assert!(sym.contains("a=d,d=I"), "past the cap → frees the oldest");
                assert!(
                    sym.contains(&format!("i={}", ids[0])),
                    "and it is the FIRST upload that is freed, not the visible one"
                );
            }
        }
        assert_eq!(gr.kitty_uploads(4).map(|(n, _)| n), Some(KITTY_CACHE), "cap holds");
    }

    #[test]
    fn thin_but_detailed_toolbar_is_not_a_rule() {
        // SQ-0520: advent.blb's clickable toolbar is a DETAILED graphics window
        // that lands at 2 cells tall on common pane widths (its ~36px request /
        // an 18px cell). The thin-strip shortcut must not claim it as a rule and
        // paint colour-averaged ─ glyphs ("two thin strips of pixels") — a thin
        // window whose cells disagree in colour is an image for the protocol.
        let mut img = image::RgbaImage::new(1104, 36); // 138×2 cells at 8×18
        for (x, _y, p) in img.enumerate_pixels_mut() {
            // Blocks of strongly different hues, like toolbar icons.
            *p = match (x / 48) % 4 {
                0 => image::Rgba([200, 60, 40, 255]),
                1 => image::Rgba([40, 160, 60, 255]),
                2 => image::Rgba([50, 80, 200, 255]),
                _ => image::Rgba([220, 210, 200, 255]),
            };
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 138, 2);
        let mut buf = Buffer::empty(area);
        assert!(
            !render_graphics_as_cells(&gw, area, &mut buf, false),
            "a thin-but-detailed canvas must fall through to the image protocol"
        );
        // force=true (the no-picker fallback) still paints the approximation.
        assert!(render_graphics_as_cells(&gw, area, &mut buf, true), "forced → approximated as cells");
    }

    #[test]
    fn thin_fully_transparent_paints_nothing() {
        // A thin window the game hasn't drawn (all transparent) leaves cells alone.
        let img = image::RgbaImage::new(90, 19);
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        buf.cell_mut((5, 0)).unwrap().set_style(Style::default().bg(Color::Rgb(1, 2, 3)));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "thin → handled");
        assert_eq!(buf.cell((5, 0)).unwrap().style().bg, Some(Color::Rgb(1, 2, 3)), "transparent → underlying kept");
    }

    #[test]
    fn large_uniform_graphics_paints_cells() {
        // A big but uniform canvas is still cheap-and-exact as cells.
        let gw = solid(1, 90, 190, [10, 20, 30, 255]);
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "uniform → cells");
        assert_eq!(buf.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn detailed_graphics_falls_back_to_protocol() {
        // A non-thin, non-uniform canvas (checker) must NOT be handled as cells.
        let mut img = image::RgbaImage::new(90, 190);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let on = ((x / 9) + (y / 19)) % 2 == 0;
            *p = if on { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([0, 0, 0, 255]) };
        }
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        assert!(!render_graphics_as_cells(&gw, area, &mut buf, false), "detailed image → protocol, not cells");
    }

    #[test]
    fn large_fully_transparent_is_handled_not_sent_to_protocol() {
        // narco opens big border frames around its story but never paints them.
        // A blank (all-transparent) window must be reported HANDLED (painting
        // nothing), NOT bounced to the image protocol — a transparent image gets
        // garbled into artifacts (stray chars/lines) over the neighbouring
        // windows in a real terminal. (SQ-0338)
        let img = image::RgbaImage::new(90, 190); // 10×10 cells, all transparent
        let gw = GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        buf.cell_mut((5, 5)).unwrap().set_style(Style::default().bg(Color::Rgb(1, 2, 3)));
        assert!(render_graphics_as_cells(&gw, area, &mut buf, false), "blank window → handled, not protocol");
        assert_eq!(buf.cell((5, 5)).unwrap().style().bg, Some(Color::Rgb(1, 2, 3)), "blank → underlying kept");
    }

    /// Drive the background v6 encode to completion (test helper, SQ-0469).
    fn drain_v6_job(gr: &mut GraphicsRender) {
        while gr.v6_encode_in_flight() {
            std::thread::sleep(std::time::Duration::from_millis(1));
            gr.poll_v6_job();
        }
    }

    #[test]
    fn v6_encode_gates_on_generation_off_thread() {
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        let area = Rect::new(0, 0, 4, 2);
        let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));

        // First frame for gen 7: nothing ready, no job → wants a build. With no
        // last-ready composite the encode is SYNCHRONOUS (SQ-0578: a transition
        // frame has no honest previous image to show), so it is ready this frame.
        assert!(gr.v6_wants_build(7, area), "cold start wants the first build");
        gr.spawn_v6_encode(&picker, canvas.clone(), 7, area);
        assert!(gr.v6_job.is_none(), "a cold-start encode runs synchronously, no worker");
        assert!(gr.v6.is_some(), "the cold-start encode installed immediately");
        assert_eq!(gr.v6.as_ref().unwrap().gen, 7);

        // With a composite ready, a NEW generation encodes off-thread. While the
        // encode is in flight, no second build is requested — coalesced, even
        // for a newer generation (newest wins: the in-flight one finishes, then
        // the next frame builds whatever the current generation is).
        assert!(gr.v6_wants_build(8, area), "a changed generation wants a fresh build");
        gr.spawn_v6_encode(&picker, canvas.clone(), 8, area);
        assert!(gr.v6_job.is_some(), "a warm re-encode runs on the worker");
        assert!(!gr.v6_wants_build(8, area), "an in-flight encode suppresses a rebuild");
        assert!(!gr.v6_wants_build(9, area), "an in-flight encode suppresses even a newer generation");
        drain_v6_job(&mut gr);
        assert!(gr.v6.is_some(), "the worker installed the encoded protocol");
        assert_eq!(gr.v6.as_ref().unwrap().gen, 8);

        // `invalidate_v6` (the hybrid band path ran): back to cold — the next
        // raster frame wants a build and will encode synchronously again.
        gr.invalidate_v6();
        assert!(gr.v6.is_none() && gr.v6_job.is_none(), "invalidation drops composite and job");
        assert!(gr.v6_wants_build(8, area), "an invalidated cache wants a rebuild");
        gr.spawn_v6_encode(&picker, canvas.clone(), 7, area);
        assert!(gr.v6_job.is_none() && gr.v6.is_some(), "post-invalidation encode is synchronous");

        // Same generation → no rebuild (the gate is the win: idle frames skip
        // build + encode entirely).
        assert!(!gr.v6_wants_build(7, area), "unchanged generation reuses the ready encode");
        // A new generation → wants a rebuild.
        assert!(gr.v6_wants_build(9, area), "a changed generation wants a fresh build");
        // A pane resize → wants a rebuild even at the same generation.
        assert!(gr.v6_wants_build(7, Rect::new(0, 0, 5, 2)), "a resize wants a fresh build");
    }

    #[test]
    fn v6_encode_caps_upscale_at_4x() {
        // A 32×32 native canvas in a huge pane must encode at 4× (128×128), not
        // the full device box. Halfblocks font is 10×20 px; a 200×100-cell pane
        // is 2000×2000 device, which would otherwise scale ~62×.
        let picker = Picker::halfblocks();
        let area = Rect::new(0, 0, 200, 100);
        let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));
        let ready = GraphicsRender::encode_v6(&picker, &canvas, 1, area).expect("encode");
        // The encoded protocol reports its device size; 4× of 32 = 128 px = at
        // most ceil(128/20)=7 cells tall / ceil(128/10)=13 wide. Assert it is far
        // smaller than the pane (the cap engaged), not the full 200×100.
        let sz = ready.proto.size();
        assert!(sz.width <= 14 && sz.height <= 8, "capped image is ~4× native, got {sz:?}");
    }

    #[test]
    fn draw_chrome_band_caches_and_retain_prunes() {
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        // Native 32×20 chrome (opaque), scaled 1:1 into a 32×20-device pane.
        let chrome = image::RgbaImage::from_pixel(32, 20, image::Rgba([10, 20, 30, 255]));
        let fs = picker.font_size();
        let pane = Rect::new(0, 0, 32 / fs.width.max(1), 20 / fs.height.max(1));
        let scale = uniform_scale((32, 20), (pane.width as u32 * fs.width as u32, pane.height as u32 * fs.height as u32));
        let band = Rect::new(pane.x, pane.y, pane.width, 1); // a top ring band
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.len(), 1, "first draw uploads + caches the band protocol");
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let hash0 = gr.chrome_bands.get(&key).unwrap().0;
        // Same content + band → cache hit, no rebuild.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.get(&key).unwrap().0, hash0, "identical band keeps the cached upload");

        // retain_chrome_bands drops any band not in the live set.
        gr.retain_chrome_bands(&std::collections::HashSet::new());
        assert!(gr.chrome_bands.is_empty(), "empty live set clears the band cache");
    }

    #[test]
    fn draw_chrome_band_isolates_bands_by_native_footprint() {
        // SQ-0514: a change confined to the TOP band's native rows must re-encode
        // only that band; a disjoint BOTTOM band stays fresh (its stored hash and
        // cached upload unchanged). Before the fix, the freshness hash covered the
        // WHOLE canvas, so any pixel change staled every band.
        use crate::render::v6_layout::uniform_scale;
        let picker = Picker::halfblocks();
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1) as u32, fs.height.max(1) as u32);
        let mut gr = GraphicsRender::default();
        // Native canvas exactly the pane's device size → scale 1:1 (s=1, off=0),
        // so a band's native footprint equals its device rows (no letterbox slop).
        let (cols, rows) = (2u16, 4u16);
        let (nw, nh) = (cols as u32 * cw, rows as u32 * ch);
        let mut chrome = image::RgbaImage::from_pixel(nw, nh, image::Rgba([10, 20, 30, 255]));
        let pane = Rect::new(0, 0, cols, rows);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        assert_eq!((scale.s, scale.off_x, scale.off_y), (1.0, 0, 0), "test canvas must map 1:1");
        let top = Rect::new(0, 0, cols, 1); // device rows [0, ch)
        let bottom = Rect::new(0, rows - 1, cols, 1); // device rows [nh-ch, nh)
        let mut buf = Buffer::empty(pane);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, top, &mut buf);
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, bottom, &mut buf);
        let before = gr.chrome_band_hashes();
        let top_key = (BandSlot::Art as u8, top.x, top.y, top.width, top.height);
        let bot_key = (BandSlot::Art as u8, bottom.x, bottom.y, bottom.width, bottom.height);
        assert!(before.contains_key(&top_key) && before.contains_key(&bot_key), "both bands cached");

        // Change a pixel that lives ONLY in the top band's native footprint.
        chrome.put_pixel(0, 1, image::Rgba([200, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, top, &mut buf);
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, bottom, &mut buf);
        let after = gr.chrome_band_hashes();

        assert_ne!(before[&top_key], after[&top_key], "the changed top band re-encodes (hash changed)");
        assert_eq!(before[&bot_key], after[&bot_key], "the disjoint bottom band stays fresh (hash unchanged)");
    }

    #[test]
    fn scaled_chrome_shares_one_resize_and_matches_direct_resize() {
        // SQ-0514: the shared scaled-canvas cache returns pixels byte-identical to
        // a direct whole-canvas Nearest resize (so band output is unchanged), and
        // it memoises — reused for the same content/scale, rebuilt on a change.
        let mut gr = GraphicsRender::default();
        // A detailed (non-uniform) canvas so the resize actually samples pixels.
        let mut canvas = image::RgbaImage::new(17, 11);
        for (x, y, p) in canvas.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 13) as u8, (y * 7) as u8, ((x + y) * 5) as u8, 255]);
        }
        let (s, sw, sh) = (2.5f32, 42u32, 27u32); // fractional scale → floor-map matters
        let expected = image::imageops::resize(&canvas, sw, sh, image::imageops::FilterType::Nearest);
        {
            let got = gr.scaled_chrome(&canvas, s, sw, sh);
            assert_eq!(got.as_raw(), expected.as_raw(), "shared scaled == direct whole-canvas resize");
        }
        let key0 = gr.chrome_scaled.as_ref().unwrap().0;
        // Same args → same cached key (memoised, no rebuild).
        let _ = gr.scaled_chrome(&canvas, s, sw, sh);
        assert_eq!(gr.chrome_scaled.as_ref().unwrap().0, key0, "identical content/scale reuses the cache");
        // A content change → rebuilt (key changes).
        canvas.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        let _ = gr.scaled_chrome(&canvas, s, sw, sh);
        assert_ne!(gr.chrome_scaled.as_ref().unwrap().0, key0, "a content change rebuilds the shared scaled canvas");
    }

    #[test]
    fn retain_live_empty_clears_all() {
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        populate(&mut gr, &picker, &[1, 2]);
        assert_eq!(gr.cache.len(), 2);

        gr.retain_live(&std::collections::HashSet::new());
        assert_eq!(gr.cache.len(), 0);
    }

    // ── Kitty upload lifetime (SQ-0637) ──────────────────────────────────────

    /// Render `win` at `area` through the kitty path with `n` DISTINCT canvases,
    /// returning the ids the terminal was told to keep.
    fn upload_generations(gr: &mut GraphicsRender, picker: &Picker, win: u32, area: Rect, n: u8) -> Vec<u32> {
        let mut ids = Vec::new();
        for i in 0..n {
            let mut img = image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 9, 9, 255]));
            img.put_pixel(0, 0, image::Rgba([i, 0, 0, 255]));
            let gw = GraphicsWindow { win, canvas: std::sync::Arc::new(img), version: i as u64 + 1, upscale: false };
            let mut buf = Buffer::empty(area);
            gr.render(picker, &gw, area, Style::default(), &mut buf);
            let sym = buf.cell((area.x, area.y)).unwrap().symbol().to_string();
            assert!(sym.contains("a=T"), "generation {i} is a new picture → uploaded");
            ids.push(id_of(&sym));
        }
        ids
    }

    #[test]
    fn closing_a_kitty_window_deletes_its_uploads() {
        // SQ-0637: `retain_live` used to forget a closed window's `KittyWindowImage`
        // silently. The terminal keeps every transmitted generation until told to
        // free it, so a Glulx game that closes its graphics window (or reopens one
        // under a new id) leaked all of them.
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 8, 2);
        let mut gr = GraphicsRender::default();
        let ids = upload_generations(&mut gr, &picker, 2, area, 3);

        gr.begin_band_log(); // frame boundary: only the close's ops below
        gr.retain_live(&std::collections::HashSet::new());
        let queued = gr.queued_deletes().to_string();
        for id in &ids {
            assert!(
                queued.contains(&format!("a=d,d=I,i={id}")),
                "every upload of the closed window is freed (missing i={id}): {queued:?}"
            );
        }
        assert_eq!(
            gr.ops().iter().filter(|o| matches!(o, GraphicsOp::Drop { .. })).count(),
            ids.len(),
            "one Drop op per freed upload"
        );

        // The escapes reach the terminal through the buffer, like every other bit of
        // our kitty traffic — and the cell they ride on keeps its glyph and width.
        let mut buf = Buffer::empty(area);
        buf.cell_mut((0, 0)).unwrap().set_symbol("X");
        gr.flush_kitty_deletes(area, &mut buf);
        let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
        assert!(sym.starts_with('\x1b') && sym.ends_with('X'), "deletes prepended, glyph kept: {sym:?}");
        assert_eq!(
            buf.cell((0, 0)).unwrap().diff_option,
            ratatui::buffer::CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()),
            "the escape must not be measured as visible width"
        );
        assert!(gr.queued_deletes().is_empty(), "flushed once, not re-emitted every frame");
    }

    #[test]
    fn resizing_a_kitty_window_deletes_the_uploads_it_abandons() {
        // A size change restarts the window's cache (the r×c grid is baked into the
        // transmission), so the old generations can never be re-placed — they must be
        // freed, in the SAME batch as the new upload (SQ-0637).
        let picker = kitty_picker(8, 18);
        let mut gr = GraphicsRender::default();
        let old_ids = upload_generations(&mut gr, &picker, 2, Rect::new(0, 0, 8, 2), 2);

        let bigger = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(bigger);
        let gw = GraphicsWindow {
            win: 2,
            canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 9, 9, 255]))),
            version: 9,
            upscale: false,
        };
        gr.render(&picker, &gw, bigger, Style::default(), &mut buf);
        let sym = buf.cell((0, 0)).unwrap().symbol().to_string();
        for id in &old_ids {
            assert!(sym.contains(&format!("a=d,d=I,i={id}")), "pre-resize upload i={id} freed: {sym:?}");
        }
        assert!(sym.contains("a=T"), "and the new size is transmitted in the same batch");
        let delete_at = sym.find("a=d").expect("a delete");
        let transmit_at = sym.find("a=T").expect("a transmit");
        assert!(delete_at < transmit_at, "frees precede the transmit they make room for");
        assert!(gr.queued_deletes().is_empty(), "the placement carried them; nothing left queued");
    }

    #[test]
    fn queued_deletes_wait_for_a_safe_cell_and_are_never_dropped() {
        // The flush never disturbs a cell that belongs to a live placement (an
        // escape-carrying placeholder, or one marked Skip): the deletes stay queued
        // for a later frame instead (SQ-0637).
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 2, 1);
        let mut gr = GraphicsRender::default();
        upload_generations(&mut gr, &picker, 5, area, 1);
        gr.retain_live(&std::collections::HashSet::new());
        assert!(!gr.queued_deletes().is_empty(), "the close queued a delete");

        let mut buf = Buffer::empty(area);
        buf.cell_mut((0, 0)).unwrap().set_symbol("\x1b_Gplaceholder\x1b\\");
        buf.cell_mut((1, 0)).unwrap().set_diff_option(ratatui::buffer::CellDiffOption::Skip);
        let queued = gr.queued_deletes().to_string();
        gr.flush_kitty_deletes(area, &mut buf);
        assert_eq!(gr.queued_deletes(), queued, "no safe cell this frame → deferred, not lost");
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "\x1b_Gplaceholder\x1b\\", "placement untouched");

        // A later frame with an ordinary cell carries them.
        let mut buf2 = Buffer::empty(area);
        gr.flush_kitty_deletes(area, &mut buf2);
        assert!(gr.queued_deletes().is_empty(), "flushed on the next frame that has room");
    }

    /// The half of SQ-0772 that is not ours to encode: everything drawn through a
    /// `ratatui-image` [`Protocol`] — the v6 raster composite, every chrome band,
    /// inline art, the picker's covers — is placed by the crate's own kitty backend,
    /// one anchored cell per row followed by bare `Skip` continuations. [`place_protocol`]
    /// re-lays that row into the same self-describing, buffer-visible cells our own
    /// emitter writes, and this pins that it actually happens on a REAL protocol
    /// rather than on a hand-written imitation of one.
    #[test]
    fn a_ratatui_image_placement_is_reseated_into_self_describing_cells() {
        let picker = kitty_picker(8, 18);
        let (cols, rows) = (7u16, 3u16);
        let mut img = image::RgbaImage::new(u32::from(cols) * 8, u32::from(rows) * 18);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255]);
        }
        let proto = picker
            .new_protocol(image::DynamicImage::ImageRgba8(img), Size::new(cols, rows), Resize::Fit(None))
            .expect("the kitty backend encodes a plain RGBA image");

        let dest = Rect::new(2, 1, cols, rows);
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 5));
        place_protocol(&proto, dest, &mut buf);

        let mut row_diacritics = Vec::new();
        let mut fg = None;
        for y in 0..rows {
            for x in 0..cols {
                let cell = buf.cell((dest.x + x, dest.y + y)).expect("inside the buffer");
                assert_eq!(
                    cell.diff_option, PLACEHOLDER_WIDTH,
                    "cell ({x},{y}) must be a width-1 placeholder, not a Skip the diff cannot see",
                );
                let sym = cell.symbol();
                let at = sym.find('\u{10EEEE}').unwrap_or_else(|| panic!("cell ({x},{y}): {sym:?}"));
                let mut d = sym[at..].chars().skip(1);
                let (row_d, col_d, _extra) = (d.next(), d.next(), d.next());
                assert_eq!(
                    col_d,
                    Some(KITTY_DIACRITICS[usize::from(x)]),
                    "cell ({x},{y}) must name its OWN column, not lean on its neighbour",
                );
                if x == 0 {
                    row_diacritics.push(row_d);
                }
                let seen = *fg.get_or_insert(cell.fg);
                assert_eq!(cell.fg, seen, "every cell of one image carries the same id colour");
            }
        }
        row_diacritics.dedup();
        assert_eq!(row_diacritics.len(), usize::from(rows), "each row names a different image row");

        // And the whole 32-bit id is readable back off the cells: the low 24 bits
        // in the foreground, the high byte as the third diacritic's index. That is
        // the thing SQ-0753 recorded as impossible ("the image id lives inside
        // ratatui-image's `Kitty` struct with no accessor, so we cannot name what to
        // delete") — worth pinning here, since a delete built on it would be silently
        // aiming at the wrong image if this ever stopped holding.
        let lead = buf.cell((dest.x, dest.y)).unwrap();
        let sym = lead.symbol().to_string();
        let Some(Color::Rgb(r, g, b)) = fg else { panic!("the id colour is truecolour") };
        let extra_d = sym[sym.find('\u{10EEEE}').unwrap()..].chars().nth(3).expect("the third diacritic");
        let high = KITTY_DIACRITICS.iter().position(|&c| c == extra_d).expect("a table entry");
        let recovered = ((high as u32) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        assert_eq!(recovered, id_of(&sym), "the id the upload declared, read back off the screen");
    }

    /// A kitty chrome ring at a fixed 1:1 scale: a native canvas exactly the pane's
    /// device size, so a band's crop is its own device rows and a pixel edit inside
    /// one band re-encodes only that band.
    fn band_fixture() -> (Picker, image::RgbaImage, crate::render::v6_layout::Scale, Rect, Rect, Buffer) {
        use crate::render::v6_layout::uniform_scale;
        let picker = kitty_picker(8, 18);
        let (cols, rows) = (4u16, 3u16);
        let (nw, nh) = (u32::from(cols) * 8, u32::from(rows) * 18);
        let mut chrome = image::RgbaImage::new(nw, nh);
        for (x, y, p) in chrome.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 256) as u8, (y % 256) as u8, 40, 255]);
        }
        let pane = Rect::new(0, 0, cols, rows);
        let scale = uniform_scale((nw as u16, nh as u16), (nw, nh));
        let band = Rect::new(0, 0, cols, 1);
        let buf = Buffer::empty(pane);
        (picker, chrome, scale, pane, band, buf)
    }

    /// Every `a=d,d=I,i=` id in some emitted text, in order.
    fn delete_ids(text: &str) -> Vec<u32> {
        text.split("\x1b_Gq=2,a=d,d=I,i=")
            .skip(1)
            .filter_map(|s| s.split('\x1b').next()?.parse().ok())
            .collect()
    }

    /// Every `a=d,d=I,i=` id currently queued, in order.
    fn queued_delete_ids(gr: &GraphicsRender) -> Vec<u32> {
        delete_ids(gr.queued_deletes())
    }

    /// Every id this frame actually frees: the deletes riding out on a placement in
    /// `buf`, then anything still queued for a later frame. A delete lives in one
    /// place or the other and never in neither — which is the property worth
    /// asserting, since "queued" and "written" are both correct outcomes.
    fn freed_ids(gr: &GraphicsRender, buf: &Buffer) -> Vec<u32> {
        let cells: String = buf.content().iter().map(|c| c.symbol()).collect();
        let mut ids = delete_ids(&cells);
        ids.extend(queued_delete_ids(gr));
        ids
    }

    /// SQ-0753: a chrome band that goes away must be FREED in the terminal, not
    /// merely forgotten here.
    ///
    /// `retain_chrome_bands`/`invalidate_chrome_bands` recorded a `Drop` and dropped
    /// the `Protocol`, which releases nothing — `ratatui-image` has no output channel
    /// and never deletes. The terminal kept every band babelmap ever encoded, and
    /// kitty evicts by LRU including images that are CURRENTLY PLACED, so a big
    /// enough pile can blank a live one. Naming the abandoned upload was the blocker
    /// the quest recorded ("the image id lives inside ratatui-image's `Kitty` struct
    /// with no accessor"); [`place_protocol`] now reads it back off the placement.
    #[test]
    fn abandoning_a_chrome_band_deletes_its_upload() {
        let (picker, chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);

        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let id = gr.chrome_band_id(key).expect("a placed kitty band knows the id it lives under");
        assert!(gr.queued_deletes().is_empty(), "a band still on screen is not freed");

        gr.retain_chrome_bands(&std::collections::HashSet::new());
        assert!(gr.chrome_bands.is_empty(), "the band left the live set");
        assert_eq!(queued_delete_ids(&gr), vec![id], "the abandoned upload is freed by id");

        // …and the wholesale invalidation (a terminal clear, SQ-0587) does the same.
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let id = gr.chrome_band_id(key).expect("placed");
        gr.invalidate_chrome_bands();
        assert_eq!(queued_delete_ids(&gr), vec![id], "invalidate frees what it drops");
    }

    /// SQ-0753, the leak that runs on EVERY frame rather than at a transition: a
    /// band whose pixels changed re-encodes into the same cache slot, and the
    /// `insert` used to be the last anyone ever heard of its predecessor's id.
    /// Journey's picture column re-encodes on each menu step, so this is where the
    /// megabytes went.
    #[test]
    fn re_encoding_a_chrome_band_deletes_the_upload_it_replaces() {
        let (picker, mut chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let first = gr.chrome_band_id(key).expect("placed");

        // A cache HIT sends nothing and frees nothing — the upload is still live.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert!(freed_ids(&gr, &buf).is_empty(), "a cache hit must not free the image it is re-placing");
        assert_eq!(gr.chrome_band_id(key), Some(first), "and it stays the same image");

        // Change a pixel inside this band's native footprint → re-encode.
        chrome.put_pixel(1, 2, image::Rgba([255, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let second = gr.chrome_band_id(key).expect("placed again");
        assert_ne!(second, first, "a re-encode is a new upload with a new id");
        // The delete rides out on the replacement's own placement, in one batch.
        assert_eq!(freed_ids(&gr, &buf), vec![first], "the replaced upload is freed, not orphaned");
        assert!(gr.queued_deletes().is_empty(), "and it went with the frame rather than waiting");
    }

    /// SQ-0817: …and it rides BEHIND that placement, never ahead of it.
    ///
    /// The id being freed here is the one the terminal is drawing at this very rect,
    /// and its replacement is up to 618 KB of image data away (Zork Zero's banner
    /// band, measured). Freed first, those cells have nothing to draw for the length
    /// of that transfer — which is the flicker the compass, the on-screen map and
    /// Arthur's Merlin composite all showed, once per frame the game changed
    /// anything. SQ-0753 introduced it by putting every delete in the placement's
    /// PREFIX; before that nothing was ever freed, so nothing could blink.
    ///
    /// A frame's cells are emitted in row-major order, so their concatenated symbols
    /// are the frame's byte order: the assertion is simply that this delete is the
    /// last escape in it.
    #[test]
    fn the_upload_being_replaced_is_freed_only_after_its_replacement_is_placed() {
        let (picker, mut chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);

        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let first = gr.chrome_band_id(key).expect("placed");

        // The replacement frame gets a clean buffer, so what it carries is its own.
        let mut buf = Buffer::empty(pane);
        chrome.put_pixel(1, 2, image::Rgba([255, 0, 0, 255]));
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let second = gr.chrome_band_id(key).expect("placed again");
        assert_ne!(second, first, "a re-encode is a new upload with a new id");

        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        let del = format!("\x1b_Gq=2,a=d,d=I,i={first}\x1b\\");
        let at = text.find(&del).expect("the replaced upload is freed in this frame");
        assert!(
            text[..at].contains(&format!("i={second}")),
            "the replacement was still un-transmitted when its predecessor was freed — the \
             rect draws nothing until the upload lands, which is the flicker"
        );
        assert_eq!(
            at,
            text.rfind("\x1b_G").expect("the frame carries APC traffic"),
            "the supersede delete must be the LAST escape of the frame: anything emitted after \
             it is traffic the freed image was still covering for"
        );
    }

    /// SQ-0753 for the biggest upload babelmap makes: the v6 raster composite is the
    /// whole pane in one image (2.8 MB on Journey at 117x64). It is abandoned twice
    /// over — wholesale when the hybrid ring takes the screen (`invalidate_v6`), and
    /// once per visible change in a raster-mode game, where `poll_v6_job` installs
    /// the new encode over the old.
    #[test]
    fn abandoning_the_v6_raster_composite_deletes_its_upload() {
        let picker = kitty_picker(8, 18);
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        let mut gr = GraphicsRender::default();
        let canvas = image::RgbaImage::from_pixel(32, 54, image::Rgba([9, 8, 7, 255]));

        gr.spawn_v6_encode(&picker, canvas.clone(), 1, area);
        gr.redraw_v6(&picker, area, &mut buf);
        let first = gr.v6.as_ref().and_then(|r| r.placed_id).expect("the composite knows its id");

        // A second generation replaces it on the worker thread. The composite it
        // replaces is the whole pane the terminal is showing, so its delete waits
        // BEHIND the placement that covers the pane again rather than riding ahead of
        // a pane-sized upload (SQ-0817).
        gr.spawn_v6_encode(&picker, canvas.clone(), 2, area);
        drain_v6_job(&mut gr);
        assert!(
            queued_delete_ids(&gr).is_empty(),
            "the live composite must not be freed ahead of its own replacement"
        );
        assert_eq!(
            delete_ids(&gr.deletes_after_place),
            vec![first],
            "…it is queued to ride out behind that replacement"
        );
        let mut frame = Buffer::empty(area);
        gr.redraw_v6(&picker, area, &mut frame);
        let second = gr.v6.as_ref().and_then(|r| r.placed_id).expect("placed");
        assert_ne!(second, first, "the new generation is a different upload");

        let text: String = frame.content().iter().map(|c| c.symbol()).collect();
        let at = text
            .find(&format!("\x1b_Gq=2,a=d,d=I,i={first}\x1b\\"))
            .expect("the superseded composite is freed on the frame that replaces it");
        assert_eq!(
            at,
            text.rfind("\x1b_G").expect("the frame carries APC traffic"),
            "the composite is freed only after every byte of its replacement"
        );

        // …and the raster→ring transition frees whatever was up. Nothing places
        // after it — that is what the transition IS — so it waits for the frame's
        // closing flush.
        gr.invalidate_v6();
        assert_eq!(queued_delete_ids(&gr), vec![second], "invalidation frees the live composite too");
        assert_eq!(freed_ids(&gr, &frame), vec![first, second], "both generations end up freed");
    }

    /// The deletes above are worthless unless they reach the terminal, and the v6
    /// pixel paths have no placement of their own to piggyback on — the frame's
    /// closing flush is what carries them (SQ-0753). Pinned here as the property
    /// `main`'s end-of-frame flush relies on: a queued delete lands in the buffer
    /// as a real, diffable change.
    #[test]
    fn a_queued_protocol_delete_rides_out_on_the_frame() {
        let (picker, chrome, scale, pane, band, mut buf) = band_fixture();
        let mut gr = GraphicsRender::default();
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        let key = (BandSlot::Art as u8, band.x, band.y, band.width, band.height);
        let id = gr.chrome_band_id(key).expect("placed");
        gr.retain_chrome_bands(&std::collections::HashSet::new());

        // The next frame draws no band at all — ordinary cells everywhere.
        let mut next = Buffer::empty(pane);
        gr.flush_kitty_deletes(pane, &mut next);
        assert!(gr.queued_deletes().is_empty(), "the frame carried them");
        let carried: String = next.content().iter().map(|c| c.symbol()).collect();
        assert!(
            carried.contains(&format!("a=d,d=I,i={id}")),
            "the delete must be IN the frame's cells, or it is never written: {carried:?}"
        );
    }
}
