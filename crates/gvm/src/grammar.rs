// Inform grammar (syntax) tables in a Glulx image — which verbs the story
// knows, and what sentence shapes each of them accepts.
//
// ── Where the format is specified ────────────────────────────────────────────
//
// The Glulx specification describes the virtual machine and says nothing about
// grammar: these tables are Inform's, not Glulx's. Two authoritative sources,
// both consulted directly rather than recalled:
//
//   * **"The Glulx Inform Technical Reference"**, Andrew Plotkin — §4 "The
//     Dictionary", §6 "Grammar Table", §7 "Actions Table". This is the Glulx
//     counterpart of the Inform Technical Manual's §8.6, written by the person
//     who designed the layout.
//     <https://eblong.com/zarf/glulx/Glulx-Inform-Tech.html>
//
//   * **The Inform 6 compiler itself** — `tables.c::construct_storyfile_g`,
//     which emits the tables in the order this module relies on, `verbs.c` for
//     the `/`-alternation bits ($20 on the token before a slash, $10 on the
//     token after one), and `text.c` for the dictionary record's shape and
//     `header.h` for the `*_DFLAG` flag bits.
//     <https://github.com/DavidKinder/Inform6>
//
// Cross-checked against `glulxdump` (Andrew Plotkin, shipped in the Glulxe
// source tree), which dumps the same tables when handed their address.
//
// ── The layout ───────────────────────────────────────────────────────────────
//
//   grammar table   long   number of verbs
//                   long   address of this verb's lines     × that many
//
//   per verb        byte   number of lines
//                   per line:
//                     short  action number
//                     byte   flags ($01 = swap noun and second)
//                     per token:
//                       byte  token type
//                       long  token data
//                     byte   ENDIT (15)
//
//   actions table   long   number of actions
//                   long   address of the action's routine  × that many
//
//   dictionary      long   number of words
//                   per word:
//                     byte     $60 (type tag for a dictionary word)
//                     byte[W]  lower-case text, zero-padded (W = DICT_WORD_SIZE)
//                     short    flags
//                     short    verb number
//                     short    unused
//
// Plotkin: "This is nearly identical to the grammar version 2 format in
// Z-machine Inform. The only differences are that the token data is 4 bytes
// long, and the switch flag is no longer stuck in the action number." Token
// type bytes carry the same three fields as GV2 — top two bits the data kind,
// next two the `/`-alternation state, bottom four the type.
//
// ── The hard part is not the format; it is finding the tables ────────────────
//
// **A Glulx image records the grammar table's address nowhere.** On the
// Z-machine, header word $0E points at it (`zvm::grammar` relies on that). The
// Glulx header names RAMSTART, EXTSTART, ENDMEM, the start function and the
// string-decoding table, and nothing else; Inform's own 24-byte block after it
// holds a layout tag, two version strings, a release number and a serial, and
// no table addresses at all (`Inform6/src/files.c`, `GLULX_STATIC_ROM_SIZE`).
//
// This is not an oversight we can route around: `glulxdump` — written by the
// designer of both Glulx and this layout — requires the address on the command
// line (`-g <addr>`), and its header comment says so outright: "This whole
// situation could be improved by adding a 'layout convention' field, at the
// start of ROM, which could contain compiler-specific information about how to
// decompile the file. Maybe someday."
//
// So the tables are *derived*, by a chain that is verified end to end rather
// than guessed. Inform emits grammar, actions and dictionary contiguously and
// in that order, and each is self-describing:
//
//   1. **The dictionary** is found first, because it has the strongest
//      signature in the image: a run of records at a constant stride, each
//      beginning with the byte $60, whose length equals the count word
//      immediately before the run. Nothing else in memory looks like that.
//   2. **The actions table** ends exactly where the dictionary begins (Inform
//      inserts up to three bytes of alignment padding, and only for Unicode
//      dictionaries). Its own count word must agree with its length, and every
//      entry must be a plausible code address — below RAMSTART, since Glulx
//      Inform keeps all code and strings in ROM.
//   3. **The grammar table** ends exactly where the actions table begins, and
//      its first verb pointer must equal `base + 4 + 4 * verb_count` exactly.
//      Walking every verb, every line and every token of it must land on the
//      actions table's first byte and not one byte elsewhere.
//
// A candidate that satisfies all three is not a guess. The last step is the one
// that does the work, and it is worth being precise about how much: across the
// 22 Glulx stories in the local corpus, **889 byte offsets satisfy the
// pointer-array precondition alone** — 279 in one game — and **exactly 22
// survive the walk**, one per story. The walk is what discriminates; scanning
// backwards from the actions table merely means the right answer is usually the
// first one tried. Where nothing survives, this module refuses — see
// [`GrammarError`].

use std::collections::BTreeMap;

use crate::memory::Memory;

/// Inform's `*_DFLAG` dictionary flag bits (`Inform6/src/header.h`).
const VERB_DFLAG: u16 = 1;
const META_DFLAG: u16 = 2;
const PLURAL_DFLAG: u16 = 4;
const PREP_DFLAG: u16 = 8;
const SING_DFLAG: u16 = 16;
const TRUNC_DFLAG: u16 = 64;
const NOUN_DFLAG: u16 = 128;

/// The type tag every dictionary record begins with (Glulx Inform Tech. Ref. §4).
const DICT_TAG: u8 = 0x60;
/// End of a grammar line (Glulx Inform Tech. Ref. §6).
const ENDIT: u8 = 15;

/// Smallest dictionary this module will accept as a positive identification.
/// Real games run to hundreds of words; a shorter run is noise.
const MIN_DICT_WORDS: u32 = 16;
/// `1 + DICT_WORD_SIZE + 6` for a byte-valued dictionary. Inform's default word
/// size is 9 (stride 16); the corpus also contains 10 and 12 (17 and 19).
const DICT_STRIDE_RANGE: std::ops::RangeInclusive<u32> = 8..=80;
/// Sanity ceilings. The largest table seen in the corpus is Cragne Manor's 368
/// verbs and 375 actions; these are far above that and far below noise.
const MAX_VERBS: u32 = 20_000;
const MAX_ACTIONS: u32 = 8_000;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Why a Glulx story's grammar could not be read.
///
/// Refusing is the contract. Because the tables are located rather than looked
/// up, a reader that answered anyway would hand its consumer a confident
/// reading of the wrong bytes, and nothing downstream could tell.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GrammarError {
    /// No Inform tables in this image. A Glulx file need not be an Inform
    /// game at all, and even an Inform one need not have a parser —
    /// `glulxercise.ulx` is a VM conformance suite with a dictionary and no
    /// grammar.
    Absent,
    /// The dictionary was found but no actions table ends where it begins, or
    /// no grammar table ends where that begins. The chain could not be closed,
    /// so no address here is trustworthy.
    TablesNotFound,
    /// A table address or entry ran past the end of memory.
    Truncated,
    /// A grammar line held a value the format forbids — an unknown token type,
    /// an elementary token above 9, or a line that never reached its ENDIT.
    BadSyntaxLine,
    /// The dictionary is Unicode-valued (`$DICT_CHAR_SIZE=4`), whose record
    /// shape this module does not read. No story in the corpus uses one;
    /// refusing beats guessing at a layout never seen in practice.
    UnicodeDictionary,
}

// ── The value types ──────────────────────────────────────────────────────────
//
// Deliberately the same vocabulary as `zvm::grammar`, so that a consumer
// written against one reads the other with a rename. They are separate types
// rather than shared ones; see the note at the bottom of this file.

/// The parser's built-in noun slots (Glulx Inform Tech. Ref. §6, sharing
/// Z-machine GV2's elementary-token numbering).
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
    Topic,
}

impl NounKind {
    fn from_elementary(v: u32) -> Option<NounKind> {
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

    /// The name Inform uses for this slot.
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

/// One position in a syntax line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token {
    /// A noun phrase the player supplies.
    Noun(NounKind),
    /// A literal word the player must type — a preposition, in practice.
    Word(String),
    /// A noun slot the game filters with a routine (`noun = Routine`). Glulx
    /// addresses are plain, so this is the routine's address as written.
    FilteredNoun(u32),
    /// A slot parsed entirely by a game routine.
    Routine(u32),
    /// A slot whose scope a game routine decides (`scope = Routine`).
    Scope(u32),
    /// A noun slot restricted to objects holding an attribute.
    Attribute(u32),
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
/// Inform writes `'in' / 'into' / 'inside'` as one position with three
/// alternatives; a consumer that flattened them would report a sentence two
/// words longer than the story accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Slot {
    /// The alternatives, in table order; never empty.
    pub alternatives: Vec<Token>,
}

impl Slot {
    fn one(t: Token) -> Slot {
        Slot { alternatives: vec![t] }
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

/// One sentence shape a verb accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SyntaxLine {
    /// The action this line performs; indexes the actions table.
    pub action: u16,
    /// The line's flags byte, bit 0: swap `noun` and `second` when calling the
    /// action. Glulx moved this out of the action number, where Z-machine GV2
    /// keeps it as `$400`.
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
        self.slots.iter().flat_map(|s| s.alternatives.iter().filter_map(Token::word)).collect()
    }

    /// True if this line accepts `nouns` noun phrases with exactly `words` as
    /// its literal words, in order.
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

    /// A one-line rendering for debug inspectors and for diffing this module
    /// against `glulxdump`. Not player-facing text.
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
                    Token::FilteredNoun(a) => out.push_str(&format!("noun = [{a:#x}]")),
                    Token::Routine(a) => out.push_str(&format!("[{a:#x}]")),
                    Token::Scope(a) => out.push_str(&format!("scope = [{a:#x}]")),
                    Token::Attribute(a) => out.push_str(&format!("ATTRIBUTE({a})")),
                }
            }
        }
        if self.reverse {
            out.push_str(" REVERSE");
        }
        out
    }
}

/// One verb of the story's grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verb {
    /// The verb's index in the grammar table's pointer array. Dictionary
    /// records name a verb by this number.
    pub number: u32,
    /// Address of this verb's line block.
    pub address: u32,
    /// Every dictionary spelling of this verb, in dictionary order.
    pub words: Vec<String>,
    /// The sentence shapes, in table order.
    pub lines: Vec<SyntaxLine>,
}

impl Verb {
    /// The spelling to use when naming this verb; `None` for a verb slot no
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

    /// Every literal word any of this verb's lines uses, deduplicated.
    pub fn prepositions(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().flat_map(SyntaxLine::words).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// True if some line accepts `nouns` noun phrases with exactly `words` as
    /// its literal words: `put IN` yes, `put WITH` no.
    pub fn accepts(&self, nouns: usize, words: &[&str]) -> bool {
        self.lines.iter().any(|l| l.accepts(nouns, words))
    }
}

/// What parts of speech the dictionary marks a word with — Inform's `*_DFLAG`
/// bits (`Inform6/src/header.h`), which Glulx stores as a 16-bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct WordRoles {
    /// Bit 0: used as a verb in the grammar.
    pub verb: bool,
    /// Bit 1: a meta verb — a command to the interpreter, not the game.
    pub meta: bool,
    /// Bit 2: plural, declared with `//p`.
    pub plural: bool,
    /// Bit 3: used as a preposition in the grammar.
    pub preposition: bool,
    /// Bit 4: singular, declared with `//s`.
    pub singular: bool,
    /// Bit 6: the word was truncated to the dictionary's word length.
    pub truncated: bool,
    /// Bit 7: used as a noun.
    pub noun: bool,
    /// The flag field exactly as stored.
    pub raw: u16,
}

/// Where the three Inform tables were found, and how big each is.
///
/// Worth having on its own: no Glulx tool can be told "dump this game's
/// grammar" without these numbers, so they are the input `glulxdump -g` wants
/// and the thing to quote in any finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Tables {
    /// Address of the grammar table's verb count.
    pub grammar: u32,
    /// Number of verbs.
    pub verb_count: u32,
    /// Address of the actions table's count.
    pub actions: u32,
    /// Number of actions.
    pub action_count: u32,
    /// Address of the dictionary's word count.
    pub dictionary: u32,
    /// Number of dictionary words.
    pub word_count: u32,
    /// Bytes per dictionary record.
    pub dict_stride: u32,
    /// `DICT_WORD_SIZE` — characters of text per record.
    pub dict_word_size: u32,
}

/// A Glulx story's grammar: which words are verbs, and what each verb accepts.
///
/// Self-contained once loaded — no `&Memory` is needed to query it, so it can
/// be cached beside a session or handed to another thread.
#[derive(Debug, Clone)]
pub struct Grammar {
    tables: Tables,
    verbs: Vec<Verb>,
    by_word: BTreeMap<String, usize>,
    prepositions: Vec<String>,
    roles: BTreeMap<String, WordRoles>,
    action_routines: Vec<u32>,
}

impl Grammar {
    /// Locate and read the story's Inform tables.
    pub fn load(mem: &Memory) -> Result<Grammar, GrammarError> {
        let tables = locate(mem)?;
        let words = read_dictionary(mem, &tables)?;

        let mut roles = BTreeMap::new();
        let mut spellings: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for w in &words {
            roles.insert(w.text.clone(), w.roles);
            if w.roles.verb {
                spellings.entry(w.verb_number).or_default().push(w.text.clone());
            }
        }

        let mut verbs = Vec::with_capacity(tables.verb_count as usize);
        for i in 0..tables.verb_count {
            let address = read32(mem, tables.grammar + 4 + i * 4)?;
            let lines = read_verb_lines(mem, address, &words)?.1;
            verbs.push(Verb {
                number: i,
                address,
                words: spellings.remove(&i).unwrap_or_default(),
                lines,
            });
        }

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

        let mut action_routines = Vec::with_capacity(tables.action_count as usize);
        for i in 0..tables.action_count {
            action_routines.push(read32(mem, tables.actions + 4 + i * 4)?);
        }

        Ok(Grammar { tables, verbs, by_word, prepositions, roles, action_routines })
    }

    /// Where the tables were found. Also reachable without reading them, via
    /// [`locate`].
    pub fn tables(&self) -> Tables {
        self.tables
    }

    /// Every verb, in grammar-table order.
    pub fn verbs(&self) -> &[Verb] {
        &self.verbs
    }

    /// The verb a spelling belongs to, if it is one.
    pub fn verb_for_word(&self, word: &str) -> Option<&Verb> {
        self.by_word.get(&word.to_lowercase()).map(|&i| &self.verbs[i])
    }

    /// True if the story can begin a command with this word.
    pub fn is_verb(&self, word: &str) -> bool {
        self.by_word.contains_key(&word.to_lowercase())
    }

    /// Every spelling that can begin a command, sorted.
    pub fn verb_words(&self) -> impl Iterator<Item = &str> {
        self.by_word.keys().map(String::as_str)
    }

    /// Every literal word the grammar names, deduplicated and sorted.
    pub fn prepositions(&self) -> &[String] {
        &self.prepositions
    }

    /// True if the grammar uses this word literally in some line.
    pub fn is_preposition(&self, word: &str) -> bool {
        self.prepositions.binary_search(&word.to_lowercase()).is_ok()
    }

    /// The parts of speech the dictionary marks `word` with, if it knows it.
    pub fn roles(&self, word: &str) -> Option<WordRoles> {
        self.roles.get(&word.to_lowercase()).copied()
    }

    /// Every word the dictionary holds, sorted.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.roles.keys().map(String::as_str)
    }

    /// Addresses of the action routines, indexed by action number.
    pub fn action_routines(&self) -> &[u32] {
        &self.action_routines
    }

    /// Every verb with a line matching `nouns` noun phrases and exactly `words`
    /// as its literal words.
    pub fn verbs_accepting(&self, nouns: usize, words: &[&str]) -> Vec<&Verb> {
        self.verbs.iter().filter(|v| v.accepts(nouns, words)).collect()
    }
}

// ── Locating the tables ──────────────────────────────────────────────────────

/// Find the grammar, actions and dictionary tables without reading them.
///
/// The chain is described at the top of this file. Every step is checked
/// against the next, so a returned `Tables` is a reading no other arrangement
/// of the image's bytes can produce.
pub fn locate(mem: &Memory) -> Result<Tables, GrammarError> {
    let ram = mem.ramstart();
    let lim = mem.extstart();
    let (dictionary, word_count, dict_stride) = find_dictionary(mem, ram, lim)?;

    for (actions, action_count) in action_candidates(mem, dictionary, ram) {
        if let Some((grammar, verb_count)) = find_grammar(mem, ram, lim, actions) {
            return Ok(Tables {
                grammar,
                verb_count,
                actions,
                action_count,
                dictionary,
                word_count,
                dict_stride,
                dict_word_size: dict_stride - 7,
            });
        }
    }
    Err(GrammarError::TablesNotFound)
}

/// The longest run of `$60`-tagged records at a constant stride whose length
/// matches the count word immediately before it.
fn find_dictionary(mem: &Memory, ram: u32, lim: u32) -> Result<(u32, u32, u32), GrammarError> {
    let mut best: Option<(u32, u32, u32)> = None;
    let mut p = ram;
    while p < lim {
        if byte(mem, p) != DICT_TAG {
            p += 1;
            continue;
        }
        for stride in DICT_STRIDE_RANGE {
            // Only start a chain at its head, so each run is measured once.
            if p >= ram + stride && byte(mem, p - stride) == DICT_TAG {
                continue;
            }
            let mut n = 1u32;
            let mut q = p + stride;
            while q < lim && byte(mem, q) == DICT_TAG {
                n += 1;
                q += stride;
            }
            if n < MIN_DICT_WORDS || p < ram + 4 {
                continue;
            }
            if read32(mem, p - 4).ok() == Some(n) && best.is_none_or(|(_, bn, _)| n > bn) {
                best = Some((p - 4, n, stride));
            }
        }
        p += 1;
    }
    best.ok_or(GrammarError::Absent)
}

/// Actions tables that could end where the dictionary begins.
///
/// Inform pads to a four-byte boundary before a Unicode dictionary and not
/// otherwise, so up to three bytes may sit between the two. Every entry must
/// look like a code address — Glulx Inform keeps all code and strings in ROM,
/// below RAMSTART.
fn action_candidates(mem: &Memory, dictionary: u32, ram: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for pad in 0..4u32 {
        let Some(end) = dictionary.checked_sub(pad) else { continue };
        for k in 1..MAX_ACTIONS {
            let Some(a) = end.checked_sub(4 + 4 * k) else { break };
            if a < ram {
                break;
            }
            if read32(mem, a).ok() != Some(k) {
                continue;
            }
            let plausible = (0..k).all(|i| {
                let v = read32(mem, a + 4 + i * 4).unwrap_or(0);
                (60..ram).contains(&v)
            });
            if plausible {
                out.push((a, k));
            }
        }
    }
    out
}

/// The grammar table that ends exactly at `actions`, walking every verb, every
/// line and every token to prove it.
fn find_grammar(mem: &Memory, ram: u32, lim: u32, actions: u32) -> Option<(u32, u32)> {
    let mut base = actions;
    while base > ram {
        base -= 1;
        let Ok(n) = read32(mem, base) else { continue };
        if !(1..=MAX_VERBS).contains(&n) {
            continue;
        }
        let first = base.checked_add(4 + 4 * n)?;
        if first >= actions || read32(mem, base + 4).ok() != Some(first) {
            continue;
        }
        if walk_grammar(mem, base, n, lim, actions) {
            return Some((base, n));
        }
    }
    None
}

/// True if the whole table reads cleanly and ends on `actions`'s first byte.
fn walk_grammar(mem: &Memory, base: u32, n: u32, lim: u32, actions: u32) -> bool {
    let mut cur = base + 4 + 4 * n;
    for i in 0..n {
        if read32(mem, base + 4 + i * 4).ok() != Some(cur) {
            return false;
        }
        match skip_verb(mem, cur, lim, actions) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    cur == actions
}

/// Advance past one verb's line block without decoding it.
fn skip_verb(mem: &Memory, at: u32, lim: u32, ceiling: u32) -> Option<u32> {
    let mut cur = at;
    if cur >= lim {
        return None;
    }
    let lines = byte(mem, cur);
    cur += 1;
    for _ in 0..lines {
        cur = cur.checked_add(3)?;
        loop {
            if cur >= lim || cur > ceiling {
                return None;
            }
            let t = byte(mem, cur);
            cur += 1;
            if t == ENDIT {
                break;
            }
            cur = cur.checked_add(4)?;
        }
    }
    (cur <= ceiling).then_some(cur)
}

// ── Reading the tables ───────────────────────────────────────────────────────

/// One dictionary record, decoded far enough to answer the questions above.
struct DictWord {
    address: u32,
    text: String,
    roles: WordRoles,
    verb_number: u32,
}

fn read_dictionary(mem: &Memory, t: &Tables) -> Result<Vec<DictWord>, GrammarError> {
    let w = t.dict_word_size;
    let mut out = Vec::with_capacity(t.word_count as usize);
    for i in 0..t.word_count {
        let entry = t.dictionary + 4 + i * t.dict_stride;
        // A Unicode dictionary pads the tag out to four bytes before the text,
        // so the byte after the tag is zero for every record. A byte-valued
        // one always has a character there.
        if i < 8 && byte(mem, entry + 1) == 0 {
            return Err(GrammarError::UnicodeDictionary);
        }
        let mut text = String::new();
        for j in 0..w {
            let c = byte(mem, entry + 1 + j);
            if c == 0 {
                break;
            }
            text.push(c as char); // records are Latin-1, lower-cased by Inform
        }
        let flags = read16(mem, entry + 1 + w)?;
        // The stored verb number is INVERTED: Inform counts down from $FFFF in
        // Glulx (and from $FF on the Z-machine), so the grammar table's index
        // for this verb is $FFFF minus what the record holds
        // (`Inform6/src/text.c::dictionary_set_verb_number`).
        let verb_number = 0xFFFF - read16(mem, entry + 3 + w)? as u32;
        out.push(DictWord {
            address: entry,
            text,
            roles: WordRoles {
                verb: flags & VERB_DFLAG != 0,
                meta: flags & META_DFLAG != 0,
                plural: flags & PLURAL_DFLAG != 0,
                preposition: flags & PREP_DFLAG != 0,
                singular: flags & SING_DFLAG != 0,
                truncated: flags & TRUNC_DFLAG != 0,
                noun: flags & NOUN_DFLAG != 0,
                raw: flags,
            },
            verb_number,
        });
    }
    Ok(out)
}

/// Read one verb's lines. Returns the address just past the block alongside
/// them, so a caller can check the block ended where it should.
fn read_verb_lines(
    mem: &Memory,
    at: u32,
    words: &[DictWord],
) -> Result<(u32, Vec<SyntaxLine>), GrammarError> {
    let mut cur = at;
    let count = byte(mem, cur);
    cur += 1;
    let mut lines = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let action = read16(mem, cur)?;
        let flags = byte(mem, cur + 2);
        cur += 3;
        let mut slots: Vec<Slot> = Vec::new();
        loop {
            let ty = byte(mem, cur);
            cur += 1;
            if ty == ENDIT {
                break;
            }
            let data = read32(mem, cur)?;
            cur += 4;
            let token = decode_token(ty & 0x0F, data, words)?;
            // Bits 4-5 are the `/`-alternation state: Inform sets $20 on a
            // token followed by a slash and $10 on one preceded by a slash
            // (`verbs.c`), so bit 4 means "continues the slot before me".
            match ((ty >> 4) & 0x01 != 0, slots.last_mut()) {
                (true, Some(last)) => last.alternatives.push(token),
                _ => slots.push(Slot::one(token)),
            }
            if slots.len() > 64 {
                return Err(GrammarError::BadSyntaxLine);
            }
        }
        lines.push(SyntaxLine { action, reverse: flags & 0x01 != 0, slots });
    }
    Ok((cur, lines))
}

fn decode_token(ty: u8, data: u32, words: &[DictWord]) -> Result<Token, GrammarError> {
    Ok(match ty {
        1 => Token::Noun(NounKind::from_elementary(data).ok_or(GrammarError::BadSyntaxLine)?),
        2 => Token::Word(
            words
                .iter()
                .find(|w| w.address == data)
                .map(|w| w.text.clone())
                .ok_or(GrammarError::BadSyntaxLine)?,
        ),
        3 => Token::FilteredNoun(data),
        4 => Token::Attribute(data),
        5 => Token::Scope(data),
        6 => Token::Routine(data),
        _ => return Err(GrammarError::BadSyntaxLine),
    })
}

// ── Bounds-checked reads ─────────────────────────────────────────────────────

fn byte(mem: &Memory, addr: u32) -> u8 {
    mem.read8(addr).unwrap_or(0) as u8
}

fn read16(mem: &Memory, addr: u32) -> Result<u16, GrammarError> {
    mem.read16(addr).map(|v| v as u16).ok_or(GrammarError::Truncated)
}

fn read32(mem: &Memory, addr: u32) -> Result<u32, GrammarError> {
    mem.read32(addr).ok_or(GrammarError::Truncated)
}

// ── Why these types are gvm's own, and not shared with zvm ───────────────────
//
// The two engines' grammar READERS have almost nothing in common. The formats
// agree on the token type numbering and on nothing else: the Z-machine's table
// is at a header-named address and this one is not recorded at all; its verb
// numbers count down from 255 and these count up from zero; its line header is
// two bytes with the reverse flag packed into the action and this one is three
// with a flags byte; its tokens are 1+2 bytes and these are 1+4; its dictionary
// is Z-encoded with a game-chosen record length and this one is plain bytes
// behind a type tag. `zvm::grammar` additionally carries four Infocom-era
// shapes that have no Glulx counterpart at all.
//
// So a trait over "read a byte / read a word at an address" would abstract a
// handful of lines out of several hundred while forcing both zero-dependency
// crates to name a shared vocabulary. What the two DO share is the shape of the
// ANSWER — `Token`, `NounKind`, `Slot`, `SyntaxLine`, `Verb`, `WordRoles` — and
// the right way to share that is a small zero-dependency workspace crate both
// depend on, not a trait over memory.
//
// That change is deliberately NOT made here: it would rewrite `zvm::grammar`'s
// just-landed public API, and it is worth doing once, on purpose, rather than
// as a side effect of adding the second reader. Until then the names are kept
// identical so the conversion is mechanical, and the duplicated knowledge is
// confined to the elementary-token numbering and the six token types — a
// transcription of Inform's published constants, which is the same thing both
// crates already do independently for their own opcode tables.

#[cfg(test)]
mod tests {
    use super::*;

    const RAM: u32 = 0x0100;
    const EXT: u32 = 0x0400;

    /// A hand-built Glulx image with the three Inform tables laid out the way
    /// `Inform6/src/tables.c::construct_storyfile_g` lays them out: grammar,
    /// then actions, then dictionary, contiguous and in that order.
    struct Story {
        buf: Vec<u8>,
    }

    impl Story {
        fn new() -> Story {
            let mut buf = vec![0u8; EXT as usize];
            buf[0..4].copy_from_slice(b"Glul");
            let w = |buf: &mut Vec<u8>, at: usize, v: u32| {
                buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
            };
            w(&mut buf, 0x04, 0x0003_0102); // version 3.1.2
            w(&mut buf, 0x08, RAM);
            w(&mut buf, 0x0C, EXT);
            w(&mut buf, 0x10, EXT);
            w(&mut buf, 0x14, 0x1000);
            w(&mut buf, 0x18, 0x40);
            Story { buf }
        }

        fn b(&mut self, at: u32, v: u8) {
            self.buf[at as usize] = v;
        }

        fn w32(&mut self, at: u32, v: u32) {
            self.buf[at as usize..at as usize + 4].copy_from_slice(&v.to_be_bytes());
        }

        fn w16(&mut self, at: u32, v: u16) {
            self.buf[at as usize..at as usize + 2].copy_from_slice(&v.to_be_bytes());
        }

        /// Write a byte-valued dictionary of `(word, flags, verb_index)` at
        /// `at`, with nine characters per record (Inform's default).
        fn dictionary(&mut self, at: u32, entries: &[(&str, u16, u32)]) {
            self.w32(at, entries.len() as u32);
            for (i, (word, flags, verb)) in entries.iter().enumerate() {
                let e = at + 4 + i as u32 * 16;
                self.b(e, DICT_TAG);
                for (j, c) in word.bytes().take(9).enumerate() {
                    self.b(e + 1 + j as u32, c);
                }
                self.w16(e + 10, *flags);
                self.w16(e + 12, (0xFFFF - *verb) as u16);
            }
        }

        fn mem(&self) -> Memory {
            Memory::new(self.buf.clone()).expect("synthetic image is valid")
        }

        fn grammar(&self) -> Result<Grammar, GrammarError> {
            Grammar::load(&self.mem())
        }

        fn error(&self) -> Option<GrammarError> {
            self.grammar().err()
        }
    }

    /// Two verbs. "take" has `take noun` and `take noun in / into noun`
    /// (reversed); "look" has a bare line and one with an attribute token.
    ///
    /// Layout is computed rather than hard-coded, because the locator's whole
    /// contract is that the three tables abut exactly.
    fn story() -> Story {
        let mut s = Story::new();
        let g = RAM; // grammar table
        let verbs = 2u32;
        let v0 = g + 4 + 4 * verbs;
        // verb 0: 2 lines
        //   [ac 0007][fl 00] noun ENDIT                       = 3 + 5 + 1
        //   [ac 0008][fl 01] noun, 'in'/'into', noun ENDIT    = 3 + 20 + 1
        let v0_len = 1 + (3 + 5 + 1) + (3 + 20 + 1);
        let v1 = v0 + v0_len;
        // verb 1: 4 lines — a bare one, the decoy described below, and two
        // attribute lines after it so the decoy's forged pointer still lands
        // inside the grammar region.
        let v1_len = 1 + (3 + 1) + 3 * (3 + 5 + 1);
        let actions = v1 + v1_len;
        let action_count = 12u32;
        let dict = actions + 4 + 4 * action_count;

        // At least `MIN_DICT_WORDS` records, because a shorter run is not a
        // positive identification and the locator declines it. Real dictionaries
        // run to hundreds; the filler nouns stand in for those.
        let mut words: Vec<(&str, u16, u32)> = vec![
            ("hold", VERB_DFLAG, 0),
            ("in", PREP_DFLAG, 0),
            ("into", PREP_DFLAG, 0),
            ("lamp", NOUN_DFLAG, 0),
            ("look", VERB_DFLAG, 1),
            ("take", VERB_DFLAG, 0),
        ];
        for filler in [
            "aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii", "jj", "kk", "ll", "mm", "nn",
        ] {
            words.push((filler, NOUN_DFLAG, 0));
        }
        s.dictionary(dict, &words);
        let in_addr = dict + 4 + 16;
        let into_addr = dict + 4 + 2 * 16;

        s.w32(g, verbs);
        s.w32(g + 4, v0);
        s.w32(g + 8, v1);

        let mut m = v0;
        s.b(m, 2);
        m += 1;
        s.w16(m, 7);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, ENDIT);
        m += 1;
        s.w16(m, 8);
        s.b(m + 2, 0x01);
        m += 3; // reverse
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, 0x62);
        s.w32(m + 1, in_addr);
        m += 5; // 'in', opens a list
        s.b(m, 0x52);
        s.w32(m + 1, into_addr);
        m += 5; // 'into', continues it
        s.b(m, 0x01);
        s.w32(m + 1, 0);
        m += 5; // noun
        s.b(m, ENDIT);
        m += 1;
        assert_eq!(m, v1, "verb 0 block length");

        s.b(m, 4);
        m += 1;
        s.w16(m, 9);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, ENDIT);
        m += 1;

        // A DECOY, planted on purpose. Read as a grammar table, the four bytes
        // at this line's start are `00 00 00 04` — a verb count of 4 — and the
        // four after them are this attribute's value, set to exactly
        // `decoy + 4 + 4*4`. So the line satisfies `find_grammar`'s pointer-array
        // precondition perfectly, sits ABOVE the real table, and is therefore
        // the first thing a backward scan meets. Only walking it, and finding
        // that it does not end on the actions table, rejects it.
        //
        // This shape is not contrived. Across the 22 Glulx stories in the local
        // corpus, 889 byte offsets satisfy that precondition — 279 in one game —
        // and exactly 22 survive the walk.
        let decoy = m;
        s.w16(m, 0);
        s.b(m + 2, 0);
        m += 3;
        s.b(m, 0x04);
        s.w32(m + 1, decoy + 20);
        m += 5;
        s.b(m, ENDIT);
        m += 1;

        for (action, attr) in [(10u16, 17u32), (11, 18)] {
            s.w16(m, action);
            s.b(m + 2, 0);
            m += 3;
            s.b(m, 0x04);
            s.w32(m + 1, attr);
            m += 5;
            s.b(m, ENDIT);
            m += 1;
        }
        assert_eq!(m, actions, "verb 1 block length");
        assert!(decoy + 20 < actions, "the decoy must forge a pointer inside the region");

        s.w32(actions, action_count);
        for i in 0..action_count {
            s.w32(actions + 4 + i * 4, 0x60 + i); // plausible ROM addresses
        }
        s
    }

    #[test]
    fn locates_the_three_tables_by_the_chain() {
        let s = story();
        let t = locate(&s.mem()).expect("the chain closes");
        // Pinned exactly, not merely "found": the story plants a decoy that
        // satisfies every check except the walk landing on the actions table,
        // and it sits closer to `actions` than the real table does. An address
        // assertion is the only thing that can tell the two apart.
        assert_eq!(t.grammar, RAM);
        assert_eq!(t.verb_count, 2);
        assert_eq!(t.action_count, 12);
        assert_eq!(t.word_count, 20);
        assert_eq!(t.dict_stride, 16);
        assert_eq!(t.dict_word_size, 9);
        // The tables abut: that is the property the locator proves, and the
        // only reason its answer can be trusted at all.
        assert!(t.grammar < t.actions && t.actions < t.dictionary);
        assert_eq!(t.actions + 4 + 4 * t.action_count, t.dictionary);
    }

    #[test]
    fn reads_verbs_lines_and_tokens() {
        let g = story().grammar().expect("synthetic story has a grammar");
        assert_eq!(g.verbs().len(), 2);

        // Inform numbers verbs downwards from $FFFF in the dictionary, so both
        // spellings of verb 0 must land on it.
        let take = g.verb_for_word("take").expect("knows 'take'");
        assert_eq!(take.number, 0);
        assert_eq!(take.words, vec!["hold".to_string(), "take".to_string()]);
        assert_eq!(take.lines.len(), 2);
        assert_eq!(take.lines[0].describe("take"), "take noun");
        assert_eq!(take.lines[0].action, 7);
        assert!(!take.lines[0].reverse);

        // Glulx keeps the swap flag in its own byte rather than in the action.
        assert!(take.lines[1].reverse);
        assert_eq!(take.lines[1].action, 8);
        assert_eq!(take.lines[1].describe("take"), "take noun in / into noun REVERSE");
        assert_eq!(take.lines[1].noun_count(), 2);
        assert_eq!(take.lines[1].slots[1].alternatives.len(), 2);
        assert!(take.accepts(2, &["in"]));
        assert!(take.accepts(2, &["into"]));
        assert!(!take.accepts(2, &["under"]));

        let look = g.verb_for_word("look").expect("knows 'look'");
        assert!(look.takes_bare());
        assert_eq!(look.lines.len(), 4);
        assert_eq!(look.lines[2].slots, vec![Slot::one(Token::Attribute(17))]);

        assert!(g.is_preposition("in") && g.is_preposition("into"));
        assert!(!g.is_verb("lamp"));
        assert!(g.roles("lamp").is_some_and(|r| r.noun && !r.verb));
        assert!(g.roles("take").is_some_and(|r| r.verb));
        assert_eq!(g.action_routines().len(), 12);
        assert_eq!(g.verbs_accepting(2, &["in"]).len(), 1);
    }

    // ── Falsification ────────────────────────────────────────────────────────
    //
    // The locator derives three addresses that nothing in the file records. If
    // it can be made to answer when the chain does not actually close, every
    // address it returns is a guess and the consumer cannot tell.

    #[test]
    fn refuses_when_the_actions_table_does_not_abut_the_dictionary() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Change the action count so the table no longer ends at the dictionary.
        s.w32(t.actions, 10);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_when_the_grammar_walk_misses_the_actions_table() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Give the first verb one line too many: the walk now overruns.
        s.b(t.grammar + 4 + 4 * t.verb_count, 3);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_a_verb_pointer_that_does_not_follow_the_pointer_array() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        s.w32(t.grammar + 4, t.grammar + 4 + 4 * t.verb_count + 1);
        assert_eq!(s.error(), Some(GrammarError::TablesNotFound));
    }

    #[test]
    fn refuses_an_unknown_token_type() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Verb 0's first token: elementary (1) becomes 7, which no version of
        // the format defines. The walk still terminates, so this is caught by
        // the reader rather than the locator.
        s.b(t.grammar + 4 + 4 * t.verb_count + 4, 0x07);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_an_elementary_token_above_nine() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        s.w32(t.grammar + 4 + 4 * t.verb_count + 5, 40);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn refuses_a_preposition_that_names_no_dictionary_word() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // Verb 0's second line, second token, is the 'in' preposition. Point it
        // between two records: a reader that shrugged would report a verb whose
        // preposition is a word the player can never type.
        let line2 = t.grammar + 4 + 4 * t.verb_count + 1 + 9;
        s.w32(line2 + 3 + 5 + 1, t.dictionary + 4 + 3);
        assert_eq!(s.error(), Some(GrammarError::BadSyntaxLine));
    }

    #[test]
    fn reports_absent_when_there_is_no_dictionary_at_all() {
        let s = Story::new();
        assert_eq!(s.error(), Some(GrammarError::Absent));
    }

    #[test]
    fn refuses_a_unicode_dictionary_rather_than_misreading_it() {
        let mut s = story();
        let t = locate(&s.mem()).unwrap();
        // A Unicode record pads the tag to four bytes, so the byte after the
        // tag is zero. Blank the first characters to look like one.
        for i in 0..8u32 {
            s.b(t.dictionary + 4 + i * t.dict_stride + 1, 0);
        }
        assert_eq!(s.error(), Some(GrammarError::UnicodeDictionary));
    }
}
