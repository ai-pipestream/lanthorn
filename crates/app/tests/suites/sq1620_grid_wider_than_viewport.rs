//! SQ-1620 — a v6 Hybrid pane narrow enough (relative to its own font size)
//! that the story-slot Grid's cell-quantized viewport is narrower than the
//! grid's own native column count must fall through to the raster composite,
//! not silently clip the grid's right-hand column.
//!
//! # The bug
//!
//! Zork Zero's Amiga InvisiClues menu (`hint`, `y`) puts its topic list in a
//! promoted story-slot `Grid`, 58 native columns wide (SQ-0934/SQ-1599). At a
//! 59-column pane with a 20x44px cell, `native_viewport_box`'s inward
//! rounding computes a 43-column viewport — `hybrid_story_slot_grid` and the
//! terminal's own hybrid-ring draw (`draw_grid_transparent`) both clip to
//! `viewport.width.min(grid.cols)` — so the whole right-hand column of
//! topics ("AS A LAST RESORT", "FOR YOUR AMUSEMENT") silently vanishes, with
//! no ring geometry on the frame able to show it. Shogun's InvisiClues menu
//! (`shogun-r322-s890706.z6`, 62-column grid) has the identical shape and is
//! driven here too, matching CLAUDE.md's rule that a defect shown on one v6
//! hint-menu title must be looked for on the other (SQ-0934).
//!
//! # The fix
//!
//! `screen::story_slot_grid_wider_than_viewport` (`crates/app/src/render/screen.rs`)
//! detects this — a story-slot `Grid` whose own `cols` exceed the viewport
//! [`build_hybrid_frame_with`] would actually place it at, on a pane that is
//! UPSCALING the native screen (`scale.s >= 1.0`; the pane genuinely has more
//! device-pixel resolution than the clip credits it with — see that
//! function's own doc for why a DOWNSCALING pane, like
//! `v6_hint_menu_mouse.rs`'s own pinned 50-column case, is deliberately
//! excluded) — and is OR'd into `picture_takeover_reason`'s own answer at the
//! three places that ask it: the terminal's real hybrid-ring dispatch,
//! [`hybrid_chrome_layout`], and [`hybrid_story_slot_grid`], so a host
//! reading either published function and the terminal's own draw agree. The
//! takeover then routes straight to the raster composite (never the
//! chrome-runs-only "painted screen" shortcut, which would drop the grid's
//! text entirely rather than merely clip a column of it — see
//! `hybrid_raster_fallback_reason`'s own `"grid_wider_than_viewport"` special
//! case), which renders the WHOLE native canvas as one scaled image with no
//! per-column clipping at all.
//!
//! # Specimens
//!
//! Reuses `v6_hint_menu_mouse.rs`'s own proven boot sequence (`hint`, then
//! `y`), not re-derived (CLAUDE.md).
//!
//! | fixture                   | release | to the menu                        |
//! |----------------------------|---------|-------------------------------------|
//! | `zork0-r393-s890714.z6`   | 393     | `hint`, then `y` — 2 inputs         |
//! | `shogun-r322-s890706.z6`  | 322     | Enter past the splash, `hint`, `y`  |
//!
//! Skip-if-missing (gitignored stories, symlink `stories/` into a worktree
//! per CLAUDE.md), and non-vacuous: present fixtures that yield no check fail
//! rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::render::screen::{build_v6_raster_canvas, hybrid_chrome_layout, hybrid_story_slot_grid, render_story_pane};
use app::render::v6_layout as v6;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a press to its hint/InvisiClues topic menu, or `None` when the
/// gitignored story is absent — `v6_hint_menu_mouse.rs`'s own `hint_menu`,
/// reused rather than re-derived.
fn hint_menu(file: &str, release: u16) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let bytes = std::fs::read(&path).ok()?;
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), release, "{file} is not the pinned release");
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
        .expect("the press should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    // Zork Zero asks for a LINE first; Shogun holds a title splash on a CHAR read.
    for _ in 0..8 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    s.submit("hint");
    let entered = s.submit_char(b'y');
    assert!(entered.fault.is_none(), "{file}: entering the hint menu faulted: {:?}", entered.fault);
    Some(s)
}

/// The narrow pane the quest reports: `Rect{x:9,y:4,w:41,h:20}` was the
/// original research's own device-pixel account of the viewport this
/// specimen computes; a 59x51-cell pane at a 20x44px cell (kitty's own
/// convention for a large font) reproduces it — measured here at
/// `frame.scale.s == 1.84375`, an UPSCALE, which is the property that fires
/// the fix (see `story_slot_grid_wider_than_viewport`'s own doc).
const NARROW: (u16, u16) = (59, 51);
const NARROW_CELL: (u16, u16) = (20, 44);

/// `v6_hint_menu_mouse.rs`'s own two pinned panes, reused verbatim rather
/// than re-derived — both must stay on today's `hybrid-ring` path.
const WIDE: (u16, u16) = (190, 60);
const SMALL: (u16, u16) = (50, 60);
const REGRESSION_CELL: (u16, u16) = (8, 16);

fn state_at(cell_px: (u16, u16), honor: bool) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(app::render::graphics::kitty_picker(cell_px.0, cell_px.1));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = honor;
    st
}

/// The story-slot Grid's own right-hand-column runs this bug drops —
/// per-title, since the two games' topic lists read differently.
fn right_column_runs(file: &str) -> &'static [&'static str] {
    if file.starts_with("zork0") {
        &["AS A LAST RESORT", "FOR YOUR AMUSEMENT"]
    } else {
        &["As a Last Resort (Part II)", "Have you tried?"]
    }
}

/// Does the raster composite's canvas actually paint ink (non-uniform
/// pixels, i.e. glyph strokes over the page) in `run`'s own native bounding
/// box? A genuine check on the RENDERED pixels — not a metadata flag — that
/// the grid's own text reached the canvas, the same shape `sq1618`'s own
/// pixel-recipe checks take.
fn run_is_painted(canvas: &image::RgbaImage, run: &app::engine::PxText) -> bool {
    let x0 = run.x as u32;
    let y0 = run.y as u32;
    let w = (run.text.chars().count() as u32 * 8).min(canvas.width().saturating_sub(x0));
    let h = 16u32.min(canvas.height().saturating_sub(y0));
    if w == 0 || h == 0 {
        return false;
    }
    let mut min = [255u8; 3];
    let mut max = [0u8; 3];
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let px = canvas.get_pixel(x, y).0;
            for c in 0..3 {
                min[c] = min[c].min(px[c]);
                max[c] = max[c].max(px[c]);
            }
        }
    }
    // A blank page samples uniform; real glyph strokes over it do not.
    (0..3).any(|c| max[c].saturating_sub(min[c]) > 40)
}

const SPECIMENS: &[(&str, u16)] = &[("zork0-r393-s890714.z6", 393), ("shogun-r322-s890706.z6", 322)];

#[test]
fn narrow_pane_falls_to_raster_and_shows_the_whole_grid() {
    let mut ran = 0;
    for &(file, release) in SPECIMENS {
        let Some(s) = hint_menu(file, release) else {
            eprintln!("SKIP: gitignored story missing at {}", stories_dir().join(file).display());
            continue;
        };
        ran += 1;

        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{file}: a v6 frame has a Layered root") };
        let cell = zvm::screen::V6Cell::DEFAULT;
        let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(cell));
        let layout = v6::classify_windows(items, cell);

        // Premise: the hint screen's story slot really is a Grid (SQ-1026's
        // own specimen), and it is wide enough to reproduce the bug.
        let Some(story) = layout.story else { panic!("{file}: expected a story window") };
        let WinNode::Grid(g) = &story.node else { panic!("{file}: premise — the hint screen's story slot must be a Grid") };
        assert!(g.cols >= 58, "{file}: premise — the topic-list grid must be at least 58 native columns wide, got {}", g.cols);

        // 1. The published host APIs agree: no ring, no story-slot grid
        // placement, at the narrow pane.
        let st = state_at(NARROW_CELL, true);
        let pane = Rect::new(0, 0, NARROW.0, NARROW.1);
        assert!(
            hybrid_story_slot_grid(&layout, native, pane, NARROW_CELL, &st).is_none(),
            "{file} {NARROW:?}/{NARROW_CELL:?}: hybrid_story_slot_grid must publish no placement — \
             the grid is wider than the viewport it would compute"
        );
        assert!(
            hybrid_chrome_layout(&layout, native, pane, NARROW_CELL, &st).is_none(),
            "{file} {NARROW:?}/{NARROW_CELL:?}: hybrid_chrome_layout must publish no ring either — \
             a host must not draw chrome around a grid it cannot place"
        );

        // 2. The REAL terminal dispatch: falls all the way through to the
        // raster composite, not the chrome-runs-only painted-screen shortcut
        // (which would drop the grid's text entirely) and not the ring
        // (which would clip it).
        let mut buf = Buffer::empty(Rect::new(0, 0, pane.right() + 1, pane.bottom() + 1));
        let _ = render_story_pane(&model, false, None, &st, pane, &mut buf);
        assert_eq!(
            st.v6_path_log.borrow().last().map(|(l, _)| l.clone()),
            Some("raster".into()),
            "{file} {NARROW:?}/{NARROW_CELL:?}: a story-slot grid wider than its viewport must fall \
             through to the raster composite"
        );
        assert_eq!(
            st.v6_takeover_reason.get(),
            Some("grid_wider_than_viewport"),
            "{file} {NARROW:?}/{NARROW_CELL:?}: the takeover must be published as SQ-1620's own reason"
        );

        // 3. The regression check: the raster composite genuinely PAINTS the
        // right-hand column's own text — the exact content the ring's clip
        // used to drop — not merely that the route changed.
        let (canvas, _) = build_v6_raster_canvas(&layout, native, &st);
        let mut checked = 0;
        for needle in right_column_runs(file) {
            let run = g
                .px_texts
                .iter()
                .find(|t| t.text.trim() == *needle)
                .unwrap_or_else(|| panic!("{file}: premise — the grid must carry a run reading {needle:?}"));
            assert!(
                run_is_painted(&canvas, run),
                "{file}: the raster composite must paint {needle:?} at its native bbox \
                 ({},{}) — the right-hand column this bug used to drop",
                run.x,
                run.y
            );
            checked += 1;
        }
        assert_eq!(checked, right_column_runs(file).len(), "{file}: every right-column specimen run must be checked");

        // 4. Regression guard: `v6_hint_menu_mouse.rs`'s own two pinned panes
        // are completely unaffected — still resolve to the ring, exactly as
        // before this fix (an UPSCALING pane wide enough to fit the grid, and
        // a DOWNSCALING pane that cannot show it all regardless of technique).
        for &(w, h) in &[WIDE, SMALL] {
            let st2 = state_at(REGRESSION_CELL, true);
            let pane2 = Rect::new(0, 0, w, h);
            let mut buf2 = Buffer::empty(Rect::new(0, 0, pane2.right() + 1, pane2.bottom() + 1));
            let _ = render_story_pane(&model, false, None, &st2, pane2, &mut buf2);
            assert_eq!(
                st2.v6_path_log.borrow().last().map(|(l, _)| l.clone()),
                Some("hybrid-ring".into()),
                "{file} {w}x{h}/{REGRESSION_CELL:?}: must still take the ring, unaffected by SQ-1620's fix"
            );
            assert_eq!(
                st2.v6_takeover_reason.get(),
                None,
                "{file} {w}x{h}/{REGRESSION_CELL:?}: must publish no takeover, unaffected by SQ-1620's fix"
            );
        }
    }
    if stories_dir().join(SPECIMENS[0].0).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}

/// SQ-1623 — a host that draws its own text (never the terminal) can opt out
/// of the raster fallback above via `AppState::host_shrinks_story_grid_text`
/// and get back a `Grid` placement plus the grid's true native size instead,
/// so it can shrink its own glyphs to fit. Reuses the identical narrow-pane
/// frame `narrow_pane_falls_to_raster_and_shows_the_whole_grid` pins above —
/// same specimens, same `NARROW`/`NARROW_CELL` pane, same `hint_menu` boot.
#[test]
fn host_opt_in_publishes_grid_instead_of_raster_fallback() {
    let mut ran = 0;
    for &(file, release) in SPECIMENS {
        let Some(s) = hint_menu(file, release) else {
            eprintln!("SKIP: gitignored story missing at {}", stories_dir().join(file).display());
            continue;
        };
        ran += 1;

        let model = s.screen();
        let WinNode::Layered(items) = &model.root else { panic!("{file}: a v6 frame has a Layered root") };
        let cell = zvm::screen::V6Cell::DEFAULT;
        let native = v6::native_extent(items, &app::native_font::TextFace::cell_only(cell));
        let layout = v6::classify_windows(items, cell);
        let Some(story) = layout.story else { panic!("{file}: expected a story window") };
        let WinNode::Grid(g) = &story.node else { panic!("{file}: premise — the hint screen's story slot must be a Grid") };

        let pane = Rect::new(0, 0, NARROW.0, NARROW.1);

        // 1. Default (`host_shrinks_story_grid_text` left at `false`):
        // bit-for-bit the SQ-1620 behaviour — both functions still answer
        // `None`. This is the regression guard that the default truly
        // changes nothing.
        let st_default = state_at(NARROW_CELL, true);
        assert!(
            !st_default.host_shrinks_story_grid_text,
            "{file}: premise — the flag must default to false"
        );
        assert!(
            hybrid_story_slot_grid(&layout, native, pane, NARROW_CELL, &st_default).is_none(),
            "{file} {NARROW:?}/{NARROW_CELL:?}: with the opt-in left off, hybrid_story_slot_grid must \
             still publish no placement, exactly as SQ-1620 left it"
        );
        assert!(
            hybrid_chrome_layout(&layout, native, pane, NARROW_CELL, &st_default).is_none(),
            "{file} {NARROW:?}/{NARROW_CELL:?}: with the opt-in left off, hybrid_chrome_layout must \
             still publish no ring, exactly as SQ-1620 left it"
        );

        // 2. Opted in: both functions now answer `Some`, and the grid's own
        // native size (`grid_cols`/`grid_rows`) is wider than the pane's
        // actual viewport — the fact a host is meant to notice and shrink to.
        let mut st_opt_in = state_at(NARROW_CELL, true);
        st_opt_in.host_shrinks_story_grid_text = true;

        let chrome = hybrid_chrome_layout(&layout, native, pane, NARROW_CELL, &st_opt_in);
        assert!(
            chrome.is_some(),
            "{file} {NARROW:?}/{NARROW_CELL:?}: with the opt-in on, hybrid_chrome_layout must publish \
             a ring for a story-slot grid wider than the viewport"
        );

        let grid = hybrid_story_slot_grid(&layout, native, pane, NARROW_CELL, &st_opt_in)
            .unwrap_or_else(|| panic!("{file} {NARROW:?}/{NARROW_CELL:?}: with the opt-in on, hybrid_story_slot_grid must publish a placement"));
        assert_eq!(
            grid.grid_cols, g.cols,
            "{file}: grid_cols must reflect the story's true native grid width"
        );
        assert_eq!(
            grid.grid_rows, g.rows,
            "{file}: grid_rows must reflect the story's true native grid height"
        );
        assert!(
            grid.grid_cols > grid.viewport.width,
            "{file}: grid_cols ({}) must exceed the narrow pane's own viewport.width ({}) — \
             this is the mismatch a host must notice and shrink its glyphs to close",
            grid.grid_cols,
            grid.viewport.width
        );

        // 3. The terminal's own dispatch (`render_story_pane`) is completely
        // unaffected by the flag either way — it never reads it and keeps
        // falling back to raster regardless, which is the "terminal keeps
        // its raster fallback" half of SQ-1623.
        for (label, st) in [("flag off", &st_default), ("flag on", &st_opt_in)] {
            let mut buf = Buffer::empty(Rect::new(0, 0, pane.right() + 1, pane.bottom() + 1));
            let _ = render_story_pane(&model, false, None, st, pane, &mut buf);
            assert_eq!(
                st.v6_path_log.borrow().last().map(|(l, _)| l.clone()),
                Some("raster".into()),
                "{file} {NARROW:?}/{NARROW_CELL:?} ({label}): the terminal's own dispatch must still \
                 fall through to raster regardless of the host opt-in"
            );
            assert_eq!(
                st.v6_takeover_reason.get(),
                Some("grid_wider_than_viewport"),
                "{file} {NARROW:?}/{NARROW_CELL:?} ({label}): the terminal's takeover reason must be \
                 unaffected by the host opt-in"
            );
        }
    }
    if stories_dir().join(SPECIMENS[0].0).exists() {
        assert!(ran > 0, "the fixtures are present but nothing ran — check the filenames");
    }
}
