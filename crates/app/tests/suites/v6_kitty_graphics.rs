//! The kitty pixel path, exercised headlessly — SQ-0590.
//!
//! Every other v6 render test builds `Picker::halfblocks()`, which reports a 10×20
//! cell and draws ordinary glyphs. That never reaches `render_kitty_virtual` or the
//! chrome-band cache, so uploads, re-uploads, evictions and placements — the machinery
//! every v6 graphics regression this cycle lived in — had no coverage at all, and the
//! only way to find a fault in it was to play the game and look.
//!
//! The protocol traffic is invisible in the rendered `Buffer` (under kitty the cells
//! carry placeholder glyphs, not pixels), so these tests assert on the op log
//! `GraphicsRender` records as it works: what was uploaded, what was merely re-placed,
//! what was dropped, and what ended up on screen.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::render::graphics::{kitty_picker, GraphicsTarget};
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn picts() -> PictSource {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    PictSource::new(blorb::resolve_resource_blorb(&p).map(|(b, _)| b))
}

/// Arthur: a full-screen ornate border around an inset story viewport, i.e. the
/// hybrid ring at its most demanding — several bands, one of which is the header
/// art that vanished after a restore (SQ-0587).
fn boot() -> Option<GameSession> {
    let p = stories_dir().join("arthur-r74-s890714.z6");
    let bytes = std::fs::read(&p).ok()?;
    let mut pic = picts();
    let dims = pic.all_pict_dims();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, pic.std_window(), None, None)
        .expect("Arthur (v6) boots");
    s.set_pict_source(Some(pic));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    let _ = s.submit("look");
    let _ = s.take_transcript();
    Some(s)
}

/// An app state wired for a HYBRID v6 render through a kitty terminal at a 14×28
/// cell — a real kitty cell size, not halfblocks' 10×20. The pixel path is
/// sensitive to it: art scales by pixel while text places by cell.
fn kitty_state() -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(kitty_picker(14, 28));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state
}

const PANE: Rect = Rect { x: 0, y: 0, width: 100, height: 40 };

fn frame(session: &GameSession, state: &app::state::AppState) {
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    app::render::screen::render_story_pane(&model, false, None, state, PANE, &mut buf);
}

fn bands(set: &std::collections::BTreeSet<GraphicsTarget>) -> Vec<GraphicsTarget> {
    set.iter().filter(|t| matches!(t, GraphicsTarget::Band(..))).copied().collect()
}

#[test]
fn the_kitty_ring_uploads_its_bands_and_places_every_one() {
    let Some(session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let state = kitty_state();
    frame(&session, &state);

    let gr = state.graphics_render.borrow();
    let uploaded = bands(&gr.uploaded_targets());
    let placed = bands(&gr.placed_targets());
    assert!(
        !uploaded.is_empty(),
        "the first hybrid frame under kitty uploads chrome bands; op log:\n{:#?}",
        gr.ops()
    );
    for t in &uploaded {
        assert!(
            placed.contains(t),
            "{t:?} was uploaded but never placed — pixels sent to the terminal that \
             nothing puts on screen; op log:\n{:#?}",
            gr.ops()
        );
    }
}

/// The SQ-0587 invariant, and the one a cache is most likely to break: a band that
/// is a cache HIT must still be PLACED. Skipping the placement leaves the terminal
/// holding an image that nothing points at, which is exactly how Arthur's header art
/// went missing while the cache believed it was still on screen.
#[test]
fn an_unchanged_frame_re_places_every_band_without_re_uploading() {
    let Some(session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let state = kitty_state();
    frame(&session, &state);
    let first = bands(&state.graphics_render.borrow().placed_targets());
    assert!(!first.is_empty(), "first frame places bands");

    frame(&session, &state);

    let gr = state.graphics_render.borrow();
    assert!(
        bands(&gr.uploaded_targets()).is_empty(),
        "an unchanged frame re-uploads nothing (every band is a cache hit); op log:\n{:#?}",
        gr.ops()
    );
    assert_eq!(
        bands(&gr.placed_targets()),
        first,
        "...but still PLACES every band it placed before — a cache hit that skips the \
         placement loses the image while the cache believes it is on screen"
    );
}

/// What a terminal clear does (`main.rs` on resize): drop the band cache. Every band
/// must then be sent again — the terminal no longer has the pixels, so a cache that
/// merely believed itself fresh would leave the ring blank.
#[test]
fn invalidating_the_band_cache_makes_every_band_upload_again() {
    let Some(session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let state = kitty_state();
    frame(&session, &state);
    let placed = bands(&state.graphics_render.borrow().placed_targets());

    state.graphics_render.borrow_mut().invalidate_chrome_bands();
    frame(&session, &state);

    let gr = state.graphics_render.borrow();
    assert_eq!(
        bands(&gr.uploaded_targets()),
        placed,
        "after the cache is dropped every band re-uploads, not just re-places; op log:\n{:#?}",
        gr.ops()
    );
}

/// The SQ-0587 fix itself: when the ring's band set changes shape, some bands are
/// evicted and the rest survive — but a graphics protocol releases an image's
/// PLACEMENT when the image goes, so a survivor that is merely a cache hit would
/// re-place nothing and its art disappears while the cache calls it fresh. Evicting
/// one band must therefore make every other band re-upload, not just re-place.
#[test]
fn evicting_one_band_makes_the_survivors_re_upload_too() {
    let Some(session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let state = kitty_state();
    frame(&session, &state);
    let placed = bands(&state.graphics_render.borrow().placed_targets());
    assert!(placed.len() >= 2, "the ring has several bands to evict between: {placed:?}");

    // Everything EXCEPT the first band stays live — the shape change a restore and
    // its first move produce.
    let live: std::collections::HashSet<app::render::graphics::BandKey> = placed
        .iter()
        .skip(1)
        .map(|t| match *t {
            GraphicsTarget::Band(x, y, w, h) => (app::render::graphics::BandSlot::Art as u8, x, y, w, h),
            GraphicsTarget::Window(_) | GraphicsTarget::Raster => unreachable!("filtered to bands"),
        })
        .collect();
    state.graphics_render.borrow_mut().retain_chrome_bands(&live);
    frame(&session, &state);

    let gr = state.graphics_render.borrow();
    assert_eq!(
        bands(&gr.uploaded_targets()),
        placed,
        "every band re-uploads after ONE was evicted — a survivor left as a cache hit \
         re-places nothing and its art is gone; op log:\n{:#?}",
        gr.ops()
    );
}

/// The field scenario from SQ-0587, end to end and through the real archive: restore,
/// then MOVE (which changes the palette), and the ring must still be on screen. The
/// frame right after a restore is exactly when everything still looks correct, so the
/// assertion is on the frame after the move.
#[test]
fn the_ring_is_still_placed_after_a_restore_and_a_move() {
    let Some(session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let state = kitty_state();
    frame(&session, &state);
    let before = bands(&state.graphics_render.borrow().placed_targets());
    assert!(!before.is_empty(), "the ring is on screen before saving");

    let mapper = mapper::mapper::Mapper::default();
    let es = app::engine::Engine::save_state(&session);
    let path = std::env::temp_dir().join(format!("arthur-kitty-{}.babelmap", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&session.machine.screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(),
            location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &[], &[], &[], &[], &[], &[], &[],
        &session.pictures_png(),
        None,
    )
    .expect("save archive");
    let ac = app::archive::load_archive(&path).expect("load archive");
    let _ = std::fs::remove_file(&path);

    let mut fresh = boot().expect("fresh boot");
    app::engine::Engine::restore_state(&mut fresh, &ac.engine_save()).expect("restore");
    app::session::restore_screen(&mut fresh, ac.screen.clone().expect("screen"));
    fresh.load_pictures_png(&ac.pictures);
    fresh.set_pict_source(Some(picts()));

    let restored = kitty_state();
    frame(&fresh, &restored);

    // The move that recolours the palette — the frame that used to lose the ring.
    let r = fresh.submit("e");
    assert!(!r.quit && r.fault.is_none(), "\"e\" faulted/quit");
    let _ = fresh.take_transcript();
    frame(&fresh, &restored);

    let gr = restored.graphics_render.borrow();
    let after = bands(&gr.placed_targets());
    assert!(
        !after.is_empty(),
        "the ring is still placed one move after a restore — the frame where the \
         surround art used to vanish; op log:\n{:#?}",
        gr.ops()
    );
}
