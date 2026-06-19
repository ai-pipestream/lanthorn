// CZECH / Praxix regression acceptance gate — Task 16.
//
// Drives CZECH/Praxix headlessly: feed no input (they auto-run), collect all
// output, assert the suite reports success and zero failures.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

/// Build a `Machine` with a buffer sink, call `init_caps()`, and run until
/// `Quit` (or the step limit).  Returns all captured output as a String.
///
/// CZECH/Praxix auto-run with no input, but we handle `NeedLine`/`NeedChar`
/// defensively (supply empty input) so the runner can't hang.
fn run_to_quit(story: Vec<u8>) -> String {
    let mem = Memory::new(story).expect("Memory::new failed");
    let mut machine = Machine::new(mem);
    machine.init_caps();

    const MAX_STEPS: u64 = 10_000_000;
    for _ in 0..MAX_STEPS {
        match machine.step() {
            StepResult::Quit => break,
            StepResult::Continue => {}
            StepResult::Restart => break, // shouldn't happen in CZECH
            StepResult::NeedLine { .. } => {
                machine.supply_line("");
            }
            StepResult::NeedChar => {
                machine.supply_char(b'\n');
            }
            StepResult::SaveRequest => {
                machine.complete_save(false);
            }
            StepResult::RestoreRequest => {
                machine.complete_restore_failure();
            }
        }
    }

    // Extract captured output from the buffer sink.
    machine
        .buffer_output()
        .map(|b| b.buf.clone())
        .unwrap_or_default()
}

#[test]
fn czech_reports_no_failures() {
    let Some(story) = zvm::fixtures::load("czech.z5") else {
        // Skip if fixture absent.
        return;
    };
    let out = run_to_quit(story);
    // Print full output for debugging during development.
    println!("CZECH output:\n{out}");

    // Hard-coded section presence: all major sections must run.
    for section in &["Jumps", "Variables", "Arithmetic ops", "Logical ops",
                     "Memory", "Subroutines", "Objects", "Indirect Opcodes",
                     "Misc"] {
        assert!(
            out.contains(section),
            "CZECH missing section {section:?}:\n{out}"
        );
    }

    // Extract "Passed- NNN" and "Failed- NNN" from the summary line.
    // CZECH format: "Passed- 406. Failed- 0. Print tests- 19"
    let passed: u32 = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.starts_with("Passed-") || l.starts_with("Passed- ") {
                l.split_whitespace().nth(1).and_then(|s| s.trim_end_matches('.').parse().ok())
            } else {
                None
            }
        })
        .unwrap_or(0);
    let failed: u32 = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.contains("Failed-") {
                // "Failed- 0." appears mid-line in the summary
                l.split("Failed-")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.trim_end_matches('.').parse().ok())
            } else {
                None
            }
        })
        .unwrap_or(u32::MAX);

    assert!(
        passed >= 406,
        "CZECH passed {passed} tests, expected >= 406:\n{out}"
    );
    assert!(
        failed == 0,
        "CZECH reported {failed} failure(s):\n{out}"
    );
}

#[test]
fn praxix_reports_no_failures() {
    let Some(story) = zvm::fixtures::load("praxix.z5") else {
        return; // skip if absent
    };
    let out = run_to_quit(story);
    println!("Praxix output:\n{out}");
    assert!(
        !out.to_lowercase().contains("fail"),
        "Praxix reported failures:\n{out}"
    );
}
