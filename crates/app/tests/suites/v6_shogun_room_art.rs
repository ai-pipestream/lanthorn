//! Shogun's Apple IIgs press draws its room illustration beside the prose in
//! HYBRID, not as a three-row sliver at the top of the pane — SQ-0888.
//!
//! THE REPORT. On the five-volume ProDOS set the opening Bridge illustration came
//! out squashed into a sliver across the top of the pane while the prose ran full
//! width underneath it. Raster drew the ship.
//!
//! WHAT IT WAS, measured on `shogun_s1.dsk` (release 311 / serial 890510) at a
//! 100x40 pane under kitty. The game puts window 0 at (1,65) 560x320 and gives it
//! `right_margin` 320 — its prose lives in the left 240 native px — then draws
//! picture 7, 312x348 after the Apple `art_scale` of (4,2), at screen x 248 into
//! WINDOW 6, a graphics window at (1,33) 560x352 that contains window 0 outright.
//! Window 6 is chrome, and hybrid's ring is pane MINUS the story viewport, so with
//! the viewport spanning window 0's full width there was nowhere for the ship to
//! be drawn: 316 of its 348 rows were discarded outright and the 32 that happen to
//! sit above window 0 survived as the sliver. The emitted stream said it exactly —
//! **13 kitty placements, every one of them at rows 3..5, and nothing else on the
//! frame**.
//!
//! WHAT FIXED IT. The game had already said where its prose goes and where it does
//! not: `set_margins` (ZMSD §8.8.3.2, window properties 6 and 7) reserves a column
//! of window 0 that its text will never enter. So hybrid's story viewport is that
//! TEXT column, not the whole window box, whenever chrome art is actually painting
//! in the column the margin gave up — and the ring's flank carries the picture, at
//! the game's own coordinates and the uniform scale. The prose stays terminal
//! cells, which is what SQ-0750 asks for and what taking the composite would have
//! cost: this frame's raster composite draws the ship and NOT ONE WORD of the
//! prose, because window 6 sits above window 0 in z-order.
//!
//! THE OTHER PRESS IS THE CONTROL. The Amiga (release 295 / serial 890321) and the
//! IBM Blorb (release 322 / serial 890706) state the same layout the other way
//! round — `draw_picture 7` into window 0 with a `set_margins` right after, which
//! `is_win0_inline_float` already reads as a margin picture — and carry
//! `right_margin` 328 on the very same frame with NO chrome art behind it. They
//! must not narrow, and their frames must not move.
//!
//! `stories/` is gitignored, so every case skips vacuously.

use std::path::PathBuf;
use std::sync::Mutex;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global, so no two renditions may boot at
/// once (the same lock `v6_shogun_menu_ground` takes, for the same reason).
static PALETTE: Mutex<()> = Mutex::new(());

/// The five-volume 5.25-inch ProDOS press, named by the exact build it carries.
const APPLE: (&str, u16, &str) = ("shogun_s1.dsk", 311, "890510");
/// The two presses that draw the same ship into window 0 instead.
const WINDOW_0_PRESSES: &[(&str, u16, &str)] =
    &[("James Clavell's Shogun.adf", 295, "890321"), ("shogun-r322-s890706.z6", 322, "890706")];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` does — the profile, the artwork, the native screen
/// and the art scale all come from the medium. The Apple press needs every one of
/// them: read through the Blorb-shaped path instead it comes up 640x400 at scale
/// (2,2) rather than its own 560x384 at (4,2), and none of the coordinates below
/// are its own any more.
fn boot(file: &str, release: u16, serial: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file}: this medium carries a DIFFERENT build than the table says"
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px =
        picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    let mut s = GameSession::new_with_art_scale(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        art_scale,
        profile.default_colours(),
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{file}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    // Two keypresses: past the title splash, then START on the credits menu. The
    // opening Bridge scene is the frame this file measures — the one turn on which
    // the game holds a margin open beside its illustration.
    for _ in 0..2 {
        match s.pending_input() {
            InputKind::Char => s.submit_char(13),
            _ => s.submit(""),
        };
    }
    Some(s)
}

/// The story window and the chrome graphics canvas of the frame now on screen.
fn frame_model(s: &GameSession) -> Option<(app::engine::PositionedWindow, image::RgbaImage, (u16, u16))> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { return None };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);
    let story = layout.story?.clone();
    let gfx = app::render::v6_layout::build_graphics_canvas(&layout.chrome, native);
    Some((story, gfx, native))
}

#[allow(deprecated)] // `from_fontsize`: a headless test has no terminal to query.
fn render(s: &GameSession, honor: bool, pane: (u16, u16)) -> (Buffer, String) {
    let model = s.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // Halfblocks is the protocol, which is what lets a case assert on the pane's
    // own CELLS: the ring's bands land in them.
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, pane.0, pane.1);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    let path = state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default();
    (buf, path)
}

/// Is this cell showing the game's own ink rather than a grey? The story page, the
/// theme backdrop and the prose are all neutral on this frame in every press, so a
/// channel spread of any size is artwork. (The same test `v6_shogun_menu_ground`
/// uses, for the same reason.)
fn is_art(buf: &Buffer, x: u16, y: u16) -> bool {
    [buf[(x, y)].bg, buf[(x, y)].fg].iter().any(|c| {
        let ratatui::style::Color::Rgb(r, g, b) = *c else { return false };
        r.max(g).max(b) - r.min(g).min(b) >= 24
    })
}

/// The last pane row carrying any of the game's artwork, and how many rows do.
fn art_rows(buf: &Buffer, pane: (u16, u16)) -> (Option<u16>, usize) {
    let rows: Vec<u16> = (0..pane.1).filter(|&y| (0..pane.0).any(|x| is_art(buf, x, y))).collect();
    (rows.last().copied(), rows.len())
}

const PANES: &[(u16, u16)] = &[(98, 38), (78, 28), (157, 59)];

// ── 1. The premise: this frame really is the one the quest measured ─────────

/// The Apple press states its layout in window properties, and the ship is chrome.
///
/// Everything below rests on these four facts, so they are asserted rather than
/// assumed: if a later build of the loader resolves this medium differently, the
/// cases that follow should say so here instead of quietly measuring some other
/// screen.
#[test]
fn the_apple_press_reserves_a_margin_and_paints_chrome_art_in_it() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(APPLE.0, APPLE.1, APPLE.2) else { return };
    let (story, gfx, native) = frame_model(&s).expect("the Bridge scene is a layered v6 frame");
    assert_eq!(
        (native.0, native.1),
        (560, 384),
        "the Apple press runs on its own 560x384 screen (140x192 picture space at art_scale (4,2))"
    );
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 64, 560, 320),
        "window 0's box on the Bridge"
    );
    assert_eq!(
        (story.left_margin, story.right_margin),
        (0, 320),
        "the game reserves the right 320 px of window 0 for something that is not its prose"
    );
    // …and that something is CHROME art, painting inside window 0's own box.
    let ceded = (64..384u32).any(|y| (240..560u32).any(|x| gfx.get_pixel(x, y)[3] >= 128));
    assert!(
        ceded,
        "picture 7 (the ship) is drawn into window 6 at screen x 248, so the column the margin \
         reserved is full of chrome artwork — that is what makes this frame different from the \
         Amiga's, where the same ship is a window-0 margin float and the column is empty"
    );
}

// ── 2. The reported symptom, stated directly ────────────────────────────────

/// The illustration reaches the pane BESIDE the prose, not as a sliver above it.
///
/// The defect's signature is exact and worth keeping in the failure message: every
/// placement on the frame at rows 3..5, which is the 32 native rows of the picture
/// that happen to lie above window 0's box, and not one pixel of the other 316.
#[test]
fn the_room_illustration_reaches_the_pane_beside_the_prose() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(APPLE.0, APPLE.1, APPLE.2) else { return };
    let (story, _, native) = frame_model(&s).expect("layered v6 frame");
    for honor in [true, false] {
        for &pane in PANES {
            let (buf, path) = render(&s, honor, pane);
            assert_eq!(path, "hybrid-ring", "honor={honor} {pane:?}: this frame takes the ring");
            // The row window 0's own top maps to. Everything at or below it used to
            // be prose and backdrop and nothing else.
            let s_uniform = ((pane.0 as f32 * 8.0) / native.0 as f32)
                .min((pane.1 as f32 * 18.0) / native.1 as f32);
            let story_top_row = ((story.y_px as f32 * s_uniform) / 18.0).ceil() as u16;
            let below: usize = (story_top_row..pane.1)
                .map(|y| (0..pane.0).filter(|&x| is_art(&buf, x, y)).count())
                .sum();
            assert!(
                below >= 200,
                "{} [release {}] honor={honor} {pane:?}: only {below} cells of the game's artwork \
                 sit at or below row {story_top_row}, where window 0's box begins. The room \
                 illustration is 312x348 native px and 316 of its 348 rows are inside that box — \
                 the reported defect is that the ring drew pane-minus-viewport, threw all 316 \
                 away and left the surviving 32 as a three-row sliver at the top of the pane \
                 (measured on the emitted stream as 13 placements, every one of them at rows \
                 3..5, with nothing else on the frame).",
                APPLE.0,
                APPLE.1
            );
        }
    }
}

/// …and the story's own column is left to the story.
///
/// The other half of the same rule: the ring took the column the game reserved, so
/// it must not have taken the one the game kept. Window 0's text column is the
/// left 240 native px; art appearing there would mean the ring had eaten the prose.
#[test]
fn the_column_the_game_kept_for_its_prose_carries_no_artwork() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(APPLE.0, APPLE.1, APPLE.2) else { return };
    let (story, _, native) = frame_model(&s).expect("layered v6 frame");
    let text_px = story.w_px as u32 - story.right_margin as u32; // 240
    for honor in [true, false] {
        for &pane in PANES {
            let (buf, _) = render(&s, honor, pane);
            let s_uniform = ((pane.0 as f32 * 8.0) / native.0 as f32)
                .min((pane.1 as f32 * 18.0) / native.1 as f32);
            let story_top_row = ((story.y_px as f32 * s_uniform) / 18.0).ceil() as u16;
            // One column of slack: the viewport's left edge is quantized to a whole
            // cell, so the boundary column can belong to either side.
            let last_text_col = (((text_px as f32 * s_uniform) / 8.0).floor() as u16).saturating_sub(1);
            let intruding: Vec<(u16, u16)> = (story_top_row..pane.1)
                .flat_map(|y| (0..last_text_col).map(move |x| (x, y)))
                .filter(|&(x, y)| is_art(&buf, x, y))
                .collect();
            assert!(
                intruding.is_empty(),
                "honor={honor} {pane:?}: {} cells of artwork stand in the columns window 0 kept \
                 for its own text (native x 0..{text_px}, i.e. pane cols 0..{last_text_col}); \
                 first at {:?}. The ring may only take the column `set_margins` gave up.",
                intruding.len(),
                intruding[0]
            );
        }
    }
}

/// The picture is drawn ONCE, at its own height — not tiled down the column.
///
/// The flank the fix creates is tall and thin, which is the shape `v6_border`
/// looks for, and `recognize` names the border it is shown or falls through to
/// `ArthurPoles` for anything else — a four-row strip stamped down the column to
/// the screen bottom. Run over the ship that smeared the hull's last rows across
/// the frame's whole lower third, so the flank must be excluded from it: a column
/// carrying an ILLUSTRATION is never a border, however tall and thin it is. (The
/// same shape SQ-0819 records for Journey's picture column, from the other side.)
#[test]
fn the_illustration_is_not_tiled_down_the_column() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = boot(APPLE.0, APPLE.1, APPLE.2) else { return };
    let (_, gfx, native) = frame_model(&s).expect("layered v6 frame");
    // The lowest native row the game painted anything on.
    let art_bottom = (0..gfx.height())
        .rev()
        .find(|&y| (0..gfx.width()).any(|x| gfx.get_pixel(x, y)[3] >= 128))
        .expect("the frame has art on it") as f32;
    for honor in [true, false] {
        for &pane in PANES {
            let s_uniform = ((pane.0 as f32 * 8.0) / native.0 as f32)
                .min((pane.1 as f32 * 18.0) / native.1 as f32);
            // Only meaningful where the pane is taller than the scaled screen: with
            // no letterbox slack the art legitimately reaches the last row.
            if (native.1 as f32 * s_uniform) >= (pane.1 as f32 * 18.0) - 18.0 {
                continue;
            }
            let (buf, _) = render(&s, honor, pane);
            let (last, _) = art_rows(&buf, pane);
            let last = last.expect("the frame draws artwork");
            // The ring top-anchors under the Extend plan (`off_y = 0`), so the art's
            // native bottom maps straight through the uniform scale. One row of
            // slack for the cell quantization at each end.
            let expect = (art_bottom * s_uniform / 18.0).ceil() as u16;
            assert!(
                last <= expect + 1,
                "honor={honor} {pane:?}: the game's artwork reaches pane row {last}, but its own \
                 lowest painted native row ({art_bottom}) is row {expect}. The flank is being \
                 TILED to the pane bottom as though the ship were a repeating border column."
            );
        }
    }
}

// ── 3. …and the presses that draw into window 0 do not move ────────────────

/// The Amiga and IBM presses hold `right_margin` 328 on this very frame and must
/// NOT be narrowed: nothing is painted in the column, because their ship is a
/// window-0 margin float that `is_win0_inline_float` already handles.
///
/// This is the guard that keeps the rule from being a blanket "inset by the
/// margins", which would have taken 328 of their 548 native columns off the prose
/// to hand the ring an empty flank.
#[test]
fn a_reserved_margin_with_nothing_painted_in_it_is_left_alone() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for &(file, release, serial) in WINDOW_0_PRESSES {
        let Some(s) = boot(file, release, serial) else { continue };
        ran += 1;
        let (story, gfx, _) = frame_model(&s).expect("layered v6 frame");
        assert_eq!(
            story.right_margin, 328,
            "{file} [release {release}]: this press reserves 328 px beside its ship too"
        );
        assert!(
            app::render::v6_layout::story_text_box(&story, &gfx).is_none(),
            "{file} [release {release}]: the reserved column has no CHROME art in it — the ship is \
             a window-0 margin float here — so window 0's box must be left exactly as the game set \
             it. Narrowing it would take 328 of its {} native columns off the prose and hand the \
             ring a blank flank.",
            story.w_px
        );
    }
    assert!(ran > 0 || !stories_dir().exists(), "no window-0 Shogun press present — every case skipped");
}
