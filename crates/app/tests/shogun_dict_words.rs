//! Shogun (v6) custom-alphabet dictionary smoke (SQ-0517).
//!
//! Shogun ships a custom alphabet table that moves the lowercase letters
//! j/q/v/x/z out of A0 and into A1. Encoding player input with the default
//! alphabet made every word containing one of them miss the game's own
//! dictionary — «I don't know the word "save".» This drives the real story a
//! few turns and asserts those words now reach the parser / game verbs instead
//! of the unknown-word error.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind, PendingIo};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot Shogun to its first in-game line prompt, mirroring the proven boot
/// sequence in `v6_shogun_gameplay.rs`. Returns the booted session.
fn boot_shogun() -> Option<GameSession> {
    let story_path = stories_dir().join("shogun-r322-s890706.z6");
    let story_bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        story_bytes,
        false,
        false,
        None,
        false,
        picture_dims,
        picts.std_window(),
        None,
    )
    .expect("Shogun (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    Some(session)
}

/// Advance non-line prompts (boot menu, [MORE], events) until the game asks for
/// a line of input, or the budget runs out. Returns true if a Line prompt is
/// reached.
fn advance_to_line(session: &mut GameSession, budget: usize) -> bool {
    for _ in 0..budget {
        match session.pending_input() {
            InputKind::Line => return true,
            InputKind::Char => {
                let _ = session.submit_char(13);
            }
            InputKind::Event => {
                let _ = session.submit("");
            }
        }
    }
    matches!(session.pending_input(), InputKind::Line)
}

#[test]
fn shogun_dictionary_words_reach_the_parser() {
    let Some(mut session) = boot_shogun() else {
        eprintln!("SKIP: gitignored Shogun story missing");
        return;
    };

    assert!(
        advance_to_line(&mut session, 12),
        "Shogun should reach an in-game line prompt after the boot menu"
    );

    // Sanity: an all-A0 word ("look") already worked; keep it as a live-parser
    // control alongside the fix targets.
    let look = session.submit("look");
    assert!(!look.quit);
    assert!(
        !look.transcript.contains("know the word"),
        "'look' must parse; got: {:?}",
        look.transcript
    );

    // "japan" — contains the A1-relocated 'j'. In context the game may reject it
    // as not-useful-here, but it must NOT be the unknown-word error.
    assert!(advance_to_line(&mut session, 6), "line prompt before 'japan'");
    let japan = session.submit("japan");
    assert!(!japan.quit);
    assert!(
        !japan.transcript.contains("know the word"),
        "'japan' (A1 'j') must be a known dictionary word; got: {:?}",
        japan.transcript
    );

    // "save" — contains the A1-relocated 'v'. It must reach the game's SAVE verb,
    // which suspends the VM on @save (PendingIo::Save), NOT the word error. We
    // cancel the save (host wrote nothing) to return to the prompt.
    assert!(advance_to_line(&mut session, 6), "line prompt before 'save'");
    let save = session.submit("save");
    assert!(
        !save.transcript.contains("know the word"),
        "'save' (A1 'v') must reach the SAVE verb, not the word error; got: {:?}",
        save.transcript
    );
    assert_eq!(
        save.pending_io,
        Some(PendingIo::Save),
        "'save' must drive the game to its @save flow"
    );
    let _ = session.resume_save(false); // cancel; nothing written

    // "quit" — contains the A1-relocated 'q'. Reaches the QUIT verb, which asks a
    // confirmation question; it must not immediately quit and must not be the
    // word error.
    assert!(advance_to_line(&mut session, 6), "line prompt before 'quit'");
    let quit = session.submit("quit");
    assert!(
        !quit.transcript.contains("know the word"),
        "'quit' (A1 'q') must reach the QUIT verb, not the word error; got: {:?}",
        quit.transcript
    );
    assert!(
        !quit.quit,
        "'quit' should ask for confirmation, not quit outright; got: {:?}",
        quit.transcript
    );

    // Control: a genuine non-word still gets Shogun's unknown-word error — proves
    // the parser is live and the smoke is meaningful. (Answer the quit prompt with
    // 'no' first if the game is waiting on a char/line to get back to a verb prompt.)
    for probe in ["no", "n", "xyzzy"] {
        if !advance_to_line(&mut session, 6) {
            break;
        }
        let r = session.submit(probe);
        if probe == "xyzzy" {
            assert!(
                r.transcript.contains("know the word"),
                "control: 'xyzzy' must still miss the dictionary; got: {:?}",
                r.transcript
            );
        }
    }
}
