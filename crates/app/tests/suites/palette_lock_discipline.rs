//! One rule, checked mechanically: a suite that sets the process-global palette
//! must take the shared lock (SQ-0905).
//!
//! **A mutex only excludes the parties that take it.** SQ-0904 gave every suite that
//! already declared a `PALETTE` mutex the shared [`app::V6_PALETTE_LOCK`], which
//! fixed twenty-three suites that were each locking their own private mutex —
//! indistinguishable from one lock under nextest, from none under `cargo test`. But
//! three files set `zvm::screen::set_palette` without acquiring anything at all, and
//! those do not merely go unprotected: they can flip the palette in the middle of a
//! lock-holder's case, and the lock-holder has no way to notice.
//!
//! Every file under `tests/suites/` is compiled into one of the ~14 group binaries,
//! so any two of them may share a process and run on parallel threads. That is what
//! makes this a rule about the directory rather than about individual files.
//!
//! # Why this case exists at all rather than a one-off sweep
//!
//! The sweep is the easy half. The hard half is that the next suite to set a palette
//! will be written by someone who has no reason to know any of the above, and the
//! failure it causes is invisible to the gate — `cargo nextest run` gives every test
//! its own process, so the race is structurally unobservable there, and CI's
//! `cargo test` is the only command that can see it. A source-level assertion is the
//! cheapest thing that catches it at the moment the file is added.
//!
//! # What is deliberately NOT covered
//!
//! `crates/zvm/tests/amiga_palette.rs` sets the palette and takes no lock, correctly:
//! it is its own test binary, it holds three cases, and both of its writes plus every
//! palette-dependent assertion sit inside ONE of them — the other two use only the
//! pure `amiga_true_colour`/`zmsd_true_colour`, which do not read the global. Its
//! isolation argument is at the binary level and holds. `zvm` also takes zero
//! external dependencies, so it could not share this lock even if it wanted to.
//!
//! Production callers are exempt for the same kind of reason: `startup.rs` and
//! `zvm-cli` are single-threaded and set one machine's palette per run.

use std::path::PathBuf;

fn suites_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/suites")
}

/// Every suite that writes the process-global palette also takes the shared lock.
///
/// Asserted over the source, because there is no runtime signal for it: a suite that
/// ignores the lock produces a green run until the day its scheduling happens to
/// interleave with a lock-holder, and then produces a failure in the OTHER suite.
///
/// Falsified by deleting the `V6_PALETTE_LOCK` line from any guarded suite, which
/// names that file here.
#[test]
fn a_suite_that_sets_the_palette_takes_the_shared_lock() {
    let dir = suites_dir();
    let entries = std::fs::read_dir(&dir).expect("the suites directory is part of the checkout");
    let (mut setters, mut unguarded) = (0usize, Vec::new());
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("a suite in the checkout is readable");
        // A plain substring scan, which is the right trade for a guard nobody will
        // maintain — but it cannot tell a CALL from a MENTION, and a suite that
        // merely names the function in a comment is reported as a setter. If that
        // happens, reword the comment rather than taking a lock the file does not
        // need; the guard erring toward noise is what makes it safe to leave alone.
        if !src.contains("zvm::screen::set_palette") {
            continue;
        }
        setters += 1;
        if !src.contains("V6_PALETTE_LOCK") {
            unguarded.push(path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string());
        }
    }
    unguarded.sort();
    assert!(
        unguarded.is_empty(),
        "these suites set the process-global palette without taking app::V6_PALETTE_LOCK: {unguarded:?}\n\
         Every file under tests/suites/ shares a group binary with a dozen others, and cargo test \
         runs a binary's cases on parallel threads. A mutex only excludes the parties that take it, \
         so an unguarded setter can flip the palette under a lock-holder — which fails the OTHER \
         suite, and only sometimes (SQ-0904/SQ-0905). Add:\n\
         \x20   static PALETTE: &std::sync::Mutex<()> = &app::V6_PALETTE_LOCK;\n\
         and open each case that stands up a session with\n\
         \x20   let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());",
    );
    assert!(
        setters >= 25,
        "only {setters} suites appear to set the palette — this case is looking in the wrong place \
         or matching the wrong string, and would pass vacuously",
    );
}
