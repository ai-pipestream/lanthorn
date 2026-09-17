//! SQ-0818: the v6 chrome ring's full-width bands go up as column TILES, so a small
//! change re-sends one tile instead of the whole strip.
//!
//! Measured on Zork Zero before this: its banner band was ONE image — 920x126 device
//! pixels, 618 KB of base64 in 151 kitty chunks — and every byte of it went down the
//! wire each time one compass tile changed, eight times over the boot animation
//! (~4.9 MB). Nothing blinked (SQ-0817 saw to that); this is bandwidth, and it is the
//! difference between snappy and sluggish over ssh.
//!
//! The mechanism is granularity, not a protocol trick — neither kitty nor iterm2 can
//! patch pixels into a resident image, so "send only the dirty region" means uploading
//! a smaller image over just those cells. Tiles are byte-identical to the strip they
//! replace because every band crops its sub-rect out of the ONE frame-shared scaled
//! canvas (`chrome_scaled`, SQ-0514).
//!
//! The pane and cell here are the SQ-0817 pty capture's, so these numbers and that
//! capture's flush table describe the same screen. Skip-if-missing per the other
//! gitignored-story smokes.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::render::graphics::{kitty_picker, GraphicsOp, GraphicsTarget};
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

const PANE: Rect = Rect { x: 0, y: 0, width: 117, height: 64 };
const CELL: (u16, u16) = (8, 18);
/// `screen.rs`'s `BAND_TILE_COLS`, restated: a private const, but the partition it
/// produces is the observable contract.
const TILE_COLS: u16 = 8;

fn boot_zork0() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(story_bytes, true, false, None, false, dims, picts.std_window(), None, None)
        .expect("Zork0 (v6) boots");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "Zork0 booted clean");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// A hybrid v6 state under `proto`, at the capture's cell.
fn state_for(honor: bool, proto: ratatui_image::picker::ProtocolType) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker = kitty_picker(CELL.0, CELL.1);
    picker.set_protocol_type(proto);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

fn frame(session: &GameSession, state: &app::state::AppState) {
    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    app::render::screen::render_story_pane(&model, false, None, state, PANE, &mut buf);
}

/// Reap the background band worker (SQ-1188), the way the app's loop tick does.
/// A changed band's `Upload` op lands HERE, in the change frame's own op epoch —
/// the next frame's `begin_band_log` would clear it — so a case measuring what a
/// change sent settles between the change frame and the read.
fn settle(state: &app::state::AppState) {
    for _ in 0..500 {
        let _ = state.graphics_render.borrow_mut().poll_v6_job();
        if !state.graphics_render.borrow().band_encode_in_flight() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("band encodes never settled");
}

/// The banner's bands: the ring's TOP strip, which is the one this quest is about.
///
/// Identified by sitting on the pane's first row AND not being pane-tall. The
/// original selector was `y == PANE.y` alone, on the reasoning that "the side
/// flanks start below it" — true while the ring was `pane − viewport`, and false
/// since SQ-0894 built it from content: the flanks now begin on the pane's first
/// row too, and own the corners the banner used to.
fn banner(set: &std::collections::BTreeSet<GraphicsTarget>) -> Vec<(u16, u16, u16, u16)> {
    let mut v: Vec<_> = set
        .iter()
        .filter_map(|t| match *t {
            GraphicsTarget::Band(x, y, w, h) if y == PANE.y && h < PANE.height => Some((x, y, w, h)),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

/// The full-height side flanks — one object per side since SQ-0894.
fn flank_bands(set: &std::collections::BTreeSet<GraphicsTarget>) -> Vec<(u16, u16, u16, u16)> {
    let mut v: Vec<_> = set
        .iter()
        .filter_map(|t| match *t {
            GraphicsTarget::Band(x, y, w, h) if h >= PANE.height => Some((x, y, w, h)),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

/// The columns the banner strip must span: exactly the gap the two flanks leave.
///
/// Stronger than the `== PANE.width` it replaces — it asserts the ring's pieces
/// ABUT, which is the property the seam SQ-0894 removed used to violate.
fn banner_span(set: &std::collections::BTreeSet<GraphicsTarget>) -> (u16, u16) {
    let fl = flank_bands(set);
    let left = fl.iter().find(|b| b.0 == PANE.x).map(|b| b.0 + b.2).unwrap_or(PANE.x);
    let right = fl.iter().find(|b| b.0 > PANE.x).map(|b| b.0).unwrap_or(PANE.right());
    (left, right)
}

/// Cells of banner pixels the frame actually SENT — the quest's whole quantity, in the
/// only unit the op log carries. One cell is `CELL.0 * CELL.1 * 4` bytes of RGBA before
/// base64, so cells and bytes are the same number scaled.
fn banner_cells_uploaded(ops: &[GraphicsOp]) -> u32 {
    ops.iter()
        .filter_map(|o| match *o {
            GraphicsOp::Upload { target: GraphicsTarget::Band(_, y, _, _), cells, .. } if y == PANE.y => {
                Some(cells.0 as u32 * cells.1 as u32)
            }
            _ => None,
        })
        .sum()
}

fn reused_banner(ops: &[GraphicsOp]) -> Vec<(u16, u16, u16, u16)> {
    let mut v: Vec<_> = ops
        .iter()
        .filter_map(|o| match *o {
            GraphicsOp::Reuse { target: GraphicsTarget::Band(x, y, w, h), .. } if y == PANE.y => Some((x, y, w, h)),
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Drive the boot animation to the first playable turn — the state the pty capture's
/// `wait:400` keys reach.
fn to_first_turn(session: &mut GameSession, state: &app::state::AppState) {
    for _ in 0..8 {
        frame(session, state);
        let _ = session.submit("");
        let _ = session.take_transcript();
    }
    frame(session, state);
    // Boot-time band encodes land before any case measures a LATER frame
    // (SQ-1188): a straggler installing mid-assertion would count as that
    // frame's upload.
    settle(state);
}

/// The partition proof, and it is the whole safety argument: N tiles are pixel-identical
/// to the strip they replace only if their x-spans tile the strip's EXACTLY — no gap
/// (an unwritten column), no overlap (two images fighting over one cell), every tile at
/// least one cell wide, and every tile at the strip's own y and height.
#[test]
fn the_banner_tiles_partition_the_strip_exactly() {
    let Some(session) = boot_zork0() else { return };
    for honor in [true, false] {
        let state = state_for(honor, ratatui_image::picker::ProtocolType::Kitty);
        frame(&session, &state);
        let tiles = banner(&state.graphics_render.borrow().placed_targets());

        assert!(tiles.len() > 1, "honor={honor}: the banner strip is tiled, not one image: {tiles:?}");
        let (span_l, span_r) = banner_span(&state.graphics_render.borrow().placed_targets());
        assert_eq!(
            tiles.len(),
            (span_r - span_l).div_ceil(TILE_COLS) as usize,
            "honor={honor}: {}-cell strip in {TILE_COLS}-column tiles: {tiles:?}",
            span_r - span_l
        );
        let (_, y0, _, h0) = tiles[0];
        // The strip starts where the left flank ends, not at the pane edge (SQ-0894).
        let mut x = span_l;
        for &(tx, ty, tw, th) in &tiles {
            assert_eq!(tx, x, "honor={honor}: no gap and no overlap at column {x}: {tiles:?}");
            assert!((1..=TILE_COLS).contains(&tw), "honor={honor}: tile width {tw} is in 1..={TILE_COLS}: {tiles:?}");
            assert_eq!((ty, th), (y0, h0), "honor={honor}: every tile keeps the strip's rows: {tiles:?}");
            x += tw;
        }
        assert_eq!(x, span_r, "honor={honor}: the tiles reach the strip's right edge: {tiles:?}");
    }
}

/// The quest, in one assertion. Zork Zero's banner carries a compass that redraws when
/// the player moves; before tiling, that one 45x40-pixel change re-encoded and re-sent
/// all 117 columns of the strip.
#[test]
fn a_one_tile_change_re_uploads_that_tile_and_reuses_the_rest() {
    if boot_zork0().is_none() {
        return;
    }
    for honor in [true, false] {
        // Each colour mode gets its own boot: the first frame under a new state
        // uploads everything, and the assertion below is about a LATER frame.
        let mut session = boot_zork0().expect("story present");
        let state = state_for(honor, ratatui_image::picker::ProtocolType::Kitty);
        to_first_turn(&mut session, &state);
        let tiles = banner(&state.graphics_render.borrow().placed_targets());
        let strip_cells: u32 = tiles.iter().map(|&(_, _, w, h)| w as u32 * h as u32).sum();

        // An idle frame sends nothing at all — the baseline the change is measured from.
        frame(&session, &state);
        assert!(
            banner(&state.graphics_render.borrow().uploaded_targets()).is_empty(),
            "honor={honor}: an unchanged frame re-uploads no tile; op log:\n{:#?}",
            state.graphics_render.borrow().ops()
        );

        let _ = session.submit("n");
        let _ = session.take_transcript();
        frame(&session, &state);
        // SQ-1188: the changed tile's encode runs on the worker; its Upload op
        // lands in this frame's op epoch when reaped. The unchanged tiles'
        // Reuse ops are already there — one epoch carries the whole story.
        settle(&state);

        let gr = state.graphics_render.borrow();
        let uploaded = banner(&gr.uploaded_targets());
        let reused = reused_banner(gr.ops());
        // THE SYMPTOM, stated in pixels rather than in images, because "one upload" is
        // true of the untiled strip too — it was always one upload, of everything. A
        // move redraws a compass a handful of columns wide; the strip must not follow
        // it down the wire.
        let sent = banner_cells_uploaded(gr.ops());
        assert!(
            sent * 4 <= strip_cells,
            "honor={honor}: moving redraws one compass tile, so at most a quarter of the \
             banner's {strip_cells} cells go down the wire — this frame sent {sent}, i.e. \
             {}% of the strip; op log:\n{:#?}",
            sent * 100 / strip_cells.max(1),
            gr.ops()
        );
        assert_eq!(
            uploaded.len(),
            1,
            "honor={honor}: moving redraws ONE tile of the banner, so exactly one tile's \
             pixels go down the wire — not all {} of them; uploaded {uploaded:?}; op log:\n{:#?}",
            tiles.len(),
            gr.ops()
        );
        assert!(tiles.contains(&uploaded[0]), "honor={honor}: {uploaded:?} is one of the banner's tiles {tiles:?}");
        // Only the BANNER's tiles are the subject; since SQ-0894 the reuse set also
        // carries the two full-height flanks, which are one object per side and not
        // tiles of this strip.
        let reused: Vec<_> = reused.into_iter().filter(|b| tiles.contains(b)).collect();
        assert_eq!(
            reused.len(),
            tiles.len() - 1,
            "honor={honor}: every other tile is a cache hit — re-placed, not re-sent; \
             reused {reused:?}; op log:\n{:#?}",
            gr.ops()
        );
        // …and every tile is still on screen. A cache hit that skips its placement
        // loses the image while the cache believes it is up (SQ-0587).
        assert_eq!(banner(&gr.placed_targets()), tiles, "honor={honor}: every tile is re-placed");
    }
}

/// Granularity is BACKEND-CONDITIONAL. Every sixel image re-declares its own palette,
/// so N tiles would mean N palettes where the strip had one — a real first-frame
/// regression bought for a redraw win. Sixel keeps the single strip.
#[test]
fn sixel_keeps_the_strip_whole() {
    let Some(session) = boot_zork0() else { return };
    for honor in [true, false] {
        let state = state_for(honor, ratatui_image::picker::ProtocolType::Sixel);
        frame(&session, &state);
        let tiles = banner(&state.graphics_render.borrow().placed_targets());
        assert_eq!(
            tiles.len(),
            1,
            "honor={honor}: sixel uploads the banner as ONE image — a palette per tile is \
             a first-frame regression: {tiles:?}"
        );
        let (l, r) = banner_span(&state.graphics_render.borrow().placed_targets());
        assert_eq!((tiles[0].0, tiles[0].0 + tiles[0].2), (l, r),
            "honor={honor}: and it spans the whole strip, abutting both flanks: {tiles:?}");
    }
}

/// Halfblocks draws glyphs, not pixels: ratatui's own cell diff already sends only the
/// cells that changed, so tiling would buy it nothing and it is left alone.
#[test]
fn halfblocks_keeps_the_strip_whole() {
    let Some(session) = boot_zork0() else { return };
    for honor in [true, false] {
        let state = state_for(honor, ratatui_image::picker::ProtocolType::Halfblocks);
        frame(&session, &state);
        let tiles = banner(&state.graphics_render.borrow().placed_targets());
        assert_eq!(tiles.len(), 1, "honor={honor}: halfblocks keeps the banner as one band: {tiles:?}");
        let (l, r) = banner_span(&state.graphics_render.borrow().placed_targets());
        assert_eq!((tiles[0].0, tiles[0].0 + tiles[0].2), (l, r),
            "honor={honor}: spanning the whole strip, abutting both flanks: {tiles:?}");
    }
}
