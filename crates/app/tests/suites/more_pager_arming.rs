//! `[more]` pager arming, driven by real games (SQ-0539 + SQ-0534 app half).
//!
//! The governing directive: *"[more] should work any time output is larger than
//! what fits on the screen (including boot) … we should behave as the original
//! game intended (with exceptions for meta output)."* An authentic Infocom
//! interpreter paged whenever the output since the last player interaction
//! exceeded the screen — **regardless of what input came next**. lanthorn's v1
//! pager (SQ-0404) armed only behind a LINE read on a turn that did not clear the
//! screen, so two whole classes of overflow scrolled away unread:
//!
//! 1. a turn that dumps more than a screenful and then waits on a **keypress**
//!    (`read_char`) — hint pages, credits, "press any key" dumps, menus;
//! 2. a turn that **clears** and then paints more than a screenful.
//!
//! Both now page. See [`app::pager`] for the full arming table.
//!
//! The real game here is `zork0izm.z5` — Zork Zero's standalone InvisiClues hint
//! program, a plain v5 story that is `read_char`-driven end to end. Its credits
//! page is a single keypress turn that adds ~24 wrapped rows, which overflows any
//! story pane narrower than the full terminal. Skip-if-missing (gitignored
//! stories).

use std::path::PathBuf;

use app::engine::Engine;
use app::input::{key_to_action, Action};
use app::pager::{activation_target, more_suppressed, should_arm, Driver, Pager};
use app::session::{GameSession, InputKind};
use app::state::AppState;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(file: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    Some(
        GameSession::new_with_trace(bytes, false, false, None, false, Default::default(), None, None, None)
            .expect("story boots"),
    )
}

/// A v6 story needs the app's real picture setup to boot the way it does in play.
fn boot_v6(file: &str) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = app::graphics::PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_win, Some((2, 9)), None)
        .expect("v6 story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

fn story_state() -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st
}

/// Render `state`'s transcript through the REAL story-pane renderer and return
/// `(total wrapped rows, viewport rows, prompt rows)` — the three numbers the run
/// loop feeds [`activation_target`] after every frame.
fn measure(session: &GameSession, state: &AppState, w: u16, h: u16) -> (u16, u16, u16) {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&session.screen(), true, None, state, area, &mut buf);
    (m.total_rows, m.viewport_rows, m.prompt_rows)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

/// The run loop's char-input gate (main.rs): a plain keypress is forwarded to the
/// VM only when the game wants one, no overlay covers the pane, and — new in
/// SQ-0539 — the `[more]` pager is not showing.
fn char_gate_forwards(state: &AppState) -> bool {
    state.char_mode && !state.any_overlay_open() && !state.pager.active
}

/// A keypress turn that overflows the story pane must PAGE, and the keys that page
/// it must not reach the game's pending `read_char`.
///
/// Directive: *"[more] should work any time output is larger than what fits on the
/// screen … we should behave as the original game intended."* The v1 rule refused
/// to arm whenever the game "switched to char/event input (a keypress menu or
/// 'press any key'), where a keypress must reach the game, not page the pane" —
/// which is precisely backwards for an overflowing turn: the original interpreters
/// paged FIRST and consumed the paging keys themselves.
#[test]
fn char_input_overflow_pages_before_the_read_char_gets_a_key() {
    let Some(mut s) = boot("zork0izm.z5") else { return };
    let mut state = story_state();
    state.push_transcript(&s.take_transcript());
    assert_eq!(s.pending_input(), InputKind::Char, "InvisiClues is keypress-driven from boot");

    // Walk to the credits page: [press any key] → menu → select.
    s.submit_char(b'\r');
    s.submit_char(b'1');

    // A 52x24 story pane — an 80x24 terminal with the map pane docked beside it.
    // The menu is a full-height upper window, so the last frame before the turn
    // reported no transcript viewport at all; that IS the baseline the run loop
    // caches in `last_transcript_total_rows`.
    let (before_rows, _, _) = measure(&s, &state, 52, 24);
    state.last_transcript_total_rows = before_rows;

    let turn = s.submit_char(b'\r');
    state.push_transcript(&turn.transcript);
    assert_eq!(s.pending_input(), InputKind::Char, "the credits page waits on another keypress");
    assert!(
        turn.transcript.lines().count() >= 19,
        "premise: this one keypress prints a real page of credits, got {} lines",
        turn.transcript.lines().count()
    );

    // 1. Arming: the game wants a keypress, and there is no v6 [MORE] veto.
    assert!(
        should_arm(s.pending_input(), more_suppressed(&s)),
        "a keypress read must arm the pager (v1 refused, and the credits scrolled away)"
    );
    let mut pager = Pager::default();
    pager.arm_after_turn(before_rows, s.pending_input(), more_suppressed(&s), Driver::PlayerInput);
    assert_eq!(pager.pending_before_rows, Some(before_rows));

    // 2. Activation, measured by the real renderer.
    let (after_rows, viewport, prompt) = measure(&s, &state, 52, 24);
    eprintln!("zork0izm credits: before={before_rows} after={after_rows} viewport={viewport}");
    let target = activation_target(before_rows, after_rows, viewport, prompt).unwrap_or_else(|| {
        panic!(
            "premise: this keypress turn brings the story pane to {after_rows} rows — {} more than \
             the last frame showed — which must overflow the {viewport}-row viewport",
            after_rows - before_rows
        )
    });
    assert!(target > 0, "the view parks above the bottom, on the FIRST new screenful");
    state.transcript_scroll = target;
    state.pager.active = true;
    state.char_mode = true;

    // 3. Key delivery: while [more] is up the game gets NOTHING. Even a key the
    //    game would happily accept pages instead, and the read stays pending.
    assert!(!char_gate_forwards(&state), "the pager owns the keyboard while it is showing");
    for code in [KeyCode::Char(' '), KeyCode::Enter, KeyCode::Char('n'), KeyCode::Esc] {
        assert!(
            matches!(key_to_action(&state, key(code)), Action::PagerAdvance),
            "{code:?} must page, not answer the read"
        );
    }
    assert_eq!(
        s.pending_input(),
        InputKind::Char,
        "the game's read_char must still be pending — no paging key was delivered"
    );

    // 4. Caught up: the pager steps aside and the very NEXT key reaches the game.
    state.transcript_scroll = 0;
    state.pager.active = false;
    assert!(char_gate_forwards(&state), "with the pager gone, keys are game input again");
    let resumed = s.submit_char(b'\r');
    assert!(resumed.fault.is_none(), "the game resumes on the post-catch-up key: {:?}", resumed.fault);
}

/// The same overflow decision is a function of the pane, not a switch: give the
/// story the whole terminal and the very same turn fits, so no `[more]` appears.
#[test]
fn the_same_char_turn_does_not_page_when_it_fits() {
    let Some(mut s) = boot("zork0izm.z5") else { return };
    let mut state = story_state();
    state.push_transcript(&s.take_transcript());
    s.submit_char(b'\r');
    s.submit_char(b'1');

    let (before_rows, _, _) = measure(&s, &state, 80, 30);
    let turn = s.submit_char(b'\r');
    state.push_transcript(&turn.transcript);
    let (after_rows, viewport, prompt) = measure(&s, &state, 80, 30);
    eprintln!("zork0izm credits at 80x30: before={before_rows} after={after_rows} viewport={viewport}");
    assert_eq!(
        activation_target(before_rows, after_rows, viewport, prompt),
        None,
        "{} added rows fit in {viewport}: nothing was missed, so no [more]",
        after_rows - before_rows
    );
}

/// Boot pages too — including a boot that ends on a keypress read.
///
/// Directive: *"[more] should work any time output is larger than what fits on the
/// screen **(including boot)**."* The wave-5 boot arm (SQ-0532) carried the same
/// line-input-only guard as the turn path, so a story whose opening screen ends on
/// `read_char` — a splash "press any key", a startup menu, this hint program's
/// screen-width warning — booted with its banner pinned to the newest rows.
#[test]
fn boot_that_overflows_and_waits_on_a_keypress_arms_the_pager() {
    let Some(mut s) = boot("zork0izm.z5") else { return };
    let mut state = story_state();
    state.push_transcript(&s.take_transcript());
    assert_eq!(s.pending_input(), InputKind::Char, "this story boots straight into a keypress read");

    assert!(
        should_arm(s.pending_input(), more_suppressed(&s)),
        "startup must arm behind a boot-time read_char (the wave-5 guard did not)"
    );
    // A small pane (a 40-column story column on a short terminal) — the opening
    // warning is 13 wrapped rows there.
    let (total, viewport, prompt) = measure(&s, &state, 40, 12);
    eprintln!("zork0izm boot at 40x12: total={total} viewport={viewport}");
    assert!(
        activation_target(0, total, viewport, prompt).is_some(),
        "the {total}-row opening screen overflows a {viewport}-row pane and must page"
    );
}

/// SQ-0534 (app half): ZMSD §8.8.3.2.6 — "A line count of -999 means 'never print
/// [MORE]'". Zork Zero's demonstration mode parks the count there, and the host
/// must honour it: no `[more]`, no matter how much the game prints.
#[test]
fn v6_never_print_more_vetoes_arming() {
    let Some(mut s) = boot_v6("zork0-r393-s890714.z6") else { return };
    assert!(!more_suppressed(&s), "Zork Zero boots with [MORE] enabled");
    assert!(should_arm(InputKind::Line, more_suppressed(&s)), "…so its turns arm normally");

    // Park the CURRENT window's line count (property 15) at the sentinel — what
    // `put_wind_prop win 15 -999` does from inside the story.
    {
        let v6 = s.machine.screen.v6_mut().expect("a v6 story has v6 windows");
        let cur = (v6.current as usize).min(7);
        v6.windows[cur].line_count = zvm::screen::NEVER_MORE as u16;
    }
    assert_eq!(s.machine.v6_line_count(), Some(zvm::screen::NEVER_MORE));
    assert!(more_suppressed(&s), "the app must see the engine's -999 sentinel");
    for pending in [InputKind::Line, InputKind::Char] {
        assert!(!should_arm(pending, more_suppressed(&s)), "{pending:?} must not arm under the veto");
    }
    let mut pager = Pager::default();
    pager.arm_after_turn(7, InputKind::Char, more_suppressed(&s), Driver::PlayerInput);
    assert!(pager.pending_before_rows.is_none(), "vetoed turns leave the pager unarmed");
}

/// A non-v6 story has no line counter at all, so the veto is safely false — the
/// pager keeps working for every ordinary game.
#[test]
fn the_v6_veto_is_off_for_stories_that_have_no_line_count() {
    let Some(s) = boot("zork0izm.z5") else { return };
    assert_eq!(s.machine.v6_line_count(), None, "a v5 story has no window line count");
    assert!(!more_suppressed(&s));
    assert!(should_arm(InputKind::Char, more_suppressed(&s)));
}

/// The directive's one carve-out: *"with exceptions for meta output"*. lanthorn's
/// own dumps (`/help`, trace pointers, notices) are not game turns — they never
/// reach an arm site — so however long they are, they never raise `[more]`.
#[test]
fn lanthorn_meta_output_never_arms_the_pager() {
    let mut state = story_state();
    state.last_transcript_total_rows = 4;
    let long_help: String = (0..200).map(|i| format!("/help line {i}\n")).collect();
    state.push_transcript_internal(&long_help, app::state::TranscriptKind::Meta);
    assert!(state.transcript.len() > 100, "premise: the meta dump is far taller than any screen");
    assert!(
        state.pager.pending_before_rows.is_none() && !state.pager.active,
        "meta output must never arm or engage the pager"
    );
}
