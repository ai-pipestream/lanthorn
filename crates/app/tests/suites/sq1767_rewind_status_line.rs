//! SQ-1767: rewinding to an earlier turn puts that turn's status line back.
//!
//! Quetzal carries no screen, `restore_state` blanks the upper window, and a v4+
//! story parked at its read never repaints its status line, so a rewind used to
//! leave the bar empty until the next command. Each `TurnRecord` now keeps the
//! `ScreenState` and `resume_from_turn` installs it as a loaded save does.
//!
//! Tangle (v5) and Mini-Zork (v3), both manifest fixtures so this runs on CI.

use app::engine_helpers::zvm_session_opt;
use app::host::persist::resume_from_turn;
use app::host::{boot_story, finish_command_turn, BootRequest, BootedStory, LaunchFlags, QuietBoot, TerminalFacts};
use app::launch_options::LaunchOverrides;
use app::session::InputKind;

use crate::fixture_paths::fixture_path;

fn boot(name: &str, home: &std::path::Path, honor: bool) -> BootedStory {
    let overrides = LaunchOverrides::default();
    let req = BootRequest {
        story_path: fixture_path(name),
        disk_entry: None,
        overrides: &overrides,
        cfg: app::config::Config {
            user_dir: home.to_path_buf(),
            config_file: home.join("config.toml"),
            random_seed: Some(1),
            enable_sound: false,
            auto_save: false,
            record_turn_history: true,
            honor_game_colours: honor,
            ..app::config::Config::default()
        },
        roots: app::data_roots::DataRoots::single(home.join("saves")),
        flags: LaunchFlags::default(),
        terminal: TerminalFacts::default(),
        fresh_start: true,
    };
    boot_story(req, &mut QuietBoot).expect("story boots headlessly")
}

fn play(b: &mut BootedStory, cmd: &str) {
    assert!(matches!(b.session.pending_input(), InputKind::Line), "premise: a line is awaited for `{cmd}`");
    let result = b.session.submit(cmd);
    let mut tidy = 0u32;
    let _ = finish_command_turn(cmd, true, result, &mut b.state, &mut b.mapper, &mut *b.session, &b.game_dir, &b.ifid, &b.arc_file, None, &mut tidy);
}

/// The upper window's text, rows joined by `|`, trimmed.
fn upper(b: &BootedStory) -> String {
    let z = zvm_session_opt(&*b.session).expect("a Z-machine session");
    let u = &z.machine.screen.upper;
    let cols = u.cols as usize;
    if cols == 0 {
        return String::new();
    }
    u.cells
        .chunks(cols)
        .map(|r| r.iter().map(|c| c.ch).collect::<String>().trim().to_string())
        .collect::<Vec<_>>()
        .join("|")
}

fn rewind(b: &mut BootedStory, idx: usize) {
    assert!(resume_from_turn(&mut *b.session, &mut b.mapper, &mut b.state, idx, None).ok, "the rewind succeeds");
}

/// Play `cmds`, rewind to the first record, and check the bar is the one the
/// player saw at that prompt, right away and after a further move.
fn check(name: &str, cmds: &[&str], then: &str) {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1767");
        let mut b = boot(name, &home, honor);
        let mut seen = vec![];
        for c in cmds {
            play(&mut b, c);
            seen.push(upper(&b));
        }
        assert_eq!(b.state.history.len(), cmds.len(), "premise: one record per command");
        assert!(seen.iter().all(|s| !s.is_empty()), "premise: the bar is drawn at every prompt: {seen:?}");
        assert!(seen.first() != seen.last(), "premise: the bar changed across the turns: {seen:?}");
        // The later bar is on screen now, so a blank or unchanged result is
        // distinguishable from the right one.
        rewind(&mut b, 0);
        assert_eq!(upper(&b), seen[0], "bar right after the rewind (honor={honor})");
        // Perturb, then assert again.
        play(&mut b, then);
        assert!(!upper(&b).is_empty(), "bar after the next move (honor={honor})");
    }
}

#[test]
fn v5_rewind_shows_that_turns_status_line() {
    check("Tangle.z5", &["look", "n", "s", "e"], "look");
}

/// A v3 story has no upper window (the host builds its status from globals), so
/// the grid stays empty throughout; what must hold is that the rewind still lands
/// on the earlier room and the game goes on from there.
#[test]
fn v3_rewind_is_unaffected() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1767");
        let mut b = boot("minizork-r34-s871124.z3", &home, honor);
        let mut rooms = vec![];
        for c in ["north", "east", "open window", "west"] {
            play(&mut b, c);
            assert_eq!(upper(&b), "", "premise: a v3 story draws no upper window");
            rooms.push(b.session.current_location().map(|l| l.number));
        }
        assert!(rooms[0] != rooms[3], "premise: the player moved between turn 1 and 4: {rooms:?}");
        rewind(&mut b, 0);
        assert_eq!(b.session.current_location().map(|l| l.number), rooms[0], "back in turn 1's room (honor={honor})");
        assert_eq!(upper(&b), "");
        play(&mut b, "look");
        assert_eq!(b.session.current_location().map(|l| l.number), rooms[0]);
    }
}

/// Every Glk grid window's text, `id:rows` joined, trimmed.
fn grids(b: &BootedStory) -> String {
    fn walk(n: &app::engine::WinNode, out: &mut Vec<String>) {
        match n {
            app::engine::WinNode::Grid(g) => {
                let cols = g.cols.max(1) as usize;
                let rows: Vec<String> = g
                    .cells
                    .chunks(cols)
                    .map(|r| r.iter().map(|c| c.ch).collect::<String>().trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                out.push(format!("{}:{}", g.win, rows.join("|")));
            }
            app::engine::WinNode::Pair { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    walk(&b.session.screen().root, &mut v);
    v.join(" ## ")
}


/// Glulx keeps its status grid across a rewind through the snapshot's own Glk
/// contents chunk (SQ-1712), so it needed no fix here; this pins that it stays so.
#[test]
fn glulx_rewind_shows_that_turns_status_grid() {
    for honor in [true, false] {
        let home = app::scratch_dir("sq1767");
        let mut b = boot("AnchorheadDemo.gblorb", &home, honor);
        // The intro pauses on a key.
        while matches!(b.session.pending_input(), InputKind::Char) {
            let r = b.session.submit_key(app::engine::KeyInput::Char(' ')).expect("key");
            let _ = app::host::turn::apply_game_driven_result(
                &mut b.state, &mut b.mapper, &r, &b.game_dir, None, &*b.session, app::pager::Driver::PlayerInput,
            );
        }
        let mut seen = vec![];
        for c in ["w", "in"] {
            play(&mut b, c);
            seen.push(grids(&b));
        }
        assert!(seen[0].contains("Narrow Street") && seen[1].contains("Local Pub"), "premise: {seen:?}");
        rewind(&mut b, 0);
        assert_eq!(grids(&b), seen[0], "status grid right after the rewind (honor={honor})");
        play(&mut b, "in");
        assert_eq!(grids(&b), seen[1], "status grid after the next move (honor={honor})");
    }
}
