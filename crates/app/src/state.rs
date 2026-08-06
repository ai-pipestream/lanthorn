use std::time::{Duration, Instant};

// ── Hint system state ─────────────────────────────────────────────────────────

/// The source driving the open Hints panel.
///
/// `Zcode` wraps a second Z-machine session running the companion Invisiclues
/// (or any hint `.z5`) file.  The enum is a seam for future sources (e.g. UHS).
pub enum HintSource {
    /// A companion Invisiclues / hint program run as a second Z-machine session.
    Zcode(crate::session::GameSession),
}

// GameSession does not implement Debug, so we implement Debug manually for HintSource.
impl std::fmt::Debug for HintSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HintSource::Zcode(_) => write!(f, "HintSource::Zcode(<GameSession>)"),
        }
    }
}

/// Transient state for the Hints panel modal.
///
/// Held in `AppState.hints: Option<HintSession>` — `Some` while the panel is
/// open, `None` when closed.  The session is NOT persisted into the `.babelmap`
/// archive; only the per-IFID hint-file association is saved (Task A).
pub struct HintSession {
    /// The active hint source (currently always `Zcode`).
    pub source: HintSource,
    /// The hint program's own output (its scrollback transcript).
    pub transcript: Vec<String>,
    /// Scroll offset within the hint transcript (logical target).
    pub scroll: u16,
    /// Transcript index to truncate back to when the companion clears its
    /// screen for a menu redraw (menu-reprint collapse anchor). `None` until
    /// the first screen clear establishes an anchor.
    pub clear_anchor: Option<usize>,
    /// Smooth-scroll animation easing the *displayed* offset toward `scroll`.
    /// `None` when settled or animation is disabled (the instant path).
    pub scroll_anim: Option<ScrollAnim>,
    /// The hint panel's own input line (typed by the player).
    pub input: String,
    /// Dialog title, e.g. "Invisiclues: Zork I".
    pub label: String,
    /// When true, show the suggestion "This game has its own hints — type HINT".
    pub builtin_hint: bool,
}

impl HintSession {
    /// Scroll the transcript by `delta` rows, clamped to `[0, max]`, easing the
    /// displayed offset per the `[animation]` config (instant when disabled).
    ///
    /// `delta > 0` scrolls toward older content (matching the story transcript's
    /// wheel-up direction); `max` is the last-rendered maximum scroll offset.
    pub fn scroll_by(&mut self, delta: i32, max: u16, anim: &crate::config::AnimationConfig) {
        let from = self.effective_scroll() as usize;
        let next = (self.scroll as i32 + delta).clamp(0, max as i32) as u16;
        self.scroll = next;
        self.scroll_anim = ScrollAnim::to(from, next as usize, anim);
    }

    /// The displayed scroll offset this frame: the eased value while animating,
    /// else the logical target.
    pub fn effective_scroll(&self) -> u16 {
        self.scroll_anim
            .as_ref()
            .map(|a| a.current().round() as u16)
            .unwrap_or(self.scroll)
    }

    /// Drop a completed scroll animation (called from the run loop). Returns
    /// `true` iff a running animation was cleared this call, so the loop can
    /// force the one redraw that paints the settled offset. (SQ-0305)
    pub fn finalize_scroll_if_done(&mut self) -> bool {
        if self.scroll_anim.as_ref().is_some_and(|a| a.done()) {
            self.scroll_anim = None;
            true
        } else {
            false
        }
    }

    /// True while the displayed scroll offset is still easing.
    pub fn has_active_animation(&self) -> bool {
        self.scroll_anim.as_ref().is_some_and(|a| !a.done())
    }

    /// Fold one companion-VM turn's output into the panel transcript, mirroring the
    /// main session's game-driven handling: on a screen clear (menu redraw), collapse
    /// the previous reprint by truncating back to the anchor, then re-anchor; then
    /// append this turn's lines.
    ///
    /// A turn with no lower-window output (e.g. an upper-window menu keystroke that
    /// only moves the highlight) adds NOTHING — `"".split('\n')` yields one empty
    /// string, so a naive push would drop a blank line into the clue window on every
    /// keypress. Scroll only snaps to the newest content when something actually
    /// changed, so paging up to reread clues survives menu navigation.
    pub fn apply_turn(&mut self, result: &crate::session::TurnResult) {
        let mut changed = false;
        if result.erase_lower {
            let anchor = self.clear_anchor.unwrap_or(self.transcript.len());
            self.transcript.truncate(anchor);
            self.clear_anchor = Some(self.transcript.len());
            changed = true;
        }
        if !result.transcript.is_empty() {
            for line in result.transcript.split('\n') {
                self.transcript.push(line.to_owned());
            }
            changed = true;
        }
        if changed {
            self.scroll = 0;
            self.scroll_anim = None;
        }
    }
}

// GameSession does not implement Debug, so we implement Debug manually for HintSession.
impl std::fmt::Debug for HintSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HintSession")
            .field("source", &self.source)
            .field("transcript", &self.transcript)
            .field("scroll", &self.scroll)
            .field("input", &self.input)
            .field("label", &self.label)
            .field("builtin_hint", &self.builtin_hint)
            .finish()
    }
}

// ── Room panel ────────────────────────────────────────────────────────────────

/// Which display mode the room panel is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPanelMode {
    /// Story/game info view (name, notes, exits, objects for current room).
    Info,
    /// Layout diagnostics view (reuses draw_inspector).
    Diagnostics,
}

/// The currently-open room info/diagnostics panel, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomPanel {
    pub id: mapper::graph::RoomId,
    pub mode: RoomPanelMode,
}

// ── Drag-pan state ────────────────────────────────────────────────────────────

/// Middle-button drag-pan accumulator state.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    /// Terminal cell position of the last drag event.
    pub last: (u16, u16),
    /// Sub-cell accumulator for x (in terminal columns).
    pub acc_x: i32,
    /// Sub-cell accumulator for y (in terminal rows).
    pub acc_y: i32,
}

// ── Command band state ────────────────────────────────────────────────────────

use crate::render::command_band::{
    Arity, VerbEntry, BAND_COLS, COL_CARRIED, COL_HERE, COL_SECOND, COL_VERB,
};

/// Which grammatical slot a picked token fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandSlot {
    Verb,
    Object,
    Second,
}

/// One token the player has picked, in pick order — so Backspace is a pop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandPick {
    pub slot: BandSlot,
    pub text: String,
}

/// The token still under construction at the end of `input`: its trailing
/// whitespace-delimited word, or `""` when the line ends in whitespace (the
/// player finished that word, so the band should be looking at the NEXT slot
/// rather than still matching the last one).
pub fn band_typed_token(input: &str) -> &str {
    if input.ends_with(char::is_whitespace) {
        return "";
    }
    input.split_whitespace().next_back().unwrap_or("")
}

/// How well `item` matches the word being typed (already lowercased):
/// `0` = the item starts with it, `1` = one of the item's words starts with it
/// (so `do` finds `iron door`), `2` = it appears anywhere. `None` = no match at
/// all, which is what makes Tab a no-op rather than a wrong guess.
fn band_match_rank(item: &str, token_lower: &str) -> Option<u8> {
    let lower = item.to_lowercase();
    if lower.starts_with(token_lower) {
        return Some(0);
    }
    if lower.split_whitespace().any(|w| w.starts_with(token_lower)) {
        return Some(1);
    }
    if lower.contains(token_lower) {
        return Some(2);
    }
    None
}

/// One parse of the real input line into the band's grammar state.
struct ParsedPhrase {
    picks: Vec<BandPick>,
    /// The columns the grammar expects the NEXT word to come from — where the
    /// nearest-match highlight looks. Empty when nothing more is expected (a
    /// solo verb).
    expected: Vec<usize>,
}

/// The command band's whole state. `None` in `OverlayState.command_band` means
/// the band is closed.
///
/// `picks` is the grammar's bookkeeping — which slot has what, for arity and
/// column reachability — but the composed phrase ITSELF lives on the real
/// story input line (`state.input`), not a band-local phrase row (retired
/// 2026-08-05, SQ-0667): every successful pick mirrors onto `state.input`'s
/// tail (see `input::sync_band_phrase_to_input`, called from `apply_action`,
/// which is why that mirroring isn't done here — this type has no sibling
/// field access to `state.input`).
///
/// **SQ-0676 (2026-08-05) inverted the focus model.** The band never owns the
/// keyboard now: typing always reaches the story prompt, and `picks` is
/// re-derived from what is typed there ([`CommandBandState::sync_from_input`])
/// rather than only from clicks. There is therefore no type-to-filter state
/// and no story-focus flag left — the band is a REACTIVE suggestion surface:
/// it highlights the nearest match for the word under construction
/// ([`CommandBandState::nearest_match`]) and holds one armable quick-word
/// selection (`quick_sel`) that the arrow keys drive.
#[derive(Debug, Clone, Default)]
pub struct CommandBandState {
    /// The verb table in force (built-ins, or the config's replacement/extras).
    pub verbs: Vec<VerbEntry>,
    /// The one-click quick-action row.
    pub quick: Vec<String>,
    /// Tokens picked so far, in order.
    pub picks: Vec<BandPick>,
    /// The column the band is currently pointing at: the one holding the
    /// nearest match, else the first column the grammar expects next. Kept in
    /// step by `sync_from_input`; the keyboard no longer moves it (SQ-0676).
    pub focus: usize,
    /// The armed quick-word selection: an index into `quick`, or `None` when
    /// the highlight is disarmed. Arrow keys arm and move it; ANY change to
    /// the typed line disarms it (SQ-0676's "the last gesture decides").
    pub quick_sel: Option<usize>,
    /// Which quick layout was drawn last: `true` = the narrow flat row (linear
    /// arrow navigation), `false` = the compass-rose block (spatial). Written
    /// by the renderer, read by the arrow-key handler — the same
    /// render-informs-input pattern `AppState::input_text_origin` uses.
    pub quick_flat: std::cell::Cell<bool>,
    /// Per-column selection + animated scroll.
    pub scroll: [crate::list_scroll::ListScroll; BAND_COLS],
    /// Objects in the current room, refreshed every frame from the engine.
    pub here: Vec<String>,
    /// Objects the player carries, refreshed every frame from the engine.
    pub carried: Vec<String>,
    /// true when `here` came from the transcript scrape rather than the object
    /// tree (Glulx / Scott, which have no `Introspect`). The column relabels
    /// itself "seen" so the lower quality is visible instead of mixed in.
    pub here_is_seen: bool,
}

impl CommandBandState {
    pub fn new(verbs: Vec<VerbEntry>, quick: Vec<String>) -> Self {
        CommandBandState { verbs, quick, ..Default::default() }
    }

    // ── Phrase ───────────────────────────────────────────────────────────────

    fn slot_text(&self, slot: BandSlot) -> Option<&str> {
        self.picks.iter().find(|p| p.slot == slot).map(|p| p.text.as_str())
    }

    /// The table entry for `word`, matched case-insensitively (the player types
    /// the prompt now, and `Take` is the same verb as `take`).
    pub fn verb_by_word(&self, word: &str) -> Option<&VerbEntry> {
        self.verbs.iter().find(|v| v.word.eq_ignore_ascii_case(word))
    }

    /// The picked verb's table entry (grammar for everything downstream).
    pub fn verb_entry(&self) -> Option<&VerbEntry> {
        let w = self.slot_text(BandSlot::Verb)?;
        self.verb_by_word(w)
    }

    /// The picked verb's arity, if a verb is picked.
    ///
    /// A picked word that is not in the table is [`Arity::Solo`]. That can only
    /// come from the quick row — every column pick is drawn from the table —
    /// and a quick action IS the whole command (`n`, `again`), so it is complete
    /// as it stands. Without this, a quick row holding `n` (not a table verb,
    /// which spells it `north`) could never arm.
    pub fn arity(&self) -> Option<Arity> {
        let word = self.slot_text(BandSlot::Verb)?;
        Some(self.verb_by_word(word).map_or(Arity::Solo, |v| v.arity))
    }

    /// The preposition joining the two objects of the picked pair verb.
    pub fn prep(&self) -> Option<&str> {
        self.verb_entry().and_then(|v| v.prep.as_deref())
    }

    /// Materialize the phrase as the plain text the game will parse. Multi-word
    /// object names go in as-is: `unlock iron door with brass key` is exactly
    /// what a player would type, so nothing is quoted or escaped.
    pub fn phrase_text(&self) -> String {
        let mut out: Vec<&str> = Vec::new();
        if let Some(v) = self.slot_text(BandSlot::Verb) {
            out.push(v);
        }
        if let Some(o) = self.slot_text(BandSlot::Object) {
            out.push(o);
        }
        if let Some(s) = self.slot_text(BandSlot::Second) {
            if let Some(p) = self.prep() {
                out.push(p);
            }
            out.push(s);
        }
        out.join(" ")
    }

    /// Whether the phrase is grammatically complete — i.e. whether the text
    /// now sitting on the real story input line is a valid command on its
    /// own (Enter there sends it, exactly like anything typed by hand).
    pub fn complete(&self) -> bool {
        match self.arity() {
            None => false,
            Some(Arity::Solo) | Some(Arity::ObjectOpt) => true,
            Some(Arity::Object) => self.slot_text(BandSlot::Object).is_some(),
            Some(Arity::Pair) => {
                self.slot_text(BandSlot::Object).is_some()
                    && self.slot_text(BandSlot::Second).is_some()
            }
        }
    }

    // ── Columns ──────────────────────────────────────────────────────────────

    /// Whether `col` can be picked from yet, given what the phrase already has.
    /// Columns to the right of the active one stay unreachable until the
    /// grammar opens them.
    pub fn col_reachable(&self, col: usize) -> bool {
        match col {
            COL_VERB => true,
            COL_HERE | COL_CARRIED => !matches!(self.arity(), None | Some(Arity::Solo)),
            COL_SECOND => {
                matches!(self.arity(), Some(Arity::Pair))
                    && self.slot_text(BandSlot::Object).is_some()
            }
            _ => false,
        }
    }

    /// Whether `col`'s grammatical slot is already filled.
    pub fn col_filled(&self, col: usize) -> bool {
        match col {
            COL_VERB => self.slot_text(BandSlot::Verb).is_some(),
            COL_HERE | COL_CARRIED => self.slot_text(BandSlot::Object).is_some(),
            COL_SECOND => self.slot_text(BandSlot::Second).is_some(),
            _ => false,
        }
    }

    /// Which slot a pick in `col` fills.
    pub fn col_slot(col: usize) -> BandSlot {
        match col {
            COL_VERB => BandSlot::Verb,
            COL_SECOND => BandSlot::Second,
            _ => BandSlot::Object,
        }
    }

    /// The column's header text.
    pub fn column_label(&self, col: usize) -> String {
        match col {
            COL_VERB => "VERB".to_string(),
            COL_HERE => {
                if self.here_is_seen {
                    "WHAT — seen".to_string()
                } else {
                    "WHAT — here".to_string()
                }
            }
            COL_CARRIED => "WHAT — carried".to_string(),
            COL_SECOND => match self.prep() {
                Some(p) => format!("{}…", p.to_uppercase()),
                None => "WITH…".to_string(),
            },
            _ => String::new(),
        }
    }

    /// The column's items before filtering.
    ///
    /// `COL_VERB` excludes any word that is also in the quick row (SQ-0667):
    /// showing `look`/`wait`/`again`/etc. in both places is redundant, and the
    /// quick row is one click away regardless. The exclusion follows
    /// `self.quick` — the *effective* list (the user's configured `quick` when
    /// set, else the built-in row) — so removing a word from a custom `quick`
    /// puts it back here. Direction words compare by the direction they name,
    /// not by spelling, so the quick row's `n` excludes the table's `north`
    /// (the compass otherwise appeared in both places).
    pub fn items(&self, col: usize) -> Vec<String> {
        use mapper::direction::parse_direction;
        let same_word = |q: &str, w: &str| {
            q.eq_ignore_ascii_case(w)
                || matches!(
                    (parse_direction(q), parse_direction(w)),
                    (Some(a), Some(b)) if a == b
                )
        };
        match col {
            COL_VERB => self
                .verbs
                .iter()
                .filter(|v| !self.quick.iter().any(|q| same_word(q, &v.word)))
                .map(|v| v.word.clone())
                .collect(),
            COL_HERE => self.here.clone(),
            COL_CARRIED => self.carried.clone(),
            COL_SECOND => {
                // Carried first (the usual instrument), then anything here that
                // isn't already listed.
                let mut out = self.carried.clone();
                for h in &self.here {
                    if !out.iter().any(|c| c.eq_ignore_ascii_case(h)) {
                        out.push(h.clone());
                    }
                }
                out
            }
            _ => Vec::new(),
        }
    }

    // ── Reading the typed line (SQ-0676) ─────────────────────────────────────

    /// Parse the real input line into the band's grammar state.
    ///
    /// The phrase is anchored on the FIRST typed token that is a table verb;
    /// anything before it is free text the player wrote (`well, take mailbox`)
    /// and is deliberately ignored rather than mistaken for a verb. Everything
    /// after it fills the object slot — split at the pair verb's preposition
    /// when one has been typed — which is also why the parse never counts
    /// tokens: object names are routinely multi-word (`iron door`).
    ///
    /// The word still under construction counts as a token here, unlike in
    /// [`Self::nearest_match`]: a bare `take` with no trailing space is a
    /// chosen verb (it is exactly what a click on the VERB column leaves on the
    /// prompt), so the object columns must open on it rather than waiting for a
    /// space that a mouse-only player would never type.
    fn parse_phrase(&self, input: &str) -> ParsedPhrase {
        let toks: Vec<&str> = input.split_whitespace().collect();
        let Some(vi) = toks.iter().position(|t| self.verb_by_word(t).is_some()) else {
            return ParsedPhrase { picks: Vec::new(), expected: vec![COL_VERB] };
        };
        let entry = self.verb_by_word(toks[vi]).expect("just matched");
        let (arity, prep) = (entry.arity, entry.prep.clone());
        // Store the TABLE's spelling, so downstream lookups (`arity`, `prep`)
        // and `phrase_text` are canonical regardless of how it was typed.
        let mut picks = vec![BandPick { slot: BandSlot::Verb, text: entry.word.clone() }];
        let rest = &toks[vi + 1..];
        let push = |picks: &mut Vec<BandPick>, slot, text: String| {
            if !text.is_empty() {
                picks.push(BandPick { slot, text });
            }
        };
        let expected = match arity {
            Arity::Solo => Vec::new(),
            Arity::Object | Arity::ObjectOpt => {
                push(&mut picks, BandSlot::Object, rest.join(" "));
                vec![COL_HERE, COL_CARRIED]
            }
            Arity::Pair => {
                let p = prep.unwrap_or_default();
                match rest.iter().position(|t| t.eq_ignore_ascii_case(&p)) {
                    Some(j) => {
                        push(&mut picks, BandSlot::Object, rest[..j].join(" "));
                        push(&mut picks, BandSlot::Second, rest[j + 1..].join(" "));
                        vec![COL_SECOND]
                    }
                    None => {
                        push(&mut picks, BandSlot::Object, rest.join(" "));
                        vec![COL_HERE, COL_CARRIED]
                    }
                }
            }
        };
        ParsedPhrase { picks, expected }
    }

    /// Re-derive the phrase state from the real input line and point the band
    /// at whatever it should be suggesting (SQ-0676). Called after every change
    /// to `state.input`, so the columns follow typing exactly as they follow
    /// clicking — a click composes onto the prompt, and the prompt is what this
    /// reads back.
    pub fn sync_from_input(&mut self, input: &str) {
        let parsed = self.parse_phrase(input);
        self.picks = parsed.picks;
        self.focus = self
            .nearest_match(input)
            .map(|(col, _)| col)
            .or_else(|| parsed.expected.first().copied())
            .unwrap_or(COL_VERB);
    }

    /// The columns the grammar expects the next word to come from: VERB for the
    /// first word, the object columns after an object/pair verb, the
    /// prepositional column once that verb's preposition has been typed. Empty
    /// once nothing more is expected (a solo verb).
    pub fn expected_cols(&self, input: &str) -> Vec<usize> {
        self.parse_phrase(input).expected
    }

    /// The nearest match for the word being typed, as `(column, index)`:
    /// prefix first, then a prefix on any word of a multi-word name, then a
    /// substring — searching only the columns [`Self::expected_cols`] says are
    /// live, earliest column and earliest row winning ties. `None` when nothing
    /// matches (no highlight, and Tab does nothing).
    pub fn nearest_match(&self, input: &str) -> Option<(usize, usize)> {
        let token = band_typed_token(input).to_lowercase();
        if token.is_empty() {
            return None;
        }
        // (rank, column order, row index, column) — ordered exactly by the
        // tie-break the doc promises.
        let mut best: Option<(u8, usize, usize, usize)> = None;
        for (order, &col) in self.expected_cols(input).iter().enumerate() {
            for (idx, item) in self.items(col).iter().enumerate() {
                let Some(rank) = band_match_rank(item, &token) else { continue };
                let cand = (rank, order, idx, col);
                if best.is_none_or(|b| cand < b) {
                    best = Some(cand);
                }
            }
        }
        best.map(|(_, _, idx, col)| (col, idx))
    }

    /// The text of [`Self::nearest_match`] — what Tab completes the current
    /// word to while the band is open.
    pub fn nearest_match_text(&self, input: &str) -> Option<String> {
        let (col, idx) = self.nearest_match(input)?;
        self.items(col).get(idx).cloned()
    }

    // ── Focus ────────────────────────────────────────────────────────────────

    /// Every reachable column, left to right. Once the ring the arrow keys
    /// walked (retired with the keyboard by SQ-0676 — the arrows drive the
    /// quick block now); still what `advance` clamps against, so the band never
    /// points at a column `col_reachable` disagrees with.
    pub fn focus_stops(&self) -> Vec<usize> {
        (0..BAND_COLS).filter(|&c| self.col_reachable(c)).collect()
    }

    /// Move focus to the next reachable UNFILLED column, or — when there is
    /// nothing left to pick — the last reachable column, so the cursor never
    /// parks somewhere `col_reachable` no longer agrees with (e.g. picking a
    /// Solo verb collapses the ring back down to just VERB).
    pub fn advance(&mut self) {
        for c in 0..BAND_COLS {
            if self.col_reachable(c) && !self.col_filled(c) {
                self.focus = c;
                return;
            }
        }
        self.focus = self.focus_stops().into_iter().next_back().unwrap_or(COL_VERB);
    }

    // ── Picking ──────────────────────────────────────────────────────────────

    fn set_slot(&mut self, slot: BandSlot, text: String) {
        // Filling a slot invalidates everything picked after it: choosing a new
        // verb must not leave the old verb's object stranded in the phrase.
        if let Some(pos) = self.picks.iter().position(|p| p.slot == slot) {
            self.picks.truncate(pos);
        }
        self.picks.push(BandPick { slot, text });
    }

    /// Pick row `idx` of column `col`. No-op when the column is unreachable or
    /// the index is out of range.
    pub fn pick(&mut self, col: usize, idx: usize) {
        if !self.col_reachable(col) {
            return;
        }
        let Some(text) = self.items(col).get(idx).cloned() else { return };
        self.set_slot(Self::col_slot(col), text);
        self.advance();
    }

    /// Pick a verb by word (the quick row, and tests). Replaces the phrase
    /// wholesale: a quick action is a fresh command, not an addition.
    pub fn pick_word(&mut self, word: &str) {
        self.picks.clear();
        self.set_slot(BandSlot::Verb, word.to_string());
        self.advance();
    }

    /// Drop the whole phrase and return to the verb column. (Backspace's
    /// un-pick ladder retired with the band's keyboard ownership — SQ-0676:
    /// Backspace edits the real prompt, and `sync_from_input` re-derives the
    /// phrase from what is left there.)
    pub fn clear_phrase(&mut self) {
        self.picks.clear();
        self.focus = COL_VERB;
    }

    /// Whether any of the band's per-column scrolls is animating.
    pub fn has_active_animation(&self) -> bool {
        self.scroll.iter().any(|s| s.has_active_animation())
    }
}

/// Maximum number of submitted command lines retained in `command_history`.
pub const COMMAND_HISTORY_CAP: usize = 500;

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::LayerId;

/// How many walked rooms the maze breadcrumb remembers (SQ-0666). Eight is about as far back as
/// "which way did I come in?" is still a live question in a maze; past that the trail is just a
/// second highlight competing with the here-marker.
pub const MAP_TRAIL_LEN: usize = 8;

// ── Transcript kind ───────────────────────────────────────────────────────────

/// Category tag for each transcript entry.
///
/// `Story` = game output. `Input` = the player's echoed command. `Meta` =
/// app/slash output. `Warning` = VM diagnostics. The `/filter` view is coarse
/// (story = Story+Input, meta = Meta+Warning); the styling is per-variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptKind {
    Story,
    Input,
    Meta,
    Warning,
}

/// Push a turn's ordered elements into the transcript: text runs via
/// `push_transcript_runs`, images via `push_transcript_image`, in order. The
/// Glulx run-loop path uses this when a `TurnResult` carries interleaved inline
/// images; the plain-text path stays on `push_transcript_runs`.
pub fn apply_transcript_elems(state: &mut AppState, elems: &[crate::session::TranscriptElem]) {
    use crate::session::TranscriptElem;
    for e in elems {
        match e {
            TranscriptElem::Text { text, runs } => {
                state.push_transcript_runs(text, TranscriptKind::Story, runs);
            }
            TranscriptElem::Image(img) => state.push_transcript_image(img.clone()),
        }
    }
}

/// A run of characters in a transcript line carrying Z-machine text-style bits and colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StyleRun {
    pub start: usize, // char offset within the line, inclusive
    pub end: usize,   // char offset, exclusive
    pub bits: u8,     // 1=reverse, 2=bold, 4=italic, 8=fixed
    #[serde(default)]
    pub fg: u32, // packed ZColour (see pack_zcolour / unpack_zcolour); 0 = Default
    #[serde(default)]
    pub bg: u32, // packed ZColour; 0 = Default
    #[serde(default)]
    pub link: u32, // Glk hyperlink value for this span (0 = no link)
    #[serde(default)]
    pub glk_style: u8, // Glk style class (0=Normal .. 10=User2); indexes the theme's per-style colour slot (SQ-0331)
}

/// Per-paragraph layout formatting for one transcript line, derived from the Glk
/// paragraph stylehints (SQ-0330). A logical transcript line is one paragraph, so
/// this is stored parallel to `transcript`. `Default` = left-flush, no indent —
/// the Z-machine and any buffer that set no layout hints render exactly as before.
/// The renderer turns these into LEADING-SPACE padding in the wrap (so
/// selection/copy/search coordinates stay consistent with what is drawn).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParaFmt {
    /// Left indent applied to every wrapped row of the paragraph, in cells.
    #[serde(default)]
    pub indent: u16,
    /// Extra indent applied to the FIRST wrapped row only (Glk ParaIndentation);
    /// may be negative for a hanging indent.
    #[serde(default)]
    pub para_indent: i16,
    /// Justification: 0=left-flush (default), 1=fill (rendered as left for now),
    /// 2=centered, 3=right-flush.
    #[serde(default)]
    pub justify: u8,
    /// Char offset within the line at/after which the game had output BUFFERING
    /// OFF (`buffer_mode 0`, ZMSD §7.2/§7.2.1): from there on the text must break
    /// after the last character that fits, never word-wrap. `None` (the default)
    /// = fully buffered, i.e. ordinary word-wrap.
    ///
    /// Per-line (a logical transcript line is one paragraph), derived by
    /// [`AppState::push_transcript_runs`] from the per-run flag `CaptureSink`
    /// records. It is a wrap POLICY, not a style, which is why it lives here
    /// beside the other paragraph layout rather than in [`StyleRun`].
    #[serde(default)]
    pub nowrap_from: Option<u32>,
}

/// Encode a [`zvm::screen::ZColour`] as a packed `u32` for serde-safe storage in
/// [`StyleRun`] (zvm is zero-dep and cannot derive serde).
///
/// Scheme (tag in the high byte): `Default` → 0, `Standard(n)` → `(1 << 24) | n`,
/// `True(v)` → `(2 << 24) | v`, `True24(rgb)` → `(3 << 24) | rgb` (24-bit RGB
/// occupies the low 24 bits exactly).
pub fn pack_zcolour(c: zvm::screen::ZColour) -> u32 {
    use zvm::screen::ZColour;
    match c {
        ZColour::Default    => 0,
        ZColour::Standard(n) => (1 << 24) | n as u32,
        ZColour::True(v)    => (2 << 24) | v as u32,
        ZColour::True24(v)  => (3 << 24) | (v & 0x00FF_FFFF),
    }
}

/// Decode a packed `u32` back to a [`zvm::screen::ZColour`].  Unknown tags → `Default`.
pub fn unpack_zcolour(p: u32) -> zvm::screen::ZColour {
    use zvm::screen::ZColour;
    match p >> 24 {
        1 => ZColour::Standard((p & 0xFF) as u8),
        2 => ZColour::True((p & 0xFFFF) as u16),
        3 => ZColour::True24(p & 0x00FF_FFFF),
        _ => ZColour::Default,
    }
}

/// Map a Blorb sound kind to a backend format, or None for unsupported kinds.
pub fn sound_kind_to_format(k: blorb::SoundKind) -> Option<audio::SoundFormat> {
    match k {
        blorb::SoundKind::Aiff => Some(audio::SoundFormat::Aiff),
        blorb::SoundKind::Ogg => Some(audio::SoundFormat::Ogg),
        blorb::SoundKind::Mod => Some(audio::SoundFormat::Mod),
        blorb::SoundKind::Other => None,
    }
}

/// Map a Glk channel volume (`0x10000` = full; may exceed for amplification) to a
/// linear pre-master gain fraction for the audio backend.
pub fn glk_volume_to_gain(vol: u32) -> f32 {
    vol as f32 / 65536.0
}

/// One in-flight Sound2 volume ramp (`glk_schannel_set_volume_ext` with a nonzero
/// duration): the host interpolates the channel's linear gain from `start_gain`
/// to `target_gain` over `duration_ms`, starting at `start`.
#[derive(Clone, Copy, Debug)]
pub struct VolumeRamp {
    pub start: std::time::Instant,
    pub duration_ms: u32,
    pub start_gain: f32,
    pub target_gain: f32,
}

/// Linearly interpolate a ramping gain: `start_gain` at `elapsed_ms == 0`,
/// `target_gain` once `elapsed_ms >= duration_ms` (and for a zero duration).
pub fn ramp_gain(start_gain: f32, target_gain: f32, elapsed_ms: u32, duration_ms: u32) -> f32 {
    if duration_ms == 0 || elapsed_ms >= duration_ms {
        return target_gain;
    }
    let t = elapsed_ms as f32 / duration_ms as f32;
    start_gain + (target_gain - start_gain) * t
}

/// Map Glk `repeats` to the audio backend's `repeats` byte, or `None` to skip
/// playing entirely. Glk: `0xFFFFFFFF` = loop forever; `0` = play zero times;
/// `N` = N plays. The audio byte reserves `255` for "forever", so finite counts
/// are clamped to `254`.
fn glk_repeats_to_audio(repeats: u32) -> Option<u8> {
    match repeats {
        0 => None,
        0xFFFF_FFFF => Some(255),
        n if n >= 255 => Some(254),
        n => Some(n as u8),
    }
}

/// Short display label for a [`blorb::SoundKind`].
pub fn sound_kind_label(k: blorb::SoundKind) -> &'static str {
    match k {
        blorb::SoundKind::Aiff => "AIFF",
        blorb::SoundKind::Ogg => "OGG",
        blorb::SoundKind::Mod => "MOD",
        blorb::SoundKind::Other => "other",
    }
}

/// Format the `Snd ` resources of a (possibly absent) sound Blorb for the
/// `/play-sound` diagnostic's list mode (no argument).
pub fn format_sound_resource_list(blorb: Option<&blorb::Blorb>) -> Vec<String> {
    let Some(blorb) = blorb else {
        return vec!["no sound blorb resolved".to_string()];
    };
    let sounds: Vec<_> = blorb.resources().iter().filter(|r| &r.usage == b"Snd ").collect();
    if sounds.is_empty() {
        return vec!["no Snd resources".to_string()];
    }
    let mut lines = vec![format!("{} sound resource(s):", sounds.len())];
    for r in sounds {
        let kind = match &r.chunk_type {
            b"FORM" => blorb::SoundKind::Aiff,
            b"OGGV" => blorb::SoundKind::Ogg,
            b"MOD " => blorb::SoundKind::Mod,
            _ => blorb::SoundKind::Other,
        };
        let playable = if sound_kind_to_format(kind).is_some() { "playable" } else { "not decodable" };
        lines.push(format!("  #{}  {}  {} bytes  {playable}", r.number, sound_kind_label(kind), r.len));
    }
    lines
}

/// Step-by-step report for the `/play-sound <n>` diagnostic, built by the
/// (impure) glue in `main.rs` and rendered here as plain transcript lines.
#[derive(Debug, Default, Clone)]
pub struct PlaySoundReport {
    pub number: u32,
    pub enable_sound: bool,
    pub backend_present: bool,
    pub blorb_present: bool,
    pub resource: Option<(blorb::SoundKind, usize)>,
    pub format: Option<audio::SoundFormat>,
    pub sound_id: Option<audio::SoundId>,
}

/// Render a [`PlaySoundReport`] as transcript lines, stopping early at the
/// first failed stage (resource not found, or format not decodable).
pub fn format_play_sound_report(r: &PlaySoundReport) -> Vec<String> {
    let mut lines = vec![format!("/play-sound {}", r.number)];
    lines.push(format!(
        "enable_sound: {}",
        if r.enable_sound { "on" } else { "off (attempting playback anyway — diagnostic)" }
    ));
    lines.push(format!("audio backend: {}", if r.backend_present { "present" } else { "NONE" }));
    lines.push(format!("sound blorb: {}", if r.blorb_present { "resolved" } else { "NONE" }));
    let Some((kind, len)) = r.resource else {
        lines.push(format!("resource #{}: NOT FOUND", r.number));
        return lines;
    };
    lines.push(format!("resource #{}: found, kind={:?}, {len} bytes", r.number, kind));
    let Some(format) = r.format else {
        lines.push("format: not decodable".to_string());
        return lines;
    };
    lines.push(format!("format: {format:?} — decodable"));
    match r.sound_id {
        Some(id) => lines.push(format!("playback: started (sound id {id})")),
        None => lines.push("playback: backend returned None".to_string()),
    }
    lines
}

// ── Transcript filter ─────────────────────────────────────────────────────────

/// Which categories of transcript entries are currently visible.
///
/// `Both` (the default) shows all entries. `Story` shows only game output.
/// `Meta` shows only app-generated output (slash commands, /help, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TranscriptFilter {
    #[default]
    Both,
    Story,
    Meta,
}

// ── Tidy animation ────────────────────────────────────────────────────────────

/// One captured stage of the tidy pipeline, held for playback. `graph` is a clone
/// of the layout as it stood after the named stage ran.
#[derive(Debug, Clone)]
pub struct TidyFrame {
    pub label: String,
    pub graph: MapGraph,
    pub description: String,
    pub stats: mapper::layout::TidyStats,
    pub stage_start: bool,
    /// When `Some`, the map pane renders these lines as text (the Build frame's
    /// connection manifest) instead of drawing rooms. `None` for every layout stage.
    pub manifest: Option<Vec<String>>,
}

/// Transient playback state for the tidy animation. While this is `Some`, the map
/// pane renders the current frame's graph instead of the live one. Playback holds
/// on the final frame; `Esc` clears it back to the live map.
#[derive(Debug)]
pub struct TidyAnim {
    pub frames: Vec<TidyFrame>,
    pub idx: usize,
    pub playing: bool,
    /// The layer these frames tidy. Carried explicitly because a frame's graph CANNOT answer it:
    /// every frame is a `layer_subgraph`, whose rooms keep their real layer while its `layers()`
    /// map always reports main-only — so asking the frame yields `MAIN_LAYER`, a layer it holds no
    /// rooms for, and the map draws blank (SQ-0359).
    pub layer: LayerId,
    last_advance: Instant,
}

impl TidyAnim {
    pub fn new(frames: Vec<TidyFrame>, layer: LayerId) -> Self {
        Self { frames, idx: 0, playing: true, layer, last_advance: Instant::now() }
    }

    pub fn current(&self) -> &TidyFrame {
        &self.frames[self.idx]
    }

    fn at_end(&self) -> bool {
        self.idx + 1 >= self.frames.len()
    }

    /// Step `delta` frames (clamped to range) and pause — manual control overrides playback.
    pub fn step(&mut self, delta: isize) {
        let last = self.frames.len().saturating_sub(1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle play/pause; resuming restarts the dwell clock so the current frame holds full time.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one frame if playing and `dwell` has elapsed since the last advance. Stops (holds)
    /// at the final frame. Returns true if the frame index changed.
    pub fn tick(&mut self, dwell: Duration) -> bool {
        if !self.playing || self.at_end() {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.at_end() {
            self.playing = false;
        }
        true
    }
}

// ── Replay / rewind ───────────────────────────────────────────────────────────

/// Transient state for the rewind/replay modal. While `Some`, the map pane
/// renders the reconstructed snapshot for `idx` instead of the live graph
/// (like `TidyAnim`). `Esc`/`q` clears it back to the live game with no change.
#[derive(Debug)]
pub struct ReplayState {
    /// Selected turn index into `AppState.history`. The source of truth that
    /// drives map-snapshot reconstruction; `scroll` mirrors it for list display.
    pub idx: usize,
    pub playing: bool,
    last_advance: Instant,
    /// Animated list scroll, synced to `idx` each frame for windowing + scrollbar.
    pub scroll: crate::list_scroll::ListScroll,
}

impl ReplayState {
    /// Open seeded at the last turn (`last_idx`), paused.
    pub fn new(last_idx: usize) -> Self {
        let mut scroll = crate::list_scroll::ListScroll::new();
        scroll.selected = last_idx;
        Self { idx: last_idx, playing: false, last_advance: Instant::now(), scroll }
    }

    /// Step `delta` turns (clamped to `[0, len-1]`) and pause.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 { self.idx = 0; self.playing = false; return; }
        let last = (len - 1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle auto-play; resuming restarts the dwell clock.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one turn if playing and `dwell` elapsed; holds at the last turn.
    /// Returns true if `idx` changed.
    pub fn tick(&mut self, dwell: Duration, len: usize) -> bool {
        if !self.playing || len == 0 || self.idx + 1 >= len {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.idx + 1 >= len {
            self.playing = false;
        }
        true
    }
}

// ── Sound pulse ──────────────────────────────────────────────────────────────

/// A host-side bleep classification for the border-pulse visual cue.
#[derive(Debug, Clone, Copy)]
pub enum BeepKind {
    High,
    Low,
}

/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
#[derive(Debug)]
pub struct SoundPulse {
    pub kind: BeepKind,
    pub started: std::time::Instant,
}

// ── Smooth transcript scroll ─────────────────────────────────────────────────

/// An in-flight smooth-scroll animation over a `usize` row offset. The logical
/// target (e.g. `transcript_scroll`, a `ListScroll` offset) is updated
/// immediately by the caller; this eases the *displayed* offset from `from` to
/// `to` over the tween. Driven by the run loop, which snaps to `to` and clears
/// this once the tween is `done()`. The single animated-offset type, reused by
/// the transcript and by `ListScroll`.
#[derive(Debug, Clone)]
pub struct ScrollAnim {
    /// Displayed offset (rows) when the animation was armed.
    pub from: usize,
    /// Target offset (rows) the animation eases toward.
    pub to: usize,
    /// The timing curve.
    pub tween: crate::anim::Tween,
}

impl ScrollAnim {
    /// Arm an animation easing the displayed offset from `from` to `to` per the
    /// `[animation]` config. Returns `None` when animation is disabled or
    /// `scroll_ms == 0` (the caller should jump instantly and clear any anim) —
    /// the byte-for-byte instant path.
    pub fn to(from: usize, to: usize, cfg: &crate::config::AnimationConfig) -> Option<Self> {
        if !cfg.enabled || cfg.scroll_ms == 0 {
            return None;
        }
        Some(Self {
            from,
            to,
            tween: crate::anim::Tween::new(Duration::from_millis(cfg.scroll_ms), cfg.easing),
        })
    }

    /// The current displayed offset: `lerp(from, to, tween.progress())`.
    pub fn current(&self) -> f64 {
        crate::anim::lerp(self.from as f64, self.to as f64, self.tween.progress())
    }

    /// The settled target offset.
    pub fn target(&self) -> usize {
        self.to
    }

    /// True once the tween has reached its duration.
    pub fn done(&self) -> bool {
        self.tween.done()
    }
}

// ── Background tidy job ───────────────────────────────────────────────────────

/// What a background [`TidyJob`] does. Both run off the main thread and pulse the
/// map border; they differ only in how much they move rooms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TidyKind {
    /// Full relayout + overlap cleanup + hint repair + compaction. Config-driven
    /// (the `background_tidy` setting) — rearranges the whole layer for aesthetics.
    Full,
    /// Overlap cleanup only — nudges rooms just enough to remove rendered overlaps,
    /// preserving the existing layout. Runs on EVERY geometry change regardless of
    /// the `background_tidy` setting, so the map is never left showing an overlap
    /// (the guarantee the old synchronous `apply_turn` cleanup used to provide).
    Cleanup,
}

/// An in-flight background tidy job. The worker thread runs the relayout on a
/// clone of the graph and returns the tidied clone. The run loop polls
/// `handle.is_finished()` each iteration and joins when done.
pub struct TidyJob {
    /// Worker thread handle. Returns the tidied graph clone on success.
    pub handle: std::thread::JoinHandle<mapper::graph::MapGraph>,
    /// The layer being tidied.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn time. Used to detect stale results.
    pub gen: u64,
    /// Instant the job was spawned. Used to compute the pulse phase for the border color.
    pub started: std::time::Instant,
    /// Full relayout vs. overlap-cleanup-only — governs the stale re-trigger.
    pub kind: TidyKind,
}

impl std::fmt::Debug for TidyJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TidyJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// An in-flight background map-render job (SQ-0379). The worker builds the routed
/// `RenderMap` for `(gen, layer)` off the main thread — the routing is the
/// expensive part — and pushes a short label into `steps` at the start of each
/// phase so the map pane can show a live progress trace. The run loop polls
/// `handle.is_finished()` and installs the result when `gen` still matches.
pub struct RenderJob {
    /// Worker thread handle. Returns the routed `RenderMap`.
    pub handle: std::thread::JoinHandle<mapper::render::RenderMap>,
    /// The layer this model is being routed for.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn; a mismatch on completion means the
    /// map changed again mid-build, so the result is stale and a fresh job runs.
    pub gen: u64,
    /// Instant the job was spawned, for the border pulse phase.
    pub started: std::time::Instant,
}

impl std::fmt::Debug for RenderJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

/// An in-flight background job that builds a tidy *animation* off the main thread.
/// The worker runs `run_tidy_pipeline` on a clone of the graph and returns both the
/// captured frames and the mutated (tidied) clone. The run loop polls
/// `handle.is_finished()`, then applies the tidied graph (with a staleness check) and
/// installs the animation. Unlike `TidyJob`, this is NOT an overlay — input keeps
/// flowing while the frames are built.
pub struct AnimBuildJob {
    /// Worker thread handle. Returns the captured frames and the tidied graph clone.
    pub handle: std::thread::JoinHandle<(Vec<TidyFrame>, mapper::graph::MapGraph)>,
    /// The layer being tidied.
    pub layer: mapper::layer::LayerId,
    /// Graph generation recorded at spawn time. Used to detect stale results.
    pub gen: u64,
    /// Instant the job was spawned.
    pub started: std::time::Instant,
    /// Frames captured so far, bumped once per emitted frame by the worker. Read by the
    /// renderer to drive the progress bar while the build runs.
    pub progress: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Estimated final frame count (room count + headroom). The bar is approximate: the
    /// real frame total isn't known until the build finishes, so this is only an estimate.
    pub total: usize,
    /// Whether to install the tidy animation when the build finishes. `animate-tidy`
    /// sets this `true` (play the frames); the instant `tidy-map`/`Retidy` re-tidy sets
    /// it `false` — it reuses the same off-thread build purely to surface the progress
    /// bar, then applies the tidied graph without an animation. (SQ-0261)
    pub animate: bool,
}

impl std::fmt::Debug for AnimBuildJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimBuildJob")
            .field("layer", &self.layer)
            .field("gen", &self.gen)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

// ── Saves manager state ───────────────────────────────────────────────────────

/// Transient state for the saves-manager modal.
/// `None` in `AppState.saves` = modal closed.
#[derive(Debug, Clone)]
pub struct SavesState {
    /// All discovered save files for the current story (default first, then named).
    pub entries: Vec<crate::persist_files::SaveInfo>,
    /// Selection + animated scroll offset for the entry list.
    pub scroll: crate::list_scroll::ListScroll,
}

// ── File picker state (read-mode create_by_prompt) ─────────────────────────────

/// A minimal list picker over existing VFS filenames, for a read-mode
/// `create_by_prompt`. Rendered with the shared dialog chrome. `None` in
/// `AppState.file_picker` = closed.
#[derive(Debug, Clone)]
pub struct FilePickerState {
    pub names: Vec<String>,
    pub scroll: crate::list_scroll::ListScroll,
}

impl FilePickerState {
    pub fn new(names: Vec<String>) -> Self {
        FilePickerState { names, scroll: Default::default() }
    }
    pub fn selected(&self) -> Option<&str> {
        self.names.get(self.scroll.selected).map(|s| s.as_str())
    }
    pub fn move_up(&mut self) {
        self.scroll.selected = self.scroll.selected.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        if self.scroll.selected + 1 < self.names.len() {
            self.scroll.selected += 1;
        }
    }
}

// ── Command palette state ───────────────────────────────────────────────────────

/// Transient state for the command-palette popup (SQ-0419): a fuzzy-searchable
/// list of every registry command, usable even where no text prompt exists
/// (modal / debug views). `None` in `AppState.overlays.palette` = closed.
///
/// The palette owns its own input line ([`input`](Self::input)): its first
/// whitespace token is the fuzzy *query* that filters commands; anything after it
/// is passed to the chosen command as *arguments*. Enter runs the selected
/// candidate with those args through the normal slash dispatch path; Esc closes.
#[derive(Debug, Clone)]
pub struct PaletteState {
    /// The palette's own editable line (query + optional args), with a caret.
    pub input: crate::text_field::TextField,
    /// Selection + animated scroll over the current filtered candidate list.
    pub scroll: crate::list_scroll::ListScroll,
    /// True when the palette was promoted from the hotkey (leader) dialog by
    /// pressing `/`; Esc then returns to that dialog instead of closing outright.
    pub from_hotkey: bool,
}

impl PaletteState {
    /// Open an empty palette. `from_hotkey` records whether it was promoted from
    /// the leader dialog (so Esc returns there).
    pub fn new(from_hotkey: bool) -> PaletteState {
        PaletteState {
            input: crate::text_field::TextField::new(String::new()),
            scroll: crate::list_scroll::ListScroll::new(),
            from_hotkey,
        }
    }

    /// The fuzzy query: the first whitespace-delimited token of the input line.
    pub fn query(&self) -> &str {
        self.input.value.split_whitespace().next().unwrap_or("")
    }

    /// The argument string passed to the chosen command: everything after the
    /// first token (leading whitespace trimmed). Empty when only a query is typed.
    pub fn args(&self) -> &str {
        let v = &self.input.value;
        match v.find(char::is_whitespace) {
            Some(sp) => v[sp..].trim_start(),
            None => "",
        }
    }

    /// The command line to execute for command `name`: the command name plus any
    /// typed arguments (`"name"` when there are none).
    pub fn command_line(&self, name: &str) -> String {
        let args = self.args();
        if args.is_empty() {
            name.to_string()
        } else {
            format!("{name} {args}")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Game,
    Map,
}

// ── Text-entry prompt kinds ─────────────────────────────────────────────────

/// Which path field is being edited in the config screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPathField {
    UserDir,
}

/// Which text prompt a [`TextEntryDialog`] serves, carrying the target room (and
/// edge direction where applicable) or config context each submit needs. Used by
/// `apply_text_entry` to know which mapper/config method to call on submit.
/// (SQ-0307 — replaces the retired bottom-bar `PromptKind`; the y/n
/// `ConfirmDeleteSave` moved to a two-button confirm dialog.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEntryKind {
    RenameRoom(RoomId),
    EditNotes(RoomId),
    /// Relabel the edge that exits `RoomId` in the given direction.
    RelabelEdge(RoomId, Direction),
    /// Rename the layer with the given id.
    RenameLayer(LayerId),
    /// Edit a config path field (user_dir) from the config screen.
    ConfigEditPath { field: ConfigPathField },
    /// Enter a filename for a game `create_by_prompt` (write modes). The pending
    /// request lives on `AppState.pending_filename`.
    CreateFile,
}

// ── Save-name dialog ──────────────────────────────────────────────────────────

/// State for the save-name modal (a common-dialog with a caret text field). Opened
/// for both the host "Save State as named slot" and an in-game `@save` (which one is
/// tracked by `AppState.ingame_io`). The `field` is prefilled with a date-time
/// default shown greyed until `active`; see the field-behavior state machine in
/// `main.rs`'s save-name intercept.
#[derive(Debug, Clone)]
pub struct SaveNameDialog {
    /// The editable buffer + caret.
    pub field: crate::text_field::TextField,
    /// false = the greyed default placeholder; true = live editing.
    pub active: bool,
    /// True when opened for an in-game `@save` (vs a host Save State slot). Kept for
    /// clarity; the actual save branch is chosen by `AppState.ingame_io`.
    pub ingame: bool,
}

impl SaveNameDialog {
    /// Open with the given date-time default name, greyed and inactive.
    pub fn new(default_name: String, ingame: bool) -> SaveNameDialog {
        SaveNameDialog {
            field: crate::text_field::TextField::new(default_name),
            active: false,
            ingame,
        }
    }
}

// ── Overwrite-confirm dialog (SQ-0648) ──────────────────────────────────────────

/// What a confirmed [`ConfirmOverwriteSave`] resumes.
///
/// The save-as dialog path and the slash `/save <name>` path both write named
/// saves through `save_named`, but only the dialog path has a modal to fall
/// back into on Cancel — the slash path has nothing to reopen, so its pending
/// name has to ride along here instead of being re-read off a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingOverwrite {
    /// The save-as dialog path (host Save State slot or in-game `@save`). The
    /// save-name dialog is left open BEHIND the confirm overlay the whole
    /// time it's up, so a confirm reads the typed name back off it and a
    /// cancel needs no recovery at all — the dialog is exactly as the player
    /// left it.
    SaveAs,
    /// The slash `/save <name>` path: there is no dialog to read the name
    /// back from, so it travels with the pending state instead.
    Slash(String),
}

/// Pending overwrite-confirmation state (SQ-0648): a save-as target whose file
/// already exists. Confirm writes, replacing it; cancel leaves it untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmOverwriteSave {
    /// Absolute path to the target `.babelmap` file.
    pub path: std::path::PathBuf,
    /// Display name of the EXISTING save at `path` — may differ from the name
    /// the player just typed when two names slugify to the same file.
    pub existing_name: String,
    /// What to do once the player answers.
    pub pending: PendingOverwrite,
}

// ── Text-entry dialog ─────────────────────────────────────────────────────────

/// A single-field text-entry modal (title, one caret field, OK/Cancel) — the
/// common home for the former bottom-bar map-edit / config-path / create-file
/// prompts (SQ-0307). Unlike the save-name dialog, the field opens ACTIVE with
/// the prompt's initial value and the caret at the end (normal caret editing, no
/// greyed-placeholder adopt semantics). Held in `AppState.text_entry`.
#[derive(Debug, Clone)]
pub struct TextEntryDialog {
    /// Which prompt this dialog serves (carries the submit context).
    pub kind: TextEntryKind,
    /// The editable buffer + caret, prefilled with the prompt's initial value.
    pub field: crate::text_field::TextField,
}

impl TextEntryDialog {
    /// Open for `kind`, prefilled with `initial` (caret at the end).
    pub fn new(kind: TextEntryKind, initial: impl Into<String>) -> TextEntryDialog {
        TextEntryDialog { kind, field: crate::text_field::TextField::new(initial) }
    }
}

/// Which modal the run loop opens for a `create_by_prompt` filename request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilenameModal {
    /// Read mode with existing files: let the player pick one (Task 5).
    Picker,
    /// Write / WriteAppend / ReadWrite: prompt for a new name.
    NamePrompt,
    /// Read mode with no existing files: nothing to pick — cancel immediately.
    AutoCancel,
}

/// Decide the modal for a filename request. Read mode (`fmode == 0x02`) picks from
/// existing VFS files (or auto-cancels when there are none); every other mode
/// prompts for a name.
pub fn filename_modal_for(req: crate::session::FilenameReq, existing_files: usize) -> FilenameModal {
    if req.fmode == 0x02 {
        if existing_files == 0 { FilenameModal::AutoCancel } else { FilenameModal::Picker }
    } else {
        FilenameModal::NamePrompt
    }
}

// ── File browser state ────────────────────────────────────────────────────────

/// Mode for the file browser: picking a file to import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FbMode {
    /// Import: browse and pick a `.qzl`/`.sav` file.
    PickFile,
}

/// One entry in the file browser listing.
#[derive(Debug, Clone)]
pub struct FbEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Transient state for the file-browser modal.
/// `None` in `AppState.file_browser` = modal closed.
#[derive(Debug, Clone)]
pub struct FileBrowserState {
    /// Current working directory shown by the browser.
    pub cwd: std::path::PathBuf,
    /// Sorted entries: `..` (if not root), then dirs, then matching files.
    pub entries: Vec<FbEntry>,
    /// Selection + animated scroll offset for the entry list.
    pub scroll: crate::list_scroll::ListScroll,
    /// The file-browser mode (currently always import).
    pub mode: FbMode,
}

impl FileBrowserState {
    /// Build a new `FileBrowserState` for `cwd`, reading the filesystem.
    /// Entries: `..` when not at root, then dirs sorted, then `.qzl`/`.sav` files sorted
    /// (PickFile only).  Entries that fail to read are silently omitted.
    pub fn build(cwd: std::path::PathBuf, mode: FbMode) -> Self {
        let entries = Self::read_entries(&cwd, mode);
        FileBrowserState { cwd, entries, scroll: Default::default(), mode }
    }

    /// (Re)build entries for the current `cwd` and `mode`.
    pub fn refresh(&mut self) {
        self.entries = Self::read_entries(&self.cwd, self.mode);
        self.scroll = Default::default();
    }

    /// Navigate into a subdirectory or parent.
    pub fn cd(&mut self, dir: std::path::PathBuf) {
        self.cwd = dir;
        self.refresh();
    }

    fn read_entries(cwd: &std::path::Path, mode: FbMode) -> Vec<FbEntry> {
        let mut dirs: Vec<String> = Vec::new();
        let mut files: Vec<String> = Vec::new();

        if let Ok(iter) = std::fs::read_dir(cwd) {
            for entry in iter.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                // Skip hidden files (starting with '.') except we add '..' explicitly.
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(name.to_owned());
                } else if mode == FbMode::PickFile {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".qzl") || lower.ends_with(".sav") {
                        files.push(name.to_owned());
                    }
                }
            }
        }

        dirs.sort_unstable();
        files.sort_unstable();

        let mut entries: Vec<FbEntry> = Vec::new();
        // Prepend ".." if not at root.
        if cwd.parent().is_some() {
            entries.push(FbEntry { name: "..".to_owned(), is_dir: true });
        }
        for d in dirs {
            entries.push(FbEntry { name: d, is_dir: true });
        }
        for f in files {
            entries.push(FbEntry { name: f, is_dir: false });
        }
        entries
    }
}

// ── Config screen state ───────────────────────────────────────────────────────

/// Transient state for the config-screen modal.
/// `None` in `AppState.config_screen` = modal closed.
#[derive(Debug, Clone)]
pub struct ConfigScreenState {
    /// A working copy of the config, edited in the modal.
    /// On Save this is copied to `state.config`; on Cancel it is dropped.
    pub working: crate::config::Config,
    /// Selection + animated scroll offset for the settings list.
    pub scroll: crate::list_scroll::ListScroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Story and map panes side by side.
    Split,
    /// Story only; the map panel is hidden.
    TranscriptFull,
}

/// Which pane the interactive resize mode is currently adjusting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    /// The story/map split ratio (`Layout::Split` only).
    StoryMap,
    /// The inventory dock height.
    InvDock,
    /// The command band's height (only while the band is open; SQ-0238).
    CommandBand,
}

/// Where the event loop should go when the current story ends. `Exit` leaves
/// babelmap entirely (the classic quit); `Library` exits the story but returns
/// to the story picker (only meaningful when launched from a directory). Set by
/// the quit / `/quit-to-library` dispatch and read at the loop's break sites, where
/// the binary maps it to its own `RunOutcome`. (SQ-0435)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitTarget {
    #[default]
    Exit,
    Library,
}

/// Percentage-based pane sizes, seeded from `Config` at startup and mirrored
/// here for the layout code to consume. `config` stays the persisted source
/// of truth; this is the runtime-facing copy.
#[derive(Debug, Clone, Copy)]
pub struct PaneSizes {
    /// Story's % of the story/map Split (default 50).
    pub split_ratio: u16,
    /// Command band height in rows, including its frame (default 8).
    pub band_height: u16,
    /// Inventory dock height cap as % of screen height (default 33).
    pub inv_dock_pct: u16,
}

/// Zoom levels for the map pane. `Boxes` is the closest/most-detailed view;
/// `Overview` is the most zoomed-out view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    Boxes,
    Compact,
    Overview,
}

/// Map a fine zoom level (0–8) to the three-variant `Zoom` enum used for rendering.
///
/// Fine levels 0–2 → Overview, 3–5 → Compact, 6–8 → Boxes.
/// The fine level slows down zoom transitions so the middle (Compact) level is
/// reachable without accidentally skipping it when scrolling quickly.
pub(crate) fn zoom_from_level(level: u8) -> Zoom {
    match level {
        0..=2 => Zoom::Overview,
        3..=5 => Zoom::Compact,
        _ => Zoom::Boxes,
    }
}

impl Zoom {
    /// Returns (step_w, step_h): the terminal cell stride per map-grid cell.
    ///
    /// The stride is larger than the box size (see `zoom_box_size`), and the
    /// difference is gutter where connectors route. The Boxes-zoom box is 11×5
    /// (a ~2:1 width:height ratio so it looks square given the ~1:2 terminal cell
    /// aspect; both odd so side anchors land on the exact box centre). The stride
    /// adds an 8-col / 6-row gutter for the direction-aware router's clearance and
    /// perpendicular-crossing lanes.
    pub fn steps(self) -> (i32, i32) {
        match self {
            Zoom::Boxes => (19, 11),
            Zoom::Compact => (12, 5),
            Zoom::Overview => (2, 2),
        }
    }
}

/// Stashed restore data for the launch dialog: (engine save, transcript lines,
/// transcript kinds, screen state).
pub type PendingResume =
    Option<(crate::engine::EngineSave, Vec<String>, Vec<TranscriptKind>, Option<zvm::screen::ScreenState>)>;

/// Transcript bundle loaded from an archive at startup: (lines, kinds, per-line
/// style runs, per-line paragraph layout, per-line inline image).
pub type LoadedTranscript = Option<(
    Vec<String>,
    Vec<TranscriptKind>,
    Vec<Vec<StyleRun>>,
    Vec<ParaFmt>,
    Vec<Option<crate::inline_image::InlineImage>>,
)>;

/// Cache key for the wrapped-transcript product: every input the wrap output
/// depends on. A change in any field forces a re-wrap; an unchanged key lets an
/// idle redraw / scroll reuse the cached rows without re-wrapping. Only the
/// inputs `wrap_lines_kinded` actually consumes are keyed — search query,
/// command_bar mode and viewport height do NOT change the wrapped rows (they act
/// at draw / windowing time), so they are deliberately excluded. (SQ-0305)
#[derive(Debug, Clone)]
pub(crate) struct TranscriptWrapKey {
    /// Content generation (see `AppState::transcript_gen`) — catches a same-length
    /// content replacement (rewind / restore) that a length check alone misses.
    pub gen: u64,
    /// Transcript length — a cheap co-key catching append / clear even if a gen
    /// bump were ever missed.
    pub len: usize,
    /// Active filter (which kinds are visible → which lines are wrapped).
    pub filter: TranscriptFilter,
    /// Wrap width (body columns).
    pub width: u16,
    /// Whether inline-image bands are emitted (a game picker is present).
    pub images_enabled: bool,
    /// Picker cell pixel size (drives image-band fit).
    pub char_px: (u16, u16),
    /// Frameless v6 render mode: applies its own inline-image sizing and draws
    /// content splashes inline, so a live mode toggle must re-wrap (SQ-0461).
    pub frameless: bool,
    /// Screen-clear anchor (full-transcript index); drives top-anchoring.
    pub clear_anchor: Option<usize>,
    /// Current room name — location-header style matching in `resolve_story_style`.
    pub room_name: Option<String>,
    /// Resolved colour scheme (theme / `/reload`); Story styles resolve from it.
    pub colors: crate::colors::ColorScheme,
}

/// The cached wrapped-transcript product for one [`TranscriptWrapKey`]. Holds the
/// fully wrapped rows (so the per-frame filter+clone+wrap waterfall runs only on
/// change, not every frame) plus the two derived products the draw path needs.
/// (SQ-0305)
pub(crate) struct TranscriptWrapCache {
    /// The key these products were built for.
    pub key: TranscriptWrapKey,
    /// Fully wrapped rows for the whole filtered transcript (oldest-first).
    pub rows: Vec<crate::render::transcript::WrappedRow>,
    /// Wrapped-row count before the filtered screen-clear anchor, when it maps
    /// into range (drives top-anchoring); `None` otherwise.
    pub anchor_row: Option<usize>,
    /// Arc-ptr set of every inline image present in the filtered transcript, for
    /// bounding the inline-image protocol cache (`InlineImageRender::retain_live`).
    pub live_bands: std::collections::HashSet<usize>,
}

// `WrappedRow` carries images and is not `Debug`; summarise instead of recursing.
impl std::fmt::Debug for TranscriptWrapCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptWrapCache")
            .field("rows", &self.rows.len())
            .field("anchor_row", &self.anchor_row)
            .field("live_bands", &self.live_bands.len())
            .finish()
    }
}

/// The cached map render model for the live graph, keyed by graph generation and
/// viewed layer. `render_layer` re-runs chain detection + edge routing every call,
/// so re-doing it on an animation / transcript / mouse-move redraw of an otherwise
/// unchanged map is pure waste. The live graph only changes on a turn / tidy apply /
/// map edit — each bumps `graph_gen` — so an unchanged `(gen, layer)` reuses this.
/// Only the live map is cached; replay and tidy-animation graphs are not tracked by
/// `graph_gen` and are rebuilt per frame (see `AppState::cached_map_render`). (SQ-0305)
#[derive(Debug)]
pub(crate) struct MapRenderCache {
    /// Graph generation (`AppState::graph_gen`) this model was routed for.
    pub gen: u64,
    /// Viewed layer this model was routed for.
    pub layer: LayerId,
    /// The routed, zoom-independent render model.
    pub rm: mapper::render::RenderMap,
}

/// Modal / overlay UI state carved off `AppState` (SQ-0307). Each field's
/// presence (`true` / `Some`) means the corresponding modal, dialog, or
/// full-screen overlay is open; `dialog_focus` is the shared button-focus
/// index for the currently-open modal. Grouping only — no behaviour change.
#[derive(Debug, Default)]
pub struct OverlayState {
    /// When true, show the hotkey dialog overlay. Opened by the prefix key (Ctrl+P),
    /// closed by the prefix key again or 'q'.
    pub hotkey_dialog: bool,
    /// Active saves-manager modal state. `None` means the modal is closed.
    pub saves: Option<SavesState>,
    /// Active file-browser modal state. `None` means the browser is closed.
    pub file_browser: Option<FileBrowserState>,
    /// Active VFS file-picker modal state (read-mode `create_by_prompt`).
    /// `None` means the picker is closed.
    pub file_picker: Option<FilePickerState>,
    /// Active command-band dock state. `None` means the band is closed.
    /// Deliberately NOT a modal — see [`AppState::any_modal_overlay_open`].
    pub command_band: Option<CommandBandState>,
    /// Active command-palette popup state (fuzzy slash-command search). `None`
    /// means the palette is closed. (SQ-0419)
    pub palette: Option<PaletteState>,
    /// Active config-screen modal state. `None` means the screen is closed.
    pub config_screen: Option<ConfigScreenState>,
    /// Active rewind/replay modal state. `None` means the modal is closed.
    pub replay: Option<ReplayState>,
    /// When true, the reset-confirmation dialog is open.
    pub reset_dialog: bool,
    /// When true, the Scott-only "game is now over" dialog is open. Set when a
    /// Scott engine's turn quits cleanly (win/loss) instead of exiting the app.
    pub game_over: bool,
    /// When true, the "Also clear the map" checkbox is checked in the reset dialog.
    pub reset_clear_map: bool,
    /// When true, the "Delete saved progress" checkbox is checked in the reset
    /// dialog: on confirm, the game's auto persistent data (VFS cache + aux + auto
    /// Save State) is deleted so the game re-initializes from scratch.
    pub reset_delete_data: bool,
    /// When `Some`, the save-name modal is open (host Save State slot or in-game
    /// `@save`). Replaces the old bottom-bar `PromptKind::SaveAs` overlay so the
    /// prompt renders in the graphics-free dialog area instead of hiding behind a
    /// Glulx graphics window.
    pub save_name_dialog: Option<SaveNameDialog>,
    /// Active single-field text-entry dialog (rename room / edit notes / relabel
    /// edge / rename layer / config path / create-file), if any. A modal drawn in
    /// the graphics-free dialog area; its run-loop intercept owns key/mouse input
    /// while open. (SQ-0307)
    pub text_entry: Option<TextEntryDialog>,
    /// When `Some`, the two-button "delete this save?" confirm dialog is open for
    /// the named save at this path. Confirm deletes it; cancel keeps it. (SQ-0307)
    pub confirm_delete_save: Option<std::path::PathBuf>,
    /// When `Some`, the two-button "overwrite existing save?" confirm dialog is
    /// open: a save-as target that already exists. Confirm writes, replacing
    /// it; cancel leaves it untouched. See [`PendingOverwrite`] for what a
    /// confirm resumes. (SQ-0648)
    pub confirm_overwrite_save: Option<ConfirmOverwriteSave>,
    /// When true, the first-use aux-storage prompt is open.
    pub aux_prompt: bool,
    /// When true, the "Save state before quitting?" confirmation dialog is open.
    pub quit_dialog: bool,
    /// When true, the "Resume saved game?" dialog is shown at startup.
    pub launch_dialog: bool,
    /// Active Hints panel session. `None` means the panel is closed.
    pub hints: Option<HintSession>,
    /// Index of the currently focused button in an open modal dialog. Reset to
    /// a button index when a modal opens; cycled by Tab/Shift-Tab.
    pub dialog_focus: usize,
}

/// Where the last v6 frame put one thing on the terminal, for `/dump-windows`
/// (SQ-0585). `native` is the game-pixel rect it came from — zeroed for entries that
/// are not a game window (the pane, the viewport, a carved strip) — so the engine can
/// match each record back to the window it belongs to and report both halves
/// together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V6CellRect {
    pub label: String,
    pub native: (u16, u16, u16, u16),
    pub cells: (u16, u16, u16, u16),
}

#[derive(Debug)]
pub struct AppState {
    pub focus: Focus,
    /// Active debug-inspector pane. Tiled into the map slot while `Some`; `None` = closed.
    pub debug: Option<crate::debug_panel::DebugPanelState>,
    pub layout: Layout,
    pub zoom: Zoom,
    /// Fine zoom level (0–8): 0–2 = Overview, 3–5 = Compact, 6–8 = Boxes.
    /// Derived from this level; `zoom_in`/`zoom_out`/`zoom_reset` update both.
    pub zoom_level: u8,
    /// Map scroll offset in grid cells: (x, y).
    pub scroll: (i32, i32),
    pub selected_room: Option<RoomId>,
    /// Matrix-view scroll offset: `(first direction column, first room row)` (SQ-0666).
    ///
    /// Separate from `scroll`, which is the drawn map's viewport in grid cells. They measure
    /// different things in different units, and sharing one field would make a pan of one view
    /// silently derange the other every time `/view-map` was pressed.
    pub matrix_scroll: (u16, u16),
    /// Layers the tangle detector has already offered `/mark-maze-layer` for (SQ-0666). Session
    /// state, not persisted: the offer is a nudge, and a nudge you have to remember forever is a
    /// setting. Once the layer is flagged the detector stops on its own.
    pub tangle_suggested: std::collections::BTreeSet<LayerId>,
    /// The last few rooms the player walked into, most recent LAST (SQ-0666). Bounded at
    /// [`MAP_TRAIL_LEN`]; used only on maze-flagged layers, where "how did I get here" is the
    /// question a drawn map would have answered by itself.
    pub map_trail: std::collections::VecDeque<RoomId>,
    /// What the mapper still owes a death the game has not finished with (SQ-0671, SQ-0673): the
    /// `tried` record a fatal move may have to take back, and whether a reported death is still
    /// waiting to be resolved by a resurrection. See [`crate::session::DeathWatch`] for the
    /// lifetime of each. Session state; nothing about it is worth persisting, and a restart or
    /// restore replaces it wholesale.
    pub death_watch: crate::session::DeathWatch,
    pub transcript: Vec<String>,
    /// Parallel kind tag for each entry in `transcript` (always same length).
    pub transcript_kinds: Vec<TranscriptKind>,
    /// Optional per-line render-style override, parallel to `transcript`. In-memory
    /// only (not persisted). `None` = use the line's per-kind style. Kept length-
    /// synced by `push_transcript_kind`; read defensively by the renderer.
    pub transcript_styles: Vec<Option<ratatui::style::Style>>,
    /// Per-line Z-machine text-style runs, parallel to `transcript` (always same
    /// length). Empty for the common unstyled line. Populated only by game-turn
    /// output via `push_transcript_runs`; persisted in `transcript.json`.
    pub transcript_runs: Vec<Vec<StyleRun>>,
    /// Per-line paragraph layout format, parallel to `transcript` (always same
    /// length). `ParaFmt::default()` = left-flush, no indent (the Z-machine and
    /// any buffer that set no Glk layout hints). Populated by `push_transcript_runs`
    /// from each line's first content run; persisted in `transcript.json` (SQ-0330).
    pub transcript_para: Vec<ParaFmt>,
    /// Optional inline image parallel to `transcript` (always same length).
    /// `Some` marks a logical unit that renders as an image band instead of
    /// text; its `transcript` entry is an empty placeholder. In-memory only
    /// (not persisted — pixels don't serialize).
    pub transcript_images: Vec<Option<crate::inline_image::InlineImage>>,
    /// Which categories of transcript entries are currently visible.
    pub transcript_filter: TranscriptFilter,
    pub transcript_scroll: u16,
    /// The `[more]` pager (SQ-0404): runtime state for paging a single command's
    /// overflowing output one screen at a time instead of jumping to the bottom.
    pub pager: crate::pager::Pager,
    /// The transcript's total wrapped-row count from the last rendered frame,
    /// cached so a command turn can measure how many rows it added (the pager arm
    /// needs the pre-turn total, which is only known at render time). (SQ-0404)
    pub last_transcript_total_rows: u16,
    /// Transcript length at the most recent game screen-clear (`erase_window`),
    /// or `None`. When set and the view is at the bottom, the renderer pins the
    /// post-clear lines to the top of the pane so a screen clear looks fresh
    /// while older scrollback stays reachable above it. See `mark_screen_clear`.
    pub clear_anchor: Option<usize>,
    /// Monotonic transcript-content generation, bumped by every mutation of the
    /// transcript vecs (append / insert / merge / in-place edit / wholesale
    /// reset). Distinguishes a same-length content replacement (rewind / restore)
    /// from an unchanged buffer, which a length check alone cannot. Read by the
    /// transcript wrap cache. (SQ-0305)
    pub transcript_gen: u64,
    /// Cache of the fully wrapped transcript rows, keyed by [`TranscriptWrapKey`],
    /// so an unchanged transcript (idle redraw / scroll) is not re-wrapped and the
    /// per-line filter+clone waterfall is skipped. Published/consumed by render.
    /// (SQ-0305)
    pub(crate) transcript_wrap: std::cell::RefCell<Option<TranscriptWrapCache>>,
    /// Cache of the live map's routed render model, keyed by graph generation +
    /// viewed layer, so an animation / transcript / mouse-move redraw of an
    /// unchanged map reuses the routed model instead of re-running `render_layer`.
    /// See [`MapRenderCache`] and [`AppState::cached_map_render`]. (SQ-0305)
    pub(crate) map_render: std::cell::RefCell<Option<MapRenderCache>>,
    /// In-flight background map-render job (SQ-0379): rebuilds `map_render` for a
    /// new `(graph_gen, layer)` off the main thread so a re-route never blocks the
    /// interpreter. `RefCell` so it can be spawned from within the draw closure
    /// (which holds only `&self`); polled/installed from the loop body.
    pub(crate) render_job: std::cell::RefCell<Option<RenderJob>>,
    /// Live progress trace for the in-flight `render_job`, shared with the worker
    /// thread. The worker pushes a phase label as each starts; the map pane shows
    /// them top-right and they are cleared when the job completes (SQ-0379).
    pub(crate) render_steps: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// Screen origin `(col, row)` of the input line's TEXT (just past the `"> "` prompt), captured
    /// by the renderer each frame so a click can be mapped back to a caret position (SQ-0354).
    ///
    /// Only the renderer knows where the line landed: the prompt's width and the row both depend on
    /// layout the input code cannot see. `None` before the first frame, or when the command bar is
    /// hidden.
    pub(crate) input_text_origin: std::cell::Cell<Option<(u16, u16)>>,
    /// Size `(cols, rows)` of the map pane's inner content, captured by the renderer each frame
    /// so `Action::Recenter` can centre against the pane the player is actually looking at
    /// (SQ-0349).
    ///
    /// Every other recentre path runs in the event loop, which holds the pane rects `draw_frame`
    /// returns; a key action reaches `apply_action`, which does not. It used to assume 80×24, and
    /// `recenter_on` divides the pane by the zoom step to place the view — so on any other pane
    /// size the target landed off-centre. `None` before the first frame.
    pub(crate) map_pane_size: std::cell::Cell<Option<(u16, u16)>>,
    /// List-row viewport (rows) of the currently-open selection-list modal,
    /// captured from the last render so `apply_action` nav can keep the
    /// selection visible and arm scroll animations (mirrors the transcript's
    /// `transcript_viewport_rows`). 0 when no list modal is open.
    pub modal_list_viewport: usize,
    /// The player's command line, with a caret (SQ-0354).
    ///
    /// A `TextField` like every other text entry in the app, so the caret arithmetic — char vs
    /// byte indexing above all — lives in one tested place instead of being rewritten here.
    /// Story-controlled input never lands in this buffer: when the story asks for a single
    /// keypress the run loop's char-mode gate forwards the key straight to the VM, before app
    /// routing sees it.
    pub input: crate::text_field::TextField,
    /// Transient top-right notification popups (save/restore/export banners,
    /// slash results, VM faults, map-command refusals). Fed by [`set_status`] and
    /// [`push_notice`]; they slide in, hold a few seconds, and slide out without
    /// ever touching the score bar or the transcript. `/dump-notifications`
    /// replays the retained history into the story. (SQ-0176)
    ///
    /// [`set_status`]: AppState::set_status
    /// [`push_notice`]: AppState::push_notice
    pub notifications: crate::notify::Notifications,
    /// Modal / overlay UI cluster: every field whose presence means a
    /// modal, dialog, or full-screen overlay is open, plus the shared
    /// modal button-focus index. Grouped off `AppState` in SQ-0307.
    pub overlays: OverlayState,
    /// When true, draw each chained room's alignment code (`R{id}` / `C{id}`) in
    /// its box interior (Boxes zoom only). Palette-only since SQ-0446 (reached
    /// through the `/` command palette) — `Ctrl+A` is a readline caret shortcut
    /// at the story prompt now, not this toggle's direct key.
    pub show_alignment: bool,
    /// When true, portal icons additionally show their destination room name (Boxes zoom only).
    /// Dialog-only; toggled via the leader panel (default group "View", letter
    /// `l`) — `Ctrl+P` is the leader-dialog prefix now, not this toggle's direct key.
    pub show_portal_labels: bool,
    /// Active tidy-animation playback, if any. While `Some`, the map renders the current
    /// captured stage instead of the live graph. Started by `Ctrl+Y`, cleared by `Esc`.
    pub tidy_anim: Option<TidyAnim>,
    /// In-flight background tidy job, if any. The worker runs the relayout on a clone
    /// of the graph and returns the tidied clone. Driven by the run loop (spawn, poll, apply).
    pub tidy_job: Option<TidyJob>,
    /// In-flight job building a tidy *animation* off-thread, if any. The worker runs the
    /// tidy pipeline on a clone and returns the frames + tidied graph; the run loop installs
    /// the animation when it finishes. Not an overlay — input stays live during the build.
    pub anim_build_job: Option<AnimBuildJob>,
    /// In-flight one-shot story-border flash, if any. Armed by a beep event; expires after SOUND_PULSE_MS.
    pub sound_pulse: Option<SoundPulse>,
    /// Host audio backend, present when audio was enabled at launch. `None` when
    /// disabled or when construction was skipped.
    pub audio: Option<audio::AudioBackend>,
    /// The Blorb holding this story's `Snd ` resources, resolved at launch.
    pub sound_blorb: Option<blorb::Blorb>,
    /// Playing sampled sounds keyed by Z-machine sound number (for `effect` 3 stop).
    pub sound_ids: std::collections::HashMap<u16, audio::SoundId>,
    /// Finish-routines to fire when a sampled sound ends, keyed by its SoundId.
    pub sound_routines: std::collections::HashMap<audio::SoundId, u16>,
    /// Playing Glulx sounds keyed by Glk channel ref (for stop / replace).
    pub glulx_channels: std::collections::HashMap<u32, audio::SoundId>,
    /// Pending sound-notify per playing SoundId: `(sound resource, notify value)`.
    pub glulx_sound_notify: std::collections::HashMap<audio::SoundId, (u32, u32)>,
    /// Pending volume-notify per Glk channel ref (Sound2 `set_volume_ext`):
    /// `(ramp-completion deadline, notify value)`. The host owns the ramp clock;
    /// when the deadline passes the event loop delivers an `evtype_VolumeNotify`.
    /// At most one entry per channel (a new volume change interrupts the prior).
    pub glulx_volume_notify: std::collections::HashMap<u32, (std::time::Instant, u32)>,
    /// Current linear pre-master gain per Glk channel ref, tracking the value a
    /// live volume ramp is interpolating toward (and where a fresh ramp starts).
    /// Seeded on play / set_volume; stepped by [`AppState::advance_volume_ramps`].
    pub glulx_gain: std::collections::HashMap<u32, f32>,
    /// Active Sound2 volume ramps keyed by Glk channel ref (Sound2
    /// `set_volume_ext` with a nonzero duration). The event loop steps these each
    /// pass via [`AppState::advance_volume_ramps`], interpolating the sink gain
    /// linearly from start to target; a completed ramp is removed. At most one per
    /// channel (a new volume change interrupts the prior).
    pub glulx_volume_ramp: std::collections::HashMap<u32, VolumeRamp>,
    /// A pending change to the running Glulx VM's Sound gestalt, set when the
    /// sound toggle / config save flips `enable_sound`. The event loop drains it
    /// (it holds the session) and calls `GlulxSession::set_sound`, so a game that
    /// re-queries `gestalt_Sound` per play honors the toggle. `None` = nothing pending.
    pub pending_vm_sound: Option<bool>,
    /// Set by a config-screen Save when `watch_style` changed; the run loop
    /// (which owns the file-watcher) reconciles it live and clears this.
    pub pending_watch_style: Option<bool>,
    /// In-flight smooth transcript-scroll animation, if any. `transcript_scroll`
    /// holds the target; this eases the displayed offset toward it.
    pub scroll_anim: Option<ScrollAnim>,
    /// Monotonically increasing generation counter. Bumped each time the real graph is mutated
    /// by an applied turn. Used to detect stale tidy results (job's gen vs current gen).
    pub graph_gen: u64,
    /// Explicit layer override for the map view. `None` means follow the current room's layer.
    pub viewed_layer: Option<LayerId>,
    /// When true, draw the per-room diagnostics inspector overlay over the map pane.
    /// Toggled by the `i` key in map focus.
    /// Deprecated: use `room_panel` for new code. Kept for keyboard-toggle compat.
    pub show_inspector: bool,
    /// Currently-open room panel (info or diagnostics), if any.
    /// Set by mouse clicks and the keyboard inspector toggle; drives `draw_frame`.
    pub room_panel: Option<RoomPanel>,
    /// Middle-button drag-pan state. `Some` while a drag gesture is in progress.
    pub drag: Option<DragState>,
    /// Story-pane text selection (left-drag). `Some` while selecting; the
    /// highlight is shown during the drag and copied on release.
    pub selection: Option<crate::clipboard::Selection>,
    /// Auto-scroll direction while a story-pane selection drag sits at an edge:
    /// -1 = top edge (reveal older), +1 = bottom edge (reveal newer), 0 = interior. (SQ-0197)
    pub selection_edge: i32,
    /// This frame's transcript geometry, published by render for the mouse/copy paths. (SQ-0197)
    pub transcript_geom: std::cell::Cell<Option<crate::clipboard::TranscriptGeom>>,
    /// v6 hybrid letterbox scale, published per-frame by the render's Layered
    /// arm so inline story pictures scale to match the chrome frame (see
    /// `render_transcript`). 0.0 (the Default) and 1.0 both mean "no scaling".
    pub v6_image_scale: std::cell::Cell<f32>,
    /// Where the last v6 frame actually PUT each window, in terminal cells, for
    /// `/dump-windows`. The engine can report the game's pixel rects but has no idea
    /// what the renderer did with them, and a v6 layout defect is nearly always in
    /// that mapping — art scales by pixel, text by cell, and the two disagree.
    /// Rebuilt every frame by the v6 render paths.
    pub v6_cell_map: std::cell::RefCell<Vec<V6CellRect>>,
    /// Recent v6 render paths, newest last, consecutive repeats collapsed to a count
    /// (SQ-0587). `/dump-windows` cannot observe the steady state on its own: typing
    /// `/` opens the command palette, a modal overlay, which itself routes the frame
    /// away from the pixel path — so the last frame before the command runs is always
    /// a palette frame. This history shows what the frames BEFORE it did.
    pub v6_path_log: std::cell::RefCell<Vec<(String, u32)>>,
    /// Save-time display-list diagnostics (SQ-0588), newest last, shown by
    /// `/dump-windows`. Written by `note_v6_save` when a window's recorded ops do
    /// not reproduce its live canvas.
    pub v6_save_log: std::cell::RefCell<Vec<String>>,
    /// The hybrid ring's bottom PLAN for the last frame, and the clip it applied
    /// (SQ-0587): `(lowest opaque native art row, the terminal row the ring was cut
    /// at)`, or `None` when that plan does not clip. The side-border flank bands are
    /// what this drops, so when a game's surrounding art goes missing while its
    /// centre picture stays, these two numbers say whether the ring was cut and how
    /// far up — i.e. whether the graphics canvas still has art down there at all.
    pub v6_ring_plan: std::cell::Cell<&'static str>,
    pub v6_ring_clip: std::cell::Cell<Option<(u16, u16)>>,
    /// Last v6 raster story metrics (SQ-0469), cached so a frame that skips the
    /// canvas rebuild (unchanged generation) can still republish the scroll/pager
    /// geometry the render arm returns. Valid across skipped frames because every
    /// input that alters these metrics also bumps the v6 raster generation.
    pub v6_raster_metrics: std::cell::Cell<Option<crate::render::screen::RasterMetrics>>,
    /// Horizontal text margin actually inset on each side of the story text this
    /// frame (SQ-0345), published by `reserve_text_margin` so `render_middle` can
    /// draw the scrollbar flush against the pane border rather than inside the
    /// margin band — only the text gets the margin.
    pub text_margin_applied: std::cell::Cell<u16>,
    /// The selection's extracted copy text, published by render, read on mouse-release. (SQ-0197)
    pub selection_text: std::cell::RefCell<Option<String>>,
    /// Sub-character pan offset in terminal columns/rows, applied on top of `scroll`.
    /// Allows 1-character precision drag panning without changing the cell-unit scroll.
    /// Cleared by `recenter_on`.
    pub char_pan: (i32, i32),

    /// Resolved glyph set for the map renderer.  Defaults to today's hardcoded glyphs;
    /// overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub symbols: crate::symbols::SymbolSet,

    /// Resolved color scheme.  Defaults to `ColorScheme::terminal_default()` (today's exact
    /// ANSI colors); overwritten at startup (and on `/reload`) from `style.toml` via `style::resolve`.
    pub colors: crate::colors::ColorScheme,

    /// Per-game garglk.ini colour overlay (SQ-0319), discovered beside the story
    /// at boot. Kept here so `reload_style` can re-apply it over the freshly
    /// resolved `colors` (garglk sits between the global theme and the user's
    /// per-game `<game_dir>/style.toml`). `None` when no sidecar was found.
    pub garglk_overlay: Option<crate::garglk_ini::GarglkOverlay>,

    /// The global `honor_game_colours` default (from config.toml/CLI, before any
    /// garglk.ini or per-game override), captured at boot. `reload_style` recomputes
    /// the live `config.honor_game_colours` as `per_game > garglk.ini > this base`
    /// (SQ-0318), so `auto` (no per-game override) falls back here.
    pub honor_game_colours_base: bool,

    /// Resolved keymap.  Defaults to `KeyMap::default()` (today's hardcoded bindings);
    /// overwritten at startup via `KeyMap::resolve(&cfg.keymap)` when a config is present.
    pub keymap: crate::keymap::KeyMap,

    /// Hotkey layout: prefix key, direct command set, dialog groups.
    /// Defaults to the built-in layout; overwritten at startup from config.
    pub hotkeys: crate::keymap::HotkeyLayout,



    /// Set while a game-initiated (v4+) `@save`/`@restore` is awaiting the host's
    /// file I/O. The saves dialog runs in "in-game" mode: its confirm/cancel call
    /// `session.resume_save`/`resume_restore` instead of the Ctrl+S/Ctrl+R path.
    pub ingame_io: Option<crate::session::PendingIo>,

    /// Flag-hop: set by `handle_save_as` after a successful in-game SAVE so
    /// the run loop (where `session`/`mapper`/`last_panes` are in scope) performs
    /// the VM resume + recenter. `Some(true)` = file written. Cleared on resume.
    pub ingame_resume_save: Option<bool>,




    /// The resolved runtime config. Set at startup; updated on config-screen Save.
    pub config: crate::config::Config,

    /// Percentage-based pane sizes, seeded from `config` at startup.
    pub pane_sizes: PaneSizes,

    /// Set (by `apply_action`) whenever `config` has changed in a way that
    /// should be persisted to `config.toml` on the next loop iteration,
    /// regardless of which dispatch path (`KeyResolve::Action` or
    /// `KeyResolve::Command`) handled the key. The run loop checks this after
    /// every event and writes + clears it. See `Action::ResizeExit` /
    /// `Action::ResizeReset`.
    pub pending_config_write: bool,

    /// True while the interactive pane-resize mode is active (leader `z`).
    pub resize_mode: bool,
    /// Which visible pane resize mode is currently adjusting.
    pub resize_target: ResizeTarget,

    /// The pane boundary the mouse is currently dragging, if any (SQ-0669).
    /// While this is set the drag owns every mouse event — see
    /// [`crate::pane_drag`].
    pub pane_drag: Option<crate::pane_drag::PaneDrag>,
    /// The pane boundary the pointer is hovering, if any. Drives the grab
    /// affordance only; `pane_drag` outranks it while a boundary is held.
    pub pane_hover: Option<crate::layout::Boundary>,




    /// Session turn counter; incremented on each non-empty `SubmitCommand`.
    /// Written into `Meta` on every save (quick-save and named).
    pub turns: u32,

    /// True when the game has advanced since the last Save State was written
    /// (or since load/restart) — i.e. there is progress not captured in a Save
    /// State. Drives the "unsaved Save State" quit prompt. Set on each turn,
    /// cleared by a Save State save, a restore/load, and restart.
    pub unsaved_progress: bool,

    /// Where the event loop should resume once this story ends: `Exit` (quit
    /// babelmap) or `Library` (return to the story picker). Set by the quit /
    /// `/quit-to-library` dispatch; read at the loop's break sites. (SQ-0435)
    pub exit_target: ExitTarget,

    /// True when babelmap was launched against a directory (a story library),
    /// so a picker exists to return to. Set once at startup; gates
    /// `/quit-to-library`. (SQ-0435)
    pub launched_from_library: bool,

    /// True when launched with `--debug`: the cumulative executed-PC coverage set
    /// is written to the per-story `.pcs` sidecar on story-end. Set once at
    /// startup. (SQ-0449)
    pub persist_debug_trace: bool,

    /// Per-turn rewind/replay history. Filled when `config.record_turn_history`
    /// is on; persisted into the `.babelmap` archive. Empty otherwise.
    pub history: Vec<crate::history::TurnRecord>,


    /// A game `create_by_prompt` awaiting a host filename (its modal is open).
    pub pending_filename: Option<crate::session::FilenameReq>,
    /// Flag-hop: the chosen filename (`Some(name)`) or cancel (`None`) from the
    /// CreateFile prompt / file picker, drained by the run loop to call
    /// `resume_filename`. Outer `Some` = a decision is ready.
    pub filename_submitted: Option<Option<String>>,

    // ── Autocomplete state ────────────────────────────────────────────────────

    /// Cached parser-vocabulary words from the Z-machine dictionary.
    /// Populated once by the run loop after session creation via
    /// `zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem)`.
    /// If empty, autocomplete draws only from room-description words.
    pub dict_words: Vec<String>,
    /// Current list of completion candidates, recomputed whenever `input` changes
    /// while in Game focus. Empty means no suggestions are shown.
    pub suggestions: Vec<String>,
    /// Index into `suggestions` of the currently-highlighted candidate.
    /// `Tab` advances this (cycling); typing resets it to 0.
    pub suggestion_idx: usize,
    /// Whether a suggestion has been applied to `input` in the current cycle.
    /// `false` after typing (the candidate at `suggestion_idx` is only a preview);
    /// the first Tab/Shift-Tab applies that candidate and sets this `true`, and
    /// only subsequent presses advance `suggestion_idx`. This keeps the bracketed
    /// highlight aligned with the word actually on the command line.
    pub suggestion_active: bool,

    // ── Command history (shell-style Up/Down recall) ──────────────────────────

    /// Every non-empty submitted line (game commands and slash commands), oldest
    /// first. Capped at `COMMAND_HISTORY_CAP`; consecutive duplicates are skipped.
    /// Persisted per-game in the `.babelmap` archive.
    pub command_history: Vec<String>,
    /// Navigation cursor into `command_history`. `None` means "not navigating"
    /// (the input line holds the live draft); `Some(i)` means the input shows
    /// `command_history[i]`.
    pub history_cursor: Option<usize>,
    /// In-progress input saved on the first Up press, restored when the player
    /// pages Down past the newest entry.
    pub history_draft: String,

    // ── Adventure title ───────────────────────────────────────────────────────

    /// Resolved adventure title (override > banner > filename stem).
    /// Set once at startup; used by pane chrome to label the story pane.
    pub title: String,

    /// The current story's IFID (set at session creation). Used for
    /// title/hint lookup. Empty until set.
    pub ifid: String,

    /// The per-game storage directory (`<data_base>/<story-key>.save/`) holding
    /// this story's saves and sidecars, including the per-game `style.toml` and
    /// `config.toml` overrides. Set once at startup; empty until then (no
    /// per-game reads/writes happen against an empty path).
    pub game_dir: std::path::PathBuf,

    // ── Inventory panel state ─────────────────────────────────────────────────

    /// When true, the inventory strip is shown above the input line.
    pub show_inventory: bool,
    /// Locked player object number once detected by the heuristic. None until
    /// the player moves between two rooms and exactly one object follows.
    pub player_obj: Option<u16>,
    /// Last parsed output from an inventory command (parse fallback when player_obj
    /// is not yet locked).
    pub inventory_fallback: Vec<String>,
    /// The player's previous room (global 0 value from the previous turn).
    pub prev_location: Option<u16>,
    /// Objects whose parent was prev_location at the end of the previous turn.
    pub prev_objects_here: std::collections::BTreeSet<u16>,

    // ── Reset dialog state ────────────────────────────────────────────────────


    // ── Save-name dialog state ────────────────────────────────────────────────


    // ── Aux-storage prompt state ──────────────────────────────────────────────


    // ── Quit dialog state ─────────────────────────────────────────────────────


    // ── Launch dialog state ───────────────────────────────────────────────────

    /// Stashed restore data shown while the launch dialog is open.
    /// Tuple is (engine-tagged save, transcript lines, transcript kinds, screen).
    pub pending_resume: PendingResume,
    /// When true, room numbers (#id) are shown in Boxes-zoom room boxes.
    pub show_room_numbers: bool,
    /// How the current room was detected (for the map indicator). Retained
    /// across turns; updated when a turn reports a method.
    pub loc_method: Option<zvm::location::LocationMethod>,
    /// The current room's display name (from `TurnResult.location`), retained
    /// across turns. Drives the built-in `transcript:location` story rule.
    pub current_room_name: Option<String>,
    /// Whether the detection-method indicator is shown. Default false.
    /// Whether the status/score bar (top row of the story pane) is shown.
    /// Default true; toggled by ToggleStatusBar. Hidden, the row collapses into
    /// the transcript but still pops up briefly for a transient status message.
    pub show_status_bar: bool,

    // ── Hints panel state ─────────────────────────────────────────────────────


    // ── Search state ──────────────────────────────────────────────────────────

    /// The active search query, if any. `None` means no search is active.
    pub search_query: Option<String>,
    /// Positions (0-based) within the visible-index list of lines that match the query.
    pub search_matches: Vec<usize>,
    /// Index into `search_matches` of the current match.
    pub search_idx: usize,

    // ── Dialog focus state ────────────────────────────────────────────────────


    // ── Char-input mode ───────────────────────────────────────────────────────

    /// True when the Z-machine is awaiting a single keypress (`read_char`).
    /// Set each frame by the run loop from `session.pending_input()`.
    /// Used by the renderer to hide the bottom input prompt.
    pub char_mode: bool,

    /// True when a Glulx game is blocked on a non-input Glk event only (a
    /// timer/mouse/hyperlink `glk_select` with no line/char request; see
    /// [`InputKind::Event`]). Set each frame from `session.pending_input()`. Like
    /// `char_mode` it hides the typed-input prompt/cursor, but keystrokes are NOT
    /// forwarded to the game (there is no request to satisfy) — the host delivers
    /// the timer tick / click instead.
    pub event_wait: bool,

    // ── Timed input ───────────────────────────────────────────────────────────

    /// When a timed read/read_char is active and honored (`config.honor_timed_input`),
    /// the wall-clock instant the next interrupt tick is due. `None` when no timer
    /// is armed (untimed read, honor disabled, or an overlay/dialog is open).
    pub input_deadline: Option<std::time::Instant>,

    /// When a Glulx game has armed Glk timer events (`glk_request_timer_events`),
    /// the wall-clock instant the next `evtype_Timer` tick is due. `None` when no
    /// Glk timer is armed. Independent of `input_deadline` (which is the
    /// Z-machine timed-input clock).
    pub glulx_timer_next_fire: Option<std::time::Instant>,

    /// The in-game graphics Picker (None when images are disabled or unbuilt).
    pub game_picker: Option<ratatui_image::picker::Picker>,
    /// The terminal's own default fg/bg, probed once at startup (SQ-0510). Seeds
    /// the v6 raster canvas's default ink/page when the theme leaves them at
    /// "terminal default"; each field is `None` when the terminal didn't answer.
    pub term_default_colors: crate::term_colors::TermDefaultColors,
    /// Cached graphics-window protocols (interior-mutable for the render pass).
    pub graphics_render: std::cell::RefCell<crate::render::graphics::GraphicsRender>,
    /// Inline-image band blitter (interior-mutable for the render pass).
    pub inline_image_render: std::cell::RefCell<crate::render::inline_image::InlineImageRender>,

    /// Set when a gvm runtime fault has halted the VM (as opposed to a clean
    /// `glk_exit`). While true, the run loop no longer exits on `TurnResult::quit`
    /// so the fault stays visible and the user can review/save before quitting
    /// deliberately. Reset to `false` on game restart.
    pub vm_halted: bool,

    /// Slide-in inventory dock (bottom). Session-only; starts closed.
    pub inv_dock: crate::anim::PanelSlide,
    /// Slide-in command band (bottom). Session-only; starts closed.
    pub band_dock: crate::anim::PanelSlide,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            focus: Focus::Game,
            debug: None,
            layout: Layout::Split,
            zoom: Zoom::Boxes,
            zoom_level: 7, // default = Boxes (level 7)
            scroll: (0, 0),
            selected_room: None,
            matrix_scroll: (0, 0),
            tangle_suggested: std::collections::BTreeSet::new(),
            map_trail: std::collections::VecDeque::new(),
            death_watch: crate::session::DeathWatch::default(),
            transcript: Vec::new(),
            transcript_kinds: Vec::new(),
            transcript_styles: Vec::new(),
            transcript_runs: Vec::new(),
            transcript_para: Vec::new(),
            transcript_images: Vec::new(),
            transcript_filter: TranscriptFilter::Both,
            garglk_overlay: None,
            honor_game_colours_base: true,
            transcript_scroll: 0,
            pager: crate::pager::Pager::default(),
            last_transcript_total_rows: 0,
            clear_anchor: None,
            transcript_gen: 0,
            transcript_wrap: std::cell::RefCell::new(None),
            map_render: std::cell::RefCell::new(None),
            render_job: std::cell::RefCell::new(None),
            render_steps: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            input_text_origin: std::cell::Cell::new(None),
            map_pane_size: std::cell::Cell::new(None),
            modal_list_viewport: 0,
            input: crate::text_field::TextField::default(),
            notifications: crate::notify::Notifications::default(),
            overlays: OverlayState::default(),
            show_alignment: false,
            show_portal_labels: false,
            tidy_anim: None,
            tidy_job: None,
            anim_build_job: None,
            sound_pulse: None,
            audio: None,
            sound_blorb: None,
            sound_ids: std::collections::HashMap::new(),
            sound_routines: std::collections::HashMap::new(),
            glulx_channels: std::collections::HashMap::new(),
            glulx_sound_notify: std::collections::HashMap::new(),
            glulx_volume_notify: std::collections::HashMap::new(),
            glulx_gain: std::collections::HashMap::new(),
            glulx_volume_ramp: std::collections::HashMap::new(),
            pending_vm_sound: None,
            pending_watch_style: None,
            scroll_anim: None,
            graph_gen: 0,
            viewed_layer: None,
            show_inspector: false,
            room_panel: None,
            drag: None,
            selection: None,
            selection_edge: 0,
            transcript_geom: std::cell::Cell::new(None),
            v6_image_scale: std::cell::Cell::new(1.0),
            v6_cell_map: std::cell::RefCell::new(Vec::new()),
            v6_path_log: std::cell::RefCell::new(Vec::new()),
            v6_save_log: std::cell::RefCell::new(Vec::new()),
            v6_ring_plan: std::cell::Cell::new("—"),
            v6_ring_clip: std::cell::Cell::new(None),
            v6_raster_metrics: std::cell::Cell::new(None),
            text_margin_applied: std::cell::Cell::new(0),
            selection_text: std::cell::RefCell::new(None),
            char_pan: (0, 0),
            symbols: crate::symbols::SymbolSet::default(),
            colors: crate::colors::ColorScheme::terminal_default(),
            keymap: crate::keymap::KeyMap::default(),
            hotkeys: crate::keymap::HotkeyLayout::default(),
            ingame_io: None,
            ingame_resume_save: None,
            config: crate::config::Config::default(),
            pane_sizes: PaneSizes {
                split_ratio: 50,
                band_height: crate::render::command_band::DEFAULT_BAND_ROWS,
                inv_dock_pct: 33,
            },
            pending_config_write: false,
            resize_mode: false,
            resize_target: ResizeTarget::StoryMap,
            pane_drag: None,
            pane_hover: None,
            turns: 0,
            unsaved_progress: false,
            exit_target: ExitTarget::Exit,
            launched_from_library: false,
            persist_debug_trace: false,
            history: Vec::new(),
            pending_filename: None,
            filename_submitted: None,
            dict_words: Vec::new(),
            suggestions: Vec::new(),
            suggestion_idx: 0,
            suggestion_active: false,
            command_history: Vec::new(),
            history_cursor: None,
            history_draft: String::new(),
            title: String::new(),
            ifid: String::new(),
            game_dir: std::path::PathBuf::new(),
            show_inventory: false,
            player_obj: None,
            inventory_fallback: Vec::new(),
            prev_location: None,
            prev_objects_here: std::collections::BTreeSet::new(),
            pending_resume: None,
            show_room_numbers: false,
            loc_method: None,
            current_room_name: None,
            show_status_bar: true,
            search_query: None,
            search_matches: Vec::new(),
            search_idx: 0,
            char_mode: false,
            event_wait: false,
            input_deadline: None,
            glulx_timer_next_fire: None,
            game_picker: None,
            term_default_colors: crate::term_colors::TermDefaultColors::default(),
            graphics_render: std::cell::RefCell::new(Default::default()),
            inline_image_render: std::cell::RefCell::new(Default::default()),
            vm_halted: false,
            inv_dock: crate::anim::PanelSlide::closed(),
            band_dock: crate::anim::PanelSlide::closed(),
        }
    }
}

impl AppState {
    /// Return true if any time-based animation/effect is in flight, so the run
    /// loop should fast-poll (and redraw) to advance it without input. Covers the
    /// tidy border pulse, the sound-beep flash, and smooth transcript scroll.
    pub fn has_active_animation(&self) -> bool {
        self.tidy_job.is_some()
            || self.render_job.borrow().is_some()
            || self.anim_build_job.is_some()
            || self.sound_pulse.is_some()
            || self.scroll_anim.is_some()
            || self.overlays.saves.as_ref().is_some_and(|s| s.scroll.has_active_animation())
            || self.overlays.file_browser.as_ref().is_some_and(|fb| fb.scroll.has_active_animation())
            || self.overlays.config_screen.as_ref().is_some_and(|cs| cs.scroll.has_active_animation())
            || self.overlays.command_band.as_ref().is_some_and(|b| b.has_active_animation())
            || self.overlays.replay.as_ref().is_some_and(|r| r.scroll.has_active_animation())
            || self.overlays.hints.as_ref().is_some_and(|h| h.has_active_animation())
            || self.inv_dock.active()
            || self.band_dock.active()
            || self.notifications.needs_tick()
    }

    /// Clear `command_band` once its slide-out has fully settled. The band is a
    /// "drawer": content persists while `band_dock` animates closed (so the
    /// panel visibly slides out instead of vanishing instantly), and is only
    /// dropped once the slide is done and it's logically closed.
    pub fn settle_command_band(&mut self) {
        if self.overlays.command_band.is_some() && !self.band_dock.open && !self.band_dock.active()
        {
            self.overlays.command_band = None;
        }
    }

    /// True while the command band is on screen (open, or still sliding).
    pub fn command_band_visible(&self) -> bool {
        self.overlays.command_band.is_some() || self.band_dock.active()
    }

    /// Play the turn's sound events through the backend (gated on config +
    /// backend availability). Bleeps (#1/#2) → tones; samples (#>=3) → Blorb
    /// resource playback, remembering the SoundId (and finish routine) per number.
    /// `effect`: 2/default = start, 3 = stop, 1 = prepare (no-op).
    pub fn play_turn_sounds(&mut self, sounds: &[zvm::cpu::exec::SoundEvent]) {
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for ev in sounds {
            match ev.number {
                0 => {}
                1 | 2 => {
                    if ev.effect == 0 || ev.effect == 2 {
                        let freq = if ev.number == 1 { 800.0 } else { 400.0 };
                        backend.play_tone(freq, 150, ev.volume);
                    }
                }
                n => match ev.effect {
                    3 => {
                        if let Some(id) = self.sound_ids.remove(&n) {
                            backend.stop(id);
                        }
                    }
                    1 => {} // prepare: decode on start
                    _ => {
                        if let Some(blorb) = &self.sound_blorb {
                            if let Some((bytes, kind)) = blorb.sound(n as u32) {
                                if let Some(fmt) = sound_kind_to_format(kind) {
                                    if let Some(id) = backend.play_sample(bytes, fmt, ev.volume, ev.repeats) {
                                        self.sound_ids.insert(n, id);
                                        if ev.routine != 0 {
                                            self.sound_routines.insert(id, ev.routine);
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }

    /// Apply this turn's Glk sound-channel operations to the shared audio backend
    /// (Glulx). Mirrors `play_turn_sounds`: gated on the sound config flag and a
    /// present backend; resolves sound resources from `sound_blorb` and tracks the
    /// channel→SoundId and SoundId→notify maps.
    pub fn play_glulx_sound_ops(&mut self, ops: &[crate::session::SchannelOp]) {
        use crate::session::SchannelOp;
        if !self.config.enable_sound {
            return;
        }
        let Some(backend) = self.audio.as_mut() else { return };
        for op in ops {
            match *op {
                SchannelOp::Play { chan, snd, repeats, notify, volume, paused } => {
                    // Playing on a busy channel stops the old sound first; the
                    // replaced sound fires no notify.
                    if let Some(old) = self.glulx_channels.remove(&chan) {
                        backend.stop(old);
                        self.glulx_sound_notify.remove(&old);
                    }
                    let Some(reps) = glk_repeats_to_audio(repeats) else { continue };
                    if let Some(blorb) = &self.sound_blorb {
                        if let Some((bytes, kind)) = blorb.sound(snd) {
                            if let Some(fmt) = sound_kind_to_format(kind) {
                                let gain = glk_volume_to_gain(volume);
                                if let Some(id) = backend.play_sample_gain(bytes, fmt, gain, reps) {
                                    self.glulx_channels.insert(chan, id);
                                    // A fresh sound starts at the channel's snapshot
                                    // volume; cancel any ramp left over from a prior
                                    // sound and seed the current gain.
                                    self.glulx_gain.insert(chan, gain);
                                    self.glulx_volume_ramp.remove(&chan);
                                    if notify != 0 {
                                        self.glulx_sound_notify.insert(id, (snd, notify));
                                    }
                                    // A sound played on a channel paused while empty
                                    // starts paused (Glk 0.7.3 §8.3); a paused sink is
                                    // not "empty", so no finish-notify fires until it
                                    // is unpaused and actually plays out.
                                    if paused {
                                        backend.pause(id);
                                    }
                                }
                            }
                        }
                    }
                }
                SchannelOp::Stop { chan } => {
                    if let Some(id) = self.glulx_channels.remove(&chan) {
                        backend.stop(id);
                        self.glulx_sound_notify.remove(&id);
                    }
                }
                SchannelOp::Destroy { chan } => {
                    if let Some(id) = self.glulx_channels.remove(&chan) {
                        backend.stop(id);
                        self.glulx_sound_notify.remove(&id);
                    }
                    // A destroyed channel can never complete a pending ramp.
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    self.glulx_gain.remove(&chan);
                }
                SchannelOp::SetVolume { chan, vol } => {
                    let gain = glk_volume_to_gain(vol);
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.set_sample_gain(id, gain);
                    }
                    // A plain set_volume is an immediate change; it interrupts any
                    // in-progress ramp (whose notify is then dropped, per spec §8.3).
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    self.glulx_gain.insert(chan, gain);
                }
                SchannelOp::Pause { chan } => {
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.pause(id);
                    }
                }
                SchannelOp::Unpause { chan } => {
                    if let Some(&id) = self.glulx_channels.get(&chan) {
                        backend.unpause(id);
                    }
                }
                SchannelOp::SetVolumeExt { chan, vol, duration_ms, notify } => {
                    let target = glk_volume_to_gain(vol);
                    // A new volume change interrupts any prior one on this channel:
                    // the prior notify AND ramp are dropped (spec §8.3).
                    self.glulx_volume_notify.remove(&chan);
                    self.glulx_volume_ramp.remove(&chan);
                    if duration_ms == 0 {
                        // Immediate change: jump the sink and current gain to target.
                        if let Some(&id) = self.glulx_channels.get(&chan) {
                            backend.set_sample_gain(id, target);
                        }
                        self.glulx_gain.insert(chan, target);
                    } else {
                        // Gradual ramp: interpolate from the channel's current gain to
                        // target over duration_ms. The event loop steps the sink each
                        // pass via advance_volume_ramps; the sink stays at start until
                        // the first step (no jump). Interrupting a live ramp starts the
                        // new one from wherever the gain currently sits.
                        let start_gain = self.glulx_gain.get(&chan).copied().unwrap_or(target);
                        self.glulx_volume_ramp.insert(chan, VolumeRamp {
                            start: std::time::Instant::now(),
                            duration_ms,
                            start_gain,
                            target_gain: target,
                        });
                    }
                    // Schedule the notify at now + duration, regardless of whether a
                    // sound is playing (a volume change may occur between sounds).
                    if notify != 0 {
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_millis(duration_ms as u64);
                        self.glulx_volume_notify.insert(chan, (deadline, notify));
                    }
                }
            }
        }
    }

    /// Stop all playing audio and clear the per-game sound-tracking maps. Call on
    /// game restart and when sound is turned off, so a finished/blorb SoundId from a
    /// prior game can never misfire a finish-routine or Glk sound-notify into a new
    /// game state, and no sink keeps playing across a restart.
    pub fn reset_sound_sidecars(&mut self) {
        if let Some(b) = self.audio.as_mut() {
            b.stop_all();
        }
        self.sound_ids.clear();
        self.sound_routines.clear();
        self.glulx_channels.clear();
        self.glulx_sound_notify.clear();
        self.glulx_volume_notify.clear();
        self.glulx_gain.clear();
        self.glulx_volume_ramp.clear();
    }

    /// Step every active Sound2 volume ramp to time `now`: interpolate the
    /// channel's linear gain, apply it to the live sink, and drop the ramp once
    /// it completes. Called each event-loop pass (host owns the ramp clock).
    pub fn advance_volume_ramps(&mut self, now: std::time::Instant) {
        if self.glulx_volume_ramp.is_empty() {
            return;
        }
        let mut done: Vec<u32> = Vec::new();
        for (&chan, ramp) in self.glulx_volume_ramp.iter() {
            let elapsed = now.saturating_duration_since(ramp.start).as_millis() as u32;
            let gain = ramp_gain(ramp.start_gain, ramp.target_gain, elapsed, ramp.duration_ms);
            self.glulx_gain.insert(chan, gain);
            if let Some(&id) = self.glulx_channels.get(&chan) {
                if let Some(b) = self.audio.as_mut() {
                    b.set_sample_gain(id, gain);
                }
            }
            if elapsed >= ramp.duration_ms {
                done.push(chan);
            }
        }
        for chan in done {
            self.glulx_volume_ramp.remove(&chan);
        }
    }

    /// Set the transcript scroll target to `target`. When animation is enabled
    /// and `scroll_ms > 0`, arm (or retarget) a smooth-scroll animation from the
    /// current displayed offset toward `target`; otherwise jump instantly and
    /// clear any in-flight animation (exactly today's instant scroll).
    pub fn scroll_transcript_to(&mut self, target: u16) {
        let from = self.effective_transcript_scroll() as usize;
        self.transcript_scroll = target;
        self.scroll_anim = ScrollAnim::to(from, target as usize, &self.config.animation);
    }

    /// The transcript offset to render this frame: the animated displayed offset
    /// (line-rounded) while a smooth scroll is in flight, else the logical target.
    /// Still clamped to `[0, max_scroll]` by the renderer.
    pub fn effective_transcript_scroll(&self) -> u16 {
        self.scroll_anim
            .as_ref()
            .map(|a| a.current().round() as u16)
            .unwrap_or(self.transcript_scroll)
    }

    /// Return true if any modal, dialog, or overlay is currently open — including
    /// the CORNER overlays (room panel, tidy animation) that let you keep playing
    /// underneath them.
    ///
    /// For anything about the story pane's live input — the line, its caret, its
    /// suggestions — use [`any_modal_overlay_open`](Self::any_modal_overlay_open)
    /// instead: a modal genuinely means you are not typing at the prompt, but a
    /// corner panel does not.
    pub fn any_overlay_open(&self) -> bool {
        self.any_modal_overlay_open() || self.room_panel.is_some() || self.tidy_anim.is_some()
    }

    /// [`any_overlay_open`](Self::any_overlay_open) minus the corner overlays.
    ///
    /// The room panel and the tidy animation live in the MAP pane and deliberately
    /// do not swallow input, so they must not suppress the story pane's live input
    /// line or caret — doing so hid a half-typed command with no sign it was still
    /// buffered, and Enter would then run something the player could not see.
    pub fn any_modal_overlay_open(&self) -> bool {
        self.overlays.saves.is_some()
            || self.overlays.file_browser.is_some()
            || self.overlays.file_picker.is_some()
            || self.overlays.config_screen.is_some()
            // NOT the command band (SQ-0664): it is a dock, not dialog chrome.
            // Counting it here is what made the old verb menu degrade the
            // session — it hid the story prompt line, swallowed paste, blocked
            // v6/Glk click delivery, and dropped graphical v6 off the pixel
            // path for as long as it was open.
            || self.overlays.palette.is_some()
            || self.overlays.hotkey_dialog
            || self.overlays.text_entry.is_some()
            || self.overlays.confirm_delete_save.is_some()
            || self.overlays.confirm_overwrite_save.is_some()
            || self.overlays.reset_dialog
            || self.overlays.game_over
            || self.overlays.save_name_dialog.is_some()
            || self.overlays.aux_prompt
            || self.overlays.quit_dialog
            || self.overlays.launch_dialog
            || self.overlays.hints.is_some()
            || self.overlays.replay.is_some()
            || self.resize_mode
    }

    /// Note a v6 save-time display-list diagnostic for `/dump-windows` (SQ-0588).
    ///
    /// The save-time self-check replays each window's ops and compares against the
    /// live canvas; a window that fails names itself here. That is the whole reason
    /// the archive stores ops rather than pixels — a pixel snapshot would restore
    /// correctly and leave the recording gap invisible until it surfaced later as
    /// missing or mis-coloured art. `&self` because the auto-save paths hold an
    /// immutable `AppState` (same reason as `note_v6_path`).
    pub fn note_v6_save(&self, msg: &str) {
        let mut log = self.v6_save_log.borrow_mut();
        if log.iter().any(|m| m == msg) {
            return; // one line per distinct problem, not one per save
        }
        log.push(msg.to_string());
        if log.len() > 8 {
            log.remove(0);
        }
    }

    /// Note one v6 render path for the `/dump-windows` history (SQ-0587). Consecutive
    /// repeats collapse into a count, and only the last few distinct runs are kept.
    pub fn note_v6_path(&self, label: &str) {
        let mut log = self.v6_path_log.borrow_mut();
        match log.last_mut() {
            Some((last, n)) if last == label => *n += 1,
            _ => {
                log.push((label.to_string(), 1));
                if log.len() > 6 {
                    log.remove(0);
                }
            }
        }
    }

    /// The modal overlays currently open, by name (SQ-0587). A v6 story drops its
    /// pixel path while one is up, so `/dump-windows` can say WHICH — "the ring did
    /// not run" is only half an answer.
    pub fn open_modal_overlays(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.overlays.saves.is_some() { v.push("saves"); }
        if self.overlays.file_browser.is_some() { v.push("file_browser"); }
        if self.overlays.file_picker.is_some() { v.push("file_picker"); }
        if self.overlays.config_screen.is_some() { v.push("config_screen"); }
        if self.overlays.palette.is_some() { v.push("palette"); }
        if self.overlays.hotkey_dialog { v.push("hotkey_dialog"); }
        if self.overlays.text_entry.is_some() { v.push("text_entry"); }
        if self.overlays.confirm_delete_save.is_some() { v.push("confirm_delete_save"); }
        if self.overlays.confirm_overwrite_save.is_some() { v.push("confirm_overwrite_save"); }
        if self.overlays.reset_dialog { v.push("reset_dialog"); }
        if self.overlays.game_over { v.push("game_over"); }
        if self.overlays.save_name_dialog.is_some() { v.push("save_name_dialog"); }
        if self.overlays.aux_prompt { v.push("aux_prompt"); }
        if self.overlays.quit_dialog { v.push("quit_dialog"); }
        if self.overlays.launch_dialog { v.push("launch_dialog"); }
        if self.overlays.hints.is_some() { v.push("hints"); }
        if self.overlays.replay.is_some() { v.push("replay"); }
        if self.resize_mode { v.push("resize_mode"); }
        v
    }

    /// Set the explicit layer override. `None` means follow the current room's layer.
    pub fn set_viewed_layer(&mut self, layer: Option<LayerId>) {
        self.viewed_layer = layer;
    }

    /// The layer the map pane must draw THIS frame, for the graph it is drawing.
    ///
    /// A tidy animation is the case [`active_layer`](Self::active_layer) cannot serve. Its frames
    /// are `layer_subgraph`s, and a subgraph reports `layers()` as main-only whatever layer it
    /// holds — so `active_layer` finds `viewed_layer` "absent", falls back to `MAIN_LAYER`, and the
    /// map draws every room of a layer the subgraph has none of: blank. The animation therefore
    /// states its own layer rather than letting the frame be asked (SQ-0359).
    ///
    /// `replay` takes precedence over an animation, matching the map pane's own order.
    pub fn frame_layer(&self, live: &MapGraph, replay: Option<&MapGraph>) -> LayerId {
        if let Some(g) = replay {
            return self.active_layer(g);
        }
        match &self.tidy_anim {
            Some(anim) => anim.layer,
            None => self.active_layer(live),
        }
    }

    /// Return the layer to render the map with.
    /// Priority: `viewed_layer` (if set and still present), else the current room's layer, else `MAIN_LAYER`.
    pub fn active_layer(&self, graph: &MapGraph) -> LayerId {
        use mapper::layer::MAIN_LAYER;
        if let Some(l) = self.viewed_layer {
            if graph.layers().contains_key(&l) {
                return l;
            }
        }
        graph.current().map(|id| graph.layer_of(id)).unwrap_or(MAIN_LAYER)
    }

    /// Borrow the live map's routed render model for `layer`. The routing in
    /// `render_layer` is the dominant map cost, so it is kept OFF the main thread
    /// (SQ-0379): the very first model is built synchronously (small, at game
    /// start), but every later rebuild — triggered by a `graph_gen` change —
    /// happens on a background worker while this keeps returning the last-ready
    /// model. Only the current-room highlight and labels are refreshed here, live
    /// and cheaply, so the highlight follows the player with no re-route (SQ-0378).
    ///
    /// Only the live map uses this — replay and tidy-animation graphs are not
    /// tracked by `graph_gen`, so their models are built fresh each frame. (SQ-0305)
    pub fn cached_map_render(
        &self,
        layer: LayerId,
        graph: &mapper::graph::MapGraph,
    ) -> std::cell::Ref<'_, mapper::render::RenderMap> {
        let gen = self.graph_gen;
        let fresh = matches!(self.map_render.borrow().as_ref(), Some(c) if c.gen == gen && c.layer == layer);
        if !fresh {
            // No routing ever runs on the main thread (SQ-0379). If there is no
            // model yet, seed an empty one so the pane can draw (blank) this frame;
            // otherwise keep drawing the stale model. Either way the real re-route
            // runs on the background worker, with the pulse + step overlay showing.
            if self.map_render.borrow().is_none() {
                *self.map_render.borrow_mut() = Some(MapRenderCache {
                    gen,
                    layer,
                    rm: mapper::render::render(&mapper::graph::MapGraph::new()),
                });
            }
            self.spawn_render_job(layer, graph, gen);
        }
        // Live, cheap refresh of the per-move-changeable fields on whatever model
        // is currently shown — no re-route (SQ-0378).
        if let Some(c) = self.map_render.borrow_mut().as_mut() {
            let current = graph.current();
            for r in &mut c.rm.rooms {
                r.is_current = Some(r.id) == current;
                if let Some(room) = graph.room(r.id) {
                    r.label = room.label().to_string();
                }
            }
        }
        std::cell::Ref::map(self.map_render.borrow(), |c| {
            &c.as_ref().expect("populated above").rm
        })
    }

    /// True when the map pane draws the MATRIX for the layer it is showing (SQ-0666).
    ///
    /// The matrix is built from the graph at draw time — rooms, edges, `tried` — and owes nothing
    /// to the routed layout model or to the background jobs that produce it.
    pub fn map_shows_matrix(&self, graph: &mapper::graph::MapGraph) -> bool {
        graph.layer_view(self.active_layer(graph)) == mapper::layer::MapView::Matrix
    }

    /// The routed model for the LIVE map this frame, or `None` when the pane is showing the
    /// matrix and there is no map to route (SQ-0671).
    ///
    /// This is the churn the player saw as a cycling map pane: [`AppState::cached_map_render`]
    /// re-routes whenever `graph_gen` moves, and every arriving tidy result bumps that generation,
    /// so a background job for ANY layer spawned a render worker whose in-flight pulse restyled
    /// the border of a pane drawing a table it never touched. Asking for no model at all is what
    /// makes the matrix independent of the layout pipeline, rather than merely ignoring its
    /// output.
    pub fn live_map_render(
        &self,
        layer: LayerId,
        graph: &mapper::graph::MapGraph,
    ) -> Option<std::cell::Ref<'_, mapper::render::RenderMap>> {
        if self.map_shows_matrix(graph) {
            return None;
        }
        Some(self.cached_map_render(layer, graph))
    }

    /// Spawn the background map-render worker for `(gen, layer)` unless one is
    /// already in flight (coalesced — like the tidy worker). The worker routes a
    /// clone of the graph and reports each phase into `render_steps`. (SQ-0379)
    fn spawn_render_job(&self, layer: LayerId, graph: &mapper::graph::MapGraph, gen: u64) {
        let mut job = self.render_job.borrow_mut();
        if job.is_some() {
            // A job is already running; let it finish. `poll_render_job` discards
            // it if it turns out stale, and the next frame respawns for `gen`.
            return;
        }
        let g = graph.clone();
        let steps = self.render_steps.clone();
        if let Ok(mut s) = steps.lock() {
            s.clear();
        }
        let handle = std::thread::spawn(move || {
            let mut push = |name: &str| {
                if let Ok(mut s) = steps.lock() {
                    s.push(name.to_string());
                }
            };
            mapper::render::render_layer_traced(&g, layer, &mut push)
        });
        *job = Some(RenderJob { handle, layer, gen, started: std::time::Instant::now() });
    }

    /// Poll the background map-render worker: if it has finished, install its
    /// model as the new last-ready render (when its generation still matches) or
    /// discard it as stale (a fresh job then spawns on the next draw). Returns
    /// true when a completed job was handled (the caller should redraw). (SQ-0379)
    pub fn poll_render_job(&mut self) -> bool {
        let done = self
            .render_job
            .borrow()
            .as_ref()
            .is_some_and(|j| j.handle.is_finished());
        if !done {
            return false;
        }
        let job = self.render_job.borrow_mut().take().expect("checked above");
        match job.handle.join() {
            Ok(rm) => {
                if job.gen == self.graph_gen {
                    if self.config.trace.map {
                        let steps = self.render_steps_snapshot();
                        write_map_trace(&self.config.user_dir, &steps, true);
                    }
                    *self.map_render.borrow_mut() =
                        Some(MapRenderCache { gen: job.gen, layer: job.layer, rm });
                    if let Ok(mut s) = self.render_steps.lock() {
                        s.clear();
                    }
                }
                // else: stale — a newer geometry arrived mid-build; drop it and
                // let the next draw respawn for the current generation.
            }
            Err(_) => {
                // Worker panicked: keep the last-ready model, drop the trace.
                if let Ok(mut s) = self.render_steps.lock() {
                    s.clear();
                }
            }
        }
        true
    }

    /// True while the background map-render worker (SQ-0379) is in flight.
    pub fn map_render_in_flight(&self) -> bool {
        self.render_job.borrow().is_some()
    }

    /// Poll the background v6 raster encode (SQ-0469): install a completed
    /// protocol and report whether a redraw is warranted. Called once per loop
    /// tick so an off-thread encode surfaces within a poll interval.
    pub fn poll_v6_encode_job(&self) -> bool {
        self.graphics_render.borrow_mut().poll_v6_job()
    }

    /// How long the in-flight background map job (tidy relayout or render worker)
    /// has been running, for the border-pulse phase — `None` when neither runs.
    ///
    /// Also `None` while the pane is showing the matrix (SQ-0671). The pulse announces "the
    /// layout you are looking at is being rebuilt"; over a table read straight from the graph it
    /// announces nothing, and a job for some other layer finishing mid-turn made the map pane's
    /// border cycle red/green while the player walked a maze.
    pub fn map_job_pulse_elapsed(
        &self,
        graph: &mapper::graph::MapGraph,
    ) -> Option<std::time::Duration> {
        if self.map_shows_matrix(graph) {
            return None;
        }
        self.tidy_job
            .as_ref()
            .map(|j| j.started.elapsed())
            .or_else(|| self.render_job.borrow().as_ref().map(|j| j.started.elapsed()))
    }

    /// Snapshot the in-flight render worker's phase trace, for the map's top-right
    /// progress overlay (SQ-0379).
    pub fn render_steps_snapshot(&self) -> Vec<String> {
        self.render_steps.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Advance keyboard focus one step (Tab). See [`cycle_focus`].
    pub fn toggle_focus(&mut self) {
        self.cycle_focus(true);
    }

    /// Cycle keyboard focus one step forward (`forward = true`, Tab) or back
    /// (Shift-Tab). The stops are **per window**, not per sub-tab: the story
    /// pane, then — when the debug inspector is open — each of its windows in
    /// turn (story → debug 0 → 1 → 2 → story). With the inspector closed there
    /// is nowhere else to go and Tab does nothing.
    ///
    /// The map pane is deliberately NOT a stop (SQ-0599). It used to be, and
    /// that made the same keystroke mean two different things depending on a
    /// focus state with no obvious on-screen cue — press an arrow and you were
    /// either editing the command line or panning the map, with nothing to say
    /// which. The map is now driven entirely modelessly: Shift+Arrow pans and
    /// the mouse does the rest, from wherever you are.
    pub fn cycle_focus(&mut self, forward: bool) {
        // Focus stops after the story pane (position 0) — the inspector's
        // windows, and nothing else.
        let extra = if self.debug.is_some() {
            crate::debug_panel::WINDOW_TABS.len() // one stop per debug window
        } else {
            0
        };
        let total = extra + 1;
        let cur = match self.focus {
            Focus::Game => 0,
            Focus::Map => 1 + self.debug.as_ref().map_or(0, |p| p.focus),
        };
        let next = if forward { (cur + 1) % total } else { (cur + total - 1) % total };
        if next == 0 {
            self.focus = Focus::Game;
        } else {
            self.focus = Focus::Map;
            if let Some(p) = &mut self.debug {
                p.focus = next - 1;
            }
        }
    }

    /// Toggle the map panel on (`Split`) / off (`TranscriptFull`).
    ///
    /// No longer touches focus: since SQ-0599 the map is not a focus stop at
    /// all, so hiding it cannot strand the keyboard on a hidden pane (the
    /// SQ-0333 hazard this used to guard against).
    pub fn toggle_map(&mut self) {
        self.layout = match self.layout {
            Layout::Split => Layout::TranscriptFull,
            Layout::TranscriptFull => Layout::Split,
        };
    }

    /// Which panes are currently visible and eligible for resize mode, in
    /// Tab-cycle order: StoryMap (Split layout only), InvDock (inventory
    /// shown), CommandBand (band open).
    ///
    /// The band is only a resize target while it is open — resize mode preempts
    /// the band's key intercept, so the two can be active at once (SQ-0238). Its
    /// height (`command_band.height`) is also a persisted, resettable config
    /// value.
    pub fn resize_targets_visible(&self) -> Vec<ResizeTarget> {
        let mut targets = Vec::new();
        if self.layout == Layout::Split {
            targets.push(ResizeTarget::StoryMap);
        }
        if self.show_inventory {
            targets.push(ResizeTarget::InvDock);
        }
        if self.overlays.command_band.is_some() {
            targets.push(ResizeTarget::CommandBand);
        }
        targets
    }

    /// Move `resize_target` to the next (`forward`) or previous visible target,
    /// wrapping. Snaps to the first visible target if the current one isn't
    /// visible; leaves it unchanged if nothing is visible.
    pub fn cycle_resize_target(&mut self, forward: bool) {
        let targets = self.resize_targets_visible();
        let Some(first) = targets.first() else { return };
        match targets.iter().position(|t| *t == self.resize_target) {
            Some(idx) => {
                let n = targets.len();
                let next = if forward { (idx + 1) % n } else { (idx + n - 1) % n };
                self.resize_target = targets[next];
            }
            None => self.resize_target = *first,
        }
    }

    /// Mirror the live pane sizes into `config`, the persisted source of truth.
    ///
    /// The one place the three runtime sizes meet their config keys, so resize
    /// mode's arrows and the mouse drag (SQ-0669) persist through identical
    /// paths; the actual write still waits for `pending_config_write`.
    pub fn sync_pane_sizes_to_config(&mut self) {
        self.config.split_ratio = self.pane_sizes.split_ratio;
        self.config.command_band.height = self.pane_sizes.band_height;
        self.config.inv_dock_pct = self.pane_sizes.inv_dock_pct;
    }

    /// Reset all pane sizes to their config defaults and mirror into `config`.
    pub fn reset_pane_sizes(&mut self) {
        self.pane_sizes = PaneSizes {
            split_ratio: crate::config::default_split_ratio(),
            band_height: crate::config::default_band_height(),
            inv_dock_pct: crate::config::default_inv_dock_pct(),
        };
        self.sync_pane_sizes_to_config();
    }

    /// Whether `b` should draw with the resize accent: the boundary being
    /// dragged, or — when nothing is held — the one under the pointer (SQ-0669).
    pub fn boundary_active(&self, b: crate::layout::Boundary) -> bool {
        match self.pane_drag {
            Some(d) => d.boundary == b,
            None => self.pane_hover == Some(b),
        }
    }

    /// Zoom in one VISIBLE step (toward Boxes). Already at Boxes → no change; it is the most
    /// detailed view there is.
    pub fn zoom_in(&mut self) {
        self.zoom_to(match self.zoom {
            Zoom::Overview => Zoom::Compact,
            Zoom::Compact | Zoom::Boxes => Zoom::Boxes,
        });
    }

    /// Zoom out one VISIBLE step (toward Overview). Already at Overview → no change.
    pub fn zoom_out(&mut self) {
        self.zoom_to(match self.zoom {
            Zoom::Boxes => Zoom::Compact,
            Zoom::Compact | Zoom::Overview => Zoom::Overview,
        });
    }

    /// Zoom by `n` VISIBLE steps: positive in (toward Boxes), negative out (SQ-0355).
    ///
    /// Repeats the one-step move rather than doing arithmetic on the fine level, so `zoom-map 5`
    /// clamps at the most detailed view exactly as five presses of `+` would — the command and the
    /// key cannot drift apart.
    pub fn zoom_by(&mut self, n: i32) {
        for _ in 0..n.unsigned_abs() {
            if n > 0 {
                self.zoom_in();
            } else {
                self.zoom_out();
            }
        }
    }

    /// Jump to `z`, landing in the MIDDLE of its fine band (SQ-0350).
    ///
    /// A keypress must move the map, and only whole bands are visible: the nine fine levels
    /// collapse to three views (0–2 Overview, 3–5 Compact, 6–8 Boxes). Stepping one fine level per
    /// press meant the default (level 7, mid-Boxes) needed TWO presses of `-` before anything
    /// happened, and `+` did nothing at all, ever — 7→8 is still Boxes. That is the whole of "the
    /// zoom keys are not responsive": the keys always fired, they just moved a counter nobody
    /// could see.
    ///
    /// Landing mid-band rather than on its edge keeps `+` and `-` exact inverses, and leaves the
    /// wheel (which still steps one fine level at a time, see `zoom_in_fine`) equal room either way
    /// before it tips into the next view.
    fn zoom_to(&mut self, z: Zoom) {
        self.zoom_level = match z {
            Zoom::Overview => 1,
            Zoom::Compact => 4,
            Zoom::Boxes => 7,
        };
        self.zoom = z;
    }

    /// Zoom in one FINE step (toward Boxes). Clamps at level 8.
    ///
    /// The wheel's step, not the keyboard's: fine levels exist so a fast ctrl+scroll cannot skip
    /// straight past Compact. A keypress uses `zoom_in`, which moves a whole band.
    pub fn zoom_in_fine(&mut self) {
        self.zoom_level = self.zoom_level.saturating_add(1).min(8);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Zoom out one FINE step (toward Overview). Clamps at level 0. See `zoom_in_fine`.
    pub fn zoom_out_fine(&mut self) {
        self.zoom_level = self.zoom_level.saturating_sub(1);
        self.zoom = zoom_from_level(self.zoom_level);
    }

    /// Reset zoom to the default level (7 = Boxes) and clear char_pan.
    pub fn zoom_reset(&mut self) {
        self.zoom_level = 7;
        self.zoom = Zoom::Boxes;
        self.char_pan = (0, 0);
    }

    /// Pan the map scroll by (dx, dy).
    pub fn pan(&mut self, dx: i32, dy: i32) {
        self.scroll = (self.scroll.0 + dx, self.scroll.1 + dy);
    }

    /// Set scroll so that `cell` is centered in a pane of size `pane_w` × `pane_h`.
    ///
    /// `pane_w` and `pane_h` are in terminal characters; this method converts
    /// them to map-grid cells using the current zoom step before centering,
    /// so that `scroll` stays in cell units (matching `cell_to_screen`).
    ///
    /// For Boxes zoom the non-uniform layout places rooms at roughly
    /// `(BOX_W + MIN_GUTTER)` × `(BOX_H + MIN_GUTTER)` pixels per cell, which
    /// is smaller than `zoom.steps()` (19×11). Using the actual cell footprint
    /// keeps the target room near the pane centre rather than at the top edge.
    pub fn recenter_on(&mut self, cell: (i32, i32), pane_w: u16, pane_h: u16) {
        use crate::render::map::{BOX_W, BOX_H, MIN_GUTTER};
        let (sw, sh) = match self.zoom {
            Zoom::Boxes => (BOX_W + MIN_GUTTER, BOX_H + MIN_GUTTER), // 13 × 7
            _ => self.zoom.steps(),
        };
        let cells_w = (pane_w as i32 / sw).max(1);
        let cells_h = (pane_h as i32 / sh).max(1);
        self.scroll = (cell.0 - cells_w / 2, cell.1 - cells_h / 2);
        // Reset char-granular pan offset when re-centering the view.
        self.char_pan = (0, 0);
    }

    /// Return the indices (into `self.transcript`) of entries that pass the active
    /// `transcript_filter`, in order. `Both` returns all indices; `Story`/`Meta`
    /// return only indices whose kind matches. Defensively tolerates any length
    /// mismatch between `transcript` and `transcript_kinds` by defaulting to `Story`.
    pub fn visible_transcript_indices(&self) -> Vec<usize> {
        (0..self.transcript.len())
            .filter(|&i| {
                let kind = self.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
                match self.transcript_filter {
                    TranscriptFilter::Both => true,
                    TranscriptFilter::Story => matches!(kind, TranscriptKind::Story | TranscriptKind::Input),
                    TranscriptFilter::Meta => matches!(kind, TranscriptKind::Meta | TranscriptKind::Warning),
                }
            })
            .collect()
    }

    /// Split `text` on `'\n'` and append each line to the transcript, tagged as `Story`.
    /// Record a screen-clear boundary (game `erase_window`, ZMSD §8.7.3) at the
    /// current end of the transcript, WITHOUT deleting scrollback. The renderer
    /// pins post-clear output to the top of a fresh screen (blanks below) while
    /// everything above the boundary stays reachable by scrolling up — a
    /// scrollback-preserving "clear" rather than a destructive wipe. Also snaps
    /// the view to the bottom so the cleared screen is what's shown.
    pub fn mark_screen_clear(&mut self) {
        self.clear_anchor = Some(self.transcript.len());
        self.transcript_scroll = 0;
        self.scroll_anim = None;
    }

    /// Truncate the transcript — and every parallel sidecar vec — back to `len`,
    /// collapsing a menu-redraw reprint to a screen-clear boundary so consecutive
    /// reprints replace each other instead of piling up in scrollback. A no-op if
    /// `len` is already at/beyond the end. (SQ-0407)
    pub fn truncate_transcript(&mut self, len: usize) {
        if len >= self.transcript.len() {
            return;
        }
        self.transcript.truncate(len);
        self.transcript_kinds.truncate(len);
        self.transcript_styles.truncate(len);
        self.transcript_runs.truncate(len);
        self.transcript_para.truncate(len);
        self.transcript_images.truncate(len);
        self.bump_transcript_gen();
    }

    pub fn push_transcript(&mut self, text: &str) {
        self.push_transcript_kind(text, TranscriptKind::Story);
    }

    /// Whether app-internal transcript output (status/slash/save-restore messages,
    /// pushed via [`push_transcript_kind`](Self::push_transcript_kind) /
    /// [`push_transcript_styled`](Self::push_transcript_styled)) should be inserted
    /// just ABOVE a trailing game `>` prompt rather than appended after it. True in
    /// inline-prompt mode when the game's prompt is the last line, so these messages
    /// don't bury the prompt the caret sits at (SQ-0270). Game turn output goes
    /// through `push_transcript_runs`/`apply_transcript_elems` and is unaffected.
    fn insert_above_prompt_at(&self) -> Option<usize> {
        (!self.config.command_bar && self.last_transcript_line_is_story())
            .then(|| self.transcript.len().saturating_sub(1))
    }

    /// Bump the transcript-content generation, invalidating the wrap cache. Call
    /// from every method that mutates the transcript vecs (content, kinds, runs,
    /// styles, or images). (SQ-0305)
    fn bump_transcript_gen(&mut self) {
        self.transcript_gen = self.transcript_gen.wrapping_add(1);
    }

    /// Bump the graph generation, invalidating the map render memo. Call after ANY
    /// mutation of the mapper graph or its layout/labels — rename/notes/relabel/nudge,
    /// tidy applies, room reassignment (restore/import/reset). The map render memo is
    /// keyed on this (`cached_map_render`), so a missed bump paints a STALE MAP.
    /// Double-bumping is harmless (`wrapping_add`); a missing bump is a wrong map. (SQ-0305)
    pub fn bump_graph_gen(&mut self) {
        self.graph_gen = self.graph_gen.wrapping_add(1);
    }

    /// Split `text` on `'\n'` and append each line to the transcript with the given kind tag.
    pub fn push_transcript_kind(&mut self, text: &str, kind: TranscriptKind) {
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        self.transcript_runs.resize(self.transcript.len(), Vec::new()); // self-heal alignment
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default()); // self-heal alignment
        self.transcript_images.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(None);
            self.transcript_runs.push(Vec::new());
            self.transcript_para.push(ParaFmt::default());
            self.transcript_images.push(None);
        }
    }

    /// Add app-internal output (a `[…]` status line, a slash-command dump, a
    /// save/restore/copy banner) to the transcript. Like [`push_transcript_kind`],
    /// but in inline-prompt mode it inserts the line(s) ABOVE a trailing game `>`
    /// prompt so the prompt the caret sits at is never buried (SQ-0270).
    pub fn push_transcript_internal(&mut self, text: &str, kind: TranscriptKind) {
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        let base = self.insert_above_prompt_at();
        for (k, line) in text.split('\n').enumerate() {
            match base {
                Some(b) => {
                    let idx = b + k;
                    self.transcript.insert(idx, line.to_owned());
                    self.transcript_kinds.insert(idx, kind);
                    self.transcript_styles.insert(idx, None);
                    self.transcript_runs.insert(idx, Vec::new());
                    self.transcript_para.insert(idx, ParaFmt::default());
                    self.transcript_images.insert(idx, None);
                }
                None => {
                    self.transcript.push(line.to_owned());
                    self.transcript_kinds.push(kind);
                    self.transcript_styles.push(None);
                    self.transcript_runs.push(Vec::new());
                    self.transcript_para.push(ParaFmt::default());
                    self.transcript_images.push(None);
                }
            }
        }
    }

    /// Surface an app-internal `[…]` bracketed notice as a top-right toast
    /// (SQ-0176). It slides in, holds a few seconds, and slides out — it is NOT
    /// written to the transcript; `/dump-notifications` replays the history.
    pub fn push_notice(&mut self, text: &str) {
        self.notifications.push(text);
    }

    /// Append `text` to the last transcript line in place (used in inline-prompt
    /// mode so the typed command joins the game's `>` line). If the transcript is
    /// empty, push `text` as a new line instead. Only the last line's String is
    /// edited; its parallel runs/kinds/images/styles are left as-is (the appended
    /// chars carry no style runs and render in the input style).
    pub fn append_to_last_transcript_line(&mut self, text: &str) {
        if self.transcript.is_empty() {
            self.push_transcript_kind(text, TranscriptKind::Input);
            return;
        }
        self.bump_transcript_gen();
        let start = self.transcript.last().unwrap().chars().count();
        self.transcript.last_mut().unwrap().push_str(text);
        let end = start + text.chars().count();
        // Inherit the colour of the line's trailing style run so the echoed command
        // is drawn in the game's prompt colours instead of the uncoloured theme base,
        // and the SQ-0263 background band stays continuous across it. Only fg/bg are
        // carried (not reverse/bold bits or a hyperlink). No coloured run → plain
        // append (the theme case), unchanged. (SQ-0269)
        if let Some(runs) = self.transcript_runs.last_mut() {
            if let Some(&StyleRun { fg, bg, .. }) = runs.last() {
                if fg != 0 || bg != 0 {
                    runs.push(StyleRun { start, end, bits: 0, fg, bg, link: 0, glk_style: 0 });
                }
            }
        }
    }

    /// Merge transcript line `idx` into line `idx - 1`, concatenating the text and
    /// appending `idx`'s style runs shifted by the previous line's length, then
    /// removing line `idx`. Used to fold a game's own command echo (the first line
    /// of its turn output) onto the `>` prompt line, preserving the game's styling
    /// (e.g. CounterfeitMonkey's bold echo) so it reads as `>look` at the prompt
    /// rather than on a detached line below (SQ-0274). A no-op for `idx == 0` or
    /// out of range. Assumes the parallel arrays are length-aligned (the push
    /// methods maintain this).
    pub fn merge_line_into_previous(&mut self, idx: usize) {
        if idx == 0 || idx >= self.transcript.len() {
            return;
        }
        self.bump_transcript_gen();
        let base = self.transcript[idx - 1].chars().count();
        let moved = std::mem::take(&mut self.transcript[idx]);
        self.transcript[idx - 1].push_str(&moved);
        let shifted: Vec<StyleRun> = self
            .transcript_runs
            .get(idx)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|mut r| {
                r.start += base;
                r.end += base;
                r
            })
            .collect();
        if let Some(prev) = self.transcript_runs.get_mut(idx - 1) {
            prev.extend(shifted);
        }
        self.transcript.remove(idx);
        if idx < self.transcript_kinds.len() {
            self.transcript_kinds.remove(idx);
        }
        if idx < self.transcript_styles.len() {
            self.transcript_styles.remove(idx);
        }
        if idx < self.transcript_runs.len() {
            self.transcript_runs.remove(idx);
        }
        if idx < self.transcript_para.len() {
            self.transcript_para.remove(idx);
        }
        if idx < self.transcript_images.len() {
            self.transcript_images.remove(idx);
        }
    }

    /// The fg/bg of the most recent coloured style run in transcript lines before
    /// `upto` (scanning lines then runs from the end), or `None` if none is set.
    /// Approximates the game's "current" colour state at that point so a Default
    /// (unset) colour can be resolved to it rather than reset to the theme.
    pub fn prevailing_run_colour_before(&self, upto: usize) -> Option<(u32, u32)> {
        let end = upto.min(self.transcript_runs.len());
        for runs in self.transcript_runs[..end].iter().rev() {
            if let Some(r) = runs.iter().rev().find(|r| r.fg != 0 || r.bg != 0) {
                return Some((r.fg, r.bg));
            }
        }
        None
    }

    /// For transcript line `idx`, resolve every Default (0) fg/bg channel — in
    /// existing runs and on chars that carry no run — to `fg`/`bg`, preserving
    /// style bits, links, and any colour the game DID set. Used so a folded
    /// self-echo (which a game like CounterfeitMonkey prints in the default
    /// colour) keeps the game's current page colours instead of resetting to the
    /// theme (SQ-0274). No-op when both `fg` and `bg` are Default.
    pub fn fill_line_default_colours(&mut self, idx: usize, fg: u32, bg: u32) {
        if (fg == 0 && bg == 0) || idx >= self.transcript.len() {
            return;
        }
        let len = self.transcript[idx].chars().count();
        if len == 0 {
            return;
        }
        self.bump_transcript_gen();
        // Resolve per char: (bits, fg, bg, link, glk_style). Unstyled chars take the fill.
        let mut per: Vec<(u8, u32, u32, u32, u8)> = vec![(0, fg, bg, 0, 0); len];
        for r in self.transcript_runs.get(idx).cloned().unwrap_or_default() {
            for item in per.iter_mut().take(r.end.min(len)).skip(r.start) {
                *item = (
                    r.bits,
                    if r.fg != 0 { r.fg } else { fg },
                    if r.bg != 0 { r.bg } else { bg },
                    r.link,
                    r.glk_style,
                );
            }
        }
        // Coalesce adjacent identical cells back into runs.
        let mut runs: Vec<StyleRun> = Vec::new();
        for (c, &(bits, cf, cb, link, gs)) in per.iter().enumerate() {
            match runs.last_mut() {
                Some(last) if last.end == c && last.bits == bits && last.fg == cf && last.bg == cb && last.link == link && last.glk_style == gs => {
                    last.end = c + 1;
                }
                _ => runs.push(StyleRun { start: c, end: c + 1, bits, fg: cf, bg: cb, link, glk_style: gs }),
            }
        }
        if idx < self.transcript_runs.len() {
            self.transcript_runs[idx] = runs;
        }
    }

    /// Whether the last transcript line is game (Story) output — i.e. the game's
    /// inline `>` prompt is the last line, so the typed command can be appended to
    /// it. False when a non-game line (e.g. a `/help` Meta dump) is last, in which
    /// case the inline echo must go on its own line rather than corrupt that text.
    pub fn last_transcript_line_is_story(&self) -> bool {
        matches!(self.transcript_kinds.last(), Some(TranscriptKind::Story))
    }

    /// Append lines with the given kind and an explicit per-line render style.
    pub fn push_transcript_styled(&mut self, text: &str, kind: TranscriptKind, style: ratatui::style::Style) {
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        self.transcript_runs.resize(self.transcript.len(), Vec::new()); // self-heal alignment
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default()); // self-heal alignment
        self.transcript_images.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(Some(style));
            self.transcript_runs.push(Vec::new());
            self.transcript_para.push(ParaFmt::default());
            self.transcript_images.push(None);
        }
    }

    /// Like [`push_transcript_styled`], but inserts app-internal styled output
    /// above a trailing game prompt in inline-prompt mode (SQ-0270).
    pub fn push_transcript_internal_styled(&mut self, text: &str, kind: TranscriptKind, style: ratatui::style::Style) {
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        let base = self.insert_above_prompt_at();
        for (k, line) in text.split('\n').enumerate() {
            match base {
                Some(b) => {
                    let idx = b + k;
                    self.transcript.insert(idx, line.to_owned());
                    self.transcript_kinds.insert(idx, kind);
                    self.transcript_styles.insert(idx, Some(style));
                    self.transcript_runs.insert(idx, Vec::new());
                    self.transcript_para.insert(idx, ParaFmt::default());
                    self.transcript_images.insert(idx, None);
                }
                None => {
                    self.transcript.push(line.to_owned());
                    self.transcript_kinds.push(kind);
                    self.transcript_styles.push(Some(style));
                    self.transcript_runs.push(Vec::new());
                    self.transcript_para.push(ParaFmt::default());
                    self.transcript_images.push(None);
                }
            }
        }
    }

    /// Split `text` on `'\n'` and append each line tagged with `kind`, deriving a
    /// per-line `Vec<StyleRun>` from `chunks` — a `(char_count, bits, fg, bg)` list
    /// whose total char-count covers every char of `text` INCLUDING the `'\n'`
    /// separators (as recorded by `CaptureSink`). Adjacent equal-attribute spans
    /// merge; zero-bits/default-colour spans are omitted (so an unstyled line
    /// yields an empty run vec).
    ///
    /// A chunk's `nowrap` flag (printed while `buffer_mode` was off, ZMSD §7.2.1)
    /// is folded per line into [`ParaFmt::nowrap_from`] — the first char offset on
    /// that line that was emitted unbuffered — so the wrap can char-break from
    /// exactly there.
    pub fn push_transcript_runs(
        &mut self,
        text: &str,
        kind: TranscriptKind,
        chunks: &[crate::session::CaptureRun],
    ) {
        // A turn with no new lower-window output (e.g. a read_char keypress that
        // only redrew the upper window) yields an empty string; appending it would
        // add a spurious blank line (`"".split('\n')` yields one empty element),
        // scrolling the transcript up one row per keypress. Skip it.
        if text.is_empty() {
            return;
        }
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);

        // Walk `text` char-by-char while consuming the chunk list in lockstep.
        let mut chunk_iter = chunks.iter().copied();
        let mut rem: usize = 0;
        let mut bits: u8 = 0;
        let mut fg = zvm::screen::ZColour::Default;
        let mut bg = zvm::screen::ZColour::Default;
        let mut link: u32 = 0;
        let mut para = ParaFmt::default();
        let mut glk_style: u8 = 0;
        let mut nowrap = false;
        // Advance to the next non-exhausted chunk when the current one is spent.
        // When chunks run out, treat the remainder as plain (bits 0, default
        // colours, no link, default layout, Normal style, buffered).
        let mut refill = |rem: &mut usize,
                          bits: &mut u8,
                          fg: &mut zvm::screen::ZColour,
                          bg: &mut zvm::screen::ZColour,
                          link: &mut u32,
                          para: &mut ParaFmt,
                          glk_style: &mut u8,
                          nowrap: &mut bool| {
            while *rem == 0 {
                match chunk_iter.next() {
                    Some((c, b, f, bk, lk, pf, gs, nw)) => {
                        *rem = c;
                        *bits = b;
                        *fg = f;
                        *bg = bk;
                        *link = lk;
                        *para = pf;
                        *glk_style = gs;
                        *nowrap = nw;
                    }
                    None => {
                        *rem = usize::MAX;
                        *bits = 0;
                        *fg = zvm::screen::ZColour::Default;
                        *bg = zvm::screen::ZColour::Default;
                        *link = 0;
                        *para = ParaFmt::default();
                        *glk_style = 0;
                        *nowrap = false;
                        break;
                    }
                }
            }
        };

        let mut first = true;
        for line in text.split('\n') {
            // Consume the '\n' separator's chunk char (one per separator, i.e.
            // before every line except the first).
            if !first {
                refill(&mut rem, &mut bits, &mut fg, &mut bg, &mut link, &mut para, &mut glk_style, &mut nowrap);
                rem = rem.saturating_sub(1);
            }
            first = false;

            // The paragraph's layout comes from the FIRST content run on the line
            // (a paragraph is normally one style). `None` until the first char.
            let mut line_para: Option<ParaFmt> = None;
            // First column on this line printed with buffering off (§7.2.1).
            let mut line_nowrap: Option<u32> = None;
            let mut runs: Vec<StyleRun> = Vec::new();
            for (col, _ch) in line.chars().enumerate() {
                refill(&mut rem, &mut bits, &mut fg, &mut bg, &mut link, &mut para, &mut glk_style, &mut nowrap);
                if line_para.is_none() {
                    line_para = Some(para);
                }
                if nowrap && line_nowrap.is_none() {
                    line_nowrap = Some(col as u32);
                }
                let pfg = pack_zcolour(fg);
                let pbg = pack_zcolour(bg);
                // A run is also emitted for a non-Normal Glk style with no other
                // styling, so the theme's per-style colour slot can apply at render
                // (SQ-0331). A Normal (glk_style 0) unstyled char still yields no run.
                let has_style = bits != 0 || pfg != 0 || pbg != 0 || link != 0 || glk_style != 0;
                if has_style {
                    match runs.last_mut() {
                        Some(r) if r.end == col && r.bits == bits && r.fg == pfg && r.bg == pbg && r.link == link && r.glk_style == glk_style => {
                            r.end = col + 1;
                        }
                        _ => runs.push(StyleRun { start: col, end: col + 1, bits, fg: pfg, bg: pbg, link, glk_style }),
                    }
                }
                rem = rem.saturating_sub(1);
            }

            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(None);
            self.transcript_runs.push(runs);
            self.transcript_para.push(ParaFmt { nowrap_from: line_nowrap, ..line_para.unwrap_or_default() });
            self.transcript_images.push(None);
        }
    }

    /// Append a logical image unit: an empty placeholder line tagged `Story`
    /// carrying an inline image, keeping the parallel Vecs length-synced.
    pub fn push_transcript_image(&mut self, img: crate::inline_image::InlineImage) {
        self.bump_transcript_gen();
        self.transcript_styles.resize(self.transcript.len(), None);
        self.transcript_runs.resize(self.transcript.len(), Vec::new());
        self.transcript_para.resize(self.transcript.len(), ParaFmt::default());
        self.transcript_images.resize(self.transcript.len(), None);
        self.transcript.push(String::new());
        self.transcript_kinds.push(TranscriptKind::Story);
        self.transcript_styles.push(None);
        self.transcript_runs.push(Vec::new());
        self.transcript_para.push(ParaFmt::default());
        self.transcript_images.push(Some(img));
    }

    /// Reset the in-memory transcript sidecars (`transcript_styles`,
    /// `transcript_images`) to match a freshly reassigned `transcript`. Call this
    /// after replacing `transcript` wholesale (load / restore / reset / history
    /// jump). These sidecars carry no persisted data — the `.babelmap` archive
    /// stores only lines/kinds/runs (see `archive::TranscriptData`) — so the
    /// correct post-reassignment state is all-`None`, sized to the new transcript.
    /// A plain `resize` cannot do this: it only truncates the tail, leaving stale
    /// `Some(..)` at retained head indices (e.g. an inline image a Glulx game drew
    /// before the load, now indexing a different, shorter transcript).
    pub fn reset_transcript_sidecars(&mut self) {
        self.bump_transcript_gen();
        self.transcript_styles = vec![None; self.transcript.len()];
        self.transcript_images = vec![None; self.transcript.len()];
    }

    /// Surface a transient message as a top-right notification toast (SQ-0176).
    /// No longer reuses the score bar — the message slides in, holds a few
    /// seconds, and slides out; `/dump-notifications` replays the history.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.notifications.push(msg);
    }

    /// Set the selected room.
    pub fn select_room(&mut self, room: Option<RoomId>) {
        self.selected_room = room;
    }

    /// Note that the player just walked into `room`, for the maze breadcrumb (SQ-0666).
    ///
    /// A repeat of the room already at the head is dropped: a `look` or a failed move is not a
    /// step, and letting either one in would flush the trail with eight copies of where you are
    /// standing. Bounded at [`MAP_TRAIL_LEN`].
    pub fn push_trail(&mut self, room: RoomId) {
        if self.map_trail.back() == Some(&room) {
            return;
        }
        self.map_trail.push_back(room);
        while self.map_trail.len() > MAP_TRAIL_LEN {
            self.map_trail.pop_front();
        }
    }

    /// How far back in the trail `room` is: 0 for the room just entered, `None` when it is not on
    /// the trail at all. Drives the breadcrumb's fade.
    pub fn trail_age(&self, room: RoomId) -> Option<usize> {
        self.map_trail.iter().rev().position(|&r| r == room)
    }

    /// Insert a character at the caret.
    pub fn push_input_char(&mut self, c: char) {
        self.input.insert(c);
    }

    /// Delete the character before the caret, if any.
    pub fn backspace(&mut self) {
        self.input.backspace();
    }

    /// Return the current input line and clear it. Also clears autocomplete state.
    pub fn take_input(&mut self) -> String {
        self.suggestions.clear();
        self.suggestion_idx = 0;
        self.suggestion_active = false;
        self.input.take()
    }

    /// Char index in the input line under a click at screen `(col, row)`, or `None` when the click
    /// is not on that line (SQ-0354).
    ///
    /// A click past the last character clamps to the end, which is what every other text field
    /// does — clicking the empty space after a line puts the caret at its end.
    ///
    /// `col - x0` is a CELL offset, and a cell is not a char: a double-width glyph
    /// earlier in the line pushes everything after it one column right, so the offset
    /// has to be converted through the line's display widths or the caret lands short
    /// of the click (SQ-0655). Either cell of a wide glyph puts the caret before it.
    pub fn input_click_index(&self, col: u16, row: u16) -> Option<usize> {
        let (x0, y0) = self.input_text_origin.get()?;
        if row != y0 || col < x0 {
            return None;
        }
        let cell = (col - x0) as usize;
        Some(crate::textwidth::col_to_char_idx(self.input.as_str(), cell).min(self.input.char_len()))
    }

    /// Clear the current autocomplete suggestions.
    pub fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggestion_idx = 0;
        self.suggestion_active = false;
    }

    /// Return the partial word the player is currently typing (the last
    /// whitespace-delimited token in `input`).
    pub fn current_partial(&self) -> &str {
        // Find the last space; if none, the whole input is the partial word.
        match self.input.value.rfind(' ') {
            Some(pos) => &self.input.value[pos + 1..],
            None => &self.input.value,
        }
    }

    // ── Command history (Up/Down recall) ──────────────────────────────────────

    /// Record a submitted command line into `command_history`.
    ///
    /// No-op when `line.trim()` is empty. A line equal to the current last entry
    /// is skipped (consecutive-duplicate dedupe). The list is capped at
    /// `COMMAND_HISTORY_CAP` (oldest dropped). Always resets the navigation cursor
    /// and clears the saved draft.
    pub fn record_command(&mut self, line: &str) {
        self.history_cursor = None;
        self.history_draft.clear();
        if line.trim().is_empty() {
            return;
        }
        if self.command_history.last().map(String::as_str) == Some(line) {
            return;
        }
        self.command_history.push(line.to_string());
        if self.command_history.len() > COMMAND_HISTORY_CAP {
            let overflow = self.command_history.len() - COMMAND_HISTORY_CAP;
            self.command_history.drain(0..overflow);
        }
    }

    /// Recall the previous (older) command into the input buffer (Up arrow).
    ///
    /// The first Up saves the in-progress input as the draft. At the oldest entry
    /// further Up is a no-op. No-op when the history is empty.
    pub fn history_prev(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        let next = match self.history_cursor {
            None => {
                self.history_draft = self.input.value.clone();
                self.command_history.len() - 1
            }
            Some(0) => return, // oldest: stay
            Some(i) => i - 1,
        };
        self.history_cursor = Some(next);
        self.input.set(self.command_history[next].clone(), true);
        self.clear_suggestions();
    }

    /// Recall the next (newer) command into the input buffer (Down arrow).
    ///
    /// Stepping past the newest entry restores the saved draft and leaves
    /// navigation. No-op when not currently navigating.
    pub fn history_next(&mut self) {
        let i = match self.history_cursor {
            None => return,
            Some(i) => i,
        };
        if i + 1 < self.command_history.len() {
            self.history_cursor = Some(i + 1);
            self.input.set(self.command_history[i + 1].clone(), true);
        } else {
            // Past the newest entry: restore the draft.
            self.history_cursor = None;
            self.input.set(std::mem::take(&mut self.history_draft), true);
        }
        self.clear_suggestions();
    }

    // ── Search helpers ────────────────────────────────────────────────────────

    /// Run a case-insensitive substring search over the visible transcript lines.
    ///
    /// Fills `search_matches` with the 0-based positions (within the visible list)
    /// of lines that contain `query`. Sets `search_idx` to the last match index
    /// when `start_backward` is true (landing on the most recent match), or the
    /// first (0) when false. Sets `search_query` to the query string regardless of
    /// whether matches were found (so the status line can show "no matches").
    /// Returns the number of matches.
    pub fn run_search(&mut self, query: &str, start_backward: bool) -> usize {
        let query_lower = query.to_lowercase();
        let visible = self.visible_transcript_indices();
        self.search_matches = visible
            .iter()
            .enumerate()
            .filter(|&(_, &raw_idx)| {
                self.transcript[raw_idx].to_lowercase().contains(&query_lower)
            })
            .map(|(pos, _)| pos)
            .collect();
        let count = self.search_matches.len();
        self.search_idx = if start_backward && count > 0 { count - 1 } else { 0 };
        self.search_query = Some(query.to_string());
        count
    }

    /// Advance the current match by one step and return the new match's visible-list position.
    ///
    /// `forward = true` moves toward the end (newer lines); `forward = false` moves
    /// toward the start (older lines). Both directions wrap around. Returns `None` if
    /// there are no matches.
    pub fn search_next(&mut self, forward: bool) -> Option<usize> {
        let count = self.search_matches.len();
        if count == 0 {
            return None;
        }
        if forward {
            self.search_idx = (self.search_idx + 1) % count;
        } else {
            self.search_idx = self.search_idx.checked_sub(1).unwrap_or(count - 1);
        }
        Some(self.search_matches[self.search_idx])
    }

    /// Clear all search state: query, matches, and index.
    pub fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.search_idx = 0;
    }
}

/// Append this render pass's pipeline stage labels to trace.log when `on`. (trace feature)
pub(crate) fn write_map_trace(user_dir: &std::path::Path, steps: &[String], on: bool) {
    if on {
        crate::trace::write(user_dir, crate::trace::Section::Map, steps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_history_defaults_empty() {
        let s = AppState::default();
        assert!(s.history.is_empty(), "history starts empty");
    }

    #[test]
    fn filename_modal_for_picks_prompt_picker_or_autocancel() {
        use super::{filename_modal_for, FilenameModal};
        use crate::session::FilenameReq;
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x01 }, 3), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x05 }, 0), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x03 }, 0), FilenameModal::NamePrompt);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 2), FilenameModal::Picker);
        assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 0), FilenameModal::AutoCancel);
    }

    #[test]
    fn file_picker_navigation_clamps_and_selects() {
        use super::FilePickerState;
        let mut p = FilePickerState::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(p.selected(), Some("a"));
        p.move_up(); // clamps at top
        assert_eq!(p.selected(), Some("a"));
        p.move_down();
        p.move_down();
        assert_eq!(p.selected(), Some("c"));
        p.move_down(); // clamps at bottom
        assert_eq!(p.selected(), Some("c"));
    }

    #[test]
    fn reset_sound_sidecars_clears_tracking_maps() {
        let mut state = AppState::default();
        state.sound_ids.insert(3, 1);
        state.sound_routines.insert(1, 0x1234);
        state.glulx_channels.insert(1, 5);
        state.glulx_sound_notify.insert(5, (7, 42));
        state.glulx_volume_notify.insert(1, (std::time::Instant::now(), 9));
        state.glulx_gain.insert(1, 0.5);
        state.glulx_volume_ramp.insert(1, VolumeRamp {
            start: std::time::Instant::now(), duration_ms: 500, start_gain: 1.0, target_gain: 0.0,
        });
        state.reset_sound_sidecars();
        assert!(state.sound_ids.is_empty());
        assert!(state.sound_routines.is_empty());
        assert!(state.glulx_channels.is_empty());
        assert!(state.glulx_sound_notify.is_empty());
        assert!(state.glulx_volume_notify.is_empty());
        assert!(state.glulx_gain.is_empty());
        assert!(state.glulx_volume_ramp.is_empty());
    }

    #[test]
    fn ramp_gain_interpolates_at_endpoints_and_midpoint() {
        // t=0 → start; t=mid → linear midpoint; t>=end (and zero duration) → target.
        assert_eq!(ramp_gain(1.0, 0.0, 0, 1000), 1.0, "t=0 sits at the start gain");
        assert_eq!(ramp_gain(1.0, 0.0, 500, 1000), 0.5, "halfway is the midpoint");
        assert_eq!(ramp_gain(1.0, 0.0, 1000, 1000), 0.0, "t=end lands on target");
        assert_eq!(ramp_gain(1.0, 0.0, 5000, 1000), 0.0, "past the end clamps to target");
        assert_eq!(ramp_gain(0.2, 0.8, 0, 0), 0.8, "zero duration is an immediate jump");
        // Rising ramp, quarter point.
        assert!((ramp_gain(0.0, 1.0, 250, 1000) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn set_volume_ext_ramps_interpolates_interrupts_and_completes() {
        use crate::session::SchannelOp;
        let mut state = AppState::default();
        state.config.enable_sound = true;
        audio::disable_output_for_tests(); // silent backend: no real device to open/tear down
        state.audio = Some(audio::AudioBackend::new(50));
        // Seed the channel's current gain (as a prior play/set_volume would).
        state.glulx_gain.insert(1, 1.0);
        // A ramped set_volume_ext installs a ramp from the current gain to target
        // (vol 0 → gain 0) and does NOT jump the stored gain immediately.
        let t0 = std::time::Instant::now();
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0, duration_ms: 1000, notify: 7 }]);
        let ramp = *state.glulx_volume_ramp.get(&1).expect("ramp installed");
        assert_eq!(ramp.start_gain, 1.0, "ramp starts from the channel's current gain");
        assert_eq!(ramp.target_gain, 0.0, "ramp targets the new volume's gain");
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(7), "notify still scheduled at the deadline");
        // Halfway through, the interpolated gain is ~0.5.
        state.advance_volume_ramps(t0 + std::time::Duration::from_millis(500));
        let mid = *state.glulx_gain.get(&1).unwrap();
        assert!((mid - 0.5).abs() < 0.1, "gain interpolates to ~0.5 at half-time, got {mid}");
        assert!(state.glulx_volume_ramp.contains_key(&1), "ramp still active mid-way");
        // A new change interrupts: the old ramp is replaced and heads to the new
        // target from wherever the gain currently sits (spec §8.3).
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x10000, duration_ms: 1000, notify: 9 }]);
        let ramp2 = *state.glulx_volume_ramp.get(&1).expect("new ramp installed");
        assert_eq!(ramp2.target_gain, 1.0, "interruption retargets to the new volume");
        assert!((ramp2.start_gain - mid).abs() < 1e-6, "new ramp starts from the current gain");
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(9), "the prior notify was dropped, the new one scheduled");
        // Stepping past the duration completes the ramp: gain lands on target and
        // the ramp is removed.
        state.advance_volume_ramps(ramp2.start + std::time::Duration::from_millis(1000));
        assert_eq!(*state.glulx_gain.get(&1).unwrap(), 1.0, "completed ramp lands on target");
        assert!(!state.glulx_volume_ramp.contains_key(&1), "completed ramp is dropped");
        // An immediate (duration 0) change jumps and installs no ramp.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolume { chan: 1, vol: 0x8000 }]);
        assert_eq!(*state.glulx_gain.get(&1).unwrap(), 0.5, "plain set_volume jumps the gain");
        assert!(!state.glulx_volume_ramp.contains_key(&1), "plain set_volume leaves no ramp");
    }

    #[test]
    fn play_glulx_sound_ops_schedules_and_interrupts_volume_notify() {
        use crate::session::SchannelOp;
        let mut state = AppState::default();
        state.config.enable_sound = true;
        audio::disable_output_for_tests(); // silent backend: no real device to open/tear down
        state.audio = Some(audio::AudioBackend::new(50));
        // A ramped set_volume_ext with a nonzero notify schedules a pending
        // volume-notify keyed by channel (no live sound needed).
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x8000, duration_ms: 1000, notify: 7 }]);
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(7), "notify scheduled");
        // A second change on the same channel interrupts the first (spec §8.3):
        // the prior notify is dropped and replaced.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0x2000, duration_ms: 500, notify: 9 }]);
        assert_eq!(state.glulx_volume_notify.get(&1).map(|&(_, n)| n), Some(9), "new change replaces the prior notify");
        // notify == 0 requests no event → schedules nothing and clears the channel.
        state.play_glulx_sound_ops(&[SchannelOp::SetVolumeExt { chan: 1, vol: 0, duration_ms: 0, notify: 0 }]);
        assert!(state.glulx_volume_notify.is_empty(), "notify=0 schedules no event and clears the channel");
        // Destroy cancels a channel's pending ramp notify.
        state.play_glulx_sound_ops(&[
            SchannelOp::SetVolumeExt { chan: 2, vol: 0x8000, duration_ms: 1000, notify: 3 },
            SchannelOp::Destroy { chan: 2 },
        ]);
        assert!(state.glulx_volume_notify.is_empty(), "destroy cancels the pending volume-notify");
    }

    #[test]
    fn turn_elems_interleave_text_and_image_in_transcript() {
        use crate::session::TranscriptElem;
        let mut st = AppState::default();
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None, source: crate::inline_image::ImageSource::Story,
        };
        let elems = vec![
            TranscriptElem::Text { text: "a".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false)] },
            TranscriptElem::Image(dummy),
            TranscriptElem::Text { text: "b".into(), runs: vec![(1, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false)] },
        ];
        apply_transcript_elems(&mut st, &elems);
        assert_eq!(st.transcript, vec!["a".to_string(), "".to_string(), "b".to_string()]);
        assert!(st.transcript_images[1].is_some());
        assert!(st.transcript_images[0].is_none() && st.transcript_images[2].is_none());
    }

    // ── Command history (feature D) ────────────────────────────────────────────

    #[test]
    fn record_command_skips_empty_and_consecutive_duplicates() {
        let mut s = AppState::default();
        s.record_command("north");
        s.record_command("north"); // consecutive dup → skipped
        s.record_command("   ");    // blank → skipped
        s.record_command("south");
        s.record_command("north"); // not consecutive → recorded
        assert_eq!(s.command_history, vec!["north", "south", "north"]);
    }

    #[test]
    fn record_command_caps_at_500_dropping_oldest() {
        let mut s = AppState::default();
        for i in 0..600 {
            s.record_command(&format!("cmd{i}"));
        }
        assert_eq!(s.command_history.len(), COMMAND_HISTORY_CAP);
        assert_eq!(s.command_history.first().unwrap(), "cmd100");
        assert_eq!(s.command_history.last().unwrap(), "cmd599");
    }

    #[test]
    fn record_command_resets_cursor_and_draft() {
        let mut s = AppState::default();
        s.command_history = vec!["a".into(), "b".into()];
        s.history_cursor = Some(0);
        s.history_draft = "draft".into();
        s.record_command("c");
        assert_eq!(s.history_cursor, None);
        assert!(s.history_draft.is_empty());
    }

    #[test]
    fn history_up_down_recall_with_draft_save_and_restore() {
        let mut s = AppState::default();
        s.command_history = vec!["one".into(), "two".into(), "three".into()];
        s.input = "partial".into(); // in-progress draft

        // First Up: save draft, recall newest ("three").
        s.history_prev();
        assert_eq!(s.input.value, "three");
        // Up again: "two".
        s.history_prev();
        assert_eq!(s.input.value, "two");
        // Up again: "one" (oldest).
        s.history_prev();
        assert_eq!(s.input.value, "one");
        // Up at oldest: no-op (stays).
        s.history_prev();
        assert_eq!(s.input.value, "one");
        // Down: "two".
        s.history_next();
        assert_eq!(s.input.value, "two");
        // Down: "three".
        s.history_next();
        assert_eq!(s.input.value, "three");
        // Down past newest: restore the saved draft.
        s.history_next();
        assert_eq!(s.input.value, "partial");
        assert_eq!(s.history_cursor, None);
        // Down again while not navigating: no-op.
        s.history_next();
        assert_eq!(s.input.value, "partial");
    }

    #[test]
    fn history_prev_on_empty_history_is_noop() {
        let mut s = AppState::default();
        s.input = "x".into();
        s.history_prev();
        assert_eq!(s.input.value, "x");
        assert_eq!(s.history_cursor, None);
    }

    #[test]
    fn replay_state_step_clamps_and_pauses() {
        let mut r = ReplayState::new(4); // start at last idx
        assert_eq!(r.idx, 4);
        r.step(-1, 5);
        assert_eq!(r.idx, 3);
        assert!(!r.playing, "manual step pauses");
        r.step(-10, 5);
        assert_eq!(r.idx, 0, "clamped at 0");
        r.step(10, 5);
        assert_eq!(r.idx, 4, "clamped at len-1");
    }

    #[test]
    fn replay_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.replay = Some(ReplayState::new(0));
        assert!(s.any_overlay_open(), "replay open => any_overlay_open true");
    }

    #[test]
    fn filter_maps_input_with_story_and_warning_with_meta() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("> go north", TranscriptKind::Input);
        s.push_transcript_kind("meta", TranscriptKind::Meta);
        s.push_transcript_kind("warn", TranscriptKind::Warning);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1]); // Story + Input
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![2, 3]); // Meta + Warning
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn current_room_name_defaults_none() {
        let s = AppState::default();
        assert_eq!(s.current_room_name, None);
    }

    #[test]
    fn visible_transcript_indices_respects_filter() {
        let mut s = AppState::default();
        s.push_transcript("story0");
        s.push_transcript_kind("meta1", TranscriptKind::Meta);
        s.push_transcript("story2");
        s.transcript_filter = TranscriptFilter::Both;
        assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2]);
        s.transcript_filter = TranscriptFilter::Story;
        assert_eq!(s.visible_transcript_indices(), vec![0, 2]);
        s.transcript_filter = TranscriptFilter::Meta;
        assert_eq!(s.visible_transcript_indices(), vec![1]);
    }

    #[test]
    fn transcript_tags_story_and_meta() {
        let mut s = AppState::default();
        s.push_transcript("West of House");
        s.push_transcript_kind("/help line", TranscriptKind::Meta);
        // last entry is Meta, prior is Story
        assert_eq!(s.transcript_kinds.len(), 2);
        assert!(matches!(s.transcript_kinds[0], TranscriptKind::Story));
        assert!(matches!(s.transcript_kinds[1], TranscriptKind::Meta));
    }

    #[test]
    fn append_to_last_transcript_line_appends_to_existing_last_line() {
        let mut s = AppState::default();
        s.push_transcript_kind(">", TranscriptKind::Story);
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript[0], ">look");
    }

    #[test]
    fn append_to_last_transcript_line_pushes_new_line_when_empty_and_keeps_arrays_aligned() {
        let mut s = AppState::default();
        assert!(s.transcript.is_empty());
        s.append_to_last_transcript_line("hi");
        assert_eq!(s.transcript, vec!["hi".to_string()]);
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_styles.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
    }

    #[test]
    fn append_to_last_transcript_line_inherits_trailing_run_colour() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // A coloured game prompt line: ">" drawn on a white page background.
        s.push_transcript_runs(
            ">",
            TranscriptKind::Story,
            &[(1, 0, ZColour::Default, ZColour::True24(0x00FF_FFFF), 0, ParaFmt::default(), 0, false)],
        );
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.last().unwrap(), ">look");
        let tail = s.transcript_runs.last().unwrap().last().unwrap();
        assert_eq!((tail.start, tail.end), (1, 5), "run covers the appended 'look'");
        assert_eq!(tail.bg, pack_zcolour(ZColour::True24(0x00FF_FFFF)),
            "echo inherits the prompt's white background (keeps the SQ-0263 band)");
        assert_eq!(tail.bits, 0, "reverse/bold bits are not carried onto the echo");
    }

    #[test]
    fn append_to_last_transcript_line_adds_no_run_when_prompt_is_uncoloured() {
        let mut s = AppState::default();
        s.push_transcript_kind(">", TranscriptKind::Story); // no coloured runs
        let before = s.transcript_runs.last().map(|r| r.len()).unwrap_or(0);
        s.append_to_last_transcript_line("look");
        assert_eq!(s.transcript.last().unwrap(), ">look");
        let after = s.transcript_runs.last().map(|r| r.len()).unwrap_or(0);
        assert_eq!(before, after, "no coloured trailing run → plain append (theme case unchanged)");
    }

    #[test]
    fn merge_line_into_previous_folds_game_echo_onto_the_prompt() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // The `>` prompt line (bare, no runs), then the game's own bold echo "look"
        // as a separate pushed line — CounterfeitMonkey's shape.
        s.push_transcript_kind(">", TranscriptKind::Story);
        s.push_transcript_runs("look", TranscriptKind::Story,
            &[(4, 2 /* bold */, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert_eq!(s.transcript.len(), 2);
        s.merge_line_into_previous(1);
        // One line now: ">look", and the bold run is shifted past the ">".
        assert_eq!(s.transcript, vec![">look".to_string()]);
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
        let run = s.transcript_runs[0].iter().find(|r| r.bits == 2).expect("bold run kept");
        assert_eq!((run.start, run.end), (1, 5), "bold echo run shifted past the `>`");
    }

    #[test]
    fn fill_line_default_colours_preserves_current_colour_and_keeps_game_overrides() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        let white = pack_zcolour(ZColour::True24(0x00FF_FFFF));
        let black = pack_zcolour(ZColour::True24(0));
        let red = pack_zcolour(ZColour::True24(0x00FF_0000));
        // A `>look` line where `>` has no run and "look" is bold with DEFAULT colour,
        // except a single char CM coloured red explicitly.
        s.push_transcript_runs(">look", TranscriptKind::Story, &[
            (1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // '>' — unstyled
            (1, 2, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // 'l' — bold, default colour
            (1, 2, ZColour::True24(0x00FF_0000), ZColour::Default, 0, ParaFmt::default(), 0, false), // 'o' — bold + explicit red fg
            (2, 2, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),       // 'ok' — bold, default colour
        ]);
        s.fill_line_default_colours(0, black, white);
        // Every char now has fg/bg resolved; the '>' and default chars take black/white,
        // the explicit red fg is kept, bold bits preserved.
        let runs = &s.transcript_runs[0];
        // Char 0 ('>'): no bits, black on white.
        let r0 = runs.iter().find(|r| r.start == 0).unwrap();
        assert_eq!((r0.bits, r0.fg, r0.bg), (0, black, white), "bare '>' gets current colours");
        // The explicit-red char keeps red fg but gains the current white bg.
        let red_run = runs.iter().find(|r| r.fg == red).expect("explicit red preserved");
        assert_eq!((red_run.bits, red_run.bg), (2, white), "override kept, default bg filled, bold kept");
        // A default bold char: bold + black on white.
        assert!(runs.iter().any(|r| r.bits == 2 && r.fg == black && r.bg == white),
            "default bold echo chars take the current black-on-white");
    }

    #[test]
    fn merge_line_into_previous_is_a_noop_at_zero_or_out_of_range() {
        let mut s = AppState::default();
        s.push_transcript_kind("only", TranscriptKind::Story);
        s.merge_line_into_previous(0);
        s.merge_line_into_previous(5);
        assert_eq!(s.transcript, vec!["only".to_string()]);
    }

    #[test]
    fn internal_messages_insert_above_the_inline_prompt() {
        let mut s = AppState::default();
        s.config.command_bar = false; // inline-prompt mode
        // Game output goes through push_transcript_runs (appends); it ends with the
        // kept `>` prompt.
        s.push_transcript_runs("You are in a hall.", TranscriptKind::Story, &[]);
        s.push_transcript_runs(">", TranscriptKind::Story, &[]);
        // A /help-style internal dump must land ABOVE the `>`, keeping it last.
        s.push_transcript_internal("help: N, S, LOOK, X", TranscriptKind::Meta);
        assert_eq!(
            s.transcript,
            vec!["You are in a hall.".to_string(), "help: N, S, LOOK, X".to_string(), ">".to_string()],
        );
        assert!(s.last_transcript_line_is_story(), "the `>` prompt stays the last line");
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
        assert_eq!(s.transcript.len(), s.transcript_styles.len());
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_images.len());
        assert!(matches!(s.transcript_kinds[1], TranscriptKind::Meta), "inserted line keeps its kind");

        // Multi-line internal output preserves order above the prompt.
        s.push_transcript_internal("a\nb", TranscriptKind::Meta);
        assert_eq!(s.transcript.last().unwrap(), ">");
        assert_eq!(&s.transcript[s.transcript.len() - 3..], &["a".to_string(), "b".to_string(), ">".to_string()]);
    }

    #[test]
    fn internal_messages_append_in_command_bar_mode_or_without_a_prompt() {
        // Command-bar mode: the `>` isn't in the transcript, so append normally.
        let mut s = AppState::default();
        s.config.command_bar = true;
        s.push_transcript_runs(">", TranscriptKind::Story, &[]);
        s.push_transcript_internal("help", TranscriptKind::Meta);
        assert_eq!(s.transcript, vec![">".to_string(), "help".to_string()]);
        // Inline mode but the last line isn't a game prompt (Meta) → append.
        let mut s2 = AppState::default();
        s2.config.command_bar = false;
        s2.push_transcript_internal("note", TranscriptKind::Meta);
        s2.push_transcript_internal("more", TranscriptKind::Meta);
        assert_eq!(s2.transcript, vec!["note".to_string(), "more".to_string()]);
    }

    #[test]
    fn last_transcript_line_is_story_distinguishes_game_output_from_meta() {
        let mut s = AppState::default();
        assert!(!s.last_transcript_line_is_story(), "empty transcript is not a story prompt");
        s.push_transcript_kind(">", TranscriptKind::Story);
        assert!(s.last_transcript_line_is_story(), "game `>` prompt is a story line");
        // A /help-style meta dump lands after the prompt: no longer a story last line.
        s.push_transcript_kind("help: available commands", TranscriptKind::Meta);
        assert!(!s.last_transcript_line_is_story(), "meta dump is not the game prompt");
    }

    #[test]
    fn transcript_styles_track_and_self_heal() {
        use ratatui::style::{Color, Style};
        let mut s = AppState::default();
        s.push_transcript_kind("a", TranscriptKind::Meta);
        let cyan = Style::new().fg(Color::Cyan);
        s.push_transcript_styled("b", TranscriptKind::Meta, cyan);
        assert_eq!(s.transcript.len(), s.transcript_styles.len(), "lengths stay equal");
        assert_eq!(s.transcript_styles[0], None, "plain push has no override");
        assert_eq!(s.transcript_styles[1], Some(cyan), "styled push records the style");

        // Simulate a wholesale reassignment that leaves transcript_styles short.
        s.transcript = vec!["x".into(), "y".into(), "z".into()];
        s.transcript_kinds = vec![TranscriptKind::Story; 3];
        s.push_transcript_kind("w", TranscriptKind::Meta); // must self-heal
        assert_eq!(s.transcript.len(), s.transcript_styles.len(), "self-heal re-aligns lengths");
    }

    #[test]
    fn push_transcript_image_keeps_parallel_vecs_synced() {
        let mut st = AppState::default();
        st.push_transcript("hello");
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None, source: crate::inline_image::ImageSource::Story,
        };
        st.push_transcript_image(dummy);
        st.push_transcript("world");
        let n = st.transcript.len();
        assert_eq!(st.transcript_kinds.len(), n);
        assert_eq!(st.transcript_styles.len(), n);
        assert_eq!(st.transcript_runs.len(), n);
        assert_eq!(st.transcript_images.len(), n);
        // The image unit sits between the two text lines.
        let img_idx = st.transcript_images.iter().position(|o| o.is_some()).unwrap();
        assert_eq!(st.transcript[img_idx], "");
        assert_eq!(st.transcript_kinds[img_idx], TranscriptKind::Story);
        assert!(st.transcript_images.iter().filter(|o| o.is_some()).count() == 1);
        assert_eq!(st.transcript_para.len(), n, "transcript_para stays length-synced");
    }

    #[test]
    fn push_transcript_runs_captures_paragraph_layout_from_first_run() {
        // SQ-0330: a chunk carrying a Centered ParaFmt sets the line's layout, and
        // the parallel `transcript_para` stays length-synced. A default-layout line
        // keeps the default ParaFmt (the Z-machine path).
        let mut st = AppState::default();
        let centered = ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None };
        st.push_transcript_runs(
            "plain\ncentered",
            TranscriptKind::Story,
            &[
                (6, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, ParaFmt::default(), 0, false), // "plain\n"
                (8, 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default, 0, centered, 0, false),        // "centered"
            ],
        );
        assert_eq!(st.transcript, vec!["plain".to_string(), "centered".to_string()]);
        assert_eq!(st.transcript_para.len(), st.transcript.len());
        assert_eq!(st.transcript_para[0], ParaFmt::default(), "first line is left/no-indent");
        assert_eq!(st.transcript_para[1], centered, "second line takes its run's Centered layout");
    }

    #[test]
    fn truncate_transcript_trims_all_sidecars_and_bumps_gen() {
        // SQ-0407: collapsing a menu reprint truncates the transcript AND every
        // parallel sidecar vec (they must stay length-synced) and invalidates the
        // wrap cache via the generation bump.
        let mut s = AppState::default();
        s.push_transcript("line1");
        s.push_transcript("line2");
        s.push_transcript("line3");
        let gen = s.transcript_gen;

        s.truncate_transcript(1);
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript_kinds.len(), 1);
        assert_eq!(s.transcript_styles.len(), 1);
        assert_eq!(s.transcript_runs.len(), 1);
        assert_eq!(s.transcript_para.len(), 1);
        assert_eq!(s.transcript_images.len(), 1);
        assert_ne!(s.transcript_gen, gen, "truncation must bump the wrap-cache generation");

        // No-op when the length is already at/beyond the end.
        let gen2 = s.transcript_gen;
        s.truncate_transcript(5);
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.transcript_gen, gen2, "a no-op truncate must not bump the generation");
    }

    #[test]
    fn reset_transcript_sidecars_clears_stale_head_entries() {
        // Reproduce the load/restore/reset bug: a Glulx game drew an inline image
        // (sidecar holds `Some` at a head index), then the transcript is replaced
        // wholesale with a SHORTER one. `resize` alone would keep the stale `Some`
        // at index 0 indexing the wrong line; `reset_transcript_sidecars` must wipe
        // it and length-match the new transcript.
        let mut s = AppState::default();
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(4, 4)),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None, source: crate::inline_image::ImageSource::Story,
        };
        s.push_transcript_image(dummy); // transcript_images[0] = Some, len 1
        s.push_transcript("a\nb"); // grow to len 3
        assert!(s.transcript_images[0].is_some(), "precondition: stale image present");

        // Wholesale reassign to a shorter transcript, then apply the fix.
        s.transcript = vec!["only".into()];
        s.reset_transcript_sidecars();

        assert_eq!(s.transcript_images.len(), s.transcript.len(), "length matches");
        assert_eq!(s.transcript_styles.len(), s.transcript.len(), "length matches");
        assert!(
            s.transcript_images.iter().all(Option::is_none),
            "no stale Some survives within range",
        );
    }

    #[test]
    fn push_runs_extracts_per_line_spans() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("ab cd", TranscriptKind::Story,
            &[(2, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false), (3, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert_eq!(s.transcript.last().unwrap(), "ab cd");
        assert_eq!(s.transcript_runs.last().unwrap(), &vec![StyleRun { start: 0, end: 2, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
    }

    #[test]
    fn push_runs_splits_across_newlines() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("A\nB", TranscriptKind::Story,
            &[(1, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
              (1, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false),
              (1, 0x02, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        let n = s.transcript.len();
        assert_eq!(s.transcript[n - 2], "A");
        assert_eq!(s.transcript[n - 1], "B");
        assert_eq!(s.transcript_runs[n - 2], vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
        assert_eq!(s.transcript_runs[n - 1], vec![StyleRun { start: 0, end: 1, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    #[test]
    fn push_runs_carries_hyperlink_value() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        // Two adjacent spans with DIFFERENT Glk hyperlink values (42 then 99):
        // the link value must be carried AND be part of the coalescing key, so
        // the two spans stay separate runs. (Unstyled/default spans emit no run,
        // which is why both spans here carry a nonzero link.)
        s.push_transcript_runs("link more", TranscriptKind::Story,
            &[(4, 0, ZColour::Default, ZColour::Default, 42, ParaFmt::default(), 0, false),
              (5, 0, ZColour::Default, ZColour::Default, 99, ParaFmt::default(), 0, false)]);
        let runs = s.transcript_runs.last().unwrap();
        assert_eq!(runs.len(), 2, "differing links do not coalesce");
        assert_eq!((runs[0].start, runs[0].end, runs[0].link), (0, 4, 42));
        assert_eq!((runs[1].start, runs[1].end, runs[1].link), (4, 9, 99));
    }

    #[test]
    fn style_run_link_defaults_to_zero_from_old_json() {
        // Old transcript.json has no `link` field; #[serde(default)] must load it as 0.
        let old: StyleRun =
            serde_json::from_str(r#"{"start":0,"end":3,"bits":2,"fg":0,"bg":0}"#).unwrap();
        assert_eq!(old.link, 0, "a missing link field defaults to 0");
    }

    #[test]
    fn push_runs_all_plain_is_empty() {
        use zvm::screen::ZColour;
        let mut s = AppState::default();
        s.push_transcript_runs("hello", TranscriptKind::Story,
            &[(5, 0, ZColour::Default, ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }

    /// ZMSD §7.2.1: a run captured with `buffer_mode` off marks its line's
    /// `nowrap_from` at the first unbuffered column, per line (offsets are
    /// line-relative, and a fully buffered line stays `None`).
    #[test]
    fn push_runs_folds_buffering_off_into_para_nowrap_from() {
        use zvm::screen::ZColour;
        let plain = |n: usize, nw: bool| (n, 0u8, ZColour::Default, ZColour::Default, 0u32, ParaFmt::default(), 0u8, nw);
        let mut s = AppState::default();
        // "buffered\nmixed" + "DOTS" printed unbuffered on the second line.
        s.push_transcript_runs(
            "buffered\nmixedDOTS",
            TranscriptKind::Story,
            &[plain(14, false), plain(4, true)], // 8 + '\n' + 5 chars, then "DOTS"
        );
        let n = s.transcript_para.len();
        assert_eq!(s.transcript_para[n - 2].nowrap_from, None, "fully buffered line");
        assert_eq!(s.transcript_para[n - 1].nowrap_from, Some(5), "offset is line-relative");
    }

    #[test]
    fn push_runs_empty_text_adds_no_line() {
        // Regression (BeyondZork v4+ upper-window menu): a read_char turn that only
        // redraws the upper window emits NO new lower-window text. Pushing that
        // empty transcript must append nothing — otherwise each keypress added a
        // blank line and scrolled the (bottom-anchored) transcript up one row.
        let mut s = AppState::default();
        let before = s.transcript.len();
        s.push_transcript_runs("", TranscriptKind::Story, &[]);
        assert_eq!(s.transcript.len(), before, "empty transcript must add no line");
    }

    #[test]
    fn push_kind_keeps_runs_synced_empty() {
        let mut s = AppState::default();
        s.push_transcript_kind("x", TranscriptKind::Meta);
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }

    #[test]
    fn pack_roundtrip_and_run_carries_colour() {
        use zvm::screen::ZColour;
        // Round-trip all cases.
        for c in [
            ZColour::Default,
            ZColour::Standard(3),
            ZColour::Standard(12),
            ZColour::True(0x1234),
            ZColour::True24(0x00AB_CDEF),
        ] {
            assert_eq!(unpack_zcolour(pack_zcolour(c)), c,
                "pack/unpack round-trip failed for {c:?}");
        }
        // Thread colour through push_transcript_runs → StyleRun.
        let mut s = AppState::default();
        s.push_transcript_runs("ab", TranscriptKind::Story,
            &[(2, 0x02, ZColour::Standard(3), ZColour::Default, 0, ParaFmt::default(), 0, false)]);
        let run = s.transcript_runs.last().unwrap().first()
            .expect("coloured chunk must produce a StyleRun");
        assert_eq!(unpack_zcolour(run.fg), ZColour::Standard(3),
            "fg colour must survive the push → StyleRun round-trip");
        assert_eq!(unpack_zcolour(run.bg), ZColour::Default,
            "bg colour must survive the push → StyleRun round-trip");
    }

    #[test]
    fn play_turn_sounds_never_panics_without_device() {
        use zvm::cpu::exec::SoundEvent;
        let mut s = AppState::default();       // audio = None, sound_blorb = None
        s.config.enable_sound = true;
        // A #1 bleep event: play_turn_sounds must not panic with no backend.
        let ev = SoundEvent { number: 1, effect: 2, volume: 8, repeats: 0, routine: 0 };
        s.play_turn_sounds(&[ev]);             // no device -> silent, no panic
        // A #3 sampled start with no blorb loaded: no id remembered, no panic.
        let ev3 = SoundEvent { number: 3, effect: 2, volume: 8, repeats: 1, routine: 0 };
        s.play_turn_sounds(&[ev3]);
        assert!(s.sound_ids.is_empty(), "no sound id remembered without a blorb");
    }

    #[test]
    fn glk_volume_to_gain_is_linear_over_0x10000() {
        assert_eq!(glk_volume_to_gain(0), 0.0);
        assert_eq!(glk_volume_to_gain(0x10000), 1.0);   // Glk full
        assert_eq!(glk_volume_to_gain(0x8000), 0.5);    // half
        assert!(glk_volume_to_gain(0x20000) > 1.0, "amplification passes through");
    }

    #[test]
    fn glk_repeats_to_audio_maps_counts_and_forever() {
        assert_eq!(glk_repeats_to_audio(0), None);            // play zero times → skip
        assert_eq!(glk_repeats_to_audio(1), Some(1));         // once
        assert_eq!(glk_repeats_to_audio(5), Some(5));         // N times
        assert_eq!(glk_repeats_to_audio(0xFFFF_FFFF), Some(255)); // -1 → forever
        assert_eq!(glk_repeats_to_audio(300), Some(254));     // clamp below the forever sentinel
    }

    #[test]
    fn sound_pulse_defaults_none_and_holds_kind() {
        use crate::state::BeepKind;
        let mut s = AppState::default();
        assert!(s.sound_pulse.is_none(), "no pulse by default");
        s.sound_pulse = Some(SoundPulse { kind: BeepKind::High, started: std::time::Instant::now() });
        assert!(matches!(s.sound_pulse.as_ref().map(|p| p.kind), Some(BeepKind::High)));
    }

    #[test]
    fn has_active_animation_reflects_sources() {
        use crate::state::BeepKind;
        let mut s = AppState::default();
        assert!(!s.has_active_animation(), "idle state has no active animation");

        s.sound_pulse = Some(SoundPulse { kind: BeepKind::High, started: std::time::Instant::now() });
        assert!(s.has_active_animation(), "sound pulse counts as active");
        s.sound_pulse = None;

        s.scroll_anim = Some(ScrollAnim {
            from: 0,
            to: 5,
            tween: crate::anim::Tween::new(
                std::time::Duration::from_millis(100),
                crate::anim::Easing::EaseOut,
            ),
        });
        assert!(s.has_active_animation(), "scroll anim counts as active");
        s.scroll_anim = None;
        assert!(!s.has_active_animation());

        // An open selection-list modal's ListScroll animation also counts.
        let cfg = crate::config::AnimationConfig {
            enabled: true,
            easing: crate::anim::Easing::Linear,
            scroll_ms: 80,
        };
        let mut cs = ConfigScreenState {
            working: crate::config::Config::default(),
            scroll: Default::default(),
        };
        cs.scroll.len(100);
        cs.scroll.move_by(40, 5, &cfg); // arms a scroll animation
        s.overlays.config_screen = Some(cs);
        assert!(s.has_active_animation(), "an open modal's list scroll anim counts as active");
        s.overlays.config_screen = None;
        assert!(!s.has_active_animation());
    }

    #[test]
    fn has_active_animation_true_while_dock_slides() {
        let mut s = AppState::default();
        assert!(!s.has_active_animation(), "fresh default state has no active animation");

        let cfg = crate::config::AnimationConfig {
            enabled: true,
            easing: crate::anim::Easing::Linear,
            scroll_ms: 100,
        };
        s.inv_dock.toggle_to(true, false);
        s.inv_dock.arm(&cfg);
        assert!(s.has_active_animation(), "arming inv_dock open counts as active");
    }

    #[test]
    fn scroll_transcript_to_arms_when_enabled() {
        let mut s = AppState::default();
        s.transcript_scroll = 3;
        s.scroll_transcript_to(8);
        assert_eq!(s.transcript_scroll, 8, "logical target updated immediately");
        let a = s.scroll_anim.as_ref().expect("animation armed when enabled");
        assert_eq!(a.from, 3, "from = previous displayed offset");
        assert_eq!(a.target(), 8, "to = new target");
    }

    #[test]
    fn scroll_transcript_to_jumps_when_disabled() {
        let mut s = AppState::default();
        s.config.animation.enabled = false;
        s.transcript_scroll = 3;
        s.scroll_transcript_to(8);
        assert_eq!(s.transcript_scroll, 8);
        assert!(s.scroll_anim.is_none(), "disabled = instant, no animation");
    }

    #[test]
    fn scroll_transcript_to_zero_ms_jumps() {
        let mut s = AppState::default();
        s.config.animation.scroll_ms = 0;
        s.transcript_scroll = 2;
        s.scroll_transcript_to(6);
        assert_eq!(s.transcript_scroll, 6);
        assert!(s.scroll_anim.is_none(), "scroll_ms = 0 = instant, no animation");
    }

    #[test]
    fn scroll_transcript_to_retargets_from_current_displayed() {
        let mut s = AppState::default();
        s.transcript_scroll = 0;
        s.scroll_transcript_to(10); // arm 0 -> 10
        // Immediately retarget: progress is ~0, so the new `from` is ~current (~0).
        s.scroll_transcript_to(4);
        let a = s.scroll_anim.as_ref().unwrap();
        assert!(a.from < 1, "retarget starts from current displayed offset, got {}", a.from);
        assert_eq!(a.target(), 4);
    }

    #[test]
    fn effective_transcript_scroll_uses_target_or_rounded_anim() {
        let mut s = AppState::default();
        s.transcript_scroll = 9;
        assert_eq!(s.effective_transcript_scroll(), 9, "no anim = logical target");
        // A done tween reports current() == to; the offset is line-rounded.
        s.scroll_anim = Some(ScrollAnim {
            from: 0,
            to: 4,
            tween: crate::anim::Tween::new(std::time::Duration::ZERO, crate::anim::Easing::Linear),
        });
        assert_eq!(s.effective_transcript_scroll(), 4, "done tween shows rounded target");
    }

    #[test]
    fn scroll_anim_current_interpolates() {
        let a = ScrollAnim {
            from: 2,
            to: 10,
            tween: crate::anim::Tween::new(
                std::time::Duration::from_millis(100),
                crate::anim::Easing::Linear,
            ),
        };
        // Right after construction progress is ~0, so current() is near `from`.
        let c = a.current();
        assert!((2.0..3.0).contains(&c), "current near from at start, got {c}");
    }

    #[test]
    fn scroll_anim_instant_when_disabled() {
        use crate::anim::Easing;
        let cfg = crate::config::AnimationConfig { enabled: false, easing: Easing::EaseOut, scroll_ms: 120 };
        assert!(ScrollAnim::to(0, 10, &cfg).is_none(), "disabled animation arms nothing");
        let cfg0 = crate::config::AnimationConfig { enabled: true, easing: Easing::EaseOut, scroll_ms: 0 };
        assert!(ScrollAnim::to(0, 10, &cfg0).is_none(), "scroll_ms = 0 arms nothing");
    }

    #[test]
    fn scroll_anim_interpolates_then_settles() {
        use crate::anim::Easing;
        let cfg = crate::config::AnimationConfig { enabled: true, easing: Easing::Linear, scroll_ms: 40 };
        let a = ScrollAnim::to(0, 10, &cfg).expect("armed");
        assert_eq!(a.target(), 10);
        let c = a.current();
        assert!((0.0..=10.0).contains(&c), "current within range during ease: {c}");
    }

    #[test]
    fn any_overlay_open_reflects_state() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open(), "default AppState must have no overlay open");

        // saves
        s.overlays.saves = Some(SavesState { entries: vec![], scroll: Default::default() });
        assert!(s.any_overlay_open(), "saves open => any_overlay_open true");
        s.overlays.saves = None;

        // file_browser
        s.overlays.file_browser = Some(FileBrowserState::build(
            std::path::PathBuf::from("/tmp"),
            FbMode::PickFile));
        assert!(s.any_overlay_open(), "file_browser open => any_overlay_open true");
        s.overlays.file_browser = None;

        // config_screen
        s.overlays.config_screen = Some(ConfigScreenState {
            working: crate::config::Config::default(),
            scroll: Default::default(),
        });
        assert!(s.any_overlay_open(), "config_screen open => any_overlay_open true");
        s.overlays.config_screen = None;

        // The command band is deliberately absent from this list: it is a dock,
        // not a modal, and must NOT register as an overlay at all (SQ-0664).
        s.overlays.command_band = Some(CommandBandState::default());
        assert!(!s.any_overlay_open(), "the command band is not an overlay");
        assert!(!s.any_modal_overlay_open(), "…and certainly not a modal one");
        s.overlays.command_band = None;

        // hotkey_dialog
        s.overlays.hotkey_dialog = true;
        assert!(s.any_overlay_open(), "hotkey_dialog true => any_overlay_open true");
        s.overlays.hotkey_dialog = false;

        // room_panel
        s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        assert!(s.any_overlay_open(), "room_panel open => any_overlay_open true");
        s.room_panel = None;

        // tidy_anim
        s.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
            manifest: None,
        }], mapper::layer::MAIN_LAYER));
        assert!(s.any_overlay_open(), "tidy_anim active => any_overlay_open true");
        s.tidy_anim = None;

        // text_entry
        s.overlays.text_entry = Some(TextEntryDialog::new(TextEntryKind::CreateFile, ""));
        assert!(s.any_overlay_open(), "text_entry active => any_overlay_open true");
        s.overlays.text_entry = None;

        // confirm_delete_save
        s.overlays.confirm_delete_save = Some(std::path::PathBuf::from("/x.babelmap"));
        assert!(s.any_overlay_open(), "confirm_delete_save active => any_overlay_open true");
        s.overlays.confirm_delete_save = None;

        // confirm_overwrite_save
        s.overlays.confirm_overwrite_save = Some(ConfirmOverwriteSave {
            path: std::path::PathBuf::from("/x.babelmap"),
            existing_name: "x".to_string(),
            pending: PendingOverwrite::SaveAs,
        });
        assert!(s.any_overlay_open(), "confirm_overwrite_save active => any_overlay_open true");
        s.overlays.confirm_overwrite_save = None;

        // launch_dialog
        s.overlays.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.overlays.launch_dialog = false;

        // debug: a tiled pane, not a modal overlay
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        assert!(!s.any_overlay_open(), "debug pane open => any_overlay_open still false");
        s.debug = None;

        assert!(!s.any_overlay_open(), "all cleared => any_overlay_open false again");
    }

    #[test]
    fn toggle_map_flips_the_panel_and_never_moves_focus() {
        let mut s = AppState::default();
        assert!(matches!(s.layout, Layout::Split));
        // SQ-0599: Tab does not reach the map, so focus is on the game before
        // and after — there is no hidden-pane hazard left to guard against.
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "Tab has nowhere to go without the inspector");
        s.toggle_map();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        assert!(matches!(s.focus, Focus::Game));
        s.toggle_map();
        assert!(matches!(s.layout, Layout::Split));
        assert!(matches!(s.focus, Focus::Game));
    }

    /// The whole point of SQ-0599: with the inspector closed, Tab is inert and
    /// every keystroke keeps going to the story. It used to hand the keyboard
    /// to the map, silently changing what the next arrow key did.
    #[test]
    fn tab_does_not_hand_the_keyboard_to_the_map() {
        let mut s = AppState::default();
        assert!(matches!(s.layout, Layout::Split), "map is visible");
        for _ in 0..4 {
            s.toggle_focus();
            assert!(matches!(s.focus, Focus::Game), "Tab must never land on the map pane");
        }
        // Shift-Tab likewise.
        for _ in 0..4 {
            s.cycle_focus(false);
            assert!(matches!(s.focus, Focus::Game));
        }
    }

    #[test]
    fn toggle_focus_stays_on_game_when_map_hidden() {
        let mut s = AppState::default();
        s.toggle_map(); // → TranscriptFull, map hidden
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "Tab can't focus a hidden map");
    }

    #[test]
    fn toggle_focus_reaches_debug_region_when_map_hidden() {
        let mut s = AppState::default();
        s.toggle_map(); // → TranscriptFull, map hidden
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        s.toggle_focus();
        assert!(
            matches!(s.focus, Focus::Map),
            "Tab reaches the debug region even though the map itself is hidden"
        );
        // 4 focus stops (story + 3 debug windows): three more Tabs return to story.
        s.toggle_focus();
        s.toggle_focus();
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game));
    }

    #[test]
    fn cycle_focus_steps_per_window_including_the_story() {
        let mut s = AppState::default();
        s.layout = Layout::Split;
        s.debug = Some(crate::debug_panel::DebugPanelState::new(0));
        // Forward: story → debug win 0 → 1 → 2 → story.
        assert!(matches!(s.focus, Focus::Game));
        s.cycle_focus(true);
        assert!(matches!(s.focus, Focus::Map));
        assert_eq!(s.debug.as_ref().unwrap().focus, 0);
        s.cycle_focus(true);
        assert_eq!(s.debug.as_ref().unwrap().focus, 1);
        s.cycle_focus(true);
        assert_eq!(s.debug.as_ref().unwrap().focus, 2);
        s.cycle_focus(true);
        assert!(matches!(s.focus, Focus::Game), "wraps from the last window back to the story");
        // Backward from the story lands on the last debug window.
        s.cycle_focus(false);
        assert!(matches!(s.focus, Focus::Map));
        assert_eq!(s.debug.as_ref().unwrap().focus, 2);
    }

    #[test]
    fn focus_layout_zoom_transitions() {
        let mut s = AppState::default();
        assert!(matches!(s.focus, Focus::Game));
        s.toggle_focus();
        assert!(matches!(s.focus, Focus::Game), "no map focus stop (SQ-0599)");
        s.toggle_map();
        assert!(matches!(s.layout, Layout::TranscriptFull));
        s.toggle_map();
        assert!(matches!(s.layout, Layout::Split));
        // Zoom clamps. A keypress moves one VISIBLE step (SQ-0350); the fine 0-8 counter is
        // the wheel's, not the keyboard's.
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Compact));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview)); // clamped
        s.zoom_in();
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes)); // clamped
    }

    #[test]
    fn input_line_and_transcript() {
        let mut s = AppState::default();
        s.push_input_char('g');
        s.push_input_char('o');
        s.backspace();
        assert_eq!(s.input.value, "g");
        let cmd = s.take_input();
        assert_eq!(cmd, "g");
        assert_eq!(s.input.value, "");
        s.push_transcript("line1\nline2");
        assert_eq!(s.transcript.len(), 2);
    }

    #[test]
    fn recenter_on_centers_cell() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (5, 5) in a 20×10 character pane:
        // cells_w = 20 / 13 = 1, cells_h = 10 / 7 = 1
        // scroll = (5 - 1/2, 5 - 1/2) = (5 - 0, 5 - 0) = (5, 5)
        s.recenter_on((5, 5), 20, 10);
        assert_eq!(s.scroll, (5, 5));
    }

    #[test]
    fn recenter_on_boxes_larger_pane() {
        let mut s = AppState::default(); // Boxes zoom: effective step 13×7
        // Centering cell (0, 0) in a 80×24 character pane:
        // cells_w = 80 / 13 = 6, cells_h = 24 / 7 = 3
        // scroll = (0 - 6/2, 0 - 3/2) = (0 - 3, 0 - 1) = (-3, -1)
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.scroll, (-3, -1));
    }

    #[test]
    fn recenter_on_compact_zoom() {
        use crate::state::Zoom;
        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // steps = (12, 5)
        // Centering cell (4, 4) in a 48×20 pane:
        // cells_w = 48 / 12 = 4, cells_h = 20 / 5 = 4
        // scroll = (4 - 4/2, 4 - 4/2) = (4 - 2, 4 - 2) = (2, 2)
        s.recenter_on((4, 4), 48, 20);
        assert_eq!(s.scroll, (2, 2));
    }

    #[test]
    fn pan_accumulates() {
        let mut s = AppState::default();
        s.pan(3, -2);
        s.pan(1, 4);
        assert_eq!(s.scroll, (4, 2));
    }

    #[test]
    fn select_room_roundtrip() {
        let mut s = AppState::default();
        assert_eq!(s.selected_room, None);
        s.select_room(Some(42));
        assert_eq!(s.selected_room, Some(42));
        s.select_room(None);
        assert_eq!(s.selected_room, None);
    }

    #[test]
    fn active_layer_follows_current_then_view_override() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_current(1);
        let l = g.new_layer(Some(0), "B".into());
        let mut s = AppState::default();
        assert_eq!(s.active_layer(&g), 0, "defaults to current room's layer");
        s.set_viewed_layer(Some(l));
        assert_eq!(s.active_layer(&g), l, "explicit view wins");
        s.set_viewed_layer(Some(999)); // stale id (no such layer)
        assert_eq!(s.active_layer(&g), 0, "stale view falls back to current room's layer");
    }

    /// SQ-0359: the bug in one assertion. A tidy animation's frames are `layer_subgraph`s, and a
    /// subgraph reports `layers()` as main-only however it was built — so asking the FRAME which
    /// layer to draw answers `MAIN_LAYER`, which it holds no rooms for, and the map goes blank.
    #[test]
    fn frame_layer_takes_an_animations_layer_from_the_animation_not_its_subgraph() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut live = MapGraph::new();
        live.upsert_room(1, "Hall".into());
        live.set_current(1);
        live.set_pos(1, (0, 0));
        let cellar = live.new_layer(Some(0), "Cellar".into());
        live.upsert_room(2, "Cellar".into());
        live.set_room_layer(2, cellar);
        live.set_pos(2, (0, 0)); // placed: `render` only emits rooms that have a position

        let mut s = AppState::default();
        s.set_viewed_layer(Some(cellar));

        // The frame the animation would carry: the Cellar layer, extracted.
        let sub = live.layer_subgraph(cellar);
        assert!(
            !sub.layers().contains_key(&cellar),
            "a subgraph does not admit which layer it is — the whole reason for this field"
        );
        assert_eq!(
            s.active_layer(&sub),
            MAIN_LAYER,
            "so asking the frame yields main: the blank-map bug"
        );
        // And that is not a cosmetic mislabel — it is the blank map the user reported.
        assert!(
            mapper::render::render_layer(&sub, MAIN_LAYER).rooms.is_empty(),
            "the frame holds no main-layer rooms, so drawing it as main draws nothing"
        );
        assert!(
            !mapper::render::render_layer(&sub, cellar).rooms.is_empty(),
            "drawn as the layer it actually is, the frame has rooms"
        );

        s.tidy_anim = Some(TidyAnim::new(
            vec![TidyFrame {
                label: "Build".into(),
                graph: sub,
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }],
            cellar,
        ));
        assert_eq!(
            s.frame_layer(&live, None),
            cellar,
            "the animation states its own layer, so the map draws the rooms being tidied"
        );

        // With no animation, nothing changes: the live graph still answers for itself.
        s.tidy_anim = None;
        assert_eq!(s.frame_layer(&live, None), cellar, "viewed layer still wins on the live graph");
        s.set_viewed_layer(None);
        assert_eq!(s.frame_layer(&live, None), MAIN_LAYER, "falls back to the current room's layer");
    }

    /// Replay outranks an animation, matching the map pane's own order.
    #[test]
    fn frame_layer_prefers_a_replay_graph_over_an_animation() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut live = MapGraph::new();
        live.upsert_room(1, "Hall".into());
        live.set_current(1);
        let cellar = live.new_layer(Some(0), "Cellar".into());

        let mut replay = MapGraph::new();
        replay.upsert_room(1, "Hall".into());
        replay.set_current(1);

        let mut s = AppState::default();
        s.tidy_anim = Some(TidyAnim::new(
            vec![TidyFrame {
                label: "Build".into(),
                graph: MapGraph::new(),
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }],
            cellar,
        ));
        assert_eq!(
            s.frame_layer(&live, Some(&replay)),
            MAIN_LAYER,
            "replay is what's on screen, so it picks the layer — not the stale animation"
        );
    }

    #[test]
    fn appstate_default_symbols_are_default_set() {
        let st = AppState::default();
        assert_eq!(st.symbols, crate::symbols::SymbolSet::default());
    }

    /// Wait for the in-flight render worker to finish, then install it.
    fn drain_render_job(s: &mut AppState) {
        while s
            .render_job
            .borrow()
            .as_ref()
            .is_some_and(|j| !j.handle.is_finished())
        {
            std::thread::yield_now();
        }
        s.poll_render_job();
    }

    /// SQ-0391, flipped by SQ-0666: a direction that goes NOWHERE adds no room and no
    /// connection, so `graph_gen` deliberately does not bump for it (SQ-0378 keeps a plain step
    /// from re-routing the whole map). The untried-exits OVERLAY was memoised on that generation
    /// and needed a hand-written refresh to notice a foiled move; the matrix view that replaced
    /// it reads the graph directly on every frame and so cannot go stale at all. Same fact, same
    /// awkward turn, one fewer cache to keep honest.
    #[test]
    fn a_foiled_move_shows_up_immediately_without_a_geometry_change() {
        use mapper::direction::Direction;
        use mapper::mapper::Mapper;
        use mapper::matrix::{classify, MatrixCell};

        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        let mut s = AppState::default();
        let _ = s.cached_map_render(0, &m.graph);
        drain_render_job(&mut s);

        assert_eq!(classify(&m.graph, 1, Direction::N), MatrixCell::Untried, "north starts `·`");

        let (gen, rooms, conns) =
            (s.graph_gen, m.graph.rooms().count(), m.graph.connections().len());
        m.observe(1, "Hall", Some(Direction::N)); // typed, went nowhere
        assert_eq!(m.graph.rooms().count(), rooms, "a foiled move adds no room");
        assert_eq!(m.graph.connections().len(), conns, "and no connection");
        assert_eq!(s.graph_gen, gen, "so the map memo is NOT invalidated");

        assert_eq!(
            classify(&m.graph, 1, Direction::N),
            MatrixCell::Probed,
            "the cell becomes `_` on the same turn — nothing waits for the geometry to change"
        );
    }

    /// SQ-0379: no routing runs on the main thread — even the first build. The
    /// first draw serves an empty placeholder while the real model routes on a
    /// worker; a later geometry change re-routes off-thread while the last-ready
    /// model keeps being served, so the interpreter never blocks.
    #[test]
    fn cached_map_render_routes_off_thread_including_first_build() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let mut s = AppState::default();

        // First call: empty placeholder served, real model routes off-thread.
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 0, "empty placeholder while the first route runs");
        }
        assert!(s.render_job.borrow().is_some(), "even the first build is off-thread");
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 1, "routed model served once it lands");
        }
        // Same (gen, layer): reused, no new worker.
        let _ = s.cached_map_render(0, &g);
        assert!(s.render_job.borrow().is_none());

        // A geometry change (new room + gen bump): re-route OFF-thread, the STALE
        // model (still 1 room) served meanwhile.
        g.upsert_room(2, "B".into());
        g.set_pos(2, (1, 0));
        s.graph_gen = s.graph_gen.wrapping_add(1);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 1, "last-ready model served while routing");
        }
        assert!(s.render_job.borrow().is_some(), "a stale model re-routes off-thread");
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert_eq!(rm.rooms.len(), 2, "the freshly routed model is now served");
        }
    }

    /// SQ-0378: a step between already-placed rooms changes the current-room
    /// highlight but not the routed geometry, so `graph_gen` does not bump. The
    /// cache must follow the player WITHOUT re-routing (no worker spawns).
    #[test]
    fn cached_map_render_refreshes_current_without_rerouting() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into());
        g.set_pos(2, (1, 0));
        g.set_current(1);
        let mut s = AppState::default();

        // Build + install the initial model.
        let _ = s.cached_map_render(0, &g);
        drain_render_job(&mut s);
        {
            let rm = s.cached_map_render(0, &g);
            assert!(rm.rooms.iter().find(|r| r.id == 1).unwrap().is_current);
            assert!(!rm.rooms.iter().find(|r| r.id == 2).unwrap().is_current);
        }

        // Move the player to room 2 WITHOUT bumping graph_gen.
        g.set_current(2);
        {
            let rm = s.cached_map_render(0, &g);
            assert!(!rm.rooms.iter().find(|r| r.id == 1).unwrap().is_current);
            assert!(
                rm.rooms.iter().find(|r| r.id == 2).unwrap().is_current,
                "the highlight follows the player"
            );
        }
        assert!(
            s.render_job.borrow().is_none(),
            "a current-room change must NOT spawn a re-route worker"
        );
    }

    // ── FileBrowserState tests ────────────────────────────────────────────────

    /// Create a temporary directory with a unique tag.
    /// Contents: subdir/, save.qzl, notes.txt.
    fn make_test_fb_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("babelmap-fb-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        std::fs::write(dir.join("save.qzl"), b"fake quetzal").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a save").unwrap();
        dir
    }

    #[test]
    fn filebrowser_pickfile_shows_dirs_and_qzl_not_txt() {
        let dir = make_test_fb_dir("pickfile");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "should contain parent link");
        assert!(names.contains(&"subdir"), "should contain subdir");
        assert!(names.contains(&"save.qzl"), "should contain .qzl file");
        assert!(!names.contains(&"notes.txt"), ".txt file must not appear in PickFile mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_dotdot_absent_at_root() {
        // Synthesize a state rooted at "/" (or the filesystem root on this OS).
        let root = std::path::Path::new("/");
        let fb = FileBrowserState::build(root.to_path_buf(), FbMode::PickFile);
        let has_dotdot = fb.entries.iter().any(|e| e.name == "..");
        assert!(!has_dotdot, "'..' must not appear when at filesystem root");
    }

    #[test]
    fn filebrowser_cd_into_subdir_and_refresh() {
        let dir = make_test_fb_dir("cd");
        let mut fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        let subdir = dir.join("subdir");
        fb.cd(subdir.clone());
        assert_eq!(fb.cwd, subdir, "cwd should update after cd");
        assert_eq!(fb.scroll.selected, 0, "selection should reset to 0 after cd");
        // subdir is empty (no qzl files), but ".." should be present.
        let names: Vec<&str> = fb.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."), "subdir should show '..'");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filebrowser_entries_sorted_dirs_before_files() {
        let dir = make_test_fb_dir("sorted");
        let fb = FileBrowserState::build(dir.clone(), FbMode::PickFile);
        // Verify: ".." first, then dirs, then files.
        let mut saw_dir = false;
        let mut saw_file = false;
        for e in &fb.entries {
            if e.is_dir {
                assert!(!saw_file, "dirs should appear before files, but saw a file first");
                saw_dir = true;
            } else {
                saw_file = true;
            }
        }
        assert!(saw_dir, "should have at least one dir");
        assert!(saw_file, "should have at least one file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── zoom_level / fine zoom tests (item 2) ────────────────────────────────

    #[test]
    fn zoom_level_default_is_boxes() {
        let s = AppState::default();
        assert_eq!(s.zoom_level, 7);
        assert!(matches!(s.zoom, Zoom::Boxes));
    }

    #[test]
    fn a_zoom_keypress_always_moves_the_map() {
        // SQ-0350: the zoom keys "did not respond". They always fired — they just moved a fine
        // counter nobody could see. Nine fine levels collapse to three views (0-2 Overview,
        // 3-5 Compact, 6-8 Boxes) and the default sits at 7, mid-Boxes: so `-` needed TWO presses
        // for the first visible change, and `+` never did anything at all (7 -> 8 is still Boxes).
        // One press must now equal one visible step.
        let mut s = AppState::default();
        assert!(matches!(s.zoom, Zoom::Boxes), "default is the most detailed view");

        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Compact), "one press of `-` moves the map");
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview), "and again");
        s.zoom_out();
        assert!(matches!(s.zoom, Zoom::Overview), "clamped at the least detailed view");

        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Compact), "one press of `+` moves it back");
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in();
        assert!(matches!(s.zoom, Zoom::Boxes), "clamped at the most detailed view");
    }

    #[test]
    fn zoom_keypresses_are_exact_inverses() {
        // Landing mid-band is what makes this hold: an edge landing would let `+` then `-` drift
        // into a different view than it started in, and the wheel would tip over a boundary on its
        // very first notch.
        let mut s = AppState::default();
        for _ in 0..2 {
            s.zoom_out();
        }
        let (lvl, z) = (s.zoom_level, s.zoom);
        s.zoom_in();
        s.zoom_out();
        assert_eq!(s.zoom_level, lvl, "+ then - returns to the same fine level");
        assert_eq!(s.zoom, z);
    }

    #[test]
    fn zoom_by_honours_its_magnitude_and_clamps() {
        // SQ-0355: `zoom-map <n>` promises "step by signed n". Every n used to collapse to one
        // step, so `zoom-map 5` moved exactly as far as `zoom-map in`.
        let mut s = AppState::default(); // Boxes
        s.zoom_by(-1);
        assert!(matches!(s.zoom, Zoom::Compact), "-1 is one step out");

        let mut s = AppState::default();
        s.zoom_by(-2);
        assert!(matches!(s.zoom, Zoom::Overview), "-2 goes two steps, not one");

        // Clamps exactly as the equivalent run of keypresses would.
        let mut s = AppState::default();
        s.zoom_by(-99);
        assert!(matches!(s.zoom, Zoom::Overview), "over-stepping clamps, it does not wrap");
        s.zoom_by(99);
        assert!(matches!(s.zoom, Zoom::Boxes));

        // n and a run of single steps must agree — the command and the key cannot drift apart.
        let (mut by, mut steps) = (AppState::default(), AppState::default());
        by.zoom_by(-2);
        steps.zoom_out();
        steps.zoom_out();
        assert_eq!(by.zoom_level, steps.zoom_level);
        assert_eq!(by.zoom, steps.zoom);

        // 0 is a no-op here; the parser maps `zoom-map 0` to ZoomReset before it reaches this.
        let mut s = AppState::default();
        let before = s.zoom_level;
        s.zoom_by(0);
        assert_eq!(s.zoom_level, before);
    }

    #[test]
    fn the_wheel_still_steps_finely() {
        // The fine levels exist so a fast ctrl+scroll cannot skip straight past Compact. Fixing the
        // KEYBOARD must not flatten the wheel into the same coarse step.
        let mut s = AppState::default(); // level 7, Boxes
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 6);
        assert!(matches!(s.zoom, Zoom::Boxes), "one notch stays inside the band");
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 5);
        assert!(matches!(s.zoom, Zoom::Compact), "the third notch tips into the next view");
        for _ in 0..5 {
            s.zoom_out_fine();
        }
        assert_eq!(s.zoom_level, 0);
        assert!(matches!(s.zoom, Zoom::Overview));
        s.zoom_out_fine();
        assert_eq!(s.zoom_level, 0, "clamped at 0");

        for _ in 0..8 {
            s.zoom_in_fine();
        }
        assert_eq!(s.zoom_level, 8);
        assert!(matches!(s.zoom, Zoom::Boxes));
        s.zoom_in_fine();
        assert_eq!(s.zoom_level, 8, "clamped at 8");
    }

    #[test]
    fn zoom_reset_returns_to_default_level() {
        let mut s = AppState::default();
        // Go to Overview (two visible steps from Boxes).
        for _ in 0..2 {
            s.zoom_out();
        }
        assert!(matches!(s.zoom, Zoom::Overview));
        // Also set char_pan to something non-zero
        s.char_pan = (4, -2);
        // Reset
        s.zoom_reset();
        assert_eq!(s.zoom_level, 7, "zoom_reset must restore level to 7");
        assert!(matches!(s.zoom, Zoom::Boxes), "zoom_reset must restore Zoom::Boxes");
        assert_eq!(s.char_pan, (0, 0), "zoom_reset must clear char_pan");
    }

    #[test]
    fn zoom_from_level_maps_correctly() {
        use super::zoom_from_level;
        assert!(matches!(zoom_from_level(0), Zoom::Overview));
        assert!(matches!(zoom_from_level(1), Zoom::Overview));
        assert!(matches!(zoom_from_level(2), Zoom::Overview));
        assert!(matches!(zoom_from_level(3), Zoom::Compact));
        assert!(matches!(zoom_from_level(4), Zoom::Compact));
        assert!(matches!(zoom_from_level(5), Zoom::Compact));
        assert!(matches!(zoom_from_level(6), Zoom::Boxes));
        assert!(matches!(zoom_from_level(7), Zoom::Boxes));
        assert!(matches!(zoom_from_level(8), Zoom::Boxes));
    }

    // ── char_pan / drag-pan tests (item 1) ───────────────────────────────────

    #[test]
    fn char_pan_default_is_zero() {
        let s = AppState::default();
        assert_eq!(s.char_pan, (0, 0));
    }

    #[test]
    fn recenter_on_clears_char_pan() {
        let mut s = AppState::default();
        s.char_pan = (5, -3);
        s.recenter_on((0, 0), 80, 24);
        assert_eq!(s.char_pan, (0, 0), "recenter_on must reset char_pan to (0,0)");
    }

    #[test]
    fn reset_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.reset_dialog = true;
        assert!(s.any_overlay_open(), "reset_dialog open => any_overlay_open true");
    }

    #[test]
    fn game_over_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.game_over = true;
        assert!(s.any_overlay_open(), "game_over open => any_overlay_open true");
        s.overlays.game_over = false;
        assert!(!s.any_overlay_open(), "game_over false => any_overlay_open false");
    }

    #[test]
    fn quit_dialog_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());
        s.overlays.quit_dialog = true;
        assert!(s.any_overlay_open(), "quit_dialog open => any_overlay_open true");
        s.overlays.quit_dialog = false;
        assert!(!s.any_overlay_open(), "quit_dialog false => any_overlay_open false");
    }

    #[test]
    fn hints_panel_counts_as_overlay() {
        let mut s = AppState::default();
        assert!(!s.any_overlay_open());

        // Build a minimal HintSession using the minizork fixture (same approach as
        // the reset test in input.rs). If the fixture is absent we skip.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        s.overlays.hints = Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        });
        assert!(s.any_overlay_open(), "hints open => any_overlay_open true");
        s.overlays.hints = None;
        assert!(!s.any_overlay_open(), "hints closed => any_overlay_open false");
    }

    #[test]
    fn hint_session_scroll_by_clamps_to_range() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None).expect("GameSession::new");
        let mut hs = HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        };
        // Instant (animation disabled) so the logical offset settles immediately.
        let anim = crate::config::AnimationConfig { enabled: false, easing: crate::anim::Easing::EaseOut, scroll_ms: 0 };
        // Scrolling down (negative) at the top is clamped to 0.
        hs.scroll_by(-1, 5, &anim);
        assert_eq!(hs.scroll, 0, "scroll cannot go below 0");
        // Scrolling up (positive) advances within range.
        hs.scroll_by(3, 5, &anim);
        assert_eq!(hs.scroll, 3);
        // Scrolling past max is clamped to max.
        hs.scroll_by(10, 5, &anim);
        assert_eq!(hs.scroll, 5, "scroll clamps to max");
        // A max of 0 (nothing to scroll) pins scroll at 0.
        hs.scroll_by(4, 0, &anim);
        assert_eq!(hs.scroll, 0);
    }

    /// Build a minimal `HintSession` off the minizork fixture (its transcript
    /// starts empty so `apply_turn` math is easy to assert), or `None` if the
    /// fixture is absent (caller skips).
    fn make_hint_session() -> Option<HintSession> {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return None;
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");
        let session = crate::session::GameSession::new(story_bytes, true, false, None)
            .expect("GameSession::new");
        Some(HintSession {
            source: HintSource::Zcode(session),
            transcript: vec![],
            scroll: 0,
            clear_anchor: None,
            scroll_anim: None,
            input: String::new(),
            label: "Hints: Test".to_string(),
            builtin_hint: false,
        })
    }

    /// A `TurnResult` carrying just the fields `apply_turn` reads; everything
    /// else is empty/default.
    fn turn_result(transcript: &str, erase_lower: bool) -> crate::session::TurnResult {
        crate::session::TurnResult {
            transcript: transcript.to_string(),
            transcript_runs: vec![],
            location: None,
            quit: false,
            erase_lower,
            info: None,
            sounds: vec![],
            glulx_sound_ops: vec![],
            diagnostics: vec![],
            location_method: None,
            pending_io: None,
            timed_out: false,
            fault: None,
            pictures: Vec::new(),
            transcript_elems: vec![],
        }
    }

    #[test]
    fn apply_turn_collapses_successive_menu_reprints() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear(); // fixture opening irrelevant; start clean

        // First menu redraw (screen clear): anchors at 0, appends its 3 lines.
        hs.apply_turn(&turn_result("m1a\nm1b\nm1c", true));
        assert_eq!(hs.transcript.len(), 3);
        assert_eq!(hs.clear_anchor, Some(0));

        // Second menu redraw: must collapse the first reprint, not stack on it.
        hs.apply_turn(&turn_result("m2a\nm2b", true));
        assert_eq!(
            hs.transcript.len(),
            2,
            "second erase_lower collapses the prior reprint back to the anchor"
        );
        assert_eq!(hs.transcript, vec!["m2a".to_string(), "m2b".to_string()]);
        assert_eq!(hs.scroll, 0);
    }

    #[test]
    fn apply_turn_normal_turn_appends() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear();
        hs.transcript.push("existing".to_string());

        // A normal (non-clearing) turn appends its lines and leaves the anchor unset.
        hs.apply_turn(&turn_result("clue line 1\nclue line 2", false));
        assert_eq!(
            hs.transcript,
            vec![
                "existing".to_string(),
                "clue line 1".to_string(),
                "clue line 2".to_string()
            ]
        );
        assert_eq!(hs.clear_anchor, None, "no screen clear => no anchor");
        assert_eq!(hs.scroll, 0);
    }

    #[test]
    fn apply_turn_empty_turn_adds_no_blank_line_and_keeps_scroll() {
        let Some(mut hs) = make_hint_session() else { return };
        hs.transcript.clear();
        hs.transcript.push("clue A".to_string());
        hs.transcript.push("clue B".to_string());
        hs.scroll = 1; // user paged up to reread

        // A menu keystroke that only moves the upper-window highlight produces no
        // lower-window text: it must NOT append a blank line, and must NOT yank the
        // scroll back to the bottom.
        hs.apply_turn(&turn_result("", false));
        assert_eq!(
            hs.transcript,
            vec!["clue A".to_string(), "clue B".to_string()],
            "an empty turn must not append a blank line to the clue window"
        );
        assert_eq!(hs.scroll, 1, "an empty turn must not reset the scroll position");
    }

    #[test]
    fn run_search_direction_and_next_wrap() {
        let mut s = AppState::default();
        for t in ["alpha", "beta", "alpha again", "gamma", "ALPHA"] { s.push_transcript(t); }
        // matches for "alpha" at visible positions 0, 2, 4 (case-insensitive)
        let n = s.run_search("alpha", true); // start backward → last match
        assert_eq!(n, 3);
        assert_eq!(s.search_matches, vec![0, 2, 4]);
        assert_eq!(s.search_idx, 2); // index into search_matches → position 4
        // n = back
        assert_eq!(s.search_next(false), Some(2)); // now at match position 2
        // forward wraps from 2 → 4 → back to 0
        let _ = s.search_next(true); // → 4
        assert_eq!(s.search_next(true), Some(0)); // wrap to first
        let f = s.run_search("alpha", false); // start forward → first match
        assert_eq!(f, 3);
        assert_eq!(s.search_idx, 0);
        s.clear_search();
        assert!(s.search_query.is_none() && s.search_matches.is_empty());
    }

    #[test]
    fn map_trace_routes_render_steps_to_log_only_when_on() {
        let dir = std::env::temp_dir().join(format!("bm-maptrace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let steps = vec!["detect chains".to_string(), "route lanes".to_string()];
        // helper under test:
        write_map_trace(&dir, &steps, /* on = */ true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.contains("[map]    detect chains"), "{body:?}");

        let dir2 = std::env::temp_dir().join(format!("bm-maptrace-off-{}", std::process::id()));
        std::fs::create_dir_all(&dir2).unwrap();
        write_map_trace(&dir2, &steps, false);
        assert!(!dir2.join("trace.log").exists() || std::fs::read_to_string(dir2.join("trace.log")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok(); std::fs::remove_dir_all(&dir2).ok();
    }
}

#[cfg(test)]
mod play_sound_tests {
    use super::*;

    // Build an IFF chunk: type + BE len + data + pad-to-even. Mirrors
    // blorb::tests::chunk (not exported, so duplicated here).
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    // Build a Blorb with the given resources. Mirrors blorb::tests::build_blorb
    // (not exported, so duplicated here).
    type BlorbRes<'a> = (&'a [u8; 4], u32, &'a [u8; 4], &'a [u8]);
    fn build_blorb(res: &[BlorbRes]) -> Vec<u8> {
        let count = res.len() as u32;
        let ridx_data_len = 4 + 12 * res.len();
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_u, _n, ty, data) in res {
            offsets.push(cursor as u32);
            let c = chunk(ty, data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&count.to_be_bytes());
        for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&number.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let ridx_chunk = chunk(b"RIdx", &ridx);
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    // ── sound_kind_label ────────────────────────────────────────────────────

    #[test]
    fn sound_kind_label_maps_all_variants() {
        assert_eq!(sound_kind_label(blorb::SoundKind::Aiff), "AIFF");
        assert_eq!(sound_kind_label(blorb::SoundKind::Ogg), "OGG");
        assert_eq!(sound_kind_label(blorb::SoundKind::Mod), "MOD");
        assert_eq!(sound_kind_label(blorb::SoundKind::Other), "other");
    }

    // ── format_sound_resource_list ──────────────────────────────────────────

    #[test]
    fn resource_list_no_blorb_reports_none_resolved() {
        let lines = format_sound_resource_list(None);
        assert_eq!(lines, vec!["no sound blorb resolved".to_string()]);
    }

    #[test]
    fn resource_list_no_snd_resources() {
        let bytes = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let lines = format_sound_resource_list(Some(&blorb));
        assert_eq!(lines, vec!["no Snd resources".to_string()]);
    }

    #[test]
    fn resource_list_enumerates_and_marks_playability() {
        let bytes = build_blorb(&[
            (b"Exec", 0, b"ZCOD", b"abcd"),
            (b"Snd ", 3, b"FORM", b"aiffbytes"),
            (b"Snd ", 5, b"OGGV", b"oggdata!"),
            (b"Snd ", 9, b"WEIR", b"whatever"),
        ]);
        let blorb = blorb::Blorb::parse(bytes).unwrap();
        let lines = format_sound_resource_list(Some(&blorb));
        assert_eq!(lines[0], "3 sound resource(s):");
        let three = lines.iter().find(|l| l.contains("#3")).unwrap();
        assert!(three.contains("AIFF") && three.contains("playable") && !three.contains("not decodable"));
        let five = lines.iter().find(|l| l.contains("#5")).unwrap();
        assert!(five.contains("OGG") && five.contains("playable") && !five.contains("not decodable"));
        let nine = lines.iter().find(|l| l.contains("#9")).unwrap();
        assert!(nine.contains("other") && nine.contains("not decodable"));
    }

    // ── format_play_sound_report ────────────────────────────────────────────

    #[test]
    fn report_resource_not_found() {
        let r = PlaySoundReport {
            number: 42,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: None,
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("NOT FOUND")));
    }

    #[test]
    fn report_undecodable_kind_stops_before_playback() {
        let r = PlaySoundReport {
            number: 9,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Other, 128)),
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("not decodable")));
        assert!(!lines.iter().any(|l| l.contains("playback")));
    }

    #[test]
    fn report_success_shows_sound_id() {
        let r = PlaySoundReport {
            number: 3,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Aiff, 256)),
            format: Some(audio::SoundFormat::Aiff),
            sound_id: Some(7),
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("sound id 7")));
    }

    #[test]
    fn report_backend_none_when_playback_fails() {
        let r = PlaySoundReport {
            number: 3,
            enable_sound: true,
            backend_present: true,
            blorb_present: true,
            resource: Some((blorb::SoundKind::Aiff, 256)),
            format: Some(audio::SoundFormat::Aiff),
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("playback: backend returned None")));
    }

    #[test]
    fn report_notes_disabled_gate() {
        let r = PlaySoundReport {
            number: 3,
            enable_sound: false,
            backend_present: true,
            blorb_present: true,
            resource: None,
            format: None,
            sound_id: None,
        };
        let lines = format_play_sound_report(&r);
        assert!(lines.iter().any(|l| l.contains("off (attempting playback anyway")));
    }
}
