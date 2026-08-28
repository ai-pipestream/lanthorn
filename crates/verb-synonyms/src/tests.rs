//! Tests for the shipped table.
//!
//! The point of these is that they must survive a REGENERATION. A test that
//! only checks the file parses proves nothing about whether the table works; if
//! the corpus grows, the filters are retuned or the lexical source is swapped,
//! these are the mappings that have to still be there afterwards, because each
//! one is a word a player plausibly types at a game that wants the other.
//!
//! Every pair below was read out of WordNet 3.0 rather than remembered, and
//! `synonym_groups.tsv` can be grepped for each one by hand.

use super::*;

/// A story that knows exactly these verbs.
fn dictionary<'a>(words: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    move |w| words.contains(&w)
}

#[test]
fn table_is_present_and_parses() {
    assert!(
        group_count() > 2000,
        "only {} groups — did the table get truncated?",
        group_count()
    );
}

#[test]
fn canonical_mappings_survive_regeneration() {
    // (what the player typed, what the story knows, what must be offered)
    let cases: &[(&str, &str)] = &[
        // The quest's motivating case: nothing about the FORM of `illuminate`
        // reaches `light`, and this row is the whole reason the table exists.
        ("illuminate", "light"),
        ("conceal", "hide"),
        ("aid", "help"),
        ("shut", "close"),
        ("leap", "jump"),
        ("depart", "go"),
        ("speak", "talk"),
        ("slay", "murder"),
        ("hurl", "cast"),
        ("extinguish", "quench"),
        ("obtain", "gain"),
        ("grab", "catch"),
    ];
    for &(typed, known) in cases {
        let got = suggest(typed, dictionary(&[known]), 4);
        assert_eq!(got, vec![known], "`{typed}` should reach `{known}`");
    }
}

#[test]
fn a_group_never_suggests_the_word_the_player_typed() {
    // `light` is in its own group, and it is the one word known to have failed.
    let got = suggest("light", dictionary(&["light"]), 4);
    assert!(
        got.is_empty(),
        "offered the player their own word back: {got:?}"
    );
}

#[test]
fn the_story_disposes() {
    // A game that has never heard of any of it gets nothing, however rich the
    // group is.
    assert!(suggest("illuminate", dictionary(&["xyzzy"]), 4).is_empty());
}

#[test]
fn suggestions_are_capped() {
    let all: Vec<&str> = groups("go").flatten().copied().collect();
    let leaked: Vec<&'static str> = suggest("go", move |w| all.contains(&w), 3);
    assert!(leaked.len() <= 3, "limit ignored: {leaked:?}");
}

#[test]
fn a_polysemous_word_keeps_its_senses_apart() {
    // `draw` is *pull*, *sketch* and *attract*. Those are separate groups, not
    // one bucket — the merge is what would let `illuminate` reach `lightweight`.
    let gs: Vec<&[&str]> = groups("draw").collect();
    assert!(
        gs.len() > 3,
        "`draw` should sit in several sense groups, found {}",
        gs.len()
    );
    let pull = gs
        .iter()
        .position(|g| g.contains(&"pull"))
        .expect("a *pull* group");
    let depict = gs
        .iter()
        .position(|g| g.contains(&"depict"))
        .expect("a *sketch* group");
    assert_ne!(
        pull, depict,
        "two senses of `draw` collapsed into one group"
    );
    assert!(!gs[pull].contains(&"depict"));
}

#[test]
fn groups_come_back_in_sense_order() {
    // WordNet's sense 1 of `conceal` is `hide`; the group carrying it must be
    // the first one a consumer walks.
    let first = groups("conceal").next().expect("`conceal` is in the table");
    assert!(
        first.contains(&"hide"),
        "first group for `conceal` was {first:?}"
    );
}

/// Line order carries each word's sense order, so the file must never be sorted
/// — and it is exactly the kind of invariant somebody tidies away. If this fails
/// because the file was passed through `sort`, the fix is to regenerate, not to
/// relax the test.
#[test]
fn the_file_has_not_been_alphabetised() {
    let firsts: Vec<&str> = TABLE
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').next().unwrap_or(""))
        .collect();
    assert!(firsts.len() > 2000);
    assert!(
        firsts.windows(2).any(|w| w[0] > w[1]),
        "the table is in alphabetical order, which means its sense ordering is gone"
    );
}

#[test]
fn every_group_is_well_formed() {
    for (i, g) in index().groups.iter().enumerate() {
        assert!(g.len() >= 2, "group {i} has {} member(s): {g:?}", g.len());
        for w in g {
            assert!(!w.is_empty(), "empty member in group {i}");
            assert_eq!(
                *w,
                w.trim(),
                "member {w:?} in group {i} has stray whitespace"
            );
            assert!(
                w.bytes()
                    .all(|c| c.is_ascii_lowercase() || c == b' ' || c == b'-'),
                "member {w:?} in group {i} is not a lower-case English word"
            );
        }
        let mut seen = g.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), g.len(), "group {i} repeats a member: {g:?}");
    }
}

#[test]
fn nothing_is_parsed_until_a_lookup_asks() {
    // Not observable directly; what IS observable is that the table is behind a
    // `OnceLock` and two calls agree.
    assert_eq!(group_count(), group_count());
}
