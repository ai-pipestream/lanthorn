//! SQ-0794 — an EGA or CGA rendition is drawn through the colours its video card
//! fixed, not through the archive's stock fallback.
//!
//! REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.eg1"`,
//! once SQ-0790 had the plate landing at its true aspect: Zork Zero's proscenium
//! arch came out pink and olive where the same frame from `zork0.mg1` is bronze,
//! and the CGA rendition was worse — flat green and cyan line art.
//!
//! MEASURED CAUSE, which `blorb::infocom_pics` had already written down before
//! there was a symptom. A PC EGA or CGA directory record is **12 bytes**, with a
//! pad byte where MCGA and the Amiga keep a 3-byte palette pointer, because
//! those adapters fixed their colours in the video hardware and there was nothing
//! to store. So every picture in such an archive answers "no palette of my own",
//! `PictSource` read that as Blorb §11.3 *adaptive*, no non-adaptive draw ever
//! established a Current Palette, and all of it expanded through
//! `DEFAULT_PALETTE` — which is the EGA table with **one entry wrong**, index 6
//! dark yellow `(170, 170, 0)` where the hardware shows brown `(170, 85, 0)`.
//! Zork Zero's EGA artwork dithers that brown against bright red to make its
//! bronze, so one entry took the whole plate olive. CGA fared worse still:
//! `DEFAULT_PALETTE` slots 2 and 3 are green and cyan, and a `.CG1` uses
//! precisely 2 and 3.
//!
//! The fix is `InfocomPics::hardware_palette`, which reads the rendition off the
//! directory (no picture carries a palette ⇒ EGA or CGA; every picture with
//! pixels sets `EF_MONO` ⇒ CGA), and a `PictSource` that draws through it and
//! keeps such an archive out of the adaptive machinery entirely.
//!
//! MCGA (`.MG1`) and the Amiga/Mac `Pic.data` carry real per-picture palettes and
//! are untouched — pinned below rather than assumed.
//!
//! Every fixture here is gitignored, so each case **skips vacuously** when absent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use blorb::infocom_pics::{InfocomPics, Rgb, CGA_PALETTE, DEFAULT_PALETTE, EGA_PALETTE};

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

fn native(name: &str) -> Option<PictSource> {
    Some(PictSource::from_native(
        InfocomPics::parse(read(name)?).expect("a native Infocom archive parses"),
    ))
}

/// Boot Zork Zero against `archive` and play far enough for the border to be up,
/// exactly as `startup.rs` builds a session: the archive supplies the standard
/// window and the per-axis art scale, and nothing else differs between
/// renditions.
fn boot(archive: &str, honor_game_colours: bool) -> Option<GameSession> {
    let story = read("zork0-r393-s890714.z6")?;
    let mut picts = native(archive)?;
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

/// Every opaque colour on Zork Zero's full-screen border canvas (window 7).
fn frame_colours(session: &GameSession) -> BTreeSet<Rgb> {
    let c = session.pictures_canvas.get(&7).expect("Zork Zero's border is window 7");
    c.img
        .pixels()
        .filter(|p| p.0[3] != 0)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect()
}

/// The reported defect, on the reported fixture: the EGA arch is bronze, which
/// on a sixteen-colour card means brown dithered against bright red.
///
/// Falsified by restoring the fallback (`native_image(pics, resnum, None)` in
/// `PictSource::get`): brown vanishes from the frame and dark yellow takes its
/// place — 18202 pixels of it, the exact count SQ-0794 measured — which is the
/// olive the report describes.
#[test]
fn zork_zeros_ega_frame_is_drawn_in_ega_brown_not_dark_yellow() {
    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.eg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert!(
            seen.contains(&EGA_PALETTE[6]),
            "honor={honor_game_colours}: the EGA frame must contain brown {:?}",
            EGA_PALETTE[6]
        );
        assert!(
            !seen.contains(&DEFAULT_PALETTE[6]),
            "honor={honor_game_colours}: dark yellow {:?} is the defect, and must be gone",
            DEFAULT_PALETTE[6]
        );
        // Nothing outside the card's sixteen may reach the frame.
        for c in &seen {
            assert!(EGA_PALETTE.contains(c), "honor={honor_game_colours}: {c:?} is not an EGA colour");
        }
    }
}

/// CGA is two colours, not four: its only 640-wide mode was 640x200 mode 6. The
/// archive's indices 2 and 3 are white and black, and `DEFAULT_PALETTE` renders
/// them green and cyan.
///
/// Falsified the same way as the EGA case: the frame comes back as
/// `(0, 170, 0)` and `(0, 170, 170)` and nothing else.
#[test]
fn zork_zeros_cga_frame_is_black_and_white() {
    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.cg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert_eq!(
            seen,
            BTreeSet::from([[0, 0, 0], [255, 255, 255]]),
            "honor={honor_game_colours}: a CGA frame is black and white only"
        );
        assert!(!seen.contains(&DEFAULT_PALETTE[2]), "the fallback's green is the defect");
        assert!(!seen.contains(&DEFAULT_PALETTE[3]), "the fallback's cyan is the defect");
    }
}

/// The other half of the fix, and the one the Current-Palette machinery cares
/// about (SQ-0743, Blorb §11.3): an EGA or CGA picture is not adaptive. Its
/// record has no palette pointer to be silent with — the colours were in the
/// card — so nothing may ever splice a `PLTE` into it, including a Current
/// Palette carried in from a host Save State.
#[test]
fn a_hardware_palette_archive_has_no_adaptive_pictures() {
    for (archive, hardware) in [
        ("zork0.eg1", true),
        ("zork0.cg1", true),
        ("zork0.mg1", false),
        ("zork0.pic", false),
    ] {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        let adaptive_by_record = pics.adaptive_pictures();
        assert!(
            !adaptive_by_record.is_empty(),
            "{archive}: every one of these archives has palette-less records — that is the trap"
        );
        assert_eq!(pics.hardware_palette().is_some(), hardware, "{archive} hardware table");

        let src = PictSource::from_native(InfocomPics::parse(read(archive).unwrap()).unwrap());
        for id in adaptive_by_record {
            assert_eq!(
                src.is_adaptive(u32::from(id)),
                !hardware,
                "{archive} picture {id}: a hardware rendition defers to nobody, an MCGA/Amiga one does"
            );
        }
    }
}

/// MCGA and the Amiga are UNCHANGED. Both carry their colours per picture, so
/// neither takes a hardware table and neither loses a single adaptive picture —
/// and the frame Zork Zero's MCGA archive draws is still its bronze, in the
/// 12-bit palette the format stores (every channel a multiple of 17), with none
/// of the EGA card's four levels showing through.
#[test]
fn the_mcga_and_amiga_renditions_are_untouched() {
    for archive in ["zork0.mg1", "zork0.pic"] {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        assert_eq!(pics.hardware_palette(), None, "{archive} carries its own colours");
        // Zork Zero's 16 compass overlays are the canonical adaptive set (SQ-0743).
        let adaptive = pics.adaptive_pictures();
        for id in 9..=24u16 {
            assert!(adaptive.contains(&id), "{archive}: compass overlay {id} is still adaptive");
        }
    }

    for honor_game_colours in [true, false] {
        let Some(session) = boot("zork0.mg1", honor_game_colours) else { return };
        let seen = frame_colours(&session);
        assert!(
            seen.iter().all(|c| c.iter().all(|ch| ch % 17 == 0)),
            "honor={honor_game_colours}: an MCGA palette is 4 bits per channel"
        );
        assert!(
            seen.len() > 4,
            "honor={honor_game_colours}: the MCGA frame is a full bronze ramp, not two colours"
        );
        assert!(
            !seen.contains(&CGA_PALETTE[1]) && !seen.contains(&EGA_PALETTE[6]),
            "honor={honor_game_colours}: no hardware table may leak into an MCGA frame"
        );
    }
}

/// SQ-0806 — a two-colour rendition tells the story the interpreter has no
/// colours, so it never asks for the white page that would paint out its own art.
///
/// REPORTED, on `stories/zork0-r393-s890714.z6` with `pictures = "zork0.cg1"`:
/// the background comes out white and the black-and-white border art blends into
/// it.
///
/// MEASURED CAUSE. A `.CG1` is a two-colour STENCIL. On Zork Zero's border:
/// 46,336 opaque white pixels, 17,152 opaque black, and 192,512 transparent —
/// one row through the pillars runs transparent at the screen edge, opaque white
/// for the column's lit face, transparent again across the story area. Its white
/// is PAINT and its transparency reveals a colour the artwork never stored; a
/// white page destroys both. Zork Zero asks for one regardless of card, because
/// it issues `set_colour(fg=2, bg=9)` for every video card alike and the story
/// file cannot see which archive was loaded — pinned below.
///
/// The lever is `honor_game_colours`, which already means "declare the
/// interpreter colourless" (§8.3.2). NOT the interpreter number: header `$1E`
/// steers far more of a v6 game than colour, and advertising 1 (DECSystem-20)
/// costs Shogun its whole right border.
#[test]
fn a_two_colour_rendition_is_told_the_interpreter_has_no_colours() {
    use zvm::screen::ZColour;

    // The premise: told it has colours, the game sets the same pair whatever the
    // card — so nothing about the ARCHIVE is what stops it.
    for archive in ["zork0.cg1", "zork0.eg1", "zork0.mg1"] {
        let Some(session) = boot(archive, true) else { return };
        let v6 = session.machine.screen.v6.as_ref().expect("v6 window table");
        assert_eq!(
            (v6.windows[0].fg, v6.windows[0].bg),
            (ZColour::Standard(2), ZColour::Standard(9)),
            "{archive}: Zork Zero sets black-on-white whatever the card"
        );
    }

    // …and told it has none, it asks for nothing at all — on windows 0 (story),
    // 1 (status) and 7 (border) alike, which are Zork Zero's three coloured
    // windows. The host theme then owns the ground the stencil reveals.
    for archive in ["zork0.cg1", "zork0.mg1"] {
        let Some(session) = boot(archive, false) else { return };
        let v6 = session.machine.screen.v6.as_ref().expect("v6 window table");
        for w in [0usize, 1, 7] {
            assert_eq!(
                (v6.windows[w].fg, v6.windows[w].bg),
                (ZColour::Default, ZColour::Default),
                "{archive}: a colourless interpreter is never asked for window {w}'s colours"
            );
        }
    }
}

/// The archive says two-colour from its CONTENT, not from a filename a rename
/// could make a lie — this is what `startup` reads to force the flag off.
#[test]
fn a_cga_archive_reports_itself_monochrome_and_the_others_do_not() {
    for (archive, want) in
        [("zork0.cg1", true), ("zork0.eg1", false), ("zork0.mg1", false), ("zork0.pic", false)]
    {
        let Some(raw) = read(archive) else { continue };
        let pics = InfocomPics::parse(raw).expect("parses");
        assert_eq!(pics.is_monochrome(), want, "{archive}: InfocomPics::is_monochrome");
        let Some(src) = native(archive) else { continue };
        assert_eq!(src.is_monochrome(), want, "{archive}: PictSource::is_monochrome");
    }
}
