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
/// …and the LINE it lands on. Those same `set_cursor` calls declare `row=80` — a
/// 1-based pixel row down a 156px window in a 16px cell, i.e. text line
/// `(80-1)/16 = 4`. The row is honoured for the same reason the column is (SQ-0729,
/// second pass): fmvpoker prints its running totals into WINDOW 0 at absolute
/// y = 247 and 265, which fall in this window's first two lines, so a panel that
/// stacked its own prose from its top edge collided with the game's own layout.
const MENU_LINE: usize = 4;

/// SQ-0729: a v6 prose window is still a pixel surface, and fmvpoker places every
/// menu label on one row with its own `set_cursor`. `ZWindow::push_prose` kept no
/// cursor x, so the five runs butted together into
/// `PLAY CURRENT BETCHANGE CURRENT BETSAVERESTOREQUIT` — inside the VM, before any
/// render code could see the columns. It now pads the line out TO the declared
/// column (not BY it: the run has to land where the game named, not that far past
/// wherever the previous run ended), and pads the BUFFER out to the declared line
/// the same way.
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
        b.lines.get(MENU_LINE).map(String::as_str),
        Some(MENU_ROW),
        "honor={honor}: fmvpoker's five menu labels must reach the model at the columns AND the \
         line its own set_cursor named, not run together and not stacked from the window's top \
         edge (SQ-0729). Lines: {:?}",
        b.lines
    );
    let row: Vec<char> = b.lines[MENU_LINE].chars().collect();
    for &(col, label) in MENU_LABELS {
        let got: String = row.iter().skip(col).take(label.chars().count()).collect();
        assert_eq!(
            got, label,
            "honor={honor}: {label:?} must start at column {col} — the column the game's \
             set_cursor declared for it. Row: {:?}",
            b.lines[MENU_LINE]
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
    let hint = b
        .lines
        .iter()
        .position(|l| l.contains("Select an option"))
        .unwrap_or_else(|| panic!("premise (honor={honor}): the hint line is in that window: {:?}", b.lines));
    // …and its labels arrive at the columns and line the game printed them at.
    assert_eq!(
        b.lines.get(MENU_LINE).map(String::as_str),
        Some(MENU_ROW),
        "premise (honor={honor}): the menu labels keep their declared columns and line"
    );

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The hint line's row in native pixels, one 16px text line per buffer line.
    let (hx, hy) = (menu.x_px as u32 + menu.left_margin as u32, menu.y_px as u32 + hint as u32 * 16);
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

/// Play a hand: past the boot screen, `p` to deal, then a key per card until the
/// game announces the draw. Stops on the frame where its bottom prose window is
/// carrying "You draw (a) …" — the line the player reads to decide what to hold.
fn fmvpoker_dealt_hand(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let (mut session, mut state) = fmvpoker_title(honor)?;
    for step in 0..24 {
        let r = session.submit_char(if step == 0 { b'p' } else { b' ' });
        assert!(r.fault.is_none(), "fmvpoker faulted dealing: {:?}", r.fault);
        app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
        *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        let has_draw = {
            let model = session.screen();
            let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
            app::render::v6_layout::classify_windows(items).chrome.iter().any(|it| {
                matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw")))
            })
        };
        if has_draw {
            return Some((session, state));
        }
    }
    panic!("fmvpoker never announced a draw within 24 keypresses");
}

/// SQ-0729: the story transcript wrote its boot banner straight through
/// "You draw (a) …", the line the player needs in order to see their draw.
///
/// Nothing about the model is wrong. The announcement is window 2's prose and
/// window 2 is already the bottom window, exactly where it belongs. What moves is
/// window 0: it is the whole 640x400 screen here, and once the five dealt cards
/// fill the frame's interior its largest CLEAR rectangle drops onto the very box
/// the game gave window 2. `fill_story_page_under_chrome_text` spared those pixels
/// from the page FILL (SQ-0728) but nothing spared them from the GLYPHS, so
/// `draw_story_text` rasterized "FROBOZZ MAGIC VIDEOPOKER / by Zach Matley / …"
/// over the announcement.
///
/// The rule the fix states: a story transcript is the HOST's re-render of
/// everything window 0 ever printed, while a label another window is holding is on
/// the screen *now* — so where they collide the live label wins.
///
/// Asserted by difference, which is what makes it falsifiable: the rows window 2's
/// own lines occupy must be byte-identical to the same frame rendered with an
/// EMPTY host transcript, since with no transcript there is nothing to overdraw
/// with. The rows below must NOT be identical — the money lines still belong to
/// the transcript and must still reach the screen — so the test cannot pass by
/// suppressing the story text wholesale.
fn fmvpoker_the_draw_announcement_survives_the_transcript(honor: bool) {
    let Some((session, mut state)) = fmvpoker_dealt_hand(honor) else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);

    // Premise: the announcement is in the bottom prose window, not the transcript…
    let panel = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw"))))
        .expect("fmvpoker announces the draw in its bottom prose window");
    let WinNode::Buffer(b) = &panel.node else { unreachable!() };
    let lines = b.lines.len().min((panel.h_px / 16) as usize);
    // …and the host transcript has boot prose of its own to collide with.
    assert!(
        state.transcript.iter().any(|l| l.contains("FROBOZZ MAGIC VIDEOPOKER")),
        "premise (honor={honor}): the transcript carries the boot banner"
    );

    let (img, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The same frame with nothing for `draw_story_text` to draw: every other layer
    // is unchanged, so any difference in these rows IS the transcript overdrawing.
    state.transcript.clear();
    state.transcript_styles.clear();
    state.transcript_runs.clear();
    state.transcript_para.clear();
    state.transcript_images.clear();
    let (bare, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);

    let (x0, x1) = (panel.x_px as u32, (panel.x_px as u32 + panel.w_px as u32).min(img.width()));
    let row_band = |row: usize| -> (u32, u32) {
        let y = panel.y_px as u32 + row as u32 * 16;
        (y, (y + 16).min(img.height()))
    };
    for row in 0..lines {
        let (y0, y1) = row_band(row);
        for y in y0..y1 {
            for x in x0..x1 {
                assert_eq!(
                    img.get_pixel(x, y),
                    bare.get_pixel(x, y),
                    "honor={honor}: at ({x},{y}) the story transcript is drawn over row {row} of \
                     fmvpoker's own bottom window — {:?}. Once the cards fill the frame, window 0's \
                     clear rectangle lands on window 2's box, and the transcript painted the boot \
                     banner through \"You draw (a) …\" (SQ-0729).",
                    b.lines[row]
                );
            }
        }
    }
    // …and sparing those rows must not silence what window 0 prints in the rest of
    // that box: "Current Bet: / 10 / Total Winnings: / 990" belong there too. (They
    // reach the screen as PAINT rather than as transcript — see
    // `fmvpoker_paints_the_runs_it_positions` — so this looks for ink, not for a
    // difference against the bare render.)
    let money = panel.y_px as u32 + 12;
    let mut seen = std::collections::HashSet::new();
    for y in money..(money + 16).min(img.height()) {
        for x in x0..x1 {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the row carrying \"Current Bet:\" and \"Total Winnings:\" is one flat \
         colour ({} seen) — sparing the rows the game's own bottom window holds must not take \
         window 0's own prose off the screen with them",
        seen.len()
    );
}

#[test]
fn fmvpoker_the_draw_announcement_survives_the_transcript_honoring_game_colours() {
    fmvpoker_the_draw_announcement_survives_the_transcript(true);
}

#[test]
fn fmvpoker_the_draw_announcement_survives_the_transcript_theme_only() {
    fmvpoker_the_draw_announcement_survives_the_transcript(false);
}

/// SQ-0729, rule (d): **a story window whose own art ENCLOSES it is a canvas.**
///
/// fmvpoker positions essentially everything it shows. It prints "HOLD" under each
/// card it is holding at `set_cursor(row=203, col=70/183/296/409/522, window=0)`
/// and its running totals at (76,247), (76,265), (420,247) and (420,265) — and
/// every one of those landed in the story SCROLL instead, because window 0 is the
/// host transcript. The player's own reading of it was the right one: there is not
/// supposed to be a scroll window in this game at all.
///
/// What makes it decidable is refusing to ask what a RUN means. A `set_cursor`
/// before a run is genuinely ambiguous — Arthur positions every room headline in
/// window 0 one character at a time, Shogun and Journey centre each header line,
/// mysterious01 re-homes before its prompt, and all of them mean "resume the
/// transcript here" with the identical signal fmvpoker uses for "paint this under
/// that card". Measuring that rule broke Arthur's room names ("CHURCH" came out as
/// "C" painted and "HURCH" streamed). So the question asked instead is what kind of
/// SURFACE the window is: Arthur's window 0 is a transcript that happens to have
/// plates drawn on it, and fmvpoker's is a picture frame that happens to have text
/// positioned in it. Its own art encloses it on all four sides and it covers the
/// whole screen.
///
/// The window then renders as what is SITTING on it — zvm's streamed shadow, which
/// an `erase_window` empties exactly as it empties any painted layer — at the
/// coordinates the game named, and the frame carries no transcript at all.
///
/// Asserted the way that makes it unambiguous: the composite must be BYTE-IDENTICAL
/// with and without a host transcript. A page depends on its transcript; a canvas
/// does not.
fn fmvpoker_paints_the_runs_it_positions(honor: bool) {
    let Some((mut session, mut state)) = fmvpoker_dealt_hand(honor) else { return };
    // …then hold card (a), which is when the game paints its HOLD label.
    let r = session.submit_char(b'a');
    assert!(r.fault.is_none(), "fmvpoker faulted holding a card: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);

    // Premise: this frame's story window is a canvas…
    assert!(
        app::render::screen::story_window_is_a_canvas(&layout, native),
        "premise (honor={honor}): fmvpoker's window 0 covers the screen and its own table art \
         encloses it"
    );
    // …and the runs the game positioned are on it, at the coordinates it named.
    let story = layout.story.expect("fmvpoker publishes a story window");
    let WinNode::Buffer(b) = &story.node else { panic!("window 0 is the primary prose Buffer") };
    let at = |x: u16, y: u16| b.px_runs.iter().find(|t| (t.x, t.y) == (x, y)).map(|t| t.text.clone());
    assert_eq!(
        at(70, 203).as_deref(),
        Some("HOLD"),
        "premise (honor={honor}): the HOLD label the game printed at set_cursor(row=203, col=70) \
         is recorded where it was printed. Runs: {:?}",
        b.px_runs.iter().map(|t| (t.x, t.y, t.text.clone())).collect::<Vec<_>>()
    );
    assert_eq!(
        at(76, 247).as_deref(),
        Some("Current Bet:"),
        "premise (honor={honor}): so is the running total, at set_cursor(row=247, col=76)"
    );
    // And they are a SCREEN, not a log: no two of them claim the same pixels. The
    // running total is reprinted every hand into a window the game erases first —
    // 1000 becomes 990 — and a shadow that kept the erased figure would leave "990"
    // standing on the tail of "1000", reading "9900". An erase covers pixels, so it
    // trims this layer exactly as it trims any other painted one.
    let rect = |t: &app::engine::PxText| {
        let (x, y) = (t.x.max(1) as u32 - 1, t.y.max(1) as u32 - 1);
        (x, y, x + t.text.chars().count() as u32 * 8, y + 16)
    };
    for (i, a) in b.px_runs.iter().enumerate() {
        for c in &b.px_runs[i + 1..] {
            let (ax0, ay0, ax1, ay1) = rect(a);
            let (cx0, cy0, cx1, cy1) = rect(c);
            assert!(
                ax0 >= cx1 || cx0 >= ax1 || ay0 >= cy1 || cy0 >= ay1,
                "honor={honor}: {:?} at ({},{}) and {:?} at ({},{}) are both recorded on the same \
                 pixels — the older one was erased off the screen and its record has to go with \
                 it (SQ-0729)",
                a.text,
                a.x,
                a.y,
                c.text,
                c.x,
                c.y
            );
        }
    }
    // Premise: the host transcript is not empty, so "identical without it" means
    // something.
    assert!(
        state.transcript.iter().any(|l| l.contains("FROBOZZ MAGIC VIDEOPOKER")),
        "premise (honor={honor}): the transcript carries the boot banner"
    );

    let (img, metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    // The HOLD label is really on the screen: its cells are not one flat colour.
    let mut seen = std::collections::HashSet::new();
    for y in 202..218u32 {
        for x in 69..(69 + 4 * 8) {
            seen.insert(img.get_pixel(x, y).0);
        }
    }
    assert!(
        seen.len() >= 2,
        "honor={honor}: the cells under card (a) at (70,203) are one flat colour ({} seen) — the \
         HOLD label the game positioned there is not being painted (SQ-0729)",
        seen.len()
    );
    assert!(
        metrics.is_none(),
        "honor={honor}: a canvas has no scroll — reporting scroll metrics for it puts a pager on \
         a frame with no transcript to page through"
    );

    state.transcript.clear();
    state.transcript_styles.clear();
    state.transcript_runs.clear();
    state.transcript_para.clear();
    state.transcript_images.clear();
    let (bare, _) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
    let differing = img.pixels().zip(bare.pixels()).filter(|(a, b)| a != b).count();
    assert_eq!(
        differing, 0,
        "honor={honor}: {differing} pixels of this frame come from the host transcript. A story \
         window the game has drawn a frame AROUND is a canvas, and what it shows is the runs \
         sitting on it — not a re-render of everything window 0 has ever printed (SQ-0729)"
    );

    // …and the game's two layers do not collide. Painting window 0's runs at the
    // coordinates it named only reads as a fix if the OTHER window's prose is at
    // the coordinates IT named too: fmvpoker prints its bottom panel's lines at
    // `set_cursor(row=80, …)`, five text lines down that panel, while these runs
    // land in its first two lines. Stacking the panel's prose from its top edge
    // instead put "Current Bet: / 10 / Total Winnings: / 990" straight through
    // "You draw (a) an Eight, …" — the same symptom, arrived at from the other side.
    let panel = layout
        .chrome
        .iter()
        .find(|it| matches!(&it.node, WinNode::Buffer(b) if b.lines.iter().any(|l| l.starts_with("You draw"))))
        .expect("fmvpoker announces the draw in its bottom prose window");
    let WinNode::Buffer(pb) = &panel.node else { unreachable!() };
    for (row, line) in pb.lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (ly0, ly1) = (panel.y_px as u32 + row as u32 * 16, panel.y_px as u32 + row as u32 * 16 + 16);
        let (lx0, lx1) = (
            panel.x_px as u32 + panel.left_margin as u32,
            panel.x_px as u32 + panel.w_px as u32,
        );
        for t in &b.px_runs {
            if t.text.trim().is_empty() {
                continue;
            }
            let (ry0, ry1) = (t.y.max(1) as u32 - 1, t.y.max(1) as u32 - 1 + 16);
            let (rx0, rx1) = (t.x.max(1) as u32 - 1, t.x.max(1) as u32 - 1 + t.text.chars().count() as u32 * 8);
            assert!(
                ry0 >= ly1 || ly0 >= ry1 || rx0 >= lx1 || lx0 >= rx1,
                "honor={honor}: window 0's {:?} at ({},{}) lands on row {row} of the bottom \
                 panel, {line:?}. Both are the game's own text and it placed neither of them \
                 there by accident (SQ-0729).",
                t.text,
                t.x,
                t.y
            );
        }
    }
}

#[test]
fn fmvpoker_paints_the_runs_it_positions_honoring_game_colours() {
    fmvpoker_paints_the_runs_it_positions(true);
}

#[test]
fn fmvpoker_paints_the_runs_it_positions_theme_only() {
    fmvpoker_paints_the_runs_it_positions(false);
}

/// The corpus guard on rule (d), and the reason it was worth trying at all: the
/// enclosure test was measured when it was written and fires for ONE title.
///
/// Zork Zero and Shogun keep window 0 inset inside their frames; advent and scopa
/// paint nothing behind it; Arthur's intro plate and Journey's title are solid and
/// answer the FILL arm rather than the enclosure one; mysterious01's plate is a
/// band across the lower half that reaches neither the top edge nor the right one.
/// Every one of them keeps its transcript, which is the whole point — window 0 is
/// the story window of every v6 game.
#[test]
fn fmvpoker_is_the_only_canvas_story_window() {
    let expected: &[(&str, [bool; 4])] = &[
        ("zork0-r393-s890714.z6", [false; 4]),
        ("arthur-r74-s890714.z6", [false; 4]),
        ("shogun-r322-s890706.z6", [false; 4]),
        ("journey-r83-s890706.z6", [false; 4]),
        ("advent.z6", [false; 4]),
        ("scopa.z6", [false; 4]),
        ("mysterious01.z6", [false; 4]),
        // …and fmvpoker, which boots with no art at all and is an ordinary page for
        // that one frame, then draws its table and is a canvas from there.
        ("fmvpoker.z6", [false, true, true, true]),
    ];
    for &(game, want) in expected {
        let Some((mut session, mut state)) = boot(game, true) else { continue };
        let mut got = [false; 4];
        for (step, slot) in got.iter_mut().enumerate() {
            {
                let model = session.screen();
                let WinNode::Layered(items) = &model.root else { panic!("{game}: v6 Layered root") };
                let native = app::render::v6_layout::native_extent(items);
                let layout = app::render::v6_layout::classify_windows(items);
                *slot = app::render::screen::story_window_is_a_canvas(&layout, native);
            }
            if step == 3 {
                break;
            }
            let r = match session.pending_input() {
                InputKind::Char => session.submit_char(13),
                _ => session.submit(""),
            };
            assert!(r.fault.is_none(), "{game} faulted at step {step}: {:?}", r.fault);
            app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
            *state.v6_paint.borrow_mut() = Engine::paint_surface(&session);
        }
        assert_eq!(
            got, want,
            "{game}: which frames read as a CANVAS changed. Rule (d) turns off a v6 title's whole \
             transcript, so a game that starts qualifying loses its narration (SQ-0729)."
        );
    }
}
