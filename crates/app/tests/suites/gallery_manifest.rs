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
    for owned in ["--image-protocol", "--user-dir", "--sound", "--v6-pixel-lock"] {
        let bad = parse_one(&format!("{GOOD}args = [\"{owned}\", \"x\"]\n"));
        let e = bad.unwrap_err();
        assert!(e.contains(owned), "the error must name `{owned}`: {e}");
    }
    // And the committed file keeps its hands off all four.
    for s in &manifest().shots {
        for owned in ["--image-protocol", "--user-dir", "--sound", "--v6-pixel-lock"] {
            assert!(
                !s.args.iter().any(|a| a == owned),
                "`{}` passes `{owned}`, which the harness sets for the whole run",
                s.id
            );
        }
    }
}

/// SQ-1152: the whole gallery captures under one set of settings, written into each
/// shot's throwaway user directory by `write_run_settings`.
///
/// The pixel lock and the patched-font icons are the two the user asked for by
/// name, and both are properties of the SET rather than of any frame: a gallery
/// where one frame is softer than its neighbours, or draws `◈` where the rest draw
/// a Nerd Font chevron, is inconsistent in a way no per-shot field would make
/// obvious. Pinned here because the alternative is remembering, and the whole point
/// of moving them into the harness was to stop anyone having to.
#[test]
fn every_shot_captures_pixel_locked_and_with_the_patched_font_icons() {
    use super::pty_stream::gallery::write_run_settings;

    let dir = app::scratch_dir("gallery-run-settings");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let shot = &parse_one(GOOD).expect("the control shot parses").shots[0];
    write_run_settings(&dir, shot, std::path::Path::new("stories/a.z6")).expect("settings written");

    let cfg = std::fs::read_to_string(dir.join("config.toml")).expect("config.toml written");
    assert!(
        cfg.contains("v6_pixel_lock = true"),
        "every shot captures pixel-locked, so a fractional magnification cannot enter one \
         frame while its neighbours stay crisp — got:\n{cfg}"
    );
    assert!(cfg.contains("random_seed = "), "the seed still goes through the same seam:\n{cfg}");

    // The icons are asserted through the app's own resolver rather than by matching
    // the strings this harness happens to write: what matters is the SymbolConfig a
    // run ends up with, and reading it back the way lanthorn does is the only thing
    // that proves the file it wrote is the file lanthorn reads.
    let style = app::style::personal_style_path(&dir);
    assert!(style.is_file(), "a style.toml is written beside the config");
    let (doc, warns) = app::style::load_style(None, &dir);
    assert!(warns.is_empty(), "the written style must load cleanly: {warns:?}");
    let sym = app::style::finalize_symbols(&doc.symbols);
    assert_eq!(
        sym.control_icons, "nerdfont",
        "the frames are captured through a Nerd Font, so the plain Geometric Shapes \
         fallback is the wrong half of the choice — `zork1-map` reported `NO GLYPH \
         ANYWHERE FOR: ◈◌` before this was wired"
    );
    assert_eq!(sym.arrow_set, "nerdfont", "the map's arrows come from the same answer");
    assert_eq!(sym.portal_icons, "nerdfont-stairs", "…and its portal icons");
    // Exactly what a `yes` to the font check sets, because that is the call this
    // makes. If the app's presets are renamed, this fails and the gallery follows
    // rather than drifting.
    assert_eq!(sym.arrow_set, app::render::font_check_dialog::NERD_ARROWS);
    assert_eq!(sym.portal_icons, app::render::font_check_dialog::NERD_PORTALS);
    assert_eq!(sym.control_icons, app::render::font_check_dialog::NERD_CONTROLS);

    let _ = std::fs::remove_dir_all(&dir);
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

/// `v6_render` is a MODE, not a pair of bools (SQ-1152).
///
/// It was `raster = true` until `extended` arrived. Two bools would have been able
/// to spell "raster and extended at once", which is not a state the app has; one
/// token cannot. And the tokens are the app's own — the manifest writes the value
/// straight into the run's `config.toml`, so a spelling this file accepted and
/// `v6_render_from_key` did not would be a shot that silently captured hybrid.
#[test]
fn the_render_mode_is_a_token_the_app_itself_parses() {
    use app::config::V6RenderMode;
    for (token, want) in [
        ("hybrid", V6RenderMode::Hybrid),
        ("raster", V6RenderMode::Raster),
        ("extended", V6RenderMode::Extended),
    ] {
        let m = parse_one(&format!("{GOOD}v6_render = \"{token}\"\n"))
            .unwrap_or_else(|e| panic!("`{token}` must be a valid mode: {e}"));
        assert_eq!(m.shots[0].v6_render, Some(want));
        assert_eq!(app::config::v6_render_key(want), token, "the manifest and the config key must be one spelling");
    }
    assert_eq!(parse_one(GOOD).expect("valid").shots[0].v6_render, None, "a shot that names no mode takes the shipped default");
    // A typo must not read as hybrid. The app's own config deserializer is
    // deliberately forgiving there (a bad `~/.lanthorn/config.toml` must not stop a
    // game from launching); a manifest has no such excuse, and a shot that quietly
    // captured the wrong mode is exactly the drift `expect` exists to catch.
    assert!(
        parse_one(&format!("{GOOD}v6_render = \"rastr\"\n")).is_err(),
        "an unrecognised mode must be refused, not silently rendered as hybrid"
    );
    assert!(
        parse_one(&format!("{GOOD}raster = true\n")).is_err(),
        "the old `raster` bool must not still parse — a shot carrying it would capture hybrid"
    );
    // And the committed manifest exercises both modes that change anything.
    let m = manifest();
    for want in [V6RenderMode::Raster, V6RenderMode::Extended] {
        assert!(
            m.shots.iter().any(|s| s.v6_render == Some(want)),
            "no committed shot captures in {want:?} — the gallery exists to show the modes differ"
        );
    }
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
    for extra in [
        "v6_render = \"raster\"\n",
        // SQ-1152: the refusal is on the FIELD, not on one of its values — an
        // `extended` library shot is exactly as meaningless as a `raster` one.
        "v6_render = \"extended\"\n",
        "show_map = true\n",
        "args = [\"--pictures\", \"Pic.data\"]\n",
    ] {
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

/// SQ-1164: `expect_prose_cells` is refused wherever it could only ever read
/// zero, which is the same refusal `expect_art_cells` gets on half-blocks.
///
/// A guard that can only fail is worse than no guard: whoever set it believed
/// they had one, and the shot never captures at all.
#[test]
fn a_prose_floor_is_refused_where_it_could_only_read_zero() {
    assert!(
        parse_one(&format!("{GOOD}expect_prose_cells = 400\n")).is_ok(),
        "a plain hybrid shot is exactly what this field is for"
    );
    for (extra, why) in [
        ("v6_render = \"raster\"\n", "raster puts the whole screen in one image — no text cells at all"),
        ("v6_render = \"extended\"\n", "extended is raster, so the same is true of it"),
        ("show_map = true\n", "a split pane's width is ratatui's arithmetic, not this file's"),
    ] {
        let e = parse_one(&format!("{GOOD}expect_prose_cells = 400\n{extra}"))
            .expect_err(why);
        assert!(
            e.contains("expect_prose_cells"),
            "the error must name the field the author set: {e}"
        );
    }
    // And on a library shot, where the picker has opened no story to narrate.
    let lib = "[[libraries]]\nid = \"lib\"\nfrom = \"stories\"\nmembers = [\"a.z6\"]\n";
    let base = format!("{}media = \"lib\"\nlibrary = true\n", GOOD.replace(r#"media = "stories/a.z6""#, ""));
    let e = Manifest::parse(&format!("{lib}[[shots]]{base}expect_prose_cells = 400\n"))
        .expect_err("no story has been opened");
    assert!(e.contains("library shot"), "the error must say why: {e}");
}

/// Every Journey shot guards its PROSE, and that is not decoration (SQ-1164).
///
/// Journey is menu-driven: it has no typed parser prompt, so a recipe that stops
/// short lands on its opening menu with an EMPTY story pane, and both strings the
/// five shots name in `expect` — "The Party", "Individual Commands" — are
/// headings on that very menu. Five frames shipped for months under captions
/// about prose that showed none, and a human eye on a proof sheet is what caught
/// it. Keyed on the MEDIUM rather than on shot ids, so curating a row out of the
/// gallery does not fail this case.
#[test]
fn every_journey_shot_guards_its_prose() {
    let m = manifest();
    let mut seen = 0usize;
    for s in m.shots.iter().filter(|s| s.media.to_lowercase().contains("journey")) {
        seen += 1;
        assert!(
            s.expect_prose_cells > 0,
            "`{}` shows Journey's story pane and does not guard it. `expect` cannot: Journey is \
             menu-driven, and every string it names is a heading on the opening menu the recipe \
             has to drive PAST",
            s.id
        );
    }
    assert!(seen > 0, "no Journey shot at all — this case has stopped checking anything");
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

// ── The composite shot (SQ-1165) ──────────────────────────────────────────────

/// A composite names at least two machines, names none of them twice, and names
/// only machines `zvm::interpreter` has actually MEASURED.
///
/// The last clause is the one worth having. A number with no measured period look
/// is a machine lanthorn declines to guess at, and a tile of a guess is worse than
/// a missing tile — it looks exactly like a measurement in a frame whose whole
/// claim is that its colours are measurements.
#[test]
fn a_composite_names_at_least_two_measured_machines() {
    assert!(
        parse_one(&format!("{GOOD}machines = [2, 3]\n")).is_ok(),
        "two measured machines is the smallest real comparison"
    );
    // An empty list is simply not a composite; it must not be REFUSED, it must be
    // inert — otherwise `machines = []` and no `machines` line would mean
    // different things, which is a trap rather than a rule.
    assert!(parse_one(&format!("{GOOD}machines = []\n")).is_ok(), "an empty list is not a composite at all");
    for (machines, why) in [
        ("[3]", "one tile is a comparison with nothing"),
        ("[3, 3]", "two tiles of one machine are identical by construction"),
        // 1 is DECSystem-20 and 11 is the MSDOS row lanthorn auto-selects from;
        // neither has a period screen anyone has captured.
        ("[3, 1]", "interpreter 1 has no measured period look"),
        ("[3, 250]", "interpreter 250 is not a machine at all"),
    ] {
        assert!(parse_one(&format!("{GOOD}machines = {machines}\n")).is_err(), "{why}");
    }
}

/// The harness appends `--interpreter N --colour machine` per tile, so a
/// composite that names either itself is restating the truth at best and
/// contradicting it at worst.
///
/// `--colour machine` is the half that matters: the interpreter NUMBER tells the
/// story which machine it is on, and the opt-in is what licenses lanthorn to
/// paint that machine's screen on a bare story file (SQ-1154). A composite whose
/// `args` forced `--colour theme` would capture six tiles of the reader's theme
/// under a caption about 1984.
#[test]
fn a_composite_may_not_name_the_arguments_the_tiles_are_launched_with() {
    for owned in ["--interpreter", "--colour", "--color"] {
        let bad = parse_one(&format!("{GOOD}machines = [2, 3]\nargs = [\"{owned}\", \"3\"]\n"));
        let e = bad.expect_err("the harness owns both flags on a composite");
        assert!(e.contains(owned), "the error must name `{owned}`: {e}");
    }
    // And a shot with NO `machines` may still pass `--interpreter`, which
    // `macintosh-embedded-face` does and has since SQ-1001.
    assert!(
        parse_one(&format!("{GOOD}args = [\"--interpreter\", \"3\"]\n")).is_ok(),
        "an ordinary shot pinning one machine is not a composite and keeps its flag"
    );
}

/// Every tile's launch carries BOTH halves, and an ordinary shot carries neither.
#[test]
fn each_tile_launches_with_its_number_and_the_machine_colour_opt_in() {
    let shot = &parse_one(&format!("{GOOD}machines = [4, 6]\n")).expect("valid").shots[0];
    assert_eq!(shot.runs(), vec![Some(4), Some(6)], "one run per machine, in the order written");
    for n in [4u8, 6] {
        let args = shot.lanthorn_args_for(Some(n));
        assert!(
            args.windows(2).any(|w| w[0] == "--interpreter" && w[1] == n.to_string()),
            "tile {n} must be told which machine it is: {args:?}"
        );
        assert!(
            args.windows(2).any(|w| w[0] == "--colour" && w[1] == "machine"),
            "tile {n} must carry the opt-in that licenses a machine's colours on a bare story file \
             (SQ-1154) — without it every tile resolves page and ink through the theme and the six \
             come out identical: {args:?}"
        );
    }
    let plain = &parse_one(GOOD).expect("valid").shots[0];
    assert_eq!(plain.runs(), vec![None], "an ordinary shot is exactly one run");
    assert!(
        !plain.lanthorn_args().iter().any(|a| a == "--interpreter" || a == "--colour"),
        "an ordinary shot's launch is untouched by any of this"
    );
}

/// Each tile gets its own throwaway user directory.
///
/// A composite boots the SAME story once per machine, and a per-game sidecar is
/// keyed on the story path — so one shared directory would let the first tile's
/// saved answer decide what the sixth one launches with, which is the frame's one
/// variable leaking between its own tiles.
#[test]
fn every_tile_gets_its_own_user_directory() {
    let shot = &parse_one(&format!("{GOOD}machines = [2, 3, 4]\n")).expect("valid").shots[0];
    let ids: std::collections::BTreeSet<String> = shot.runs().iter().map(|m| shot.run_id(*m)).collect();
    assert_eq!(ids.len(), 3, "three tiles, three directories: {ids:?}");
    assert_eq!(
        parse_one(GOOD).expect("valid").shots[0].run_id(None),
        "a-shot",
        "an ordinary shot keeps its own id"
    );
}

/// Six tiles are 2 wide and 3 down, because the squarest GRID is not the squarest
/// PICTURE (SQ-1165).
///
/// `ceil(sqrt(6))` is 3, and 3x2 is the answer that looks obvious — it produced a
/// 3128x1402 frame, wider than 2:1, which any ordinary screen scales down until
/// the prose in it cannot be read. That defeats a shot whose entire subject is how
/// six machines paint text. The step being skipped is that a TILE is a terminal and
/// already landscape, so laying wide things out wide multiplies it.
///
/// **The control is the whole case.** A rule that only checked the answer we wanted
/// would pass just as happily if `tile_columns` returned a constant 2 — so this
/// also pins that the count follows the tile's SHAPE: six PORTRAIT tiles want three
/// columns, which is the mirror image and the thing a constant cannot do.
#[test]
fn the_tiles_are_laid_out_in_the_squarest_picture_not_the_squarest_grid() {
    let measured: Vec<u8> =
        (0u8..=255).filter(|&n| zvm::interpreter::period_look_for(n, None).is_some()).collect();
    let with = |n: usize, size: &str| {
        assert!(measured.len() >= n, "only {} machines are measured; this case wants {n}", measured.len());
        let list = measured[..n].iter().map(u8::to_string).collect::<Vec<_>>().join(", ");
        let body = GOOD.replace(r#"size = "117x40""#, &format!("size = {size:?}"));
        parse_one(&format!("{body}machines = [{list}]\n")).expect("valid").shots.remove(0)
    };
    // 64x20 on a 16x32 cell is a 1024x640 tile — landscape, like every terminal.
    // Two of those already make a wide enough picture to want stacking.
    for (n, want) in [(2usize, 1usize), (3, 2), (4, 2), (6, 2)] {
        assert_eq!(with(n, "64x20").tile_columns(), want, "{n} landscape tiles");
    }
    // THE CONTROL, and the reason this is not a case that would pass against a
    // constant 2: turn the tile on its side — 20x40 cells is a 320x1280 portrait
    // tile — and three of them lay out in a ROW where three landscape tiles stack
    // two-and-one. The count follows the tile's shape, which is the claim.
    assert_eq!(with(3, "20x40").tile_columns(), 3, "three portrait tiles lay out across");
    assert_eq!(with(3, "64x20").tile_columns(), 2, "…where three landscape ones do not");
}

/// The badge is right-aligned in the pane's lowest CLEAR band, so it lands on
/// nothing the frame is about (SQ-1165).
///
/// The status band is the pane's first row, the caret sits at the prompt, and the
/// prose is between them — those four things are the entire subject, and a badge
/// on top of any of them is worse than no badge. So the spot is found off the
/// tile's own resolved screen rather than assumed, and a pane with no clear band
/// answers `None` for the caller to report.
///
/// Built from a synthetic screen so it runs in CI: what is pinned is the SEARCH.
#[test]
fn the_badge_lands_on_clear_ground_below_the_prose() {
    use super::pty_stream::gallery;
    use super::pty_stream::oracle::Resolved;

    // Content rect is cols 1..=8, rows 1..=5, on a 16x32 cell.
    let shot = &parse_one(&GOOD.replace(r#"size = "117x40""#, r#"size = "10x8""#)).expect("valid").shots[0];
    let with_text_on = |rows: &[u16]| {
        let mut screen = Resolved::blank(10, 8);
        for row in 0..8u16 {
            for col in 0..10u16 {
                screen.cell_mut(row, col).ch = if rows.contains(&row) { 'x' } else { ' ' };
            }
        }
        screen
    };
    // Two cells wide: the badge right-aligns at cols 6..=7, leaving col 8 clear
    // inside the border, and takes the lowest clear PAIR of rows, which is 4 and 5.
    let at = gallery::badge_anchor(shot, &with_text_on(&[0, 1, 2, 3]), 2).expect("rows 4 and 5 are clear");
    assert_eq!(at, (6 * 16, 4 * 32), "right-aligned, and as low as clear ground allows");
    // A tile whose prose reaches the BOTTOM row pushes the badge up rather than
    // over it — which is the whole point, since the six tiles do not fill alike.
    let higher = gallery::badge_anchor(shot, &with_text_on(&[0, 1, 5]), 2).expect("rows 3 and 4 are clear");
    assert_eq!(higher, (6 * 16, 3 * 32), "the lowest clear pair is now 3 and 4");
    assert!(higher.1 < at.1, "never over the prose: {higher:?} vs {at:?}");
    // TWO rows and not one, so the badge's border never sits on the descenders of
    // the line above it: a pane clear only on its last row has nowhere to put it.
    assert_eq!(
        gallery::badge_anchor(shot, &with_text_on(&[1, 2, 3, 4]), 2),
        None,
        "one clear row is not a place for a two-row badge"
    );
    // A pane full to the last row is the same refusal — reported by the caller,
    // never papered over, because a badge on the prose is worse than no badge.
    assert_eq!(gallery::badge_anchor(shot, &with_text_on(&[1, 2, 3, 4, 5]), 2), None, "no clear ground at all");
    // And a badge too wide for the pane.
    assert_eq!(gallery::badge_anchor(shot, &with_text_on(&[]), 99), None, "wider than the pane");
    // The width the search asks for is the width the stamp draws, through one
    // function — two spellings of it is a check of a box the drawing does not use.
    assert_eq!(
        gallery::badge_cells(shot, "Amiga \u{b7} 4"),
        6,
        "nine glyphs at 8px plus 10px of padding is 82px, which is six 16px cells"
    );
}

/// THE MEASUREMENT BEHIND THE SIX, recomputed rather than restated (SQ-1165).
///
/// The frame shows one tile per DISTINCT look, and this is where that claim is
/// checked against `zvm::interpreter` instead of against a comment in
/// `gallery.toml`. Nine machines are measured and they are not nine looks: the
/// three Apple rows share `APPLE_PERIOD_LOOK`, and the Atari ST shares the
/// Macintosh's pair, differing only in caret shape.
///
/// The case asserts the SHAPE of that argument rather than a hard-coded six, so
/// it stays true if a tenth machine is ever measured: no two tiles in the shot
/// may share a full period look.
#[test]
fn the_composite_shows_one_tile_per_distinct_period_look() {
    let m = manifest();
    let mut composites = 0usize;
    for s in m.shots.iter().filter(|s| !s.machines.is_empty()) {
        composites += 1;
        let mut looks: std::collections::BTreeMap<String, Vec<u8>> = std::collections::BTreeMap::new();
        for &n in &s.machines {
            let look = zvm::interpreter::period_look_for(n, None)
                .unwrap_or_else(|| panic!("`{}` names interpreter {n}, which has no measured look", s.id));
            looks.entry(format!("{look:?}")).or_default().push(n);
        }
        for (look, ns) in &looks {
            assert_eq!(
                ns.len(),
                1,
                "`{}` names {ns:?}, which share one measured period look — they would render \
                 pixel-identical tiles, and a grid that repeats itself teaches less than a smaller \
                 one that does not: {look}",
                s.id
            );
        }
    }
    assert!(composites > 0, "no composite shot at all — this case has stopped checking anything");
}

/// The composite's machines resolve to more than one BODY pair (SQ-1165).
///
/// This is the half that can be checked without booting anything: `period_look_for`
/// is a pure lookup, so the obligation the capture-time guard enforces is
/// recomputed here and lands in CI, where `stories/` is absent and the frame
/// itself can never be taken.
///
/// **Two of the six share a body pair, and that is correct rather than a defect.**
/// Interpreter 2 (Apple) and interpreter 8 (Commodore 64) were both measured white
/// on black with a full-width reverse band; they differ in CARET —
/// `CursorShape::Block` against `CursorShape::Underscore` — which is a glyph and
/// not a colour. So the assertion is on the count of distinct bodies, not on every
/// pair differing: six tiles show five bodies, and a composite whose bodies all
/// collapsed to one would be six copies of a palette under a caption about six
/// machines.
#[test]
fn the_composites_machines_resolve_to_more_than_one_body_pair() {
    let m = manifest();
    for s in m.shots.iter().filter(|s| !s.machines.is_empty()) {
        // The story's Version decides the IBM PC's white — XZIP's #AAAAAA below
        // v6, YZIP's #FFFFFF at it — so the question is asked at the version the
        // shot's own medium carries, read off the filename it names rather than
        // off a mount, because the media are gitignored and CI has none of them.
        let zversion = if s.media.ends_with(".z3") { 3u8 } else { 5 };
        let bodies: std::collections::BTreeSet<String> = s
            .machines
            .iter()
            .filter_map(|&n| zvm::interpreter::period_look_for(n, Some(zversion)))
            .map(|l| format!("{:?}/{:?}/{:?}", l.page, l.ink, l.status))
            .collect();
        assert!(
            bodies.len() > 1,
            "`{}` names {:?} and every one of them resolves to the same page, ink and status band \
             at v{zversion}. The frame's only claim is that the tiles differ",
            s.id,
            s.machines
        );
        assert!(
            bodies.len() <= s.machines.len(),
            "`{}`: {} distinct bodies across {} machines",
            s.id,
            bodies.len(),
            s.machines.len()
        );
    }
}

/// `pane_look` reads the PANE, not the chrome — and it undoes SGR 7, which is the
/// only way a v3's reverse-video status band can be seen at all (SQ-1165).
///
/// Built from a synthetic screen rather than a capture, so it runs in CI: what is
/// being pinned is the READING, and a reading that folded the reverse away, or
/// counted lanthorn's own header, would compare two tiles equal while their story
/// panes differed completely.
#[test]
fn a_tiles_look_is_read_off_the_pane_with_the_reverse_undone() {
    use super::pty_stream::decode::Color;
    use super::pty_stream::gallery;
    use super::pty_stream::oracle::Resolved;

    let shot = &parse_one(&GOOD.replace(r#"size = "117x40""#, r#"size = "10x8""#)).expect("valid").shots[0];
    // Content rect is cols 1..=8, rows 1..=5.
    let page = Color::Rgb(0x07, 0x4B, 0xA1);
    let ink = Color::Rgb(0xFF, 0xFF, 0xFF);
    let mut screen = Resolved::blank(10, 8);
    for row in 0..8u16 {
        for col in 0..10u16 {
            // The chrome outside the pane is painted in colours NO machine can
            // move. If they leaked into the reading, every tile would read alike.
            let outside = row == 0 || row > 5 || col == 0 || col > 8;
            let cell = screen.cell_mut(row, col);
            cell.ch = 'x';
            cell.bg = if outside { Color::Rgb(0x11, 0x11, 0x11) } else { page };
            cell.fg = if outside { Color::Rgb(0x22, 0x22, 0x22) } else { ink };
            // Row 1 is the status band: SGR 7 over the body pair, which states no
            // colour of its own and is invisible to a reading that does not swap.
            cell.inverse = !outside && row == 1;
        }
    }
    let look = gallery::pane_look(shot, &screen).expect("a full-width pane");
    assert_eq!(look.page, page, "the page is the pane's dominant ground, not the chrome's");
    assert_eq!(look.ink, ink, "the ink is what the letters are written in");
    assert_eq!(
        look.pairs,
        [(page, ink), (ink, page)].into_iter().collect(),
        "two pairs: the body, and the status band the reverse turns inside out. A reading that \
         carried SGR 7 instead of undoing it would see one, and a Version 3 story was chosen for \
         this frame precisely because it has that second surface"
    );
}

/// The capture-time guard fails a composite whose tiles came out the same, and
/// spares one whose only twins are machines the table says are twins (SQ-1165).
///
/// Driven from synthetic looks, because the thing under test is the RULE. The
/// second half is the one that matters: interpreter 2 and interpreter 8 really do
/// share a body pair, so a guard written as "every tile must differ" would refuse
/// a perfectly correct frame — and whoever hit that would weaken the guard rather
/// than read the table.
#[test]
fn the_guard_fails_identical_tiles_and_spares_the_machines_that_match() {
    use super::pty_stream::decode::Color;
    use super::pty_stream::gallery::{self, PaneLook};

    let shot = &parse_one(&format!("{GOOD}machines = [2, 3, 8]\n")).expect("valid").shots[0];
    let look = |page: Color, ink: Color| PaneLook { page, ink, pairs: [(page, ink)].into_iter().collect() };
    let white_on_black = look(Color::Rgb(0, 0, 0), Color::Rgb(0xFF, 0xFF, 0xFF));
    let black_on_white = look(Color::Rgb(0xFF, 0xFF, 0xFF), Color::Rgb(0, 0, 0));

    // Apple (2) and the C64 (8) share a measured body pair; the Macintosh (3)
    // does not. This is the correct frame.
    let good = [(2u8, white_on_black.clone()), (3, black_on_white), (8, white_on_black.clone())];
    assert!(
        gallery::check_machines_differ(shot, 3, &good).is_ok(),
        "interpreter 2 and interpreter 8 were MEASURED the same — white on black, full reverse — \
         and differ only in caret shape. A guard that failed this would be refusing a correct frame"
    );

    // And the failure the whole kind exists to catch: the machine-colour licence
    // never reaches the story, so every tile falls through to one palette.
    let collapsed =
        [(2u8, white_on_black.clone()), (3, white_on_black.clone()), (8, white_on_black)];
    let e = gallery::check_machines_differ(shot, 3, &collapsed)
        .expect_err("the Macintosh's screen is black on white and cannot be white on black");
    assert!(e.contains("do not differ"), "the error must say what is wrong: {e}");
    assert!(e.contains("SQ-1154"), "…and point at the usual cause, the missing colour licence: {e}");
}

/// The composite's label says how many tiles it holds, and with which flags.
///
/// A picture gets separated from its page the first time somebody drags it into a
/// chat window, and this frame is the one in the set whose `size` describes a TILE
/// rather than the image: `64x20 cells` under a 3000-pixel picture, with nothing
/// else said, would be a claim that is simply wrong.
#[test]
fn a_composites_label_names_its_tiles() {
    use super::pty_stream::gallery::{self, Backend, Provenance, StoryProvenance, Taken};

    let taken = |machines: Vec<u8>| Taken {
        id: "machine-colours".into(),
        png: std::path::PathBuf::from("machine-colours.png"),
        provenance: Provenance::Story(StoryProvenance {
            version: 3,
            release: 27,
            serial: "831005".into(),
            medium: "a story file".into(),
            native: None,
        }),
        cols: 64,
        rows: 20,
        cell_w: 16,
        cell_h: 32,
        turns: 0,
        seed: gallery::DEFAULT_SEED,
        backend: Backend::Kitty,
        face: "a face".into(),
        verdict: "kitty".into(),
        attempts: 1,
        captured_bytes: 0,
        width: 3000,
        height: 1400,
        native: None,
        magnification: None,
        unresolved_glyphs: Vec::new(),
        machines,
    };
    let line = gallery::label_lines(&taken(vec![2, 3, 4, 6, 7, 8])).join(" ");
    assert!(line.contains("6 tiles"), "the label must say the picture is six frames: {line}");
    assert!(
        line.contains("--interpreter 2/3/4/6/7/8 --colour machine"),
        "…and carry the flags that get any one of them back, since the opt-in is the half a reader \
         would not guess (SQ-1154): {line}"
    );
    let plain = gallery::label_lines(&taken(Vec::new())).join(" ");
    assert!(!plain.contains("tiles"), "an ordinary frame says nothing about tiles: {plain}");
}
