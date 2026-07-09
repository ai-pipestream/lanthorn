// SQ-0158 — READ-direction save-format interop.
//
// Proves babelmap's zvm can restore a bare Quetzal `.qzl` save produced by a
// *different* interpreter (`dfrotz`) and land in the same state a native
// play-through would reach. The golden fixture's PC points at the `save`
// instruction's result descriptor (Quetzal §5.8), so this exercises the
// descriptor-COMPLETING restore path (`complete_restore_success`), not a
// bare resume.
//
// See `crates/zvm/tests/fixtures/interop/PROVENANCE.md` for how the golden
// was produced.

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

/// Verbatim commands that reach interop point P: room "North of House",
/// leaflet carried.
const PREFIX: [&str; 3] = ["open mailbox", "take leaflet", "north"];

/// Verbatim commands that reveal the room and the carried leaflet.
const PROBE: [&str; 2] = ["look", "inventory"];

/// Boot a story and run until the first line-read prompt (or a step cap),
/// answering any char-reads with '\n' and refusing save/restore along the
/// way. Mirrors `story_location_verify.rs`'s `boot_to_first_read`.
fn boot_to_first_read(data: Vec<u8>) -> Machine {
    let mem = Memory::new(data).expect("valid story file");
    let mut machine = Machine::new(mem);
    machine.init_caps();
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } => return machine,
            StepResult::Quit | StepResult::Restart | StepResult::Fault => return machine,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
    panic!("boot_to_first_read: never reached a line-read prompt within step cap");
}

/// Drive `machine` for one more turn by supplying `input` as a line, stepping
/// until the next prompt (or a step cap). Mirrors `run_one_turn`.
fn run_one_turn(machine: &mut Machine, input: &str) {
    machine.supply_line(input, 13);
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
    panic!("run_one_turn({input:?}): never reached the next prompt within step cap");
}

/// Drive `machine` until the next line-read prompt (or a step cap) WITHOUT
/// supplying any input line first. Needed right after
/// `complete_restore_success`: the restored PC resumes mid-turn (completing
/// whichever command the foreign interpreter was mid-executing when it saved,
/// e.g. printing "Ok." for the `save` verb itself) before it reaches the next
/// actual prompt.
fn drain_to_next_read(machine: &mut Machine) {
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart | StepResult::Fault => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
    panic!("drain_to_next_read: never reached the next prompt within step cap");
}

fn run_turns(machine: &mut Machine, cmds: &[&str]) {
    for cmd in cmds {
        run_one_turn(machine, cmd);
    }
}

/// Transcript text accumulated in `machine`'s buffer output since `mark`.
fn transcript_since(machine: &Machine, mark: usize) -> String {
    let buf = &machine.buffer_output().expect("buffer output sink").buf;
    buf[mark..].to_string()
}

#[test]
fn zmachine_reads_reference_save() {
    let story = zvm::fixtures::load("minizork.z3").expect("required CI fixture minizork.z3 missing");
    let golden = zvm::fixtures::load("interop/minizork-at-P.qzl")
        .expect("required CI fixture interop/minizork-at-P.qzl missing");

    // Baseline: boot minizork, play PREFIX then PROBE, capture the PROBE-phase transcript.
    let played = {
        let mut machine = boot_to_first_read(story.clone());
        run_turns(&mut machine, &PREFIX);
        let mark = machine.buffer_output().expect("buffer output sink").buf.len();
        run_turns(&mut machine, &PROBE);
        transcript_since(&machine, mark)
    };

    // Cross-load: boot a FRESH minizork, descriptor-complete the foreign
    // dfrotz save, then run the SAME PROBE, capture only the PROBE-phase
    // transcript. Two things must be excluded: the boot banner/initial room,
    // and the tail of dfrotz's own `save` turn that the restored PC resumes
    // mid-execution (it prints "Ok." before reaching the next real prompt).
    let restored = {
        let mut machine = boot_to_first_read(story);
        machine
            .complete_restore_success(&golden)
            .expect("restoring the dfrotz golden save must succeed");
        drain_to_next_read(&mut machine);
        let mark = machine.buffer_output().expect("buffer output sink").buf.len();
        run_turns(&mut machine, &PROBE);
        transcript_since(&machine, mark)
    };

    assert_eq!(
        restored.trim(),
        played.trim(),
        "restoring dfrotz's save must reproduce the state reached by playing the prefix"
    );
    assert!(
        restored.contains("leaflet"),
        "probe output must reveal the mutated state (guards against a vacuous match)"
    );
}
