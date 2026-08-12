//! SQ-0696: Anchorhead's startup quote boxes must be readable, where the game put them.
//!
//! `anchor.z8`'s intro shows two Inform **box quotes** — the Lovecraft epigraph
//! beside the title splash, and `* THE FIRST DAY *` after the prologue. Each is
//! drawn the way the library's `box` statement always has: split the upper
//! window tall, print reverse-video text into it, then shrink the window back to
//! the status line — and only THEN wait for a keypress.
//!
//! Truncating the grid at that shrink destroyed the quote before it could be
//! read, so both screens rendered blank while waiting for a key. ZMSD does not
//! cover the case (§8.6.1.1.2 specifies only Version 3's clear-on-split) and the
//! note its remarks cite names the failure exactly (Plotkin, *Quote Boxes in
//! Z-Machine Games*): "In a naive Glk implementation -- or _any_ simple
//! implementation using text windows -- this trick fails."
//!
//! The upper-window allocation now only ever grows on a split, so the quote
//! stays in the grid at the rows the game painted, and every host renders it
//! from there — the drawn pane, the CLI's pinned region, and `--screen-reader`,
//! where a region taller than one row is read as content rather than quietened
//! as chrome. `erase_window` is what clears it, which is when a real screen
//! loses it too.
//!
//! The story is gitignored, so these skip vacuously when absent.

use std::path::PathBuf;

use app::engine::{Engine, GridWindow, WinNode};
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

fn find_grid(n: &WinNode) -> Option<&GridWindow> {
    match n {
        WinNode::Grid(g) => Some(g),
        WinNode::Pair { first, second, .. } => find_grid(first).or_else(|| find_grid(second)),
        _ => None,
    }
}

/// The upper window's rendered rows as plain text — what every host draws, and
/// what the CLI's pinned region and screen-reader block narrate.
fn grid_rows(s: &GameSession) -> Vec<String> {
    let model = s.screen();
    let Some(g) = find_grid(&model.root) else { return Vec::new() };
    (0..g.active_rows)
        .filter_map(|r| {
            let start = r as usize * g.cols as usize;
            let end = (start + g.cols as usize).min(g.cells.len());
            (start < g.cells.len())
                .then(|| g.cells[start..end].iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
        })
        .collect()
}

/// True when any rendered cell of the grid carries the reverse-video bit — the
/// styling the library paints a quote box with, and what makes it read as a box.
fn grid_has_reverse(s: &GameSession) -> bool {
    let model = s.screen();
    find_grid(&model.root).is_some_and(|g| {
        let shown = (g.active_rows as usize * g.cols as usize).min(g.cells.len());
        g.cells[..shown].iter().any(|c| c.style & 0x01 != 0)
    })
}

#[test]
fn anchor_startup_quote_boxes_stay_in_the_upper_window() {
    let Some(mut session) = boot_anchor() else { return };

    // ── Quote 1: the epigraph, at boot ──────────────────────────────────────
    let boot_text = session.take_transcript();
    assert!(
        boot_text.contains("A N C H O R H E A D"),
        "the title splash is ordinary lower-window text and was never at risk: {boot_text:?}"
    );
    let rows = grid_rows(&session);
    let quote = rows.join("\n");
    assert!(
        quote.contains("The oldest and strongest emotion of mankind"),
        "the boot quote box must survive the upper window shrinking to the status line; grid was {rows:#?}"
    );
    assert!(quote.contains("H.P. Lovecraft"), "…including its attribution: {rows:#?}");
    assert!(grid_has_reverse(&session), "the box keeps the reverse video the library painted it with");
    // Placed, not streamed: the quote belongs to the window, so it must NOT also
    // be appended to the story text (which would duplicate it on every host that
    // renders the grid).
    assert!(
        !boot_text.contains("oldest and strongest"),
        "the quote is placed in the upper window, not pushed into the story stream: {boot_text:?}"
    );

    // ── Quote 2: `* THE FIRST DAY *`, two keypresses later ──────────────────
    assert!(matches!(session.pending_input(), InputKind::Char), "boot waits for a keypress");
    let prologue = session.submit_char(13);
    assert!(
        prologue.transcript.contains("November, 1997"),
        "the prologue screen follows the title: {:?}",
        prologue.transcript
    );
    assert!(
        grid_rows(&session).is_empty(),
        "the prologue's erase_window clears the first quote — the moment a real screen loses it"
    );

    assert!(matches!(session.pending_input(), InputKind::Char), "the prologue waits for a keypress");
    let day_one = session.submit_char(13);
    let rows = grid_rows(&session);
    let quote = rows.join("\n");
    assert!(
        quote.contains("* THE FIRST DAY *"),
        "the second quote box is the WHOLE screen here — without it this turn renders completely \
         blank while waiting for a keypress, the reported symptom; grid was {rows:#?}"
    );
    assert!(quote.contains("I was far from home"), "…and carries the quote body: {rows:#?}");
    assert!(
        day_one.transcript.trim().is_empty(),
        "this screen is the quote and nothing else: {:?}",
        day_one.transcript
    );

    // ── Play resumes with an ordinary one-row status line ───────────────────
    assert!(matches!(session.pending_input(), InputKind::Char), "the quote screen waits for a key");
    let start = session.submit_char(13);
    assert!(
        start.transcript.contains("Outside the Real Estate Office"),
        "play begins after the second quote: {:?}",
        start.transcript
    );
    assert_eq!(
        grid_rows(&session).len(),
        1,
        "the tall quote window must not persist into play as a stale panel"
    );
}

/// The grown allocation must not strand a tall upper window across ordinary
/// turns, and the status line must stay out of the story stream. Every v4+ game
/// re-splits to the same height each turn, so a shrink that kept growing the
/// grid — or one that streamed its rows — would leak Anchorhead's status line
/// ("Outside the Real Estate Office / day one") on every single move.
#[test]
fn ordinary_turns_keep_a_one_row_status_line() {
    let Some(mut session) = boot_anchor() else { return };
    for _ in 0..4 {
        if matches!(session.pending_input(), InputKind::Char) {
            let _ = session.submit_char(13);
        }
    }
    let _ = session.take_transcript();

    for cmd in ["look", "wait", "look"] {
        let r = session.submit(cmd);
        assert_eq!(grid_rows(&session).len(), 1, "{cmd:?}: the status line stays one row");
        assert!(
            !r.transcript.contains("day one"),
            "{cmd:?}: the status line's own text must never reach the story stream: {:?}",
            r.transcript
        );
    }
}
