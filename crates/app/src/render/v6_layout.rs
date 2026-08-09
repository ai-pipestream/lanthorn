//! v6 layout classification: split the engine's flat window list into the
//! single scrolling story window (a primary `Buffer`) and everything else
//! (chrome — frame graphics, status grids, etc.). Pure classification, no
//! rendering (Phase 1a).

use image::{Rgba, RgbaImage};

use crate::colors::ColorScheme;
use crate::engine::{PositionedWindow, PxText, WinNode};

/// Resolve a packed z-colour (see `crate::state::pack_zcolour`) to an opaque
/// RGBA. `0` (Default) → `fallback`. True24 → its RGB. Palette/standard colours
/// resolve through the theme; anything that doesn't reduce to a concrete RGB
/// falls back (v1 — richer palette handling is SQ-0450).
/// A packed z-colour (see [`crate::state::pack_zcolour`]) is EXPLICIT only when
/// the game named a real colour. `ZColour::Default` (0) and Standard 0/1
/// ("current"/"default", ZMSD §8.3.1) are not choices — they're inheritance —
/// so they are NOT explicit and the theme keeps the channel. Standard 2-9 and
/// every True/True24 value ARE explicit. Shared by the raster block-paint
/// decision and the cell colour paths so both gate identically. (SQ-0487/0488)
pub(crate) fn packed_explicit(packed: u32) -> bool {
    packed != 0 && !((packed >> 24) == 1 && (packed & 0xFF) <= 1)
}

/// The ZMSD §8.3.1 recommended true-colour equivalents for the Standard palette
/// colours 2..=9. On the pixel/canvas paths we resolve Standard colours to these
/// DOS/spec-authentic RGBs directly rather than routing them through the theme's
/// ANSI palette, which a user theme may remap arbitrarily (SQ-0506). Greys
/// (10..=12) still go through `resolve_zcolour`/`grey_rgb`, which already carry
/// their own fixed RGB.
fn standard_pixel_rgb(n: u8) -> Option<Rgba<u8>> {
    // SQ-0532/A-F5: the table itself now lives in `colors::STANDARD_COLOUR_RGB15`
    // so the terminal cell palette resolves Standard colours to the SAME §8.3.1
    // RGBs this pixel path uses (they used to disagree — e.g. white).
    let (r, g, b) = crate::colors::standard_colour_rgb(n)?;
    Some(Rgba([r, g, b, 255]))
}

/// The pixel colour a packed z-colour names OUTRIGHT, or `None` when it names
/// none (SQ-0706).
///
/// Every case here resolves without a [`ColorScheme`]: true-colour is arithmetic,
/// and Standard 2..=9 have fixed RGB in ZMSD §8.3.1 (see [`standard_pixel_rgb`]),
/// which is what the pixel path already uses so that white is real white rather
/// than VGA grey. `Default`, and the "current"/"default" sentinels 0 and 1, name
/// no colour: they mean "inherit", which is the host's business, not a painted
/// rectangle's.
///
/// This is what lets `GameSession` rasterize `erase_window` fills into a bounded
/// surface as they arrive, instead of hoarding an unbounded list of rects to
/// resolve later against a theme it cannot see.
pub(crate) fn explicit_pixel_rgba(packed: u32) -> Option<Rgba<u8>> {
    match packed >> 24 {
        3 => {
            let v = packed & 0x00FF_FFFF;
            Some(Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
        }
        2 => {
            let v = (packed & 0xFFFF) as u16;
            let (r, g, b) = ((v & 0x1F) as u8, ((v >> 5) & 0x1F) as u8, ((v >> 10) & 0x1F) as u8);
            // 5 bits per channel → 8, replicating the high bits (0x1F → 0xFF).
            Some(Rgba([(r << 3) | (r >> 2), (g << 3) | (g >> 2), (b << 3) | (b >> 2), 255]))
        }
        1 => standard_pixel_rgb((packed & 0xFF) as u8),
        _ => None,
    }
}

pub(crate) fn packed_to_rgba(packed: u32, fallback: Rgba<u8>, colors: &ColorScheme) -> Rgba<u8> {
    if packed == 0 {
        return fallback;
    }
    let tag = packed >> 24;
    if tag == 3 {
        let v = packed & 0x00FF_FFFF;
        return Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]);
    }
    // Standard(n)=tag 1, True(v)=tag 2 → reconstruct the ZColour and resolve via
    // the scheme; use the concrete RGB when the theme yields one, else fallback.
    let z = match tag {
        1 => zvm::screen::ZColour::Standard((packed & 0xFF) as u8),
        2 => zvm::screen::ZColour::True((packed & 0xFFFF) as u16),
        _ => return fallback,
    };
    // Pixel path: Standard 2..=9 resolve to their ZMSD §8.3.1 true-colour RGB,
    // bypassing the theme ANSI palette so white is real white, not VGA grey.
    if let zvm::screen::ZColour::Standard(n) = z {
        if let Some(rgb) = standard_pixel_rgb(n) {
            return rgb;
        }
    }
    color_to_rgba(crate::render::resolve_zcolour(z, colors), fallback)
}

/// Resolve a ratatui [`Color`] to an opaque RGBA for the pixel canvas. The cell
/// path renders NAMED ANSI colours (the terminal_default palette maps Standard
/// 2–9 to `Color::Red`/`Color::Blue`/… — the terminal draws them directly), but
/// the raster canvas needs concrete bytes: mapping only `Color::Rgb` dropped
/// every palette colour to the fallback, so Zork Zero's compass-direction
/// letters blitted in the default ink instead of their own colour (SQ-0480). The
/// 16 base ANSI colours resolve to the standard VGA RGB values; `Reset` and
/// `Indexed` (no canonical RGB here) fall back.
pub(crate) fn color_to_rgba(c: ratatui::style::Color, fallback: Rgba<u8>) -> Rgba<u8> {
    use ratatui::style::Color;
    let (r, g, b) = match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
        Color::Reset | Color::Indexed(_) => return fallback,
    };
    Rgba([r, g, b, 255])
}

/// 1:1 opaque-over blit of `src` into `dst` at `(dx, dy)`, clipped to the
/// `max_w × max_h` box anchored at `(dx, dy)` (a v6 window's pixel box).
pub(crate) fn blit_clipped(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// Like [`blit_clipped`], but starts reading `src` at row `src_y` — for a
/// margin float partially scrolled off the top of the story view.
pub(crate) fn blit_clipped_src(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, src_y: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().saturating_sub(src_y).min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, src_y + oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// The v6 font cell size in game pixels — matches `zvm::screen::V6_FONT_WIDTH`
/// / `V6_FONT_HEIGHT`. The cell is NON-SQUARE (8×16, SQ-0479): X quantizes by
/// `FONT_W`, Y by `FONT_H`. Glyph masters are 8×8; `blit_glyph` fills the 8×16
/// cell by nearest-neighbour vertical doubling (DOS-authentic).
const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

/// A window-0 inline picture floated beside the story text: anchored to a
/// wrapped display row, reserving columns for the picture and narrowing the rows
/// beside it. `row` is relative to the visible window and may be negative when
/// the float has partially scrolled off the top.
///
/// The float side is expressed by the column fields (not an enum): a LEFT float
/// (Zork Zero's drop-cap) blits at `img_col == 0` with text pushed right
/// (`text_col == reserve_cols`); a RIGHT float (Shogun's opening picture, ZMSD
/// §15 margin picture) blits at `img_col` near the right edge with text flush
/// left (`text_col == 0`). Either way the wrap width on covered rows is
/// `cols - reserve_cols`.
#[derive(Debug, Clone)]
pub struct RasterFloat {
    pub row: i32,
    pub rows: u16,
    /// Columns removed from the text width on the rows this float covers.
    pub reserve_cols: u16,
    /// Column where each covered row's text begins.
    pub text_col: u16,
    /// Column where the picture is blitted.
    pub img_col: u16,
    pub img: std::sync::Arc<RgbaImage>,
}

/// The story (primary) window's rasterizable content: visible wrapped lines
/// (oldest-first), the live input line, and the caret column. `awaiting` gates
/// the input line + block cursor (drawn while the view sits at the bottom of the
/// transcript; deliberately independent of which pane holds keyboard focus).
/// `floats` carries the window-0 inline pictures anchored within the visible
/// rows — blitted at the left margin with text indented beside them.
#[derive(Debug, Default, Clone)]
pub struct MainText {
    pub lines: Vec<String>,
    /// Per-character ZMSD §8.7.1 style bytes for `lines`, parallel to it
    /// (SQ-0540). An EMPTY inner vec — the common case, and what a short vec's
    /// missing tail means — is an all-roman row, so a transcript with no
    /// emphasis costs nothing. Only bold (2) and italic (4) are carried; the
    /// raster prose path has no reverse-video block to draw into.
    pub styles: Vec<Vec<u8>>,
    pub input: String,
    pub cursor_col: u16,
    pub awaiting: bool,
    pub floats: Vec<RasterFloat>,
}

/// The native screen extent (max window bottom-right) in game pixels; min 1×1.
///
/// A window whose `w_px`/`h_px` is an unresolved size sentinel — a small
/// negative value stored as a large `u16` (Shogun leaks `0xFFFE` ≈ −2 into a
/// window's `x_size`, ballooning the extent to 65534×200 and the raster canvas
/// allocation with it, SQ-0481) — must not drive the extent. Any dimension with
/// the high bit set (`>= 0x8000`, i.e. negative as `i16`) is far past any real
/// v6 screen (~640 px) so it's treated as unresolved and skipped for that axis;
/// clamping here (presentation) keeps zvm storing window props verbatim for the
/// game to read back (ZMSD §8.8.3.2).
pub fn native_extent(items: &[PositionedWindow]) -> (u16, u16) {
    let mut w = 1u16;
    let mut h = 1u16;
    let resolved = |px: u16| (px as i16) >= 0; // high bit clear ⇒ a real size
    for it in items {
        if resolved(it.w_px) {
            w = w.max(it.x_px.saturating_add(it.w_px));
        }
        if resolved(it.h_px) {
            h = h.max(it.y_px.saturating_add(it.h_px));
        }
        // A window sized to zero can still hold painted text runs at their
        // screen-absolute pixel positions (Journey's height-0 command menu,
        // SQ-0492): its w_px/h_px don't reach the runs, so grow the extent to
        // cover them directly, or the chrome canvas clips the menu off the
        // bottom. Runs carry 1-based top-left coords; a glyph spans FONT×FONT.
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                let n = t.text.chars().count() as u32;
                let right = (t.x.max(1) as u32 - 1) + n * FONT_W;
                let bottom = (t.y.max(1) as u32 - 1) + FONT_H;
                w = w.max(right.min(u16::MAX as u32) as u16);
                h = h.max(bottom.min(u16::MAX as u32) as u16);
            }
        }
    }
    (w, h)
}

/// The v6 window list split into the story window, the story window's own
/// picture (the room illustration — story content, NOT chrome), and everything
/// else (chrome), in input order.
pub struct V6Layout<'a> {
    pub story: Option<&'a PositionedWindow>,
    /// The primary window's Graphics entry (window 0's picture canvas — a room
    /// illustration). It belongs to the story, so it is rendered inside the story
    /// region rather than composited as absolute chrome over the frame.
    pub story_gfx: Option<&'a PositionedWindow>,
    pub chrome: Vec<&'a PositionedWindow>,
}

/// Classify `items`: the first primary `Buffer` becomes `story`; window 0's own
/// `Graphics` entry becomes `story_gfx` (story content); every other entry (in
/// input order) goes into `chrome`. With no primary `Buffer`, `story` is `None`
/// and non-window-0 graphics/grids are chrome.
pub fn classify_windows(items: &[PositionedWindow]) -> V6Layout<'_> {
    let mut story = None;
    let mut story_gfx = None;
    let mut chrome = Vec::new();
    for pw in items {
        if story.is_none() && matches!(&pw.node, WinNode::Buffer(b) if b.primary) {
            story = Some(pw);
        } else if story_gfx.is_none() && matches!(&pw.node, WinNode::Graphics(g) if g.win == 0) {
            story_gfx = Some(pw);
        } else {
            chrome.push(pw);
        }
    }
    V6Layout { story, story_gfx, chrome }
}

/// The story window's own background colour (set by the game via
/// `set_colour`), resolved to an opaque RGBA for filling the story rect
/// before floats/text. `None` when the game set no colour — the caller then
/// falls back to its resolved default page (SQ-0510); either way the rect ends
/// up opaque, never left for a compositor to colour in.
pub fn story_bg_rgba(story: Option<&PositionedWindow>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    let WinNode::Buffer(b) = &story?.node else { return None };
    // `bg`, when `Some`, always packs a non-Default channel (see
    // `state::pack_zcolour`), so the fallback here is never actually used —
    // it exists only to satisfy `packed_to_rgba`'s signature.
    Some(packed_to_rgba(b.bg?, Rgba([0, 0, 0, 255]), colors))
}

/// The story window's own FOREGROUND colour (set by the game via `set_colour`),
/// resolved to an opaque RGBA for the ink the story prose is rasterized in.
/// `None` when the game set no colour — the caller then falls back to its
/// resolved default ink.
///
/// The exact mirror of [`story_bg_rgba`], and for the same reason (SQ-0532
/// wave-5): the pair is the game's, so it has to be honoured as a pair. Zork
/// Zero boots `set_colour(fg=2 black, bg=9 white)` on window 0; taking its white
/// page but keeping the host's own (light) default ink rasterized white-on-white
/// prose that could not be read at all.
pub fn story_fg_rgba(story: Option<&PositionedWindow>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    let WinNode::Buffer(b) = &story?.node else { return None };
    // `fg`, when `Some`, always packs a non-Default channel (see
    // `state::pack_zcolour`), so the fallback here is never actually used —
    // it exists only to satisfy `packed_to_rgba`'s signature.
    Some(packed_to_rgba(b.fg?, Rgba([255, 255, 255, 255]), colors))
}

/// The story window's explicit `(fg, bg)` pair as PACKED z-colours (`0` when the
/// game set none), for the cell-side callers — the live input line resolves them
/// through `resolve_zcolour`, exactly as the transcript's prose runs do, rather
/// than through the pixel path's [`story_fg_rgba`]/[`story_bg_rgba`]. Same
/// source, same window, one resolution per path. (SQ-0532 wave-6)
pub fn story_pair_packed(story: Option<&PositionedWindow>) -> (u32, u32) {
    match story.map(|s| &s.node) {
        Some(WinNode::Buffer(b)) => (b.fg.unwrap_or(0), b.bg.unwrap_or(0)),
        _ => (0, 0),
    }
}

/// Whether two positioned windows' native pixel boxes intersect at all.
fn boxes_overlap(a: &PositionedWindow, b: &PositionedWindow) -> bool {
    let (ax0, ay0) = (a.x_px as u32, a.y_px as u32);
    let (bx0, by0) = (b.x_px as u32, b.y_px as u32);
    let (ax1, ay1) = (ax0 + a.w_px as u32, ay0 + a.h_px as u32);
    let (bx1, by1) = (bx0 + b.w_px as u32, by0 + b.h_px as u32);
    ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1
}

/// SQ-0704: resolve each chrome window's still-UNPAINTED area to that window's
/// OWN background colour.
///
/// ZMSD §8.8.3.2 gives every Version 6 window its own foreground/background pair
/// (property 11). [`build_chrome_canvas`] resolves everything against a SINGLE
/// host `default_fg`/`default_bg`, and consults a window's own pair only for its
/// text runs (`fill_explicit_bg_rows`, SQ-0519) — it never paints a window's own
/// page, and the graphics pass blits alpha untouched. So a window whose art is
/// mostly transparent reached the terminal as transparency, and whatever the
/// protocol composited it over became the backdrop. Zork Zero's room/compass
/// icons (pictures 9/10/11/13, 45×40, ~95 % alpha-0 line art drawn into its
/// 640×78 banner window) hang below the banner artwork, and there the clear
/// ground rendered as an opaque BLACK box where the DOS original shows the
/// window's white page.
///
/// Only pixels no layer has touched (`alpha == 0`) are painted, so frame art,
/// status bands, glyphs and the icons' own lit strokes are left byte-for-byte
/// alone — the faithful alpha compositing is preserved, and a window the game
/// gave no colour is skipped entirely and keeps today's behaviour (its holes
/// still fall through to the caller's page).
///
/// A window that overlaps the STORY box is skipped too: Zork Zero's window 7
/// carries the same white page across the whole 640×400 screen, and both the
/// hybrid transcript viewport and [`story_clear_native`]'s clear-interior probe
/// need that region to stay transparent. The story window's page is painted by
/// the story paths (`fill_pane_page` in hybrid, the story-rect fill plus
/// [`flatten_onto_page`] in raster), which already honour its own colour.
///
/// Callers gate this on `honor_game_colours`: with the game's colours declined
/// the host page governs everywhere, exactly as before — except for the windows
/// [`fill_painted_window_pages`] carves back out, which are the game's own canvas
/// rather than its colour preference.
pub fn fill_window_pages(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
) {
    fill_pages_where(canvas, chrome, story, colors, |_| true);
}

/// SQ-0716: the half of [`fill_window_pages`] that survives `honor_game_colours
/// = off` — a window the game has DRAWN INTO keeps its declared page.
///
/// scopa's felt table is why. Measured from the screen ops rather than the model,
/// it boots `@set_true_colour(fg=true(0x0000), bg=true(0x0200), window=1)` — an
/// explicit green — sizes window 1 to the full 640×400 screen and issues
/// `@erase_window`. That is a FILL, the same drawing operation SQ-0706 declared
/// ungatable when it made the cards survive a declined palette; the cards and the
/// table come out of the identical opcode. It reaches us as a window page only
/// because `drain_erase_fills` classifies a fill spanning the whole screen as a
/// screen clear and drops it, leaving window 1's background as the sole surviving
/// record of the paint. Gating that record on the colour flag therefore deleted
/// half of one drawing: declining game colours left a BLACK table carrying the
/// green stripes and cards the game had drawn onto it — worse than either
/// honouring the game or ignoring it.
///
/// The discriminator is the painted ground, exactly as in SQ-0711: a window with
/// the game's own pixels inside it is a canvas, and its page is the ground those
/// pixels were drawn on. A window with none is presentation, and the host page
/// governs it as before. Zork Zero, Arthur, Shogun, Journey and advent paint no
/// ground at all, so none of them can reach this path.
///
/// The STORY window is deliberately NOT included (and `fill_pages_where` skips
/// anything overlapping it anyway): its page and ink are the surface prose is read
/// on, they have to be honoured or declined as a PAIR (SQ-0532 wave-5), and that
/// pair is precisely what `honor_game_colours` exists to govern.
pub fn fill_painted_window_pages(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
    paint: Option<&RgbaImage>,
) {
    let Some(paint) = paint else { return };
    fill_pages_where(canvas, chrome, story, colors, |it| window_has_paint(it, paint));
}

/// Whether the game's painted ground has any pixel inside `it`'s native box.
fn window_has_paint(it: &PositionedWindow, paint: &RgbaImage) -> bool {
    let (x0, y0) = (it.x_px as u32, it.y_px as u32);
    let x1 = (x0 + it.w_px as u32).min(paint.width());
    let y1 = (y0 + it.h_px as u32).min(paint.height());
    (y0..y1).any(|y| (x0..x1).any(|x| paint.get_pixel(x, y)[3] > 0))
}

/// The shared body of [`fill_window_pages`] and [`fill_painted_window_pages`]:
/// paint each `keep`-approved chrome window's own page into its untouched pixels.
fn fill_pages_where(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
    keep: impl Fn(&PositionedWindow) -> bool,
) {
    for it in chrome {
        let bg = match &it.node {
            WinNode::Grid(g) => g.bg,
            WinNode::Buffer(b) => b.bg,
            _ => None,
        };
        // Only a colour the game actually NAMED counts (`packed_explicit`):
        // "current"/"default" are inheritance, not a page choice.
        let Some(bg) = bg.filter(|&p| packed_explicit(p)) else { continue };
        // A size sentinel (negative read as i16) is not a real box (SQ-0481).
        if it.w_px == 0 || it.h_px == 0 || (it.w_px as i16) < 0 || (it.h_px as i16) < 0 {
            continue;
        }
        if story.is_some_and(|s| boxes_overlap(it, s)) {
            continue;
        }
        if !keep(it) {
            continue;
        }
        // `bg` is explicit here, so the fallback can never be reached.
        let page = packed_to_rgba(bg, Rgba([0, 0, 0, 255]), colors);
        let (x0, y0) = (it.x_px as u32, it.y_px as u32);
        let x1 = (x0 + it.w_px as u32).min(canvas.width());
        let y1 = (y0 + it.h_px as u32).min(canvas.height());
        for y in y0..y1 {
            for x in x0..x1 {
                if canvas.get_pixel(x, y)[3] == 0 {
                    canvas.put_pixel(x, y, page);
                }
            }
        }
    }
}

/// Fill the STORY window's still-unpainted pixels with its own declared page
/// (SQ-0704, hybrid half).
///
/// [`fill_window_pages`] deliberately skips any window overlapping the story box,
/// because in RASTER mode the story page is painted separately by
/// `build_v6_raster_canvas` and the whole canvas is flattened opaque before it
/// ships. HYBRID has no such flatten: it draws the story as terminal text and
/// ships only the ring bands as images — and those bands overlap the story box,
/// both in the one-row sliver under a top banner and along the flanks. Every pixel
/// left transparent there is resolved by the TERMINAL, not by us, so Zork Zero's
/// room icons came out sitting on the terminal background instead of the white page
/// the game declared for the window they live in.
///
/// Only pixels no layer has touched are filled, and only when the story window
/// named a page explicitly — a game that set none keeps today's behaviour.
pub fn fill_story_page_clear(
    canvas: &mut RgbaImage,
    story: Option<&PositionedWindow>,
    colors: &ColorScheme,
) {
    let Some(it) = story else { return };
    let Some(page) = story_bg_rgba(Some(it), colors) else { return };
    if it.w_px == 0 || it.h_px == 0 || (it.w_px as i16) < 0 || (it.h_px as i16) < 0 {
        return;
    }
    let (x0, y0) = (it.x_px as u32, it.y_px as u32);
    let x1 = (x0 + it.w_px as u32).min(canvas.width());
    let y1 = (y0 + it.h_px as u32).min(canvas.height());
    for y in y0..y1 {
        for x in x0..x1 {
            if canvas.get_pixel(x, y)[3] == 0 {
                canvas.put_pixel(x, y, page);
            }
        }
    }
}

/// The native pixel rects the game's own chrome TEXT occupies — one per painted
/// `px_texts` run, one per non-blank cell of a plain character grid (SQ-0728).
///
/// It is deliberately the runs the GAME printed, not every opaque pixel the text
/// pass left: `fill_reverse_row_gaps` also paints, and its screen-wide fill is a
/// host device for closing the bare cells inside a bar (SQ-0504), not something
/// the game drew. Journey draws a one-cell reversed divider on each of nineteen
/// rows, which qualifies every one of them as a "pure reverse row" and floods the
/// gap either side — right across window 0's text panel. That flood must yield to
/// the story page; the labels a game deliberately printed inside window 0's box
/// must not.
fn chrome_text_rects(chrome: &[&PositionedWindow]) -> Vec<(u32, u32, u32, u32)> {
    let mut rects = Vec::new();
    for it in chrome {
        // A secondary prose window's lines are drawn onto the composite too
        // (SQ-0729), so the story page must spare them exactly as it spares a
        // grid's runs — else fmvpoker's menu bar, printed inside window 0's box,
        // is painted out the moment it is painted in.
        rects.extend(buffer_line_rects(it));
        let WinNode::Grid(g) = &it.node else { continue };
        if !g.px_texts.is_empty() {
            for t in &g.px_texts {
                let x = t.x.max(1) as u32 - 1;
                let y = t.y.max(1) as u32 - 1;
                rects.push((x, y, x + t.text.chars().count().max(1) as u32 * FONT_W, y + FONT_H));
            }
            continue;
        }
        let (ox, oy) = (it.x_px as u32, it.y_px as u32);
        for row in 0..g.rows {
            for col in 0..g.cols {
                let cell = g.cell(row + 1, col + 1);
                if cell.ch == '\0' || (cell.ch == ' ' && cell.bg == 0) {
                    continue;
                }
                let (x, y) = (ox + col as u32 * FONT_W, oy + row as u32 * FONT_H);
                rects.push((x, y, x + FONT_W, y + FONT_H));
            }
        }
    }
    rects
}

/// Paint the story window's clear interior with its `page`, sparing every pixel a
/// chrome text run claimed (SQ-0728).
///
/// The page has to be opaque — raster ships one image, and a transparent pixel is
/// resolved by whoever composites it rather than by us (SQ-0510) — but it is also
/// the OLDEST thing in the box: the game filled window 0, then other windows
/// printed on top of it. Shogun's title is the measured case. Its menu window sits
/// inside window 0's 548x64 box and prints "START the game" there while window 0
/// prints "You may choose to:" beside it; both are on the screen at once on a real
/// interpreter. A flat fill of the box erased the menu.
pub fn fill_story_page_under_chrome_text(
    canvas: &mut RgbaImage,
    (bx, by, bw, bh): (u32, u32, u32, u32),
    page: Rgba<u8>,
    chrome: &[&PositionedWindow],
) {
    let text: Vec<(u32, u32, u32, u32)> = chrome_text_rects(chrome)
        .into_iter()
        .filter(|&(x0, y0, x1, y1)| x0 < bx + bw && bx < x1 && y0 < by + bh && by < y1)
        .collect();
    let (cw, ch) = (canvas.width(), canvas.height());
    for y in by..(by + bh).min(ch) {
        let row: Vec<(u32, u32)> =
            text.iter().filter(|&&(_, y0, _, y1)| y >= y0 && y < y1).map(|&(x0, _, x1, _)| (x0, x1)).collect();
        for x in bx..(bx + bw).min(cw) {
            if row.iter().any(|&(x0, x1)| x >= x0 && x < x1) {
                continue;
            }
            canvas.put_pixel(x, y, page);
        }
    }
}

/// Whether any pixel in the `w × h` box at `(px, py)` of `canvas` is opaque
/// (alpha ≥ 128). Used to tell a reverse-video run sitting ON frame art from one
/// over a clear background, so the art is preserved but a bare selection bar still
/// gets its highlight block (SQ-0487). Out-of-bounds pixels count as transparent.
pub(crate) fn region_has_opaque(canvas: &RgbaImage, px: u32, py: u32, w: u32, h: u32) -> bool {
    let (cw, ch) = (canvas.width(), canvas.height());
    for y in py..(py + h).min(ch) {
        for x in px..(px + w).min(cw) {
            if canvas.get_pixel(x, y)[3] >= 128 {
                return true;
            }
        }
    }
    false
}

pub(crate) fn fill_cell(canvas: &mut RgbaImage, px: u32, py: u32, cw: u32, ch: u32, color: Rgba<u8>) {
    let (w, h) = (canvas.width(), canvas.height());
    for y in py..(py + ch).min(h) {
        for x in px..(px + cw).min(w) {
            canvas.put_pixel(x, y, color);
        }
    }
}

/// Flatten a FULLY COMPOSED raster canvas onto an opaque `page` (SQ-0510):
/// every pixel the composite left completely transparent (`alpha == 0`) becomes
/// `page`; every pixel any layer touched (`alpha > 0` — frame art, status bands,
/// the story page fill, glyphs, inline drop-caps) is left exactly as it was.
///
/// Why: raster mode ships the whole canvas as ONE image, and a transparent pixel
/// is then resolved by whoever composites it — not by us. The kitty encoder
/// (`ratatui_image`'s `transmit_virtual`, `f=32`) keeps the alpha channel and
/// lets the terminal decide; the halfblocks encoder flattens with `to_rgb8()`
/// and maps an untouched cell's `Color::Reset` to **white**. So "transparent"
/// renders differently per protocol and per terminal, and is never safe in
/// raster mode. Painting the leftovers ourselves makes the composite
/// self-contained and identical everywhere.
///
/// Only ever called on the raster path's finished canvas. The HYBRID path must
/// NOT use this — there transparency is load-bearing (the chrome ring's clear
/// middle is what lets the terminal transcript show through).
pub(crate) fn flatten_onto_page(canvas: &mut RgbaImage, page: Rgba<u8>) {
    for px in canvas.pixels_mut() {
        if px[3] == 0 {
            *px = page;
        }
    }
}

/// Build the CHROME image: one native-resolution RGBA canvas containing only
/// the frame graphics and status text (everything `classify_windows` put in
/// `chrome`). The story region and any gaps stay fully transparent — a later
/// task scales this canvas to the pane and layers it over the story text.
///
/// Two passes, in list order, frame graphics behind status text: Graphics
/// entries are blitted first (later entries draw over earlier ones only where
/// opaque, giving correct z-order for overlapping frame art like Zork Zero's
/// compass); Grid entries are rasterized second, one glyph per `FONT × FONT`
/// native-pixel cell, drawing every row regardless of the window's pixel
/// height (a v6 status grid can legitimately exceed its pixel box).
///
/// A `px_texts` run's `style` bit 1 (reverse) swaps its resolved fg/bg: the
/// glyph ink is drawn in the run's (window) background colour and a solid
/// block in the run's foreground colour is painted behind it — reverse always
/// paints an opaque block (there is no "transparent ink"), so a run whose
/// colours are unset falls back to `default_bg`/`default_fg` respectively
/// rather than leaving the swapped-in channel transparent.
/// Blit every chrome Graphics window onto `canvas`, in list order (later entries
/// draw over earlier ones only where opaque). The window canvas is authored in
/// native game pixels (pictures at their native size/coords), so blit it 1:1 at
/// the window origin — never scaled — clipped to the window's pixel box (ZMSD §8:
/// plotting is always clipped to the window; a canvas can be larger than the
/// current box when the window has since shrunk). Shared by [`build_chrome_canvas`]
/// (pass 1) and [`build_graphics_canvas`].
fn blit_chrome_graphics(canvas: &mut RgbaImage, chrome: &[&PositionedWindow]) {
    for it in chrome {
        if let WinNode::Graphics(gwn) = &it.node {
            let src = &gwn.canvas;
            blit_clipped(canvas, src, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
        }
    }
}

/// Composite the v6 PAINTED GROUND onto `canvas` — the filled rectangles an
/// `erase_window` left behind (SQ-0706), at their absolute native positions.
///
/// It is GROUND: it goes UNDER everything already drawn, and is itself drawn
/// before the window pages claim the rest.
///
/// A painted fill is the oldest thing on the screen — the game filled a rectangle,
/// then printed its label on top. Compositing the surface OVER the chrome canvas
/// erased exactly those labels: scopa's menu came out as white buttons with no
/// text, because its button fills landed on top of the glyphs that had already
/// been rasterized. So only pixels no layer has touched take paint, and the order
/// is: chrome art and glyphs, then this ground beneath them, then the window pages
/// filling whatever neither claimed.
pub fn blit_paint_ground(canvas: &mut RgbaImage, paint: Option<&RgbaImage>) {
    let Some(src) = paint else { return };
    let (w, h) = (src.width().min(canvas.width()), src.height().min(canvas.height()));
    for y in 0..h {
        for x in 0..w {
            let p = *src.get_pixel(x, y);
            if p[3] > 0 && canvas.get_pixel(x, y)[3] == 0 {
                canvas.put_pixel(x, y, p);
            }
        }
    }
}

/// Blit the STORY window's own absolutely-placed artwork ([`V6Layout::story_gfx`])
/// onto `canvas` at its native origin (SQ-0695).
///
/// `classify_windows` has always set this entry aside — a `WinNode::Graphics` whose
/// `win` is 0 is story content, not chrome — but nothing ever drew it, so it was
/// classified and dropped. Arthur's intro is what needs it: each illustrated screen
/// centres a 584×392 plate in window 0, so the plate is a BACKDROP occupying the
/// story window rather than part of the frame ring.
///
/// Callers blit it after the story page fill and before the story text, which is
/// the painter's order the game itself used: page, then plate, then prose — see
/// [`story_prose_box`] for whether any prose belongs on this frame at all.
pub fn blit_story_gfx(canvas: &mut RgbaImage, story_gfx: Option<&PositionedWindow>) {
    let Some(it) = story_gfx else { return };
    let WinNode::Graphics(gwn) = &it.node else { return };
    blit_clipped(canvas, &gwn.canvas, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
}

/// A prose column narrower than this (cells) is not a text box — it is a sliver.
/// Mirrors the identical floor `build_main_text` applies before wrapping prose
/// beside an inline float, and the SQ-0578 lesson that a one-column story box
/// re-wraps the whole transcript a character per line.
const MIN_PROSE_COLS: u32 = 8;

/// The largest axis-aligned rectangle inside `clear` (native game pixels) that the
/// `story_gfx` plate painted no pixel of. `None` when the plate leaves nothing.
/// With no plate, or an unpainted one, the whole of `clear` is free.
///
/// Standard largest-rectangle-under-a-histogram sweep over the plate's alpha mask:
/// row by row, each column carries the run of consecutive free pixels above it, and
/// the monotone stack reads off every maximal rectangle ending at that row.
fn plate_free_box(
    clear: (u32, u32, u32, u32),
    story_gfx: Option<&PositionedWindow>,
) -> Option<(u32, u32, u32, u32)> {
    let (cx, cy, cw, chh) = clear;
    if cw == 0 || chh == 0 {
        return None;
    }
    let mut blocked = vec![false; (cw * chh) as usize];
    if let Some(it) = story_gfx {
        if let WinNode::Graphics(gwn) = &it.node {
            let (ox, oy) = (it.x_px as u32, it.y_px as u32);
            for (x, y, px) in gwn.canvas.enumerate_pixels() {
                if px.0[3] == 0 {
                    continue;
                }
                let (sx, sy) = (ox + x, oy + y);
                if sx < cx || sy < cy || sx >= cx + cw || sy >= cy + chh {
                    continue;
                }
                blocked[((sy - cy) * cw + (sx - cx)) as usize] = true;
            }
        }
    }
    let mut heights = vec![0u32; cw as usize];
    let mut best: Option<(u32, u32, u32, u32)> = None;
    let mut stack: Vec<(u32, u32)> = Vec::new(); // (start column, height)
    for r in 0..chh {
        for c in 0..cw {
            heights[c as usize] = if blocked[(r * cw + c) as usize] { 0 } else { heights[c as usize] + 1 };
        }
        stack.clear();
        for c in 0..=cw {
            let h = if c == cw { 0 } else { heights[c as usize] };
            let mut start = c;
            while let Some(&(s, sh)) = stack.last() {
                if sh <= h {
                    break;
                }
                stack.pop();
                let area = (c - s) as u64 * sh as u64;
                if best.is_none_or(|(_, _, bw, bh)| (bw as u64) * (bh as u64) < area) {
                    best = Some((cx + s, cy + r + 1 - sh, c - s, sh));
                }
                start = s;
            }
            stack.push((start, h));
        }
    }
    best.filter(|&(_, _, w, h)| w > 0 && h > 0)
}

/// Where the story window's prose goes once its absolutely-placed plate has the
/// floor — `None` when the plate owns the screen and no prose belongs on the
/// frame at all (SQ-0707).
///
/// An absolutely-placed window-0 picture is a BACKDROP the game draws INSTEAD of
/// prose, not underneath it. Arthur's intro is the measured case: each screen
/// `@erase_window(-1)`s, draws its plate, hides the cursor with `@set_cursor(-1)`
/// and waits on a `read_char` — the narration is a separate, picture-less screen
/// that the game erases before printing. The whole graveyard→Merlin turn is 31
/// instructions and prints not one character. So rasterizing the app's scrollback
/// onto the plate (which is what SQ-0695 shipped, on the mistaken premise that the
/// game "narrates over it") painted the previous screen's prose across the art.
///
/// The rule is the SQ-0578 one — "no room for text → the picture owns the screen"
/// — applied to a plate that blocks the MIDDLE rather than one that outgrew the
/// window. `story_clear_native` cannot see this: it insets from the EDGES, and a
/// centred plate touches none of them. So the free area is measured directly, as
/// the largest rectangle of `clear` the plate painted no pixel of
/// ([`plate_free_box`]); one too narrow to wrap into ([`MIN_PROSE_COLS`]) or too
/// short for one line means there is no prose box. A plate that leaves a genuine
/// column — a corner logo, a margin illustration — still gets prose beside it.
///
/// The free area is measured against what the plate PAINTED, never its bounding
/// box (SQ-0729). fmvpoker draws its poker table as a 640x400 frame with a hollow
/// middle: the ring's bounding box is the whole screen, so the bbox rule read the
/// game's own backdrop as a plate that owns the screen and the title dropped every
/// line of text it prints inside that frame. Only 17% of the picture is opaque, and
/// the hole in it is exactly where the game puts its prose.
pub fn story_prose_box(
    clear: (u32, u32, u32, u32),
    story_gfx: Option<&PositionedWindow>,
) -> Option<(u32, u32, u32, u32)> {
    plate_free_box(clear, story_gfx).filter(|&(_, _, w, h)| w >= MIN_PROSE_COLS * FONT_W && h >= FONT_H)
}

/// Build a native-resolution canvas containing ONLY the chrome frame graphics —
/// no status/menu text. Used by the hybrid band decomposition to tell a band
/// strip that sits over real artwork (keeps the pixel ring) from a pure-text
/// strip (paints as terminal cells), via [`region_has_opaque`] — the full chrome
/// canvas can't answer that because rasterized text is itself opaque (SQ-0500).
pub fn build_graphics_canvas(chrome: &[&PositionedWindow], native: (u16, u16)) -> RgbaImage {
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);
    blit_chrome_graphics(&mut canvas, chrome);
    canvas
}

/// SQ-0499: fill the unpainted interior cells of a PURE reverse-video row (one
/// whose every painted run is reversed) so a status/menu bar the game drew as
/// separate runs with bare gaps between them reads as one solid block. Games paint
/// a reversed bar as its text runs plus, sometimes, reversed spacer spaces — but
/// leave odd cells unpainted (Arthur's status skips one cell before "St Anne's
/// Day"; Journey's menu header leaves a wide gap between its two labels), and the
/// per-run block painting can't fill a cell no run covers. Only PURE reverse rows
/// qualify: a row carrying any NON-reversed run is a mixed layout (Journey's menu
/// BODY — reversed column dividers among normal verb text) and its gaps are real
/// background, left alone. Inherited reverse over opaque frame art still paints no
/// block (Zork0's ribbon labels sit ON the banner), matching the per-run over-art
/// rule so `region_has_opaque` gates each filled cell.
fn fill_reverse_row_gaps(
    canvas: &mut RgbaImage,
    art: &RgbaImage,
    texts: &[PxText],
    default_fg: Rgba<u8>,
    colors: &ColorScheme,
) {
    use std::collections::BTreeMap;
    let full_w = canvas.width();
    let mut rows: BTreeMap<u32, Vec<&PxText>> = BTreeMap::new();
    for t in texts {
        rows.entry(t.y.max(1) as u32 - 1).or_default().push(t);
    }
    for (py, runs) in rows {
        // Pure reverse-video row only: every run reversed (and at least one run).
        if runs.is_empty() || runs.iter().any(|t| t.style & 1 == 0) {
            continue;
        }
        // A pure reverse-video row is a bar the game draws edge to edge, so the
        // fill spans the ENTIRE screen width (SQ-0504): the runs the game painted,
        // plus every bare cell around AND between them. A row that named real
        // colours fills unconditionally; an inherited row defers to the over-art
        // rule per gap (so Zork0's ribbon labels on the banner never gain a bar).
        let mut explicit_block: Option<Rgba<u8>> = None;
        let mut spans: Vec<(u32, u32)> = runs
            .iter()
            .map(|t| {
                if explicit_block.is_none() && (packed_explicit(t.fg) || packed_explicit(t.bg)) {
                    explicit_block = Some(packed_to_rgba(t.fg, default_fg, colors));
                }
                let s = t.x.max(1) as u32 - 1;
                (s, s + t.text.chars().count().max(1) as u32 * FONT_W)
            })
            .collect();
        spans.sort_unstable();
        // The bare stretches: from x=0 to the first run, between the runs, and from
        // the last run to the screen edge. Filled at EXACT pixel extent (not cell-
        // quantized): a run's start is `x - 1`, rarely 8-aligned, so a quantized
        // fill cell would bleed a pixel into the next run — harmless to the over-art
        // test (SQ-0487), which reads the ART layer, but still the game's geometry.
        let mut gaps: Vec<(u32, u32)> = Vec::new();
        let mut cursor = 0u32;
        for &(s, e) in &spans {
            if s > cursor {
                gaps.push((cursor, s));
            }
            cursor = cursor.max(e);
        }
        if cursor < full_w {
            gaps.push((cursor, full_w));
        }
        for (gs, ge) in gaps {
            let block = match explicit_block {
                Some(b) => Some(b),
                None if region_has_opaque(art, gs, py, ge - gs, FONT_H) => None,
                None => Some(default_fg),
            };
            if let Some(b) = block {
                fill_cell(canvas, gs, py, ge - gs, FONT_H, b);
            }
        }
    }
}

/// SQ-0519: the window-wide background-flood colour for a chrome grid row, or
/// `None` when the row must not flood. Mirrors SQ-0512's hybrid per-row flood at
/// the raster canvas level: a NON-reverse row that names an explicit background
/// (first-explicit-wins per channel — Shogun's in-game status band prints
/// black-on-white, non-reversed) floods its whole window width with that bg so the
/// band reads as one solid bar in the pixel composite, not just behind the glyph
/// runs (the gaps between "Erasmus :", "SHOGUN", "Score:" otherwise showed the page
/// through). Two kinds of row return `None`, keeping the canvas byte-identical to
/// before: a row with no explicit background (Zork0's compass letters — explicit
/// FG only, no bg — so their windows never paint an opaque box over the banner
/// art), and a PURE reverse-video row (every run reversed — Zork0's on-banner ribbon
/// labels), which [`fill_reverse_row_gaps`] already handles edge to edge with the
/// over-art gate that leaves the art untouched. A mixed row (some reversed runs,
/// some not) still floods when it names an explicit bg, first-explicit-wins.
fn row_flood_bg(runs: &[&PxText], default_bg: Rgba<u8>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    // Pure reverse-video (or empty) rows are owned by `fill_reverse_row_gaps`.
    if runs.is_empty() || runs.iter().all(|t| t.style & 1 != 0) {
        return None;
    }
    let bg = runs.iter().map(|t| t.bg).find(|&p| packed_explicit(p))?;
    Some(packed_to_rgba(bg, default_bg, colors))
}

/// SQ-0519: flood the window-width background of each explicit-bg chrome grid row
/// (see [`row_flood_bg`]) BEFORE its glyphs stamp, so an explicitly-coloured status
/// band (Shogun's black-on-white location/score bar) reads as one solid bar across
/// the whole window — not just behind each run. `ox`/`win_w` are the window's own
/// native pixel extent: the flood spans only THIS window (unlike the screen-wide
/// pure-reverse SQ-0504 fill and unlike the hybrid full-width title-bar rule
/// SQ-0515, which are look decisions on other paths). Runs carry screen-absolute
/// pixel rows, so each row floods at its own run `y` (`y - 1`, one `FONT_H` tall).
fn fill_explicit_bg_rows(
    canvas: &mut RgbaImage,
    texts: &[PxText],
    ox: u32,
    win_w: u32,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
) {
    use std::collections::BTreeMap;
    let mut rows: BTreeMap<u32, Vec<&PxText>> = BTreeMap::new();
    for t in texts {
        rows.entry(t.y.max(1) as u32 - 1).or_default().push(t);
    }
    for (py, runs) in rows {
        if let Some(bg) = row_flood_bg(&runs, default_bg, colors) {
            // The point of this flood is to close the GAPS BETWEEN runs, so a bar
            // the game painted as several runs reads as one solid block. The runs'
            // own hull is therefore the floor; whether it also reaches the window's
            // edges is the question below.
            let lo = runs.iter().map(|t| u32::from(t.x.max(1)) - 1).min().unwrap_or(ox);
            let hi = runs
                .iter()
                .map(|t| (u32::from(t.x.max(1)) - 1) + t.text.chars().count().max(1) as u32 * FONT_W)
                .max()
                .unwrap_or(ox + win_w);
            // A window is the bar only when its runs REACH BOTH OF ITS EDGES —
            // within one character cell, the padding a game leaves at the ends of a
            // band it filled. Shogun's status band is that: runs 49..592 in a 46..594
            // window, three pixels of slack at one end and two at the other, so the
            // flood rounds it out edge to edge and the gaps between "Erasmus :",
            // "SHOGUN" and "Score:" close.
            //
            // Anything else is a label parked in a scratch window whose box describes
            // nothing, and flooding that box smears the label's background across the
            // screen. scopa positions its "abort"/"OK" button labels with one window 5
            // it moves and resizes for every draw — and whose size its `measure`
            // routine leaves at a 1000×1000 sentinel, clamped to the screen. Its
            // "abort" run lands at 567..607 while the box reads 579..640, outside on
            // the left (SQ-0706); selecting a card redraws the same button's label as
            // "OK" at 579..595, which starts exactly ON that left edge and stops 45 px
            // short of the right one — inside the box, but 45 px is not padding. That
            // flooded a white tab from the button's rounded outline out to the screen
            // edge, which is what the player saw as the OK label spreading rightwards
            // (SQ-0721). There, flood only what the runs occupy.
            let spans_window = lo <= ox + FONT_W && hi + FONT_W >= ox + win_w;
            let (fx, fe) = if spans_window { (lo.min(ox), hi.max(ox + win_w)) } else { (lo, hi) };
            fill_cell(canvas, fx, py, fe.saturating_sub(fx), FONT_H, bg);
        }
    }
}

/// SQ-0504: carve the native rows occupied by pure-TEXT chrome runs out of `canvas`
/// (make them fully transparent). Those rows render as crisp terminal CELLS in the
/// hybrid path, so their rasterized ink must not survive into the pixel bands built
/// from this canvas: a sub-cell letterbox-scale boundary otherwise samples the top
/// slice of a status/menu glyph into the neighbouring ART band and shows the raster
/// bar BEHIND the cells. Clearing them also decouples each art band's content hash
/// from the menu text, so navigating the menu re-encodes only genuinely changed art
/// (picture column, status panel), not every band. `run_tops` are each cleared run's
/// native top-y (`y - 1`); every run spans `FONT_H`. On-art status (Zork0's banner
/// ribbon) is an art strip, never a text run, so it is never passed here and stays
/// imaged.
pub fn clear_text_rows(canvas: &mut RgbaImage, run_tops: &[u16]) {
    let (w, h) = (canvas.width(), canvas.height());
    for &top in run_tops {
        let y0 = top as u32;
        let y1 = (y0 + FONT_H).min(h);
        for y in y0..y1 {
            for x in 0..w {
                canvas.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
}

pub fn build_chrome_canvas(
    chrome: &[&PositionedWindow],
    native: (u16, u16),
    default_fg: Rgba<u8>,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
) -> RgbaImage {
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);

    // Pass 1 — Graphics entries.
    blit_chrome_graphics(&mut canvas, chrome);
    // The ART layer, frozen (SQ-0727). Every "is this run sitting on artwork?"
    // question below (SQ-0487's per-run block rule, SQ-0499's gap fill) is asked
    // of THIS canvas, never of the live one: rasterized text is itself opaque, so
    // a live probe answers "yes, artwork" for a run whose span another run's own
    // highlight block already claimed — the lesson `build_graphics_canvas` records
    // for the hybrid side (SQ-0500).
    //
    // advent.z6's help screen is the case that needed it. Its navigation bar is a
    // pure reverse-video row painted as one run per label plus reversed spacer
    // spaces, and the spacer at x=289 lands INSIDE "About Adventure" (248..368).
    // The spacer draws first, so by the time the label's turn came the probe saw
    // the spacer's own white block, concluded the label sat on frame art, and drew
    // it as dark ink with no block — black ink that `flatten_onto_page` then
    // resolved onto a black page. The whole navigation bar was invisible in the
    // raster composite while rendering correctly as cells.
    let art = canvas.clone();

    // Pass 2 — Grid (status) entries, in list order. A v6 grid with
    // pixel-positioned runs draws those at their EXACT game pixel positions
    // (Zork Zero's banner text sits at rows 6/14, on the ribbon art — cell
    // quantization would snap it to the banner's top edge); the cell grid is
    // the fallback for grids without them.
    for it in chrome {
        if let WinNode::Grid(g) = &it.node {
            let ox = it.x_px as u32;
            let oy = it.y_px as u32;
            if !g.px_texts.is_empty() {
                // A packed colour is EXPLICIT only when the game named a real
                // colour (see `packed_explicit`): inherited colours + reverse
                // over frame art (Zork0's ribbon labels) must NOT paint an
                // opaque block — the original renders dark ink directly ON the
                // art. A block is painted only when the game chose colours.
                let explicit = packed_explicit;
                // Fill pure-reverse-row gaps FIRST, so the glyph loop paints the run
                // cells on top of them (SQ-0499). Both this fill and the glyph loop
                // put their over-art question to `art`, never to `canvas`.
                fill_reverse_row_gaps(&mut canvas, &art, &g.px_texts, default_fg, colors);
                // SQ-0519: then flood the full WINDOW width of each explicit-bg,
                // non-reverse row with its own background, so an explicitly-coloured
                // status band (Shogun's black-on-white location/score bar) reads as
                // one solid bar in the pixel composite rather than showing the page
                // in the gaps between its runs. Only when the window's width is
                // resolved (a size sentinel would balloon the flood, SQ-0481). The
                // glyph loop then stamps the runs on top.
                if (it.w_px as i16) >= 0 {
                    fill_explicit_bg_rows(&mut canvas, &g.px_texts, ox, it.w_px as u32, default_bg, colors);
                }
                for t in &g.px_texts {
                    let px0 = t.x.max(1) as u32 - 1;
                    let py = t.y.max(1) as u32 - 1;
                    let (fg, bg) = if t.style & 1 != 0 {
                        if explicit(t.fg) || explicit(t.bg) {
                            // Real colour pair: swap and paint the block.
                            (packed_to_rgba(t.bg, default_bg, colors), Some(packed_to_rgba(t.fg, default_fg, colors)))
                        } else {
                            // Inherited colours + reverse: whether to paint a block
                            // depends on what's BEHIND the run (SQ-0487). Over opaque
                            // frame art (Zork0's ribbon labels) a block would erase the
                            // art, so draw dark ink (default_bg) directly on it, no
                            // block. Over a CLEAR background (Shogun's boot-menu
                            // selection bar — no art behind it) the highlight must be
                            // visible, so paint the swapped block: a solid default_fg
                            // bar with default_bg ink, INCLUDING the blank gap runs the
                            // game paints between the item's words (a reversed space
                            // then fills its cell — no more moth-eaten bar). `art` is
                            // pass 1 frozen, so this sees the real artwork (or
                            // transparency) and never another run's own block.
                            let span_w = t.text.chars().count().max(1) as u32 * FONT_W;
                            if region_has_opaque(&art, px0, py, span_w, FONT_H) {
                                (default_bg, None)
                            } else {
                                (default_bg, Some(default_fg))
                            }
                        }
                    } else {
                        (
                            packed_to_rgba(t.fg, default_fg, colors),
                            explicit(t.bg).then(|| packed_to_rgba(t.bg, default_bg, colors)),
                        )
                    };
                    // Run coords are SCREEN-absolute 1-based pixels stamped at
                    // paint time (v6 paint semantics) — no window-origin
                    // offset: the window may have moved/shrunk since (Shogun
                    // turns its menu window into a 1-px caret after printing).
                    // The run's own §8.7.1 style byte rides along: the raster
                    // font synthesizes bold/italic (SQ-0540). Reverse (bit 1) is
                    // already resolved into the fg/bg pair above and fixed-pitch
                    // (bit 8) is a no-op in a bitmap font, so `blit_glyph_styled`
                    // ignores both — passing the raw byte can't double-apply.
                    for (i, ch) in t.text.chars().enumerate() {
                        let px = px0 + i as u32 * FONT_W;
                        crate::render::bitfont::blit_glyph_styled(&mut canvas, ch, px, py, FONT_W, FONT_H, fg, bg, t.style);
                    }
                }
                continue;
            }
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let idx = row as usize * g.cols as usize + col as usize;
                    let Some(cell) = g.cells.get(idx) else { continue };
                    let px = ox + col as u32 * FONT_W;
                    let py = oy + row as u32 * FONT_H;
                    if cell.ch == '\0' || cell.ch == ' ' {
                        if cell.bg != 0 {
                            let b = packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors);
                            fill_cell(&mut canvas, px, py, FONT_W, FONT_H, b);
                        }
                        continue;
                    }
                    let fg = packed_to_rgba(cell.fg, default_fg, colors);
                    let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors));
                    crate::render::bitfont::blit_glyph_styled(&mut canvas, cell.ch, px, py, FONT_W, FONT_H, fg, cellbg, cell.style);
                }
            }
        }
    }

    canvas
}

/// Draw every SECONDARY PROSE window's lines onto the pixel composite (SQ-0729).
///
/// A v6 game's second flowing-text window is published as a non-primary `Buffer`
/// (SQ-0585), and [`build_chrome_canvas`] draws Graphics and Grid windows and
/// nothing else — so every line such a window carried was absent from the raster
/// screen while both cell paths showed it. fmvpoker is the report: it prints its
/// menu bar and "Select an option with your mouse or by typing the first letter."
/// into one, and the composite showed neither. It matters more since the same
/// quest routed fmvpoker's hybrid frames here.
///
/// Separate from `build_chrome_canvas` because the ink is `honor_game_colours`-
/// gated and that function is not: `ink` is the caller's already-resolved page ink
/// (the game's own where honored, else the host's), and the window's OWN colour is
/// consulted only when the player is honoring game colours. Painting fmvpoker's
/// declared black regardless put black glyphs on the host's black page.
///
/// Placement — and therefore what [`fill_story_page_under_chrome_text`] must spare
/// — is [`buffer_line_rects`].
pub fn draw_secondary_prose(
    canvas: &mut RgbaImage,
    chrome: &[&PositionedWindow],
    ink: Rgba<u8>,
    honor: bool,
    colors: &ColorScheme,
) {
    for it in chrome {
        let WinNode::Buffer(b) = &it.node else { continue };
        let fg = match b.fg.filter(|_| honor) {
            Some(p) => packed_to_rgba(p, ink, colors),
            None => ink,
        };
        let right = it.x_px as u32 + it.w_px as u32;
        for (line, (x0, y0, _, _)) in b.lines.iter().zip(buffer_line_rects(it)) {
            for (i, ch) in line.chars().enumerate() {
                let px = x0 + i as u32 * FONT_W;
                if px + FONT_W > right {
                    break;
                }
                crate::render::bitfont::blit_glyph(canvas, ch, px, y0, FONT_W, FONT_H, fg, None);
            }
        }
    }
}

/// Where a SECONDARY prose window's lines land on the pixel composite (SQ-0729),
/// one `(x0, y0, x1, y1)` per line it carries, in the order of `lines`.
///
/// A `Buffer` is flowing prose with no pixel runs to place, so its lines stack from
/// the window's origin (plus the game's own left margin), one 16px text row each,
/// and stop at the bottom of the box the game declared — which is where the cell
/// paths put them too. Shared by the draw in [`build_chrome_canvas`] and by
/// [`chrome_text_rects`], whose caller must spare exactly the pixels the draw
/// claims; measuring them twice is how Shogun's menu got erased once already.
///
/// A PRIMARY buffer is the transcript and is not drawn here at all — it yields
/// nothing.
fn buffer_line_rects(it: &PositionedWindow) -> Vec<(u32, u32, u32, u32)> {
    let WinNode::Buffer(b) = &it.node else { return Vec::new() };
    if b.primary {
        return Vec::new();
    }
    let x0 = it.x_px as u32 + it.left_margin as u32;
    let bottom = it.y_px as u32 + it.h_px as u32;
    let right = it.x_px as u32 + it.w_px as u32;
    let mut out = Vec::new();
    for (row, line) in b.lines.iter().enumerate() {
        let y0 = it.y_px as u32 + row as u32 * FONT_H;
        if y0 + FONT_H > bottom {
            break;
        }
        let x1 = (x0 + line.chars().count() as u32 * FONT_W).min(right);
        out.push((x0, y0, x1, y0 + FONT_H));
    }
    out
}

/// A uniform (aspect-preserving) letterbox scale from native game pixels to
/// pane device pixels, plus the device-pixel offset of the letterboxed area.
pub struct Scale {
    pub s: f32,
    pub off_x: u32,
    pub off_y: u32,
}

/// Compute the uniform letterbox scale that fits `native` game-pixel
/// dimensions into `pane_dev` device-pixel dimensions, centering the result.
pub fn uniform_scale(native: (u16, u16), pane_dev: (u32, u32)) -> Scale {
    let nw = if native.0 == 0 { 1 } else { native.0 as u32 } as f32;
    let nh = if native.1 == 0 { 1 } else { native.1 as u32 } as f32;
    let s = (pane_dev.0 as f32 / nw).min(pane_dev.1 as f32 / nh);
    let scaled_w = nw * s;
    let scaled_h = nh * s;
    let off_x = ((pane_dev.0 as f32 - scaled_w) / 2.0).max(0.0) as u32;
    let off_y = ((pane_dev.1 as f32 - scaled_h) / 2.0).max(0.0) as u32;
    Scale { s, off_x, off_y }
}

/// The story window's clear-interior rect in NATIVE game pixels: its native rect
/// inset (interleaved per-edge) until no edge overlaps an opaque chrome pixel.
/// `None` when there is no story window. May be zero-size if fully occluded.
///
/// Inset one native pixel at a time per edge, banner first then columns, but
/// *interleaved* round by round (rather than each edge run to completion before
/// the next starts): a story window can overlap chrome on both axes at once
/// (e.g. a banner AND side columns), and letting the top/bottom scan run to
/// completion against the still-full width would never see a "clear" row while
/// side-band columns persist down the whole height. Shrinking left/right a step
/// at a time alongside top/bottom lets each edge's scan range narrow in
/// lockstep, converging on the true clear interior.
pub fn story_clear_native(
    story: Option<&PositionedWindow>,
    chrome_canvas: &RgbaImage,
) -> Option<(u32, u32, u32, u32)> {
    let story = story?;
    let (cw, ch) = chrome_canvas.dimensions();
    let opaque = |x: u32, y: u32| -> bool { x < cw && y < ch && chrome_canvas.get_pixel(x, y)[3] >= 128 };
    let mut left = story.x_px as u32;
    let mut top = story.y_px as u32;
    let mut right = (story.x_px as u32 + story.w_px as u32).min(cw);
    let mut bottom = (story.y_px as u32 + story.h_px as u32).min(ch);
    loop {
        let mut changed = false;
        if top < bottom && (left..right).any(|x| opaque(x, top)) {
            top += 1;
            changed = true;
        }
        if bottom > top && (left..right).any(|x| opaque(x, bottom - 1)) {
            bottom -= 1;
            changed = true;
        }
        if left < right && (top..bottom).any(|y| opaque(left, y)) {
            left += 1;
            changed = true;
        }
        if right > left && (top..bottom).any(|y| opaque(right - 1, y)) {
            right -= 1;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    Some((left, top, right.saturating_sub(left), bottom.saturating_sub(top)))
}

/// The cell rect (relative to the pane's top-left cell) where story text
/// goes: the largest cell-aligned rect inside the story window's device rect
/// that touches no opaque chrome pixel. Falls back to the full pane when
/// there is no story window.
pub fn story_viewport(
    story: Option<&PositionedWindow>,
    chrome_canvas: &image::RgbaImage,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    let Some((left, top, w, h)) = story_clear_native(story, chrome_canvas) else {
        return ratatui::layout::Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let (right, bottom) = (left + w, top + h);

    let dev_left = scale.off_x as f32 + left as f32 * scale.s;
    let dev_top = scale.off_y as f32 + top as f32 * scale.s;
    let dev_right = scale.off_x as f32 + right as f32 * scale.s;
    let dev_bottom = scale.off_y as f32 + bottom as f32 * scale.s;

    let cw_px = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let ch_px = if cell_px.1 == 0 { 1 } else { cell_px.1 } as f32;

    let cell_left = (dev_left / cw_px).ceil() as u16;
    let cell_top = (dev_top / ch_px).ceil() as u16;
    let cell_right = (dev_right / cw_px).floor() as u16;
    let cell_bottom = (dev_bottom / ch_px).floor() as u16;

    let width = cell_right.saturating_sub(cell_left).max(1);
    let height = cell_bottom.saturating_sub(cell_top).max(1);

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// The story viewport cell rect (relative to the pane's top-left cell) for the
/// HYBRID render mode: the win0 box (`story` x_px/y_px/w_px/h_px, native game
/// pixels) mapped through the letterbox [`Scale`] to device pixels, then quantized
/// to whole cells rounding INWARD (ceil the top-left, floor the bottom-right) so
/// no surrounding chrome cell overlaps the terminal story region. Unlike
/// [`story_viewport`], this does NOT inset around opaque chrome pixels — the raw
/// window box is the viewport, and the chrome ring is drawn around it. Falls back
/// to the full pane when there is no story window.
pub fn story_viewport_box(
    story: Option<&PositionedWindow>,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    let Some(story) = story else {
        return ratatui::layout::Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let left = story.x_px as f32;
    let top = story.y_px as f32;
    let right = (story.x_px as u32 + story.w_px as u32) as f32;
    let bottom = (story.y_px as u32 + story.h_px as u32) as f32;

    let dev_left = scale.off_x as f32 + left * scale.s;
    let dev_top = scale.off_y as f32 + top * scale.s;
    let dev_right = scale.off_x as f32 + right * scale.s;
    let dev_bottom = scale.off_y as f32 + bottom * scale.s;

    let cw_px = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let ch_px = if cell_px.1 == 0 { 1 } else { cell_px.1 } as f32;

    // Round INWARD: ceil the top-left, floor the bottom-right, so the viewport is
    // the largest whole-cell rect fully inside the win0 box.
    let cell_left = (dev_left / cw_px).ceil() as u16;
    let cell_top = (dev_top / ch_px).ceil() as u16;
    let cell_right = (dev_right / cw_px).floor() as u16;
    let cell_bottom = (dev_bottom / ch_px).floor() as u16;

    let width = cell_right.saturating_sub(cell_left).max(1);
    let height = cell_bottom.saturating_sub(cell_top).max(1);

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// The chrome RING cell rects around a story `viewport` inside a `pane`: up to
/// four non-overlapping rects (top, bottom, left, right) that exactly tile
/// `pane − viewport`. The top and bottom bands span the pane's full width (and so
/// own the corners); the left and right bands span only the viewport's vertical
/// extent. An edge-flush viewport omits that side's band; `viewport == pane`
/// yields an empty list. `viewport` is assumed to lie within `pane`; it is clamped
/// defensively. Both rects share one coordinate space (both absolute, or both
/// pane-relative).
pub fn chrome_bands(pane: ratatui::layout::Rect, viewport: ratatui::layout::Rect) -> Vec<ratatui::layout::Rect> {
    use ratatui::layout::Rect;
    // Clamp the viewport within the pane so the band arithmetic can't underflow.
    let vx = viewport.x.clamp(pane.x, pane.right());
    let vy = viewport.y.clamp(pane.y, pane.bottom());
    let vr = viewport.right().clamp(vx, pane.right());
    let vb = viewport.bottom().clamp(vy, pane.bottom());

    let mut out = vec![
        // Top band: full pane width, from the pane top down to the viewport top.
        Rect::new(pane.x, pane.y, pane.width, vy - pane.y),
        // Bottom band: full pane width, from the viewport bottom to the pane bottom.
        Rect::new(pane.x, vb, pane.width, pane.bottom() - vb),
        // Left band: the viewport's vertical span, from the pane left to the viewport left.
        Rect::new(pane.x, vy, vx - pane.x, vb - vy),
        // Right band: the viewport's vertical span, from the viewport right to the pane right.
        Rect::new(vr, vy, pane.right() - vr, vb - vy),
    ];
    out.retain(|r| r.width > 0 && r.height > 0);
    out
}

/// Rasterize `main`'s wrapped lines (then the input line + block cursor when
/// `main.awaiting`) into `canvas` starting at native px `(ox, oy)`, one glyph per
/// FONT×FONT cell, transparent glyph bg (draws over chrome/background art).
/// Clipped to `rows` lines and `cols` columns.
pub fn draw_story_text(canvas: &mut RgbaImage, main: &MainText, ox: u32, oy: u32, cols: u16, rows: u16, fg: Rgba<u8>) {
    let region_h = rows as u32 * FONT_H;
    // Floats first (text draws over/beside them). A float that has partially
    // scrolled off the top (row < 0) is drawn cropped from its own top. Blitted
    // at `img_col` (0 = left float; near the right edge = right float), clamped
    // to the columns from there to the region's right edge.
    for f in &main.floats {
        let src = &*f.img;
        let crop_top = if f.row < 0 { (-f.row) as u32 * FONT_H } else { 0 };
        if crop_top >= src.height() {
            continue;
        }
        let dy = oy + (f.row.max(0) as u32) * FONT_H;
        let max_h = region_h.saturating_sub(dy - oy);
        let img_x = ox + f.img_col as u32 * FONT_W;
        let max_w = (cols as u32).saturating_sub(f.img_col as u32) * FONT_W;
        blit_clipped_src(canvas, src, img_x, dy, crop_top, max_w, max_h);
    }
    // The active float's (reserved cols, text start col) for a given row — one
    // float is active at a time; when several overlap take the widest reserve.
    let float_at = |row: u32| -> (u32, u32) {
        main.floats
            .iter()
            .filter(|f| f.row <= row as i32 && (row as i32) < f.row + f.rows as i32)
            .map(|f| (f.reserve_cols as u32, f.text_col as u32))
            .max_by_key(|(reserve, _)| *reserve)
            .unwrap_or((0, 0))
    };
    let mut row = 0u32;
    let mut last_row_end = 0u32; // (text_col + text len) of the last drawn line
    for line in &main.lines {
        if row >= rows as u32 {
            return;
        }
        let (reserve, text_col) = float_at(row);
        let avail = (cols as u32).saturating_sub(reserve);
        let mut drawn = 0u32;
        // Per-char emphasis for this row (SQ-0540): the raster font synthesizes
        // bold/italic, so a game's emphasised prose (Zork Zero's bold room
        // names, Shogun's italic "Erasmus") reads as emphasis here too. A row
        // with no `styles` entry — or a char past its end — is roman.
        let row_styles = main.styles.get(row as usize);
        for (col, glyph) in line.chars().take(avail as usize).enumerate() {
            let style = row_styles.and_then(|s| s.get(col)).copied().unwrap_or(0);
            crate::render::bitfont::blit_glyph_styled(canvas, glyph, ox + (text_col + col as u32) * FONT_W, oy + row * FONT_H, FONT_W, FONT_H, fg, None, style);
            drawn = col as u32 + 1;
        }
        last_row_end = text_col + drawn;
        row += 1;
    }
    if main.awaiting {
        // The live input continues the game's kept prompt line (the last drawn
        // row — Zork Zero's "…HINT): >"), NOT a fresh row below it (SQ-0470a):
        // the caret sits right after the prompt. When the transcript ended on a
        // newline the last line is empty (`last_row_end == 0`) so the input
        // starts a clean row of its own, matching the terminal inline prompt.
        let input_row = row.saturating_sub(1);
        let start = last_row_end;
        if input_row < rows as u32 {
            for (i, glyph) in main.input.chars().enumerate() {
                let col = start + i as u32;
                if col >= cols as u32 {
                    break;
                }
                crate::render::bitfont::blit_glyph(canvas, glyph, ox + col * FONT_W, oy + input_row * FONT_H, FONT_W, FONT_H, fg, None);
            }
            let caret = (start + main.cursor_col as u32).min(cols.saturating_sub(1) as u32);
            fill_cell(canvas, ox + caret * FONT_W, oy + input_row * FONT_H, FONT_W, FONT_H, fg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{BorderPref, BufferWindow, GraphicsWindow, GridCell, GridWindow, PxText};
    use std::sync::Arc;

    fn grid_item(x_px: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        }
    }

    fn graphics_item(x_px: u16) -> PositionedWindow {
        graphics_item_win(x_px, 7)
    }

    fn graphics_item_win(x_px: u16, win: u32) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::new(1, 1));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win, canvas, version: 0, upscale: false }),
        }
    }

    fn buffer_item(x_px: u16, primary: bool) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary, ..Default::default() }),
        }
    }

    /// A window-0 plate `w`×`h` painted opaque at native `(x, y)`.
    fn plate_at(x: u16, y: u16, w: u32, h: u32) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255])));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: x, y_px: y, w_px: w as u16, h_px: h as u16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        }
    }

    // ── story_prose_box (SQ-0707) ────────────────────────────────────────────

    #[test]
    fn story_prose_box_without_a_plate_is_the_whole_clear_interior() {
        assert_eq!(story_prose_box((0, 0, 640, 400), None), Some((0, 0, 640, 400)));
    }

    /// Arthur's real geometry: a 584×392 plate centred in window 0's 640×400 box
    /// leaves 28px side margins (3 cells — below the 8-column floor) and 4px
    /// top/bottom (under one 16px line). Nothing survives, so the plate owns the
    /// screen and no prose is drawn. This is the SQ-0707 symptom in one line.
    #[test]
    fn story_prose_box_yields_the_screen_to_a_centred_full_bleed_plate() {
        let plate = plate_at(28, 4, 584, 392);
        assert_eq!(
            story_prose_box((0, 0, 640, 400), Some(&plate)),
            None,
            "a plate leaving only a 3-cell side margin is not a prose box — the picture owns \
             the screen exactly as a window-filling one does (SQ-0578)"
        );
    }

    /// Graceful degradation: a plate that leaves a genuine column still gets
    /// prose beside it, in the widest strip it left. A 240px-wide plate down the
    /// left of a 640px box leaves 400px (50 cells) on the right.
    #[test]
    fn story_prose_box_keeps_the_column_a_margin_illustration_leaves() {
        let plate = plate_at(0, 0, 240, 400);
        assert_eq!(
            story_prose_box((0, 0, 640, 400), Some(&plate)),
            Some((240, 0, 400, 400)),
            "prose wraps in the column beside a margin illustration"
        );
    }

    /// A plate wholly outside the story's clear interior changes nothing.
    #[test]
    fn story_prose_box_ignores_a_plate_that_misses_the_text_box() {
        let plate = plate_at(0, 0, 100, 100);
        assert_eq!(story_prose_box((200, 200, 400, 200), Some(&plate)), Some((200, 200, 400, 200)));
    }

    /// Only PAINTED pixels count: a plate whose canvas is fully transparent (a
    /// window-0 graphics leaf that has drawn nothing yet) never takes the screen.
    #[test]
    fn story_prose_box_ignores_an_unpainted_plate() {
        let canvas = Arc::new(image::RgbaImage::new(584, 392));
        let plate = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 28, y_px: 4, w_px: 584, h_px: 392,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        };
        assert_eq!(story_prose_box((0, 0, 640, 400), Some(&plate)), Some((0, 0, 640, 400)));
    }

    #[test]
    fn story_is_the_primary_buffer_and_chrome_preserves_order() {
        let items = vec![graphics_item(1), grid_item(2), buffer_item(3, true)];
        let layout = classify_windows(&items);
        let story = layout.story.expect("primary buffer found");
        assert!(matches!(&story.node, WinNode::Buffer(b) if b.primary));
        assert_eq!(story.x_px, 3);
        assert_eq!(layout.chrome.len(), 2);
        assert_eq!(layout.chrome[0].x_px, 1);
        assert_eq!(layout.chrome[1].x_px, 2);
    }

    #[test]
    fn no_primary_buffer_means_no_story_and_all_chrome() {
        let items = vec![grid_item(1), graphics_item(2), buffer_item(3, false)];
        let layout = classify_windows(&items);
        assert!(layout.story.is_none());
        assert!(layout.story_gfx.is_none());
        assert_eq!(layout.chrome.len(), items.len());
    }

    #[test]
    fn window_zero_graphics_is_story_content_not_chrome() {
        // The primary window's own picture (window 0) is the room illustration —
        // story content, kept out of chrome so it renders inside the story region.
        let items = vec![
            graphics_item_win(1, 0), // window 0's illustration
            graphics_item_win(2, 7), // window 7 frame → chrome
            buffer_item(3, true),    // story
        ];
        let layout = classify_windows(&items);
        assert_eq!(layout.story.expect("story").x_px, 3);
        assert_eq!(layout.story_gfx.expect("story_gfx").x_px, 1);
        assert_eq!(layout.chrome.len(), 1, "only window 7 graphics is chrome");
        assert_eq!(layout.chrome[0].x_px, 2);
    }

    fn colors() -> ColorScheme {
        ColorScheme::default()
    }

    #[test]
    fn story_text_wraps_right_of_float_and_blits_it() {
        // Rows covered by a float are inset by its indent (text flows beside the
        // picture); rows past it are flush left; the float's pixels are blitted
        // at its anchored row.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // A 16×32 opaque red image → float of 2 rows (32px / FONT_H(16) = 2).
        let img = RgbaImage::from_pixel(16, 32, Rgba([200, 20, 20, 255]));
        let main = MainText {
            lines: vec!["AAAA".into(), "BBBB".into(), "CCCC".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 3, text_col: 3, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]));
        // Rows 0-1 (beside float): glyph ink starts at column 3.
        assert!(cell_has_ink(&canvas, 0, 0), "float pixels occupy row 0 col 0");
        assert_eq!(*canvas.get_pixel(4, 20), Rgba([200, 20, 20, 255]), "float blitted at its row (spans y 0..32)");
        assert!(cell_has_ink(&canvas, 3, 0), "row 0 col 3 inked (text beside the float)");
        assert!(cell_has_ink(&canvas, 3, 1), "row 1 col 3 inked (text beside the float)");
        // Row 2 (past the float): ink flush left.
        assert!(cell_has_ink(&canvas, 0, 2), "row 2 col 0 inked (flush left below float)");
    }

    #[test]
    fn story_text_wraps_left_of_right_float_and_blits_it_right() {
        // A RIGHT float (Shogun's opening picture): text stays flush LEFT and is
        // narrowed to `cols - reserve_cols`; the picture blits at `img_col` near
        // the right edge; rows past the picture reclaim full width.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // 10-col region; a 32×32 image → 4 cols wide, 2 rows tall; reserve 5 cols
        // (image + gutter), text confined to cols 0..5, image blits at col 6.
        let img = RgbaImage::from_pixel(32, 32, Rgba([20, 200, 20, 255]));
        let main = MainText {
            lines: vec!["AAAAAAAA".into(), "BBBB".into(), "CCCCCCCC".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 5, text_col: 0, img_col: 6, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]));
        // Row 0 text is flush left but clipped to the narrowed column (cols 0..5).
        assert!(cell_has_ink(&canvas, 0, 0), "row 0 col 0 inked (text flush left)");
        assert!(!cell_has_ink(&canvas, 5, 0), "row 0 col 5 blank (text narrowed away from the picture)");
        // The picture blits at col 6 (img_col), on the right.
        assert_eq!(*canvas.get_pixel(6 * FONT_W, 0), Rgba([20, 200, 20, 255]), "float blitted at img_col 6");
        // Row 2 (past the float) reclaims full width.
        assert!(cell_has_ink(&canvas, 6, 2), "row 2 col 6 inked (full width below the float)");
    }

    #[test]
    fn packed_standard_palette_colour_blits_its_own_rgb_not_default() {
        // SQ-0480/SQ-0506: a run coloured with a Standard palette colour (the
        // compass letters) must blit in that colour, not the default ink. On the
        // PIXEL path, Standard 2..=9 resolve to the ZMSD §8.3.1 true-colour RGB
        // (DOS/spec-authentic) rather than the theme's dim VGA ANSI values — so
        // red is the spec red $001D → (239,0,0), NOT the old VGA base-red
        // (170,0,0). White(9) likewise becomes real white (255,255,255).
        let colors = ColorScheme::terminal_default();
        let fallback = Rgba([1, 2, 3, 255]);
        // Standard(3): packed tag 1, value 3 (see state::pack_zcolour).
        let packed_std3 = (1u32 << 24) | 3;
        let got = packed_to_rgba(packed_std3, fallback, &colors);
        assert_ne!(got, fallback, "a palette colour must NOT fall back to the default ink");
        assert_eq!(got, Rgba([239, 0, 0, 255]), "Standard(3) → spec red $001D on the pixel path");
        // Standard(9) white must be TRUE white, not the VGA base-grey it used to be.
        let packed_std9 = (1u32 << 24) | 9;
        assert_eq!(
            packed_to_rgba(packed_std9, fallback, &colors),
            Rgba([255, 255, 255, 255]),
            "Standard(9) → true white 255,255,255 (ZMSD $7FFF), not VGA grey 170,170,170"
        );
        // Standard(2) black stays black.
        assert_eq!(
            packed_to_rgba((1u32 << 24) | 2, fallback, &colors),
            Rgba([0, 0, 0, 255]),
            "Standard(2) → black 0,0,0 (ZMSD $0000)"
        );
        // And the full blit through build_chrome_canvas carries it: a space-only
        // run has no ink, so probe an inked glyph's fg by asserting SOME cell pixel
        // is the run's red.
        let win = px_text_grid_item("N", 0, packed_std3, 0);
        let c = build_chrome_canvas(&[&win], (8, 8), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors);
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == Rgba([239, 0, 0, 255]))),
            "the compass glyph blits in its own spec red, not the default fg"
        );
    }

    #[test]
    fn native_extent_ignores_unresolved_size_sentinel() {
        // SQ-0481: a real 320×200 window plus a bogus window whose x_size leaked
        // the -2 sentinel (0xFFFE ≈ 65534). The sentinel must NOT balloon the
        // native extent (and thus the raster canvas allocation) — the real
        // 320×200 screen size stands.
        let real = || PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 200, ..buffer_item(0, true) };
        let bogus = PositionedWindow { x_px: 0, y_px: 0, w_px: 0xFFFE, h_px: 200, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus]), (320, 200), "sentinel width excluded");
        // A sentinel HEIGHT is likewise ignored on its axis.
        let bogus_h = PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 0xFFFD, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus_h]), (320, 200), "sentinel height excluded");
    }

    #[test]
    fn story_text_input_continues_the_prompt_row() {
        // SQ-0470a: the live input sits on the game's kept ">" prompt row,
        // appended right after it — NOT a fresh row below it.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Room desc.".into(), ">".into()],
            styles: Vec::new(),
            input: "go".into(),
            cursor_col: 2,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]));
        // ">" is on row 1; input "go" appends after it at cols 1 and 2.
        assert!(cell_has_ink(&canvas, 1, 1), "input 'g' on the prompt row, after '>'");
        assert!(cell_has_ink(&canvas, 2, 1), "input 'o' on the prompt row");
        // Caret block after the input: col = 1 (\">\".len) + 2 (cursor) = 3.
        assert!(cell_has_ink(&canvas, 3, 1), "caret after the input on the prompt row");
        // The row BELOW the prompt is empty — input no longer drops a row.
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "nothing on the row below the prompt");
    }

    #[test]
    fn story_text_input_after_newline_starts_a_clean_row() {
        // When the transcript ended on a newline the last line is empty, so the
        // input starts a clean row of its own (col 0) — the universal rule that
        // makes SQ-0470a correct for both prompt and non-prompt endings.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Prose line.".into(), String::new()],
            styles: Vec::new(),
            input: "x".into(),
            cursor_col: 1,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]));
        assert!(cell_has_ink(&canvas, 0, 1), "input on the empty last row at col 0");
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "not the row below");
    }

    #[test]
    fn story_text_scrolled_float_is_cropped_not_pinned() {
        // A float whose anchor scrolled above the view (row = -1) draws only its
        // remaining rows, cropped from its own top (one FONT_H = 16px row).
        let mut img = RgbaImage::new(8, 32);
        for y in 0..32 {
            // Top row (y<16) green, bottom row (y>=16) blue — the visible part,
            // after cropping the scrolled-off top FONT_H row, must be blue.
            let c = if y < 16 { Rgba([0, 200, 0, 255]) } else { Rgba([0, 0, 200, 255]) };
            for x in 0..8 { img.put_pixel(x, y, c); }
        }
        let main = MainText {
            lines: vec!["XXXX".into()],
            styles: Vec::new(),
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: -1, rows: 2, reserve_cols: 2, text_col: 2, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 3 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 3, Rgba([255, 255, 255, 255]));
        assert_eq!(*canvas.get_pixel(4, 4), Rgba([0, 0, 200, 255]), "visible slice is the float's BOTTOM half");
    }

    #[test]
    fn chrome_graphics_blits_native_and_clips_to_window_box() {
        // The window canvas is authored in native game pixels: build_chrome_canvas
        // blits it 1:1 at the window origin (never scaled to the declared box) and
        // clips at the box edge (ZMSD §8: plotting is always clipped to the window).
        let mut src = image::RgbaImage::new(48, 43);
        src.put_pixel(40, 38, Rgba([10, 200, 30, 255])); // marker low in the canvas
        src.put_pixel(2, 2, Rgba([200, 10, 30, 255])); // marker near the top-left
        let win = |h_px: u16, canvas: image::RgbaImage| PositionedWindow {
            x: 0, y: 0, w: 40, h: 1,
            x_px: 4, y_px: 4, // window origin
            w_px: 320, h_px,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow {
                win: 1, canvas: Arc::new(canvas), version: 0, upscale: false,
            }),
        };
        // Box tall enough (40): both markers land 1:1 — never squashed.
        let tall = win(40, src.clone());
        let canvas = build_chrome_canvas(&[&tall], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker at native (6,6)");
        assert_eq!(canvas.get_pixel(44, 42)[3], 255, "low marker 1:1 at native (44,42)");
        // Box only 5 tall: content past the box clips; nothing squashes into it.
        let short = win(5, src);
        let canvas = build_chrome_canvas(&[&short], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker inside the box survives");
        assert_eq!(canvas.get_pixel(44, 42)[3], 0, "content below the 5px box is clipped");
        for y in 4..9 {
            assert_eq!(canvas.get_pixel(44, y)[3], 0, "no squashed copy inside the box (y={y})");
        }
    }

    fn graphics_window(x_px: u16, y_px: u16, w: u16, h: u16, canvas: image::RgbaImage) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w, h, x_px, y_px, w_px: w, h_px: h, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas: Arc::new(canvas), version: 0, upscale: false }),
        }
    }

    #[test]
    fn frame_opaque_border_transparent_interior_and_outside_stays_transparent() {
        // 20x20 native canvas, one chrome Graphics window covering it whose
        // source canvas has an opaque 1px border ring and a transparent
        // center. The built chrome canvas should mirror that: opaque at the
        // border, transparent at the center, and transparent outside the
        // window (there is none here, but the whole canvas is checked).
        let mut src = image::RgbaImage::new(20, 20);
        for x in 0..20u32 {
            src.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
            src.put_pixel(x, 19, Rgba([255, 255, 255, 255]));
        }
        for y in 0..20u32 {
            src.put_pixel(0, y, Rgba([255, 255, 255, 255]));
            src.put_pixel(19, y, Rgba([255, 255, 255, 255]));
        }
        let win = graphics_window(0, 0, 20, 20, src);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(c.get_pixel(0, 0)[3], 255, "border pixel is opaque");
        assert_eq!(c.get_pixel(10, 10)[3], 0, "center is transparent");
    }

    #[test]
    fn later_graphics_entry_draws_over_earlier_through_its_transparent_margin() {
        // Two overlapping chrome Graphics entries at the same native spot
        // (4,4), 8x8 each: "base" solid colour A, then "indicator" solid
        // colour B on its left half and transparent on its right half.
        // Later-drawn wins where opaque; the base shows through the
        // indicator's transparent right half.
        let color_a = Rgba([200, 0, 0, 255]);
        let color_b = Rgba([0, 200, 0, 255]);
        let base = image::RgbaImage::from_pixel(8, 8, color_a);
        let mut indicator = image::RgbaImage::new(8, 8);
        for y in 0..8u32 {
            for x in 0..4u32 {
                indicator.put_pixel(x, y, color_b);
            }
        }
        let base_win = graphics_window(4, 4, 8, 8, base);
        let indicator_win = graphics_window(4, 4, 8, 8, indicator);
        let chrome: Vec<&PositionedWindow> = vec![&base_win, &indicator_win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(*c.get_pixel(5, 8), color_b, "left half shows the indicator (last-drawn wins)");
        assert_eq!(*c.get_pixel(10, 8), color_a, "right half shows the base through the transparent margin");
    }

    #[test]
    fn status_grid_glyph_paints_fg_in_its_native_pixel_cell() {
        let mut cells = vec![GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }; 6];
        // row 1, col 2 in a 3-col grid.
        cells[3 + 2] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let win = PositionedWindow {
            x: 0, y: 0, w: 3, h: 2, x_px: 10, y_px: 4, w_px: 24, h_px: 32, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                fill: None,
                cols: 3, rows: 2, cells, active_rows: 2, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let fg = Rgba([0, 255, 255, 255]);
        let c = build_chrome_canvas(&chrome, (40, 40), fg, Rgba([0, 0, 0, 255]), &colors());
        // cell (col=2,row=1) native px box: x = 10 + 2·FONT_W(8) = 26..34,
        // y = 4 + 1·FONT_H(16) = 20..36 (non-square 8×16 cell, SQ-0479).
        assert!(
            (26..34).any(|x| (20..36).any(|y| *c.get_pixel(x, y) == fg)),
            "glyph fg pixels appear within the status cell's native box"
        );
    }

    // ── px_text colour + reverse-video (Lane C) ─────────────────────────────
    //
    // These probe the SOLID FILL colour behind a run, not individual glyph
    // pixels: a run whose text is a single space has no ink bits set, so its
    // whole FONT×FONT cell is exactly `blit_glyph`'s `bg` fill colour (or
    // fully transparent when `bg` is `None`) — a robust way to assert which
    // colour the resolver chose without depending on font-bitmap geometry.
    const RED: u32 = 0x03FF_0000; // True24 packed
    const BLUE: u32 = 0x0300_00FF; // True24 packed

    fn px_text_grid_item(text: &str, style: u8, fg: u32, bg: u32) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: vec![PxText { y: 1, x: 1, text: text.into(), style, fg, bg }],
            }),
        }
    }

    /// Lit-`fg` pixel coordinates of a rendered canvas — the shape the raster
    /// font actually drew, independent of where the cells sit.
    fn ink(c: &RgbaImage, fg: Rgba<u8>) -> std::collections::BTreeSet<(u32, u32)> {
        c.enumerate_pixels().filter(|(_, _, p)| **p == fg).map(|(x, y, _)| (x, y)).collect()
    }

    #[test]
    fn px_text_bold_run_double_strikes_the_raster_glyphs() {
        // SQ-0540: a painted run carrying style bit 2 (Journey stamps its command
        // menu labels — "Proceed", "Combat", "Cast" — exactly this way) renders
        // emboldened in the pixel composite, not roman.
        let fg = Rgba([255, 0, 0, 255]);
        let canvas = |style: u8| {
            let win = px_text_grid_item("Ab", style, RED, 0);
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (24, 16), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors())
        };
        let roman = ink(&canvas(0), fg);
        let bold = ink(&canvas(2), fg);
        assert_ne!(bold, roman, "a bold run must not render identically to a roman one");
        assert!(roman.is_subset(&bold), "bold keeps every roman pixel");
        for &(x, y) in bold.difference(&roman) {
            assert!(x > 0 && roman.contains(&(x - 1, y)), "bold pixel ({x},{y}) is not a +1 double-strike");
        }
        // Italic leans the top half; bold-italic is heavier still.
        let italic = ink(&canvas(4), fg);
        assert_ne!(italic, roman, "an italic run must not render roman");
        assert!(ink(&canvas(6), fg).len() > italic.len(), "bold-italic is heavier than italic");
    }

    #[test]
    fn px_text_reverse_only_run_keeps_the_roman_face() {
        // Zork Zero's banner/ribbon chrome is style-REVERSE with no emphasis: the
        // reverse bit is resolved into the fg/bg pair before the blit, so the
        // glyphs must be the same roman shapes a plain run with the swapped pair
        // draws — SQ-0540's faces must not touch it (nor may fixed-pitch, bit 8).
        let render = |style: u8, fg: u32, bg: u32| {
            let win = px_text_grid_item("Ab", style, fg, bg);
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (24, 16), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors())
        };
        let blue = Rgba([0, 0, 255, 255]);
        // Reversed: the run's fg becomes the block, its bg becomes the ink. The
        // INK pixels (blue) must be the same roman glyph shapes a plain blue-on-
        // transparent run draws. (The two canvases differ elsewhere — reverse
        // also floods the row gaps — so compare the ink, not the whole image.)
        let reversed = render(1, RED, BLUE);
        assert_eq!(ink(&reversed, blue), ink(&render(0, BLUE, 0), blue), "reverse ink keeps the roman face");
        assert_eq!(render(1 | 8, RED, BLUE), reversed, "fixed-pitch changes nothing in a bitmap font");
    }

    #[test]
    fn status_grid_cell_carries_bold() {
        // The cell-grid fallback path (no pixel-positioned runs) gets faces too:
        // a v6 game can `set_text_style` bold in any window.
        let cells = |style: u8| vec![GridCell { ch: 'A', style, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        let canvas = |style: u8| {
            let win = PositionedWindow {
                x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 16, left_margin: 0, right_margin: 0,
                node: WinNode::Grid(GridWindow {
                    fill: None,
                    cols: 1, rows: 1, cells: cells(style), active_rows: 1, cursor: (0, 0), cursor_active: false,
                    border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                    px_texts: Vec::new(),
                }),
            };
            let chrome: Vec<&PositionedWindow> = vec![&win];
            build_chrome_canvas(&chrome, (8, 16), Rgba([0, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors())
        };
        let fg = Rgba([0, 255, 255, 255]);
        let roman = ink(&canvas(0), fg);
        let bold = ink(&canvas(2), fg);
        assert!(roman.is_subset(&bold) && bold.len() > roman.len(), "a bold grid cell is emboldened");
    }

    #[test]
    fn story_text_applies_per_char_emphasis() {
        // The prose path (Zork Zero's bold room names, Shogun's italic "Erasmus")
        // takes per-char style bytes parallel to its lines; chars with no entry
        // stay roman, and emphasis never spills into the neighbouring cells.
        let fg = Rgba([255, 255, 255, 255]);
        let draw = |styles: Vec<Vec<u8>>| {
            let main = MainText { lines: vec!["AAAA".into()], styles, input: String::new(), cursor_col: 0, awaiting: false, floats: vec![] };
            let mut c = RgbaImage::new(6 * FONT_W, 2 * FONT_H);
            draw_story_text(&mut c, &main, 0, 0, 6, 2, fg);
            c
        };
        let roman = ink(&draw(Vec::new()), fg);
        // Bold only the two middle chars (cols 1..3).
        let mixed = ink(&draw(vec![vec![0, 2, 2, 0]]), fg);
        assert_ne!(mixed, roman, "an emphasised row must differ from the roman one");
        assert!(roman.is_subset(&mixed), "double-strike is additive");
        for &(x, y) in mixed.difference(&roman) {
            let col = x / FONT_W;
            assert!((1..3).contains(&col), "only the bold columns changed, got a new pixel in col {col} at ({x},{y})");
            assert!(roman.contains(&(x - 1, y)), "new pixel ({x},{y}) is a +1 double-strike");
        }
        // A short/absent style row is all-roman.
        assert_eq!(ink(&draw(vec![Vec::new()]), fg), roman, "an empty style row renders roman");
        assert_eq!(ink(&draw(vec![vec![0, 0]]), fg), roman, "a short style row's tail renders roman");
    }

    #[test]
    fn px_text_run_fills_its_cell_with_the_explicit_background() {
        let win = px_text_grid_item(" ", 0, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([0, 0, 255, 255]), "cell filled with the run's bg (blue) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_swaps_the_fill_to_the_foreground_colour() {
        // Same run as above but with style bit 1 (reverse) set: the swap makes
        // the run's FOREGROUND (red) the fill colour instead of its background.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([255, 0, 0, 255]), "reverse fill is the run's fg (red) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_inherited_over_art_draws_dark_ink_no_block() {
        // The run never chose an explicit colour (fg=bg=0/Default) and sits OVER
        // opaque frame art: reverse video must NOT paint a block — Zork0's ribbon
        // labels print in reverse with inherited colours and the original shows dark
        // ink directly ON the banner art (a block would erase it, the black-box
        // regression the user hit). A blank glyph therefore leaves the art
        // untouched; an inked glyph draws in default_bg (dark) on the art. (SQ-0487
        // keeps this by testing the canvas is opaque behind the run.)
        let default_fg = Rgba([10, 20, 30, 255]);
        let default_bg = Rgba([40, 50, 60, 255]);
        let art_color = Rgba([200, 150, 100, 255]);
        // An opaque 8×8 art window behind the run (pass 1), then the reverse run.
        let art = graphics_window(0, 0, 8, 8, image::RgbaImage::from_pixel(8, 8, art_color));
        let blank = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &blank];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert_eq!(*c.get_pixel(4, 4), art_color, "blank reverse glyph over art leaves the art (no block)");
        let inked = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &inked];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "reverse ink over art draws in the themed default_bg (dark on the art)"
        );
    }

    #[test]
    fn px_text_reverse_inherited_over_clear_bg_paints_the_highlight_block() {
        // SQ-0487: the same inherited-colour reverse run over a CLEAR background
        // (Shogun's boot-menu selection bar — no frame art behind it) MUST paint the
        // swapped highlight block: a solid default_fg bar with default_bg ink. A
        // blank gap run between words fills its whole cell with the bar colour, so
        // the selection bar reads solid (not moth-eaten).
        let default_fg = Rgba([210, 210, 210, 255]);
        let default_bg = Rgba([12, 12, 12, 255]);
        // A blank reverse run (an inter-word gap) over the transparent canvas fills
        // its cell with the bar colour (default_fg).
        let gap = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&gap];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), default_fg, "gap cell filled with the bar colour at ({x},{y})");
            }
        }
        // An inked reverse glyph paints the bar (default_fg) with dark (default_bg) ink.
        let glyph = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&glyph];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_fg)),
            "the highlight bar (default_fg) is painted behind the glyph"
        );
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "the glyph ink is drawn in default_bg (dark on the bright bar)"
        );
    }

    #[test]
    fn px_text_reverse_with_explicit_colours_paints_the_swapped_block() {
        // A run whose game explicitly chose colours DOES paint the swap block.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([1, 1, 1, 255]), Rgba([2, 2, 2, 255]), &colors());
        assert_eq!(c.get_pixel(4, 4)[3], 255, "explicit reverse paints an opaque block");
    }

    #[test]
    fn px_text_no_bg_stays_transparent_without_reverse() {
        // Regression guard: a run with no explicit bg (0/Default) and no
        // reverse style stays transparent — unchanged from before colour
        // handling existed, so frame art under status text still shows through.
        let win = px_text_grid_item(" ", 0, RED, 0);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(c.get_pixel(x, y)[3], 0, "no bg, no reverse ⇒ transparent at ({x},{y})");
            }
        }
    }

    // ── explicit-bg status-band flood (SQ-0519) ─────────────────────────────

    #[test]
    fn row_flood_bg_predicate_first_explicit_wins_and_skips_reverse() {
        // The window-wide flood predicate (raster twin of SQ-0512's hybrid per-row
        // flood): a NON-reverse row that names an explicit bg floods with it; a pure
        // reverse-video row and a row with no explicit bg do NOT (byte-identical).
        let colors = colors();
        let default_bg = Rgba([9, 9, 9, 255]);
        let z_black = (1u32 << 24) | 2; // Standard 2 (explicit)
        let z_white = (1u32 << 24) | 9; // Standard 9 (explicit) → spec white
        let run = |x: u16, style: u8, fg: u32, bg: u32| PxText { y: 1, x, text: "AB".into(), style, fg, bg };
        // (a) explicit-bg non-reverse row → floods the resolved white.
        let a = run(1, 0, z_black, z_white);
        let b = run(50, 0, z_black, z_white);
        assert_eq!(
            row_flood_bg(&[&a, &b], default_bg, &colors),
            Some(Rgba([255, 255, 255, 255])),
            "explicit-bg row floods z-colour 9 white"
        );
        // (b) pure reverse-video, non-explicit row (Zork0's on-art ribbon) → None:
        // fill_reverse_row_gaps owns it (with the over-art gate).
        let rev = run(1, 1, 0, 0);
        assert_eq!(row_flood_bg(&[&rev], default_bg, &colors), None, "reverse row: no window flood");
        // (c) mixed partial-explicit row → first-explicit-wins (the second run's white).
        let plain = run(1, 0, 0, 0);
        let white = run(50, 0, z_black, z_white);
        assert_eq!(
            row_flood_bg(&[&plain, &white], default_bg, &colors),
            Some(Rgba([255, 255, 255, 255])),
            "mixed row floods the first explicit bg"
        );
        // (d) explicit-FG-only, non-reverse row (Zork0's compass letters) → None: no
        // explicit bg means no opaque box painted over the banner art.
        let fg_only = run(1, 0, z_black, 0);
        assert_eq!(row_flood_bg(&[&fg_only], default_bg, &colors), None, "explicit-fg-only row: no window flood");
    }

    fn band_grid(w_px: u16, runs: Vec<PxText>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: (w_px / 8).max(1), h: 1, x_px: 0, y_px: 0, w_px, h_px: 16, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                fill: None,
                cols: (w_px / 8).max(1), rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false, px_texts: runs,
            }),
        }
    }

    #[test]
    fn explicit_bg_status_row_floods_the_whole_window_width() {
        // SQ-0519: two explicit black-on-white runs with a bare gap between them —
        // the gap (and the whole window width) floods the explicit white, so the band
        // reads as one solid bar rather than showing the page between the runs.
        let z_black = (1u32 << 24) | 2;
        let z_white = (1u32 << 24) | 9;
        let win = band_grid(64, vec![
            PxText { y: 1, x: 1, text: "AB".into(), style: 0, fg: z_black, bg: z_white },
            PxText { y: 1, x: 41, text: "CD".into(), style: 0, fg: z_black, bg: z_white },
        ]);
        let c = build_chrome_canvas(&[&win], (64, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        // px 24 is a gap between run A (px 0..16) and run C (px 40..): flooded white.
        assert_eq!(*c.get_pixel(24, 8), Rgba([255, 255, 255, 255]), "the inter-run gap floods the explicit white");
        // The window's far edge is flooded too — the whole window width is one bar.
        assert_eq!(*c.get_pixel(60, 8), Rgba([255, 255, 255, 255]), "the flood spans the full window width");
    }

    #[test]
    fn explicit_fg_only_run_over_art_is_not_flooded() {
        // SQ-0519 byte-identity guard: Zork0's compass letters are explicit-FG-only,
        // non-reverse, ON opaque banner art. With no explicit bg the flood must NOT
        // fire — an art pixel beside the letter keeps its value (no black box).
        let z_red = (1u32 << 24) | 3;
        let art_color = Rgba([180, 140, 90, 255]);
        let art = graphics_window(0, 0, 16, 16, image::RgbaImage::from_pixel(16, 16, art_color));
        let letter = band_grid(16, vec![PxText { y: 1, x: 1, text: "N".into(), style: 0, fg: z_red, bg: 0 }]);
        let c = build_chrome_canvas(&[&art, &letter], (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        // px 12 is the second cell (no ink, no run) — the banner art shows through.
        assert_eq!(*c.get_pixel(12, 8), art_color, "explicit-fg-only run leaves the banner art (no bg flood)");
    }

    // ── per-window page fill (SQ-0704, ZMSD §8.8.3.2) ───────────────────────

    /// A chrome grid window at `(0,0)` covering `w × h` native pixels, carrying
    /// `bg` as its own Normal-style background.
    fn page_grid(w: u16, h: u16, bg: Option<u32>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: (w / 8).max(1), h: (h / 16).max(1), x_px: 0, y_px: 0, w_px: w, h_px: h,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                fill: None,
                cols: (w / 8).max(1), rows: (h / 16).max(1), cells: vec![], active_rows: 1,
                cursor: (0, 0), cursor_active: false, border: BorderPref::Unspecified,
                bg, fg: None, reverse: false, px_texts: Vec::new(),
            }),
        }
    }

    #[test]
    fn window_page_fills_only_the_holes_and_leaves_art_alone() {
        // A window whose art covers the top half only: the untouched bottom half
        // becomes the window's own page, the art stays byte-for-byte.
        let art_color = Rgba([180, 140, 90, 255]);
        let art = graphics_window(0, 0, 16, 8, image::RgbaImage::from_pixel(16, 8, art_color));
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&art, &win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(c.get_pixel(4, 12)[3], 0, "precondition: the window's lower half is unpainted");
        fill_window_pages(&mut c, &chrome, None, &colors());
        assert_eq!(*c.get_pixel(4, 12), Rgba([0, 0, 255, 255]), "an unpainted pixel takes the window's own page");
        assert_eq!(*c.get_pixel(4, 4), art_color, "artwork is never repainted");
    }

    #[test]
    fn window_with_no_page_of_its_own_keeps_todays_transparency() {
        let win = page_grid(16, 16, None);
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        let before = c.as_raw().clone();
        fill_window_pages(&mut c, &chrome, None, &colors());
        assert_eq!(*c.as_raw(), before, "a window the game gave no colour is left exactly as before");
    }

    #[test]
    fn a_window_overlapping_the_story_box_is_skipped() {
        // Zork Zero's window 7 carries the same page across the WHOLE screen;
        // filling it would flood the hybrid transcript viewport and defeat
        // `story_clear_native`'s clear-interior probe.
        let full = page_grid(16, 16, Some(BLUE));
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 4, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let chrome = [&full];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        fill_window_pages(&mut c, &chrome, Some(&story), &colors());
        assert_eq!(c.get_pixel(8, 8)[3], 0, "the story box stays clear for the transcript");
        assert_eq!(c.get_pixel(0, 0)[3], 0, "and the covering window is skipped whole, not clipped");
    }

    #[test]
    fn an_inherited_colour_is_not_a_page_choice() {
        // Standard 0/1 ("current"/"default", ZMSD §8.3.1) are inheritance, not a
        // colour the game named — `packed_explicit` rejects them.
        let win = page_grid(16, 16, Some(1u32 << 24)); // Standard(0)
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        fill_window_pages(&mut c, &chrome, None, &colors());
        assert_eq!(c.get_pixel(4, 4)[3], 0, "an inherited colour leaves the window's page to the host");
    }

    // ── declined colours: a PAINTED window keeps its page (SQ-0716) ─────────

    /// A painted ground with one opaque pixel at `(px, py)` of a `w × h` surface.
    fn ground(w: u32, h: u32, px: u32, py: u32) -> image::RgbaImage {
        let mut g = image::RgbaImage::new(w, h);
        g.put_pixel(px, py, Rgba([255, 0, 0, 255]));
        g
    }

    #[test]
    fn a_painted_window_keeps_its_page_with_colours_declined() {
        // scopa's shape: the game drew inside this window, so its declared page is
        // the ground of that drawing rather than a palette preference.
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), Some(&ground(16, 16, 4, 4)));
        assert_eq!(*c.get_pixel(10, 10), Rgba([0, 0, 255, 255]), "the painted window's page arrives anyway");
    }

    #[test]
    fn an_unpainted_window_still_declines_its_page() {
        // The flag keeps its meaning for every window the game only coloured:
        // Zork Zero, Arthur, Shogun, Journey and advent paint no ground at all.
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        let before = c.as_raw().clone();
        // A ground that exists but lies entirely outside this window's box.
        let mut g = image::RgbaImage::new(64, 64);
        g.put_pixel(40, 40, Rgba([255, 0, 0, 255]));
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), Some(&g));
        assert_eq!(*c.as_raw(), before, "a window the game never drew into keeps the host page");
    }

    #[test]
    fn no_painted_ground_at_all_changes_nothing() {
        let win = page_grid(16, 16, Some(BLUE));
        let chrome = [&win];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        let before = c.as_raw().clone();
        fill_painted_window_pages(&mut c, &chrome, None, &colors(), None);
        assert_eq!(*c.as_raw(), before, "no ground, no exception");
    }

    #[test]
    fn a_painted_window_over_the_story_box_is_still_skipped() {
        // The story window's page and ink are the reading surface: they are the
        // pair `honor_game_colours` governs, painted ground or not.
        let full = page_grid(16, 16, Some(BLUE));
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 4, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let chrome = [&full];
        let mut c = build_chrome_canvas(&chrome, (16, 16), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors());
        fill_painted_window_pages(&mut c, &chrome, Some(&story), &colors(), Some(&ground(16, 16, 4, 4)));
        assert_eq!(c.get_pixel(0, 0)[3], 0, "the story-overlapping window is skipped whole, exactly as when colours are honoured");
    }

    // ── story region background fill (Lane C) ───────────────────────────────

    #[test]
    fn story_bg_rgba_resolves_the_windows_own_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(BLUE), ..Default::default() }),
        };
        let color = story_bg_rgba(Some(&story), &colors()).expect("win0 set a bg colour");
        assert_eq!(color, Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn story_bg_rgba_is_none_when_the_game_set_no_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        assert!(story_bg_rgba(Some(&story), &colors()).is_none(), "no game colour ⇒ None (caller leaves it transparent)");
    }

    #[test]
    fn story_bg_rgba_fills_the_clear_interior_rect() {
        // End-to-end through the same calls screen.rs makes: resolve the colour,
        // then fill_cell the story_clear_native rect with it.
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 2, y_px: 2, w_px: 4, h_px: 4, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(RED), ..Default::default() }),
        };
        let mut canvas = RgbaImage::new(8, 8);
        let (sx, sy, sw, sh) = story_clear_native(Some(&story), &canvas).expect("story window present");
        let color = story_bg_rgba(Some(&story), &colors()).expect("bg set");
        fill_cell(&mut canvas, sx, sy, sw, sh, color);
        for y in 2..6 {
            for x in 2..6 {
                assert_eq!(*canvas.get_pixel(x, y), Rgba([255, 0, 0, 255]), "story rect filled red at ({x},{y})");
            }
        }
        assert_eq!(canvas.get_pixel(0, 0)[3], 0, "outside the story rect stays transparent");
    }

    #[test]
    fn flatten_onto_page_only_repaints_fully_transparent_pixels() {
        // SQ-0510: the raster composite's leftover holes become the page, but any
        // pixel a layer touched — however faintly — is left byte-for-byte alone,
        // so frame art, status bands, glyphs and drop-caps can never be covered.
        let page = Rgba([26, 26, 26, 255]);
        let art = Rgba([102, 34, 0, 255]);
        let faint = Rgba([1, 2, 3, 1]); // alpha 1: touched, so untouchable
        let mut canvas = RgbaImage::new(3, 1);
        canvas.put_pixel(0, 0, Rgba([0, 0, 0, 0])); // an untouched hole
        canvas.put_pixel(1, 0, art);
        canvas.put_pixel(2, 0, faint);

        flatten_onto_page(&mut canvas, page);

        assert_eq!(*canvas.get_pixel(0, 0), page, "a fully transparent pixel becomes the page");
        assert_eq!(*canvas.get_pixel(1, 0), art, "an opaque art pixel is never repainted");
        assert_eq!(*canvas.get_pixel(2, 0), faint, "even alpha==1 counts as drawn and survives");
        assert!(canvas.pixels().all(|p| p[3] > 0), "no fully transparent pixel is left behind");
    }

    #[test]
    fn uniform_scale_letterboxes() {
        let scale = uniform_scale((320, 200), (640, 480));
        assert_eq!(scale.s, 2.0);
        assert_eq!(scale.off_x, 0);
        assert_eq!(scale.off_y, 40);
    }

    #[test]
    fn story_viewport_clears_the_chrome_ring() {
        // 40x40 native canvas: opaque top band rows 0..8, opaque left cols
        // 0..8 and right cols 32..40 across all rows; interior transparent.
        let mut canvas = image::RgbaImage::new(40, 40);
        let opaque = Rgba([255, 255, 255, 255]);
        for y in 0..40u32 {
            for x in 0..40u32 {
                let in_band = y < 8;
                let in_side = !(8..32).contains(&x);
                if in_band || in_side {
                    canvas.put_pixel(x, y, opaque);
                }
            }
        }
        let story = buffer_item(0, true);
        // buffer_item defaults x_px/y_px to 0 and w_px/h_px to 8; override via
        // a fresh PositionedWindow spanning the whole native area.
        let story = PositionedWindow { x_px: 0, y_px: 0, w_px: 40, h_px: 40, ..story };
        let scale = uniform_scale((40, 40), (40, 40));
        let rect = story_viewport(Some(&story), &canvas, &scale, (40, 40), (1, 1));
        assert!(rect.x >= 8, "left edge clears the left band: x={}", rect.x);
        assert!(rect.y >= 8, "top edge clears the top band: y={}", rect.y);
        assert!(rect.x + rect.width <= 32, "right edge clears the right band: x+w={}", rect.x + rect.width);
        assert!(rect.width >= 1);
        assert!(rect.height >= 1);
    }

    #[test]
    fn story_viewport_no_story_is_full_pane() {
        let canvas = image::RgbaImage::new(40, 40);
        let scale = uniform_scale((40, 40), (40, 40));
        let rect = story_viewport(None, &canvas, &scale, (40, 40), (1, 1));
        assert_eq!(rect, ratatui::layout::Rect { x: 0, y: 0, width: 40, height: 40 });
    }

    // ── Hybrid render mode: story_viewport_box + chrome_bands ──────────────────

    #[test]
    fn story_viewport_box_maps_win0_box_inward_to_cells() {
        // Native 320×200 game, win0 box (43,39,234,160). Scale 1:1 (native px ==
        // device px), 8 px/cell. Rounding INWARD: left ceil(43/8)=6, top
        // ceil(39/8)=5, right floor((43+234)/8)=floor(277/8)=34,
        // bottom floor((39+160)/8)=floor(199/8)=24 → 28×19 cells at (6,5).
        let story = PositionedWindow { x_px: 43, y_px: 39, w_px: 234, h_px: 160, ..buffer_item(0, true) };
        let scale = uniform_scale((320, 200), (320, 200)); // s = 1.0, no offset
        assert_eq!(scale.s, 1.0);
        let rect = story_viewport_box(Some(&story), &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 6, y: 5, width: 28, height: 19 });
    }

    #[test]
    fn story_viewport_box_no_story_is_full_pane() {
        let scale = uniform_scale((320, 200), (320, 200));
        let rect = story_viewport_box(None, &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 0, y: 0, width: 40, height: 25 });
    }

    #[test]
    fn chrome_bands_tile_pane_minus_viewport_without_overlap() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        let viewport = Rect::new(6, 5, 28, 19); // interior, all four edges inset
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4, "all four edges produce a band");
        // Non-overlap + exact tiling: every pane cell OUTSIDE the viewport is
        // covered exactly once; every viewport cell is covered zero times.
        let mut cover = vec![0u8; (pane.width as usize) * (pane.height as usize)];
        for b in &bands {
            for y in b.y..b.bottom() {
                for x in b.x..b.right() {
                    cover[y as usize * pane.width as usize + x as usize] += 1;
                }
            }
        }
        for y in 0..pane.height {
            for x in 0..pane.width {
                let inside_vp = (viewport.x..viewport.right()).contains(&x) && (viewport.y..viewport.bottom()).contains(&y);
                let c = cover[y as usize * pane.width as usize + x as usize];
                if inside_vp {
                    assert_eq!(c, 0, "viewport cell ({x},{y}) untouched by chrome bands");
                } else {
                    assert_eq!(c, 1, "chrome cell ({x},{y}) covered exactly once");
                }
            }
        }
    }

    #[test]
    fn chrome_bands_omit_flush_edges() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        // Viewport flush to the left and top edges → only bottom + right bands.
        let viewport = Rect::new(0, 0, 30, 20);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 2, "left+top flush → those bands omitted");
        assert!(bands.iter().all(|b| b.x >= 30 || b.y >= 20), "remaining bands are the right/bottom ring");
    }

    #[test]
    fn chrome_bands_full_viewport_is_empty() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        assert!(chrome_bands(pane, pane).is_empty(), "viewport == pane → no chrome");
    }

    #[test]
    fn chrome_bands_absolute_coords_offset_pane() {
        use ratatui::layout::Rect;
        // A pane not anchored at the origin: bands must tile pane − viewport in the
        // same absolute space (the hybrid path passes absolute rects).
        let pane = Rect::new(10, 4, 20, 12);
        let viewport = Rect::new(13, 6, 12, 6);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4);
        for b in &bands {
            assert!(b.x >= pane.x && b.right() <= pane.right() && b.y >= pane.y && b.bottom() <= pane.bottom(),
                "band {b:?} stays inside the pane");
        }
    }
}
