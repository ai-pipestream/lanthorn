//! Announcing a score that changed.
//!
//! Screen-reader mode stops narrating the status line every turn, because a
//! Z-machine v3 one carries a move counter and would be re-read on every single
//! turn (SQ-0612). That is the right trade for the *line*, but it takes the
//! score with it — and the score is the part of a status line that carries
//! news. A sighted player sees it tick over; a listener would have to type
//! `/status` on the off-chance.
//!
//! So the score is watched, and announced only when it moves.
//!
//! ## Where the number comes from is not the same question in each engine
//!
//! This module deliberately does not know. It takes an `Option<i32>` and reports
//! changes; finding the number is the host's job, and the hosts disagree because
//! the formats do:
//!
//! - **Z-machine v1–v3** — the interpreter owns the status line and the
//!   standard puts the score in global 2 (ZMSD §8.2). Exact.
//! - **Z-machine v4+** — the game draws its own status line and the globals mean
//!   whatever it wants, so there is no number to read: it has to be recovered
//!   from the text the game drew.
//! - **Glulx** — Glk has no concept of a score at all. Same recovery from text.
//! - **Scott Adams** — no stored score either; the score *is* the count of
//!   treasures deposited in the treasure room, which the VM can count exactly.
//!
//! Two of the four are therefore exact and two are a guess at someone's
//! formatting. [`score_in_status`] is that guess, kept in one place and honest
//! about being one.

/// Watches a score and reports the changes.
#[derive(Debug, Default)]
pub struct ScoreWatch {
    last: Option<i32>,
}

impl ScoreWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the current score; get back an announcement if it changed.
    ///
    /// The first observation is silent. It is not news — it is the score the
    /// game started with, and announcing it would put a line in front of the
    /// player before anything has happened.
    ///
    /// `None` means "no score visible this turn", which is not the same as zero:
    /// a game whose status line vanishes while a menu is open has not scored
    /// zero, so the last known value is kept and a later reappearance at the
    /// same number stays silent.
    pub fn update(&mut self, score: Option<i32>) -> Option<String> {
        let score = score?;
        let previous = self.last.replace(score)?;
        if score == previous {
            return None;
        }
        let delta = score - previous;
        // Words rather than a signed number: a reader announces "minus five"
        // for -5 only if its punctuation level is set high enough, and reads
        // "+5" as "5" at the default.
        let direction = if delta > 0 { "up" } else { "down" };
        Some(format!("[Score {score}, {direction} {}]", delta.abs()))
    }

    /// The last score seen, if any.
    pub fn last(&self) -> Option<i32> {
        self.last
    }
}

/// Recover a score from status-line text, for the formats that do not expose one.
///
/// Matches `Score: 10`, `Score 10`, and `SCORE:10`, taking the last such field
/// on the line — Inform's default status line is
/// `Location                    Score: 0  Moves: 1`, and taking the last guards
/// against a room genuinely called something like "Score Board".
///
/// This is pattern-matching on someone else's formatting and will miss a game
/// that words it differently ("Points", a bare number, a translated status
/// line). It fails by returning `None`, which is silence rather than a wrong
/// announcement.
pub fn score_in_status(text: &str) -> Option<i32> {
    let lower = text.to_ascii_lowercase();
    let mut found = None;
    let mut from = 0;
    while let Some(at) = lower[from..].find("score") {
        let after = from + at + "score".len();
        let rest = text[after..]
            .trim_start()
            .strip_prefix(':')
            .unwrap_or(&text[after..])
            .trim_start();
        let digits: String = rest
            .strip_prefix('-')
            .map(|d| format!("-{}", d.chars().take_while(char::is_ascii_digit).collect::<String>()))
            .unwrap_or_else(|| rest.chars().take_while(char::is_ascii_digit).collect());
        if let Ok(n) = digits.parse::<i32>() {
            found = Some(n);
        }
        from = after;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opening_score_is_not_news() {
        let mut w = ScoreWatch::new();
        // Non-zero on purpose: a game that starts you at 10 (or a restored save)
        // must still be silent, which a starting score of 0 cannot distinguish
        // from "no previous value" if the seed is ever treated as zero.
        assert_eq!(w.update(Some(10)), None, "the starting score is not an event");
        assert_eq!(w.update(Some(10)), None, "nor is it every turn after");
        assert_eq!(w.last(), Some(10));
    }

    #[test]
    fn a_change_is_announced_with_its_direction() {
        let mut w = ScoreWatch::new();
        w.update(Some(0));
        assert_eq!(w.update(Some(5)).as_deref(), Some("[Score 5, up 5]"));
        assert_eq!(w.update(Some(5)), None, "and then stays quiet");
        assert_eq!(w.update(Some(3)).as_deref(), Some("[Score 3, down 2]"));
        // Negative scores are legal in the Z-machine (a signed word).
        assert_eq!(w.update(Some(-1)).as_deref(), Some("[Score -1, down 4]"));
    }

    #[test]
    fn a_missing_score_is_not_a_score_of_zero() {
        // A menu covering the status line must not read as "lost all your
        // points", and its disappearance must not lose the last known value.
        let mut w = ScoreWatch::new();
        w.update(Some(10));
        assert_eq!(w.update(None), None, "no status this turn: silence");
        assert_eq!(w.last(), Some(10), "the last known score is kept");
        assert_eq!(w.update(Some(10)), None, "and coming back unchanged is not news");
    }

    #[test]
    fn score_is_read_out_of_an_inform_status_line() {
        assert_eq!(score_in_status("West of House          Score: 0  Moves: 1"), Some(0));
        assert_eq!(score_in_status(" In the Wings      Score: 25  Moves: 140"), Some(25));
        assert_eq!(score_in_status("Back Alley, noon    Goals: 1  Score: 7"), Some(7));
        assert_eq!(score_in_status("SCORE:42"), Some(42), "no space, any case");
        assert_eq!(score_in_status("Score -3  Moves: 2"), Some(-3), "scores can go negative");
    }

    #[test]
    fn a_room_named_after_the_word_does_not_become_the_score() {
        // Taking the LAST match is what saves this: the real field comes after.
        assert_eq!(score_in_status("Score Board        Score: 12  Moves: 3"), Some(12));
    }

    #[test]
    fn silence_rather_than_a_wrong_number_when_there_is_nothing_to_read() {
        assert_eq!(score_in_status(""), None);
        assert_eq!(score_in_status("I'm in a forest"), None);
        assert_eq!(score_in_status("Time: 09:05"), None, "a time game has no score");
        assert_eq!(score_in_status("Score: none"), None, "unparseable is not zero");
    }
}
