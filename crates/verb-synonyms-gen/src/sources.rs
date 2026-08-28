//! Readers for the two external lexical sources this generator consumes.
//!
//! Both are read from a directory the caller names; neither is vendored into the
//! repository, because they are large and because their licences are satisfied
//! by reproducing a notice (see `THIRD-PARTY-NOTICES.md`) rather than by
//! shipping the corpora. `fetch-sources.sh` downloads the exact versions below
//! and checks their digests.
//!
//! * **WordNet 3.0** (`dict/`) — Princeton University, © 2006. Supplies
//!   synonymy (words sharing a synset) and the pointer graph (hypernym `@`,
//!   verb group `$`, derivational `+`) used to reach an IF verb when plain
//!   synonymy does not.
//! * **12dicts 6.0.2**, `Lemmatized/2+2+3frq.txt` — Alan Beale, under the AGID
//!   terms. Supplies both the frequency ranking (21 bands, commonest first) and
//!   — because the list is LEMMATIZED, headword at column 0 with its inflected
//!   and derived forms indented beneath — a source-backed inflection map. That
//!   second use matters: the rule that this table holds base forms only is
//!   enforced with Beale's lemmatisation and WordNet's `verb.exc`, never with a
//!   suffix rule, which would eat `dress`, `press` and `sing`.

use std::collections::BTreeMap;
use std::path::Path;

// ── WordNet ──────────────────────────────────────────────────────────────────

/// One WordNet synset: a set of words that mean the same thing, plus its
/// outgoing pointers.
#[derive(Debug, Default, Clone)]
pub struct Synset {
    /// The synset's words, lower-cased, `_` replaced by a space.
    pub words: Vec<String>,
    /// `(pointer symbol, target offset)` for every pointer to another VERB
    /// synset. Noun and adjective targets are dropped: this generator only ever
    /// walks from a verb to a verb.
    pub pointers: Vec<(String, u32)>,
}

/// The slice of WordNet 3.0 this generator reads: verbs only.
#[derive(Default)]
pub struct WordNet {
    /// Lemma → its synset offsets, in WordNet's own sense order (commonest
    /// first). Multi-word lemmas are keyed with spaces, not underscores.
    pub senses: BTreeMap<String, Vec<u32>>,
    /// Synset offset → the synset.
    pub synsets: BTreeMap<u32, Synset>,
    /// `verb.exc`: an irregular inflected form → its base form.
    pub exceptions: BTreeMap<String, String>,
}

/// A `dict/` line that is part of the licence header rather than data.
fn is_header(line: &str) -> bool {
    line.starts_with("  ")
}

impl WordNet {
    /// Read `index.verb`, `data.verb` and `verb.exc` from a WordNet `dict/`.
    pub fn load(dict: &Path) -> std::io::Result<WordNet> {
        let mut wn = WordNet::default();

        let index = read_latin1(&dict.join("index.verb"))?;
        for line in index.lines() {
            if is_header(line) || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split_whitespace().collect();
            // lemma pos synset_cnt p_cnt [ptr_symbol…] sense_cnt tagsense_cnt offsets…
            if f.len() < 6 {
                continue;
            }
            let Ok(synset_cnt) = f[2].parse::<usize>() else {
                continue;
            };
            let Ok(p_cnt) = f[3].parse::<usize>() else {
                continue;
            };
            let offsets_at = 4 + p_cnt + 2;
            if f.len() < offsets_at + synset_cnt {
                continue;
            }
            let offsets: Vec<u32> = f[offsets_at..offsets_at + synset_cnt]
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            wn.senses.insert(unslash(f[0]), offsets);
        }

        let data = read_latin1(&dict.join("data.verb"))?;
        for line in data.lines() {
            if is_header(line) || line.trim().is_empty() {
                continue;
            }
            let body = line.split(" | ").next().unwrap_or(line);
            let f: Vec<&str> = body.split_whitespace().collect();
            // offset lex_filenum ss_type w_cnt(hex) [word lex_id]… p_cnt [ptr]…
            if f.len() < 4 {
                continue;
            }
            let Ok(offset) = f[0].parse::<u32>() else {
                continue;
            };
            let Ok(w_cnt) = usize::from_str_radix(f[3], 16) else {
                continue;
            };
            let mut s = Synset::default();
            let mut i = 4;
            for _ in 0..w_cnt {
                if i + 1 >= f.len() {
                    break;
                }
                s.words.push(unslash(f[i]));
                i += 2;
            }
            if i < f.len() {
                if let Ok(p_cnt) = f[i].parse::<usize>() {
                    i += 1;
                    for _ in 0..p_cnt {
                        if i + 3 >= f.len() {
                            break;
                        }
                        // symbol offset pos source/target
                        if f[i + 2] == "v" {
                            if let Ok(t) = f[i + 1].parse::<u32>() {
                                s.pointers.push((f[i].to_string(), t));
                            }
                        }
                        i += 4;
                    }
                }
            }
            wn.synsets.insert(offset, s);
        }

        let exc = read_latin1(&dict.join("verb.exc"))?;
        for line in exc.lines() {
            let mut it = line.split_whitespace();
            if let (Some(inflected), Some(base)) = (it.next(), it.next()) {
                wn.exceptions.insert(unslash(inflected), unslash(base));
            }
        }

        Ok(wn)
    }

    /// The words of a synset, or an empty slice for an offset that is not there.
    pub fn words_of(&self, offset: u32) -> &[String] {
        self.synsets.get(&offset).map_or(&[], |s| &s.words[..])
    }
}

/// WordNet spells multi-word lemmas with `_`; everything else in this generator
/// spells them with a space.
fn unslash(w: &str) -> String {
    w.to_ascii_lowercase().replace('_', " ")
}

/// WordNet's files are ASCII with a handful of Latin-1 bytes; decoding them
/// lossily as UTF-8 would corrupt those words silently.
fn read_latin1(path: &Path) -> std::io::Result<String> {
    Ok(std::fs::read(path)?.iter().map(|&b| b as char).collect())
}

// ── 12dicts frequency list ───────────────────────────────────────────────────

/// The lemmatized frequency list: which words are common, and which surface
/// forms belong to which headword.
#[derive(Default)]
pub struct Frequency {
    /// Headword → frequency band, 1 (commonest) upward.
    pub band: BTreeMap<String, u16>,
    /// Headwords in band order, commonest first.
    pub ranked: Vec<String>,
    /// Any listed form (headword or inflection) → its headword. A headword maps
    /// to itself.
    pub lemma_of: BTreeMap<String, String>,
}

impl Frequency {
    /// Read `Lemmatized/2+2+3frq.txt`.
    pub fn load(path: &Path) -> std::io::Result<Frequency> {
        let text = read_latin1(path)?;
        let mut f = Frequency::default();
        let mut band: u16 = 0;
        let mut head = String::new();
        for line in text.lines() {
            let t = line.trim_end();
            if t.is_empty() {
                continue;
            }
            if let Some(rest) = t.strip_prefix("----- ") {
                if let Some(n) = rest.split(' ').next().and_then(|n| n.parse().ok()) {
                    band = n;
                }
                continue;
            }
            if t.starts_with(' ') || t.starts_with('\t') {
                // Inflected and derived forms of the headword above.
                for w in t.split(',') {
                    if let Some(w) = clean(w) {
                        if !head.is_empty() {
                            f.lemma_of.entry(w).or_insert_with(|| head.clone());
                        }
                    }
                }
                continue;
            }
            head = clean(t).unwrap_or_default();
            if head.is_empty() {
                continue;
            }
            f.band.entry(head.clone()).or_insert(band);
            f.lemma_of.insert(head.clone(), head.clone());
            f.ranked.push(head.clone());
        }
        Ok(f)
    }

    /// Every headword in bands 1..=`bands`, commonest first.
    pub fn top(&self, bands: u16) -> Vec<&str> {
        self.ranked
            .iter()
            .filter(|w| self.band[*w] <= bands)
            .map(String::as_str)
            .collect()
    }
}

/// Strip 12dicts' annotations from one entry and refuse anything that is not an
/// ordinary lower-case word.
///
/// The annotations that appear in `2+2+3frq.txt` are `*` (this form is listed
/// under another headword), `!` (neologism), `:` (abbreviation used without a
/// period) and parentheses around capitalised words, abbreviations and
/// contractions. Refusing the parenthesised and capitalised entries is how
/// proper nouns and `can't` stay out; nothing here is a judgement about an
/// individual word.
fn clean(raw: &str) -> Option<String> {
    let w = raw.trim().trim_end_matches(['*', '!', ':', '?']);
    if w.is_empty() || w.starts_with('(') {
        return None;
    }
    if !w
        .bytes()
        .all(|c| c.is_ascii_lowercase() || c == b'-' || c == b' ')
    {
        return None;
    }
    Some(w.to_string())
}
