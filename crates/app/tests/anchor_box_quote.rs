//! SQ-0696: Anchorhead's startup quote boxes must actually be readable.
//!
//! `anchor.z8`'s intro shows two Inform **box quotes** — the Lovecraft epigraph
//! beside the title splash, and `* THE FIRST DAY *` on the screen after the
//! prologue. Each is drawn the way the Inform library's `box` statement always
//! has: split the upper window tall, print reverse-video text into it, then
//! shrink the window back to the status line — and only THEN wait for a
//! keypress.
//!
//! A window model that simply truncates the discarded rows shows nothing at all,
//! which is exactly what the reporter saw: a screen that clears and waits for a
//! key with no quote on it. That failure is named in the note ZMSD §8's remarks
//! cite for this case (Plotkin, *Quote Boxes in Z-Machine Games*): "In a naive
//! Glk implementation -- or _any_ simple implementation using text windows --
//! this trick fails. The interpreter will display the quote text for a tiny
//! fraction of a second, or (if the display system has built-in buffering) not
//! at all." Infocom's own V4 interpreter left the reverse-video text "overlaid
//! on the top of the story window", where it "would then scroll away as part of
//! the story window's natural scrolling".
//!
//! The fix prints those rows into the story stream at the shrink, which is that
//! same reading in a host whose lower window is a scrollback transcript. The
//! quote arrives with its reverse-video styling intact.
//!
//! The story is gitignored, so this skips vacuously when absent.

use std::path::PathBuf;

use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot_anchor() -> Option<GameSession> {
    let path = stories_dir().join("anchor.z8");
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
fn anchor_startup_quote_boxes_reach_the_transcript() {
    let Some(mut session) = boot_anchor() else { return };

    // ── Quote 1: the epigraph beside the title splash, at boot ──────────────
    let boot = session.take_transcript();
    assert!(
        boot.contains("A N C H O R H E A D"),
        "the title splash is ordinary lower-window text and was never at risk: {boot:?}"
    );
    assert!(
        boot.contains("The oldest and strongest emotion of mankind"),
        "the boot quote box must survive the upper window shrinking back to the status line\n{boot}"
    );
    assert!(boot.contains("H.P. Lovecraft"), "…including its attribution line\n{boot}");

    // ── Quote 2: `* THE FIRST DAY *`, two keypresses later ──────────────────
    assert!(matches!(session.pending_input(), InputKind::Char), "boot waits for a keypress");
    let prologue = session.submit_char(13);
    assert!(
        prologue.transcript.contains("November, 1997"),
        "the prologue screen follows the title: {:?}",
        prologue.transcript
    );

    assert!(matches!(session.pending_input(), InputKind::Char), "the prologue waits for a keypress");
    let day_one = session.submit_char(13);
    assert!(
        day_one.transcript.contains("* THE FIRST DAY *"),
        "the second quote box is the WHOLE screen here — without it this turn renders \
         completely blank while waiting for a keypress, the reported symptom\n{}",
        day_one.transcript
    );
    assert!(
        day_one.transcript.contains("I was far from home"),
        "…and carries the quote body\n{}",
        day_one.transcript
    );

    // The box arrives styled: the Inform library prints it in reverse video, and
    // that is what makes it read as a box rather than stray prose.
    // `transcript_runs` are (char_count, style_bits, ...) spans over the
    // transcript, so walk the offsets and collect the reverse-video ones.
    let chars: Vec<char> = day_one.transcript.chars().collect();
    let mut at = 0usize;
    let mut reversed = String::new();
    for run in &day_one.transcript_runs {
        let (n, bits) = (run.0, run.1);
        let end = (at + n).min(chars.len());
        if bits & 0x01 != 0 {
            reversed.extend(&chars[at.min(chars.len())..end]);
        }
        at = end;
    }
    assert!(
        reversed.contains("THE FIRST DAY"),
        "the quote keeps the reverse-video styling the library painted it with; \
         reverse-styled text this turn was {reversed:?}"
    );

    // The game plays on normally afterwards — the upper window is back to the
    // one-row status line and the next turn is an ordinary line prompt.
    assert!(matches!(session.pending_input(), InputKind::Char), "the quote screen waits for a key");
    let start = session.submit_char(13);
    assert!(
        start.transcript.contains("Outside the Real Estate Office"),
        "play begins after the second quote: {:?}",
        start.transcript
    );
}

/// The per-turn status-line re-split must NOT dump the status bar into the
/// transcript. Every v4+ game re-splits to the same height each turn, and
/// Anchorhead's own status line ("Outside the Real Estate Office / day one")
/// would otherwise be appended to the story on every single move.
#[test]
fn the_status_line_never_leaks_into_the_transcript() {
    let Some(mut session) = boot_anchor() else { return };
    // Advance past the intro to real play.
    for _ in 0..4 {
        if matches!(session.pending_input(), InputKind::Char) {
            let _ = session.submit_char(13);
        }
    }
    let _ = session.take_transcript();

    for cmd in ["look", "wait", "look"] {
        let r = session.submit(cmd);
        assert!(
            !r.transcript.contains("day one"),
            "{cmd:?}: the status line's own text must never reach the story stream: {:?}",
            r.transcript
        );
    }
}
