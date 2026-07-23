//! Shogun (v6) gameplay smoke (SQ-0456).
//!
//! Two regressions covered:
//! 1. Custom-alphabet ZSCII leak: Shogun's alphabet table holds ZSCII 11 (the
//!    sentence gap); decoding it as a raw char put `\u{b}` in every prose
//!    string, panicking ratatui's `cell_width` debug assert in live play.
//! 2. Zero-width input: the game sizes its READ buffer from the current
//!    window's font width (window prop 13). Uninitialized font props made
//!    that 0, so every typed command arrived empty ("[I beg your pardon?]").
//!
//! Skip-if-missing pattern per the other gitignored-story smokes.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// All control chars (except newline) in `s` — must always be empty for
/// anything the renderer will touch.
fn ctrl_chars(s: &str) -> Vec<char> {
    s.chars().filter(|c| c.is_control() && *c != '\n').collect()
}

#[test]
fn shogun_boots_plays_and_emits_no_control_chars() {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Shogun (v6) should load and boot without a ZError");
    assert!(!session.quit, "Shogun quit during boot");
    assert!(session.machine.fault_trace.is_none(), "Shogun faulted during boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    // Select START on the boot menu (Enter), then drive the opening: the intro
    // prose must arrive clean (regression 1) and typed commands must actually
    // reach the parser (regression 2) — "look" re-describes the Bridge and an
    // unknown word gets Shogun's word error, never the empty-input
    // "[I beg your pardon?]".
    let mut saw_bridge_look = false;
    let mut saw_unknown_word = false;
    for turn in 0..8 {
        let result = match session.pending_input() {
            InputKind::Line => session.submit(if turn % 2 == 0 { "look" } else { "xyzzy" }),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        assert!(!result.quit, "Shogun quit on turn {turn}");
        assert!(result.fault.is_none(), "Shogun faulted on turn {turn}: {:?}", result.fault);
        assert_eq!(
            ctrl_chars(&result.transcript),
            Vec::<char>::new(),
            "turn {turn}: control chars leaked into the transcript (custom-alphabet ZSCII bug): {:?}",
            result.transcript
        );
        assert!(
            !result.transcript.contains("I beg your pardon"),
            "turn {turn}: typed command arrived empty (v6 window font-prop init bug): {:?}",
            result.transcript
        );
        if result.transcript.contains("Bridge") {
            saw_bridge_look = true;
        }
        if result.transcript.contains("know the word") {
            saw_unknown_word = true;
        }
        // The v6 window model's own text runs feed the raster/hybrid renderer;
        // they must be clean too.
        if let Some(v6) = session.machine.screen.v6.as_ref() {
            for (i, w) in v6.windows.iter().enumerate() {
                let joined: String = w.texts.iter().map(|t| t.text.as_str()).collect();
                assert_eq!(
                    ctrl_chars(&joined),
                    Vec::<char>::new(),
                    "turn {turn}: control chars in window {i} text runs"
                );
            }
        }
    }
    assert!(saw_bridge_look, "\"look\" never re-described the Bridge — parser not receiving input");
    assert!(saw_unknown_word, "\"xyzzy\" never got the unknown-word reply — parser not receiving input");
}
