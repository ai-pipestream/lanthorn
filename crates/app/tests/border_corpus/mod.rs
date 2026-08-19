//! Discovery of the v6 border-flank corpus: every flank an ARCHIVE states on
//! its own, found by shape, with no picture id written down anywhere.
//!
//! This was SQ-0842's, and it lives here because it now has two consumers that
//! must not drift apart:
//!
//! * `tests/suites/v6_archive_border_sweep.rs` — asserts the composition's
//!   properties over the whole corpus;
//! * `examples/border_preview.rs` — composes the same flanks and writes them out
//!   as PNGs, so a border set can be LOOKED at without playing to its room.
//!
//! A preview that discovered its flanks differently from the sweep would be
//! previewing something the sweep never checked, which is the one thing a
//! developer tool beside a test must not do.
//!
//! ## The two flank geometries an ARCHIVE can state on its own
//!
//! A flank is a screen REGION, not a picture, so a candidate has to be painted
//! into a frame before `recognize` can see it. Only two arrangements are
//! derivable from the archive itself, and both are the game's own:
//!
//! 1. **A full-screen plate with a GUTTER.** A picture covering at least nine
//!    tenths of the picture space on both axes, painted at `(0, 0)`, which leaves
//!    a run of columns clear below its own top strip. This is how the
//!    Amiga/Macintosh archives ship a border — `Pic.data`'s ids 5, 6, 7 and 8 are
//!    480x300 plates carrying top strip and both pillars together — and the
//!    gutter is the plate SAYING where the story window goes, which is the only
//!    thing that can fix a flank's width. See [`plate_gutter`].
//! 2. **A strip over a column.** A full-width picture and a narrow one whose unit
//!    heights sum to the picture space EXACTLY, strip at `(0, 0)` and column at
//!    `(0, strip height)`. That is `DISPLAY_BORDER`'s own composition, and the
//!    sum is what discovers it: `zork0.mg1`'s id 5 is 320x34 and its castle
//!    pillar 166, its underground strip 39 and pillar 161, and so on for every
//!    scene and every PC rendition, with no id written down here. Here the column
//!    picture's own width IS the flank width.
//!
//! **A full-screen plate with no gutter states no flank**, and is counted rather
//! than cropped — see [`Discovered::stateless_plates`] and the sweep suite's
//! module doc for what that costs and why it is the honest number.
//!
//! **Arthur's poles are not reconstructable** and deliberately absent: they do
//! not tile the picture space (native rows 11..379 of 400) and the height they
//! hang at is a Z-machine quantity the archive does not carry, so neither rule
//! reaches them. See the sweep suite's module doc for why placing a lone column
//! at an invented offset was tried and rejected.

// Each consumer uses a different slice of this module, and neither should have
// to name the other's parts to keep the build quiet.
#![allow(dead_code)]

use std::path::PathBuf;

use app::graphics::{PictSource, PictureOverride};
use app::render::v6_border as border;
use image::RgbaImage;

/// Where the gitignored fixtures live.
pub fn stories() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A scratch directory for the per-game sidecar `resolve_with_session` looks
/// for. Nothing is ever written to it — the session name outranks the key — but
/// the door takes a directory and this is an honest one.
pub fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("lanthorn-archive-sweep-{}", std::process::id()))
}

// ── The corpus ───────────────────────────────────────────────────────────────

/// One archive, with the geometry the app derives from it.
pub struct Archive {
    pub label: String,
    src: PictSource,
    /// `(width, height)` of the picture space the archive declares.
    pub space: (u32, u32),
    /// The per-axis factor the app scales this archive's art by.
    pub scale: (u32, u32),
    /// The screen the art itself covers — picture space times scale.
    pub art_screen: (u32, u32),
    /// The screen the GAME is told it has: `art_screen` rounded to a whole v6
    /// text cell. The two differ on exactly one rendition in the corpus, and
    /// that difference was SQ-0841.
    pub native: (u32, u32),
}

impl Archive {
    /// Wrap a resolved source, or `None` when it carries no native archive.
    pub fn wrap(label: String, src: PictSource) -> Option<Archive> {
        let space = src.native_std_window()?;
        let space = (u32::from(space.0), u32::from(space.1));
        let scale = src.art_scale()?;
        let art_screen = (space.0 * scale.0, space.1 * scale.1);
        // `session.rs`: the screen is rounded to the NEAREST whole cell, because
        // rounding 300 down would tell the game its screen is 288 pixels tall
        // and clip the bottom twelve rows off its own artwork.
        let cells = |px: u32, cell: u32| ((px + cell / 2) / cell).clamp(1, 255) * cell;
        let native = (
            cells(art_screen.0, u32::from(zvm::screen::V6_FONT_WIDTH)),
            cells(art_screen.1, u32::from(zvm::screen::V6_FONT_HEIGHT)),
        );
        Some(Archive { label, src, space, scale, art_screen, native })
    }

    /// A loose archive beside the stories — the PC renditions, reached through
    /// the same tier-3 door a player uses, so a multi-part `.EG1`/`.EG2` set is
    /// absorbed here exactly as it is at launch.
    pub fn loose(name: &str) -> Option<Archive> {
        Archive::named(&stories().join("any-story-in-this-directory"), name, name)
    }

    /// An archive INSIDE a disk image, by name — the `--pictures Pic.data` door.
    pub fn inside(image: &str, name: &str) -> Option<Archive> {
        let path = stories().join(image);
        if !path.exists() {
            return None;
        }
        Archive::named(&path, name, &format!("{image} [{name}]"))
    }

    pub fn named(story: &std::path::Path, name: &str, label: &str) -> Option<Archive> {
        let dir = scratch();
        let _ = std::fs::create_dir_all(&dir);
        let over = PictureOverride::resolve_with_session(story, &dir, Some(name));
        if !matches!(over, PictureOverride::Loaded { .. }) {
            eprintln!("SKIP: gitignored picture archive {label} is absent or will not parse");
            return None;
        }
        Archive::wrap(label.to_string(), PictSource::resolve_with_override(story, over, None))
    }

    /// The archive a disk image supplies by itself — the Amiga floppies' own
    /// `Pic.data`, and the Macintosh's COLOUR `CPic.data`.
    pub fn medium(image: &str) -> Option<Archive> {
        let path = stories().join(image);
        if !path.exists() {
            eprintln!("SKIP: gitignored medium {image} is absent");
            return None;
        }
        Archive::wrap(image.to_string(), PictSource::resolve(&path, None))
    }

    /// Every picture the archive carries, in UNIT space: `(id, width, height)`,
    /// empty ones dropped.
    pub fn unit_dims(&mut self) -> Vec<(u16, u32, u32)> {
        self.src
            .all_pict_dims()
            .into_iter()
            .map(|(id, w, h)| (id, u32::from(w) * self.scale.0, u32::from(h) * self.scale.1))
            .filter(|&(_, w, h)| w > 0 && h > 0)
            .collect()
    }

    /// Picture `id` in UNIT space — the art's own pixels replicated by the
    /// per-axis scale, which is how it reaches the graphics canvas.
    pub fn unit_image(&mut self, id: u16) -> Option<RgbaImage> {
        let art = self.src.image(u32::from(id))?.to_rgba8();
        let (sx, sy) = self.scale;
        let mut out = RgbaImage::new(art.width() * sx, art.height() * sy);
        for y in 0..out.height() {
            for x in 0..out.width() {
                out.put_pixel(x, y, *art.get_pixel(x / sx, y / sy));
            }
        }
        Some(out)
    }
}

// ── Discovery ────────────────────────────────────────────────────────────────

/// A flank as the renderer hands it to `border::flank_source`: a screen region,
/// with the art's opaque extent over its own columns.
pub struct Flank {
    pub what: String,
    /// The picture(s) this flank was composed from — both flanks of one plate
    /// share it, so a caller can check that a symmetric border came out
    /// symmetric.
    pub source: String,
    /// `"left"`, `"right"`, or `"only"` for an arrangement that states one side.
    pub side: &'static str,
    pub canvas: RgbaImage,
    pub x0: u32,
    pub x1: u32,
    pub art: (u32, u32),
}

/// How narrow a picture must be to be a pillar rather than a scene, as a
/// fraction of **this archive's own screen width** — a sixth of it.
///
/// This was `100` unit columns flat (SQ-0842), which is a 640-screen number: it
/// admits 15.6% of an EGA screen and 20.8% of the Macintosh's 480-wide one. As a
/// fraction it is the same statement on every picture space, and it moves
/// nothing in the corpus — a sixth of 640 is 106 against the old 100, and the
/// widest column any archive here offers is Zork Zero's 86, so the set of
/// pictures admitted is identical on every 640-wide rendition and the 480-wide
/// one gains nothing it can pair with.
fn column_max(screen_w: u32) -> u32 {
    screen_w / 6
}

/// The story-window **gutter** a full-screen plate declares: `(left, right)`
/// unit columns of flank, or `None` when the plate paints its whole width and so
/// states no story window at all.
///
/// This is the number `FLANK_WIDTHS` used to guess, and guessing it is the whole
/// of SQ-0845. A border plate is a top strip with two pillars hanging under it,
/// so BELOW the strip its middle is clear — and the longest run of columns that
/// are clear over the plate's lower half is exactly the rectangle the story
/// window is about to be opened in. The run must be most of the plate's width,
/// because a story window is: that is what separates a border from an
/// illustration with a ragged edge, and it is what makes an illustration return
/// `None` here rather than a one-column notch.
///
/// **Measured against the games themselves.** Booting each medium to a gameplay
/// frame and reading the story window's own `x_px` — the `TEXT_WINDOW_PIC_LOC`
/// the game sets, which is the flank width by definition — gives:
///
/// | medium (release / serial)                        | picture space | widest gutter | story `x_px` |
/// |--------------------------------------------------|---------------|---------------|--------------|
/// | `Zork Zero - The Revenge of Megaboz.adf` (366/890323) | 320x200   | 86            | 88           |
/// | `Zork Zero Disk.image` (296/881019)              | 320x200       | 86            | **86**       |
/// | `Zork Zero Disk.image [Pic.data]` (296/881019)   | **480x300**   | **53**        | **61**       |
/// | `James Clavell's Shogun.adf` (295/890321)        | 320x200       | 60            | 46           |
///
/// The gutter is the ART's own edge and `x_px` is where the game puts the window,
/// so the two differ by the margin the artist left — and on Shogun by more, in
/// the other direction, because its widest plate is an alternative border style
/// 60 columns wide while the window it opens for the frame it usually draws is
/// 46. Neither gap changes what a flank IS: [`border::recognize`] returns the
/// same layout at the gutter as at `x_px` on all **thirty** flanks those four
/// media state, because a crop that already holds the art can only gain clear
/// columns and `painted_widths` reads the painted span alone.
///
/// **What it must not be is per plate.** Zork Zero's four Amiga scene borders
/// gutter at 72, 86, 84 and 60 while the game opens one window for all four, and
/// cropping picture 8 at its own 60 cuts the top strip that makes it a pillar —
/// it reads `ShogunSinglePiece` at 60 and `ZorkZeroPillars` at the archive's
/// widest 86, which is the in-game answer. So the width an archive states is the
/// WIDEST gutter any of its border plates declares, and every plate is cropped at
/// that.
pub fn plate_gutter(img: &RgbaImage) -> Option<(u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let clear = |x: u32| !(h / 2..h).any(|y| img.get_pixel(x, y)[3] >= 128);
    let (mut best, mut run) = ((0u32, 0u32), (0u32, 0u32));
    for x in 0..w {
        if !clear(x) {
            continue;
        }
        if run.1 != x {
            run = (x, x);
        }
        run.1 = x + 1;
        if run.1 - run.0 > best.1 - best.0 {
            best = run;
        }
    }
    ((best.1 - best.0) * 2 >= w).then_some((best.0, w - best.1))
}

/// What one archive states: the flanks it can compose, and what it could not.
pub struct Discovered {
    /// Every flank the archive states, at the width it states.
    pub flanks: Vec<Flank>,
    /// Full-screen plates carrying no story-window gutter. These are the
    /// archive's ILLUSTRATIONS — they paint their whole width, declare no flank,
    /// and are counted here rather than cropped at an invented one.
    pub stateless_plates: usize,
    /// The flank width this archive states, in its own unit columns — see
    /// [`flanks`]. `None` when it states no border piece at all.
    pub stated_width: Option<u32>,
}

/// Every flank the archive itself states — see the module doc for the two
/// arrangements and why there are only two.
///
/// ## One width per archive, and it is the widest border piece the archive has
///
/// A game opens ONE story window and its border is symmetric about it, so an
/// archive has exactly one flank width and every piece of border art in it has to
/// fit inside that width. So the width is the widest piece the archive states —
/// the widest [`plate_gutter`], or the widest column picture that pairs with a
/// strip — and every flank is cropped at it, whichever arrangement it came from.
///
/// **Cropping a piece at its own width is the same bug as cropping it at another
/// machine's**, and this is where it bites the arrangement that looked safe. The
/// column pictures pairing with `zork0.mg1`'s jungle strip are 72, 74, 84 and 86
/// unit columns wide, but the strip above them is full-width: crop the 72-wide one
/// at 72 and the banner over it is cut to the same 72, so the flank holds one
/// width top to bottom and `recognize` calls it Shogun's slab. At the archive's
/// own 86 the banner is wider than the shaft and it is the pillar it always was.
/// `painted_widths` exists to see a banner wider than the column under it, and a
/// crop pinned to the column cannot show it one.
///
/// Measured against the games: the widest piece is **86** on all three of Zork
/// Zero's PC archives, which is `zork0-r393-s890714.z6`'s own story window `x_px`
/// exactly, and **58** on `shogun.cg1` against its window at 57.
pub fn flanks(a: &mut Archive) -> Discovered {
    let dims = a.unit_dims();
    let (sw, sh) = a.art_screen;
    let is_wide = |w: u32| w * 10 >= sw * 9;
    let plates: Vec<u16> =
        dims.iter().filter(|&&(_, w, h)| is_wide(w) && h * 10 >= sh * 9).map(|d| d.0).collect();
    let strips: Vec<(u16, u32)> =
        dims.iter().filter(|&&(_, w, h)| is_wide(w) && h * 10 < sh * 9).map(|&(id, _, h)| (id, h)).collect();
    let columns: Vec<(u16, u32, u32)> =
        dims.iter().copied().filter(|&(_, w, h)| w <= column_max(sw) && h * 3 >= sh).collect();

    // Which plates declare a story window…
    let bordered: Vec<(u16, u32, u32)> = plates
        .iter()
        .filter_map(|&id| {
            let (l, r) = plate_gutter(&a.unit_image(id)?)?;
            Some((id, l, r))
        })
        .collect();
    // …and which strip-over-column pairs the archive's heights admit.
    // `DISPLAY_BORDER`'s composition, found by the one thing the archive states
    // about it: the two pieces tile the picture space exactly.
    let pairs: Vec<(u16, u32, u16, u32)> = strips
        .iter()
        .flat_map(|&(sid, strip_h)| {
            columns
                .iter()
                .filter(move |&&(_, _, ch)| strip_h + ch == sh)
                .map(move |&(cid, cw, _)| (sid, strip_h, cid, cw))
        })
        .collect();
    let stated_width = bordered
        .iter()
        .map(|&(_, l, r)| l.max(r))
        .chain(pairs.iter().map(|&(_, _, _, cw)| cw))
        .max();
    let Some(fw) = stated_width else {
        return Discovered { flanks: Vec::new(), stateless_plates: plates.len(), stated_width };
    };

    let mut out = Vec::new();
    for &(id, _, _) in &bordered {
        let Some(img) = a.unit_image(id) else { continue };
        let mut canvas = RgbaImage::new(a.native.0, a.native.1);
        blit(&mut canvas, &img, 0, 0);
        // The right flank is taken at the PLATE's own right edge, not the
        // screen's: a plate narrower than the picture space (Arthur's 584- and
        // 632-wide ones) would otherwise put its right crop past the art, which
        // is not a flank of anything.
        let pw = img.width().min(a.native.0);
        for (side, x0, x1) in [("left", 0, fw.min(pw)), ("right", pw.saturating_sub(fw), pw)] {
            if x1 <= x0 {
                continue;
            }
            let art = border::art_extent(&canvas, x0, x1);
            out.push(Flank {
                what: format!("plate {id}, {side} flank {} wide", x1 - x0),
                source: format!("plate {id}"),
                side,
                canvas: canvas.clone(),
                x0,
                x1,
                art,
            });
        }
    }
    for &(sid, strip_h, cid, _) in &pairs {
        let (Some(top), Some(col)) = (a.unit_image(sid), a.unit_image(cid)) else { continue };
        let mut canvas = RgbaImage::new(a.native.0, a.native.1);
        blit(&mut canvas, &top, 0, 0);
        blit(&mut canvas, &col, 0, strip_h);
        let x1 = fw.min(canvas.width());
        let art = border::art_extent(&canvas, 0, x1);
        let what = format!("strip {sid} over column {cid}");
        out.push(Flank { source: what.clone(), what, side: "only", canvas, x0: 0, x1, art });
    }
    Discovered { flanks: out, stateless_plates: plates.len() - bordered.len(), stated_width }
}

pub fn blit(dst: &mut RgbaImage, src: &RgbaImage, x0: u32, y0: u32) {
    for y in 0..src.height().min(dst.height().saturating_sub(y0)) {
        for x in 0..src.width().min(dst.width().saturating_sub(x0)) {
            dst.put_pixel(x0 + x, y0 + y, *src.get_pixel(x, y));
        }
    }
}
