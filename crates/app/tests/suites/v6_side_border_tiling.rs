//! SQ-0698 / SQ-0781 — the v6 side border art is **tiled**, not stretched, and
//! the frame agrees at its corners.
//!
//! Three titles frame their story window with side artwork authored for a
//! 320x200 screen. Until this suite existed, babelmap made up the difference by
//! stretching the flank band vertically (SQ-0511) — measured here at 2.2x the
//! horizontal factor on Zork Zero and 3.0x on Shogun at a 117x64 terminal — and
//! Arthur's flank was simply CLIPPED at the row his poles stop, leaving the
//! frame open down the lower half of the pane.
//!
//! What is asserted, in the order the requirements were set:
//!
//! 1. **Every side flank is TILED.** The render's own band log says which draw
//!    each band took, so this reads the render's report rather than
//!    re-implementing the pipeline.
//! 2. **The frame reaches the bottom.** The pane's own cells, at the flank's
//!    columns, on the band's last row.
//! 3. **The corners agree.** The invariant a uniform stretch gives Bocfel for
//!    free (it composes the whole frame in native pixels and scales the canvas
//!    once): the side art and the top plate must land at the same horizontal
//!    factor at every pane width, and no band may be anisotropic. Asserted as a
//!    RELATION between the bands, never as a particular factor — the factor is
//!    an implementation detail, the agreement is the requirement.
//! 4. **Nothing else moved.** No other v6 title in the corpus draws a tiled
//!    band at all.
//! 5. **Every RENDITION tiles cleanly** (SQ-0799). Since SQ-0790 the player
//!    picks the picture archive, and Zork Zero's MCGA, EGA and CGA plates do not
//!    agree on where its pillars begin — so a repeat unit pinned to one of them
//!    seams on the other two.
//! 6. **Every rendition is recognised as its own TITLE** (SQ-0802). Shogun's DOS
//!    art reaches the native screen bottom where its Amiga art stops short, so a
//!    recogniser that decides on that alone hands it Zork Zero's masonry.
//! 7. **No tile join steps harder than the art does by itself** (SQ-0808). The
//!    cut landing in the plain shaft is not sufficient: Zork Zero's CGA pillar is
//!    a *lit* column, and a repeat that only translates it resets the shading.
//!
//! Fixtures are named by exact release, per CLAUDE.md: a disk image is a
//! different build, not the same story on other media. `stories/` is gitignored,
//! so every case skips vacuously when its fixture is absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::v6_border::BorderArt;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global (an Amiga medium loads the
/// Amiga palette), so no two cases here may boot at once.
static PALETTE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One border specimen: the exact build it was measured on, and the layout its
/// flanks must be recognised as.
struct Specimen {
    title: &'static str,
    file: &'static str,
    release: u16,
    serial: &'static str,
    /// Blank turns from boot to a gameplay frame with the frame on screen.
    turns: usize,
}

/// The three titles whose side art this work extends — the same three named in
/// Bocfel's `draw_border.cpp` header ("Used by Arthur, Shogun, and Zork Zero").
/// Journey is deliberately absent: its frame is glyphs, not artwork (SQ-0750),
/// and Bocfel's border file does not mention it either.
const SPECIMENS: &[Specimen] = &[
    Specimen { title: "Arthur", file: "Arthur - The Quest for Excalibur.adf", release: 54, serial: "890606", turns: 12 },
    Specimen { title: "Shogun", file: "James Clavell's Shogun.adf", release: 295, serial: "890321", turns: 12 },
    Specimen { title: "Zork Zero", file: "zork0-r393-s890714.z6", release: 393, serial: "890714", turns: 12 },
];

/// v6 stories with no side border art of these shapes. None of them may grow a
/// tiled band: this work is the three titles above and nothing else.
const UNAFFECTED: &[&str] = &[
    "advent.z6",
    "journey-r83-s890706.z6",
    "Journey - The Quest Begins.adf",
    "fmvpoker.z6",
    "mysterious01.z6",
];

/// Pane sizes swept. The last is the deliberately TALL one, where the stretch
/// factor this work removes was largest.
const PANES: &[(u16, u16)] = &[(100, 40), (115, 61), (117, 64), (120, 90)];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile comes from the MEDIUM, the
/// artwork from whatever that medium supplies — after checking the build.
fn boot(file: &str, release: Option<(u16, &str)>) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    if let Some((rel, serial)) = release {
        assert_eq!(
            u16::from_be_bytes([bytes[2], bytes[3]]),
            rel,
            "{file}: this medium carries a DIFFERENT build than the table says"
        );
        assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    }
    // No tier-3 archive is named here — this suite resolves art through
    // `PictSource::resolve`, so the profile comes from the medium alone.
    let profile = InterpreterProfile::resolve(&path, None, None);
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    assert!(!s.quit, "{file}: quit during boot");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Boot a story against a NAMED picture archive — SQ-0734 tier 3, the door the
/// player uses to pick a rendition. The archive's own flavour selects the
/// machine (`for_art_flavour`), it supplies the standard window, and it supplies
/// the per-axis art scale, which is `(1, 2)` for a 640-wide EGA or CGA plate and
/// `(2, 2)` for a 320-wide MCGA or Amiga one (SQ-0790). Exactly what
/// `startup.rs` does. `None` when either file is absent.
fn boot_named(story: &str, archive: &str, release: (u16, &str)) -> Option<GameSession> {
    let path = stories_dir().join(story);
    let apath = stories_dir().join(archive);
    let raw = std::fs::read(&apath)
        .map_err(|_| eprintln!("SKIP: gitignored archive missing at {}", apath.display()))
        .ok()?;
    let pics = blorb::infocom_pics::InfocomPics::parse(raw).expect("a native Infocom archive parses");
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release.0, "{story}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), release.1, "{story}: serial");
    let profile = InterpreterProfile::for_art_flavour(pics.flavour());
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::from_native(pics);
    let picture_dims = picts.all_pict_dims();
    // `PictSource::std_window` answers from a Blorb's `Reso` chunk only; the
    // standard window a NAMED archive implies is `PictureOverride::std_window`,
    // and it is the same for every rendition (SQ-0790).
    let v6_screen_px = picts.std_window().or(Some(app::graphics::INFOCOM_V6_STD_WINDOW));
    let v6_art_scale = picts.art_scale();
    let mut s = GameSession::new_with_art_scale(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        v6_art_scale,
        profile.default_colours(),
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{story} + {archive}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive to a gameplay frame: Enter at a keypress, an empty line at a prompt,
/// `n` at Arthur's "Please press Y or N" restore question.
fn drive(s: &mut GameSession, turns: usize) {
    for _ in 0..turns {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        if s.quit {
            return;
        }
    }
}

/// A hybrid render at a plausible kitty cell (8x18). Halfblocks is the protocol
/// (no terminal to query), which is what lets a case assert on the pane's own
/// CELLS: the image lands in them.
#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn render_state_with(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

/// The shipped default, and this suite's primary baseline.
fn render_state() -> app::state::AppState {
    render_state_with(true)
}

/// One line of the render's band log, parsed: the band's device cell rect, how
/// it was drawn, and the size of the source it was drawn from in NATIVE pixels.
#[derive(Debug, Clone, Copy)]
struct Band {
    cells: (u16, u16),
    at: (u16, u16),
    tiled: bool,
    stretched: bool,
    src: (u32, u32),
    /// The longest contiguous run of source rows with no opaque pixel at all,
    /// and the row it starts at — a hole in a tiled flank (SQ-0698).
    blank: (u32, u32),
}

/// Parse `band WxH@(x,y) [Slot, how]: … · source WxH native px` (tiled) and
/// `band WxH@(x,y): … · native WxH@(x,y)` (plain / stretched).
fn parse_bands(log: &[String]) -> Vec<Band> {
    fn pair(s: &str, sep: char) -> Option<(u32, u32)> {
        let (a, b) = s.split_once(sep)?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    }
    log.iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("band ")?;
            let (dims, rest) = rest.split_once('@')?;
            let (w, h) = pair(dims, 'x')?;
            let (at, rest) = rest.strip_prefix('(')?.split_once(')')?;
            let (x, y) = pair(at, ',')?;
            let tiled = rest.contains("tiled]");
            let stretched = rest.contains("stretched]");
            let src = if tiled {
                let s = rest.rsplit_once("· source ")?.1;
                pair(s.split_once(" native")?.0, 'x')?
            } else {
                let s = rest.rsplit_once("· native ")?.1;
                pair(s.split('@').next()?, 'x')?
            };
            let blank = rest
                .rsplit_once("longest run ")
                .and_then(|(_, t)| t.split_once(" at "))
                .and_then(|(n, a)| Some((n.trim().parse().ok()?, a.trim().parse().ok()?)))
                .unwrap_or((0, 0));
            Some(Band { cells: (w as u16, h as u16), at: (x as u16, y as u16), tiled, stretched, src, blank })
        })
        .collect()
}

/// Render one frame and hand back the pane's cells and the bands it drew.
fn frame(session: &GameSession, pane: (u16, u16)) -> (Buffer, Vec<Band>, (u16, u16)) {
    let model = session.screen();
    let state = render_state();
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let bands = parse_bands(&state.graphics_render.borrow().band_log);
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    (buf, bands, native)
}

/// The bands that are SIDE flanks: narrower than the pane.
fn flanks(bands: &[Band], pane_w: u16) -> Vec<Band> {
    bands.iter().copied().filter(|b| b.cells.0 < pane_w).collect()
}

// ── 1. Every side flank is tiled ─────────────────────────────────────────────

#[test]
fn every_side_flank_is_tiled_and_none_is_stretched() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            let fl = flanks(&bands, w);
            assert_eq!(
                fl.len(),
                2,
                "{} [release {}] at {w}x{h}: expected a left and a right flank band, got {fl:?}",
                sp.title,
                sp.release
            );
            for b in &fl {
                assert!(
                    b.tiled && !b.stretched,
                    "{} [release {}] at {w}x{h}: flank {b:?} must be TILED, not stretched",
                    sp.title,
                    sp.release
                );
            }
            ran += 1;
        }
    }
    if stories_dir().join(SPECIMENS[0].file).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

// ── 2. The frame reaches the bottom ──────────────────────────────────────────

/// The defect as reported: *"the side columns for arthur does not stretch all
/// the way down"*.
///
/// Two statements, both of which fail with the original symptom when the
/// extension is removed. The band must run to the story viewport's own bottom —
/// SQ-0511 used to CLIP it to the artwork's lowest opaque native row instead,
/// which on Arthur is row 379 of 400 and left the frame standing open down the
/// pane's lower quarter — and the pane cell beside the bottom of the viewport
/// must carry something, rather than the theme backdrop the clip left behind.
#[test]
fn a_flank_reaches_the_story_viewports_bottom() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let model = s.screen();
            let state = render_state();
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
            let bands = parse_bands(&state.graphics_render.borrow().band_log);
            let vp = state.transcript_geom.get().expect("hybrid renders the story as a transcript").area;
            for b in flanks(&bands, w) {
                assert!(
                    b.at.1 + b.cells.1 >= vp.bottom(),
                    "{} [release {}] at {w}x{h}: the flank band {b:?} stops at row {} while the \
                     story viewport runs to {} — the frame stands open below it",
                    sp.title,
                    sp.release,
                    b.at.1 + b.cells.1,
                    vp.bottom()
                );
                let cell = &buf[(b.at.0, vp.bottom() - 1)];
                assert_ne!(
                    cell.bg,
                    ratatui::style::Color::Reset,
                    "{} [release {}] at {w}x{h}: column {} beside the bottom of the story is the \
                     theme backdrop, not border art",
                    sp.title,
                    sp.release,
                    b.at.0
                );
            }
        }
    }
}

/// SQ-0698 — **the gap between the tiled pieces**, reported on Shogun:
/// *"there is a gap between the tiled shogun side-art pieces"*.
///
/// Case 2 above only asks that the band REACH the bottom, which a flank with a
/// hole punched through its middle does — the placement rect is the full height
/// either way, and that is precisely why this defect survived the first suite.
/// The source image is the only place it shows, so the render reports the
/// longest run of rows in it that carry no opaque pixel, and where that run
/// starts; this reads those two numbers.
///
/// A run at row 0 is legitimate and is measured, not tolerated: the renderer
/// clears the rows a chrome TEXT strip covers so they draw as crisp cells
/// (SQ-0500), and a flank whose crop begins inside that band starts blank —
/// 3 rows on Shogun at 100x40, 4 and 6 on Arthur. A run starting anywhere BELOW
/// row 0 is a hole.
///
/// Measured on `James Clavell's Shogun.adf` (release 295, serial 890321) at
/// 120x90: an interior run of 64 blank native rows starting at band row 599 —
/// native 636, centred on the join at native 668 (`2·border_height − overlap`) —
/// which the uniform 1.475 scale put on the user's screen as a 94px black band
/// between the two ornate gold panels.
///
/// The cause is worth naming here because any future handler can fall into it:
/// the chrome canvas a band ships is the artwork MINUS whatever the renderer
/// draws as cells instead, and Shogun's status line is two 16px rows that the
/// top of its border sits behind. A repeat cut from that canvas carries the hole
/// twice — once at the flipped copy's foot, once at the tiled block's head — and
/// the two meet at the join. A repeat cut from the graphics-only canvas does not.
#[test]
fn a_flank_has_no_gap_between_its_tiled_pieces() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            for b in flanks(&bands, w) {
                let (run, at) = b.blank;
                assert_eq!(
                    (run > 0).then_some(at),
                    (run > 0).then_some(0),
                    "{} [release {}] at {w}x{h}: the flank source {b:?} has an INTERIOR hole — \
                     {run} row(s) with no art in them starting at row {at} of the band. A blank \
                     run at row 0 is the chrome text strip's own rows, cleared so they draw as \
                     cells; one below that is a gap between the tiled pieces",
                    sp.title,
                    sp.release
                );
            }
        }
    }
}

// ── 3. The corners agree ─────────────────────────────────────────────────────

/// **A REGRESSION TRIPWIRE, NOT A NEW PROPERTY.** This case passes on main
/// before SQ-0698 as well as after it, and that is the point: the user's
/// horizontal rule ("top artwork present → stretch the side artwork by the top
/// plate's factor, so the frame agrees at the corners") describes a result they
/// already see and prefer to the reference implementation —
///
/// > "our arthur side-art looks much cleaner than spatterlight, since our side
/// > art perfectly aligns with the header image (in spacing and width). I want
/// > to keep this look."
///
/// Bocfel gets its horizontal consistency from a single uniform stretch of the
/// whole native canvas at the end (`flush_bitmap` stretch-blits the pixmap to
/// fill the window), which guarantees agreement but gives no control over how
/// the pieces relate. babelmap places each band itself, so the agreement is a
/// property that can be broken — hence this case. SQ-0698 changed the VERTICAL
/// axis only; nothing here should ever move.
///
/// The relation asserted is between the bands actually drawn: every one of them,
/// side flanks and the full-width top plate alike, maps its native columns to
/// the pane at one horizontal factor.
#[test]
fn side_art_and_top_plate_share_one_horizontal_scale() {
    const CELL_W: f32 = 8.0;
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            assert!(!bands.is_empty(), "{} at {w}x{h}: no bands drawn", sp.title);
            let factors: Vec<f32> =
                bands.iter().map(|b| b.cells.0 as f32 * CELL_W / b.src.0.max(1) as f32).collect();
            let (lo, hi) = factors.iter().fold((f32::MAX, 0.0f32), |(lo, hi), f| (lo.min(*f), hi.max(*f)));
            assert!(
                hi - lo < 0.06 * hi,
                "{} [release {}] at {w}x{h}: the bands disagree on the horizontal factor \
                 ({lo:.3}..{hi:.3}) — the side art no longer aligns with the header plate",
                sp.title,
                sp.release
            );
        }
    }
}

/// …and the vertical half, which is what SQ-0698 changed: no band is
/// ANISOTROPIC. The stretch this work removes elongated the side art by whatever
/// the letterbox slack happened to be — measured here at 1.84 vertical against
/// 1.26 horizontal on Shogun at a 100x40 pane, and 2.2x against 1.0x on Zork
/// Zero at 117x64. Tiling fills the same space at the art's own aspect.
#[test]
fn no_side_flank_is_stretched_out_of_aspect() {
    const CELL: (f32, f32) = (8.0, 18.0);
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for sp in SPECIMENS {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            for b in &bands {
                // A one-row band is a rounding artefact of its own height, not a
                // statement about scale; skip it rather than loosen the bound.
                if b.cells.1 <= 1 {
                    continue;
                }
                let hx = b.cells.0 as f32 * CELL.0 / b.src.0.max(1) as f32;
                let vy = b.cells.1 as f32 * CELL.1 / b.src.1.max(1) as f32;
                assert!(
                    (hx - vy).abs() < 0.06 * hx.max(vy),
                    "{} [release {}] at {w}x{h}: band {b:?} is anisotropic — \
                     horizontal {hx:.3}, vertical {vy:.3}",
                    sp.title,
                    sp.release
                );
            }
        }
    }
}

// ── 4. Nothing else moved ────────────────────────────────────────────────────

/// The corpus guard the quest asked for: check for any OTHER title whose side
/// art this reaches. A tiled band is the signature; no other v6 story may draw
/// one.
#[test]
fn no_other_v6_title_grows_a_tiled_band() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for file in UNAFFECTED {
        let Some(mut s) = boot(file, None) else { continue };
        drive(&mut s, 12);
        for &(w, h) in PANES {
            let (_, bands, _) = frame(&s, (w, h));
            let tiled: Vec<_> = bands.iter().filter(|b| b.tiled).collect();
            assert!(
                tiled.is_empty(),
                "{file} at {w}x{h}: this title's flanks are not one of the three border \
                 layouts and must be drawn exactly as before, but got {tiled:?}"
            );
        }
    }
}

// ── 5. Every rendition tiles cleanly ─────────────────────────────────────────

/// The opaque column span of row `y` over columns `[x0, x1)`, as an offset from
/// `x0` so a native flank and the band cropped out of it are comparable.
/// `None` when nothing in that row is painted.
fn opaque_span(img: &image::RgbaImage, y: u32, x0: u32, x1: u32) -> Option<(u32, u32)> {
    let (mut first, mut last) = (None, 0);
    for x in x0..x1.min(img.width()) {
        if img.get_pixel(x, y)[3] >= 128 {
            first.get_or_insert(x - x0);
            last = x - x0;
        }
    }
    first.map(|f| (f, last))
}

/// SQ-0799, as the user reported it: *"for cg1 and eg1 we get a horizontal line
/// on zork0 where we are tiling"*.
///
/// Zork Zero's banner is **34 raw rows on MCGA, 37 on EGA and 39 on CGA** while
/// its pillars are 166 rows in all three (`zork0.mg1` id 5 is 320x34 and id 497
/// 36x166; `zork0.eg1` 640x37 and 74x166; `zork0.cg1` 640x39 and 70x166) — so
/// the pillars begin 6 unit rows lower under EGA and 10 lower under CGA, and a
/// repeat unit cut at a pinned row lands inside the ring beneath the capital
/// instead of in the plain shaft. Every tile boundary then repeats that ring.
///
/// Neither lane that built this could have seen it: SQ-0698's constants were
/// measured and verified when there was exactly ONE Zork Zero geometry, and
/// SQ-0790 made the renditions selectable afterwards.
///
/// **The oracle is independent of the fix.** The shaft's span is taken as the
/// MODAL opaque column span of the flank's own native rows — the one span that
/// holds for hundreds of rows, against a capital, a banner and a base that each
/// hold theirs for a few dozen. Below the art's own bottom every band row is
/// something this code composed, so every one of them, down to where the base is
/// stamped, must carry that span and nothing else. A ring tiled into the shaft
/// is a wider span and shows here; so would a base, or a slice of banner.
///
/// Falsifiable: pin the cut back to unit row 86 and `zork0.eg1` and `zork0.cg1`
/// fail while `zork0.mg1`, `zork0.pic` and the Blorb still pass.
#[test]
fn every_zork_zero_rendition_tiles_only_its_pillar_shaft() {
    /// Tall enough for a SECOND tile to land below the art's own bottom, which
    /// is what makes a repeated ring visible in the extension itself.
    const BAND: u32 = 800;
    /// Every picture archive shipped for Zork Zero, plus its Blorb (`None`).
    const RENDITIONS: &[Option<&str>] = &[
        Some("zork0.mg1"), // MCGA — the layout SQ-0698's constants were tuned to
        Some("zork0.eg1"), // EGA  — banner 3 raw rows taller
        Some("zork0.cg1"), // CGA  — banner 5 raw rows taller
        Some("zork0.pic"), // Amiga/Mac
        None,              // Zork0.blb
    ];
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    for rendition in RENDITIONS {
        let booted = match rendition {
            Some(a) => boot_named(sp.file, a, (sp.release, sp.serial)),
            None => boot(sp.file, Some((sp.release, sp.serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, sp.turns);
        let name = rendition.unwrap_or("Zork0.blb");
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items);
        let layout = app::render::v6_layout::classify_windows(items);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        let flank_x = [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())];
        for (x0, x1) in flank_x {
            // The flank's own columns of the native canvas; the extended band is
            // already cropped to them, so it is read from column 0.
            let native_span = |y: u32| opaque_span(&gfx, y, x0, x1);
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            assert_eq!(
                app::render::v6_border::recognize(&gfx, x0, x1, art, native.1 as u32),
                Some(BorderArt::ZorkZeroPillars),
                "{name} cols {x0}..{x1}: Zork Zero's flank is pillars in every rendition"
            );
            // The modal native span — the shaft's, by a margin of hundreds of rows.
            let mut tally: std::collections::HashMap<Option<(u32, u32)>, u32> = Default::default();
            for y in 0..art.1 {
                *tally.entry(native_span(y)).or_default() += 1;
            }
            let (shaft, held) = tally.iter().max_by_key(|(_, n)| **n).map(|(s, n)| (*s, *n)).expect("rows");
            assert!(held > art.1 / 2, "{name} cols {x0}..{x1}: no span holds for most of the flank");
            // …and the base is whatever sits below the last row that carries it.
            let base = art.1 - (0..art.1).filter(|&y| native_span(y) == shaft).max().expect("a shaft row") - 1;
            let out = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, BAND)
                .unwrap_or_else(|| panic!("{name} cols {x0}..{x1}: the flank should be extended"));
            let w = out.width();
            let wrong: Vec<u32> = (art.1..BAND - base).filter(|&y| opaque_span(&out, y, 0, w) != shaft).collect();
            assert!(
                wrong.is_empty(),
                "{name} [release {}] cols {x0}..{x1}: {} row(s) of the EXTENSION are not the \
                 pillar's own shaft span {shaft:?} — first at band row {:?}. The banner above \
                 these pillars is a different height in this rendition, so a repeat unit cut at a \
                 pinned row carries the ring under the capital and tiles it down the column",
                sp.release,
                wrong.len(),
                wrong.first(),
            );
        }
    }
}

// ── 7. No tile join steps harder than the art itself (SQ-0808) ───────────────

/// Mean luminance of row `y` over the whole of `img`; a transparent pixel
/// contributes nothing. This is the flank reduced to one number per row, which
/// is what lets a SHADING discontinuity be seen through a dither that changes
/// every pixel from row to row.
fn row_luma(img: &image::RgbaImage, y: u32) -> f64 {
    let mut s = 0.0;
    for x in 0..img.width() {
        let p = img.get_pixel(x, y);
        if p[3] >= 128 {
            s += (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
        }
    }
    s / img.width() as f64
}

/// `|mean(y-k..y) - mean(y..y+k)|` — a low-pass step detector. Zork Zero's CGA
/// masonry differs wildly between ADJACENT rows (its 1-bit line work is a dither,
/// and consecutive raw rows are essentially uncorrelated: a mean absolute row
/// difference of 7486 against 16065 at the worst), so a per-pixel comparison
/// cannot tell a seam from the texture. A 16-row average can: it sees the
/// pillar's shading and nothing else.
fn luma_step(prof: &[f64], y: usize, k: usize) -> f64 {
    let a: f64 = prof[y - k..y].iter().sum::<f64>() / k as f64;
    let b: f64 = prof[y..y + k].iter().sum::<f64>() / k as f64;
    (a - b).abs()
}

/// SQ-0808, as reported: *"the tiling seam on zork0.cg1 is plainly visible, and
/// Spatterlight's CGA flank does not show it"* — with the page no longer painted
/// white after SQ-0806, there was nothing left to hide it.
///
/// **The cause is not the dither and not the repeat unit.** SQ-0797's blend was
/// ruled out of CGA by its own note; SQ-0799 already derives the cut from the art,
/// and `every_zork_zero_rendition_tiles_only_its_pillar_shaft` proves the cut
/// lands in the plain shaft on all five renditions. The seam survived both
/// because **Zork Zero's CGA pillar is a lit column**: mean row luminance down
/// its shaft runs 97 → 82 top to bottom, where `zork0.mg1` holds a flat 54 and
/// `zork0.eg1` a flat 51. A translation repeat butts the strip's darkest row
/// against its brightest and resets the shading at every join. Measured on
/// `zork0-r393-s890714.z6` + `zork0.cg1` at an 800-row band: a step of **29.3**
/// at band row 654 — the second tile boundary — against the art's own steepest
/// internal step of 16.8.
///
/// The oracle is the art's own behaviour, so it needs no per-rendition constant:
/// nothing this code composes below the artwork may step harder than the pillar
/// shaft steps by itself. Falsifiable: pass `flip = false` to `extend_pillars`
/// in `v6_border::zork_zero` and `zork0.cg1` fails on both flanks with that
/// 29.3-against-16.8 step at row 654, while the other four renditions still pass
/// — which is why no earlier lane could have caught this on MCGA alone.
#[test]
fn no_tile_join_steps_harder_than_the_pillar_shaft_itself() {
    /// Tall enough for two tile boundaries to land below the artwork.
    const BAND: u32 = 800;
    /// The low-pass window, in unit rows — 8 raw rows of a doubled archive.
    const K: usize = 16;
    const RENDITIONS: &[Option<&str>] =
        &[Some("zork0.mg1"), Some("zork0.eg1"), Some("zork0.cg1"), Some("zork0.pic"), None];
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let sp = &SPECIMENS[2];
    assert_eq!(sp.title, "Zork Zero");
    for rendition in RENDITIONS {
        let booted = match rendition {
            Some(a) => boot_named(sp.file, a, (sp.release, sp.serial)),
            None => boot(sp.file, Some((sp.release, sp.serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, sp.turns);
        let name = rendition.unwrap_or("Zork0.blb");
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items);
        let layout = app::render::v6_layout::classify_windows(items);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        for (x0, x1) in [(0u32, story.x_px as u32), ((story.x_px + story.w_px) as u32, gfx.width())] {
            let art = app::render::v6_border::art_extent(&gfx, x0, x1);
            // The shaft, as the modal opaque span — the same oracle case 5 uses.
            let mut tally: std::collections::HashMap<Option<(u32, u32)>, u32> = Default::default();
            for y in 0..art.1 {
                *tally.entry(opaque_span(&gfx, y, x0, x1)).or_default() += 1;
            }
            let shaft = tally.iter().max_by_key(|(_, n)| **n).map(|(s, _)| *s).expect("rows");
            let rows: Vec<u32> = (0..art.1).filter(|&y| opaque_span(&gfx, y, x0, x1) == shaft).collect();
            let (top, bottom) = (*rows.first().expect("a shaft row"), rows.last().expect("a shaft row") + 1);
            let base = art.1 - bottom;
            let out = app::render::v6_border::flank_source(&gfx, &gfx, x0, x1, art, native.1 as u32, 0, BAND)
                .unwrap_or_else(|| panic!("{name} cols {x0}..{x1}: the flank should be extended"));
            let prof: Vec<f64> = (0..out.height()).map(|y| row_luma(&out, y)).collect();
            // What the pillar does to itself, inside its own shaft…
            let natural = (top as usize + K..bottom as usize - K)
                .map(|y| luma_step(&prof, y, K))
                .fold(0.0f64, f64::max);
            // …against every row this code composed below the artwork.
            let (at, worst) = (art.1 as usize + K..(BAND - base) as usize - K)
                .map(|y| (y, luma_step(&prof, y, K)))
                .fold((0usize, 0.0f64), |a, b| if b.1 > a.1 { b } else { a });
            assert!(
                worst <= natural,
                "{name} [release {}] cols {x0}..{x1}: the extension steps {worst:.2} at band row \
                 {at}, harder than the {:.2} the pillar shaft ({top}..{bottom}) ever steps by \
                 itself. That is a tile join resetting the pillar's shading — the CGA column is \
                 lit, so a repeat that merely translates it cannot be seamless",
                sp.release,
                natural,
            );
        }
    }
}

// ── The recogniser, pinned per specimen ──────────────────────────────────────

/// What each title's flank art measures, and therefore which handler runs. These
/// are the numbers every constant in `v6_border` was derived against; if a
/// release ever lays its border out differently, this is where it shows.
#[test]
fn each_specimen_is_recognised_as_its_own_layout() {
    use app::render::v6_border::{art_extent, recognize};
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let expected = [
        ("Arthur", BorderArt::ArthurPoles, (11u32, 379u32)),
        ("Shogun", BorderArt::ShogunSinglePiece, (0, 336)),
        ("Zork Zero", BorderArt::ZorkZeroPillars, (0, 400)),
    ];
    for (sp, (_, want, want_rows)) in SPECIMENS.iter().zip(expected) {
        let Some(mut s) = boot(sp.file, Some((sp.release, sp.serial))) else { continue };
        drive(&mut s, sp.turns);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items);
        let layout = app::render::v6_layout::classify_windows(items);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story = layout.story.expect("a story window");
        let rows = art_extent(&gfx, 0, story.x_px as u32);
        assert_eq!(
            rows, want_rows,
            "{} [release {}]: the left flank's native art rows",
            sp.title, sp.release
        );
        assert_eq!(
            recognize(&gfx, 0, story.x_px as u32, rows, native.1 as u32),
            Some(want),
            "{} [release {}]: recognised layout",
            sp.title,
            sp.release
        );
    }
}

// ── 6. Every rendition is recognised as its own title (SQ-0802) ──────────────

/// SQ-0802 — **Shogun's DOS renditions were classified as Zork Zero pillars.**
///
/// `recognize` used to decide on one measurement, and reaching the native screen
/// bottom won outright. Shogun's Amiga border stops at native row 336 of 400, but
/// its DOS art is authored for the full 200-row screen: `shogun.mg1` (23x200),
/// `shogun.eg1` (46x200), `shogun.cg1` (58x195) and `Shogun.blb` all paint to row
/// 400 in unit space and therefore satisfied it, so every one of them was handed
/// Zork Zero's masonry recipe — cut at unit row 86, a 284-row repeat, the bottom
/// 26 rows stamped back as a foot — applied to a Japanese lacquer frame. Worse,
/// `shogun.cg1`'s two flanks DISAGREED: the left stops at row 390 and was
/// correctly a single piece while the right reached 400 and was pillars.
///
/// The second measurement is the flank's SHAPE. Both ends of the cut are pinned
/// here from the corpus, at a gameplay frame, both flanks:
///
/// | flank                                       | narrowest ÷ widest painted row |
/// |---------------------------------------------|--------------------------------|
/// | Shogun `.mg1` / `.eg1` / `Shogun.blb`       | 1.00                           |
/// | Shogun `.cg1`                               | 1.00 (L), 0.96 (R)             |
/// | `James Clavell's Shogun.adf` (release 295)  | 1.00                           |
/// | Zork Zero castle, all five renditions       | 0.02 – 0.56                    |
/// | Zork Zero underground / jungle (composed)   | 0.37 – 0.81                    |
///
/// Falsifiable: drop the width test and the four DOS Shogun rows below come back
/// `ZorkZeroPillars` while the Amiga row still passes — which is exactly why the
/// original suite, which only ever booted the `.adf`, could not see it.
#[test]
fn every_rendition_is_recognised_as_its_own_titles_layout() {
    use app::render::v6_border::{art_extent, recognize};
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    /// One rendition and the layout its flanks must be recognised as. `archive`
    /// is `None` for whatever `PictSource::resolve` picks off the medium.
    struct Rendition {
        name: &'static str,
        story: &'static str,
        archive: Option<&'static str>,
        release: u16,
        serial: &'static str,
        want: BorderArt,
    }
    const SHOGUN: &str = "shogun-r322-s890706.z6";
    const ZORK0: &str = "zork0-r393-s890714.z6";
    let cases = [
        Rendition { name: "Shogun Amiga", story: SPECIMENS[1].file, archive: None, release: 295, serial: "890321", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun Blorb", story: SHOGUN, archive: None, release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun MCGA", story: SHOGUN, archive: Some("shogun.mg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun EGA", story: SHOGUN, archive: Some("shogun.eg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Shogun CGA", story: SHOGUN, archive: Some("shogun.cg1"), release: 322, serial: "890706", want: BorderArt::ShogunSinglePiece },
        Rendition { name: "Zork Zero MCGA", story: ZORK0, archive: Some("zork0.mg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero EGA", story: ZORK0, archive: Some("zork0.eg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero CGA", story: ZORK0, archive: Some("zork0.cg1"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero Amiga", story: ZORK0, archive: Some("zork0.pic"), release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Zork Zero Blorb", story: ZORK0, archive: None, release: 393, serial: "890714", want: BorderArt::ZorkZeroPillars },
        Rendition { name: "Arthur Amiga", story: SPECIMENS[0].file, archive: None, release: 54, serial: "890606", want: BorderArt::ArthurPoles },
    ];
    for Rendition { name, story, archive, release, serial, want } in &cases {
        let booted = match archive {
            Some(a) => boot_named(story, a, (*release, serial)),
            None => boot(story, Some((*release, serial))),
        };
        let Some(mut s) = booted else { continue };
        drive(&mut s, 12);
        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
        let native = app::render::v6_layout::native_extent(items);
        let layout = app::render::v6_layout::classify_windows(items);
        let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
        let story_win = layout.story.expect("a story window");
        // BOTH flanks: `shogun.cg1`'s disagreed, and one of them was right.
        for (x0, x1, side) in [
            (0u32, story_win.x_px as u32, "left"),
            ((story_win.x_px + story_win.w_px) as u32, gfx.width(), "right"),
        ] {
            let art = art_extent(&gfx, x0, x1);
            assert_eq!(
                recognize(&gfx, x0, x1, art, native.1 as u32),
                Some(*want),
                "{name} [release {release}] {side} flank, native art rows {art:?} of {}: this \
                 title's border is {want:?}. Reaching the screen bottom is not what makes a flank \
                 Zork Zero's pillars — narrowing below its banner is",
                native.1,
            );
        }
    }
}

/// …and the same fix seen through the real render, in both `honor_game_colours`
/// modes: Shogun's DOS flanks draw one tiled band per side with no hole in it,
/// exactly as its Amiga art does. The colour mode cannot reach the flank SOURCE
/// (it is composed from the graphics canvas before any theme decision), but it
/// can reach what the pane does with it, and single-mode suites have masked
/// regressions here before.
#[test]
fn shoguns_dos_flanks_tile_cleanly_in_both_colour_modes() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for archive in ["shogun.mg1", "shogun.eg1", "shogun.cg1"] {
        let Some(mut s) = boot_named("shogun-r322-s890706.z6", archive, (322, "890706")) else { continue };
        drive(&mut s, 12);
        for honor in [true, false] {
            for &(w, h) in PANES {
                let model = s.screen();
                let state = render_state_with(honor);
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let bands = parse_bands(&state.graphics_render.borrow().band_log);
                let fl = flanks(&bands, w);
                assert_eq!(
                    fl.len(),
                    2,
                    "{archive} [release 322] at {w}x{h}, honor_game_colours={honor}: expected a \
                     left and a right flank band, got {fl:?}"
                );
                for b in &fl {
                    assert!(
                        b.tiled && !b.stretched,
                        "{archive} at {w}x{h}, honor_game_colours={honor}: flank {b:?} must be TILED"
                    );
                    let (run, at) = b.blank;
                    assert!(
                        run == 0 || at == 0,
                        "{archive} at {w}x{h}, honor_game_colours={honor}: the flank source {b:?} \
                         has an INTERIOR hole — {run} row(s) with no art starting at row {at}"
                    );
                }
            }
        }
    }
}


