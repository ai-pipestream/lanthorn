//! SQ-0964: on the half-blocks backend the v6 composite grows with the cell grid,
//! because there is no encode for a cap to budget.
//!
//! `MAX_V6_UPSCALE` is a PNG-encode budget: under kitty every extra factor of
//! magnification is more bytes to build and write for every frame that changes, so
//! the composite stops at 2x and the protocol's `Fit` shrinks and centres from
//! there. Half-blocks encodes nothing — `ratatui-image` resolves the image straight
//! into terminal cells at one pixel per column and two per row — so the budget it
//! protects does not exist, while its cost is entirely real: `Resize::Fit` only ever
//! SHRINKS, so the pre-scale is what decides how many CELLS the composite occupies.
//! Pinned at 2x, a 640x400 screen reached a fixed 128x40 cells however fine the grid
//! got, which is the reported symptom in one line: *shrinking the terminal font, which
//! should give half-blocks more pixels and a sharper picture, just made the game window
//! smaller.*
//!
//! The two titles here are the ones the report names, and they are the visible cases
//! for a reason: **neither publishes a primary `Buffer`**, so both fall through to the
//! raster composite in HYBRID mode as well as raster mode (SQ-0711), and the cap is
//! the only thing deciding their size in either. Every other v6 title reaches the pane
//! through the hybrid ring, which has never had a ceiling.
//!
//! ## Specimens
//!
//! | story | fixture | archive | release / serial | turns | declared screen | unit screen |
//! |---|---|---|---|---|---|---|
//! | scopa | `scopa.z6` | `scopa.blb` | r1 / 110321 | 3 | none, art_scale (1, 1) | 640x400 |
//! | fmvpoker | `fmvpoker.z6` | `fmvpoker.blb` | r60 / 001227 | 4 | 320x200, art_scale (2, 2) | 640x400 |
//!
//! Both are modern Inform v6 titles off bare story files, so neither names a machine
//! and both resolve to the `IbmPc` FALLBACK profile — which declares no standard
//! window, hence scopa's `None` above. The harness prints all of it on every run.
//!
//! Both are booted the way `startup.rs` boots — the profile from the medium (a bare
//! file names none, so the default), then
//! `picts.std_window() -> picts.native_std_window() -> profile.std_window()` with
//! `art_scale` alongside — and the harness prints what it got, because a screen size
//! taken by a shorter chain is a screen the player never sees (SQ-0901).
//!
//! `stories/` is gitignored (CLAUDE.md), so every case here skips vacuously without
//! the fixtures — with the `!any_present || seen > 0` shape, so a case that finds a
//! fixture and measures nothing still fails.
//!
//! FALSIFY by restoring the unconditional `.min(MAX_V6_UPSCALE)` in `v6_fit_source`:
//! `a_fine_grid_reaches_the_pane_under_halfblocks` fails on both titles with the
//! reported symptom — the composite stalls at 40 rows while the pane has 60.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::graphics::{kitty_picker, v6_fit_source, v6_upscale_cap};
use app::session::{GameSession, InputKind};
use ratatui::layout::Size;
use ratatui_image::picker::Picker;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// A title that publishes no primary `Buffer` and so reaches the screen through the
/// raster composite whichever mode the player is in.
struct Specimen {
    title: &'static str,
    file: &'static str,
    /// How the frame under test was reached: this many turns of "press on", each a
    /// character or a blank line depending on what the story is waiting for. A frame
    /// is a fixture and the turn count is part of it (CLAUDE.md).
    turns: usize,
}

const SPECIMENS: [Specimen; 2] = [
    Specimen { title: "scopa", file: "scopa.z6", turns: 3 },
    Specimen { title: "fmvpoker", file: "fmvpoker.z6", turns: 4 },
];

/// The palette this suite resolves through, **stated rather than inherited**
/// (SQ-0958). Both stories are bare files that name no machine, so their colour
/// numbers resolve through ZMSD 8.3.1's own table — which is the ground every canvas
/// measured below is painted in. Hold the guard for the whole case.
fn standard_palette() -> std::sync::MutexGuard<'static, ()> {
    app::v6_palette(zvm::screen::Palette::Standard)
}

/// The booted session, the release it carries, and the unit screen it was booted at
/// — all three printed, because a measurement whose boot chain is not stated cannot
/// be checked against anything (SQ-0901).
fn boot(spec: &Specimen) -> Option<GameSession> {
    let path = stories_dir().join(spec.file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let release = u16::from_be_bytes([bytes[2], bytes[3]]);
    let serial = String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned();
    // The profile comes from the MEDIUM, and a bare story file is no medium at all.
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    // `startup.rs`'s own chain, in order. Skipping `native_std_window` is what booted
    // a 560x384 press at 640x400 and fabricated a frame a whole quest was fixed
    // against; there is no named-archive override in this harness.
    let v6_screen_px = picts
        .std_window()
        .or_else(|| picts.native_std_window())
        .or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    eprintln!(
        "{}: v{} release {release} serial {serial}, profile {profile:?}, unit screen {v6_screen_px:?}, art_scale {art_scale:?}",
        spec.title, bytes[0],
    );
    let mut s = GameSession::new_with_art_scale(
        bytes, true, false, None, false, dims, v6_screen_px, art_scale, None, None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..spec.turns {
        match s.pending_input() {
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            _ => {
                let _ = s.submit("");
            }
        }
        let _ = s.take_transcript();
    }
    Some(s)
}

/// The composite this title hands the backend: the game's own painted ground, at the
/// unit extent every window on the screen is laid out in.
fn composite(s: &GameSession) -> (image::RgbaImage, (u16, u16)) {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let paint = Engine::paint_surface(s).expect("both specimens paint a ground of their own");
    (paint.as_ref().clone(), native)
}

/// How many CELLS of a `cols x rows` pane the composite reaches on a backend with
/// this upscale ceiling — the protocol's own `Fit` arithmetic, not a restatement of
/// it, so what is measured is what the pane gets.
fn reach(canvas: &image::RgbaImage, cap: Option<f64>, fs: ratatui_image::FontSize, cols: u16, rows: u16) -> Size {
    let (box_w, box_h) = (u32::from(cols) * u32::from(fs.width), u32::from(rows) * u32::from(fs.height));
    let (src, fit) = v6_fit_source(canvas, box_w, box_h, None, cap);
    fit.size_for(&image::DynamicImage::ImageRgba8(src), fs, Size::new(cols, rows))
}

/// The premise, and the non-vacuity guard for everything below: both titles compose a
/// 640x400 unit screen and paint it themselves. A frame that is not this shape would
/// make every number after it a measurement of some other screen.
#[test]
fn both_specimens_paint_a_640x400_screen_of_their_own() {
    let _g = standard_palette();
    let (mut any_present, mut seen) = (false, 0usize);
    for spec in &SPECIMENS {
        let Some(s) = boot(spec) else { continue };
        any_present = true;
        let (canvas, native) = composite(&s);
        assert_eq!(native, (640, 400), "{}: the v6 unit screen every window lays out on", spec.title);
        assert_eq!(
            canvas.dimensions(),
            (640, 400),
            "{}: its painted ground covers that screen — the composite handed to the backend",
            spec.title
        );
        let painted = canvas.pixels().filter(|p| p.0[3] > 0).count();
        eprintln!("{}: {painted} painted pixels of 256000 after {} turns", spec.title, spec.turns);
        assert!(
            painted > 5_000,
            "{}: the ground has to be real painted pixels, not a stray box — got {painted}",
            spec.title
        );
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a present fixture must have been measured");
}

/// The fix, on the games it was reported against: at a fine grid the composite fills
/// the pane's short axis under half-blocks, and stalls under the encode cap.
///
/// 200x60 cells is a 1920x1200 terminal at a small font. Half-blocks reports its own
/// nominal 10x20 whatever the real font is — that IS the protocol, one pixel per
/// column and two per row — so a finer grid is straightforwardly more room, and the
/// picture should take it.
#[test]
fn a_fine_grid_reaches_the_pane_under_halfblocks() {
    let _g = standard_palette();
    let hb = Picker::halfblocks();
    let fs = hb.font_size();
    let (mut any_present, mut seen) = (false, 0usize);
    for spec in &SPECIMENS {
        let Some(s) = boot(spec) else { continue };
        any_present = true;
        let (canvas, _) = composite(&s);
        let free = reach(&canvas, v6_upscale_cap(&hb), fs, 200, 60);
        let capped = reach(&canvas, Some(2.0), fs, 200, 60);
        assert_eq!(
            free.height, 60,
            "{}: half-blocks must fill the pane's short axis at a 200x60 grid — it reached \
             {free:?}, and a picture that does not grow with the grid is the whole defect",
            spec.title
        );
        assert!(
            free.width > capped.width && free.height > capped.height,
            "{}: uncapped {free:?} must beat the capped {capped:?} on BOTH axes; that gap is \
             what the player sees when the font shrinks",
            spec.title
        );
        assert_eq!(
            capped.height, 40,
            "{}: and the encode cap still stops the encoded backends at 40 rows of the 60 \
             the pane has — deliberately, because kitty pays for every one of those pixels",
            spec.title
        );
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a present fixture must have been measured");
}

/// Nothing moved for kitty. Its ceiling is a budget it genuinely spends, and the pane
/// sweep it was tuned against answers exactly as it did before — including the coarse
/// panes, where the cap was never the binding constraint and both backends agree.
#[test]
fn the_kitty_composite_is_unchanged() {
    let _g = standard_palette();
    let kitty = kitty_picker(8, 18);
    let fs = kitty.font_size();
    let (mut any_present, mut seen) = (false, 0usize);
    for spec in &SPECIMENS {
        let Some(s) = boot(spec) else { continue };
        any_present = true;
        let (canvas, _) = composite(&s);
        assert_eq!(
            v6_upscale_cap(&kitty),
            Some(2.0),
            "{}: kitty encodes, so it keeps the ceiling",
            spec.title
        );
        for (cols, rows) in [(60u16, 24u16), (100, 40), (166, 44), (240, 80)] {
            let got = reach(&canvas, v6_upscale_cap(&kitty), fs, cols, rows);
            let capped = reach(&canvas, Some(2.0), fs, cols, rows);
            assert_eq!(
                got, capped,
                "{}: at {cols}x{rows} kitty must answer exactly as the flat 2x cap always did",
                spec.title
            );
            assert!(
                got.width <= cols && got.height <= rows,
                "{}: at {cols}x{rows} the composite reached {got:?} — the pane is still the bound",
                spec.title
            );
        }
        seen += 1;
    }
    assert!(!any_present || seen > 0, "a present fixture must have been measured");
}
