//! End-to-end harness test: run the compiled `scott-cli` binary on the shared
//! "Tiny Cave" fixture with a piped command script and assert on the captured
//! transcript. This is the headless smoke path the binary exists to provide.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The fixture lives in the `scott` crate's tests dir, one level up.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scott/tests/tiny_cave.dat")
}

/// A fresh empty directory for a case that writes saves.
fn scratch(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d =
        std::env::temp_dir().join(format!("scott-cli-play-{name}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run the binary with `stdin_script` piped in; return captured stdout.
fn play(stdin_script: &str, extra_args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scott-cli"));
    cmd.arg(fixture());
    cmd.args(extra_args);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("scott-cli spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("scott-cli runs");
    assert!(out.status.success(), "scott-cli exited non-zero");
    String::from_utf8(out.stdout).expect("stdout is utf-8")
}

#[test]
fn plays_fixture_and_echoes_prompt() {
    // Seed matches the golden test; the only occurrence is noun=100 (always).
    let transcript = play("down\n", &["--seed", "1"]);

    // Opening room description is emitted before the first prompt.
    assert!(
        transcript.contains("sunlit forest clearing"),
        "intro room described:\n{transcript}"
    );
    // The authentic Scott prompt, never the Infocom `> ` command prompt. (The
    // separator rule below legitimately contains '>', so match the prompt shape.)
    assert!(transcript.contains("Tell me what to do ?"), "prompt shown");
    assert!(!transcript.contains("> "), "no Infocom-style '> ' prompt");
    // The room block (top "window") is printed inline, with its exits line and the
    // authentic <—…—> divider between it and the command area.
    assert!(transcript.contains("Obvious exits:"), "room block shown:\n{transcript}");
    assert!(transcript.contains("<\u{2014}"), "separator rule shown:\n{transcript}");
    // The piped command is echoed after the prompt so the transcript reads
    // naturally (e.g. "Tell me what to do ? down").
    assert!(transcript.contains("Tell me what to do ? down\n"), "command echoed when piped");
    // "down" moves room 1 -> room 2 (the dark chamber), where the always-on
    // water-dripping occurrence fires. The move is via the room's exit table:
    // ScottFree resolves GO + a direction before the action list, so a GO-action
    // never intercepts it (and its flavor never prints).
    assert!(
        transcript.contains("water dripping"),
        "the move to the dark chamber happened:\n{transcript}"
    );
}

#[test]
fn max_turns_caps_a_scripted_run() {
    // Feed more commands than the cap; only the first is consumed.
    let transcript = play("look\nlook\nlook\n", &["--max-turns", "1"]);
    let turns = transcript.matches("Tell me what to do ?").count();
    assert_eq!(turns, 1, "exactly one command prompted before the cap:\n{transcript}");
}

#[test]
fn win_script_quits_cleanly() {
    // The golden win script; the final `score` fires the win action (quit).
    let script = "\
push button\npull lever\npush button\ncount\ntally\nstash\ntally\nmark\nstash\ntally\n\
take lamp\ndown\nrub lamp\nget idol\nup\ndown\ndown\nscore\ndrop idol\nscore\n";
    let transcript = play(script, &["--seed", "1"]);
    assert!(transcript.contains("*** You have won! ***"), "win reached:\n{transcript}");
}

/// SQ-0616. Screen-reader mode stops narrating the status every turn, which
/// takes the score with it — so a score that moves is announced instead. Scott
/// stores no score at all: it is the count of treasures in the treasure room,
/// recounted each turn, and `drop idol` in the win script is what moves it.
#[test]
fn a_score_change_is_announced_in_screen_reader_mode() {
    let script = "\
push button\npull lever\npush button\ncount\ntally\nstash\ntally\nmark\nstash\ntally\n\
take lamp\ndown\nrub lamp\nget idol\nup\ndown\ndown\nscore\ndrop idol\nscore\n";
    let transcript = play(script, &["--seed", "1", "--screen-reader"]);
    assert!(
        transcript.contains("[Score 1, up 1]"),
        "depositing the idol should be announced:\n{transcript}"
    );
    // Exactly once — the score is recounted every turn and must not be
    // re-announced while it sits unchanged.
    assert_eq!(transcript.matches("[Score 1, up 1]").count(), 1, "announced once only");
}

/// ...and not at all outside screen-reader mode, where the game's own SCORE
/// verb is the way to ask and an extra line would change every transcript.
#[test]
fn no_score_announcement_without_screen_reader_mode() {
    let script = "take lamp\ndown\nrub lamp\nget idol\nup\ndown\ndown\ndrop idol\n";
    let transcript = play(script, &["--seed", "1"]);
    assert!(!transcript.contains("[Score"), "no announcement by default:\n{transcript}");
}

// ── host save / restore (SQ-0919) ─────────────────────────────────────────────

/// The real binary, a piped script, and a state change that has to survive it.
///
/// Scott has no save protocol of its own, which is not the same as the host being
/// unable to save — the header used to run those two together and that is how this
/// stayed missing while the other two CLIs had it. `scott::Vm::snapshot`/`restore`
/// have been there all along; this is the loop finally reaching them.
#[test]
fn a_host_save_and_restore_survive_a_move() {
    let dir = scratch("roundtrip");
    let transcript = play(
        "/save here\ndown\n/restore here\n",
        &["--seed", "1", "--pager", "off", "--data-dir", dir.to_str().unwrap()],
    );

    assert!(transcript.contains("Saved to"), "the save reports where it went:\n{transcript}");
    assert!(transcript.contains("Restored from"), "and the restore says so:\n{transcript}");

    // The restore has to SHOW you where it put you. The outer loop only prints the
    // room when it changes and cannot see a change made from inside the input
    // loop, so without an explicit redraw the player is silently teleported.
    let after = transcript.split("Restored from").nth(1).unwrap_or_default();
    assert!(
        after.contains("sunlit forest clearing"),
        "the room is redrawn after a restore:\n{transcript}"
    );

    // …and it really landed under `.sav`, because these bytes are not Quetzal.
    let saves: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .flat_map(|e| std::fs::read_dir(e.path()).ok())
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(saves, vec!["here.sav".to_string()], "one save, named honestly");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A number at the restore prompt picks from the listed saves (SQ-0918), and the
/// list is printed before the prompt so there is something to count.
#[test]
fn a_bare_restore_lists_the_saves_and_takes_a_number() {
    let dir = scratch("bynumber");
    let transcript = play(
        "/save alpha\ndown\n/restore\n1\n",
        &["--seed", "1", "--pager", "off", "--data-dir", dir.to_str().unwrap()],
    );
    assert!(transcript.contains("saves: 1 alpha"), "the list is shown:\n{transcript}");
    assert!(transcript.contains("Restored from"), "and the number picked it:\n{transcript}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Saving over a name asks first, and a refusal leaves the earlier save alone —
/// the guard `zvm-cli` got under SQ-0918, which `scott-cli` had no save to need.
#[test]
fn saving_over_a_name_asks_and_a_no_keeps_the_original() {
    let dir = scratch("overwrite");
    let transcript = play(
        "/save keep\n/save keep\nn\n",
        &["--seed", "1", "--pager", "off", "--data-dir", dir.to_str().unwrap()],
    );
    assert!(transcript.contains("already exists. Overwrite? (y/N)"), "asks:\n{transcript}");
    assert!(transcript.contains("Save cancelled."), "and a no is a no:\n{transcript}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A bare word is the adventure's, not ours.** `save` and `restore` are
/// ordinary things to type at a Scott prompt; a host that swallowed them would be
/// worse than no feature at all.
#[test]
fn an_unslashed_save_still_reaches_the_game() {
    let dir = scratch("bareword");
    let transcript = play(
        "save\n",
        &["--seed", "1", "--pager", "off", "--data-dir", dir.to_str().unwrap()],
    );
    assert!(!transcript.contains("Save as ?"), "the host must not claim it:\n{transcript}");
    assert!(!transcript.contains("Saved to"), "nor save:\n{transcript}");
    assert!(
        std::fs::read_dir(&dir).unwrap().flatten().next().is_none(),
        "and nothing reached disk",
    );
    let _ = std::fs::remove_dir_all(&dir);
}
