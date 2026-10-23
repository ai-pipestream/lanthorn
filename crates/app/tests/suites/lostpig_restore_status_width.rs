//! SQ-1604, SQ-1778, SQ-1808, reworked for SQ-1809: a Save State moved between
//! pane widths keeps the status line whole and on one row.
//!
//! The history: a restore swaps in memory laid out for the SAVING session's
//! width, and a story that computed its status columns once (Zork 1 r52) needs
//! that width kept while one that lays the line out afresh from `$21` (Lost Pig,
//! Inform 7) must be allowed to follow the pane. SQ-0681 floored the width,
//! SQ-1604 and SQ-1778 released the floor by asking the story at the first command
//! after the restore. SQ-1809 asks the story once, BEFORE its boot, by playing a
//! throwaway copy ([`zvm::width_probe`]):
//!
//!   - Lost Pig re-reads `$21`: it is told the pane, and a restore needs no floor
//!     because it relays out from the width it reads next turn;
//!   - Hitchhiker's Guide r31 and Zork 1 cache it: they are told 80, always, so a
//!     restore into any pane restores under the width they were laid out for.
//!
//! The intent these cases keep from the earlier ones is the player's: **restore,
//! make a move, and the status line is whole and on one row** (CLAUDE.md's
//! "perturb before asserting" — the frame right after a restore is never the
//! evidence). They are written against Lost Pig's real status line, whose text a
//! fresh session at the restoring width is the oracle for.
//!
//! Uses the same lightweight save/restore shape as `zork1_restore_status_width.rs`
//! (`Engine::save_state`/`restore_state` + `app::session::restore_screen`) rather
//! than the full `host::persist` archive path, since only the VM snapshot and the
//! screen it carries matter here.
//!
//! Gitignored fixture (`stories/`) for HHG; Lost Pig is a fetched fixture.

use app::engine::Engine;
use app::machine_boot::MachineBoot;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;

/// Boot `name` at `cols` columns through the real door (so the probe decides what
/// it is told) and tap through to the first line prompt.
fn boot_story(name: &str, cols: u16) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return None;
    };
    let mut session = GameSession::new_for_machine(
        bytes, true, false, false, Vec::new(), Some((25, cols)), None, &MachineBoot::bare(),
    )
    .unwrap_or_else(|e| panic!("{name} should load and boot: {e:?}"));
    let mut n = 0;
    while session.pending_input() == InputKind::Char && n < 10 {
        let _ = session.submit_char(13);
        n += 1;
    }
    assert_eq!(session.pending_input(), InputKind::Line, "boot should reach a line prompt");
    Some(session)
}

/// Declare `pane_w` as the TUI's poll does, then play `cmd`.
fn play(s: &mut GameSession, pane_w: u16, cmd: &str) {
    let state = app::state::AppState::default();
    app::host::screen::sync_zvm_screen_dims(s, &state, (pane_w, 25));
    let r = s.submit(cmd);
    assert!(r.fault.is_none() && !r.quit, "{cmd:?} faulted/quit: {:?}", r.fault);
}

/// The upper window as text, one entry per row.
fn upper_rows(s: &GameSession) -> Vec<String> {
    let u = &s.machine.screen.upper;
    (1..=u.rows).map(|r| (1..=u.cols).map(|c| u.cell(r, c).ch).collect()).collect()
}

/// The width a session following the pane is declared at pane width `pane_w`.
fn declared_cols(pane_w: u16) -> u16 {
    let state = app::state::AppState::default();
    app::render::screen::story_screen_dims(ratatui::layout::Rect::new(0, 0, pane_w, 25), &state)
        .expect("a pane")
        .1
}

/// Lost Pig lays its status line out afresh from the width it is told. A Save
/// State moved to a different pane — narrower OR wider — has, after the next move,
/// the same whole one-row line a fresh session at that pane draws.
#[test]
fn a_save_moved_between_pane_widths_keeps_the_status_line_whole() {
    for (from, to) in [(120u16, 60u16), (60, 120), (100, 45)] {
        let Some(mut saver) = boot_story("LostPig.z8", from) else { return };
        assert_eq!(saver.fixed_cols(), None, "Lost Pig re-reads $21, so it follows the pane");
        play(&mut saver, from, "look");
        let save = Engine::save_state(&saver);
        let screen = saver.machine.screen.clone();

        let mut other = boot_story("LostPig.z8", to).expect("present");
        Engine::restore_state(&mut other, &save).expect("the Save State restores");
        app::session::restore_screen(&mut other, screen);

        // The oracle: a fresh session at the restoring width, same moves.
        let mut fresh = boot_story("LostPig.z8", to).expect("present");

        // Perturb: play after the restore, twice, as the poller would.
        for cmd in ["look", "look"] {
            play(&mut other, to, cmd);
            play(&mut fresh, to, cmd);
        }
        let ctx = format!("{from} -> {to}");
        assert_eq!(other.fixed_cols(), None, "a restore fixes nothing for an adapting story ({ctx})");
        assert_eq!(other.machine.mem.read_byte(0x21) as u16, declared_cols(to), "$21 follows the pane ({ctx})");
        let rows = upper_rows(&other);
        assert_eq!(rows.len(), 1, "one status row, no stray wrap ({ctx}): {rows:?}");
        assert!(!rows[0].trim().is_empty(), "the status line is drawn ({ctx})");
        assert_eq!(rows, upper_rows(&fresh), "the same whole line a fresh session draws ({ctx})");
    }
}

/// Negative control: a save restored into a session that is ALREADY at its own
/// width, with nothing to adapt, is simply undisturbed by the whole mechanism.
#[test]
fn restoring_into_the_same_width_changes_nothing() {
    let Some(mut a) = boot_story("LostPig.z8", 80) else { return };
    play(&mut a, 80, "look");
    let save = Engine::save_state(&a);
    let screen = a.machine.screen.clone();
    let before = upper_rows(&a);

    let mut b = boot_story("LostPig.z8", 80).expect("present");
    Engine::restore_state(&mut b, &save).expect("the Save State restores");
    app::session::restore_screen(&mut b, screen);
    play(&mut b, 80, "look");
    assert_eq!(b.fixed_cols(), None);
    assert_eq!(upper_rows(&b), before, "same pane, same line");
}

/// SQ-1808: Hitchhiker's Guide r31 (Solid Gold) bakes its status layout at the
/// width it booted with. Restored into a narrower session, its status write ran
/// one cell past the right edge and window 1 then WRAPPED, growing a stray second
/// row holding one reverse-video space (zvm now clips window 1 at the right margin
/// as well). With SQ-1809 the story is told 80 at both boots, so a save made in a
/// 48-column pane and restored in a 45-column one is restored under the very width
/// it was laid out for.
///
/// Gitignored fixture (`stories/`): skips vacuously when absent.
#[test]
fn a_baked_status_layout_restored_narrower_keeps_one_row() {
    const SAVED: u16 = 48;
    const NARROW: u16 = 45;
    let Some(mut saver) = boot_story("hitchhiker-invclues-r31-s871119.z5", SAVED) else { return };
    assert_eq!(saver.fixed_cols(), Some(80), "HHG caches the width, so it is told 80 in a 48-column pane");
    play(&mut saver, SAVED, "look");
    assert_eq!(saver.machine.screen.upper.rows, 1, "the saved game has a one-row status bar");
    let save = Engine::save_state(&saver);
    let screen = saver.machine.screen.clone();

    let mut narrow = boot_story("hitchhiker-invclues-r31-s871119.z5", NARROW).expect("present");
    Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
    app::session::restore_screen(&mut narrow, screen);
    assert_eq!(narrow.fixed_cols(), Some(80), "the told width is the same 80 on both sides");

    for cmd in ["look", "look"] {
        play(&mut narrow, NARROW, cmd);
        assert_eq!(narrow.machine.screen.upper.rows, 1, "no stray second status row after {cmd:?}");
        assert_eq!(narrow.machine.mem.read_byte(0x21), 80, "$21 stays 80 in a {NARROW}-column pane");
    }
}
