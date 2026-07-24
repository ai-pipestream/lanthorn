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
    let thin = area.width.min(area.height) <= 2;
    let first = cell_color(0, 0);
    let uniform = first.is_some()
        && (0..area.height).all(|cy| (0..area.width).all(|cx| cell_color(cx, cy).is_some_and(|c| close(c, first.unwrap()))));
    if !(force || thin || uniform) {
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
        let fy = (dy - self.img_y) / self.img_h;
        if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
            return None;
        }
        let gx = (fx * self.native_w as f32).floor() as u16 + 1;
        let gy = (fy * self.native_h as f32).floor() as u16 + 1;
        Some((gx.min(self.native_w), gy.min(self.native_h)))
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
}

/// The worker-thread handle for an in-flight v6 raster encode (SQ-0469). The
/// heavy resize + PNG/protocol encode (tens–hundreds of ms) runs off the UI
/// thread and yields a ready-to-render [`V6Ready`]. Coalesced: only one runs at a
/// time, and `poll_v6_job` installs its result (see `spawn_v6_encode`).
type V6Job = std::thread::JoinHandle<Option<V6Ready>>;

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
    /// content+scale hash so an unchanged frame reuses the upload. Pruned each
    /// frame to the live band set by [`GraphicsRender::retain_chrome_bands`].
    chrome_bands: std::collections::HashMap<(u16, u16, u16, u16), (u64, Protocol)>,
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
            Image::new(proto).render(dest, buf);
        }
    }

    /// Drop cache entries for windows no longer live (evicts on close; bounds growth).
    pub fn retain_live(&mut self, live: &std::collections::HashSet<u32>) {
        self.cache.retain(|win, _| live.contains(win));
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
        let (cw, ch) = (canvas.width(), canvas.height());
        let scale =
            ((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64)).clamp(1.0, MAX_V6_UPSCALE);
        let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
        let scaled = image::imageops::resize(canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
        let img = image::DynamicImage::ImageRgba8(scaled);
        match picker.new_protocol(img, Size::new(area.width, area.height), Resize::Fit(None)) {
            Ok(proto) => Some(V6Ready {
                gen,
                area_w: area.width,
                area_h: area.height,
                proto,
                native_w: canvas.width() as u16,
                native_h: canvas.height() as u16,
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
    /// worker's result is installed by [`poll_v6_job`].
    pub fn spawn_v6_encode(&mut self, picker: &Picker, canvas: image::RgbaImage, gen: u64, area: Rect) {
        if area.width == 0 || area.height == 0 || canvas.width() == 0 || canvas.height() == 0 {
            return;
        }
        let picker = picker.clone();
        self.v6_job = Some(std::thread::spawn(move || Self::encode_v6(&picker, &canvas, gen, area)));
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
            self.v6 = Some(ready);
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
        Image::new(proto).render(dest, buf);
        // Record the letterbox geometry so a click in the pane can be mapped
        // back to a game pixel (Lane M). The image fills `dest`'s cells
        // aspect-preserved, so the click map's image rect is `dest` in device
        // pixels and its native extent is the canvas dimensions.
        let fs = picker.font_size();
        let (cw, ch) = (fs.width.max(1), fs.height.max(1));
        self.last_v6_map = Some(V6ClickMap {
            pane_x: area.x,
            pane_y: area.y,
            cell_w: cw,
            cell_h: ch,
            img_x: (dest.x - area.x) as f32 * cw as f32,
            img_y: (dest.y - area.y) as f32 * ch as f32,
            img_w: dest.width as f32 * cw as f32,
            img_h: dest.height as f32 * ch as f32,
            native_w: ready.native_w,
            native_h: ready.native_h,
        });
    }

    /// Record the letterbox click map for the HYBRID draw path (Lane H), where the
    /// game image is drawn as a chrome ring around a terminal viewport rather than
    /// one canvas. `scale` is the same [`uniform_scale`](crate::render::v6_layout::uniform_scale)
    /// the chrome bands were placed through, so the recovered game pixel matches
    /// what the player sees. `pane` is the whole v6 pane's cell rect; `native` is
    /// the chrome canvas's game-pixel extent; `cell_px` is the font cell size.
    pub fn record_hybrid_click_map(
        &mut self,
        pane: Rect,
        scale: &crate::render::v6_layout::Scale,
        native: (u16, u16),
        cell_px: (u16, u16),
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
        });
    }

    /// Drop cached chrome-band protocols whose band rect is not in `live` — called
    /// once per hybrid frame so a resize/layout change can't leave stale band
    /// uploads accumulating.
    pub fn retain_chrome_bands(&mut self, live: &std::collections::HashSet<(u16, u16, u16, u16)>) {
        self.chrome_bands.retain(|k, _| live.contains(k));
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

        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        chrome_canvas.as_raw().hash(&mut h);
        scale.s.to_bits().hash(&mut h);
        (scale.off_x, scale.off_y).hash(&mut h);
        (cw, ch).hash(&mut h);
        (rel_x0, rel_y0, bw, bh).hash(&mut h);
        let hash = h.finish();
        let key = (band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _)) if *v == hash);
        if !fresh {
            // Scale the whole native chrome once (Nearest → crisp), then copy the
            // sub-rect under this band into a band-sized image (letterbox area
            // outside the scaled chrome stays transparent).
            let scaled = image::imageops::resize(chrome_canvas, sw, sh, image::imageops::FilterType::Nearest);
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
            let img = image::DynamicImage::ImageRgba8(band_img);
            match picker.new_protocol(img, Size::new(band.width, band.height), Resize::Fit(None)) {
                Ok(p) => { self.chrome_bands.insert(key, (hash, p)); }
                Err(_) => return,
            }
        }
        if let Some((_, proto)) = self.chrome_bands.get(&key) {
            let sz = proto.size();
            let w = sz.width.min(band.width);
            let ht = sz.height.min(band.height);
            // The band image is exactly band-sized, so it places at the band's
            // top-left (no centering — the crop is already positioned).
            let dest = Rect::new(band.x, band.y, w, ht);
            Image::new(proto).render(dest, buf);
        }
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

        // First frame for gen 7: nothing ready, no job → wants a build.
        assert!(gr.v6_wants_build(7, area), "cold start wants the first build");
        gr.spawn_v6_encode(&picker, canvas.clone(), 7, area);
        // While the encode is in flight, no second build is requested — coalesced,
        // even for a NEWER generation (newest wins: the in-flight one finishes,
        // then the next frame builds whatever the current generation is).
        assert!(!gr.v6_wants_build(7, area), "an in-flight encode suppresses a rebuild");
        assert!(!gr.v6_wants_build(8, area), "an in-flight encode suppresses even a newer generation");
        drain_v6_job(&mut gr);
        assert!(gr.v6.is_some(), "the worker installed the encoded protocol");
        assert_eq!(gr.v6.as_ref().unwrap().gen, 7);

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
        let key = (band.x, band.y, band.width, band.height);
        let hash0 = gr.chrome_bands.get(&key).unwrap().0;
        // Same content + band → cache hit, no rebuild.
        gr.draw_chrome_band(&picker, &chrome, &scale, pane, band, &mut buf);
        assert_eq!(gr.chrome_bands.get(&key).unwrap().0, hash0, "identical band keeps the cached upload");

        // retain_chrome_bands drops any band not in the live set.
        gr.retain_chrome_bands(&std::collections::HashSet::new());
        assert!(gr.chrome_bands.is_empty(), "empty live set clears the band cache");
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
}
