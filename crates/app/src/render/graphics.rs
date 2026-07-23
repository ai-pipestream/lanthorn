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

#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
    /// One-image cache for the v6 pixel composite (Phase 1c), keyed on a content
    /// hash + area so unchanged frames reuse the uploaded protocol.
    v6: Option<(u64, u16, u16, Protocol)>,
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

    /// Draw a pre-composited v6 canvas as ONE terminal image, upscaled to fill
    /// `area`. The canvas is in the game's native pixel space (e.g. 320×200); we
    /// explicitly upscale it (Nearest → crisp pixel art) to the pane's device
    /// pixels, preserving aspect, then hand it to the image protocol at native
    /// size. (Relying on the protocol's own `Resize::Scale` left it at native
    /// size — small in a large pane.) Cached on a content hash + area so
    /// identical frames don't re-encode/upload.
    pub fn draw_v6_canvas(&mut self, picker: &Picker, canvas: &image::RgbaImage, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || canvas.width() == 0 || canvas.height() == 0 {
            return;
        }
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        canvas.as_raw().hash(&mut h);
        let hash = h.finish();
        let fresh = matches!(&self.v6, Some((v, w, ht, _)) if *v == hash && *w == area.width && *ht == area.height);
        if !fresh {
            // Target device-pixel box for the pane, then the largest integer-ish
            // uniform upscale that fits it (aspect preserved).
            let fs = picker.font_size();
            let box_w = area.width as u32 * fs.width.max(1) as u32;
            let box_h = area.height as u32 * fs.height.max(1) as u32;
            let (cw, ch) = (canvas.width(), canvas.height());
            let scale = ((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64)).max(1.0);
            let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
            let scaled = image::imageops::resize(canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
            let img = image::DynamicImage::ImageRgba8(scaled);
            match picker.new_protocol(img, Size::new(area.width, area.height), Resize::Fit(None)) {
                Ok(p) => self.v6 = Some((hash, area.width, area.height, p)),
                Err(_) => return,
            }
        }
        if let Some((_, _, _, proto)) = &self.v6 {
            let sz = proto.size();
            let w = sz.width.min(area.width);
            let ht = sz.height.min(area.height);
            let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - ht) / 2, w, ht);
            Image::new(proto).render(dest, buf);
        }
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

    #[test]
    fn draw_v6_canvas_caches_on_content_hash() {
        let picker = Picker::halfblocks();
        let mut gr = GraphicsRender::default();
        let area = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(area);
        let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));
        gr.draw_v6_canvas(&picker, &canvas, area, &mut buf);
        assert!(gr.v6.is_some(), "first draw builds + caches the protocol");
        let (hash0, _, _, _) = gr.v6.as_ref().unwrap();
        let hash0 = *hash0;
        // Same content → same hash (no rebuild churn on identical frames).
        gr.draw_v6_canvas(&picker, &canvas, area, &mut buf);
        assert_eq!(gr.v6.as_ref().unwrap().0, hash0, "identical canvas keeps the cached entry");
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
