//! What an object can be CALLED, read out of the story — SQ-1118.
//!
//! `objects::short_name` answers what a game PRINTS for a thing.
//! [`zvm::objects::ParseNames`] answers what a player may TYPE for it, and the
//! two sets barely overlap: Zork I prints "brass lantern" and accepts `lamp`,
//! `lanter` and `light`, none of which is in the printed name.
//!
//! **The oracle for every word below is the game's own parser, not this
//! reader.** Each was confirmed by driving `zvm-cli` through the real story and
//! watching the command land on the right object. From Zork I r88, in the
//! Living Room after `open mailbox` at West of House:
//!
//! ```text
//!   take advertisement → Taken.      drop booklet   → Dropped.
//!   take mail          → Taken.      take glamdring → Taken.
//!   drop orcrist       → Dropped.    take blade     → Taken.
//!   take light         → Taken.      drop lantern   → Dropped.
//!   take lamp          → Taken.
//!   inventory          → A brass lantern / A sword / A leaflet
//! ```
//!
//! The cases against `crates/zvm/tests/fixtures/` are the ones CI can see;
//! `stories/` is gitignored commercial media and those skip vacuously.

use zvm::memory::Memory;
use zvm::objects::{self, ParseNames};

fn fixture(name: &str) -> Memory {
    Memory::new(zvm::fixtures::load(name).expect("committed fixture")).unwrap()
}

/// A gitignored commercial story, or `None` so the case can skip.
fn story(name: &str) -> Option<Memory> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(std::fs::read(&path).ok()?).ok()
}

fn words_of(pn: &ParseNames, mem: &Memory, obj: u16) -> Vec<String> {
    pn.of(mem, obj).unwrap_or_else(|| panic!("object {obj} has no parse names")).words
}

// ── Committed fixtures: what CI sees ─────────────────────────────────────────

#[test]
fn an_infocom_story_keeps_its_words_in_a_property_of_its_own_choosing() {
    let mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).expect("the Zork I sampler has parse names");

    // NOT property 1. ZIL numbers `SYNONYM` per game and this one landed on 17.
    assert_eq!(pn.property(), 17);

    let lantern = pn.of(&mem, 102).unwrap();
    assert_eq!(lantern.printed_name, "brass lantern");
    assert_eq!(lantern.words, ["lamp", "lanter", "light"]);
    assert_eq!(lantern.property, Some(17));
    // v1-3 keys hold six Z-characters (ZMSD §13.3), so the player's whole word
    // still matches the story's truncation of it.
    assert_eq!(lantern.truncated_at, Some(6));
    assert!(lantern.refers_to("lantern") && lantern.refers_to("LAMP"));
    assert!(!lantern.refers_to("sword"));

    assert_eq!(words_of(&pn, &mem, 89), ["leafle", "mail"]);
    assert_eq!(words_of(&pn, &mem, 164), ["sword", "blade"]);
    assert_eq!(words_of(&pn, &mem, 167), ["mailbo", "box"]);
    assert_eq!(pn.of(&mem, 167).unwrap().printed_name, "small mailbox");

    // The printed name is not the word list, which is the whole point: `lamp`
    // and `light` appear nowhere in "brass lantern", and `box` appears nowhere
    // in "small mailbox".
    assert!(lantern.words.iter().filter(|w| !lantern.printed_name.contains(w.as_str())).count() >= 2);
    let mailbox = pn.of(&mem, 167).unwrap();
    assert!(!mailbox.printed_name.contains("box "));
    assert!(mailbox.refers_to("box"));

    // And the reader can be asked the other way round.
    let found = pn.find(&mem, "glamdring");
    assert!(found.is_none(), "the sampler's sword has no elvish names: {found:?}");
    assert_eq!(pn.find(&mem, "mailbox").unwrap().id, 167);
}

#[test]
fn an_inform_story_uses_property_one_and_says_so() {
    let mem = fixture("praxix.z5");
    let pn = ParseNames::detect(&mem).expect("praxix has parse names");
    assert_eq!(pn.property(), objects::INFORM_NAME_PROPERTY);

    let look = pn.of(&mem, 6).unwrap();
    assert_eq!(look.words, ["look", "l", "help", "?"]);
    // v4+ keys hold nine Z-characters (§13.4).
    assert_eq!(look.truncated_at, Some(9));
    assert_eq!(pn.of(&mem, 10).unwrap().words, ["comarith", "comparith"]);
}

#[test]
fn a_story_with_no_object_words_is_refused_rather_than_guessed_at() {
    // czech is a VM conformance suite: ten objects, no vocabulary, no game.
    let mem = fixture("czech.z5");
    assert!(ParseNames::detect(&mem).is_none());
}

// ── Falsification: it must refuse, not invent ────────────────────────────────

#[test]
fn asking_the_wrong_property_yields_nothing_instead_of_plausible_words() {
    let mem = fixture("minizork.z3");
    // Property 1 is Inform's answer and is wrong here; the sampler's property 1
    // holds something else entirely on the objects that have it at all.
    let wrong = ParseNames::with_property(&mem, objects::INFORM_NAME_PROPERTY);
    let wrong = wrong.expect("the reader still builds; it is the objects that refuse");
    assert_eq!(wrong.property(), 1);
    assert_eq!(
        wrong.all(&mem).len(),
        0,
        "reading the wrong property must produce no words at all, not fewer words"
    );
    assert!(wrong.of(&mem, 102).is_none());
}

#[test]
fn one_corrupted_entry_refuses_the_whole_object() {
    let mut mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).unwrap();
    assert_eq!(words_of(&pn, &mem, 102).len(), 3);

    // Point the lantern's second word at an address that is not a dictionary
    // entry. A reader that decoded whatever is there would return three
    // plausible-looking words, one of them nonsense.
    let data = objects::get_prop_addr(&mem, 102, pn.property()) as u32;
    mem.write_word(data + 2, 0x0123);
    assert!(
        pn.of(&mem, 102).is_none(),
        "one entry that is not a dictionary word must refuse the object"
    );
    // Its neighbours are untouched, so the refusal is the object's, not the
    // reader's giving up.
    assert_eq!(words_of(&pn, &mem, 167), ["mailbo", "box"]);
}

#[test]
fn an_object_without_the_property_answers_none() {
    let mem = fixture("minizork.z3");
    let pn = ParseNames::detect(&mem).unwrap();
    // Object 0 is the null object and object 2 is one of the many that carry no
    // synonym list; neither may be answered with a guess.
    assert!(pn.of(&mem, 0).is_none());
    let without: Vec<u16> = (1..=objects::object_count(&mem))
        .filter(|&o| pn.of(&mem, o).is_none())
        .collect();
    assert!(!without.is_empty(), "the sampler has objects with no synonyms");
    assert_eq!(pn.all(&mem).len(), 105);
}

#[test]
fn the_object_count_stops_where_the_property_tables_begin() {
    let mem = fixture("minizork.z3");
    let n = objects::object_count(&mem);
    assert_eq!(n, 179);
    // The last object is real and the one past it is not addressable as an
    // entry, which is what the count means.
    assert!(!objects::short_name(&mem, n).is_empty());
}

// ── Real commercial media (skips vacuously without `stories/`) ───────────────

#[test]
fn zork_one_answers_the_words_its_parser_accepts() {
    let Some(mem) = story("zork1-r88-s840726.z3") else { return };
    let pn = ParseNames::detect(&mem).expect("Zork I has parse names");
    // Zork I's SYNONYM is 18 where the sampler's is 17 — the same game, a
    // different compilation, a different property number.
    assert_eq!(pn.property(), 18);

    let sword = pn.of(&mem, 110).unwrap();
    assert_eq!(sword.printed_name, "sword");
    assert_eq!(sword.words, ["sword", "orcris", "glamdr", "blade"]);
    assert!(sword.refers_to("glamdring") && sword.refers_to("orcrist"));

    assert_eq!(words_of(&pn, &mem, 164), ["lamp", "lanter", "light"]);
    assert_eq!(words_of(&pn, &mem, 161), ["advert", "leafle", "bookle", "mail"]);
    assert_eq!(words_of(&pn, &mem, 160), ["mailbo", "box"]);
    assert_eq!(pn.all(&mem).len(), 136);
}

#[test]
fn a_version_six_story_is_settled_by_containment_where_counts_are_not() {
    // Zork Zero's word arrays lead 432 objects to 306, which no margin
    // separates; its V6 dictionary flags mark 24 of 1624 words a noun, which no
    // part-of-speech filter separates either. The adjectives being a strict
    // subset of the nouns is what settles it.
    let Some(mem) = story("zork0-r393-s890714.z6") else { return };
    let pn = ParseNames::detect(&mem).expect("Zork Zero has parse names");
    assert_eq!(pn.property(), 51);
    let hangar = pn.of(&mem, 5).unwrap();
    assert_eq!(hangar.printed_name, "Dirigible Hangar");
    assert_eq!(hangar.words, ["hangar"]);
    assert_eq!(pn.all(&mem).len(), 432);
}

#[test]
fn a_runner_up_that_is_not_the_adjectives_is_beaten_on_count_instead() {
    // Planetfall's property 14 is a word array on twelve objects sharing just
    // one with property 19's 146, so containment cannot settle it and the
    // margin does.
    let Some(mem) = story("planetfall-r37-s851003.z3") else { return };
    let pn = ParseNames::detect(&mem).expect("Planetfall has parse names");
    assert_eq!(pn.property(), 19);
    assert_eq!(pn.all(&mem).len(), 146);
}

#[test]
fn stories_with_no_vocabulary_of_their_own_are_refused() {
    // Journey is menu-driven and Scopa is a card game: neither has a parser,
    // and neither keeps word arrays on its objects.
    for name in ["journey-r83-s890706.z6", "scopa.z6"] {
        let Some(mem) = story(name) else { continue };
        assert!(ParseNames::detect(&mem).is_none(), "{name} should be refused");
    }
    // `advent.z8` boots and plays, and tokenises with its own word table: its
    // Z-machine dictionary declares zero entries, so there is nothing for a
    // parse-name property to point at.
    if let Some(mem) = story("advent.z8") {
        assert!(ParseNames::detect(&mem).is_none());
    }
}

#[test]
fn every_infocom_story_here_answers_with_a_property_of_its_own() {
    // The numbers move from game to game, which is the reason detection exists.
    let expected = [
        ("zork2-r48-s840904.z3", 17u8),
        ("zork3-r17-s840727.z3", 17),
        ("seastalker-r16-s850603.z3", 14),
        ("hitchhiker-r59-s851108.z3", 31),
        ("plunderedhearts-r26-s870730.z3", 29),
        ("nordandbert-r19-s870722.z4", 63),
        ("amfv-r77-s850814.z4", 52),
        ("shogun-r322-s890706.z6", 45),
    ];
    let mut checked = 0;
    for (name, property) in expected {
        let Some(mem) = story(name) else { continue };
        let pn = ParseNames::detect(&mem).unwrap_or_else(|| panic!("{name} has parse names"));
        assert_eq!(pn.property(), property, "{name}");
        assert!(pn.all(&mem).len() > 100, "{name}");
        checked += 1;
    }
    if checked == 0 {
        eprintln!("SKIP: no Infocom media present");
    }
}
