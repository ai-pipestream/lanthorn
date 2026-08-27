// Story grammar (syntax) tables — the parts of speech and sentence shapes a
// story knows, as opposed to the flat word list `dictionary.rs` returns.
//
// ── Where the formats are specified ──────────────────────────────────────────
//
// The Z-Machine Standards Document specifies the DICTIONARY (§13) and nothing
// else here: "The grammar tables, used by the parser in an adventure game, are
// not specified by the Z-machine at all (contrary to popular opinion)"
// — Inform Technical Manual §8.6. Everything below therefore comes from two
// non-ZMSD sources, both consulted directly rather than from memory:
//
//   * **Inform Technical Manual** (Graham Nelson), §8.5 "Dictionary" for the
//     `dict_par1..3` flag byte and verb/preposition numbering, and §8.6
//     "Grammar version numbers GV1 and GV2" for both Inform line formats, the
//     token value tables, the ENDIT marker, the $400 REVERSE bit and the
//     "adjectives" (preposition) table's 4-byte entries.
//     <https://www.inform-fiction.org/source/tm/TechMan.txt>
//
//   * **ztools / infodump** (Mark Howell; V6 grammar work by Matthew T. Russotto),
//     `showverb.c` — the reference implementation, and the only written
//     description of Infocom's own table shapes: the fixed 8-byte and variable
//     2/4/7-byte ZIL syntax entries, the preposition-table two forms, and the
//     wholly different Version 6 layout used by Zork Zero, Shogun and Arthur.
//     Format constants (`VERB` $40, `PREP` $08, `DESC` $20, `NOUN` $80,
//     `DATA_FIRST` $03, `ENDIT` $0F) are from its `tx.h`.
//     <https://github.com/ecliptik/ztools>
//
// Every table shape below was checked against `infodump -g` output for real
// stories; see `crates/zvm/tests/grammar_tables.rs`.
//
// ── The five shapes ──────────────────────────────────────────────────────────
//
// All but the Version 6 Infocom games put a table of 2-byte pointers at the
// base of static memory (header $0E), one pointer per verb. A verb's number in
// the dictionary counts DOWN from 255, so verb number 255 is table slot 0.
// Each pointer leads to a count byte and then that many syntax lines:
//
//   InfocomFixed     8 bytes/line: [objs][prep1][prep2][.. 4 ..][action]
//   InfocomVariable  2, 4 or 7 bytes/line, sized by the top two bits of byte 0
//   Inform5 / Gv1    8 bytes/line: [params][6 token bytes][action]
//   InformGv2        variable: [action word][3-byte tokens..][ENDIT]
//
//   InfocomV6        no pointer table at all — the dictionary entry itself
//                    holds the address of an 8-byte verb record, which points
//                    at separate one-object and two-object entry blocks.
//
// ── What this module is not ──────────────────────────────────────────────────
//
// A read-only description of what the story's parser will accept. It is not a
// parser, it does not rewrite input, and it emits no player-facing text. The
// `describe` methods exist to diff this module against `infodump -g` and for
// debug inspectors; a consumer showing something to a player writes its own
// wording.

use std::collections::BTreeMap;

use crate::dictionary::{self, Dictionary};
use crate::memory::Memory;
use crate::text::decode_string;

// ── Format constants, from ztools `tx.h` and Inform Technical Manual §8.5 ────

/// Infocom V1–5 dictionary flag: the word can be a verb.
const F_INFOCOM_VERB: u8 = 0x40;
/// Infocom V1–5 dictionary flag: the word can be a noun.
const F_INFOCOM_NOUN: u8 = 0x80;
/// Infocom V1–5 dictionary flag: the word can be an adjective ("descriptor").
const F_INFOCOM_DESC: u8 = 0x20;
/// Infocom V1–5 dictionary flag: the word can be a preposition.
const F_INFOCOM_PREP: u8 = 0x08;
/// Infocom V1–5 dictionary flag: the word is "special" (buzzword/direction).
const F_INFOCOM_SPECIAL: u8 = 0x04;
/// Infocom V1–5: which data byte comes first (`DATA_FIRST` in `tx.h`).
const F_INFOCOM_DATA_FIRST: u8 = 0x03;
/// `DATA_FIRST` value meaning the verb number is the first data byte.
const F_INFOCOM_VERB_FIRST: u8 = 0x01;

/// Inform dictionary flag bit 0: the word can be a verb.
const F_INFORM_VERB: u8 = 0x01;
/// Inform dictionary flag bit 1: the verb is "meta".
const F_INFORM_META: u8 = 0x02;
/// Inform dictionary flag bit 2: the noun is plural (`//p`).
const F_INFORM_PLURAL: u8 = 0x04;
/// Inform dictionary flag bit 3: the word appears literally in grammar lines
/// (Inform calls it "adj"; in practice these are prepositions).
const F_INFORM_ADJ: u8 = 0x08;
/// Inform dictionary flag bit 7: the word can be a noun.
const F_INFORM_NOUN: u8 = 0x80;

/// GV2's end-of-line marker (Inform Technical Manual §8.6).
const ENDIT: u8 = 0x0F;

/// A sanity ceiling on the verb-pointer table. Both Inform and Infocom number
/// verbs downwards from 255, so 256 is the real limit; the slack allows for a
/// future 2-byte non-inverted index without turning a valid table into an error.
const MAX_VERBS: u32 = 512;

/// A sanity ceiling on syntax lines per verb. Inform 6.10 allows 32; Infocom's
/// own games stay well under. Anything larger means we are not reading a table.
const MAX_LINES_PER_VERB: u8 = 64;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Why a story's grammar could not be read.
///
/// Every variant except [`Absent`](GrammarError::Absent) means the bytes did not
/// describe a table this module recognises. Refusing is the point: a grammar
/// parser that silently yields a wrong-but-well-formed table hands its consumer
/// confident nonsense with no way to detect it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarError {
    /// The story has no grammar table. Journey is the canonical example: a
    /// Version 6 game driven entirely by menus, whose dictionary marks no verbs.
    /// Also reached when the verb-pointer table's first entry is zero.
    Absent,
    /// A table address or entry ran past the end of the story file.
    Truncated,
    /// The verb-pointer table's shape is impossible (zero-length, absurdly
    /// large, or pointing backwards into itself).
    BadVerbTable,
    /// A syntax line held a value the format forbids — an object count above 2,
    /// a GV1 token in the illegal 9–15 or 112–127 ranges, an unknown GV2 token
    /// type, or a GV2 line that never reached its ENDIT.
    BadSyntaxLine,
    /// The grammar table's total size is not a whole number of entries for
    /// either Inform grammar version (Inform Technical Manual §8.6).
    BadTableSize,
}

// ── The value types ──────────────────────────────────────────────────────────

/// Which of the five table shapes a story uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarFormat {
    /// Infocom ZIL, fixed 8-byte syntax lines (the earlier games).
    InfocomFixed,
    /// Infocom ZIL, variable 2/4/7-byte syntax lines (the later V3–V5 games).
    InfocomVariable,
    /// Infocom's Version 6 games — Zork Zero, Shogun, Arthur.
    InfocomV6,
    /// Inform 1–5, whose lines are GV1-shaped.
    Inform5,
    /// Inform 6 grammar version 1.
    InformGv1,
    /// Inform 6 grammar version 2 (library 6/3 and later).
    InformGv2,
}

impl GrammarFormat {
    /// True for the three Infocom-era shapes.
    pub fn is_infocom(self) -> bool {
        matches!(
            self,
            GrammarFormat::InfocomFixed | GrammarFormat::InfocomVariable | GrammarFormat::InfocomV6
        )
    }

    /// True for the three Inform-compiler shapes.
    pub fn is_inform(self) -> bool {
        !self.is_infocom()
    }
}

/// The parser's built-in noun slots. Inform names all ten; Infocom's own tables
/// distinguish none of them and always yield [`NounKind::Noun`].
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
    /// GV2 only — Inform 5 and GV1 have no `topic` token.
    Topic,
}

impl NounKind {
    /// Inform's elementary token numbering (Technical Manual §8.6), shared by
    /// GV1's values 0–8 and GV2's type-1 data 0–9.
    fn from_elementary(v: u16) -> Option<NounKind> {
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

    /// The name Inform and `infodump` use for this slot.
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

/// How a grammar line names a game routine. The two grammar versions number
/// routines differently and neither number is the other's, so the distinction
/// travels with the value rather than being inferred from the format later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RoutineRef {
    /// Inform 5 / GV1: an index into the "preactions" table, counted upwards
    /// from 0 in order of first use (Inform Technical Manual §8.6).
    Index(u8),
    /// GV2: the routine's packed address, written straight into the token.
    Packed(u16),
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
    Attribute(u8),
    /// Infocom Version 6's object slot. `attribute` is the attribute the game's
    /// own "suggest a command" helper associates with the slot; `selector` is a
    /// flags byte whose meaning Russotto's notes in `showverb.c` record as only
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
/// Outside GV2's `/` alternative lists there is always exactly one; GV2 encodes
/// `'in' / 'into' / 'inside'` as a single slot with three tokens, and a consumer
/// that flattened those into three positions would report a sentence three words
/// longer than the story accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Slot {
    /// The alternatives, in table order; never empty.
    pub alternatives: Vec<Token>,
}

impl Slot {
    fn one(token: Token) -> Slot {
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
/// The action number, the slot order and GV2's reverse bit are one subject and
/// travel together — a caller handed the slots alone can tell you the sentence
/// is legal but not what the story will do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyntaxLine {
    /// The action this line performs. Indexes the actions table; the same
    /// number appears in the `performing: nn` line of games with debugging on.
    pub action: u16,
    /// GV2's $400 bit: the action takes its two parameters in the other order.
    /// Always false outside GV2.
    pub reverse: bool,
    /// The slots after the verb, in the order the player types them.
    pub slots: Vec<Slot>,
}

impl SyntaxLine {
    /// How many noun phrases the player supplies.
    pub fn noun_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_noun_slot()).count()
    }

    /// Every literal word this line requires, in order.
    pub fn words(&self) -> Vec<&str> {
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

    /// A one-line rendering in `infodump -g`'s style, for debug inspectors and
    /// for diffing this module against the reference implementation. **Not**
    /// player-facing text: a consumer showing a suggestion writes its own.
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
                    Token::FilteredNoun(r) => out.push_str(&format!("noun = [{}]", describe_ref(*r))),
                    Token::Routine(r) => out.push_str(&format!("[{}]", describe_ref(*r))),
                    Token::Scope(r) => out.push_str(&format!("scope = [{}]", describe_ref(*r))),
                    Token::Attribute(a) => out.push_str(&format!("ATTRIBUTE({a})")),
                    Token::InfocomObject { .. } => out.push_str("OBJ"),
                }
            }
        }
        out
    }
}

fn describe_ref(r: RoutineRef) -> String {
    match r {
        RoutineRef::Index(i) => format!("parse {i}"),
        RoutineRef::Packed(a) => format!("parse ${a:04x}"),
    }
}

/// One verb of the story's grammar, with every spelling the dictionary gives it
/// and every sentence shape it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verb {
    /// The grammar verb number (Infocom/Inform count downwards from 255). For
    /// [`GrammarFormat::InfocomV6`] this is the byte address of the verb record,
    /// which is what that format's dictionary entries hold.
    pub number: u16,
    /// Every dictionary spelling of this verb, in dictionary order. The first
    /// is the one `infodump` prints; the rest are its synonyms. May be empty for
    /// a Version 6 verb slot no dictionary word reaches.
    pub words: Vec<String>,
    /// The sentence shapes, in table order.
    pub lines: Vec<SyntaxLine>,
}

impl Verb {
    /// The spelling to use when naming this verb; `None` for an unreachable slot.
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

    /// Every literal word any of this verb's lines uses, deduplicated and sorted
    /// — the prepositions this verb expects.
    pub fn prepositions(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().flat_map(SyntaxLine::words).collect();
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
/// The flag byte's meaning differs between the Infocom and Inform families, so
/// each field below documents which family sets it. `raw` is always the byte as
/// stored, for a caller that needs a bit this does not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WordRoles {
    /// Both families. Infocom bit $40; Inform bit 0.
    pub verb: bool,
    /// Both families, and the same bit ($80) in each.
    pub noun: bool,
    /// Infocom's DESC bit ($20) — a true adjective. Inform has no such bit and
    /// this is always false there.
    pub adjective: bool,
    /// Infocom's PREP bit ($08). Inform's bit 3 covers the same ground (words
    /// written literally into grammar lines), and is reported here.
    pub preposition: bool,
    /// Inform only (bit 1): the verb is a command to the interpreter rather
    /// than a request in the game.
    pub meta: bool,
    /// Inform only (bit 2): the noun was declared plural with `//p`.
    pub plural: bool,
    /// Infocom only ($04): the word is "special" — a buzzword or direction.
    pub special: bool,
    /// The flag byte exactly as stored.
    pub raw: u8,
}

/// A story's grammar: which words are verbs, and what each verb accepts.
///
/// Built once from the story image and self-contained afterwards — no `&Memory`
/// is needed to query it, so it can be cached beside a session or handed to
/// another thread. Cost is proportional to the dictionary (a few hundred
/// kilobytes for the largest stories).
#[derive(Debug, Clone)]
pub struct Grammar {
    format: GrammarFormat,
    verbs: Vec<Verb>,
    /// Dictionary spelling → index into `verbs`.
    by_word: BTreeMap<String, usize>,
    prepositions: Vec<String>,
    roles: BTreeMap<String, WordRoles>,
    action_routines: Vec<u32>,
}

impl Grammar {
    /// Read the story's grammar tables.
    ///
    /// Returns [`GrammarError::Absent`] for a story that has no grammar (a menu
    /// -driven Version 6 game such as Journey), and one of the other variants
    /// when the bytes do not describe a table this module recognises.
    pub fn load(mem: &Memory) -> Result<Grammar, GrammarError> {
        let format = detect_format(mem);
        let dict = dictionary::load(mem);
        let words = scan_dictionary(mem, &dict, format);

        let mut roles = BTreeMap::new();
        for w in &words {
            roles.insert(w.text.clone(), w.roles);
        }

        // `detect_format` narrows the family from the header; the two
        // within-family ambiguities (Infocom fixed vs variable, Inform GV1 vs
        // GV2) can only be settled by the table itself, so `load_classic`
        // returns the format it actually read.
        let (format, verbs, action_routines) = if format == GrammarFormat::InfocomV6 {
            let (v, a) = load_v6(mem, &words)?;
            (format, v, a)
        } else {
            load_classic(mem, format, &words)?
        };

        let mut by_word = BTreeMap::new();
        for (i, v) in verbs.iter().enumerate() {
            for w in &v.words {
                by_word.entry(w.clone()).or_insert(i);
            }
        }

        let mut prepositions: Vec<String> = verbs
            .iter()
            .flat_map(|v| v.lines.iter())
            .flat_map(SyntaxLine::words)
            .map(str::to_string)
            .collect();
        prepositions.sort();
        prepositions.dedup();

        Ok(Grammar { format, verbs, by_word, prepositions, roles, action_routines })
    }

    /// Which table shape this story uses.
    pub fn format(&self) -> GrammarFormat {
        self.format
    }

    /// Every verb, in grammar-table order.
    pub fn verbs(&self) -> &[Verb] {
        &self.verbs
    }

    /// The verb a spelling belongs to, if it is one.
    pub fn verb_for_word(&self, word: &str) -> Option<&Verb> {
        let key = word.to_lowercase();
        self.by_word.get(&key).map(|&i| &self.verbs[i])
    }

    /// True if the story can begin a command with this word.
    pub fn is_verb(&self, word: &str) -> bool {
        self.by_word.contains_key(&word.to_lowercase())
    }

    /// Every spelling that can begin a command, sorted.
    pub fn verb_words(&self) -> impl Iterator<Item = &str> {
        self.by_word.keys().map(String::as_str)
    }

    /// Every literal word the grammar names, deduplicated and sorted — the
    /// story's prepositions.
    pub fn prepositions(&self) -> &[String] {
        &self.prepositions
    }

    /// True if the grammar uses this word literally in some line.
    pub fn is_preposition(&self, word: &str) -> bool {
        let key = word.to_lowercase();
        self.prepositions.binary_search(&key).is_ok()
    }

    /// The parts of speech the dictionary marks `word` with, if it knows it.
    pub fn roles(&self, word: &str) -> Option<WordRoles> {
        self.roles.get(&word.to_lowercase()).copied()
    }

    /// Unpacked byte addresses of the action routines, indexed by action number.
    /// Empty for [`GrammarFormat::InfocomV6`], whose action table this module
    /// locates but does not walk.
    pub fn action_routines(&self) -> &[u32] {
        &self.action_routines
    }

    /// Every verb with a line matching `nouns` noun phrases and exactly `words`
    /// as its literal words — the shape query a caller uses to keep a suggestion
    /// plausible instead of merely near.
    pub fn verbs_accepting(&self, nouns: usize, words: &[&str]) -> Vec<&Verb> {
        self.verbs.iter().filter(|v| v.accepts(nouns, words)).collect()
    }
}

// ── Dictionary scan ──────────────────────────────────────────────────────────

/// One dictionary entry, decoded far enough to answer part-of-speech questions.
struct DictWord {
    text: String,
    roles: WordRoles,
    /// The verb number (classic formats) or verb-record address (V6), when the
    /// flags mark this word a verb.
    verb_key: Option<u16>,
}

/// Decode the whole dictionary once, reading each entry's flag byte and, for
/// verbs, the number that links it to the grammar table.
///
/// Flag byte position: the entry's data follows the 4-byte (v1–3) or 6-byte
/// (v4+) key, so the flags are the first data byte — except in Infocom's V6
/// games, where `showverb.c` records that the flags moved to the LAST byte of
/// the entry and the first data word became the verb record's address.
fn scan_dictionary(mem: &Memory, dict: &Dictionary, format: GrammarFormat) -> Vec<DictWord> {
    let elen = dict.entry_length as u32;
    let klen = dict.key_len() as u32;
    let mut out = Vec::with_capacity(dict.count as usize);

    for i in 0..dict.count as u32 {
        let entry = dict.base + i * elen;
        if (entry + elen) as usize > mem.len() {
            break;
        }
        let (text, _) = decode_string(mem, entry);
        let text = text.trim().to_lowercase();
        if text.is_empty() {
            continue;
        }

        let (flags, data0, data1) = if format == GrammarFormat::InfocomV6 {
            (mem.read_byte(entry + elen - 1), 0, 0)
        } else {
            (
                mem.read_byte(entry + klen),
                mem.read_byte(entry + klen + 1),
                mem.read_byte(entry + klen + 2),
            )
        };

        let roles = decode_roles(flags, format);

        let verb_key = if format == GrammarFormat::InfocomV6 {
            // The verb record's address lives in the first data word, and the
            // verb bit is $01 as in Inform (`VERB_V6` in ztools' `tx.h`). A
            // word may be both noun and verb — Shogun's "who", "what", "why" —
            // so the $80 bit does not disqualify it here. `load_v6` applies the
            // stricter test separately, where it belongs: to bounding the verb
            // record area.
            let addr = mem.read_word(entry + klen);
            (flags & F_INFORM_VERB != 0 && addr != 0).then_some(addr)
        } else if format.is_inform() {
            // Inform Technical Manual §8.5: dict_par2 is the verb number.
            (flags & F_INFORM_VERB != 0).then_some(data0 as u16)
        } else if flags & F_INFOCOM_VERB != 0 {
            // ztools `lookup_word`: the verb number is the first data byte when
            // DATA_FIRST says VERB_FIRST, otherwise the second.
            let n = if flags & F_INFOCOM_DATA_FIRST == F_INFOCOM_VERB_FIRST { data0 } else { data1 };
            Some(n as u16)
        } else {
            None
        };

        out.push(DictWord { text, roles, verb_key });
    }

    out
}

fn decode_roles(flags: u8, format: GrammarFormat) -> WordRoles {
    if format == GrammarFormat::InfocomV6 {
        // V6 moved the flags but kept Inform's bit 0 for "verb" (ztools
        // `VERB_V6` = $01) and $80 for noun.
        WordRoles {
            verb: flags & F_INFORM_VERB != 0,
            noun: flags & F_INFORM_NOUN != 0,
            raw: flags,
            ..WordRoles::default()
        }
    } else if format.is_inform() {
        WordRoles {
            verb: flags & F_INFORM_VERB != 0,
            noun: flags & F_INFORM_NOUN != 0,
            adjective: false,
            preposition: flags & F_INFORM_ADJ != 0,
            meta: flags & F_INFORM_META != 0,
            plural: flags & F_INFORM_PLURAL != 0,
            special: false,
            raw: flags,
        }
    } else {
        WordRoles {
            verb: flags & F_INFOCOM_VERB != 0,
            noun: flags & F_INFOCOM_NOUN != 0,
            adjective: flags & F_INFOCOM_DESC != 0,
            preposition: flags & F_INFOCOM_PREP != 0,
            meta: false,
            plural: false,
            special: flags & F_INFOCOM_SPECIAL != 0,
            raw: flags,
        }
    }
}

// ── Format detection ─────────────────────────────────────────────────────────

/// Decide which family compiled this story, from the header alone.
///
/// `showverb.c`'s test, unchanged: a serial number whose six bytes are a
/// plausible YYMMDD *and* whose first digit is not '8' means the Inform
/// compiler, since Infocom's own serials are all 8x. Inform 6 additionally
/// writes its version string into the four bytes at $3C, so a '6' or later
/// there separates Inform 6 from Inform 1–5.
///
/// GV1 versus GV2 needs the table itself and is settled in [`load_classic`].
fn detect_format(mem: &Memory) -> GrammarFormat {
    let s: Vec<u8> = (0x12..0x18).map(|a| mem.read_byte(a)).collect();
    let digit = |b: u8, lo: u8, hi: u8| b >= lo && b <= hi;
    let inform = digit(s[0], b'0', b'9')
        && digit(s[1], b'0', b'9')
        && digit(s[2], b'0', b'1')
        && digit(s[3], b'0', b'9')
        && digit(s[4], b'0', b'3')
        && digit(s[5], b'0', b'9')
        && s[0] != b'8';

    if inform {
        // Byte $3C is the first character of Inform 6's version string.
        if mem.read_byte(0x3C) >= b'6' {
            GrammarFormat::InformGv1 // refined to GV2 once the table is read
        } else {
            GrammarFormat::Inform5
        }
    } else if mem.version() == 6 {
        GrammarFormat::InfocomV6
    } else {
        GrammarFormat::InfocomFixed // refined to Variable once the table is read
    }
}

// ── The classic (pointer-table) formats ──────────────────────────────────────

/// Everything the two passes over the verb table need to agree about.
struct ClassicLayout {
    format: GrammarFormat,
    verb_table_base: u32,
    verb_count: u32,
    action_table_base: u32,
    action_count: u32,
    /// Base of the preposition ("adjectives") table; zero for GV2, which has none.
    prep_table_base: u32,
    /// 0 = 4-byte entries with a word index, 1 = 3-byte entries with a byte index.
    prep_entry_form: u8,
}

/// Read every verb of a pointer-table format, returning the format actually
/// found alongside the verbs and the action-routine table.
fn load_classic(
    mem: &Memory,
    detected: GrammarFormat,
    words: &[DictWord],
) -> Result<(GrammarFormat, Vec<Verb>, Vec<u32>), GrammarError> {
    let layout = configure_classic(mem, detected)?;
    let preps = read_preposition_table(mem, &layout)?;

    // Verb number → its dictionary spellings, in dictionary order.
    let mut spellings: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for w in words {
        if let Some(n) = w.verb_key {
            spellings.entry(n).or_default().push(w.text.clone());
        }
    }

    let mut verbs = Vec::with_capacity(layout.verb_count as usize);
    for i in 0..layout.verb_count {
        // The dictionary's verb numbers count downwards from 255, so table slot
        // i belongs to verb number 255 - i (Inform Technical Manual §8.5; the
        // same convention in Infocom's own games).
        let number = 255u16.wrapping_sub(i as u16);
        let entry_ptr = layout.verb_table_base + i * 2;
        let mut cur = read_word(mem, entry_ptr)? as u32;
        let count = read_byte(mem, cur)?;
        cur += 1;
        if count > MAX_LINES_PER_VERB {
            return Err(GrammarError::BadVerbTable);
        }

        let mut lines = Vec::with_capacity(count as usize);
        for _ in 0..count {
            lines.push(read_classic_line(mem, &layout, &preps, &mut cur)?);
        }

        verbs.push(Verb { number, words: spellings.remove(&number).unwrap_or_default(), lines });
    }

    let mut action_routines = Vec::with_capacity(layout.action_count as usize);
    for i in 0..layout.action_count {
        let packed = read_word(mem, layout.action_table_base + i * 2)?;
        action_routines.push(mem.unpack_routine(packed));
    }

    Ok((layout.format, verbs, action_routines))
}

/// Locate the tables and settle the two format ambiguities (Infocom fixed vs
/// variable, Inform GV1 vs GV2), following `showverb.c::configure_parse_tables`.
fn configure_classic(mem: &Memory, detected: GrammarFormat) -> Result<ClassicLayout, GrammarError> {
    let verb_table_base = mem.static_mem_base() as u32;
    let first = read_word(mem, verb_table_base)? as u32;
    if first == 0 {
        return Err(GrammarError::Absent);
    }
    if first <= verb_table_base || !(first - verb_table_base).is_multiple_of(2) {
        return Err(GrammarError::BadVerbTable);
    }
    let verb_count = (first - verb_table_base) / 2;
    if verb_count == 0 || verb_count > MAX_VERBS {
        return Err(GrammarError::BadVerbTable);
    }
    if first as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }

    // The second pointer bounds the first verb's data, which is what tells the
    // two Infocom shapes and the two Inform grammar versions apart.
    let second = if verb_count >= 2 { read_word(mem, verb_table_base + 2)? as u32 } else { first };
    let entry_count = read_byte(mem, first)?;
    let span = second.saturating_sub(first);

    let format = match detected {
        GrammarFormat::InfocomFixed => {
            if entry_count > 0 && span > 0 && span / entry_count as u32 <= 7 {
                GrammarFormat::InfocomVariable
            } else {
                GrammarFormat::InfocomFixed
            }
        }
        GrammarFormat::InformGv1 => classify_inform6(mem, first, span, entry_count)?,
        other => other,
    };

    // Pass one: the highest action number and parsing-routine number in use,
    // and where the grammar data ends.
    let mut action_count = 0u32;
    let mut parse_count = 0u32;
    let mut data_end = first;
    for i in 0..verb_count {
        let mut cur = read_word(mem, verb_table_base + i * 2)? as u32;
        let n = read_byte(mem, cur)?;
        cur += 1;
        if n > MAX_LINES_PER_VERB {
            return Err(GrammarError::BadVerbTable);
        }
        for _ in 0..n {
            let (action, parse_max) = measure_classic_line(mem, format, &mut cur)?;
            action_count = action_count.max(action as u32 + 1);
            parse_count = parse_count.max(parse_max);
        }
        data_end = data_end.max(cur);
    }

    // The action table follows the grammar data, after any zero padding.
    let mut action_table_base = data_end;
    let mut skipped = 0;
    while read_byte(mem, action_table_base)? == 0 {
        action_table_base += 1;
        skipped += 1;
        if skipped > 32 {
            return Err(GrammarError::BadVerbTable);
        }
    }

    let preact_table_base = action_table_base + action_count * 2;
    let (prep_table_base, prep_entry_form) = if format == GrammarFormat::InformGv2 {
        // GV2 has neither a preactions nor an adjectives table.
        (0, 0)
    } else {
        let base = if format == GrammarFormat::InformGv1 {
            preact_table_base + parse_count * 2
        } else {
            preact_table_base + action_count * 2
        };
        // Form 0 stores the index in a word, so the byte after the first
        // entry's dictionary address is that word's zero high byte.
        let form = u8::from(read_byte(mem, base + 4)? != 0);
        (base, form)
    };

    Ok(ClassicLayout {
        format,
        verb_table_base,
        verb_count,
        action_table_base,
        action_count,
        prep_table_base,
        prep_entry_form,
    })
}

/// GV1 or GV2, by the size of the first verb's grammar data.
///
/// A GV1 line is 8 bytes and a table is `1 + 8n`; a GV2 line is `2 + 3t + 1` and
/// a table is `1 + sum`, so `1 mod 3`. `1 mod 24` fits both and is settled by
/// looking for values GV1 forbids (Inform Technical Manual §8.6 lists 9–15 and
/// 112–127 as illegal token bytes).
///
/// `showverb.c` writes that disambiguation as
/// `if ((val >= 9) || (val <= 15) || (val >= 112) || (val <= 127))`, which is
/// true for almost every byte and so declares GV2 unconditionally. The `&&`
/// pairing plainly intended is used here instead; the difference can only show
/// on a table whose first verb's data is `1 mod 24` bytes long.
fn classify_inform6(
    mem: &Memory,
    first: u32,
    span: u32,
    entry_count: u8,
) -> Result<GrammarFormat, GrammarError> {
    if span == 0 {
        return Ok(GrammarFormat::InformGv1);
    }
    if span % 3 == 1 {
        if entry_count as u32 * 8 + 1 != span {
            return Ok(GrammarFormat::InformGv2);
        }
        // Ambiguous: walk the GV1 reading and see whether it is legal.
        let mut cur = first + 1;
        for _ in 0..entry_count {
            if read_byte(mem, cur)? > 6 {
                return Ok(GrammarFormat::InformGv2);
            }
            cur += 1;
            for _ in 0..6 {
                let v = read_byte(mem, cur)?;
                cur += 1;
                if (9..=15).contains(&v) || (112..=127).contains(&v) {
                    return Ok(GrammarFormat::InformGv2);
                }
            }
            cur += 1; // action byte, unconstrained
        }
        Ok(GrammarFormat::InformGv1)
    } else if span % 8 == 1 {
        Ok(GrammarFormat::InformGv1)
    } else {
        Err(GrammarError::BadTableSize)
    }
}

/// Advance `cur` past one syntax line without decoding it, reporting the action
/// number and the highest GV1 parsing-routine number it names.
fn measure_classic_line(
    mem: &Memory,
    format: GrammarFormat,
    cur: &mut u32,
) -> Result<(u16, u32), GrammarError> {
    match format {
        GrammarFormat::InfocomFixed => {
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok((action as u16, 0))
        }
        GrammarFormat::InfocomVariable => {
            let b0 = read_byte(mem, *cur)?;
            let action = read_byte(mem, *cur + 1)?;
            *cur += variable_line_size(b0)?;
            Ok((action as u16, 0))
        }
        GrammarFormat::Inform5 | GrammarFormat::InformGv1 => {
            let mut parse_max = 0;
            for j in 1..7 {
                let v = read_byte(mem, *cur + j)?;
                if (16..112).contains(&v) {
                    parse_max = parse_max.max((v as u32 - 16) % 32 + 1);
                }
            }
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok((action as u16, parse_max))
        }
        GrammarFormat::InformGv2 => {
            let action = read_word(mem, *cur)? & 0x03FF;
            *cur += 2;
            let mut guard = 0;
            loop {
                let t = read_byte(mem, *cur)?;
                *cur += 1;
                if t == ENDIT {
                    break;
                }
                *cur += 2;
                guard += 1;
                if guard > 64 {
                    return Err(GrammarError::BadSyntaxLine);
                }
            }
            Ok((action, 0))
        }
        GrammarFormat::InfocomV6 => unreachable!("V6 has no pointer table"),
    }
}

/// Byte length of an Infocom variable-form line, from the object count in the
/// top two bits of its first byte (`verb_sizes` in `showverb.c`).
fn variable_line_size(b0: u8) -> Result<u32, GrammarError> {
    match (b0 >> 6) & 0x03 {
        0 => Ok(2),
        1 => Ok(4),
        2 => Ok(7),
        _ => Err(GrammarError::BadSyntaxLine),
    }
}

fn read_classic_line(
    mem: &Memory,
    layout: &ClassicLayout,
    preps: &BTreeMap<u16, String>,
    cur: &mut u32,
) -> Result<SyntaxLine, GrammarError> {
    match layout.format {
        GrammarFormat::InfocomFixed => {
            let objs = read_byte(mem, *cur)?;
            let p0 = read_byte(mem, *cur + 1)?;
            let p1 = read_byte(mem, *cur + 2)?;
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            infocom_line(objs, prep_index(p0), prep_index(p1), action, preps)
        }
        GrammarFormat::InfocomVariable => {
            let b0 = read_byte(mem, *cur)?;
            let action = read_byte(mem, *cur + 1)?;
            let size = variable_line_size(b0)?;
            let objs = (b0 >> 6) & 0x03;
            // The first preposition's low six bits are packed into byte 0; the
            // top two bits of a preposition index are always set.
            let p0 = if b0 & 0x3F != 0 { Some(b0 as u16 | 0xC0) } else { None };
            let p1 = if objs > 1 {
                let b4 = read_byte(mem, *cur + 4)?;
                if b4 & 0x3F != 0 {
                    Some(b4 as u16 | 0xC0)
                } else {
                    None
                }
            } else {
                None
            };
            *cur += size;
            infocom_line(objs, p0, p1, action, preps)
        }
        GrammarFormat::Inform5 | GrammarFormat::InformGv1 => {
            let mut params = read_byte(mem, *cur)?;
            let mut slots = Vec::new();
            for j in 1..7u32 {
                let v = read_byte(mem, *cur + j)?;
                if v >= 0xB0 {
                    let word = preps.get(&(v as u16)).cloned().unwrap_or_default();
                    slots.push(Slot::one(Token::Word(word)));
                    continue;
                }
                if v == 0 && params == 0 {
                    break; // trailing null padding
                }
                slots.push(Slot::one(gv1_token(v)?));
                params = params.saturating_sub(1);
            }
            let action = read_byte(mem, *cur + 7)?;
            *cur += 8;
            Ok(SyntaxLine { action: action as u16, reverse: false, slots })
        }
        GrammarFormat::InformGv2 => {
            let head = read_word(mem, *cur)?;
            *cur += 2;
            let action = head & 0x03FF;
            let reverse = head & 0x0400 != 0;
            let mut slots: Vec<Slot> = Vec::new();
            loop {
                let t = read_byte(mem, *cur)?;
                *cur += 1;
                if t == ENDIT {
                    break;
                }
                let data = read_word(mem, *cur)?;
                *cur += 2;
                let token = gv2_token(mem, t & 0x0F, data)?;
                // Bits 4–5: $$10 opens a list of alternatives, $$01 continues one.
                let continues = (t >> 4) & 0x01 != 0;
                match (continues, slots.last_mut()) {
                    (true, Some(last)) => last.alternatives.push(token),
                    _ => slots.push(Slot::one(token)),
                }
                if slots.len() > 64 {
                    return Err(GrammarError::BadSyntaxLine);
                }
            }
            Ok(SyntaxLine { action, reverse, slots })
        }
        GrammarFormat::InfocomV6 => unreachable!("V6 has no pointer table"),
    }
}

/// A preposition byte counts as one only with its top bit set (`showverb.c`
/// checks `>= 0x80` in the fixed form).
fn prep_index(b: u8) -> Option<u16> {
    (b >= 0x80).then_some(b as u16)
}

/// Assemble an Infocom line: `verb [prep] [noun] [prep] [noun]`, exactly the
/// order `showverb.c` prints.
fn infocom_line(
    objs: u8,
    p0: Option<u16>,
    p1: Option<u16>,
    action: u8,
    preps: &BTreeMap<u16, String>,
) -> Result<SyntaxLine, GrammarError> {
    if objs > 2 {
        return Err(GrammarError::BadSyntaxLine);
    }
    let mut slots = Vec::new();
    for (i, p) in [p0, p1].into_iter().enumerate() {
        if let Some(idx) = p {
            slots.push(Slot::one(Token::Word(preps.get(&idx).cloned().unwrap_or_default())));
        }
        if objs as usize > i {
            slots.push(Slot::one(Token::Noun(NounKind::Noun)));
        }
    }
    Ok(SyntaxLine { action: action as u16, reverse: false, slots })
}

/// GV1 token byte → token (Inform Technical Manual §8.6).
fn gv1_token(v: u8) -> Result<Token, GrammarError> {
    Ok(match v {
        0..=8 => Token::Noun(NounKind::from_elementary(v as u16).expect("0..=8 is elementary")),
        16..=47 => Token::FilteredNoun(RoutineRef::Index(v - 16)),
        48..=79 => Token::Routine(RoutineRef::Index(v - 48)),
        80..=111 => Token::Scope(RoutineRef::Index(v - 80)),
        128..=175 => Token::Attribute(v - 128),
        // 9–15 and 112–127 are listed as illegal; 176+ was handled as a
        // preposition before reaching here.
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

/// GV2 token type and data → token (Inform Technical Manual §8.6).
fn gv2_token(mem: &Memory, ty: u8, data: u16) -> Result<Token, GrammarError> {
    Ok(match ty {
        1 => Token::Noun(NounKind::from_elementary(data).ok_or(GrammarError::BadSyntaxLine)?),
        2 => Token::Word(dict_text(mem, data as u32)?),
        3 => Token::FilteredNoun(RoutineRef::Packed(data)),
        4 => Token::Attribute(data as u8),
        5 => Token::Scope(RoutineRef::Packed(data)),
        6 => Token::Routine(RoutineRef::Packed(data)),
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

/// The preposition ("adjectives") table: a count word, then entries pairing a
/// dictionary address with an index. Inform stores them lowest index first;
/// Infocom's order varies. Either way we only ever look entries up by index.
fn read_preposition_table(
    mem: &Memory,
    layout: &ClassicLayout,
) -> Result<BTreeMap<u16, String>, GrammarError> {
    let mut map = BTreeMap::new();
    if layout.prep_table_base == 0 {
        return Ok(map); // GV2
    }
    let mut cur = layout.prep_table_base;
    let count = read_word(mem, cur)?;
    cur += 2;
    if count as u32 > 512 {
        return Err(GrammarError::BadVerbTable);
    }
    for _ in 0..count {
        let addr = read_word(mem, cur)? as u32;
        cur += 2;
        let index = if layout.prep_entry_form == 0 {
            let w = read_word(mem, cur)?;
            cur += 2;
            w
        } else {
            let b = read_byte(mem, cur)?;
            cur += 1;
            b as u16
        };
        if addr != 0 {
            map.entry(index).or_insert(dict_text(mem, addr)?);
        }
    }
    Ok(map)
}

// ── Infocom's Version 6 shape ────────────────────────────────────────────────

/// Zork Zero, Shogun and Arthur. There is no pointer table: each verb's
/// dictionary entry carries the address of an 8-byte record, and the one- and
/// two-object sentence shapes live in separate blocks that record points at.
/// Layout from `showverb.c`'s commentary (Matthew T. Russotto).
fn load_v6(mem: &Memory, words: &[DictWord]) -> Result<(Vec<Verb>, Vec<u32>), GrammarError> {
    let objects = mem.object_table() as u32;
    if objects < 4 {
        return Err(GrammarError::BadVerbTable);
    }
    // The action and pre-action table addresses sit in the last two globals,
    // immediately below the object table.
    let action_table_base = read_word(mem, objects - 4)? as u32;

    // Bound the verb-record area from the dictionary words that can only be
    // verbs. A word carrying the noun bit as well is a weaker witness — its
    // data word might be a property, not a record address — so it is admitted
    // as a spelling below but never allowed to move the bounds.
    let mut lo = u32::MAX;
    let mut hi = 0u32;
    for w in words {
        if let Some(addr) = w.verb_key {
            if w.roles.noun || (addr as u32) >= action_table_base {
                continue;
            }
            lo = lo.min(addr as u32);
            hi = hi.max(addr as u32 + 8);
        }
    }
    if hi == 0 {
        return Err(GrammarError::Absent);
    }
    if !(hi - lo).is_multiple_of(8) || (hi - lo) / 8 > MAX_VERBS {
        return Err(GrammarError::BadVerbTable);
    }

    // Now attach every spelling that lands on a record inside those bounds.
    let mut spellings: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for w in words {
        if let Some(addr) = w.verb_key {
            let a = addr as u32;
            if a >= lo && a < hi && (a - lo).is_multiple_of(8) {
                spellings.entry(addr).or_default().push(w.text.clone());
            }
        }
    }

    let mut verbs = Vec::new();
    let mut addr = lo;
    while addr < hi {
        let bare_action = read_word(mem, addr)?;
        let one_obj = read_word(mem, addr + 4)? as u32;
        let two_obj = read_word(mem, addr + 6)? as u32;

        let mut lines = Vec::new();
        if bare_action != 0xFFFF {
            lines.push(SyntaxLine { action: bare_action, reverse: false, slots: Vec::new() });
        }
        if one_obj != 0 {
            lines.extend(read_v6_block(mem, one_obj, 1)?);
        }
        if two_obj != 0 {
            lines.extend(read_v6_block(mem, two_obj, 2)?);
        }

        verbs.push(Verb {
            number: addr as u16,
            words: spellings.remove(&(addr as u16)).unwrap_or_default(),
            lines,
        });
        addr += 8;
    }

    // The V6 action table's extent is not derivable the way the classic one's
    // is; report none rather than a guess.
    Ok((verbs, Vec::new()))
}

/// One block of Version 6 sentence entries: a count word, then that many
/// entries of `2 + 4 * objects` bytes — an action word, then per object a
/// preposition's dictionary address and an attribute/selector pair.
fn read_v6_block(mem: &Memory, base: u32, objects: u32) -> Result<Vec<SyntaxLine>, GrammarError> {
    let count = read_word(mem, base)?;
    if count as u32 > 256 {
        return Err(GrammarError::BadSyntaxLine);
    }
    let stride = 2 + 4 * objects;
    let mut lines = Vec::with_capacity(count as usize);
    for i in 0..count as u32 {
        let mut cur = base + 2 + i * stride;
        let action = read_word(mem, cur)?;
        cur += 2;
        let mut slots = Vec::new();
        for _ in 0..objects {
            let prep = read_word(mem, cur)? as u32;
            cur += 2;
            let attribute = read_byte(mem, cur)?;
            let selector = read_byte(mem, cur + 1)?;
            cur += 2;
            if prep != 0 {
                slots.push(Slot::one(Token::Word(dict_text(mem, prep)?)));
            }
            slots.push(Slot::one(Token::InfocomObject { attribute, selector }));
        }
        lines.push(SyntaxLine { action, reverse: false, slots });
    }
    Ok(lines)
}

// ── Bounds-checked reads ─────────────────────────────────────────────────────
//
// `Memory::read_byte` latches a memory fault and returns 0 past the end of the
// image, which would turn a corrupt table into a plausible one and disturb the
// interpreter's own fault reporting. A read-only parser checks its own bounds.

fn read_byte(mem: &Memory, addr: u32) -> Result<u8, GrammarError> {
    if addr as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    Ok(mem.read_byte(addr))
}

fn read_word(mem: &Memory, addr: u32) -> Result<u16, GrammarError> {
    if addr as usize + 1 >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    Ok(mem.read_word(addr))
}

/// Decode the dictionary word stored at `addr`.
fn dict_text(mem: &Memory, addr: u32) -> Result<String, GrammarError> {
    if addr as usize >= mem.len() {
        return Err(GrammarError::Truncated);
    }
    let (s, _) = decode_string(mem, addr);
    Ok(s.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::text::encode::encode_word;

    const BASE: u32 = 0x0400; // static memory base in `sample_story`
    const DICT: u32 = 0x0200; // dictionary base in `sample_story`

    /// A hand-built story image: header, dictionary and grammar tables written
    /// byte by byte, so each format's layout is asserted against an oracle we
    /// control rather than against the implementation's own assumptions.
    struct Story {
        buf: Vec<u8>,
        version: u8,
        /// Byte address of each dictionary entry, in the order given.
        dict_addrs: Vec<u32>,
    }

    impl Story {
        /// `serial` picks the family: an Infocom serial starts with '8'.
        fn new(version: u8, serial: &[u8; 6], inform6: bool) -> Story {
            let mut buf = sample_story(version);
            buf.resize(0x1000, 0);
            buf[0x12..0x18].copy_from_slice(serial);
            if inform6 {
                buf[0x3C..0x40].copy_from_slice(b"6.15");
            }
            Story { buf, version, dict_addrs: Vec::new() }
        }

        fn b(&mut self, addr: u32, v: u8) {
            self.buf[addr as usize] = v;
        }

        fn w(&mut self, addr: u32, v: u16) {
            self.buf[addr as usize] = (v >> 8) as u8;
            self.buf[addr as usize + 1] = v as u8;
        }

        /// Write a dictionary of `(word, flags, [d0, d1, d2])` entries.
        fn dictionary(&mut self, entries: &[(&str, u8, [u8; 3])]) {
            let klen: u32 = if self.version <= 3 { 4 } else { 6 };
            let elen = klen + 3;
            let mut cur = DICT;
            self.b(cur, 0); // no word separators
            cur += 1;
            self.b(cur, elen as u8);
            cur += 1;
            self.w(cur, entries.len() as u16);
            cur += 2;
            for (word, flags, data) in entries {
                let key = encode_word(word, self.version);
                for (i, k) in key.iter().take(klen as usize).enumerate() {
                    self.b(cur + i as u32, *k);
                }
                self.b(cur + klen, *flags);
                self.b(cur + klen + 1, data[0]);
                self.b(cur + klen + 2, data[1]);
                self.b(cur + klen + 3, data[2]);
                self.dict_addrs.push(cur);
                cur += elen;
            }
        }

        /// Lay out the grammar area: pointer table, one blob per verb, then the
        /// action, pre-action and preposition tables at the addresses the
        /// format says they follow at.
        fn grammar(
            &mut self,
            blobs: &[Vec<u8>],
            action_count: u32,
            second_table_count: u32,
            preps: &[(u32, u16)],
        ) {
            let n = blobs.len() as u32;
            let mut cur = BASE + n * 2;
            for (i, blob) in blobs.iter().enumerate() {
                self.w(BASE + i as u32 * 2, cur as u16);
                for (j, byte) in blob.iter().enumerate() {
                    self.b(cur + j as u32, *byte);
                }
                cur += blob.len() as u32;
            }
            // Action table: any nonzero packed address will do.
            for i in 0..action_count {
                self.w(cur + i * 2, 0x0101);
            }
            cur += action_count * 2;
            // Pre-action / parsing-routine table.
            for i in 0..second_table_count {
                self.w(cur + i * 2, 0x0202);
            }
            cur += second_table_count * 2;
            // Preposition table, word-index form.
            self.w(cur, preps.len() as u16);
            cur += 2;
            for (addr, index) in preps {
                self.w(cur, *addr as u16);
                self.w(cur + 2, *index);
                cur += 4;
            }
        }

        fn mem(&self) -> Memory {
            Memory::new(self.buf.clone()).expect("sample story is valid")
        }

        fn grammar_of(&self) -> Result<Grammar, GrammarError> {
            Grammar::load(&self.mem())
        }

        /// The refusal, for the falsification cases. `Grammar` itself is not
        /// comparable, so the error is compared on its own.
        fn error(&self) -> Option<GrammarError> {
            self.grammar_of().err()
        }
    }

    // ── Infocom, fixed 8-byte lines ──────────────────────────────────────────

    /// "take OBJ", "take OBJ with OBJ", "open OBJ".
    fn infocom_fixed_story() -> Story {
        let mut s = Story::new(3, b"840726", false);
        // Infocom flags: VERB ($40) with DATA_FIRST = VERB_FIRST ($01), so the
        // verb number is the first data byte.
        s.dictionary(&[
            ("take", 0x41, [255, 0, 0]),
            ("grab", 0x41, [255, 0, 0]),
            ("open", 0x41, [254, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
            ("lamp", 0x80, [0, 0, 0]),
        ]);
        let take = vec![
            2, // two lines
            1, 0x00, 0x00, 0, 0, 0, 0, 3, // "take OBJ", action 3
            2, 0x00, 0xF0, 0, 0, 0, 0, 4, // "take OBJ with OBJ", action 4
        ];
        let open = vec![1, 1, 0x00, 0x00, 0, 0, 0, 0, 5];
        let with_addr = s.dict_addrs[3];
        s.grammar(&[take, open], 6, 6, &[(with_addr, 0xF0)]);
        s
    }

    #[test]
    fn infocom_fixed_tables_decode() {
        let g = infocom_fixed_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InfocomFixed);
        assert_eq!(g.verbs().len(), 2);

        let take = g.verb_for_word("take").unwrap();
        assert_eq!(take.number, 255);
        assert_eq!(take.words, vec!["take".to_string(), "grab".to_string()]);
        assert_eq!(take.lines.len(), 2);
        assert_eq!(take.lines[0].describe("take"), "take noun");
        assert_eq!(take.lines[1].describe("take"), "take noun with noun");
        assert_eq!(take.lines[1].action, 4);

        // The shape query SQ-1041 needs: "with" is legal here, "from" is not.
        assert!(take.accepts(2, &["with"]));
        assert!(!take.accepts(2, &["from"]));
        assert!(take.accepts(1, &[]));
        assert!(!take.takes_bare());

        // A synonym resolves to the same verb.
        assert_eq!(g.verb_for_word("grab").unwrap().number, 255);
        assert!(g.is_verb("open"));
        assert!(!g.is_verb("lamp"));
        assert_eq!(g.prepositions(), ["with".to_string()]);
        assert!(g.is_preposition("with"));
    }

    // ── Infocom, variable 2/4/7-byte lines ───────────────────────────────────

    /// "hop", "pull up OBJ", "unlock OBJ with OBJ".
    fn infocom_variable_story() -> Story {
        let mut s = Story::new(3, b"871124", false);
        s.dictionary(&[
            ("hop", 0x41, [255, 0, 0]),
            ("pull", 0x41, [254, 0, 0]),
            ("unlock", 0x41, [253, 0, 0]),
            ("up", 0x08, [0, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
        ]);
        // Byte 0 packs the object count into the top two bits and the first
        // preposition's low six bits into the rest.
        let hop = vec![1, 0x00, 7]; // 0 objects, action 7
        let pull = vec![1, 0x7C, 8, 0, 0]; // 1 object, prep $FC "up", action 8
        let unlock = vec![1, 0x80, 9, 0, 0, 0x3E, 0, 0]; // 2 objects, prep $FE
        let up = s.dict_addrs[3];
        let with = s.dict_addrs[4];
        s.grammar(&[hop, pull, unlock], 10, 10, &[(up, 0xFC), (with, 0xFE)]);
        s
    }

    #[test]
    fn infocom_variable_tables_decode() {
        let g = infocom_variable_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InfocomVariable);
        assert_eq!(g.verbs().len(), 3);

        assert_eq!(g.verb_for_word("hop").unwrap().lines[0].describe("hop"), "hop");
        assert!(g.verb_for_word("hop").unwrap().takes_bare());

        let pull = g.verb_for_word("pull").unwrap();
        assert_eq!(pull.lines[0].describe("pull"), "pull up noun");
        assert!(pull.accepts(1, &["up"]));

        let unlock = g.verb_for_word("unlock").unwrap();
        assert_eq!(unlock.lines[0].describe("unlock"), "unlock noun with noun");
        assert_eq!(unlock.max_nouns(), 2);
        assert_eq!(unlock.prepositions(), vec!["with"]);
    }

    // ── Inform 6, grammar version 1 ──────────────────────────────────────────

    /// One verb with three lines covering every GV1 token class.
    fn gv1_story() -> Story {
        let mut s = Story::new(5, b"961202", true);
        s.dictionary(&[
            ("take", 0x01, [255, 0, 0]),
            ("put", 0x01, [254, 0, 0]),
            ("in", 0x08, [0, 0, 0xFF]),
        ]);
        // 8 bytes each: [parameter count][6 tokens][action].
        // Four lines, so this verb's data is 33 bytes. The length matters:
        // `classify_inform6` only reaches its ambiguous branch when the data is
        // 1 mod 24, and the reference implementation misreads that branch (see
        // the note there), so a table meant as a shared oracle stays out of it.
        let take = vec![
            4, // four lines
            1, 0x00, 0, 0, 0, 0, 0, 20, // "take noun"
            2, 0x03, 0xFF, 0x01, 0, 0, 0, 21, // "take multiheld in held"
            2, 0x91, 0x35, 0, 0, 0, 0, 22, // ATTRIBUTE(17) then TEXT [parse 5]
            1, 0x14, 0, 0, 0, 0, 0, 24, // NOUN [parse 4]
        ];
        let put = vec![1, 1, 0x56, 0, 0, 0, 0, 0, 23]; // SCOPE [parse 6]
        let in_addr = s.dict_addrs[2];
        s.grammar(&[take, put], 25, 7, &[(in_addr, 0xFF)]);
        s
    }

    #[test]
    fn inform_gv1_tokens_decode() {
        let g = gv1_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InformGv1);

        let take = g.verb_for_word("take").unwrap();
        assert_eq!(take.lines.len(), 4);
        assert_eq!(take.lines[0].slots, vec![Slot::one(Token::Noun(NounKind::Noun))]);
        assert_eq!(
            take.lines[1].slots,
            vec![
                Slot::one(Token::Noun(NounKind::MultiHeld)),
                Slot::one(Token::Word("in".into())),
                Slot::one(Token::Noun(NounKind::Held)),
            ]
        );
        assert!(take.accepts(2, &["in"]));
        assert_eq!(
            take.lines[2].slots,
            vec![
                Slot::one(Token::Attribute(17)),
                Slot::one(Token::Routine(RoutineRef::Index(5))),
            ]
        );
        assert_eq!(
            take.lines[3].slots,
            vec![Slot::one(Token::FilteredNoun(RoutineRef::Index(4)))]
        );

        let put = g.verb_for_word("put").unwrap();
        assert_eq!(put.lines[0].slots, vec![Slot::one(Token::Scope(RoutineRef::Index(6)))]);
        assert_eq!(put.lines[0].action, 23);
    }

    // ── Inform 6, grammar version 2 ──────────────────────────────────────────

    /// Two lines: one with an alternative list and the reverse bit, one with
    /// a packed-address routine token.
    fn gv2_story() -> Story {
        let mut s = Story::new(5, b"981115", true);
        s.dictionary(&[
            ("get", 0x01, [255, 0, 0]),
            ("in", 0x08, [0, 0, 0]),
            ("into", 0x08, [0, 0, 0]),
        ]);
        let in_addr = s.dict_addrs[1] as u16;
        let into_addr = s.dict_addrs[2] as u16;
        let mut get: Vec<u8> = vec![2];
        // Line 1: action 30 with the $400 reverse bit; 'in' / 'into', then noun.
        get.extend_from_slice(&[0x04, 0x1E]);
        get.extend_from_slice(&[0x62, (in_addr >> 8) as u8, in_addr as u8]); // $$10 alt open
        get.extend_from_slice(&[0x52, (into_addr >> 8) as u8, into_addr as u8]); // $$01 continue
        get.extend_from_slice(&[0x01, 0x00, 0x00]); // elementary "noun"
        get.push(ENDIT);
        // Line 2: action 31, scope routine at packed $1234, attribute 9.
        get.extend_from_slice(&[0x00, 0x1F]);
        get.extend_from_slice(&[0x85, 0x12, 0x34]);
        get.extend_from_slice(&[0x04, 0x00, 0x09]);
        get.push(ENDIT);
        // A second verb: GV1 and GV2 are told apart by the length of the first
        // verb's data, which needs a second pointer to bound it.
        let drop = vec![1, 0x00, 0x28, 0x01, 0x00, 0x02, ENDIT];
        s.grammar(&[get, drop], 42, 0, &[]);
        s
    }

    #[test]
    fn inform_gv2_tokens_decode() {
        let g = gv2_story().grammar_of().unwrap();
        assert_eq!(g.format(), GrammarFormat::InformGv2);

        let get = g.verb_for_word("get").unwrap();
        assert_eq!(get.lines.len(), 2);

        let first = &get.lines[0];
        assert_eq!(first.action, 30);
        assert!(first.reverse);
        assert_eq!(first.slots.len(), 2);
        assert_eq!(
            first.slots[0].alternatives,
            vec![Token::Word("in".into()), Token::Word("into".into())]
        );
        assert!(first.slots[0].only().is_none());
        assert_eq!(first.slots[1].alternatives, vec![Token::Noun(NounKind::Noun)]);
        assert_eq!(first.noun_count(), 1);
        // Either spelling of the preposition satisfies the same slot.
        assert!(first.accepts(1, &["in"]));
        assert!(first.accepts(1, &["into"]));
        assert!(!first.accepts(1, &["on"]));

        let second = &get.lines[1];
        assert!(!second.reverse);
        assert_eq!(second.action, 31);
        assert_eq!(
            second.slots,
            vec![
                Slot::one(Token::Scope(RoutineRef::Packed(0x1234))),
                Slot::one(Token::Attribute(9)),
            ]
        );
    }

    // ── Parts of speech ──────────────────────────────────────────────────────

    #[test]
    fn roles_report_infocom_parts_of_speech() {
        let mut s = Story::new(3, b"840726", false);
        s.dictionary(&[
            ("take", 0x41, [255, 0, 0]),
            ("brass", 0x20, [0, 0, 0]),
            ("lamp", 0x80, [0, 0, 0]),
            ("with", 0x08, [0, 0, 0]),
            ("north", 0x04, [0, 0, 0]),
        ]);
        s.grammar(&[vec![1, 1, 0, 0, 0, 0, 0, 0, 1]], 2, 2, &[]);
        let g = s.grammar_of().unwrap();

        assert!(g.roles("take").unwrap().verb);
        assert!(g.roles("brass").unwrap().adjective);
        assert!(g.roles("lamp").unwrap().noun);
        assert!(!g.roles("lamp").unwrap().verb);
        assert!(g.roles("with").unwrap().preposition);
        assert!(g.roles("north").unwrap().special);
        assert_eq!(g.roles("nosuchword"), None);
    }

    #[test]
    fn roles_report_inform_parts_of_speech() {
        let mut s = gv2_story();
        // Add a meta verb and a plural noun to the dictionary.
        s.dictionary(&[
            ("get", 0x01, [255, 0, 0]),
            ("in", 0x08, [0, 0, 0]),
            ("into", 0x08, [0, 0, 0]),
            ("save", 0x03, [254, 0, 0]),
            ("coins", 0x84, [0, 0, 0]),
        ]);
        let g = s.grammar_of().unwrap();
        assert!(g.roles("save").unwrap().meta);
        assert!(g.roles("save").unwrap().verb);
        assert!(g.roles("coins").unwrap().plural);
        assert!(g.roles("coins").unwrap().noun);
        assert!(g.roles("in").unwrap().preposition);
        // Inform has no separate adjective bit.
        assert!(!g.roles("in").unwrap().adjective);
    }

    #[test]
    fn verbs_accepting_filters_by_shape() {
        let g = infocom_fixed_story().grammar_of().unwrap();
        let with: Vec<&str> =
            g.verbs_accepting(2, &["with"]).iter().filter_map(|v| v.word()).collect();
        assert_eq!(with, vec!["take"]);
        let bare: Vec<&str> =
            g.verbs_accepting(1, &[]).iter().filter_map(|v| v.word()).collect();
        assert_eq!(bare, vec!["take", "open"]);
        assert!(g.verbs_accepting(2, &["from"]).is_empty());
    }

    // ── Falsification ────────────────────────────────────────────────────────
    //
    // Each case corrupts exactly one byte of a table that parses cleanly above.
    // A grammar parser that answers plausibly here is worse than one that
    // fails: its consumer has no way to tell the difference.

    #[test]
    fn refuses_object_count_above_two() {
        let mut s = infocom_fixed_story();
        // First line's object count: 1 → 3.
        s.b(BASE + 4 + 1, 3);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_variable_line_with_three_objects() {
        let mut s = infocom_variable_story();
        // "pull"'s byte 0 is $7C; $FC sets the object count to 3, which
        // `verb_sizes` has no length for.
        s.b(BASE + 6 + 3 + 1, 0xFC);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_illegal_gv1_token() {
        let mut s = gv1_story();
        // First line's first token: 0 (noun) → 10, in the illegal 9–15 range.
        s.b(BASE + 4 + 2, 10);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_unknown_gv2_token_type() {
        let mut s = gv2_story();
        // The first token's type nibble: 2 (preposition) → 7, which no version
        // of the format defines.
        s.b(BASE + 4 + 3, 0x67);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_gv2_line_that_never_ends() {
        let mut s = gv2_story();
        // Overwrite the first line's ENDIT with another token type.
        s.b(BASE + 4 + 12, 0x01);
        assert!(matches!(
            s.error(),
            Some(GrammarError::BadSyntaxLine) | Some(GrammarError::Truncated)
        ));
    }

    #[test]
    fn refuses_backwards_verb_pointer() {
        let mut s = infocom_fixed_story();
        s.w(BASE, 0x0300); // points below the table it heads
        assert_eq!(s.error(), Some(GrammarError::BadVerbTable));
    }

    #[test]
    fn refuses_verb_entry_past_end_of_story() {
        let mut s = infocom_fixed_story();
        s.w(BASE + 2, 0x0FFF); // second verb's data starts at the last byte
        assert_eq!(s.error(), Some(GrammarError::Truncated));
    }

    #[test]
    fn reports_absent_rather_than_guessing_when_there_is_no_table() {
        let mut s = infocom_fixed_story();
        s.w(BASE, 0);
        assert_eq!(s.error(), Some(GrammarError::Absent));
    }

    #[test]
    fn refuses_absurd_line_count() {
        let mut s = infocom_fixed_story();
        // The first verb's line count: 2 → 200, far past what either format
        // allows and enough to walk the parser off the end of the tables.
        s.b(BASE + 4, 200);
        assert!(matches!(
            s.error(),
            Some(GrammarError::BadVerbTable) | Some(GrammarError::Truncated)
        ));
    }
}
