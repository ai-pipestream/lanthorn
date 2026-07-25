//! Zork Zero InvisiClues hint-menu regression (SQ-0456 follow-up).
//!
//! The hint menu clears window 0's WRAPPING attribute (window_style op 2) and
//! paints topics via set_cursor, one row per item. Before the win0 paint-mode
//! routing, that output went through the flat transcript: every topic strung
//! together on one line, and every menu navigation appended another copy.
//! Skip-if-missing (gitignored story).

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

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

/// SQ-0515: Zork0's InvisiClues hint menu paints its " InvisiClues (tm)" title as
/// a reverse-video run (style bit 1) in a FULL-native-width grid window (w_px=640
/// of 640). In HYBRID mode the painted-screen renderer must flood that whole
/// terminal row edge to edge with the reverse bar — not reverse across only the
/// title's own glyphs — while the nav-help row beneath it (non-reversed runs) and
/// the selected-TOPIC highlight (a reverse run in the NARROW w_px=468 topic window)
/// stay text-width. Skip-if-missing (gitignored story).
#[test]
fn zork0_hybrid_hint_header_floods_full_width_reverse_bar() {
    let story_path = stories_dir().join("zork0-r393-s890714.z6");
    let Ok(story_bytes) = std::fs::read(&story_path) else {
        eprintln!("SKIP: gitignored story missing at {}", story_path.display());
        return;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(story_bytes, false, false, None, false, picture_dims, picts.std_window())
            .expect("Zork0 (v6) should load and boot");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    session.submit("hint");
    let entered = session.submit_char(b'y');
    assert!(entered.fault.is_none(), "entering the hint menu faulted: {:?}", entered.fault);

    let model = session.screen();
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row_text = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
    };
    let reversed_count = |y: u16| -> u16 {
        (0..area.width).filter(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED)).count() as u16
    };

    // Native row 0 (px y=1) carries the " InvisiClues (tm)" title — as terminal
    // cells on terminal row 0, flooded edge to edge with the reverse bar.
    let title = row_text(0);
    assert!(title.contains("InvisiClues"), "row 0 is the InvisiClues title bar: {title:?}");
    assert_eq!(
        reversed_count(0), area.width,
        "the title row is a solid reverse bar edge to edge (every cell reversed): {title:?}"
    );

    // The nav-help row below (native row 1 → terminal row 1: "N for next item." /
    // "Return for hints.", non-reversed runs in the SAME full-width window) must
    // NOT be flooded — its background gaps stay un-reversed.
    let nav = row_text(1);
    assert!(nav.contains("N for next item."), "row 1 is the nav-help row: {nav:?}");
    assert!(
        reversed_count(1) < area.width,
        "the nav-help row is not flooded full-width (non-reversed runs): {nav:?} ({} reversed)",
        reversed_count(1)
    );

    // The selected TOPIC highlight ("PROLOGUE", a reverse run in the NARROW
    // w_px=468 topic window) stays a text-width block — its row is not flooded
    // edge to edge.
    let topic_row = (0..area.height).find(|&y| row_text(y).contains("PROLOGUE"));
    if let Some(ty) = topic_row {
        assert!(
            reversed_count(ty) < area.width,
            "the selected-topic highlight (narrow topic window) is not flooded full-width: {:?}",
            row_text(ty)
        );
    }

    // FRAMELESS routes the hint menu through the SAME painted-screen renderer, so
    // the header floods identically there.
    state.config.v6_render = app::config::V6RenderMode::Frameless;
    let mut fbuf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut fbuf);
    let f_title: String = (0..area.width).map(|x| fbuf.cell((x, 0)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
    assert!(f_title.contains("InvisiClues"), "frameless: row 0 is the InvisiClues title bar: {f_title:?}");
    let f_rev = (0..area.width).filter(|&x| fbuf.cell((x, 0)).unwrap().modifier.contains(Modifier::REVERSED)).count() as u16;
    assert_eq!(f_rev, area.width, "frameless: the title row floods edge to edge too: {f_title:?}");
}

