//! SQ-0734 — which picture archive a story draws from, and who gets to say so.
//!
//! Three tiers, ordered by confidence in the PAIRING between a story and its art:
//!
//! 1. **Blorb.** The container validates its own contents. Zero config.
//! 2. **Disk image.** Story and archive came off one floppy, so the medium
//!    guarantees they belong together (SQ-0719). Zero config.
//! 3. **The user names it** — `pictures = "…"` in the per-game sidecar
//!    `<game_dir>/config.toml`. Naming an archive ASSERTS the pairing, and it
//!    wins over both tiers above.
//!
//! There is no fourth tier that guesses from the filename, and that is a decision
//! rather than an omission. A native archive carries no release number and no
//! serial; every Infocom Amiga release names its file `Pic.data`; the PC names
//! are a DOS 8.3 convention that a library naming its stories by release and
//! serial (`beyondzork-r57-s871221.z5` beside `beyondzo.mg1`) no longer matches.
//! A stem rule would be wrong sometimes, and wrong here is INVISIBLE — Arthur's
//! plates drawn into Zork Zero look like art, not like an error. Spatterlight's
//! bocfel does auto-discover, and it validates nothing: it claims any `PIC.DATA`
//! in the story's directory for whatever v6 game is open, and patches the
//! resulting misses with a hardcoded table of four title names.
//!
//! Two things the key buys, both covered below:
//!
//! - **Rescue.** `fmvpoker.z6` is a story whose art is Zork Zero's picture file
//!   under another name — its readme tells the player to rename one — so no rule
//!   could ever pair them. Tier 3 is the only route that reaches it.
//! - **Choice.** Zork Zero in `stories/` has four usable archives at once
//!   (`zork0.pic` Amiga, `zork0.mg1` MCGA, `zork0.eg1` EGA, `zork0.cg1` CGA)
//!   plus `Zork0.blb`. The key is how a player picks a rendition, not only how
//!   they rescue a game that has none.
//!
//! And one thing it must never do: fail quietly. A named file that is absent or
//! will not decode leaves the Blorb in charge and produces a warning naming the
//! file and the reason, because the alternative is a player who believes they are
//! seeing the native art they asked for and is not.
//!
//! Real archives are gitignored, so every case using one skips vacuously.

use std::path::{Path, PathBuf};

use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::GameSession;
use blorb::infocom_pics::Flavour;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A throwaway `game_dir` holding the sidecar `config.toml`, seeded with the
/// bare lines given. `None` writes no sidecar at all.
fn game_dir_with(tag: &str, body: Option<&str>) -> PathBuf {
    let d = std::env::temp_dir().join(format!("babelmap-picover-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    if let Some(body) = body {
        std::fs::write(d.join("config.toml"), body).unwrap();
    }
    d
}

/// A story fixture, or `None` (→ skip) when the gitignored file is absent.
fn story(name: &str) -> Option<PathBuf> {
    let p = stories_dir().join(name);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("SKIP: gitignored fixture missing at {}", p.display());
        None
    }
}

// ── tier 3 resolution ────────────────────────────────────────────────────────

#[test]
fn no_key_means_no_override() {
    let dir = game_dir_with("unset", Some("honor_game_colours = true\n"));
    let over = PictureOverride::resolve(Path::new("/nowhere/story.z6"), &dir);
    assert!(matches!(over, PictureOverride::Unset), "got {over:?}");
    assert_eq!(over.flavour(), None, "nothing named → nothing inferred");
    assert_eq!(over.warning(), None, "nothing named → nothing to complain about");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_absent_sidecar_means_no_override() {
    let dir = game_dir_with("nosidecar", None);
    let over = PictureOverride::resolve(Path::new("/nowhere/story.z6"), &dir);
    assert!(matches!(over, PictureOverride::Unset), "got {over:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_named_archive_that_is_not_there_is_reported_by_name() {
    let dir = game_dir_with("missing", Some("pictures = \"NOPE.EG1\"\n"));
    let over = PictureOverride::resolve(Path::new("/nowhere/story.z6"), &dir);
    let PictureOverride::Missing { ref path } = over else {
        panic!("expected Missing, got {over:?}")
    };
    // Relative names resolve beside the STORY — that is where these archives sit.
    assert_eq!(path, Path::new("/nowhere/NOPE.EG1"), "resolved beside the story");
    let w = over.warning().expect("a missing file is never silent");
    assert!(w.contains("NOPE.EG1"), "the warning must name the file: {w}");
    // The user asked for native art and is not getting it. Say which they get.
    assert!(w.contains("Blorb"), "the warning must say what is used instead: {w}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_named_file_that_is_not_an_archive_is_reported_with_its_reason() {
    // The case the policy exists for: the file EXISTS, so "if the file exists"
    // is satisfied, and it still cannot be drawn. Silently dropping to the Blorb
    // here is exactly the inverted failure the tiers are meant to prevent.
    let dir = game_dir_with("unusable", Some("pictures = \"junk.eg1\"\n"));
    std::fs::write(dir.join("junk.eg1"), b"this is not a picture archive at all").unwrap();
    let story = dir.join("story.z6");
    let over = PictureOverride::resolve(&story, &dir);
    let PictureOverride::Unusable { ref path, ref reason } = over else {
        panic!("expected Unusable, got {over:?}")
    };
    assert!(path.ends_with("junk.eg1"), "{path:?}");
    assert!(!reason.is_empty(), "an unusable file must carry a reason");
    let w = over.warning().expect("an undecodable file is never silent");
    assert!(w.contains("junk.eg1"), "the warning must name the file: {w}");
    assert!(w.contains(reason), "the warning must carry the reason: {w}");
    // A file that will not parse names no machine.
    assert_eq!(over.flavour(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_absolute_name_is_used_as_given() {
    let Some(eg1) = story("zork0.eg1") else { return };
    let dir = game_dir_with("abs", Some(&format!("pictures = {:?}\n", eg1.display().to_string())));
    // The story path is nonsense on purpose: an absolute name must not consult it.
    let over = PictureOverride::resolve(Path::new("/nowhere/story.z6"), &dir);
    let PictureOverride::Loaded { ref path, .. } = over else {
        panic!("expected Loaded, got {over:?}")
    };
    assert_eq!(path, &eg1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_relative_name_resolves_beside_the_story() {
    let Some(z0) = story("zork0-r393-s890714.z6") else { return };
    if story("zork0.eg1").is_none() {
        return;
    }
    let dir = game_dir_with("rel", Some("pictures = \"zork0.eg1\"\n"));
    let over = PictureOverride::resolve(&z0, &dir);
    let PictureOverride::Loaded { ref path, ref pics } = over else {
        panic!("expected Loaded, got {over:?}")
    };
    assert_eq!(path, &stories_dir().join("zork0.eg1"));
    assert_eq!(pics.flavour(), Flavour::Pc);
    assert_eq!(over.warning(), None, "a key that worked says nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── precedence: tier 3 outranks tiers 1 and 2 ────────────────────────────────

/// The user's rule, on the case the fmvpoker story alone cannot test: Zork Zero
/// has a perfectly good `Zork0.blb` beside it, and naming `zork0.eg1` must still
/// win. This is rendition SELECTION — the same game, four archives on disk, the
/// player choosing which one — not a rescue.
///
/// The two sources are told apart by picture 1's size, which is not a detail:
/// EGA stores 640-wide art with half-width pixels where the Blorb (an Amiga
/// conversion) stores 320. So `640x200` can only have come from the `.EG1`.
#[test]
fn a_named_archive_beats_a_perfectly_good_blorb() {
    let Some(z0) = story("zork0-r393-s890714.z6") else { return };
    if story("zork0.eg1").is_none() || story("Zork0.blb").is_none() {
        return;
    }

    // Baseline: with no key at all, tier 1 answers and picture 1 is the Blorb's.
    let plain = game_dir_with("beats-blorb-off", None);
    let mut blorb_src =
        PictSource::resolve_with_override(&z0, PictureOverride::resolve(&z0, &plain));
    assert_eq!(
        blorb_src.dims(1),
        Some((320, 200)),
        "without the key, Zork Zero draws its Blorb art",
    );

    // With the key, the named EGA archive wins outright.
    let dir = game_dir_with("beats-blorb-on", Some("pictures = \"zork0.eg1\"\n"));
    let over = PictureOverride::resolve(&z0, &dir);
    assert!(matches!(over, PictureOverride::Loaded { .. }), "got {over:?}");
    let mut ega = PictSource::resolve_with_override(&z0, over);
    assert_eq!(
        ega.dims(1),
        Some((640, 200)),
        "the named EGA archive must outrank the Blorb sitting beside the story",
    );
    assert!(ega.image(1).is_some(), "and it must actually decode");

    let _ = std::fs::remove_dir_all(&plain);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Tier 3 also outranks tier 2. Booting Zork Zero off its Amiga floppy normally
/// draws the `Pic.data` on that floppy (SQ-0719); naming an archive overrides
/// even that, because the medium is a guarantee about provenance and the key is
/// an instruction.
#[test]
fn a_named_archive_beats_the_disk_image_it_was_mounted_from() {
    let Some(adf) = story("Zork Zero - The Revenge of Megaboz.adf") else { return };
    if story("zork0.eg1").is_none() {
        return;
    }

    // Baseline: the floppy's own art, 320-wide like every Amiga archive.
    let plain = game_dir_with("beats-adf-off", None);
    let mut native =
        PictSource::resolve_with_override(&adf, PictureOverride::resolve(&adf, &plain));
    assert_eq!(native.dims(1), Some((320, 200)), "the floppy supplies its own Pic.data");

    // The key wins over the medium.
    let dir = game_dir_with("beats-adf-on", Some("pictures = \"zork0.eg1\"\n"));
    let mut ega =
        PictSource::resolve_with_override(&adf, PictureOverride::resolve(&adf, &dir));
    assert_eq!(ega.dims(1), Some((640, 200)), "the named archive outranks the medium");

    let _ = std::fs::remove_dir_all(&plain);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A named file that will not decode does NOT take the art with it: the Blorb
/// still draws. The loudness is the warning, not a blank screen.
#[test]
fn an_unusable_name_still_leaves_the_blorb_drawing() {
    let Some(z0) = story("zork0-r393-s890714.z6") else { return };
    if story("Zork0.blb").is_none() {
        return;
    }
    let dir = game_dir_with("unusable-falls-back", Some("pictures = \"junk.eg1\"\n"));
    std::fs::write(dir.join("junk.eg1"), b"not an archive").unwrap();
    // Absolute, so it is found where the test wrote it rather than beside the story.
    std::fs::write(
        dir.join("config.toml"),
        format!("pictures = {:?}\n", dir.join("junk.eg1").display().to_string()),
    )
    .unwrap();

    let over = PictureOverride::resolve(&z0, &dir);
    assert!(matches!(over, PictureOverride::Unusable { .. }), "got {over:?}");
    assert!(over.warning().is_some(), "and it is loud");
    let mut src = PictSource::resolve_with_override(&z0, over);
    assert_eq!(src.dims(1), Some((320, 200)), "the Blorb keeps drawing");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the archive names the machine ────────────────────────────────────────────

/// SQ-0734 precedence 2, on real archives: the flavour is read from the file's
/// CONTENT, so it is right even when the extension lies. `FMVPOKER.EG1` is a
/// Zork Zero EGA archive under a name that matches no Infocom convention at all,
/// and it still resolves as the PC container it is.
#[test]
fn the_named_archives_flavour_comes_from_its_content() {
    let cases: [(&str, Flavour, InterpreterProfile); 3] = [
        ("zork0.eg1", Flavour::Pc, InterpreterProfile::IbmPc),
        ("FMVPOKER.EG1", Flavour::Pc, InterpreterProfile::IbmPc),
        ("zork0.pic", Flavour::AmigaMac, InterpreterProfile::Amiga),
    ];
    for (name, want_flavour, want_profile) in cases {
        if story(name).is_none() {
            continue;
        }
        let dir = game_dir_with(&format!("flavour-{name}"), Some(&format!("pictures = {name:?}\n")));
        let story_path = stories_dir().join("anything.z6");
        let over = PictureOverride::resolve(&story_path, &dir);
        assert_eq!(over.flavour(), Some(want_flavour), "{name}");
        // …and the flavour is what selects the machine, with no interpreter
        // number configured and no disk image involved.
        assert_eq!(
            InterpreterProfile::resolve(&story_path, None, over.flavour()),
            want_profile,
            "{name} selects the machine",
        );
        // An explicit number still outranks it (precedence 1 over 2).
        assert_eq!(
            InterpreterProfile::resolve(&story_path, Some(6), over.flavour()),
            InterpreterProfile::IbmPc,
            "{name}: an explicit interpreter number wins",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The blast-radius pin, restated where the feature actually fires. Header byte
/// `$1E` is not inert — `zvm`'s `exec.rs` branches on `read_byte(0x1E) == 6` —
/// so naming a PC archive must leave the byte exactly where zvm's own rule put
/// it. It does, because `IbmPc` has no opinion on the number.
#[test]
fn naming_a_pc_archive_leaves_the_v6_corpus_on_ibm_pc() {
    if story("zork0.eg1").is_none() {
        return;
    }
    let dir = game_dir_with("blast", Some("pictures = \"zork0.eg1\"\n"));
    let over = PictureOverride::resolve(&stories_dir().join("anything.z6"), &dir);
    let profile = InterpreterProfile::resolve(&stories_dir().join("anything.z6"), None, over.flavour());
    assert_eq!(profile, InterpreterProfile::IbmPc);
    assert_eq!(profile.interpreter_number(), None, "zvm's 6-for-v6 rule stays in force");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── the standard window a native archive has no chunk to declare ─────────────

/// SQ-0736's lesson applied to tier 3: a native archive has no `Reso` chunk
/// because the format has no such concept, and reading that silence as Blorb
/// §11's "non-scalable, draw 1:1" declaration is what left Zork Zero's art at
/// half size. The archive does say it, in the picture space its coordinates use.
///
/// Every rendition of a game is a drawing of the SAME screen, so every one of
/// them supplies the same 320×200 standard window. What differs is how dense the
/// drawing is, and that is the art scale, not the window (SQ-0790).
///
/// SQ-0734 answered `None` here for a 640-wide EGA/CGA archive, as a recorded
/// deferral: its true presentation looked like a 640×200 screen on an 8×8 cell,
/// which a single uniform `V6_ART_SCALE` cannot express. Half of that reading
/// was wrong and this test is where it changes. 640×200 on an 8×8 cell is 80×25
/// characters — the same character grid the 640×400 unit screen already gives on
/// its 8×16 cell — so the screen never needed to move. Only the art density did.
#[test]
fn every_rendition_supplies_the_same_standard_window() {
    for name in ["zork0.pic", "zork0.mg1", "zork0.eg1", "zork0.cg1"] {
        if story(name).is_none() {
            continue;
        }
        let dir = game_dir_with(&format!("stdwin-{name}"), Some(&format!("pictures = {name:?}\n")));
        let over = PictureOverride::resolve(&stories_dir().join("anything.z6"), &dir);
        assert_eq!(over.std_window(), Some((320, 200)), "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Nothing named declares nothing.
    let none = game_dir_with("stdwin-unset", None);
    assert_eq!(PictureOverride::resolve(Path::new("/nowhere/s.z6"), &none).std_window(), None);
    let _ = std::fs::remove_dir_all(&none);
}

/// The other half of SQ-0790: the density.
///
/// A 320-wide rendition (Amiga `Pic.data`, MCGA `.MG1`) doubles onto the 640×400
/// unit screen, which is what has always happened. A 640-wide one (EGA `.EG1`,
/// CGA `.CG1`) stores the same artwork with pixels half as wide, so it arrives
/// at **(1, 2)** — one unit pixel across per art pixel, two down.
///
/// Not our invention: bocfel calls it `pixelwidth` and sets it to 0.5 whenever
/// `hw_screenwidth` is 640, and Frotz reads the same header bit as
/// `x_scale = (flags & 0x08) ? 640 : 320`.
///
/// A Blorb answers `None` and keeps the uniform rule — the path every story in
/// the corpus takes.
#[test]
fn a_640_wide_rendition_arrives_at_half_width_pixels() {
    let cases: [(&str, (u32, u32)); 4] = [
        ("zork0.pic", (2, 2)),
        ("zork0.mg1", (2, 2)),
        ("zork0.eg1", (1, 2)),
        ("zork0.cg1", (1, 2)),
    ];
    for (name, want) in cases {
        if story(name).is_none() {
            continue;
        }
        let dir = game_dir_with(&format!("scale-{name}"), Some(&format!("pictures = {name:?}\n")));
        let over = PictureOverride::resolve(&stories_dir().join("anything.z6"), &dir);
        let picts = PictSource::resolve_with_override(&stories_dir().join("anything.z6"), over);
        assert_eq!(picts.art_scale(), Some(want), "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // A Blorb has no picture space to declare one with, so the uniform rule stands.
    if let Some(z0) = story("zork0-r393-s890714.z6") {
        if story("Zork0.blb").is_some() {
            let none = game_dir_with("scale-blorb", None);
            let picts =
                PictSource::resolve_with_override(&z0, PictureOverride::resolve(&z0, &none));
            assert_eq!(picts.art_scale(), None, "a Blorb keeps the uniform V6_ART_SCALE");
            let _ = std::fs::remove_dir_all(&none);
        }
    }
}

/// The measurement that makes the (1, 2) rule a finding rather than a reading:
/// the two DOS renditions of one game must land in the same places.
///
/// Arthur is the clean case — `arthur.mg1` (320-wide) and `arthur.eg1`
/// (640-wide) share 125 pictures, and under this rule all 125 produce byte-equal
/// unit-space dimensions. Zork Zero agrees on 446 of the 503 it shares; the rest
/// differ by a pixel or two because EGA and MCGA are separately drawn artwork,
/// not one scaled copy of the other, so the assertion there is on the bulk.
///
/// Falsified by reverting `art_scale` to a uniform `(2, 2)`: Arthur's agreement
/// drops from 125/125 to 0/125, every EGA plate coming out twice as wide as the
/// screen.
#[test]
fn the_ega_and_mcga_renditions_of_one_game_land_in_the_same_places() {
    let unit_dims = |name: &str| -> Option<std::collections::BTreeMap<u16, (u32, u32)>> {
        story(name)?;
        let dir = game_dir_with(&format!("agree-{name}"), Some(&format!("pictures = {name:?}\n")));
        let over = PictureOverride::resolve(&stories_dir().join("anything.z6"), &dir);
        let mut picts = PictSource::resolve_with_override(&stories_dir().join("anything.z6"), over);
        let (sx, sy) = picts.art_scale().expect("a native archive declares its picture space");
        let table = picts
            .all_pict_dims()
            .into_iter()
            .map(|(n, w, h)| (n, (u32::from(w) * sx, u32::from(h) * sy)))
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        Some(table)
    };

    for (mcga, ega, floor) in [("arthur.mg1", "arthur.eg1", 100), ("zork0.mg1", "zork0.eg1", 85)] {
        let (Some(a), Some(b)) = (unit_dims(mcga), unit_dims(ega)) else { continue };
        let shared: Vec<u16> = b.keys().filter(|k| a.contains_key(k)).copied().collect();
        assert!(shared.len() > 50, "{mcga}/{ega} share only {} pictures", shared.len());
        let agree = shared.iter().filter(|k| a[k] == b[k]).count();
        assert!(
            agree * 100 >= shared.len() * floor,
            "{mcga} vs {ega}: only {agree}/{} pictures land in the same place",
            shared.len(),
        );
        // The full-screen plate is the one a player looks at first, and it must
        // agree exactly on both games.
        if let (Some(&am), Some(&be)) = (a.get(&1), b.get(&1)) {
            assert_eq!(am, be, "{mcga} vs {ega}: picture 1");
        }
    }
}

/// The consequence of the rule above, on the case a player is most likely to
/// choose: naming Zork Zero's MCGA rendition must put its art exactly where the
/// Blorb's was. Both are 320×200, both double onto the 640×400 unit screen, so
/// the dimension table the engine boots against is identical — the pixels differ
/// (native colours, and the five plates the Blorb truncates), the geometry does
/// not.
#[test]
fn a_named_mcga_rendition_lands_where_the_blorb_art_did() {
    let Some(z0) = story("zork0-r393-s890714.z6") else { return };
    if story("zork0.mg1").is_none() || story("Zork0.blb").is_none() {
        return;
    }

    let plain = game_dir_with("mcga-off", None);
    let over_off = PictureOverride::resolve(&z0, &plain);
    let blorb_std = PictSource::resolve_with_override(&z0, PictureOverride::resolve(&z0, &plain))
        .std_window()
        .or(over_off.std_window());

    let dir = game_dir_with("mcga-on", Some("pictures = \"zork0.mg1\"\n"));
    let over = PictureOverride::resolve(&z0, &dir);
    let mut mcga = PictSource::resolve_with_override(&z0, PictureOverride::resolve(&z0, &dir));
    let mcga_std = mcga.std_window().or(over.std_window());

    assert_eq!(blorb_std, Some((320, 200)), "the Blorb's own Reso chunk");
    assert_eq!(mcga_std, blorb_std, "the MCGA archive must imply the same standard window");
    assert_eq!(mcga.dims(1), Some((320, 200)), "and the same art-native size");

    let _ = std::fs::remove_dir_all(&plain);
    let _ = std::fs::remove_dir_all(&dir);
}

/// SQ-0790 end to end, on the real game: booting Zork Zero against its EGA
/// rendition must put the machine, the screen and the artwork exactly where its
/// MCGA rendition does.
///
/// This is the assertion the whole quest reduces to. The two archives hold the
/// same game's art at two pixel densities, so once each is mapped onto the
/// 640×400 unit screen the ENGINE cannot tell them apart: same screen words,
/// same `picture_data` answers, same full-screen plate.
///
/// The falsification is in the test, at the bottom: the same EGA boot with the
/// art scale withheld (`None`, i.e. the uniform rule SQ-0734 shipped) reports
/// picture 1 as 1280×400 — twice the width of the screen it is meant to fill.
#[test]
fn zork_zeros_ega_rendition_boots_the_geometry_its_mcga_one_does() {
    let Some(z0) = story("zork0-r393-s890714.z6") else { return };
    if story("zork0.mg1").is_none() || story("zork0.eg1").is_none() {
        return;
    }
    let bytes = std::fs::read(&z0).expect("Zork Zero reads");

    // Boot exactly the way `startup.rs` does: the sidecar names the archive, the
    // archive supplies both the standard window and the art scale.
    let boot = |archive: &str, scale: Option<(u32, u32)>| -> GameSession {
        let dir = game_dir_with(&format!("boot-{archive}"), Some(&format!("pictures = {archive:?}\n")));
        let over = PictureOverride::resolve(&z0, &dir);
        let std_window = over.std_window();
        let mut picts = PictSource::resolve_with_override(&z0, over);
        let art_scale = scale.map(Some).unwrap_or_else(|| picts.art_scale());
        let dims = picts.all_pict_dims();
        let mut s = GameSession::new_with_art_scale(
            bytes.clone(), false, false, None, false, dims, std_window, art_scale, None, None,
        )
        .expect("Zork Zero boots");
        s.set_pict_source(Some(picts));
        s.flush_boot_pictures();
        let _ = std::fs::remove_dir_all(&dir);
        s
    };
    let reported = |s: &GameSession, n: u16| -> Option<(u16, u16)> {
        s.machine.picture_dims.iter().find(|&&(i, _, _)| i == n).map(|&(_, w, h)| (w, h))
    };

    let mcga = boot("zork0.mg1", None);
    let ega = boot("zork0.eg1", None);

    for (name, s) in [("MCGA", &mcga), ("EGA", &ega)] {
        assert!(!s.quit, "{name} quit during boot");
        assert!(s.machine.fault_trace.is_none(), "{name} faulted during boot");
        // The unit screen is babelmap's, not the card's, and never moves.
        assert_eq!(s.machine.mem.read_word(0x22), 640, "{name} screen width, header $22");
        assert_eq!(s.machine.mem.read_word(0x24), 400, "{name} screen height, header $24");
        // …on the 8×16 cell, which is the EGA 80×25 character grid too.
        assert_eq!(s.machine.mem.read_byte(0x27), 8, "{name} font width, header $27");
        assert_eq!(s.machine.mem.read_byte(0x26), 16, "{name} font height, header $26");
    }

    assert_eq!(reported(&ega, 1), Some((640, 400)), "the EGA plate fills the unit screen");
    assert_eq!(reported(&ega, 1), reported(&mcga, 1), "and lands where the MCGA plate does");

    // Not just picture 1: the table the game lays itself out from agrees on the
    // overwhelming majority of the 503 pictures the two renditions share.
    let agree = mcga
        .machine
        .picture_dims
        .iter()
        .filter(|&&(n, w, h)| reported(&ega, n) == Some((w, h)))
        .count();
    assert!(
        agree * 100 >= mcga.machine.picture_dims.len() * 85,
        "only {agree}/{} picture_data answers agree between the two renditions",
        mcga.machine.picture_dims.len(),
    );

    // FALSIFICATION: the shipped uniform rule, applied to the same archive.
    let uniform = boot("zork0.eg1", Some((2, 2)));
    assert_eq!(
        reported(&uniform, 1),
        Some((1280, 400)),
        "without the per-axis scale the EGA plate is twice the width of the screen",
    );
}

// ── the motivating case, end to end ──────────────────────────────────────────

/// `fmvpoker.z6` drawing from `FMVPOKER.EG1`, all the way to decoded pixels.
///
/// This is the story SQ-0734 was written around. Its readme tells the player to
/// rename a Zork Zero graphics file to `FMVPOKER.EG1`, and the fixture in
/// `stories/` is byte-identical to `stories/zork0.eg1`. Nothing about the name,
/// the header or the contents ties it to this story — only the user saying so —
/// which is why it can be reached by tier 3 and by nothing else.
///
/// Asserted end to end: the key resolves, the archive parses as the PC container,
/// the flavour picks the IBM PC, the `PictSource` prefers it over `fmvpoker.blb`
/// (which is itself a copy of `Zork0.blb`), the dimension table the engine boots
/// against comes from the archive, and a picture decodes to real pixels.
#[test]
fn fmvpoker_draws_from_the_archive_its_readme_names() {
    let Some(story_path) = story("fmvpoker.z6") else { return };
    if story("FMVPOKER.EG1").is_none() {
        return;
    }
    let dir = game_dir_with("fmvpoker", Some("pictures = \"FMVPOKER.EG1\"\n"));

    let over = PictureOverride::resolve(&story_path, &dir);
    let PictureOverride::Loaded { ref path, ref pics } = over else {
        panic!("expected Loaded, got {over:?}")
    };
    assert!(path.ends_with("FMVPOKER.EG1"));
    assert_eq!(pics.flavour(), Flavour::Pc, "content, not the odd filename");
    assert_eq!(pics.entries().len(), 503, "Zork Zero's EGA directory");
    assert_eq!(
        InterpreterProfile::resolve(&story_path, None, over.flavour()),
        InterpreterProfile::IbmPc,
        "an EGA archive is an IBM PC",
    );

    let mut src = PictSource::resolve_with_override(&story_path, over);
    let dims = src.all_pict_dims();
    assert_eq!(dims.len(), 503, "the engine's dimension table comes from the archive");
    assert!(
        dims.contains(&(1, 640, 200)),
        "picture 1 is the archive's 640-wide EGA plate, not the Blorb's 320-wide one",
    );
    let img = src.image(1).expect("picture 1 decodes");
    assert_eq!(
        (image::GenericImageView::dimensions(&*img)),
        (640, 200),
        "and it decodes to the archive's pixels",
    );

    // FALSIFICATION: without the key the same story resolves `fmvpoker.blb`, a
    // byte-identical copy of `Zork0.blb`, whose picture 1 is 320 wide. If tier 3
    // were not doing anything, the two would agree.
    let plain = game_dir_with("fmvpoker-off", None);
    let mut blorb_src = PictSource::resolve_with_override(
        &story_path,
        PictureOverride::resolve(&story_path, &plain),
    );
    assert_eq!(
        blorb_src.dims(1),
        Some((320, 200)),
        "the Blorb path must differ, or this test proves nothing",
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&plain);
}
