//! End-to-end oracle test for the `scott` crate: loads the "Tiny Cave" fixture,
//! drives a fixed command script through the turn loop, and freezes the exact
//! transcript. A second test exercises `Vm::snapshot`/`Vm::restore`.

use scott::{Database, Vm};

const TINY_CAVE: &str = include_str!("tiny_cave.dat");

fn run_script(vm: &mut Vm, cmds: &[&str]) -> String {
    let mut transcript = String::new();
    transcript.push_str(&vm.take_output()); // intro (room 1 description)
    for cmd in cmds {
        vm.supply_line(cmd);
        vm.step();
        transcript.push_str(&format!("> {cmd}\n"));
        transcript.push_str(&vm.take_output());
    }
    transcript
}

#[test]
fn golden_transcript() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    vm.seed_rng(1); // deterministic, though this fixture uses no percentage occurrences

    let script = [
        "down",       // scripted GO-down (room1->room2), no lamp yet
        "up",         // occurrence sets DARK_FLAG; unlit -> "dangerous to move" warning
        "take lamp",  // verb synonym TAKE -> GET(10); auto-noun picks up the lamp
        "down",       // scripted GO-down again, now carrying the lamp
        "rub lamp",   // scripted action: code-0 params feed opcode 62 (item->room) + 58 (flag)
        "get idol",   // auto-noun GET picks up the now-revealed idol
        "up",         // lamp carried now -> no darkness warning this time
        "down",       // scripted GO-down (room1->room2)
        "down",       // unmatched by any scripted action -> built-in GO (room2->room3)
        "score",      // idol still carried, not deposited -> fallback score action (0/1)
        "drop idol",  // auto-noun DROP deposits the idol in the treasure room
        "score",      // idol now in treasure room -> win action fires, quits
    ];

    let transcript = run_script(&mut vm, &script);
    let expected = "\
You are in a sunlit forest clearing. A narrow path leads down into darkness.\n\
You can see: a brass lamp\n\
> down\n\
You descend into darkness, feeling your way along the damp rock wall.\n\
> up\n\
You hear water dripping somewhere in the darkness.\n\
It is dangerous to move in the dark!\n\
You are in a sunlit forest clearing. A narrow path leads down into darkness.\n\
You can see: a brass lamp\n\
> take lamp\n\
OK.\n\
> down\n\
You descend into darkness, feeling your way along the damp rock wall.\n\
> rub lamp\n\
You hear water dripping somewhere in the darkness.\n\
The lamp's glow reveals a niche in the rock - and within it, a gleaming gold idol!\n\
> get idol\n\
You hear water dripping somewhere in the darkness.\n\
OK.\n\
> up\n\
You hear water dripping somewhere in the darkness.\n\
You are in a sunlit forest clearing. A narrow path leads down into darkness.\n\
> down\n\
You descend into darkness, feeling your way along the damp rock wall.\n\
> down\n\
You hear water dripping somewhere in the darkness.\n\
You are in a crystal grotto glittering with reflected light.\n\
> score\n\
You have 0 out of 1 treasures.\n\
> drop idol\n\
OK.\n\
> score\n\
You set the idol down. *** You have won! ***\n";
    assert_eq!(transcript, expected);
    assert!(vm.has_quit(), "win action should have executed opcode 63 (quit)");
}

#[test]
fn snapshot_restore_round_trip() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    vm.take_output();

    // Reach room2 carrying the lamp, with flag3 set (RUB LAMP guard) and the
    // counter at 7 (via the fixture's COUNT verb), then snapshot. The lamp
    // lives in room1, so fetch it before descending again.
    for cmd in ["down", "up", "take lamp", "down", "rub lamp", "count"] {
        vm.supply_line(cmd);
        vm.step();
        vm.take_output();
    }
    assert_eq!(vm.current_room(), 2);
    assert_eq!(vm.item_loc(9), scott::CARRIED); // lamp carried
    assert!(vm.flag(3)); // RUB LAMP guard flag set
    assert_eq!(vm.counter(), 7);

    let snap = vm.snapshot();

    // Mutate: move away, drop the lamp.
    vm.supply_line("up");
    vm.step();
    vm.take_output();
    vm.supply_line("drop lamp");
    vm.step();
    vm.take_output();

    assert_ne!(vm.current_room(), 2);
    assert_ne!(vm.item_loc(9), scott::CARRIED);

    vm.restore(&snap).expect("restore succeeds");

    assert_eq!(vm.current_room(), 2);
    assert_eq!(vm.item_loc(9), scott::CARRIED);
    assert!(vm.flag(3));
    assert_eq!(vm.counter(), 7);
}

#[test]
fn restore_rejects_malformed_input() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let mut vm = Vm::new(db);
    assert!(vm.restore(&[]).is_err());
    assert!(vm.restore(&[1, 2, 3]).is_err());

    let mut snap = vm.snapshot();
    snap.truncate(snap.len() - 1);
    assert!(vm.restore(&snap).is_err()); // truncated

    let mut bad_count = vm.snapshot();
    bad_count[0] = 0xFF; // corrupt the item_loc length prefix
    assert!(vm.restore(&bad_count).is_err());
}
