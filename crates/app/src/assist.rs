//! The voice every assist speaks in (SQ-1045).
//!
//! An **assist** is lanthorn helping the player *play*: the game's own words when
//! the parser rejects theirs, a completed noun, a caution before a move that
//! cannot be taken back, a pointer at the hints that exist. It is not a report of
//! something lanthorn did (that is [`TranscriptKind::Meta`]) and not a fault
//! (that is [`TranscriptKind::Warning`]). Ask which of the three a line is before
//! reaching for this module: *does it help the player play?*
//!
//! # The register
//!
//! A lanthorn is a lantern, and the lamp is the most iconic object in the genre,
//! so the helper reads as the interpreter holding up a light. But atmosphere must
//! not cost **attribution**, and attribution is the whole point:
//!
//! > Infocom's parser already speaks in brackets — `[I don't know the word
//! > "illuminate".]` — and a helper writing in the same register, in the same
//! > stream, is confusing. It is worse than confusing when the helper is
//! > **wrong**, because the player attributes our mistake to the game.
//!
//! So the flavour lives in the WORDS and the separation lives in the KIND and the
//! STYLE. Three mechanisms, and only the first survives every form the text can
//! take:
//!
//! 1. **The marker.** Every assist line begins `Lanthorn: `. This is not
//!    decoration, it is the mechanism — it survives a copy-paste, a saved
//!    transcript, a screenshot in a bug report, and a screen reader narrating the
//!    pane, to which neither a colour nor a gutter glyph is audible. The words
//!    are the only carrier that reaches every one of those. [`Assist::lines`]
//!    applies the marker; nothing else may.
//! 2. **The kind.** [`TranscriptKind::Assist`] tags the line, so `/filter story`
//!    hides every assist and a player who wants 1982 gets 1982. Code can tell an
//!    assist from a slash dump without reading its text.
//! 3. **The style.** `transcript_assist` / `transcript_assist_caution` in
//!    `style.toml`, with their own gutter glyph, so a player can make these
//!    quieter or louder without touching anything else.
//!
//! # What an assist line looks like
//!
//! ```text
//! Lanthorn holds up a light — lines that start with "Lanthorn" are the interpreter's, not the story's.
//! Lanthorn: this story knows — light · turn on · burn
//! ```
//!
//! and, every time after the first in a session, just the second line. The
//! **flourish tapers; the marker never does.** Same identity both times, so
//! attribution never weakens — only the prose does. That is also the answer to
//! the paperclip worry: a lantern that announces itself on every completion is
//! Clippy, and one that stops naming itself is unattributable. Naming itself
//! plainly, forever, is neither.
//!
//! # What an assist line must never look like
//!
//! * **Bracketed.** `[…]` is the Z-machine parser's own voice. Never.
//! * **Unmarked.** Not even a short one, not even an obviously-ours one. The
//!   moment one line slips through unmarked the marker stops being a guarantee
//!   and becomes a habit, and a reader can no longer tell by looking.
//! * **In the story's second person.** "You could try turning on the lamp" is the
//!   game's voice. Say what the *story* knows and let the player decide:
//!   `Lanthorn: this story knows — light · turn on · burn`.
//! * **A boast, or an apology.** It fires mid-play, twenty times a session, and
//!   is sometimes wrong. Read every candidate line back on the twentieth firing
//!   and again assuming the suggestion is useless; anything that grates in either
//!   reading is the wrong line.
//! * **Long.** These arrive between the player's command and the game's reply, on
//!   a pane that may be forty columns wide. One line. Two if the second is a list.
//! * **A spoiler.** The assists volunteer; the hints wait to be asked. An assist
//!   may say hints exist and how to reach them, never what they say.
//!
//! # How to emit one
//!
//! ```ignore
//! state.push_assist(&Assist::help(format!("this story knows — {}", verbs.join(" · "))));
//! state.push_assist(&Assist::caution("burning the leaflet cannot be undone."));
//! ```
//!
//! Build the [`Assist`] and hand it over whole: the text and its tone are one
//! subject and travel as one value, so a caller cannot supply the words and
//! forget the weight. Everything else — the marker, the kind, the style lookup,
//! the once-per-session flourish — belongs to [`AppState::push_assist`], which is
//! the only door. `tests/suites/assist_voice.rs` fails any source file that
//! builds a `TranscriptKind::Assist` line by hand.
//!
//! [`TranscriptKind::Meta`]: crate::state::TranscriptKind::Meta
//! [`TranscriptKind::Warning`]: crate::state::TranscriptKind::Warning
//! [`TranscriptKind::Assist`]: crate::state::TranscriptKind::Assist
//! [`AppState::push_assist`]: crate::state::AppState::push_assist

/// The name that marks a line as ours, in every form the text can take.
pub const NAME: &str = "Lanthorn";

/// The mandatory prefix of an assist's first line.
pub const PREFIX: &str = "Lanthorn: ";

/// Indent for an assist's continuation lines. Two spaces, which is what
/// `render::transcript`'s hanging wrap already treats as a continuation.
pub const CONT_INDENT: &str = "  ";

/// The once-per-session flourish, emitted above the first assist a session
/// shows. It introduces the name that every later line is marked with, which is
/// why it says what the marker MEANS rather than merely being atmospheric.
pub const PREAMBLE: &str =
    "Lanthorn holds up a light — lines that start with \"Lanthorn\" are the interpreter's, not the story's.";

/// How much weight an assist carries. Two, deliberately: "here is something that
/// helps" and "you are about to do something you cannot undo". A third would be a
/// distinction the player cannot act on differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistTone {
    /// The ordinary light: vocabulary, completions, where the hints are.
    Help,
    /// A consequence worth knowing before it happens (SQ-1043's irreversible
    /// move). Louder, because ignoring it costs the player their game.
    Caution,
}

impl AssistTone {
    /// The `style.toml` selector this tone draws in.
    pub fn selector(self) -> &'static str {
        match self {
            AssistTone::Help => "transcript_assist",
            AssistTone::Caution => "transcript_assist_caution",
        }
    }
}

/// One thing lanthorn has to say to the player, with the weight it carries.
///
/// The text and the tone are one subject, so they travel together rather than as
/// two arguments a caller can get out of order or supply half of (see CLAUDE.md's
/// refactoring policy). A later fact — which feature spoke, say, once `/assist`
/// can be turned off per feature — is a field here, not another parameter at
/// every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assist {
    text: String,
    tone: AssistTone,
}

impl Assist {
    /// The ordinary light. `text` is the assist WITHOUT the marker — the marker
    /// is applied here and cannot be opted out of.
    pub fn help(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: AssistTone::Help }
    }

    /// A consequence worth knowing before it happens.
    pub fn caution(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: AssistTone::Caution }
    }

    pub fn tone(&self) -> AssistTone {
        self.tone
    }

    /// The unmarked text, as the caller supplied it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The transcript lines this assist becomes: the first carries [`PREFIX`],
    /// every continuation carries [`CONT_INDENT`] so it hangs under the marker
    /// instead of masquerading as a fresh unmarked line.
    pub fn lines(&self) -> Vec<String> {
        self.text
            .split('\n')
            .enumerate()
            .map(|(i, l)| if i == 0 { format!("{PREFIX}{l}") } else { format!("{CONT_INDENT}{l}") })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_first_line_carries_the_marker() {
        assert_eq!(Assist::help("this story knows — light").lines(), vec!["Lanthorn: this story knows — light"]);
        assert_eq!(Assist::caution("that cannot be undone.").lines(), vec!["Lanthorn: that cannot be undone."]);
    }

    #[test]
    fn continuations_hang_under_the_marker_rather_than_reading_as_story() {
        let a = Assist::help("this story knows:\nlight · turn on · burn");
        assert_eq!(a.lines(), vec!["Lanthorn: this story knows:", "  light · turn on · burn"]);
    }

    #[test]
    fn the_two_tones_draw_from_different_selectors() {
        assert_ne!(AssistTone::Help.selector(), AssistTone::Caution.selector());
    }

    /// The register's own rule, applied to the constants: no assist text may wear
    /// the Z-machine parser's brackets.
    #[test]
    fn the_preamble_is_not_in_the_parsers_bracket_voice() {
        assert!(!PREAMBLE.starts_with('['));
        assert!(PREAMBLE.starts_with(NAME));
        assert!(PREFIX.starts_with(NAME));
    }
}
