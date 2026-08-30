//! SQ-1107: the momentary reveal — the words on screen the parser really knows.
//!
//! # The parser is the oracle
//!
//! Every claim below was settled by driving the real story under `zvm-cli` and
//! reading what its own parser said. Mini-Zork I r34/s871124, at the opening
//! screen (West of House, 0 moves in):
//!
//! ```text
//!   examine mailbox → The small mailbox is closed.
//!   examine house   → The house is a beautiful white colonial. …
//!   examine door    → The door is closed.
//!   examine field   → [I don't know the word "field".]
//!   examine window  → You can't see any window here!
//! ```
//!
//! Five nouns printed in three lines of prose, and the line the reveal draws
//! runs between two of the three groups, not three (SQ-1135):
//!
//! - **`mailbox`, `house`, `door` — words this story knows.** They light.
//! - **`window` — a word this story knows, for something elsewhere.** It would
//!   light too, wherever the prose printed it; nothing here does.
//! - **`field` — not a word at all.** The story has never heard it, so nothing
//!   lights it. It is also the word the room description leads with, which is
//!   exactly why the feature exists: the prose opens with a noun that does not
//!   exist.
//!
//! And one move later, the mailbox is a room away while its sentence is still on
//! screen:
//!
//! ```text
//!   north            → North of House …
//!   examine mailbox  → You can't see any mailbox here!
//! ```
//!
//! `There is a small mailbox here.` is still on screen and `mailbox` **keeps
//! lighting**, because the claim is about the dictionary and not about scope.
//! That case is [`a_word_the_story_knows_lights_wherever_the_thing_is`], and it
//! asserted the reverse until SQ-1135: the scope test made the engines with the
//! most introspection light the least, and a description naming something in the
//! next room — Arthur's crystal in the torque — lit nothing at all.
//!
//! # The specimens
//!
//! | fixture | release | turns in | what it shows |
//! |---|---|---|---|
//! | `crates/zvm/tests/fixtures/minizork.z3` | r34/s871124 | 0 | the whole path, in CI |
//! | `crates/zvm/tests/fixtures/minizork.z3` | r34/s871124 | 1 (`north`) | scope moving under old text |
//!
//! Mini-Zork is tracked, so every case here runs on CI; nothing skips.

use app::engine::Engine;
use app::reveal::Armed;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::fixture_paths::fixture_path;

// ── Booting and drawing ─────────────────────────────────────────────────────

/// Mini-Zork I, tracked in the checkout.
fn minizork() -> GameSession {
    let path = fixture_path("minizork-r34-s871124.z3");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("tracked at {}: {e}", path.display()));
    GameSession::new_with_trace(bytes, true, false, None, false, Vec::new(), None, None, Some((25, 80)))
        .expect("a Version 3 story should load and boot")
}

const AREA: Rect = Rect { x: 0, y: 0, width: 72, height: 20 };

/// The state a player is looking at: the story's own output in the transcript,
/// the Guiding Light on (this lives under its switch), and one frame drawn —
/// which is what fills the wrap cache and the viewport geometry the reveal reads.
fn screen(session: &mut GameSession) -> AppState {
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.guidance = true;
    for line in session.take_transcript().split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    draw(&state);
    state
}

/// Render one frame into a throwaway buffer, for its side effects on the wrap
/// cache and `transcript_geom`.
fn draw(state: &AppState) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    app::render::transcript::render_transcript(
        &app::engine::StatusModel::HostManaged,
        None,
        state,
        AREA,
        &mut buf,
        None,
    );
    buf
}

/// Every cell of the drawn frame that the reveal lit, as `(row, text)`.
fn lit_rows(buf: &Buffer) -> Vec<(u16, String)> {
    (AREA.y..AREA.bottom())
        .filter_map(|y| {
            let s: String = (AREA.x..AREA.right())
                .map(|x| {
                    let c = buf.cell((x, y)).unwrap();
                    if c.modifier.contains(Modifier::UNDERLINED) {
                        c.symbol().chars().next().unwrap_or(' ')
                    } else {
                        ' '
                    }
                })
                .collect();
            (!s.trim().is_empty()).then(|| (y, s.trim_end().to_string()))
        })
        .collect()
}

/// The whole drawn frame, for a failure message worth reading.
fn frame(buf: &Buffer) -> String {
    (AREA.y..AREA.bottom())
        .map(|y| {
            let s: String =
                (AREA.x..AREA.right()).map(|x| buf.cell((x, y)).unwrap().symbol()).collect();
            format!("\n  |{}|", s.trim_end())
        })
        .collect()
}

fn words(state: &AppState) -> Vec<String> {
    state.reveal.as_ref().map(|r| r.words.iter().cloned().collect()).unwrap_or_default()
}

// ── The reveal ──────────────────────────────────────────────────────────────

/// The opening screen, lit. The reveal's question is *"does this story know the
/// word?"* — the dictionary's answer, with no scope walk in it (SQ-1135).
#[test]
fn the_opening_screen_lights_the_words_the_story_knows() {
    let mut session = minizork();
    let mut state = screen(&mut session);

    let armed = app::reveal::arm(&mut state, &session);
    assert_eq!(armed, Armed::Lit { words: words(&state).len() });

    let lit = words(&state);
    println!("lit: {lit:?}");
    for here in ["mailbox", "house", "door"] {
        assert!(lit.contains(&here.to_string()), "{here:?} is here and must light: {lit:?}");
    }
    // `field` is not in the dictionary at all — the parser answers
    // `[I don't know the word "field".]` — and it is the noun the description
    // opens with. The dictionary IS the test, so this is what it excludes.
    assert!(!lit.contains(&"field".to_string()), "the story has never heard of `field`: {lit:?}");
    // `window` is in the dictionary (Mini-Zork's kitchen window) and is not on
    // this screen. The viewport is still the bound: a reveal lights words the
    // story PRINTED, never the dictionary at large.
    assert!(!lit.contains(&"window".to_string()), "nothing printed it here: {lit:?}");

    // …and it reaches the screen. The lit words are drawn underlined
    // (`transcript_reveal`), over the story's own prose, without moving it.
    let buf = draw(&state);
    let painted = lit_rows(&buf);
    println!("the screen:{}", frame(&buf));
    println!("what lit:{}", painted.iter().map(|(_, s)| format!("\n  |{s}|")).collect::<String>());
    let all: String = painted.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" ");
    for here in ["mailbox", "house", "door"] {
        assert!(all.contains(here), "{here:?} is not underlined on screen:{}", frame(&buf));
    }
    assert!(!all.contains("field"), "`field` must not be underlined:{}", frame(&buf));
}

/// **A verb never lights.** The verb panel answers "what can I do"; this answers
/// "what does the story know about". `open` and `take` are all over Mini-Zork's
/// grammar and its opening prose says `open field` — lighting the verb would
/// leave the prose saying nothing at all.
///
/// The compass words are NOT part of this claim, and never were a filter's to
/// make: Mini-Zork files `west` with the DESC bit, exactly as it files `white`
/// — see `the_reveal_inherits_the_dictionary_and_says_so`.
#[test]
fn verbs_do_not_light() {
    let mut session = minizork();
    let vocab = <GameSession as Engine>::story_vocabulary(&session).expect("a readable dictionary");
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    for verb in ["open", "take"] {
        let r = vocab.roles(verb).expect("in the dictionary");
        assert!(
            !r.noun && !r.adjective,
            "{verb:?} must be filed as a verb and nothing else, or this proves nothing: {r:?}"
        );
        assert!(!lit.contains(&verb.to_string()), "{verb:?} is a verb: {lit:?}");
    }
}

/// **A word for something in another room lights, and that is intended**
/// (SQ-1135).
///
/// After `north`, `There is a small mailbox here.` is still on screen — the
/// player can read it — and the parser now answers `You can't see any mailbox
/// here!`. The word still lights, because the claim the highlight makes is about
/// the DICTIONARY: this story knows `mailbox`. Lighting a word the story has
/// already printed on the player's own screen tells them nothing they were not
/// told.
///
/// This case used to assert the opposite, back when the reveal walked the object
/// tree wherever it could. That is the inversion SQ-1135 removes, and the reason
/// Arthur's crystal reaches a player at all: the description naming it is right
/// there on screen, and a scope-tested reveal lit nothing in it.
#[test]
fn a_word_the_story_knows_lights_wherever_the_thing_is() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    assert!(words(&state).contains(&"mailbox".to_string()), "lit at West of House");

    session.submit("north");
    for line in session.take_transcript().split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    draw(&state);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    println!("after `north`, lit: {lit:?}");

    assert!(
        state.transcript.iter().any(|l| l.contains("small mailbox")),
        "the sentence naming the mailbox must still be on screen, or this proves nothing",
    );
    assert!(lit.contains(&"mailbox".to_string()), "the story still knows the word: {lit:?}");
    assert!(lit.contains(&"house".to_string()), "…and the house with it: {lit:?}");
}

// ── Momentary ───────────────────────────────────────────────────────────────

/// One press lights it; the next keystroke, the next turn or the hold puts it
/// out. There is no fourth way and no way to leave it on.
#[test]
fn it_goes_out_on_a_keystroke_on_a_turn_and_on_the_clock() {
    let mut session = minizork();
    let mut state = screen(&mut session);

    // The keystroke path (`main.rs` clears ahead of every dispatch arm).
    app::reveal::arm(&mut state, &session);
    assert!(state.reveal.is_some());
    assert!(app::reveal::clear(&mut state), "a lit reveal goes out");
    assert!(state.reveal.is_none());
    assert!(!app::reveal::clear(&mut state), "…and clearing nothing changes nothing");

    // The turn path. `begin_turn` is what every finished command runs through,
    // including one no key was pressed for (a timed read firing).
    app::reveal::arm(&mut state, &session);
    state.begin_turn();
    assert!(state.reveal.is_none(), "a turn ends the moment the reveal was about");

    // The clock. `expire` is the loop's tick; a reveal whose hold has passed is
    // dropped there and nowhere else, so a player who presses and then does
    // nothing still watches it go out.
    app::reveal::arm(&mut state, &session);
    assert!(!app::reveal::expire(&mut state), "not yet — the hold has not passed");
    state.reveal.as_mut().unwrap().until = std::time::Instant::now();
    assert!(app::reveal::expire(&mut state), "the hold passed");
    assert!(state.reveal.is_none());
}

/// It lives under the Guiding Light's switch, like every other assist — and says
/// so, instead of appearing to be broken.
#[test]
fn with_the_guiding_light_out_it_does_nothing_and_admits_it() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    state.config.guidance = false;

    assert_eq!(app::reveal::arm(&mut state, &session), Armed::GuidanceOff);
    assert!(state.reveal.is_none(), "nothing lights with the light out");

    state.config.guidance = true;
    assert!(matches!(app::reveal::arm(&mut state, &session), Armed::Lit { .. }));
}

/// Before a frame has been drawn there is no viewport to read, and the reveal
/// says that rather than lighting the whole scrollback. This is also the v6
/// RASTER answer: raster's text never passes through the cell wrap cache, so it
/// takes this branch and the feature is honestly absent there rather than
/// silently wrong.
#[test]
fn with_no_drawn_text_there_is_nothing_to_light() {
    let mut session = minizork();
    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.config.guidance = true;
    // The transcript is full; the SCREEN is not, because nothing has drawn yet.
    for line in session.take_transcript().split('\n') {
        state.push_transcript_kind(line, TranscriptKind::Story);
    }
    assert_eq!(app::reveal::arm(&mut state, &session), Armed::NoText);
    assert!(state.reveal.is_none());
}

/// The viewport is the answer to "how far back?", so a reveal only ever knows
/// about words that were on screen when it was lit.
#[test]
fn only_what_is_on_screen_is_considered() {
    let mut session = minizork();
    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let at_opening = words(&state);
    assert!(at_opening.contains(&"mailbox".to_string()));

    // Push enough plain lines to scroll the opening description off a 20-row
    // pane, then draw so the viewport moves with them.
    for _ in 0..40 {
        state.push_transcript_kind("Time passes.", TranscriptKind::Story);
    }
    draw(&state);
    app::reveal::arm(&mut state, &session);
    let scrolled = words(&state);
    println!("scrolled off: {scrolled:?}");
    assert!(
        !scrolled.contains(&"mailbox".to_string()),
        "the mailbox is still in scope and still in the transcript — but not on screen: {scrolled:?}",
    );
}

// ── The claim, and the label on it ──────────────────────────────────────────

/// **The reveal inherits the dictionary's own idea of what a word is**, and
/// there is no filter that could rescue it — which is why it says what it is
/// rather than pretending to a stronger claim.
///
/// `west` on the opening screen is not a slip on our part; it is what
/// Mini-Zork's own dictionary says. Its flag byte is `0x33`, which sets the DESC
/// bit — the same bit `white` and `boarded` carry — so nothing in the story
/// distinguishes the compass from an adjective, and `north` and `south` (`0x13`)
/// are not filed as either and do not light at all.
///
/// What the filter DOES drop is the buzzword bit (`$04`): `the`, `a`, `please`
/// and their kin, which every story files as words and no player wants lit.
#[test]
fn the_reveal_inherits_the_dictionary_and_says_so() {
    let mut session = minizork();
    let vocab = <GameSession as Engine>::story_vocabulary(&session).expect("a readable dictionary");
    for w in ["west", "north", "white", "boarded", "mailbox", "the"] {
        println!("{w}: {:?}", vocab.roles(w));
    }
    // The compass word and the colour are indistinguishable in this dictionary.
    let west = vocab.roles("west").expect("in the dictionary");
    let white = vocab.roles("white").expect("in the dictionary");
    assert_eq!(
        (west.noun, west.adjective, west.special),
        (white.noun, white.adjective, white.special),
        "Mini-Zork files `west` exactly as it files `white`, so no part-of-speech \
         filter can tell them apart — which is what the caveat is for",
    );

    let mut state = screen(&mut session);
    app::reveal::arm(&mut state, &session);
    let lit = words(&state);
    // Both, or neither: the dictionary cannot separate them, so neither can this.
    assert_eq!(
        lit.contains(&"west".to_string()),
        lit.contains(&"white".to_string()),
        "the two words this dictionary files identically must fare identically: {lit:?}",
    );
    // The buzzword bit is what the filter really removes, and Mini-Zork's
    // opening prose is full of `the` and `a`.
    for buzz in ["the", "a"] {
        assert!(!lit.contains(&buzz.to_string()), "{buzz:?} carries the buzzword bit: {lit:?}");
    }
}

/// The reveal states its claim out loud (SQ-1135). It cannot tell "implemented
/// HERE" from "implemented SOMEWHERE" — by design now, not by limitation — and
/// the legend says so rather than leaving a player to infer the stronger reading.
#[test]
fn the_reveal_admits_what_it_cannot_tell_apart() {
    println!("the reveal says: {}", app::reveal::CAVEAT);
    assert!(
        app::reveal::CAVEAT.contains("not necessarily"),
        "it has to say what it cannot promise: {:?}",
        app::reveal::CAVEAT,
    );
}
