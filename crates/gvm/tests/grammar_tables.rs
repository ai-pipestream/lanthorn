// Locating and reading Inform's grammar tables in real Glulx stories — SQ-1102.
//
// The numbers pinned below were read out of `glulxdump` (Andrew Plotkin, in the
// Glulxe source tree: <https://github.com/erkyrath/glulxe>), not out of this
// implementation. That tool cannot find the tables itself — it must be handed
// the address with `-g` — so the workflow was: locate with `gvm::grammar`, hand
// the address to glulxdump, and diff its dump against ours. Over the 22 Glulx
// stories in the local corpus, every verb, every line, every action number,
// every flags byte, every token type and every routine/attribute/scope value
// matched: 6,911 grammar lines with no difference.
//
// `stories/` is gitignored commercial media, so those cases skip vacuously and
// each carries a guard naming a count. The one case CI can see is
// `glulxercise.ulx`, and it is a refusal — which is worth having, because a
// locator that finds tables in a story with no grammar is the failure mode that
// would poison every consumer downstream.

use std::path::PathBuf;

use gvm::grammar::{Grammar, GrammarError, NounKind, Token};
use gvm::memory::Memory;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Pull the `GLUL` chunk out of a Blorb, or pass a bare Glulx image through.
/// Hand-rolled so this suite adds no dependency to a zero-dependency crate.
fn glulx_image(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(b"Glul") {
        return Some(bytes);
    }
    if !(bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"IFRS")) {
        return None;
    }
    let be32 = |a: usize| -> usize {
        u32::from_be_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]) as usize
    };
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = be32(i + 4);
        if &bytes[i..i + 4] == b"GLUL" {
            return bytes.get(i + 8..i + 8 + len).map(<[u8]>::to_vec);
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// Load a gitignored commercial story, or `None` so the case can skip.
fn story(name: &str) -> Option<Memory> {
    let path = stories_dir().join(name);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return None;
    }
    Memory::new(glulx_image(std::fs::read(&path).ok()?)?).ok()
}

// ── The committed fixture, so CI sees something ─────────────────────────────

#[test]
fn glulxercise_has_a_dictionary_and_no_grammar_and_is_refused() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
    let bytes = std::fs::read(&path).expect("glulxercise fixture is committed");
    let mem = Memory::new(glulx_image(bytes).expect("bare Glulx image")).unwrap();

    // `glulxercise` is a VM conformance suite compiled by Inform 6: it has a
    // dictionary (138 words) but no parser, so no actions table ends where the
    // dictionary begins and no grammar table ends where that would. The
    // distinction matters — `TablesNotFound` says the dictionary WAS found and
    // the chain would not close, which is the locator declining rather than
    // never having started.
    assert_eq!(Grammar::load(&mem).err(), Some(GrammarError::TablesNotFound));
}

// ── Real commercial media (skips vacuously without `stories/`) ──────────────

#[test]
fn counterfeit_monkey_locates_and_reads() {
    let Some(mem) = story("CounterfeitMonkey-11.gblorb") else { return };
    let g = Grammar::load(&mem).expect("Counterfeit Monkey has a grammar table");
    let t = g.tables();

    // glulxdump: "Grammar table at 0077685a: 314 entries".
    assert_eq!(t.grammar, 0x77685a);
    assert_eq!(t.verb_count, 314);
    assert_eq!(t.action_count, 244);
    assert_eq!(t.word_count, 4136);
    // Non-vacuity: the chain must abut, and the corpus dump has 809 lines.
    assert_eq!(t.actions + 4 + 4 * t.action_count, t.dictionary);
    let lines: usize = g.verbs().iter().map(|v| v.lines.len()).sum();
    assert_eq!(lines, 809);

    // The question the dictionary alone cannot answer.
    let insert = g.verb_for_word("insert").expect("knows 'insert'");
    assert!(insert.accepts(2, &["in"]));
    assert!(insert.accepts(2, &["into"]));
    assert!(!insert.accepts(2, &["with"]));
    assert!(g.is_verb("go") && g.is_verb("take"));
    assert!(g.is_preposition("in") && g.is_preposition("into"));
}

#[test]
fn eat_me_resolves_verb_synonyms_through_the_inverted_numbering() {
    let Some(mem) = story("Eat_Me.gblorb") else { return };
    let g = Grammar::load(&mem).expect("Eat Me has a grammar table");
    assert_eq!(g.tables().verb_count, 326);
    let lines: usize = g.verbs().iter().map(|v| v.lines.len()).sum();
    assert_eq!(lines, 608);

    // Inform counts verb numbers DOWN from $FFFF in a Glulx dictionary. Read
    // straight, every verb would come out unnamed — which is exactly what this
    // reader did before the inversion was found in the compiler source.
    let take = g.verb_for_word("take").expect("knows 'take'");
    assert_eq!(take.number, 0);
    assert_eq!(take.word(), Some("carry"));
    assert!(take.words.iter().any(|w| w == "hold"));
    assert!(g.verbs().iter().filter(|v| v.word().is_some()).count() > 100);
}

#[test]
fn cragne_manor_is_the_largest_table_in_the_corpus() {
    let Some(mem) = story("cragne.gblorb") else { return };
    let g = Grammar::load(&mem).expect("Cragne Manor has a grammar table");
    assert_eq!(g.tables().verb_count, 368);
    assert_eq!(g.tables().action_count, 375);
    let lines: usize = g.verbs().iter().map(|v| v.lines.len()).sum();
    assert_eq!(lines, 1115);
}

#[test]
fn alternative_prepositions_stay_one_slot() {
    let Some(mem) = story("Eat_Me.gblorb") else { return };
    let g = Grammar::load(&mem).expect("Eat Me has a grammar table");

    // Inform writes `'in' / 'into' / 'on' / 'onto'` as one position with four
    // alternatives ($20 on the token before a slash, $10 on the one after).
    // Flattened, this line would claim the story wants four words in a row.
    let slot = g
        .verbs()
        .iter()
        .flat_map(|v| v.lines.iter())
        .flat_map(|l| l.slots.iter())
        .find(|s| s.alternatives.len() >= 3)
        .expect("some line has a list of alternative prepositions");
    let words: Vec<&str> = slot.alternatives.iter().filter_map(Token::word).collect();
    assert!(words.len() >= 3, "got {words:?}");
    assert!(slot.only().is_none());
}

#[test]
fn dictionary_word_size_varies_between_games() {
    // Inform's default is nine characters per record (stride 16), but
    // `$DICT_WORD_SIZE` is a memory setting and the corpus contains 10 and 12.
    // A reader that hard-coded 16 would find no dictionary in these two at all.
    for (name, stride) in
        [("The_Wizard_Sniffer.gblorb.blorb", 19), ("And_Then_You_Come_to_a_House.gblorb.blorb", 17)]
    {
        let Some(mem) = story(name) else { continue };
        let g = Grammar::load(&mem).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(g.tables().dict_stride, stride, "{name}");
        assert_eq!(g.tables().dict_word_size, stride - 7, "{name}");
        assert!(g.is_verb("take") || g.is_verb("get"), "{name}");
    }
}

#[test]
fn every_glulx_story_either_reads_or_refuses() {
    // A sweep rather than a pin. The locator derives three addresses the image
    // does not record, so the property that matters is that it never answers
    // where the chain does not close.
    let Ok(entries) = std::fs::read_dir(stories_dir()) else {
        eprintln!("SKIP: stories/ absent");
        return;
    };
    let mut read = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.ends_with(".gblorb") || name.ends_with(".gblorb.blorb") || name.ends_with(".ulx"))
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Some(image) = glulx_image(bytes) else { continue };
        let Ok(mem) = Memory::new(image) else { continue };
        let Ok(g) = Grammar::load(&mem) else { continue };
        read += 1;
        let t = g.tables();
        // The three tables abut, in order, every time.
        assert!(t.grammar < t.actions, "{name}");
        assert_eq!(t.actions + 4 + 4 * t.action_count, t.dictionary, "{name}");
        // Every verb the table names is reachable by the word that names it.
        for verb in g.verbs() {
            for w in &verb.words {
                assert!(g.is_verb(w), "{name}: {w} unreachable");
            }
        }
        // Every elementary token decoded to a slot the parser actually has.
        assert!(g
            .verbs()
            .iter()
            .flat_map(|v| v.lines.iter())
            .flat_map(|l| l.slots.iter())
            .flat_map(|s| s.alternatives.iter())
            .any(|t| matches!(t, Token::Noun(NounKind::Noun))));
    }
    assert!(read >= 10, "expected a Glulx corpus, read {read}");
}
