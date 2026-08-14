//! SQ-0862: **a story's assets span the release, not the platter** — measured on
//! the two DOS presses of Zork Zero and on every set in the corpus that must not
//! be widened.
//!
//! # The report
//!
//! > "with the zork0 dos disk images I'm not seeing all of the artwork options in
//! > the story list. and when `… (360K) (Disk 2) …ima` starts, no artwork is used
//! > so the story doesn't display any."
//!
//! Both halves are one defect. The DOS press splits the story from its artwork
//! across disks, and asset discovery mounted only the image the story came off:
//!
//! | volume | holds |
//! | --- | --- |
//! | 360K Disk 1 | `INSTALL.EXE`, `ZORK0.CG1`, `EZR.EXE`, `IZORK0.RUN` |
//! | 360K Disk 2 | `ZORK0.ZIP`, `ZORKZERO.EXE` — **the story, and no artwork at all** |
//! | 360K Disk 3 | `ZORK0.EG1` |
//! | 720K Disk 1 | the story, `ZORK0.MG1`, and the DOS launchers |
//! | 720K Disk 2 | `ZORK0.CG1` |
//!
//! So booting the 360K story disk found nothing to draw with and offered nothing
//! to pick, and the 720K story disk found its own MCGA and missed the CGA next to
//! it. `app::assets::volumes` is the fix, and the interesting half of it is what
//! it refuses — see `a_multi_game_compilation_shares_no_artwork` below.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! when its fixture is missing and every `ran > 0` guard is gated on
//! [`any_media_present`] — CI has none of this on any platform and must not fail
//! on its absence.

use std::path::{Path, PathBuf};

use app::graphics::{PictSource, PictureOverride};
use app::launch_options::{discover_art_candidates, ArtCandidate};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The five Zork Zero DOS volumes, spelled exactly as the corpus does.
const P360_1: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";
const P360_2: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 2) [!].ima";
const P360_3: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 3) [!].ima";
const P720_1: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 1) [!].ima";
const P720_2: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 2) [!].ima";

const DOS_PRESS: &[&str] = &[P360_1, P360_2, P360_3, P720_1, P720_2];

fn media(name: &str) -> Option<PathBuf> {
    let p = stories_dir().join(name);
    p.is_file().then_some(p)
}

fn any_media_present() -> bool {
    DOS_PRESS.iter().any(|n| media(n).is_some())
}

/// The archives the launch dialog offers off the MEDIUM, by filename. The loose
/// arm is deliberately excluded: the corpus directory carries a `zorkzero.mg1`
/// beside these images and it is not what this quest is about.
fn on_medium(story: &Path) -> Vec<String> {
    let mut v: Vec<String> = discover_art_candidates(story)
        .into_iter()
        .filter(|c| c.on_medium)
        .map(|c| c.filename)
        .collect();
    v.sort();
    v
}

fn candidate(story: &Path, filename: &str) -> Option<ArtCandidate> {
    discover_art_candidates(story).into_iter().find(|c| c.filename.eq_ignore_ascii_case(filename))
}

/// A per-game sidecar directory carrying a `pictures` key.
fn game_dir_with(tag: &str, body: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("babelmap-sq0862-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), body).unwrap();
    dir
}

// ── The reported case ────────────────────────────────────────────────────────

/// **The user's defect, end to end.** Booting the 360K story disk — the one whose
/// volume holds no artwork whatever — must now offer both renditions the press
/// shipped and must actually draw one.
#[test]
fn the_360k_story_disk_offers_and_draws_the_releases_artwork() {
    let Some(disk2) = media(P360_2) else { return };
    // The premise, stated so the test cannot pass by measuring the wrong thing:
    // this volume really does carry no archive of its own.
    let raw = std::fs::read(&disk2).unwrap();
    let mounted = blorb::medium::MountedDisk::mount(raw).expect("a FAT12 floppy");
    assert!(mounted.pictures().is_none(), "the story disk carries no artwork of its own");
    assert_eq!(mounted.stories().len(), 1, "and exactly one story");

    // Both siblings' renditions are offered.
    assert_eq!(
        on_medium(&disk2),
        vec!["ZORK0.CG1".to_string(), "ZORK0.EG1".to_string()],
        "the 360K press ships CGA on disk 1 and EGA on disk 3; the story disk must offer both",
    );

    // …and each one names the image that actually carries it, not the story's.
    let cg1 = candidate(&disk2, "ZORK0.CG1").expect("CGA is offered");
    let eg1 = candidate(&disk2, "ZORK0.EG1").expect("EGA is offered");
    assert_eq!(cg1.path, stories_dir().join(P360_1), "CGA lives on disk 1");
    assert_eq!(eg1.path, stories_dir().join(P360_3), "EGA lives on disk 3");
    assert_eq!((cg1.rendition, eg1.rendition), ("CGA", "EGA"));
    assert_eq!((cg1.pictures, eg1.pictures), (503, 503));

    // And the automatic resolution draws. This is the half of the report that a
    // list of candidates cannot settle: "no artwork is used so the story doesn't
    // display any."
    let mut src = PictSource::resolve(&disk2);
    assert_eq!(
        src.dims(1),
        Some((640, 200)),
        "the story disk must resolve to the release's artwork, not to nothing",
    );
    assert!(src.image(1).is_some(), "and it must decode a real picture");
    assert!(!src.is_monochrome(), "with EGA preferred over CGA's two colours");
}

/// The other half of the press: MCGA shares the story's disk and was always
/// found; CGA sits alone on disk 2 and was not.
#[test]
fn the_720k_story_disk_offers_both_renditions_and_keeps_its_own() {
    let Some(disk1) = media(P720_1) else { return };
    assert_eq!(
        on_medium(&disk1),
        vec!["ZORK0.CG1".to_string(), "ZORK0.MG1".to_string()],
        "MCGA is on the story's own disk, CGA on disk 2; both must be offered",
    );
    // The story's own volume still wins the automatic pick, so nothing that
    // worked before this quest moved: 320-wide MCGA, not disk 2's 640-wide CGA.
    let mut src = PictSource::resolve(&disk1);
    assert_eq!(src.dims(1), Some((320, 200)), "the story's own volume keeps precedence");
    assert!(!src.is_monochrome());
}

// ── Guard: only the same press contributes ───────────────────────────────────

/// **The sharpest control in the corpus.** `(360K)` and `(720K)` are two presses
/// of one build, and `disk_set` refuses to merge them because `{360, 720}` is a
/// capacity and not an index. Asset discovery must inherit that refusal exactly:
/// a 360K volume may never see `ZORK0.MG1`, which exists only on the 720K press,
/// and a 720K volume may never see `ZORK0.EG1`, which exists only on the 360K one.
#[test]
fn the_two_presses_never_lend_each_other_artwork() {
    let mut ran = 0;
    for name in DOS_PRESS {
        let Some(path) = media(name) else { continue };
        ran += 1;
        let is_360 = name.contains("(360K)");
        let listed = on_medium(&path);

        // Every archive offered off the medium comes off a volume of THIS press.
        let members = app::disk_set::members(&path).expect("each press is a set");
        for c in discover_art_candidates(&path).iter().filter(|c| c.on_medium) {
            assert!(
                members.contains(&c.path),
                "{name}: {} came off {}, which is not a volume of this press",
                c.filename,
                c.path.display(),
            );
        }

        if is_360 {
            assert!(
                !listed.iter().any(|f| f.eq_ignore_ascii_case("ZORK0.MG1")),
                "{name}: MCGA exists only on the 720K press: {listed:?}",
            );
            assert_eq!(listed, vec!["ZORK0.CG1".to_string(), "ZORK0.EG1".to_string()], "{name}");
        } else {
            assert!(
                !listed.iter().any(|f| f.eq_ignore_ascii_case("ZORK0.EG1")),
                "{name}: EGA exists only on the 360K press: {listed:?}",
            );
            assert_eq!(listed, vec!["ZORK0.CG1".to_string(), "ZORK0.MG1".to_string()], "{name}");
        }
    }
    assert!(ran > 0 || !any_media_present(), "the DOS press is present but nothing ran");
}

// ── Guard: a shelf is not a release ──────────────────────────────────────────

/// The case that decides the whole shape of the rule. DOS `floppy1.ima`…`floppy5`
/// is *The Lost Treasures of Infocom*: twenty stories across five disks, with
/// Zork Zero's `ZORK0.CG1` on floppy 4 and its `ZORK0.EG1` on floppy 5. It is a
/// recognised set, exactly like the Zork Zero presses — so if the widening were
/// unconditional it would offer Zork Zero's plates to Zork I, which is precisely
/// the invisible mis-pairing SQ-0734's tiers exist to prevent.
///
/// Each volume must therefore see its own artwork and no other volume's.
#[test]
fn a_multi_game_compilation_shares_no_artwork() {
    const EXPECT: &[(&str, &[&str])] = &[
        ("floppy1.ima", &[]),
        ("floppy2.ima", &[]),
        ("floppy3.ima", &[]),
        ("floppy4.ima", &["ZORK0.CG1"]),
        ("floppy5.ima", &["ZORK0.EG1"]),
    ];
    let mut ran = 0;
    for (name, want) in EXPECT {
        let Some(path) = media(name) else { continue };
        ran += 1;
        // The premise: this really is one recognised five-volume set.
        assert_eq!(
            app::disk_set::members(&path).map(|m| m.len()),
            Some(5),
            "{name}: the Lost Treasures DOS press is a set",
        );
        assert_eq!(
            on_medium(&path),
            want.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "{name}: a twenty-game shelf must not lend one game's artwork to another",
        );
        // And the volumes it may read are only its own.
        let vols = app::assets::volumes(&path);
        assert_eq!(vols.len(), 1, "{name}: a compilation volume stands alone");
        assert_eq!(vols[0].path, path);
    }
    assert!(
        ran > 0 || !stories_dir().join("floppy1.ima").exists(),
        "the DOS compilation is present but nothing ran",
    );
}

// ── Guard: single images and loose files are untouched ───────────────────────

/// Most media are one disk, and every one of them must resolve exactly as it did
/// before this quest. Named on real fixtures rather than asserted in the abstract.
#[test]
fn a_single_volume_release_is_unchanged() {
    // Names are compared lowercased: the volumes spell them as they please
    // (`Pic.data` on Zork Zero's floppy, `pic.data` on Arthur's) and this test is
    // about which files are reachable, not about their capitals. The third column
    // is the archive's PICTURE SPACE, which is a fact about the rendition; picture
    // 1's own size is not (292×196 on Arthur, 320×200 on Zork Zero).
    const SINGLE: &[(&str, &[&str], (u16, u16))] = &[
        // The Amiga floppy: one `Pic.data`, 320-wide.
        ("Zork Zero - The Revenge of Megaboz.adf", &["pic.data"], (320, 200)),
        // The Macintosh disk: two archives on ONE volume, colour picked (SQ-0838).
        ("Zork Zero Disk.image", &["cpic.data", "pic.data"], (320, 200)),
        ("Arthur - The Quest for Excalibur.adf", &["pic.data"], (320, 200)),
        ("Journey - The Quest Begins.adf", &["pic.data"], (320, 200)),
    ];
    let mut ran = 0;
    for (name, want, space) in SINGLE {
        let Some(path) = media(name) else { continue };
        ran += 1;
        assert!(app::disk_set::members(&path).is_none(), "{name}: not a multi-disk set");
        let vols = app::assets::volumes(&path);
        assert_eq!(vols.len(), 1, "{name}: one image, one volume");
        assert_eq!(
            on_medium(&path).iter().map(|f| f.to_lowercase()).collect::<Vec<_>>(),
            want.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "{name}",
        );
        let mut src = PictSource::resolve(&path);
        assert_eq!(
            src.native_std_window(),
            Some(*space),
            "{name}: the medium's own art, as before",
        );
        assert!(src.image(1).is_some(), "{name}: and it still decodes");
    }
    assert!(
        ran > 0 || !SINGLE.iter().any(|(n, _, _)| media(n).is_some()),
        "single-volume media are present but nothing ran",
    );
}

/// A story that is not release media at all mounts nothing, so none of this can
/// cost it anything or change what it resolves to. Runs everywhere, fixtures or
/// not.
#[test]
fn an_ordinary_story_file_spans_no_volumes() {
    let dir = std::env::temp_dir()
        .join(format!("babelmap-sq0862-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let story = dir.join("story.z6");
    std::fs::write(&story, b"not a disk image").unwrap();
    assert!(app::assets::volumes(&story).is_empty());
    assert!(app::assets::files(&story).iter().all(|f| !f.is_on_medium()));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Guard: a named archive still wins, and now reaches a sibling ─────────────

/// Tier 3 is unmoved: naming an archive outranks whatever the release would have
/// picked. What is new is only *where the name may point* — `ZORK0.CG1` is on
/// disk 1 and the story is on disk 2, and a name the dialog offered has to be a
/// name the override can find.
#[test]
fn a_named_archive_on_a_sibling_volume_wins() {
    let Some(disk2) = media(P360_2) else { return };

    // Automatic: the release's EGA, 640-wide and in colour.
    let plain = game_dir_with("auto", "");
    let mut auto = PictSource::resolve_with_override(&disk2, PictureOverride::resolve(&disk2, &plain));
    assert_eq!(auto.dims(1), Some((640, 200)));
    assert!(!auto.is_monochrome(), "EGA by default");

    // Named: the CGA plates on disk 1 instead — the same 640-wide space, in two
    // colours, which is how the two are told apart.
    let dir = game_dir_with("named", "pictures = \"ZORK0.CG1\"\n");
    let over = PictureOverride::resolve(&disk2, &dir);
    assert!(matches!(over, PictureOverride::Loaded { .. }), "got {over:?}");
    assert!(over.warning().is_none(), "a name that resolved is not loud");
    let mut named = PictSource::resolve_with_override(&disk2, over);
    assert_eq!(named.dims(1), Some((640, 200)));
    assert!(named.is_monochrome(), "the CGA archive the user named, not the EGA default");
    assert!(named.image(1).is_some(), "and it decodes off the sibling volume");

    let _ = std::fs::remove_dir_all(&plain);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A name that is on no volume of the release is still Missing, and still loud.
/// Widening where we look must not turn a bad name into a silent one.
///
/// The name is deliberately one that exists nowhere at all. `ZORK0.MG1` would
/// have been the sharper choice — MCGA is on the *other* press — but the corpus
/// directory carries a loose `zork0.mg1` beside these images, and on a
/// case-insensitive filesystem the host arm finds it before the medium is ever
/// consulted. The two-presses guard covers that case where it belongs, over the
/// candidate list, which reads the volumes and not the directory.
#[test]
fn a_name_on_no_volume_of_the_release_is_still_loud() {
    let Some(disk2) = media(P360_2) else { return };
    let dir = game_dir_with("absent", "pictures = \"NOSUCH.EG1\"\n");
    let over = PictureOverride::resolve(&disk2, &dir);
    assert!(matches!(over, PictureOverride::Missing { .. }), "got {over:?}");
    assert!(over.warning().is_some(), "and the user hears about it");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Guard: every rendition lands on the same geometry ────────────────────────

/// Widening discovery must not disturb what SQ-0790 pinned: the renditions of one
/// game are drawings of one screen, so each lands on the 640×400 unit screen at
/// the density its picture space implies. Measured across both presses.
#[test]
fn every_rendition_of_the_release_lands_on_one_screen() {
    /// One rendition: the archive, the picture space it draws into, and the
    /// per-axis scale onto the 640×400 unit screen — the table in
    /// `PictSource::art_scale`.
    struct Geometry {
        archive: &'static str,
        space: (u16, u16),
        scale: (u32, u32),
    }
    const GEOMETRY: &[Geometry] = &[
        Geometry { archive: "ZORK0.MG1", space: (320, 200), scale: (2, 2) },
        Geometry { archive: "ZORK0.EG1", space: (640, 200), scale: (1, 2) },
        Geometry { archive: "ZORK0.CG1", space: (640, 200), scale: (1, 2) },
    ];
    let mut ran = 0;
    for name in DOS_PRESS {
        let Some(path) = media(name) else { continue };
        let dir = game_dir_with("geometry", "");
        for c in discover_art_candidates(&path).iter().filter(|c| c.on_medium) {
            let Some(want) =
                GEOMETRY.iter().find(|g| g.archive.eq_ignore_ascii_case(&c.filename))
            else {
                panic!("{name}: unexpected archive {}", c.filename);
            };
            ran += 1;
            std::fs::write(dir.join("config.toml"), format!("pictures = {:?}\n", c.filename))
                .unwrap();
            let over = PictureOverride::resolve(&path, &dir);
            assert!(matches!(over, PictureOverride::Loaded { .. }), "{}: {over:?}", c.filename);
            assert_eq!(over.std_window(), Some(want.space), "{name}/{}", c.filename);
            let src = PictSource::resolve_with_override(&path, over);
            assert_eq!(src.native_std_window(), Some(want.space), "{name}/{}", c.filename);
            assert_eq!(src.art_scale(), Some(want.scale), "{name}/{}", c.filename);
            // Space times scale is the same unit screen for all three.
            assert_eq!(
                (u32::from(want.space.0) * want.scale.0, u32::from(want.space.1) * want.scale.1),
                (640, 400),
                "{name}/{}: every rendition covers the same rectangle",
                c.filename,
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(ran > 0 || !any_media_present(), "the DOS press is present but nothing ran");
}

// ── SQ-0865: which disk, and what "Automatic" will actually open ─────────────
//
// The report, against the launch-options panel:
//
// > "in the story list options panel the defaul artwork selection is 'Use this
// > story's own art (Blorb / disk image)'. We should be more specific on the
// > default format and match the text/formatting of the other options. In terms
// > of the other options, 'on disk' is confusing. What do you think about 'from
// > game disk'"
//
// Two defects, and SQ-0862 is what sharpened the second. Before it, an archive
// "on disk" was on the disk you booted; now a candidate can live on a **sibling
// volume of the release**, so booting `(360K) (Disk 2)` offers `ZORK0.EG1`,
// which is physically on disk 3. "on disk" stopped being merely vague and became
// wrong by implication. This press is the fixture that can prove it, being the
// only release in the corpus whose artwork sits on a disk the story does not.

/// The disk number on every row is the RELEASE's — not a position in a list, and
/// not something parsed out of a filename in the dialog.
///
/// Pinned against the press's actual layout: on the 360K press CGA is disk 1,
/// the story is alone on disk 2 and EGA is disk 3; on the 720K press the story
/// and MCGA share disk 1 and CGA is disk 2. Every one of those numbers is one a
/// person can read off the label of the floppy in their hand.
#[test]
fn every_candidate_names_the_disk_of_the_release_it_comes_off() {
    // (image, [(archive, the disk it is really on)])
    let want: &[(&str, &[(&str, u64)])] = &[
        (P360_1, &[("ZORK0.CG1", 1), ("ZORK0.EG1", 3)]),
        (P360_2, &[("ZORK0.CG1", 1), ("ZORK0.EG1", 3)]),
        (P360_3, &[("ZORK0.CG1", 1), ("ZORK0.EG1", 3)]),
        (P720_1, &[("ZORK0.CG1", 2), ("ZORK0.MG1", 1)]),
        (P720_2, &[("ZORK0.CG1", 2), ("ZORK0.MG1", 1)]),
    ];
    let mut ran = 0;
    for (image, rows) in want {
        let Some(path) = media(image) else { continue };
        for (archive, disk) in *rows {
            let c = candidate(&path, archive)
                .unwrap_or_else(|| panic!("{image}: {archive} must be offered"));
            ran += 1;
            assert!(c.on_medium, "{image}/{archive}");
            assert_eq!(
                c.disk_number,
                Some(*disk),
                "{image}: {archive} is on disk {disk} of this press",
            );
            // …and the phrase both surfaces print, which is what a person reads.
            // Never the bare "on disk" it replaced.
            assert_eq!(
                app::launch_options::medium_note(&c),
                format!("from disk {disk}"),
                "{image}/{archive}",
            );
        }
    }
    assert!(ran > 0 || !any_media_present(), "the DOS press is present but nothing ran");
}

/// **The guard worth the most: the default row must not claim one archive while
/// the boot opens another.**
///
/// Asserted as a property rather than as a string, and deliberately not against
/// `release_art` — comparing that function with itself would pass however wrong
/// the row was. The oracle is the ART ITSELF: take the name the row shows, feed
/// it back through tier 3 (`PictureOverride` → `PictSource`) as if the user had
/// typed it, and require the picture source that comes out to be
/// indistinguishable from the one `PictSource::resolve` builds when nobody names
/// anything. A row naming CGA where EGA boots differs in `is_monochrome`; a row
/// naming MCGA where EGA boots differs in `dims`.
#[test]
fn the_default_row_names_the_archive_the_boot_actually_opens() {
    // What the release supplies when nothing is overridden: the story's own
    // volume wins outright, then colour beats monochrome — SQ-0862's policy,
    // which this quest may only ever LABEL.
    let want: &[(&str, &str, u64)] = &[
        (P360_1, "ZORK0.CG1", 1), // disk 1 carries CGA itself
        (P360_2, "ZORK0.EG1", 3), // the story disk has none; EGA over CGA
        (P360_3, "ZORK0.EG1", 3),
        (P720_1, "ZORK0.MG1", 1), // the story's own volume, unmoved
        (P720_2, "ZORK0.CG1", 2),
    ];
    let mut ran = 0;
    for (image, archive, disk) in want {
        let Some(path) = media(image) else { continue };
        ran += 1;
        let st = app::launch_options::LaunchOptionsState::new(
            "Zork Zero",
            &path,
            None,
            None,
            Some(6),
            None,
        );
        let d = st.default_art.as_ref().unwrap_or_else(|| {
            panic!("{image}: the release supplies artwork, so the default row must name it")
        });
        assert_eq!(d.filename, *archive, "{image}: the default row names the wrong archive");
        assert_eq!(d.disk_number, Some(*disk), "{image}: …and the wrong disk");
        assert_eq!(d.medium_note(), format!("from disk {disk}"), "{image}");
        assert_eq!(d.pictures, 503, "{image}: Zork Zero's directory, whole");

        // The agreement itself. Accepting the default and naming what the row
        // says must reach the same artwork, in everything observable about it.
        let empty = game_dir_with("agree", "");
        let mut auto = PictSource::resolve(&path);
        let over = PictureOverride::resolve_with_session(&path, &empty, Some(&d.filename));
        assert!(matches!(over, PictureOverride::Loaded { .. }), "{image}: {over:?}");
        let mut named = PictSource::resolve_with_override(&path, over);
        assert_eq!(
            named.is_monochrome(),
            auto.is_monochrome(),
            "{image}: the row names {archive}, which is not what boots",
        );
        assert_eq!(named.native_std_window(), auto.native_std_window(), "{image}");
        assert_eq!(named.art_scale(), auto.art_scale(), "{image}");
        assert_eq!(named.dims(1), auto.dims(1), "{image}");
        assert_eq!(named.image(1).is_some(), auto.image(1).is_some(), "{image}");
        let _ = std::fs::remove_dir_all(&empty);
    }
    assert!(ran > 0 || !any_media_present(), "the DOS press is present but nothing ran");
}

/// **The panel the user is judging this by.** All five volumes are rendered and
/// printed, so the rows can be read here rather than inferred (`cargo nextest
/// run -p app the_launch_dialog_over_the_whole_dos_press -- --nocapture`).
///
/// The assertions are about SHAPE, not a snapshot: the default row is columnar
/// and names its archive, no row says the bare "on disk", and the picture counts
/// of the default row and the candidate rows land in one column — which is the
/// complaint that started this quest.
#[test]
fn the_launch_dialog_over_the_whole_dos_press_names_its_disks() {
    let mut ran = 0;
    for image in DOS_PRESS {
        let Some(path) = media(image) else { continue };
        ran += 1;
        let frame = render_launch_dialog(&path);
        println!("── {image}\n{frame}");

        assert!(frame.contains("Automatic — "), "the default row names what it picks: {frame}");
        assert!(
            !frame.contains("Use this story's own art"),
            "the prose default row is gone: {frame}",
        );
        for line in frame.lines().filter(|l| l.contains(" pictures")) {
            // The bare marker, in the exact spelling the report called
            // confusing. It must survive nowhere.
            assert!(
                !line.contains("  on disk") && !line.contains("· on disk"),
                "a row still says the bare 'on disk': {line:?}",
            );
            // Every archive that came off this press is on a numbered disk and
            // says so. The `zorkzero.mg1` sitting loose in the corpus directory
            // beside these images is the control: it is a file in a folder, it
            // needs no explanation, and it gets none.
            if line.contains("ZORK0.") {
                assert!(line.contains("from disk "), "a volume's row must name its disk: {line:?}");
            } else {
                assert!(
                    !line.contains("from disk") && !line.contains("from game disk"),
                    "a loose file beside the story explains nothing: {line:?}",
                );
            }
        }
        // The columns really do line up: every option row carries its picture
        // count in the same SCREEN column, the default row included — which is
        // the half of the report about matching the other options' formatting.
        //
        // Counted in characters and not in bytes: the default row's `·` and `—`
        // are three bytes wider than the `( )` of the rows under it, so a byte
        // offset says they are misaligned when the screen says they are not.
        let cols: Vec<usize> = frame
            .lines()
            .filter(|l| l.contains(") ") && l.contains(" pictures"))
            .map(|l| l[..l.find(" pictures").unwrap()].chars().count())
            .collect();
        assert!(cols.len() >= 3, "the default row plus its candidates: {frame}");
        assert!(
            cols.iter().all(|c| *c == cols[0]),
            "the picture counts are in one column, default row included: {cols:?}\n{frame}",
        );
    }
    assert!(ran > 0 || !any_media_present(), "the DOS press is present but nothing ran");
}

/// Render the launch-options dialog for `story` into a plain string, one line per
/// buffer row, at a size the whole dialog fits in.
fn render_launch_dialog(story: &Path) -> String {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let disk = std::fs::read(story).ok().and_then(|b| blorb::medium::DiskImage::detect(&b));
    let st =
        app::launch_options::LaunchOptionsState::new("Zork Zero", story, None, None, Some(6), disk);
    let cs = app::colors::ColorScheme::terminal_default();
    let mut term = Terminal::new(TestBackend::new(100, 26)).unwrap();
    term.draw(|f| {
        app::render::launch_options_dialog::draw_launch_options(&st, f.area(), &cs, f.buffer_mut());
    })
    .unwrap();
    let buf = term.backend().buffer();
    let mut frame = String::new();
    for y in 0..buf.area().height {
        for x in 0..buf.area().width {
            frame.push_str(buf.cell((x, y)).unwrap().symbol());
        }
        frame.push('\n');
    }
    frame
}
