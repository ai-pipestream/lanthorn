//! SQ-1041: the story's own vocabulary, offered when the parser cannot have
//! understood the player — the first feature to speak in Lanthorn's Guiding
//! Light.
//!
//! `assist_voice.rs` pins the REGISTER (whose line it is, on which surface, and
//! that `push_assist` is the only door). This pins the FEATURE: when it speaks,
//! what it says, and — more of these cases than any other kind — when it does
//! not.
//!
//! # What the cases are really guarding
//!
//! **That the detection never reads the game's prose.** Every family words its
//! refusal differently and a story may reword it entirely; Dr Ludwig answers an
//! unknown verb with "Why, I don't even know what that verb means!". The offer
//! fires there exactly as it does under Infocom's `I don't know the word "…".`,
//! because it is looking at the story's dictionary and not at its output.
//!
//! **That silence is the ordinary answer.** A suggestion on every failed turn is
//! wallpaper, and the register's own test is the twentieth firing. Most cases
//! below assert that nothing was said.
//!
//! # The specimens
//!
//! | fixture | engine | dictionary | what it shows |
//! |---|---|---|---|
//! | a pocket story built here | stub | 6 chars | the wiring, on a machine with no `stories/` |
//! | `crates/scott/tests/tiny_cave.dat` | Scott | 3 chars | the two-word adapter, in-repo |
//! | `stories/zork1-r88-s840726.z3` | Z-machine v3 | 6 Z-chars | truncation, and the prose that undoes it |
//! | `stories/Dr Ludwig and the Devil.gblorb` | Glulx | 9 chars | a story that rewords the refusal |
//! | `stories/adv14a.dat` | Scott | 4 chars | an offer from a two-word parser |
//!
//! `stories/` is gitignored commercial media, so the last three skip vacuously.
//! The first two do not, and they are what CI actually runs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use app::engine::Engine;
use app::state::{AppState, TranscriptKind};
use app::vocab::{Position, StoryVocabulary};
use grammar_model::{NounKind, Slot, SyntaxLine, Token, Verb, WordRoles};

// ── A story with no engine under it ─────────────────────────────────────────

fn roles(verb: bool, noun: bool) -> WordRoles {
    let mut r = WordRoles::default();
    r.verb = verb;
    r.noun = noun;
    r
}

/// A pocket Version 3 story: `light`/`burn`, `take`/`get`, and a lantern whose
/// dictionary key is the six characters such a story can hold.
fn pocket_vocabulary() -> StoryVocabulary {
    let noun = || Slot::one(Token::Noun(NounKind::Noun));
    let verbs = vec![
        Verb::new(
            255,
            0,
            vec!["light".into(), "burn".into()],
            vec![SyntaxLine::new(1, false, vec![noun()])],
        ),
        Verb::new(
            254,
            0,
            vec!["take".into(), "get".into()],
            vec![SyntaxLine::new(2, false, vec![noun()])],
        ),
    ];
    let mut words = BTreeMap::new();
    for w in ["light", "burn", "take", "get"] {
        words.insert(w.to_string(), roles(true, false));
    }
    for w in ["lanter", "lamp", "the"] {
        words.insert(w.to_string(), roles(false, true));
    }
    StoryVocabulary::new(verbs, words, BTreeSet::new(), 6)
}

/// An engine that is nothing but a vocabulary. Everything a turn would need is
/// `unreachable!` — this double is never driven, only asked what its story knows.
struct PocketStory;

impl Engine for PocketStory {
    fn story_vocabulary(&self) -> Option<StoryVocabulary> {
        Some(pocket_vocabulary())
    }
    fn submit(&mut self, _command: &str) -> app::session::TurnResult {
        unreachable!("this double is asked about its dictionary, never driven")
    }
    fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> {
        unreachable!("this double is asked about its dictionary, never driven")
    }
    fn take_transcript(&mut self) -> String {
        String::new()
    }
    fn pending_input(&self) -> app::session::InputKind {
        app::session::InputKind::Line
    }
    fn resume_save(&mut self, _ok: bool) -> app::session::TurnResult {
        unreachable!("no save path here")
    }
    fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult {
        unreachable!("no restore path here")
    }
    fn has_quit(&self) -> bool {
        false
    }
    fn screen(&self) -> app::engine::ScreenModel {
        unreachable!("this double draws nothing")
    }
    fn save_state(&self) -> app::engine::EngineSave {
        unreachable!("no save path here")
    }
    fn restore_state(&mut self, _s: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> {
        unreachable!("no restore path here")
    }
    fn restore_game_save(&mut self, _b: &[u8]) -> Result<(), app::engine::EngineError> {
        unreachable!("no restore path here")
    }
    fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> {
        unreachable!("no aux data here")
    }
    fn set_aux_data(&mut self, _d: BTreeMap<String, Vec<u8>>) {}
    fn aux_dirty(&self) -> bool {
        false
    }
    fn clear_aux_dirty(&mut self) {}
    fn current_location(&self) -> Option<app::engine::LocationInfo> {
        None
    }
    fn drain_screen_clear(&mut self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// A state with the story's last reply already in the transcript, which is where
/// `finish_command_turn` calls the offer from.
fn after(reply: &str) -> AppState {
    let mut s = AppState::default();
    s.assist_preamble_shown = true; // the introduction has its own case in assist_voice
    s.push_transcript_kind(reply, TranscriptKind::Story);
    s
}

fn assists(s: &AppState) -> Vec<String> {
    s.transcript
        .iter()
        .zip(&s.transcript_kinds)
        .filter(|(_, k)| **k == TranscriptKind::Assist)
        .map(|(l, _)| l.clone())
        .collect()
}

/// The feature, on a machine with no `stories/`: an unknown word, and the story's
/// own words underneath it.
#[test]
fn an_unknown_word_is_answered_with_what_the_story_knows() {
    let mut s = after("I don't know the word \"lanturn\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lanter"]);
}

/// One line, in lanthorn's own words, and never in the parser's brackets or the
/// story's second person — the register, checked at the one place that writes it.
#[test]
fn the_offer_is_one_unbracketed_line_that_speaks_of_the_story() {
    let mut s = after("I don't know the word \"tkae\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lamp", true);
    let lines = assists(&s);
    assert_eq!(lines.len(), 1, "one line, on a pane that may be forty columns: {lines:?}");
    let line = &lines[0];
    assert!(!line.starts_with('['), "the brackets are the parser's voice: {line:?}");
    assert!(line.starts_with("this story knows — "), "{line:?}");
    assert!(!line.contains("You "), "the second person is the story's voice: {line:?}");
    assert_eq!(line.matches('·').count() + 1, line["this story knows — ".len()..].split(" · ").count());
    assert!(
        line["this story knows — ".len()..].split(" · ").count() <= 3,
        "three at most, or the player reads instead of playing: {line:?}"
    );
}

/// A word the story DOES hold is not a vocabulary problem. It may be out of
/// scope, or not a verb here, and answering either with near-misses is what makes
/// interactive-fiction help feel stupid.
#[test]
fn a_word_the_story_knows_is_never_answered() {
    let mut s = after("You can't see any lamp here!");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lamp", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// Two unknown words is not a command with one word wrong in it — it is a
/// sentence about things this story has never heard of, or a name typed at a
/// prompt. Speaking into one of those is the expensive mistake.
#[test]
fn two_unknown_words_are_never_answered() {
    let mut s = after("I don't know the word \"lanturn\".");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lanturn", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// Once a session. The twentieth `lanturn` is the register's own test, and a line
/// that fires every time is furniture.
#[test]
fn a_word_is_answered_once_a_session() {
    let mut s = after("I don't know the word \"lanturn\".");
    for _ in 0..20 {
        app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    }
    assert_eq!(assists(&s).len(), 1, "{:?}", assists(&s));
    // …and a DIFFERENT unknown word still gets its answer.
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "tkae lamp", true);
    assert_eq!(assists(&s).len(), 2, "{:?}", assists(&s));
}

/// A turn that printed nothing rejected nothing.
#[test]
fn a_silent_turn_is_never_answered() {
    let mut s = after("");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", false);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));
}

/// The player's switch reaches this feature, and reaches it before the story's
/// tables are read — so switching the light back on later still owes them the
/// answer rather than finding the word already marked as given.
#[test]
fn guidance_off_silences_the_offer_and_forgets_nothing() {
    let mut s = after("I don't know the word \"lanturn\".");
    s.config.guidance = false;
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert!(assists(&s).is_empty(), "{:?}", assists(&s));

    s.config.guidance = true;
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lanter"]);
}

/// The story printed the whole word, so the truncated key is shown whole. The
/// same command answered `lanter` above, with nothing in the transcript to spell
/// it out of.
#[test]
fn a_truncated_key_is_spelled_out_of_the_storys_own_prose() {
    let mut s = after("A battery-powered brass lantern is on the trophy case.");
    app::vocab::offer_vocabulary(&mut s, &PocketStory, "take lanturn", true);
    assert_eq!(assists(&s), vec!["this story knows — lantern"]);
}

// ── The Scott Adams adapter, on an in-repo database ─────────────────────────

fn tiny_cave() -> Vec<u8> {
    include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec()
}

/// A two-word parser has no grammar module and needs none: the adapter builds the
/// same neutral value the other two engines answer with. Its vocabulary lists pad
/// unused slots with `.`, which is a placeholder and not a word anybody could
/// type — falsified by keeping them, when `.` appears among the story's words.
#[test]
fn the_scott_adapter_answers_in_the_same_neutral_shape() {
    let session = app::scott_session::ScottSession::new(tiny_cave(), None)
        .expect("tiny_cave.dat is in the checkout");
    let v = session.story_vocabulary().expect("a Scott database always has a vocabulary");
    assert!(!v.is_empty());

    // Every verb reaches its own record, and a synonym reaches the same one.
    let take = v.verb_named("take").expect("tiny_cave knows TAKE");
    assert_eq!(take.words.first().map(String::as_str), Some("get"), "{:?}", take.words);
    assert!(take.words.iter().any(|w| w == "take"), "the synonym joins its canonical verb: {:?}", take.words);
    assert!(take.takes_bare() && take.max_nouns() == 1, "a two-word grammar is VERB [NOUN]");

    for verb in v.verbs() {
        for w in &verb.words {
            assert!(w.chars().any(char::is_alphanumeric), "`{w}` is a padding slot, not a word");
        }
    }

    // The database truncates to its own word length, and so does the snapshot.
    assert!(v.knows("score"), "the story's own verb");
    assert!(v.knows("scoreboard"), "and anything that truncates to it, as the parser sees it");
    assert!(!v.knows("xyzzy"));
}

/// Nothing is offered that the parser would reject — checked against a real
/// database rather than a hand-built one, across both positions.
#[test]
fn a_scott_offer_never_names_a_word_its_parser_would_refuse() {
    let session = app::scott_session::ScottSession::new(tiny_cave(), None)
        .expect("tiny_cave.dat is in the checkout");
    let v = session.story_vocabulary().expect("a Scott database always has a vocabulary");
    for typed in ["scoer", "taek", "pushing", "lanturn", "xyzzy"] {
        for pos in [Position::Opening, Position::Inside] {
            for w in v.offer(typed, pos, &[], &[]) {
                assert!(v.knows(&w), "{w:?} is not in tiny_cave's vocabulary");
            }
        }
    }
}

// ── Real stories. `stories/` is gitignored; these skip vacuously. ───────────

fn story(name: &str) -> Option<Vec<u8>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", path.display());
            None
        }
    }
}

/// Drive a real story through the same two steps `finish_command_turn` takes —
/// the game's reply into the transcript, then the offer — and hand back every
/// assist line it produced.
fn play(session: &mut dyn Engine, commands: &[&str]) -> (AppState, Vec<String>) {
    let mut state = AppState::default();
    state.assist_preamble_shown = true;
    let _ = session.take_transcript();
    for cmd in commands {
        let r = session.submit(cmd);
        state.push_transcript_kind(&format!("> {cmd}"), TranscriptKind::Input);
        state.push_transcript_kind(r.transcript.trim_end_matches('\n'), TranscriptKind::Story);
        let printed = !r.transcript.trim().is_empty();
        app::vocab::offer_vocabulary(&mut state, &*session, cmd, printed);
    }
    let lines = assists(&state);
    (state, lines)
}

fn zork1() -> Option<app::session::GameSession> {
    let bytes = story("zork1-r88-s840726.z3")?;
    let mut s = app::session::GameSession::new_with_trace(
        bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)),
    )
    .expect("zork1-r88-s840726.z3 boots without a ZError");
    s.set_strip_prompt(false); // inline-prompt mode, the shipped default
    Some(s)
}

/// Zork I release 88 / serial 840726, eight turns in: walk to the Living Room so
/// the story has PRINTED `lantern`, then mistype it. The dictionary holds
/// `lanter` and the offer says `lantern`, which is the word to type and the word
/// the parser matches.
///
/// The walk is the fixture. Falsify by asking before the Living Room — the offer
/// is still right and reads `lanter`, which is the case above.
#[test]
fn zork1_answers_a_mistyped_lantern_with_the_word_it_printed() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &["north", "east", "open window", "enter window", "west", "take lanturn"],
    );
    assert_eq!(lines, vec!["this story knows — lantern"]);
}

/// Three shapes of miss on one story, and the four kinds of turn that must stay
/// quiet — including `xyzzy`, which Zork answers with a hollow voice rather than
/// a refusal, and `unlock chest with key`, whose words the story all knows.
#[test]
fn zork1_answers_the_misses_it_can_and_stays_quiet_otherwise() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &[
            "opne mailbox",  // a transposition in the opening word
            "smel mailbox",  // a dropped letter, and a verb with its own synonym
            "opening mailbox", // an ending the story does not inflect
            "xyzzy",         // known, and not a refusal at all
            "take leaflet",  // understood
            "unlock mailbox with key", // every word known; the failure is not vocabulary
            "marcus",        // a name: near nothing, so nothing is said
        ],
    );
    assert_eq!(
        lines,
        vec![
            "this story knows — open",
            "this story knows — smell · sniff",
            "this story knows — open",
        ],
        "three offers and four silences"
    );
}

/// **What the whole synonym effort was for**, on the game everybody meets first.
/// `illuminate` is eight keystrokes from `light`, stems to nothing, and Zork's
/// grammar relates them not at all — every source that reads FORM is blind to
/// it, and the shipped table (SQ-1110, SQ-1115) is what closes the gap. This is
/// the wire SQ-1041 left hanging and SQ-1119 ran.
///
/// `inspect` is the one the player reported. `doff` is the case that says the
/// answer is not a fragment: `remove` is exactly the six characters a Version 3
/// dictionary keeps, and the aside rule discarded it until a word's WHOLENESS
/// travelled with it — without that, this reads `carry · remove · catch`, an
/// answer that has thrown away its own reason for existing.
///
/// Falsify by removing `by_meaning` from `candidates`: all four fall silent.
#[test]
fn zork1_answers_a_word_it_never_heard_with_what_that_word_means() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(
        &mut s,
        &["illuminate lamp", "inspect lamp", "conceal lamp", "doff sword"],
    );
    assert_eq!(
        lines,
        vec![
            "this story knows — light",
            "this story knows — examine · describe · see",
            "this story knows — hide · place · put",
            "this story knows — remove · carry · catch",
        ]
    );
}

/// SQ-1113: an IRREGULAR inflection, on the game everybody meets first. `took`
/// is `take` by no rule at all — there is nothing to strip off it — which is why
/// `stems` reached nothing here until WordNet's exception list shipped, and why
/// the near miss cannot stand in: `took` is three keystrokes from `take` and
/// that threshold is one, on purpose.
///
/// Zork I then adds its own synonyms for the verb once it is identified, which
/// is the aside source doing its usual work on top.
///
/// Every word here is out of reach of the near miss as well as of the rule —
/// `threw` was tried and dropped, because a single substitution reaches `throw`
/// and the case would have passed with the table removed.
///
/// Falsify by dropping the `irregular_bases` loop from `vocab::stems`: all three
/// lines fall silent.
#[test]
fn zork1_answers_an_irregular_inflection_with_the_verb_it_knows() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["took lamp", "broke lamp", "caught rope"]);
    assert_eq!(
        lines,
        vec![
            "this story knows — take · look · carry",
            "this story knows — break · block · smash",
            "this story knows — catch · carry · get",
        ]
    );
}

/// And `lit lamp` — the line the quest was NAMED for — is still silent, for a
/// reason that has nothing to do with the table: `lit` is three letters and
/// `MIN_LEN` answers nothing under four.
///
/// That gate is older than both word sources and is refused to both alike: `don`
/// means `wear`, which Zork knows, and is unanswered in the case below for
/// exactly the same reason. Pinned here so the silence reads as the policy it is
/// rather than as a hole in the data — the two assertions after it show the
/// table reaching `light` and the story holding it, with only the length between
/// them.
#[test]
fn zork1_stays_quiet_on_a_three_letter_irregular() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["lit lamp"]);
    assert!(lines.is_empty(), "{lines:?}");

    assert_eq!(verb_synonyms::irregular_bases("lit"), ["light"], "the table does reach it");
    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(v.knows("light"), "and the story does hold the word it reaches");
}

/// And the silences the meaning source must keep, on the same story — the ones
/// it is MOST able to erode, because a table of three thousand groups can always
/// find something.
///
/// `purchase` and `hint` are in the table and answered by nothing, because Zork
/// I's dictionary holds neither `buy` nor `help`: the intersection in `offer` is
/// what makes the feature honest, and it is the whole reason no censorship of
/// the table is needed. `don` means `wear`, which Zork DOES know, and is still
/// unanswered — `MIN_LEN` refuses anything under four characters and is older
/// than this source. `marcus` is a name and reaches the table not at all.
#[test]
fn zork1_stays_quiet_where_meaning_reaches_nothing_the_story_implements() {
    let Some(mut s) = zork1() else { return };
    let (_state, lines) = play(&mut s, &["purchase lamp", "hint", "don sword", "marcus"]);
    assert!(lines.is_empty(), "{lines:?}");

    let v = <app::session::GameSession as Engine>::story_vocabulary(&s).expect("zork1 has one");
    assert!(!v.knows("buy") && !v.knows("help"), "the two the table proposes and Zork lacks");
    assert!(v.knows("wear"), "and the one it has, which `don` is too short to reach");
}

/// **The case the whole detection design is for.** Dr Ludwig and the Devil
/// rewords Inform's refusal completely — "Why, I don't even know what that verb
/// means!" — and the offer fires anyway, because it asked the dictionary and
/// never read the reply.
///
/// Falsify by matching on the printed text instead: this story says none of the
/// things any such matcher would look for.
#[test]
fn a_glulx_story_that_rewords_the_refusal_is_answered_all_the_same() {
    let Some(bytes) = story("Dr Ludwig and the Devil.gblorb") else { return };
    let b = blorb::Blorb::parse(bytes).expect("a gblorb parses");
    let exec = b.executable().expect("an Exec chunk").1.to_vec();
    let mut s =
        app::glulx_session::GlulxSession::new(exec, 80, 24, true, false, false, (1, 1), Some(b), &[])
            .expect("Dr Ludwig boots");
    s.set_strip_prompt(false);
    // Its opening runs on keypresses; step past them to the first line prompt.
    for _ in 0..12 {
        if s.pending_input() != app::session::InputKind::Char {
            break;
        }
        let _ = s.submit_key(app::engine::KeyInput::Enter);
    }
    let (state, lines) = play(&mut s, &["opne door", "examien me"]);
    assert!(
        state.transcript.iter().any(|l| l.contains("I don't even know what that verb means")),
        "the specimen is this story's OWN wording of the refusal"
    );
    assert!(
        !state.transcript.iter().any(|l| l.contains("I don't know the word")),
        "and it never uses the wording a text matcher would have looked for"
    );
    assert_eq!(
        lines,
        vec!["this story knows — open · uncover · unwrap", "this story knows — examine · check · describe"],
        "the story's own synonym groups, free, once a verb is identified"
    );
}

/// A two-word parser, end to end. `adv14a.dat` keeps four characters of a word,
/// which is enough for a near miss to mean something — three keeps so little that
/// the parser refuses almost nothing, and the offer correctly never speaks there.
#[test]
fn a_scott_story_answers_a_mistyped_verb() {
    let Some(bytes) = story("adv14a.dat") else { return };
    let mut s = app::scott_session::ScottSession::new(bytes, None).expect("adv14a.dat loads");
    let (_state, lines) = play(&mut s, &["quti", "loko"]);
    assert_eq!(
        lines,
        vec!["this story knows — quit", "this story knows — look"],
        "a fragment (`exam`, `desc`) is fit to be the answer and not fit to be an aside"
    );
}
