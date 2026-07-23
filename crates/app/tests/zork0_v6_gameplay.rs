//! Zork0 v6 gameplay smoke (SQ-0186, Lane S of
//! `docs/superpowers/plans/2026-07-22-v6-completion.md`).
//!
//! Drives `stories/zork0-r393-s890714.z6` headlessly past boot and asserts:
//! no fault; the banner **compass overlay** — Zork0 draws 8 direction-
//! indicator tiles (picture numbers in `9..=24`) into window 1 at the
//! Rect-derived banner centre `x=139, y=1` — fires correctly for the initial
//! room; a single safe movement command doesn't fault and leaves window 0's
//! box unchanged. Mirrors the skip-if-missing / Pict-source setup pattern in
//! `zork0_v6_windows.rs`.
//!
//! IMPORTANT FINDING (see the doc comment on the second test below): a second
//! command submitted after a room-changing move (any direction) deterministically
//! faults, so this file does **not** drive a second turn after a move — doing so
//! would make the test fail. The fault is reported, not asserted as a pass/fail
//! condition, per the "don't fix, don't fail" rule for smoke tests that expose bugs.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Compass overlay tile picture numbers (banner direction indicators).
const COMPASS_PICS: std::ops::RangeInclusive<u16> = 9..=24;

/// A `@draw_picture(...)` trace line is a compass-overlay redraw iff its
/// picture number falls in `COMPASS_PICS` AND it targets window 1 at the
/// banner centre (y=1, x=139).
fn is_compass_draw(line: &str) -> bool {
    if !line.starts_with("@draw_picture(") {
        return false;
    }
    if !(line.contains("window=1") && line.contains("x=139") && line.contains("y=1")) {
        return false;
    }
    let Some(rest) = line.strip_prefix("@draw_picture(number=") else { return false };
    let Some(comma) = rest.find(',') else { return false };
    rest[..comma].parse::<u16>().map(|n| COMPASS_PICS.contains(&n)).unwrap_or(false)
}

#[test]
fn zork0_v6_gameplay_smoke_boot_compass_and_one_safe_move() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();

    // trace_from_boot = true: capture screen-op tracing from the very first
    // boot instruction (via `take_screen_trace`, `Engine` trait) so the
    // boot-time compass draws are visible.
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, true, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");

    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot: {:?}", session.machine.fault_trace);

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    // Snapshot window 0's box (the main text/status window) before any moves,
    // to assert the layered model stays sane (no drift/underflow) across the
    // one move driven below.
    let window0_before = {
        let v6 = session.machine.screen.v6.as_ref().expect("v6 story must populate ScreenState.v6");
        let w = &v6.windows[0];
        (w.x_coord, w.y_coord, w.x_size, w.y_size)
    };

    // (a) Compass overlay fires correctly for the initial room: Zork0 draws 8
    // direction-indicator tiles into window 1 at the banner centre during boot
    // (the game's first "room entry" event). This is the real, verified
    // compass-overlay behavior; see the module doc comment for why a *second*,
    // movement-triggered re-fire could not be safely exercised here.
    let boot_trace = session.take_screen_trace();
    let boot_compass_draws: Vec<&String> = boot_trace.iter().filter(|l| is_compass_draw(l)).collect();
    eprintln!("boot screen trace: {} lines total", boot_trace.len());
    for l in &boot_compass_draws {
        eprintln!("  boot compass draw: {l}");
    }
    assert_eq!(
        boot_compass_draws.len(),
        8,
        "expected 8 compass-overlay draw_picture events (number in 9..=24, window=1, y=1, x=139) at boot, got: {boot_trace:?}"
    );
    for l in &boot_compass_draws {
        // Every compass tile's picture number must be in the direction-tile range.
        assert!(is_compass_draw(l), "not a compass draw: {l}");
    }

    // Clear the boot transcript so the move's own output is isolated.
    let _ = session.take_transcript();

    // (b) Drive exactly one safe move ("ne" — Zork Zero's opening narration
    // literally points the player northeast: "An insistent finger points
    // northeast."). Assert no fault, no quit.
    assert_eq!(
        session.pending_input(),
        app::session::InputKind::Line,
        "expected a line-input prompt before submitting the move"
    );
    let result = session.submit("ne");
    assert!(!result.quit, "Zork0 quit on the \"ne\" move");
    assert!(result.fault.is_none(), "Zork0 faulted on the \"ne\" move: {:?}", result.fault);

    // Document (not assert) the screen ops this move actually emits: in the
    // observed trace it's `@split_window(...)` (a v6 legacy no-op, per the
    // comment in `exec.rs`'s 0x0A arm) and `@erase_line(0)` — no draw_picture,
    // i.e. no compass redraw is emitted within the SAME turn as the move
    // itself. See the module doc comment for what happens on the *next* turn.
    let move_trace = session.take_screen_trace();
    eprintln!("\"ne\" move screen trace ({} lines): {move_trace:?}", move_trace.len());

    // (c) The layered v6 window model stays sane after the move: window 0's
    // box is unchanged (only banner/graphics content would change, not the
    // text window's own geometry).
    let v6 = session.machine.screen.v6.as_ref().expect("v6 story must still populate ScreenState.v6");
    let w = &v6.windows[0];
    assert_eq!(
        (w.x_coord, w.y_coord, w.x_size, w.y_size),
        window0_before,
        "window 0 box drifted after the \"ne\" move"
    );

    let final_screen = session.screen();
    assert!(
        matches!(final_screen.root, app::engine::WinNode::Layered(_)),
        "v6 story's screen() root must stay WinNode::Layered after the move, got {:?}",
        final_screen.root
    );
}

/// Regression guard for a high-severity v6 VM fault (Lane F): before the fix,
/// submitting **any** command on the turn AFTER a room-changing move
/// deterministically faulted the VM with
///
/// ```text
/// memory fault: read8 @0x000a9438
/// PC=0x01296e  op=op:Two/0x0a   (2OP:10 = test_attr)
/// ```
///
/// Root cause: Zork Zero's post-room-change display code walks its window-record
/// pool and deterministically derives a record *address* (0xc118) which it then
/// hands to `test_attr` as if it were an object number, guarded only by
/// `je gb1, #021c`. The story assumes the interpreter answers object queries on
/// such an out-of-range "object" with the null-object result (attribute clear)
/// rather than reading past the story image — our stricter bounds check faulted
/// the whole VM instead. The engine now treats an out-of-range object number as
/// the null object (see `objects::addressable`), matching lenient reference
/// interpreters, so the crash is gone.
///
/// This drives the exact sequence that used to fault — a room-changing move
/// followed by several more turns — supplying whatever input kind the game asks
/// for, and asserts the VM never faults and never quits. (Note: Zork Zero's full
/// post-move display path still exercises v6 window-model behaviour that is not
/// yet complete in this engine, so this test verifies the *crash* is fixed and
/// the VM stays responsive, not full intro playability.)
#[test]
fn zork0_v6_gameplay_turns_after_move_do_not_fault() {
    use app::session::InputKind;

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
    let _ = session.take_transcript();

    // Turn 1 is a room-changing move ("ne" — the opening narration points the
    // player northeast); it sets up the state that used to make the NEXT turn
    // fault. Turns 2..N then drive the exact post-move code path — a mix of
    // line commands and, once the game enters its keypress-driven display, single
    // keypresses. Every interaction must complete without a VM fault or quit.
    assert_eq!(session.pending_input(), InputKind::Line, "expected a line prompt before the move");

    let mut line_cmds = ["ne", "look", "n", "wait"].into_iter();
    for turn in 0..12 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit(line_cmds.next().unwrap_or("wait")),
            InputKind::Char => session.submit_char(b' '),
            InputKind::Event => session.submit(""),
        };
        assert!(
            !result.quit,
            "Zork0 quit on post-move interaction {turn} (should stay in play)"
        );
        assert!(
            result.fault.is_none(),
            "Zork0 faulted on post-move interaction {turn} — the high-severity bug: {:?}",
            result.fault
        );
        let _ = session.take_transcript();
    }

    // The story image is only 0x49400 bytes, yet the pre-fix fault read object
    // 0xc118's entry at 0x000a9438. Confirm no such out-of-range object fault is
    // latched anywhere in the run.
    assert!(
        session.machine.fault_trace.is_none(),
        "a VM fault was latched during post-move play: {:?}",
        session.machine.fault_trace
    );
}

/// True if the soft-function-key definition screen ("Function Keys …") is
/// showing. Zork Zero renders that screen into a v6 window *grid* (SOFT-WINDOW =
/// window 2) via `DIROUT`/`PRINTT`, not the scrolling transcript, so scan both.
/// Its glyphs land one-per-run in the grid, so the joined-without-separator text
/// reads "Function Keys …".
fn function_key_screen_showing(session: &GameSession, transcript: &str) -> bool {
    if transcript.contains("Function Key") {
        return true;
    }
    if let Some(v6) = session.machine.screen.v6.as_ref() {
        for w in v6.windows.iter() {
            let joined: String = w.texts.iter().map(|t| t.text.as_str()).collect();
            if joined.contains("Function Key") {
                return true;
            }
        }
    }
    false
}

/// Boot fresh and return a ready-to-play session (no boot-time screen tracing).
fn boot_session() -> Option<GameSession> {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");
    assert!(!session.quit, "Zork0 quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Zork0 faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// SQ-0452 regression: a normal typed command must actually *dispatch* — parse to
/// a real action and return the player to the main line prompt — instead of
/// soft-locking into the phantom `V-DEFINE` function-key screen. Before the v6
/// user-stack opcode fix (`push_stack`/`pop_stack`/`pull` on a user stack) the
/// parser's syntax-template walk thrashed and produced PRSA=0, dispatching verb 0
/// (`DEFINE`) which reads a keypress forever.
///
/// Drives boot → "look" → several more verb/direction commands and asserts every
/// turn stays a normal line prompt with no fault, no quit, and the function-key
/// screen never appears.
#[test]
fn zork0_v6_gameplay_look_then_commands_dispatch_normally() {
    use app::session::InputKind;
    let Some(mut session) = boot_session() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };

    for (turn, cmd) in ["look", "i", "wait", "look", "examine me"].into_iter().enumerate() {
        assert_eq!(
            session.pending_input(),
            InputKind::Line,
            "turn {turn}: expected a line prompt (typed commands must dispatch, not soft-lock the fkey screen) before {cmd:?}"
        );
        let result = session.submit(cmd);
        let transcript = session.take_transcript();
        assert!(!result.quit, "turn {turn} ({cmd:?}) quit the game");
        assert!(result.fault.is_none(), "turn {turn} ({cmd:?}) faulted: {:?}", result.fault);
        assert!(
            !function_key_screen_showing(&session, &transcript),
            "turn {turn} ({cmd:?}) fell into the Function Key screen (SQ-0452 phantom V-DEFINE)"
        );
    }
}

/// SQ-0452 regression for the harder DIRECTION path: a bare direction command
/// (verbless move) drives Zork Zero's direction-specific syntax path, which needs
/// the v6 user-stack `pull` to carry a store byte (`pull stack -> result`) so the
/// popped token pointer survives. Without it the direction walk never advances and
/// dispatches the function-key screen.
///
/// Drives boot → "ne" (the opening narration points northeast) → several more
/// moves and commands, asserting every turn dispatches normally.
#[test]
fn zork0_v6_gameplay_move_then_commands_dispatch_normally() {
    use app::session::InputKind;
    let Some(mut session) = boot_session() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };

    for (turn, cmd) in ["ne", "look", "n", "e", "wait", "s", "i"].into_iter().enumerate() {
        assert_eq!(
            session.pending_input(),
            InputKind::Line,
            "turn {turn}: expected a line prompt before the move/command {cmd:?} (directions must dispatch, not soft-lock)"
        );
        let result = session.submit(cmd);
        let transcript = session.take_transcript();
        assert!(!result.quit, "turn {turn} ({cmd:?}) quit the game");
        assert!(result.fault.is_none(), "turn {turn} ({cmd:?}) faulted: {:?}", result.fault);
        assert!(
            !function_key_screen_showing(&session, &transcript),
            "turn {turn} ({cmd:?}) fell into the Function Key screen (SQ-0452 direction-path soft-lock)"
        );
    }
}
