//! SQ-0716: declining the game's colours must not delete half of a drawing.
//!
//! scopa's felt table used to vanish under `honor_game_colours = false`, leaving a
//! BLACK table carrying the two green bands and the cards the game had drawn onto
//! it — the worst of both readings.
//!
//! Measured from the SCREEN OPS rather than the model, the mechanism is not "a
//! page is a preference, a fill is drawing". scopa's boot is:
//!
//! ```text
//! @set_true_colour(fg=true(0x0000), bg=true(0x0200), window=1)   # explicit green
//! @window_size(win=1, y=400, x=640)                              # the whole screen
//! @erase_window(all(unsplit))
//! @erase_window(upper)                                           # = window 1
//! ```
//!
//! The table is a FILL — the same `erase_window` opcode SQ-0706 declared ungatable
//! when it made the cards survive a declined palette. It only reaches the renderer
//! as a window *page* because `drain_erase_fills` classifies a fill spanning the
//! whole screen as a screen clear and drops it, so window 1's background is the
//! sole surviving record of the paint. Gating that record on the colour flag while
//! leaving the sub-screen fills ungated is what split one drawing in half.
//!
//! The fix keys on the painted ground, the same discriminator SQ-0711 uses: a
//! window with the game's own pixels inside it is a canvas and keeps its page
//! either way; a window with none is presentation and the flag still governs it.
//!
//! The story is gitignored (CLAUDE.md), so these skip cleanly when it is absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot scopa to its title table, optionally dealing a hand. `Engine::set_mouse`
/// takes the coordinates Y FIRST; a click reaches `read_char` as ZSCII 254
/// (ZMSD §3.8) with the coordinates already set.
fn scopa(deal: bool, trace: bool) -> Option<GameSession> {
    let path = stories_dir().join("scopa.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, trace, dims, picts.std_window(), None, None,
    )
    .expect("scopa is a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    if deal {
        for _ in 0..3 {
            match s.pending_input() {
                app::session::InputKind::Char => {
                    Engine::set_mouse(&mut s, 350, 245);
                    let _ = s.submit_char(254);
                }
                _ => {
                    let _ = s.submit("");
                }
            }
            let _ = s.take_transcript();
        }
    }
    Some(s)
}

/// scopa's baize: `set_true_colour` 0x0200 is 15-bit RGB with only G bit 4 set,
/// which resolves to `Rgb(0, 132, 0)`. Matched by shape so an anti-aliased edge
/// cell still counts.
fn is_baize(c: Color) -> bool {
    matches!(c, Color::Rgb(r, g, b) if g >= 100 && r < 90 && b < 90)
}

fn render(session: &GameSession, mode: app::config::V6RenderMode, honor: bool, w: u16, h: u16) -> Buffer {
    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = mode;
    state.config.honor_game_colours = honor;
    *state.v6_paint.borrow_mut() = session.paint_surface();
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    buf
}

/// The premise, from the screen ops: the table is an explicit-colour full-screen
/// `erase_window`, and the painted ground does NOT carry it — so the window page
/// is the only record there is.
#[test]
fn the_table_is_a_full_screen_fill_that_survives_only_as_a_window_page() {
    let Some(session) = scopa(false, true) else { return };
    let mut session = session;
    let ops = Engine::take_screen_trace(&mut session);
    assert!(
        ops.iter().any(|l| l.contains("@set_true_colour") && l.contains("bg=true(0x0200)") && l.contains("window=1")),
        "scopa names its green outright on window 1 — an explicit colour, not an inherited one:\n{ops:#?}"
    );
    assert!(
        ops.iter().any(|l| l == "@window_size(win=1, y=400, x=640)"),
        "…and window 1 is the whole 640x400 screen, which is why its erase is dropped as a clear"
    );
    assert!(
        ops.iter().any(|l| l.starts_with("@erase_window")),
        "…and the table arrives as an erase_window fill"
    );

    // The dropped half: the painted ground has no pixel in the table's corners.
    let paint = session.paint_surface().expect("scopa paints its cards (SQ-0706)");
    assert_eq!(
        paint.get_pixel(0, 0)[3],
        0,
        "the full-screen fill is dropped as a screen clear, so the ground carries no table"
    );

    // …and window 1 still declares that green as its page.
    let model = session.screen();
    let app::engine::WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let layout = app::render::v6_layout::classify_windows(items);
    let table = layout
        .chrome
        .iter()
        .find(|it| it.w_px == 640 && it.h_px == 400)
        .expect("the full-screen table window is published");
    let app::engine::WinNode::Grid(g) = &table.node else { panic!("scopa's screen is Grids") };
    assert_eq!(g.bg, Some(0x0200_0200), "the page is the surviving record of that fill (true-colour 0x0200)");
}

/// The symptom: with the game's colours declined the table must still be there.
///
/// Both render modes, both scopa screens (title and dealt) and four pane sizes —
/// the felt is resolved in native pixels, so no pane size may lose it to
/// cell quantization.
///
/// Falsified by reverting `fill_painted_window_pages` to the plain
/// `if honor { fill_window_pages(..) }` gate:
/// "Hybrid honor=false title pane=100x34: the felt table reaches the pane — 0 of
/// 3400 cells are baize, wanted more than 1700".
#[test]
fn declining_game_colours_keeps_the_felt_table() {
    for (label, deal) in [("title", false), ("dealt", true)] {
        let Some(session) = scopa(deal, false) else { return };
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            for (w, h) in [(100u16, 34u16), (80, 25), (132, 50), (64, 20)] {
                let buf = render(&session, mode, false, w, h);
                let cells = (w as usize) * (h as usize);
                let green = buf.content().iter().filter(|c| is_baize(c.bg)).count();
                assert!(
                    green > cells / 2,
                    "{mode:?} honor=false {label} pane={w}x{h}: the felt table reaches the pane — \
                     {green} of {cells} cells are baize, wanted more than {}",
                    cells / 2
                );
            }
        }
    }
}

/// Half-honoured is the thing that was wrong, so pin the whole: for a game that
/// draws its board and streams no prose, declining its colours changes nothing at
/// all. Every pixel on scopa's screen is its own drawing.
#[test]
fn scopa_looks_the_same_either_way_because_all_of_it_is_drawing() {
    for (label, deal) in [("title", false), ("dealt", true)] {
        let Some(session) = scopa(deal, false) else { return };
        for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
            for (w, h) in [(100u16, 34u16), (80, 25)] {
                let on = render(&session, mode, true, w, h);
                let off = render(&session, mode, false, w, h);
                assert_eq!(
                    on, off,
                    "{mode:?} {label} pane={w}x{h}: scopa's screen is drawing end to end, so the \
                     colour flag has nothing left to decline"
                );
            }
        }
    }
}
