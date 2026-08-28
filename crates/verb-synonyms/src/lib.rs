//! Synonym groups for interactive-fiction verbs — the bridge from a word the
//! player typed to one the story actually knows.
//!
//! ## What this is for
//!
//! A player types `illuminate lamp`. The story wants `light`. Nothing in the
//! story file records what `illuminate` MEANS, and every mechanism that works on
//! FORM fails on it: edit distance is 8 on a 10-letter word, stemming reaches
//! `illuminat-` and nothing else, and the grammar's shape only says "a verb
//! taking one noun", which is most verbs. The bridge has to come from outside
//! the story, so it is generated offline and shipped: [`synonym_groups.tsv`],
//! beside this file.
//!
//! ## Where the data came from, and how to regenerate it
//!
//! `crates/verb-synonyms-gen` builds the table from WordNet 3.0 (Princeton
//! University) and the 12dicts 6.0.2 lemmatized frequency list (Alan Beale,
//! under the AGID terms), against the verb vocabulary harvested from every story
//! its three parsers can read. See that crate's `README.md` for the exact source
//! versions and the two commands; the licence notices both sources require are
//! in `THIRD-PARTY-NOTICES.md` at the repository root.
//!
//! ## How to use it
//!
//! Two rules, and both are the caller's job:
//!
//! 1. **Lemmatise first.** Members are base forms, because an IF parser accepts
//!    the imperative — you type `take lamp`, never `took lamp`. `illuminated`
//!    will not be found; reduce it to `illuminate` before calling. Skipping this
//!    makes a missing morphology step look like a hole in the data.
//! 2. **Intersect with THIS story's dictionary.** The table proposes; the story
//!    disposes. [`suggest`] takes the predicate and does the rest, including
//!    dropping the word the player typed — it is in its own group by
//!    construction and it is the one word known to have failed.
//!
//! ```ignore
//! let known = |w: &str| grammar.is_verb(w);
//! for word in verb_synonyms::suggest("illuminate", known, 4) {
//!     // "light", if the story knows it
//! }
//! ```
//!
//! Nothing is parsed until the first lookup: most sessions never have a rejected
//! word at all.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

/// The shipped table, as it sits on disk.
///
/// Line order is significant — see [`groups`].
const TABLE: &str = include_str!("synonym_groups.tsv");

/// The parsed table: the groups in file order, and a word's group indices in
/// that same order.
struct Index {
    groups: Vec<Vec<&'static str>>,
    by_word: HashMap<&'static str, Vec<u32>>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut groups: Vec<Vec<&'static str>> = Vec::new();
        let mut by_word: HashMap<&'static str, Vec<u32>> = HashMap::new();
        for line in TABLE.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let members: Vec<&'static str> = line.split('\t').filter(|w| !w.is_empty()).collect();
            if members.len() < 2 {
                continue;
            }
            let id = groups.len() as u32;
            for w in &members {
                by_word.entry(w).or_default().push(id);
            }
            groups.push(members);
        }
        Index { groups, by_word }
    })
}

/// Every group `word` belongs to, in that word's own WordNet sense order —
/// commonest sense first.
///
/// The order is the whole reason the file must never be sorted. A word is
/// polysemous: `draw` is *pull*, *sketch* and *attract*, and it sits in one
/// group per sense. Walking them in order and stopping early shows the player
/// the reading they most likely meant, and keeps a rare fifth sense from
/// crowding out the common first one.
///
/// `word` must already be a base form; see the module docs.
pub fn groups(word: &str) -> impl Iterator<Item = &'static [&'static str]> {
    let idx = index();
    idx.by_word
        .get(word)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|&i| idx.groups[i as usize].as_slice())
}

/// What to offer a player whose word the parser rejected: the words in that
/// word's groups that THIS story's dictionary actually contains, most likely
/// sense first, at most `limit` of them.
///
/// `known` is asked about candidate spellings only, so it can be as expensive as
/// a dictionary lookup. The player's own word is never returned: it is in its
/// own group by construction, and it is the one word known to have failed.
///
/// `word` must already be a base form; see the module docs.
pub fn suggest(word: &str, known: impl Fn(&str) -> bool, limit: usize) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for group in groups(word) {
        for &candidate in group {
            if out.len() >= limit {
                return out;
            }
            if candidate != word && !out.contains(&candidate) && known(candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

/// How many groups the shipped table holds. For diagnostics and for tests that
/// want to know the data is really there.
pub fn group_count() -> usize {
    index().groups.len()
}

#[cfg(test)]
mod tests;
