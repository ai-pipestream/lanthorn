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
