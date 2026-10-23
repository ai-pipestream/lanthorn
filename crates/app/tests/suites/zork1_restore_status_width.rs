//! SQ-0681, reworked for SQ-1809: a restore brings a game's LAYOUT WIDTH with it.
//!
//! A v4/v5 story that lays its status line out once (Zork 1 r52 is the reference
//! case: field columns 52 and 65 computed from the width it was booted with) has
//! those columns baked into the memory image a Save State carries. Restored into a
//! session that tells it a narrower width, every `set_cursor` to a baked column
//! becomes illegal (ZMSD §8.7.2.3), the interpreter drops it, and the digits print
//! at the cursor's home — column 1, over the room name. The SQ-0679 garble,
//! re-manifested by a save file, on every turn.
//!
//! SQ-1809 removes the cause rather than guarding it: a story found (before its
//! boot, by playing a throwaway copy) to cache the width is told 80 for the whole
//! session, so a save made by ANY such session and restored into ANY pane is
//! restored under the width it was laid out for. The one case that needs more is a
//! save made where the story was told MORE than 80 (an older build told it the
//! pane): the told width then becomes `max(80, the save's width)` for the session.
//!
//! **Perturb before asserting** (CLAUDE.md): the frame right after a restore still
//! looks correct, because nothing has repainted yet. Every case here restores, then
//! plays a REAL MOVE, and only then reads the screen back.
//!
//! **Colour mode**: both `honor_game_colours` modes, per the testing convention.
//!
//! Gitignored fixture: skips vacuously when the story is absent.

use std::path::PathBuf;

use app::config::StatusWidth;
use app::engine::Engine;
use app::graphics::PictSource;
use app::machine_boot::MachineBoot;
use app::session::{GameSession, InputKind};

const STORY: &str = "zork1-invclues-r52-s871125.z5";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork 1 through `new_for_machine` (so the width probe runs under `Auto`)
/// with `host_screen` seeded before the first instruction, and tap through to its
/// first line prompt. `None` when the gitignored fixture is absent.
fn boot_mode(honor: bool, host_screen: (u16, u16), mode: StatusWidth) -> Option<GameSession> {
    let path = stories_dir().join(STORY);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let boot = MachineBoot::bare().with_status_width(mode);
    let mut s = GameSession::new_for_machine(bytes, honor, false, false, dims, Some(host_screen), None, &boot)
        .expect("Zork 1 should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..10 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                s.submit_char(13);
            }
            InputKind::Event => {
                s.submit("");
            }
        }
        let _ = s.take_transcript();
    }
    Some(s)
}

fn boot(honor: bool, host_screen: (u16, u16)) -> Option<GameSession> {
    boot_mode(honor, host_screen, StatusWidth::Auto)
}

/// Declare `pane_w` to an already-booted session exactly as
/// `loop_tick::poll_zvm_screen_dims` would, then play `cmd`.
fn play(s: &mut GameSession, pane_w: u16, cmd: &str) {
    let state = app::state::AppState::default();
    app::host::screen::sync_zvm_screen_dims(s, &state, (pane_w, 25));
    let r = s.submit(cmd);
    assert!(r.fault.is_none() && !r.quit, "{cmd:?} faulted: {:?}", r.fault);
    let _ = s.take_transcript();
}

/// The game's OWN status row, straight out of the upper-window grid.
fn grid_row(s: &GameSession) -> String {
    let u = &s.machine.screen.upper;
    (1..=u.cols).map(|c| u.cell(1, c).ch).collect()
}

/// Boot at `cols` (told per `mode`), play a turn there, and hand back a host Save
/// State captured exactly as `/save-state` captures one: the engine save plus the
/// screen the archive carries beside it.
fn capture_save(honor: bool, cols: u16, mode: StatusWidth) -> Option<(app::engine::EngineSave, zvm::screen::ScreenState)> {
    let mut s = boot_mode(honor, (24, cols), mode)?;
    play(&mut s, cols, "look");
    Some((Engine::save_state(&s), s.machine.screen.clone()))
}

/// The columns Zork 1's 80-column layout puts its two fields at.
const OPENING_BAR_80: &str =
    " West of House                                      Score: 0     Moves: 1";

/// The bar must be the 80-column layout, intact: room name leading, the fields at
/// the columns the 80-wide layout put them at, one row.
fn assert_bar_intact(s: &GameSession, ctx: &str) {
    let row = grid_row(s);
    let name_region = &row[..row.len().min(40)];
    assert!(
        !name_region.chars().any(|c| c.is_ascii_digit()),
        "no field digit on the room name ({ctx}): {row:?}"
    );
    assert!(
        row.starts_with(' ') && row.trim_start().chars().next().is_some_and(|c| c.is_alphabetic()),
        "the room name leads the bar ({ctx}): {row:?}"
    );
    assert_eq!(row.find("Score:"), OPENING_BAR_80.find("Score:"), "Score column ({ctx}): {row:?}");
    assert_eq!(row.find("Moves:"), OPENING_BAR_80.find("Moves:"), "Moves column ({ctx}): {row:?}");
    let moves_at = row.find("Moves:").unwrap();
    assert!(row[moves_at..].chars().any(|c| c.is_ascii_digit()), "move count printed ({ctx}): {row:?}");
    assert_eq!(s.machine.screen.upper.rows, 1, "the status line is one row ({ctx})");
}

/// THE BUG, in the new model. A Save State restored into a session at ANY pane
/// width — narrower or wider than the 80 columns the story was laid out for —
/// keeps painting its intact bar after a move: the restored story is still told 80.
///
/// Falsified by letting the pane through (`status_width = "pane"` on the
/// restoring session): at 50 columns both field columns are out of range, the
/// cursor stays home, and "Score: 0" prints at column 1 straight over the room
/// name — the garble as reported.
#[test]
fn a_save_state_restored_at_any_pane_width_keeps_the_bar_intact() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save(honor, 80, StatusWidth::Auto) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        for pane_w in [50u16, 60, 120] {
            let mut other = boot(honor, (24, pane_w)).expect("story present");
            assert_eq!(other.fixed_cols(), Some(80), "booted at {pane_w}, it is still told 80");
            Engine::restore_state(&mut other, &save).expect("the Save State restores");
            app::session::restore_screen(&mut other, screen.clone());
            assert_eq!(other.fixed_cols(), Some(80), "an 80-wide save leaves the told width at 80");
            assert_eq!(other.machine.mem.read_byte(0x21), 80, "and $21 says so (pane={pane_w})");

            // Perturb: real moves, so the game repaints the whole bar.
            for (i, cmd) in ["north", "east", "south"].iter().enumerate() {
                play(&mut other, pane_w, cmd);
                assert_bar_intact(&other, &format!("honor={honor}, pane={pane_w}, turn {} {cmd:?}", i + 1));
            }
            assert_eq!(other.machine.screen.upper.cols, 80, "the grid stays the game's 80 columns");
        }

        // The garble itself, for the falsification claim above: the same restore
        // into a session that was told the pane.
        let mut told_pane = boot_mode(honor, (24, 50), StatusWidth::Pane).expect("story present");
        Engine::restore_state(&mut told_pane, &save).expect("the Save State restores");
        app::session::restore_screen(&mut told_pane, screen.clone());
        play(&mut told_pane, 50, "north");
        let row = grid_row(&told_pane);
        assert!(
            !row.contains("Moves:") || row.find("Score:") != OPENING_BAR_80.find("Score:"),
            "a story told the pane under a baked layout is the SQ-0679 garble (honor={honor}): {row:?}"
        );
    }
}

/// A save from a session that told the story MORE than 80 columns (an older build
/// told a caching story the pane): the restoring session raises the told width to
/// the save's, `max(80, saved)`, so the baked columns stay inside the window.
#[test]
fn a_save_made_wider_than_80_raises_the_told_width() {
    for honor in [true, false] {
        // `Pane` stands for the older build: the story is told the 100-column pane.
        let Some((save, screen)) = capture_save(honor, 100, StatusWidth::Pane) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let saved = screen.upper.cols;
        assert!(saved > 80, "the capture really was laid out wider than 80, at {saved}");

        let mut other = boot(honor, (24, 45)).expect("story present");
        assert_eq!(other.fixed_cols(), Some(80));
        Engine::restore_state(&mut other, &save).expect("the Save State restores");
        app::session::restore_screen(&mut other, screen);
        assert_eq!(other.fixed_cols(), Some(saved), "told max(80, {saved}) for the rest of the session");
        assert_eq!(other.machine.mem.read_byte(0x21) as u16, saved, "and $21 says so");

        for (i, cmd) in ["north", "east", "south"].iter().enumerate() {
            play(&mut other, 45, cmd);
            let row = grid_row(&other);
            let ctx = format!("honor={honor}, turn {} {cmd:?}: {row:?}", i + 1);
            assert!(!row[..14].chars().any(|c| c.is_ascii_digit()), "no digit on the room name ({ctx})");
            assert!(row.contains("Score:") && row.contains("Moves:"), "both fields survive ({ctx})");
            assert_eq!(other.machine.screen.upper.rows, 1, "one status row ({ctx})");
            assert_eq!(other.machine.mem.read_byte(0x21) as u16, saved, "the wide width is held ({ctx})");
        }
    }
}

/// The stale-cell half, stated as an expectation rather than a scrub: a restore
/// installs the SAVED row verbatim (nothing repaints it at restore time), and it
/// heals on the game's next repaint. Zork 1 repaints the whole bar every turn, so
/// one move is enough — the row that came out of the archive says "West of House"
/// and the row after the move says "North of House".
#[test]
fn a_restored_status_row_heals_on_the_next_repaint() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save(honor, 80, StatusWidth::Auto) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };

        let mut narrow = boot(honor, (24, 60)).expect("story present");
        Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
        app::session::restore_screen(&mut narrow, screen);

        let restored = grid_row(&narrow);
        assert!(
            restored.starts_with(" West of House"),
            "the restore hands back the archived row as it was saved (honor={honor}): {restored:?}"
        );

        play(&mut narrow, 60, "north");
        let healed = grid_row(&narrow);
        assert!(
            healed.starts_with(" North of House"),
            "the game's own repaint rewrites the name region (honor={honor}): {healed:?}"
        );
        assert_eq!(healed.chars().count(), 80, "and it is still the game's own 80-column row (honor={honor})");
    }
}

/// The measurement the whole rule rests on: Zork 1 r52 does NOT re-lay its bar
/// out when the header width drops mid-game. Lowered to 60, "Moves:" (column 65)
/// is dropped; lowered to 40, the digits land on the room name. (The fields are
/// computed from the width once, at boot.) This is what makes "tell it 80 and
/// leave it" necessary rather than a preference.
#[test]
fn r52_keeps_its_boot_columns_when_the_header_width_is_lowered_mid_game() {
    let Some(mut s) = boot(true, (24, 80)) else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let _ = s.submit("look");
    Engine::set_screen_dims(&mut s, 24, 60);
    let _ = s.submit("north");
    let row = grid_row(&s);
    assert_eq!(row.find("Score:"), OPENING_BAR_80.find("Score:"), "Score stays at its boot column: {row:?}");
    assert!(!row.contains("Moves:"), "Moves (column 65) is past a 60-wide header and is dropped: {row:?}");
    Engine::set_screen_dims(&mut s, 24, 40);
    let _ = s.submit("south");
    let row = grid_row(&s);
    assert!(row[..20].chars().any(|c| c.is_ascii_digit()), "at 40 the digits land on the room name: {row:?}");
}

/// The same through the game's own Quetzal path (`.qzl`): a bare Quetzal carries
/// no screen, restored into a session at any pane the story is still told 80 and
/// the bar is intact for several turns.
#[test]
fn a_restored_qzl_keeps_the_bar_intact_for_several_turns() {
    for honor in [true, false] {
        let Some(mut wide) = boot(honor, (24, 80)) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        play(&mut wide, 80, "look");
        let r = wide.submit("save");
        assert_eq!(r.pending_io, Some(app::session::PendingIo::Save), "Zork 1 reaches @save on `save`");
        let blob = wide.machine.save_quetzal();
        for pane_w in [60u16, 40] {
            let mut narrow = boot(honor, (24, pane_w)).expect("story present");
            Engine::restore_game_save(&mut narrow, &blob).expect("the .qzl restores");
            assert_eq!(narrow.fixed_cols(), Some(80), "a bare Quetzal changes nothing: still told 80");
            for (i, cmd) in ["north", "east", "south"].iter().enumerate() {
                play(&mut narrow, pane_w, cmd);
                let row = grid_row(&narrow);
                let ctx = format!("qzl, honor={honor}, pane={pane_w}, turn {} {cmd:?}, row {row:?}", i + 1);
                assert!(
                    row.trim_start().chars().next().is_some_and(|c| c.is_alphabetic()),
                    "the room name leads the bar ({ctx})"
                );
                assert!(!row[..20].chars().any(|c| c.is_ascii_digit()), "no field digit on the room name ({ctx})");
                assert!(row[20..].chars().any(|c| c.is_ascii_digit()), "the score/moves digits are on the bar ({ctx})");
                assert_eq!(narrow.machine.screen.upper.rows, 1, "one status row ({ctx})");
            }
        }
    }
}

/// A Save State taken during an intro (title card waiting on a key) and restored
/// into a 60-column session. The verdict was settled at THAT session's boot, long
/// before the restore: Curses re-reads the width and follows the pane; A Mind
/// Forever Voyaging caches it and is told 80. Neither verdict moves through the
/// intro or play.
#[test]
fn a_save_state_taken_during_an_intro_keeps_the_boot_verdict() {
    for (name, fixed) in [("curses.z5", None), ("amfv-r77-s850814.z4", Some(80u16))] {
        let path = stories_dir().join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: {name} missing");
            continue;
        };
        let new_at = |cols: u16| {
            GameSession::new_for_machine(
                bytes.clone(), true, false, false, vec![], Some((24, cols)), None, &MachineBoot::bare(),
            )
            .expect("story boots")
        };
        let wide = new_at(80);
        assert_eq!(wide.pending_input(), InputKind::Char, "{name}: boots to a key wait (the intro)");
        let save = Engine::save_state(&wide);
        let screen = wide.machine.screen.clone();

        let mut narrow = new_at(60);
        assert_eq!(narrow.fixed_cols(), fixed, "{name}: the verdict at boot");
        Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
        app::session::restore_screen(&mut narrow, screen);
        assert_eq!(narrow.fixed_cols(), fixed, "{name}: a restore does not move the verdict");
        assert_eq!(narrow.pending_input(), InputKind::Char, "{name}: restored into the intro");

        let mut keys = 0;
        while narrow.pending_input() == InputKind::Char && keys < 10 {
            narrow.submit_char(32);
            keys += 1;
        }
        assert_eq!(narrow.pending_input(), InputKind::Line, "{name}: play has started");
        for _ in 0..2 {
            play(&mut narrow, 60, "look");
        }
        assert_eq!(narrow.fixed_cols(), fixed, "{name}: nor does play");
        let pane = app::state::AppState::default();
        let pane_cols = app::render::screen::story_screen_dims(ratatui::layout::Rect::new(0, 0, 60, 25), &pane)
            .expect("a pane")
            .1;
        let want = fixed.unwrap_or(pane_cols);
        assert_eq!(narrow.machine.mem.read_byte(0x21) as u16, want, "{name}: and $21 is what it was decided to be");
    }
}
