//! SQ-0873, the consuming half: a v1–v4 story off a release disk gets its own
//! machine's screen, and everything else gets the theme.
//!
//! `app::period`'s unit tests pin the gate and the paint against the machine
//! table. This suite pins the part they cannot reach — the chain from a **real
//! disk on disk** to a look: the medium is read out of the bytes
//! (`InterpreterProfile::resolve`), the machine comes from the medium, the look
//! comes from the machine, and the story's own Version decides whether it applies
//! at all. Every link is somebody else's code and the whole point is that they
//! agree.
//!
//! **The Version is still read off the story rather than assumed**, and the same
//! Amiga shelf carries v3 (*Zork I*), v5 (*Sherlock*) and v6 (*Zork Zero*) — but
//! all three are dressed now (SQ-0935). What the Version decides is narrower than
//! it was: the machine's SCREEN applies to every version an Infocom interpreter
//! shipped for, and only the STATUS BAND stops at v4, because that is the row whose
//! owner changes. v7 and v8 decline outright, never having run on one of these
//! machines at all.
//!
//! **And the CARET moves with the Version too** (SQ-0947). The Amiga's orange
//! block and the IBM PC's underscore were measured on v1–v5 captures, and both
//! machines' v6 interpreters draw the pair on screen reversed instead — so a look
//! off a v6 disk is no longer the stored row, and the case that used to compare it
//! with one now compares it with the machine's answer for that Version.
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously when the
//! disks are absent — and each one asserts it saw at least one specimen first, so
//! a skip cannot pass for a pass. Watch the SKIP notes when adding a specimen: a
//! release whose story is on none of its disks individually loads as nothing, and
//! a present fixture then skips exactly like an absent one.

use std::path::{Path, PathBuf};

use app::interpreter::InterpreterProfile;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// One release medium, and what it is: the machine its bytes imply, the story's
/// Version, and whether that combination gets a period look.
struct Specimen {
    file: &'static str,
    machine: InterpreterProfile,
    zversion: u8,
    /// Whether this launch is dressed in its machine's screen. True for every
    /// version an Infocom machine shipped an interpreter for — v1–v6 since
    /// SQ-0935, where it used to stop at v4.
    dressed: bool,
}

/// The disks this suite drives, with the release each was confirmed to carry.
///
/// The Versions are the ones the games print at their own banners, checked
/// against the files in `stories/`: *Zork I* revision 88 / 840726 and *Zork II*
/// version 48 / 840904 are v3; *Sherlock* release 26 / 880127 announces
/// "Interpreter 4 Version A" and is v5; *Hitchhiker's* release 47 / 840914 off
/// the 1541 image is v3.
const SPECIMENS: &[Specimen] = &[
    Specimen {
        file: "Zork I - The Great Underground Empire.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 3,
        dressed: true,
    },
    Specimen {
        file: "Zork II - The Wizard of Frobozz.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 3,
        dressed: true,
    },
    Specimen {
        file: "Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64",
        machine: InterpreterProfile::Commodore128,
        zversion: 3,
        dressed: true,
    },
    // The same shelf, past the OLD colour boundary. These two are why the suite's
    // doc used to say the Version was "the interesting variable": they were the
    // clause that had to decline. They are dressed now, and the reason is in
    // `app::period`'s module docs — the machine's measured RGB and the `$2C`/`$2D`
    // numbers a v5+ story reads are two spellings of one row, so painting the first
    // while answering the second is not a lie about the screen, it IS the screen.
    Specimen {
        file: "Sherlock - The Riddle of the Crown Jewels.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 5,
        dressed: true,
    },
    Specimen {
        file: "Zork Zero - The Revenge of Megaboz.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 6,
        dressed: true,
    },
    // SQ-0947: the same game on the other machine whose v6 caret was reported
    // wrong, so both halves of the report are driven off real media here.
    //
    // Zork Zero release 393 / serial 890714 on a DOS floppy — the row
    // `real_media_releases.rs` pins as "Zork Zero (DOS)", and a different press
    // from the Amiga's release 366 above (CLAUDE.md: a disk image is a different
    // release). It is `floppy5.ima` because that is *The Lost Treasures of
    // Infocom*'s fifth disk; the retail 360K set is in `stories/` too and is
    // deliberately NOT used, because its Disk 1 holds the boot files and no story
    // at all, so it would skip exactly like a fixture that is not there.
    Specimen {
        file: "floppy5.ima",
        machine: InterpreterProfile::IbmPc,
        zversion: 6,
        dressed: true,
    },
];

/// The story's Version byte, straight off whatever the disk yielded, or `None`
/// when the disk is not here.
fn zversion_of(path: &Path) -> Option<u8> {
    match app::hints::load_story(path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b.first().copied(),
        _ => None,
    }
}

#[test]
fn the_medium_names_the_machine_and_the_version_decides_whether_it_dresses() {
    let dir = stories_dir();
    let mut seen = 0;
    for s in SPECIMENS {
        let path = dir.join(s.file);
        let Some(zversion) = zversion_of(&path) else {
            eprintln!("SKIP: gitignored disk missing at {}", path.display());
            continue;
        };
        seen += 1;
        assert_eq!(zversion, s.zversion, "{} is not the release this suite pins", s.file);

        // The medium is read out of the bytes, not inferred from the extension.
        let profile = InterpreterProfile::resolve(&path, None, None, None);
        assert_eq!(profile, s.machine, "{} implies the wrong machine", s.file);

        // The machine has a look either way — all three here are captured.
        assert!(profile.period_look().is_some(), "{:?} is a measured machine", s.machine);

        // …and only a pre-colour story is dressed in it.
        let look = app::period::resolve(profile, true, true, true, Some(zversion));
        assert_eq!(
            look.is_some(),
            s.dressed,
            "{} is v{zversion}: every version an Infocom machine shipped for is dressed",
            s.file
        );
        if let Some(look) = look {
            // Against the machine's answer FOR THIS VERSION, not against the stored
            // row. The two diverge on purpose — the IBM PC's white moves with the
            // palette (SQ-0939) and two machines' carets move at v6 (SQ-0947) — so
            // comparing with the row would pin the drift rather than the screen.
            assert_eq!(
                Some(look),
                zvm::interpreter::period_look_for(profile.row_number(), Some(zversion)),
                "{} got another machine's screen",
                s.file
            );
        }
    }
    // A fixture that is ABSENT is a clean skip (CLAUDE.md: `stories/` is
    // gitignored, so CI has none). A fixture that is PRESENT and yielded nothing
    // is a defect, and that is what this catches — the shape
    // `honor_colours_artwork_pin` already uses.
    let any_present = SPECIMENS.iter().any(|s| dir.join(s.file).is_file());
    assert!(!any_present || seen > 0, "disks are present but none was read");
}

/// The two switches, on real media. `honor_game_colours` is the broad one and
/// takes the look with it; `period_look` is the narrow one and goes alone.
#[test]
fn either_switch_declines_and_only_the_broad_one_is_the_master() {
    let dir = stories_dir();
    let mut seen = 0;
    for s in SPECIMENS.iter().filter(|s| s.dressed) {
        let path = dir.join(s.file);
        let Some(zversion) = zversion_of(&path) else {
            eprintln!("SKIP: gitignored disk missing at {}", path.display());
            continue;
        };
        seen += 1;
        let profile = InterpreterProfile::resolve(&path, None, None, None);
        assert!(app::period::resolve(profile, true, true, true, Some(zversion)).is_some());
        assert!(
            app::period::resolve(profile, true, false, true, Some(zversion)).is_none(),
            "{}: colours off takes the look with it",
            s.file
        );
        assert!(
            app::period::resolve(profile, false, true, true, Some(zversion)).is_none(),
            "{}: and the narrow key declines on its own",
            s.file
        );
    }
    let any_present = SPECIMENS.iter().filter(|s| s.dressed).any(|s| dir.join(s.file).is_file());
    assert!(!any_present || seen > 0, "disks are present but none was read");
}

/// SQ-0947: the caret a **Version 6** launch off a release disk gets is the pair
/// on screen reversed, on the two machines whose v6 interpreter drew one that way.
///
/// Reported by eye against the retail games: "amiga zork-zero is wrong color --
/// orange, dos v6 games should be reversed space". Both were stored measurements
/// applied a version too far — the Amiga's fixed `#FF7E1C` block and the IBM PC's
/// underscore come from v3 captures (`amiga-spellbreaker.png`,
/// `dos-hitchhiker.png`), and three v6 frames in `machine-screenshots/` disagree
/// with them: `amiga-zorkzero.png` draws a black 8x15 block after `[MORE]` on Zork
/// Zero's own grey page, `amiga-shogun.png` a white 8x16 one after the `>` on
/// Shogun's dark page, and `dos-arthur.png` a solid white 18x36 cell after `>exam`
/// on EGA blue.
///
/// The unit cases pin the rule against the table. What this one adds is that the
/// disks in `stories/` actually reach it: a real Amiga floppy and a real DOS floppy
/// of the same game, through `InterpreterProfile::resolve` and `app::period`.
///
/// The same shelf's v3 and v5 disks are asserted alongside, because a fix that
/// simply stopped drawing the machine's caret would pass the v6 half and silently
/// undo SQ-0873 everywhere else.
#[test]
fn the_version_six_caret_off_a_release_disk_is_the_pair_reversed() {
    use zvm::interpreter::CursorShape;

    let dir = stories_dir();
    let mut seen = 0;
    let mut reversing = 0;
    for s in SPECIMENS {
        let path = dir.join(s.file);
        let Some(zversion) = zversion_of(&path) else {
            eprintln!("SKIP: gitignored disk missing at {}", path.display());
            continue;
        };
        seen += 1;
        let profile = InterpreterProfile::resolve(&path, None, None, None);
        let look = app::period::resolve(profile, true, true, true, Some(zversion))
            .expect("every specimen here is dressed");

        let v6_reversing_machine = zversion == 6
            && matches!(profile, InterpreterProfile::Amiga | InterpreterProfile::IbmPc);
        if v6_reversing_machine {
            reversing += 1;
            assert_eq!(
                look.cursor_shape,
                CursorShape::ReverseSpace,
                "{}: v6 on {profile:?} reverses the live pair",
                s.file
            );
            assert_ne!(
                look.cursor_colour,
                (0xFF, 0x7E, 0x1C),
                "{}: and never the v3 interpreter's orange",
                s.file
            );
        } else {
            // The rest of the shelf keeps the caret its own capture measured.
            assert_eq!(
                look.cursor_shape,
                profile.period_look().expect("measured").cursor_shape,
                "{}: v{zversion} on {profile:?} keeps its measured caret",
                s.file
            );
        }
    }
    // Non-vacuity, twice over: the suite must have read a disk at all, and — when
    // the v6 disks are here — must have exercised the branch this case exists for.
    let any_present = SPECIMENS.iter().any(|s| dir.join(s.file).is_file());
    assert!(!any_present || seen > 0, "disks are present but none was read");
    let v6_present = SPECIMENS.iter().any(|s| s.zversion == 6 && dir.join(s.file).is_file());
    assert!(
        !v6_present || reversing > 0,
        "a v6 disk is here and the branch this case exists for never ran",
    );
}

/// An ordinary story file is not a machine, so it is not dressed as one — the
/// same rule that keeps `--interpreter` the only other door in.
///
/// Uses a freely-redistributable fixture from `unit_tests/`, so unlike the cases
/// above this one always runs.
#[test]
fn a_bare_story_file_off_no_disk_at_all_keeps_the_theme() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zvm/tests/fixtures/czech.z5");
    assert!(path.exists(), "the redistributable fixture is checked in");
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    assert_eq!(profile, InterpreterProfile::IbmPc, "no medium, no machine");
    // SQ-0873/SQ-0928: the IBM PC HAS a period look now (`dos-hitchhiker.png`), so
    // what keeps a bare story file on the player's theme is the LICENCE, not the
    // absence of a measurement. That is the stronger guarantee — it holds for every
    // machine, including ones measured later.
    assert!(profile.period_look().is_some(), "the machine states one");
    for v in 1..=8 {
        assert!(
            app::period::resolve(profile, true, true, false, Some(v)).is_none(),
            "v{v}: a launch that named no machine is never dressed as one",
        );
    }
}
