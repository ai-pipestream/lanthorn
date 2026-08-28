//! Tests for the generator's readers and for the committed harvest.
//!
//! The shipped table's own guarantees are tested in `verb-synonyms`; what is
//! tested here is that the two source formats are read the way their
//! documentation says, using fixtures small enough to check by eye against the
//! real files.

use crate::build::{build, IfVerb, Params, Report};
use crate::sources::{Frequency, WordNet};

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("verbsyn-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// Three real lines from `index.verb`, `data.verb` and `verb.exc`, transcribed
/// byte for byte — the `light` synset that carries `illuminate`, plus the
/// `ignite` one, plus one hypernym for the pointer walk.
fn wordnet_fixture() -> WordNet {
    let d = scratch("wn");
    std::fs::write(
        d.join("index.verb"),
        "  1 This software and database is being provided to you\n\
         light v 2 1 @ 2 1 00291873 02759614  \n\
         illuminate v 1 1 @ 1 1 00291873  \n\
         ignite v 1 1 @ 1 0 02759614  \n\
         sprint v 1 1 @ 1 0 02058590  \n\
         run v 1 1 @ 1 1 02091410  \n",
    )
    .unwrap();
    std::fs::write(
        d.join("data.verb"),
        "  1 This software and database is being provided to you\n\
         00291873 30 v 05 light 0 illume 0 illumine 0 light_up 0 illuminate 3 001 @ 00280930 v 0000 | make lighter\n\
         02759614 43 v 02 ignite 0 light 0 001 @ 02762468 v 0000 | cause to start burning\n\
         02058590 38 v 01 sprint 0 001 @ 02091410 v 0000 | run very fast\n\
         02091410 38 v 02 run 0 hurry 0 000 | move fast\n",
    )
    .unwrap();
    std::fs::write(d.join("verb.exc"), "lit light\nran run\nsaw see\n").unwrap();
    WordNet::load(&d).expect("fixture loads")
}

fn frequency_fixture() -> Frequency {
    let d = scratch("freq");
    let p = d.join("frq.txt");
    std::fs::write(
        &p,
        "----- 1 -----\nup\nlight\n    lighted, lit\nrun\n    ran\n\
         ----- 2 -----\nignite\nsprint\nhurry\n\
         ----- 9 -----\nilluminate\n(April)\nfoo*\n",
    )
    .unwrap();
    Frequency::load(&p).expect("fixture loads")
}

#[test]
fn wordnet_synsets_carry_their_words_and_verb_pointers() {
    let wn = wordnet_fixture();
    assert_eq!(wn.senses["light"], vec![291873, 2759614]);
    assert_eq!(
        wn.words_of(291873),
        ["light", "illume", "illumine", "light up", "illuminate"],
        "underscores must become spaces and the licence header must be skipped"
    );
    assert_eq!(wn.synsets[&2058590].pointers, [("@".to_string(), 2091410)]);
    assert_eq!(wn.exceptions["lit"], "light");
}

#[test]
fn frequency_bands_and_lemmatisation() {
    let f = frequency_fixture();
    assert_eq!(f.band["light"], 1);
    assert_eq!(f.band["illuminate"], 9);
    assert_eq!(
        f.lemma_of["lit"], "light",
        "indented forms belong to the headword above"
    );
    assert_eq!(f.lemma_of["light"], "light", "a headword is its own lemma");
    assert!(
        !f.band.contains_key("April"),
        "parenthesised entries are not words"
    );
    assert_eq!(
        f.top(1),
        ["up", "light", "run"],
        "band order, not alphabetical"
    );
}

#[test]
fn a_synset_becomes_a_group_and_the_rare_words_are_filtered_out() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &wn, &freq, &p, &mut r);
    let light = groups
        .iter()
        .find(|g| g.contains(&"illuminate".to_string()))
        .expect("a group");
    assert_eq!(light[0], "light", "the IF verb leads the line");
    assert!(light.contains(&"light up".to_string()));
    assert!(
        !light.contains(&"illume".to_string()) && !light.contains(&"illumine".to_string()),
        "`illume` is not in the frequency list and must be filtered out: {light:?}"
    );
}

#[test]
fn the_gap_fill_only_rescues_a_synset_no_story_can_match() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    // `run` is the IF verb; `sprint` sits alone in a synset of its own, whose
    // hypernym is `run`.
    let verbs = vec![IfVerb {
        emit: "run".into(),
        lemma: "run".into(),
        stories: 100,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &wn, &freq, &p, &mut r);
    assert!(
        groups
            .iter()
            .any(|g| g.contains(&"sprint".to_string()) && g.contains(&"run".to_string())),
        "sprint should reach run through its hypernym: {groups:?}"
    );
    let mut off = Params {
        band_cap: 9,
        gap_fill: false,
        ..Params::default()
    };
    off.gap_fill = false;
    let mut r2 = Report::default();
    let plain = build(&verbs, &wn, &freq, &off, &mut r2);
    assert!(
        !plain.iter().any(|g| g.contains(&"sprint".to_string())),
        "with --no-gap-fill the table is pure synsets"
    );
}

/// The committed harvest is what makes `build` reproducible without a corpus of
/// commercial game files, so its shape is worth pinning.
#[test]
fn the_committed_harvest_is_well_formed() {
    let text = include_str!("../if_verbs.tsv");
    let mut n = 0;
    let mut previous = String::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 3, "expected spelling/stories/lemma: {line:?}");
        assert!(
            f[0].bytes()
                .all(|c| c.is_ascii_lowercase() || c == b' ' || c == b'-'),
            "not a lower-case English word: {:?}",
            f[0]
        );
        assert!(
            f[1].parse::<usize>().is_ok_and(|n| n > 0),
            "bad story count: {line:?}"
        );
        assert!(
            f[0] > previous.as_str(),
            "the harvest must be sorted: {line:?}"
        );
        previous = f[0].to_string();
        n += 1;
    }
    assert!(
        n > 2000,
        "only {n} spellings — did the harvest run against an empty corpus?"
    );
    for expected in ["take", "drop", "open", "light", "examine", "turn on"] {
        assert!(
            text.lines()
                .any(|l| l.starts_with(&format!("{expected}\t"))),
            "`{expected}` is missing from the harvest"
        );
    }
}
