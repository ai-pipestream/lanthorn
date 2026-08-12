//! SQ-0797 — a 640-wide rendition's dither blends, because its pixels are half as
//! wide.
//!
//! REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.eg1"`,
//! once SQ-0794 had the EGA colours right: the proscenium arch still reads as
//! salmon-and-olive SPECKLE where the same frame from `zork0.mg1` is bronze.
//!
//! MEASURED CAUSE. EGA has no bronze, so Zork Zero's artist made one. The arch is
//! a column-by-column dither — 22,496 pixels of bright red (index 12) against
//! 18,202 of brown (index 6) on the boot frame — and on a 640×200 EGA screen
//! those columns are half as wide as an MCGA pixel, so the card fused each pair
//! into a colour the palette does not contain. Bocfel says the same of Zork
//! Zero's EGA hint background (`z6/draw_border.cpp:745`): "no single pixel of the
//! artwork is the colour the eye actually sees". babelmap keeps all 640 columns —
//! `PictSource::art_scale` maps them at (1, 2), onto exactly the rectangle a
//! 320-wide plate covers — so every art pixel survived as a distinct unit pixel
//! and the dither reached the screen at full contrast.
//!
//! WHY NOT LEAVE IT TO THE RENDERER. Because the renderer cannot do it. Every
//! unit-space→pane scale in the v6 path is `FilterType::Nearest` on purpose
//! (`render/graphics.rs`, `render/screen.rs`), and a nearest resample never
//! blends: below 640 px it drops columns, above 640 it replicates them. What the
//! player saw was therefore a function of their terminal width —
//! `the_fusion_is_not_a_function_of_the_pane_width` measures exactly that and is
//! the case that chose the design.
//!
//! NOT CGA. A `.CG1`'s 640-wide art is genuine one-bit line work — Zork Zero's
//! CGA pillar is a lit column of mirrored tiles (SQ-0808) and SQ-0806 hands its
//! two colours to the terminal — and blending one-bit line work only makes it
//! grey. `PictSource::is_monochrome` is the test, read off the archive's own
//! `EF_MONO` flags rather than off a filename.
//!
//! Every fixture here is gitignored, so each case **skips vacuously** when absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use blorb::infocom_pics::InfocomPics;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn read(name: &str) -> Option<Vec<u8>> {
    let p = stories_dir().join(name);
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored fixture missing at {}", p.display());
            None
        }
    }
}

/// Boot Zork Zero against `archive` and play far enough for the border to be up —
/// the same wiring `startup.rs` uses and `v6_hardware_palette` measures colour
/// with: the archive supplies the standard window and the per-axis art scale, and
/// nothing else differs between renditions.
fn boot(archive: &str, honor_game_colours: bool) -> Option<GameSession> {
    let story = read("zork0-r393-s890714.z6")?;
    let mut picts =
        PictSource::from_native(InfocomPics::parse(read(archive)?).expect("a native archive"));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or(Some(app::graphics::INFOCOM_V6_STD_WINDOW));
    let v6_art_scale = picts.art_scale();
    let mut session = GameSession::new_with_art_scale(
        story,
        honor_game_colours,
        false,
        None,
        false,
        picture_dims,
        v6_screen_px,
        v6_art_scale,
        None,
        None,
    )
    .expect("Zork Zero (v6) loads and boots without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..2 {
        match session.pending_input() {
            InputKind::Line => session.submit("look"),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
    }
    Some(session)
}

/// Zork Zero's full-screen border canvas (window 7), in unit space.
fn frame(session: &GameSession) -> image::RgbaImage {
    (*session.pictures_canvas.get(&7).expect("Zork Zero's border is window 7").img).clone()
}

/// How SPECKLED a frame is: mean per-channel |x − x_right| over horizontally
/// adjacent opaque pairs. A column dither at full contrast scores high; a fused
/// bronze scores low. This is the number the report is about — "salmon-and-olive
/// speckle" is a statement about neighbouring columns, not about any one pixel.
fn speckle(img: &image::RgbaImage) -> f64 {
    let (w, h) = img.dimensions();
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w.saturating_sub(1) {
            let (a, b) = (*img.get_pixel(x, y), *img.get_pixel(x + 1, y));
            if a.0[3] != 255 || b.0[3] != 255 {
                continue;
            }
            for k in 0..3 {
                sum += u64::from(a.0[k].abs_diff(b.0[k]));
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum as f64 / n as f64 }
}

/// Mean per-channel distance between two frames, over pixels opaque in both —
/// the measurement SQ-0797 used to decide the two renditions had converged.
fn distance(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "both renditions land in one unit space");
    let (mut sum, mut n) = (0u64, 0u64);
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        if pa.0[3] == 0 || pb.0[3] == 0 {
            continue;
        }
        for k in 0..3 {
            sum += u64::from(pa.0[k].abs_diff(pb.0[k]));
            n += 1;
        }
    }
    sum as f64 / n as f64
}

/// The renderer's own unit-space→pane scale: nearest-neighbour, at every call
/// site in the v6 path (`render/graphics.rs:741`, `render/screen.rs:8548`).
fn to_pane(img: &image::RgbaImage, pane_w: u32) -> image::RgbaImage {
    let h = img.height();
    image::imageops::resize(img, pane_w, h, image::imageops::FilterType::Nearest)
}

/// The reported defect. Zork Zero's EGA arch fuses into bronze instead of
/// arriving as speckle, and in doing so lands next to the MCGA rendition of the
/// same frame rather than 60% further away.
///
/// Falsified by reverting `blend_columns` to `false` in `PictSource::from_native`:
///
/// ```text
/// honor=true: the EGA frame is still a column dither at full contrast —
/// horizontal speckle 49.118, and MCGA's own frame scores 4.331
/// ```
///
/// …which is the salmon-and-olive speckle as reported, at 11.3× the MCGA frame's
/// own neighbour-to-neighbour variation.
#[test]
fn the_ega_column_dither_fuses_instead_of_reaching_the_screen_as_speckle() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let (e, m) = (frame(&ega), frame(&mcga));

        // MEASURED: 8.401 fused, against 49.118 raw and 4.331 for MCGA.
        let (se, sm) = (speckle(&e), speckle(&m));
        assert!(
            se < 12.0,
            "honor={honor_game_colours}: the EGA frame is still a column dither at full \
             contrast — horizontal speckle {se:.3}, and MCGA's own frame scores {sm:.3}"
        );
        assert!(
            se < sm * 2.5,
            "honor={honor_game_colours}: EGA speckle {se:.3} must land near the MCGA \
             rendition's {sm:.3}, not a multiple of it"
        );

        // MEASURED: 27.79 fused, 44.51 raw. The residue is the two renditions
        // being separately drawn artwork, which no filter closes.
        let d = distance(&e, &m);
        assert!(
            d < 32.0,
            "honor={honor_game_colours}: mean per-channel distance to the MCGA frame is \
             {d:.4}; unfused it is 44.51"
        );

        // And the fused colour is one the card's palette does not hold, which is
        // the entire point of a dither: the arch's shadow is brown (170,85,0)
        // fused with black, i.e. (85,43,0).
        let seen: BTreeSet<[u8; 3]> =
            e.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
        assert!(
            seen.contains(&[85, 43, 0]),
            "honor={honor_game_colours}: the fused shadow bronze is missing from the frame"
        );
        assert!(
            !blorb::infocom_pics::EGA_PALETTE.contains(&[85, 43, 0]),
            "premise: bronze is not an EGA colour — the artist had to dither for it"
        );
    }
}

/// The design decision, measured. Fusing at the archive boundary makes the answer
/// a property of the ARTWORK; leaving it to the renderer's own scale would make it
/// a property of the player's terminal, because that scale is nearest-neighbour
/// at every call site and never blends at any width.
///
/// Falsified by reverting `blend_columns` to `false`: the EGA frame's speckle runs
/// 22.272 / 40.302 / 49.118 / 39.219 / 24.441 at pane widths 320 / 480 / 640 / 800
/// / 1280 — a 2.2× swing with terminal size, and never once near MCGA's 8.746 /
/// 5.793 / 4.331 / 3.458 / 2.155. The first assertion below fails at every one of
/// the five widths.
#[test]
fn the_fusion_is_not_a_function_of_the_pane_width() {
    for honor_game_colours in [true, false] {
        let Some(ega) = boot("zork0.eg1", honor_game_colours) else { return };
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let (e, m) = (frame(&ega), frame(&mcga));

        // Panes narrower than the unit screen, equal to it, and wider — the case
        // the renderer's Nearest scale cannot help with at all.
        for pane_w in [320u32, 480, 640, 800, 1280] {
            let (pe, pm) = (to_pane(&e, pane_w), to_pane(&m, pane_w));
            let (se, sm) = (speckle(&pe), speckle(&pm));
            assert!(
                se < sm * 2.5,
                "honor={honor_game_colours}, pane {pane_w}: EGA speckle {se:.3} against MCGA's \
                 {sm:.3} — what the arch reads as still depends on the terminal's width"
            );
            let d = distance(&pe, &pm);
            assert!(
                d < 32.0,
                "honor={honor_game_colours}, pane {pane_w}: distance to MCGA {d:.4}, unfused \
                 44.3–45.1 at these widths"
            );
        }
    }
}

/// CGA is UNTOUCHED, and this is the constraint that shapes the whole fix. A
/// `.CG1` is 640-wide exactly as an `.EG1` is, so a rule keyed on width alone
/// would blur it; the key is `is_monochrome`, off the archive's own `EF_MONO`
/// flags. Blending one-bit line work produces greys, and greys are precisely what
/// SQ-0806 stopped the CGA stencil from having.
///
/// Falsified by dropping `&& !src.is_monochrome()` from the gate: the frame comes
/// back carrying (64,64,64), (128,128,128) and (191,191,191) — the greys a tent
/// makes of a one-bit edge — and the two-colour assertion below fails on them.
#[test]
fn cga_line_art_is_never_blended() {
    for honor_game_colours in [true, false] {
        let Some(cga) = boot("zork0.cg1", honor_game_colours) else { return };
        let c = frame(&cga);
        let seen: BTreeSet<[u8; 3]> =
            c.pixels().filter(|p| p.0[3] != 0).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
        assert_eq!(
            seen,
            BTreeSet::from([[0, 0, 0], [255, 255, 255]]),
            "honor={honor_game_colours}: a CGA frame stays two colours — no grey may appear"
        );
        // Its one-bit edges keep their full contrast: measured 57.412, and a tent
        // across them would take it to 15.140.
        let s = speckle(&c);
        assert!(
            s > 40.0,
            "honor={honor_game_colours}: CGA line work has been softened (speckle {s:.3}, \
             sharp is 57.4 and tented is 15.1)"
        );
    }

    // …and the gate says so directly, off the content rather than the extension.
    for (archive, blends) in [("zork0.cg1", false), ("zork0.eg1", true)] {
        let Some(raw) = read(archive) else { return };
        let src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
        assert_eq!(src.art_scale().map(|s| s.0), Some(1), "{archive}: both are 640-wide");
        assert_eq!(src.is_monochrome(), !blends, "{archive}: only the two-colour one opts out");
    }
}

/// The 320-wide renditions are UNTOUCHED, and cannot be otherwise: their art
/// scale is (2, 2), there is no dither at this frequency, and nothing fuses.
/// Measured through the palette rather than the pixels — an MCGA archive stores
/// 4 bits per channel, so every channel on its frame is a multiple of 17, and a
/// blend of two such colours is not.
#[test]
fn the_320_wide_renditions_are_untouched() {
    for archive in ["zork0.mg1", "zork0.pic"] {
        let Some(raw) = read(archive) else { return };
        let src = PictSource::from_native(InfocomPics::parse(raw).expect("parses"));
        assert_eq!(src.art_scale().map(|s| s.0), Some(2), "{archive}: 320-wide picture space");
    }

    for honor_game_colours in [true, false] {
        let Some(mcga) = boot("zork0.mg1", honor_game_colours) else { return };
        let m = frame(&mcga);
        assert!(
            m.pixels().filter(|p| p.0[3] != 0).all(|p| p.0[..3].iter().all(|ch| ch % 17 == 0)),
            "honor={honor_game_colours}: an MCGA frame is 4 bits per channel; a blend is not"
        );
    }
}

/// Alpha is never blended, so a stencil keeps its edges. Every native archive
/// marks nearly every picture transparent, and a filter that averaged alpha would
/// ring every cut-out with half-transparent fringe — invisible on a black ground
/// and wrong everywhere else. `zork0.eg1`'s card body (picture 101) is cut out on
/// colour ONE, which is blue, so it is the case where a fringe would show.
///
/// Falsified by blending alpha along with the colour channels: the card's first
/// cut-out pixel arrives at alpha 64 instead of 0.
#[test]
fn a_cut_out_keeps_its_exact_alpha() {
    let Some(raw) = read("zork0.eg1") else { return };
    let pics = InfocomPics::parse(raw).expect("parses");
    let card = pics.decode(101).expect("Zork Zero's EGA card body");
    assert_eq!(card.transparent, Some(1), "premise: cut out on colour 1, not 0 (SQ-0801)");

    let mut src = PictSource::from_native(InfocomPics::parse(read("zork0.eg1").unwrap()).unwrap());
    let img = src.image(101).expect("the same picture, through the fusing source");
    let img = img.to_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (u32::from(card.width), u32::from(card.height)),
        "fusing never resizes: 640 columns in, 640 columns out"
    );
    for (i, p) in img.pixels().enumerate() {
        let want = if card.indices[i] == 1 { 0 } else { 255 };
        assert_eq!(
            p.0[3], want,
            "pixel {i} (index {}) came through at alpha {}, not {want}",
            card.indices[i], p.0[3]
        );
    }
}
