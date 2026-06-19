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
    assert!(
        out.contains("Passed") || out.contains("passed") || out.contains("PASSED"),
        "CZECH did not report passing:\n{out}"
    );
    assert!(
        !out.to_lowercase().contains("failed:"),
        "CZECH reported failures:\n{out}"
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
