//! SQ-0866: **artwork from a different build is not drawn into a disk-mounted
//! story** — and the far larger set of pairings that must survive the rule.
//!
//! # The report
//!
//! > "the graphics on 'Arthur Quest 4 Excalibur.2mg' are corrupted once the game
//! > starts. looks like some of the indexes might be corrupted?"
//!
//! and, when a neighbouring loose file was guessed at:
//!
//! > "no, i'm using the default and loading from the 2gs image"
//!
//! # What was measured
//!
//! The reassembly is sound (SQ-0852's packed-volume reader verifies that story
//! against its own header checksum, `$45EB`, and it boots and plays) and the
//! archive on the volume is not misparsed — `MountedDisk::pictures()` offers
//! *nothing* for that image, because `InfocomPics::parse` rejects all five
//! `ARTHUR.D*` segments and every other file on it. So tier 1 ran, and
//! `blorb::resolve_resource_blorb`'s directory scan matched `Arthur.blb` on a
//! six-character stem prefix:
//!
//! | | release | serial | pictures |
//! | --- | --- | --- | --- |
//! | `Arthur Quest 4 Excalibur.2mg` (Apple IIgs) | 63 | 890622 | 168 |
//! | `Arthur.blb` (DOS press) | 74 | 890714 | **326** |
//!
//! The game asked for its own picture numbers and got another build's.
//!
//! # The rule this suite pins
//!
//! `app::graphics::resource_blorb`. A Blorb is refused when it **contradicts**
//! the story — it carries the Blorb spec's optional `IFhd` Game Identifier, the
//! story came off a disk image, and that identifier matches no build on the
//! release. Everything else keeps its artwork, and the cases below are chosen to
//! be the ones that would break first if the rule were widened:
//!
//! - a Blorb that states no build (most of the corpus) — [`a_blorb_that_states_no_build_is_never_refused`]
//! - a loose story whose Blorb states a DIFFERENT build on purpose (FMV Poker) —
//!   [`fmvpoker_keeps_the_plates_its_readme_tells_the_player_to_borrow`]
//! - a disk that supplies its own art — [`a_disk_with_its_own_artwork_is_never_asked_about_a_blorb`]
//! - the whole corpus — [`no_medium_in_the_corpus_loses_artwork_except_the_mismatched_one`]
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! when its fixture is missing and every `ran > 0` guard is gated on a
//! presence check — CI has none of this on any platform and must not fail on its
//! absence.

use std::path::PathBuf;

use app::graphics::{resource_blorb, PictSource};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn media(name: &str) -> Option<PathBuf> {
    let p = stories_dir().join(name);
    p.is_file().then_some(p)
}

fn any_present(names: &[&str]) -> bool {
    names.iter().any(|n| media(n).is_some())
}

/// The Apple IIgs press, and the DOS-press Blorb that was being drawn into it.
const IIGS_ARTHUR: &str = "Arthur Quest 4 Excalibur.2mg";
const ARTHUR_BLORB: &str = "Arthur.blb";

/// How many pictures the story would actually draw with.
fn drawn_pictures(path: &std::path::Path) -> usize {
    PictSource::resolve(path).all_pict_dims().len()
}

/// The reported case. The IIgs disk draws **nothing** rather than the 326
/// pictures of a release it is not.
#[test]
fn the_apple_iigs_arthur_draws_nothing_rather_than_another_builds_pictures() {
    let Some(disk) = media(IIGS_ARTHUR) else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_ARTHUR}");
        return;
    };
    // The premise: the Blorb the old scan reached really does hold 326 pictures,
    // so "nothing" below is a refusal and not an empty file.
    if let Some(blb) = media(ARTHUR_BLORB) {
        let raw = std::fs::read(&blb).unwrap();
        let b = blorb::Blorb::parse(raw).unwrap();
        assert_eq!(
            b.resources().iter().filter(|r| &r.usage == b"Pict").count(),
            326,
            "{ARTHUR_BLORB} is the 326-picture DOS set"
        );
        assert_eq!(
            b.game_identifier().map(|g| (g.release, g.serial_str().into_owned())),
            Some((74, "890714".to_string())),
            "and it says so in its IFhd"
        );
    }
    assert_eq!(drawn_pictures(&disk), 0, "{IIGS_ARTHUR} must draw no artwork at all");
    assert!(
        resource_blorb(&disk).found.is_none(),
        "and must reach no resource Blorb to draw it from"
    );
}

/// Drawing nothing is only honest if the player is told why. The complaint names
/// the file, the build it is for, and the build on the disk.
#[test]
fn the_refusal_names_the_archive_and_both_builds() {
    let Some(disk) = media(IIGS_ARTHUR) else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_ARTHUR}");
        return;
    };
    if media(ARTHUR_BLORB).is_none() {
        eprintln!("SKIP: gitignored sidecar missing: {ARTHUR_BLORB}");
        return;
    }
    let said = resource_blorb(&disk).refused.expect("a refusal must say so");
    for want in ["Arthur.blb", "release 74", "890714", "release 63", "890622"] {
        assert!(said.contains(want), "the complaint must name {want:?}: {said}");
    }
}

/// The launch dialog's default row is derived from the boot path, so it must now
/// read "no artwork found" rather than offering an archive the boot will refuse
/// (SQ-0865's property, held under SQ-0866's rule).
#[test]
fn the_launch_dialog_offers_no_default_where_the_boot_draws_nothing() {
    let Some(disk) = media(IIGS_ARTHUR) else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_ARTHUR}");
        return;
    };
    assert_eq!(
        app::launch_options::resolved_default_art(&disk),
        None,
        "the default row must not name an archive the boot refuses"
    );
}

/// **The line, half one.** A Blorb that carries no `IFhd` states no build, and
/// silence is not disagreement — most of the corpus is in this case and all of it
/// must keep drawing.
#[test]
fn a_blorb_that_states_no_build_is_never_refused() {
    // Loose stories whose sidecars carry no IFhd at all.
    // Deliberately not `advent.z6`: `advent.blb` carries its own `Exec`, so
    // `resolve_resource_blorb` declines it as a sidecar for any story and this
    // rule never sees it.
    const CASES: &[(&str, &str)] = &[
        ("scopa.z6", "scopa.blb"),
        ("sherlock-r26-s880127.z5", "Sherlock.blb"),
        ("mysterious01.z6", "mysterious01.blb"),
        ("mysterious07.z6", "Mysterious07.blb"),
        ("beyondzork-r57-s871221.z5", "beyondzork.blb"),
    ];
    let names: Vec<&str> = CASES.iter().map(|(s, _)| *s).collect();
    let mut ran = 0;
    for (story, sidecar) in CASES {
        let Some(p) = media(story) else { continue };
        let rb = resource_blorb(&p);
        let refused = rb.refused.clone();
        let (blorb, path) =
            rb.found.unwrap_or_else(|| panic!("{story} must still reach {sidecar}: {refused:?}"));
        assert!(blorb.game_identifier().is_none(), "{sidecar} states no build");
        assert!(
            path.file_name().unwrap().eq_ignore_ascii_case(std::ffi::OsStr::new(sidecar)),
            "{story} resolves {sidecar}, got {path:?}"
        );
        assert!(rb.refused.is_none(), "{story}: nothing to refuse");
        ran += 1;
    }
    assert!(ran > 0 || !any_present(&names), "no case ran but the fixtures are here");
}

/// **The line, half two.** A loose story sits in a directory a *person*
/// assembled, and that placement is the pairing assertion.
///
/// *Frobozz Magic Video Poker* states the case outright: `fmvpoker.blb` is a
/// byte-for-byte copy of `Zork0.blb`, so its `IFhd` names Zork Zero (release 393,
/// serial 890714) while `fmvpoker.z6` is release 60, serial 001227. Its own
/// readme is an instruction to do exactly that — *"Obtain one of the Zork Zero
/// graphics files (zork0.eg1, zork0.cg1, or zork0.mg1). Rename the graphics file
/// to FMVPOKER"* — and borrowing another game's plates is the whole design of it.
///
/// This is the case that forbids applying the rule on the `IFhd` alone.
#[test]
fn fmvpoker_keeps_the_plates_its_readme_tells_the_player_to_borrow() {
    let Some(p) = media("fmvpoker.z6") else {
        eprintln!("SKIP: gitignored story missing: fmvpoker.z6");
        return;
    };
    let rb = resource_blorb(&p);
    let (blorb, path) = rb.found.expect("FMV Poker must keep its borrowed art");
    assert_eq!(path.file_name().unwrap(), "fmvpoker.blb");
    // The mismatch is real and deliberate: the Blorb says Zork Zero, the story is
    // not Zork Zero, and it is drawn anyway.
    let stated = blorb.game_identifier().expect("the copied Blorb carries Zork Zero's IFhd");
    assert_eq!((stated.release, stated.serial_str().into_owned()), (393, "890714".into()));
    let own = blorb::GameIdentifier::of_story(&std::fs::read(&p).unwrap()).unwrap();
    assert_ne!(own.release, stated.release, "the story really is a different build");
    assert!(rb.refused.is_none(), "a loose story's own directory is not overruled");
    assert!(drawn_pictures(&p) > 0, "and it draws");
}

/// A Blorb whose `IFhd` AGREES keeps drawing — the positive half of the check,
/// and the four Infocom v6 releases the corpus pairs correctly.
#[test]
fn a_loose_story_keeps_the_blorb_that_states_its_own_build() {
    const CASES: &[(&str, &str, u16)] = &[
        ("arthur-r74-s890714.z6", "Arthur.blb", 74),
        ("journey-r83-s890706.z6", "Journey.blb", 83),
        ("shogun-r322-s890706.z6", "Shogun.blb", 322),
        ("zork0-r393-s890714.z6", "Zork0.blb", 393),
    ];
    let names: Vec<&str> = CASES.iter().map(|(s, _, _)| *s).collect();
    let mut ran = 0;
    for (story, sidecar, release) in CASES {
        let Some(p) = media(story) else { continue };
        let rb = resource_blorb(&p);
        let (blorb, path) = rb.found.expect("a matching build must not be refused");
        assert_eq!(path.file_name().unwrap(), *sidecar);
        assert_eq!(blorb.game_identifier().unwrap().release, *release);
        assert!(drawn_pictures(&p) > 0, "{story} draws");
        ran += 1;
    }
    assert!(ran > 0 || !any_present(&names), "no case ran but the fixtures are here");
}

/// **The medium always wins first**, so a disk carrying its own artwork never
/// reaches the rule at all — including the Amiga *Arthur*, whose neighbouring
/// `Arthur.blb` states a build it does not share.
#[test]
fn a_disk_with_its_own_artwork_is_never_asked_about_a_blorb() {
    const CASES: &[(&str, &str)] = &[
        ("Arthur - The Quest for Excalibur.adf", "pic.data"),
        ("Journey - The Quest Begins.adf", "Pic.data"),
        ("James Clavell's Shogun.adf", "Pic.data"),
        ("Zork Zero Disk.image", "CPic.data"),
        ("Zork Zero - The Revenge of Megaboz.adf", "Pic.data"),
    ];
    let names: Vec<&str> = CASES.iter().map(|(s, _)| *s).collect();
    let mut ran = 0;
    for (disk, archive) in CASES {
        let Some(p) = media(disk) else { continue };
        let art = app::graphics::release_art(&p).expect("this medium supplies its own art");
        assert_eq!(&art.name, archive, "{disk} draws with its own {archive}");
        assert!(drawn_pictures(&p) > 0, "{disk} draws");
        ran += 1;
    }
    assert!(ran > 0 || !any_present(&names), "no case ran but the fixtures are here");
}

/// **The rule refuses only on proof, never on absence of evidence.**
///
/// Swept over the whole corpus as an invariant rather than pinned to a list, so
/// it stays true as the disk readers improve: a refusal requires that the release
/// yielded at least one identifiable build AND that the Blorb's stated build is
/// none of them. Nothing may be refused merely because a story could not be read.
///
/// Three media are in that "could not be read" state today and all three keep
/// their Blorb: `Journey.2mg`, whose volume mounts but yields no story, and the
/// `shogun_s*.dsk` / `zork_zero_*.dsk` Apple II presses, whose stories are paged
/// across the whole set while the identity check asks each volume on its own.
/// When that reader lands, those releases start being checked and this test keeps
/// passing either way — which is the point of stating it as an invariant.
#[test]
fn nothing_is_refused_for_want_of_evidence() {
    let dir = stories_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: gitignored corpus missing at {}", dir.display());
        return;
    };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    paths.sort();
    let mut refusals = 0;
    let mut ran = 0;
    for p in paths {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "2mg" | "adf" | "image" | "dsk" | "ima" | "img" | "st" | "d64") {
            continue;
        }
        ran += 1;
        let Some(said) = resource_blorb(&p).refused else { continue };
        // Only a medium with no artwork of its own ever reaches tier 1, so a
        // refusal on one that has some is inert — the Amiga `Arthur` and
        // `Journey` floppies are both contradicted by the DOS Blorb beside them
        // and both draw their own `Pic.data` regardless.
        if app::graphics::release_art(&p).is_none() {
            refusals += 1;
        }
        let builds = app::graphics::release_builds(&p);
        assert!(
            !builds.is_empty(),
            "{p:?} was refused with no identifiable build to compare against: {said}"
        );
        let stated = blorb::resolve_resource_blorb(&p)
            .and_then(|(b, _)| b.game_identifier())
            .expect("a refusal implies the Blorb stated a build");
        assert!(
            !builds.contains(&stated),
            "{p:?} was refused although {stated} is on the release"
        );
    }
    assert!(ran > 0 || !dir.is_dir(), "the corpus is here but no medium was measured");
    // The corpus does contain a real mismatch, so an invariant that held only
    // because nothing was ever refused would be worthless.
    if media(IIGS_ARTHUR).is_some() && media(ARTHUR_BLORB).is_some() {
        assert_eq!(refusals, 1, "exactly one refusal changes what a player sees");
    }
}

/// **Guard 1, as a test.** Sweep every medium and story in `stories/` and require
/// that the artwork source each one resolves is unchanged by the rule — with the
/// single, named exception that is the whole point of the quest.
///
/// This is the case that catches a widening: an over-broad rule strips artwork
/// from most of the corpus, and it would fail here with a list of what it took.
#[test]
fn no_medium_in_the_corpus_loses_artwork_except_the_mismatched_one() {
    let dir = stories_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: gitignored corpus missing at {}", dir.display());
        return;
    };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    paths.sort();
    let mut lost: Vec<String> = Vec::new();
    let mut ran = 0;
    for p in paths {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(
            ext.as_str(),
            "z3" | "z4"
                | "z5"
                | "z6"
                | "z8"
                | "zblorb"
                | "blorb"
                | "gblorb"
                | "blb"
                | "2mg"
                | "adf"
                | "image"
                | "dsk"
                | "ima"
                | "img"
                | "st"
                | "d64"
        ) {
            continue;
        }
        // The medium's own art is resolved ahead of tier 1 and cannot be touched
        // by the rule, so only the Blorb tier is compared.
        if app::graphics::release_art(&p).is_some() {
            ran += 1;
            continue;
        }
        let before = blorb::resolve_resource_blorb(&p).map(|(_, bp)| bp);
        let after = resource_blorb(&p).found.map(|(_, bp)| bp);
        if before.is_some() {
            ran += 1;
        }
        if before != after {
            lost.push(format!(
                "{}: {:?} -> {:?}",
                p.file_name().unwrap().to_string_lossy(),
                before.as_ref().and_then(|b| b.file_name()),
                after.as_ref().and_then(|b| b.file_name()),
            ));
        }
    }
    // A partial corpus is normal (and CI has none of it), so the expectation is
    // built from what is actually on disk rather than asserted flat.
    let expected: Vec<String> = match media(IIGS_ARTHUR).is_some() && media(ARTHUR_BLORB).is_some() {
        true => vec![format!("{IIGS_ARTHUR}: Some(\"{ARTHUR_BLORB}\") -> None")],
        false => Vec::new(),
    };
    assert_eq!(lost, expected, "exactly one pairing may change, and it is the reported one");
    assert!(ran > 0 || !dir.is_dir(), "the corpus is here but nothing was measured");
}
