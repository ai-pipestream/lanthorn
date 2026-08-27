//! SQ-1032: the third v6 render mode — `extended`.
//!
//! `raster` builds the game's own screen, letterboxes it into the pane, and spends
//! every surplus device pixel on MAGNIFICATION. `extended` builds the same composite,
//! pins the magnification to a whole number of device pixels per NATIVE pixel, and
//! spends the surplus on CONTENT instead: the canvas grows downward and the extra
//! height becomes whole text rows of prose in the game's own bitmap typeface.
//!
//! **The game is told nothing.** `v6_screen_px` is fixed at construction and no
//! resize path updates it, so the game lays its windows out on exactly the screen it
//! always had — which is now the top of a taller canvas. Nothing here fabricates a
//! screen size no machine had (the SQ-0901 trap), and no per-title layout can break
//! from it.
//!
//! What this suite pins, in the order the cases run:
//!
//!   1. the arithmetic — a whole magnification, and a surplus measured in whole text
//!      rows of the machine's own cell;
//!   2. the game's screen is untouched WIDTHWISE and the canvas grows only downward;
//!   3. the extra rows reach the prose box, so the story viewport really is bigger
//!      (which is also the whole of the `[MORE]` improvement — the pager pages on
//!      *added rows > viewport* and the viewport is what the renderer reports);
//!   4. the flanks tile down the extension rather than leaving bare page beside it;
//!   5. and every frame with nowhere to put the rows declines and is **byte-identical
//!      to `raster`** — which is the regression guard that matters, because `extended`
//!      shares the whole composite with `raster` and a change that reached the shared
//!      code would show up here first.
//!
//! Specimens (release and turn count are part of the fixture — CLAUDE.md):
//!
//! ```text
//!   fixture                                release  turns  role
//!   zork0-r393-s890714.z6                    393       6    art reaches the screen bottom
//!   arthur-r74-s890714.z6                     74      12    poles stop short of it
//!   journey-r83-s890706.z6                    83      40    command menu UNDER the story
//! ```
//!
//! Journey is not decoration: it is the frame that must NOT extend. Hybrid meets it by
//! bottom-anchoring the command strip and filling between (`BottomPlan::Menu`), and
//! this mode cannot — the composite is one image built in the game's own coordinates,
//! so relocating the game's own chrome inside it is a composition change rather than a
//! layout one. The flank extension has declined the identical frame since SQ-0819 for
//! the identical reason. So Journey in `extended` is Journey in `raster`, to the byte,
//! and that is asserted rather than described.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// One fixture: file, the key answered to a character read while tapping in, how many
/// taps reach the frame this case is about, the release it must be holding, and
/// whether this frame has anywhere to spend an extension.
struct Specimen {
    file: &'static str,
    keys: u8,
    taps: usize,
    release: u16,
    /// Does this frame EXTEND? False is the Journey case — a text-only command strip
    /// below the story window, which the composite cannot bottom-anchor.
    extends: bool,
}

const CORPUS: &[Specimen] = &[
    Specimen { file: "zork0-r393-s890714.z6", keys: 13, taps: 6, release: 393, extends: true },
    Specimen { file: "arthur-r74-s890714.z6", keys: b'n', taps: 12, release: 74, extends: true },
    Specimen { file: "journey-r83-s890706.z6", keys: 13, taps: 40, release: 83, extends: false },
];

/// A pane with real surplus height at the 8x18 kitty cell: 800x900 device pixels
/// against a 640x400 screen, so the whole magnification is 1 and 500 native rows are
/// left over — 31 text rows at an 8x16 cell.
const TALL: (u16, u16) = (100, 50);
/// A pane with LESS than one text row of surplus at the same whole magnification:
/// 640x414 device pixels, so the extension is zero rows and the frame is the game's
/// screen exactly. The control the brief asks for — nothing extends, nothing changes.
const SNUG: (u16, u16) = (80, 23);
/// A pane wide enough for a whole magnification of TWO — 1280x1080 device pixels, so
/// the screen doubles and 140 native rows are left over, 8 text rows of them. Every
/// other pane in this suite pins `s = 1`, and a mode whose whole point is a whole
/// magnification should be measured on more than one of them.
const WIDE: (u16, u16) = (160, 60);
const CELL: (u16, u16) = (8, 18);

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the MOUNT
/// returned, and the screen size through the whole `picts.std_window() →
/// native_std_window → profile.std_window()` chain with `art_scale` beside it, all of
/// it inside one [`app::machine_boot::MachineBoot`] so a fact cannot be omitted — then
/// tap in to the frame. `None` (with a SKIP note) when the gitignored fixture is absent.
///
/// Skipping `native_std_window` is what booted two 560x384 presses at 640x400 and
/// fabricated a frame a whole quest was fixed against (CLAUDE.md, SQ-0901/SQ-1021), so
/// the profile, release, screen and cell this harness booted are all PRINTED.
fn boot(s: &Specimen) -> Option<(GameSession, (u32, u32))> {
    let path = stories_dir().join(s.file);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    app::v6_set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(
        release, s.release,
        "{}: a disk image is a different BUILD, not the same story on other media — this case is \
         pinned to release {}",
        s.file, s.release
    );
    let boot = app::machine_boot::MachineBoot::resolve(
        profile,
        &picts,
        None,
        profile.interpreter_number(),
        profile.default_colours(),
        app::native_font::FaceSet::none(),
    );
    let art_scale = boot.art_scale;
    eprintln!(
        "{}: booted as {profile:?} off {medium:?} · release {release} · screen {:?} · \
         art_scale {art_scale:?} · v6 cell {:?}",
        s.file,
        boot.screen_px,
        app::state::AppState::default().v6_text.cell(),
    );
    let mut session = GameSession::new_for_machine(bytes, true, false, false, dims, None, None, &boot)
        .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", s.file));
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..s.taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(s.keys).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some((session, art_scale.unwrap_or((2, 2))))
}

/// A v6 app state at a real kitty cell, in `mode`, with the art scale the mount
/// resolved. Only the MODE differs between the two states any case here compares.
#[allow(deprecated)]
fn state_for(
    mode: app::config::V6RenderMode,
    transcript: &str,
    art_scale: (u32, u32),
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(CELL.0, CELL.1));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = mode;
    state.config.honor_game_colours = true;
    state.v6_art_scale = art_scale;
    for line in transcript.lines() {
        state.push_transcript(line);
    }
    state
}

/// The pane in device pixels at [`CELL`] — the unit `RasterFrame::extended` is stated in.
fn pane_dev(pane: (u16, u16)) -> (u32, u32) {
    (u32::from(pane.0) * u32::from(CELL.0), u32::from(pane.1) * u32::from(CELL.1))
}

/// The two composites one specimen builds at `pane`: the game's own screen
/// (`raster`), and the frame `extended` asks for. Both come out of the same
/// `build_v6_raster_frame`, so a difference between them is the extension and
/// nothing else.
type Pair = (
    (image::RgbaImage, Option<app::render::screen::RasterMetrics>, v6::RasterFrame),
    (image::RgbaImage, Option<app::render::screen::RasterMetrics>, v6::RasterFrame),
);

fn pair(session: &mut GameSession, art_scale: (u32, u32), pane: (u16, u16)) -> Pair {
    let transcript = session.take_transcript();
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("a v6 frame has a Layered root") };
    let plain = state_for(app::config::V6RenderMode::Raster, &transcript, art_scale);
    let ext = state_for(app::config::V6RenderMode::Extended, &transcript, art_scale);
    let native = v6::native_extent(items, &plain.v6_text);
    let layout = v6::classify_windows(items, plain.v6_text.cell());
    let cell = plain.v6_text.cell();
    let want = v6::RasterFrame::extended(native, pane_dev(pane), cell, Some(2.0));
    (
        app::render::screen::build_v6_raster_frame(&layout, v6::RasterFrame::native(native), &plain),
        app::render::screen::build_v6_raster_frame(&layout, want, &ext),
    )
}

// ── 1. The arithmetic ─────────────────────────────────────────────────────────

/// A whole magnification and a surplus measured in whole text rows of the MACHINE's
/// own cell.
///
/// Stated over both cells the corpus has — 8x16 everywhere and 7x15 on a Macintosh
/// (SQ-0917) — because "whole text rows" is a question about the cell and writing it
/// `/ 16` is the shape of SQ-1020.
#[test]
fn the_extension_is_a_whole_magnification_and_whole_text_rows() {
    for cell in [zvm::screen::V6Cell { w: 8, h: 16 }, zvm::screen::V6Cell { w: 7, h: 15 }] {
        let f = v6::RasterFrame::extended((640, 400), (800, 900), cell, Some(2.0));
        let s = f.lock.expect("a pane that holds the screen at 1:1 pins a magnification");
        assert_eq!(s, s.floor(), "cell {cell:?}: the magnification is a whole number");
        assert!(s >= 1.0, "cell {cell:?}: and never a minification");
        assert_eq!(
            f.extension() % u32::from(cell.h),
            0,
            "cell {cell:?}: the surplus is whole text rows of this machine's cell"
        );
        assert_eq!(f.native, (640, 400), "cell {cell:?}: the GAME's screen is never changed");
        // 900 device rows at s=1 is 900 native rows; 500 of them lie below the screen.
        assert_eq!(f.extension(), 500 / u32::from(cell.h) * u32::from(cell.h));

        // …and the rung above it. 1280x1080 doubles the screen, so the pane is 540
        // native rows and 140 of them lie below it.
        let two = v6::RasterFrame::extended((640, 400), pane_dev(WIDE), cell, Some(2.0));
        assert_eq!(two.lock, Some(2.0), "cell {cell:?}: a pane twice the screen pins 2");
        assert_eq!(two.extension(), 140 / u32::from(cell.h) * u32::from(cell.h));
    }
}

/// A pane with less than one text row of surplus extends by nothing, and a pane that
/// cannot hold the game's screen at 1:1 falls all the way back to the plain letterbox
/// — no lock, no extension, which is `raster` exactly.
#[test]
fn a_pane_with_no_surplus_is_the_plain_letterboxed_frame() {
    let cell = zvm::screen::V6Cell { w: 8, h: 16 };
    let snug = v6::RasterFrame::extended((640, 400), pane_dev(SNUG), cell, Some(2.0));
    assert_eq!(snug.extension(), 0, "640x414 leaves 14 native rows — under one text row");
    assert_eq!(snug.canvas_h, 400);

    let small = v6::RasterFrame::extended((640, 400), (500, 300), cell, Some(2.0));
    assert_eq!(small, v6::RasterFrame::native((640, 400)), "below 1:1 there is no whole rung");
    assert_eq!(small.lock, None, "…so the composite keeps the fitted letterbox");
}

// ── 2..4. The frame the corpus actually builds ────────────────────────────────

/// On a title with surplus pane: the canvas grows DOWNWARD only, the extra rows reach
/// the prose box, and the flanks are carried into them rather than left as bare page.
///
/// FALSIFY by dropping the `th + extension` line in `build_v6_raster_frame` — the
/// canvas still grows and the flanks still tile, and the story viewport does not move,
/// which is the "taller frame, same eleven rows of prose" version of this mode.
#[test]
fn the_extension_grows_downward_and_the_prose_box_takes_it() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS.iter().filter(|s| s.extends) {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        for pane in [TALL, WIDE] {
        let ((plain, pm, pf), (extended, em, ef)) = pair(&mut session, art_scale, pane);
        eprintln!(
            "{}: at {pane:?} raster {}x{} → extended {}x{} (lock {:?})",
            spec.file,
            plain.width(),
            plain.height(),
            extended.width(),
            extended.height(),
            ef.lock,
        );

        // Non-vacuity: this case is about a frame that HAS a prose box and DID extend.
        assert!(pm.is_some(), "{}: the raster frame has a story box to compare against", spec.file);
        assert!(ef.extension() > 0, "{}: this frame is supposed to extend", spec.file);
        assert_eq!(pf.extension(), 0, "{}: and the raster frame is supposed not to", spec.file);

        // (2) Downward only. The width is the game's screen — that is the whole of
        // "fixed raster width" — and the height is the game's screen plus the extension.
        assert_eq!(extended.width(), plain.width(), "{}: the frame never grows sideways", spec.file);
        assert_eq!(
            extended.height(),
            plain.height() + ef.extension(),
            "{}: and grows by exactly the extension",
            spec.file
        );

        // (3) The rows reach the PROSE, which is the point of having them. The pager
        // pages on added-rows > viewport (`pager.rs`), and this is that viewport.
        let (pv, ev) = (pm.expect("checked").viewport_rows, em.expect("extended keeps a story box").viewport_rows);
        assert!(
            ev > pv,
            "{}: the extension must reach the story viewport — raster {pv} rows, extended {ev}",
            spec.file
        );
        assert_eq!(
            u32::from(ev - pv),
            ef.extension() / u32::from(app::state::AppState::default().v6_text.cell().h),
            "{}: and it is exactly the whole text rows the frame added",
            spec.file
        );

        // (4) The flanks were carried down. Every pixel below the game's screen is
        // opaque (the composite is self-contained — SQ-0510), and the outermost column
        // of the extension is not merely the story page: the border art tiled into it.
        let below = u32::from(ef.native.1);
        assert!(below < extended.height(), "{}: there is an extension to look at", spec.file);
        assert!(
            (below..extended.height()).all(|y| (0..extended.width()).all(|x| extended.get_pixel(x, y)[3] == 255)),
            "{}: the extension ships opaque, like every other pixel of the composite",
            spec.file
        );
        // The BORDER, specifically. The middle of the extension's last row is the
        // story page — nothing is drawn there but prose — and each outer eighth of
        // the frame must carry something that is not it, which is the side artwork
        // tiled down by `flank_source` at the taller target. Stated as an eighth
        // rather than as column 0 because a flank's outermost columns are not
        // necessarily painted: Zork Zero's pillars start inboard of the screen edge.
        let page = extended.get_pixel(extended.width() / 2, extended.height() - 1).0;
        let eighth = (extended.width() / 8).max(1);
        for (label, cols) in [
            ("left", 0..eighth),
            ("right", extended.width() - eighth..extended.width()),
        ] {
            let inked = (below..extended.height())
                .flat_map(|y| cols.clone().map(move |x| (x, y)))
                .any(|(x, y)| extended.get_pixel(x, y).0 != page);
            assert!(
                inked,
                "{}: the {label} border must reach the bottom of the extension rather than \
                 stopping at the game's own screen (page {page:?})",
                spec.file
            );
        }
        seen += 1;
        }
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

// ── 5. The regression guard ──────────────────────────────────────────────────

/// A frame with nowhere to put the extra rows declines it, and the composite it
/// builds is **byte-identical** to the one `raster` builds.
///
/// Journey is the frame: a text-only command strip below the story window, which
/// hybrid bottom-anchors and this mode cannot. The SNUG pane is the other half — every
/// title, including the two that do extend, at a pane with no surplus to spend.
///
/// This is the guard that matters. `extended` shares the entire composite with
/// `raster`, so a change that leaked out of the extension's own branches lands here.
#[test]
fn a_frame_that_declines_the_extension_is_byte_identical_to_raster() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS {
        any_present |= stories_dir().join(spec.file).exists();
        // Journey declines at any pane; the others decline at a pane with no surplus.
        let panes: &[(u16, u16)] = if spec.extends { &[SNUG] } else { &[SNUG, TALL] };
        for &pane in panes {
            let Some((mut session, art_scale)) = boot(spec) else { continue };
            let ((plain, _, pf), (extended, _, ef)) = pair(&mut session, art_scale, pane);
            assert_eq!(
                ef.extension(),
                0,
                "{} at {pane:?}: this frame must decline the extension",
                spec.file
            );
            assert_eq!(ef, pf, "{} at {pane:?}: a declined frame IS the raster frame", spec.file);
            assert_eq!(
                extended.dimensions(),
                plain.dimensions(),
                "{} at {pane:?}: same canvas",
                spec.file
            );
            assert!(
                extended.as_raw() == plain.as_raw(),
                "{} at {pane:?}: a declined extension must not move one byte of the raster \
                 composite",
                spec.file
            );
            seen += 1;
        }
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}

/// The whole-pane render agrees with the canvas: in `extended` the story pane reports
/// more viewport rows than in `raster`, through the REAL render entry rather than the
/// canvas builder — so the mode is wired to the frame path and not only to the helper.
#[test]
fn the_render_path_reports_the_larger_viewport() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any_present = false;
    for spec in CORPUS.iter().filter(|s| s.extends) {
        any_present |= stories_dir().join(spec.file).exists();
        let Some((mut session, art_scale)) = boot(spec) else { continue };
        let transcript = session.take_transcript();
        let model = session.screen();
        let area = Rect::new(0, 0, TALL.0, TALL.1);
        let rows = |mode| {
            let state = state_for(mode, &transcript, art_scale);
            let mut buf = Buffer::empty(area);
            app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf)
                .viewport_rows
        };
        let plain = rows(app::config::V6RenderMode::Raster);
        let ext = rows(app::config::V6RenderMode::Extended);
        eprintln!("{}: raster {plain} viewport rows → extended {ext}", spec.file);
        assert!(plain > 0, "{}: the raster path reports a story viewport at all", spec.file);
        assert!(ext > plain, "{}: extended must report more of one ({ext} vs {plain})", spec.file);
        seen += 1;
    }
    if any_present {
        assert!(seen > 0, "a present fixture must have been measured, not skipped");
    }
}
