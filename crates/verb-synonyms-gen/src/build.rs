//! Steps 2 and 3 — turn WordNet's verb synsets into the shipped table, keeping
//! only the groups a story could ever match, and audit the result against the
//! commonest English verbs.
//!
//! ## What a row is
//!
//! One line, one **synonym group**, members tab-separated:
//!
//! ```text
//! light   burn    ignite  illuminate
//! pull    draw    drag    tug
//! ```
//!
//! There is no key column. A word may sit in several groups — one per sense —
//! and that is the point: `light` is *illuminate*, *not heavy* and *a lamp*, and
//! keeping the senses apart is what stops `illuminate` from ever reaching
//! `lightweight`. This is WordNet's own structure, kept rather than flattened.
//!
//! Storing groups rather than an inverted `word → words` map also stores each
//! set once instead of once per member, and `grep illuminate` returns the whole
//! group in a single step.
//!
//! ## LINE ORDER IS SIGNIFICANT — do not sort this file
//!
//! WordNet orders a word's senses commonest-first, and that ordering is a real
//! signal: a player who types `draw` at a game that knows neither `pull` nor
//! `sketch` should be shown `pull` first, because pulling is what `draw` most
//! often means. The file preserves it — the groups containing any given word
//! appear in that word's own sense order — so the consumer can walk a word's
//! groups most-common-sense-first and stop after three or four matches, and a
//! rare fifth sense never crowds out the common first one.
//!
//! A linear file cannot always satisfy every word at once (two words can rank
//! two shared senses in opposite orders), so the order is a topological sort of
//! all the per-word constraints, with any genuine cycle broken deterministically
//! and COUNTED in the run report. Sorting this file alphabetically would destroy
//! the signal silently, which is why the header says so too.
//!
//! ## Exactly one hop, and never a transitive closure
//!
//! A group is ONE synset. The only exception is the gap-fill (below), which
//! unions a synset with its immediate hypernym — one pointer, still bounded,
//! and only where the plain synset could match nothing at all. Nothing is
//! chained further, and groups are never merged with each other. Two hops join
//! two senses through a word that carries both, and the table then makes a
//! confident wrong suggestion with nothing left in it to diagnose the mistake.
//! The next person here will be tempted to raise the depth to improve coverage;
//! that is how this table dies.
//!
//! ## What the table means at lookup
//!
//! Two rules the consumer implements, stated here because they define what the
//! data IS:
//!
//!   * **Intersect the group with THIS story's dictionary** before showing
//!     anything. The table proposes; the story disposes. A group holding
//!     `illuminate` is harmless in a game that has never heard of it — and if
//!     some game does implement `illuminate`, the same table suggests it with no
//!     regeneration, which is why members are not pre-filtered to a snapshot of
//!     the harvest.
//!   * **Drop the word the player actually typed.** It is in the group by
//!     construction and it is the one word known to have failed.
//!
//! ## Base forms, with one deliberate exception
//!
//! An IF parser accepts the imperative: you type `take lamp`, never `took lamp`,
//! and the consumer lemmatises before it looks anything up. Members are
//! therefore WordNet lemmas, which are base forms by construction. The exception
//! is a story whose own dictionary spells a verb in a form that looks inflected
//! and which no other story offers as a base — `seen`, in three Infocom
//! mysteries. That spelling is added to its lemma's groups, because the game's
//! vocabulary is ground truth: a suggestion the parser would reject is
//! worthless.
//!
//! ## The three filters, and what they are made of
//!
//! None of them is a list of words someone thought were good or bad.
//!
//! 1. **Register filter.** A member is kept only if every one of its words is a
//!    12dicts headword in band ≤ [`Params::band_cap`]. That is what removes
//!    `illume`, `enkindle` and `conflagrate` while keeping `illuminate` — a
//!    frequency judgement made by Beale's corpus, not here. A word some story's
//!    parser accepts is exempt: a player demonstrably can type it.
//! 2. **Sense cap.** WordNet orders a lemma's senses commonest-first. A synset
//!    survives only if it is among the first [`Params::sense_cap`] senses of
//!    some IF verb in it, which is how a twentieth-sense fringe meaning of a
//!    common word stays out.
//! 3. **The prune.** A group containing no harvested IF verb at all can never
//!    survive the intersection at lookup, so it is general English costing
//!    bytes for nothing.

use std::collections::{BTreeMap, BTreeSet};

use crate::sources::{Frequency, WordNet};

/// Every threshold the generation depends on, so a rebuild can be reproduced or
/// argued with from the command line.
#[derive(Debug, Clone)]
pub struct Params {
    /// A synset survives only if it is among this many senses of some IF verb
    /// in it, counting from WordNet's own commonest-first order.
    pub sense_cap: usize,
    /// The highest 12dicts frequency band a member may sit in.
    pub band_cap: u16,
    /// Discard a group with more members than this. Only the gap-fill can
    /// produce one, by unioning a synset with a very general hypernym.
    pub group_cap: usize,
    /// Bands 1..=this define "common English" for the coverage audit.
    pub common_bands: u16,
    /// Refuse to gap-fill through a hypernym with more hyponyms than this: a
    /// synset with two hundred kinds beneath it is an abstraction (`change`,
    /// `move`, `be`), not a synonym, and unioning every one of those hyponyms
    /// with it produces hundreds of groups that all say the same thing.
    pub hyponym_cap: usize,
    /// Union a synset with its immediate hypernym when the synset alone
    /// contains no IF verb. Off, the table is pure synsets.
    pub gap_fill: bool,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            sense_cap: 6,
            band_cap: 16,
            group_cap: 12,
            common_bands: 11,
            hyponym_cap: 25,
            gap_fill: true,
        }
    }
}

/// One IF verb as the harvest recorded it.
#[derive(Debug, Clone)]
pub struct IfVerb {
    /// The story's own spelling — what its parser will accept.
    pub emit: String,
    /// The WordNet verb lemma it was looked up under.
    pub lemma: String,
    /// How many stories in the corpus accept it.
    pub stories: usize,
}

/// One group as the generator holds it, before the members are written out.
struct Group {
    members: Vec<String>,
    /// The synset the group came from.
    origin: u32,
    /// The hypernym it was unioned with, for a gap-fill group.
    via: Option<u32>,
}

/// Everything the run learned, for the report and the tests.
#[derive(Default)]
pub struct Report {
    /// Synsets that passed the sense cap and left two or more members.
    pub groups_before_prune: usize,
    /// …and how many of those contain an IF verb, so survive the prune.
    pub groups_after_prune: usize,
    /// Groups whose membership another group already had, exactly.
    pub duplicates: usize,
    /// Groups dropped because another group's membership contains theirs.
    pub subsumed: usize,
    /// Groups the gap-fill produced by unioning a synset with its hypernym.
    pub gap_filled: usize,
    /// Common verbs (bands 1..=`common_bands`) after lemmatising and dedup.
    pub common_verbs: Vec<String>,
    /// Common verbs that reach a surviving group by plain synonymy.
    pub hits_synonymy: usize,
    /// …and after the gap-fill.
    pub hits_total: usize,
    /// Common verbs that reach nothing.
    pub misses: Vec<String>,
    /// Per-word sense-order constraints the linear file could not satisfy,
    /// because two words rank two shared senses in opposite orders.
    pub order_conflicts: usize,
    /// The words in the most groups, worst first — a word collecting many
    /// groups is highly polysemous, and the cheapest evidence there is that the
    /// filters are too loose.
    pub widest: Vec<(String, usize)>,
}

/// Build the groups.
pub fn build(
    verbs: &[IfVerb],
    wn: &WordNet,
    freq: &Frequency,
    p: &Params,
    report: &mut Report,
) -> Vec<Vec<String>> {
    // The stories' own spellings, indexed by the lemma they were looked up
    // under, so a synset naming `see` also offers the `seen` that three Infocom
    // mysteries insist on.
    let mut by_lemma: BTreeMap<&str, Vec<&IfVerb>> = BTreeMap::new();
    for v in verbs {
        by_lemma.entry(v.lemma.as_str()).or_default().push(v);
    }
    let stories = |w: &str| -> usize {
        by_lemma
            .get(w)
            .map_or(0, |vs| vs.iter().map(|v| v.stories).max().unwrap_or(0))
    };
    let is_if_verb = |w: &str| by_lemma.contains_key(w);

    // Which synsets are inside the sense cap of some IF verb.
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    for lemma in by_lemma.keys() {
        if let Some(senses) = wn.senses.get(*lemma) {
            wanted.extend(senses.iter().take(p.sense_cap));
        }
    }

    let member_ok = |w: &str| {
        is_if_verb(w)
            || w.split(' ')
                .all(|t| freq.band.get(t).is_some_and(|&b| b <= p.band_cap))
    };

    let common: BTreeSet<String> = common_verbs(wn, freq, p).into_iter().collect();

    let mut groups: Vec<Group> = Vec::new();
    let mut seen_sets: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut in_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_synonymy: BTreeSet<String> = BTreeSet::new();

    // ── Pass 1: one group per synset ─────────────────────────────────────────
    for (&offset, syn) in &wn.synsets {
        if !wanted.contains(&offset) {
            continue;
        }
        let members = assemble(&syn.words, &by_lemma, &member_ok, true);
        if members.len() < 2 {
            continue;
        }
        report.groups_before_prune += 1;
        if !members.iter().any(|w| is_if_verb(w)) {
            continue;
        }
        report.groups_after_prune += 1;
        for w in &members {
            by_synonymy.insert(w.clone());
        }
        keep(
            Group {
                members,
                origin: offset,
                via: None,
            },
            &mut groups,
            &mut seen_sets,
            &mut in_group,
            report,
        );
    }

    // ── Pass 2: the gap-fill ─────────────────────────────────────────────────
    //
    // A synset no story can match is dead weight — unless its immediate
    // hypernym CAN be matched, in which case the player's specific word and the
    // general verb the story knows belong together: `sprint` is a kind of `run`.
    // One pointer, only where plain synonymy reached nothing, and never
    // extended: hyponyms are not walked either, because a general word would
    // then drag in every specific verb beneath it (`move` → push, pull, turn,
    // slide, …), which is the over-inclusion this whole design exists to avoid.
    if p.gap_fill {
        for (&offset, syn) in &wn.synsets {
            let members = assemble(&syn.words, &by_lemma, &member_ok, false);
            // ONLY a synset no story can match, and this is the load-bearing
            // condition, not a coverage heuristic. A group is symmetric while
            // hypernymy is not: `sprint` is a kind of `run`, but `run` is not a
            // kind of `sprint`, and a group holding both would suggest `sprint`
            // to a player who typed `run`. Requiring that the CHILD synset
            // contains no IF verb makes that impossible by construction — none
            // of the specific words is in any story's dictionary, so the
            // intersection at lookup can never surface one. Relaxing this to
            // "any synset" was measured: it reached 92.0% instead of 88.8% and
            // put `fish`, `hook` and `net` in a group with `grab`, every one of
            // them a wrong suggestion waiting for a game that implements it.
            if members.iter().any(|w| is_if_verb(w)) {
                continue;
            }
            // And only rescue a synset a PLAYER might reach for. Gap-filling
            // every dead synset in WordNet produces thousands of groups nobody
            // will ever type a member of.
            if !members.iter().any(|w| common.contains(w)) {
                continue;
            }
            for (sym, target) in &syn.pointers {
                if sym != "@" && sym != "@i" {
                    continue;
                }
                let Some(up) = wn.synsets.get(target) else {
                    continue;
                };
                if !wanted.contains(target) {
                    continue;
                }
                if up.pointers.iter().filter(|(s, _)| s == "~").count() > p.hyponym_cap {
                    continue;
                }
                let mut union: Vec<String> = syn.words.clone();
                union.extend(up.words.iter().cloned());
                let union = assemble(&union, &by_lemma, &member_ok, false);
                if union.len() < 2
                    || union.len() > p.group_cap
                    || !union.iter().any(|w| is_if_verb(w))
                {
                    continue;
                }
                report.gap_filled += 1;
                keep(
                    Group {
                        members: union,
                        origin: offset,
                        via: Some(*target),
                    },
                    &mut groups,
                    &mut seen_sets,
                    &mut in_group,
                    report,
                );
            }
        }
    }

    // ── Drop groups another group already contains ───────────────────────────
    //
    // The gap-fill makes families of near-identical unions — a dozen sibling
    // synsets all unioned with the same hypernym. A group whose members are a
    // subset of another group's says nothing the larger one does not, and
    // costs a line. This is not merging: no group gains a member.
    {
        let sets: Vec<BTreeSet<&str>> = groups
            .iter()
            .map(|g| g.members.iter().map(String::as_str).collect())
            .collect();
        let mut drop = vec![false; groups.len()];
        let mut by_member: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, s) in sets.iter().enumerate() {
            for w in s {
                by_member.entry(w).or_default().push(i);
            }
        }
        for (i, s) in sets.iter().enumerate() {
            let Some(anchor) = s.iter().next() else {
                continue;
            };
            for &j in &by_member[*anchor] {
                if i != j
                    && !drop[j]
                    && (sets[j].len() > s.len() || (sets[j].len() == s.len() && j < i))
                    && s.is_subset(&sets[j])
                {
                    drop[i] = true;
                    report.subsumed += 1;
                    break;
                }
            }
        }
        let mut i = 0;
        groups.retain(|_| {
            i += 1;
            !drop[i - 1]
        });
    }

    // ── Order the members ────────────────────────────────────────────────────
    //
    // Verbs the corpus actually uses first, commonest first, so the leading
    // members of a line are the likeliest suggestions and the file diffs
    // stably.
    for g in &mut groups {
        g.members.sort_by(|a, b| {
            is_if_verb(b)
                .cmp(&is_if_verb(a))
                .then(stories(b).cmp(&stories(a)))
                .then(a.cmp(b))
        });
    }

    let groups = order_by_sense(groups, wn, report);
    audit(&groups, &by_synonymy, wn, freq, p, report);
    report.widest.extend(
        in_group
            .iter()
            .map(|(w, n)| (w.clone(), *n))
            .filter(|(_, n)| *n > 1),
    );
    report
        .widest
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.widest.truncate(15);
    groups
}

/// Filter a synset's words to those that may appear as members, and add the
/// stories' own spellings for any lemma among them.
fn assemble(
    words: &[String],
    by_lemma: &BTreeMap<&str, Vec<&IfVerb>>,
    member_ok: &impl Fn(&str) -> bool,
    story_spellings: bool,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for w in words {
        if !member_ok(w) {
            continue;
        }
        out.insert(w.clone());
        if story_spellings {
            for v in by_lemma.get(w.as_str()).map_or(&[][..], Vec::as_slice) {
                out.insert(v.emit.clone());
            }
        }
    }
    out.into_iter().collect()
}

/// Record a group unless an identical one is already there.
fn keep(
    g: Group,
    groups: &mut Vec<Group>,
    seen: &mut BTreeSet<Vec<String>>,
    in_group: &mut BTreeMap<String, usize>,
    report: &mut Report,
) {
    if !seen.insert(g.members.clone()) {
        report.duplicates += 1;
        return;
    }
    for w in &g.members {
        *in_group.entry(w.clone()).or_default() += 1;
    }
    groups.push(g);
}

/// Put the groups in an order that, for every word, presents that word's groups
/// in WordNet's own sense order — commonest sense first.
///
/// Each word contributes a chain of "this group before that one" constraints
/// over the groups it belongs to. Satisfying all of them at once is a
/// topological sort; two words CAN rank two shared senses in opposite orders,
/// which makes a cycle, so the sort is Kahn's algorithm with an alphabetical
/// tie-break and any surviving cycle broken by taking the alphabetically first
/// remaining group. Every constraint broken that way is counted, because a large
/// number would mean the ordering claim in the file header is not worth much.
fn order_by_sense(groups: Vec<Group>, wn: &WordNet, report: &mut Report) -> Vec<Vec<String>> {
    // For each word, its groups in that word's sense order.
    let mut by_word: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, g) in groups.iter().enumerate() {
        for w in &g.members {
            let senses = wn.senses.get(w.as_str()).map_or(&[][..], Vec::as_slice);
            let rank = senses
                .iter()
                .position(|o| *o == g.origin)
                .or_else(|| g.via.and_then(|v| senses.iter().position(|o| *o == v)))
                .unwrap_or(usize::MAX);
            by_word.entry(w.as_str()).or_default().push((rank, i));
        }
    }

    let n = groups.len();
    let mut edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for chain in by_word.values_mut() {
        chain.sort();
        for pair in chain.windows(2) {
            if pair[0].1 != pair[1].1 {
                edges.insert((pair[0].1, pair[1].1));
            }
        }
    }

    let mut indegree = vec![0usize; n];
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in &edges {
        out[a].push(b);
        indegree[b] += 1;
    }
    // Deterministic tie-break: the group's first member, then its offset.
    let mut key: Vec<(&str, usize)> = Vec::with_capacity(n);
    for g in &groups {
        key.push((
            g.members.first().map_or("", String::as_str),
            g.origin as usize,
        ));
    }
    let mut ready: BTreeSet<(&str, usize, usize)> = (0..n)
        .filter(|&i| indegree[i] == 0)
        .map(|i| (key[i].0, key[i].1, i))
        .collect();
    let mut remaining: BTreeSet<(&str, usize, usize)> = (0..n)
        .filter(|&i| indegree[i] != 0)
        .map(|i| (key[i].0, key[i].1, i))
        .collect();
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = match ready.iter().next().copied() {
            Some(x) => {
                ready.remove(&x);
                x
            }
            None => {
                // A cycle: two words disagree about which sense comes first.
                let x = *remaining.iter().next().expect("nodes remain");
                remaining.remove(&x);
                report.order_conflicts += indegree[x.2];
                indegree[x.2] = 0;
                x
            }
        };
        order.push(next.2);
        for &b in &out[next.2] {
            // Saturating because breaking a cycle zeroes a node's indegree
            // while its predecessors still hold edges to it.
            indegree[b] = indegree[b].saturating_sub(1);
            if indegree[b] == 0 && remaining.remove(&(key[b].0, key[b].1, b)) {
                ready.insert((key[b].0, key[b].1, b));
            }
        }
    }

    let mut members: Vec<Option<Vec<String>>> =
        groups.into_iter().map(|g| Some(g.members)).collect();
    order
        .into_iter()
        .map(|i| members[i].take().expect("each group once"))
        .collect()
}

/// How much of the commonest English verb vocabulary the table reaches.
///
/// This is the quality metric for the whole exercise: a table that misses the
/// words players actually reach for is not useful however many rows it has.
fn audit(
    groups: &[Vec<String>],
    by_synonymy: &BTreeSet<String>,
    wn: &WordNet,
    freq: &Frequency,
    p: &Params,
    report: &mut Report,
) {
    let all: BTreeSet<&str> = groups.iter().flatten().map(String::as_str).collect();
    let common = common_verbs(wn, freq, p);
    report.hits_synonymy = common
        .iter()
        .filter(|w| by_synonymy.contains(*w) && all.contains(w.as_str()))
        .count();
    report.hits_total = common.iter().filter(|w| all.contains(w.as_str())).count();
    report.misses = common
        .iter()
        .filter(|w| !all.contains(w.as_str()))
        .cloned()
        .collect();
    report.common_verbs = common;
}

/// The commonest English verbs: 12dicts bands 1..=`common_bands`, lemmatised,
/// deduplicated, and reduced to those WordNet knows as verbs.
///
/// Lemmatising BEFORE the dedup is what makes the count honest — `go`, `going`
/// and `went` are three entries for one verb, and counting them separately gets
/// the hit rate wrong in both directions.
fn common_verbs(wn: &WordNet, freq: &Frequency, p: &Params) -> Vec<String> {
    let mut common = Vec::new();
    let mut seen = BTreeSet::new();
    for w in freq.top(p.common_bands) {
        let lemma = freq.lemma_of.get(w).map_or(w, String::as_str);
        let lemma = if wn.senses.contains_key(lemma) {
            lemma.to_string()
        } else if let Some(b) = wn
            .exceptions
            .get(lemma)
            .filter(|b| wn.senses.contains_key(*b))
        {
            b.clone()
        } else {
            continue; // not a verb at all: a noun, adjective or function word.
        };
        if seen.insert(lemma.clone()) {
            common.push(lemma);
        }
    }
    common
}
