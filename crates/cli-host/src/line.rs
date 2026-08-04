//! Knowing where you are in a line, and holding a prompt back.
//!
//! Both text hosts have the same problem: they stream a game's output to
//! stdout, and every so often they must insert text of their own — a status
//! line, a grid window — at a point that reads correctly. Two facts decide
//! where that is, and neither is available from the sink:
//!
//! - **Are we mid-line?** Host text appended to `>` produces
//!   `> In the Wings   Score: 0  Moves: 0`, which reads as though the prompt
//!   were showing you a room.
//! - **Has the prompt already gone out?** It has. The last thing a game writes
//!   each turn is its prompt, with no trailing newline, and the host only learns
//!   the turn is over when the game asks for input — by which time the prompt is
//!   on its way to the terminal and the status can only follow it.
//!
//! [`LineHold`] answers both. It tracks the line position always, and in plain
//! mode it also holds back the unterminated tail, so the status can be written
//! first and the prompt released after it. The turn then reads
//!
//! ```text
//! In the Wings
//! The big top can be entered to the north...
//!
//! In the Wings                          Score: 0  Moves: 0
//! >north
//! ```
//!
//! rather than welding the status to the prompt and leaving the player's typing
//! stranded below it.
//!
//! It is here rather than in either CLI because both had grown their own half of
//! the answer: `gvm-cli` tracked the line position and inserted a newline (so
//! the status still landed *after* the prompt), and `zvm-cli` tracked nothing at
//! all. One mechanism, two configurations (SQ-0611).

/// Tracks position within the output line, optionally holding the tail.
pub struct LineHold {
    hold: bool,
    /// Text written since the last newline. Only ever non-empty when `hold`.
    partial: String,
    at_line_start: bool,
    /// The last tail released — the prompt, in practice. Kept so a host answer
    /// mid-turn (`/status`) can put the prompt back afterwards.
    last_released: String,
}

impl LineHold {
    /// `hold` = withhold unterminated tails (plain mode). When false this is a
    /// pure line-position tracker and `feed` returns its input unchanged.
    pub fn new(hold: bool) -> Self {
        LineHold { hold, partial: String::new(), at_line_start: true, last_released: String::new() }
    }

    /// Absorb `s` and return what may go to the sink now.
    ///
    /// Holding: everything up to and including the last newline; the tail after
    /// it is kept. Not holding: all of `s`.
    pub fn feed(&mut self, s: &str) -> String {
        if let Some(last) = s.chars().last() {
            self.at_line_start = last == '\n';
        }
        if !self.hold {
            return s.to_string();
        }
        self.partial.push_str(s);
        match self.partial.rfind('\n') {
            Some(i) => self.partial.drain(..=i).collect(),
            None => String::new(),
        }
    }

    /// Is the sink at the start of a line — i.e. would host text written now
    /// begin its own line?
    ///
    /// True while a tail is held, because the held text has not reached the sink
    /// and host text written now really would start a line.
    pub fn at_line_start(&self) -> bool {
        self.at_line_start || !self.partial.is_empty()
    }

    /// Take the held tail, to be written *after* whatever the host just
    /// inserted. Empty when nothing is held.
    ///
    /// Every path that blocks, exits, or lets something else write to the sink
    /// must call this, or the prompt arrives behind whatever overtook it — or
    /// never arrives at all.
    pub fn release(&mut self) -> String {
        let out = std::mem::take(&mut self.partial);
        if !out.is_empty() {
            self.last_released = out.clone();
        }
        out
    }

    /// The prompt most recently released, for putting back after a host answer.
    ///
    /// A player who types `/status` at `>` should get the status and then see
    /// `>` again — otherwise the host has answered and left them typing into
    /// what looks like the middle of the transcript. Empty when nothing has ever
    /// been held (a plain pipe, where the prompt went straight out and there is
    /// nothing to restore).
    pub fn last_prompt(&self) -> &str {
        &self.last_released
    }

    /// Is anything being held?
    pub fn is_holding(&self) -> bool {
        !self.partial.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_holding_it_only_tracks_position() {
        let mut h = LineHold::new(false);
        assert!(h.at_line_start(), "a fresh stream is at a line start");
        assert_eq!(h.feed("You are in a room.\n"), "You are in a room.\n");
        assert!(h.at_line_start());
        assert_eq!(h.feed(">"), ">", "text passes straight through");
        assert!(!h.at_line_start(), "an unterminated prompt leaves us mid-line");
        assert_eq!(h.release(), "", "nothing was held");
    }

    #[test]
    fn holding_keeps_the_prompt_back_until_released() {
        let mut h = LineHold::new(true);
        // A complete line goes out immediately.
        assert_eq!(h.feed("The big top can be entered.\n"), "The big top can be entered.\n");
        // The prompt does not.
        assert_eq!(h.feed("\n>"), "\n", "the newline goes, the prompt stays");
        assert!(h.is_holding());
        assert!(h.at_line_start(), "the sink really is at a line start — `>` has not reached it");
        // ...so the status can be written first, and the prompt released after.
        assert_eq!(h.release(), ">");
        assert!(!h.is_holding());
        assert_eq!(h.release(), "", "releasing twice is not a duplicate prompt");
    }

    #[test]
    fn a_held_tail_accumulates_across_writes() {
        // Games emit a prompt one character at a time as often as not.
        let mut h = LineHold::new(true);
        assert_eq!(h.feed("\n"), "\n");
        for ch in [">", " "] {
            assert_eq!(h.feed(ch), "", "still holding: {ch:?}");
        }
        assert_eq!(h.release(), "> ");
    }

    #[test]
    fn a_newline_arriving_later_flushes_what_was_held() {
        // If the game turns out not to be prompting after all, the held text
        // must still reach the sink under its own steam.
        let mut h = LineHold::new(true);
        assert_eq!(h.feed("Time "), "");
        assert_eq!(h.feed("passes...\n"), "Time passes...\n", "the whole line, in order");
        assert!(!h.is_holding());
    }

    #[test]
    fn multiple_lines_in_one_write_split_at_the_last_newline() {
        let mut h = LineHold::new(true);
        assert_eq!(h.feed("one\ntwo\n>"), "one\ntwo\n");
        assert_eq!(h.release(), ">");
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn the_released_prompt_is_remembered_so_it_can_be_put_back() {
        let mut h = LineHold::new(true);
        assert_eq!(h.last_prompt(), "", "nothing released yet");
        h.feed("Time passes...\n");
        h.feed(">");
        assert_eq!(h.release(), ">");
        assert_eq!(h.last_prompt(), ">", "remembered for after a /status answer");
        // Releasing nothing must not forget the real prompt.
        assert_eq!(h.release(), "");
        assert_eq!(h.last_prompt(), ">", "an empty release does not clear it");
    }

    #[test]
    fn nothing_is_remembered_when_not_holding() {
        // A plain pipe writes the prompt straight through, so there is no held
        // prompt to restore and the host answer just starts its own line.
        let mut h = LineHold::new(false);
        h.feed(">");
        assert_eq!(h.release(), "");
        assert_eq!(h.last_prompt(), "");
    }
}
