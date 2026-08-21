//! Where a v6 status WINDOW ends, and what the hybrid ring may draw there —
//! SQ-0948 and SQ-0949, which are one defect seen from its two sides.
//!
//! A v6 frame puts a wide status window between two columns of border artwork, and
//! its edge almost never falls on a terminal cell boundary. Hybrid draws the window
//! with CELLS and the artwork with an IMAGE, so that one native column is claimed
//! twice — and whichever of the two claims it paints its own ground over the other's
//! pixels. The two reports are the two directions:
//!
//! * **SQ-0948, the band reaching in.** `stories/shogun-r322-s890706.z6` (release
//!   322, serial 890706, IBM PC), six taps into play at a 117x40 kitty terminal. The
//!   status window is `548x32` at native `(46, 0)`; the left flank band's last
//!   terminal cell inverts to native `44.5..50.1`, so four columns of the window's
//!   PAGE sat inside the flank's own image. The strip drew that ribbon 36 device px
//!   tall — two whole cells — while the band drew the same page 46 px tall, its true
//!   32 native rows, and the 10-pixel difference reached the screen as a 6x10 white
//!   block hanging below each end of the score bar. The user reported it twice; the
//!   first time it was dismissed as a harness artifact, which it was not.
//!
//! * **SQ-0949, the strip reaching out.** `stories/arthur-r74-s890714.z6` (release
//!   74, serial 890714, IBM PC, art from the Blorb beside it), twelve taps in. Its
//!   status window is `584x16` at native `(28, 192)` — 91% of the screen, wide enough
//!   that `full_width_flood_rows` calls the row a bar, and 28 native columns short of
//!   each edge, which is exactly where Arthur's poles stand. The flank veto read "a
//!   bar owns the row" as "a bar owns the PANE", so the strip's ground flooded across
//!   both poles and the ring carved each flank into a piece above the ribbon and a
//!   piece below it. That seam is the step the report calls the side strip not lining
//!   up with the panel above it.
//!
//! **The reference machines settle which side is right**, and neither shows a bar
//! that reaches the screen's edge. `machine-screenshots/dos-arthur.png` — the EGA
//! press at the Churchyard, "Merlin disappears as suddenly as he came" — puts the
//! white ribbon at native 28..610 and the grey pole rule beside it at native
//! 6.5..8.7, unbroken from the panel's foot to the bottom of the screen.
//! `machine-screenshots/mac-arthur.jpg` is the same moment with the black ribbon
//! inset and the green poles at one constant x above and below it.
//!
//! Both stories are gitignored, so every case here skips cleanly when absent.
//!
//! **Colour mode**: both `honor_game_colours` settings are swept. The page fill
//! SQ-0948 turns on is gated on that flag, so a single-mode case would pin only half
//! of it.
//!
//! **Palette: stated, not inherited** (SQ-0958). Each case installs the profile's
//! own table under `app::v6_palette`, which takes `V6_PALETTE_LOCK` in the same call.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way `startup.rs` does — the profile from the medium the mount
/// returned, and the screen size through the whole `std_window → native_std_window →
/// profile` chain — then tap `taps` times, answering `n` to a restore question.
///
/// A harness that skips `native_std_window` measures a screen the player never sees
/// (CLAUDE.md); both fixtures here are 320x200 presses drawn at art scale (2,2), so
/// the chain matters even though neither is a disk image.
fn boot(name: &str, keys: u8, taps: usize) -> Option<GameSession> {
    let path = stories_dir().join(name);
    let (bytes, medium) = match app::hints::load_mounted_story(&path) {
        Ok((loaded, medium)) => (loaded.bytes().to_vec(), medium),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, None, medium);
    let mut picts = PictSource::resolve(&path, None);
    let dims = picts.all_pict_dims();
    let std_win =
        picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    let mut session = GameSession::new_with_art_scale(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        dims,
        std_win,
        art_scale,
        profile.default_colours(),
        None,
        None,
    )
    .expect("the story should boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..taps {
        let t = match session.pending_input() {
            InputKind::Line => session.submit("").transcript,
            InputKind::Char => session.submit_char(keys).transcript,
            InputKind::Event => session.submit("").transcript,
        };
        if t.to_lowercase().contains("y or n") {
            let _ = session.submit_char(b'n');
        }
    }
    Some(session)
}

/// The chrome GRID window that carries `needle`, as native `(x, y, w, h)` — the
/// status window, found by its own text rather than by a hard-coded rect.
fn status_window(model: &app::engine::ScreenModel, needle: &str) -> Option<(u16, u16, u16, u16)> {
    let WinNode::Layered(items) = &model.root else { return None };
    items.iter().find_map(|pw| {
        let WinNode::Grid(g) = &pw.node else { return None };
        // Joined, because a game with proportional metrics emits one run per GLYPH —
        // Arthur's band arrives as `C` `h` `u` … and no single run contains the word.
        let joined: String = g.px_texts.iter().map(|t| t.text.as_str()).collect();
        joined.contains(needle).then_some((pw.x_px, pw.y_px, pw.w_px, pw.h_px))
    })
}

/// A hybrid render at a real kitty cell (8x18). `Picker::halfblocks()` reports a 1x2
/// cell, a layout regime in which no sub-cell boundary exists to fall on.
#[allow(deprecated)]
fn render(model: &app::engine::ScreenModel, honor: bool, cols: u16, rows: u16) -> (Rect, Buffer) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    let mut picker =
        ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18));
    picker.set_protocol_type(ratatui_image::picker::ProtocolType::Kitty);
    state.game_picker = Some(picker);
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    (area, buf)
}

/// A cell the ring drew as part of an uploaded band: a kitty virtual placeholder.
fn is_band_cell(c: &ratatui::buffer::Cell) -> bool {
    c.symbol().starts_with('\u{10eeee}')
}

// ── SQ-0948: a flank band may not carry a page the cells draw ────────────────

/// Shogun's chrome canvas carries NO pixel of the status window's page on the rows
/// the ring draws with glyphs — so the flank band beside it cannot ship one.
///
/// The band is the only thing that draws at sub-cell resolution, so a page it carries
/// is a second rendering of a ribbon the strip has already drawn: at the window's true
/// 32 native rows rather than the strip's two whole cells, and the ten device pixels
/// of difference are the block hanging below each end of the score bar. Measured on
/// the canvas the bands are cut from, since the cell layer cannot tell a page inside a
/// band from artwork.
///
/// Self-falsifying: the same canvas built with `TextLayer::All` — what raster wants,
/// and what hybrid used to pass — must still carry the page, or this case is asserting
/// nothing.
#[test]
fn shoguns_chrome_canvas_keeps_the_status_page_off_the_ring() {
    let _g = app::v6_palette(InterpreterProfile::IbmPc.palette());
    let Some(session) = boot("shogun-r322-s890706.z6", 13, 6) else { return };
    let model = session.screen();
    let Some((sx, sy, sw, sh)) = status_window(&model, "Score:") else {
        panic!("six taps in, Shogun's status window carries `Score:` — the frame this case is about")
    };
    // The shape this case depends on: a status window whose left edge is native 46,
    // which at a 115-column pane is column 8.27 and so falls INSIDE the flank's last
    // cell. Without this guard a frame that had moved the window would pass vacuously.
    assert_eq!(
        (sx, sy, sw, sh),
        (46, 0, 548, 32),
        "the specimen is the status window at native (46,0) 548x32; its edge landing \
         inside a terminal cell is the whole of SQ-0948"
    );

    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items.as_slice());
    let layout = app::render::v6_layout::classify_windows(items.as_slice());
    // The rows the ring claims: Shogun prints its band at native y 1 and 17, so their
    // tops are 0 and 16 and between them they are the window's whole 32 rows.
    let glyph_rows: std::collections::HashSet<u16> = layout
        .chrome
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().map(|t| t.y.max(1) - 1)),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        glyph_rows.contains(&0) && glyph_rows.contains(&16),
        "the ring draws both of the band's native rows with glyphs; got {glyph_rows:?}"
    );

    // The game's own erase of that window, as the app records it. MEASURED from the
    // running binary two turns in: `state.v6_paint` is a 548x368 surface — sized to
    // the STORY window — carrying the status window's fill in SCREEN coordinates, so
    // the rectangle the game erased at native (46,0) 548x32 is clipped at native 548
    // and never reaches the right flank. That asymmetry is why the page half of this
    // fix cured one side of the score bar and left the other.
    let mut paint = image::RgbaImage::new(548, 368);
    for y in 0..32u32 {
        for x in 46..548u32 {
            paint.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }

    let colors = app::colors::ColorScheme::terminal_default();
    let build = |text: app::render::v6_layout::TextLayer<'_>| {
        let mut canvas = app::render::v6_layout::build_chrome_canvas(
            &layout.chrome,
            native,
            image::Rgba([255, 255, 255, 255]),
            image::Rgba([0, 0, 0, 255]),
            &colors,
            text,
        );
        // The ring's own order (SQ-0706): art and glyphs, then the painted ground
        // beneath them, then the window pages filling whatever neither claimed.
        app::render::v6_layout::blit_paint_ground(&mut canvas, Some(&paint), text);
        app::render::v6_layout::fill_window_pages(&mut canvas, &layout.chrome, layout.story, &colors, text);
        canvas
    };
    // The four native columns the flank's last cell reaches into, over the window's
    // own rows — the exact pixels that became the white block.
    let leak = |canvas: &image::RgbaImage| -> usize {
        (u32::from(sy)..u32::from(sy) + u32::from(sh))
            .flat_map(|y| (u32::from(sx)..u32::from(sx) + 4).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.get_pixel(x, y)[3] != 0)
            .count()
    };

    let hybrid = build(app::render::v6_layout::TextLayer::SkipGlyphRows(&glyph_rows));
    assert_eq!(
        leak(&hybrid),
        0,
        "on a row the ring draws with GLYPHS the canvas keeps artwork and nothing else \
         (SQ-0750/SQ-0903), and neither a window PAGE nor the ground the game erased \
         under a ribbon is artwork — the strip stamps that ground into its own cells, \
         so a band carrying it draws the same ribbon a second time at the window's \
         true height"
    );
    let raster = build(app::render::v6_layout::TextLayer::All);
    assert_eq!(
        leak(&raster),
        (sh as usize) * 4,
        "…and the raster composite, which has no cells to draw the ribbon with, must \
         still get every pixel of that page — if it does not, the case above is \
         measuring a canvas that never had one"
    );
}
