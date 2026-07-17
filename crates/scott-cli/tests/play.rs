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
    // The authentic Scott prompt, never the Infocom `>`.
    assert!(transcript.contains("Tell me what to do ?"), "prompt shown");
    assert!(!transcript.contains('>'), "no Infocom-style prompt");
    // The room block (top "window") is printed inline, with its exits line.
    assert!(transcript.contains("Obvious exits:"), "room block shown:\n{transcript}");
    // The piped command is echoed after the prompt so the transcript reads
    // naturally (e.g. "Tell me what to do ? down").
    assert!(transcript.contains("Tell me what to do ? down\n"), "command echoed when piped");
    // "down" moves room 1 -> room 2.
    assert!(
        transcript.contains("You descend into darkness"),
        "the move happened:\n{transcript}"
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
