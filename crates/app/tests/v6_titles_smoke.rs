//! Other-v6-title headless smokes (SQ-0186, Lane S of
//! `docs/superpowers/plans/2026-07-22-v6-completion.md`).
//!
//! For each of `stories/Arthur.blb`, `stories/Shogun.blb`, `stories/Journey.blb`
//! (each skip-if-missing, mirroring the skip-if-absent pattern in
//! `zork0_v6_windows.rs`): resolve the story path the same way `startup.rs`'s
//! `ZCode` arm does (`hints::load_story` classifies a Blorb's embedded
//! executable), boot the ZCOD exec headless with picture dims + std window,
//! step-capped to the first input, and assert no fault, a `Layered` screen
//! model root, and (where the boot state has populated windows) nonzero
//! window geometry. `machine.diagnostics` is collected and reported verbatim.
//!
//! FINDING: none of the three `.blb` files in this worktree's (gitignored,
//! local-only) `stories/` actually carry a `ZCOD` executable — each is a
//! resources-only Blorb (Pict/Reso data), exactly like Zork0's own release
//! layout (`zork0-r393-s890714.z6` beside a resources-only `Zork0.blb`
//! sidecar). `hints::load_story` reports `NoExecutable` for all three, so
//! these tests skip gracefully per the Lane S spec ("if a .blb has no ZCOD
//! exec, note that in the report and skip it gracefully in the test").

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::GameSession;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot `story_path` as a v6 story (mirroring `zork0_v6_windows.rs`'s setup)
/// and assert Lane S's headless-smoke invariants at the first input: no
/// fault, a `Layered` screen-model root, and — for any window the boot state
/// actually populated — a nonzero pixel size. Returns the collected
/// `machine.diagnostics` for the caller to report verbatim.
///
/// `story_path` is resolved the same way `startup.rs`'s `ZCode` arm resolves
/// a story: `hints::load_story` classifies a Blorb's embedded executable
/// (`ZCOD`/`GLUL`/Scott) or passes raw bytes through as Z-code. If the file at
/// `story_path` is a Blorb with no `ZCOD` executable (a resources-only
/// sidecar, not a self-contained story), this returns `None` — the caller
/// skips gracefully rather than asserting anything.
fn smoke_v6_title(title: &str, story_path: &PathBuf) -> Option<Vec<String>> {
    let bytes = match app::hints::load_story(story_path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        Ok(other) => {
            eprintln!("SKIP {title}: {} classified as non-ZCode ({other:?}), not a v6 Z-machine story", story_path.display());
            return None;
        }
        Err(e) => {
            eprintln!(
                "SKIP {title}: {} has no ZCOD executable ({e}) — a resources-only Blorb sidecar, not a self-contained story",
                story_path.display()
            );
            return None;
        }
    };

    let mut picts = PictSource::new(blorb::resolve_resource_blorb(story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    eprintln!("{title}: resolved {} Pict dimension entries, std_window={:?}", picture_dims.len(), picts.std_window());

    let mut session =
        GameSession::new_with_trace(bytes, false, false, None, false, picture_dims, picts.std_window())
            .unwrap_or_else(|e| panic!("{title} should load and boot without a ZError, got {e:?}"));

    assert!(!session.quit, "{title} quit during boot (fault or premature quit), before reaching the first prompt");
    assert!(
        session.machine.fault_trace.is_none(),
        "{title} faulted during boot: {:?}",
        session.machine.fault_trace.as_ref().map(|t| t.to_lines())
    );

    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let screen = session.screen();
    let WinNode::Layered(items) = &screen.root else {
        panic!("{title}'s v6 screen() root must be WinNode::Layered, got {:?}", screen.root);
    };
    eprintln!("{title}: {} layered window(s) at first input, content_size={:?}", items.len(), screen.content_size);
    for it in items {
        assert!(it.w_px > 0 && it.h_px > 0, "{title}: a positioned window has zero pixel size: {it:?}");
        eprintln!("  window x_px={} y_px={} w_px={} h_px={}", it.x_px, it.y_px, it.w_px, it.h_px);
    }
    if items.is_empty() {
        eprintln!("{title}: NOTE — no windows populated yet at the very first input (likely a pre-gameplay menu/prompt)");
    }

    Some(session.machine.diagnostics.clone())
}

#[test]
fn arthur_v6_boot_smoke() {
    let path = stories_dir().join("Arthur.blb");
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    }
    if let Some(diagnostics) = smoke_v6_title("Arthur", &path) {
        eprintln!("Arthur diagnostics: {diagnostics:?}");
    }
}

#[test]
fn shogun_v6_boot_smoke() {
    let path = stories_dir().join("Shogun.blb");
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    }
    if let Some(diagnostics) = smoke_v6_title("Shogun", &path) {
        eprintln!("Shogun diagnostics: {diagnostics:?}");
    }
}

#[test]
fn journey_v6_boot_smoke() {
    let path = stories_dir().join("Journey.blb");
    if !path.exists() {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    }
    if let Some(diagnostics) = smoke_v6_title("Journey", &path) {
        eprintln!("Journey diagnostics: {diagnostics:?}");
    }
}
