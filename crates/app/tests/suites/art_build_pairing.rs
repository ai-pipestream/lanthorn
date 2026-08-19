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
//! archive on the volume was not misparsed — `MountedDisk::pictures()` offered
//! *nothing* for that image, because `InfocomPics::parse` rejected all five
//! `ARTHUR.D*` segments and every other file on it. (SQ-0863 has since read
//! them: the archives are INSIDE the segments, in an Apple flavour with 8-byte
//! directory records, and the disk now draws its own 168 pictures. The rule
//! below is unchanged by that and the cases say so.) So tier 1 ran, and
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

/// **The same press, dumped twice** — a bare ProDOS 3.5-inch image of release
/// 63, differing from the `.2mg`'s body in nine of 1600 blocks (`FINDER.DATA`
/// entries the other lacks). It is the control for the whole reader: two
/// independent dumps of one pressing must yield the same story and the same
/// artwork, or something in the reader is keying off incidental block placement
/// rather than off the container, which is a class of defect a single fixture
/// cannot show.
const ARTHUR_PO: &str = "Arthur.po";

/// The Apple II press of *Journey*, whose story is paged across five segments of
/// which the image carries four (SQ-0852) — and the DOS-press Blorb beside it.
///
/// **A short pressing, not an unreadable one**, and the corpus can now prove it:
/// [`JOURNEY_SET`] and [`JOURNEY_PO`] are complete images of the same release 77
/// and both load and draw. This one is byte-identical to its archive's canonical
/// copy, so the missing `JOURNEY.D5` is how it shipped.
const IIGS_JOURNEY: &str = "Journey.2mg";
const JOURNEY_BLORB: &str = "Journey.blb";

/// The five-volume 5.25-inch press of *Journey* — the same release 77 with every
/// segment present, so it reassembles and draws where the `.2mg` does neither.
const JOURNEY_SET: [&str; 5] =
    ["journey_s1.dsk", "journey_s2.dsk", "journey_s3.dsk", "journey_s4.dsk", "journey_s5.dsk"];

/// …and the 3.5-inch consolidated pressing of it: one BARE ProDOS volume with
/// all five segments in `JOURNEY.1/`…`JOURNEY.5/`, which is *Arthur*'s layout.
const JOURNEY_PO: &str = "Journey.po";

/// The five-volume Apple II press of *Shogun*, whose story is on no single
/// floppy, and the Blorb beside it.
const SHOGUN_SET: [&str; 5] =
    ["shogun_s1.dsk", "shogun_s2.dsk", "shogun_s3.dsk", "shogun_s4.dsk", "shogun_s5.dsk"];
const SHOGUN_BLORB: &str = "Shogun.blb";

/// …and the 3.5-inch consolidation of that set, which joined the census in
/// SQ-0889 when `blorb::prodos` learned to unwrap its DiskCopy 4.2 header.
///
/// It is contradicted by the same `Shogun.blb` for the same reason the five
/// floppies are — the Blorb is release 322 / serial 890706 and the Apple press
/// is release 311 / serial 890510 — so it arrives already refused, and that is
/// the census moving without the rule moving, exactly as this file's invariant
/// promises. The refusal is inert: the disk draws its own segmented plates.
const SHOGUN_PO: &str = "Shogun.po";

/// Every medium in the corpus whose sidecar Blorb is refused, paired with the
/// sidecar refused for it, **in `stories/` sort order** (SQ-0866, SQ-0867).
///
/// Stated once because two corpus-wide guards below both need it, and they must
/// never be able to disagree about which pairings the rule changes: one counts
/// the refusals and the other names them. Each entry is a measured
/// contradiction, and the case that measures it is named in this file.
fn mismatched() -> Vec<(String, &'static str)> {
    let mut all = vec![
        (IIGS_ARTHUR.to_string(), ARTHUR_BLORB),
        (ARTHUR_PO.to_string(), ARTHUR_BLORB),
        (IIGS_JOURNEY.to_string(), JOURNEY_BLORB),
        (JOURNEY_PO.to_string(), JOURNEY_BLORB),
        // The two Amiga floppies, contradicted by the same DOS Blorbs and
        // drawing their own `Pic.data` throughout — refused since SQ-0866 and
        // listed here since SQ-0863, when the census stopped counting only the
        // refusals a player could SEE. Nothing about them has ever moved.
        ("Arthur - The Quest for Excalibur.adf".to_string(), ARTHUR_BLORB),
        ("Journey - The Quest Begins.adf".to_string(), JOURNEY_BLORB),
    ];
    all.extend(SHOGUN_SET.iter().map(|v| (v.to_string(), SHOGUN_BLORB)));
    all.push((SHOGUN_PO.to_string(), SHOGUN_BLORB));
    all.extend(JOURNEY_SET.iter().map(|v| (v.to_string(), JOURNEY_BLORB)));
    all.retain(|(m, b)| media(m).is_some() && media(b).is_some());
    all.sort();
    all
}

/// How many pictures the story would actually draw with.
fn drawn_pictures(path: &std::path::Path) -> usize {
    PictSource::resolve(path, None).all_pict_dims().len()
}

/// The reported case. The IIgs disk draws **its own 168 pictures** rather than
/// the 326 of a release it is not.
///
/// It drew nothing at all when SQ-0866 fixed the corruption, and that was the
/// right answer with the evidence of the day: the archives were on the disk and
/// unreadable, so refusing the DOS Blorb left the screen bare. SQ-0863 read
/// them — `blorb::infocom_pics`'s Apple flavour, off the segmented container —
/// and `ProDos::pictures` now offers them, so the refusal costs nothing at all.
/// **The rule is unchanged and the outcome is better**, which is the only way
/// this assertion was ever supposed to move.
#[test]
fn the_apple_iigs_arthur_draws_its_own_pictures_not_another_builds() {
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
    assert_eq!(drawn_pictures(&disk), 168, "{IIGS_ARTHUR} must draw its own 168 pictures");
    let art = app::graphics::release_art(&disk, None).expect("off the disk's own segments");
    assert_eq!(art.name, "ARTHUR.1/ARTHUR.D1", "named for the segment carrying the index");
    assert!(
        resource_blorb(&disk).found.is_none(),
        "and must still reach no resource Blorb — the refusal is unchanged"
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

/// The launch dialog's default row is derived from the boot path, so it must
/// name exactly what the boot will draw and nothing else (SQ-0865's property,
/// held under SQ-0866's rule and then under SQ-0863's reader).
///
/// Two media, because the property has two sides and only pinning both keeps it
/// honest: `Arthur Quest 4 Excalibur.2mg` draws its own segmented archive and the
/// row names it, while `Journey.2mg` is a short pressing that draws nothing and
/// the row must stay empty rather than offer the Blorb the boot refuses.
#[test]
fn the_launch_dialog_names_exactly_what_the_boot_will_draw() {
    if let Some(disk) = media(IIGS_ARTHUR) {
        let row = app::launch_options::resolved_default_art(&disk, None)
            .expect("the boot draws, so the dialog must say what with");
        assert_eq!(row.filename, "ARTHUR.1/ARTHUR.D1");
        assert_eq!(row.pictures, 168);
        assert!(row.on_medium, "it is on the disk, not beside it");
    } else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_ARTHUR}");
    }
    if let Some(disk) = media(IIGS_JOURNEY) {
        assert_eq!(
            app::launch_options::resolved_default_art(&disk, None),
            None,
            "the default row must not name an archive the boot refuses"
        );
    } else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_JOURNEY}");
    }
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
///
/// And so it is never TOLD about it either (SQ-0882). The refusal still fires
/// on these disks — `Arthur.blb` genuinely contradicts release 54/890606 and
/// 63/890622 — but it declined a file the boot was never going to reach, so
/// there is no news in it. Reported anyway, the one sentence the player saw
/// ended "a different build's pictures are not being drawn" while their disk was
/// drawing perfectly well from `Pic.data`. `unpaired_art_warning` asks what won
/// rather than what was declined; every row here must come back silent.
#[test]
fn a_disk_with_its_own_artwork_is_never_asked_about_a_blorb() {
    const CASES: &[(&str, &str)] = &[
        ("Arthur - The Quest for Excalibur.adf", "pic.data"),
        ("Journey - The Quest Begins.adf", "Pic.data"),
        ("James Clavell's Shogun.adf", "Pic.data"),
        ("Zork Zero Disk.image", "CPic.data"),
        ("Zork Zero - The Revenge of Megaboz.adf", "Pic.data"),
        // …and the Apple II presses, whose archive is not a FILE at all but the
        // artwork inside the segmented container (SQ-0863). Three of them are
        // exactly the media SQ-0866 had to leave dark, so these rows are where
        // "the medium always wins first" stopped being a rule with a hole in it.
        ("Arthur Quest 4 Excalibur.2mg", "ARTHUR.1/ARTHUR.D1"),
        ("Journey.po", "JOURNEY.1/JOURNEY.D1"),
        ("journey_s1.dsk", "JOURNEY.D1"),
        ("shogun_s1.dsk", "SHOGUN.D1"),
        ("zork_zero_1.dsk", "ZORK0.D1"),
    ];
    let names: Vec<&str> = CASES.iter().map(|(s, _)| *s).collect();
    let mut ran = 0;
    for (disk, archive) in CASES {
        let Some(p) = media(disk) else { continue };
        let art = app::graphics::release_art(&p, None).expect("this medium supplies its own art");
        assert_eq!(&art.name, archive, "{disk} draws with its own {archive}");
        assert!(drawn_pictures(&p) > 0, "{disk} draws");
        assert_eq!(
            app::graphics::unpaired_art_warning(&p, None),
            None,
            "{disk} draws its own {archive}, so it has nothing to be warned about — \
             whatever sidecar was refused was never going to be reached"
        );
        ran += 1;
    }
    assert!(ran > 0 || !any_present(&names), "no case ran but the fixtures are here");
}

/// The warning SURVIVES where it was earned: a disk with no artwork of its own
/// and a sidecar refused for naming another build (SQ-0882's other half).
///
/// `Journey.2mg` is release 77 / serial 890616 and lanthorn reads no archive off
/// it, so the boot really does draw nothing and `Journey.blb` (release 83 /
/// 890706) really is why. That is the case SQ-0866 was written for, and it is
/// the ONLY one left in the corpus: measured across `stories/`, sixteen media
/// refuse a sidecar and fifteen of them draw their own artwork anyway. Narrowing
/// the warning to what actually won turned fifteen false alarms off and left the
/// true one on — which is the whole claim, so both halves are pinned.
///
/// Falsifiable: drop the `release_art` check from `unpaired_art_warning` and the
/// silence asserted above turns back into fifteen warnings.
#[test]
fn a_disk_that_really_has_no_artwork_still_says_why() {
    const IIGS_JOURNEY_77: &str = "Journey.2mg";
    let Some(p) = media(IIGS_JOURNEY_77) else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_JOURNEY_77}");
        return;
    };
    if media(JOURNEY_BLORB).is_none() {
        eprintln!("SKIP: gitignored sidecar missing: {JOURNEY_BLORB}");
        return;
    }
    assert!(
        app::graphics::release_art(&p, None).is_none(),
        "{IIGS_JOURNEY_77} supplies no artwork of its own — if it starts to, this case \
         has been overtaken by a better reader and the warning is right to go quiet"
    );
    let said = app::graphics::unpaired_art_warning(&p, None)
        .expect("nothing draws, so the player must be told why");
    for want in ["Journey.blb", "release 83", "890706", "release 77", "890616"] {
        assert!(said.contains(want), "the complaint must name {want:?}: {said}");
    }
}

/// **The rule refuses only on proof, never on absence of evidence.**
///
/// Swept over the whole corpus as an invariant rather than pinned to a list, so
/// it stays true as the disk readers improve: a refusal requires that the release
/// yielded at least one identifiable build AND that the Blorb's stated build is
/// none of them. Nothing may be refused merely because a story could not be read.
///
/// That reader has since landed and this test did keep passing, which is the
/// point of stating it as an invariant: SQ-0867 taught the identity check to ask
/// the RELEASE and not each volume on its own, so `Journey.2mg` and the
/// `shogun_s*.dsk` and `zork_zero_*.dsk` Apple II presses — whose stories are
/// paged across a whole set — went from unidentifiable to identified without a
/// line here changing. Two of them turned out to be contradicted and are now
/// refused; the census at the end moved and the invariant did not.
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
        if !matches!(
            ext.as_str(),
            "2mg" | "adf" | "image" | "dsk" | "po" | "ima" | "img" | "st" | "d64"
        ) {
            continue;
        }
        ran += 1;
        let Some(said) = resource_blorb(&p).refused else { continue };
        refusals += 1;
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
    // The corpus does contain real mismatches, so an invariant that held only
    // because nothing was ever refused would be worthless.
    //
    // A refusal is counted here whether or not the medium HAS artwork of its
    // own, which is a change of bookkeeping SQ-0863 forced and not of rule: most
    // of these media now draw their own segmented plates, so their contradiction
    // is inert — a refusal nobody sees. What a player actually loses is the
    // other guard's question, and `no_medium_in_the_corpus_loses_artwork_…`
    // asks it.
    assert_eq!(
        refusals,
        mismatched().len(),
        "only the measured contradictions are refused"
    );
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
                | "po"
                | "ima"
                | "img"
                | "st"
                | "d64"
        ) {
            continue;
        }
        // The medium's own art is resolved ahead of tier 1 and cannot be touched
        // by the rule, so only the Blorb tier is compared.
        if app::graphics::release_art(&p, None).is_some() {
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
    //
    // **And only the contradicted media with no artwork of their own are in it**
    // (SQ-0863). A medium that supplies its own plates never reaches the Blorb
    // tier at all, so its refusal takes nothing from the player — which is where
    // the Apple *Arthur*, the five *Shogun* floppies and the complete *Journey*
    // pressings went the moment their segmented archives became readable. The
    // list is derived rather than retyped so it cannot drift from the loop's own
    // skip above, and then pinned by name below so it cannot quietly empty.
    let expected: Vec<String> = mismatched()
        .into_iter()
        .filter(|(m, _)| media(m).is_some_and(|p| app::graphics::release_art(&p, None).is_none()))
        .map(|(m, b)| format!("{m}: Some(\"{b}\") -> None"))
        .collect();
    if media(IIGS_JOURNEY).is_some() && media(JOURNEY_BLORB).is_some() {
        assert_eq!(
            expected,
            [format!("{IIGS_JOURNEY}: Some(\"{JOURNEY_BLORB}\") -> None")],
            "the short Journey pressing is the one medium the rule still leaves dark"
        );
    }
    lost.sort();
    assert_eq!(lost, expected, "only the measured pairings may change, and they are the listed ones");
    assert!(ran > 0 || !dir.is_dir(), "the corpus is here but nothing was measured");
}

// ── SQ-0867: a story that is on no single volume ─────────────────────────────

/// Every volume of the *Shogun* press names the same build, whichever one is
/// opened — the property that makes the rule's answer independent of which
/// floppy a person happened to hand lanthorn.
///
/// Release 311 / serial 890510 is the build SQ-0864 verified against the story's
/// own header checksum `$E200` when it taught lanthorn to reassemble this set.
#[test]
fn every_volume_of_the_shogun_press_names_release_311() {
    let mut ran = 0;
    for volume in SHOGUN_SET {
        let Some(p) = media(volume) else { continue };
        ran += 1;
        let builds = app::graphics::release_builds(&p);
        assert_eq!(builds.len(), 1, "{volume} must name exactly one build: {builds:?}");
        assert_eq!(builds[0].release, 311, "{volume}");
        assert_eq!(builds[0].serial_str(), "890510", "{volume}");
        assert_eq!(builds[0].checksum, 0xE200, "{volume}: SQ-0864's verified checksum");
    }
    assert!(ran > 0 || !any_present(&SHOGUN_SET), "no volume ran but the fixtures are here");
}

/// **The cheap answer is the verified answer.** `story_header` reads one 512-byte
/// page and `story` reassembles and checksums 344 KB; on the set where both can
/// run they must agree, or the page being read is not the story's.
///
/// This is what licenses using the page alone on a set that is INCOMPLETE, where
/// no checksum can be taken — see `infocom_packed::story_header`.
#[test]
fn the_header_page_agrees_with_the_checksum_verified_reassembly() {
    let Some(p) = media(SHOGUN_SET[0]) else {
        eprintln!("SKIP: gitignored medium missing: {}", SHOGUN_SET[0]);
        return;
    };
    let files: Vec<(String, Vec<u8>)> =
        app::assets::volumes(&p).iter().flat_map(|v| v.disk.contents()).collect();
    let (_, whole) = blorb::infocom_packed::story(&files).expect("the set reassembles");
    let (_, page) = blorb::infocom_packed::story_header(&files).expect("and states a header");
    assert_eq!(
        blorb::GameIdentifier::of_story(&page),
        blorb::GameIdentifier::of_story(&whole),
        "one page and the whole verified story must name the same build"
    );
    assert_eq!(&page[..64], &whole[..64], "and it must be the story's own header, byte for byte");
}

/// *Shogun*'s Blorb is release 322 / serial 890706 and the press is release 311,
/// so it is refused on exactly the evidence `Arthur.blb` was — which is the
/// pairing SQ-0866 could name but not reach.
#[test]
fn the_shogun_blorb_is_refused_by_the_press_it_contradicts() {
    let Some(p) = media(SHOGUN_SET[0]) else {
        eprintln!("SKIP: gitignored medium missing: {}", SHOGUN_SET[0]);
        return;
    };
    let Some(blb) = media(SHOGUN_BLORB) else {
        eprintln!("SKIP: gitignored sidecar missing: {SHOGUN_BLORB}");
        return;
    };
    // The premise: the Blorb really does claim a build, and a different one.
    let stated = blorb::Blorb::parse(std::fs::read(&blb).unwrap())
        .unwrap()
        .game_identifier()
        .expect("Shogun.blb states a build");
    assert_eq!((stated.release, stated.serial_str().into_owned()), (322, "890706".to_string()));

    let said = resource_blorb(&p).refused.expect("a contradicted Blorb must be refused");
    for want in [SHOGUN_BLORB, "release 322", "890706", "release 311", "890510"] {
        assert!(said.contains(want), "the complaint must name {want:?}: {said}");
    }
    // …and the press draws its OWN plates instead, merged across the five
    // floppies by `MountedDisk::pictures`'s set-spanning arm (SQ-0863). Refusing
    // the Blorb and drawing nothing was the state of things while that arm did
    // not exist; what the rule forbids is release 322's pictures, not artwork.
    assert_eq!(drawn_pictures(&p), 55, "the press must draw its own 55 plates");
    let art = app::graphics::release_art(&p, None).expect("off the set's own segments");
    assert_eq!(art.name, "SHOGUN.D1");
    assert_ne!(
        drawn_pictures(&p),
        drawn_pictures(&blb),
        "and they are not the Blorb's, which is the whole point"
    );
}

/// **An incomplete press still says what it is.** `Journey.2mg` declares five
/// segments and carries four, so 92 of its 552 pages are absent and the story
/// cannot be reassembled at all — but page 0 is on `JOURNEY.D1` and intact, and
/// it says release 77 / serial 890616 against `Journey.blb`'s release 83.
///
/// The brief for this quest expected this release to be unknowable and to keep
/// its Blorb for want of evidence. It is not unknowable: the evidence is on the
/// disk, in the one page a build is named from.
#[test]
fn the_incomplete_journey_press_is_identified_from_the_page_it_still_has() {
    let Some(p) = media(IIGS_JOURNEY) else {
        eprintln!("SKIP: gitignored medium missing: {IIGS_JOURNEY}");
        return;
    };
    let files: Vec<(String, Vec<u8>)> =
        app::assets::volumes(&p).iter().flat_map(|v| v.disk.contents()).collect();
    assert!(
        blorb::infocom_packed::story(&files).is_none(),
        "the premise: this press is missing a segment and cannot be reassembled"
    );

    let builds = app::graphics::release_builds(&p);
    assert_eq!(builds.len(), 1, "and is still identified: {builds:?}");
    assert_eq!(builds[0].release, 77);
    assert_eq!(builds[0].serial_str(), "890616");

    let Some(_) = media(JOURNEY_BLORB) else { return };
    let said = resource_blorb(&p).refused.expect("release 83 contradicts release 77");
    for want in [JOURNEY_BLORB, "release 83", "release 77", "890616"] {
        assert!(said.contains(want), "the complaint must name {want:?}: {said}");
    }
}

/// Identifying a build is not the same as changing one, and the Apple II *Zork
/// Zero* press is the case that keeps the two apart: it becomes identifiable
/// (release 383 / serial 890602) and **nothing about what it draws moves**,
/// because no Blorb in the corpus stem-matches `zork_zero_*.dsk` for the rule to
/// weigh. Evidence gained, behaviour unchanged.
#[test]
fn the_zork_zero_apple_press_becomes_identified_without_changing_what_it_draws() {
    const SET: [&str; 4] =
        ["zork_zero_1.dsk", "zork_zero_2.dsk", "zork_zero_3.dsk", "zork_zero_4.dsk"];
    let mut ran = 0;
    for volume in SET {
        let Some(p) = media(volume) else { continue };
        ran += 1;
        let builds = app::graphics::release_builds(&p);
        assert_eq!(builds.len(), 1, "{volume} must name exactly one build: {builds:?}");
        assert_eq!(builds[0].release, 383, "{volume}");
        assert_eq!(builds[0].serial_str(), "890602", "{volume}");
        let rb = resource_blorb(&p);
        assert!(rb.found.is_none() && rb.refused.is_none(), "{volume}: no Blorb to weigh either way");
    }
    assert!(ran > 0 || !any_present(&SET), "no volume ran but the fixtures are here");
}

/// A loose story file is never checked, however plainly a Blorb beside it
/// disagrees — SQ-0866's line, restated here because SQ-0867 widened what
/// "identifiable" means and that widening must not have reached across it.
///
/// *The Lurking Horror*'s `Lurking.blb` states release 221 / serial 870918
/// against a release 219 / serial 870912 story, on the SOUND path. It is a real
/// `IFhd` mismatch and it changes nothing, because the story is loose: a person
/// assembled that folder.
#[test]
fn the_lurking_horror_sound_sidecar_is_untouched_because_the_story_is_loose() {
    const LURKING: &str = "lurkinghorror-r219-s870912.z3";
    let Some(story) = media(LURKING) else {
        eprintln!("SKIP: gitignored story missing: {LURKING}");
        return;
    };
    // The premise: the sidecar really does state a different build.
    if let Some(blb) = media("Lurking.blb") {
        let stated = blorb::Blorb::parse(std::fs::read(&blb).unwrap())
            .unwrap()
            .game_identifier()
            .expect("Lurking.blb states a build");
        assert_eq!(
            (stated.release, stated.serial_str().into_owned()),
            (221, "870918".to_string()),
            "against a release 219 / serial 870912 story"
        );
    }
    let rb = resource_blorb(&story);
    assert!(rb.refused.is_none(), "a loose story is never refused: {:?}", rb.refused);
    assert!(rb.found.is_some(), "and keeps the sidecar it was placed beside");
    assert!(
        app::graphics::release_builds(&story).is_empty(),
        "a loose story is on no release, so there is nothing to contradict"
    );
}

/// **The rule is the container's, not the picture tier's** (SQ-0867). `IFhd`
/// describes the whole Blorb, and a container built for another build numbers
/// its sounds as build-specifically as its pictures — so the boot resolves the
/// SOUND container through the same door, and a refused Blorb supplies neither.
///
/// The corpus makes that inert and says why: no Blorb the rule refuses holds a
/// single `Snd ` resource. What it removes is a boot that refused a release's
/// artwork out loud and then announced it had loaded 48 images from it.
#[test]
fn no_refused_blorb_in_the_corpus_holds_a_sound_to_lose() {
    let mut ran = 0;
    for (medium, sidecar) in mismatched() {
        let (Some(m), Some(s)) = (media(&medium), media(sidecar)) else { continue };
        ran += 1;
        assert!(resource_blorb(&m).refused.is_some(), "{medium} must be a refusal");
        let b = blorb::Blorb::parse(std::fs::read(&s).unwrap()).unwrap();
        assert_eq!(
            b.resources().iter().filter(|r| &r.usage == b"Snd ").count(),
            0,
            "{sidecar} holds sound, so refusing it is no longer inert — measure the corpus again"
        );
    }
    assert!(ran > 0 || mismatched().is_empty(), "no pairing ran but the fixtures are here");
}
