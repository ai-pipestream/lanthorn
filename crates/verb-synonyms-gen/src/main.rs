//! `verb-synonyms-gen` — rebuild the shipped player-word → IF-verb table.
//!
//! ```text
//! verb-synonyms-gen harvest --corpus stories --corpus unit_tests -o if_verbs.tsv
//! verb-synonyms-gen build --wordnet <dict> --freq <2+2+3frq.txt> \
//!     --if-verbs if_verbs.tsv -o crates/verb-synonyms/src/player_verbs.tsv
//! ```
//!
//! Argument parsing is hand-rolled rather than `clap`'d: this crate takes no
//! external dependencies, which keeps it buildable in any checkout of the
//! workspace and makes it obvious that nothing here reaches the shipped binary.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use verb_synonyms_gen::build::{self, IfVerb, Params, Report};
use verb_synonyms_gen::harvest::{self, Harvest};
use verb_synonyms_gen::sources::{Frequency, WordNet};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("harvest") => cmd_harvest(&args[1..]),
        Some("build") => cmd_build(&args[1..]),
        _ => {
            eprintln!("usage: verb-synonyms-gen <harvest|build> [options]  (see crate docs)");
            return ExitCode::FAILURE;
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("verb-synonyms-gen: {e}");
            ExitCode::FAILURE
        }
    }
}

/// One `--flag value` scan. Repeated flags collect; a missing value is an error.
fn opts(args: &[String]) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let name = match a.as_str() {
            "-o" => "out",
            _ => a
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument `{a}`"))?,
        };
        if name == "no-gap-fill" {
            m.entry(name.to_string()).or_default().push(String::new());
            i += 1;
            continue;
        }
        let v = args
            .get(i + 1)
            .ok_or_else(|| format!("`--{name}` wants a value"))?;
        m.entry(name.to_string()).or_default().push(v.clone());
        i += 2;
    }
    Ok(m)
}

fn one(m: &BTreeMap<String, Vec<String>>, k: &str) -> Result<String, String> {
    m.get(k)
        .and_then(|v| v.first())
        .cloned()
        .ok_or_else(|| format!("`--{k}` is required"))
}

fn num<T: std::str::FromStr>(
    m: &BTreeMap<String, Vec<String>>,
    k: &str,
    d: T,
) -> Result<T, String> {
    match m.get(k).and_then(|v| v.first()) {
        None => Ok(d),
        Some(s) => s
            .parse()
            .map_err(|_| format!("`--{k}` wants a number, got `{s}`")),
    }
}

// ── harvest ──────────────────────────────────────────────────────────────────

fn cmd_harvest(args: &[String]) -> Result<(), String> {
    let m = opts(args)?;
    let corpora = m
        .get("corpus")
        .ok_or("`--corpus <dir>` is required (repeatable)")?;
    let out = PathBuf::from(one(&m, "out")?);
    let wn = WordNet::load(std::path::Path::new(&one(&m, "wordnet")?))
        .map_err(|e| format!("wordnet: {e}"))?;
    let freq = Frequency::load(std::path::Path::new(&one(&m, "freq")?))
        .map_err(|e| format!("freq: {e}"))?;

    let mut h = Harvest::default();
    for dir in corpora {
        harvest::sweep(std::path::Path::new(dir), &mut h).map_err(|e| format!("{dir}: {e}"))?;
    }

    eprintln!(
        "read {} stories ({} z-machine, {} glulx, {} scott)",
        h.read,
        h.by_engine[harvest::ENGINE_Z],
        h.by_engine[harvest::ENGINE_GLULX],
        h.by_engine[harvest::ENGINE_SCOTT]
    );
    eprintln!("{} distinct verb spellings", h.verbs.len());
    let single = h.verbs.iter().filter(|w| !w.contains(' ')).count();
    eprintln!(
        "  {single} single words, {} verb+preposition phrases",
        h.verbs.len() - single
    );
    eprintln!("{} files skipped:", h.skipped.len());
    for (p, why) in &h.skipped {
        eprintln!("  {} — {why}", p.display());
    }

    let resolved = lemmatise(&h, &wn, &freq);

    let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
    writeln!(
        f,
        "# Verb vocabulary harvested from a corpus of interactive fiction."
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(f, "# Regenerate:").unwrap();
    writeln!(
        f,
        "#   verb-synonyms-gen harvest --corpus stories --corpus unit_tests \\"
    )
    .unwrap();
    writeln!(
        f,
        "#       --wordnet <WordNet-3.0/dict> --freq <12dicts/Lemmatized/2+2+3frq.txt> -o …"
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# Tab-separated: spelling, the number of stories whose parser accepts it, and"
    )
    .unwrap();
    writeln!(
        f,
        "# the WordNet verb lemma to expand it under (empty when WordNet has no verb"
    )
    .unwrap();
    writeln!(
        f,
        "# entry, which is most of the abbreviations, magic words and game-specific"
    )
    .unwrap();
    writeln!(
        f,
        "# actions).  A spelling containing a space is a verb plus a literal word from"
    )
    .unwrap();
    writeln!(
        f,
        "# one of its syntax lines — `turn on`, `pick up` — which is how English"
    )
    .unwrap();
    writeln!(
        f,
        "# lexicalises them and how a thesaurus indexes them, even though a dictionary"
    )
    .unwrap();
    writeln!(f, "# can only hold the head word.").unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# These are the STORIES' OWN spellings and are ground truth: a suggestion the"
    )
    .unwrap();
    writeln!(
        f,
        "# parser would reject is worthless.  An inflected spelling is dropped only when"
    )
    .unwrap();
    writeln!(
        f,
        "# every story that accepts it also accepts its base form."
    )
    .unwrap();
    writeln!(f, "#").unwrap();
    writeln!(
        f,
        "# {} stories read; {} spellings.",
        h.read,
        resolved.len()
    )
    .unwrap();
    for (w, stories, lemma) in &resolved {
        writeln!(f, "{w}\t{stories}\t{lemma}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Attach a WordNet lemma to each harvested spelling, and drop the inflected
/// spellings that are genuinely redundant.
///
/// The design assumes IF verbs are imperatives — you type `take lamp`, never
/// `took lamp` — and conversation does not change that, because `ask x about y`
/// and `floyd, take the card` are imperative too. Parsers that accept questions
/// are where a real inflection could appear, so this MEASURES it rather than
/// asserting it, and prints which stories each one came from.
///
/// Two things it deliberately does not do:
///
///   * It never applies a suffix rule. `dress`, `press` and `sing` end in the
///     letters of an inflection and are lemmas; only WordNet's `verb.exc` and
///     12dicts' lemmatisation get a vote.
///   * It never lemmatises a spelling WordNet lists as a lemma in its own
///     right. `saw`, `lay`, `rent`, `wound`, `fell` and `bound` all reach a base
///     form through `verb.exc` and every one of them is also a verb a player
///     types: you saw a log, you lay a rug, you rent a room. Rewriting those
///     would delete a real IF verb.
fn lemmatise(h: &Harvest, wn: &WordNet, freq: &Frequency) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let mut inflected = Vec::new();
    let mut dropped = Vec::new();
    let mut homographs = 0usize;
    for w in &h.verbs {
        let lemma = if wn.senses.contains_key(w) {
            if wn.exceptions.contains_key(w) {
                homographs += 1;
            }
            w.clone()
        } else if let Some(b) = wn.exceptions.get(w).filter(|b| wn.senses.contains_key(*b)) {
            inflected.push((w.clone(), b.clone(), "verb.exc"));
            b.clone()
        } else if let Some(b) = freq
            .lemma_of
            .get(w)
            .filter(|b| *b != w && wn.senses.contains_key(*b))
        {
            inflected.push((w.clone(), b.clone(), "12dicts"));
            b.clone()
        } else {
            out.push((w.clone(), h.story_count(w), String::new()));
            continue;
        };
        // Redundant only if every story that accepts the inflection also
        // accepts the base. Otherwise the base is not a spelling that story's
        // parser would take, and dropping this one loses a real verb.
        if lemma != *w {
            let covered = h
                .sources
                .get(&lemma)
                .is_some_and(|base| h.sources[w].iter().all(|s| base.contains(s)));
            if covered {
                dropped.push(format!("{w}→{lemma}"));
                continue;
            }
        }
        out.push((w.clone(), h.story_count(w), lemma));
    }

    eprintln!("\ninflected IF verbs — spellings WordNet knows ONLY as an inflection:");
    for (w, base, via) in &inflected {
        let from: Vec<&str> = h.sources[w].iter().map(String::as_str).take(6).collect();
        eprintln!("  {w} → {base}  ({via})  [{}]", from.join(", "));
    }
    eprintln!(
        "  {} of {} single-word spellings; a further {homographs} look inflected but are \
         lemmas in their own right and are left as the story spells them",
        inflected
            .iter()
            .filter(|(w, _, _)| !w.contains(' '))
            .count(),
        h.verbs.iter().filter(|w| !w.contains(' ')).count()
    );
    eprintln!(
        "  dropped as redundant ({}): {}",
        dropped.len(),
        dropped.join(" ")
    );
    eprintln!(
        "  kept because some story has no base form: {}",
        inflected
            .iter()
            .filter(|(w, l, _)| !dropped.contains(&format!("{w}→{l}")))
            .map(|(w, l, _)| format!("{w}(→{l})"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    out
}

// ── build ────────────────────────────────────────────────────────────────────

fn cmd_build(args: &[String]) -> Result<(), String> {
    let m = opts(args)?;
    let p = Params {
        sense_cap: num(&m, "sense-cap", Params::default().sense_cap)?,
        band_cap: num(&m, "band-cap", Params::default().band_cap)?,
        group_cap: num(&m, "group-cap", Params::default().group_cap)?,
        hyponym_cap: num(&m, "hyponym-cap", Params::default().hyponym_cap)?,
        common_bands: num(&m, "common-bands", Params::default().common_bands)?,
        gap_fill: !m.contains_key("no-gap-fill"),
    };
    let wn = WordNet::load(std::path::Path::new(&one(&m, "wordnet")?))
        .map_err(|e| format!("wordnet: {e}"))?;
    let freq = Frequency::load(std::path::Path::new(&one(&m, "freq")?))
        .map_err(|e| format!("freq: {e}"))?;

    let text = std::fs::read_to_string(one(&m, "if-verbs")?).map_err(|e| e.to_string())?;
    let mut rows = 0usize;
    let verbs: Vec<IfVerb> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            rows += 1;
            let mut f = l.split('\t');
            let emit = f.next()?.to_string();
            let stories = f.next()?.trim().parse().ok()?;
            let lemma = f.next()?.trim().to_string();
            (!lemma.is_empty()).then_some(IfVerb {
                emit,
                lemma,
                stories,
            })
        })
        .collect();

    let mut report = Report::default();
    let table = build::build(&verbs, &wn, &freq, &p, &mut report);

    let out = PathBuf::from(one(&m, "out")?);
    write_table(&out, &table, &p, rows, verbs.len())?;
    print_report(&report, &table, &p, rows, verbs.len());
    Ok(())
}

fn write_table(
    out: &std::path::Path,
    groups: &[Vec<String>],
    p: &Params,
    harvested: usize,
    expanded: usize,
) -> Result<(), String> {
    let mut f = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let members: usize = groups.iter().map(Vec::len).sum();
    let head = format!(
        "\
# Synonym groups for interactive-fiction verbs.  GENERATED; do not hand-edit.
#
# Regenerate with `verb-synonyms-gen` — see crates/verb-synonyms-gen/README.md.
# Derived from WordNet 3.0 (Princeton University) and the 12dicts 6.0.2
# lemmatized frequency list (Alan Beale, under the AGID terms).  The notices
# both licences require are in THIRD-PARTY-NOTICES.md at the repository root.
#
# FORMAT: one group per line, members separated by TABS.  There is no key
# column — every member is equal, and a word may appear in SEVERAL groups, one
# per sense.  A tab is the separator because a member may itself contain a
# space (`turn on`), and no dictionary word contains a tab.
#
# A group is one WordNet synset, filtered to words a player might type, plus —
# where a synset no story could match has a hypernym that one can — that synset
# unioned with its immediate hypernym.  Exactly one hop, never chained.
#
# AT LOOKUP, two rules define what this data means:
#   1. Lemmatise the player's word FIRST.  Members are base forms, because an IF
#      parser accepts the imperative; `illuminated` never arrives here, and if
#      the consumer skips this step a miss looks like a hole in the data instead
#      of a missing step in the caller.
#   2. Intersect the group with THIS story's dictionary and show only what
#      survives, then drop the word the player actually typed — it is in the
#      group by construction and it is the one word known to have failed.
#
# LINE ORDER IS SIGNIFICANT — DO NOT SORT THIS FILE.  The groups containing any
# given word appear in that word's own WordNet sense order, commonest sense
# first, so a consumer can walk them most-likely-meaning first and stop after
# three or four dictionary matches.  Sorting the file alphabetically destroys
# that signal silently.  Member order within a line is significant too: verbs
# the corpus actually uses come first, commonest first.
#
# Built with: sense-cap {} band-cap {} group-cap {} hyponym-cap {} gap-fill {}
# From {} harvested IF spellings, {} of which WordNet knows as verbs.
# {} groups, {} memberships.
",
        p.sense_cap,
        p.band_cap,
        p.group_cap,
        p.hyponym_cap,
        p.gap_fill,
        harvested,
        expanded,
        groups.len(),
        members,
    );
    write!(f, "{head}").map_err(|e| e.to_string())?;
    for g in groups {
        writeln!(f, "{}", g.join("\t")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn print_report(r: &Report, groups: &[Vec<String>], p: &Params, harvested: usize, expanded: usize) {
    eprintln!("harvested spellings           {harvested}");
    eprintln!("  WordNet knows as a verb     {expanded}");
    eprintln!("synsets inside the sense cap  {}", r.groups_before_prune);
    eprintln!("  …containing an IF verb      {}", r.groups_after_prune);
    eprintln!("gap-fill groups (synset ∪ hypernym)  {}", r.gap_filled);
    eprintln!("duplicate groups discarded    {}", r.duplicates);
    eprintln!("groups subsumed by another    {}", r.subsumed);
    eprintln!("sense-order constraints broken {}", r.order_conflicts);
    eprintln!("groups written                {}", groups.len());
    let members: usize = groups.iter().map(Vec::len).sum();
    eprintln!("  memberships                 {members}");
    eprintln!(
        "  mean group size             {:.2}",
        members as f64 / groups.len().max(1) as f64
    );
    eprintln!(
        "\ncoverage audit — commonest English verbs (12dicts bands 1..={})",
        p.common_bands
    );
    let n = r.common_verbs.len().max(1);
    eprintln!("  common verbs (lemmatised)   {}", r.common_verbs.len());
    eprintln!(
        "  reached by synonymy         {} ({:.1}%)",
        r.hits_synonymy,
        100.0 * r.hits_synonymy as f64 / n as f64
    );
    eprintln!(
        "  reached after gap-fill      {} ({:.1}%)",
        r.hits_total,
        100.0 * r.hits_total as f64 / n as f64
    );
    eprintln!("\nwords in the most groups (polysemy check):");
    for (w, n) in &r.widest {
        eprintln!("  {w} ({n})");
    }
    eprintln!("\nstill unreachable ({}):", r.misses.len());
    for chunk in r.misses.chunks(12) {
        eprintln!("  {}", chunk.join(" "));
    }
}
