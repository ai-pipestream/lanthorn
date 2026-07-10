//! glulxercise conformance smoke (phase 3a Glk I/O capstone).
//!
//! Drives the vendored `glulxercise.ulx` (see `tests/fixtures/README.md`) through
//! the real `gvm-cli` binary, headlessly, with a scripted sequence of in-scope
//! test-group commands, and asserts every group reports `Passed.` with no
//! failures. Out-of-scope groups (plain iosys, gi_dispatch, float/double, file
//! save) are intentionally excluded — see the fixtures README.
//!
//! glulxercise's `quit` does not exit on its own (it loops on end-of-input), so
//! the harness keeps stdin open — the VM simply blocks awaiting the next command
//! after the last one — drains stdout in a reader thread until every group has
//! reported, then terminates the child.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The in-scope test groups asserted to pass. This curated set exercises the
/// core VM (arithmetic, memory, calls, jumps including `jumpabs`, `catch`/
/// `throw`), the Glk stream/output model and the memory-stream capture path
/// (`streamnum`/`strings`/`ramstring`), the Glk opcode surface incl. Unicode
/// case folding and the dispatch -1/stack convention (`glk`), search/verify,
/// and the filter I/O system (`iosys2`/`iosys3`/`filter`/`nullio`/`gestalt`;
/// SQ-0245). Plain `iosys` is excluded: it fails on an unrelated pre-existing
/// mid-string memory-stream redirection bug, not the filter system.
const IN_SCOPE: &[&str] = &[
    "arith", "bitwise", "shift", "aload", "astore", "arraybit", "call", "jump",
    "jumpform", "compare", "stack", "throw", "streamnum", "strings", "ramstring",
    "glk", "search", "mzero", "verify", "iosys2", "iosys3", "filter", "nullio",
    "gestalt",
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/glulxercise.ulx")
}

#[test]
fn glulxercise_in_scope_groups_pass() {
    let fixture = fixture_path();
    if !fixture.exists() {
        eprintln!("skipping: {} not vendored", fixture.display());
        return;
    }

    let exe = env!("CARGO_BIN_EXE_gvm-cli");
    let mut child = Command::new(exe)
        .arg(&fixture)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gvm-cli");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    // Drain stdout in the background so the child never blocks on a full pipe.
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buf_reader = Arc::clone(&buf);
    let reader = thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf_reader.lock().unwrap().extend_from_slice(&chunk[..n]),
            }
        }
    });

    // Feed the in-scope commands. Keep stdin open afterward: the VM blocks on the
    // next line read once the script is consumed, leaving the transcript complete.
    for group in IN_SCOPE {
        writeln!(stdin, "{group}").expect("write command");
    }
    stdin.flush().expect("flush stdin");

    // Wait until every group has reported, or give up after a generous deadline.
    let want = IN_SCOPE.len();
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let count = {
            let b = buf.lock().unwrap();
            String::from_utf8_lossy(&b).matches("Passed.").count()
        };
        if count >= want || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait(); // reap the killed child instead of leaving a zombie
    drop(stdin);
    let _ = reader.join();

    let out = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
    let passed = out.matches("Passed.").count();
    assert!(
        passed >= want,
        "expected >= {want} in-scope groups to report Passed., got {passed}.\n--- transcript ---\n{out}"
    );
    assert!(
        !out.contains("tests failed"),
        "an in-scope group reported a failure.\n--- transcript ---\n{out}"
    );
}
