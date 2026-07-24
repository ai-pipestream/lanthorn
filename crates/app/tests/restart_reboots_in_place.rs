//! In-game `restart` re-boots the story in place instead of quitting the app
//! (SQ-0493). Before the fix, the `@restart` opcode surfaced as `RunStop::Quit`,
//! so typing `restart` (and confirming) dropped the player out of babelmap to the
//! terminal. Now the VM re-boots (v6 re-enters `main`; v1–5 jump to the initial
//! PC) and play continues from the game's opening.
//!
//! Two real stories, both gitignored (SKIP when absent):
//!   * Zork Zero (v6) — the reported case; the v6 boot re-entry path.
//!   * Zork I (v3)    — guards the v1–5 initial-PC path from regression.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Collapse runs of whitespace so boot-vs-reboot text compares independently of
/// v6 layout spacing.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// After a `restart` command, answer the game's Y/N confirmation (line or single
/// key) up to a few turns, collecting the reboot output. Asserts the session
/// never quits or faults along the way. Returns the accumulated reboot text.
fn confirm_restart(session: &mut GameSession) -> String {
    let mut collected = String::new();
    for _ in 0..4 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit("yes"),
            InputKind::Char => session.submit_char(b'y'),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit, "restart must NOT quit the app: {:?}", result.transcript);
        assert!(result.fault.is_none(), "restart faulted: {:?}", result.fault);
        collected.push_str(&result.transcript);
        // Once the game is back at a normal prompt with fresh output, stop.
        if !result.transcript.trim().is_empty() {
            break;
        }
    }
    collected
}

#[test]
fn zork0_v6_in_game_restart_reboots_and_keeps_playing() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // Snapshot the boot banner/intro — the reboot must reproduce it.
    let boot_banner = norm(&session.take_transcript());
    eprintln!("ZORK0 BOOT BANNER ({} chars): {:?}", boot_banner.len(), boot_banner);

    // Advance the game so a restart has something to undo.
    assert_eq!(session.pending_input(), InputKind::Line, "expected an opening line prompt");
    let mv = session.submit("look");
    assert!(!mv.quit && mv.fault.is_none(), "'look' failed: {:?}", mv.fault);
    eprintln!("ZORK0 after look: {:?}", norm(&mv.transcript));

    // Type restart and confirm.
    let r = session.submit("restart");
    assert!(!r.quit, "the restart command itself must not quit");
    assert!(r.fault.is_none(), "restart faulted: {:?}", r.fault);
    eprintln!("ZORK0 restart-turn transcript: {:?}", norm(&r.transcript));
    let mut reboot_text = norm(&r.transcript);
    reboot_text.push(' ');
    reboot_text.push_str(&norm(&confirm_restart(&mut session)));
    eprintln!("ZORK0 reboot text ({} chars): {:?}", reboot_text.len(), reboot_text);

    // The game is alive and waiting for the player again.
    assert!(!session.quit, "session must stay alive after restart");
    assert!(session.machine.fault_trace.is_none(), "no fault after restart");
    assert!(
        matches!(session.pending_input(), InputKind::Line | InputKind::Char),
        "the rebooted game must be waiting for input again"
    );

    // It re-ran from the start: the boot banner reappears.
    let probe: String = boot_banner.chars().take(40).collect();
    eprintln!("ZORK0 probe: {:?}", probe);
    assert!(
        boot_banner.len() >= 20 && reboot_text.contains(&probe),
        "restart must re-run Zork0 from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}

#[test]
fn zork1_v3_in_game_restart_reboots_and_keeps_playing() {
    let story_path = stories_dir().join("zork1-r88-s840726.z3");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut session = GameSession::new(story_bytes, false, false, None)
        .expect("Zork1 (v3) should load and boot without a ZError");
    assert!(!session.quit, "Zork1 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork1 faulted during boot");

    let boot_banner = norm(&session.take_transcript());
    eprintln!("ZORK1 BOOT BANNER ({} chars): {:?}", boot_banner.len(), boot_banner);

    assert_eq!(session.pending_input(), InputKind::Line, "expected an opening line prompt");
    let mv = session.submit("north");
    assert!(!mv.quit && mv.fault.is_none(), "'north' failed: {:?}", mv.fault);

    let r = session.submit("restart");
    assert!(!r.quit, "the restart command itself must not quit");
    assert!(r.fault.is_none(), "restart faulted: {:?}", r.fault);
    let mut reboot_text = norm(&r.transcript);
    reboot_text.push(' ');
    reboot_text.push_str(&norm(&confirm_restart(&mut session)));
    eprintln!("ZORK1 reboot text ({} chars): {:?}", reboot_text.len(), reboot_text);

    assert!(!session.quit, "session must stay alive after restart");
    assert!(session.machine.fault_trace.is_none(), "no fault after restart");
    assert_eq!(
        session.pending_input(),
        InputKind::Line,
        "the rebooted game must be waiting for a command again"
    );

    let probe: String = boot_banner.chars().take(40).collect();
    assert!(
        boot_banner.len() >= 20 && reboot_text.contains(&probe),
        "restart must re-run Zork1 from the opening (boot banner should reappear)\n  probe: {probe:?}\n  reboot: {reboot_text:?}"
    );
}
