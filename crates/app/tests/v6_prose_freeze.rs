//! SQ-0697: prose freezes where it was painted when its window moves.
//!
//! ZMSD §15 is explicit that `move_window`/`window_size` "do not change the
//! current display": text already printed stays as pixels where it was drawn. A
//! scrolling window is no exception, and Shogun's opening turns on it — the game
//! prints its whole title header while window 0 is the full 640x400 screen, then
//! moves window 0 down to a 548x64 box beside its menu and prints "You may choose
//! to:" there. A real interpreter leaves the header painted up top. babelmap
//! streamed both halves into one transcript, so they came out adjacent and the
//! header scrolled out of a four-row box.
//!
//! Measured from the screen ops (`trace_screen`), one turn, in order:
//!
//! ```text
//!   @set_cursor(row=49, col=297, window=0)   … nine centred header lines
//!   @move_window(win=0, y=33,  x=47)         <- the freeze fires here
//!   @window_size(win=0, y=368, x=548)
//!   @move_window(win=0, y=337, x=47)
//!   @window_size(win=0, y=64,  x=548)
//!   @erase_window(lower)                     … clears the NEW box only
//!   @set_cursor(row=1, col=1, window=0)
//!   "You may choose to:"
//! ```
//!
//! So the engine shadows what a wrap+scroll window streams, and hands that shadow
//! to `ZWindow::texts` — real paint — the moment the window's box changes. The
//! host answers with a `TranscriptElem::ScreenClear` at the same point in the
//! turn's output: the frozen half stays in scrollback, the live screen restarts
//! at the window's new origin.
//!
//! The corpus guard matters as much as the fix: Zork Zero, Arthur and Journey all
//! move window 0 during play, and freezing on every move would eat their
//! transcripts. They do it right after an `erase_window`, which empties the
//! shadow, so nothing is ever retired for them — asserted below over 25 turns of
//! real play each.
//!
//! Stories are gitignored (CLAUDE.md), so each case skips cleanly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind, TranscriptElem};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(name: &str) -> Option<GameSession> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        dims,
        picts.std_window(),
        None,
        None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    Some(s)
}

/// Window 0's painted runs, as `(y, x, text)`.
fn win0_runs(session: &GameSession) -> Vec<(u16, u16, String)> {
    session
        .machine
        .screen
        .v6
        .as_ref()
        .map(|v6| v6.windows[0].retired.iter().map(|t| (t.y, t.x, t.text.clone())).collect())
        .unwrap_or_default()
}

/// Shogun's title header freezes at the pixel columns the game declared.
///
/// Falsified by making `ZWindow::retire_streamed` a no-op: "Shogun's title header
/// must freeze where it was painted when window 0 moves down to the menu — window
/// 0 carries 0 painted run(s)".
#[test]
fn shogun_title_header_freezes_where_it_was_painted() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    let result = match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };

    let runs = win0_runs(&session);
    assert!(
        !runs.is_empty(),
        "Shogun's title header must freeze where it was painted when window 0 moves down to \
         the menu — window 0 carries {} painted run(s)",
        runs.len()
    );

    // The game centres every line itself, by cursor position: it reads window 0's
    // width (640) and its own row, then set_cursors to the exact column. Those
    // columns are what the freeze has to preserve — a transcript cannot carry them.
    let header: Vec<(u16, u16, &str)> =
        runs.iter().map(|(y, x, t)| (*y, *x, t.as_str())).collect();
    assert!(
        header.contains(&(49, 297, "SHOGUN")),
        "the title lands at its declared column (8px/char centring of 6 chars in 640px = 297): \
         {header:?}"
    );
    assert!(
        header.contains(&(65, 257, "A Story of Japan")),
        "…and every line below it keeps its own: {header:?}"
    );

    // Row spacing is one 16px text row per line, so the block reads as a paragraph
    // and not as nine independently-placed labels.
    let mut rows: Vec<u16> = header.iter().map(|(y, _, _)| *y).collect();
    rows.sort_unstable();
    rows.dedup();
    assert_eq!(
        rows,
        vec![49, 65, 81, 97, 113, 129, 145, 161, 177],
        "the frozen header keeps the game's own 16px row grid"
    );

    // The frozen half is BEFORE the boundary; the prompt the game printed at the
    // window's new origin is after it.
    let mut before = String::new();
    let mut after = String::new();
    let mut seen_clear = false;
    for e in &result.transcript_elems {
        match e {
            TranscriptElem::ScreenClear => seen_clear = true,
            TranscriptElem::Text { text, .. } => {
                if seen_clear {
                    after.push_str(text)
                } else {
                    before.push_str(text)
                }
            }
            TranscriptElem::Image(_) => {}
        }
    }
    assert!(seen_clear, "the turn carries a screen-clear boundary at the freeze");
    assert!(
        before.contains("SHOGUN") && !before.contains("You may choose to"),
        "everything printed before the move stays above the boundary: {before:?}"
    );
    assert_eq!(
        after.trim(),
        "You may choose to:",
        "and the live screen restarts with what the game printed at the new origin"
    );
    // The reported offset is the boundary in the flat transcript; the element
    // split drops the '\n' it lands after (the break IS the element boundary), so
    // the head is exactly one char shorter.
    assert_eq!(
        result.prose_retired,
        Some(before.chars().count() + 1),
        "the reported offset is the boundary itself"
    );
}

/// …and the header reaches the composite, up top, clear of the story box — in
/// both colour modes (the text is the game's own, not a palette preference).
#[test]
fn shogun_frozen_header_reaches_the_composite() {
    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 builds a Layered root") };
    let native = app::render::v6_layout::native_extent(items);
    let layout = app::render::v6_layout::classify_windows(items);

    // The frozen prose publishes as its own paint layer, at the prose's extent —
    // NOT at the window's new box, which would read as an overlay strip sitting
    // inside the story and shove the live transcript out of it.
    let frozen = layout
        .chrome
        .iter()
        .find(|pw| matches!(&pw.node, WinNode::Grid(g) if g.px_texts.iter().any(|t| t.text == "SHOGUN")))
        .expect("the frozen header is published as a paint layer");
    assert!(
        frozen.y_px + frozen.h_px <= layout.story.expect("story window").y_px,
        "the frozen layer sits clear above the story box: frozen y={} h={}, story y={}",
        frozen.y_px,
        frozen.h_px,
        layout.story.unwrap().y_px
    );

    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        if let Some(p) = Engine::paint_surface(&session) {
            *state.v6_paint.borrow_mut() = Some(p);
        }
        let (canvas, _metrics) =
            app::render::screen::build_v6_raster_canvas(&layout, native, &state);

        // Ink on the header's own rows, in the middle third of the screen where
        // the frame art never reaches — so this cannot pass on the border alone.
        let inked = (40..190)
            .flat_map(|y| (200..440).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                canvas.get_pixel_checked(x, y).is_some_and(|p| p[3] == 255 && (p[0], p[1], p[2]) != (0, 0, 0))
            })
            .count();
        assert!(
            inked > 500,
            "honor={honor}: the frozen header must be painted in the composite's upper half; \
             only {inked} inked pixel(s) there"
        );
    }
}

/// The corpus guard, and the reason the freeze is scoped to what the window's new
/// box LEAVES rather than to the box merely changing.
///
/// Zork Zero moves window 0 at boot and again after its title splash, Journey
/// after `split_window(400)`, Arthur on almost every turn of play — its story
/// window is resized around the narration it has just printed. Arthur is the one
/// that bites: freezing whenever the box changed froze its prose seven turns
/// running and stacked its stale `>` prompts up as paint over the churchyard,
/// which broke `v6_arthur_status`'s hybrid cases outright. The window still
/// covers that text, so the text is still the window's own and still belongs to
/// the streaming transcript.
///
/// Both drivers are run: the plain play loop, and the 'n'-at-the-restore-prompt
/// path `v6_arthur_status` uses — the first never reaches Arthur's resize at all,
/// which is exactly how the regression got through the first time.
#[test]
fn a_window_that_still_covers_its_prose_freezes_nothing() {
    for name in [
        "zork0-r393-s890714.z6",
        "arthur-r74-s890714.z6",
        "journey-r83-s890706.z6",
        "advent.z6",
    ] {
        for answer_n in [false, true] {
            let Some(mut session) = boot(name) else { continue };
            for turn in 0..25 {
                let result = match session.pending_input() {
                    InputKind::Char => session.submit_char(13),
                    InputKind::Line if answer_n => session.submit(""),
                    InputKind::Line => session.submit(if turn % 2 == 0 { "look" } else { "wait" }),
                    InputKind::Event => session.submit(""),
                };
                if answer_n && result.transcript.to_lowercase().contains("y or n") {
                    let _ = session.submit_char(b'n');
                }
                assert_eq!(
                    result.prose_retired, None,
                    "{name} turn {turn} (answer_n={answer_n}): nothing may freeze here — this \
                     game's window 0 still covers the prose it printed, so freezing would \
                     restart the transcript under a live session"
                );
                assert!(
                    win0_runs(&session).is_empty(),
                    "{name} turn {turn} (answer_n={answer_n}): window 0 must carry no frozen \
                     paint: {:?}",
                    win0_runs(&session)
                );
            }
        }
    }
}
