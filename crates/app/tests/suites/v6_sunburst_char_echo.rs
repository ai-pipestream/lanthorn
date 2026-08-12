//! SQ-0726: a keystroke the game echoes back belongs on the line the player is
//! typing on, not on a line of its own.
//!
//! sunburst.z6 has no line reader. Past its boot menu it runs `read_char` in a
//! loop and `print_char`s every key straight back — its own line editor, built out
//! of the one opcode that echoes NOTHING (ZMSD §10.7). The host used to answer
//! each of those turns by opening a fresh transcript line, so typing `look` came
//! out as
//!
//!     …to the north you can see your ship.
//!     l
//!     o
//!     o
//!     k
//!
//! with the first character a line below the prompt. That per-turn line break is
//! deliberate everywhere else: an interpreter echoes a `read` together with its
//! terminating newline (§7.1.1.1), and the host reproduces it by appending the
//! typed command to the game's `>` and letting the reply open the line below —
//! zork1 needs exactly that, its turn output carrying no leading newline of its
//! own. `read_char` supplies no such newline, so for a keypress turn the host was
//! inventing one.
//!
//! Only the ECHO folds, and the narrowness is measured rather than timid. Dropping
//! the invented newline for every keypress turn moves six other titles — it joins
//! Arthur's, Shogun's, Journey's, advent's and fmvpoker's menu repaints to the line
//! above and concatenates four of mysterious01's re-asked prompts into one, because
//! those games reposition between reprints in ways the host transcript does not
//! model. Nothing else in the corpus answers a keypress with a single character.
//! SQ-0804 carries that sweep and the general case.
//!
//! The fixture earns its keep twice over: 65 KB, no resource blorb, zero pictures
//! declared — the only story in the corpus that drives the v6 path with no graphics
//! at all, so it exercises v6 TEXT in isolation.
//!
//! `stories/` is gitignored (CLAUDE.md), so every test here skips vacuously when
//! the story is absent.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind, TurnResult};
use app::state::{AppState, TranscriptKind};

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn open(name: &str) -> Option<GameSession> {
    let path = stories_dir().join(name);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut s = GameSession::new_with_trace(
        bytes, true, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("a valid v6 story");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// The transcript half of `apply_game_driven_result` (`crates/app/src/turn.rs`),
/// which lives in the binary and so cannot be called from here. Kept to the two
/// lines that matter: an `erase_lower` turn opens a new line, everything else goes
/// through the char-echo push.
fn apply_keypress(state: &mut AppState, r: &TurnResult) {
    if r.erase_lower {
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    } else {
        state.push_transcript_runs_char_echo(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    }
}

/// sunburst booted, `s` (Start game) taken at the menu — the point past which the
/// game is reading keys one at a time. Returns the session and a transcript that
/// has been fed the boot output and that turn, the way the run loop feeds it.
fn started() -> Option<(GameSession, AppState)> {
    let mut s = open("sunburst.z6")?;
    let mut state = AppState::default();
    state.push_transcript_runs(&s.take_transcript(), TranscriptKind::Story, &[]);
    // The boot menu is keyed by each item's lowercase initial and nothing else
    // (SQ-0706): `s` is "Start game."
    let r = s.submit_char(b's');
    apply_keypress(&mut state, &r);
    Some((s, state))
}

/// The premise this whole suite rests on, and the reason the fixture is worth
/// keeping: v6 with no graphics whatsoever.
#[test]
fn sunburst_is_the_only_v6_story_in_the_corpus_with_no_graphics_at_all() {
    let Some(session) = open("sunburst.z6") else { return };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else {
        panic!("a v6 story builds a Layered root")
    };
    assert!(
        !items.iter().any(|pw| matches!(pw.node, WinNode::Graphics(_))),
        "sunburst declares no pictures, so nothing in its screen model is a Graphics leaf — \
         this is the fixture that drives the v6 TEXT path on its own"
    );
    assert!(
        session.paint_surface().is_none_or(|p| p.pixels().all(|px| px[3] == 0)),
        "…and it paints no ground either"
    );
}

/// The other premise: the game really does read one key at a time and hand the key
/// straight back. Without this the fix has nothing to act on.
#[test]
fn sunburst_runs_its_own_line_editor_over_read_char() {
    let Some((mut session, _)) = started() else { return };
    assert_eq!(
        session.pending_input(),
        InputKind::Char,
        "past the menu sunburst reads single keys, not lines"
    );
    for ch in b"look" {
        let r = session.submit_char(*ch);
        assert_eq!(
            r.transcript,
            (*ch as char).to_string(),
            "the game's whole answer to a keypress is the keypress — it is echoing, and it \
             prints no newline with it (ZMSD §10.7)"
        );
        assert_eq!(session.pending_input(), InputKind::Char, "…and it asks for the next key");
    }
}

/// The report, and the fix.
///
/// Falsified by calling `push_transcript_runs` instead of
/// `push_transcript_runs_char_echo` in `apply_keypress`:
///
/// ```text
/// typing `look` must not break the transcript into one line per character — got
/// ["You stand right outside the entrance to Geeron Space harbor. To the east there
/// is a green park, and through the guarded gate to the north you can see your
/// ship.", "l", "o", "o", "k"]
/// ```
#[test]
fn a_typed_word_stays_on_the_line_the_player_is_typing_on() {
    let Some((mut session, mut state)) = started() else { return };
    let before = state.transcript.len();
    let prompt_line = state.transcript.last().cloned().unwrap_or_default();
    assert!(
        !prompt_line.is_empty(),
        "the game leaves the cursor at the end of a line it has printed — that is the line \
         the player types on"
    );

    for ch in b"look" {
        let r = session.submit_char(*ch);
        apply_keypress(&mut state, &r);
    }

    let added: Vec<String> = state.transcript[before.saturating_sub(1)..].to_vec();
    assert_eq!(
        state.transcript.len(),
        before,
        "typing `look` must not break the transcript into one line per character — got {added:?}"
    );
    assert_eq!(
        state.transcript.last().map(String::as_str),
        Some(format!("{prompt_line}look").as_str()),
        "the four echoed keys continue the line the game left the cursor on, in order"
    );

    // …and the pane shows it that way, in both v6 render modes. A single character
    // on a line of its own is the shape the report described.
    let model = session.screen();
    for mode in [app::config::V6RenderMode::Hybrid, app::config::V6RenderMode::Raster] {
        let mut st = AppState::default();
        st.colors = app::colors::ColorScheme::terminal_default();
        st.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        st.config.v6_render = mode;
        st.config.honor_game_colours = true;
        st.transcript = state.transcript.clone();
        st.transcript_kinds = state.transcript_kinds.clone();
        st.transcript_styles = state.transcript_styles.clone();
        st.transcript_runs = state.transcript_runs.clone();
        st.transcript_para = state.transcript_para.clone();
        st.transcript_images = state.transcript_images.clone();
        *st.v6_paint.borrow_mut() = session.paint_surface();

        let area = ratatui::layout::Rect::new(0, 0, 100, 34);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        // `char_mode` is true: sunburst is waiting on a key, so the host draws no
        // prompt of its own and what is on the pane is the game's own line.
        let _ = app::render::screen::render_story_pane(&model, true, None, &st, area, &mut buf);
        let rows: Vec<String> = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert!(
            !rows.iter().any(|r| matches!(r.as_str(), "l" | "o" | "k")),
            "{mode:?}: no row is a single echoed character — rows were {rows:?}"
        );
    }
}

/// The corpus guard. mysterious01 answers an unrecognised key by re-asking
/// "Resume play on a game ?" with no newline either side; folding THAT onto the
/// line above runs four copies of the question together. Only a one-character
/// answer folds, so this title's transcript is byte-identical to what it was.
#[test]
fn a_keypress_turn_that_reprints_a_prompt_still_opens_its_own_line() {
    let Some(mut session) = open("mysterious01.z6") else { return };
    let mut plain = AppState::default();
    let mut echoed = AppState::default();
    let boot = session.take_transcript();
    plain.push_transcript_runs(&boot, TranscriptKind::Story, &[]);
    echoed.push_transcript_runs(&boot, TranscriptKind::Story, &[]);

    let mut reprints = 0usize;
    for k in *b" \r\r\r" {
        if session.pending_input() != InputKind::Char {
            break;
        }
        let r = session.submit_char(k);
        if r.transcript.chars().count() > 1 {
            reprints += 1;
        }
        plain.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
        apply_keypress(&mut echoed, &r);
    }
    assert!(
        reprints > 0,
        "the premise: mysterious01 answers a key with a whole reprinted prompt, not one character"
    );
    assert_eq!(
        echoed.transcript, plain.transcript,
        "a multi-character keypress turn is left exactly where it was — folding it is SQ-0804"
    );
}
