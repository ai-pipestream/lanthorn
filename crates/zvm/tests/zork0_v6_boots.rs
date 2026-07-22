//! Drives the real v6 story `stories/zork0-r393-s890714.z6` headlessly to its
//! first input request. Reaching input means v6 boot + addressing + EXT stubs
//! kept the VM in sync through Zork Zero's initialisation. The story is
//! gitignored (absent in CI) → the test SKIPs rather than fails when missing.

use std::path::PathBuf;

use zvm::cpu::exec::{Machine, StepResult};
use zvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

const MAX_STEPS: u64 = 50_000_000;

#[test]
fn zork0_v6_boots_to_first_prompt() {
    let path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };

    let mem = Memory::new(raw).expect("Zork0 is a valid v6 story");
    let mut m = Machine::new(mem);

    let mut steps = 0u64;
    loop {
        match m.step() {
            StepResult::NeedLine { .. } | StepResult::NeedChar => break, // reached input ✓
            StepResult::Continue => {
                steps += 1;
                assert!(steps < MAX_STEPS,
                    "runaway: Zork0 did not reach an input request within {MAX_STEPS} steps");
            }
            StepResult::Fault => {
                let t = m.take_fault_trace();
                panic!("Zork0 v6 faulted before first prompt: {t:?}");
            }
            StepResult::Quit => panic!("Zork0 quit before reaching input"),
            other => panic!("unexpected boot signal before first prompt: {other:?}"),
        }
    }

    eprintln!("Zork0 v6 booted to first input prompt in {steps} steps");
}
