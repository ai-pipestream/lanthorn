//! Autocomplete / word-suggestion for the game input line.
//!
//! The core `suggest` function is pure and has no I/O dependencies so it can be
//! unit-tested without spinning up a Z-machine.
//!
//! Suggestion sources:
//!   (a) The story's parser vocabulary from the Z-machine dictionary.
//!   (b) Nouns/words parsed from the current room's visible description text.
//!
//! Ranking (highest priority first):
//!   1. Room words that share the prefix (most contextually relevant).
//!   2. Dictionary words that share the prefix.
//!      Within each group, words are sorted alphabetically.
//!      Duplicates across groups are deduplicated (room-word wins).

/// Return up to `limit` completions for `partial` drawn from `dictionary` and
/// `room_words`.
///
/// - Matching is case-insensitive prefix match on `partial`.
/// - `partial` is lowercased before matching; all returned strings are
///   lowercase.
/// - An empty `partial` returns an empty list (nothing to complete yet).
/// - Duplicates are removed; if a word appears in both `room_words` and
///   `dictionary`, it is treated as a room word (ranked higher).
/// - Results are capped at `limit`.
pub fn suggest(
    dictionary: &[String],
    room_words: &[String],
    partial: &str,
    limit: usize,
) -> Vec<String> {
    if partial.is_empty() || limit == 0 {
        return Vec::new();
    }

    let lower = partial.to_lowercase();

    // Collect room-word matches (dedup via seen set).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut room_matches: Vec<String> = room_words
        .iter()
        .map(|w| w.to_lowercase())
        .filter(|w| w.starts_with(&lower) && w.as_str() != lower.as_str())
        .filter(|w| seen.insert(w.clone()))
        .collect();
    room_matches.sort_unstable();

    // Collect dictionary matches, skipping already-seen words.
    let mut dict_matches: Vec<String> = dictionary
        .iter()
        .map(|w| w.to_lowercase())
        .filter(|w| w.starts_with(&lower) && w.as_str() != lower.as_str())
        .filter(|w| seen.insert(w.clone()))
        .collect();
    dict_matches.sort_unstable();

    // Merge: room words first, then dictionary words, capped at limit.
    room_matches.extend(dict_matches);
    room_matches.truncate(limit);
    room_matches
}

/// Tokenise visible room-description text into candidate noun words.
///
/// Rules:
/// - Split on whitespace and punctuation (anything not alphanumeric or `'`).
/// - Lowercase everything.
/// - Drop words shorter than 3 characters.
/// - Drop common stop words that are not useful for autocomplete.
pub fn room_words_from_text(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "are", "you", "can", "not", "has", "was",
        "for", "with", "its", "this", "that", "have", "been",
        "from", "into", "onto", "there", "here", "some", "your",
        "also", "very", "than", "then", "will", "would", "could",
        "they", "them", "their", "but", "all", "any",
    ];

    let mut words: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for word in text.split(|c: char| !c.is_alphanumeric() && c != '\'') {
        if word.is_empty() {
            continue;
        }
        let lower = word.to_lowercase();
        // Drop short words and stop words.
        if lower.len() < 3 {
            continue;
        }
        if STOP_WORDS.contains(&lower.as_str()) {
            continue;
        }
        if seen.insert(lower.clone()) {
            words.push(lower);
        }
    }

    words
}

// ── Fuzzy matching (command palette) ───────────────────────────────────────────

/// A subsequence fuzzy-match of a query against one candidate string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Higher is a better match. Ranking is tiered: a prefix match always beats a
    /// word-boundary match, which always beats a scattered match; within a tier,
    /// consecutive runs and boundary hits refine the order.
    pub score: i32,
    /// Char indices into the candidate that the query matched, in order (for
    /// highlighting the matched characters in the list).
    pub positions: Vec<usize>,
}

/// Subsequence fuzzy-match `query` against `candidate`, case-insensitively.
///
/// Returns `None` when `query` is not a subsequence of `candidate`. An empty
/// query matches everything with a neutral score (keeps the full list visible).
///
/// Ranking tiers (highest first): the query is a **prefix** of the candidate; the
/// query matches only at **word boundaries** (start, or just after a `-`/`_`/space);
/// otherwise a **scattered** match. Within a tier, consecutive matched chars and
/// boundary hits raise the score, gaps and length lower it.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(FuzzyMatch { score: 0, positions: Vec::new() });
    }
    let cand_lower = candidate.to_lowercase();
    let cand: Vec<char> = cand_lower.chars().collect();

    // Greedy left-to-right subsequence match, recording each matched position.
    let mut positions: Vec<usize> = Vec::with_capacity(q.len());
    let mut ci = 0usize;
    for &qc in &q {
        let mut hit = None;
        while ci < cand.len() {
            let c = cand[ci];
            ci += 1;
            if c == qc {
                hit = Some(ci - 1);
                break;
            }
        }
        positions.push(hit?);
    }

    let is_boundary = |i: usize| i == 0 || !cand[i - 1].is_alphanumeric();
    let prefix = cand_lower.starts_with(&q.iter().collect::<String>());
    let all_boundary = positions.iter().all(|&p| is_boundary(p));

    // Tier base: prefix ≫ word-boundary ≫ scattered.
    let mut score = if prefix { 10_000 } else if all_boundary { 5_000 } else { 0 };

    // Within-tier refinement.
    for w in positions.windows(2) {
        if w[1] == w[0] + 1 {
            score += 20; // consecutive run
        } else {
            score -= (w[1] - w[0]) as i32; // gap penalty
        }
    }
    for &p in &positions {
        if is_boundary(p) {
            score += 10;
        }
    }
    score -= positions[0] as i32; // earlier first match is better
    score -= cand.len() as i32 / 4; // mild shorter-is-better tie-break

    Some(FuzzyMatch { score, positions })
}

/// One command-palette candidate: an index into [`crate::slash::COMMANDS`] plus
/// the fuzzy-match result its name scored against the query.
#[derive(Debug, Clone)]
pub struct PaletteCandidate {
    pub cmd_index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

/// Rank every registry command by fuzzy-matching its name against `query`.
///
/// Non-matching commands are dropped. Results are sorted best-first, ties broken
/// alphabetically by command name so the order is stable. An empty query returns
/// every command in registry order (scores are all neutral).
pub fn palette_candidates(query: &str) -> Vec<PaletteCandidate> {
    let mut out: Vec<PaletteCandidate> = crate::slash::COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, spec)| {
            fuzzy_match(query, spec.name).map(|m| PaletteCandidate {
                cmd_index: i,
                score: m.score,
                positions: m.positions,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| crate::slash::COMMANDS[a.cmd_index].name.cmp(crate::slash::COMMANDS[b.cmd_index].name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── suggest ────────────────────────────────────────────────────────────────

    #[test]
    fn empty_partial_returns_nothing() {
        let dict = vec!["open".to_string(), "north".to_string()];
        let room = vec!["mailbox".to_string()];
        assert!(suggest(&dict, &room, "", 6).is_empty());
    }

    #[test]
    fn prefix_match_basic() {
        let dict = vec!["open".to_string(), "north".to_string(), "object".to_string()];
        let room: Vec<String> = vec![];
        let result = suggest(&dict, &room, "o", 6);
        assert!(result.contains(&"open".to_string()));
        assert!(result.contains(&"object".to_string()));
        assert!(!result.contains(&"north".to_string()));
    }

    #[test]
    fn case_insensitive_matching() {
        let dict = vec!["Open".to_string(), "NORTH".to_string()];
        let room: Vec<String> = vec![];
        let result = suggest(&dict, &room, "Op", 6);
        assert!(result.contains(&"open".to_string()), "result: {:?}", result);
    }

    #[test]
    fn room_words_ranked_before_dict_words() {
        let dict = vec!["open".to_string(), "orange".to_string()];
        let room = vec!["oak".to_string()];
        let result = suggest(&dict, &room, "o", 6);
        // oak is a room word and must appear before dict words.
        assert_eq!(result[0], "oak");
    }

    #[test]
    fn dedup_room_wins_over_dict() {
        let dict = vec!["open".to_string()];
        let room = vec!["open".to_string()];
        // "open" appears in both; should appear exactly once, in the room group.
        let result = suggest(&dict, &room, "op", 6);
        assert_eq!(result, vec!["open".to_string()]);
    }

    #[test]
    fn limit_is_respected() {
        let dict: Vec<String> = (0..20).map(|i| format!("word{}", i)).collect();
        let room: Vec<String> = vec![];
        let result = suggest(&dict, &room, "w", 4);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn exact_match_is_excluded() {
        // If partial already exactly equals a candidate it should NOT be suggested
        // (nothing to complete).
        let dict = vec!["open".to_string()];
        let room: Vec<String> = vec![];
        let result = suggest(&dict, &room, "open", 6);
        assert!(result.is_empty(), "exact match should not appear: {:?}", result);
    }

    #[test]
    fn alphabetical_within_each_group() {
        let dict = vec!["orange".to_string(), "open".to_string(), "oak".to_string()];
        let room: Vec<String> = vec![];
        let result = suggest(&dict, &room, "o", 6);
        assert_eq!(result, vec!["oak", "open", "orange"]);
    }

    #[test]
    fn zero_limit_returns_nothing() {
        let dict = vec!["open".to_string()];
        let room: Vec<String> = vec![];
        assert!(suggest(&dict, &room, "op", 0).is_empty());
    }

    // ── room_words_from_text ───────────────────────────────────────────────────

    #[test]
    fn room_words_basic_tokenisation() {
        let words = room_words_from_text("A small wooden mailbox sits here.");
        assert!(words.contains(&"small".to_string()));
        assert!(words.contains(&"wooden".to_string()));
        assert!(words.contains(&"mailbox".to_string()));
        // "A" and "sits" and short words filtered
        assert!(!words.contains(&"a".to_string()));
    }

    #[test]
    fn room_words_stop_words_dropped() {
        let words = room_words_from_text("You are in the forest.");
        assert!(!words.contains(&"you".to_string()));
        assert!(!words.contains(&"are".to_string()));
        assert!(!words.contains(&"the".to_string()));
        assert!(words.contains(&"forest".to_string()));
    }

    #[test]
    fn room_words_deduplicates() {
        let words = room_words_from_text("open the box to open it");
        // "open" should appear only once
        assert_eq!(words.iter().filter(|w| w.as_str() == "open").count(), 1);
    }

    #[test]
    fn room_words_short_words_dropped() {
        // "it" is 2 chars — should be dropped
        let words = room_words_from_text("it is on a table");
        assert!(!words.contains(&"it".to_string()));
        assert!(!words.contains(&"is".to_string()));
        assert!(!words.contains(&"on".to_string()));
        assert!(!words.contains(&"a".to_string()));
        assert!(words.contains(&"table".to_string()));
    }

    // ── fuzzy_match ─────────────────────────────────────────────────────────────

    #[test]
    fn fuzzy_non_subsequence_is_none() {
        assert!(fuzzy_match("xyz", "zoom-map").is_none());
        assert!(fuzzy_match("zm", "zoom-map").is_some()); // z…m subsequence
    }

    #[test]
    fn fuzzy_empty_query_matches_everything() {
        let m = fuzzy_match("", "anything").expect("empty query matches");
        assert!(m.positions.is_empty());
    }

    #[test]
    fn fuzzy_is_case_insensitive() {
        let m = fuzzy_match("ZO", "zoom-map").expect("case-insensitive match");
        assert_eq!(m.positions, vec![0, 1]);
    }

    #[test]
    fn fuzzy_records_matched_positions() {
        // Greedy left-to-right: "zm" matches z@0 and the FIRST 'm' (index 3, in "zoom").
        let m = fuzzy_match("zm", "zoom-map").expect("subsequence");
        assert_eq!(m.positions, vec![0, 3]);
    }

    /// The core ranking contract: prefix > word-boundary > scattered.
    #[test]
    fn fuzzy_ranks_prefix_over_boundary_over_scattered() {
        // query "sm":
        //  - "smart"    → prefix (s,m at 0,1)
        //  - "save-map" → word-boundary (s at start, m just after '-')
        //  - "prism"    → scattered (s,m mid-word)
        let prefix = fuzzy_match("sm", "smart").unwrap().score;
        let boundary = fuzzy_match("sm", "save-map").unwrap().score;
        let scattered = fuzzy_match("sm", "prism").unwrap().score;
        assert!(prefix > boundary, "prefix {prefix} should beat boundary {boundary}");
        assert!(boundary > scattered, "boundary {boundary} should beat scattered {scattered}");
    }

    #[test]
    fn fuzzy_prefix_outranks_midword_hit_of_same_query() {
        // "zoom" as a prefix of "zoom-map" beats "zoom" buried mid-name.
        let prefix = fuzzy_match("zoom", "zoom-map").unwrap().score;
        let buried = fuzzy_match("zoom", "cycle-zoom-mode").unwrap().score;
        assert!(prefix > buried, "prefix {prefix} should beat buried {buried}");
    }

    // ── palette_candidates ──────────────────────────────────────────────────────

    #[test]
    fn palette_candidates_ranks_zoom_map_first_for_zoom() {
        let cands = palette_candidates("zoom");
        assert!(!cands.is_empty(), "expected zoom matches");
        let top = crate::slash::COMMANDS[cands[0].cmd_index].name;
        assert_eq!(top, "zoom-map", "zoom-map should rank first for query 'zoom', got {top}");
    }

    #[test]
    fn palette_candidates_empty_query_returns_all() {
        let cands = palette_candidates("");
        assert_eq!(cands.len(), crate::slash::COMMANDS.len());
    }

    #[test]
    fn palette_candidates_filters_nonmatches() {
        let cands = palette_candidates("quit");
        assert!(cands.iter().all(|c| fuzzy_match("quit", crate::slash::COMMANDS[c.cmd_index].name).is_some()));
        assert!(cands.iter().any(|c| crate::slash::COMMANDS[c.cmd_index].name == "quit"));
    }
}
