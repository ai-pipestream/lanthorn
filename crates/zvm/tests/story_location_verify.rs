// MAP#3 verification — v4+ location detection (commit 4adf7f9).
//
// Drives real story files (gitignored, kept locally under `stories/` at the
// repo root — see TODO.md) through boot and a few turns, then checks what
// `zvm::location::detect_location` reports. All tests skip gracefully when
// the `stories/` directory or an individual story file is absent (e.g. CI),
// so this is best-effort local verification, not a required CI gate.
//
// Findings (2026-07-02, see docs/TODO.md MAP#3):
//   - v3 games are unaffected: they still resolve via GlobalVar0 (unchanged
//     path). Regression check below.
//   - Commit 4adf7f9 fixed detection for v4+ games whose status line is
//     LEFT-JUSTIFIED (the common Infocom form: "Room Name    Score: n  Moves: n")
//     or uses a "Location:" label. Several such games now resolve.
//   - BeyondZork's status line is NOT left-justified: it CENTERS the room
//     name in row 1 and puts stats in row 2 ("EN:16  ST:08 ..."), which
//     `status_line_room_name`'s common-form heuristic (text before the first
//     run of 2+ spaces) reduces to an empty string, so `detect_location`
//     still returns `None` for BeyondZork. TODO MAP#3's BeyondZork case
//     remains OPEN — see the dedicated test below.

use std::path::PathBuf;
use zvm::cpu::exec::{Machine, StepResult};
use zvm::location::{detect_location, LocationMethod};
use zvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn load_story(name: &str) -> Option<Vec<u8>> {
    let path = stories_dir().join(name);
    if !path.exists() {
        return None;
    }
    std::fs::read(&path).ok()
}

/// Boot a story and run until the first line-read prompt (or a step cap),
/// answering any char-reads with '\n' and refusing save/restore along the
/// way. Returns the machine positioned at that first prompt.
fn boot_to_first_read(data: Vec<u8>) -> Option<Machine> {
    let mem = Memory::new(data).ok()?;
    let mut machine = Machine::new(mem);
    machine.init_caps();
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } => return Some(machine),
            StepResult::Quit | StepResult::Restart => return Some(machine),
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
    None
}

/// Run `machine` for one more turn by supplying `input` as a line, stepping
/// until the next prompt (or a step cap). Char-reads get '\n'.
fn run_one_turn(machine: &mut Machine, input: &str) {
    machine.supply_line(input, 13);
    for _ in 0..2_000_000u64 {
        match machine.step() {
            StepResult::NeedLine { .. } | StepResult::Quit | StepResult::Restart => return,
            StepResult::Continue => {}
            StepResult::NeedChar => machine.supply_char(b'\n'),
            StepResult::SaveRequest => machine.complete_save(false),
            StepResult::RestoreRequest => machine.complete_restore_failure(),
        }
    }
}

// ── v3 regression: unaffected by the v4+ fix ────────────────────────────────

#[test]
fn v3_games_still_resolve_via_global_var0() {
    if !stories_dir().exists() {
        return; // stories/ absent (fresh checkout / CI) — skip.
    }
    // A representative spread of v3 titles; each entry skips individually if
    // its file is absent so this test degrades gracefully on any machine.
    let v3_games = [
        ("zork1-r88-s840726.z3", "West of House"),
        ("deadline-r27-s831005.z3", "South Lawn"),
        ("enchanter-r29-s860820.z3", "Fork"),
        ("planetfall-r37-s851003.z3", "Deck Nine"),
        ("wishbringer-r69-s850920.z3", "Hilltop"),
    ];
    let mut checked = 0;
    for (file, expected_prefix) in v3_games {
        let Some(story) = load_story(file) else { continue };
        let Some(machine) = boot_to_first_read(story) else {
            panic!("{file}: never reached a line-read prompt");
        };
        let loc = detect_location(&machine)
            .unwrap_or_else(|| panic!("{file}: expected a v3 GlobalVar0 location, got None"));
        assert_eq!(loc.method(), LocationMethod::GlobalVar0, "{file}: v3 must use GlobalVar0, not the v4+ path");
        let name = loc.object().expect("GlobalVar0 always carries an object").name.clone();
        assert!(
            name.starts_with(expected_prefix),
            "{file}: expected room starting with {expected_prefix:?}, got {name:?}"
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no v3 fixture files present under {:?}", stories_dir());
    }
}

// ── v4+ games the 4adf7f9 fix newly resolves (left-justified status lines) ──

#[test]
fn v4plus_left_justified_status_lines_now_resolve() {
    if !stories_dir().exists() {
        return;
    }
    let v4plus_games = [
        "zork1-invclues-r52-s871125.z5",
        "wishbringer-invclues-r23-s880706.z5",
        "sherlock-r26-s880127.z5",
        "LostPig.z8",
        "hitchhiker-invclues-r31-s871119.z5",
        "planetfall-invclues-r10-s880531.z5",
        "leathergoddesses-invclues-r4-s880405.z5",
        "amfv-r77-s850814.z4",
    ];
    let mut checked = 0;
    for file in v4plus_games {
        let Some(story) = load_story(file) else { continue };
        let Some(machine) = boot_to_first_read(story) else {
            panic!("{file}: never reached a line-read prompt");
        };
        let loc = detect_location(&machine);
        assert!(loc.is_some(), "{file}: expected a v4+ location (PlayerParent/StatusName), got None — regression?");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no v4+ fixture files present under {:?}", stories_dir());
    }
}

// ── BeyondZork: TODO MAP#3 residual gap — still open ────────────────────────

#[test]
fn beyondzork_centered_status_line_still_undetected() {
    // Documents the still-open half of TODO MAP#3. Drives past the VT220
    // prompt / title crawl / character-creation menus (accepting the default
    // character via '\n' at each char-read) into the first real room, then
    // asserts the room banner IS present in the upper window but
    // `detect_location` still can't parse it — because BeyondZork centers
    // the room name in row 1 (padded with leading spaces) instead of
    // left-justifying it, which defeats `status_line_room_name`'s
    // common-form heuristic (text before the first run of 2+ spaces reduces
    // to an empty string for a centered title).
    //
    // If this test starts failing because detect_location now returns
    // Some(_), status_line_room_name has gained centered-title support:
    // update this test and close out the remainder of TODO MAP#3.
    let Some(story) = load_story("beyondzork-r57-s871221.z5") else {
        return; // fixture absent — skip.
    };
    let Some(mut machine) = boot_to_first_read(story) else {
        panic!("beyondzork: never reached a line-read prompt");
    };
    // "Is this a VT220?" -> no; "begin a new game..." -> begin; character
    // creation is an arrow-key menu, accept defaults via NeedChar '\n'.
    for input in ["no", "begin"] {
        run_one_turn(&mut machine, input);
    }
    // A few more '\n'-only turns to walk through the character-creation
    // menu screens (race/class/sex/name confirmation) and the "press any
    // key to begin the story" gate.
    for _ in 0..4 {
        run_one_turn(&mut machine, "");
    }

    let upper = &machine.screen.upper;
    let row1: String = (1..=upper.cols).map(|c| upper.cell(1, c).ch).collect();
    assert!(
        row1.contains("Hilltop"),
        "expected to have reached the Hilltop starting room by now, upper row1={row1:?}"
    );

    let loc = detect_location(&machine);
    assert!(
        loc.is_none(),
        "BeyondZork now resolves a location ({loc:?}) — the centered-status-line gap \
         appears fixed; update this test (assert Some) and close the residual half of TODO MAP#3"
    );
}
