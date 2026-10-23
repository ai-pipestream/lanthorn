//! SQ-1809: which width a v4+ Z-machine story is TOLD.
//!
//! A story that lays its status line out once (older Infocom) caches whatever
//! width it is given at boot, so it is told 80 for the whole session and never
//! sees a pane change; a story that re-reads header byte $21 (Inform's library)
//! is told the pane. The verdict is reached before the real boot by playing a
//! throwaway copy (`zvm::width_probe`), through the one door every launch and
//! every `@restart` reaches, `GameSession::new_for_machine`.
//!
//! Real-story cases skip vacuously when the gitignored story is absent. The
//! fetched fixtures (`LostPig.z8`, `anchor.z8`, `Tangle.z5`) are Inform games, so
//! CI covers the ADAPTS verdict on them; the manifest carries no Infocom v4+
//! title, so the CACHES verdict is pinned CI-side by `zvm`'s own synthetic-story
//! tests (`zvm::width_probe::tests`) and here by the local corpus only.

use app::config::StatusWidth;
use app::machine_boot::MachineBoot;
use app::session::GameSession;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

fn boot_mode(name: &str, pane_cols: u16, mode: StatusWidth) -> Option<GameSession> {
    let path = fixture_path(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return None;
    };
    let boot = MachineBoot::bare().with_status_width(mode);
    Some(
        GameSession::new_for_machine(bytes, true, false, false, Vec::new(), Some((25, pane_cols)), None, &boot)
            .unwrap_or_else(|e| panic!("{name} boots: {e:?}")),
    )
}

fn boot(name: &str, pane_cols: u16) -> Option<GameSession> {
    boot_mode(name, pane_cols, StatusWidth::Auto)
}

/// Stories that re-read $21 as they play.
const ADAPTS: &[&str] = &["LostPig.z8", "curses.z5", "anchor.z8", "Tangle.z5", "borderzone-r9-s871008.z5"];

/// Stories that lay the status line out once.
const CACHES: &[&str] = &[
    "zork1-invclues-r52-s871125.z5",
    "trinity-r12-s860926.z4",
    "amfv-r77-s850814.z4",
    "sherlock-r26-s880127.z5",
    "beyondzork-r57-s871221.z5",
    "bureaucracy-r116-s870602.z4",
    "hitchhiker-invclues-r31-s871119.z5",
];

#[test]
fn stories_that_reread_the_width_are_told_the_pane() {
    for name in ADAPTS {
        let Some(s) = boot(name, 45) else { continue };
        assert_eq!(s.fixed_cols(), None, "{name} re-reads $21, so it follows the pane");
        assert_eq!(s.machine.mem.read_byte(0x21), 45, "{name} is told the 45-column pane");
    }
}

#[test]
fn stories_that_lay_out_once_are_told_eighty() {
    for name in CACHES {
        let Some(s) = boot(name, 45) else { continue };
        assert_eq!(s.fixed_cols(), Some(80), "{name} caches the width, so it is fixed");
        assert_eq!(s.machine.mem.read_byte(0x21), 80, "{name} is told 80 at a 45-column pane");
    }
}

/// Bureaucracy is classified by what it does, not by falling back: its form is a
/// status area it draws, and the probe watched it through many keypress turns
/// without one more read. (It reads char-by-char in its form, which is why the
/// early probe harness seemed stuck at "Press any key".)
#[test]
fn bureaucracy_is_classified_on_evidence() {
    let path = fixture_path("bureaucracy-r116-s870602.z4");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return;
    };
    let cfg = zvm::cpu::exec::BootConfig::new().with_screen_grid(25, 80);
    let p = zvm::width_probe::probe_status_width(bytes, cfg).expect("a v4 story");
    assert_eq!(p.verdict, zvm::width_probe::WidthVerdict::Caches);
    assert_eq!(p.evidence, zvm::width_probe::WidthEvidence::NoReadInBudget, "a budget spent, not a cap or a quit");
    assert!(p.status_drawn, "and a form was on screen the whole time");
}

fn dims(w: u16) -> (u16, u16) {
    let state = app::state::AppState::default();
    let (rows, cols) = app::render::screen::story_screen_dims(Rect::new(0, 0, w, 25), &state).unwrap();
    (cols, rows)
}

/// Resize as the TUI's per-frame poll does, then play a turn.
fn resize_and_play(s: &mut GameSession, pane_w: u16) {
    let state = app::state::AppState::default();
    app::host::screen::sync_zvm_screen_dims(s, &state, (pane_w, 25));
    let r = s.submit("look");
    assert!(!r.quit, "look quit the game");
}

#[test]
fn a_caching_story_never_sees_the_pane_change() {
    let Some(mut s) = boot("zork1-invclues-r52-s871125.z5", 45) else { return };
    assert_eq!(s.machine.mem.read_byte(0x21), 80);
    for w in [60u16, 120, 30] {
        resize_and_play(&mut s, w);
        assert_eq!(s.machine.mem.read_byte(0x21), 80, "$21 stays 80 at a {w}-column pane");
    }
}

#[test]
fn an_adapting_story_follows_the_pane() {
    let Some(mut s) = boot("LostPig.z8", 45) else { return };
    for w in [60u16, 120, 30] {
        resize_and_play(&mut s, w);
        assert_eq!(s.machine.mem.read_byte(0x21) as u16, dims(w).0, "$21 follows a {w}-column pane");
    }
}

/// The override key forces each verdict, whatever the story does.
#[test]
fn the_override_forces_each_verdict() {
    if let Some(s) = boot_mode("LostPig.z8", 45, StatusWidth::Fixed) {
        assert_eq!(s.fixed_cols(), Some(80), "fixed: an adapting story is told 80 anyway");
        assert_eq!(s.machine.mem.read_byte(0x21), 80);
    }
    if let Some(s) = boot_mode("zork1-invclues-r52-s871125.z5", 45, StatusWidth::Pane) {
        assert_eq!(s.fixed_cols(), None, "pane: a caching story is told the pane anyway");
        assert_eq!(s.machine.mem.read_byte(0x21), 45);
    }
    if let Some(s) = boot_mode("zork1-invclues-r52-s871125.z5", 45, StatusWidth::Auto) {
        assert_eq!(s.fixed_cols(), Some(80), "auto probes");
    }
}

#[test]
fn restart_reuses_the_verdict_it_does_not_probe_again() {
    // The decided mode round-trips through `with_status_width`, which is what the
    // restart path hands the new boot: Fixed stays Fixed with no probe (a probe of
    // LostPig would say Adapts, so a re-probe would flip it).
    let Some(s) = boot_mode("LostPig.z8", 45, StatusWidth::Fixed) else { return };
    assert_eq!(s.status_width_decided(), StatusWidth::Fixed);
    let Some(s) = boot("LostPig.z8", 45) else { return };
    assert_eq!(s.status_width_decided(), StatusWidth::Pane);
}

#[test]
fn a_pinned_virtual_screen_wins_over_every_mode() {
    let mut cfg = app::config::Config::default();
    for mode in [StatusWidth::Auto, StatusWidth::Fixed, StatusWidth::Pane] {
        cfg.status_width = mode;
        assert_eq!(cfg.effective_status_width(), mode);
    }
    cfg.virtual_screen_cols = Some(40);
    for mode in [StatusWidth::Auto, StatusWidth::Fixed, StatusWidth::Pane] {
        cfg.status_width = mode;
        assert_eq!(cfg.effective_status_width(), StatusWidth::Pane, "a pin is what the story is told");
    }
}

// ── SQ-1811: a hint visit in a narrow pane cannot change the told width ─────
//
// The Solid Gold built-in InvisiClues re-read header $21 each time a hint screen
// opens and refresh the game's cached width from it. Before SQ-1809 a resize
// followed by a hint visit relaid the status line out for the new width (the
// `0 2st of House` garble at 45 columns). A fixed-80 game reads 80 throughout.

use app::engine::{GridWindow, WinNode};
use app::session::{screen_model_from_machine, InputKind};

fn model_grid(s: &GameSession) -> GridWindow {
    match screen_model_from_machine(&s.machine).root {
        WinNode::Pair { first, .. } => match *first {
            WinNode::Grid(g) => g,
            other => panic!("expected a Grid, got {other:?}"),
        },
        other => panic!("expected a Pair root, got {other:?}"),
    }
}

fn grid_row(g: &GridWindow, r: u16) -> String {
    (1..=g.cols).map(|c| g.cell(r, c).ch).collect()
}

fn step(s: &mut GameSession, cmd: &str) {
    let r = match s.pending_input() {
        InputKind::Line => s.submit(cmd),
        InputKind::Char => s.submit_char(13),
        InputKind::Event => s.submit(""),
    };
    assert!(r.fault.is_none(), "fault: {:?}", r.fault);
    assert!(!r.quit, "{cmd:?} quit the game");
}

/// Boot at an 80-column pane, `look`, shrink to 45 and `look` (2 turns), visit the
/// hints (`hint`, `hint` = menu; Enter = a second hint screen), check `$21` and
/// the title while they are up, leave with `q` presses, then `look` (3 turns).
/// Returns the session and the status row after the last turn.
fn hint_visit(name: &str, mode: StatusWidth) -> Option<(GameSession, String)> {
    let mut s = boot_mode(name, 80, mode)?;
    step(&mut s, "look");
    resize_and_play(&mut s, 45);
    step(&mut s, "hint");
    step(&mut s, "hint");
    let menu = model_grid(&s);
    let all: String = (1..=menu.active_rows).map(|r| grid_row(&menu, r)).collect::<Vec<_>>().join("\n");
    assert!(all.contains("INVISICLUES"), "{name}: non-vacuity, the hint menu opened:\n{all}");
    assert_eq!(s.pending_input(), InputKind::Char, "{name}: a menu reads a key");
    let during_menu = s.machine.mem.read_byte(0x21);
    step(&mut s, ""); // Enter: the first topic's own screen, a second entry
    let during_topic = s.machine.mem.read_byte(0x21);
    if mode == StatusWidth::Auto {
        assert_eq!(during_menu, 80, "{name}: $21 while the menu is up");
        assert_eq!(during_topic, 80, "{name}: $21 while a topic is up");
    }
    for _ in 0..6 {
        if s.pending_input() == InputKind::Line {
            break;
        }
        let r = s.submit_char(b'q');
        assert!(r.fault.is_none());
    }
    assert_eq!(s.pending_input(), InputKind::Line, "{name}: back at the story's prompt");
    step(&mut s, "look");
    let g = model_grid(&s);
    let row = grid_row(&g, 1);
    Some((s, row))
}

fn reference_row(name: &str) -> Option<String> {
    let mut s = boot(name, 80)?;
    for _ in 0..3 {
        step(&mut s, "look");
    }
    Some(grid_row(&model_grid(&s), 1))
}

fn hint_visit_pins(name: &str, release: &str, room: &str) {
    let Some((s, row)) = hint_visit(name, StatusWidth::Auto) else { return };
    let want = reference_row(name).unwrap();
    let ctx = format!("{name} ({release}), 3 look turns around a hint visit, 45-column pane");
    assert_eq!(s.fixed_cols(), Some(80), "{ctx}");
    assert_eq!(s.machine.mem.read_byte(0x21), 80, "{ctx}: $21 after the visit");
    assert!(row.trim_start().starts_with(room), "{ctx}: the room leads: {row:?}");
    assert!(row.contains("Score:") && row.contains("Moves:"), "{ctx}: {row:?}");
    assert_eq!(row, want, "{ctx}: same bar as a fresh 80-column session");
}

#[test]
fn zork1_r52_hint_visit_keeps_the_status_row_at_80() {
    hint_visit_pins("zork1-invclues-r52-s871125.z5", "Zork I r52", "West of House");
}

#[test]
fn hitchhiker_r31_hint_visit_keeps_the_status_row_at_80() {
    hint_visit_pins("hitchhiker-invclues-r31-s871119.z5", "Hitchhiker's r31", "");
}

/// Control (the way SQ-0679's restore control is kept): told the pane, the same
/// sequence does not reproduce the 80-column bar.
#[test]
fn zork1_r52_hint_visit_with_pane_width_differs_from_the_reference() {
    let name = "zork1-invclues-r52-s871125.z5";
    let Some((s, row)) = hint_visit(name, StatusWidth::Pane) else { return };
    let want = reference_row(name).unwrap();
    eprintln!("pane-mode $21={} row={row:?}\nreference={want:?}", s.machine.mem.read_byte(0x21));
    assert_ne!(row, want, "control: the old behaviour must differ, or the pin proves nothing");
}
