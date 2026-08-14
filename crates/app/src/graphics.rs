//! Graphics-window canvases + Blorb Pict resolution for in-game Glulx graphics.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

use crate::interpreter::InterpreterProfile;

/// Unpack a Glk 24-bit `0xRRGGBB` color into an opaque RGBA pixel.
fn rgb(color: u32) -> Rgba<u8> {
    Rgba([(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF])
}

/// A graphics window's pixel canvas.
///
/// `img` is an `Arc` so [`arc`](Canvas::arc) — called for every graphics window
/// on every screen refresh (once per timer tick during an animation) — is a
/// cheap reference-count bump, not a full-bitmap deep copy. Mutations go through
/// `Arc::make_mut`, which copies-on-write only when a previously-handed-out clone
/// is still alive, so a static canvas is never copied. (SQ-0343)
/// Process-global draw sequence: every v6 picture draw stamps the target
/// canvas with the next value, so the renderer can z-order overlapping v6
/// windows by DRAW ORDER (later draw = on top) instead of window number — the
/// order the game actually painted them (e.g. Zork0 draws its banner, then the
/// compass overlays, then the room illustration on top). (SQ-0186)
static DRAW_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The next global draw-sequence stamp.
pub fn next_draw_seq() -> u64 {
    DRAW_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Cloning is cheap: the pixels ride an `Arc` and every mutator goes through
/// `Arc::make_mut`, so a clone is a refcount bump that only copies if one of the
/// two is painted afterwards. That is what makes the v6 picture pacer's
/// intermediate frames (SQ-0708) affordable — it snapshots the whole canvas map
/// at each step of a turn's picture sequence.
#[derive(Clone)]
pub struct Canvas {
    pub img: Arc<RgbaImage>,
    bg: Rgba<u8>,
    /// Bumped on every draw so the renderer can cache the built protocol.
    pub version: u64,
    /// Global draw-order stamp of this canvas's most recent picture draw
    /// (0 = never drawn). The v6 compositor sorts overlapping windows by this,
    /// so later-drawn windows paint on top. Set only on the v6 picture path.
    pub z_seq: u64,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        // Default background is TRANSPARENT, not opaque black: a graphics window's
        // pixels that the game hasn't painted (a fresh canvas, or one just cleared
        // by a resize before the game's Arrange redraw lands) must show the pane
        // underneath, never a solid black block. Games that want an opaque
        // background set it via glk_window_set_background_color. (SQ-0332)
        Canvas { img: Arc::new(RgbaImage::new(w.max(1), h.max(1))), bg: Rgba([0, 0, 0, 0x00]), version: 1, z_seq: 0 }
    }

    /// Resize (preserving nothing — Glk redraws) if the pixel dims changed. Cleared
    /// to `bg` (transparent unless the game set one), so an un-redrawn window shows
    /// the pane, not a black block.
    pub fn resize(&mut self, w: u32, h: u32) {
        if (self.img.width(), self.img.height()) != (w.max(1), h.max(1)) {
            self.img = Arc::new(RgbaImage::from_pixel(w.max(1), h.max(1), self.bg));
            self.version += 1;
        }
    }

    /// Grow the canvas to at least `w × h`, PRESERVING existing content (a v6
    /// window can receive several stacked pictures, and a picture may extend past
    /// the window's nominal pixel size — e.g. Zork0's 45×40 compass into a 320×5
    /// banner). Never shrinks; a no-op when already big enough. Unlike `resize`,
    /// this keeps what was already drawn. (SQ-0186)
    pub fn grow_to(&mut self, w: u32, h: u32) {
        let (cw, ch) = (self.img.width(), self.img.height());
        let (nw, nh) = (cw.max(w.max(1)), ch.max(h.max(1)));
        if (nw, nh) == (cw, ch) {
            return;
        }
        let mut grown = RgbaImage::from_pixel(nw, nh, self.bg);
        image::imageops::replace(&mut grown, &*self.img, 0, 0);
        self.img = Arc::new(grown);
        self.version += 1;
    }

    pub fn set_background(&mut self, color: u32) { self.bg = rgb(color); }

    fn paint(&mut self, px: Rgba<u8>, left: i32, top: i32, w: u32, h: u32) {
        let (cw, ch) = (self.img.width() as i64, self.img.height() as i64);
        let x0 = left.max(0) as i64;
        let y0 = top.max(0) as i64;
        let x1 = (left as i64 + w as i64).min(cw);
        let y1 = (top as i64 + h as i64).min(ch);
        let img = Arc::make_mut(&mut self.img);
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x as u32, y as u32, px);
            }
        }
        self.version += 1;
    }

    pub fn fill_rect(&mut self, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.paint(rgb(color), left, top, w, h);
    }

    pub fn erase_rect(&mut self, left: i32, top: i32, w: u32, h: u32) {
        let bg = self.bg;
        self.paint(bg, left, top, w, h);
    }

    /// Composite `src` at `(x, y)`, optionally scaled to `(sw, sh)`, honoring alpha.
    ///
    /// `(sw, sh)` come from the game (`glk_image_draw_scaled`) and are clamped
    /// to the canvas dimensions before allocating the scaled bitmap — anything
    /// larger is clipped by `overlay` anyway, and clamping bounds the
    /// allocation against a malicious/buggy game requesting e.g. a
    /// 0x40000000 x 0x40000000 image.
    ///
    /// The resample is [`crate::render::graphics::resize_directional`] (SQ-0829),
    /// not a fixed filter. A game names both axes independently here, so this one
    /// call can grow one and shrink the other, and it used to run Triangle at every
    /// size: a Glulx title card blown up 3× came back blurred — the exact opposite
    /// of the "crisp integer upscale" pixel art is drawn at — while a cut-out card
    /// (fmvpoker's deck is stencilled on colour 1) bled its transparent
    /// `(0, 0, 0)` into every edge it was shrunk past.
    pub fn draw_image(&mut self, src: &DynamicImage, x: i32, y: i32, scale: Option<(u32, u32)>) {
        let scaled;
        let view: &DynamicImage = match scale {
            Some((sw, sh)) if sw > 0 && sh > 0 => {
                let sw = sw.min(self.img.width());
                let sh = sh.min(self.img.height());
                scaled = DynamicImage::ImageRgba8(crate::render::graphics::resize_directional(
                    &src.to_rgba8(),
                    sw,
                    sh,
                ));
                &scaled
            }
            _ => src,
        };
        image::imageops::overlay(Arc::make_mut(&mut self.img), view, x as i64, y as i64);
        self.version += 1;
    }

    /// Like [`Canvas::draw_image`] (unscaled), but clipped to the `clip`
    /// pixel box `(w, h)` anchored at the canvas origin — ZMSD §8's "all
    /// plotting is always clipped to the current window" for a canvas that may
    /// be larger than the window's current box (a window that shrank keeps its
    /// old pixels; only new plotting is bounded by the new size).
    pub fn draw_image_clipped(&mut self, src: &DynamicImage, x: i32, y: i32, clip: (u32, u32)) {
        if x < 0 || y < 0 {
            // v6 draw coords are 1-based-positive by the time they reach the
            // canvas; anything else is clamped upstream.
            return;
        }
        let (cx, cy) = (x as u32, y as u32);
        let (cw, ch) = clip;
        if cx >= cw || cy >= ch {
            return;
        }
        let allow_w = (cw - cx).min(src.width());
        let allow_h = (ch - cy).min(src.height());
        if allow_w == 0 || allow_h == 0 {
            return;
        }
        if allow_w < src.width() || allow_h < src.height() {
            let cropped = src.crop_imm(0, 0, allow_w, allow_h);
            image::imageops::overlay(Arc::make_mut(&mut self.img), &cropped, x as i64, y as i64);
        } else {
            image::imageops::overlay(Arc::make_mut(&mut self.img), src, x as i64, y as i64);
        }
        self.version += 1;
    }

    /// A cheap clone of the canvas bitmap (an `Arc` ref-count bump — see the type
    /// docs), handed to the renderer each frame.
    pub fn arc(&self) -> Arc<RgbaImage> { Arc::clone(&self.img) }
}

/// Resolves + caches decoded images by Blorb `Pict` resource number.
///
/// Adaptive palettes (Blorb spec §11.3): pictures listed in the container's
/// `APal` chunk carry a PLACEHOLDER palette. When one is drawn it must be
/// plotted with the "Current Palette" — the palette (PLTE) of the most recently
/// drawn NON-adaptive picture — not its own. We track the current palette as raw
/// PLTE bytes and, when decoding an adaptive picture, splice those bytes into a
/// copy of its PNG's PLTE chunk (fixing the CRC) before handing it to the
/// decoder. Only the PLTE is substituted: the spec derives the Current Palette
/// from "PLTE, gAMA, cHRM and sRGB/iCCP" but NOT tRNS, and Infocom's adaptive
/// overlays rely on their OWN tRNS for the transparent index, so tRNS is left
/// intact. (Since the decoder reads palette RGB verbatim, copying PLTE alone
/// reproduces exactly the colours the base picture renders with.)
///
/// A story mounted out of an Amiga `.adf` disk image has no Blorb at all: its
/// art is the native `Pic.data` archive that shipped on the same floppy, held
/// here as `native` (SQ-0719 / SQ-0734 tier 2). That format carries no `APal`
/// chunk, but it states the same thing per picture: a directory record whose
/// palette offset is ZERO has no colours of its own and must be drawn through
/// the Current Palette. For Zork Zero those records are, id for id, exactly the
/// 172 numbers `Zork0.blb` lists in `APal` — the Blorb's chunk was derived from
/// this field — so a native archive feeds the machinery below through the same
/// `adaptive` set and the same Current Palette, expressed in the same RGB
/// triples a `PLTE` holds. (SQ-0743)
#[derive(Debug)]
pub struct PictSource {
    blorb: Option<blorb::Blorb>,
    /// Native Infocom picture archive, used when there is no Blorb.
    native: Option<blorb::infocom_pics::InfocomPics>,
    cache: HashMap<u32, Option<Arc<DynamicImage>>>,
    /// Pict numbers declared adaptive by the Blorb `APal` chunk (§11.3). Empty
    /// for the overwhelmingly common no-`APal` case, where `image` takes the
    /// original palette-agnostic fast path.
    adaptive: HashSet<u32>,
    /// Raw PLTE bytes (RGB triples) of the most recently drawn non-adaptive
    /// indexed picture — the "Current Palette". `None` until one is drawn; per
    /// §11.3 an adaptive picture drawn before any non-adaptive one is undefined,
    /// and we fall back to its own placeholder palette.
    current_plte: Option<Vec<u8>>,
    /// Bumped whenever `current_plte` actually changes. Adaptive decodes are
    /// cached per `(resnum, palette_gen)` so a palette change re-decodes them
    /// (the same overlay is legally drawn under different base palettes over a
    /// game's life).
    palette_gen: u64,
    /// Adaptive decodes keyed by `(resnum, palette_gen)`.
    adaptive_cache: HashMap<(u32, u64), Option<Arc<DynamicImage>>>,
    /// The colour table this source's video hardware fixed, when it had one
    /// (SQ-0794) — `blorb::infocom_pics::InfocomPics::hardware_palette`. `None`
    /// for a Blorb, an Amiga/Mac `Pic.data` and an MCGA `.MG1` alike, all of
    /// which carry their colours per picture.
    hw_palette: Option<[blorb::infocom_pics::Rgb; 16]>,
    /// Does this source's art need [`blend_half_width_columns`] on the way out —
    /// i.e. is it a SIXTEEN-colour 640-wide rendition, whose pixels are half as
    /// wide as the unit screen's and whose dithers the card fused (SQ-0797)?
    ///
    /// Set once from [`PictSource::art_scale`] and
    /// [`PictSource::is_monochrome`], because both answers come off the archive's
    /// own contents and neither ever changes for a loaded source.
    blend_columns: bool,
}

impl PictSource {
    pub fn new(blorb: Option<blorb::Blorb>) -> PictSource {
        let adaptive = blorb
            .as_ref()
            .map(|b| b.adaptive_pictures().iter().copied().collect())
            .unwrap_or_default();
        PictSource {
            blorb,
            native: None,
            cache: HashMap::new(),
            adaptive,
            current_plte: None,
            palette_gen: 0,
            adaptive_cache: HashMap::new(),
            hw_palette: None,
            blend_columns: false,
        }
    }

    /// A source backed by a native Infocom picture archive (Amiga `Pic.data`)
    /// rather than a Blorb — the artwork that shipped on the same disk image as
    /// the story (SQ-0719).
    ///
    /// An EGA or CGA archive has **no** adaptive pictures, however its directory
    /// reads (SQ-0794). Its records are 12 bytes with nowhere to keep a palette,
    /// so every one of them answers "no palette of my own" — but that is the
    /// opposite of Blorb §11.3's adaptive, which means *defer to the
    /// interpreter*. These defer to nothing: their colours were fixed in the
    /// video card, and `hardware_palette` is that card's table. Emptying the
    /// adaptive set here is what keeps the Current-Palette machinery from
    /// splicing a `PLTE` into pictures that have no say in their own colours.
    pub fn from_native(pics: blorb::infocom_pics::InfocomPics) -> PictSource {
        let hw_palette = pics.hardware_palette();
        let adaptive = match hw_palette {
            Some(_) => HashSet::new(),
            None => pics.adaptive_pictures().iter().map(|&id| u32::from(id)).collect(),
        };
        let mut src =
            PictSource { native: Some(pics), adaptive, hw_palette, ..PictSource::new(None) };
        // A 640-wide SIXTEEN-colour rendition dithers, and its dither is the
        // card's business, not the terminal's — see `blend_half_width_columns`.
        src.blend_columns = src.art_scale().is_some_and(|(sx, _)| sx == 1) && !src.is_monochrome();
        src
    }

    /// Resolve the picture source for `story_path` (SQ-0734's tiers 1 and 2).
    ///
    /// A release disk image supplies its own art: the story and the picture
    /// archive came off the same floppy, so the pairing is guaranteed by the
    /// medium and needs no configuration. **Whichever format the disk is** —
    /// this asks `blorb`'s one mount path (SQ-0840) rather than naming Amiga or
    /// Macintosh, so a format that lands in `blorb::medium::FORMATS` brings its
    /// artwork here without a line changing. Everything else — including a disk
    /// image that carries no readable archive — resolves the story's resource
    /// Blorb exactly as before.
    ///
    /// The Macintosh shipped **two** archives per game, one per screen it sold:
    /// a colour one and a monochrome one, and this decoder reads both (SQ-0838).
    /// The mount hands back the COLOUR one, because that is what every other
    /// medium here supplies and choosing two-colour art for a terminal with
    /// sixteen million of them would need a reason the disk does not give.
    /// The monochrome archive is reached by naming it — `--pictures Pic.data`,
    /// through [`PictureOverride`], which now looks inside the medium for a name
    /// that is not on the host filesystem.
    ///
    /// **The medium is the release, not the platter** (SQ-0862). A multi-disk
    /// press can put the story on one disk and its artwork on another — the DOS
    /// 360K Zork Zero puts CGA on disk 1, the story alone on disk 2 and EGA on
    /// disk 3 — so this asks [`crate::assets::volumes`] rather than mounting one
    /// image. See that function for which siblings are allowed to speak for a
    /// story and which are not.
    pub fn resolve(story_path: &std::path::Path) -> PictSource {
        if let Some(pics) = release_art(story_path) {
            return PictSource::from_native(pics);
        }
        PictSource::new(blorb::resolve_resource_blorb(story_path).map(|(b, _)| b))
    }

    /// Resolve the picture source across all three tiers (SQ-0734).
    ///
    /// `over` is the already-resolved tier-3 override, taken here by value
    /// because a loaded archive moves straight into the source. It is resolved
    /// separately and earlier by [`PictureOverride::resolve`] because its
    /// FLAVOUR also picks the interpreter profile, which has to be settled
    /// before the engine is built.
    ///
    /// A loaded override wins outright — over a resource Blorb beside the story
    /// and over an `.adf`'s own `Pic.data` alike. Everything else, including a
    /// named file that is missing or will not decode, falls through to
    /// [`PictSource::resolve`]; the caller is responsible for surfacing
    /// [`PictureOverride::warning`] in those cases rather than letting the
    /// player believe they are looking at native art.
    pub fn resolve_with_override(
        story_path: &std::path::Path,
        over: PictureOverride,
    ) -> PictSource {
        match over {
            PictureOverride::Loaded { pics, .. } => PictSource::from_native(pics),
            PictureOverride::Unset
            | PictureOverride::Missing { .. }
            | PictureOverride::Unusable { .. } => PictSource::resolve(story_path),
        }
    }

    /// Generation counter of the Current Palette — bumped whenever a non-adaptive
    /// draw establishes a DIFFERENT palette (§11.3). A caller that has already
    /// plotted adaptive pictures watches this: when it moves, everything drawn
    /// with the old palette is now showing the wrong colours and must be replotted.
    pub fn palette_gen(&self) -> u64 {
        self.palette_gen
    }

    /// Keep this source's dither UNFUSED, or fuse it (SQ-0816).
    ///
    /// `fuse` is the player's `fuse_art_dither` preference, and it can only ever
    /// turn the filter OFF: whether an archive is eligible at all stays entirely
    /// [`from_native`](PictSource::from_native)'s business, read off the archive's
    /// own contents. A `.CG1` is not fused because a person said so, and cannot be
    /// made to be — [`blend_half_width_columns`] would only make its one-bit line
    /// work grey (SQ-0806, SQ-0808).
    ///
    /// Both caches are dropped when the answer changes, because every image in
    /// them was decoded under the old one. Call it before the first draw and the
    /// caches are empty anyway; call it later — a settings edit would — and the
    /// next `image()` re-decodes rather than serving a stale blend.
    pub fn set_fuse_dither(&mut self, fuse: bool) {
        let eligible = self.art_scale().is_some_and(|(sx, _)| sx == 1) && !self.is_monochrome();
        let want = fuse && eligible;
        if want == self.blend_columns {
            return;
        }
        self.blend_columns = want;
        self.cache.clear();
        self.adaptive_cache.clear();
    }

    /// Is this source fusing a 640-wide rendition's dither on the way out?
    pub fn fuses_dither(&self) -> bool {
        self.blend_columns
    }

    /// Is the artwork a TWO-COLOUR rendition? See
    /// [`blorb::infocom_pics::InfocomPics::is_monochrome`]. `false` for a Blorb,
    /// which is never one.
    pub fn is_monochrome(&self) -> bool {
        self.native.as_ref().is_some_and(blorb::infocom_pics::InfocomPics::is_monochrome)
    }

    /// Should this launch declare the interpreter COLOURLESS to the story
    /// (`honor_game_colours = false`), because the artwork in hand has no
    /// colours to give? SQ-0806, refined by SQ-0846.
    ///
    /// **The archive's half is a guess about a machine, and that is the whole
    /// point.** A two-colour rendition is a stencil whose transparency reveals a
    /// ground the artwork never had to store, and the story cannot see which
    /// archive was loaded — Zork Zero issues `set_colour(fg=2, bg=9)` for every
    /// video card alike, so honouring it paints the white pillars out against
    /// the white page it asked for. Nothing in a `.CG1` names its machine; the
    /// monochrome flag is the only evidence there is, and bocfel says as much
    /// ("the flags always *seem* to equal 0xe if the graphics are monochrome").
    /// Declaring the interpreter colourless hands the ground back to the host
    /// theme and the stencil reads again.
    ///
    /// **A profile that states its own defaults is not a guess, and outranks
    /// one.** [`InterpreterProfile::default_colours`] is `None` for the IBM PC
    /// precisely because babelmap has no machine there to speak for — which is
    /// every launch the rule above was written for. Where it answers, the answer
    /// came off that machine's own interpreter: the Macintosh's white page under
    /// black ink is `mac/xzip.lst`'s `SetColor := (zWHITE*256) + zBLACK`, and a
    /// mono Mac `Pic.data` is the archive that same interpreter chose *for* that
    /// page, in one decision (SQ-0838). Turning colours off there does not save
    /// a stencil from a colour the game asked for; it throws away the one machine
    /// whose colours are known, and it cost SQ-0846's status banner its ink.
    pub fn declines_game_colours(&self, profile: InterpreterProfile) -> bool {
        self.is_monochrome() && profile.default_colours().is_none()
    }

    /// Is this Pict declared adaptive by the container's `APal` chunk (§11.3)?
    pub fn is_adaptive(&self, resnum: u32) -> bool {
        self.adaptive.contains(&resnum)
    }

    /// The Current Palette's raw `PLTE` bytes, for host Save State (SQ-0588).
    ///
    /// Blorb §11.3 makes this live interpreter state, not game state: it is
    /// established by whichever non-adaptive picture was drawn last, and every
    /// adaptive picture drawn after it decodes through it. A save that carries the
    /// display list but not the palette replays those pictures under whatever
    /// palette the restoring session happens to hold — the wrong colours, or none.
    pub fn current_palette(&self) -> Option<&[u8]> {
        self.current_plte.as_deref()
    }

    /// Reinstate a saved Current Palette (SQ-0588). Bumps `palette_gen` on a real
    /// change, exactly as a non-adaptive draw does, so any adaptive decode cached
    /// against the old generation is recomputed rather than reused.
    pub fn set_current_palette(&mut self, plte: Option<Vec<u8>>) {
        if self.current_plte != plte {
            self.current_plte = plte;
            self.palette_gen += 1;
        }
    }

    /// Decode `resnum` under the CURRENT palette WITHOUT establishing a new one —
    /// the replay path (SQ-0567).
    ///
    /// A v6 screen holds palette indices, so once a picture is on it, it displays
    /// through whatever palette is loaded now — base pictures included, not just the
    /// adaptive ones. Replaying a window therefore decodes every picture the same way
    /// an adaptive draw is decoded: the live PLTE spliced in, its own ignored. Calling
    /// `image` here instead would let each replayed base picture reload the palette
    /// and undo the change being replayed for.
    pub fn image_under_current_palette(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        if self.adaptive.is_empty() && self.current_plte.is_none() {
            return self.get(resnum).cloned();
        }
        self.adaptive_image(resnum)
    }

    /// The Blorb `Reso` standard window `(width, height)` in pixels — the
    /// resolution the pictures were authored for. A v6 story advertises this
    /// as its screen size so its hardcoded pixel art lines up (SQ-0186).
    /// `None` when there's no Blorb or no `Reso` chunk.
    ///
    /// A native archive has no equivalent field — Infocom's Amiga interpreter
    /// knew its own 320×200 lores screen — so it answers `None` and lands on
    /// exactly that fallback downstream, which is also what the Blorb `Reso`
    /// chunks of these games say.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        self.blorb.as_ref().and_then(|b| b.std_window())
    }

    /// The standard window a NATIVE archive implies — the picture space every
    /// Infocom archive draws into, whichever machine wrote it (SQ-0837).
    ///
    /// This is the same constant, for the same reason,
    /// [`PictureOverride::std_window`] hands back when a user names an archive
    /// outright: mounting an archive off the disk it shipped on must not produce
    /// different geometry from pointing at that very file by hand.
    ///
    /// It sits AHEAD of the machine in `startup.rs`'s chain, and that ordering
    /// is Infocom's rather than ours (SQ-0838). Their Macintosh interpreter
    /// picked its window and its picture file in one decision — *"for a small
    /// window use mono gfx, for a big window use color gfx"* — so the archive in
    /// hand is the better evidence about the screen, not the worse. SQ-0837 put
    /// this link last, when it answered one fixed pair and could only ever
    /// restate what the machine already said; now that it answers per archive it
    /// has to come first, or a mono-only Mac volume would be laid out on the
    /// colour Mac's screen. **Nothing else moves**: for a `.adf` this is
    /// (320, 200) and so is the Amiga profile, and a Blorb-less story that is
    /// not a disk image still has no native archive at all and still draws its
    /// art 1:1 (SQ-0715/SQ-0718).
    ///
    /// # The screen is the picture space, at the scale the machine drew it
    ///
    /// | rendition                     | picture space | scale | screen  |
    /// |-------------------------------|---------------|-------|---------|
    /// | Amiga/Mac colour, MCGA `.MG1` | 320×200       | (2,2) | 640×400 |
    /// | EGA `.EG1`/`.EG2`, CGA `.CG1` | 640×200       | (1,2) | 640×400 |
    /// | Macintosh mono `Pic.data`     | 480×300       | (1,1) | 480×300 |
    ///
    /// The first two rows are the corpus as it already stood: SQ-0790's reading
    /// that a 320-wide and a 640-wide rendition are *two drawings of one screen*
    /// survives intact, because the denser one's half-width pixels put it back
    /// on the same 640×400. What that reading could not express is a third
    /// rendition drawn for a genuinely different screen, and the standard
    /// Macintosh's monochrome plate is one: `mac/gfx.p` calls it "scaled for a
    /// 480x300 screen (std Mac)" and Infocom's interpreter really did open a
    /// 480×300 window for it. Multiplying the space by the scale gives every
    /// old rendition the screen it already had and the new one the screen it
    /// asks for — see [`Self::art_scale`] and
    /// [`crate::interpreter::InterpreterProfile::std_window`].
    ///
    /// **A caveat this cannot hide**: 300 is not a multiple of the 16-pixel v6
    /// cell, so `session.rs` rounds the screen to 19 rows — 304 pixels, four
    /// more than the Mac's, where a real Mac fitted 20 rows of its own 15-pixel
    /// Geneva into exactly 300. Rounding the other way would hand the game a
    /// 288-pixel screen and clip the bottom twelve pixels off its own artwork.
    pub fn native_std_window(&self) -> Option<(u16, u16)> {
        let pics = self.native.as_ref()?;
        Some((pics.picture_space_width(), pics.picture_space_height()))
    }

    /// The per-axis factor this source's art is scaled by on its way into the
    /// 640×400 unit screen, or `None` when the source has no opinion and the
    /// uniform [`crate::session::V6_ART_SCALE`] rule stands (SQ-0790).
    ///
    /// Only a NATIVE archive answers, because only a native archive states a
    /// picture space. Every Blorb is answered by its `Reso` chunk upstream, and
    /// a Blorb-less story (scopa) is answered by Blorb §11's non-scalable rule.
    ///
    /// The unit screen is the same 640×400 for every rendition — it is
    /// babelmap's presentation space, not the card's — so the factor is simply
    /// how many unit pixels one art pixel covers on each axis:
    ///
    /// | rendition                    | picture space | scale |
    /// |------------------------------|---------------|-------|
    /// | Amiga `Pic.data`, MCGA `.MG1`| 320×200       | (2, 2)|
    /// | EGA `.EG1`/`.EG2`, CGA `.CG1`| 640×200       | (1, 2)|
    /// | Macintosh mono `Pic.data`    | 480×300       | (1, 1)|
    ///
    /// The last row is why the vertical factor is derived rather than fixed at
    /// [`crate::session::V6_ART_SCALE`] (SQ-0838). Every rendition Infocom
    /// shipped is 200 lines tall except the standard Macintosh's monochrome one,
    /// which `mac/gfx.p` calls a "480x300 screen (std Mac)" and displays 1:1
    /// where it scales the colour art by 1.5 or 2 (`IF ge.mono OR myTiny THEN {
    /// scale 1x for display }`). 480×300 is the one picture space that does not
    /// double onto the 640×400 unit screen, and doubling it anyway would put a
    /// 960×600 plate on a 640×400 screen. Deriving both axes the same way leaves
    /// all four existing renditions exactly where they were — 400/200 is 2 — and
    /// is a change of reasoning rather than of behaviour for them.
    ///
    /// The 640-wide row is the whole of SQ-0790, and it is not a compromise: an
    /// EGA pixel really is half as wide as an MCGA one. Bocfel encodes exactly
    /// this as `pixelwidth` — 1.0 at `hw_screenwidth` 320, **0.5** at 640 — and
    /// its final blit (`draw_image.cpp:251`) works out to `gscreenw * H / 320`
    /// for both, i.e. the two picture spaces cover the same rectangle. Frotz
    /// says the same thing as `x_scale = (flags & 0x08) ? 640 : 320`
    /// (`src/curses/ux_pic.c:126`).
    ///
    /// And the corpus agrees, which is what makes this a measurement rather than
    /// a reading: under this rule `arthur.mg1` and `arthur.eg1` produce
    /// **identical** unit-space dimensions for all 125 pictures they share, and
    /// `zork0.mg1`/`zork0.eg1` for 446 of 503 (the remainder differ by a pixel
    /// or two because the two renditions are separately drawn artwork, not one
    /// scaled copy).
    pub fn art_scale(&self) -> Option<(u32, u32)> {
        let pics = self.native.as_ref()?;
        let space_w = u32::from(pics.picture_space_width()).max(1);
        let space_h = u32::from(pics.picture_space_height()).max(1);
        let unit_w = u32::from(INFOCOM_V6_STD_WINDOW.0) * crate::session::V6_ART_SCALE;
        let unit_h = u32::from(INFOCOM_V6_STD_WINDOW.1) * crate::session::V6_ART_SCALE;
        Some(((unit_w / space_w).max(1), (unit_h / space_h).max(1)))
    }

    fn get(&mut self, resnum: u32) -> Option<&Arc<DynamicImage>> {
        if !self.cache.contains_key(&resnum) {
            let decoded = match (&self.blorb, &self.native) {
                (Some(b), _) => b
                    .resource(b"Pict", resnum)
                    .and_then(|(_ty, bytes)| crate::cover::decode(bytes)),
                (None, Some(pics)) => {
                    native_image(pics, resnum, self.hw_palette.as_ref(), self.blend_columns)
                }
                (None, None) => None,
            };
            self.cache.insert(resnum, decoded.map(Arc::new));
        }
        self.cache.get(&resnum).and_then(|o| o.as_ref())
    }

    /// `(width, height)` of a Pict, or `None`.
    pub fn info(&mut self, resnum: u32) -> Option<(u32, u32)> {
        self.get(resnum).map(|i| i.dimensions())
    }

    /// The decoded image for a Pict about to be DRAWN, or `None`. Returns a
    /// cheap `Arc` clone rather than deep-copying the `DynamicImage`.
    ///
    /// This is the adaptive-palette establishment point (Blorb §11.3): drawing a
    /// NON-adaptive picture updates the Current Palette from its PLTE; drawing an
    /// ADAPTIVE picture decodes it with that Current Palette spliced in. Size
    /// queries (`info`/`dims`) deliberately do NOT go through here, so querying a
    /// picture's dimensions never counts as "drawing" for palette purposes.
    pub fn image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        // No APal chunk → no adaptive pictures: keep the original fast path
        // (and never touch palette state) for every non-v6 / non-adaptive blorb.
        if self.adaptive.is_empty() {
            return self.get(resnum).cloned();
        }
        if self.adaptive.contains(&resnum) {
            return self.adaptive_image(resnum);
        }
        // A non-adaptive draw establishes the Current Palette for later adaptive
        // draws, then resolves normally.
        let arc = self.get(resnum).cloned();
        if arc.is_some() {
            self.set_current_palette_from(resnum);
        }
        arc
    }

    /// Remember Pict `resnum`'s PLTE as the Current Palette (§11.3). No-op for a
    /// non-indexed picture (no PLTE); bumps `palette_gen` only on a real change.
    ///
    /// A native archive names its palette in the directory record instead of a
    /// `PLTE` chunk, and answers here without decoding the picture's pixels; the
    /// 16 RGB triples it yields are the same shape a `PLTE` holds, so the
    /// Current Palette — including the copy a host Save State carries — is one
    /// representation across both archives.
    fn set_current_palette_from(&mut self, resnum: u32) {
        let plte = match (&self.blorb, &self.native) {
            (Some(b), _) => b
                .resource(b"Pict", resnum)
                .and_then(|(_ty, bytes)| png_plte(bytes)),
            (None, Some(pics)) => u16::try_from(resnum)
                .ok()
                .and_then(|id| pics.palette_of(id))
                .map(|pal| pal.concat()),
            (None, None) => None,
        };
        let Some(plte) = plte else {
            return;
        };
        if self.current_plte.as_deref() != Some(plte.as_slice()) {
            self.current_plte = Some(plte);
            self.palette_gen += 1;
        }
    }

    /// Decode an adaptive picture with the Current Palette spliced into its PLTE
    /// (§11.3), caching per `(resnum, palette_gen)`. With no base picture drawn
    /// yet the palette is undefined per spec; we fall back to the placeholder.
    fn adaptive_image(&mut self, resnum: u32) -> Option<Arc<DynamicImage>> {
        let key = (resnum, self.palette_gen);
        if !self.adaptive_cache.contains_key(&key) {
            let decoded = match (&self.blorb, &self.native) {
                // Clone the raw PNG bytes so the immutable blorb borrow ends
                // before we mutate the cache.
                (Some(b), _) => b
                    .resource(b"Pict", resnum)
                    .map(|(_ty, bytes)| bytes.to_vec())
                    .and_then(|raw| {
                        let spliced = self
                            .current_plte
                            .as_ref()
                            .and_then(|plte| splice_plte(&raw, plte));
                        crate::cover::decode(spliced.as_deref().unwrap_or(&raw))
                    }),
                // A native picture is palette indices already: there is no PLTE
                // to splice, the Current Palette IS the colour table it expands
                // through. With none loaded yet it falls back to its own — §11.3
                // leaves that case undefined and the Blorb path does the same.
                //
                // A hardware table outranks the Current Palette outright: an EGA
                // or CGA picture's colours were the video card's, so there is
                // nothing for a loaded palette to adapt (SQ-0794). No archive
                // reaches here with one — `from_native` leaves such a source with
                // an empty adaptive set — but a restored Current Palette must not
                // be able to reach in through the replay path either.
                (None, Some(pics)) => {
                    let pal = self
                        .hw_palette
                        .or_else(|| self.current_plte.as_deref().map(colour_table));
                    native_image(pics, resnum, pal.as_ref(), self.blend_columns)
                }
                (None, None) => None,
            };
            self.adaptive_cache.insert(key, decoded.map(Arc::new));
        }
        self.adaptive_cache.get(&key).and_then(|o| o.clone())
    }

    /// The bytes + text-flag of Blorb `Data` resource `resnum`, for
    /// `glk_stream_open_resource`. `is_text` is true for a `TEXT` chunk, false
    /// for `BINA`/`FORM` (binary). `None` when there is no Blorb or no such Data
    /// resource. (The `PictSource` is AppGlk's sole Blorb holder, so Data lookup
    /// lives here alongside `Pict` lookup.)
    pub fn data_resource(&self, resnum: u32) -> Option<(Vec<u8>, bool)> {
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Data", resnum)?;
        Some((bytes.to_vec(), ty == b"TEXT"))
    }

    /// `(width, height)` of Pict `resnum`, sniffed from the image header only —
    /// no full decode. Used by the v6 Z-machine `picture_data` dimension table
    /// (Plan 1a), where only the size is needed at boot, not the pixels.
    ///
    /// A `Rect` chunk (Blorb §Rect: 8 bytes, width then height, big-endian) is a
    /// dimension-only placeholder with no pixels — Infocom v6 games (Zork Zero,
    /// Shogun, Arthur) query these via `picture_data` as invisible *placement*
    /// pictures whose (height, width) encode screen (y, x) layout coordinates.
    pub fn dims(&mut self, resnum: u32) -> Option<(u32, u32)> {
        if self.blorb.is_none() {
            // A native directory record carries the size whether or not the
            // entry has pixels, so a placeholder answers here just as a Blorb
            // `Rect` does — which is what those placeholders became on
            // conversion.
            let pics = self.native.as_ref()?;
            let e = pics.entry(u16::try_from(resnum).ok()?)?;
            return Some((u32::from(e.width), u32::from(e.height)));
        }
        let (ty, bytes) = self.blorb.as_ref()?.resource(b"Pict", resnum)?;
        if ty == b"Rect" {
            let b: &[u8] = bytes;
            if b.len() < 8 {
                return None;
            }
            let w = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            let h = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
            return Some((w, h));
        }
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()
    }

    /// `(number, width, height)` for every `Pict` resource in the Blorb, header-
    /// sniffed via [`PictSource::dims`]. Feeds the v6 `Machine::set_picture_dims`
    /// injection at session construction (Plan 1a). Empty when there is neither
    /// a Blorb nor a native archive.
    pub fn all_pict_dims(&mut self) -> Vec<(u16, u16, u16)> {
        let numbers: Vec<u32> = match (&self.blorb, &self.native) {
            (Some(b), _) => b.resources().iter().filter(|r| &r.usage == b"Pict").map(|r| r.number).collect(),
            (None, Some(pics)) => pics.entries().iter().map(|e| u32::from(e.id)).collect(),
            (None, None) => Vec::new(),
        };
        numbers
            .into_iter()
            .filter_map(|n| self.dims(n).map(|(w, h)| (n as u16, w as u16, h as u16)))
            .collect()
    }
}

/// The Infocom Version 6 standard window: the ART resolution every v6 release
/// was laid out against, which babelmap presents doubled as the 640×400 unit
/// screen (SQ-0479). It is the same on every machine that shipped one of these
/// games, and it is what every Infocom Blorb's `Reso` chunk declares — a native
/// archive has no chunk to declare it with, so this stands in.
///
/// It is NOT the archive's picture space, which is 320 or 640 depending on how
/// wide the card's pixels were; see [`PictSource::art_scale`].
pub const INFOCOM_V6_STD_WINDOW: (u16, u16) = (320, 200);

/// Tier 3 of the picture-resource policy (SQ-0734): the user names a native
/// Infocom archive in the per-game sidecar `<game_dir>/config.toml` and thereby
/// ASSERTS that it belongs to this story.
///
/// ```toml
/// pictures = "FMVPOKER.EG1"
/// ```
///
/// # Why the user has to say it
///
/// The three tiers are ordered by confidence in the PAIRING, and nothing below
/// tier 3 can be guessed. A Blorb validates its own contents. A disk image ties
/// story and archive together by the medium they shipped on. A loose archive
/// beside a story ties them together by *nothing*: the format carries no release
/// number and no serial, every Infocom Amiga release names its archive
/// `Pic.data`, and the PC names are a DOS 8.3 convention that survives neither a
/// renamed story nor a renamed archive. A stem rule would therefore have to be
/// wrong sometimes, and being wrong here is INVISIBLE — Arthur's plates drawn
/// into Zork Zero look like art, not like an error. So there is no
/// auto-discovery, and this is deliberate rather than unfinished.
///
/// **Discovery for DISPLAY is safe; discovery for PAIRING is not.** Those look
/// contradictory and are not, and the difference is the whole policy. Listing
/// the archives that happen to sit beside a story, and showing that list to a
/// person, is fine — better than fine, because the person knows which game they
/// own and can supply the assertion the file format cannot make. Taking the same
/// list and *picking from it* is what has no evidence behind it. If a future
/// feature enumerates candidates (SQ-0789 proposes exactly that, for a picker),
/// it must hand them to a human and end there; wiring an enumerator into this
/// function would reintroduce precisely the failure the tiers exist to prevent.
/// Nothing here enumerates anything today, on purpose.
///
/// # What naming one buys
///
/// It wins outright. A named archive that loads beats a resource Blorb beside
/// the story and beats the `Pic.data` an `.adf` carries: naming it is an
/// instruction, not a hint. It also picks the machine — see
/// [`crate::interpreter::InterpreterProfile::resolve`], which takes
/// [`PictureOverride::flavour`] as its second-most-specific input.
///
/// # What a bad name costs
///
/// Nothing silently. A file that is absent, or present and undecodable, leaves
/// the Blorb in charge but produces a [`PictureOverride::warning`] the host must
/// show. Falling back quietly would recreate the exact failure the policy exists
/// to prevent, inverted: the player believing they are seeing native art when
/// they are not, with nothing on screen to say otherwise.
#[derive(Debug)]
pub enum PictureOverride {
    /// No `pictures` key. Tiers 1 and 2 decide, exactly as before.
    Unset,
    /// The key names a file that is not there. "If the file exists" was the
    /// user's condition for the override winning, so it does not apply — but the
    /// user asked for something they did not get, and hears about it.
    Missing { path: std::path::PathBuf },
    /// The named file exists and cannot be used: unreadable, the wrong container
    /// flavour, corrupt, or truncated. Loud on purpose.
    Unusable { path: std::path::PathBuf, reason: String },
    /// The named file is a native archive, and it wins.
    ///
    /// `refused` carries the complaint when a file sat under the next part's
    /// name and turned out not to be one (SQ-0798). The archive still loads —
    /// part 1 is exactly what the user named — but the continuation is never
    /// dropped in silence.
    Loaded {
        path: std::path::PathBuf,
        pics: blorb::infocom_pics::InfocomPics,
        refused: Option<String>,
    },
}

impl PictureOverride {
    /// Read and validate the `pictures` key of `<game_dir>/config.toml`.
    ///
    /// A relative value resolves against the STORY's own directory — "beside the
    /// story" is where these archives sit, and the sidecar lives elsewhere, in
    /// the per-game save directory. An absolute value is used as given.
    ///
    /// The file is parsed here, not merely stat'ed, for two reasons: the flavour
    /// it turns out to be selects the interpreter profile, and a file that will
    /// not decode must be reported before the story boots rather than discovered
    /// picture by picture as blanks.
    pub fn resolve(story_path: &std::path::Path, game_dir: &std::path::Path) -> PictureOverride {
        PictureOverride::resolve_with_session(story_path, game_dir, None)
    }

    /// [`resolve`](PictureOverride::resolve), with a name supplied for THIS
    /// launch taking precedence over the sidecar's key.
    ///
    /// The session name is the other two doors into this mechanism (SQ-0791 /
    /// SQ-0789): `--pictures` on the command line, and the launch-options
    /// dialog's un-persisted choice. Both outrank the config key, matching how an
    /// explicit interpreter number already outranks the inferred one and the
    /// general rule that the more specific and more recent instruction wins.
    ///
    /// Everything downstream is unchanged: the archive still has to parse, it
    /// still beats a Blorb and an `.adf`'s own `Pic.data`, its flavour still
    /// picks the machine, and a name that is absent or will not decode is still
    /// loud. A door is not a policy.
    ///
    /// # Naming a file that is INSIDE the medium
    ///
    /// A story mounted out of a disk image has no directory to put a loose
    /// archive next to, and the archive a user would want to name is already on
    /// the volume. So when the bare name does not exist on the host filesystem
    /// and the story is a disk image, the name is looked up on the volume
    /// instead — see [`read_off_the_medium`].
    ///
    /// This is what makes the Macintosh's **monochrome** `Pic.data` reachable
    /// (SQ-0838). Its disk carries two archives, `Hfs::pictures` hands back the
    /// colour one, and choosing the other is a preference nothing on the disk
    /// states — so it is the user's to state, with `--pictures Pic.data`, and
    /// there is nowhere else for that name to point. An Amiga `.adf` gains the
    /// same door by the same code, which is the point: a door, not a policy.
    pub fn resolve_with_session(
        story_path: &std::path::Path,
        game_dir: &std::path::Path,
        session: Option<&str>,
    ) -> PictureOverride {
        let Some(name) = session
            .map(str::to_string)
            .or_else(|| crate::styles::read_per_game_pictures(game_dir))
        else {
            return PictureOverride::Unset;
        };
        let named = std::path::Path::new(&name);
        let path = if named.is_absolute() {
            named.to_path_buf()
        } else {
            story_path.parent().unwrap_or(std::path::Path::new(".")).join(named)
        };
        let raw = match std::fs::read(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match read_off_the_medium(story_path, named) {
                    Some(raw) => raw,
                    None => return PictureOverride::Missing { path },
                }
            }
            Err(e) => return PictureOverride::Unusable { path, reason: e.to_string() },
        };
        match blorb::infocom_pics::InfocomPics::parse(raw) {
            Ok(mut pics) => {
                let refused = absorb_continuations(&mut pics, &path);
                PictureOverride::Loaded { path, pics, refused }
            }
            Err(e) => PictureOverride::Unusable { path, reason: e.to_string() },
        }
    }

    /// The flavour of the archive the user named, for interpreter-profile
    /// selection. `None` whenever no usable archive was named, which leaves the
    /// medium and then the default to decide.
    pub fn flavour(&self) -> Option<blorb::infocom_pics::Flavour> {
        match self {
            PictureOverride::Loaded { pics, .. } => Some(pics.flavour()),
            PictureOverride::Unset
            | PictureOverride::Missing { .. }
            | PictureOverride::Unusable { .. } => None,
        }
    }

    /// The standard window — the machine's native ART resolution — that the
    /// named archive implies, for the `v6_screen_px` chain at boot.
    ///
    /// A native archive has no `Reso` chunk, because **the format has no such
    /// concept** (SQ-0736). Blorb §11 makes the ABSENCE of `Reso` meaningful —
    /// non-scalable art, drawn one image pixel per screen pixel — so reading a
    /// native archive's silence as that declaration is what left Zork Zero's
    /// 320×200 art at half size on a 640×400 screen. The archive is not silent:
    /// the standard window is the machine's, and every machine that wrote one of
    /// these archives drew v6 on the same one.
    ///
    /// Nearly every rendition answers with the SAME standard window, and that is
    /// the point (SQ-0790). A 320-wide archive (Amiga/Mac colour, MCGA `.MG1`)
    /// and a 640-wide one (EGA `.EG1`/`.EG2`, CGA `.CG1`) are two drawings of
    /// one screen, not two screens: the EGA plate has twice as many pixels
    /// across because each is half as wide, so both cover the same rectangle.
    /// What differs between them is the DENSITY of the art, which is
    /// [`PictSource::art_scale`]'s business — 320-wide doubles onto the 640×400
    /// unit screen, 640-wide is x1 across and x2 down. Answering with the
    /// picture space and letting the scale close the gap says exactly that, and
    /// says it for the one rendition that really is a second screen: the
    /// standard Macintosh's 480×300 monochrome plate, which lands 1:1 on a
    /// 480×300 screen. See [`PictSource::native_std_window`] for the table and
    /// the sources; mounting an archive off the disk it shipped on and naming
    /// that very file by hand must not produce different geometry, so the two
    /// are deliberately one answer.
    ///
    /// SQ-0734 shipped `None` here for a 640-wide archive as an explicit
    /// deferral, on the reading that its true presentation was a 640×200 screen
    /// on an 8×8 cell that `V6_ART_SCALE` could not express. The screen half of
    /// that turned out to be wrong: 640×200 on an 8×8 cell is 80×25 characters,
    /// which is the same character grid the 640×400 unit screen already provides
    /// on its 8×16 cell. Only the art density was ever different, and a per-axis
    /// art scale is the whole of it.
    pub fn std_window(&self) -> Option<(u16, u16)> {
        match self {
            // The archive's own picture space, rather than
            // `interpreter::AMIGA_STD_WINDOW`: for an MCGA `.MG1` the two are
            // the same numbers, but naming the Amiga's constant here would read
            // as a claim that an MCGA archive is Amiga media, and it is not.
            PictureOverride::Loaded { pics, .. } => {
                Some((pics.picture_space_width(), pics.picture_space_height()))
            }
            _ => None,
        }
    }

    /// The complaint to show the user, naming the file and the reason. `None`
    /// when there is nothing to complain about — no key, or a key that worked.
    ///
    /// A key that worked can still have one thing to say: an archive loaded
    /// whose *continuation* was refused draws with fewer pictures than the set it
    /// claims to be, which is the SQ-0798 defect wearing a different hat.
    pub fn warning(&self) -> Option<String> {
        match self {
            PictureOverride::Unset => None,
            PictureOverride::Loaded { refused, .. } => refused.clone(),
            PictureOverride::Missing { path } => Some(format!(
                "pictures = \"{}\" names a picture archive that is not there — \
                 falling back to this story's Blorb art",
                path.display(),
            )),
            PictureOverride::Unusable { path, reason } => Some(format!(
                "pictures = \"{}\" cannot be used: {reason} — \
                 falling back to this story's Blorb art",
                path.display(),
            )),
        }
    }
}

/// The path of part `part` of the multi-part set `path` belongs to, or `None`
/// when the name cannot express one (SQ-0798).
///
/// **The format states the part number, and the filename carries it.** Header
/// byte 0 is the part; Frotz's DOS port turns that number straight back into a
/// filename — `extension[3] = '0' + number`, under the comment *"EGA pictures
/// may be stored in two separate graphics files"* (`src/dos/bcpic.c`) — and its
/// `open_graphics_file(int number)` takes the part as a parameter for exactly
/// this reason. So the rule here is Infocom's own: replace the final character
/// of the extension with the part's digit, leaving everything else, case
/// included, untouched. `Pic.data` has no trailing digit and therefore no
/// continuation, which is correct — the Amiga releases ship one file.
///
/// # This is NOT the stem-based discovery SQ-0734 rejected
///
/// Those look alike and are opposites, and the difference is the whole of the
/// tier policy. What SQ-0734 forbids is pairing an archive to a **story** on the
/// evidence of a name: `arthur.z6` sitting beside `arthur.mg1` proves nothing,
/// every Amiga release calls its archive `Pic.data`, and a wrong pairing draws
/// Arthur's plates into Zork Zero with nothing on screen to say so.
///
/// Nothing of that kind happens here. The pairing has **already been asserted**
/// — by a user naming the archive outright, which is tier 3 — and this only
/// follows that one archive's own in-band part number to the rest of itself. The
/// story is not consulted and could not be. What is guessed is a filename; what
/// is then *verified* is the header, by
/// [`blorb::infocom_pics::InfocomPics::append_part`], which refuses a file whose
/// part byte, codec or picture ids say it is not the continuation. A stem rule
/// has nothing to verify against; this one does.
pub fn part_path(path: &std::path::Path, part: u8) -> Option<std::path::PathBuf> {
    // One digit is all a DOS 8.3 extension can hold, and it is all `bcpic.c`
    // writes; 0 is not a part number.
    if !(1..=9).contains(&part) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let (stem, ext) = name.rsplit_once('.')?;
    let mut ext: Vec<u8> = ext.as_bytes().to_vec();
    if !ext.last()?.is_ascii_digit() {
        return None;
    }
    *ext.last_mut()? = b'0' + part;
    let ext = String::from_utf8(ext).ok()?;
    Some(path.with_file_name(format!("{stem}.{ext}")))
}

/// The artwork the release `story_path` came off supplies for it, or `None` when
/// it supplies none (SQ-0862).
///
/// Tier 2 of the picture policy, widened from the platter to the release. Which
/// volumes may answer at all is [`crate::assets::volumes`]'s question, and the
/// interesting half of the argument is there; this only decides which of the
/// answers to take when more than one volume has one.
///
/// # The order, and the one preference in it
///
/// **The story's own volume wins outright.** It is the strongest pairing on
/// offer and it is what every single-image release already resolved to, so
/// nothing that worked before this function existed moves — the 720K Zork Zero
/// press keeps the `ZORK0.MG1` sharing its story's disk and does not start
/// preferring disk 2's CGA.
///
/// Among the siblings, **colour beats monochrome**, and then the disk order the
/// set is already in. `blorb`'s per-volume `pictures()` deliberately expresses no
/// preference between video cards — and it never had to, because no image in the
/// corpus carries two renditions. A release does: the DOS 360K press offers CGA
/// on disk 1 and EGA on disk 3, and taking the earlier disk would hand a terminal
/// with sixteen million colours a two-colour Zork Zero. CGA's two colours are a
/// 1989 hardware constraint, not an authorial choice, so a rendition that kept
/// its colour is the better default. It is only a default: every rendition on the
/// release is listed in the launch dialog and reachable by name, which is where
/// a person who wants the CGA plates says so.
///
/// Note what is NOT decided here — MCGA against EGA. Both are colour, both draw
/// the same 503 pictures into the same geometry (`PictSource::art_scale`), and no
/// release in the corpus puts the two on sibling volumes of one set, so ranking
/// them would be a rule with no example. Disk order settles it if one ever does.
fn release_art(story_path: &std::path::Path) -> Option<blorb::infocom_pics::InfocomPics> {
    let volumes = crate::assets::volumes(story_path);
    let (own, siblings) = volumes.split_first()?;
    if let Some(art) = own.disk.pictures() {
        return Some(art.pictures);
    }
    let mut found: Vec<blorb::infocom_pics::InfocomPics> =
        siblings.iter().filter_map(|v| v.disk.pictures().map(|a| a.pictures)).collect();
    // Stable, so disk order survives as the last tiebreak.
    found.sort_by_key(|p| (p.is_monochrome(), std::cmp::Reverse(p.entries().len())));
    found.into_iter().next()
}

/// Read a bare filename off the release `story_path` was mounted from, for a
/// user naming an archive that lives INSIDE the medium (SQ-0838).
///
/// The Macintosh release is the case that needs it: its disk carries a colour
/// `CPic.data` and a monochrome `Pic.data`, the automatic choice is colour, and
/// the other one exists nowhere on the host filesystem for `--pictures` to point
/// at. An Amiga `.adf` is read by the same three lines.
///
/// **Only after the host filesystem has been tried and come up empty**, and
/// never for an absolute path, which is an instruction about the host and is
/// honoured as one. A name that resolves to a real file beside the story still
/// wins, so nothing that worked before moves. The lookup is by name because that
/// is what the user typed — the CONTENT tiers ([`PictSource::resolve`]) are what
/// identify an archive nobody named.
///
/// # Every volume of the release, not just the story's own (SQ-0862)
///
/// A name the launch dialog showed must be a name this can find, and the dialog
/// lists the whole release's artwork — booting the DOS 360K Zork Zero's disk 2
/// offers `ZORK0.EG1`, which is on disk 3. Both doors ask
/// [`crate::assets::volumes`] so they cannot answer differently; the story's own
/// volume is searched first, so a name that resolved before still resolves to the
/// same file.
///
/// # One mount path, and why this had to become one (SQ-0833)
///
/// This read `if looks_like_adf … else if looks_like_hfs` until the DOS and
/// Atari ST formats arrived — the last copy of the two-reader chain SQ-0840
/// replaced everywhere else, missed because it predates `MountedDisk`. It was
/// merely stale while two formats existed and became a **defect** the moment a
/// third registered: `crate::assets::files` enumerates a disk's artwork through
/// `blorb::medium` and so offered a FAT12 disk's `ZORK0.EG1` in the launch
/// dialog, while this function had no arm that could load it. Offered, picked,
/// and nothing drawn. It goes through the one table now, so the next format
/// cannot reintroduce the gap.
///
/// # A name may carry a directory now
///
/// It used to be rejected outright, on the reasonable ground that a directory is
/// an instruction about where to look — reasonable while no volume HAD
/// directories. FAT12 does: every story on an Atari ST compilation is called
/// `STORY.DAT` and the folder is the only thing that tells four of them apart,
/// so `HITCHHIK/STORY.DAT` is the volume's own spelling and `contents` shows it
/// that way. What a caller is shown, it must be able to ask for. The components
/// are re-joined with `/` so a Windows user's backslashes reach the same file;
/// AmigaDOS and HFS are flat, so a name with a separator simply matches nothing
/// there, exactly as before.
fn read_off_the_medium(story_path: &std::path::Path, named: &std::path::Path) -> Option<Vec<u8>> {
    if named.is_absolute() {
        return None;
    }
    let parts: Option<Vec<&str>> =
        named.components().map(|c| c.as_os_str().to_str()).collect();
    let name = parts?.join("/");
    crate::assets::volumes(story_path).iter().find_map(|v| v.disk.read_named(&name))
}

/// Merge every continuation of `path` into `pics`, and return the complaint if
/// one was found and refused (SQ-0798).
///
/// # How far it looks
///
/// **Until a part is missing.** Arthur and Journey stop at two, but the format
/// does not: the part byte is a whole byte and `open_graphics_file(int number)`
/// takes any number, so a title we do not have could ship three. Stopping at 2
/// would be a corpus fact hardcoded as a rule, and it would fail silently — the
/// symptom is missing pictures, which is precisely the defect this fixes.
/// Scanning instead costs one `open` of a file that is not there per set, once
/// at boot, and the naming bounds the walk on its own: a DOS extension holds one
/// digit, so [`part_path`] answers `None` past part 9 and the loop cannot run
/// away.
///
/// An absent next part is the ordinary end of the walk and says nothing. A part
/// that is *present* and does not check out is refused and reported: SQ-0734's
/// rule that an unusable named archive must be loud applies to its continuation,
/// because a half-loaded set looks exactly like a complete one until the game
/// draws the picture that is not there.
pub fn absorb_continuations(
    pics: &mut blorb::infocom_pics::InfocomPics,
    path: &std::path::Path,
) -> Option<String> {
    while let Some(next) = part_path(path, pics.next_part()) {
        let Ok(raw) = std::fs::read(&next) else {
            return None; // no such part: the set ends here, as most do.
        };
        let outcome = blorb::infocom_pics::InfocomPics::parse(raw)
            .and_then(|part| pics.append_part(part));
        if let Err(e) = outcome {
            return Some(format!(
                "{} sits under this archive's next part name but {e} — \
                 using only the parts that check out",
                next.display(),
            ));
        }
    }
    None
}

/// Decode one picture out of a native Infocom archive into the same
/// `DynamicImage` the Blorb path yields (SQ-0719).
///
/// `Picture::rgba` already expands the palette indices and gives the
/// transparent index alpha 0, which is exactly what `Canvas`'s alpha-honoring
/// overlay wants. `None` covers every "no image here" case alike: an unknown
/// id, a size-only placeholder, or a compression variant we do not decode.
///
/// `palette`, when given, overrides the picture's own — the Current Palette an
/// adaptive picture is drawn through (SQ-0743).
fn native_image(
    pics: &blorb::infocom_pics::InfocomPics,
    resnum: u32,
    palette: Option<&[blorb::infocom_pics::Rgb; 16]>,
    blend_columns: bool,
) -> Option<DynamicImage> {
    let pic = pics.decode(u16::try_from(resnum).ok()?).ok()?;
    let rgba = match palette {
        Some(pal) => pic.rgba_with(pal),
        None => pic.rgba(),
    };
    let mut buf = image::RgbaImage::from_raw(u32::from(pic.width), u32::from(pic.height), rgba)?;
    if blend_columns {
        blend_half_width_columns(&mut buf);
    }
    Some(DynamicImage::ImageRgba8(buf))
}

/// Fuse a 640-wide rendition's column dither, because its pixels are half as wide
/// as the unit screen's (SQ-0797).
///
/// # Why there is anything to fuse
///
/// EGA has no bronze, so Zork Zero's artist made one. The proscenium arch is a
/// column-by-column dither — bright red (index 12) against brown (index 6) for
/// the lit stone, light grey against bright red for the highlights, brown against
/// black for the shadow — and on a 640×200 EGA screen those columns are half as
/// wide as an MCGA pixel, so the card and the eye fused each pair into a colour
/// the palette does not contain. Bocfel says the same of Zork Zero's EGA hint
/// background (`z6/draw_border.cpp:745`): "no single pixel of the artwork is the
/// colour the eye actually sees". babelmap keeps all 640 columns — geometrically
/// right, [`PictSource::art_scale`] maps them onto exactly the rectangle a
/// 320-wide plate covers — so without this the dither arrives at full contrast
/// and the arch reads as salmon-and-olive speckle.
///
/// # Why here and not in the renderer
///
/// Because the unit-space→pane scale MAGNIFIES at every pane worth playing on, and
/// magnification is `FilterType::Nearest`, deliberately: crisp DOS pixels are the
/// house style, and a nearest resample never blends. Above 640 px it replicates the
/// columns, so the dither arrives at full contrast however wide the pane is; below
/// 640 the resampler now takes the area arm (SQ-0824,
/// [`resize_directional`](crate::render::graphics::resize_directional)) and would
/// fuse the pair itself — but that was never a reason to leave the artwork alone,
/// because it would make the fused colour a function
/// of how wide the player's terminal happens to be. Measured horizontal speckle on
/// Zork Zero's EGA border ran 22.3 / 40.3 / 49.1 / 39.2 / 24.4 at pane widths of
/// 320 / 480 / 640 / 800 / 1280, against 4.3 for the same frame in MCGA. Fusing
/// at the archive boundary instead makes the answer a property of the artwork.
///
/// # The filter
///
/// A three-tap tent, `[1, 2, 1] / 4`, across columns only. It is chosen over the
/// obvious two-tap box on fixed column pairs because it is PHASE-INDEPENDENT: for
/// any period-2 alternation `…a b a b…` the tent yields `(a + b) / 2` at *both*
/// phases and so collapses the dither exactly, wherever it starts, while a fixed
/// pairing only collapses the half of it that happens to be aligned. It is also
/// symmetric (no half-pixel shift) and leaves flat colour untouched. Measured on
/// Zork Zero's EGA border, mean per-channel distance to the MCGA frame: 44.5 raw,
/// 29.2 with the box, **27.8** with the tent.
///
/// Alpha is never touched and never blended. A transparent pixel is left exactly
/// as it is, and an opaque one at a transparency edge folds the missing tap's
/// weight back onto itself — a native archive's art is a stencil (fmvpoker's
/// cards are cut out on colour 1, SQ-0801), and a fringe of half-transparent
/// pixels around every cut-out is not what the card did.
///
/// # What is deliberately NOT fused
///
/// CGA. A `.CG1`'s 640-wide art is genuine one-bit line work — Zork Zero's CGA
/// pillar is a lit column of mirrored tiles (SQ-0808), and SQ-0806 hands its two
/// colours to the terminal — and blending one-bit line work only makes it grey.
/// [`PictSource::is_monochrome`] is the test, read off the archive's own
/// `EF_MONO` flags rather than off a filename. MCGA and the Amiga are 320-wide,
/// have no dither at this frequency, and never reach here.
fn blend_half_width_columns(img: &mut image::RgbaImage) {
    let (w, h) = img.dimensions();
    if w < 2 {
        return;
    }
    let w = w as usize;
    let stride = w * 4;
    let mut row = vec![0u8; stride];
    let buf: &mut [u8] = img;
    for y in 0..h as usize {
        let base = y * stride;
        row.copy_from_slice(&buf[base..base + stride]);
        for x in 0..w {
            let c: [u8; 4] = row[x * 4..x * 4 + 4].try_into().expect("4 bytes per pixel");
            if c[3] != 255 {
                continue; // a cut-out stays cut out, at its own colour
            }

            let tap = |i: usize| -> [u8; 4] {
                let p: [u8; 4] = row[i * 4..i * 4 + 4].try_into().expect("4 bytes per pixel");
                if p[3] == 255 { p } else { c }
            };
            let l = if x > 0 { tap(x - 1) } else { c };
            let r = if x + 1 < w { tap(x + 1) } else { c };
            for k in 0..3 {
                let sum = u32::from(l[k]) + 2 * u32::from(c[k]) + u32::from(r[k]);
                buf[base + x * 4 + k] = ((sum + 2) / 4) as u8;
            }
        }
    }
}

/// The Current Palette's raw RGB triples as a native 16-entry colour table.
/// Entries the palette does not reach keep the archive's default, so no pixel
/// index is ever left without a colour.
fn colour_table(plte: &[u8]) -> [blorb::infocom_pics::Rgb; 16] {
    let mut pal = blorb::infocom_pics::DEFAULT_PALETTE;
    for (slot, c) in pal.iter_mut().zip(plte.chunks_exact(3)) {
        *slot = [c[0], c[1], c[2]];
    }
    pal
}

/// PNG 8-byte signature.
const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// The raw PLTE chunk data (palette RGB triples) of a PNG byte stream, or `None`
/// when the bytes aren't a PNG or carry no PLTE (e.g. a truecolor picture, which
/// therefore never becomes the Current Palette — Blorb §11.3 tracks PLTE).
fn png_plte(png: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let mut q = 8;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            break;
        }
        if ty == b"PLTE" {
            return Some(png[ds..ds + len].to_vec());
        }
        q = ds + len + 4; // data + 4-byte CRC
    }
    None
}

/// The bit depth from a PNG's IHDR (always the first chunk), used to cap the
/// spliced palette to the `2^bitdepth`-entry PLTE maximum. `None` if `png` isn't
/// a PNG opening with an IHDR chunk.
fn png_bit_depth(png: &[u8]) -> Option<u8> {
    // [sig 8][len 4][IHDR 4][width 4][height 4][bit_depth 1]… → offset 24.
    if png.len() <= 24 || &png[0..8] != PNG_SIG || &png[12..16] != b"IHDR" {
        return None;
    }
    Some(png[24])
}

/// A copy of PNG `png` with its PLTE chunk data replaced by the Current Palette
/// `new_plte` (CRC recomputed), or `None` if `png` isn't an indexed PNG carrying
/// a PLTE. Entry-count differences (Blorb §11.3): the replacement is capped to
/// the picture's bit-depth maximum (`2^bitdepth` entries); when the Current
/// Palette is SHORTER than the placeholder it replaces, the placeholder's
/// trailing entries are kept so no pixel index is left without a colour. Only
/// PLTE is touched — every other chunk (crucially tRNS, which carries the
/// overlay's transparent index) is copied verbatim.
fn splice_plte(png: &[u8], new_plte: &[u8]) -> Option<Vec<u8>> {
    if png.len() < 8 || &png[0..8] != PNG_SIG {
        return None;
    }
    let max_bytes = (1usize << png_bit_depth(png)?).saturating_mul(3);
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&png[0..8]);
    let mut q = 8;
    let mut replaced = false;
    while q + 8 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        let ty = &png[q + 4..q + 8];
        let ds = q + 8;
        if ds + len + 4 > png.len() {
            return None; // truncated chunk → don't hand a corrupt stream on
        }
        if ty == b"PLTE" {
            let orig = &png[ds..ds + len];
            let mut pal = new_plte[..new_plte.len().min(max_bytes)].to_vec();
            if pal.len() < orig.len() {
                pal.extend_from_slice(&orig[pal.len()..]); // keep trailing indices in range
            }
            pal.truncate(max_bytes);
            pal.truncate(pal.len() - pal.len() % 3); // whole RGB triples only
            out.extend_from_slice(&(pal.len() as u32).to_be_bytes());
            out.extend_from_slice(b"PLTE");
            out.extend_from_slice(&pal);
            out.extend_from_slice(&png_crc(b"PLTE", &pal).to_be_bytes());
            replaced = true;
        } else {
            out.extend_from_slice(&png[q..ds + len + 4]);
        }
        q = ds + len + 4;
    }
    replaced.then_some(out)
}

/// CRC-32 (PNG/zlib polynomial `0xEDB88320`) over a chunk's type bytes followed
/// by its data — the value PNG stores after each chunk's payload.
fn png_crc(ty: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in ty.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Test-only: build a minimal Blorb containing one `Pict` resource whose raw
/// bytes are `data`, at resource number `resnum` — for tests that need a
/// resolvable image without a full story file.
#[cfg(test)]
pub(crate) fn test_blorb_with_pict(resnum: u32, data: &[u8]) -> blorb::Blorb {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }
    let ridx_data_len = 4 + 12; // count + one 12-byte entry
    let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
    let pict_chunk = chunk(b"PNG ", data);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Pict");
    ridx.extend_from_slice(&resnum.to_be_bytes());
    ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
    let ridx_chunk = chunk(b"RIdx", &ridx);
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&ridx_chunk);
    inner.extend_from_slice(&pict_chunk);
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    blorb::Blorb::parse(file).expect("valid test blorb")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that chose the tent over a two-tap box on fixed column pairs
    /// (SQ-0797): a period-2 dither collapses to its exact mean at BOTH phases, so
    /// a run of `a b a b` and a run of `b a b a` come out as the same flat colour.
    /// A fixed pairing only collapses the half of the artwork that happens to be
    /// aligned with it, and nothing in the format aligns a dither to anything.
    #[test]
    fn the_column_tent_collapses_a_dither_at_either_phase() {
        let (a, b) = ([255u8, 85, 85, 255], [170u8, 85, 0, 255]); // EGA 12 and 6
        let mut even = image::RgbaImage::from_fn(8, 1, |x, _| {
            image::Rgba(if x % 2 == 0 { a } else { b })
        });
        let mut odd = image::RgbaImage::from_fn(8, 1, |x, _| {
            image::Rgba(if x % 2 == 0 { b } else { a })
        });
        blend_half_width_columns(&mut even);
        blend_half_width_columns(&mut odd);
        // Bronze: the mean of bright red and brown, which EGA does not hold.
        let bronze = image::Rgba([213u8, 85, 43, 255]);
        for x in 1..7 {
            assert_eq!(*even.get_pixel(x, 0), bronze, "even phase, column {x}");
            assert_eq!(*odd.get_pixel(x, 0), bronze, "odd phase, column {x}");
        }
    }

    /// Flat colour survives untouched, and so does a transparent pixel and the
    /// opaque pixel beside it — the tap that would have reached across a cut-out
    /// folds back onto the centre instead of averaging a fringe into it.
    #[test]
    fn the_column_tent_leaves_flat_colour_and_stencil_edges_alone() {
        let solid = image::Rgba([170u8, 85, 0, 255]);
        let hole = image::Rgba([0u8, 0, 0, 0]);
        let mut img = image::RgbaImage::from_fn(5, 1, |x, _| if x == 2 { hole } else { solid });
        blend_half_width_columns(&mut img);
        for x in [0u32, 1, 3, 4] {
            assert_eq!(*img.get_pixel(x, 0), solid, "column {x} keeps its own colour");
        }
        assert_eq!(*img.get_pixel(2, 0), hole, "a cut-out is never painted in");
    }

    #[test]
    fn arc_shares_while_unchanged_and_copies_on_write() {
        // arc() must be a cheap Arc share when the canvas hasn't changed (the
        // per-tick deep-clone this removes), and a later draw must NOT mutate a
        // frame already handed to the renderer (copy-on-write isolation). (SQ-0343)
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4); // red
        let snap = c.arc(); // renderer's frame this tick
        assert!(Arc::ptr_eq(&snap, &c.img), "arc() shares the bitmap, no deep copy");
        c.fill_rect(0x0000_00FF, 0, 0, 4, 4); // game draws blue next tick
        assert_eq!(snap.get_pixel(0, 0).0, [0xFF, 0, 0, 0xFF], "handed-out frame stays red");
        assert_eq!(c.img.get_pixel(0, 0).0, [0, 0, 0xFF, 0xFF], "live canvas is now blue");
        assert!(!Arc::ptr_eq(&snap, &c.img), "make_mut copied-on-write for the new draw");
    }

    #[test]
    fn fill_rect_paints_pixels_and_bumps_version() {
        let mut c = Canvas::new(10, 10);
        let v0 = c.version;
        c.fill_rect(0x00FF_0000, 2, 3, 4, 5); // red
        assert!(c.version > v0);
        let px = c.img.get_pixel(2, 3);
        assert_eq!(px.0, [0xFF, 0x00, 0x00, 0xFF]);
        // outside the rect stays transparent/default
        assert_ne!(c.img.get_pixel(9, 9).0, [0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn fill_rect_clips_out_of_bounds() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(0x0000_FF00, -2, -2, 100, 100); // green, way oversized
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0xFF, 0x00, 0xFF]);
        // no panic; whole canvas filled
        assert_eq!(c.img.get_pixel(3, 3).0, [0x00, 0xFF, 0x00, 0xFF]);
    }

    #[test]
    fn erase_uses_background_color() {
        let mut c = Canvas::new(4, 4);
        c.set_background(0x0000_00FF); // blue
        c.fill_rect(0x00FF_0000, 0, 0, 4, 4);
        c.erase_rect(0, 0, 2, 2);
        assert_eq!(c.img.get_pixel(0, 0).0, [0x00, 0x00, 0xFF, 0xFF]); // erased → bg
        assert_eq!(c.img.get_pixel(3, 3).0, [0xFF, 0x00, 0x00, 0xFF]); // untouched
    }

    #[test]
    fn draw_image_composites_scaled() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 1, 1, Some((4, 4)));
        assert_eq!(c.img.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(c.img.get_pixel(4, 4).0, [10, 20, 30, 255]); // scaled to 4x4
    }

    #[test]
    fn draw_image_clamps_absurd_scale() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut c = Canvas::new(8, 8);
        // A malicious/buggy game could request a ~4 exabyte scaled bitmap;
        // this must clamp to the canvas size instead of allocating it.
        c.draw_image(&image::DynamicImage::ImageRgba8(img), 0, 0, Some((1_000_000_000, 1_000_000_000)));
        assert_eq!(c.img.dimensions(), (8, 8));
        assert_eq!(c.img.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn pict_source_resolves_and_caches() {
        // No blorb → None.
        let mut none = PictSource::new(None);
        assert!(none.info(1).is_none());
        assert!(none.image(1).is_none());
    }

    /// A valid 2x2 red PNG, encoded via the `image` crate.
    fn png_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// Build a minimal Blorb carrying one `Data` resource at `resnum` with the
    /// given chunk type (`b"TEXT"` / `b"BINA"`) and raw bytes.
    fn test_blorb_with_data(resnum: u32, chunk_ty: &[u8; 4], data: &[u8]) -> blorb::Blorb {
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12;
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let data_chunk = chunk(chunk_ty, data);
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes());
        ridx.extend_from_slice(b"Data");
        ridx.extend_from_slice(&resnum.to_be_bytes());
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&data_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid test blorb")
    }

    #[test]
    fn data_resource_reads_text_and_binary_chunks() {
        // A TEXT chunk reports is_text=true; a BINA chunk false; a missing
        // number and a Blorb-less source both yield None.
        let src = PictSource::new(Some(test_blorb_with_data(3, b"TEXT", b"hello")));
        assert_eq!(src.data_resource(3), Some((b"hello".to_vec(), true)));
        assert_eq!(src.data_resource(4), None, "no such Data resource");

        let bin = PictSource::new(Some(test_blorb_with_data(1, b"BINA", &[1, 2, 3])));
        assert_eq!(bin.data_resource(1), Some((vec![1, 2, 3], false)));

        assert_eq!(PictSource::new(None).data_resource(1), None, "no blorb → None");
    }

    #[test]
    fn dims_and_all_pict_dims_header_sniff_without_full_decode() {
        // A tiny in-memory Blorb with one Pict (a 2x2 PNG) at resource number 5.
        let blorb = test_blorb_with_pict(5, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        assert_eq!(src.dims(5), Some((2, 2)));
        assert_eq!(src.dims(99), None, "no such resource");
        assert_eq!(src.all_pict_dims(), vec![(5u16, 2u16, 2u16)]);

        assert_eq!(PictSource::new(None).all_pict_dims(), Vec::<(u16, u16, u16)>::new());
    }

    #[test]
    fn image_hands_out_cheap_arc_clones_of_one_decode() {
        // SQ-0175 part B: `PictSource::image` must not deep-clone the decoded
        // `DynamicImage` on every draw — repeated calls for the same resnum
        // should return `Arc` clones pointing at the same allocation.
        let blorb = test_blorb_with_pict(1, &png_bytes());
        let mut src = PictSource::new(Some(blorb));
        let a = src.image(1).expect("resolves");
        let b = src.image(1).expect("resolves");
        assert!(Arc::ptr_eq(&a, &b), "both calls must share one cached decode");
        assert_eq!(a.dimensions(), (2, 2));
    }

    // ── Adaptive palettes (Blorb spec §11.3, SQ-0485) ───────────────────────

    /// zlib "stored" (uncompressed) wrapper so we can hand-build indexed PNGs
    /// without a compressor: header, one final stored block, adler32 trailer.
    fn zlib_store(raw: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(raw);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in raw {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn png_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(ty);
        v.extend_from_slice(data);
        v.extend_from_slice(&super::png_crc(ty, data).to_be_bytes());
        v
    }

    /// A `w`×`h` 4-bit indexed PNG. `rows[y][x]` = palette index; `palette` is
    /// RGB triples; `trns` optional per-index alpha. Filter-none scanlines.
    fn indexed_png(w: u32, h: u32, palette: &[u8], trns: Option<&[u8]>, rows: &[&[u8]]) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[4, 3, 0, 0, 0]); // bitdepth=4, indexed, comp/filter/interlace=0
        let mut raw = Vec::new();
        for row in rows {
            raw.push(0); // filter: none
            let mut x = 0usize;
            while x < w as usize {
                let hi = row[x] & 0xf;
                let lo = if x + 1 < w as usize { row[x + 1] & 0xf } else { 0 };
                raw.push((hi << 4) | lo);
                x += 2;
            }
        }
        let mut png = super::PNG_SIG.to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
        png.extend_from_slice(&png_chunk(b"PLTE", palette));
        if let Some(t) = trns {
            png.extend_from_slice(&png_chunk(b"tRNS", t));
        }
        png.extend_from_slice(&png_chunk(b"IDAT", &zlib_store(&raw)));
        png.extend_from_slice(&png_chunk(b"IEND", b""));
        png
    }

    /// The stored trailing CRC of the first chunk of type `ty` in a PNG.
    fn stored_crc(png: &[u8], ty: &[u8; 4]) -> Option<u32> {
        let mut q = 8;
        while q + 12 <= png.len() {
            let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
            let t = &png[q + 4..q + 8];
            let ds = q + 8;
            if ds + len + 4 > png.len() {
                break;
            }
            if t == ty {
                let c = &png[ds + len..ds + len + 4];
                return Some(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
            }
            q = ds + len + 4;
        }
        None
    }

    /// Build a Blorb with the given `(number, png_bytes)` Pict resources and an
    /// `APal` chunk listing `apal` as adaptive.
    fn blorb_apal(picts: &[(u32, &[u8])], apal: &[u32]) -> blorb::Blorb {
        fn iff(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let ridx_data_len = 4 + 12 * picts.len();
        let mut apal_bytes = Vec::new();
        for n in apal {
            apal_bytes.extend_from_slice(&n.to_be_bytes());
        }
        let apal_chunk = iff(b"APal", &apal_bytes);
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2) + apal_chunk.len();
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_n, data) in picts {
            offsets.push(cursor as u32);
            let c = iff(b"PNG ", data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&(picts.len() as u32).to_be_bytes());
        for (i, (n, _d)) in picts.iter().enumerate() {
            ridx.extend_from_slice(b"Pict");
            ridx.extend_from_slice(&n.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff(b"RIdx", &ridx));
        inner.extend_from_slice(&apal_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        blorb::Blorb::parse(file).expect("valid apal test blorb")
    }

    fn top_left(img: &DynamicImage) -> [u8; 4] {
        img.to_rgba8().get_pixel(0, 0).0
    }

    #[test]
    fn splice_plte_substitutes_palette_fixes_crc_and_decodes() {
        // 2×1, both pixels index 1. Placeholder idx1 = magenta.
        let png = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        assert_eq!(top_left(&crate::cover::decode(&png).unwrap()), [170, 0, 170, 255], "placeholder is magenta");
        // Current Palette: idx1 = green.
        let current = [0u8, 0, 0, 0, 170, 0];
        let spliced = super::splice_plte(&png, &current).expect("indexed PNG splices");
        assert_eq!(super::png_plte(&spliced).as_deref(), Some(&current[..]), "PLTE now the current palette");
        // CRC was recomputed over the new PLTE (not left stale).
        assert_eq!(stored_crc(&spliced, b"PLTE"), Some(super::png_crc(b"PLTE", &current)));
        // Decodes cleanly (CRC valid) to the substituted colour.
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [0, 170, 0, 255], "now green");
    }

    #[test]
    fn splice_plte_keeps_trailing_entries_when_current_is_shorter() {
        // Placeholder has 4 entries; the pixel uses index 3.
        let placeholder = [0, 0, 0, 10, 10, 10, 20, 20, 20, 200, 100, 50];
        let png = indexed_png(2, 1, &placeholder, None, &[&[3, 3]]);
        // Current Palette shorter (2 entries): index 3 would otherwise dangle.
        let spliced = super::splice_plte(&png, &[1, 2, 3, 4, 5, 6]).unwrap();
        let pal = super::png_plte(&spliced).unwrap();
        assert_eq!(pal.len(), placeholder.len(), "length kept so index 3 stays in range");
        assert_eq!(&pal[0..6], &[1, 2, 3, 4, 5, 6], "leading entries from the current palette");
        assert_eq!(&pal[9..12], &[200, 100, 50], "trailing placeholder entry retained");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [200, 100, 50, 255], "index 3 still resolves");
    }

    #[test]
    fn splice_plte_caps_current_palette_to_bit_depth_max() {
        let png = indexed_png(2, 1, &[0, 0, 0, 9, 9, 9], None, &[&[1, 1]]);
        // A 20-entry current palette exceeds the 16-entry (2^4) PLTE cap.
        let current: Vec<u8> = (0..20u8).flat_map(|i| [i, i, i]).collect();
        let spliced = super::splice_plte(&png, &current).unwrap();
        assert_eq!(super::png_plte(&spliced).unwrap().len(), 16 * 3, "capped to 16 entries");
        assert_eq!(top_left(&crate::cover::decode(&spliced).unwrap()), [1, 1, 1, 255], "idx1 → current[1]");
    }

    #[test]
    fn splice_and_plte_reject_non_indexed_png() {
        // A truecolor PNG has no PLTE; §11.3 derives the palette from PLTE, so
        // there is nothing to substitute and the adaptive path decodes it as-is.
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([9, 8, 7]));
        let mut rgb = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut rgb), image::ImageFormat::Png)
            .unwrap();
        assert!(super::png_plte(&rgb).is_none(), "truecolor PNG has no PLTE");
        assert!(super::splice_plte(&rgb, &[1, 2, 3]).is_none(), "nothing to splice");
    }

    #[test]
    fn adaptive_picture_uses_current_palette_and_reacts_to_palette_change() {
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let base_red = indexed_png(2, 1, &[0, 0, 0, 200, 0, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]); // placeholder magenta
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive), (3, &base_red)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        assert!(src.adaptive.contains(&2) && !src.adaptive.contains(&1), "APal set parsed");

        // (a) Adaptive drawn before any base is undefined per §11.3 → placeholder.
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "no base yet → own placeholder");

        // (b) Draw the green base, then the adaptive: it takes the green palette.
        src.image(1).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [0, 170, 0, 255], "plotted with current (green) palette");

        // (c) A different base re-establishes the palette; the SAME adaptive
        //     picture re-decodes (cache keyed by palette generation).
        src.image(3).unwrap();
        assert_eq!(top_left(&src.image(2).unwrap()), [200, 0, 0, 255], "palette change re-decodes adaptive");
    }

    #[test]
    fn size_queries_do_not_establish_the_palette() {
        // `info`/`dims` must not count as "drawing": querying a base picture's
        // size must NOT set the Current Palette that later adaptive draws use.
        let base_green = indexed_png(2, 1, &[0, 0, 0, 0, 170, 0], None, &[&[1, 1]]);
        let adaptive = indexed_png(2, 1, &[0, 0, 0, 170, 0, 170], None, &[&[1, 1]]);
        let blorb = blorb_apal(&[(1, &base_green), (2, &adaptive)], &[2]);
        let mut src = PictSource::new(Some(blorb));
        src.info(1); // size query on the base — not a draw
        src.dims(1);
        assert!(src.current_plte.is_none(), "a size query must not establish the palette");
        assert_eq!(top_left(&src.image(2).unwrap()), [170, 0, 170, 255], "adaptive still on its placeholder");
    }
}
