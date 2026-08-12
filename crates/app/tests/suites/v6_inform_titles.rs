//! Inform-compiled v6 title smokes (SQ-0458).
//!
//! Inform 6's v6 code generation exercises the core opcodes under v6 encoding
//! rules differently from Infocom's ZILCH (v6 call frames, the always-stored
//! `pull`, its own screen-model idioms), so these three freely-distributable
//! titles catch a failure class the Zork0/Shogun smokes can't. All three are
//! gitignored local stories — each test skips cleanly when absent, per the
//! established pattern.
//!
//! Verified behaviors these tests pin (observed against the real games):
//! - `advent.z6` (Graham Nelson's Adventure): boots to a keypress title, then
//!   plays. The Inform v6 library prints story prose into WINDOW 7 (not window
//!   0) but sets that window's wrapping attribute, so the prose STREAMS to the
//!   transcript like any window-0 game (SQ-0459); its cursor-positioned status
//!   line stays in window 1's paint model. The assertions read both.
//! - `scopa.z6`: checks the reported screen size at boot and refuses below
//!   600×400 — exercising our header screen-dims reporting end to end.
//! - `sunburst.z6`: keypress-driven boot menu ('s' = start), then a
//!   read_char-based input loop (the game builds its own command line).

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story headless; `screen_px` overrides the reported v6 Reso window
/// — the ART resolution, which the engine doubles to the unit screen (SQ-0479).
/// So `Some((160,100))` → a 320×200 screen, `Some((320,240))` → 640×480. `None`
/// falls back to the Blorb Reso (or the 320×200 art → 640×400 screen default).
/// Returns None (skip) if the story file is absent.
fn boot(name: &str, screen_px: Option<(u16, u16)>) -> Option<GameSession> {
    let story_path = stories_dir().join(name);
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let screen_px = screen_px.or_else(|| picts.std_window());
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, screen_px, None, None)
            .unwrap_or_else(|e| panic!("{name} should boot without a ZError: {e:?}"));
    assert!(!session.quit, "{name} quit during boot");
    assert!(session.machine.fault_trace.is_none(), "{name} faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// All the v6 window model's text runs, joined per window.
fn window_texts(session: &GameSession) -> Vec<String> {
    session
        .machine
        .screen
        .v6
        .as_ref()
        .map(|v6| v6.windows.iter().map(|w| w.texts.iter().map(|t| t.text.as_str()).collect()).collect())
        .unwrap_or_default()
}

#[test]
fn advent_z6_boots_and_look_streams_room_to_transcript() {
    let Some(mut session) = boot("advent.z6", None) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = session.take_transcript();

    // The title screens wait on keypresses; a few keys reach the command
    // prompt.
    assert_eq!(session.pending_input(), InputKind::Char, "advent boots to a keypress title");
    for _ in 0..4 {
        if session.pending_input() == InputKind::Line {
            break;
        }
        let result = session.submit_char(b' ');
        assert!(result.fault.is_none(), "advent faulted leaving the title: {:?}", result.fault);
        assert!(!result.quit, "advent quit leaving the title");
    }
    assert_eq!(session.pending_input(), InputKind::Line, "advent reaches the command prompt");

    // The status line lives in window 1 (upper), whose wrapping bit is CLEAR —
    // it stays PAINT and shows up in the v6 window model, not the transcript.
    let wins = window_texts(&session);
    assert!(
        wins.iter().any(|t| t.contains("At End Of Road")),
        "status window should name the opening room, windows: {wins:?}"
    );

    // SQ-0459: Inform's v6 library prints prose into window 7 with its wrapping
    // attribute SET (attrs 0b1111), so that prose now STREAMS to the transcript
    // exactly like a window-0 game's — a clean, ungarbled room description
    // (unlike the pixel-overprinted paint runs it used to leave in win7). Two
    // turns exercise the parser + v6 pull path under Inform codegen.
    for turn in 0..2 {
        let result = session.submit("look");
        assert!(result.fault.is_none(), "look {turn} faulted: {:?}", result.fault);
        assert!(!result.quit, "look {turn} quit");
        assert!(
            result.transcript.contains("end of a road before a small brick building"),
            "look {turn} should stream the End Of Road description into the transcript, got: {:?}",
            result.transcript
        );
    }
}

#[test]
fn scopa_z6_screen_size_gate_respects_reported_dims() {
    // Reporting a 320×200 screen (art 160×100 ×2), scopa itself refuses to run —
    // proving the game reads the size we report. At 640×480 (art 320×240 ×2) it
    // boots past the gate to its (graphical) title, waiting on a key with no fault.
    let Some(mut small) = boot("scopa.z6", Some((160, 100))) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let refusal = small.take_transcript();
    assert!(
        refusal.contains("too small"),
        "scopa should refuse a 320x200 screen with its own message, got: {refusal:?}"
    );

    let mut big = boot("scopa.z6", Some((320, 240))).expect("story vanished between boots");
    let boot_text = big.take_transcript();
    assert!(
        !boot_text.contains("too small"),
        "scopa must accept a reported 640x480 screen, got: {boot_text:?}"
    );
    assert!(!big.quit, "scopa quit at 640x480");
    assert_eq!(big.pending_input(), InputKind::Char, "scopa waits at its title screen");
}

#[test]
fn sunburst_z6_menu_starts_and_keystroke_loop_plays() {
    let Some(mut session) = boot("sunburst.z6", None) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let menu = session.take_transcript();
    assert!(
        menu.contains("Sunburst Contamination") && menu.contains("Start game"),
        "boot menu should list its options, got: {menu:?}"
    );

    // 's' selects "Start game."; the opening room prints immediately.
    assert_eq!(session.pending_input(), InputKind::Char);
    let result = session.submit_char(b's');
    assert!(result.fault.is_none(), "starting faulted: {:?}", result.fault);
    assert!(
        result.transcript.contains("Entrance to space harbor"),
        "start should print the opening room, got: {:?}",
        result.transcript
    );

    // The game reads commands one keypress at a time (its own input loop):
    // type "look" + Enter and expect the room re-described on the Enter.
    let mut last = String::new();
    for ch in [b'l', b'o', b'o', b'k', 13] {
        assert_eq!(session.pending_input(), InputKind::Char, "sunburst stays in its keystroke loop");
        let result = session.submit_char(ch);
        assert!(result.fault.is_none(), "keystroke {ch} faulted: {:?}", result.fault);
        assert!(!result.quit, "keystroke {ch} quit");
        last = result.transcript;
    }
    assert!(
        last.contains("Entrance to space harbor"),
        "typed \"look\" should re-describe the room, got: {last:?}"
    );
}
