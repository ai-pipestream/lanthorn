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
//! **The Version is the interesting variable and it is read off the story, not
//! assumed.** The same Amiga floppy shelf carries v3 (*Zork I*), v5 (*Sherlock*)
//! and v6 (*Zork Zero*), so one medium answering one machine has to produce a
//! look for the first and nothing for the other two. That is the clause a future
//! refactor is most likely to lose, because it is the one that looks redundant:
//! the profile is right, the machine is right, the look is right, and applying it
//! would still be wrong.
//!
//! `stories/` is gitignored (CLAUDE.md), so every case skips vacuously when the
//! disks are absent — and each one asserts it saw at least one specimen first, so
//! a skip cannot pass for a pass.

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
    /// A capture-bearing machine AND a pre-colour story.
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
    // The same shelf, past the colour boundary: the machine is right, the look
    // exists, and it must not be applied.
    Specimen {
        file: "Sherlock - The Riddle of the Crown Jewels.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 5,
        dressed: false,
    },
    Specimen {
        file: "Zork Zero - The Revenge of Megaboz.adf",
        machine: InterpreterProfile::Amiga,
        zversion: 6,
        dressed: false,
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
            "{} is v{zversion}: colour arrives with v5 and the look stops there",
            s.file
        );
        if let Some(look) = look {
            assert_eq!(Some(look), profile.period_look(), "{} got another machine's screen", s.file);
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
