//! SQ-0679: Zork 1's interpreter-facing status bar, driven through the real
//! render path at several pane widths.
//!
//! Zork 1 (r52) paints its status bar ONCE — a run of reverse-video spaces the
//! width of byte $21 as it stood when the bar was laid out — and thereafter only
//! `set_cursor`s to the two field columns it derived from that width and reprints
//! the score/moves digits in place. It never re-reads $21. Two things followed
//! from the app declaring a pane-measured width AFTER the story had booted at 80
//! (SQ-0532/SQ-0533, 2026-07-28):
//!
//!   - NARROWER pane: the baked-in field columns (60 and 73) fall outside the
//!     window, §8.7.2.3 makes the move illegal, the interpreter drops it, and the
//!     digits land at the cursor's home — column 1, on top of the room name
//!     ("0North of House0 2 …").
//!   - WIDER pane: the grid grows past the 80 columns the game painted, and the
//!     columns that appear are blank/unstyled — the reverse-video band stops
//!     short of its own box.
//!
//! Both are pinned here, end to end: boot the real story, declare the dims the
//! app declares, play a turn, render the story pane, read the cells back.
//!
//! **Colour mode**: every case runs in BOTH `honor_game_colours` modes. Zork 1
//! sets no colours on the bar (both channels stay Default), so the reverse bit
//! alone has to carry it either way — a mode-specific regression would otherwise
//! hide behind the shipped default.
//!
//! Gitignored fixture: skips vacuously when the story is absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

const STORY: &str = "zork1-invclues-r52-s871125.z5";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork 1 and tap through to its first line prompt, or `None` when the
/// gitignored fixture is absent.
fn boot(honor: bool) -> Option<GameSession> {
    boot_with_host_screen(honor, None)
}

/// Like [`boot`], but seeds `host_screen` into the constructor BEFORE the boot
/// run — the SQ-0680 pre-boot pane seed, exercised directly rather than through
/// `startup::boot_story`'s terminal-size probe. `None` reproduces the pre-fix
/// boot path exactly (the zvm 80×24 fallback `init_caps` seeds absent a hint).
fn boot_with_host_screen(honor: bool, host_screen: Option<(u16, u16)>) -> Option<GameSession> {
    let path = stories_dir().join(STORY);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, std_window, None, host_screen,
    )
    .expect("Zork 1 should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..10 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                s.submit_char(13);
            }
            InputKind::Event => {
                s.submit("");
            }
        }
        let _ = s.take_transcript();
    }
    Some(s)
}

/// The status row as the player sees it: the framed upper window's interior on
/// the row the bar sits on, as `(text, all_cells_reversed)`.
///
/// The upper window is drawn as a box; the interior is the span between its
/// left and right rules. Reading the span from the drawn rules (rather than
/// recomputing it) is the point — the reverse band has to match however the box
/// was actually drawn.
fn status_interior(buf: &Buffer, area: Rect) -> (String, bool) {
    let row_at = |y: u16| -> String {
        (area.x..area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    // The box's top rule is row 0; the single grid row is row 1.
    let top = row_at(area.y);
    assert!(top.contains('┌') && top.contains('┐'), "the upper window is framed: {top:?}");
    let y = area.y + 1;
    let row = row_at(y);
    let cells: Vec<char> = row.chars().collect();
    let left = cells.iter().position(|&c| c == '│').expect("left rule") as u16;
    let right = cells.iter().rposition(|&c| c == '│').expect("right rule") as u16;
    assert!(right > left + 1, "the box has an interior: {row:?}");
    let xs: Vec<u16> = ((area.x + left + 1)..(area.x + right)).collect();
    let text: String = xs
        .iter()
        .map(|&x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
        .collect();
    let all_rev = xs
        .iter()
        .all(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED));
    (text, all_rev)
}

/// Declare `pane_w` to an already-booted session exactly as
/// `loop_tick::poll_zvm_screen_dims` would, play one turn (so a stale-width
/// layout actually lands in the wrong place, per the restore-test convention
/// of perturbing before asserting), and return the rendered status-bar
/// interior.
fn play_and_render_status_bar(s: &mut GameSession, honor: bool, pane_w: u16) -> (String, bool) {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, pane_w, 25);

    let version = s.machine.mem.version();
    let (rows, cols) = app::render::screen::declared_story_screen_dims(
        area,
        &state,
        version,
        s.boot_screen_cols,
    )
    .expect("a non-empty pane reports dims");
    Engine::set_screen_dims(s, rows, cols);

    let r = s.submit("look");
    assert!(r.fault.is_none() && !r.quit, "\"look\" faulted: {:?}", r.fault);
    let _ = s.take_transcript();

    let model = s.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    status_interior(&buf, area)
}

/// Boot unseeded (the zvm 80×24 fallback) and play one turn at `pane_w`.
fn status_bar_at(honor: bool, pane_w: u16) -> Option<(String, bool)> {
    let mut s = boot(honor)?;
    Some(play_and_render_status_bar(&mut s, honor, pane_w))
}

/// The opening room, and the exact bar Zork 1 paints for it at its own width.
const OPENING_BAR: &str =
    " West of House                                      Score: 0     Moves: 1";

/// The room name must survive at the left of the bar — at EVERY pane width, in
/// both colour modes.
///
/// This is the garble the report describes. With the pane-measured width
/// declared under a story that had already baked its field columns in, the
/// narrow case rendered `"0North of House0 2 …"`: the score and moves digits
/// printed at column 1 because their `set_cursor` was out of range and dropped.
#[test]
fn zork1_status_bar_keeps_its_room_name_at_every_width() {
    for honor in [true, false] {
        for pane_w in [60u16, 80, 100, 120] {
            let Some((bar, _)) = status_bar_at(honor, pane_w) else {
                eprintln!("SKIP: gitignored story missing");
                return;
            };
            assert!(
                bar.starts_with(" West of House"),
                "the room name leads the bar (honor={honor}, pane={pane_w}): {bar:?}"
            );
            // The precise corruption signature: a digit where the bar's leading
            // blank and room name belong.
            assert!(
                !bar[..14].chars().any(|c| c.is_ascii_digit()),
                "no field digit may land on the room name (honor={honor}, pane={pane_w}): {bar:?}"
            );
        }
    }
}

/// The reverse-video band fills the whole box interior — at a pane WIDER than
/// the 80 columns the game painted, which is where the grid grows past the
/// game's own band and nothing ever repaints the columns that appear.
#[test]
fn zork1_status_bar_reverse_fill_spans_the_whole_box_interior() {
    for honor in [true, false] {
        for pane_w in [60u16, 80, 100, 120] {
            let Some((bar, all_rev)) = status_bar_at(honor, pane_w) else {
                eprintln!("SKIP: gitignored story missing");
                return;
            };
            assert!(
                all_rev,
                "the reversed band must reach both edges of the box interior \
                 (honor={honor}, pane={pane_w}); bar was {bar:?}"
            );
        }
    }
}

/// Whole-bar pin at the two widths that bracket the defect: a pane wide enough
/// to show all 80 columns (the fields are there, the grown columns are blank
/// band), and a pane too narrow for them (the left of the bar is intact and the
/// pane clips the right — what an 80-column game in a 60-column window looks
/// like anywhere).
#[test]
fn zork1_status_bar_text_is_exact() {
    for honor in [true, false] {
        let Some((wide, _)) = status_bar_at(honor, 100) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        assert_eq!(
            wide.trim_end(),
            OPENING_BAR,
            "the whole bar, verbatim, at a pane that fits it (honor={honor})"
        );

        let (narrow, _) = status_bar_at(honor, 60).expect("story present");
        assert!(
            OPENING_BAR.starts_with(narrow.trim_end()),
            "a narrow pane clips the RIGHT of the same bar, nothing else \
             (honor={honor}): {narrow:?}"
        );
        assert!(
            narrow.starts_with(" West of House"),
            "…and the room name is still the left of it (honor={honor}): {narrow:?}"
        );
    }
}

/// SQ-0680: seeding the REAL pane before boot (rather than leaving the story
/// to boot at the zvm 80×24 fallback and finding out a frame later) lets a v4/
/// v5 status routine that adapts its OWN layout to the screen width it is told
/// — Zork 1 falls back to a compact, unlabelled "score/moves" bar once the
/// screen is too narrow for the full "Score: N     Moves: N" one — make that
/// choice against the pane it will actually render into.
///
/// A 60-column pane never fits the labelled bar (`zork1_status_bar_text_is_exact`
/// above pins the unseeded case: the fields are laid out for 80 columns and land
/// entirely outside a 60-column pane, so nothing of them is visible). Seeded to
/// 60 at boot, Zork 1 instead lays out the compact form for 60 columns and it
/// fits inside the pane with room to spare — the score/moves fields are VISIBLE,
/// not clipped away.
///
/// Falsified by passing `None`: booting the exact same story into the exact
/// same 60-column pane without the seed reproduces the pre-existing clip
/// (`zork1_status_bar_text_is_exact`'s narrow case, re-asserted here for the
/// direct side-by-side).
#[test]
fn zork1_status_bar_lays_out_for_a_pre_boot_seeded_narrow_pane() {
    const PANE_W: u16 = 60;
    for honor in [true, false] {
        let Some(mut seeded) = boot_with_host_screen(honor, Some((24, PANE_W))) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let (seeded_bar, seeded_all_rev) =
            play_and_render_status_bar(&mut seeded, honor, PANE_W);

        assert!(
            seeded_bar.starts_with(" West of House"),
            "the room name still leads the bar (honor={honor}): {seeded_bar:?}"
        );
        assert!(
            seeded_all_rev,
            "the reversed band still reaches both edges of the box interior \
             (honor={honor}); bar was {seeded_bar:?}"
        );
        // The score/moves fields are IN RANGE: some digit lands to the right
        // of the room name, inside the 60-column pane the boot was seeded
        // with — unlike the unseeded case below, where they are laid out for
        // 80 columns and none of them are visible in only 60.
        assert!(
            seeded_bar[14..].chars().any(|c| c.is_ascii_digit()),
            "the score/moves fields must be visible, not clipped off the \
             right, once boot itself was seeded to the pane (honor={honor}): \
             {seeded_bar:?}"
        );

        // Falsification: the same story, the same 60-column render pane, but
        // an UNSEEDED boot (`None`) — the pre-SQ-0680 path. The fields were
        // laid out for the 80-column zvm fallback and fall entirely outside a
        // 60-column pane, so the bar is just the room name and blank reverse
        // space, exactly as `zork1_status_bar_text_is_exact` already pins.
        let mut unseeded = boot_with_host_screen(honor, None).expect("story present");
        let (unseeded_bar, _) = play_and_render_status_bar(&mut unseeded, honor, PANE_W);
        assert!(
            !unseeded_bar[14..].chars().any(|c| c.is_ascii_digit()),
            "without the pre-boot seed the fields are laid out for 80 columns \
             and clipped entirely off a 60-column pane — the pre-existing clip \
             this test falsifies against (honor={honor}): {unseeded_bar:?}"
        );
        assert_ne!(
            seeded_bar.trim_end(),
            unseeded_bar.trim_end(),
            "seeding the boot pane must actually change what Zork 1 painted \
             (honor={honor})"
        );
    }
}
