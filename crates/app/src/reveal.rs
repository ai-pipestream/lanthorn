//! The momentary reveal: light the words on screen the parser really knows
//! (SQ-1107).
//!
//! ```text
//! You are in a dimly lit room. Cobwebs hang from the beams, and a
//! ─────────                                       ────────
//! rusted iron lantern sits on the sill beside a faded portrait.
//! ────── ──── ───────           ────
//! ```
//!
//! The oldest frustration in the genre: a room description names a dozen nouns
//! and two of them are implemented. Players learn to probe blindly, and the ones
//! without nostalgia for that simply stop. This is the inverse of
//! [`crate::vocab`]'s offer, which can only help AFTER the parser has rejected a
//! word — it says which of the words already on screen would be accepted in the
//! first place.
//!
//! # It asks the story, twice, and never guesses
//!
//! **Where a word ends is the story's answer.** [`Engine::split_like_parser`]
//! (SQ-1116) is the code path `read` itself calls, so the dictionary's declared
//! separators, the Z-encoding and the §13.3 six / §13.4 nine Z-character
//! truncation all apply exactly as the game applies them. There is no word
//! splitter in this file, and the last one in the codebase was deleted for cause.
//!
//! **Which words light is the story's answer too**, at whichever of two
//! strengths this engine can support — see [`RevealTier`]. The strong one asks
//! the object tree what is actually here; the weak one can only ask the
//! dictionary what exists somewhere, and SAYS SO rather than passing the weaker
//! claim off as the stronger.
//!
//! # Nouns, not verbs
//!
//! A verb never lights, in either tier. The verb panel already answers "what can
//! I do"; this answers "what is real here", and they are different questions
//! that would blur into a ransom note if merged — "You are in an open field west
//! of a white house" lights `open` and `west` on a bare dictionary test, and the
//! prose then says nothing at all. Widening this to verbs is a decision, not a
//! tidy-up.
//!
//! # The viewport, judged in the present tense
//!
//! Exactly what is on screen lights, against the CURRENT game state. Scroll and
//! press again to light a different screenful, which answers "how far back do we
//! go?" with the scroll position instead of a constant somebody has to defend.
//!
//! Judging old visible text against present scope looks wrong for about as long
//! as it takes to say out loud. The player's question is present-tense: *of the
//! words I can see, which work right now?* A lamp described fifty turns ago and
//! still in scope SHOULD light; one since taken away should not. The age of the
//! text is irrelevant to it.
//!
//! # Momentary, because a terminal cannot do hold-to-reveal
//!
//! Terminals do not report key RELEASE without the kitty keyboard protocol, and
//! lanthorn never pushes keyboard enhancement flags — `input.rs` sees
//! `KeyEventKind::Press` only. Enabling them would work on Ghostty and not on
//! Windows Terminal, and a feature that silently does nothing on one platform is
//! worse than one that behaves the same everywhere. So: one press lights the
//! viewport, and it goes out on the next keystroke, the next turn, or
//! [`REVEAL_HOLD`], whichever comes first. Same feel, no protocol dependency.
//!
//! # Two known false positives, both the parser's own
//!
//! **Truncation.** In a Version 3 game `candle` and `candlesticks` are the same
//! six Z-characters, so a room holding a candle lights the word `candlesticks`
//! wherever it appears. That is not a defect here: `take candlesticks` really
//! does take the candle, because the parser truncates the player's word exactly
//! as the dictionary truncated its own. It is the game's behaviour, shown.
//!
//! **A word inside a word the story would have kept whole.** The words come from
//! the story's tokeniser but they are LOCATED in a drawn row by
//! [`lit_spans`], which accepts a match whose neighbours are not alphanumeric. A
//! story that does not declare `'` a separator holds `bird's` as one word; if
//! `bird` is separately in scope it will light inside `bird's` too. One column of
//! over-lighting, and the alternative is a second word splitter disagreeing with
//! the first.
//!
//! # Not raster
//!
//! Version 6 raster draws its text as bitmap glyphs on a canvas, where per-word
//! styling is a different problem with a different answer. This is scoped to the
//! ordinary cell text path; [`visible_text`] reads the CELL wrap cache, so on
//! raster nothing lights and [`arm`] says so rather than appearing to work.
//!
//! [`Engine::split_like_parser`]: crate::engine::Engine::split_like_parser

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use crate::state::AppState;

/// How long a reveal holds before it goes out on its own.
///
/// Long enough to read a screenful, short enough that it never feels like a mode
/// the player is stuck in. The other two exits — the next keystroke and the next
/// turn — are what actually ends most reveals; this is the one for a player who
/// pressed it and then did nothing.
pub const REVEAL_HOLD: Duration = Duration::from_millis(4_000);

/// How strong a claim this reveal is making, which is decided by what the engine
/// could be asked and not by what would be nice to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealTier {
    /// **A word lights because something in scope answers to it.**
    ///
    /// The real question, and the only one worth calling a reveal: not "is this
    /// word in the dictionary somewhere" but "does it name something HERE". Every
    /// object in the room and in the player's hands is asked
    /// [`ObjectWords::refers_to`](grammar_model::ObjectWords::refers_to), which
    /// compares against the words the story files the object under, truncated the
    /// way the story's dictionary truncates them. A sceptre named in room one and
    /// lying in room forty does not light.
    ///
    /// Reachable since SQ-1042 put `ObjectWords` through the `Introspect` seam.
    /// Unlabelled on screen: it is simply the truth.
    Scope,
    /// **A word lights because the dictionary holds it as a noun or adjective.**
    ///
    /// The fallback, for an engine with no `Introspect` and for a story whose
    /// objects cannot be read. Weaker in a way the player must be told about,
    /// because it cannot tell "implemented HERE" from "implemented SOMEWHERE" —
    /// the sceptre in room forty lights.
    ///
    /// So it says so, the way [`AppState::here_is_seen`](crate::state::AppState)
    /// makes the command band's scraped column label itself rather than pass a
    /// scrape off as the room's contents (SQ-1111 / SQ-1117). See
    /// [`RevealTier::caveat`].
    Dictionary,
}

impl RevealTier {
    /// What the player is told about a reveal at this strength, or `None` when
    /// there is nothing to admit.
    pub fn caveat(self) -> Option<&'static str> {
        match self {
            RevealTier::Scope => None,
            RevealTier::Dictionary => {
                Some("words this story knows — not necessarily things that are here")
            }
        }
    }
}

/// A reveal that is currently lit.
#[derive(Debug, Clone)]
pub struct Reveal {
    /// The spellings that light, exactly as the story's own tokeniser cut them
    /// out of the prose on screen — so `lantern` and not the `lanter` a Version 3
    /// dictionary stores, because it is the printed spelling the player is
    /// looking at.
    pub words: BTreeSet<String>,
    /// How strong a claim these words are (see [`RevealTier`]).
    pub tier: RevealTier,
    /// When it goes out on its own.
    pub until: Instant,
}

impl Reveal {
    /// Is this reveal still lit?
    pub fn is_lit(&self) -> bool {
        Instant::now() < self.until
    }
}

// ── Arming ──────────────────────────────────────────────────────────────────

/// What [`arm`] did, so the caller can say it out loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Armed {
    /// `n` words lit at this strength. The caveat, if the tier has one, is the
    /// caller's to relay.
    Lit { words: usize, tier: RevealTier },
    /// Nothing on screen is a word this story would accept. A real answer — a
    /// room of pure scenery gives it — and said plainly rather than silently.
    Nothing,
    /// There is no drawn text to read: no frame yet, or the v6 raster path, whose
    /// text is a bitmap and not a row of cells.
    NoText,
    /// The Guiding Light is out, and this is one of its lamps.
    GuidanceOff,
}

/// Light the words on screen that this story would accept, right now.
///
/// The order is the whole design: read what is DRAWN, cut it into words with the
/// STORY's tokeniser, and keep the ones the story's own world model answers to.
/// Nothing here consults English.
pub fn arm(state: &mut AppState, engine: &dyn Engine) -> Armed {
    // Under the Guiding Light's switch, like every other assist: a player who has
    // put the light out has said they do not want this kind of help (SQ-1045).
    // The control stays on the border beside the lamp, so the answer to "why did
    // nothing happen" is one click away from the question.
    if !state.config.guidance {
        state.reveal = None;
        return Armed::GuidanceOff;
    }

    let visible = visible_text(state);
    if visible.trim().is_empty() {
        state.reveal = None;
        return Armed::NoText;
    }

    // The story's own tokeniser where it lends one; `split_prose` is the
    // documented last resort for an engine that does not, and costs only an
    // unusual separator set — whatever comes out is still filtered below by the
    // story's own world model or its own dictionary.
    let tokens = engine
        .split_like_parser(&visible)
        .unwrap_or_else(|| crate::complete::split_prose(&visible));

    let (tier, words) = match crate::vocab::objects_in_scope(engine) {
        // The strong tier. An empty list is a real answer here — an empty room —
        // and NOT a reason to fall back: falling back would mean an empty room
        // lights the whole dictionary, which is the opposite of what was asked.
        Some(scope) => {
            let lit = tokens
                .iter()
                .filter(|t| scope.iter().any(|o| o.refers_to(t)))
                .cloned()
                .collect::<BTreeSet<String>>();
            (RevealTier::Scope, lit)
        }
        // The weak tier: the dictionary, filtered to the words that NAME things
        // — nouns and adjectives, minus the buzzword bit ($04), which is `the`,
        // `a`, `please` and their kin.
        //
        // A word carrying both the noun and the VERB bit — `light` in most of
        // Infocom's catalogue — does light, because the claim being made about it
        // here is the noun one.
        //
        // **And it inherits whatever the dictionary thinks a word is**, which is
        // the second half of why this tier is labelled and the strong one is not.
        // Mini-Zork files `west` with the DESC bit, exactly as it files `white`
        // and `boarded`, so no part-of-speech filter can tell the compass from a
        // colour; `north` and `south` carry neither bit and do not light at all.
        // There is no rescuing that from here — the flags are the story's answer
        // — so the caveat says what the tier is rather than pretending otherwise.
        None => {
            let Some(v) = state.vocab.get(engine) else {
                state.reveal = None;
                return Armed::Nothing;
            };
            let lit = tokens
                .iter()
                .filter(|t| v.roles(t).is_some_and(|r| (r.noun || r.adjective) && !r.special))
                .cloned()
                .collect::<BTreeSet<String>>();
            (RevealTier::Dictionary, lit)
        }
    };

    if words.is_empty() {
        state.reveal = None;
        return Armed::Nothing;
    }
    let n = words.len();
    state.reveal = Some(Reveal { words, tier, until: Instant::now() + REVEAL_HOLD });
    Armed::Lit { words: n, tier }
}

/// Put out whatever is lit. `true` when something actually went out (→ repaint).
pub fn clear(state: &mut AppState) -> bool {
    state.reveal.take().is_some()
}

/// Drop a reveal whose time is up. `true` when one did (→ repaint).
///
/// Called from the loop's expiry tick beside the sound pulse and the toasts,
/// which is what makes the hold a wall-clock hold rather than "until the next
/// event happens to arrive".
pub fn expire(state: &mut AppState) -> bool {
    if state.reveal.as_ref().is_some_and(|r| !r.is_lit()) {
        state.reveal = None;
        return true;
    }
    false
}

/// The text that is actually drawn in the story pane this frame, one string per
/// visible row.
///
/// Read from the CELL wrap cache windowed by the geometry the last frame
/// recorded — not from `AppState::transcript`, which is the whole scrollback and
/// would light words the player cannot see, and not from a re-wrap, which would
/// have to guess at a width the renderer already knows.
///
/// Empty before the first frame, and on the v6 RASTER path, whose text never
/// passes through this cache at all.
fn visible_text(state: &AppState) -> String {
    let Some(geom) = state.transcript_geom.get() else {
        return String::new();
    };
    let cache = state.transcript_wrap.borrow();
    let Some(entry) = cache.as_ref() else {
        return String::new();
    };
    entry
        .rows
        .iter()
        .skip(geom.first_abs_row)
        .take(geom.area.height as usize)
        .map(|r| r.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Painting ────────────────────────────────────────────────────────────────

/// Where in `text` each lit word was printed, as CHAR ranges, in order.
///
/// **This locates; it does not split.** The words were cut out of the prose by
/// the story's own tokeniser before they ever reached here, so the only question
/// left is where on this row the story printed them. A match counts when it is
/// bounded by non-alphanumeric characters on both sides — which is what stops
/// `rug` lighting inside `shrug`, the same anchoring
/// [`crate::vocab`]'s absent-noun check already uses.
///
/// Case-insensitive, per character rather than by lowercasing the whole row,
/// because a lowercase mapping may change a string's length and every offset
/// here has to stay an index into the ORIGINAL text.
pub fn lit_spans(text: &str, words: &BTreeSet<String>) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let pat: Vec<char> = w.chars().map(fold).collect();
        if pat.is_empty() || pat.len() > chars.len() {
            continue;
        }
        for start in 0..=(chars.len() - pat.len()) {
            let end = start + pat.len();
            if !chars[start..end].iter().map(|&c| fold(c)).eq(pat.iter().copied()) {
                continue;
            }
            let left_clear = start == 0 || !chars[start - 1].is_alphanumeric();
            let right_clear = end == chars.len() || !chars[end].is_alphanumeric();
            if left_clear && right_clear {
                out.push((start, end));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Re-style the lit words of one already-drawn row.
///
/// A pass OVER the drawn cells rather than a change to how they are drawn: the
/// reveal is a property of the moment, not of the text, and the transcript's
/// style runs are the game's own output (persisted in the archive, restored with
/// it). Folding a momentary highlight into them would mean writing a decoration
/// into a save file and then having to take it out again.
///
/// `x` is the row's first text column and `text` the string that was drawn
/// there; columns advance by each glyph's DISPLAY width, the same walk
/// `draw_str_runs` made, so a CJK glyph's two cells both light.
pub(crate) fn paint_row(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    area: ratatui::layout::Rect,
    state: &AppState,
) {
    let Some(reveal) = state.reveal.as_ref().filter(|r| r.is_lit()) else {
        return;
    };
    if y < area.y || y >= area.bottom() {
        return;
    }
    let spans = lit_spans(text, &reveal.words);
    if spans.is_empty() {
        return;
    }
    let style = state.colors.theme.get("transcript_reveal").style;
    let mut col = x;
    for (i, ch) in text.chars().enumerate() {
        if col >= area.right() {
            break;
        }
        let w = crate::textwidth::char_cells(ch) as u16;
        if spans.iter().any(|&(s, e)| i >= s && i < e) {
            for c in col..(col + w.max(1)).min(area.right()) {
                if let Some(cell) = buf.cell_mut((c, y)) {
                    let patched = cell.style().patch(style);
                    cell.set_style(patched);
                }
            }
        }
        col += w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<const N: usize>(words: [&str; N]) -> BTreeSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn a_lit_word_is_found_where_the_story_printed_it() {
        let spans = lit_spans("A brass lantern sits here.", &set(["lantern"]));
        assert_eq!(spans, vec![(8, 15)]);
    }

    #[test]
    fn matching_is_case_insensitive_and_offsets_index_the_original() {
        let spans = lit_spans("The Brass Lantern.", &set(["lantern", "brass"]));
        assert_eq!(spans, vec![(4, 9), (10, 17)]);
    }

    /// The anchoring that stops `rug` lighting inside `shrug` — the same rule
    /// `vocab::absent_nouns` uses, for the same reason.
    #[test]
    fn a_word_inside_another_word_does_not_light() {
        assert!(lit_spans("You shrug.", &set(["rug"])).is_empty());
        assert!(lit_spans("mailboxes", &set(["mailbox"])).is_empty());
        // …but the same word standing on its own does.
        assert_eq!(lit_spans("a rug", &set(["rug"])), vec![(2, 5)]);
    }

    #[test]
    fn every_occurrence_on_the_row_lights() {
        assert_eq!(lit_spans("door, door", &set(["door"])), vec![(0, 4), (6, 10)]);
    }

    #[test]
    fn overlapping_words_are_reported_once_each_and_in_order() {
        let spans = lit_spans("iron door", &set(["door", "iron"]));
        assert_eq!(spans, vec![(0, 4), (5, 9)], "sorted by position, not by word");
    }

    #[test]
    fn nothing_lit_is_no_spans() {
        assert!(lit_spans("You are in an open field.", &set([])).is_empty());
        assert!(lit_spans("", &set(["door"])).is_empty());
    }

    /// The strong tier claims nothing on screen; the weak one has to admit what
    /// it cannot tell apart.
    #[test]
    fn only_the_weak_tier_carries_a_caveat() {
        assert_eq!(RevealTier::Scope.caveat(), None);
        assert!(RevealTier::Dictionary.caveat().is_some_and(|s| s.contains("not necessarily")));
    }
}
