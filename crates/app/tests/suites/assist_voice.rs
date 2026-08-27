//! The assist voice, and the two things that keep it honest (SQ-1045).
//!
//! Five features print through this register — SQ-1041's vocabulary offer,
//! SQ-1042's completion, SQ-1043's irreversibility caution, SQ-1044's recap, and
//! whatever follows — so what it guarantees has to be a property of the CODE, not
//! a habit of the prose.
//!
//! The property is **attribution**: a player must never mistake lanthorn's help
//! for the story's own voice, because when the help is WRONG they will otherwise
//! blame the game. Infocom's parser speaks in brackets (`[I don't know the word
//! "illuminate".]`), so a helper in the same stream is confusable by default.
//!
//! Two mechanisms, and the cases below prove each rather than asserting it:
//!
//! * **`TranscriptKind::Assist`** — the line is told from the story by its KIND,
//!   which means the test holds even against a story that prints our own marker
//!   verbatim (`an_assist_is_told_from_the_story_by_its_kind_not_its_text`), and
//!   `/filter story` can hide every assist for a player who wants 1982.
//! * **the marker in the text** — `Lanthorn: `, applied by
//!   `AppState::push_assist` and by nothing else, because the kind does not
//!   survive a copy-paste, a saved transcript, a screenshot in a bug report, or a
//!   screen reader, to which no colour or gutter glyph is audible.
//!
//! The source-level case at the bottom is the one that matters most, for the same
//! reason `palette_lock_discipline` exists: the next four features are written by
//! someone with no reason to know any of this, and a hand-built `Assist` line
//! that skips the marker would look perfectly fine in review.

use app::assist::{Assist, AssistTone, PREAMBLE, PREFIX};
use app::state::{AppState, TranscriptFilter, TranscriptKind};

fn kinds_of(s: &AppState) -> Vec<TranscriptKind> {
    s.transcript_kinds.clone()
}

/// The whole quest in one case: a story that prints our exact marker is STILL
/// told apart from an assist, because the separation is the kind and not the
/// wording. Falsify by tagging the assist `TranscriptKind::Meta` (or `Story`) in
/// `push_assist` — the two lines become indistinguishable and this fails.
#[test]
fn an_assist_is_told_from_the_story_by_its_kind_not_its_text() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true; // the flourish has its own case
    s.push_assist(&Assist::help("this story knows — light · turn on · burn"));
    let ours = s.transcript.last().cloned().unwrap();

    // A hostile (or merely unlucky) game prints the identical line itself.
    s.push_transcript_kind(&ours, TranscriptKind::Story);

    assert_eq!(s.transcript[s.transcript.len() - 2], s.transcript[s.transcript.len() - 1]);
    let k = kinds_of(&s);
    assert_eq!(k[k.len() - 2], TranscriptKind::Assist);
    assert_eq!(k[k.len() - 1], TranscriptKind::Story);
}

/// `/filter story` is the player asking for 1982, and 1982 had no assists.
/// `/filter meta` is the opposite half. Both must partition the same transcript.
#[test]
fn the_transcript_filter_separates_assists_from_the_story() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_transcript_kind("West of House", TranscriptKind::Story);
    s.push_assist(&Assist::help("this story knows — light"));
    s.push_transcript_kind("style reloaded", TranscriptKind::Meta);

    let assist_row = s
        .transcript
        .iter()
        .position(|l| l.starts_with(PREFIX))
        .expect("the assist line is in the transcript");

    s.transcript_filter = TranscriptFilter::Story;
    let story_only = s.visible_transcript_indices_from(0);
    assert!(!story_only.contains(&assist_row), "filter story must hide assists");

    s.transcript_filter = TranscriptFilter::Meta;
    let meta_only = s.visible_transcript_indices_from(0);
    assert!(meta_only.contains(&assist_row), "filter meta must show assists");

    s.transcript_filter = TranscriptFilter::Both;
    assert_eq!(s.visible_transcript_indices_from(0).len(), s.transcript.len());
}

/// The marker is mandatory and never tapers. The flourish above it is what
/// tapers — once per session, so the twentieth assist of a session is one line.
#[test]
fn the_flourish_tapers_and_the_marker_never_does() {
    let mut s = AppState::default();
    s.push_assist(&Assist::help("first"));
    assert_eq!(s.transcript[0], PREAMBLE);
    assert_eq!(s.transcript[1], format!("{PREFIX}first"));

    for n in 0..20 {
        s.push_assist(&Assist::caution(format!("later {n}")));
    }
    assert_eq!(s.transcript.iter().filter(|l| l.as_str() == PREAMBLE).count(), 1);
    assert!(s.transcript[2..].iter().all(|l| l.starts_with(PREFIX)));
    assert!(kinds_of(&s).iter().all(|k| *k == TranscriptKind::Assist));
}

/// Nothing lanthorn says as an assist may wear the parser's `[…]`, and no line of
/// it may sit flush left where a skimming eye reads it as prose: a continuation
/// hangs under the marker instead.
#[test]
fn no_assist_line_is_bracketed_or_flush_left_and_unmarked() {
    let mut s = AppState::default();
    s.assist_preamble_shown = true;
    s.push_assist(&Assist::help("this story knows:\nlight · turn on · burn"));
    for line in &s.transcript {
        assert!(!line.starts_with('['), "assist line wears the parser's brackets: {line:?}");
    }
    // The FIRST line is marked — a rule the indent test below must not be able to
    // satisfy on its own, or removing the marker entirely would still pass.
    assert!(s.transcript[0].starts_with(PREFIX), "the first assist line is unmarked: {:?}", s.transcript[0]);
    assert!(
        s.transcript[1..].iter().all(|l| l.starts_with("  ")),
        "a continuation sits flush left where it reads as prose: {:?}",
        s.transcript
    );
}

/// The two tones must actually be two, or SQ-1043's "you cannot undo this" looks
/// exactly like SQ-1042's completion.
#[test]
fn the_caution_tone_is_styled_apart_from_the_ordinary_light() {
    let s = AppState::default();
    let help = s.colors.theme.get(AssistTone::Help.selector()).style;
    let caution = s.colors.theme.get(AssistTone::Caution.selector()).style;
    assert_ne!(help, caution, "both tones resolved to the same style");
    // And both are themeable rather than hard-coded: the selectors exist.
    assert!(help.fg.is_some() && caution.fg.is_some());
}

// ── The guard ────────────────────────────────────────────────────────────────

fn app_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).expect("app/src is part of the checkout").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                let rel = p.strip_prefix(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap_or(&p);
                out.push((
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read_to_string(&p).expect("a source file in the checkout is readable"),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out.sort();
    out
}

/// **`AppState::push_assist` is the only door into the assist voice.**
///
/// A caller that tags a line `TranscriptKind::Assist` itself has skipped the
/// marker, the tone's style and the once-per-session flourish — and produced a
/// line that is attributable to the CODE but not to a reader, which is the half
/// that survives a copy-paste and therefore the half that matters. The compiler
/// cannot catch it (the variant has to be public for the renderer and the filter
/// to match on it), so it is caught here, at the moment the file is written.
///
/// Three files may name the variant, and each for a reason that is not
/// production of a line: `state.rs` holds the door itself and the `/filter`
/// bucketing, `assist.rs` documents the register, and `render/transcript.rs` has
/// to MATCH on it to draw the gutter, reserve the indent and pick the style. A
/// fourth file naming it is a second producer.
///
/// Falsified by adding `TranscriptKind::Assist` to any other file under
/// `crates/app/src/` — done, during SQ-1045, in `input.rs`, which this reported
/// by name.
///
/// A substring scan cannot tell a call from a mention, so a comment that spells
/// the variant out is reported too. Reword the comment rather than relaxing the
/// case: the spelling is what the next author copies.
#[test]
fn push_assist_is_the_only_producer_of_an_assist_line() {
    const ALLOWED: &[&str] = &["src/state.rs", "src/assist.rs", "src/render/transcript.rs"];
    let offenders: Vec<String> = app_sources()
        .into_iter()
        .filter(|(name, src)| {
            src.contains("TranscriptKind::Assist") && !ALLOWED.iter().any(|a| name.ends_with(a))
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "these files build an assist line without going through AppState::push_assist, so they \
         skip the mandatory \"{PREFIX}\" marker: {offenders:?}"
    );
}

/// The register's constants, checked where the next four lanes will copy them
/// from: the marker names the app, the flourish explains what the marker MEANS
/// (it is the one chance to say so), and neither is in the parser's voice.
#[test]
fn the_register_says_who_is_speaking() {
    assert!(PREFIX.starts_with("Lanthorn"));
    assert!(PREAMBLE.starts_with("Lanthorn"));
    assert!(PREAMBLE.contains("interpreter") && PREAMBLE.contains("story"));
    assert!(!PREAMBLE.starts_with('[') && !PREFIX.starts_with('['));
    // Short enough to arrive mid-play without pushing the story off a small pane.
    assert!(PREFIX.len() <= 12, "the marker rides on every assist line: {PREFIX:?}");
}
