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
//! 1. **A full-screen plate.** A picture covering at least nine tenths of the
//!    picture space on both axes, painted at `(0, 0)`. This is how the
//!    Amiga/Macintosh archives ship a border — `Pic.data`'s ids 5, 6 and 7 are
//!    480x300 plates carrying top strip and both pillars together — and the
//!    renderer's flank is a left or right crop of it, so this crops one too.
//! 2. **A strip over a column.** A full-width picture and a narrow one whose unit
//!    heights sum to the picture space EXACTLY, strip at `(0, 0)` and column at
//!    `(0, strip height)`. That is `DISPLAY_BORDER`'s own composition, and the
//!    sum is what discovers it: `zork0.mg1`'s id 5 is 320x34 and its castle
//!    pillar 166, its underground strip 39 and pillar 161, and so on for every
//!    scene and every PC rendition, with no id written down here.
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
    std::env::temp_dir().join(format!("babelmap-archive-sweep-{}", std::process::id()))
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
        Archive::wrap(label.to_string(), PictSource::resolve_with_override(story, over))
    }

    /// The archive a disk image supplies by itself — the Amiga floppies' own
    /// `Pic.data`, and the Macintosh's COLOUR `CPic.data`.
    pub fn medium(image: &str) -> Option<Archive> {
        let path = stories().join(image);
        if !path.exists() {
            eprintln!("SKIP: gitignored medium {image} is absent");
            return None;
        }
        Archive::wrap(image.to_string(), PictSource::resolve(&path))
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
    pub canvas: RgbaImage,
    pub x0: u32,
    pub x1: u32,
    pub art: (u32, u32),
}

/// The flank widths cropped out of a full-screen plate: Shogun's measured 46
/// unit columns and Zork Zero's 86. The archive cannot state this — the crop is
/// where the story window starts — so both of the corpus's are swept.
pub const FLANK_WIDTHS: &[u32] = &[46, 86];

/// A column narrow enough to be a pillar rather than a plate, in unit columns.
/// Zork Zero's widest is 86 and Shogun's 60; 100 clears both without admitting
/// anything that could carry a scene.
pub const COLUMN_MAX: u32 = 100;

/// Every flank the archive itself states — see the module doc for the two
/// arrangements and why there are only two.
pub fn flanks(a: &mut Archive) -> Vec<Flank> {
    let dims: Vec<(u16, u32, u32)> = a
        .src
        .all_pict_dims()
        .into_iter()
        .map(|(id, w, h)| (id, u32::from(w) * a.scale.0, u32::from(h) * a.scale.1))
        .filter(|&(_, w, h)| w > 0 && h > 0)
        .collect();
    let (sw, sh) = a.art_screen;
    let is_wide = |w: u32| w * 10 >= sw * 9;
    let plates: Vec<u16> =
        dims.iter().filter(|&&(_, w, h)| is_wide(w) && h * 10 >= sh * 9).map(|d| d.0).collect();
    let strips: Vec<(u16, u32)> =
        dims.iter().filter(|&&(_, w, h)| is_wide(w) && h * 10 < sh * 9).map(|&(id, _, h)| (id, h)).collect();
    let columns: Vec<(u16, u32, u32)> =
        dims.iter().copied().filter(|&(_, w, h)| w <= COLUMN_MAX && h * 3 >= sh).collect();

    let mut out = Vec::new();
    for id in plates {
        let Some(img) = a.unit_image(id) else { continue };
        let mut canvas = RgbaImage::new(a.native.0, a.native.1);
        blit(&mut canvas, &img, 0, 0);
        for &fw in FLANK_WIDTHS {
            for (side, x0, x1) in [("left", 0, fw), ("right", a.native.0 - fw, a.native.0)] {
                let art = border::art_extent(&canvas, x0, x1);
                out.push(Flank {
                    what: format!("plate {id}, {side} flank {fw} wide"),
                    canvas: canvas.clone(),
                    x0,
                    x1,
                    art,
                });
            }
        }
    }
    for &(sid, strip_h) in &strips {
        for &(cid, cw, ch) in &columns {
            // `DISPLAY_BORDER`'s composition, found by the one thing the archive
            // states about it: the two pieces tile the picture space exactly.
            if strip_h + ch != sh {
                continue;
            }
            let (Some(top), Some(col)) = (a.unit_image(sid), a.unit_image(cid)) else { continue };
            let mut canvas = RgbaImage::new(a.native.0, a.native.1);
            blit(&mut canvas, &top, 0, 0);
            blit(&mut canvas, &col, 0, strip_h);
            let art = border::art_extent(&canvas, 0, cw);
            out.push(Flank { what: format!("strip {sid} over column {cid}"), canvas, x0: 0, x1: cw, art });
        }
    }
    out
}

pub fn blit(dst: &mut RgbaImage, src: &RgbaImage, x0: u32, y0: u32) {
    for y in 0..src.height().min(dst.height().saturating_sub(y0)) {
        for x in 0..src.width().min(dst.width().saturating_sub(x0)) {
            dst.put_pixel(x0 + x, y0 + y, *src.get_pixel(x, y));
        }
    }
}
