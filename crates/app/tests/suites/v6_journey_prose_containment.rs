//! Journey's story prose stays in its viewport, and where its frame border lives —
//! SQ-0755, with the measurement that explains SQ-0742's leftover.
//!
//! The report: *"the intro text got rendered at the bottom overwriting the menu blocks
//! under Individual Commands"*, then *"actually a lot of the text is being rendered
//! down there"*. A unified reading was proposed — that prose overwriting the header row
//! would also eat `The Party` down to `The Par`, explaining the truncated labels three
//! sweeps failed to reproduce.
//!
//! It also explains why those sweeps found nothing, and that part is the useful lesson
//! whatever the cause turns out to be: **they rendered the menu frame with an EMPTY
//! transcript.** Journey's prose never reaches the story window's `lines` — it arrives
//! as the session's transcript and is drawn from `AppState`, so a harness that renders
//! `session.screen()` without ever calling `push_transcript` draws a frame with no
//! story text in it at all. Every previous Journey harness in this repo does exactly
//! that. The first case here feeds the transcript, which is what makes it a test of
//! anything.
//!
//! It then attributes: the same game state is rendered twice, once with the transcript
//! and once without, and every cell OUTSIDE the story viewport that differs is a cell
//! the prose painted where it does not belong. That is a sharper instrument than
//! looking for prose-shaped text at the bottom, because Journey's menu legitimately
//! contains words from the prose ("pouch", "Examine") and no reader of cells can tell
//! those apart.
//!
//! The second case pins the measured reason Journey's side borders read as missing.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::render::graphics::kitty_picker;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global, so profile-booting cases serialise.
static PALETTE: Mutex<()> = Mutex::new(());

fn palette_lock() -> MutexGuard<'static, ()> {
    PALETTE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The user's terminal: 140x71 with nothing else docked, so a 138x68 story pane.
const PANE: Rect = Rect { x: 1, y: 1, width: 138, height: 68 };

const PLACEHOLDER: char = '\u{10EEEE}';

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn journey(profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

fn kitty_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(kitty_picker(8, 18));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

fn frame(session: &GameSession, state: &app::state::AppState, char_mode: bool) -> Buffer {
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, char_mode, None, state, PANE, &mut buf);
    buf
}

/// A kitty band writes its whole placeholder run into one cell, so compare the glyph a
/// reader would see rather than the raw symbol.
fn glyph(buf: &Buffer, x: u16, y: u16) -> String {
    let s = buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
    if s.chars().any(|c| c == PLACEHOLDER) { "#".into() } else { s }
}

fn advance(session: &mut GameSession, key: u8) -> String {
    match session.pending_input() {
        InputKind::Line | InputKind::Event => session.submit("").transcript,
        InputKind::Char => session.submit_char(key).transcript,
    }
}

/// SQ-0755: the story text belongs inside the story viewport, and nowhere else.
///
/// Two sessions are driven in lockstep through the same keys; one `AppState` is fed the
/// transcript and the other is not. Any cell outside the viewport that differs between
/// the two renders was painted by the prose — which is the defect, whatever it looks
/// like. The menu strip, the frame's own rows and the flank bands all sit outside the
/// viewport, so this covers the reported symptom (prose over the menu blocks) and the
/// unified reading of it (prose over the header row eating `The Party`) at once.
#[test]
fn story_prose_never_paints_outside_its_viewport() {
    for honor in [true, false] {
        // `char_mode` is what the app passes while a v6 game sits in `read_char`, which
        // is Journey's entire gameplay; a harness that hardcodes `false` renders a mode
        // the player never sees.
        for char_mode in [false, true] {
            let _guard = palette_lock();
            let Some(mut with_sess) = journey(InterpreterProfile::Amiga) else { return };
            let Some(mut without_sess) = journey(InterpreterProfile::Amiga) else { return };
            let mut with = kitty_state(honor);
            let without = kitty_state(honor);
            // Journey's menus answer arrows and Enter; a diet of Enter alone parks the
            // game in a two-state loop and never exercises its prose screens.
            let keys: [u8; 12] = [13, 130, 13, 131, 13, 129, 13, 132, 13, 13, 130, 13];
            let mut saw_prose = false;
            for step in 0..40usize {
                let a = frame(&with_sess, &with, char_mode);
                let b = frame(&without_sess, &without, char_mode);
                let viewport = with
                    .v6_cell_map
                    .borrow()
                    .iter()
                    .find(|e| e.label == "viewport")
                    .map(|e| e.cells);
                if let Some((vx, vy, vw, vh)) = viewport {
                    let mut outside: Vec<(u16, u16, String, String)> = Vec::new();
                    for y in PANE.top()..PANE.bottom() {
                        for x in PANE.left()..PANE.right() {
                            if x >= vx && x < vx + vw && y >= vy && y < vy + vh {
                                continue;
                            }
                            let (ga, gb) = (glyph(&a, x, y), glyph(&b, x, y));
                            if ga != gb {
                                outside.push((x, y, ga, gb));
                            }
                        }
                    }
                    assert!(
                        outside.is_empty(),
                        "honor={honor} char_mode={char_mode} step {step}: the story transcript \
                         painted {} cell(s) OUTSIDE its viewport {viewport:?} — the menu strip, \
                         the frame rows and the flanks all live out there. First few: {:?}",
                        outside.len(),
                        &outside[..outside.len().min(8)]
                    );
                }
                let t = advance(&mut with_sess, keys[step % keys.len()]);
                let _ = advance(&mut without_sess, keys[step % keys.len()]);
                if !t.trim().is_empty() {
                    saw_prose = true;
                    with.push_transcript(&t);
                }
            }
            assert!(
                saw_prose,
                "honor={honor} char_mode={char_mode}: no prose ever reached the transcript, so \
                 this case asserted nothing — the same way every earlier Journey sweep did"
            );
        }
    }
}

/// SQ-0742, the part the extension fix did not settle: WHERE Journey's frame border is.
///
/// Reported still-present after `a93e9218`: *"side borders ... only come partway down"*.
/// Measured on the chrome canvas at a mid-story row, 138x68, Amiga: the frame's right
/// border is **one native pixel column** (x=635), with transparency either side of it,
/// and the left divider likewise (x=259). The column immediately abutting the story box
/// — the only column the pre-fix probe looked at — is empty, which is exactly why no
/// extension was ever produced for this profile. That half is fixed and this pins it.
///
/// What it also shows is why the border still reads as absent: one native pixel through
/// a 1.72x scale is under two device pixels inside an eight-pixel character cell, so the
/// side border is a hairline drawn by the image layer while the menu rows below it carry
/// a crisp font `│`. That is SQ-0750's glyph/raster split, measured here rather than
/// argued, and it is not fixed by moving anything.
///
/// FALSIFY by restoring the single-column probe: the Amiga assertion below is the exact
/// pixel that probe read, and it is transparent.
#[test]
fn journeys_frame_border_is_a_single_native_pixel_column() {
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let _guard = palette_lock();
        let Some(mut session) = journey(profile) else { return };
        for _ in 0..40 {
            let t = advance(&mut session, 13);
            if t.contains("Praxix") || t.contains("magical resources") {
                break;
            }
        }
        let model = session.screen();
        let items: &[app::engine::PositionedWindow] = match &model.root {
            WinNode::Layered(v) => v,
            _ => &[],
        };
        let layout = app::render::v6_layout::classify_windows(items);
        let native = app::render::v6_layout::native_extent(items);
        let colors = app::colors::ColorScheme::terminal_default();
        let canvas = app::render::v6_layout::build_chrome_canvas(
            &layout.chrome,
            native,
            image::Rgba([200, 200, 200, 255]),
            image::Rgba([0, 0, 0, 255]),
            &colors,
        );
        let story = layout.story.expect("Journey publishes a story window at its menu");
        let mid = story.y_px as u32 + (story.h_px as u32) / 2;
        let x1 = story.x_px as u32 + story.w_px as u32;
        let opaque = |x: u32| canvas.get_pixel(x, mid)[3] >= 128;
        // Somewhere within ONE text cell of the story box there is a border column.
        let found = (x1..(x1 + 8).min(canvas.width())).find(|&x| opaque(x));
        assert!(
            found.is_some(),
            "{profile:?}: no border ink within a text cell of the story box's right edge \
             (x1={x1}, mid row {mid}) — the flank has nothing to carry down the gap"
        );
        let found = found.expect("checked");
        match profile {
            // A reverse-video block border inks the abutting column, which is why the
            // single-column probe always worked for this profile.
            InterpreterProfile::IbmPc => assert_eq!(
                found, x1,
                "IbmPc draws its border as reverse-video spaces, which fill the cell"
            ),
            // A box-drawing glyph's stroke sits inside its cell, leaving the abutting
            // column blank — the whole reason the probe had to widen.
            _ => {
                assert!(
                    found > x1,
                    "Amiga draws `│`, whose stroke is inset from the cell edge; the abutting \
                     column {x1} should be blank but was inked"
                );
                // And it really is a hairline: one column, transparent on both sides.
                assert!(
                    !opaque(found + 1),
                    "{profile:?}: the border at x={found} is wider than one native pixel — \
                     re-measure, the hairline reading below depends on this"
                );
            }
        }
    }
}
