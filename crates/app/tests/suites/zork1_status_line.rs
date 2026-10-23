//! SQ-0679, reworked for SQ-1809: Zork 1's interpreter-facing status bar at
//! several pane widths.
//!
//! Zork 1 (r52) paints its status bar ONCE — a run of reverse-video spaces the
//! width of byte $21 as it stood when the bar was laid out — and thereafter only
//! `set_cursor`s to the two field columns it derived from that width and reprints
//! the score/moves digits in place. Told a pane-measured width AFTER it had
//! booted at 80 (SQ-0532/SQ-0533, 2026-07-28) it garbled: in a NARROWER pane the
//! baked-in field columns (60 and 73) fall outside the window, §8.7.2.3 makes the
//! move illegal, the interpreter drops it, and the digits land at the cursor's
//! home — column 1, on top of the room name ("0North of House0 2 …").
//!
//! The first fix floored the declared width at the boot width. SQ-1809 replaced
//! the floor with knowledge: the story is played on a throwaway copy before the
//! real boot ([`zvm::width_probe`]), found to cache the width, and told 80 for the
//! whole session — whatever the pane is, before or after the boot. So the GAME's
//! screen is the same at every pane width, and this suite pins exactly that, on the
//! game's own upper-window grid. How an 80-column grid is DISPLAYED in a pane that
//! is not 80 columns (centred and extended when wider, columns dropped when
//! narrower) is the renderer's business and is pinned by its own suites.
//!
//! **Colour mode**: every case runs in BOTH `honor_game_colours` modes. Zork 1
//! sets no colours on the bar (both channels stay Default), so the reverse bit
//! alone has to carry it either way.
//!
//! Gitignored fixture: skips vacuously when the story is absent.

use std::path::PathBuf;

use app::config::StatusWidth;
use app::graphics::PictSource;
use app::machine_boot::MachineBoot;
use app::session::{GameSession, InputKind};
use ratatui::layout::Rect;

const STORY: &str = "zork1-invclues-r52-s871125.z5";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork 1 through the real door (`new_for_machine`, so the width probe runs)
/// and tap through to its first line prompt, or `None` when the gitignored fixture
/// is absent. `host_screen` is the pane seeded before the boot.
fn boot_mode(honor: bool, host_screen: Option<(u16, u16)>, mode: StatusWidth) -> Option<GameSession> {
    let path = stories_dir().join(STORY);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let boot = MachineBoot::bare().with_status_width(mode);
    let mut s = GameSession::new_for_machine(bytes, honor, false, false, dims, host_screen, None, &boot)
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

fn boot(honor: bool, host_screen: Option<(u16, u16)>) -> Option<GameSession> {
    boot_mode(honor, host_screen, StatusWidth::Auto)
}

/// Declare `pane_w` to an already-booted session exactly as
/// `loop_tick::poll_zvm_screen_dims` would, play one turn, and return the game's
/// own status row and whether every cell of it is reverse video.
fn play_status_row(s: &mut GameSession, pane_w: u16) -> (String, bool) {
    let state = app::state::AppState::default();
    app::host::screen::sync_zvm_screen_dims(s, &state, (pane_w, 25));
    let r = s.submit("look");
    assert!(r.fault.is_none() && !r.quit, "\"look\" faulted: {:?}", r.fault);
    let _ = s.take_transcript();
    let u = &s.machine.screen.upper;
    let row: String = (1..=u.cols).map(|c| u.cell(1, c).ch).collect();
    let all_rev = (1..=u.cols).all(|c| u.cell(1, c).style & 1 != 0);
    (row, all_rev)
}

/// The opening room, and the exact bar Zork 1 paints for it at its own width.
const OPENING_BAR: &str =
    " West of House                                      Score: 0     Moves: 1";

/// The room name must survive at the left of the bar — at EVERY pane width, in
/// both colour modes.
///
/// This is the garble the report describes. With the pane-measured width declared
/// under a story that had already baked its field columns in, the narrow case
/// rendered `"0North of House0 2 …"`: the score and moves digits printed at
/// column 1 because their `set_cursor` was out of range and dropped.
#[test]
fn zork1_status_bar_keeps_its_room_name_at_every_width() {
    for honor in [true, false] {
        for pane_w in [30u16, 45, 60, 80, 100, 120] {
            let Some(mut s) = boot(honor, Some((24, pane_w))) else {
                eprintln!("SKIP: gitignored story missing");
                return;
            };
            let (bar, _) = play_status_row(&mut s, pane_w);
            assert!(
                bar.starts_with(" West of House"),
                "the room name leads the bar (honor={honor}, pane={pane_w}): {bar:?}"
            );
            assert!(
                !bar[..14].chars().any(|c| c.is_ascii_digit()),
                "no field digit may land on the room name (honor={honor}, pane={pane_w}): {bar:?}"
            );
        }
    }
}

/// The reverse-video band fills the whole 80-column grid the game painted — the
/// same band at every pane width, because the game is told 80 at every one. (The
/// band's continuation into the side margins of a WIDER pane is display, not the
/// game's.)
#[test]
fn zork1_status_bar_reverse_fill_spans_the_whole_grid() {
    for honor in [true, false] {
        for pane_w in [45u16, 60, 80, 120] {
            let Some(mut s) = boot(honor, Some((24, pane_w))) else {
                eprintln!("SKIP: gitignored story missing");
                return;
            };
            let (bar, all_rev) = play_status_row(&mut s, pane_w);
            assert_eq!(bar.chars().count(), 80, "the grid is the 80 columns it was told (pane={pane_w})");
            assert!(all_rev, "the reversed band spans the grid (honor={honor}, pane={pane_w}); bar was {bar:?}");
        }
    }
}

/// The whole bar, verbatim, at a pane that fits it AND at ones that do not: the
/// game's screen no longer depends on the pane, so a narrow pane has nothing to
/// clip in the model.
#[test]
fn zork1_status_bar_text_is_exact() {
    for honor in [true, false] {
        for pane_w in [45u16, 60, 100] {
            let Some(mut s) = boot(honor, Some((24, pane_w))) else {
                eprintln!("SKIP: gitignored story missing");
                return;
            };
            let (bar, _) = play_status_row(&mut s, pane_w);
            assert_eq!(bar.trim_end(), OPENING_BAR, "the whole bar, verbatim (honor={honor}, pane={pane_w})");
        }
    }
}

/// SQ-0680 survives as an OVERRIDE. Seeding the real pane before boot used to let
/// Zork 1 — which falls back to a compact "score/moves" bar once told a screen too
/// narrow for the labelled one — choose against the pane it would render into.
/// SQ-1809 tells a caching story 80 instead, so the default no longer does that;
/// `status_width = "pane"` is how a player who prefers the compact bar in a narrow
/// window gets it back.
#[test]
fn zork1_compact_bar_is_what_the_pane_override_gives_a_narrow_boot() {
    const PANE_W: u16 = 60;
    for honor in [true, false] {
        let Some(mut auto) = boot(honor, Some((24, PANE_W))) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let (auto_bar, _) = play_status_row(&mut auto, PANE_W);
        assert_eq!(auto_bar.trim_end(), OPENING_BAR, "auto: told 80, the labelled bar (honor={honor})");

        let mut pane = boot_mode(honor, Some((24, PANE_W)), StatusWidth::Pane).expect("story present");
        let (pane_bar, _) = play_status_row(&mut pane, PANE_W);
        assert!(
            pane_bar.starts_with(" West of House"),
            "the room name still leads the bar (honor={honor}): {pane_bar:?}"
        );
        assert!(
            pane_bar[14..].chars().any(|c| c.is_ascii_digit()),
            "told the 60-column pane, Zork 1 lays the compact bar out inside it (honor={honor}): {pane_bar:?}"
        );
        assert_ne!(pane_bar.trim_end(), auto_bar.trim_end(), "the override changes what the game paints");
    }
}
