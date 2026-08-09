//! SQ-0729: fmvpoker drew NO graphics at all in hybrid, and its menu never
//! reached the pixel composite.
//!
//! **A ring around a full-screen window has nowhere to draw.** Hybrid shows
//! artwork as bands AROUND the story viewport, so a story window covering the
//! screen leaves `chrome_bands` empty and no art can be uploaded whatever its
//! shape. That is what `picture_takeover` exists to catch — such a frame goes to
//! the composite instead — but it asked whether the art FILLED the screen, sampling
//! an 8x8 grid and requiring every point painted. fmvpoker paints a 640x400 poker
//! table into full-screen window 0 and prints its whole title inside it, and that
//! table is a FRAME: 42830 of 256000 pixels opaque, the middle a hole. It missed at
//! every point that mattered, hybrid kept its empty ring, and the game drew not one
//! picture. The rule now also fires when the art ENCLOSES the screen — painted
//! pixels within a native text row of all four edges.
//!
//! `fmvpoker_is_the_only_title_this_moves` is the guard on that. Hybrid is the
//! shipped default, so a frame that stops taking the ring renders as a pixel image
//! instead of crisp cells — a mode change every v6 player would see.
//!
//! **The composite skipped a whole class of window.** Routing those frames here
//! exposed the second half: `build_chrome_canvas` draws Graphics and Grid windows
//! and never a secondary prose `Buffer` (SQ-0585). fmvpoker prints its bottom menu
//! and "Select an option with your mouse or by typing the first letter." into one,
//! so raster showed neither while both cell paths showed both — and hybrid would
//! have lost them on arrival. They are drawn now, one 16px row each from the
//! window's own origin, in ink the caller resolves against `honor_game_colours`
//! (painting fmvpoker's declared black regardless put black glyphs on the host's
//! black page), and `fill_story_page_under_chrome_text` spares them.
//!
//! **The menu labels kept their columns** (SQ-0729, second pass). They used to
//! arrive from the model already concatenated, as
//! `lines[0] == "PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT"`: the game
//! prints them at five separate columns, but a v6 window in flowing-prose mode
//! streams through `ZWindow::push_prose`, which kept no cursor x, and that window's
//! `texts` and `streamed` are both empty — so the positions were lost inside zvm
//! before any render code saw them. `push_prose` now takes the column its own
//! `set_cursor` declared and pads the line out to it, the same declaration the
//! streaming path already carried as an indent. `fmvpoker_menu_labels_keep_their
//! _columns` pins the result against the game's own `set_cursor` operands.
//!
//! `stories/fmvpoker.blb` is a byte-identical copy of `stories/Zork0.blb`: the
//! original release ships Zork Zero's picture file renamed to FMVPOKER.EG1, so the
//! table is a Zork Zero picture drawn deliberately. Both `honor_game_colours` modes
//! are pinned. Stories are gitignored (CLAUDE.md), so these skip cleanly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str, honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut s));
    Some((s, state))
}

/// Render one hybrid frame and report the path it took.
fn render_path(session: &GameSession, state: &app::state::AppState) -> String {
    let model = session.screen();
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, state, area, &mut buf);
    state.v6_path_log.borrow().last().map(|(l, _)| l.clone()).unwrap_or_default()
}

/// fmvpoker one keypress in: the table is drawn and the title printed inside it.
fn fmvpoker_title(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let (mut session, mut state) = boot("fmvpoker.z6", honor)?;
    let r = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    assert!(r.fault.is_none(), "fmvpoker faulted: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    // The game's own painted ground, as the app publishes it every frame
    // (`main.rs`): fmvpoker's `erase_window` fills live here and nowhere else.
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    Some((session, state))
}

fn fmvpoker_hybrid_draws_its_frame(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };

    // Premise: window 0 covers the screen — so hybrid's ring is empty — and the
    // backdrop behind it is mostly HOLE, the shape the fill test misses.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items);
    let story = layout.story.expect("fmvpoker publishes a story window");
    assert_eq!(
        (story.x_px, story.y_px, story.w_px, story.h_px),
        (0, 0, 640, 400),
        "premise (honor={honor}): window 0 covers the whole screen, so there is no ring to carve"
    );
    let plate = layout.story_gfx.expect("fmvpoker draws its table into window 0 (SQ-0714)");
    let WinNode::Graphics(g) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    let (pw, ph) = g.canvas.dimensions();
    let opaque = g.canvas.pixels().filter(|p| p.0[3] != 0).count();
    assert_eq!((pw, ph), (640, 400), "premise (honor={honor}): a full-screen backdrop");
    assert!(
        opaque * 3 < (pw * ph) as usize,
        "premise (honor={honor}): the table is a hollow frame — {opaque} of {} pixels painted",
        pw * ph
    );

    assert_eq!(
        render_path(&session, &state),
        "raster",
        "honor={honor}: with window 0 covering the screen the hybrid ring has no band to draw in, \
         so this frame must go to the composite — otherwise fmvpoker draws NO graphics at all \
         (SQ-0729). Its table encloses the screen; it does not fill it."
    );
}

#[test]
fn fmvpoker_hybrid_draws_its_frame_honoring_game_colours() {
    fmvpoker_hybrid_draws_its_frame(true);
}

#[test]
fn fmvpoker_hybrid_draws_its_frame_theme_only() {
    fmvpoker_hybrid_draws_its_frame(false);
}

/// The corpus guard: the enclosure arm is meant to move fmvpoker and nothing else.
///
/// Zork Zero and Shogun keep window 0 inset inside their frames, advent and scopa
/// paint nothing behind it, Arthur's intro plate and Journey's title already filled
/// the screen, and mysterious01's plate is a band across the lower half that
/// reaches neither the top edge nor the right one.
#[test]
fn fmvpoker_is_the_only_title_this_moves() {
    const RING: &str = "hybrid-ring";
    const MENU: &str = "cell — painted menu takeover routed here";
    let expected: &[(&str, &[&str])] = &[
        ("zork0-r393-s890714.z6", &[RING; 4]),
        ("arthur-r74-s890714.z6", &["raster", RING, RING, RING]),
        ("shogun-r322-s890706.z6", &["raster", MENU, RING, RING]),
        ("journey-r83-s890706.z6", &[RING, "raster", RING, RING]),
        ("advent.z6", &[RING, MENU, RING, RING]),
        ("scopa.z6", &["painted (hint/menu takeover)"; 4]),
        ("mysterious01.z6", &[RING; 4]),
        // …and fmvpoker, which boots with no art at all and takes the ring for that
        // one frame, then draws its table and must go to the composite from there.
        ("fmvpoker.z6", &[RING, "raster", "raster", "raster"]),
    ];
    for &(game, want) in expected {
        let Some((mut session, mut state)) = boot(game, true) else { continue };
        let mut got = Vec::new();
        for step in 0..want.len() {
            got.push(render_path(&session, &state));
            if step + 1 == want.len() {
                break;
            }
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        }
        assert_eq!(
            got,
            want.to_vec(),
            "{game}: the hybrid render path per frame changed. The SQ-0729 enclosure arm of \
             `picture_takeover` is meant to move fmvpoker alone — a title that stops taking the \
             ring renders as a pixel image instead of crisp terminal cells."
        );
    }
}

/// fmvpoker's bottom menu row, as the game lays it out.
///
/// Derived, not guessed: on the frame under test the story runs
/// `@set_window(win2)` and then `@set_cursor(row=80, col=C, window=2)` for
/// C = 0, 178, 372, 454, 557 — five 1-based pixel columns in an 8px fixed-cell
/// font, i.e. character columns 0, 22, 46, 56 and 69 (`(C-1)/8`, clamped at the
/// left margin). The labels are 16, 18, 4, 7 and 4 characters, so every one of
/// them ENDS short of the next column: the gaps are the game's, and a renderer
/// that concatenates the runs is losing information the story supplied.
const MENU_LABELS: &[(usize, &str)] = &[
    (0, "PLAY CURRENT BET"),
    (22, "CHANGE CURRENT BET"),
    (46, "SAVE"),
    (56, "RESTORE"),
    (69, "QUIT"),
];
const MENU_ROW: &str =
    "PLAY CURRENT BET      CHANGE CURRENT BET      SAVE      RESTORE      QUIT";

/// SQ-0729: a v6 prose window is still a pixel surface, and fmvpoker places every
/// menu label on one row with its own `set_cursor`. `ZWindow::push_prose` kept no
/// cursor x, so the five runs butted together into
/// `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT` — inside the VM, before any
/// render code could see the columns. It now pads the line out TO the declared
/// column (not BY it: the run has to land where the game named, not that far past
/// wherever the previous run ended).
fn fmvpoker_menu_labels_keep_their_columns(honor: bool) {
    let Some((session, _state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let layout = app::render::v6_layout::classify_windows(items);
    let menu = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
        .expect("fmvpoker publishes its menu as a secondary prose window (SQ-0585)");
    let WinNode::Buffer(b) = &menu.node else { unreachable!() };

    assert_eq!(
        b.lines[0], MENU_ROW,
        "honor={honor}: fmvpoker's five menu labels must reach the model at the columns its own \
         set_cursor named, not run together (SQ-0729)"
    );
    let row: Vec<char> = b.lines[0].chars().collect();
    for &(col, label) in MENU_LABELS {
        let got: String = row.iter().skip(col).take(label.chars().count()).collect();
        assert_eq!(
            got, label,
            "honor={honor}: {label:?} must start at column {col} — the column the game's \
             set_cursor declared for it. Row: {:?}",
            b.lines[0]
        );
    }
    // The window is 594px / 74 characters wide, so nothing is being bought with
    // padding the game did not ask for.
    assert!(
        row.len() <= (menu.w_px / 8) as usize,
        "honor={honor}: the menu row ({} chars) must still fit the window the game declared ({} \
         columns)",
        row.len(),
        menu.w_px / 8
    );
}

#[test]
fn fmvpoker_menu_labels_keep_their_columns_honoring_game_colours() {
    fmvpoker_menu_labels_keep_their_columns(true);
}

#[test]
fn fmvpoker_menu_labels_keep_their_columns_theme_only() {
    fmvpoker_menu_labels_keep_their_columns(false);
}

/// The composite carries a secondary prose window's lines. Asserted as ink on the
/// hint line's row: a row that reached the screen shows more than one colour.
fn fmvpoker_composite_shows_its_menu_window(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);

    // Premise: the menu is a NON-PRIMARY Buffer holding those lines…
    let menu = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
        .expect("fmvpoker publishes its menu as a secondary prose window (SQ-0585)");
    let WinNode::Buffer(b) = &menu.node else { unreachable!() };
    assert!(
        b.lines.iter().any(|l| l.contains("Select an option")),
        "premise (honor={honor}): the hint line is in that window: {:?}",
        b.lines
    );
    // …and its labels arrive at the columns the game printed them at (SQ-0729).
    assert_eq!(
        b.lines[0], MENU_ROW,
        "premise (honor={honor}): the menu labels keep their declared columns"
    );

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The hint line's row in native pixels: the window's third line.
    let (hx, hy) = (menu.x_px as u32 + menu.left_margin as u32, menu.y_px as u32 + 2 * 16);
    let mut seen = std::collections::HashSet::new();
    for y in hy..(hy + 16).min(img.height()) {
        for x in hx..(hx + 63 * 8).min(img.width()) {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the hint line's row is one flat colour ({} seen) — the composite draws \
         Graphics and Grid windows but not a secondary prose Buffer, so fmvpoker's menu bar and \
         its \"Select an option…\" hint never reach the screen (SQ-0729)",
        seen.len()
    );
}

#[test]
fn fmvpoker_composite_shows_its_menu_window_honoring_game_colours() {
    fmvpoker_composite_shows_its_menu_window(true);
}

#[test]
fn fmvpoker_composite_shows_its_menu_window_theme_only() {
    fmvpoker_composite_shows_its_menu_window(false);
}

/// The "Double Fanucci" banner, and what actually happens to it (SQ-0729).
///
/// The frame art is Zork Zero's — fmvpoker ships that picture file renamed to
/// FMVPOKER.EG1 — so its top-centre tab natively reads "Double Fanucci", a title
/// belonging to a different game. The reading under investigation was that fmvpoker
/// overprints it with a title of its own and we fail to draw that text. It does not:
/// traced from the game itself, the boot frame parks WINDOW 1 at (173,7) 289x34,
/// exactly over the banner, `erase_window`s it to the blue it declared for that
/// window, and never prints a single character into it for the rest of the session.
/// The banner is not overwritten, it is ERASED — which is how a game hides a title
/// that is not its own, and there is no fmvpoker title to draw.
///
/// What WAS wrong is the colour of the hole. The erase reached the host correctly as
/// a painted-ground fill (SQ-0706), but `fill_story_page_under_chrome_text` then
/// flooded window 0's whole box on top of it — window 0 is the entire 640x400 screen
/// here — so the tab rendered as a white gash across the top of an otherwise
/// complete blue frame. That is the "the top-centre tab is cut off at y=0" this quest
/// carried for three passes as a clipping question; the art is not clipped at all.
/// The page is now the OLDEST thing in the box, sparing the game's own fills exactly
/// as it already spares chrome text.
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named(honor: bool) {
    let Some((session, state)) = fmvpoker_title(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);

    // Premise: window 1 is parked over the banner and the game printed NOTHING into
    // it. If a later fmvpoker really does put its own title there, this is the
    // assertion that notices — and the theory this test refutes becomes true.
    let tab = layout
        .chrome
        .iter()
        .find(|it| (it.x_px, it.y_px, it.w_px, it.h_px) == (172, 6, 289, 34))
        .expect("premise: fmvpoker parks window 1 at native (173,7) 289x34, over the banner");
    let WinNode::Grid(g) = &tab.node else { panic!("window 1 is published as a Grid") };
    assert!(
        g.px_texts.is_empty() && g.cells.iter().all(|c| c.ch == '\0' || c.ch == ' '),
        "premise (honor={honor}): fmvpoker prints nothing into the window it parks over the \
         \"Double Fanucci\" banner — it erases the title rather than overwriting it"
    );

    // Premise: the erase cut the banner out of the ARTWORK — the plate is a hole
    // there — and reached us as painted ground in the colour window 1 declared. So
    // what shows at those pixels comes from under the art, and is the game's fill or
    // nothing.
    let plate = layout.story_gfx.expect("fmvpoker draws its table into window 0 (SQ-0714)");
    let WinNode::Graphics(gfx) = &plate.node else { panic!("story_gfx is a Graphics leaf") };
    let paint = Engine::paint_surface(&session).expect("premise: the erase_window fill is recorded");
    let ground = *paint.get_pixel(300, 20);
    assert_ne!(ground[3], 0, "premise (honor={honor}): the banner's pixels were filled by the game");

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    for (x, y) in [(300u32, 20u32), (180, 10), (450, 36)] {
        assert_eq!(
            gfx.canvas.get_pixel(x, y).0[3],
            0,
            "premise (honor={honor}): at ({x},{y}) the erase cut the \"Double Fanucci\" banner out \
             of the plate, so nothing of the artwork remains to be clipped"
        );
        assert_eq!(
            *paint.get_pixel(x, y),
            ground,
            "premise (honor={honor}): ({x},{y}) is inside the rectangle the game filled"
        );
        assert_eq!(
            *img.get_pixel(x, y),
            ground,
            "honor={honor}: at ({x},{y}) the banner must keep the colour fmvpoker's own \
             erase_window painted there, not window 0's page. The story page is the oldest thing \
             in its box; a fill the game issued afterwards is newer, and flooding over it renders \
             the frame's top-centre tab as a white gash (SQ-0729)."
        );
    }
}

#[test]
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named_honoring_game_colours() {
    fmvpoker_erased_banner_keeps_the_colour_the_game_named(true);
}

#[test]
fn fmvpoker_erased_banner_keeps_the_colour_the_game_named_theme_only() {
    fmvpoker_erased_banner_keeps_the_colour_the_game_named(false);
}
