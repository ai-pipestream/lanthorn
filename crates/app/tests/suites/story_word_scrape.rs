//! SQ-1116: the words the story printed, filtered by the story's own dictionary.
//!
//! The extractor behind Tab completion and the command band's *here* fallback
//! used to answer "is this a word?" in hand-rolled English: ~40 stop words, a
//! bespoke splitter, and a three-character floor. All three were guesses, and the
//! story itself can settle every one of them — `zvm::dictionary::tokenise` is the
//! routine `read` calls, and `lookup` encodes a probe exactly as the parser does.
//!
//! # The specimens
//!
//! | fixture | engine | dictionary | what it shows |
//! |---|---|---|---|
//! | `crates/zvm/tests/fixtures/minizork.z3` | Z-machine v3 | 6 Z-chars | the real tokeniser, and truncation |
//! | `crates/scott/tests/tiny_cave.dat` | Scott | 3 chars | short words, and no grammar table |
//! | a hand-built pocket vocabulary | none | 6 chars | the three bugs, stated one at a time |
//!
//! Both story fixtures are tracked, so CI runs every case here — no vacuous
//! skips.

use std::collections::{BTreeMap, BTreeSet};

use app::complete::{split_prose, story_words};
use app::engine::Engine;
use app::state::AppState;
use app::vocab::StoryVocabulary;
use grammar_model::WordRoles;

use crate::fixture_paths::fixture_path;

// ── The extractor the readers replaced ──────────────────────────────────────

/// `complete::room_words_from_text` exactly as it stood before SQ-1116, kept
/// here and nowhere else: every case that claims a bug is fixed runs the old
/// code beside the new one and shows the old one failing. This is the falsifier
/// the quest asks for, frozen instead of reverted by hand.
fn stop_word_scrape(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "are", "you", "can", "not", "has", "was", "for", "with", "its", "this",
        "that", "have", "been", "from", "into", "onto", "there", "here", "some", "your", "also",
        "very", "than", "then", "will", "would", "could", "they", "them", "their", "but", "all",
        "any",
    ];
    let mut words: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '\'') {
        if word.is_empty() {
            continue;
        }
        let lower = word.to_lowercase();
        if lower.len() < 3 || STOP_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if seen.insert(lower.clone()) {
            words.push(lower);
        }
    }
    words
}

// ── Fixtures ────────────────────────────────────────────────────────────────

/// Mini-Zork I (r34/s871124), a Version 3 story whose dictionary keeps six
/// Z-characters — the truncation that makes a printed word and a stored key
/// different strings.
fn boot_minizork() -> app::session::GameSession {
    let path = fixture_path("minizork-r34-s871124.z3");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("minizork.z3 is tracked at {}: {e}", path.display()));
    app::session::GameSession::new_with_trace(
        bytes,
        true,
        false,
        None,
        false,
        Vec::new(),
        None,
        None,
        Some((25, 80)),
    )
    .expect("minizork.z3 should load and boot")
}

fn tiny_cave() -> app::scott_session::ScottSession {
    let bytes = include_bytes!("../../../scott/tests/tiny_cave.dat").to_vec();
    app::scott_session::ScottSession::new(bytes, None).expect("tiny_cave.dat is in the checkout")
}

/// Everything the story has said so far, in an `AppState`, the way `startup.rs`
/// and `turn.rs` leave it — the scrapers read the transcript, not the engine's
/// last reply.
fn state_with(lines: &[&str]) -> AppState {
    let mut s = AppState::default();
    for line in lines {
        s.push_transcript(line);
    }
    s
}

fn roles(verb: bool, noun: bool) -> WordRoles {
    let mut r = WordRoles::default();
    r.verb = verb;
    r.noun = noun;
    r
}

/// A pocket Version 3 vocabulary: `the` really is a word (Inform and ZIL both
/// hold it), `lanter` is the six characters such a dictionary keeps of `lantern`,
/// and `n`/`up` are the one- and two-character words every story has.
fn pocket() -> StoryVocabulary {
    let mut words = BTreeMap::new();
    for w in ["take", "open", "n", "up"] {
        words.insert(w.to_string(), roles(true, false));
    }
    for w in ["lanter", "mailbo", "the", "here"] {
        words.insert(w.to_string(), roles(false, true));
    }
    StoryVocabulary::new(Vec::new(), words, BTreeSet::new(), 6)
}

fn scraped(v: &StoryVocabulary, text: &str) -> Vec<String> {
    story_words(&split_prose(text), |w| v.knows(w))
}

// ── The three bugs, one case each ───────────────────────────────────────────

/// **Bug 1 — the three-character floor.** `x`, `n`, `s`, `up`, `in` and `at` are
/// words in every story ever written, and the old floor could not offer one of
/// them however hard the player leaned on Tab.
#[test]
fn the_length_floor_dropped_words_every_story_holds() {
    let v = pocket();
    let text = "> n\nYou go up. Open the mailbox.";

    let now = scraped(&v, text);
    for w in ["n", "up"] {
        assert!(now.contains(&w.to_string()), "{w:?} is a word this story holds: {now:?}");
    }

    let before = stop_word_scrape(text);
    for w in ["n", "up"] {
        assert!(
            !before.contains(&w.to_string()),
            "the falsifier: the old floor could never offer {w:?}"
        );
    }
}

/// **Bug 2 — truncation was invisible.** A Version 3 dictionary stores six
/// Z-characters, so it holds `lanter`; the prose says `lantern`. Anything
/// matching printed words against stored keys connects the two never — and it is
/// the long, specific nouns that are worth completing.
#[test]
fn a_truncated_entry_is_reached_by_the_word_the_game_printed() {
    let v = pocket();
    // Truncation cuts both ways, and that is the parser's behaviour, not a
    // slip: `lanternx` reaches the same entry, because the game would take it
    // too. Six characters is all either side ever compares.
    assert!(v.knows("lanternx"));

    let now = scraped(&v, "The brass lantern is here.");
    assert!(now.contains(&"lantern".to_string()), "`lanter` is reached by `lantern`: {now:?}");
    assert!(!now.contains(&"lanter".to_string()), "and offered as the game printed it");

    // The naive filter this replaces — printed word against stored key — is what
    // loses it, and it loses it silently.
    let stored_keys_only: Vec<String> =
        stop_word_scrape("The brass lantern is here.").into_iter().filter(|w| w == "lanter").collect();
    assert!(stored_keys_only.is_empty(), "the falsifier: no printed word ever equals `lanter`");
}

/// **Bug 3 — the stop list overruled the story.** `the` and `here` were on a
/// hand-written list of words that could not be offered. Both are real
/// vocabulary in real stories, and the story is the only authority on that.
#[test]
fn the_stop_list_overruled_the_story() {
    let v = pocket();
    let now = scraped(&v, "The mailbox is here.");
    for w in ["the", "here"] {
        assert!(now.contains(&w.to_string()), "the story holds {w:?}, so it is a word: {now:?}");
        assert!(
            !stop_word_scrape("The mailbox is here.").contains(&w.to_string()),
            "the falsifier: {w:?} was on the stop list"
        );
    }

    // And the other direction, which is the whole point: nothing is suppressed by
    // US. A word the story does NOT hold is dropped because the story does not
    // hold it.
    assert!(!now.contains(&"brass".to_string()), "no `brass` in this pocket dictionary");
}

// ── The Z-machine, through its own tokeniser ────────────────────────────────

/// Mini-Zork's opening screen, scraped through the story's own `tokenise` and
/// its own `lookup`. Every word offered is one the parser accepts, and the words
/// the old path invented are gone.
#[test]
fn minizork_offers_only_words_its_parser_accepts() {
    let session = boot_minizork();
    let mut state = state_with(&[
        "West of House",
        "You are standing in an open field west of a white house, with a boarded front door.",
        "There is a small mailbox here.",
    ]);
    app::input::refresh_seen_words(&mut state, &session);
    let seen = state.seen_words.clone();
    assert!(!seen.is_empty(), "a room description is not a blank page: {seen:?}");

    // The oracle is the parser itself, asked word by word.
    for w in &seen {
        assert_eq!(
            session.knows_word(w),
            Some(true),
            "{w:?} was offered and the story does not know it: {seen:?}"
        );
    }

    // The nouns a player would reach for are all there…
    for w in ["mailbox", "house", "door"] {
        assert!(seen.contains(&w.to_string()), "{w:?} should be offered: {seen:?}");
    }

    // …and so, on this one screen of a real story, are all three bugs. Words the
    // LENGTH FLOOR could never offer, every one of them in mini-Zork's dictionary:
    for w in ["a", "an", "in", "is", "of"] {
        assert!(seen.contains(&w.to_string()), "{w:?} is two characters and a word: {seen:?}");
    }
    // …and words the STOP LIST overruled the story about:
    for w in ["here", "with"] {
        assert!(seen.contains(&w.to_string()), "mini-Zork holds {w:?}: {seen:?}");
    }

    // …and `mailbox` is one the story stores under a DIFFERENT string, which is
    // the truncation bug pinned against a real Version 3 dictionary.
    let dict: Vec<String> = session.introspect().expect("a v3 story has an object tree").vocabulary();
    assert!(
        !dict.contains(&"mailbox".to_string()),
        "mini-Zork's dictionary keeps six Z-characters, so it cannot hold `mailbox`"
    );
    assert!(dict.contains(&"mailbo".to_string()), "it holds the six it keeps");

    // The falsifier: the old scrape offered words this story has never heard of.
    let before = stop_word_scrape(
        "West of House You are standing in an open field west of a white house, with a boarded \
         front door. There is a small mailbox here.",
    );
    let invented: Vec<&String> =
        before.iter().filter(|w| session.knows_word(w) == Some(false)).collect();
    assert!(
        !invented.is_empty(),
        "the old path offered words the parser rejects, or this case proves nothing"
    );
    for w in invented {
        assert!(!seen.contains(w), "{w:?} is not a word and is no longer offered");
    }
}

/// The Z-machine's splitting is the story's own, not ours: `split_like_parser`
/// answers, and it answers with the dictionary's declared separators.
#[test]
fn the_z_machine_lends_its_tokeniser() {
    let session = boot_minizork();
    let tokens = session
        .split_like_parser("Open the mailbox, then read the leaflet.")
        .expect("a Z-machine story always has a dictionary to tokenise with");
    assert!(tokens.contains(&"mailbox".to_string()), "{tokens:?}");
    assert!(
        tokens.contains(&",".to_string()) || tokens.contains(&".".to_string()),
        "a separator is a token of its own, exactly as `read` produces it: {tokens:?}"
    );
    // …and a separator is never offered as vocabulary, however real a word it is
    // to the parser.
    let words = story_words(&tokens, |w| session.knows_word(w).unwrap_or(false));
    assert!(!words.iter().any(|w| w == "," || w == "."), "{words:?}");
}

/// Prose is not a typed line, and `Token::text_pos` is a single byte. Feed the
/// tokeniser more than 255 bytes and every position past the first 255 wraps —
/// so the text goes in chunks that end on a space. A word in the tail must come
/// out whole and in the right place.
#[test]
fn long_prose_survives_the_single_byte_token_positions() {
    let session = boot_minizork();
    let filler = "the white house is west of the field. ".repeat(20); // ~740 bytes
    let text = format!("{filler}There is a small mailbox here.");
    assert!(text.len() > 3 * u8::MAX as usize, "the case needs prose a text buffer could not hold");

    let tokens = session.split_like_parser(&text).expect("a dictionary to tokenise with");
    assert!(tokens.contains(&"mailbox".to_string()), "the tail is still readable: {tokens:?}");
    assert!(
        tokens.iter().all(|t| !t.contains(' ')),
        "no token spans a space, which a wrapped position would produce"
    );
}

// ── Scott Adams, which has no object tree and needs this most ───────────────

/// The scrape is the *only* source the command band's *here* column has on an
/// engine with no `Introspect`, which is Glulx and Scott. It still answers, and
/// what it answers is the database's own vocabulary.
#[test]
fn a_scott_database_answers_from_its_own_word_lists() {
    let session = tiny_cave();
    assert!(session.introspect().is_none(), "a Scott database has no object tree to read");

    let mut state = state_with(&["> get lamp", "I'm in a cave. I can see a lamp here.", "OK"]);
    app::input::refresh_seen_words(&mut state, &session);
    let seen = state.seen_words.clone();

    let v = session.story_vocabulary().expect("a Scott database always has a vocabulary");
    assert!(!seen.is_empty(), "the fallback must still say something: {seen:?}");
    for w in &seen {
        assert!(v.knows(w), "{w:?} is not in tiny_cave's vocabulary: {seen:?}");
    }
    assert!(seen.contains(&"lamp".to_string()), "the thing in the room: {seen:?}");
    assert!(seen.contains(&"get".to_string()), "and the verb it was reached with: {seen:?}");
    assert!(!seen.contains(&"cave".to_string()) || v.knows("cave"), "nothing invented");
}

/// A Scott database truncates to its header's word length — three characters
/// here — so the scrape must reach an entry through the longer word the game
/// printed, exactly as the Z-machine does through its Z-characters.
#[test]
fn a_three_character_dictionary_still_reaches_the_printed_word() {
    let session = tiny_cave();
    let v = session.story_vocabulary().expect("a vocabulary");
    assert!(v.knows("score") && v.knows("scoreboard"), "three characters, cut the same both ways");

    let mut state = state_with(&["Your score is now 10."]);
    app::input::refresh_seen_words(&mut state, &session);
    assert!(
        state.seen_words.contains(&"score".to_string()),
        "the printed word reaches the stored key: {:?}",
        state.seen_words
    );
}

// ── What the two consumers see ──────────────────────────────────────────────

/// Tab completion ranks the story's recent words above the flat dictionary, and
/// that ranking is unchanged: the only thing SQ-1116 moved is WHICH words are in
/// the first group.
#[test]
fn completion_still_ranks_the_recent_words_first() {
    let session = boot_minizork();
    let mut state = state_with(&["There is a small mailbox here."]);
    state.dict_words =
        session.introspect().map(|i| i.vocabulary()).expect("a v3 story has a dictionary");
    app::input::refresh_seen_words(&mut state, &session);

    let hits = app::complete::suggest(&state.dict_words, &state.seen_words, "mail", 6);
    assert_eq!(
        hits.first().map(String::as_str),
        Some("mailbox"),
        "the word the room just used comes first, ahead of the stored `mailbo`: {hits:?}"
    );
    assert!(hits.contains(&"mailbo".to_string()), "the dictionary's own spelling still follows");
}

/// The band's *here* fallback reads the same list, so the two consumers cannot
/// drift apart — which is what the duplicated twenty-line scrape invited.
#[test]
fn the_band_fallback_and_completion_read_one_list() {
    let session = tiny_cave();
    let mut state = state_with(&["I'm in a cave. I can see a lamp here."]);
    app::input::refresh_seen_words(&mut state, &session);
    assert_eq!(app::input::scraped_seen_nouns(&state), state.seen_words);
}
