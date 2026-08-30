//! Tests for the generator's readers and for the committed harvest.
//!
//! The shipped table's own guarantees are tested in `verb-synonyms`; what is
//! tested here is that the two source formats are read the way their
//! documentation says, using fixtures small enough to check by eye against the
//! real files.

use crate::build::{build, GameGroup, IfVerb, Params, Report};
use crate::sources::{Frequency, WordNet};

/// A directory this CALL alone owns.
///
/// Keyed on a counter as well as the pid, and that is the whole point: under
/// `cargo nextest run` every test is its own process, so a pid alone is already
/// unique and the bug below cannot happen. Under `cargo test` — which is what CI
/// runs — one binary's tests share a process and run on threads, so a pid-only
/// key gave every caller of [`wordnet_fixture`] the SAME directory. `fs::write`
/// truncates, so one thread read `index.verb` while another was rewriting it,
/// `WordNet::load` came back empty, `build` returned no groups, and the
/// assertion failed on a fixture that was correct.
///
/// Invisible to the local gate by construction — nextest's process-per-test
/// makes a shared-state race structurally unobservable, and only `cargo test`
/// can see it.
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NTH: AtomicUsize = AtomicUsize::new(0);
    let nth = NTH.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir()
        .join(format!("verbsyn-test-{name}-{}-{nth}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
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
    std::fs::write(d.join("verb.exc"), "lit light\nran run\nsaw see\nsinging sing singe\n")
        .unwrap();
    std::fs::write(d.join("noun.exc"), "mice mouse\naxes ax axis\nis is\n").unwrap();
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
    assert_eq!(wn.exceptions["lit"], ["light"]);
}

/// The two exception lists are read the same way and kept APART.
///
/// Apart because `main.rs`'s inflected-IF-verb measurement counts `exceptions`
/// and would start meaning something else if nouns joined it, and because a
/// spelling can inflect two parts of speech to different lemmas — one map would
/// answer whichever file was read second.
#[test]
fn the_noun_and_verb_exception_lists_stay_apart() {
    let wn = wordnet_fixture();
    assert_eq!(wn.noun_exceptions["mice"], ["mouse"]);
    assert!(!wn.exceptions.contains_key("mice"), "a noun is not in the verb map");
    assert_eq!(
        wn.exceptions["singing"],
        ["sing", "singe"],
        "WordNet puts two bases on some lines and neither may be dropped"
    );
    assert_eq!(wn.noun_exceptions["axes"], ["ax", "axis"]);
    assert_eq!(
        wn.noun_exceptions["is"],
        ["is"],
        "WordNet's lines are kept verbatim — `is is` is it saying `is` inflects nothing, \
         and it is the TABLE WRITER that drops a self-pair, not the reader"
    );
}

/// `noun.exc` is optional: the DB-only WordNet tarball carries no exception
/// lists at all, and a `dict/` without this one still loads for everything else.
#[test]
fn a_dict_without_noun_exc_still_loads() {
    let d = scratch("wn-no-noun");
    std::fs::write(d.join("index.verb"), "light v 1 1 @ 1 1 00291873  \n").unwrap();
    std::fs::write(d.join("data.verb"), "00291873 30 v 01 light 0 000 | make lighter\n").unwrap();
    std::fs::write(d.join("verb.exc"), "lit light\n").unwrap();
    let wn = WordNet::load(&d).expect("a dict with no noun.exc still loads");
    assert_eq!(wn.exceptions["lit"], ["light"]);
    assert!(wn.noun_exceptions.is_empty());
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
    let groups = build(&verbs, &[], &wn, &freq, &p, &mut r);
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
    let groups = build(&verbs, &[], &wn, &freq, &p, &mut r);
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
    let plain = build(&verbs, &[], &wn, &freq, &off, &mut r2);
    assert!(
        !plain.iter().any(|g| g.contains(&"sprint".to_string())),
        "with --no-gap-fill the table is pure synsets"
    );
}

#[test]
fn a_corroborated_verb_entry_becomes_a_group_and_outranks_the_synset() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    // Two stories declare `light` and `ignite` to be one verb — which WordNet
    // also happens to say, in a synset `light` reaches only as its SECOND
    // sense. The game-derived group must come first all the same.
    let games = vec![GameGroup {
        words: vec!["ignite".into(), "light".into()],
        stories: 2,
    }];
    let p = Params {
        band_cap: 9,
        ..Params::default()
    };
    let mut r = Report::default();
    let groups = build(&verbs, &games, &wn, &freq, &p, &mut r);
    let first = groups
        .iter()
        .position(|g| g.contains(&"ignite".to_string()))
        .expect("the game group");
    let illuminate = groups
        .iter()
        .position(|g| g.contains(&"illuminate".to_string()))
        .expect("the sense-1 synset");
    assert!(
        first < illuminate,
        "a game-derived group must precede every synset of its members: {groups:?}"
    );
    assert_eq!(r.game_kept, 1);
}

#[test]
fn one_story_is_not_evidence() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    let verbs = vec![IfVerb {
        emit: "light".into(),
        lemma: "light".into(),
        stories: 100,
    }];
    // A single game's idiom: `light` and `hurry` are not one action anywhere
    // but in that game.
    let games = vec![GameGroup {
        words: vec!["hurry".into(), "light".into()],
        stories: 1,
    }];
    let mut r = Report::default();
    let groups = build(
        &verbs,
        &games,
        &wn,
        &freq,
        &Params {
            band_cap: 9,
            ..Params::default()
        },
        &mut r,
    );
    assert_eq!(r.game_kept, 0, "one story must not carry a group");
    assert!(!groups
        .iter()
        .any(|g| g.contains(&"hurry".to_string()) && g.contains(&"light".to_string())));
}

#[test]
fn a_truncated_spelling_is_finished_from_the_corpus_or_left_out() {
    let wn = wordnet_fixture();
    let freq = frequency_fixture();
    // `illumi` is what a six-character dictionary holds; the corpus spells the
    // word in full elsewhere, so the group gets the whole word. `zzzzzz` is
    // six characters that finish nothing, and is dropped rather than shown to
    // a player.
    let verbs = vec![
        IfVerb {
            emit: "illuminate".into(),
            lemma: "illuminate".into(),
            stories: 3,
        },
        IfVerb {
            emit: "light".into(),
            lemma: "light".into(),
            stories: 100,
        },
    ];
    let games = vec![GameGroup {
        words: vec!["illumi".into(), "light".into(), "zzzzzz".into()],
        stories: 4,
    }];
    let mut r = Report::default();
    let groups = build(
        &verbs,
        &games,
        &wn,
        &freq,
        &Params {
            band_cap: 9,
            ..Params::default()
        },
        &mut r,
    );
    assert_eq!(r.game_kept, 1, "the entry should have become one group");
    let g = groups
        .iter()
        .find(|g| g.contains(&"illuminate".to_string()) && g.contains(&"light".to_string()))
        .expect("the game group, with the truncation finished");
    assert_eq!(
        g.len(),
        2,
        "`zzzzzz` finishes nothing and must be dropped: {g:?}"
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

/// The committed verb ENTRIES, likewise — and this one carries the evidence for
/// the quest: `inspect` and `examine` are one verb in game after game, which is
/// the fact WordNet does not have.
#[test]
fn the_committed_verb_entries_are_well_formed() {
    let text = include_str!("../if_groups.tsv");
    let mut n = 0;
    let mut corroborating_inspect_examine = 0;
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() >= 3, "a count and two or more spellings: {line:?}");
        let stories: usize = f[0].parse().expect("a story count");
        assert!(stories > 0, "a set nobody declares: {line:?}");
        let mut members = f[1..].to_vec();
        for w in &members {
            assert!(
                w.len() >= 2 && w.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'),
                "not a dictionary spelling: {w:?}"
            );
        }
        let sorted = {
            let mut m = members.clone();
            m.sort_unstable();
            m
        };
        assert_eq!(members, sorted, "members must be sorted: {line:?}");
        members.dedup();
        assert_eq!(members.len(), f.len() - 1, "repeated member: {line:?}");
        if members.contains(&"examine") && members.contains(&"inspect") {
            corroborating_inspect_examine += stories;
        }
        n += 1;
    }
    assert!(
        n > 1000,
        "only {n} verb entries — did the harvest run against an empty corpus?"
    );
    assert!(
        corroborating_inspect_examine >= 10,
        "only {corroborating_inspect_examine} stories put `inspect` and `examine` on one verb"
    );
}
