// What a story's parser accepts, as a value — the ANSWER a grammar reader
// returns, with none of the reading in it.
//
// ── Why this crate exists, and why it holds no reader ─────────────────────────
//
// `zvm::grammar` (SQ-1040) and `gvm::grammar` (SQ-1102) read Inform's and
// Infocom's syntax tables out of a Z-machine and a Glulx image respectively.
// The two READERS were deliberately kept apart, on evidence: the Z-machine's
// table address is named by header word $0E and a Glulx image records its own
// nowhere; verb numbers count down from $FF against $FFFF; a line header is 2
// bytes with the reverse flag packed into the action against 3 with a flags
// byte; a token is 1 + 2 bytes against 1 + 4; the dictionary is Z-encoded with
// a game-chosen record length against `$60`-tagged plain bytes; and `zvm`
// carries five table shapes to Glulx's one. A trait over "read a byte at an
// address" would abstract a handful of lines out of several hundred.
//
// What the two genuinely share is the shape of the RESULT — and until SQ-1103
// they shared it as two near-identical copies, with names kept identical so
// that this crate could be lifted out mechanically. A consumer asking "what
// sentences does this story accept?" should get one vocabulary whichever
// engine answered.
//
// ── Which facts stayed with the engines ──────────────────────────────────────
//
// Not everything a reader returns is shared, and the split is on whether the
// fact is about the ANSWER or about the FORMAT it was read from:
//
//   * `zvm::grammar::GrammarFormat` (and `Grammar::format`) — five table shapes
//     exist only on the Z-machine; Glulx has exactly one, so there is nothing
//     to report.
//   * `gvm::grammar::Tables` (and `locate`) — the Glulx addresses are DERIVED,
//     so where they were found is part of that engine's answer and is what
//     `glulxdump -g` must be handed. The Z-machine reads its address out of the
//     header and has nothing to report either.
//   * `GrammarError` — both engines name one, and the refusals differ down to
//     the last variant (`BadTableSize` is a Z-machine grammar-version check;
//     `TablesNotFound` and `UnicodeDictionary` belong to a locator that has to
//     close a chain). Sharing the enum would mean each engine carrying variants
//     it can never return.
//
// A few things below are likewise produced by exactly one engine —
// [`Token::InfocomObject`], [`RoutineRef::Index`], [`WordRoles::special`] — and
// each says so in its own documentation. That is the price of one vocabulary,
// and it is much cheaper than two: every one of them is a fact about a real
// story that a consumer would otherwise have to reach for a second type to see.
//
// ── What this is not ─────────────────────────────────────────────────────────
//
// A parser. Nothing here reads bytes, rewrites player input, or emits
// player-facing text. The `describe` methods exist to diff a reader against its
// reference implementation (`infodump -g`, `glulxdump`) and for debug
// inspectors; a consumer showing a suggestion to a player writes its own
// wording.

#![forbid(unsafe_code)]

/// The parser's built-in noun slots.
///
/// Inform names all ten and both engines transcribe the same numbering from the
/// Inform Technical Manual §8.6 (the Glulx Inform Technical Reference §6 shares
/// it). Infocom's own tables distinguish none of them and always yield
/// [`NounKind::Noun`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NounKind {
    Noun,
    Held,
    Multi,
    MultiHeld,
    MultiExcept,
    MultiInside,
    Creature,
    Special,
    Number,
    /// GV2 and Glulx only — Inform 5 and GV1 have no `topic` token.
    Topic,
}

impl NounKind {
    /// Inform's elementary token numbering, shared by GV1's values 0–8, GV2's
    /// type-1 data 0–9, and Glulx's type-1 data 0–9.
    ///
    /// `None` for anything else, which is a malformed line rather than a slot
    /// this crate does not name.
    pub fn from_elementary(v: u32) -> Option<NounKind> {
        Some(match v {
            0 => NounKind::Noun,
            1 => NounKind::Held,
            2 => NounKind::Multi,
            3 => NounKind::MultiHeld,
            4 => NounKind::MultiExcept,
            5 => NounKind::MultiInside,
            6 => NounKind::Creature,
            7 => NounKind::Special,
            8 => NounKind::Number,
            9 => NounKind::Topic,
            _ => return None,
        })
    }

    /// The name Inform, `infodump` and `glulxdump` use for this slot.
    pub fn name(self) -> &'static str {
        match self {
            NounKind::Noun => "noun",
            NounKind::Held => "held",
            NounKind::Multi => "multi",
            NounKind::MultiHeld => "multiheld",
            NounKind::MultiExcept => "multiexcept",
            NounKind::MultiInside => "multiinside",
            NounKind::Creature => "creature",
            NounKind::Special => "special",
            NounKind::Number => "number",
            NounKind::Topic => "topic",
        }
    }
}

/// How a grammar line names a game routine.
///
/// The three formats number routines in three unrelated ways and none of the
/// numbers is another's, so the distinction travels with the value rather than
/// being inferred from the engine later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoutineRef {
    /// Z-machine Inform 5 / GV1: an index into the "preactions" table, counted
    /// upwards from 0 in order of first use (Inform Technical Manual §8.6).
    Index(u8),
    /// Z-machine GV2: the routine's packed address, written straight into the
    /// token. Unpacking it needs the story's version and header, which is why
    /// it is carried as written.
    Packed(u16),
    /// Glulx: a plain address, which is what the token holds and what the
    /// routine is at.
    Address(u32),
}

impl RoutineRef {
    /// A short rendering in the reference tools' spelling — `parse 5` for a
    /// preactions index, `parse $1234` for a packed address, `0x7f21c` for a
    /// Glulx one. Debug output, not player-facing text.
    pub fn describe(self) -> String {
        match self {
            RoutineRef::Index(i) => format!("parse {i}"),
            RoutineRef::Packed(a) => format!("parse ${a:04x}"),
            RoutineRef::Address(a) => format!("{a:#x}"),
        }
    }
}

/// One position in a syntax line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token {
    /// A noun phrase the player supplies.
    Noun(NounKind),
    /// A literal word the player must type — a preposition, in practice.
    Word(String),
    /// A noun slot the game filters with a routine (`noun = Routine`).
    FilteredNoun(RoutineRef),
    /// A slot parsed entirely by a game routine.
    Routine(RoutineRef),
    /// A slot whose scope a game routine decides (`scope = Routine`).
    Scope(RoutineRef),
    /// A noun slot restricted to objects holding an attribute.
    Attribute(u32),
    /// Infocom Version 6's object slot — Zork Zero, Shogun and Arthur, and
    /// nothing else. `attribute` is the attribute the game's own "suggest a
    /// command" helper associates with the slot; `selector` is a flags byte
    /// whose meaning Russotto's notes in ztools' `showverb.c` record as only
    /// partly understood ($80 anything, $0F an object in scope, $14 possibly
    /// held). Both are carried raw rather than guessed at.
    InfocomObject { attribute: u8, selector: u8 },
}

impl Token {
    /// True if this token is a slot the player fills with a noun phrase.
    pub fn is_noun_slot(&self) -> bool {
        !matches!(self, Token::Word(_))
    }

    /// The literal word this token requires, if it requires one.
    pub fn word(&self) -> Option<&str> {
        match self {
            Token::Word(w) => Some(w.as_str()),
            _ => None,
        }
    }
}

/// One position in a line, together with every token that may fill it.
///
/// Outside the alternative lists both Inform grammar version 2 and Glulx write
/// with `/` there is always exactly one. `'in' / 'into' / 'inside'` is a single
/// slot with three tokens, and a consumer that flattened those into three
/// positions would report a sentence two words longer than the story accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Slot {
    /// The alternatives, in table order; never empty.
    pub alternatives: Vec<Token>,
}

impl Slot {
    /// A slot with no alternatives — every slot outside a `/` list.
    pub fn one(token: Token) -> Slot {
        Slot { alternatives: vec![token] }
    }

    /// The sole token, when the slot has no alternatives.
    pub fn only(&self) -> Option<&Token> {
        match self.alternatives.as_slice() {
            [t] => Some(t),
            _ => None,
        }
    }

    /// True if any alternative is a noun slot.
    pub fn is_noun_slot(&self) -> bool {
        self.alternatives.iter().any(Token::is_noun_slot)
    }

    /// True if `word` fills this slot literally.
    pub fn accepts_word(&self, word: &str) -> bool {
        self.alternatives.iter().any(|t| t.word() == Some(word))
    }
}

/// One sentence shape a verb accepts: `TAKE noun FROM noun` is one line,
/// `TAKE noun` another.
///
/// The action number, the slot order and the reverse flag are one subject and
/// travel together — a caller handed the slots alone can tell you the sentence
/// is legal but not what the story will do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyntaxLine {
    /// The action this line performs. Indexes the actions table; the same
    /// number appears in the `performing: nn` line of games with debugging on.
    pub action: u16,
    /// The action takes its two parameters in the other order. Z-machine GV2
    /// packs this into the action word as $400; Glulx keeps it in a flags byte
    /// of its own. Always false in the formats that have no such bit.
    pub reverse: bool,
    /// The slots after the verb, in the order the player types them.
    pub slots: Vec<Slot>,
}

impl SyntaxLine {
    /// Assemble a line. The three facts are one subject; a reader that has the
    /// slots always has the other two.
    pub fn new(action: u16, reverse: bool, slots: Vec<Slot>) -> SyntaxLine {
        SyntaxLine { action, reverse, slots }
    }

    /// How many noun phrases the player supplies.
    pub fn noun_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_noun_slot()).count()
    }

    /// Every literal word THIS LINE requires, in order — its prepositions.
    ///
    /// Not to be confused with a whole story's vocabulary, which each engine's
    /// `Grammar::words` enumerates. The name is `literals` precisely so the two
    /// cannot be reached for by mistake (SQ-1103).
    pub fn literals(&self) -> Vec<&str> {
        self.slots
            .iter()
            .flat_map(|s| s.alternatives.iter().filter_map(Token::word))
            .collect()
    }

    /// True if this line accepts `nouns` noun phrases with exactly `words` as
    /// its literal words, in order — the question "is `TAKE x FROM y` legal?".
    pub fn accepts(&self, nouns: usize, words: &[&str]) -> bool {
        if self.noun_count() != nouns {
            return false;
        }
        let mut wanted = words.iter();
        for slot in &self.slots {
            if slot.is_noun_slot() {
                continue;
            }
            match wanted.next() {
                Some(w) if slot.accepts_word(w) => {}
                _ => return false,
            }
        }
        wanted.next().is_none()
    }

    /// A one-line rendering in `infodump -g`'s and `glulxdump`'s style, for
    /// debug inspectors and for diffing a reader against its reference
    /// implementation. **Not** player-facing text.
    pub fn describe(&self, verb: &str) -> String {
        let mut out = String::from(verb);
        for slot in &self.slots {
            for (i, tok) in slot.alternatives.iter().enumerate() {
                out.push(' ');
                if i > 0 {
                    out.push_str("/ ");
                }
                match tok {
                    Token::Noun(k) => out.push_str(k.name()),
                    Token::Word(w) => out.push_str(w),
                    Token::FilteredNoun(r) => out.push_str(&format!("noun = [{}]", r.describe())),
                    Token::Routine(r) => out.push_str(&format!("[{}]", r.describe())),
                    Token::Scope(r) => out.push_str(&format!("scope = [{}]", r.describe())),
                    Token::Attribute(a) => out.push_str(&format!("ATTRIBUTE({a})")),
                    Token::InfocomObject { .. } => out.push_str("OBJ"),
                }
            }
        }
        if self.reverse {
            out.push_str(" REVERSE");
        }
        out
    }
}

/// One verb of the story's grammar, with every spelling the dictionary gives it
/// and every sentence shape it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verb {
    /// The grammar verb number. Infocom and Z-machine Inform count downwards
    /// from 255, Glulx Inform downwards from $FFFF (so a Glulx verb's number
    /// here is its index in the pointer array). For Infocom's Version 6 shape
    /// there is no pointer array at all and this is the byte address of the
    /// verb record, which is what that format's dictionary entries hold.
    pub number: u32,
    /// Where this verb's syntax lines live in the story image: the block the
    /// pointer array points at, or — for Infocom's Version 6 shape, which has
    /// no pointer array — the 8-byte verb record, the same address as
    /// [`number`](Verb::number). Worth carrying because it is what a reference
    /// dump has to be aimed at when a reading is disputed.
    pub address: u32,
    /// Every dictionary spelling of this verb, in dictionary order. The first
    /// is the one `infodump` prints; the rest are its synonyms. May be empty
    /// for a verb slot no dictionary word reaches.
    pub words: Vec<String>,
    /// The sentence shapes, in table order.
    pub lines: Vec<SyntaxLine>,
}

impl Verb {
    /// Assemble a verb. `number` first, `address` second — they are the same
    /// value only in Infocom's Version 6 shape, and different everywhere else.
    pub fn new(number: u32, address: u32, words: Vec<String>, lines: Vec<SyntaxLine>) -> Verb {
        Verb { number, address, words, lines }
    }

    /// The spelling to use when naming this verb; `None` for a slot no
    /// dictionary word reaches.
    pub fn word(&self) -> Option<&str> {
        self.words.first().map(String::as_str)
    }

    /// True if the verb can be typed on its own, with no noun.
    pub fn takes_bare(&self) -> bool {
        self.lines.iter().any(|l| l.noun_count() == 0)
    }

    /// The largest number of noun phrases any of this verb's lines accepts.
    pub fn max_nouns(&self) -> usize {
        self.lines.iter().map(SyntaxLine::noun_count).max().unwrap_or(0)
    }

    /// Every literal word any of this verb's lines uses, deduplicated and
    /// sorted — the prepositions this verb expects.
    pub fn prepositions(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().flat_map(SyntaxLine::literals).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// True if some line of this verb accepts `nouns` noun phrases with exactly
    /// `words` as its literal words: `take FROM` yes, `take WITH` no.
    pub fn accepts(&self, nouns: usize, words: &[&str]) -> bool {
        self.lines.iter().any(|l| l.accepts(nouns, words))
    }
}

/// What parts of speech the dictionary marks a word with.
///
/// The flag field's meaning differs between the Infocom and Inform families and
/// between the two Inform back-ends, so each field below says who sets it.
/// [`raw`](WordRoles::raw) is always the field as stored, for a caller that
/// needs a bit this does not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WordRoles {
    /// Every family. Infocom bit $40; Inform bit 0 on both back-ends.
    pub verb: bool,
    /// Every family, and the same bit ($80) in each.
    pub noun: bool,
    /// Infocom's DESC bit ($20) — a true adjective. Neither Inform back-end has
    /// such a bit and this is always false there.
    pub adjective: bool,
    /// Infocom's PREP bit ($08). Inform's bit 3 covers the same ground (words
    /// written literally into grammar lines) and is reported here.
    pub preposition: bool,
    /// Inform only (bit 1): the verb is a command to the interpreter rather
    /// than a request in the game.
    pub meta: bool,
    /// Inform only (bit 2): the noun was declared plural with `//p`.
    pub plural: bool,
    /// Infocom only ($04): the word is "special" — a buzzword or direction.
    pub special: bool,
    /// Glulx Inform only (bit 4): the noun was declared singular with `//s`.
    /// The Z-machine's dictionary has no room for the bit.
    pub singular: bool,
    /// Glulx Inform only (bit 6): the word was truncated to the dictionary's
    /// word length.
    pub truncated: bool,
    /// The flag field exactly as stored — one byte on the Z-machine, widened,
    /// and two on Glulx.
    pub raw: u16,
}

impl WordRoles {
    /// The roles of a word whose flag field is `raw` and none of whose bits
    /// have been read yet. A reader sets the fields its family defines; the
    /// rest stay false, which is what "this family has no such bit" means.
    pub fn from_raw(raw: u16) -> WordRoles {
        WordRoles { raw, ..WordRoles::default() }
    }
}

/// What an object is, and what it can be **called** — one answer, never two.
///
/// A story tells you two different things about a thing in it. Its *printed*
/// name is what the game writes when it mentions the object ("a
/// battery-powered brass lantern"); its *parse* names are the dictionary words
/// the parser will accept for it (`lamp`, `lantern`, `light`). They are not the
/// same set and neither implies the other — Inform 7 objects routinely have an
/// empty printed name and a full word list, and every Infocom object has words
/// that appear nowhere in its printed name.
///
/// The two travel together because a caller that has one almost always needs
/// the other: a panel offering the printed name is offering something the
/// parser has not agreed to accept, and a panel offering `lamp` with nothing
/// beside it cannot say which lamp. Handing them back as one value is the
/// refactoring policy in CLAUDE.md applied at the seam where it is cheapest —
/// there is no call that can supply half the subject.
///
/// Produced by `zvm::objects::ParseNames`, `gvm::objects::ParseNames` and
/// `scott::Database::item_words`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObjectWords {
    /// How the engine identifies this object: the object *number* on the
    /// Z-machine (1-based, §12.3), the object's *address* on Glulx (objects
    /// there are heap structures with no numbering), and the item *index* in a
    /// Scott Adams database.
    pub id: u32,
    /// What the game prints for this object. Empty is a real answer, not a
    /// failure: Inform 7 gives objects no hardware short name at all and prints
    /// them through a rule instead.
    pub printed_name: String,
    /// The dictionary words that refer to this object, in the order the story
    /// stores them. Lower-cased, and truncated exactly as the story's
    /// dictionary truncates them — `lanter` really is all Zork I holds of
    /// "lantern", and typing the full word still matches because the parser
    /// truncates the player's input the same way.
    pub words: Vec<String>,
    /// How many characters the story's dictionary keeps of a word: 6 on a v1–3
    /// Z-machine (§13.3), 9 on v4+ (§13.4), `DICT_WORD_SIZE` on Glulx, and the
    /// header's word length in a Scott Adams database. A stored word THIS long
    /// may be the front of a longer one; a shorter one is complete.
    ///
    /// It travels with the words because a caller cannot recover it from them
    /// and gets a wrong answer without it — matching the player's "lantern"
    /// against Zork I's `lanter` needs this number, and so does deciding
    /// whether it is safe to show a word to a player at all. Note that the
    /// STORED words are not always truncated already: the Z-machine's
    /// dictionary holds `lanter` and nothing more, while a Scott Adams noun
    /// table holds `LAMP` in full and truncates only when it matches. `None`
    /// where the vocabulary is not truncated.
    pub truncated_at: Option<usize>,
    /// Which property the words were read from, for a caller that wants to say
    /// where the answer came from. `1` for every Inform story on either
    /// back-end; a per-game number for Infocom's own (18 in Zork I, 17 in Zork
    /// II, 14 in Seastalker). `None` where the engine has no properties at all,
    /// which is Scott Adams.
    pub property: Option<u32>,
}

impl ObjectWords {
    /// Build one. `#[non_exhaustive]`, so this is how another crate makes one.
    pub fn new(
        id: u32,
        printed_name: String,
        words: Vec<String>,
        property: Option<u32>,
        truncated_at: Option<usize>,
    ) -> ObjectWords {
        ObjectWords { id, printed_name, words, property, truncated_at }
    }

    /// True when `word` is one of this object's parse names.
    ///
    /// BOTH sides are truncated the way the story's own vocabulary truncates
    /// them ([`truncated_at`](ObjectWords::truncated_at)) before comparing, so
    /// "lantern" matches Zork I's stored `lanter` while "lan" does not. Both
    /// sides, because the two engines differ in which side is already short:
    /// the Z-machine dictionary stores `lanter` truncated, and a Scott Adams
    /// noun table stores `LAMP` in full and truncates only when matching.
    pub fn refers_to(&self, word: &str) -> bool {
        let key = self.truncate(&word.trim().to_lowercase());
        self.words.iter().any(|s| self.truncate(s) == key)
    }

    fn truncate(&self, word: &str) -> String {
        match self.truncated_at {
            Some(n) => word.chars().take(n).collect(),
            None => word.to_string(),
        }
    }

    /// `printed name [word, word, …]`, for a debug inspector or a test failure.
    pub fn describe(&self) -> String {
        let name = if self.printed_name.is_empty() { "(unnamed)" } else { &self.printed_name };
        format!("{name} [{}]", self.words.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(w: &str) -> Slot {
        Slot::one(Token::Word(w.to_string()))
    }

    fn noun() -> Slot {
        Slot::one(Token::Noun(NounKind::Noun))
    }

    #[test]
    fn elementary_numbering_is_inform_s_and_stops_at_nine() {
        assert_eq!(NounKind::from_elementary(0), Some(NounKind::Noun));
        assert_eq!(NounKind::from_elementary(3), Some(NounKind::MultiHeld));
        assert_eq!(NounKind::from_elementary(9), Some(NounKind::Topic));
        assert_eq!(NounKind::from_elementary(10), None);
        assert_eq!(NounKind::from_elementary(0xFFFF_FFFF), None);
        assert_eq!(NounKind::MultiInside.name(), "multiinside");
    }

    #[test]
    fn a_slot_holds_every_alternative_and_answers_for_all_of_them() {
        let mut slot = word("in");
        assert!(slot.only().is_some());
        slot.alternatives.push(Token::Word("into".into()));
        assert!(slot.only().is_none());
        assert!(slot.accepts_word("in") && slot.accepts_word("into"));
        assert!(!slot.accepts_word("on"));
        assert!(!slot.is_noun_slot());
        assert!(noun().is_noun_slot());
    }

    #[test]
    fn accepts_matches_noun_count_and_literals_in_order() {
        // "take noun from noun"
        let line = SyntaxLine::new(4, false, vec![noun(), word("from"), noun()]);
        assert_eq!(line.noun_count(), 2);
        assert_eq!(line.literals(), vec!["from"]);
        assert!(line.accepts(2, &["from"]));
        assert!(!line.accepts(2, &["with"]));
        assert!(!line.accepts(1, &["from"]));
        assert!(!line.accepts(2, &[]));
    }

    #[test]
    fn describe_renders_each_engine_s_routine_reference_in_its_own_spelling() {
        let gv1 = SyntaxLine::new(1, false, vec![Slot::one(Token::Scope(RoutineRef::Index(6)))]);
        assert_eq!(gv1.describe("put"), "put scope = [parse 6]");
        let gv2 =
            SyntaxLine::new(1, false, vec![Slot::one(Token::Routine(RoutineRef::Packed(0x1234)))]);
        assert_eq!(gv2.describe("get"), "get [parse $1234]");
        let glulx = SyntaxLine::new(
            1,
            false,
            vec![Slot::one(Token::FilteredNoun(RoutineRef::Address(0x7f21c)))],
        );
        assert_eq!(glulx.describe("get"), "get noun = [0x7f21c]");
        let v6 = SyntaxLine::new(
            1,
            false,
            vec![Slot::one(Token::InfocomObject { attribute: 3, selector: 0x80 })],
        );
        assert_eq!(v6.describe("unlock"), "unlock OBJ");
    }

    #[test]
    fn describe_names_the_reverse_flag_the_way_the_reference_tools_do() {
        let mut line = SyntaxLine::new(8, true, vec![noun(), word("in"), noun()]);
        assert_eq!(line.describe("take"), "take noun in noun REVERSE");
        line.reverse = false;
        assert_eq!(line.describe("take"), "take noun in noun");
    }

    #[test]
    fn a_verb_answers_over_all_of_its_lines() {
        let verb = Verb::new(
            255,
            0x0410,
            vec!["take".to_string(), "grab".to_string()],
            vec![
                SyntaxLine::new(3, false, vec![noun()]),
                SyntaxLine::new(4, false, vec![noun(), word("with"), noun()]),
            ],
        );
        assert_eq!(verb.word(), Some("take"));
        assert_eq!(verb.address, 0x0410);
        assert!(!verb.takes_bare());
        assert_eq!(verb.max_nouns(), 2);
        assert_eq!(verb.prepositions(), vec!["with"]);
        assert!(verb.accepts(2, &["with"]));
        assert!(!verb.accepts(2, &["from"]));
    }

    #[test]
    fn word_roles_start_from_the_raw_field_with_nothing_read() {
        let r = WordRoles::from_raw(0x41);
        assert_eq!(r.raw, 0x41);
        assert!(!r.verb && !r.noun && !r.adjective && !r.singular);
        assert_eq!(r, WordRoles { raw: 0x41, ..WordRoles::default() });
    }

    #[test]
    fn an_object_s_words_answer_for_the_dictionary_s_truncated_spelling() {
        let o = ObjectWords {
            id: 102,
            printed_name: "brass lantern".into(),
            words: vec!["lamp".into(), "lanter".into(), "light".into()],
            property: Some(18),
            truncated_at: Some(6),
        };
        // Either spelling: what the story holds, and what the player types.
        assert!(o.refers_to("lanter"));
        assert!(o.refers_to("lantern"));
        assert!(o.refers_to("LAMP"));
        assert!(!o.refers_to("lan"));
        assert!(!o.refers_to("sword"));
        // An untruncated vocabulary compares whole words and nothing else.
        let mut plain = o.clone();
        plain.truncated_at = None;
        assert!(plain.refers_to("lanter") && !plain.refers_to("lantern"));
        // A vocabulary that stores whole words and truncates only on match —
        // Scott Adams — is answered from the same field.
        let scott = ObjectWords::new(9, "a brass lamp".into(), vec!["lamp".into()], None, Some(3));
        assert!(scott.refers_to("lamp") && scott.refers_to("lamps") && scott.refers_to("lam"));
        assert!(!scott.refers_to("la"));
        assert_eq!(
            ObjectWords::new(1, "x".into(), vec!["x".into()], None, None).property,
            None
        );
        assert_eq!(o.describe(), "brass lantern [lamp, lanter, light]");
    }

    #[test]
    fn an_empty_printed_name_is_an_answer_and_still_describes() {
        let o = ObjectWords {
            id: 7,
            printed_name: String::new(),
            words: vec!["pig".into()],
            property: Some(1),
            truncated_at: Some(9),
        };
        assert_eq!(o.describe(), "(unnamed) [pig]");
        assert!(o.refers_to("pig"));
    }
}
