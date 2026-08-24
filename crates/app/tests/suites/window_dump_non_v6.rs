//! SQ-0699: `/dump-windows` for non-v6 Z-machine games (v1–v5, v7, v8) used to
//! print exactly one line — `Window layout: Grid {cols}x{rows} over Buffer
//! (Z-machine v{N})` — which tells a reader nothing about what's actually on
//! screen. In particular it collapsed the split height (`upper_window_rows`)
//! and the painted grid height (`upper.rows`) into a single number, even
//! though c54c9e0f (SQ-0696) made a `split_window` shrink keep painted rows
//! (so Inform box quotes survive it) — the two can now legitimately differ,
//! and the old dump had no way to show that.
//!
//! `anchor.z8` exercises both states: at boot its upper window holds an
//! 11-row painted quote box behind a 1-row split, and once play begins the
//! split collapses to an ordinary 1-row status line with a 1-row grid to
//! match.
//!
//! The story is gitignored, so this skips vacuously when absent.


use app::engine::Engine;
use app::session::{GameSession, InputKind};

use crate::fixture_paths::fixture_path;


fn boot_anchor() -> Option<GameSession> {
    let path = fixture_path("anchor.z8");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
            .expect("anchor.z8 should load and boot without a ZError"),
    )
}

#[test]
fn dump_windows_reports_split_vs_painted_divergence_at_boot() {
    let Some(session) = boot_anchor() else { return };

    let lines = session.window_dump();
    let dump = lines.join("\n");

    assert!(dump.contains("Z-machine v8"), "the version stays reported, as before: {dump}");
    assert!(
        dump.contains("split: 1 row(s) requested"),
        "the boot quote box is behind a 1-row split: {dump}"
    );
    assert!(
        dump.contains("grid: 11 row(s) painted"),
        "the boot quote box painted 11 rows that the shrink must not truncate: {dump}"
    );
    assert!(dump.contains("<- diverge"), "split and painted height disagree at boot, and the dump must flag it: {dump}");
    assert!(dump.contains("H.P. Lovecraft"), "the painted rows are printed as quoted text: {dump}");
}

#[test]
fn dump_windows_shows_collapsed_state_during_play() {
    let Some(mut session) = boot_anchor() else { return };

    // Clear the two startup quote screens (each waits for a keypress) so the
    // upper window settles to its ordinary one-row status line.
    for _ in 0..2 {
        if matches!(session.pending_input(), InputKind::Char) {
            let _ = session.submit_char(13);
        }
    }
    let _ = session.submit("look");

    let lines = session.window_dump();
    let dump = lines.join("\n");

    assert!(
        dump.contains("split: 1 row(s) requested  ·  grid: 1 row(s) painted"),
        "once play begins the split and the painted grid both collapse to 1 row: {dump}"
    );
    assert!(!dump.contains("<- diverge"), "the two numbers agree during ordinary play, so no flag: {dump}");
}
