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
/// A 320-wide rendition (Amiga `Pic.data`, MCGA `.MG1`) is the ordinary Infocom
/// v6 standard window and doubles onto the 640×400 unit screen. A 640-wide one
/// (EGA, CGA) has half-width pixels, needs a 640×200 screen on an 8×8 cell, and
/// is DEFERRED — `V6_ART_SCALE` is one uniform integer for both axes, so there
/// is no scale that expresses it. Pinned here so the deferral is a recorded
/// decision rather than an accident.
#[test]
fn a_320_wide_rendition_supplies_the_standard_window_and_a_640_wide_one_defers() {
    let cases: [(&str, Option<(u16, u16)>); 4] = [
        ("zork0.pic", Some((320, 200))),
        ("zork0.mg1", Some((320, 200))),
        ("zork0.eg1", None),
        ("zork0.cg1", None),
    ];
    for (name, want) in cases {
        if story(name).is_none() {
            continue;
        }
        let dir = game_dir_with(&format!("stdwin-{name}"), Some(&format!("pictures = {name:?}\n")));
        let over = PictureOverride::resolve(&stories_dir().join("anything.z6"), &dir);
        assert_eq!(over.std_window(), want, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Nothing named declares nothing.
    let none = game_dir_with("stdwin-unset", None);
    assert_eq!(PictureOverride::resolve(Path::new("/nowhere/s.z6"), &none).std_window(), None);
    let _ = std::fs::remove_dir_all(&none);
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
