//! SQ-0942: the committed gallery recipe stays loadable, and stays honest.
//!
//! `examples/gallery.toml` is the only committed half of the gallery — the PNGs
//! are output and live under `target/`. That makes this file the thing that can
//! go stale silently: nobody runs the gallery except at release time, and a
//! manifest that no longer parses, or that has grown a duplicate id, or that
//! quietly sets an argument the tool owns, would be discovered by whoever is
//! trying to cut a release. These cases move that discovery to the gate.
//!
//! Everything here is about the RECIPE. No case boots a story or wants a
//! gitignored medium, so the whole suite runs in CI exactly as it runs locally.
//! What the frames LOOK like is not testable and is not tested; that is the
//! user's eye and the proof sheet's job.

#![cfg(unix)]

use super::pty_stream::gallery::{Backend, Manifest, Shot};

/// The committed manifest, or a panic naming the file — this is not a fixture
/// that may be absent.
fn manifest() -> Manifest {
    let path = Manifest::default_path();
    Manifest::load(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// One [`Shot`] built from TOML, for the negative cases.
fn parse_one(body: &str) -> Result<Manifest, String> {
    Manifest::parse(&format!("[[shots]]\n{body}"))
}

/// A shot with every required field, which the negative cases perturb.
const GOOD: &str = r#"
id = "a-shot"
title = "A Game"
press = "story file"
caption = "A caption."
media = "stories/a.z6"
keys = "cr,wait:900,text:look,cr"
size = "117x40"
expect = ["something"]
"#;

#[test]
fn the_committed_manifest_parses_and_validates() {
    let m = manifest();
    assert!(
        m.shots.len() >= 8,
        "the gallery is meant to span several titles, presses and pane sizes; {} shot(s) is not that",
        m.shots.len()
    );
}

/// The control on the negative cases below: `GOOD` really is good, so a case
/// that perturbs it and fails is failing for the reason it names.
#[test]
fn the_specimen_shot_is_valid() {
    assert!(parse_one(GOOD).is_ok(), "the specimen must parse, or every negative case below proves nothing");
}

#[test]
fn every_shot_has_a_guard() {
    for s in &manifest().shots {
        assert!(
            !s.expect.is_empty() || s.expect_art_cells > 0,
            "`{}` has no `expect` and no `expect_art_cells`. Every number a shot records — release, \
             serial, medium — is read from the FILE, so all of them stay correct while the frame shows \
             a browser, a boot prompt, or a different story off the same disk",
            s.id
        );
    }
}

#[test]
fn ids_are_unique_and_survive_being_filenames() {
    let m = manifest();
    let mut seen = std::collections::BTreeSet::new();
    for s in &m.shots {
        assert!(seen.insert(s.id.clone()), "duplicate shot id `{}` — ids are PNG filenames", s.id);
        assert!(
            s.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "`{}` is not a safe filename",
            s.id
        );
    }
}

/// Every `keys` spec parses, and the turn count the label prints is the number
/// of real keypresses in it — not the number of tokens, which counts the waits.
#[test]
fn key_specs_parse_and_the_turn_count_ignores_waits() {
    for s in &manifest().shots {
        let keys = s.keys().unwrap_or_else(|e| panic!("{e}"));
        assert!(!keys.is_empty(), "`{}` drives no keys at all", s.id);
        assert!(s.turns() <= keys.len(), "`{}`: the turn count cannot exceed the key count", s.id);
    }
    let one = parse_one(&GOOD.replace(
        r#"keys = "cr,wait:900,text:look,cr""#,
        r#"keys = "wait:100,cr,wait:200,cr,wait:300""#,
    ))
    .expect("valid");
    assert_eq!(one.shots[0].turns(), 2, "three waits and two keys is two keypresses");
}

/// The half-block trap, encoded: the picker assumes a 10x20 cell whatever the
/// terminal reported, so the cell size is the BACKEND's to choose.
#[test]
fn the_backend_chooses_the_cell_size() {
    for s in &manifest().shots {
        let want = match s.backend {
            Backend::Kitty => (16, 32),
            Backend::Halfblocks => (10, 20),
        };
        assert_eq!(s.cell_px(), want, "`{}` ({})", s.id, s.backend.as_str());
    }
}

/// Every cell this tool captures at is exactly 1:2 (SQ-0963).
///
/// A half-block sample is `cell_width` wide by `cell_height / 2` tall, so a cell
/// of any other aspect samples the artwork finer across than down — JetBrains
/// Mono's 2.200 cell, which these shots used to be taken in, is 10% coarser
/// vertically at every size anyone would pick. The kitty cell was 8x18 (2.250),
/// then 8x16, and is now 16x32 (SQ-1001) — the ratio is the invariant, not the
/// absolute size.
///
/// FALSIFY by putting 18 back, or 16x30: this fails, and
/// `the_backend_chooses_the_cell_size` fails beside it. Both are meant to,
/// because the pair is the whole pin — one says which cell, this one says why
/// that shape of cell and not a neighbouring one.
#[test]
fn the_cell_is_square_for_half_block_samples() {
    for backend in [Backend::Kitty, Backend::Halfblocks] {
        let s = parse_one(&format!("{GOOD}backend = {:?}\n", backend.as_str()))
            .expect("valid")
            .shots
            .remove(0);
        let (w, h) = s.cell_px();
        assert_eq!(
            h,
            w * 2,
            "the {} cell is {w}x{h}. A half-block sample is one cell wide and half a cell tall, so \
             only a 1:2 cell makes it square — and the gallery's face lands a whole-numbered cell on \
             exactly the 1:2 sizes (5x10, 6x12, 7x14, 8x16, 9x18, 10x20, 11x22, 13x26, 14x28, 15x30)",
            backend.as_str()
        );
    }
}

/// The default face is chosen on a measurement, so the list that names it is
/// pinned rather than left to be re-sorted by whoever next has a preference.
///
/// Fira Code's cell is 0.615 / 1.231 em = **2.000 exactly**, and it is the only
/// face measured on this machine that lands a whole-numbered 1:2 cell at ten of
/// the nineteen sizes in 6..24 px/em. Every other candidate below it is there as
/// a fallback for a machine that has no Nerd Font at all.
#[test]
fn the_default_font_list_leads_with_the_face_whose_cell_is_one_to_two() {
    use super::pty_stream::gallery::FONT_CANDIDATES;
    let first = FONT_CANDIDATES.first().copied().unwrap_or_default();
    assert!(
        first.contains("FiraCode"),
        "the gallery's default face leads with `{first}`. It is supposed to lead with Fira Code, whose \
         cell is exactly 1:2 — see FONT_CANDIDATES' own table. Reordering this list changes what every \
         shot is sampled at, so it is a measurement to redo and not a preference to express"
    );
    assert!(
        FONT_CANDIDATES.iter().any(|c| c.starts_with("~/")),
        "Nerd Fonts install per-user (`~/Library/Fonts`, `~/.local/share/fonts`), so a list of purely \
         absolute paths could never find the face this tool leads with"
    );
}

/// The arithmetic half of SQ-0963, pinned without needing a single gitignored
/// medium: at the sizes this manifest uses, a 640x400 press magnifies by a whole
/// number and the neighbouring sizes do not.
///
/// The control matters more than the assertion. 117x40 — what every shot in this
/// file used to be — is 1.4375x, and a case that only checked the sizes we chose
/// would pass just as happily if `magnification` returned a constant 2.
///
/// The kitty rungs are HALF what they were, because the cell doubled (SQ-1001):
/// the device box each grid covers is unchanged, so 82x28 on a 16x32 cell is the
/// same 1280x800 the old 162x53 was on an 8x16 one. The magnification is a
/// property of the box and not of the cell, which is exactly why the type could
/// double without a single frame changing size or crispness.
#[test]
fn the_manifest_sizes_are_the_ones_that_magnify_a_640x400_press_by_a_whole_number() {
    let on = |size: &str, backend: &str, native: (u32, u32)| -> f64 {
        let body = GOOD.replace(r#"size = "117x40""#, &format!("size = {size:?}"));
        let body = format!("{body}backend = {backend:?}\n");
        parse_one(&body).expect("valid").shots[0].magnification(native).expect("a full-width pane")
    };
    let mag = |size: &str, backend: &str| on(size, backend, (640, 400));
    for (size, want) in [("82x28", 2.0), ("122x41", 3.0), ("162x53", 4.0)] {
        assert_eq!(mag(size, "kitty"), want, "kitty {size}");
    }
    for (size, want) in [("66x23", 1.0), ("130x43", 2.0)] {
        assert_eq!(mag(size, "halfblocks"), want, "half-blocks {size}");
    }
    assert_eq!(mag("117x40", "kitty"), 2.875, "the size this manifest used to use, and the reason it no longer does");
    assert_eq!(mag("160x50", "kitty"), 3.76, "one row and two columns off 4x is still a resampled frame");
    // The Macintosh's monochrome plates are the one press here that is not
    // 640x400, and the two shots off that disk are the ones that are not 82
    // columns wide. Both
    // halves matter: the size is chosen for the SCREEN, not copied off the row
    // above it (SQ-1001).
    assert_eq!(on("62x22", "kitty", (480, 300)), 2.0, "the monochrome Macintosh's 480x300 screen");
    let sloppy = on("82x28", "kitty", (480, 300));
    assert!(
        (sloppy - 2.666_666_666_666_666_5).abs() < 1e-9,
        "82x28 on the monochrome Macintosh is {sloppy}, not a whole number — which is what copying \
         the standard size onto a press with a different screen buys"
    );
}

/// The committed manifest itself, over whichever media this machine has.
///
/// Skips vacuously where `stories/` is absent — it is gitignored, so CI has none
/// of it — but the moment a medium IS present its native screen is read off the
/// mount and the shot's size has to magnify it by a whole number.
#[test]
fn every_v6_shot_magnifies_by_a_whole_number() {
    use super::pty_stream::gallery::Provenance;
    let m = manifest();
    let mut any_present = false;
    let mut checked = 0usize;
    for s in &m.shots {
        // A library shot mounts nothing at all — see `Provenance::Library`.
        if s.library {
            continue;
        }
        let path = s.media_path();
        if !path.exists() {
            continue;
        }
        any_present = true;
        let Ok(p) = Provenance::read(&path, s.pictures()) else { continue };
        // A story with no pixel screen has no magnification, and the map shot's
        // pane is a split this file deliberately does not restate.
        let (Some(native), Some(mag)) = (p.native, p.native.and_then(|n| s.magnification(n))) else {
            continue;
        };
        checked += 1;
        assert!(
            (mag - mag.round()).abs() < 1e-9 && mag >= 1.0,
            "`{}` is {} on a {}x{} press, which magnifies by {mag}. Every edge in that frame is \
             interpolated: the composite is resized once to round(native * s) and the bands are 1:1 \
             crops out of it, so a fractional s is the one place softness can enter (SQ-0963). \
             gallery.toml's header has the sizes that land on a whole number",
            s.id,
            s.size,
            native.0,
            native.1
        );
    }
    assert!(
        !any_present || checked > 0,
        "the media are present but not one shot resolved a native screen — the derivation in \
         `Provenance::read` has stopped working, and a check that silently stops checking is worse \
         than none"
    );
}

/// A `--pictures` shot's archive is read back out of `args`, because the
/// provenance under the frame is computed FROM it (SQ-1001).
///
/// The named archive settles the machine and the picture space both, so a shot
/// whose `--pictures` the harness cannot see gets a native screen, a
/// magnification and a profile belonging to the rendition it did not draw — and
/// every one of those numbers stays self-consistent, which is why `expect` cannot
/// catch it. Two renditions of one scene look like each other.
#[test]
fn a_named_picture_archive_is_read_back_out_of_the_arguments() {
    let one = parse_one(&format!("{GOOD}args = [\"--pictures\", \"Pic.data\"]\n")).expect("valid");
    assert_eq!(one.shots[0].pictures(), Some("Pic.data"));
    assert_eq!(parse_one(GOOD).expect("valid").shots[0].pictures(), None, "a shot that names none has none");
    // A flag with nothing after it is not a name, and must not become one.
    let bare = parse_one(&format!("{GOOD}args = [\"--pictures\"]\n")).expect("valid");
    assert_eq!(bare.shots[0].pictures(), None);
    // And the committed manifest really does exercise the path.
    assert!(
        manifest().shots.iter().any(|s| s.pictures().is_some()),
        "no shot names an archive with `--pictures`. The Macintosh disk ships two, and the gallery \
         exists to show that they differ"
    );
}

/// `--image-protocol halfblocks` is added by the tool for a half-block shot and
/// by nobody else, so a manifest can never disagree with its own `backend`.
#[test]
fn halfblock_shots_carry_the_protocol_override_and_kitty_shots_do_not() {
    for s in &manifest().shots {
        let args = s.lanthorn_args();
        let has = args.windows(2).any(|w| w[0] == "--image-protocol" && w[1] == "halfblocks");
        match s.backend {
            Backend::Halfblocks => assert!(has, "`{}` is a half-block shot without the override", s.id),
            Backend::Kitty => assert!(
                !args.iter().any(|a| a == "--image-protocol"),
                "`{}` is a kitty shot and must NEGOTIATE kitty, not be told to use it — a forced \
                 protocol would turn a failed negotiation into a silent success",
                s.id
            ),
        }
    }
}

/// A half-block capture emits no placements at all, so `expect_art_cells` can
/// only ever read zero there. The manifest is refused rather than left to fail
/// at capture time, three minutes in.
#[test]
fn a_halfblock_shot_may_not_ask_for_placement_cells() {
    let bad = parse_one(&format!("{GOOD}backend = \"halfblocks\"\nexpect_art_cells = 100\n"));
    let e = bad.expect_err("half-blocks emit no placements; asking for them can never pass");
    assert!(e.contains("expect_art_cells"), "the error must name the field: {e}");
}

/// The arguments the tool owns. A manifest that sets `--image-protocol` fights
/// its own `backend`; one that sets `--user-dir` writes the gallery run into the
/// player's real lanthorn home.
#[test]
fn a_shot_may_not_pass_an_argument_the_tool_owns() {
    for owned in ["--image-protocol", "--user-dir", "--no-sound"] {
        let bad = parse_one(&format!("{GOOD}args = [\"{owned}\", \"x\"]\n"));
        let e = bad.unwrap_err();
        assert!(e.contains(owned), "the error must name `{owned}`: {e}");
    }
}

#[test]
fn a_shot_with_no_guard_is_refused() {
    let bad = parse_one(&GOOD.replace(r#"expect = ["something"]"#, ""));
    let e = bad.expect_err("a shot with no guard cannot tell its frame from a boot prompt");
    assert!(e.contains("expect"), "the error must name the field: {e}");
}

#[test]
fn a_duplicate_id_is_refused() {
    let two = format!("[[shots]]{GOOD}[[shots]]{GOOD}");
    let e = Manifest::parse(&two).expect_err("ids become filenames");
    assert!(e.contains("duplicate"), "{e}");
}

#[test]
fn a_bad_size_is_refused() {
    for size in ["117", "0x40", "117x0", "wide x tall"] {
        let bad = parse_one(&GOOD.replace(r#"size = "117x40""#, &format!("size = {size:?}")));
        assert!(bad.is_err(), "`{size}` is not a terminal size");
    }
}

#[test]
fn a_bad_key_token_is_refused() {
    let bad = parse_one(&GOOD.replace(r#"keys = "cr,wait:900,text:look,cr""#, r#"keys = "cr,sneeze,cr""#));
    let e = bad.expect_err("`sneeze` is not a key");
    assert!(e.contains("sneeze"), "the error must name the token: {e}");
}

#[test]
fn an_unknown_field_is_refused() {
    let bad = parse_one(&format!("{GOOD}turns = 4\n"));
    assert!(
        bad.is_err(),
        "`turns` is DERIVED from `keys`; letting a manifest declare one would create a second copy \
         of the truth, which is the whole thing this manifest is shaped to avoid"
    );
}

// ── Library shots (SQ-1080) ───────────────────────────────────────────────────

/// A library shot names a `[[libraries]]` entry the way every other shot names a
/// file, and a name that resolves to nothing is refused at PARSE time.
///
/// It has to be caught here rather than at capture: the alternative is a shot
/// that skips on every run for ever, which reads as coverage and is not any —
/// exactly the reason these two shots were removed from the manifest the first
/// time round instead of being left in.
#[test]
fn a_library_shot_must_name_a_declared_library() {
    let body = format!("{GOOD}library = true\n");
    let e = parse_one(&body).expect_err("`stories/a.z6` is not a declared library id");
    assert!(e.contains("[[libraries]]"), "the error must say what is missing: {e}");

    let ok = Manifest::parse(&format!(
        "[[libraries]]\nid = \"lib\"\nfrom = \"stories\"\nmembers = [\"a.z6\"]\n\
         [[shots]]{}media = \"lib\"\nlibrary = true\n",
        GOOD.replace(r#"media = "stories/a.z6""#, "")
    ));
    assert!(ok.is_ok(), "a declared library resolves: {ok:?}");
}

/// Everything that describes a STORY is refused on a library shot, because no
/// story has been opened and a field silently ignored is worse than one
/// rejected.
#[test]
fn a_library_shot_may_not_describe_a_story() {
    let lib = "[[libraries]]\nid = \"lib\"\nfrom = \"stories\"\nmembers = [\"a.z6\"]\n";
    let base = format!("{}media = \"lib\"\nlibrary = true\n", GOOD.replace(r#"media = "stories/a.z6""#, ""));
    assert!(Manifest::parse(&format!("{lib}[[shots]]{base}")).is_ok(), "the control must parse");
    for extra in ["raster = true\n", "show_map = true\n", "args = [\"--pictures\", \"Pic.data\"]\n"] {
        let e = Manifest::parse(&format!("{lib}[[shots]]{base}{extra}"))
            .expect_err("a library shot has no story for `{extra}` to apply to");
        assert!(e.contains("library shot"), "the error must say why: {e}");
    }
}

/// A library with no members, or a member that is a path rather than a name.
#[test]
fn a_malformed_library_is_refused() {
    let shot = format!("[[shots]]{}media = \"lib\"\nlibrary = true\n", GOOD.replace(r#"media = "stories/a.z6""#, ""));
    for (members, why) in [
        ("[]", "an empty picker is not a picture of a catalogue"),
        ("[\"sub/a.z6\"]", "a member must be a bare filename"),
        ("[\"a.z6\", \"a.z6\"]", "a member named twice"),
    ] {
        let text = format!("[[libraries]]\nid = \"lib\"\nfrom = \"stories\"\nmembers = {members}\n{shot}");
        assert!(Manifest::parse(&text).is_err(), "{why}");
    }
}

/// The committed manifest really does exercise the library path, and its members
/// are named rather than swept out of a directory.
#[test]
fn the_committed_manifest_opens_a_library() {
    let m = manifest();
    assert!(m.shots.iter().any(|s| s.library), "no library shot — the picker is two of README's stills");
    assert!(!m.libraries.is_empty(), "a library shot with no [[libraries]] should not have parsed");
    for l in &m.libraries {
        assert!(
            l.members.len() >= 8,
            "library `{}` has {} member(s); a cover grid wants enough of them to BE a grid",
            l.id,
            l.members.len()
        );
    }
}

/// Every shot pins a seed, whether or not its game deals randomly.
///
/// The cost of pinning one that does not need it is nothing; the cost of missing
/// one that does is a gallery that regenerates differently every release for no
/// reason — measured once at 37,097 differing pixels, every one of them a card
/// deal rather than a render change.
#[test]
fn every_shot_pins_a_seed() {
    for s in &manifest().shots {
        assert!(s.seed != 0, "`{}` has no pinned seed", s.id);
    }
}

/// The gallery is supposed to SPAN the corpus. A set that has quietly collapsed
/// onto one game at one size is still a valid manifest and a useless gallery.
#[test]
fn the_set_spans_several_media_and_more_than_one_pane_size() {
    let m = manifest();
    let media: std::collections::BTreeSet<&str> = m.shots.iter().map(|s| s.media.as_str()).collect();
    let sizes: std::collections::BTreeSet<&str> = m.shots.iter().map(|s| s.size.as_str()).collect();
    assert!(media.len() >= 5, "only {} distinct medium(s): {media:?}", media.len());
    assert!(sizes.len() >= 3, "only {} distinct pane size(s): {sizes:?}", sizes.len());
    assert!(
        m.shots.iter().any(|s| s.backend == Backend::Halfblocks),
        "no half-block row — the fallback everyone without a graphics protocol sees"
    );
    assert!(m.shots.iter().any(|s: &Shot| s.show_map), "no row with the map pane, which is the project's differentiator");
}

/// Two presses of one game are only interesting side by side, and the row the
/// site plan's `/media` page is built on is Zork Zero across four media.
#[test]
fn at_least_one_game_appears_on_several_media() {
    let m = manifest();
    let mut by_title: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for s in &m.shots {
        *by_title.entry(s.title.as_str()).or_default() += 1;
    }
    assert!(
        by_title.values().any(|&n| n >= 3),
        "no game is shown on three or more media/sizes, which is the comparison the gallery exists for: {by_title:?}"
    );
}

/// The label is what survives the picture being dragged out of the page, so it
/// has to WRAP rather than clip: the seed and the release live at the end of the
/// provenance line, and a clip drops exactly them.
#[test]
fn the_label_wraps_rather_than_clips() {
    use super::pty_stream::gallery;
    let frame = image::RgbaImage::from_pixel(320, 40, image::Rgba([0, 0, 0, 255]));
    let long = "x".repeat(400);
    let out = gallery::label(&frame, &["a short first line".into(), long]);
    assert_eq!(out.width(), frame.width(), "the label must not widen the frame");
    assert!(
        out.height() > frame.height() + 32,
        "a 400-character line in a 320px strip has to WRAP; clipping it would drop the seed or the \
         release, which is the half of the label that has to travel with the picture"
    );
}
