//! SQ-0789 / SQ-0791 — the three doors into the picture-override mechanism, and
//! the boot-time choices only a launch can make.
//!
//! SQ-0734 landed the machinery: a named native archive beats a Blorb and beats
//! an `.adf`'s own `Pic.data`, its flavour picks the machine, and a name that
//! will not decode is loud. What was missing was any way to *reach* it without
//! hand-editing a file first — which is backwards for a choice you can only
//! judge by looking at it. Three doors, one mechanism:
//!
//!     story on the command line  ->  --pictures                  (SQ-0791)
//!     picked from the browser    ->  the launch-options dialog   (SQ-0789)
//!     persistent preference      ->  pictures = "…" in the game's config.toml
//!
//! Two properties matter more than the plumbing and are what this suite pins.
//!
//! **Discovery for DISPLAY is safe; discovery for PAIRING is not.** Enumerating
//! the archives that carry a story's name and showing them to a person is safe
//! for exactly the reason auto-pairing is unsafe: the person knows which game
//! they own and supplies the assertion the format cannot make. So the list
//! exists, and nothing consumes it automatically. The name filter narrows which
//! rows a person is *shown* and decides nothing — an archive under an unrelated
//! name stays reachable by being named, through `--pictures` or the `pictures`
//! key, which is where it always belonged.
//!
//! **The sidecar's "absent key = inherit" contract survives the checkbox.** A key
//! written at the value it already inherits is not the same as a key left absent
//! — it converts an inheritance into a pin — so the dialog writes only what the
//! user actually changed, and an untouched dialog writes nothing at all.
//!
//! Real archives are gitignored, so every case using one skips vacuously.

use std::path::PathBuf;

use app::launch_options::{
    derived_interpreter, discover_art_candidates, InterpreterSource, LaunchOptionsState,
    LaunchOverrides,
};
use app::styles::{per_game_config_path, read_per_game_interpreter_number, read_per_game_pictures};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("babelmap-launchopt-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Zork Zero is the case the whole feature exists for: one game, five renditions
/// of its art sitting side by side, and no way to tell them apart from a
/// filename. The dialog's list is what makes that a choice instead of a guess.
#[test]
fn every_rendition_of_zork_zero_is_offered_with_enough_to_choose_by() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return; // gitignored fixture
    }
    let found = discover_art_candidates(&z0);
    assert!(!found.is_empty(), "Zork Zero's archives sit beside it");

    for c in &found {
        // A bare filename is not enough to choose by; the point of parsing each
        // candidate is that the list can state what it actually is.
        assert!(c.pictures > 0, "{} lists no pictures", c.filename);
        assert!(c.part >= 1, "{} has no part number", c.filename);
        assert!(
            matches!(c.rendition, "Amiga" | "MCGA" | "EGA" | "CGA" | "EGA/CGA"),
            "{} got an unrecognised rendition label {:?}",
            c.filename,
            c.rendition
        );
        assert!(c.space_width == 320 || c.space_width == 640);
        // The caveat is EGA's dithering, not anyone's geometry. SQ-0790 landed
        // the per-axis art scale, so 640-wide art IS drawn at true aspect now;
        // what is left is that EGA's column-by-column dither does not fuse at
        // 1:1 (SQ-0797). CGA is 640-wide too and carries no caveat, because
        // two-colour line art has no dither to fuse (SQ-0794).
        assert_eq!(
            c.caveat().is_some(),
            c.space_width == 640 && c.rendition != "CGA",
            "{}",
            c.filename
        );
    }

    // The Blorb beside it is tier 1 and is never a pickable native archive; the
    // story file is not one either.
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("Zork0.blb")));
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("zork0.z6")));
}

/// A story library is usually one flat folder — `stories/` holds Arthur,
/// Journey, Shogun and Zork Zero together — so "every archive beside this story"
/// is mostly other games' art, and offering it all was a dialog that made the
/// user do the filtering. The list is now what its heading says: the archives
/// detected **for this story**.
///
/// This is the table the whole change rests on, pinned against the real library.
/// Every pairing here is one the normalised name test finds, and each names the
/// direction it needed: a story stem that contains the archive's (`zork0` inside
/// `zork0-r393-s890714`), an archive stem that is only a prefix of a longer game
/// name (`beyondzo`), a spaced disk-image title that a prefix rule could never
/// have matched (`James Clavell's Shogun`), and a pair that differ only in case.
#[test]
fn each_game_in_the_library_detects_its_own_archives_and_no_others() {
    // (story, must detect, must NOT detect)
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "zork0-r393-s890714.z6",
            &["zork0.cg1", "zork0.eg1", "zork0.mg1", "zork0.pic"],
            &["arthur.mg1", "journey.mg1", "shogun.mg1", "FMVPOKER.EG1", "beyondzo.mg1"],
        ),
        // `.eg2` is deliberately in the "must NOT" column for both multi-part
        // games: it is the back half of `.eg1`, which now loads it, and offering
        // it alone would be offering 101 of Arthur's 171 ids as if it were the
        // set (SQ-0798).
        (
            "arthur-r74-s890714.z6",
            &["arthur.cg1", "arthur.eg1", "arthur.mg1", "arthur.pic"],
            &["zork0.mg1", "journey.mg1", "shogun.mg1", "arthur.eg2"],
        ),
        (
            "journey-r83-s890706.z6",
            &["journey.cg1", "journey.eg1", "journey.mg1"],
            &["zork0.mg1", "arthur.mg1", "shogun.mg1", "journey.eg2"],
        ),
        (
            "shogun-r322-s890706.z6",
            &["shogun.cg1", "shogun.eg1", "shogun.mg1"],
            &["zork0.mg1", "arthur.mg1", "journey.mg1"],
        ),
        // `beyondzo` is the DOS 8.3 truncation of the game's name: a prefix of
        // the story stem, never the whole of it.
        ("beyondzork-r57-s871221.z5", &["beyondzo.mg1"], &["zork0.mg1", "arthur.mg1"]),
        // The renamed archive the escape hatch exists for — for the fan game it
        // was renamed FOR, the name now matches outright.
        ("fmvpoker.z6", &["FMVPOKER.EG1"], &["zork0.mg1", "arthur.mg1"]),
        // Disk images: a spaced, punctuated, article-carrying title on one side
        // and an 8.3 stem on the other. Only normalising both and testing for
        // containment in either direction connects them.
        ("James Clavell's Shogun.adf", &["shogun.mg1"], &["zork0.mg1", "journey.mg1"]),
        (
            "Beyond Zork - The Coconut of Quendor.adf",
            &["beyondzo.mg1"],
            &["zork0.mg1", "arthur.mg1"],
        ),
    ];

    for (story, wanted, unwanted) in cases {
        let path = stories_dir().join(story);
        if !path.is_file() {
            continue; // gitignored fixture
        }
        let found = discover_art_candidates(&path);
        let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();
        for w in *wanted {
            // Only assert on archives this library actually has.
            if stories_dir().join(w).is_file() {
                assert!(
                    found.iter().any(|c| c.filename.eq_ignore_ascii_case(w)),
                    "{story} must detect {w}; got {names:?}"
                );
            }
        }
        for u in *unwanted {
            assert!(
                !found.iter().any(|c| c.filename.eq_ignore_ascii_case(u)),
                "{story} must NOT be offered {u}; got {names:?}"
            );
        }
    }
}

/// A multi-part EGA set is ONE row, and it advertises the whole set (SQ-0798).
///
/// Before this, Arthur offered `arthur.eg1` and `arthur.eg2` as two choices, and
/// both were wrong: the first loaded 97 of 137 pictures and the second is not an
/// archive at all, just the back half of one. Now the first carries the second
/// and the second is not offered.
///
/// The row's picture count is the merged directory — 171 for Arthur, which is
/// exactly what `arthur.mg1` holds, the same artwork shipped undivided. The
/// panel and the dialog both render from this list, so pinning it here pins both.
#[test]
fn a_split_ega_archive_is_offered_as_one_entry_carrying_both_files() {
    for (story_file, head, tail, want_entries, want_mcga) in [
        ("arthur-r74-s890714.z6", "arthur.eg1", "arthur.eg2", 171usize, "arthur.mg1"),
        ("journey-r83-s890706.z6", "journey.eg1", "journey.eg2", 135, "journey.mg1"),
    ] {
        let path = stories_dir().join(story_file);
        if !path.is_file() || !stories_dir().join(tail).is_file() {
            continue; // gitignored fixtures
        }
        let found = discover_art_candidates(&path);
        let names: Vec<&str> = found.iter().map(|c| c.filename.as_str()).collect();

        let c = found
            .iter()
            .find(|c| c.filename.eq_ignore_ascii_case(head))
            .unwrap_or_else(|| panic!("{head} must be listed; got {names:?}"));
        assert_eq!(c.parts, 2, "{head} is two files");
        assert_eq!(c.part, 1, "and the row names the first of them");
        assert_eq!(c.pictures, want_entries, "{head} advertises the WHOLE set");
        assert!(
            !found.iter().any(|x| x.filename.eq_ignore_ascii_case(tail)),
            "{tail} is half an archive and must not be a separate choice; got {names:?}",
        );

        // The undivided MCGA release of the same artwork is the oracle: if the
        // merged count were wrong, the two renditions would stop agreeing on how
        // many pictures this game has. (Journey's EGA set carries one extra —
        // id 59, a solid-colour blanking plate no other rendition has.)
        if let Some(m) = found.iter().find(|x| x.filename.eq_ignore_ascii_case(want_mcga)) {
            assert_eq!(m.parts, 1, "{want_mcga} is a single disk");
            let slack = usize::from(story_file.starts_with("journey"));
            assert_eq!(
                c.pictures,
                m.pictures + slack,
                "{head} and {want_mcga} are the same game's art",
            );
        }
    }
}

/// A continuation whose part 1 is NOT on disk is still listed — it is all there
/// is, and it says so.
///
/// The collapse rule is "hide a part whose predecessor is present", not "hide
/// anything numbered above 1". Someone with only disk two should see disk two,
/// flagged, rather than an empty dialog.
#[test]
fn a_lone_continuation_is_still_offered_and_says_which_part_it_is() {
    let eg2 = stories_dir().join("arthur.eg2");
    if !eg2.is_file() {
        return; // gitignored fixture
    }
    let dir = tmp("lone-part2");
    std::fs::copy(&eg2, dir.join("arthur.eg2")).unwrap();
    std::fs::write(dir.join("arthur.z6"), b"x").unwrap();

    let found = discover_art_candidates(&dir.join("arthur.z6"));
    let c = found
        .iter()
        .find(|c| c.filename.eq_ignore_ascii_case("arthur.eg2"))
        .expect("with no part 1 beside it, part 2 is the only choice there is");
    assert_eq!((c.part, c.parts), (2, 1));
    assert_eq!(c.pictures, 101, "and it advertises only what it actually holds");

    // Put part 1 back and the row disappears into it.
    std::fs::copy(stories_dir().join("arthur.eg1"), dir.join("arthur.eg1")).unwrap();
    let found = discover_art_candidates(&dir.join("arthur.z6"));
    assert!(!found.iter().any(|c| c.filename.eq_ignore_ascii_case("arthur.eg2")));
    assert_eq!(
        found.iter().find(|c| c.filename.eq_ignore_ascii_case("arthur.eg1")).map(|c| c.pictures),
        Some(171),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The list is sorted and stable, and it never carries the story itself or a
/// Blorb — the two files most likely to be sitting right beside it.
#[test]
fn the_detected_list_is_sorted_and_holds_only_native_archives() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let found = discover_art_candidates(&z0);
    let names: Vec<String> = found.iter().map(|c| c.filename.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the list is alphabetical");
    assert_eq!(found, discover_art_candidates(&z0), "and stable across calls");
    assert!(found.iter().all(|c| !c.filename.eq_ignore_ascii_case("Zork0.blb")));
    assert!(found.iter().all(|c| !c.filename.to_lowercase().ends_with(".z6")));
}

/// The dialog is a door into SQ-0734's mechanism, not a second one: the name it
/// hands over is exactly what `pictures = "…"` would have said, and the
/// resolution that follows is the one already tested by `picture_override.rs`.
#[test]
fn a_chosen_rendition_becomes_the_same_override_the_config_key_would_have() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), false);
    let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1"))
    else {
        return; // this library has no MCGA rendition
    };
    st.art = i + 1;
    let ov = st.overrides();
    assert_eq!(ov.pictures.as_deref(), Some("zork0.mg1"));

    // And that name resolves through the very same entry point boot uses.
    let over = app::graphics::PictureOverride::resolve_with_session(
        &z0,
        &tmp("session"),
        ov.pictures.as_deref(),
    );
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "the session name must load exactly as the config key does, got {over:?}"
    );
    // …and it beats an empty sidecar and the Blorb both, which is the precedence
    // SQ-0734 set: naming an archive is an instruction, not a hint.
    assert_eq!(over.flavour(), Some(blorb::infocom_pics::Flavour::Pc));
}

/// The session name outranks the config key. A user with `pictures` set who
/// passes `--pictures` (or picks something else in the dialog) needs the more
/// specific, more recent instruction to win — and the help text says so.
#[test]
fn a_launch_time_name_outranks_the_games_own_config_key() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    let pic = stories_dir().join("zork0.pic");
    let mg1 = stories_dir().join("zork0.mg1");
    if !z0.is_file() || !pic.is_file() || !mg1.is_file() {
        return;
    }
    let game_dir = tmp("outrank");
    std::fs::write(per_game_config_path(&game_dir), "pictures = \"zork0.pic\"\n").unwrap();

    // With no session name, the sidecar decides: the Amiga archive, hence Amiga.
    let sidecar = app::graphics::PictureOverride::resolve(&z0, &game_dir);
    assert_eq!(sidecar.flavour(), Some(blorb::infocom_pics::Flavour::AmigaMac));

    // With one, it wins outright — a different flavour, so a different machine.
    let session =
        app::graphics::PictureOverride::resolve_with_session(&z0, &game_dir, Some("zork0.mg1"));
    assert_eq!(session.flavour(), Some(blorb::infocom_pics::Flavour::Pc));

    // The session choice does not touch the file: it applies to this launch only,
    // which is the whole try-before-you-commit idea.
    assert_eq!(read_per_game_pictures(&game_dir), Some("zork0.pic".to_string()));
    let _ = std::fs::remove_dir_all(&game_dir);
}

/// The inherit contract. A dialog that wrote every field it displayed would pin
/// settings the user never touched, and the story would stop tracking later
/// changes to the global config.
#[test]
fn the_checkbox_writes_only_the_keys_the_user_changed() {
    let dir = tmp("inherit");
    let story = dir.join("story.z6");
    std::fs::write(&story, b"x").unwrap();
    let game_dir = dir.join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    // A story that already inherits a global interpreter number of 6.
    let mut st = LaunchOptionsState::new("Story", &story, None, Some(6), Some(6), false);

    // Ticking the box without changing anything writes nothing — an untouched
    // dialog must be indistinguishable from never opening it.
    st.persist = true;
    st.persist_to(&game_dir).unwrap();
    assert!(
        !per_game_config_path(&game_dir).exists(),
        "an untouched dialog must not create a sidecar"
    );
    assert_eq!(st.overrides(), LaunchOverrides::default());

    // Changing one key writes one key. Not two, and not the inherited value of
    // the other — `interpreter_number = 6` present is NOT the same as absent.
    st.interpreter = Some(4);
    st.persist_to(&game_dir).unwrap();
    let body = std::fs::read_to_string(per_game_config_path(&game_dir)).unwrap();
    assert_eq!(read_per_game_interpreter_number(&game_dir), Some(4));
    assert_eq!(read_per_game_pictures(&game_dir), None, "untouched key stays absent: {body:?}");
    assert_eq!(body.lines().filter(|l| !l.trim().is_empty()).count(), 1, "{body:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An existing hand-written `pictures` key survives the dialog writing its
/// sibling, which is the SQ-0734 carry-through rule extended to the new writers.
#[test]
fn writing_one_launch_key_never_deletes_the_other() {
    let dir = tmp("carry");
    let story = dir.join("story.z6");
    std::fs::write(&story, b"x").unwrap();
    let game_dir = dir.join("game");
    std::fs::create_dir_all(&game_dir).unwrap();
    std::fs::write(
        per_game_config_path(&game_dir),
        "pictures = \"FMVPOKER.EG1\"\nhonor_game_colours = false\n",
    )
    .unwrap();

    let mut st = LaunchOptionsState::new("Story", &story, Some("FMVPOKER.EG1"), None, Some(6), false);
    st.interpreter = Some(6);
    st.persist_to(&game_dir).unwrap();
    assert_eq!(read_per_game_pictures(&game_dir), Some("FMVPOKER.EG1".to_string()));
    assert_eq!(read_per_game_interpreter_number(&game_dir), Some(6));
    assert_eq!(app::styles::read_per_game_honor(&game_dir), Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Picking prettier art MOVES the emulated machine unless a number is set
/// outright, and header byte 0x1E is not inert — `crates/zvm/src/cpu/exec.rs`
/// branches on it. The dialog derives the number the same way boot does and
/// reports where it came from, because doing that silently is the surprise the
/// dialog exists to prevent.
#[test]
fn the_art_choice_moves_the_machine_and_the_dialog_can_name_the_source() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), false);

    // No art named: Frotz's rule, and it says so.
    assert_eq!(st.derived(), Some((6, InterpreterSource::Default)));

    // The Amiga rendition asks for the Amiga — a different number, from the art.
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.pic")) {
        st.art = i + 1;
        assert_eq!(st.derived(), Some((4, InterpreterSource::Artwork)));
        // The same answer `InterpreterProfile::resolve` reaches at boot, so the
        // dialog is reporting the real chain rather than a parallel guess.
        let profile = app::interpreter::InterpreterProfile::resolve(&z0, None, st.chosen_art().map(|c| c.flavour));
        assert_eq!(profile.interpreter_number(), Some(4));
    }
    // A DOS rendition asks for the IBM PC, which defers to zvm's own rule.
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1")) {
        st.art = i + 1;
        assert_eq!(st.derived(), Some((6, InterpreterSource::Artwork)));
    }
    // An explicit number beats the art, and reports itself as explicit.
    st.interpreter = Some(3);
    assert_eq!(st.derived(), Some((3, InterpreterSource::Explicit)));
}

/// The exact chain `boot_story` runs, for the CLI door: `--pictures` →
/// `resolve_with_session` → `InterpreterProfile::resolve`. Naming the Amiga
/// archive on the command line moves header byte 0x1E to 4, which is not a
/// cosmetic change — `crates/zvm/src/cpu/exec.rs` branches on that byte.
#[test]
fn the_cli_door_reaches_the_machine_the_same_way_boot_does() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() || !stories_dir().join("zork0.pic").is_file() {
        return;
    }
    let empty = tmp("cli-chain");

    // Baseline: no flag, no sidecar → the Blorb, and the default machine.
    let none = app::graphics::PictureOverride::resolve_with_session(&z0, &empty, None);
    let base = app::interpreter::InterpreterProfile::resolve(&z0, None, none.flavour());
    assert_eq!(base, app::interpreter::InterpreterProfile::IbmPc);
    assert_eq!(base.interpreter_number(), None, "IBM PC defers to zvm's own rule");

    // With the flag naming the Amiga archive, the machine follows the art.
    let named =
        app::graphics::PictureOverride::resolve_with_session(&z0, &empty, Some("zork0.pic"));
    assert!(matches!(named, app::graphics::PictureOverride::Loaded { .. }), "{named:?}");
    let amiga = app::interpreter::InterpreterProfile::resolve(&z0, None, named.flavour());
    assert_eq!(amiga, app::interpreter::InterpreterProfile::Amiga);
    assert_eq!(amiga.interpreter_number(), Some(4));

    // …unless a number is set outright, which still wins over the art.
    let pinned = app::interpreter::InterpreterProfile::resolve(&z0, Some(6), named.flavour());
    assert_eq!(pinned, app::interpreter::InterpreterProfile::IbmPc);
    let _ = std::fs::remove_dir_all(&empty);
}

/// A per-game or per-launch interpreter number must never reach the GLOBAL
/// config. `write_config_at` persists `interpreter_number` unless the value is
/// marked as belonging to this run — so a story whose sidecar pins the Amiga,
/// played once with the settings screen opened, would otherwise hand every other
/// story machine 4 from then on.
#[test]
fn a_per_launch_interpreter_number_never_leaks_into_the_global_config() {
    let dir = tmp("leak");
    let cfg_path = dir.join("config.toml");
    std::fs::write(&cfg_path, "interpreter_number = 6\n").unwrap();

    // What `boot_story` does for a sidecar / dialog value: in force this run,
    // and flagged as such.
    let mut cfg = app::config::Config {
        config_file: cfg_path.clone(),
        interpreter_number: Some(4),
        ..Default::default()
    };
    cfg.one_run.pin(app::config::keys::INTERPRETER_NUMBER, 4u8);
    assert!(cfg.interpreter_number_from_cli(), "the value is marked one-run");
    app::config::write_config_file(&cfg).unwrap();
    let after = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(after.contains("interpreter_number = 6"), "the global key is untouched: {after:?}");

    // A deliberate settings-screen edit is a different act and DOES persist.
    cfg.set_interpreter_number(Some(4));
    app::config::write_config_file(&cfg).unwrap();
    let after = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(after.contains("interpreter_number = 4"), "an explicit edit persists: {after:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Glulx or Scott story has no header 0x1E at all, so there is no number to
/// report and the dialog must not invent one.
#[test]
fn a_non_z_story_has_no_interpreter_number_to_report() {
    assert_eq!(derived_interpreter(None, None, false, None), None);
    assert_eq!(derived_interpreter(None, None, true, None), None);
}

/// Reopening the dialog on a story whose sidecar already names an archive lands
/// on that archive, and overrides nothing — the baseline is what it inherits.
#[test]
fn the_dialog_opens_on_what_the_story_already_inherits() {
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let all = discover_art_candidates(&z0);
    let Some(i) = all.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.eg1")) else {
        return;
    };
    let st = LaunchOptionsState::new("Zork Zero", &z0, Some("zork0.eg1"), Some(4), Some(6), false);
    assert_eq!(st.art, i + 1, "the sidecar's archive is the selected row");
    assert_eq!(st.interpreter, Some(4));
    assert!(st.overrides().is_empty(), "opening and playing changes nothing");
    // Backing out to "use this story's own art" cannot be an override — there is
    // no "override with nothing" — so the dialog flags that only the checkbox
    // can carry it, rather than appearing to act and then not acting.
    let mut cleared = st.clone();
    cleared.art = 0;
    assert!(cleared.clears_inherited_art());
    assert!(cleared.overrides().is_empty());
}

/// The rendered dialog, as the user sees it. Kept as an assertion rather than a
/// snapshot so it survives cosmetic edits, but it prints the frame so a reviewer
/// can read it (`cargo nextest run -p app launch_options -- --nocapture`).
#[test]
fn the_dialog_renders_its_list_its_derived_number_and_its_checkbox() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let z0 = stories_dir().join("zork0-r393-s890714.z6");
    if !z0.is_file() {
        return;
    }
    let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), false);
    if let Some(i) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.eg1")) {
        st.art = i + 1;
        st.cursor = app::launch_options::Row::Art(i + 1);
    }
    st.persist = true;

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
    println!("{frame}");

    assert!(frame.contains("Launch options"), "{frame}");
    assert!(frame.contains("Zork Zero"));
    assert!(frame.contains("Artwork"));
    assert!(frame.contains("zork0.eg1"), "every candidate is listed by name");
    assert!(frame.contains("pictures"), "with its picture count");
    // Zork Zero's renditions are all single-disk, so no row carries a
    // multi-part note — the column that used to say "part 1" on every line is
    // gone, because a constant is not information (SQ-0798).
    assert!(!frame.contains("part 1"), "a single-part archive says nothing about parts: {frame}");
    assert!(!frame.contains("disks"), "and claims no second disk: {frame}");
    assert!(frame.contains("header 0x1E"), "the derived interpreter number");
    assert!(frame.contains("from the artwork"), "and where it came from");
    assert!(
        frame.contains("dithered colours do not fuse"),
        "the EGA caveat, which is SQ-0797's dithering and no longer SQ-0790's geometry",
    );
    assert!(frame.contains("[x] Save as this game's default"), "the checkbox, ticked");
}
