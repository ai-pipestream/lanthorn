//! What a Scott Adams item can be CALLED — SQ-1118.
//!
//! There is nothing to derive here and nothing to detect: a Scott database has
//! no properties and no per-object word arrays, because the format never had
//! anywhere to put them. It has a flat noun table, and each item the player can
//! handle carries a `/NOUN/` marker in its description naming its one noun. The
//! words that refer to an item are that noun plus the `*`-prefixed synonyms
//! that follow it — which is precisely what the two-word parser resolves
//! through `match_noun`, so this reader and the parser cannot disagree.
//!
//! `tiny_cave.dat` is committed, so CI sees the whole of this file except the
//! Adventureland case.

use std::path::PathBuf;

use scott::Database;

const TINY_CAVE: &str = include_str!("tiny_cave.dat");

#[test]
fn an_item_answers_with_its_noun_and_the_synonyms_that_follow_it() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");

    let lamp = db.item_words(9).expect("item 9 carries a /LAMP/ marker");
    assert_eq!(lamp.printed_name, "a brass lamp");
    assert_eq!(lamp.words, ["lamp"]);
    // No property table exists to have read this out of, and saying so is
    // better than inventing a number.
    assert_eq!(lamp.property, None);
    // Scott truncates its vocabulary to the header's word length, so the
    // player's whole word still matches the three letters the file holds.
    assert_eq!(lamp.truncated_at, Some(3));
    assert!(lamp.refers_to("lamp") && lamp.refers_to("LAMP"));

    let idol = db.item_words(1).expect("item 1 carries a marker");
    assert_eq!(idol.words, ["idol"]);
    assert_eq!(idol.id, 1);
}

#[test]
fn an_item_with_no_marker_has_no_word_and_is_refused() {
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    let unnamed: Vec<usize> = (0..db.items.len()).filter(|&i| db.item_words(i).is_none()).collect();
    assert!(!unnamed.is_empty(), "the fixture has scenery items with no /NOUN/");
    for i in &unnamed {
        assert!(
            db.items[*i].auto_noun.is_none(),
            "item {i} was refused but does carry a marker"
        );
    }
    // Past the end of the item table is a refusal, not a panic.
    assert!(db.item_words(db.items.len()).is_none());
}

#[test]
fn every_word_a_reader_returns_is_one_the_parser_resolves_to_that_item() {
    // The reader and the parser must not be able to disagree: every word must
    // resolve, through the game's own matcher, to the noun number the item was
    // reached by.
    let db = Database::parse(TINY_CAVE).expect("tiny_cave.dat parses");
    for i in 0..db.items.len() {
        let Some(o) = db.item_words(i) else { continue };
        let canonical = db.match_noun(db.items[i].auto_noun.as_deref().unwrap()).unwrap();
        for w in &o.words {
            assert_eq!(db.match_noun(w), Some(canonical), "item {i}, word {w:?}");
        }
    }
}

#[test]
fn adventureland_answers_its_real_nouns_and_their_synonyms() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/adv01.dat");
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return;
    }
    let db = Database::parse(&std::fs::read_to_string(&path).unwrap()).expect("adv01.dat parses");
    assert_eq!(db.word_length, 3);

    let axe = db.item_words(11).expect("the rusty axe");
    assert!(axe.printed_name.starts_with("Rusty axe"));
    assert_eq!(axe.words, ["axe", "ax"]);
    assert!(axe.refers_to("axe") && axe.refers_to("ax"));

    // "bottle" and "container" are two spellings of one noun, and the item
    // answers both — which a reader working from the printed name alone could
    // never know.
    let bottle = db.item_words(13).expect("the empty bottle");
    assert_eq!(bottle.printed_name, "Empty bottle");
    assert_eq!(bottle.words, ["bot", "con"]);
    assert!(bottle.refers_to("bottle") && bottle.refers_to("container"));

    let mud = db.item_words(7).expect("the mud");
    assert_eq!(mud.words, ["mud", "med"]);
}
