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
    /// hash of ONLY that band's own native sub-rect (plus scale/offset/cell/rect)
    /// so a change confined to one band leaves the other bands' uploads fresh
    /// (SQ-0514). Pruned each frame to the live band set by
    /// [`GraphicsRender::retain_chrome_bands`].
    chrome_bands: std::collections::HashMap<(u16, u16, u16, u16), (u64, Protocol)>,
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
}

/// One transmitted kitty image backing a graphics window (SQ-0520).
struct KittyWindowImage {
    version: u64,
    w: u16,
    h: u16,
    id: u32,
    /// The transmit escape (plus a delete of a retired predecessor), prepended
    /// to the first placeholder row of the next emitted frame, then dropped —
    /// the terminal keeps the image by id after that.
    pending_transmit: Option<String>,
    /// The id this one displaced ON SCREEN, freed with the NEXT replacement —
    /// never together with our own transmit. Deleting it immediately blanked
    /// the window while the (large) replacement was still being parsed, which
    /// read as background flashing between the frames of an animated sequence
    /// (Kerkerkruip's opening). Deferring one generation keeps the visible
    /// frame alive until the new placeholders have landed.
    retired: Option<u32>,
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
            Image::new(proto).render(dest, buf);
        }
    }

    /// Drop cache entries for windows no longer live (evicts on close; bounds growth).
    pub fn retain_live(&mut self, live: &std::collections::HashSet<u32>) {
        self.cache.retain(|win, _| live.contains(win));
        self.kitty_wins.retain(|win, _| live.contains(win));
    }

    /// Render a graphics window's canvas through kitty unicode placeholders
    /// with an EXPLICIT `r×c` grid on the virtual placement (SQ-0520): the
    /// terminal scales the image to exactly the placeholder rect, so the
    /// visible image always fills the window's cells — independent of any
    /// logical-vs-device pixel mismatch — and the cell→game-pixel mouse
    /// mapping in `glk_mouse_target` stays truthful.
    ///
    /// The image is transmitted once per (canvas version, area size); a
    /// replaced transmission deletes its predecessor's id so a game that
    /// repaints every turn (advent.blb's toolbar) doesn't accumulate dead
    /// images in the terminal. Placeholder cells are re-written every frame
    /// (identical content, so the terminal diff drops them when unchanged).
    fn render_kitty_virtual(&mut self, gw: &GraphicsWindow, area: Rect, buf: &mut Buffer) {
        use std::fmt::Write as _;
        let fresh = matches!(self.kitty_wins.get(&gw.win),
            Some(e) if e.version == gw.version && e.w == area.width && e.h == area.height);
        if !fresh {
            let old = self.kitty_wins.get(&gw.win);
            // Free only the id retired TWO generations back — the one the
            // still-visible image displaced. Deleting the visible id here would
            // blank the window until the new transmit finishes parsing (the
            // between-frames background flash). d=I frees data + placements.
            let free_now = old.and_then(|e| e.retired);
            let retired = old.map(|e| e.id);
            // Private id range, disjoint from ratatui-image's random ids in
            // practice; top byte stays 0 so the id_extra diacritic is 0.
            self.next_kitty_id = self.next_kitty_id.wrapping_add(1) & 0x000F_FFFF;
            let id = 0x00B0_0000 | self.next_kitty_id;
            let mut transmit = String::new();
            if let Some(g) = free_now {
                write!(transmit, "\x1b_Gq=2,a=d,d=I,i={g}\x1b\\").unwrap();
            }
            transmit.push_str(&kitty_transmit_virtual(&gw.canvas, id, area.height, area.width));
            self.kitty_wins.insert(
                gw.win,
                KittyWindowImage {
                    version: gw.version,
                    w: area.width,
                    h: area.height,
                    id,
                    pending_transmit: Some(transmit),
                    retired,
                },
            );
        }
        let entry = self.kitty_wins.get_mut(&gw.win).expect("inserted above");
        let transmit = entry.pending_transmit.take();
        kitty_place_rows(entry.id, transmit.as_deref(), area, buf);
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
        });
    }

    /// Drop cached chrome-band protocols whose band rect is not in `live` — called
    /// once per hybrid frame so a resize/layout change can't leave stale band
    /// uploads accumulating.
    pub fn retain_chrome_bands(&mut self, live: &std::collections::HashSet<(u16, u16, u16, u16)>) {
        self.chrome_bands.retain(|k, _| live.contains(k));
    }

    /// Snapshot the current chrome-band freshness hashes, keyed by band cell rect
    /// (observability hook, SQ-0514). A band whose hash is unchanged across two
    /// frames did NOT re-encode — used to confirm that a change confined to one
    /// band leaves the other bands' uploads fresh.
    pub fn chrome_band_hashes(&self) -> std::collections::HashMap<(u16, u16, u16, u16), u64> {
        self.chrome_bands.iter().map(|(k, (h, _))| (*k, *h)).collect()
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
            let scaled = image::imageops::resize(chrome_canvas, sw, sh, image::imageops::FilterType::Nearest);
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
        if sx_lo < sx_hi && sy_lo < sy_hi {
            let nx0 = scaled_to_native(sx_lo as u32, nw, sw);
            let nx1 = scaled_to_native(sx_hi as u32 - 1, nw, sw) + 1;
            let ny0 = scaled_to_native(sy_lo as u32, nh, sh);
            let ny1 = scaled_to_native(sy_hi as u32 - 1, nh, sh) + 1;
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
        let key = (band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _)) if *v == hash);
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
        1u8.hash(&mut h); // discriminator vs. draw_chrome_band keys on the same map
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
        let key = (band.x, band.y, band.width, band.height);
        let fresh = matches!(self.chrome_bands.get(&key), Some((v, _)) if *v == hash);
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
            let stretched = image::imageops::resize(&src, bw, bh, image::imageops::FilterType::Nearest);
            let img = image::DynamicImage::ImageRgba8(stretched);
            match picker.new_protocol(img, Size::new(band.width, band.height), Resize::Fit(None)) {
                Ok(p) => { self.chrome_bands.insert(key, (hash, p)); }
                Err(_) => return,
            }
        }
        if let Some((_, proto)) = self.chrome_bands.get(&key) {
            let sz = proto.size();
            let w = sz.width.min(band.width);
            let ht = sz.height.min(band.height);
            let dest = Rect::new(band.x, band.y, w, ht);
            Image::new(proto).render(dest, buf);
        }
    }
}

// ── Kitty virtual-placement emission (SQ-0520) ────────────────────────────────

/// The kitty row/column placeholder diacritics (first 140 of kitty's 297-entry
/// rowcolumn-diacritics.txt). Only explicit indices need the table: rows use
/// one entry each (emission caps at 140 rows — far above any real graphics
/// window) and columns after the first are bare placeholders whose position the
/// terminal infers.
const KITTY_DIACRITICS: [char; 140] = [
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
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}',
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

/// Write the placeholder rows for image `id` into `buf` over `area`, in the
/// same cell shape ratatui-image uses: the whole row's escape string lives in
/// the row's first cell (forced width 1) and the remaining cells are marked
/// Skip so the diff never overwrites the placeholders. `transmit` (first frame
/// after an encode) is prepended to the first row.
fn kitty_place_rows(id: u32, transmit: Option<&str>, area: Rect, buf: &mut Buffer) {
    use ratatui::buffer::CellDiffOption;
    use std::fmt::Write as _;
    let [_, id_r, id_g, id_b] = id.to_be_bytes();
    // Restore the cursor saved at the row start, then park it at the area's
    // far corner so the terminal's cursor stays inside the drawn region.
    let (right, down) = (area.width - 1, area.height - 1);
    let rows = area.height.min(KITTY_DIACRITICS.len() as u16);
    let mut transmit = transmit;
    for y in 0..rows {
        let mut symbol = String::new();
        if let Some(seq) = transmit.take() {
            symbol.push_str(seq);
        }
        write!(
            symbol,
            "\x1b[s\x1b[38;2;{id_r};{id_g};{id_b}m\u{10EEEE}{}{}{}",
            KITTY_DIACRITICS[y as usize], KITTY_DIACRITICS[0], KITTY_DIACRITICS[0]
        )
        .unwrap();
        for _ in 1..area.width {
            symbol.push('\u{10EEEE}');
        }
        write!(symbol, "\x1b[u\x1b[{right}C\x1b[{down}B").unwrap();
        for x in 1..area.width {
            if let Some(cell) = buf.cell_mut((area.left() + x, area.top() + y)) {
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
        if let Some(cell) = buf.cell_mut((area.left(), area.top() + y)) {
            cell.set_symbol(&symbol)
                .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
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

        // Version bump (the game repainted / an animation frame): the canvas is
        // re-transmitted, but the id still ON SCREEN must NOT be deleted with
        // it — deleting it before the big replacement transmit finishes parsing
        // blanks the window (the between-frames background flash Kerkerkruip's
        // opening sequence showed). It is freed one generation later.
        let gw2 = GraphicsWindow { win: 7, canvas: gw.canvas.clone(), version: 4, upscale: false };
        let mut buf3 = Buffer::empty(area);
        gr.render(&picker, &gw2, area, Style::default(), &mut buf3);
        let third = buf3.cell((0, 0)).unwrap().symbol().to_string();
        assert!(third.contains("a=T,U=1"), "repainted canvas is re-transmitted");
        assert!(!third.contains("a=d"), "the still-visible predecessor is NOT deleted yet");

        // Next repaint: NOW the two-generations-old id is freed.
        let gw3 = GraphicsWindow { win: 7, canvas: gw.canvas.clone(), version: 5, upscale: false };
        let mut buf4 = Buffer::empty(area);
        gr.render(&picker, &gw3, area, Style::default(), &mut buf4);
        let fourth = buf4.cell((0, 0)).unwrap().symbol().to_string();
        assert!(fourth.contains("a=d,d=I"), "the id retired two generations back is freed");
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
        let top_key = (top.x, top.y, top.width, top.height);
        let bot_key = (bottom.x, bottom.y, bottom.width, bottom.height);
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
}
