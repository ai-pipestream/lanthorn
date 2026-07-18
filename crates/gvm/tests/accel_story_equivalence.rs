//! Full-story acceleration on/off equivalence + speed proof.
//!
//! These tests load real Inform 7 Glulx stories from the gitignored `stories/`
//! directory (a local symlink present in dev worktrees but absent in CI and
//! fresh clones). They are the primary anti-divergence guarantee: with
//! acceleration ON, the transcript to the first input prompt must be
//! byte-identical to acceleration OFF, and the opcode count must drop
//! substantially (accelerated calls bypass `step_once`, so their opcodes
//! vanish from `insn_count`).
//!
//! Because the story assets aren't committed, both tests here are `#[ignore]`d
//! so the default `cargo test -p gvm` tier stays green without them. Run
//! manually with:
//!
//! ```sh
//! cargo test -p gvm --test accel_story_equivalence -- --ignored --nocapture
//! ```
//!
//! (add `--release` if the debug build is too slow for CounterfeitMonkey).

use std::path::PathBuf;

use gvm::{Machine, Memory, StepResult, TestBackend};

/// The repo-root `stories/` directory (gitignored symlink), resolved relative
/// to this crate's manifest so the tests work regardless of `cargo test`'s
/// working directory.
fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Ceiling on opcode steps while driving to the first prompt. If a story
/// never reaches a `NeedLine`/`NeedChar` within this many steps, something is
/// wrong (infinite loop, VM bug, or a story that never asks for input) —
/// panic loudly rather than hang the test run.
const MAX_STEPS: u64 = 100_000_000;

/// Extract the Glulx executable from Blorb-wrapped `bytes` (or pass through
/// plain `.ulx` bytes unchanged), mirroring `gvm-cli`'s `extract_executable`.
fn extract_glulx(bytes: Vec<u8>) -> Vec<u8> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return bytes;
    }
    let b = blorb::Blorb::parse(bytes).expect("valid Blorb");
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => data.to_vec(),
        Ok((blorb::ExecKind::ZCode | blorb::ExecKind::Scott, _)) => {
            panic!("expected a Glulx Blorb")
        }
        Err(e) => panic!("Blorb has no executable: {e:?}"),
    }
}

/// Build a machine over `image`, set acceleration to `accel`, and drive it to
/// the first input prompt (`NeedLine` or `NeedChar`). Returns the full
/// text-buffer transcript captured to that point plus the opcode count.
fn run_to_first_prompt(image: Vec<u8>, accel: bool) -> (String, u64) {
    let mem = Memory::new(image).expect("valid Glulx image");
    let mut m = Machine::with_glk(mem, Box::new(TestBackend::new()));
    m.set_acceleration(accel);

    let mut steps = 0u64;
    loop {
        match m.step() {
            StepResult::Continue => {
                steps += 1;
                assert!(
                    steps < MAX_STEPS,
                    "runaway: did not reach the first input prompt within {MAX_STEPS} steps (accel={accel})"
                );
            }
            StepResult::NeedLine { .. } | StepResult::NeedChar { .. } => break,
            StepResult::NeedEvent { timer_ms: Some(_), .. } => m.deliver_timer(),
            StepResult::NeedEvent { .. } => {
                panic!("unexpected non-timer event wait before the first input prompt (accel={accel})")
            }
            StepResult::Quit => panic!("story quit before reaching an input prompt (accel={accel})"),
            StepResult::SaveRequest | StepResult::RestoreRequest | StepResult::NeedFilename { .. } => {
                panic!("unexpected @save/@restore before the first input prompt (accel={accel})")
            }
        }
    }

    let text = m
        .backend_mut()
        .as_any_mut()
        .downcast_mut::<TestBackend>()
        .unwrap()
        .all_text();
    (text, m.insn_count())
}

/// Load and run one on/off equivalence + speed comparison for the Blorb at
/// `path`, returning `(ops_on, ops_off)` after asserting transcript equality
/// and a material opcode reduction.
fn check_equivalence_and_speed(name: &str) -> (u64, u64) {
    let path = stories_dir().join(name);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for {name}");
    assert!(
        ops_on * 3 < ops_off,
        "accel not materially faster for {name}: on={ops_on} off={ops_off}"
    );

    (ops_on, ops_off)
}

/// The headline proof: CounterfeitMonkey (a large, real-world Inform 7 game)
/// produces an identical transcript to first prompt with acceleration on vs.
/// off, and acceleration cuts the dispatched-opcode count by more than 3x
/// (Task 0's baseline measured ~88.8% of interpreted opcodes inside
/// accel-candidate functions).
#[test]
#[ignore = "needs local gitignored stories/CounterfeitMonkey-11.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn counterfeit_monkey_accel_matches_interpreted_and_is_faster() {
    let (ops_on, ops_off) = check_equivalence_and_speed("CounterfeitMonkey-11.gblorb");
    eprintln!("CounterfeitMonkey-11: ops_on={ops_on} ops_off={ops_off} ratio={:.2}x", ops_off as f64 / ops_on as f64);
}

/// A smaller, faster secondary confirmation on another Inform Glulx title
/// present under `stories/` — the same on/off transcript equivalence check,
/// without the speed margin assertion (a tiny story may not do enough
/// accel-eligible work to clear the 3x bar, but transcript identity must
/// still hold).
#[test]
#[ignore = "needs local gitignored stories/TAKE.gblorb; run with `cargo test -p gvm --test accel_story_equivalence -- --ignored`"]
fn take_accel_matches_interpreted() {
    let path = stories_dir().join("TAKE.gblorb");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("local story (gitignored) missing at {}: {e}", path.display()));
    let image = extract_glulx(bytes);

    let (out_on, ops_on) = run_to_first_prompt(image.clone(), true);
    let (out_off, ops_off) = run_to_first_prompt(image, false);

    assert_eq!(out_on, out_off, "acceleration changed the transcript to first prompt for TAKE.gblorb");
    eprintln!("TAKE: ops_on={ops_on} ops_off={ops_off}");
}
