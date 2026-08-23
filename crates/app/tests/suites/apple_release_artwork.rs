//! SQ-0863: **the Apple II presses draw their own artwork**, and the screen that
//! comes with it.
//!
//! # What was wrong
//!
//! SQ-0863's first half read the archives — Infocom's Apple flavour, 8-byte
//! directory records, RLE+XOR pixels, sourced from `zip.equ`, `pic.asm` and
//! `apple.pal` — and then deliberately did not wire them in. The stated blocker
//! was that an archive states a picture space, the Apple's is 140×192, and
//! honouring it would move *Arthur*'s window off the 640×400 a test pinned.
//!
//! That pin was the ARTLESS fallback. 640×400 is what a Version 6 launch gets
//! when nothing declares a picture space at all, so it recorded a geometry that
//! held only while the art was missing. An archive outranks a profile — it is
//! the same order that lays Zork Zero out on 480×300 off the standard
//! Macintosh's monochrome plate (SQ-0838), and `reset.rs`'s own chain: the Blorb
//! `Reso` chunk, then the archive, then the machine. So the door opened and the
//! pin moved, and this file is where the new geometry and the artwork behind it
//! are stated.
//!
//! # The two doors that had to open
//!
//! - `blorb::prodos::ProDos::pictures` now falls through to `packed_pictures`,
//!   exactly as `story` falls through to `packed_story`. No ProDOS volume in the
//!   corpus keeps its artwork as a FILE; all four graphical releases keep it
//!   inside the segmented `.D1`…`.D5` container.
//! - `blorb::medium::MountedDisk::pictures` gained a set-spanning arm, the
//!   sibling of the one `stories` got in SQ-0864, because *Shogun*'s, *Zork
//!   Zero*'s and *Journey*'s 5.25-inch presses page their plates across the
//!   whole release. `app::assets::volumes` supplies the companions, lazily and
//!   only for a volume that has no story of its own.
//!
//! # The scale, sourced from the machine
//!
//! `app::graphics::PictSource::art_scale` carries the derivation in full. In one
//! line: `apple/yzip/rel.15/apple.equ` states `MAXWIDTH EQU 140 ; 560 / 4 = max
//! "pixels"` and `MAXHEIGHT EQU 192 ; 192 screen lines`, so one Apple picture
//! pixel is four double-hi-res dots wide and one scan line tall, and a scan line
//! measures (3/4)·(560/192) = 2.19 dots on the 4:3 display the machine drove.
//! (4, 2) is those two facts; 560×384 is 140×192 multiplied by them, and exactly
//! 70×24 whole 8×16 cells.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! per missing fixture and every `ran > 0` guard is gated on a presence check —
//! CI has none of this on any platform and must not fail on its absence.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn media(name: &str) -> Option<PathBuf> {
    let p = stories_dir().join(name);
    p.is_file().then_some(p)
}

/// The Apple's picture space, from `apple.equ` and agreed by every archive's own
/// directory (nothing in them exceeds 140×192).
const APPLE_PICTURE_SPACE: (u16, u16) = (140, 192);
/// …and the screen it asks for, which is that space at the machine's scale.
const APPLE_SCREEN: (u16, u16) = (560, 384);

/// One Apple press: the image a person opens, the archive it draws with, and how
/// many pictures come out.
///
/// The archive name is the segment carrying the container's index, spelled as
/// its volume spells it — which is why the same release reads `ARTHUR.D1` off a
/// 5.25-inch floppy and `ARTHUR.1/ARTHUR.D1` off a 3.5-inch one with
/// subdirectories.
const PRESSES: &[(&str, &str, usize)] = &[
    // *Arthur* r63, on both dumps of the one 3.5-inch pressing.
    ("Arthur Quest 4 Excalibur.2mg", "ARTHUR.1/ARTHUR.D1", 168),
    ("Arthur.po", "ARTHUR.1/ARTHUR.D1", 168),
    // *Journey* r77, on the 3.5-inch image and on the first of its five
    // floppies. Naming any volume of the set answers the same, which is the
    // set-spanning arm working.
    ("Journey.po", "JOURNEY.1/JOURNEY.D1", 135),
    ("journey_s1.dsk", "JOURNEY.D1", 135),
    ("journey_s4.dsk", "JOURNEY.D1", 135),
    // *Shogun* r311 and *Zork Zero* r383, whose plates are on no single floppy.
    ("shogun_s1.dsk", "SHOGUN.D1", 55),
    ("shogun_s3.dsk", "SHOGUN.D1", 55),
    ("zork_zero_1.dsk", "ZORK0.D1", 496),
];

/// The one Apple medium that draws nothing, and why.
const SHORT_JOURNEY: &str = "Journey.2mg";

fn any_present() -> bool {
    PRESSES.iter().any(|(m, ..)| media(m).is_some()) || media(SHORT_JOURNEY).is_some()
}

/// **The headline.** Every Apple press draws its own segmented archive, and the
/// counts are the release's own rather than a neighbouring build's.
///
/// The counts are worth reading beside the other renditions of the same games,
/// which is the sanity check that these are the right plates and not merely
/// *some*: Arthur's Amiga `pic.data` holds 169 against the Apple's 168, Zork
/// Zero's Amiga `Pic.data` 495 against 496, Shogun's 48 against 55. Two builds
/// of one game never hold exactly the same set, and none of these is off by the
/// hundreds that a wrong pairing produces (`Arthur.blb` is 326).
///
/// FALSIFICATION (measured 2026-08-14): revert `ProDos::pictures`'s fall-through
/// to `packed_pictures` — a one-line change leaving every byte of the decoder
/// intact — and this fails on the first row with the reported symptom, an image
/// that carries artwork drawing none:
///
/// ```text
/// Arthur Quest 4 Excalibur.2mg: the press must draw its own artwork
/// ```
///
/// Revert the set-spanning arm in `MountedDisk::pictures` instead and the
/// 5.25-inch rows fail the same way while the `.2mg`/`.po` rows still pass,
/// which is the two doors being genuinely two.
#[test]
fn every_apple_press_draws_its_own_segmented_archive() {
    let mut ran = 0;
    for (image, archive, pictures) in PRESSES {
        let Some(p) = media(image) else {
            eprintln!("SKIP: gitignored medium missing: {image}");
            continue;
        };
        ran += 1;
        let art = app::graphics::release_art(&p, None)
            .unwrap_or_else(|| panic!("{image}: the press must draw its own artwork"));
        assert_eq!(&art.name, archive, "{image}: named for the segment carrying the index");
        assert_eq!(art.pictures.entries().len(), *pictures, "{image}");
        assert_eq!(
            PictSource::resolve(&p, None).all_pict_dims().len(),
            *pictures,
            "{image}: and that is what the story is told it has"
        );
    }
    assert!(ran > 0 || !any_present(), "media are present but no press was measured");
}

/// **The screen every one of them asks for**, and the fact that the ARCHIVE is
/// what asks.
///
/// `InterpreterProfile::AppleIIgs::std_window` is `None` and stays `None`
/// (SQ-0857): the Apple's 140×192 CHARACTER grid on a 3×9 cell is a screen model
/// this codebase cannot express, and expressing it is a run-time-cell refactor
/// nobody has done. The 140×192 PICTURE space is a different quantity, it needs
/// no cell to be true, and the archive states it — so the artwork never had to
/// wait for that decision, which is the whole of what this quest reversed.
#[test]
fn the_screen_is_the_archives_picture_space_at_the_machines_scale() {
    assert_eq!(
        InterpreterProfile::AppleIIgs.std_window(),
        None,
        "the profile still declines; the archive is what supplies the space"
    );
    // The arithmetic, stated once so a factor change cannot pass unnoticed.
    assert_eq!(
        (APPLE_PICTURE_SPACE.0 * 4, APPLE_PICTURE_SPACE.1 * 2),
        APPLE_SCREEN,
        "140x192 at (4, 2) — see `PictSource::art_scale` for where the pair comes from"
    );
    let cell = InterpreterProfile::AppleIIgs.v6_font_cell();
    let (cw, ch) = (cell.w, cell.h);
    assert_eq!(
        (APPLE_SCREEN.0 % cw, APPLE_SCREEN.1 % ch),
        (0, 0),
        "and it is a whole number of cells: 70x24"
    );

    let mut ran = 0;
    for (image, ..) in PRESSES {
        let Some(p) = media(image) else { continue };
        ran += 1;
        let picts = PictSource::resolve(&p, None);
        assert_eq!(picts.native_std_window(), Some(APPLE_PICTURE_SPACE), "{image}");
        assert_eq!(picts.art_scale(), Some((4, 2)), "{image}");
        // The chain `reset.rs` and `startup.rs` walk, in their order: no Blorb
        // `Reso` here, so the archive answers and the profile is never reached.
        let screen = picts
            .std_window()
            .or_else(|| picts.native_std_window())
            .or_else(|| InterpreterProfile::AppleIIgs.std_window())
            .expect("something must state a screen");
        let scale = picts.art_scale().expect("a native archive states one");
        assert_eq!(
            (screen.0 * scale.0 as u16, screen.1 * scale.1 as u16),
            APPLE_SCREEN,
            "{image}: the story's Version 6 screen"
        );
    }
    assert!(ran > 0 || !any_present(), "media are present but no press was measured");
}

/// **The one that stays dark, and it is the image rather than the format.**
///
/// `Journey.2mg` declares five segments and carries four. Its `SGTPICOF` fields
/// name archives on segments 2 to 5, `JOURNEY.D5` is not on the volume, and
/// `infocom_packed::pictures` refuses the set whole rather than serving a
/// picture space with a quarter of the rooms missing — exactly as the story is
/// refused for the same absence.
///
/// This used to be indistinguishable from "lanthorn cannot read this release".
/// It is not any more: the two complete pressings of the SAME release 77 are in
/// [`PRESSES`] above and both draw, so the `.2mg` is a short dump and the
/// refusal is the reader being right.
#[test]
fn the_short_journey_pressing_draws_nothing_because_a_segment_is_absent() {
    let Some(p) = media(SHORT_JOURNEY) else {
        eprintln!("SKIP: gitignored medium missing: {SHORT_JOURNEY}");
        return;
    };
    let files: Vec<(String, Vec<u8>)> =
        app::assets::volumes(&p).iter().flat_map(|v| v.disk.contents()).collect();
    // The premise: the index says there IS artwork, on four of the five segments.
    let (_, offsets) =
        blorb::infocom_packed::picture_offsets(&files).expect("the index reads off segment 1");
    assert_eq!(
        offsets.iter().filter(|o| o.is_some()).count(),
        4,
        "SGTPICOF names an archive on segments 2 to 5"
    );
    // …and the segment carrying the last of them is not here.
    assert!(
        !files.iter().any(|(n, _)| n.to_ascii_uppercase().ends_with("JOURNEY.D5")),
        "the premise: this image is missing its fifth segment"
    );
    assert!(
        blorb::infocom_packed::pictures(&files).is_none(),
        "a partial picture set is refused whole, not served short"
    );
    assert!(app::graphics::release_art(&p, None).is_none(), "so the medium offers no artwork");
    assert_eq!(PictSource::resolve(&p, None).all_pict_dims().len(), 0, "and nothing is drawn");

    // The same release, complete, does draw — which is what makes the sentence
    // above a statement about this IMAGE and not about the Apple press.
    if let Some(whole) = media("journey_s1.dsk") {
        assert_eq!(
            app::graphics::release_art(&whole, None).map(|a| a.pictures.entries().len()),
            Some(135),
            "release 77 draws 135 pictures wherever every segment is present"
        );
    }
}

/// **The control: one press, two dumps, one answer.**
///
/// `Arthur.po` and `Arthur Quest 4 Excalibur.2mg` are the same 3.5-inch pressing
/// of release 63 dumped twice — they differ in nine of 1600 blocks, being
/// `FINDER.DATA` entries one carries and the other does not, and in the wrapper
/// (one is bare, the other 2IMG). Everything the reader answers must be
/// identical across them.
///
/// It is worth a case of its own because the failure it catches is invisible
/// against a single fixture: a reader that keyed off a block NUMBER rather than
/// off the container's own index would pass every other test in this file and
/// disagree here.
#[test]
fn two_dumps_of_one_press_agree_about_story_and_artwork() {
    let (Some(bare), Some(wrapped)) = (media("Arthur.po"), media("Arthur Quest 4 Excalibur.2mg"))
    else {
        eprintln!("SKIP: both dumps of the Arthur press are needed");
        return;
    };
    // The premise: they really are two files, not one path twice.
    assert_ne!(std::fs::read(&bare).unwrap(), std::fs::read(&wrapped).unwrap());

    let story = |p: &PathBuf| app::hints::load_mounted_story(p).expect("opens").0.bytes().to_vec();
    assert_eq!(story(&bare), story(&wrapped), "the same story, byte for byte");

    let art = |p: &PathBuf| app::graphics::release_art(p, None).expect("draws").pictures;
    let (a, b) = (art(&bare), art(&wrapped));
    assert_eq!(a.entries(), b.entries(), "the same directory, entry for entry");
    assert_eq!(a.parts(), b.parts(), "merged out of the same four archives");
    assert_eq!(
        (a.picture_space_width(), a.picture_space_height()),
        (b.picture_space_width(), b.picture_space_height()),
        "and the same picture space"
    );
}
