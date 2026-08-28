//! SQ-1106: an `erase_window` the game issues during BOOT must be drained by the
//! boot, not left for the first real turn to pick up.
//!
//! `erase_lower_requested` (ZMSD §8.7.3, the v1–5 lower-window flag) is drained in
//! exactly one place — `GameSession::drain_turn` — and the boot does not go through
//! it: `startup.rs` builds the seed `TurnResult` that puts the starting room on the
//! map, and that used to be a hand-written literal with `erase_lower: false` spelled
//! into it. So a story that clears the screen before printing its banner left the
//! flag set, nothing consumed it, and the FIRST REAL TURN's drain took it and fired
//! `AppState::mark_screen_clear` — wiping the banner and the opening room
//! description exactly one command late.
//!
//! Reported against `zork1-invclues-r52-s871125.z5` and
//! `hitchhiker-invclues-r31-s871119.z5`. Those are not hint browsers: the first
//! boots as ZORK I, Release 52 / Serial 871125, fully playable — a v5 Solid Gold
//! re-release with the hints built in — so a clear one command into normal play is
//! simply wrong. `zvm-cli` emits exactly ONE clear in a whole session, at startup,
//! where it is invisible because the screen is already empty; the two front-ends
//! disagreed about *when*, not *whether*.
//!
//! Two halves here, and they answer different questions:
//!
//! * `a_v5_re_release_clears_the_screen_during_its_own_boot` and its v3 companion
//!   confirm the MECHANISM on the real files — that these stories do erase during
//!   boot and the v3 original does not. Fixture-gated, so they skip vacuously on CI.
//! * `the_boot_seed_drains_the_clear_the_boot_itself_issued` and
//!   `the_banner_survives_the_first_command` reproduce the SYMPTOM on a hand-
//!   assembled story, so they never skip.

use app::engine::Engine;
use app::session::{GameSession, InputKind};
use app::state::{AppState, TranscriptKind};

use crate::fixture_paths::fixture_path;

// ── the mechanism, on the reported files ──────────────────────────────────────

/// Boot `name` and report whether the game asked for a screen clear on the way to
/// its first input request. `None` when the gitignored fixture is absent.
fn cleared_during_boot(name: &str) -> Option<bool> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let s = GameSession::new(bytes, true, false, None).expect("the story boots");
    Some(s.machine.screen.erase_lower_requested)
}

/// The v5 Solid Gold re-releases in the report clear the screen before they print
/// their banner. This is the link the diagnosis inferred; it is asserted here so a
/// future reader does not have to take it on trust.
#[test]
fn a_v5_re_release_clears_the_screen_during_its_own_boot() {
    for name in ["zork1-invclues-r52-s871125.z5", "hitchhiker-invclues-r31-s871119.z5"] {
        let Some(cleared) = cleared_during_boot(name) else { continue };
        assert!(
            cleared,
            "{name} issues an erase_window during boot — that pending clear is what \
             leaked into the first real turn (SQ-1106)"
        );
    }
}

/// …and the v3 original does not, which is why it never showed the symptom. The
/// control for the case above: without it, "the flag is set after boot" could just
/// be something every story does.
#[test]
fn the_v3_original_issues_no_boot_time_clear() {
    let Some(cleared) = cleared_during_boot("zork1-r88-s840726.z3") else { return };
    assert!(
        !cleared,
        "ZORK I r88 (v3) prints its banner without erasing first — the symptom was \
         reported only against the v5 re-releases (SQ-1106)"
    );
}

// ── the symptom, on a story that never skips ──────────────────────────────────

/// A v5 story that ERASES THE SCREEN and then prints `B` — its "banner" — before
/// blocking on its first `read`; the next turn prints `C` and blocks again.
///
/// Header layout mirrors zvm's crate-private `header::tests_support::sample_story`,
/// as `print_then_erase_boundary.rs` and the other synthetic suites do.
///
/// - `erase_window` is VAR:0x0D → `0xED`, type byte `0x3F` (one large constant),
///   operand `0xFFFF` = −1: "erase the whole screen and unsplit" (ZMSD §8.7.3.3).
///   A large constant is required — −1 does not fit a small one.
/// - `print_char` is VAR:0x05 → `0xE5`, type byte `0x7F` (one small constant).
/// - `read` is VAR:0x04 → `0xE4`, type byte `0x3F` (one large constant: the text
///   buffer), followed by the store byte. It is what ends the boot and the turn.
fn boot_erasing_story() -> Vec<u8> {
    let mut buf = vec![0u8; 0x1000];
    buf[0x00] = 5; // version
    buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base   = 0x0400
    buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc      = 0x0040
    buf[0x08] = 0x02; buf[0x09] = 0x00; // dictionary      = 0x0200
    buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table    = 0x0100
    buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars     = 0x0300
    buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
    buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table    = 0x0060

    // Dictionary at 0x0200: 0 word-separators, entry-size 4, 0 entries.
    buf[0x0200] = 0;
    buf[0x0201] = 4;
    buf[0x0202] = 0;
    buf[0x0203] = 0;
    // Text buffer at 0x0380: max 20 chars.
    buf[0x0380] = 20;

    const READ: [u8; 5] = [0xE4, 0x3F, 0x03, 0x80, 0x00]; // read 0x0380 → stack
    let prog: Vec<u8> = [
        &[0xED, 0x3F, 0xFF, 0xFF][..], // erase_window -1  — during BOOT, before any output
        &[0xE5, 0x7F, b'B'][..],       // print_char 'B'   — the "banner"
        &READ[..],                     // the boot parks here
        &[0xE5, 0x7F, b'C'][..],       // print_char 'C'   — the first real turn's output
        &READ[..],                     // …which ends here
        &[0xBA][..],                   // quit
    ]
    .concat();
    buf[0x0040..0x0040 + prog.len()].copy_from_slice(&prog);
    buf
}

/// Boot the story the way `startup.rs` does — run to the first input, drain the
/// banner, then seed the map — and hand back the session, the banner and the seed.
fn boot_like_startup() -> (GameSession, String, app::session::TurnResult) {
    let mut s =
        GameSession::new(boot_erasing_story(), true, false, None).expect("the synthetic v5 story boots");
    assert_eq!(s.pending_input(), InputKind::Line, "the story blocks on read");
    assert!(
        s.machine.screen.erase_lower_requested,
        "the synthetic story really does erase during boot (test is non-vacuous)"
    );
    let banner = s.take_transcript();
    assert_eq!(banner, "B", "the banner is what the game printed after its boot-time erase");
    let seed = s.seed_turn();
    (s, banner, seed)
}

/// The engine half: the boot's own clear is drained by the boot's own seed, so the
/// first real turn reports the clear it actually made — none.
///
/// Falsified by spelling `erase_lower: false` back into `Engine::seed_turn`:
///
/// ```text
/// the seed carries the clear the boot issued, rather than leaving it on the engine
/// ```
///
/// (that is the first assertion to fire; with the drain removed entirely — the
/// literal `startup.rs` used to carry — `!first.erase_lower` is the one that goes.)
#[test]
fn the_boot_seed_drains_the_clear_the_boot_itself_issued() {
    let (mut s, _banner, seed) = boot_like_startup();
    assert!(
        seed.erase_lower,
        "the seed carries the clear the boot issued, rather than leaving it on the engine"
    );
    let first = s.submit("");
    assert!(!first.quit && first.fault.is_none(), "the turn neither quit nor faulted");
    assert_eq!(first.transcript, "C", "the turn printed what the story printed");
    assert!(
        !first.erase_lower,
        "the first real turn printed `C` and erased nothing — a clear reported here \
         is the BOOT's, taken one command late (SQ-1106)"
    );
}

/// The transcript half — the symptom as reported. The banner is on screen after the
/// boot, and it is still on screen after the player's first command.
///
/// The two lines around `submit` are `turn::finish_command_turn`'s own opening
/// (`if result.erase_lower { state.mark_screen_clear(); }`, then the push), which
/// lives in the binary crate and cannot be called from here.
///
/// Falsified by spelling `erase_lower: false` back into `Engine::seed_turn`:
///
/// ```text
/// assertion `left == right` failed: the banner is still on the screen after the
/// player's first command — the only clear this session saw was the boot's, and it
/// fell on an empty screen (SQ-1106)
///   left: ["C"]
///  right: ["B", "C"]
/// ```
///
/// — the banner gone from the screen one command in, which is the report.
#[test]
fn the_banner_survives_the_first_command() {
    let (mut s, banner, _seed) = boot_like_startup();

    let mut state = AppState::default();
    state.push_transcript(&banner);
    assert_eq!(visible(&state), ["B".to_string()], "the banner is on screen after the boot");

    let first = s.submit("");
    if first.erase_lower {
        state.mark_screen_clear();
    }
    state.push_transcript_runs(&first.transcript, TranscriptKind::Story, &first.transcript_runs);

    assert_eq!(
        visible(&state),
        ["B".to_string(), "C".to_string()],
        "the banner is still on the screen after the player's first command — the \
         only clear this session saw was the boot's, and it fell on an empty screen \
         (SQ-1106)"
    );
}

/// What the player can see: everything at or below the screen-clear anchor.
fn visible(state: &AppState) -> &[String] {
    &state.transcript[state.clear_anchor.unwrap_or(0)..]
}
