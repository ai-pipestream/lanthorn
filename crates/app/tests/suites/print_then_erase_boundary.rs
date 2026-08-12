//! SQ-0751: the screen-clear boundary must fall where the erase did, not where the
//! turn started.
//!
//! `erase_window` (ZMSD §8.7.3) reaches the host on v1–5/7/8 as a per-TURN flag,
//! `ScreenState::erase_lower_requested`, and `turn::finish_command_turn` marks the
//! boundary the moment it sees that flag — at the START of the turn. A turn that
//! prints and THEN erases therefore had its pre-erase text recorded as coming AFTER
//! the clear, so the cleared screen opened with text that had already been wiped off
//! it.
//!
//! SQ-0748's sweep found no shipping game that does it: Zork I, Enchanter,
//! Bureaucracy, Trinity, Border Zone, Beyond Zork, LostPig, the four Infocom v6
//! titles, Counterfeit Monkey and Kerkerkruip all erase BEFORE they print, which is
//! exactly what marking at the turn's start describes. That is why the quest insisted
//! any fix be shown to fail without it, and why this suite hand-assembles the shape
//! rather than hunting for a game: a story that prints, erases, and prints again
//! inside one turn.
//!
//! The fix runs the boundary down the interleave channel the v6 boundaries already
//! use (SQ-0697/0755). `Output::screen_cleared` stamps the erase's position in the
//! character stream as the opcode executes, and the turn's output is split around it
//! into `TranscriptElem::Text` / `ScreenClear` / `Text`. Only a genuine mid-turn erase
//! takes that path — an erase at offset 0 is what the turn-start mark already says, so
//! every game in the sweep above stays on the flat path, byte for byte.
//!
//! No fixture: the story is two dozen bytes of hand-assembled v5 opcodes, so this
//! suite never skips.

use app::session::{GameSession, InputKind, TranscriptElem};
use app::state::{AppState, TranscriptKind};

/// A v5 story that blocks on `read`, and whose next turn prints `A`, clears the
/// screen, prints `B`, and blocks again. Header layout mirrors zvm's crate-private
/// `header::tests_support::sample_story`, as the other synthetic suites do.
///
/// - `read` is VAR:0x04 → `0xE4`, type byte `0x3F` (one large constant: the text
///   buffer), followed by the store byte. It is what ends a turn, so the whole
///   print/erase/print sequence lands inside ONE `TurnResult`.
/// - `print_char` is VAR:0x05 → `0xE5`, type byte `0x7F` (one small constant).
/// - `erase_window` is VAR:0x0D → `0xED`, type byte `0x3F` (one large constant),
///   operand `0xFFFF` = −1: "erase the whole screen and unsplit" (ZMSD §8.7.3.3). A
///   large constant is required — −1 does not fit a small one.
fn story_with(turn_body: &[u8]) -> Vec<u8> {
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
    let mut prog: Vec<u8> = READ.to_vec(); // boot parks here, before the turn body
    prog.extend_from_slice(turn_body);
    prog.extend_from_slice(&READ); // …and the body's turn ends here
    prog.push(0xBA); // quit
    buf[0x0040..0x0040 + prog.len()].copy_from_slice(&prog);
    buf
}

/// print `A`, erase the screen, print `B` — the shape no shipping game has.
fn print_then_erase_story() -> Vec<u8> {
    story_with(&[
        0xE5, 0x7F, b'A',       // print_char 'A'   — printed BEFORE the erase
        0xED, 0x3F, 0xFF, 0xFF, // erase_window -1  — the screen clear, mid-turn
        0xE5, 0x7F, b'B',       // print_char 'B'   — printed onto the cleared screen
    ])
}

/// erase the screen, then print `B` — the shape every swept game has.
fn erase_then_print_story() -> Vec<u8> {
    story_with(&[
        0xED, 0x3F, 0xFF, 0xFF, // erase_window -1 — first thing in the turn
        0xE5, 0x7F, b'B',       // print_char 'B'
    ])
}

/// Boot to the first `read`, then run the turn body and return its result.
fn one_turn(story: Vec<u8>) -> app::session::TurnResult {
    let mut s = GameSession::new(story, true, false, None).expect("the synthetic v5 story boots");
    assert_eq!(s.pending_input(), InputKind::Line, "the story blocks on read");
    let _ = s.take_transcript();
    let r = s.submit("");
    assert!(!r.quit && r.fault.is_none(), "the turn neither quit nor faulted");
    r
}

/// The engine half: a mid-turn erase splits the turn's output around the boundary.
///
/// Falsified by removing the `self.out.screen_cleared()` call from `erase_window` in
/// `crates/zvm/src/cpu/exec.rs` (equivalently, by dropping `cleared_mid_turn` from
/// `GameSession::drain_turn`):
///
/// ```text
/// assertion `left == right` failed: a turn that prints, erases, and prints again
/// must reach the host as three ordered elements — the pre-erase text, the boundary,
/// and what was printed onto the cleared screen (SQ-0751)
///   left: []
///  right: ["A", "<clear>", "B"]
/// ```
#[test]
fn a_mid_turn_erase_splits_the_turns_output_around_the_boundary() {
    let r = one_turn(print_then_erase_story());
    assert!(r.erase_lower, "the turn did erase the lower window");
    let shape: Vec<String> = r
        .transcript_elems
        .iter()
        .map(|e| match e {
            TranscriptElem::Text { text, .. } => text.clone(),
            TranscriptElem::ScreenClear => "<clear>".to_string(),
            TranscriptElem::Image(_) => "<image>".to_string(),
        })
        .collect();
    assert_eq!(
        shape,
        vec!["A".to_string(), "<clear>".to_string(), "B".to_string()],
        "a turn that prints, erases, and prints again must reach the host as three \
         ordered elements — the pre-erase text, the boundary, and what was printed \
         onto the cleared screen (SQ-0751)"
    );
}

/// The transcript half: the pre-erase text ends up ABOVE the boundary, so the cleared
/// screen opens with only what was printed onto it.
///
/// `AppState::clear_anchor` is the index the renderer pins the top of the screen to;
/// everything above it stays reachable by scrolling (a screen clear here preserves
/// scrollback, it does not wipe it). Before the fix the whole turn — `A` included —
/// landed below the anchor, i.e. on the cleared screen.
///
/// Falsified the same way as the test above:
///
/// ```text
/// called `Option::expect()` on a `None` value: the turn recorded a screen-clear
/// boundary
/// ```
#[test]
fn the_cleared_screen_opens_with_only_the_post_erase_text() {
    let r = one_turn(print_then_erase_story());

    let mut state = AppState::default();
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);

    let anchor = state.clear_anchor.expect("the turn recorded a screen-clear boundary");
    assert_eq!(
        state.transcript[anchor..],
        ["B".to_string()],
        "the cleared screen shows only what was printed after the erase — the \
         pre-erase \"A\" belongs to the screen that was wiped (SQ-0751)"
    );
    assert_eq!(
        state.transcript[..anchor],
        ["A".to_string()],
        "and the wiped text is still in scrollback above the boundary, not deleted"
    );
    assert!(
        state.transcript_kinds.iter().all(|&k| k == TranscriptKind::Story),
        "both halves are game output"
    );
}

/// The other half of the rule, and the reason nothing else in the corpus moves: an
/// erase at offset 0 — every game SQ-0748 swept — is what marking at the turn's start
/// already describes, and is left on the flat path untouched.
#[test]
fn an_erase_before_any_output_stays_on_the_flat_path() {
    let r = one_turn(erase_then_print_story());
    assert!(r.erase_lower, "the erase still reaches the host as the per-turn flag");
    assert!(
        r.transcript_elems.is_empty(),
        "an erase with nothing printed before it takes no interleave — the turn-start \
         mark already describes it, so the flat transcript path is unchanged"
    );
    assert_eq!(r.transcript, "B", "and the flat path carries the whole turn's output");
}
