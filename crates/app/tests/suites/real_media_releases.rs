//! SQ-0760: the release each **medium** actually carries, pinned.
//!
//! An Infocom Amiga release floppy is not the same story as the bare story file
//! sitting beside it in `stories/` — it is a *different build of the game*.
//! `Journey - The Quest Begins.adf` is release 30, serial 890322; `journey.z6`
//! is release 83, serial 890706, and the two do not behave alike: r83 narrates
//! through window 0, r30 through window 2. That difference was the whole of
//! SQ-0755's reopened defect, and it cost SQ-0747 five investigation passes,
//! three of them sweeping tens of thousands of configurations against a release
//! the user does not play.
//!
//! `InterpreterProfile::resolve` reads the MEDIUM, so "under the Amiga profile"
//! names a machine *and* — when the fixture is a disk image — a different build.
//! Until this file there was **no committed test that drove a floppy at all**:
//! the SQ-0755 corpus measured both media for Zork Zero, Shogun and Arthur and
//! found them to agree, but an agreement measured once and never pinned is
//! exactly how this slipped through the first time.
//!
//! So: one table, [`MEDIA`], naming every medium in `stories/` that has a
//! floppy counterpart together with the version, release and serial it must
//! load. Every case walks it, guards on that identity FIRST, and prefixes every
//! assertion message with [`ctx`] — the file, the release and the serial it
//! loaded — so a failure here can never again be attributed to the wrong build.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! per missing file, exactly like the other real-game smokes. The `ran > 0`
//! guards catch a LOCAL run whose filenames drifted; they are gated on
//! [`any_real_media_present`] so they cannot fire where the fixtures legitimately
//! cannot exist.

use std::path::PathBuf;
use std::sync::Mutex;

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global (an Amiga medium loads the
/// Amiga palette), so no two cases here may boot at once.
static PALETTE: Mutex<()> = Mutex::new(());

// ── The table ────────────────────────────────────────────────────────────────

/// One story image as it exists on one medium.
#[derive(Clone, Copy)]
struct Medium {
    /// The game, so a pair reads as a pair.
    title: &'static str,
    /// Filename under `stories/`.
    file: &'static str,
    /// An Amiga release floppy (`.adf`) rather than a bare story file.
    floppy: bool,
    /// Z-machine version, header $00.
    version: u8,
    /// Release number, header $02.
    release: u16,
    /// Serial, header $12..$18.
    serial: &'static str,
}

/// Every medium in `stories/` for a title that ships on an Amiga floppy, with
/// the build each one carries. **Measured**, 2026-08-10, by mounting each file
/// through `app::hints::load_mounted_story` and reading its header.
///
/// Read the pairs: four of the five titles that ship both ways ship *different
/// builds*, and the four v3/v5 pairs ship the same one.
const MEDIA: &[Medium] = &[
    // Journey — the pair that started this. Different builds, and they differ
    // in which window they narrate through (see `V6_FRAMES`).
    Medium { title: "Journey", file: "Journey - The Quest Begins.adf", floppy: true, version: 6, release: 30, serial: "890322" },
    Medium { title: "Journey", file: "journey-r83-s890706.z6", floppy: false, version: 6, release: 83, serial: "890706" },
    // Zork Zero — different builds, and the floppy lays its story window out at
    // a different place and size.
    Medium { title: "Zork Zero", file: "Zork Zero - The Revenge of Megaboz.adf", floppy: true, version: 6, release: 366, serial: "890323" },
    Medium { title: "Zork Zero", file: "zork0-r393-s890714.z6", floppy: false, version: 6, release: 393, serial: "890714" },
    // Shogun — different builds that (so far) lay out identically.
    Medium { title: "Shogun", file: "James Clavell's Shogun.adf", floppy: true, version: 6, release: 295, serial: "890321" },
    Medium { title: "Shogun", file: "shogun-r322-s890706.z6", floppy: false, version: 6, release: 322, serial: "890706" },
    // Arthur — different builds that (so far) lay out identically.
    Medium { title: "Arthur", file: "Arthur - The Quest for Excalibur.adf", floppy: true, version: 6, release: 54, serial: "890606" },
    Medium { title: "Arthur", file: "arthur-r74-s890714.z6", floppy: false, version: 6, release: 74, serial: "890714" },
    // Beyond Zork — the SAME build on both media.
    Medium { title: "Beyond Zork", file: "Beyond Zork - The Coconut of Quendor.adf", floppy: true, version: 5, release: 57, serial: "871221" },
    Medium { title: "Beyond Zork", file: "beyondzork-r57-s871221.z5", floppy: false, version: 5, release: 57, serial: "871221" },
    // The Zork trilogy — the same build on both media.
    Medium { title: "Zork I", file: "Zork I - The Great Underground Empire.adf", floppy: true, version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork I", file: "zork1-r88-s840726.z3", floppy: false, version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork II", file: "Zork II - The Wizard of Frobozz.adf", floppy: true, version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork II", file: "zork2-r48-s840904.z3", floppy: false, version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork III", file: "Zork III - The Dungeon Master.adf", floppy: true, version: 3, release: 17, serial: "840727" },
    Medium { title: "Zork III", file: "zork3-r17-s840727.z3", floppy: false, version: 3, release: 17, serial: "840727" },
    // Zork: The Undiscovered Underground ships on a floppy only.
    Medium { title: "ZTUU", file: "Zork - The Undiscovered Underground.adf", floppy: true, version: 5, release: 16, serial: "970828" },
];

/// The pairs, and whether the two media carry the SAME build. Every `false`
/// here is a title where a finding measured off one medium says nothing about
/// the other.
const PAIRS: &[(&str, bool)] = &[
    ("Journey", false),
    ("Zork Zero", false),
    ("Shogun", false),
    ("Arthur", false),
    ("Beyond Zork", true),
    ("Zork I", true),
    ("Zork II", true),
    ("Zork III", true),
];

/// The v6 frame each medium produces after `turns` blank turns from boot, under
/// the profile its own medium resolves to: which window its prose is streaming
/// into, and that window's box as the game set it (x, y, w, h — native pixels,
/// 1-based like the Z-machine's own coordinates).
///
/// The prose window is the load-bearing number: it is the fact SQ-0755 turned
/// on, and the one place where Journey's two releases visibly disagree.
struct V6Frame {
    file: &'static str,
    turns: usize,
    prose_window: usize,
    box_px: (u16, u16, u16, u16),
}

const V6_FRAMES: &[V6Frame] = &[
    // r30 narrates through window 2 — window 0 is a leftover strip it never
    // prints into. r83 narrates through window 0 and has no window 2 at all.
    V6Frame { file: "Journey - The Quest Begins.adf", turns: 40, prose_window: 2, box_px: (265, 17, 368, 272) },
    V6Frame { file: "journey-r83-s890706.z6", turns: 40, prose_window: 0, box_px: (241, 1, 392, 304) },
    // Zork Zero's two builds place the story window two pixels apart and size it
    // four pixels differently — small, and enough to make a geometry finding off
    // one medium untrue of the other.
    V6Frame { file: "Zork Zero - The Revenge of Megaboz.adf", turns: 12, prose_window: 0, box_px: (89, 81, 464, 320) },
    V6Frame { file: "zork0-r393-s890714.z6", turns: 12, prose_window: 0, box_px: (87, 79, 468, 320) },
    // Shogun and Arthur agree across their two builds. Pinned so that stays a
    // measured fact rather than an assumption.
    V6Frame { file: "James Clavell's Shogun.adf", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "shogun-r322-s890706.z6", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "Arthur - The Quest for Excalibur.adf", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
    V6Frame { file: "arthur-r74-s890714.z6", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
];

/// Media whose game narrates its own release, and the words it uses. The
/// strongest possible statement of which build is running: the STORY says so,
/// not the header we read to pick the fixture.
const NARRATED: &[(&str, &[&str])] = &[
    ("Zork Zero - The Revenge of Megaboz.adf", &["Release 366 / Serial number 890323"]),
    ("zork0-r393-s890714.z6", &["Release 393 / Pix 393 / Serial number 890714"]),
    ("James Clavell's Shogun.adf", &["Release 295 / Serial number 890321"]),
    ("shogun-r322-s890706.z6", &["Release 322 / Pix 322 / Serial number 890706"]),
    ("Zork I - The Great Underground Empire.adf", &["Revision 88 / Serial number 840726"]),
    ("zork1-r88-s840726.z3", &["Revision 88 / Serial number 840726"]),
    ("Zork II - The Wizard of Frobozz.adf", &["Version 48 / Serial number 840904"]),
    ("Zork III - The Dungeon Master.adf", &["Release 17 / Serial number 840727"]),
    // …and ZTUU prints the interpreter it was told it is running on, which off a
    // floppy is the Amiga's 4 (ZMSD §11.1.3). One line proving both halves of
    // "the medium picks the machine".
    ("Zork - The Undiscovered Underground.adf", &["Release 16 / Serial number 970828", "Interpreter 4 "]),
];

/// The resource Blorbs shipped beside these stories, and whether each declares
/// a `Reso` standard window. None of them carries an executable chunk, so a
/// Blorb is never a third build — it is artwork for whichever story file you
/// point at, and the release stays the one you opened.
const RESOURCE_BLORBS: &[(&str, bool)] = &[
    ("Journey.blb", true),
    ("Zork0.blb", true),
    ("Shogun.blb", true),
    ("Arthur.blb", true),
    // Beyond Zork is a v5 story: its Blorb carries sound, no scalable pictures
    // and so no standard window at all.
    ("beyondzork.blb", false),
];

/// Pane widths the render case sweeps. 80 is the game's own column count, where
/// the cell path's 1:1 chrome and the proportional placement coincide and a
/// defect can hide (SQ-0742's lesson); the other two do not coincide.
const WIDTHS: [u16; 3] = [80, 100, 138];

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Is ANY of this suite's media present? `stories/` is gitignored, so CI and a
/// fresh worktree legitimately have none and every case here skips. This gates
/// the `ran > 0` guards so they only ever catch a drifted filename on a machine
/// that really does hold the media.
fn any_real_media_present() -> bool {
    MEDIA.iter().any(|m| stories_dir().join(m.file).exists())
}

/// How every assertion in this file names its subject. A failure that does not
/// say which release it loaded is the single most expensive kind of failure this
/// project has had.
fn ctx(m: &Medium) -> String {
    format!(
        "{} [{} — {}, release {}, serial {}]",
        m.file,
        m.title,
        if m.floppy { "Amiga floppy" } else { "story file" },
        m.release,
        m.serial
    )
}

/// The story bytes off `m`'s medium, or `None` (with a SKIP note) when the
/// gitignored fixture is not there.
fn story_bytes(m: &Medium) -> Option<Vec<u8>> {
    let path = stories_dir().join(m.file);
    match app::hints::load_mounted_story(&path) {
        Ok((loaded, mounted)) => {
            assert_eq!(
                mounted,
                m.floppy,
                "{}: the mount reports the medium, and it disagrees with the table",
                ctx(m)
            );
            Some(loaded.bytes().to_vec())
        }
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            None
        }
    }
}

fn header_release(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[2], bytes[3]])
}

fn header_serial(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned()
}

/// The guard every case runs before it measures anything: the bytes in hand are
/// the build the table names. Nothing measured after this can be attributed to
/// the wrong release.
fn assert_is_the_pinned_release(m: &Medium, bytes: &[u8]) {
    assert_eq!(bytes[0], m.version, "{}: Z-machine version", ctx(m));
    assert_eq!(
        header_release(bytes),
        m.release,
        "{}: loaded release {} — this medium carries a DIFFERENT build than the table says",
        ctx(m),
        header_release(bytes)
    );
    assert_eq!(
        header_serial(bytes),
        m.serial,
        "{}: loaded serial {}",
        ctx(m),
        header_serial(bytes)
    );
}

/// Boot `m` exactly as `startup.rs` does — the profile comes from the medium,
/// the artwork from whatever that medium supplies — after checking the build.
fn boot(m: &Medium, honor_game_colours: bool) -> Option<GameSession> {
    let bytes = story_bytes(m)?;
    assert_is_the_pinned_release(m, &bytes);
    let path = stories_dir().join(m.file);
    let profile = InterpreterProfile::resolve(&path, None, None);
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        honor_game_colours,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(m)));
    assert!(!s.quit, "{}: quit during boot", ctx(m));
    assert!(s.machine.fault_trace.is_none(), "{}: faulted during boot", ctx(m));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive `turns` turns of "just keep going": Enter at a keypress, an empty line
/// at a line prompt. Deterministic, which is what a pinned frame needs.
fn drive(s: &mut GameSession, turns: usize, who: &str) {
    for t in 0..turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        assert!(!s.quit, "{who}: quit at turn {t} of {turns}");
        assert!(s.machine.fault_trace.is_none(), "{who}: faulted at turn {t} of {turns}");
    }
}

/// The window this session's prose is currently streaming into, and its box as
/// the game set it. `None` for a non-v6 story, or when nothing has been printed.
fn prose_window(s: &GameSession) -> Option<(usize, (u16, u16, u16, u16))> {
    let v6 = s.machine.screen.v6.as_ref()?;
    v6.windows
        .iter()
        .enumerate()
        .find(|(_, w)| !w.streamed.is_empty())
        .map(|(i, w)| (i, (w.x_coord, w.y_coord, w.x_size, w.y_size)))
}

/// Which render path drew this frame, as `/dump-windows` reports it.
fn path_label(state: &app::state::AppState) -> String {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label.starts_with("path:"))
        .map(|e| e.label.clone())
        .unwrap_or_else(|| "<no path recorded>".into())
}

/// The native-pixel rect the renderer treated as the STORY viewport this frame.
fn viewport_px(state: &app::state::AppState) -> Option<(u16, u16, u16, u16)> {
    state.v6_cell_map.borrow().iter().find(|e| e.label == "viewport").map(|e| e.native)
}

// ── Identity ─────────────────────────────────────────────────────────────────

/// The table itself, checked against the media. This is the finding SQ-0760
/// exists to record: what each medium in `stories/` actually contains.
#[test]
fn every_medium_loads_the_release_it_is_pinned_at() {
    let mut ran = 0;
    for m in MEDIA {
        let Some(bytes) = story_bytes(m) else { continue };
        assert_is_the_pinned_release(m, &bytes);
        ran += 1;
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but none were read");
}

/// The headline. For every title shipping on both media, the two releases are
/// pinned against **each other** — so a future agent cannot quietly measure one
/// and report it as the other, and an upgrade of either fixture fails here
/// rather than silently rebasing an investigation.
#[test]
fn a_floppy_and_the_story_file_beside_it_are_pinned_against_each_other() {
    let mut ran = 0;
    for (title, same_build) in PAIRS {
        let pair: Vec<&Medium> = MEDIA.iter().filter(|m| m.title == *title).collect();
        assert_eq!(pair.len(), 2, "{title}: PAIRS names a title that is not a pair in MEDIA");
        let (floppy, file) = (pair[0], pair[1]);
        assert!(floppy.floppy && !file.floppy, "{title}: MEDIA must list the floppy first");
        let (Some(fb), Some(sb)) = (story_bytes(floppy), story_bytes(file)) else { continue };
        assert_is_the_pinned_release(floppy, &fb);
        assert_is_the_pinned_release(file, &sb);
        ran += 1;

        let same = header_release(&fb) == header_release(&sb)
            && header_serial(&fb) == header_serial(&sb);
        assert_eq!(
            same, *same_build,
            "{title}: the floppy is release {} serial {} and the story file is release {} serial {} \
             — {}. A finding measured on one medium describes the other ONLY when these agree.",
            header_release(&fb),
            header_serial(&fb),
            header_release(&sb),
            header_serial(&sb),
            if *same_build { "they were pinned as the same build" } else { "they were pinned as different builds" },
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no pair was compared");
}

/// A `.blb` beside a story is artwork, never a third build — so "which release"
/// is decided entirely by the file you open.
#[test]
fn a_resource_blorb_beside_a_story_carries_no_release_of_its_own() {
    let mut ran = 0;
    for (name, declares_std_window) in RESOURCE_BLORBS {
        let path = stories_dir().join(name);
        if !path.exists() {
            eprintln!("SKIP: gitignored resource Blorb missing at {}", path.display());
            continue;
        }
        ran += 1;
        assert!(
            app::hints::load_story(&path).is_err(),
            "{name}: a resource Blorb must hold no executable, or it would be a third build"
        );
        let std_window =
            PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b)).std_window();
        assert_eq!(
            std_window.is_some(),
            *declares_std_window,
            "{name}: standard window is {std_window:?}"
        );
        if *declares_std_window {
            // …and it is the SAME 320×200 the Amiga profile supplies for a
            // native archive, which has no field to declare one (SQ-0736).
            assert_eq!(
                std_window,
                InterpreterProfile::Amiga.std_window(),
                "{name}: an Infocom Blorb's Reso declares the machine's own standard window"
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no Blorb was read");
}

/// The medium picks the machine, on the real files rather than a synthetic disk
/// (`interpreter_profile.rs` covers the synthetic side). This is the rule that
/// makes "the Amiga build" ambiguous in the first place, so it is pinned on the
/// same fixtures every other case here uses.
#[test]
fn the_medium_each_release_ships_on_picks_the_interpreter_profile() {
    let mut ran = 0;
    for m in MEDIA {
        let path = stories_dir().join(m.file);
        if !path.exists() {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            continue;
        }
        ran += 1;
        let expected =
            if m.floppy { InterpreterProfile::Amiga } else { InterpreterProfile::IbmPc };
        assert_eq!(
            InterpreterProfile::resolve(&path, None, None),
            expected,
            "{}: the medium decides the profile",
            ctx(m)
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no profile was resolved");
}

// ── What each medium actually does ───────────────────────────────────────────

/// Every v6 medium boots and reaches a stable frame, and that frame is pinned
/// per medium — in BOTH `honor_game_colours` modes, per the project's colour
/// convention (`true` is the shipped default).
///
/// Journey is the case with teeth: the floppy's r30 streams its prose into
/// window 2 and the story file's r83 into window 0.
#[test]
fn each_v6_medium_narrates_through_the_window_its_own_release_uses() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let found = prose_window(&s)
                .unwrap_or_else(|| panic!("{who}: nothing was streamed after {} turns", f.turns));
            assert_eq!(
                found.0, f.prose_window,
                "{who}: this release narrates through window {}, not {} — a rule aimed at the \
                 wrong window is right by luck on one release and wrong on the other (SQ-0755)",
                found.0, f.prose_window
            );
            assert_eq!(
                found.1, f.box_px,
                "{who}: window {} is at {:?}, pinned at {:?} (native px, 1-based)",
                f.prose_window, found.1, f.box_px
            );
            ran += 1;
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no v6 frame was measured");
}

/// The games that print their own release print the one that was loaded — the
/// story's word, not ours. Off the ZTUU floppy that line also names interpreter
/// 4, so the medium's profile is visible in the game's own output.
#[test]
fn a_game_that_names_its_release_names_the_one_the_medium_carries() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for (file, wanted) in NARRATED {
        let m = MEDIA.iter().find(|m| m.file == *file).expect("NARRATED names a medium in MEDIA");
        let Some(mut s) = boot(m, true) else { continue };
        ran += 1;
        // Past whatever intro the release opens with, to its first line prompt.
        let mut said = String::new();
        for _ in 0..24 {
            match s.pending_input() {
                InputKind::Line => {
                    said = s.submit("version").transcript;
                    break;
                }
                InputKind::Char => {
                    let _ = s.submit_char(13);
                }
                InputKind::Event => {
                    let _ = s.submit("");
                }
            }
        }
        let flat = said.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in *wanted {
            assert!(
                flat.contains(want),
                "{}: the game answered VERSION with {flat:?}, which does not say {want:?}",
                ctx(m)
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no game was asked");
}

/// …and the floppy build reaches the RENDERER, which is where its differently
/// placed windows could be dropped on the floor. Nothing committed had ever
/// rendered a frame produced by a disk image at all.
///
/// Two things per medium, at three pane widths and in both colour modes: the
/// frame takes the ordinary gameplay path (a release whose chrome trips the
/// painted-menu gate would be routed elsewhere — SQ-0742), and the story
/// viewport the renderer picks out is the box of the window THIS release
/// narrates through. On the Journey floppy that is window 2; on the story file
/// beside it, window 0.
#[test]
fn each_v6_medium_renders_the_story_viewport_its_own_release_lays_out() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let model = s.screen();
            // The window table is 1-based (ZMSD §8.8.1); the renderer works in
            // 0-based screen pixels, so the story viewport is that box shifted.
            let want = (f.box_px.0 - 1, f.box_px.1 - 1, f.box_px.2, f.box_px.3);
            for cols in WIDTHS {
                let state = render_hybrid(&model, honor, cols, 51);
                assert_eq!(
                    path_label(&state),
                    "path:hybrid-ring",
                    "{who} w={cols}: an ordinary gameplay frame must take the chrome ring"
                );
                assert_eq!(
                    viewport_px(&state),
                    Some(want),
                    "{who} w={cols}: the story viewport must be window {}'s box",
                    f.prose_window
                );
                ran += 1;
            }
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no frame was rendered");
}

/// A hybrid render at a plausible kitty cell (8×18); `Picker::halfblocks()`
/// reports a 1×2 cell, a layout regime that reproduces nothing (SQ-0548).
#[allow(deprecated)]
fn render_hybrid(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
    rows: u16,
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    state
}
