//! SQ-1101: what the assist features do with a story whose grammar cannot be
//! read — the Dialog case, driven end to end rather than argued from the types.
//!
//! # Why this suite exists
//!
//! `zvm::grammar` answers [`GrammarError::Absent`] for a Dialog story: the
//! compiler emits no grammar table of any shape, which
//! `crates/zvm/tests/dialog_grammar.rs` establishes from `dialogc`'s own sources
//! and from the `Dia` signature both files carry at header $39..$3B. That was
//! recorded as "degrades silently and correctly" back when one feature read the
//! grammar. Five do now — the vocabulary offer (SQ-1041), the irregular
//! inflection lookup (SQ-1113), the meaning table (SQ-1110), the command band's
//! verb column (SQ-1111) and the word reveal — so "silently and correctly" is a
//! claim that has to be re-measured rather than inherited.
//!
//! Every case below is therefore about an **absence**: a band that is populated
//! but honest about whose words it holds, an offer that says nothing rather than
//! promising vocabulary it cannot supply, a reveal that lights nothing rather
//! than panicking on a dictionary with no parts of speech. The failure this
//! guards against is not a crash — it is an assist line that claims "this story
//! knows —" and then has nothing to put after the dash.
//!
//! # Why a Dialog story and not a synthetic one
//!
//! Because the interesting half is everything a Dialog story DOES answer.
//! `Grammar::load` refuses, but the dictionary is a perfectly ordinary Z-machine
//! dictionary, the object tree is real, and the story plays. A stub that answers
//! `None` to everything would pass these assertions while proving nothing about
//! the mixed state a real one is in.
//!
//! # The specimens
//!
//! | fixture | engine | Dialog | turns |
//! |---|---|---|---|
//! | `stories/ImpossibleStairs.z8` | Z-machine v8 | 0m/03 | boot + 1 |
//! | `stories/frankenfingers_260330.z5` | Z-machine v5 | 1a/01-dev | boot + 1 |
//!
//! `stories/` is gitignored commercial media, so all of it skips vacuously
//! without it. The synthetic half of SQ-1101 — that the Dialog SIGNATURE is what
//! produces the refusal — is in `zvm::grammar`'s unit tests and runs in CI.

use app::engine::Engine;
use app::render::command_band::{default_quick, default_verbs, refresh_verbs, VerbSource, COL_VERB};
use app::session::GameSession;
use app::state::{AppState, CommandBandState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The pane the reveal reads. It reads what is DRAWN, so a case that wants an
/// answer other than `NoText` has to draw a frame first.
const AREA: Rect = Rect { x: 0, y: 0, width: 80, height: 24 };

const DIALOG_STORIES: [&str; 2] = ["ImpossibleStairs.z8", "frankenfingers_260330.z5"];

fn boot(file: &str) -> Option<GameSession> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, None)
            .expect("a Dialog story boots like any other Z-machine story");
    session.set_strip_prompt(false);
    let _ = session.take_transcript();
    Some(session)
}

fn assists(s: &AppState) -> Vec<String> {
    s.transcript
        .iter()
        .zip(&s.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

/// The premise, checked on the real files rather than assumed from the quest:
/// the engine answers "no vocabulary" — and the story is nevertheless alive,
/// printing prose in reply to a command.
#[test]
fn a_dialog_story_plays_and_offers_no_vocabulary() {
    for name in DIALOG_STORIES {
        let Some(mut s) = boot(name) else { continue };
        assert!(
            s.story_vocabulary().is_none(),
            "{name} is a Dialog story: `Grammar::load` answers Absent, so there is no snapshot"
        );

        let r = s.submit("look");
        assert!(!r.transcript.trim().is_empty(), "{name} answers a command with prose");
        assert!(!r.quit, "{name} is still running");
    }
}

/// The command band. The column stays USABLE and stays HONEST: the built-in
/// verbs, under the `VERB — generic` header that admits they are not this
/// story's own. Identical to the Journey case in `command_band.rs`, which is the
/// point — a Dialog story now takes the same already-pinned road as any other
/// story with no grammar, instead of its own.
#[test]
fn the_band_keeps_the_generic_column_and_labels_it() {
    for name in DIALOG_STORIES {
        let Some(session) = boot(name) else { continue };
        let mut state = AppState::default();
        state.overlays.command_band = Some(CommandBandState::new(default_verbs(), default_quick()));
        state.band_dock.toggle_to(true, true);

        assert!(!refresh_verbs(&mut state, &session), "{name}: no grammar, no change");

        let band = state.overlays.command_band.as_ref().unwrap();
        assert_eq!(band.verb_source, VerbSource::Builtin, "{name}");
        assert!(!band.items(COL_VERB).is_empty(), "{name}: the column is never empty");
        assert_eq!(band.column_label(COL_VERB), "VERB — generic", "{name}: it admits what it is");
    }
}

/// The vocabulary offer, on the turn it exists to serve. A misspelling that
/// would draw `this story knows — …` out of Zork draws **nothing** here, because
/// there is no vocabulary to draw on — and silence is the right answer, not a
/// lead with an empty list after it.
#[test]
fn the_offer_stays_silent_rather_than_leading_with_nothing() {
    for name in DIALOG_STORIES {
        let Some(mut session) = boot(name) else { continue };
        let mut state = AppState::default();
        state.assist_preamble_shown = true;

        for cmd in ["opne door", "xyzzyfoo", "take lanturn"] {
            let r = session.submit(cmd);
            state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
            state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
            let printed = !r.transcript.trim().is_empty();
            app::vocab::offer_vocabulary(&mut state, &session, cmd, printed);
        }

        assert_eq!(
            assists(&state),
            Vec::<String>::new(),
            "{name} has nothing to offer, and says so"
        );

        // Non-vacuity: the silence is the ABSENT grammar, not the Guiding Light
        // being off or the preamble swallowing the first line. Both of those
        // would silence a story that HAS vocabulary, and neither is in force.
        assert!(state.config.guidance, "{name}: the light is on");
        assert!(state.vocab.get(&session).is_none(), "{name}: and there is nothing to say");
    }
}

/// The word reveal. It reads the story's world model first and its dictionary
/// parts-of-speech second, and a Dialog story gives neither — Dialog keeps its
/// object data in its own arrays rather than Z-machine properties, and its
/// dictionary carries no Inform or ZIL flag bytes. Whatever tier it lands in,
/// it must arm without panicking and never leave a stale reveal behind.
///
/// **Both specimens reach `Armed::Nothing` today, and its status line is the one
/// dishonest thing this lane found**: "nothing on screen is a word this story
/// takes" is a claim about the ROOM, and here the truth is that we cannot read
/// the story's words at all. Filed as **SQ-1150** — the fix is a fourth `Armed`
/// variant, and this case is deliberately written to accept either answer so it
/// does not have to change twice. Tighten it to demand the new variant then.
#[test]
fn the_reveal_arms_without_a_grammar_and_lights_nothing_false() {
    for name in DIALOG_STORIES {
        let Some(mut session) = boot(name) else { continue };
        let mut state = AppState::default();
        state.colors = app::colors::ColorScheme::terminal_default();
        let r = session.submit("look");
        for line in r.transcript.split('\n') {
            state.push_transcript_kind(line, TranscriptKind::Story);
        }
        // Render once for its side effects: the wrap cache and `transcript_geom`
        // are what `reveal::arm` reads, and neither exists until a frame is drawn.
        let mut buf = Buffer::empty(AREA);
        app::render::transcript::render_transcript(
            &app::engine::StatusModel::HostManaged,
            None,
            &state,
            AREA,
            &mut buf,
            None,
        );

        let armed = app::reveal::arm(&mut state, &session);
        // Non-vacuity: `arm` really read the drawn text and really consulted the
        // story, rather than bailing at one of its two early doors.
        assert_ne!(armed, app::reveal::Armed::GuidanceOff, "{name}: the light is on");
        assert_ne!(armed, app::reveal::Armed::NoText, "{name}: the story printed a room");
        match armed {
            app::reveal::Armed::Lit { words, .. } => {
                assert!(words > 0, "{name}: `Lit` must mean something is lit");
                assert!(state.reveal.is_some(), "{name}: a lit reveal is held in state");
            }
            _ => assert!(state.reveal.is_none(), "{name}: nothing lit leaves nothing behind"),
        }
    }
}
