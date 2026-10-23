//! SQ-0681: a restore brings a game's LAYOUT WIDTH with it.
//!
//! The SQ-0679 floor keeps a v4/v5 story's one-shot status layout inside the
//! window it was computed for, and SQ-0680 keyed that floor to the width THIS
//! session booted at. A restore invalidates the premise of both: the memory
//! image now running was booted by some other session, at ITS width, and the
//! status routine's field columns answer to that width alone.
//!
//! Zork 1 (r52) is the reference case. Saved from an 80-column session it carries
//! field columns 60 and 73; restored into a 60-column session (a pane the
//! SQ-0680 pre-boot seed now reports honestly) the old floor stayed at 60, the
//! app declared 60, and every `set_cursor` to column 73 became illegal
//! (ZMSD §8.7.2.3). The interpreter drops an illegal move, so the digits printed
//! at the cursor's home — column 1, over the room name. The SQ-0679 garble,
//! re-manifested by a save file, on every turn.
//!
//! The floor is therefore raised on restore to the width the restored screen's
//! upper-window grid reports — the saved session's own frame of reference.
//!
//! **Perturb before asserting** (CLAUDE.md): the frame right after a restore
//! still looks correct, because nothing has repainted yet. Every case here
//! restores, then plays a REAL MOVE, and only then reads the screen back — which
//! is also what pins the stale-cell question: the saved row heals because the
//! game repaints it, not because the restore scrubbed it.
//!
//! **Colour mode**: both `honor_game_colours` modes, per the testing convention.
//!
//! Gitignored fixture: skips vacuously when the story is absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

const STORY: &str = "zork1-invclues-r52-s871125.z5";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Zork 1 with `host_screen` seeded BEFORE the first instruction (the
/// SQ-0680 pre-boot pane seed) and tap through to its first line prompt.
/// `None` when the gitignored fixture is absent.
fn boot(honor: bool, host_screen: (u16, u16)) -> Option<GameSession> {
    let path = stories_dir().join(STORY);
    let bytes = std::fs::read(&path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, std_window, None, Some(host_screen),
    )
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

/// An `AppState` with the terminal's own colours and the mode under test.
fn state_for(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    // SQ-0700 made the upper-window frame off by default. It is incidental to the
    // subject here — but `status_interior` reads the bar out of the box's rules,
    // and the frame's columns are part of the width the story is declared, so ask
    // for it and this suite measures exactly what it always did.
    state.colors.virtual_window_border = app::render::paneframe::BorderStyle::Single;
    state.colors.upper_window_border_sides =
        app::render::paneframe::PaneSides::all(app::render::paneframe::BorderStyle::Single);
    state.config.honor_game_colours = honor;
    state
}

/// Declare `pane_w` to an already-booted session exactly as
/// `loop_tick::poll_zvm_screen_dims` would — through
/// `declared_story_screen_dims`, whose floor is the session's own
/// `boot_screen_cols`.
fn declare_pane(s: &mut GameSession, honor: bool, pane_w: u16) {
    let state = state_for(honor);
    let version = s.machine.mem.version();
    let (rows, cols) = app::render::screen::declared_story_screen_dims(
        Rect::new(0, 0, pane_w, 25),
        &state,
        version,
        s.boot_screen_cols,
    )
    .expect("a non-empty pane reports dims");
    Engine::set_screen_dims(s, rows, cols);
}

/// The status row as the player sees it: the framed upper window's interior on
/// the row the bar sits on, as `(text, all_cells_reversed)`. (Same reading as
/// `zork1_status_line.rs` — the span is taken from the rules the box was
/// actually drawn with.)
fn status_interior(buf: &Buffer, area: Rect) -> (String, bool) {
    let row_at = |y: u16| -> String {
        (area.x..area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    let top = row_at(area.y);
    assert!(top.contains('┌') && top.contains('┐'), "the upper window is framed: {top:?}");
    let y = area.y + 1;
    let row = row_at(y);
    let cells: Vec<char> = row.chars().collect();
    let left = cells.iter().position(|&c| c == '│').expect("left rule") as u16;
    let right = cells.iter().rposition(|&c| c == '│').expect("right rule") as u16;
    assert!(right > left + 1, "the box has an interior: {row:?}");
    let xs: Vec<u16> = ((area.x + left + 1)..(area.x + right)).collect();
    let text: String = xs
        .iter()
        .map(|&x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
        .collect();
    let all_rev = xs
        .iter()
        .all(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED));
    (text, all_rev)
}

/// The game's OWN status row, straight out of the upper-window grid — the
/// terminal-neutral view. A pane narrower than the grid clips the render but
/// never the grid, so this is where a field column that landed where the game
/// meant it to (or at column 1) is visible either way.
fn grid_row(s: &GameSession) -> String {
    let u = &s.machine.screen.upper;
    (1..=u.cols).map(|c| u.cell(1, c).ch).collect()
}

/// Play one command at `pane_w` and render the story pane: declare the pane as
/// the poller would, run the command, draw. Returns the rendered bar interior.
fn play_and_render(s: &mut GameSession, honor: bool, pane_w: u16, cmd: &str) -> (String, bool) {
    declare_pane(s, honor, pane_w);
    let r = s.submit(cmd);
    assert!(r.fault.is_none() && !r.quit, "{cmd:?} faulted: {:?}", r.fault);
    let _ = s.take_transcript();

    let state = state_for(honor);
    let area = Rect::new(0, 0, pane_w, 25);
    let model = s.screen();
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
    status_interior(&buf, area)
}

/// Boot at `cols`, play a turn there, and hand back a host Save State captured
/// exactly as `/save-state` captures one: the engine save plus the screen the
/// archive carries beside it.
fn capture_save_at(honor: bool, cols: u16) -> Option<(app::engine::EngineSave, zvm::screen::ScreenState)> {
    let mut s = boot(honor, (24, cols))?;
    assert_eq!(s.boot_screen_cols, cols, "the capture session booted at its own pane");
    let _ = play_and_render(&mut s, honor, cols, "look");
    Some((Engine::save_state(&s), s.machine.screen.clone()))
}

/// The columns Zork 1's 80-column layout puts its two fields at, read off the
/// bar it paints at that width (rather than hardcoded twice).
const OPENING_BAR_80: &str =
    " West of House                                      Score: 0     Moves: 1";

/// THE BUG. An 80-column Save State restored into a narrower session: the
/// restored game keeps painting its 80-column bar, room name intact, fields at
/// the columns it baked in — and the pane simply clips the right of it, which is
/// what every interpreter shows for an 80-column game in a narrower window.
///
/// Two panes, because the corruption has two faces. At 60 the Score field (column
/// 52) is still legal and only Moves (column 65) falls outside, so the damage is
/// silent: the move count is dropped from the game's own screen and never comes
/// back, even if the terminal is widened later. At 50 BOTH field columns are out
/// of range, the cursor stays home, and "Score: 0" prints at column 1 straight
/// over the room name — the garble as reported.
///
/// Falsified by dropping the restore floor (`note_restored_screen_cols` made a
/// no-op): at 60 the grid comes back 60 wide and `Moves:` is gone; at 50 the
/// rendered bar reads " Score: 0…" where the room name belongs.
#[test]
fn restoring_an_80_column_save_into_a_narrower_session_keeps_the_bar_intact() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save_at(honor, 80) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        for pane_w in [50u16, 60] {
            let mut narrow = boot(honor, (24, pane_w)).expect("story present");
            assert_eq!(narrow.boot_screen_cols, pane_w, "this session booted at the narrow pane");
            Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
            app::session::restore_screen(&mut narrow, screen.clone());
            assert_eq!(
                narrow.boot_screen_cols, 80,
                "the declared-width floor follows the RESTORED game's frame of \
                 reference (honor={honor}, pane={pane_w})"
            );

            // Perturb: a real move, so the game repaints the whole bar — room
            // name region included — before anything is asserted.
            let (bar, all_rev) = play_and_render(&mut narrow, honor, pane_w, "north");

            assert!(
                bar.starts_with(" North of House"),
                "the room name leads the bar after the move \
                 (honor={honor}, pane={pane_w}): {bar:?}"
            );
            // The 80-column layout puts its leftmost field at column 52, so
            // nothing numeric belongs anywhere in the name region. A dropped
            // `set_cursor` leaves the digits at the cursor's home instead, which
            // is what " North of House0 2" — the reported garble — looks like.
            // (SQ-1809: the blank gap is dropped for display, so the fields sit
            // nearer the name than column 52 — the name itself is what is checked,
            // and the fields are asserted whole below.)
            let name_region = &bar[..bar.len().min(15)];
            assert!(
                !name_region.chars().any(|c| c.is_ascii_digit()),
                "no field digit may land on the room name — the SQ-0679/0681 garble \
                 (honor={honor}, pane={pane_w}): {bar:?}"
            );
            assert!(
                bar.contains("Score: 0") && bar.trim_end().ends_with("Moves: 2"),
                "both fields survive in the narrow pane (honor={honor}, pane={pane_w}): {bar:?}"
            );
            assert!(
                all_rev,
                "the reversed band still reaches both edges of the box interior \
                 (honor={honor}, pane={pane_w}); bar was {bar:?}"
            );

            // The game's own screen: the fields are at the columns the SAVED
            // session's 80-column layout put them at, which is only true if
            // every one of those cursor moves was legal.
            let row = grid_row(&narrow);
            assert_eq!(
                narrow.machine.screen.upper.cols, 80,
                "the restored grid keeps the restored game's width \
                 (honor={honor}, pane={pane_w})"
            );
            assert!(
                row.starts_with(" North of House"),
                "grid row (honor={honor}, pane={pane_w}): {row:?}"
            );
            assert_eq!(
                row.find("Score:"),
                OPENING_BAR_80.find("Score:"),
                "the Score field sits where the 80-column layout put it \
                 (honor={honor}, pane={pane_w}): {row:?}"
            );
            assert_eq!(
                row.find("Moves:"),
                OPENING_BAR_80.find("Moves:"),
                "the Moves field sits where the 80-column layout put it \
                 (honor={honor}, pane={pane_w}): {row:?}"
            );
            let moves_at = row.find("Moves:").expect("a Moves field");
            assert!(
                row[moves_at..].chars().any(|c| c.is_ascii_digit()),
                "the move count printed beside its own label \
                 (honor={honor}, pane={pane_w}): {row:?}"
            );
        }
    }
}

/// The reverse direction, which already worked and is pinned so it stays that
/// way: a 60-column save restored into an 80-column session. The floor must NOT
/// shrink to the save's 60 — this session's header says 80, and the restored
/// 60-column layout fits inside it — and the grid grows to the pane with the
/// band continued into the columns that appear (SQ-0679's widen half).
#[test]
fn restoring_a_60_column_save_into_an_80_column_session_still_works() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save_at(honor, 60) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };

        let mut wide = boot(honor, (24, 80)).expect("story present");
        Engine::restore_state(&mut wide, &save).expect("the Save State restores");
        app::session::restore_screen(&mut wide, screen);
        assert_eq!(
            wide.boot_screen_cols, 80,
            "a wider session keeps its own width — the floor only ever grows (honor={honor})"
        );

        let (bar, all_rev) = play_and_render(&mut wide, honor, 80, "north");

        assert!(
            bar.starts_with(" North of House"),
            "the room name leads the bar (honor={honor}): {bar:?}"
        );
        assert!(
            !bar[..15].chars().any(|c| c.is_ascii_digit()),
            "no field digit on the room name (honor={honor}): {bar:?}"
        );
        assert!(
            all_rev,
            "the reversed band spans the whole box interior, including the columns \
             the grid grew into (honor={honor}); bar was {bar:?}"
        );
        // The 60-column layout's fields are well inside an 80-column screen, so
        // they are VISIBLE here rather than clipped.
        assert!(
            bar[15..].chars().any(|c| c.is_ascii_digit()),
            "the score/moves fields are in view in the wider pane (honor={honor}): {bar:?}"
        );
    }
}

/// The stale-cell half, stated as an expectation rather than a scrub: a restore
/// installs the SAVED row verbatim (nothing repaints it at restore time), and it
/// heals on the game's next repaint. Zork 1 repaints the whole bar every turn,
/// so one move is enough — the row that came out of the archive says "West of
/// House" and the row after the move says "North of House".
///
/// This is why the fix belongs at the declared width and not in the renderer:
/// there is nothing to scrub, only a width to get right before the game paints
/// again.
#[test]
fn a_restored_status_row_heals_on_the_next_repaint() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save_at(honor, 80) else {
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

        let _ = play_and_render(&mut narrow, honor, 60, "north");
        let healed = grid_row(&narrow);
        assert!(
            healed.starts_with(" North of House"),
            "the game's own repaint rewrites the name region (honor={honor}): {healed:?}"
        );
        assert_eq!(
            healed.chars().count(),
            80,
            "and it is still the restored game's own 80-column row (honor={honor})"
        );
    }
}

// ---------------------------------------------------------------------------
// SQ-1778: the floor must survive the game's NEXT turns, not only the first.
//
// Zork 1 r52 reads header byte $21 every turn yet draws Score/Moves at the
// columns it computed once (52 and 65). SQ-1604 released the floor on that read,
// so after the first post-restore turn the poll narrowed the header, the
// out-of-range `set_cursor`s were dropped and the bar wrapped (at 60) or garbled
// the room name (at 40). Every case below therefore plays THREE turns, with the
// pane re-declared before each as the poller does.

/// The bar must be the saved 80-column layout, intact, after every turn.
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

const TURNS: [&str; 3] = ["north", "east", "south"];

#[test]
fn a_restored_save_state_keeps_the_bar_intact_for_several_turns() {
    for honor in [true, false] {
        let Some((save, screen)) = capture_save_at(honor, 80) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        for pane_w in [60u16, 40] {
            let mut narrow = boot(honor, (24, pane_w)).expect("story present");
            Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
            app::session::restore_screen(&mut narrow, screen.clone());
            for (i, cmd) in TURNS.iter().enumerate() {
                let (bar, _) = play_and_render(&mut narrow, honor, pane_w, cmd);
                let ctx = format!("save state, honor={honor}, pane={pane_w}, turn {} {cmd:?}, rendered {bar:?}", i + 1);
                assert_bar_intact(&narrow, &ctx);
            }
        }
    }
}

/// The measurement the whole rule rests on: Zork 1 r52 does NOT re-lay its bar
/// out when the header width drops mid-game. Lowered to 60, "Moves:" (column 65)
/// is dropped; lowered to 40, the digits land on the room name. (The fields are
/// computed from the width once, at boot.)
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

/// The same through the game's own Quetzal path (`.qzl`). Here r52 needs no
/// protection: restoring through its own RESTORE verb makes the game recompute
/// its field columns from the width it is now told (measured: the tail of the
/// restore already redraws a compact "0/1" bar at $21 = 60), so the bar is
/// intact at every width without any floor. What matters is that the shadow
/// check sees that, releases the guessed floor, and the pane width is what the
/// game is declared for the following turns. (Fails on neither side of
/// SQ-1778 for r52; it pins that the guess-80 floor does not clip the game.)
#[test]
fn a_restored_qzl_keeps_the_bar_intact_for_several_turns() {
    for honor in [true, false] {
        let Some(mut wide) = boot(honor, (24, 80)) else {
            eprintln!("SKIP: gitignored story missing");
            return;
        };
        let _ = play_and_render(&mut wide, honor, 80, "look");
        let r = wide.submit("save");
        assert_eq!(r.pending_io, Some(app::session::PendingIo::Save), "Zork 1 reaches @save on `save`");
        let blob = wide.machine.save_quetzal();
        for pane_w in [60u16, 40] {
            let mut narrow = boot(honor, (24, pane_w)).expect("story present");
            Engine::restore_game_save(&mut narrow, &blob).expect("the .qzl restores");
            app::engine_helpers::note_bare_quetzal_width(&mut narrow);
            assert_eq!(narrow.boot_screen_cols, 80, "a bare Quetzal guesses 80 until the story is asked");
            for (i, cmd) in TURNS.iter().enumerate() {
                let _ = play_and_render(&mut narrow, honor, pane_w, cmd);
                let row = grid_row(&narrow);
                let ctx = format!("qzl, honor={honor}, pane={pane_w}, turn {} {cmd:?}, row {row:?}", i + 1);
                let name_end = row.trim_end().len().min(30);
                assert!(
                    row.trim_start().chars().next().is_some_and(|c| c.is_alphabetic()),
                    "the room name leads the bar ({ctx})"
                );
                assert!(
                    !row[..name_end.min(20)].chars().any(|c| c.is_ascii_digit()),
                    "no field digit on the room name ({ctx})"
                );
                assert!(
                    row[20..].chars().any(|c| c.is_ascii_digit()),
                    "the score/moves digits are on the bar ({ctx})"
                );
                assert_eq!(narrow.machine.screen.upper.rows, 1, "one status row ({ctx})");
            }
            assert_eq!(
                narrow.boot_screen_cols, pane_w,
                "the shadow check released the guessed floor: r52 re-lays out on a Quetzal restore (pane={pane_w})"
            );
        }
    }
}

/// A Save State taken during an intro (title card waiting on a key) and restored
/// into a 60-column session: the saved width is kept through the intro, and the
/// width question is settled only once play starts (a line read with the status
/// line drawn). Curses and A Mind Forever Voyaging both wait on a key with text
/// already in the upper window and then reach a line prompt.
#[test]
fn a_save_state_taken_during_an_intro_holds_then_settles_once_play_starts() {
    for name in ["curses.z5", "amfv-r77-s850814.z4"] {
        let path = stories_dir().join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("SKIP: {name} missing");
            continue;
        };
        let new_at = |cols: u16| {
            GameSession::new_with_trace(bytes.clone(), true, false, None, false, vec![], None, None, Some((24, cols)))
                .expect("story boots")
        };
        let wide = new_at(80);
        assert_eq!(wide.pending_input(), InputKind::Char, "{name}: boots to a key wait (the intro)");
        let save = Engine::save_state(&wide);
        let screen = wide.machine.screen.clone();

        let mut narrow = new_at(60);
        Engine::restore_state(&mut narrow, &save).expect("the Save State restores");
        app::session::restore_screen(&mut narrow, screen);
        assert_eq!(narrow.boot_screen_cols, 80, "{name}: the saved width is kept on restore");
        assert_eq!(narrow.pending_input(), InputKind::Char, "{name}: restored into the intro");

        // Through the intro: still held, whatever the keys.
        let mut keys = 0;
        while narrow.pending_input() == InputKind::Char && keys < 10 {
            narrow.submit_char(32);
            keys += 1;
            assert_eq!(narrow.boot_screen_cols, 80, "{name}: the intro does not settle the floor");
        }
        assert_eq!(narrow.pending_input(), InputKind::Line, "{name}: play has started");
        assert_eq!(narrow.boot_screen_cols, 80, "{name}: nothing settles before the first command");

        // Play: the first line read with a drawn status line is the check.
        let r = narrow.submit("look");
        assert!(r.fault.is_none(), "{name}: faulted: {:?}", r.fault);
        let r = narrow.submit("look");
        assert!(r.fault.is_none(), "{name}: faulted: {:?}", r.fault);
        let settled = narrow.boot_screen_cols;
        // Measured: Curses re-lays its line out for the width it is told (released
        // to the pane), A Mind Forever Voyaging keeps the columns it baked (kept).
        let want = if name == "curses.z5" { 60 } else { 80 };
        assert_eq!(settled, want, "{name}: the verdict once play has started");
    }
}
