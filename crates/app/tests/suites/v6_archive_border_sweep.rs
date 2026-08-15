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
//! checked. Both arrangements — a full-screen plate cropped at the **gutter its
//! own art declares**, and a full-width strip over a narrow column whose unit
//! heights sum to the picture space EXACTLY — are documented there, as is why
//! Arthur's poles are not reconstructable from an archive alone.
//!
//! ## SQ-0845: the crop width had to come from the picture space
//!
//! This suite shipped cropping every full-screen plate at **46 and 86 unit
//! columns**, Shogun's and Zork Zero's measured flank widths — and those are
//! 640-screen numbers. The Macintosh's monochrome archive is a **480x300**
//! picture space (`graphics.rs`'s scale table), so both crops landed at the wrong
//! fraction of it, and 86 of 480 is past the middle of some of its plates.
//!
//! Worse, the pair was applied to every plate an archive carries, and most plates
//! are ILLUSTRATIONS — full-screen scene art with no story window in it. Cropping
//! one at an invented width and asking `recognize` what it is produces an answer,
//! never a failure, so the suite reported a 516-flank inventory of which **68**
//! were flanks. The Macintosh archive alone claimed 72, and they were four plates
//! of border art counted twice per side plus **56 crops of its fourteen
//! illustrations**; two of the sixteen that did touch a border plate read
//! `ShogunSinglePiece` at 46 columns, where the game's own 61-pixel window makes
//! them pillars. The two `ArthurPoles` the tally claimed were the right-hand crop
//! of Zork Zero's illustration 25 — no archive in the corpus states a poles flank,
//! exactly as [`border_corpus`]'s doc always said.
//!
//! What replaced it is measured rather than guessed: a border plate leaves the
//! story window CLEAR below its own top strip, so the plate states its flank
//! width itself, on whatever picture space it was drawn for
//! ([`border_corpus::plate_gutter`], which tabulates the four media against the
//! `x_px` their games actually set). A plate with no gutter states no flank and is
//! counted, not cropped. Arthur's and Journey's loose archives carry only
//! illustrations and now state **nothing** — the real-game coverage of both lives
//! in `v6_side_border_tiling.rs`, at the flank width the game itself sets, which
//! is the only place it was ever honest.
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
//! 6. **A symmetric border comes out symmetric** (SQ-0845). One plate is one
//!    drawing and its left and right crops are the same border, so they must be
//!    the same layout. This is the property a crop width borrowed from another
//!    picture space breaks without pinning anything: at 46 unit columns Arthur's
//!    632-wide Amiga plate read `ShogunSinglePiece` on the left and `ArthurPoles`
//!    on the right, because 640 − 46 lands past the art it was cropping.
//!
//! **There is deliberately no crop-INVARIANCE property here, and the reason is
//! worth knowing.** Widening a flank cannot leave [`recognize`] alone: nearly
//! every border in the corpus hangs its column under a FULL-WIDTH top strip, so a
//! wider crop widens the banner while the shaft under it stays put, the
//! narrowest-over-widest ratio falls, and a slab becomes a pillar. Measured:
//! `zork0.mg1`'s jungle strip over its 72-wide column is `ShogunSinglePiece` at
//! 72 and `ZorkZeroPillars` at 80; Shogun's Amiga plate 50 is a slab at 60 and a
//! pillar at 86, which is exactly the spurious pillar the old pin carried. A
//! flank width is therefore not a detail the classifier can shrug off — it is an
//! input, it belongs to the picture space, and pinning it is the point.
//!
//! …plus a per-archive **inventory**, pinned: the picture space, the flank width
//! that space states, how its flanks classify, and how many of its plates state
//! no flank at all. That inventory is half the value of the sweep: it says which
//! arm of [`recognize`] each rendition exercises, and it is what tells a later
//! reader that a newly supported format has art no arm handles. It is also the
//! second thing SQ-0841 moves — restoring the exact bottom test turns every
//! Macintosh monochrome pillar into a slab.
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

/// What one archive turned out to state. Pinned per archive — see the module doc.
#[derive(Default, PartialEq, Eq, Debug)]
struct Tally {
    /// The picture space the archive declares, so a pin that moves says which
    /// space it was measuring.
    space: (u32, u32),
    /// The flank width that space states, in its own unit columns — `None` when
    /// no plate declares a gutter and every flank came from a strip over a
    /// column, which states its own width.
    width: Option<u32>,
    pillars: usize,
    slabs: usize,
    poles: usize,
    /// Flanks the archive states that [`border::recognize`] cannot name. Counted
    /// rather than bucketed: a newly supported format whose border is none of the
    /// three shapes must show up here and not as somebody else's masonry.
    unclassified: usize,
    /// Full-screen plates carrying no story window, and so stating no flank.
    stateless_plates: usize,
}

/// The band heights sampled. Deliberately not multiples of one another and not
/// multiples of the screen: a composition that only lands its foot when the
/// extension divides evenly passes a single height perfectly.
fn sampled_heights(native_h: u32) -> [u32; 3] {
    [native_h + 97, 2 * native_h + 13, 3 * native_h - 41]
}

/// Sweep one archive: compose every flank it states, at every sampled height,
/// and assert the six properties. Returns the inventory.
fn sweep(a: &mut Archive) -> Tally {
    let label = a.label.clone();
    let (native_h, art_h) = (a.native.1, a.art_screen.1);
    let space = a.space;
    let discovered = flanks(a);
    let mut tally = Tally {
        space,
        width: discovered.stated_width,
        stateless_plates: discovered.stateless_plates,
        ..Tally::default()
    };
    // Which layout each source picture's first-seen flank was, for property 6.
    let mut sides: std::collections::HashMap<String, (&'static str, Option<BorderArt>)> =
        std::collections::HashMap::new();
    for f in discovered.flanks {
        let kind = border::recognize(&f.canvas, f.x0, f.x1, f.art, native_h);
        match kind {
            Some(BorderArt::ZorkZeroPillars) => tally.pillars += 1,
            Some(BorderArt::ShogunSinglePiece) => tally.slabs += 1,
            Some(BorderArt::ArthurPoles) => tally.poles += 1,
            None => tally.unclassified += 1,
        }

        // ── 6. A symmetric border comes out symmetric (SQ-0845) ──────────────
        //
        // One plate, one drawing, two crops of it: the two sides are the same
        // border and must be the same layout. A crop width taken from another
        // picture space breaks this outright — Arthur's 632-wide Amiga plate
        // classified `ShogunSinglePiece` on the left and `ArthurPoles` on the
        // right at 46 unit columns, because 640 − 46 lands past the art.
        if let Some((first_side, first_kind)) = sides.insert(f.source.clone(), (f.side, kind)) {
            assert_eq!(
                first_kind,
                kind,
                "{label}: {} — the {first_side} flank of a {}x{} picture space is {first_kind:?} and \
                 its {} flank is {kind:?}. One plate is one symmetric drawing, so its two crops are \
                 the same border; a width that disagrees with itself came from somewhere else \
                 (SQ-0845)",
                f.source,
                space.0,
                space.1,
                f.side,
            );
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

/// Sweep a list of archives and check each one's inventory against its pin.
/// Skips vacuously — and says so — when nothing in the list is present.
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
        got.push(Swept { label: a.label.clone(), scale: a.scale, native: a.native, tally });
    }
    // CI has no `stories/`, so an empty run is the fixtures being absent and not
    // a suite that swept nothing it should have.
    if got.is_empty() {
        eprintln!("SKIP: none of {} gitignored archive(s) is present", corpus.len());
        return;
    }
    for s in &got {
        let want = pins.iter().find(|(n, _)| *n == s.label).map(|(_, w)| w);
        assert_eq!(
            Some(&s.tally),
            want,
            "{}: the inventory moved. A {:?} picture space at scale {:?} → a {:?} screen. This \
             says which ARM of `recognize` each border set exercises, at the flank width that \
             space itself states, and how many of its plates state no flank at all — which is what \
             tells a later reader that a newly supported format has art no arm handles",
            s.label,
            s.tally.space,
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
    scale: (u32, u32),
    native: (u32, u32),
    tally: Tally,
}

/// One archive's pin. `space` is the picture space it must declare and `width`
/// the flank that space states, both named so a failure says which space moved.
const fn t(
    space: (u32, u32),
    width: Option<u32>,
    pillars: usize,
    slabs: usize,
    poles: usize,
    unclassified: usize,
    stateless_plates: usize,
) -> Tally {
    Tally { space, width, pillars, slabs, poles, unclassified, stateless_plates }
}

/// The three picture spaces the corpus covers — `graphics.rs`'s scale table.
const MCGA: (u32, u32) = (320, 200);
const EGA: (u32, u32) = (640, 200);
const MAC_MONO: (u32, u32) = (480, 300);

// ── The corpus, one test per title so they sweep in parallel ─────────────────

/// **Zork Zero's PC renditions.** The masonry `extend_pillars` was written for,
/// in the three archives the player can switch between — whose banners are 34,
/// 37 and 39 raw rows and whose pillars are 166 in all three (SQ-0799).
///
/// All three ship their border as a strip over a column, so every flank here is
/// cropped at its column picture's OWN width and none of them needs a gutter —
/// which is why `width` is `None` and all fourteen plates state no flank. Those
/// fourteen used to contribute 56 crops apiece at 46 and 86 unit columns, and
/// `zork0.mg1`'s two `ArthurPoles` among them were a crop of illustration 163.
#[test]
fn zork_zeros_pc_renditions_compose_a_well_formed_flank() {
    sweep_all(
        &[
            ("zork0.mg1", || Archive::loose("zork0.mg1")),
            ("zork0.eg1", || Archive::loose("zork0.eg1")),
            ("zork0.cg1", || Archive::loose("zork0.cg1")),
        ],
        &[
            ("zork0.mg1", t(MCGA, Some(86), 12, 0, 0, 0, 14)),
            ("zork0.eg1", t(EGA, Some(86), 10, 0, 0, 0, 14)),
            ("zork0.cg1", t(EGA, Some(86), 8, 0, 0, 0, 14)),
        ],
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
///
/// **This is where SQ-0845 lives.** All three ship their border as four
/// full-screen plates, and each states its own flank: 86 unit columns on the
/// 640-wide colour archives against **53 on the 480-wide monochrome one**, which
/// is why one pair of constants could not serve both. Every one of the eight
/// flanks each archive states is a pillar, on all three — the monochrome archive
/// previously reported 30 pillars and 42 slabs over 72 crops, of which 64 were
/// its fourteen illustrations and two were border plates cut at 46 columns and
/// read as Shogun's slab.
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
            ("Zork Zero - The Revenge of Megaboz.adf", t(MCGA, Some(86), 8, 0, 0, 0, 14)),
            ("Zork Zero Disk.image", t(MCGA, Some(86), 8, 0, 0, 0, 14)),
            ("Zork Zero Disk.image [Pic.data]", t(MAC_MONO, Some(53), 8, 0, 0, 0, 14)),
        ],
    );
}

/// **Arthur**, and what an archive genuinely does not know.
///
/// His poles are not reconstructable (see the module doc), and his three loose
/// archives carry nothing else: three full-screen illustrations apiece, none with
/// a story window in it, so all three state **no flank at all**. That is the pin,
/// and it is a statement rather than a hole — his real frame is driven at the
/// width the game sets in `v6_side_border_tiling.rs`.
///
/// The Amiga floppy is the one that states something: its picture 54 is his whole
/// frame as one plate, gutter 12 unit columns, and the corpus paints it at
/// `(0, 0)` where the game hangs it eleven rows lower — so it composes as a slab
/// here where the screen shows poles. That is the archive's own arrangement, not
/// the screen's, and the pin says so.
///
/// This test used to claim 12, 12, 12 and 16 flanks. Nine of each twelve were
/// crops of an illustration and the other three were the RIGHT crop of a 584-wide
/// plate on a 640-wide screen, which is past the art entirely — the three
/// `unrecognised` the old pin carried were empty rectangles.
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
            ("arthur.mg1", t(MCGA, None, 0, 0, 0, 0, 3)),
            ("arthur.eg1", t(EGA, None, 0, 0, 0, 0, 3)),
            ("arthur.cg1", t(EGA, None, 0, 0, 0, 0, 3)),
            // **Poles, not slabs** (SQ-0881). This pin recorded 2 slabs until
            // the width discriminator landed: swept off the archive, a flank's
            // art starts at row 0, and `top == 0` alone was `recognize`'s test
            // for Shogun's single-piece border — so Arthur's poles took Shogun's
            // arm, which extends by stamping a second copy of the whole border
            // and tiles its BANNER down the side of the screen. Reported on the
            // Macintosh monochrome press, where the art is short enough relative
            // to the pane that the extension actually runs.
            ("Arthur - The Quest for Excalibur.adf", t(MCGA, Some(12), 0, 0, 2, 0, 3)),
        ],
    );
}

/// **Journey states no border at all**, on any of its four archives, and that is
/// the finding rather than a gap.
///
/// Each carries exactly one full-screen plate and it is an illustration: no
/// gutter, no story window, nothing that fixes a flank width. The old pin claimed
/// four flanks per archive and all sixteen were that one plate cropped at
/// Shogun's 46 and Zork Zero's 86 — widths belonging to two other games. The
/// game's own left column is **264** unit pixels wide (measured on
/// `Journey - The Quest Begins.adf`, release 30 / serial 890322, at a gameplay
/// frame), which no archive says and neither of those two guesses is near.
///
/// SQ-0819's requirement — that a picture column over a command menu is not a
/// border — is a statement about a live screen, and it is asserted on one in
/// `v6_side_border_tiling.rs` §11, at that 264. Cropping the title plate at 46
/// never tested it.
#[test]
fn journeys_archives_state_no_border_flank() {
    sweep_all(
        &[
            ("journey.mg1", || Archive::loose("journey.mg1")),
            ("journey.eg1", || Archive::loose("journey.eg1")),
            ("journey.cg1", || Archive::loose("journey.cg1")),
            ("Journey - The Quest Begins.adf", || Archive::medium("Journey - The Quest Begins.adf")),
        ],
        &[
            ("journey.mg1", t(MCGA, None, 0, 0, 0, 0, 1)),
            ("journey.eg1", t(EGA, None, 0, 0, 0, 0, 1)),
            ("journey.cg1", t(EGA, None, 0, 0, 0, 0, 1)),
            ("Journey - The Quest Begins.adf", t(MCGA, None, 0, 0, 0, 0, 1)),
        ],
    );
}

/// **Shogun**, whose single-piece lacquer frame is the layout that must NOT take
/// the masonry recipe (SQ-0802) — and every flank all four of its archives state
/// is that slab, on both picture spaces.
///
/// The old pin gave the Amiga floppy **two pillars**, and they were plate 50 cut
/// at Zork Zero's 86 unit columns. Its own gutter is 60 and the game opens its
/// window at 46; at either of those it is a slab, like the other five, and its
/// "waist at the wider crop" was the crop reaching past the lacquer frame into
/// the empty story page beside it.
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
            ("shogun.mg1", t(MCGA, Some(60), 0, 2, 0, 0, 1)),
            ("shogun.eg1", t(EGA, Some(60), 0, 2, 0, 0, 1)),
            ("shogun.cg1", t(EGA, Some(58), 0, 2, 0, 0, 1)),
            ("James Clavell's Shogun.adf", t(MCGA, Some(60), 0, 6, 0, 0, 1)),
        ],
    );
}

/// **What this suite actually covers**, stated as one number so it cannot drift
/// back into a claim nobody checks.
///
/// The eighteen archives in `stories/` state **68** border flanks between them,
/// and every one is a flank: a full-screen plate with a story-window gutter in
/// it, or a strip over a column that tiles the picture space exactly. The suite
/// used to report **516**, because it cropped all 120 of the corpus's full-screen
/// plates — 104 of which carry no story window at all — at 46 and 86 unit columns
/// on each of two sides, and added the 36 strip-over-column pairs to that.
///
/// Three picture spaces are covered, and the flank width is different on each:
/// 86 unit columns on the 320x200 and 640x200 renditions, **53** on the
/// Macintosh's 480x300, 60 and 58 for Shogun, 12 for Arthur's Amiga plate. A
/// single pair of constants could not have been right for more than one of them.
///
/// Needs the whole corpus, so it skips vacuously unless every fixture is present
/// — the per-title pins above are what hold when only some are.
#[test]
fn the_corpus_states_sixty_eight_flanks_and_every_one_is_stated() {
    /// Every archive the per-title tests sweep, and the flank width its own
    /// picture space states.
    const CORPUS: &[(&str, Opener, Option<u32>)] = &[
        ("zork0.mg1", || Archive::loose("zork0.mg1"), Some(86)),
        ("zork0.eg1", || Archive::loose("zork0.eg1"), Some(86)),
        ("zork0.cg1", || Archive::loose("zork0.cg1"), Some(86)),
        ("zz.adf", || Archive::medium("Zork Zero - The Revenge of Megaboz.adf"), Some(86)),
        ("zz.image", || Archive::medium("Zork Zero Disk.image"), Some(86)),
        ("zz.image [Pic.data]", || Archive::inside("Zork Zero Disk.image", "Pic.data"), Some(53)),
        ("arthur.mg1", || Archive::loose("arthur.mg1"), None),
        ("arthur.eg1", || Archive::loose("arthur.eg1"), None),
        ("arthur.cg1", || Archive::loose("arthur.cg1"), None),
        ("arthur.adf", || Archive::medium("Arthur - The Quest for Excalibur.adf"), Some(12)),
        ("journey.mg1", || Archive::loose("journey.mg1"), None),
        ("journey.eg1", || Archive::loose("journey.eg1"), None),
        ("journey.cg1", || Archive::loose("journey.cg1"), None),
        ("journey.adf", || Archive::medium("Journey - The Quest Begins.adf"), None),
        ("shogun.mg1", || Archive::loose("shogun.mg1"), Some(60)),
        ("shogun.eg1", || Archive::loose("shogun.eg1"), Some(60)),
        ("shogun.cg1", || Archive::loose("shogun.cg1"), Some(58)),
        ("shogun.adf", || Archive::medium("James Clavell's Shogun.adf"), Some(60)),
    ];
    const STATED: usize = 68;

    let mut opened = 0;
    let mut total = 0;
    for &(name, open, width) in CORPUS {
        let Some(mut a) = open() else { continue };
        opened += 1;
        let space = a.space;
        let d = flanks(&mut a);
        assert_eq!(
            d.stated_width, width,
            "{name}: a {space:?} picture space states a {:?}-column flank, not {width:?}. This is \
             the number SQ-0845 is about — it belongs to the space the art was drawn for, and no \
             one value of it is right for all three spaces the corpus carries",
            d.stated_width,
        );
        for f in &d.flanks {
            assert_eq!(
                f.x1 - f.x0,
                width.unwrap_or(0),
                "{name}: {} is cropped {} wide where its archive states {width:?}",
                f.what,
                f.x1 - f.x0,
            );
        }
        total += d.flanks.len();
    }
    if opened < CORPUS.len() {
        eprintln!("SKIP: {} of {} gitignored archive(s) present", opened, CORPUS.len());
        return;
    }
    assert_eq!(
        total, STATED,
        "the corpus states {total} border flanks, not {STATED}. Every one of them is a flank — a \
         plate with a story window in it, or a strip over a column that tiles the picture space — \
         so this number is what the suite covers and not what it examined. If it grew because a \
         new archive arrived, say so; if it grew because something is being cropped at a width its \
         own picture space does not state, that is SQ-0845 coming back",
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
        for f in flanks(&mut a).flanks {
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
