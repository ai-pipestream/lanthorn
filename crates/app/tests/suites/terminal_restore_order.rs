//! The order in which `restore_terminal` hands the terminal back, pinned in source.
//!
//! Teardown is not the reverse of setup by accident — it has to be. Between
//! `disable_raw_mode()` and `DisableMouseCapture` the terminal is still in mode 1003
//! any-motion reporting while the tty is back in ICANON+ECHO, so every report
//! generated in that window goes to the line discipline instead of to us and is
//! handed to whatever reads stdin next. SQ-0998 is that window observed from the
//! outside: a shell prompt left carrying `35;154;45M`, the tail of
//! `ESC [ < 35;154;45 M` — an SGR (1006) motion report at column 154, row 45 — after
//! the shell consumed the `ESC [ <` as an escape and kept the rest as typed input.
//!
//! # Why a source check rather than a behavioural one
//!
//! The symptom lives in the *shell*, one process later. Reproducing it needs a real
//! terminal emulator generating real motion reports, a real line discipline, and a
//! real shell reading the queue we left behind — none of which a test harness has,
//! and the pty-stream layer (`tests/pty_stream/`) can only show the bytes lanthorn
//! WROTE, never what the tty handed to its successor. What is checkable is the one
//! thing that caused it: the sequence of calls. So this pins the sequence, in the
//! spirit of `palette_lock_discipline` — the next person to edit `restore_terminal`
//! has no reason to know any of the above, and every other instrument in the repo is
//! blind to the mistake.
//!
//! Falsified by moving the `disable_raw_mode()` line back above the `execute!`, which
//! fails [`restore_terminal_leaves_raw_mode_last`] naming exactly that.

use std::path::PathBuf;

fn app_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read_src(rel: &str) -> String {
    std::fs::read_to_string(app_src().join(rel)).expect("a source file in the checkout is readable")
}

/// The body of `fn <name>(` in `src`, by brace balance from its opening `{`.
fn fn_body<'a>(src: &'a str, name: &str) -> &'a str {
    let sig = format!("fn {name}(");
    let at = src
        .find(&sig)
        .unwrap_or_else(|| panic!("`fn {name}(` is gone — this case is looking in the wrong place"));
    let open = at + src[at..].find('{').expect("a function has a body");
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..open + i];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces in `fn {name}`");
}

/// Everything that stops the terminal TALKING is written while raw mode still holds;
/// `disable_raw_mode()` comes last.
///
/// `DisableMouseCapture` is the one that produced SQ-0998, but the same window
/// applies to every mode we turned on: bracketed paste replays a paste as raw
/// keystrokes the moment the app stops reading, and mode 1016 turns any report left
/// in flight into pixel coordinates. All three are asserted, because all three are
/// terminal state that outlives the process and all three are undone from this one
/// function.
#[test]
fn restore_terminal_leaves_raw_mode_last() {
    let src = read_src("main.rs");
    let body = fn_body(&src, "restore_terminal");
    let raw = body
        .find("disable_raw_mode()")
        .expect("restore_terminal must still leave raw mode — nothing else does it");
    for marker in ["DisableMouseCapture", "DisableBracketedPaste", "pixel_mouse::RESET"] {
        let at = body.find(marker).unwrap_or_else(|| {
            panic!("restore_terminal no longer issues {marker}; it is terminal state that outlives us")
        });
        assert!(
            at < raw,
            "restore_terminal issues {marker} AFTER disable_raw_mode().\n\
             Between those two lines the terminal is still reporting while the tty is back in \
             ICANON+ECHO, so anything it sends lands in the line discipline and is handed to the \
             shell — SQ-0998 is a prompt left carrying `35;154;45M`, the tail of an SGR motion \
             report. Write every escape while raw mode still holds, and leave raw mode last.",
        );
    }
}

/// And what is already queued is taken off the fd before the shell can have it.
///
/// Disabling reporting stops NEW reports; a report that was already sitting in the
/// tty's input queue when we started tearing down is unaffected by it, and leaving
/// raw mode hands the queue to the next reader. The drain is therefore half the fix,
/// not a refinement of it — the ordering alone still loses anything that arrived
/// between the event loop's last `read()` and the teardown.
#[test]
fn restore_terminal_drains_what_is_already_queued() {
    let src = read_src("main.rs");
    let body = fn_body(&src, "restore_terminal");
    let drain = body.find("drain_pending_input()").expect(
        "restore_terminal no longer drains queued input. Disabling mouse reporting cannot unsend \
         a report already in the tty's input queue, and disable_raw_mode hands that queue to the \
         shell (SQ-0998).",
    );
    let raw = body.find("disable_raw_mode()").expect("restore_terminal must still leave raw mode");
    assert!(
        drain < raw,
        "restore_terminal drains input AFTER disable_raw_mode(), which is too late: the queue is \
         the shell's by then.",
    );
}

/// `restore_terminal` is the only place in the TUI that can leave raw mode with the
/// mouse still captured.
///
/// A fix that repairs one restore path and leaves another inverted has fixed one of
/// several, and the app tears the terminal down from four kinds of place — clean
/// quit, panic hook, signal/watchdog, picker — all of which funnel through this one
/// function today. This case fails when a new `disable_raw_mode()` appears outside
/// it, so the funnel stays a funnel and the ordering above keeps covering every exit
/// path at once.
///
/// The two exemptions are both from BEFORE anything is captured: `picker_ui`'s
/// bail-out when `EnterAlternateScreen` fails, and `term_colors`'s standalone probe,
/// which enables raw mode itself for the length of one query.
#[test]
fn nothing_else_in_the_tui_leaves_raw_mode() {
    const EXEMPT: [&str; 2] = ["picker_ui.rs", "term_colors.rs"];
    let mut offenders: Vec<String> = Vec::new();
    let mut seen_in_main = 0usize;
    let mut stack = vec![app_src()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).expect("the src directory is part of the checkout");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let src = std::fs::read_to_string(&path).expect("a source file in the checkout is readable");
            let hits = src.matches("disable_raw_mode()").count();
            if hits == 0 || EXEMPT.contains(&name.as_str()) {
                continue;
            }
            if name == "main.rs" {
                seen_in_main = hits;
                continue;
            }
            offenders.push(name);
        }
    }
    assert_eq!(
        seen_in_main, 1,
        "main.rs leaves raw mode in {seen_in_main} places; restore_terminal is meant to be the \
         only one, so the ordering this file pins covers every exit path at once (SQ-0998).",
    );
    assert!(
        offenders.is_empty(),
        "these files leave raw mode outside restore_terminal: {offenders:?}\n\
         Every teardown in the TUI has to silence the terminal before it leaves raw mode, or a \
         mouse report generated in the gap reaches the shell (SQ-0998). Route the new path \
         through restore_terminal rather than repeating the sequence.",
    );
}
