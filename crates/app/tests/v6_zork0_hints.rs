//! Zork Zero InvisiClues hint-menu regression (SQ-0456 follow-up).
//!
//! The hint menu clears window 0's WRAPPING attribute (window_style op 2) and
//! paints topics via set_cursor, one row per item. Before the win0 paint-mode
//! routing, that output went through the flat transcript: every topic strung
//! together on one line, and every menu navigation appended another copy.
//! Skip-if-missing (gitignored story).

use std::path::PathBuf;

use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

#[test]
fn zork0_hint_menu_paints_topics_by_row_and_keeps_transcript_clean() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();

    let mut result = session.submit("hint");
    assert!(result.transcript.contains("Do you still want a hint?"), "hint warning prompt");
    assert_eq!(session.pending_input(), InputKind::Char);
    result = session.submit_char(b'y');
    assert!(result.fault.is_none(), "entering the hint menu faulted: {:?}", result.fault);

    // Topics paint into window 0 as positioned runs, one 8-px row per item —
    // NOT into the transcript (which strung them all together before).
    assert!(
        !result.transcript.contains("PROLOGUE"),
        "hint topics must not stream into the transcript: {:?}",
        result.transcript
    );
    let topic_rows = |session: &GameSession| -> std::collections::BTreeMap<u16, String> {
        let mut rows: std::collections::BTreeMap<u16, String> = Default::default();
        for t in &session.machine.screen.v6.as_ref().unwrap().windows[0].texts {
            rows.entry(t.y).or_default().push_str(&t.text);
        }
        rows
    };
    let rows = topic_rows(&session);
    assert!(rows.len() >= 15, "one painted row per topic, got {} rows: {rows:?}", rows.len());
    let all: String = rows.values().cloned().collect::<Vec<_>>().join("\n");
    for topic in ["PROLOGUE", "EAST WING", "GENERAL QUESTIONS", "THE JESTER"] {
        assert!(all.contains(topic), "missing topic {topic:?} in painted rows:\n{all}");
    }
    // Distinct topics live on distinct rows (the strung-together bug had
    // everything on one line).
    let prologue_row = rows.iter().find(|(_, s)| s.contains("PROLOGUE")).map(|(y, _)| *y);
    let jester_row = rows.iter().find(|(_, s)| s.contains("THE JESTER")).map(|(y, _)| *y);
    assert_ne!(prologue_row, jester_row, "topics must occupy different rows");

    // The instruction header paints into window 1.
    let header: String =
        session.machine.screen.v6.as_ref().unwrap().windows[1].texts.iter().map(|t| t.text.as_str()).collect();
    assert!(header.contains("InvisiClues"), "menu header missing: {header:?}");
    assert!(header.contains("N for next item."), "navigation help missing: {header:?}");

    // Menu navigation repaints in place: no transcript output, no fault.
    let nav = session.submit_char(b'n');
    assert!(nav.fault.is_none(), "menu navigation faulted: {:?}", nav.fault);
    assert!(
        nav.transcript.trim().is_empty(),
        "navigation must repaint, not stream: {:?}",
        nav.transcript
    );

    // Q resumes the story: wrapping restored, prose streams to the transcript
    // again at the game's prompt.
    let quit = session.submit_char(b'q');
    assert!(quit.fault.is_none(), "leaving the hint menu faulted: {:?}", quit.fault);
    assert_eq!(session.pending_input(), InputKind::Line, "back at the story prompt");
    let look = session.submit("look");
    assert!(
        look.transcript.contains("Banquet Hall"),
        "post-menu prose must stream to the transcript again: {:?}",
        look.transcript
    );
}
