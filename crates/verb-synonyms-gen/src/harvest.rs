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
//!   * Scott Adams — `scott::Database::verbs`, which is the verb table itself.
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
    /// The story currently being read, for [`Harvest::record`].
    story: String,
}

/// Index into [`Harvest::by_engine`] — Z-machine.
pub const ENGINE_Z: usize = 0;
/// Index into [`Harvest::by_engine`] — Glulx.
pub const ENGINE_GLULX: usize = 1;
/// Index into [`Harvest::by_engine`] — Scott Adams.
pub const ENGINE_SCOTT: usize = 2;

impl Harvest {
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
        out.story = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
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
    for v in db.verbs.clone() {
        // Scott's verb table is upper case and truncated to the game's word
        // length; what survives is still the game's own spelling.
        let w = v.to_ascii_lowercase();
        if plausible(&w) {
            out.record(w);
            n += 1;
        }
    }
    if n == 0 {
        return Err("no verbs".into());
    }
    out.read += 1;
    out.by_engine[ENGINE_SCOTT] += 1;
    Ok(())
}
