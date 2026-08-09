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

/// SQ-0717 — reported by a player as "some of shogun intro text is no longer
/// centered", minutes after the freeze landed. The centring survives in EVERY
/// render path, which is the assertion that would have caught it at merge time:
/// the previous guard checked only the raster composite, and raster is the one path
/// that was never wrong.
///
/// Measured, Shogun's title, before the fix — lines losing their centring:
///   raster     0 of 9   (the composite paints the frozen layer as pixels)
///   hybrid     6 of 9   ← the shipped default, and what the player saw
///   frameless  6 of 9   (identical to hybrid)
///
/// Both cell paths route text above the story through the frameless status band,
/// which sorts each run into a LEFT/CENTER/RIGHT anchor group by where it STARTS —
/// the right question for a status field, the wrong one for a paragraph. Shogun's
/// nine lines are centred by the game's own cursor arithmetic, so the five longest
/// begin left of the left-third boundary (208px of a 640px screen) and the shortest
/// ends past the right two-thirds: five flushed to the left margin, one to the
/// right. A run with equal margins on the game's screen was centred on purpose and
/// stays centred in the pane.
///
/// Falsified by reverting the `centred` exemption in `draw_anchored_status_band`:
/// "hybrid honor=true 80x25: \"Copyright (c) 1988 by Infocom\" keeps the centring
/// the game gave it (at col 0, want ~25)".
#[test]
fn shogun_frozen_header_stays_centred_in_every_render_path() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    let Some(mut session) = boot("shogun-r322-s890706.z6") else { return };
    match session.pending_input() {
        InputKind::Char => session.submit_char(13),
        _ => session.submit(""),
    };
    let model = session.screen();

    // Every line of the header, exactly as the game centred it.
    let lines = [
        "SHOGUN",
        "A Story of Japan",
        "Copyright (c) 1988 by Infocom",
        "All rights reserved.",
        "SHOGUN is a trademark of James Clavell",
        "Original Literary Work Copyright 1975 by James Clavell",
        "Licensed by Noble House Trading Limited, London.",
        "Release 322 / Pix 322 / Serial number 890706",
        "IBM Interpreter version 6.65",
    ];

    for (tag, mode) in [
        ("hybrid", app::config::V6RenderMode::Hybrid),
        ("frameless", app::config::V6RenderMode::Frameless),
    ] {
        for honor in [true, false] {
            for (w, h) in [(80u16, 25u16), (120, 40)] {
                let mut state = app::state::AppState::default();
                state.colors = app::colors::ColorScheme::terminal_default();
                state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
                state.config.v6_render = mode;
                state.config.honor_game_colours = honor;
                let area = Rect::new(0, 0, w, h);
                let mut buf = Buffer::empty(area);
                let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
                let row_text = |y: u16| -> String {
                    (0..w).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
                };
                let rows: Vec<String> = (0..h).map(row_text).collect();
                let ctx = format!("{tag} honor={honor} {w}x{h}");

                for line in lines {
                    let at = rows
                        .iter()
                        .find_map(|r| r.find(line))
                        .unwrap_or_else(|| panic!("{ctx}: the frozen header line {line:?} reaches the pane:\n{}", rows.join("\n")));
                    let want = (w as usize - line.chars().count()) / 2;
                    assert!(
                        (at as i32 - want as i32).abs() <= 1,
                        "{ctx}: {line:?} keeps the centring the game gave it (at col {at}, want ~{want}):\n{}",
                        rows.join("\n")
                    );
                }

                // The block still reads as one paragraph: consecutive rows, in order.
                let first = rows.iter().position(|r| r.contains("SHOGUN")).expect("header rows located above");
                for (i, line) in lines.iter().enumerate() {
                    assert!(
                        rows[first + i].contains(line),
                        "{ctx}: the header keeps the game's own row order at row {}: {:?}",
                        first + i,
                        rows[first + i]
                    );
                }
            }
        }
    }

    // …and the RASTER composite, the third path. It paints the frozen layer as
    // pixels at the game's own coordinates and was never wrong, but it is pinned
    // here so the invariant is "centred in every path" rather than "centred in the
    // two we happened to fix". Measured as the INK extent of each 16px text row,
    // inside the middle band where the frame art never reaches.
    let layout = app::render::v6_layout::classify_windows(match &model.root {
        WinNode::Layered(items) => items,
        _ => panic!("v6 builds a Layered root"),
    });
    let native = match &model.root {
        WinNode::Layered(items) => app::render::v6_layout::native_extent(items),
        _ => unreachable!(),
    };
    for honor in [true, false] {
        let mut state = app::state::AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.honor_game_colours = honor;
        if let Some(p) = Engine::paint_surface(&session) {
            *state.v6_paint.borrow_mut() = Some(p);
        }
        let (canvas, _metrics) = app::render::screen::build_v6_raster_canvas(&layout, native, &state);
        // The header occupies native text rows 3..12 (y = 49, 65, … 177).
        for row in 3..12u32 {
            let (mut lo, mut hi) = (u32::MAX, 0u32);
            for y in row * 16..(row * 16 + 16).min(canvas.height()) {
                for x in 80..560u32.min(canvas.width()) {
                    let ink = canvas
                        .get_pixel_checked(x, y)
                        .is_some_and(|p| p[3] == 255 && (p[0] as u16 + p[1] as u16 + p[2] as u16) > 200);
                    if ink {
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
            assert_ne!(lo, u32::MAX, "raster honor={honor}: header row {row} is painted");
            let right = native.0 as u32 - (hi + 1);
            // Equal margins to within one 8px cell — the glyph's ink stops short of
            // its advance, so the right margin runs a few pixels wide.
            assert!(
                lo.abs_diff(right) <= 8,
                "raster honor={honor}: header row {row} stays centred (ink {lo}..{hi}, \
                 left margin {lo}px, right margin {right}px of {}px)",
                native.0
            );
        }
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
