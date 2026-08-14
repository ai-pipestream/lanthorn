//! SQ-0842 — sweep every picture ARCHIVE for flank art and assert the border
//! composition's properties, so a format-specific defect cannot hide in a room
//! nobody plays to.
//!
//! ## Why this exists
//!
//! SQ-0841's root cause was one rounding assumption. [`recognize`] asked whether
//! a flank's art reached the native screen bottom EXACTLY, and a v6 screen is the
//! archive's picture space rounded UP to a whole 8x16 cell — so the standard
//! Macintosh's 480x300 monochrome plate, laid on a **304**-row screen, missed by
//! four pixels. The flank was read as Shogun's single-piece slab, `shogun()`
//! stamped a mirrored copy of the whole column below the first, and the foot
//! ended up stranded mid-column with bare shaft beneath it.
//!
//! **Every other rendition divides exactly, so that test had never been wrong in
//! its life.** It was found by eye, in one room, on one border set, on one newly
//! supported format. `v6_side_border_tiling.rs` and `v6_mac_pillar_feet.rs` both
//! drive real games, and a real game only ever shows the rooms someone walked
//! to. This suite exists so the siblings of that defect cannot hide.
//!
//! ## What it does instead
//!
//! The composition is a PURE FUNCTION of the art and the target height, so the
//! art comes out of the ARCHIVE rather than off a live screen. Every archive in
//! `stories/` is opened through the app's own tier-3 door
//! ([`PictureOverride::resolve_with_session`], which also reads a name off a disk
//! image), every picture in it is measured, and the ones that can be a flank are
//! composed and asserted. Nobody names a picture id anywhere in this file: the
//! vines, the ice and every other border variant are found by shape.
//!
//! ## The two flank geometries an ARCHIVE can state on its own
//!
//! Discovery itself lives in [`border_corpus`], beside this file, because
//! `examples/border_preview.rs` composes the same flanks into PNGs and a preview
//! that found its flanks differently would be previewing art the sweep never
//! checked. Both arrangements — a full-screen plate cropped at 46 and 86 unit
//! columns, and a full-width strip over a narrow column whose unit heights sum
//! to the picture space EXACTLY — are documented there, as is why Arthur's poles
//! are not reconstructable from an archive alone.
//!
//! ## The properties, and which of them are universal
//!
//! Asserted for every discovered flank at three sampled band heights:
//!
//! 1. **The band is filled exactly.** `flank_source` returns the rows it was
//!    asked for and the last of them carries ink.
//! 2. **No hole in the extension.** Every row below the art's own bottom is
//!    painted. This is SQ-0698's 64-row black band, stated for the corpus.
//! 3. **The extension is the ART.** Every extension row's opaque span occurs
//!    among the art's own row spans — the composer stamps rows, whole, so a
//!    stretch (SQ-0511), a shift or a resample cannot pass. A vertical flip
//!    preserves a row's span, so the mirrored arms satisfy this exactly.
//! 4. **Four pixels of cell rounding change nothing.** [`recognize`] returns the
//!    same layout, and `flank_source` the same pixels, whether the screen is the
//!    art's own picture space or that space rounded up to a whole text cell.
//!    **This is SQ-0841's root cause stated as a property**, and it is the one
//!    assertion here that could have caught it before a person did.
//! 5. **A pillar ends on its foot.** Conditional, and deliberately so:
//!    `tile_down`'s own doc records that not every flank has one — *"Arthur needs
//!    neither… Zork Zero's patterned masonry is the case they were written
//!    for"*. Where the layout is one of the two arms that stamp a foot, the
//!    composition's last row is the art's own last row, pixel for pixel. Shogun's
//!    slab tiles to the bottom instead and is exempt.
//!
//! …plus a per-archive **classification tally**, pinned. That inventory is half
//! the value of the sweep: it says which arm of [`recognize`] each rendition
//! exercises, and it is what tells a later reader that a newly supported format
//! has art no arm handles. It is also the second thing SQ-0841 moves — restoring
//! the exact bottom test turns all 72 Macintosh monochrome flanks into slabs.
//!
//! Every fixture is gitignored, so each case skips vacuously without it.

use app::render::v6_border as border;
use app::render::v6_border::BorderArt;
use image::RgbaImage;

#[path = "../border_corpus/mod.rs"]
mod border_corpus;

use border_corpus::{Archive, flanks};

/// Columns `[x0, x1)` of `src`, `h` rows deep — the flank's own strip.
fn crop(src: &RgbaImage, x0: u32, x1: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(x1.min(src.width()) - x0, h.min(src.height()));
    for y in 0..out.height() {
        for x in 0..out.width() {
            out.put_pixel(x, y, *src.get_pixel(x0 + x, y));
        }
    }
    out
}

/// The opaque column span of every row, or `None` where nothing is painted —
/// the shape of a column reduced to one number per row.
fn spans(img: &RgbaImage) -> Vec<Option<(u32, u32)>> {
    (0..img.height())
        .map(|y| {
            let mut first = None;
            let mut last = 0;
            for x in 0..img.width() {
                if img.get_pixel(x, y)[3] >= 128 {
                    first.get_or_insert(x);
                    last = x;
                }
            }
            first.map(|f| (f, last))
        })
        .collect()
}

// ── The properties ───────────────────────────────────────────────────────────

/// How many flanks of each layout an archive yielded: `(pillars, slabs, poles,
/// unrecognised)`. Pinned per archive — see the module doc.
#[derive(Default, PartialEq, Eq, Debug)]
struct Tally {
    pillars: usize,
    slabs: usize,
    poles: usize,
    unrecognised: usize,
}

/// The band heights sampled. Deliberately not multiples of one another and not
/// multiples of the screen: a composition that only lands its foot when the
/// extension divides evenly passes a single height perfectly.
fn sampled_heights(native_h: u32) -> [u32; 3] {
    [native_h + 97, 2 * native_h + 13, 3 * native_h - 41]
}

/// Sweep one archive: compose every flank it states, at every sampled height,
/// and assert the five properties. Returns the classification tally.
fn sweep(a: &mut Archive) -> Tally {
    let label = a.label.clone();
    let (native_h, art_h) = (a.native.1, a.art_screen.1);
    let mut tally = Tally::default();
    for f in flanks(a) {
        let kind = border::recognize(&f.canvas, f.x0, f.x1, f.art, native_h);
        match kind {
            Some(BorderArt::ZorkZeroPillars) => tally.pillars += 1,
            Some(BorderArt::ShogunSinglePiece) => tally.slabs += 1,
            Some(BorderArt::ArthurPoles) => tally.poles += 1,
            None => tally.unrecognised += 1,
        }

        // ── 4a. The layout is a property of the ART, not of the cell grid ────
        assert_eq!(
            kind,
            border::recognize(&f.canvas, f.x0, f.x1, f.art, art_h),
            "{label}: {} — the art is rows {:?} of a {art_h}-row picture space, which the v6 cell \
             grid rounds to {native_h}. Those {} pixels are a property of the FONT, and they must \
             not decide what the border IS (SQ-0841)",
            f.what,
            f.art,
            native_h.abs_diff(art_h),
        );
        let Some(kind) = kind else { continue };

        let flank = crop(&f.canvas, f.x0, f.x1, native_h);
        let art_rows = spans(&flank);
        let art_spans: std::collections::HashSet<(u32, u32)> = art_rows.iter().copied().flatten().collect();

        for d in sampled_heights(native_h) {
            let out = border::flank_source(&f.canvas, &f.canvas, f.x0, f.x1, f.art, native_h, 0, d)
                .unwrap_or_else(|| {
                    panic!("{label}: {} — a {d}-row band over {:?} of art must be extended", f.what, f.art)
                });
            let sp = spans(&out);

            // ── 1. The band is filled exactly ────────────────────────────────
            assert_eq!(out.height(), d, "{label}: {} — a {d}-row band comes back {d} rows deep", f.what);
            assert_eq!(
                sp.iter().rposition(|s| s.is_some()),
                Some(d as usize - 1),
                "{label}: {} at {d} rows — the last row of the band must carry ink, or the frame \
                 stops short of the pane's own edge",
                f.what,
            );

            // ── 2. No hole in the extension ──────────────────────────────────
            let blank: Vec<u32> = (f.art.1..d).filter(|&y| sp[y as usize].is_none()).collect();
            assert!(
                blank.is_empty(),
                "{label}: {} at {d} rows — {} transparent row(s) below the art's own bottom ({}), \
                 first at {:?}. A hole in a border is the SQ-0698 black band",
                f.what,
                blank.len(),
                f.art.1,
                blank.first(),
            );

            // ── 3. The extension is the art, stamped ─────────────────────────
            let alien: Vec<u32> = (f.art.1..d)
                .filter(|&y| sp[y as usize].is_some_and(|s| !art_spans.contains(&s)))
                .collect();
            assert!(
                alien.is_empty(),
                "{label}: {} at {d} rows — {} extension row(s) whose opaque span the art itself \
                 never has, first at {:?} spanning {:?}. The composer stamps whole rows, so an \
                 unfamiliar span means the art was stretched or shifted rather than repeated",
                f.what,
                alien.len(),
                alien.first(),
                alien.first().map(|&y| sp[y as usize]),
            );

            // ── 4b. …and the same is true of the composition, not just the
            //        classification. Skipped where the two screens are the same
            //        number, which makes the comparison f(x) against f(x).
            if native_h != art_h {
                assert_eq!(
                    Some(&out),
                    border::flank_source(&f.canvas, &f.canvas, f.x0, f.x1, f.art, art_h, 0, d).as_ref(),
                    "{label}: {} at {d} rows — the composition differs between the art's own \
                     {art_h}-row screen and the {native_h} rows the cell grid rounds it to (SQ-0841)",
                    f.what,
                );
            }

            // ── 5. A pillar ends on its foot ─────────────────────────────────
            //
            // Conditional on the layout, not on the art: Shogun's slab has no
            // separate base to stamp and tiles to the bottom instead, which is
            // what `tile_down`'s own doc records. Both pillar arms end on a
            // whole copy of the art's tail, so the band's last row is the art's.
            if kind != BorderArt::ShogunSinglePiece {
                let foot: Vec<[u8; 4]> = (0..out.width()).map(|x| out.get_pixel(x, d - 1).0).collect();
                let want: Vec<[u8; 4]> =
                    (0..flank.width()).map(|x| flank.get_pixel(x, f.art.1 - 1).0).collect();
                assert_eq!(
                    foot,
                    want,
                    "{label}: {} at {d} rows — a {kind:?} flank ends on its FOOT, so the band's \
                     last row is the art's own last row (native {}). It spans {:?} where the art \
                     spans {:?} — bare shaft below the foot is SQ-0841 as the user reported it",
                    f.what,
                    f.art.1 - 1,
                    sp[d as usize - 1],
                    art_rows[f.art.1 as usize - 1],
                );
            }
        }
    }
    tally
}

/// Sweep a list of archives and check each one's tally against its pin. Skips
/// vacuously — and says so — when nothing in the list is present.
///
/// Every archive is swept BEFORE any tally is compared, deliberately: the
/// per-flank properties name the flank and the row that broke, and a tally only
/// ever says a number moved. A regression should be reported by the sharpest
/// assertion that sees it, not by whichever archive happens to come first.
fn sweep_all(corpus: &[(&str, Opener)], pins: &[(&str, Tally)]) {
    let mut got: Vec<Swept> = Vec::new();
    for ((name, open), (pinned_name, _)) in corpus.iter().zip(pins) {
        assert_eq!(name, pinned_name, "the corpus and its pins must stay in step");
        let Some(mut a) = open() else { continue };
        let tally = sweep(&mut a);
        got.push(Swept { label: a.label.clone(), space: a.space, scale: a.scale, native: a.native, tally });
    }
    if got.is_empty() {
        eprintln!("SKIP: none of {} gitignored archive(s) is present", corpus.len());
        return;
    }
    for s in &got {
        let want = pins.iter().find(|(n, _)| *n == s.label).map(|(_, w)| w);
        assert_eq!(
            Some(&s.tally),
            want,
            "{}: the classification inventory moved. Picture space {:?} at scale {:?} → a {:?} \
             screen. This tally is which ARM of `recognize` each border set exercises, and it is \
             what says a newly supported format has art no arm handles",
            s.label,
            s.space,
            s.scale,
            s.native,
        );
    }
}

/// Opens one archive, or `None` when its gitignored fixture is absent.
type Opener = fn() -> Option<Archive>;

/// One archive's result: what it was, and what its flanks turned out to be.
struct Swept {
    label: String,
    space: (u32, u32),
    scale: (u32, u32),
    native: (u32, u32),
    tally: Tally,
}

const fn t(pillars: usize, slabs: usize, poles: usize, unrecognised: usize) -> Tally {
    Tally { pillars, slabs, poles, unrecognised }
}

// ── The corpus, one test per title so they sweep in parallel ─────────────────

/// **Zork Zero's PC renditions.** The masonry `extend_pillars` was written for,
/// in the three archives the player can switch between — whose banners are 34,
/// 37 and 39 raw rows and whose pillars are 166 in all three (SQ-0799).
#[test]
fn zork_zeros_pc_renditions_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("zork0.mg1", || Archive::loose("zork0.mg1")),
            ("zork0.eg1", || Archive::loose("zork0.eg1")),
            ("zork0.cg1", || Archive::loose("zork0.cg1")),
        ],
        &[("zork0.mg1", t(59, 7, 2, 0)), ("zork0.eg1", t(23, 43, 0, 0)), ("zork0.cg1", t(26, 38, 0, 0))],
    );
}

/// **Zork Zero on the machines that ship a whole-screen border plate** — the
/// Amiga floppy, and both of the Macintosh's archives.
///
/// `Pic.data` is the one rendition in the corpus whose picture space does not
/// divide by the v6 text cell: 480x300 on a 304-row screen, which is the whole
/// of SQ-0841. Its plates are REDRAWN rather than scaled — the full-screen ones
/// are exactly 1.5x `CPic.data`'s while its pieces are anything from 1.2x to
/// 2.9x — so nothing about its composition follows from the colour archive's,
/// and the two are swept separately here for that reason.
#[test]
fn zork_zeros_amiga_and_macintosh_archives_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("Zork Zero - The Revenge of Megaboz.adf", || {
                Archive::medium("Zork Zero - The Revenge of Megaboz.adf")
            }),
            ("Zork Zero Disk.image", || Archive::medium("Zork Zero Disk.image")),
            ("Zork Zero Disk.image [Pic.data]", || Archive::inside("Zork Zero Disk.image", "Pic.data")),
        ],
        &[
            ("Zork Zero - The Revenge of Megaboz.adf", t(62, 8, 2, 0)),
            ("Zork Zero Disk.image", t(62, 8, 2, 0)),
            ("Zork Zero Disk.image [Pic.data]", t(30, 42, 0, 0)),
        ],
    );
}

/// **Arthur.** His poles are not reconstructable from the archive (see the
/// module doc), so what is swept here is his full-screen plates — which is
/// exactly the point of a corpus sweep: the art nobody thought of as border art
/// still reaches the composer whenever the story window leaves columns beside it.
#[test]
fn arthurs_archives_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("arthur.mg1", || Archive::loose("arthur.mg1")),
            ("arthur.eg1", || Archive::loose("arthur.eg1")),
            ("arthur.cg1", || Archive::loose("arthur.cg1")),
            ("Arthur - The Quest for Excalibur.adf", || {
                Archive::medium("Arthur - The Quest for Excalibur.adf")
            }),
        ],
        &[
            ("arthur.mg1", t(0, 9, 0, 3)),
            ("arthur.eg1", t(0, 9, 0, 3)),
            ("arthur.cg1", t(0, 9, 0, 3)),
            ("Arthur - The Quest for Excalibur.adf", t(0, 13, 0, 3)),
        ],
    );
}

/// **Journey.** Its one full-screen plate is the illustration SQ-0819 had to
/// keep OUT of the tiler when it sits over a command menu; here it is composed
/// as a flank deliberately, because the recogniser has to be well behaved on it
/// either way.
#[test]
fn journeys_archives_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("journey.mg1", || Archive::loose("journey.mg1")),
            ("journey.eg1", || Archive::loose("journey.eg1")),
            ("journey.cg1", || Archive::loose("journey.cg1")),
            ("Journey - The Quest Begins.adf", || Archive::medium("Journey - The Quest Begins.adf")),
        ],
        &[
            ("journey.mg1", t(0, 4, 0, 0)),
            ("journey.eg1", t(0, 4, 0, 0)),
            ("journey.cg1", t(0, 4, 0, 0)),
            ("Journey - The Quest Begins.adf", t(0, 4, 0, 0)),
        ],
    );
}

/// **Shogun**, whose single-piece lacquer frame is the layout that must NOT take
/// the masonry recipe (SQ-0802) — and whose Amiga plate 50 does declare a waist
/// at the wider crop, so both arms are swept on one title's own art.
#[test]
fn shoguns_archives_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("shogun.mg1", || Archive::loose("shogun.mg1")),
            ("shogun.eg1", || Archive::loose("shogun.eg1")),
            ("shogun.cg1", || Archive::loose("shogun.cg1")),
            ("James Clavell's Shogun.adf", || Archive::medium("James Clavell's Shogun.adf")),
        ],
        &[
            ("shogun.mg1", t(0, 6, 0, 0)),
            ("shogun.eg1", t(0, 6, 0, 0)),
            ("shogun.cg1", t(0, 6, 0, 0)),
            ("James Clavell's Shogun.adf", t(2, 14, 0, 0)),
        ],
    );
}

/// **The colour policy cannot move a border**, which is why this suite pins one
/// mode rather than two.
///
/// The project's convention is that a colour or render area pins both
/// `honor_game_colours` settings, because a mode that hands the page back to the
/// theme has hidden regressions before. That knob changes COLOURS. Every
/// measurement in this file — [`border::art_extent`], [`border::recognize`],
/// `pillar_shaft`, and the span arithmetic above — reads the alpha channel and
/// nothing else, and every handler stamps whole rows. So the composition is a
/// function of the art's SHAPE alone.
///
/// That is asserted here rather than assumed: recolour every opaque pixel of a
/// real border flank to a flat colour it does not contain, leaving alpha exactly
/// as it was, and the layout and the composed band's alpha must not move by a
/// bit. A composition that ever consulted a colour would fail this outright.
#[test]
fn the_border_composition_reads_shape_alone_so_the_colour_policy_cannot_move_it() {
    let opened: Vec<Option<Archive>> = vec![
        Archive::medium("Zork Zero Disk.image"),
        Archive::inside("Zork Zero Disk.image", "Pic.data"),
        Archive::loose("zork0.mg1"),
        Archive::medium("James Clavell's Shogun.adf"),
    ];
    let mut seen = 0;
    for mut a in opened.into_iter().flatten() {
        let native_h = a.native.1;
        let label = a.label.clone();
        for f in flanks(&mut a) {
            let Some(kind) = border::recognize(&f.canvas, f.x0, f.x1, f.art, native_h) else { continue };
            let mut repainted = f.canvas.clone();
            for px in repainted.pixels_mut() {
                if px[3] >= 128 {
                    *px = image::Rgba([0xa7, 0x1b, 0x5e, px[3]]);
                }
            }
            seen += 1;
            assert_eq!(
                border::art_extent(&repainted, f.x0, f.x1),
                f.art,
                "{label}: {} — the art's extent is read from alpha",
                f.what,
            );
            assert_eq!(
                border::recognize(&repainted, f.x0, f.x1, f.art, native_h),
                Some(kind),
                "{label}: {} — flattening every colour in the flank to one must not change which \
                 layout it IS, or `honor_game_colours` could hand a border a different recipe",
                f.what,
            );
            let d = sampled_heights(native_h)[1];
            let plain = border::flank_source(&f.canvas, &f.canvas, f.x0, f.x1, f.art, native_h, 0, d);
            let painted = border::flank_source(&repainted, &repainted, f.x0, f.x1, f.art, native_h, 0, d);
            let alpha = |i: &Option<RgbaImage>| {
                i.as_ref().map(|i| i.pixels().map(|p| p[3]).collect::<Vec<u8>>())
            };
            assert_eq!(
                alpha(&plain),
                alpha(&painted),
                "{label}: {} at {d} rows — the composed band's SHAPE must be identical under any \
                 colouring of the same art",
                f.what,
            );
            // One flank per archive settles it; the rest is the sweep's job.
            break;
        }
    }
    if seen == 0 {
        eprintln!("SKIP: no gitignored Zork Zero or Shogun archive is present");
    }
}
