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
//!   Within each group, words are sorted alphabetically.
//!   Duplicates across groups are deduplicated (room-word wins).

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
}
