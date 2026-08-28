//! Step 1 — read the real IF verb vocabulary out of a corpus of story files.
//!
//! The whole design of this table rests on the IF side being BOUNDED: a general
//! thesaurus is enormous, but the set of words an interactive-fiction parser can
//! accept as the first word of a command is a few thousand across every game
//! anyone has written. Inverting the thesaurus against that set is what keeps the
//! shipped artifact small. So the set has to be measured, not guessed — this
//! module is the measuring.
//!
//! What it reads, per engine:
//!
//!   * Z-machine — [`zvm::grammar::Grammar::load`], then `verb_words()`.
//!   * Glulx — [`gvm::grammar::Grammar::load`], then `verb_words()`. Glulx
//!     grammar tables are DERIVED rather than named by a header field, so a
//!     story whose tables cannot be located is refused; that is the reader
//!     working, not failing, and such stories are skipped.
//!   * Scott Adams — `scott::Database::verbs`, which is the verb table itself,
//!     with a `*`-prefixed entry read as a synonym of the nearest preceding
//!     unprefixed one.
//!
//! Two things come back from each: the SPELLINGS, and the GROUPING — which
//! spellings the story's author declared to be one verb. The second is what
//! SQ-1115 added; before it, `verb_words()` was flattened and the author's own
//! synonym sets were thrown away. See [`Harvest::groups`].
//!
//! Disk images (`.dsk`, `.adf`, `.d64`, `.2mg`, …) are deliberately NOT read
//! here. Mounting them lives in `app`, and depending on `app` from a generator
//! would drag the whole TUI into a build that wants three parsers. Every game in
//! this corpus that ships on a floppy also ships as a bare story file, so the
//! vocabulary lost is nil; see the skip report the binary prints.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What one corpus sweep found.
#[derive(Default)]
pub struct Harvest {
    /// Every verb spelling, deduplicated across the corpus and sorted.
    ///
    /// Multi-word entries (`turn on`) are the verb word plus one literal word
    /// from a syntax line, joined by a space — see [`Harvest::absorb`].
    pub verbs: BTreeSet<String>,
    /// Which stories declared each spelling, by file stem. Kept for the harvest
    /// REPORT (which games have an odd vocabulary?) and for the story counts
    /// written beside each verb; the committed list carries counts, not names.
    pub sources: BTreeMap<String, BTreeSet<String>>,
    /// Files that yielded at least one verb.
    pub read: usize,
    /// Files skipped, with the reason, one line each.
    pub skipped: Vec<(PathBuf, String)>,
    /// Per-engine tallies of files that contributed, indexed by [`ENGINE_Z`] &c.
    pub by_engine: [usize; 3],
    /// Every synonym GROUP the corpus declares: the spellings one verb entry
    /// carries, sorted and deduplicated, mapped to the stories declaring
    /// exactly that set.
    ///
    /// This is the grouping `verb_words()` throws away, and recovering it is
    /// the point of SQ-1115. A verb entry is the game author's own statement
    /// that these spellings are ONE action — a stronger authority for a parser
    /// than a lexicographer's view of English, and free, since the grammar is
    /// already loaded. See [`Harvest::record_group`].
    pub groups: BTreeMap<Vec<String>, BTreeSet<String>>,
    /// Spellings that appeared in two different verb entries of ONE story.
    ///
    /// Should be empty: a dictionary word resolves to a single verb number, so
    /// a spelling belongs to one entry per story — which is what lets `build`
    /// count a pair's support by summing the entries that contain it. Measured
    /// rather than assumed, and reported.
    pub double_booked: BTreeSet<String>,
    /// The story currently being read, for [`Harvest::record`].
    story: String,
    /// Which verb entry of the current story each spelling appeared in, for
    /// [`Harvest::double_booked`].
    entry_of: BTreeMap<String, usize>,
    /// Verb entries seen so far in the current story, for the same.
    entries: usize,
}

/// Index into [`Harvest::by_engine`] — Z-machine.
pub const ENGINE_Z: usize = 0;
/// Index into [`Harvest::by_engine`] — Glulx.
pub const ENGINE_GLULX: usize = 1;
/// Index into [`Harvest::by_engine`] — Scott Adams.
pub const ENGINE_SCOTT: usize = 2;

impl Harvest {
    /// Start reading a new story. The per-entry bookkeeping is per story, so it
    /// is cleared here rather than accumulated across the corpus.
    fn begin(&mut self, story: String) {
        self.story = story;
        self.entry_of.clear();
        self.entries = 0;
    }

    /// Note that the story now being read accepts `word`.
    fn record(&mut self, word: String) {
        self.sources
            .entry(word.clone())
            .or_default()
            .insert(self.story.clone());
        self.verbs.insert(word);
    }

    /// How many stories in the corpus accept `word`.
    pub fn story_count(&self, word: &str) -> usize {
        self.sources.get(word).map_or(0, BTreeSet::len)
    }

    /// Note that the story now being read declares `words` as ONE verb — that
    /// is, as spellings of a single action.
    ///
    /// Only the spellings themselves are grouped, never the verb-plus-literal
    /// phrases `absorb` synthesises: a story that groups `turn` with `rotate`
    /// has said nothing about whether `rotate on` is a thing its syntax lines
    /// accept, and pairing the cross product would invent evidence.
    fn record_group(&mut self, words: &[String]) {
        let mut set: Vec<String> = words.to_vec();
        set.sort();
        set.dedup();
        if set.len() < 2 {
            return;
        }
        let id = self.entries;
        self.entries += 1;
        for w in &set {
            match self.entry_of.insert(w.clone(), id) {
                Some(prev) if prev != id => {
                    self.double_booked.insert(w.clone());
                }
                _ => {}
            }
        }
        self.groups
            .entry(set)
            .or_default()
            .insert(self.story.clone());
    }

    /// Merge one story's verb words, plus the verb-plus-literal PHRASES its
    /// syntax lines declare.
    ///
    /// The phrases matter because English lexicalises `turn on`, `pick up` and
    /// `look at` as units and a thesaurus indexes them that way, while a
    /// Z-machine dictionary can only hold `turn`, `pick` and `look` — the
    /// `on`/`up`/`at` is a literal token inside a syntax line. Reading both
    /// halves is the only way to ask a thesaurus the question the story
    /// actually accepts.
    fn absorb<'a>(
        &mut self,
        words: impl Iterator<Item = &'a str>,
        verbs: &[grammar_model::Verb],
    ) -> usize {
        let mut n = 0;
        for w in words {
            if plausible(w) {
                self.record(w.to_string());
                n += 1;
            }
        }
        for v in verbs {
            let heads: Vec<String> = v
                .words
                .iter()
                .filter(|w| plausible(w))
                .map(|w| w.to_string())
                .collect();
            // The GROUP is recorded at the two-character floor, not the
            // three-character one. `plausible` keeps `x`, `g` and `n` out of
            // the vocabulary because they cannot be looked up in a thesaurus —
            // but a group needs no lookup, and the entry the corpus states 28
            // times is `go` / `walk` / `run`, whose first member the
            // three-character floor silently deletes. `go` is the commonest
            // verb in interactive fiction; a group that cannot suggest it is
            // the poorer for a rule that was never about grouping.
            self.record_group(
                &v.words
                    .iter()
                    .filter(|w| groupable(w))
                    .map(|w| w.to_string())
                    .collect::<Vec<String>>(),
            );
            if heads.is_empty() {
                continue;
            }
            let literals: BTreeSet<String> = v
                .lines
                .iter()
                .flat_map(|l| l.literals())
                .filter(|w| particle(w))
                .map(str::to_string)
                .collect();
            for lit in &literals {
                for h in &heads {
                    self.record(format!("{h} {lit}"));
                    n += 1;
                }
            }
        }
        n
    }
}

/// File extensions this harvester will open. Anything else is skipped without a
/// reason rather than sniffed, because a corpus directory also holds saves,
/// configs, artwork and disk images.
const READABLE: &[&str] = &[
    "z1", "z2", "z3", "z4", "z5", "z6", "z7", "z8", "zblorb", "zlb", "blb", "blorb", "gblorb",
    "glb", "ulx", "dat", "txt",
];

/// True when `path` is worth opening at all.
fn readable(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    READABLE.contains(&ext.as_str())
}

/// A verb spelling worth keeping.
///
/// The dictionary holds abbreviations (`x`, `g`, `z`), direction letters (`n`,
/// `sw`) and, on Infocom's tables, truncated stems. None of those can be looked
/// up in a thesaurus, and letting them through only wastes a lookup — but the
/// filter is on SHAPE, not on a list of words someone decided were
/// uninteresting: three or more characters, ASCII lower-case letters and
/// interior hyphens only.
/// A literal word inside a syntax line, worth pairing with its verb.
///
/// Two characters, not three: English lexicalises `turn ON`, `pick UP` and
/// `look AT`, a thesaurus indexes all three that way, and `plausible`'s
/// three-character floor — which exists to keep `x`, `g` and `n` out of the
/// VERB set — would silently drop every one of them.
fn particle(word: &str) -> bool {
    word.len() >= 2
        && word.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
        && word.starts_with(|c: char| c.is_ascii_lowercase())
}

/// A spelling worth keeping as a member of a synonym GROUP.
///
/// Two characters rather than three — see [`Harvest::absorb`]; the floor is
/// there only to keep single-letter abbreviations (`x`, `g`, `l`, `i`) out,
/// since a player shown `x` learns nothing.
fn groupable(word: &str) -> bool {
    word.len() >= 2
        && word.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
        && word.starts_with(|c: char| c.is_ascii_lowercase())
}

fn plausible(word: &str) -> bool {
    word.len() >= 3
        && word.bytes().all(|c| c.is_ascii_lowercase() || c == b'-')
        && word.starts_with(|c: char| c.is_ascii_lowercase())
}

/// Sweep every file directly under `dir` (corpora here are flat) and merge what
/// each story's parser accepts into `out`.
pub fn sweep(dir: &Path, out: &mut Harvest) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    for path in entries {
        if !readable(&path) {
            continue;
        }
        out.begin(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        match harvest_file(&path, out) {
            Ok(()) => {}
            Err(why) => out.skipped.push((path, why)),
        }
    }
    Ok(())
}

/// Read one file, adding whatever verbs it declares.
fn harvest_file(path: &Path, out: &mut Harvest) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() < 64 {
        return Err("too small to be a story".into());
    }

    if blorb::Blorb::is_blorb(&bytes) {
        let b = blorb::Blorb::parse(bytes).map_err(|e| format!("blorb: {e:?}"))?;
        let (kind, exec) = b.executable().map_err(|e| format!("blorb exec: {e:?}"))?;
        return match kind {
            blorb::ExecKind::ZCode => harvest_zcode(exec.to_vec(), out),
            blorb::ExecKind::Glulx => harvest_glulx(exec.to_vec(), out),
            blorb::ExecKind::Scott => harvest_scott(exec, out),
        };
    }
    if bytes.starts_with(b"Glul") {
        return harvest_glulx(bytes, out);
    }
    if (1..=8).contains(&bytes[0]) {
        // Z-machine version byte. `Memory::new` validates the rest of the
        // header, so a false positive is refused there rather than here.
        return harvest_zcode(bytes, out);
    }
    harvest_scott(&bytes, out)
}

fn harvest_zcode(bytes: Vec<u8>, out: &mut Harvest) -> Result<(), String> {
    let mem = zvm::memory::Memory::new(bytes).map_err(|e| format!("zvm: {e:?}"))?;
    let g = zvm::grammar::Grammar::load(&mem).map_err(|e| format!("zvm grammar: {e:?}"))?;
    let n = out.absorb(g.verb_words(), g.verbs());
    if n == 0 {
        return Err("no verbs".into());
    }
    out.read += 1;
    out.by_engine[ENGINE_Z] += 1;
    Ok(())
}

fn harvest_glulx(bytes: Vec<u8>, out: &mut Harvest) -> Result<(), String> {
    let mem = gvm::Memory::new(bytes).map_err(|e| format!("gvm: {e:?}"))?;
    let g = gvm::grammar::Grammar::load(&mem).map_err(|e| format!("gvm grammar: {e:?}"))?;
    let n = out.absorb(g.verb_words(), g.verbs());
    if n == 0 {
        return Err("no verbs".into());
    }
    out.read += 1;
    out.by_engine[ENGINE_GLULX] += 1;
    Ok(())
}

fn harvest_scott(bytes: &[u8], out: &mut Harvest) -> Result<(), String> {
    // Scott databases are plain text, but the TRS-80 and C64 files in the wild
    // carry stray high bytes; decode them as Latin-1 so the lexer sees ASCII
    // where it matters.
    let text: String = bytes.iter().map(|&b| b as char).collect();
    if !scott::looks_like_scott(&text) {
        return Err("not a story this generator reads".into());
    }
    let db = scott::Database::parse(&text).map_err(|e| format!("scott: {e:?}"))?;
    let mut n = 0;
    // Scott's verb table is flat, and a `*`-prefixed entry is a synonym of the
    // nearest preceding unprefixed one (`scott::Database::match_verb`). So a
    // run of entries IS a verb entry in the Z-machine sense, and the `*` is the
    // grouping — which is why it is stripped BEFORE `plausible` rather than
    // after: `*take` starts with a character `plausible` refuses, so every
    // Scott synonym in the corpus was being dropped on the floor before
    // SQ-1115, vocabulary and grouping alike.
    let mut run: Vec<String> = Vec::new();
    for v in db.verbs.clone() {
        let canonical = !v.starts_with('*');
        // Upper case and truncated to the game's word length; what survives is
        // still the game's own spelling.
        let w = v.trim_start_matches('*').to_ascii_lowercase();
        if canonical {
            out.record_group(&run);
            run.clear();
        }
        if plausible(&w) {
            out.record(w.clone());
            n += 1;
        }
        if groupable(&w) {
            run.push(w);
        }
    }
    out.record_group(&run);
    if n == 0 {
        return Err("no verbs".into());
    }
    out.read += 1;
    out.by_engine[ENGINE_SCOTT] += 1;
    Ok(())
}
