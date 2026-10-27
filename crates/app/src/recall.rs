//! Local recall of observed story output, with BM25 and sentence embeddings.
//!
//! The only source is an owned snapshot of the visible transcript. Commands
//! supply context for their responses; they never become standalone advice.
//! Nothing here reads the engine, its dictionary, a walkthrough, or a save.
//! All indexing and model work happens on one lazy background thread. Both
//! mailboxes hold just one item, and newer requests supersede older requests.
//!
//! Integration: submit `RecallSnapshot::from_state(state)`, poll each tick,
//! then use `reply.visible_matches(state)` as the ranked transcript-search
//! positions. Cancel on restore, reset, game switch, or leaving recall. Never
//! append results to the story transcript. The snapshot guard also rejects
//! changed text, including a same-length replacement, before positions apply.

mod embedding;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::state::{AppState, TranscriptKind};

const CHUNK_CHARS: usize = 480;
const CHUNK_WORDS: usize = 100;
const OVERLAP_WORDS: usize = 16;
const COMMAND_CHARS: usize = 160;
const EMBED_BATCH: usize = 8;
const MAX_QUERY_CHARS: usize = 1024;
const MAX_HITS: usize = 20;
const RANK_DEPTH: usize = 60;
const RRF_K: f32 = 60.0;
const LOAD_RETRY_DELAY: Duration = Duration::from_secs(30);
// A relevance floor, never a probability. On the pinned MiniLM calibration
// corpus (24 paraphrases, 8 unrelated queries), the weakest target scored
// .1923 and the strongest unrelated match .1493. .17 lies between them with
// margin on both sides. This is empirical abstention, not a universal claim
// of relevance: hits remain inspectable original excerpts, never answers.
const MIN_COSINE: f32 = 0.17;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedLine {
    raw_line: usize,
    text: String,
    kind: TranscriptKind,
}

/// Immutable search input. This captures transcript text, never VM state.
#[derive(Clone, Debug)]
pub struct RecallSnapshot {
    game_dir: PathBuf,
    edits: u64,
    lines: Arc<[ObservedLine]>,
    // Host inline mode appends a command onto a Story prompt instead of making
    // an Input row. Only identify it as a command when observed command history
    // corroborates it. Unknown '>' prose is always retained as story text.
    commands: HashSet<String>,
}

impl RecallSnapshot {
    /// Copy the same filtered transcript the player can browse, retaining only
    /// Story and Input. Missing old kind sidecars follow the renderer's Story
    /// fallback. Meta/Warning/Assist are excluded even with the Both filter.
    pub fn from_state(state: &AppState) -> Self {
        let lines = state
            .visible_transcript_indices()
            .into_iter()
            .filter_map(|raw_line| {
                let kind = state
                    .transcript_kinds
                    .get(raw_line)
                    .copied()
                    .unwrap_or(TranscriptKind::Story);
                matches!(kind, TranscriptKind::Story | TranscriptKind::Input).then(|| {
                    ObservedLine {
                        raw_line,
                        text: state.transcript[raw_line].clone(),
                        kind,
                    }
                })
            })
            .collect::<Vec<_>>();
        let commands = state
            .command_history
            .iter()
            .map(|command| command.trim().to_lowercase())
            .chain(
                state
                    .history
                    .iter()
                    .map(|turn| turn.command.trim().to_lowercase()),
            )
            .collect();
        Self {
            game_dir: state.game_dir.clone(),
            edits: state.transcript_edits,
            lines: lines.into(),
            commands,
        }
    }

    /// Compare actual source text as well as provenance. Meta appends do not
    /// invalidate results, but edits, appended story output and filter changes
    /// that hide evidence do. Callers still cancel on session transitions, even
    /// when a restored transcript is byte-for-byte identical to the old one.
    pub fn matches_state(&self, state: &AppState) -> bool {
        if self.game_dir != state.game_dir || self.edits != state.transcript_edits {
            return false;
        }
        let mut observed = self.lines.iter();
        for raw_line in state.visible_transcript_indices() {
            let kind = state
                .transcript_kinds
                .get(raw_line)
                .copied()
                .unwrap_or(TranscriptKind::Story);
            if !matches!(kind, TranscriptKind::Story | TranscriptKind::Input) {
                continue;
            }
            let Some(old) = observed.next() else {
                return false;
            };
            if old.raw_line != raw_line
                || old.kind != kind
                || old.text != state.transcript[raw_line]
            {
                return false;
            }
        }
        observed.next().is_none()
    }

    /// Chunk whole command responses, keeping every tail and each chunk's raw
    /// source line. Identical response/context pairs keep their newest source.
    pub fn passages(&self) -> Vec<RecallPassage> {
        let mut passages = Vec::new();
        let mut command = None;
        let mut words = Vec::new();
        for line in self.lines.iter() {
            let inline_prompt = line.text.trim_start().strip_prefix('>');
            let is_recorded_command = inline_prompt.is_some_and(|input| {
                !input.trim().is_empty() && self.commands.contains(&input.trim().to_lowercase())
            });
            if line.kind == TranscriptKind::Input {
                flush_passages(&mut words, command.as_deref(), &mut passages);
                let input = line.text.trim().trim_start_matches('>').trim();
                command = (!input.is_empty() && !input.starts_with('/'))
                    .then(|| input.chars().take(COMMAND_CHARS).collect::<String>());
            } else {
                // Even an old transcript with no command history can separate
                // prompt-shaped lines into turns. Do not claim these are user
                // commands: preserve their full text and clear old context.
                if inline_prompt.is_some() {
                    flush_passages(&mut words, command.as_deref(), &mut passages);
                    command = is_recorded_command.then(|| {
                        inline_prompt
                            .unwrap()
                            .trim()
                            .chars()
                            .take(COMMAND_CHARS)
                            .collect()
                    });
                }
                for word in line.text.split_whitespace() {
                    // A single unbroken URL, identifier, or CJK line must not
                    // bypass the size bound or lose its suffix.
                    let mut start = 0;
                    for (n, (offset, _)) in word.char_indices().enumerate() {
                        if n != 0 && n % CHUNK_CHARS == 0 {
                            words.push((line.raw_line, word[start..offset].to_owned()));
                            start = offset;
                        }
                    }
                    words.push((line.raw_line, word[start..].to_owned()));
                }
            }
        }
        flush_passages(&mut words, command.as_deref(), &mut passages);
        let mut positions = HashMap::new();
        let mut deduped: Vec<RecallPassage> = Vec::new();
        for passage in passages {
            let key = (passage.command.clone(), passage.text.clone());
            if let Some(&index) = positions.get(&key) {
                deduped[index] = passage;
            } else {
                positions.insert(key, deduped.len());
                deduped.push(passage);
            }
        }
        deduped.sort_by_key(|p| p.raw_line);
        deduped
    }
}

/// One observed response fragment. The command is context, not a suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecallPassage {
    pub raw_line: usize,
    pub text: String,
    pub command: Option<String>,
}

impl RecallPassage {
    fn lexical_text(&self) -> String {
        match &self.command {
            Some(_) if self.inline_response().is_some() => self.text.clone(),
            Some(command) => format!("{command}\n{}", self.text),
            None => self.text.clone(),
        }
    }

    fn search_text(&self) -> String {
        // Embed the observed prose itself. Boilerplate and routine commands
        // such as 'examine cabinet' pull MiniLM toward the interaction format
        // and away from the clue: measured on the acceptance corpus, decorating
        // the lantern passage put the dark-place paraphrase below the floor.
        // Command context remains available to BM25 and in the result excerpt.
        self.inline_response().unwrap_or(&self.text).to_owned()
    }

    /// In the default inline host, the prompt is a Story row. Keep that entire
    /// row in the source and lexical index, but do not add its command to the
    /// sentence vector just because the display mode changed. Only a prefix
    /// corroborated by observed command history is eligible, and only when
    /// actual response text follows. Unknown or standalone '>' prose survives.
    fn inline_response(&self) -> Option<&str> {
        let command = self
            .command
            .as_ref()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let after_prompt = self.text.trim_start().strip_prefix('>')?.trim_start();
        let after_command = after_prompt.strip_prefix(&command)?;
        if !after_command.starts_with(char::is_whitespace) || after_command.trim().is_empty() {
            return None;
        }
        Some(after_command.trim_start())
    }

    fn excerpt(&self) -> String {
        match &self.command {
            // Inline Story prompts remain source text, including a quotation
            // that happens to match command history. Do not print one twice.
            Some(command)
                if self
                    .text
                    .trim_start()
                    .strip_prefix('>')
                    .is_some_and(|text| text.trim_start().starts_with(command)) =>
            {
                self.text.clone()
            }
            Some(command) => format!("> {command}\n{}", self.text),
            None => self.text.clone(),
        }
    }
}

fn flush_passages(
    words: &mut Vec<(usize, String)>,
    command: Option<&str>,
    out: &mut Vec<RecallPassage>,
) {
    let mut start = 0;
    while start < words.len() {
        let mut end = start;
        let mut chars = 0;
        while end < words.len() && end - start < CHUNK_WORDS {
            let next = words[end].1.chars().count() + usize::from(end > start);
            if end > start && chars + next > CHUNK_CHARS {
                break;
            }
            chars += next;
            end += 1;
        }
        out.push(RecallPassage {
            raw_line: words[start].0,
            text: words[start..end]
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            command: command.map(str::to_owned),
        });
        if end == words.len() {
            break;
        }
        // Always make forward progress, even for a one-word oversized fragment.
        start = end.saturating_sub(OVERLAP_WORDS).max(start + 1);
    }
    words.clear();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecallStatus {
    /// Both retrieval channels completed with the real sentence model.
    Hybrid,
    /// Results are lexical only; never describe this as semantic retrieval.
    KeywordOnly { reason: String },
    /// No story passages or no searchable query. No model was needed.
    Empty,
}

#[derive(Clone, Debug)]
pub struct RecallHit {
    /// Index in AppState::transcript, not in its filtered visible list.
    pub raw_line: usize,
    /// Original response words, with any associated command explicitly marked.
    pub excerpt: String,
    /// Reciprocal-rank fusion score, not a confidence or probability.
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct RecallReply {
    pub request_id: u64,
    pub query: String,
    pub hits: Vec<RecallHit>,
    pub status: RecallStatus,
    snapshot: RecallSnapshot,
}

impl RecallReply {
    pub fn matches_state(&self, state: &AppState) -> bool {
        self.snapshot.matches_state(state)
    }

    /// Return ranked visible-list positions for the existing search view.
    /// None means stale; an empty vector is a valid search with no matches.
    pub fn visible_matches(&self, state: &AppState) -> Option<Vec<usize>> {
        if !self.matches_state(state) {
            return None;
        }
        let visible = state.visible_transcript_indices();
        self.hits
            .iter()
            .map(|hit| visible.binary_search(&hit.raw_line).ok())
            .collect()
    }
}

struct Request {
    id: u64,
    query: String,
    snapshot: RecallSnapshot,
}

#[derive(Default)]
struct Mailbox {
    pending: Option<Request>,
    reply: Option<RecallReply>,
    shutdown: bool,
}

#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    latest: AtomicU64,
}

fn lock(shared: &Shared) -> MutexGuard<'_, Mailbox> {
    shared
        .mailbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Session-owned, lazy worker. Drop/cancel never join a model download thread.
/// Keep one instance per live game; it owns no global transcript or vector data.
#[derive(Default)]
pub struct RecallWorker {
    shared: Option<Arc<Shared>>,
}

impl std::fmt::Debug for RecallWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallWorker")
            .field("started", &self.shared.is_some())
            .finish()
    }
}

impl RecallWorker {
    /// Queue a user-initiated search. At most one request is in progress and one
    /// is pending. Superseded requests are checked between embedding batches.
    pub fn submit(&mut self, snapshot: RecallSnapshot, query: &str) -> Result<u64, String> {
        let query = query.trim();
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(format!(
                "Recall queries are limited to {MAX_QUERY_CHARS} characters."
            ));
        }
        if self.shared.is_none() {
            self.start(RealEncoder::default)?;
        }
        let shared = self.shared.as_ref().expect("worker just started");
        let id = shared.latest.fetch_add(1, Ordering::AcqRel) + 1;
        let mut mailbox = lock(shared);
        mailbox.reply = None;
        mailbox.pending = Some(Request {
            id,
            query: query.to_owned(),
            snapshot,
        });
        drop(mailbox);
        shared.wake.notify_one();
        Ok(id)
    }

    /// Nonblocking result pickup. Old queries are rejected even if they finish
    /// during a newer submit or cancellation.
    pub fn poll(&mut self) -> Option<RecallReply> {
        let shared = self.shared.as_ref()?;
        let reply = lock(shared).reply.take()?;
        (reply.request_id == shared.latest.load(Ordering::Acquire)).then_some(reply)
    }

    pub fn cancel(&mut self) {
        if let Some(shared) = &self.shared {
            shared.latest.fetch_add(1, Ordering::AcqRel);
            let mut mailbox = lock(shared);
            mailbox.pending = None;
            mailbox.reply = None;
        }
    }

    fn start<E, F>(&mut self, factory: F) -> Result<(), String>
    where
        E: Encoder + 'static,
        F: FnOnce() -> E + Send + 'static,
    {
        let shared = Arc::new(Shared::default());
        let background = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("recall".into())
            .spawn(move || {
                run_worker(background, factory());
            })
            .map_err(|error| format!("Could not start recall worker: {error}"))?;
        self.shared = Some(shared);
        Ok(())
    }
}

impl Drop for RecallWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(shared) = &self.shared {
            lock(shared).shutdown = true;
            shared.wake.notify_one();
        }
    }
}

trait Encoder {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

#[derive(Default)]
struct RealEncoder {
    model: Option<embedding::SentenceEmbedder>,
    load_failure: Option<(Instant, String)>,
}

impl RealEncoder {
    fn cached_failure(&self, now: Instant) -> Option<String> {
        let (failed_at, reason) = self.load_failure.as_ref()?;
        let remaining = LOAD_RETRY_DELAY.checked_sub(now.saturating_duration_since(*failed_at))?;
        if remaining.is_zero() {
            return None;
        }
        Some(format!(
            "{reason}; model loading will retry after {} seconds.",
            remaining.as_secs().saturating_add(1)
        ))
    }
}

impl Encoder for RealEncoder {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if self.model.is_none() {
            if let Some(reason) = self.cached_failure(Instant::now()) {
                return Err(reason);
            }
            match embedding::SentenceEmbedder::load() {
                Ok(model) => {
                    self.model = Some(model);
                    self.load_failure = None;
                }
                Err(reason) => {
                    self.load_failure = Some((Instant::now(), reason.clone()));
                    return Err(format!(
                        "{reason}; model loading will retry after 30 seconds."
                    ));
                }
            }
        }
        self.model.as_mut().expect("model just loaded").embed(texts)
    }
}

fn run_worker<E: Encoder>(shared: Arc<Shared>, mut encoder: E) {
    let mut cache = HashMap::new();
    loop {
        let request = {
            let mut mailbox = lock(&shared);
            while mailbox.pending.is_none() && !mailbox.shutdown {
                mailbox = shared
                    .wake
                    .wait(mailbox)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if mailbox.shutdown {
                return;
            }
            mailbox.pending.take().expect("pending request")
        };
        let current = || shared.latest.load(Ordering::Acquire) == request.id;
        let Some(reply) = retrieve(&request, &mut encoder, &mut cache, &current) else {
            continue;
        };
        let mut mailbox = lock(&shared);
        if current() && !mailbox.shutdown {
            mailbox.reply = Some(reply);
        }
    }
}

type VectorCache = HashMap<String, Arc<[f32]>>;

fn retrieve<E: Encoder>(
    request: &Request,
    encoder: &mut E,
    cache: &mut VectorCache,
    current: &impl Fn() -> bool,
) -> Option<RecallReply> {
    if !current() {
        return None;
    }
    let passages = request.snapshot.passages();
    let query_tokens = tokenize(&request.query);
    let reply = |hits, status| RecallReply {
        request_id: request.id,
        query: request.query.clone(),
        hits,
        status,
        snapshot: request.snapshot.clone(),
    };
    if passages.is_empty() || query_tokens.is_empty() {
        cache.clear();
        return Some(reply(Vec::new(), RecallStatus::Empty));
    }
    let texts = passages
        .iter()
        .map(RecallPassage::search_text)
        .collect::<Vec<_>>();
    // A cache contains only this corpus, never a growing history of old games
    // or discarded timelines. Reusing a vector never reuses a source position.
    let wanted: HashSet<&str> = texts.iter().map(String::as_str).collect();
    cache.retain(|text, _| wanted.contains(text.as_str()));
    let lexical_texts = passages
        .iter()
        .map(RecallPassage::lexical_text)
        .collect::<Vec<_>>();
    let lexical = lexical_ranking(&lexical_texts, &query_tokens);
    let semantic = semantic_ranking(&request.query, &texts, encoder, cache, current);
    if !current() {
        return None;
    }
    let (semantic, status) = match semantic {
        Ok(ranking) => (ranking, RecallStatus::Hybrid),
        Err(reason) => (Vec::new(), RecallStatus::KeywordOnly { reason }),
    };
    Some(reply(fuse(&passages, &lexical, &semantic), status))
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Exact Okapi BM25 (k1=1.2, b=.75) on complete token frequencies, with
/// positive Robertson IDF. No stemming changes a story's object names.
fn lexical_ranking(texts: &[String], query: &[String]) -> Vec<(usize, f32)> {
    // Question grammar is not lexical evidence: "where was there food" must
    // not promote an unrelated passage merely because it says "was". Keep a
    // literal-only query such as "in" usable, and leave the original query
    // untouched for embeddings. No document tokens or BM25 constants change.
    let mut query_terms = query
        .iter()
        .map(String::as_str)
        .filter(|word| !question_word(word))
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        query_terms.extend(query.iter().map(String::as_str));
    }
    query_terms.sort_unstable();
    query_terms.dedup();
    let documents = texts.iter().map(|text| tokenize(text)).collect::<Vec<_>>();
    let average =
        documents.iter().map(Vec::len).sum::<usize>() as f32 / documents.len().max(1) as f32;
    let frequencies = query_terms
        .iter()
        .map(|term| {
            documents
                .iter()
                .filter(|doc| doc.iter().any(|word| word == term))
                .count()
        })
        .collect::<Vec<_>>();
    let mut ranked = Vec::new();
    for (index, doc) in documents.iter().enumerate() {
        let mut score = 0.0;
        for (term, &df) in query_terms.iter().zip(&frequencies) {
            let tf = doc.iter().filter(|word| word.as_str() == *term).count() as f32;
            if tf == 0.0 {
                continue;
            }
            let idf = (1.0 + (documents.len() as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln();
            let norm = 1.2 * (0.25 + 0.75 * doc.len() as f32 / average.max(1.0));
            score += idf * tf * 2.2 / (tf + norm);
        }
        if score > 0.0 {
            ranked.push((index, score));
        }
    }
    sort_scores(&mut ranked);
    ranked
}

fn question_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "of"
            | "to"
            | "in"
            | "on"
            | "at"
            | "by"
            | "for"
            | "from"
            | "with"
            | "as"
            | "is"
            | "am"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "do"
            | "does"
            | "did"
            | "have"
            | "has"
            | "had"
            | "i"
            | "me"
            | "my"
            | "we"
            | "our"
            | "you"
            | "your"
            | "it"
            | "its"
            | "this"
            | "that"
            | "these"
            | "those"
            | "what"
            | "which"
            | "who"
            | "where"
            | "when"
            | "why"
            | "how"
            | "can"
            | "could"
            | "would"
            | "should"
            | "might"
            | "there"
            | "here"
    )
}

fn sort_scores(ranking: &mut [(usize, f32)]) {
    ranking.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
}

fn normalize(vector: Vec<f32>, dimension: Option<usize>) -> Result<Arc<[f32]>, String> {
    if vector.is_empty()
        || dimension.is_some_and(|n| n != vector.len())
        || vector.iter().any(|x| !x.is_finite())
    {
        return Err("Sentence model returned an invalid embedding shape or value.".into());
    }
    let norm = vector
        .iter()
        .map(|&x| f64::from(x).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err("Sentence model returned an empty embedding.".into());
    }
    Ok(vector
        .into_iter()
        .map(|x| (f64::from(x) / norm) as f32)
        .collect::<Vec<_>>()
        .into())
}

fn semantic_ranking<E: Encoder>(
    query: &str,
    texts: &[String],
    encoder: &mut E,
    cache: &mut VectorCache,
    current: &impl Fn() -> bool,
) -> Result<Vec<(usize, f32)>, String> {
    if !current() {
        return Ok(Vec::new());
    }
    let mut query_vectors = encoder.embed(&[query.to_owned()])?;
    if query_vectors.len() != 1 {
        return Err("Sentence model returned the wrong number of query embeddings.".into());
    }
    let query_vector = normalize(query_vectors.remove(0), None)?;
    let mut seen = HashSet::new();
    let missing = texts
        .iter()
        .filter(|text| !cache.contains_key(*text) && seen.insert(text.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for batch in missing.chunks(EMBED_BATCH) {
        if !current() {
            return Ok(Vec::new());
        }
        let vectors = encoder.embed(batch)?;
        if vectors.len() != batch.len() {
            return Err("Sentence model returned the wrong number of passage embeddings.".into());
        }
        for (text, vector) in batch.iter().zip(vectors) {
            cache.insert(text.clone(), normalize(vector, Some(query_vector.len()))?);
        }
    }
    let mut ranking = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let vector = &cache[text];
        if vector.len() != query_vector.len() {
            cache.clear();
            return Err("Sentence model changed embedding dimensions; please retry recall.".into());
        }
        let cosine = query_vector
            .iter()
            .zip(vector.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>();
        if cosine >= MIN_COSINE {
            ranking.push((index, cosine));
        }
    }
    sort_scores(&mut ranking);
    Ok(ranking)
}

fn fuse(
    passages: &[RecallPassage],
    lexical: &[(usize, f32)],
    semantic: &[(usize, f32)],
) -> Vec<RecallHit> {
    let mut scores = vec![0.0; passages.len()];
    for ranking in [lexical, semantic] {
        for (rank, &(index, _)) in ranking.iter().take(RANK_DEPTH).enumerate() {
            scores[index] += 1.0 / (RRF_K + rank as f32 + 1.0);
        }
    }
    let mut ranking = scores
        .into_iter()
        .enumerate()
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    sort_scores(&mut ranking);
    let mut source_lines = HashSet::new();
    ranking
        .into_iter()
        .filter(|(index, _)| source_lines.insert(passages[*index].raw_line))
        .take(MAX_HITS)
        .map(|(index, score)| {
            let passage = &passages[index];
            RecallHit {
                raw_line: passage.raw_line,
                excerpt: passage.excerpt(),
                score,
            }
        })
        .collect()
}

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;
    use crate::state::TranscriptFilter;
    use std::sync::mpsc;

    fn state(lines: &[(&str, TranscriptKind)]) -> AppState {
        let mut state = AppState::default();
        for &(text, kind) in lines {
            state.push_transcript_kind(text, kind);
        }
        state
    }

    fn request(state: &AppState, query: &str) -> Request {
        Request {
            id: 1,
            query: query.into(),
            snapshot: RecallSnapshot::from_state(state),
        }
    }

    struct Offline;
    impl Encoder for Offline {
        fn embed(&mut self, _: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Err("model unavailable offline".into())
        }
    }

    #[derive(Default)]
    struct FakeEncoder {
        calls: Vec<Vec<String>>,
    }
    impl Encoder for FakeEncoder {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            self.calls.push(texts.to_vec());
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("lantern") || text.contains("illumination") {
                        vec![1.0, 0.0, 0.0]
                    } else if text.contains("key") {
                        vec![0.0, 1.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 1.0]
                    }
                })
                .collect())
        }
    }

    #[test]
    fn extraction_uses_only_visible_observed_story_and_input_context() {
        use TranscriptKind::*;
        let mut state = state(&[
            ("Walkthrough: secret trapdoor", Meta),
            ("Try the secret password", Assist),
            ("secret diagnostic", Warning),
            ("> unlock door with key", Input),
            ("The key doesn't fit this lock.", Story),
            ("> open chest", Input),
        ]);
        let passages = RecallSnapshot::from_state(&state).passages();
        assert_eq!(passages.len(), 1, "bare command is not evidence or advice");
        assert_eq!(passages[0].raw_line, 4);
        assert_eq!(passages[0].command.as_deref(), Some("unlock door with key"));
        assert_eq!(passages[0].text, "The key doesn't fit this lock.");
        assert!(!passages[0].search_text().contains("secret"));
        state.transcript_filter = TranscriptFilter::Meta;
        assert!(RecallSnapshot::from_state(&state).passages().is_empty());
    }

    #[test]
    fn old_transcript_without_kind_sidecars_matches_the_renderer() {
        let mut state = AppState::default();
        state.transcript.push("An observed lantern.".into());
        assert_eq!(
            RecallSnapshot::from_state(&state).passages()[0].text,
            "An observed lantern."
        );
    }

    #[test]
    fn default_inline_host_echo_keeps_failed_command_with_its_response() {
        // The production inline path in host::turn appends the command to the
        // game's Story prompt, without creating an Input row.
        let mut state = AppState::default();
        assert!(!state.config.command_bar);
        state.push_transcript("A small chamber.\n>");
        state.record_command("unlock door with key");
        state.append_to_last_transcript_line("unlock door with key");
        state.push_transcript("The key does not fit. The door remains locked.\n>");
        state.record_command("north");
        state.append_to_last_transcript_line("north");
        state.push_transcript("A courtyard with a fountain.");
        assert!(state
            .transcript_kinds
            .iter()
            .all(|kind| *kind == TranscriptKind::Story));
        let passages = RecallSnapshot::from_state(&state).passages();
        let failed = passages
            .iter()
            .find(|p| p.text.contains("does not fit"))
            .unwrap();
        assert_eq!(failed.command.as_deref(), Some("unlock door with key"));
        assert!(!failed.text.contains("courtyard"));
        let courtyard = passages
            .iter()
            .find(|p| p.text.contains("courtyard"))
            .unwrap();
        assert_eq!(courtyard.command.as_deref(), Some("north"));
    }

    #[test]
    fn unverified_prompt_shaped_narrative_is_never_discarded_or_called_a_command() {
        use TranscriptKind::*;
        let state = state(&[
            ("> look", Input),
            ("A sign reads:", Story),
            ("> The king is a liar", Story),
            ("The words are scratched into stone.", Story),
            (">north", Story),
            ("A courtyard.", Story),
        ]);
        let passages = RecallSnapshot::from_state(&state).passages();
        let quote = passages
            .iter()
            .find(|p| p.text.contains("king is a liar"))
            .unwrap();
        assert!(quote.text.starts_with("> The king is a liar"));
        assert_eq!(quote.command, None);
        let restored_inline = passages
            .iter()
            .find(|p| p.text.contains("courtyard"))
            .unwrap();
        assert!(restored_inline.text.starts_with(">north"));
        assert_eq!(restored_inline.command, None);
    }

    #[test]
    fn model_load_error_backoff_expires_without_network_or_sleep() {
        let now = Instant::now();
        let encoder = RealEncoder {
            model: None,
            load_failure: Some((now, "offline".into())),
        };
        assert!(encoder
            .cached_failure(now + Duration::from_secs(1))
            .unwrap()
            .contains("retry after 30 seconds"));
        assert!(encoder
            .cached_failure(now + Duration::from_secs(29))
            .unwrap()
            .contains("retry after 2 seconds"));
        assert!(encoder.cached_failure(now + LOAD_RETRY_DELAY).is_none());
        assert!(encoder
            .cached_failure(now + LOAD_RETRY_DELAY + Duration::from_secs(1))
            .is_none());
    }

    #[test]
    fn quoted_prompt_matching_command_history_and_nonstandard_prompts_keep_all_story_text() {
        let mut state = AppState::default();
        state.record_command("look");
        state.push_transcript("On the ancient sign is written:\n> look\nIt is a quotation, not a command.\nWhat now? eat mushroom\nYou cannot eat that.");
        let passages = RecallSnapshot::from_state(&state).passages();
        let text = passages
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for line in &state.transcript {
            assert!(text.contains(line), "lost story line {line:?}");
        }
        let quote = passages.iter().find(|p| p.text.contains("> look")).unwrap();
        assert!(quote.text.contains("quotation"));
        assert!(quote.excerpt().contains("> look"));
        assert!(text.contains("What now? eat mushroom"));
        assert!(text.contains("You cannot eat that."));
    }

    #[test]
    fn long_response_keeps_every_word_and_its_source_line() {
        let first = (0..200)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let last = (200..400)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let state = state(&[
            ("> look", TranscriptKind::Input),
            (&first, TranscriptKind::Story),
            (&last, TranscriptKind::Story),
        ]);
        let passages = RecallSnapshot::from_state(&state).passages();
        assert!(passages.len() > 4);
        let words = passages
            .iter()
            .flat_map(|p| p.text.split_whitespace())
            .collect::<HashSet<_>>();
        for n in 0..400 {
            assert!(words.contains(format!("word{n}").as_str()), "lost word {n}");
        }
        assert!(passages
            .iter()
            .any(|p| p.raw_line == 2 && p.text.contains("word399")));
        assert!(passages
            .iter()
            .all(|p| p.text.chars().count() <= CHUNK_CHARS));
        assert!(passages
            .iter()
            .all(|p| p.command.as_deref() == Some("look")));
    }

    #[test]
    fn long_unbroken_unicode_line_keeps_all_characters_without_panicking() {
        let text = "灯".repeat(CHUNK_CHARS * 3 + 5);
        let state = state(&[(&text, TranscriptKind::Story)]);
        let passages = RecallSnapshot::from_state(&state).passages();
        // Repeated identical chunks are intentionally deduplicated, but the
        // five-character tail must remain rather than vanish under truncation.
        assert!(passages.iter().any(|p| p.text == "灯".repeat(5)));
        assert!(passages
            .iter()
            .all(|p| p.text.chars().count() <= CHUNK_CHARS));
    }

    #[test]
    fn duplicate_looks_keep_latest_but_different_commands_keep_their_context() {
        use TranscriptKind::*;
        let state = state(&[
            ("> look", Input),
            ("A lantern.", Story),
            ("> look", Input),
            ("A lantern.", Story),
            ("> take key", Input),
            ("You cannot.", Story),
            ("> open door", Input),
            ("You cannot.", Story),
        ]);
        let passages = RecallSnapshot::from_state(&state).passages();
        assert_eq!(passages.len(), 3);
        assert_eq!(passages[0].raw_line, 3);
        assert_eq!(passages[1].command.as_deref(), Some("take key"));
        assert_eq!(passages[2].command.as_deref(), Some("open door"));
    }

    #[test]
    fn snapshot_rejects_same_length_restore_edits_and_cross_story_even_if_identical() {
        let mut state = state(&[("A brass key.", TranscriptKind::Story)]);
        let snapshot = RecallSnapshot::from_state(&state);
        state.transcript[0] = "A silver key.".into();
        assert!(!snapshot.matches_state(&state));
        state.transcript[0] = "A brass key.".into();
        assert!(snapshot.matches_state(&state));
        state.game_dir = PathBuf::from("another-story.save");
        assert!(!snapshot.matches_state(&state));
        state.game_dir = snapshot.game_dir.clone();
        state.transcript_edits += 1;
        assert!(!snapshot.matches_state(&state));
    }

    #[test]
    fn appended_meta_is_safe_but_story_append_or_filter_hiding_evidence_is_stale() {
        let mut state = state(&[("A lantern.", TranscriptKind::Story)]);
        let snapshot = RecallSnapshot::from_state(&state);
        state.push_transcript_kind("Searching your memories...", TranscriptKind::Meta);
        assert!(snapshot.matches_state(&state));
        state.transcript_filter = TranscriptFilter::Meta;
        assert!(!snapshot.matches_state(&state));
        state.transcript_filter = TranscriptFilter::Both;
        state.push_transcript("The lantern goes out.");
        assert!(!snapshot.matches_state(&state));
    }

    #[test]
    fn ranked_source_ids_map_to_visible_positions_without_leaking_meta() {
        use TranscriptKind::*;
        let mut state = state(&[
            ("header", Meta),
            ("> look", Input),
            ("The brass key.", Story),
            ("help", Assist),
        ]);
        let reply = retrieve(
            &request(&state, "key"),
            &mut Offline,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.hits[0].raw_line, 2);
        assert_eq!(reply.visible_matches(&state), Some(vec![2]));
        state.transcript_filter = TranscriptFilter::Story;
        assert_eq!(reply.visible_matches(&state), Some(vec![1]));
        state.transcript[2] = "No key here.".into();
        assert_eq!(reply.visible_matches(&state), None);
    }

    #[test]
    fn bm25_matches_tokens_not_substrings_and_uses_document_frequency() {
        let texts = ["brass key key", "key", "monkey and keyboard"].map(str::to_owned);
        let ranked = lexical_ranking(&texts, &tokenize("BRASS, key!"));
        assert_eq!(ranked.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 1]);
        assert!(ranked[0].1 > ranked[1].1);
        assert!(lexical_ranking(&texts, &tokenize("unlock")).is_empty());
        assert_eq!(tokenize("CAFÉ—灯"), vec!["café", "灯"]);
    }

    #[test]
    fn bm25_numeric_score_matches_formula_and_repeated_query_terms_do_not_boost() {
        let texts = ["key key", "door door"].map(str::to_owned);
        let ranked = lexical_ranking(&texts, &tokenize("key"));
        let expected = 2.0_f32.ln() * 2.0 * 2.2 / (2.0 + 1.2);
        assert!((ranked[0].1 - expected).abs() < 1e-6);
        assert_eq!(ranked, lexical_ranking(&texts, &tokenize("key key key")));
    }

    #[test]
    fn question_function_words_do_not_make_unrelated_passages_lexical_hits() {
        let texts = ["The brooch was stolen.", "A loaf of bread."].map(str::to_owned);
        assert!(
            lexical_ranking(&texts, &tokenize("I am hungry; where was there food?")).is_empty()
        );
        let texts = ["You are in the cellar.", "The courtyard."].map(str::to_owned);
        assert_eq!(
            lexical_ranking(&texts, &tokenize("in"))[0].0,
            0,
            "literal stopword query must remain searchable"
        );
        let texts = ["The iron key.", "The copper key."].map(str::to_owned);
        assert_eq!(
            lexical_ranking(&texts, &tokenize("where was the copper key"))[0].0,
            1
        );
    }

    #[test]
    fn fusion_rewards_agreement_and_keeps_semantic_only_results() {
        let passages = (0..3)
            .map(|raw_line| RecallPassage {
                raw_line,
                text: "observed".into(),
                command: None,
            })
            .collect::<Vec<_>>();
        let hits = fuse(&passages, &[(0, 99.0), (1, 1.0)], &[(1, 0.9), (2, 0.8)]);
        assert_eq!(
            hits.iter().map(|h| h.raw_line).collect::<Vec<_>>(),
            vec![1, 0, 2]
        );
        assert!((hits[0].score - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-6);
    }

    #[test]
    fn degraded_semantic_status_is_explicit_and_keyword_results_survive() {
        let state = state(&[("The brass key is here.", TranscriptKind::Story)]);
        let reply = retrieve(
            &request(&state, "key"),
            &mut Offline,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.hits.len(), 1);
        assert_eq!(
            reply.status,
            RecallStatus::KeywordOnly {
                reason: "model unavailable offline".into()
            }
        );
        let reply = retrieve(
            &request(&state, "unrelated"),
            &mut Offline,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert!(reply.hits.is_empty());
        assert!(matches!(reply.status, RecallStatus::KeywordOnly { .. }));
    }

    #[test]
    fn lexical_channel_does_not_index_embedding_context_labels() {
        let state = state(&[
            ("> look", TranscriptKind::Input),
            ("A brass lantern.", TranscriptKind::Story),
        ]);
        for query in ["response", "context", "observed", "command"] {
            let reply = retrieve(
                &request(&state, query),
                &mut Offline,
                &mut HashMap::new(),
                &|| true,
            )
            .unwrap();
            assert!(
                reply.hits.is_empty(),
                "fabricated lexical evidence for {query}"
            );
        }
        let reply = retrieve(
            &request(&state, "look"),
            &mut Offline,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.hits.len(), 1, "actual command remains searchable");
    }

    #[test]
    fn sentence_embeddings_use_bare_observed_prose_but_results_retain_command_context() {
        let state = state(&[
            ("> examine cabinet", TranscriptKind::Input),
            ("A brass lantern casts a warm glow.", TranscriptKind::Story),
        ]);
        let mut encoder = FakeEncoder::default();
        let reply = retrieve(
            &request(&state, "illumination"),
            &mut encoder,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(encoder.calls[1], vec!["A brass lantern casts a warm glow."]);
        assert!(reply.hits[0].excerpt.starts_with("> examine cabinet\n"));
    }

    #[test]
    fn identical_responses_share_vector_even_when_command_contexts_differ() {
        use TranscriptKind::*;
        let state = state(&[
            ("> look", Input),
            ("A lantern.", Story),
            ("> examine room", Input),
            ("A lantern.", Story),
        ]);
        let mut encoder = FakeEncoder::default();
        let reply = retrieve(
            &request(&state, "lantern"),
            &mut encoder,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(encoder.calls[1], vec!["A lantern."]);
        assert_eq!(
            reply.hits.len(),
            2,
            "distinct observed commands retain their original source positions"
        );
    }

    #[test]
    fn display_mode_does_not_change_semantic_input_or_double_count_inline_commands() {
        use TranscriptKind::*;
        let response = "The librarian whispers that the secret password is winter.";
        let explicit = state(&[("> ask librarian about password", Input), (response, Story)]);
        let mut inline = AppState::default();
        inline.record_command("ask librarian about password");
        inline.push_transcript(">");
        inline.append_to_last_transcript_line("ask librarian about password");
        inline.push_transcript(response);
        let a = RecallSnapshot::from_state(&explicit).passages().remove(0);
        let b = RecallSnapshot::from_state(&inline).passages().remove(0);
        assert_eq!(a.search_text(), response);
        assert_eq!(
            b.search_text(),
            response,
            "inline command must not contaminate the vector"
        );
        assert_eq!(tokenize(&a.lexical_text()), tokenize(&b.lexical_text()));
        assert!(b.text.contains(">ask librarian about password"));
        assert!(b.excerpt().contains(">ask librarian about password"));
        for snapshot in [&explicit, &inline] {
            let reply = retrieve(
                &request(snapshot, "How do I renew an OAuth authentication token?"),
                &mut Offline,
                &mut HashMap::new(),
                &|| true,
            )
            .unwrap();
            assert!(
                reply.hits.is_empty(),
                "OAuth has no lexical evidence in either source layout"
            );
        }
    }

    #[test]
    fn standalone_or_unverified_prompt_remains_in_semantic_input() {
        let mut state = AppState::default();
        state.record_command("look");
        state.push_transcript(">look");
        let passage = RecallSnapshot::from_state(&state).passages().remove(0);
        assert_eq!(passage.search_text(), ">look");
        state.transcript[0] = "> The king is a liar".into();
        let passage = RecallSnapshot::from_state(&state).passages().remove(0);
        assert_eq!(passage.search_text(), "> The king is a liar");
    }

    #[test]
    fn empty_or_punctuation_only_request_never_calls_the_model() {
        let state = state(&[("A lantern.", TranscriptKind::Story)]);
        let mut encoder = FakeEncoder::default();
        for query in ["", "  ", "!!!"] {
            let reply = retrieve(
                &request(&state, query),
                &mut encoder,
                &mut HashMap::new(),
                &|| true,
            )
            .unwrap();
            assert_eq!(reply.status, RecallStatus::Empty);
        }
        assert!(encoder.calls.is_empty());
        let reply = retrieve(
            &request(&AppState::default(), "lantern"),
            &mut encoder,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.status, RecallStatus::Empty);
        assert!(encoder.calls.is_empty());
    }

    #[test]
    fn full_scan_finds_semantic_only_match_and_abstains_below_floor() {
        use TranscriptKind::*;
        let state = state(&[
            ("> look", Input),
            ("A key.", Story),
            ("> north", Input),
            ("A lantern.", Story),
        ]);
        let reply = retrieve(
            &request(&state, "illumination"),
            &mut FakeEncoder::default(),
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.status, RecallStatus::Hybrid);
        assert_eq!(reply.hits.len(), 1);
        assert_eq!(reply.hits[0].raw_line, 3);
        let reply = retrieve(
            &request(&state, "zebras"),
            &mut FakeEncoder::default(),
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert!(
            reply.hits.is_empty(),
            "an arbitrary nearest neighbour is not a match"
        );
    }

    #[test]
    fn embeddings_are_reused_for_unchanged_text_and_evicted_on_restore() {
        use TranscriptKind::*;
        let mut state = state(&[("> look", Input), ("A lantern.", Story)]);
        let mut encoder = FakeEncoder::default();
        let mut cache = HashMap::new();
        retrieve(
            &request(&state, "illumination"),
            &mut encoder,
            &mut cache,
            &|| true,
        )
        .unwrap();
        assert_eq!(
            encoder.calls.len(),
            2,
            "one query batch and one passage batch"
        );
        retrieve(
            &request(&state, "lantern"),
            &mut encoder,
            &mut cache,
            &|| true,
        )
        .unwrap();
        assert_eq!(
            encoder.calls.len(),
            3,
            "unchanged passage must not be embedded again"
        );
        state.transcript[1] = "A key.".into();
        retrieve(&request(&state, "key"), &mut encoder, &mut cache, &|| true).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.keys().all(|text| !text.contains("lantern")));
    }

    #[test]
    fn malformed_embeddings_fail_closed_to_lexical_results() {
        struct Malformed(Vec<Vec<f32>>);
        impl Encoder for Malformed {
            fn embed(&mut self, _: &[String]) -> Result<Vec<Vec<f32>>, String> {
                Ok(self.0.clone())
            }
        }
        let state = state(&[("A key.", TranscriptKind::Story)]);
        for vectors in [
            vec![],
            vec![vec![]],
            vec![vec![0.0]],
            vec![vec![f32::NAN]],
            vec![vec![f32::INFINITY]],
        ] {
            let reply = retrieve(
                &request(&state, "key"),
                &mut Malformed(vectors),
                &mut HashMap::new(),
                &|| true,
            )
            .unwrap();
            assert!(matches!(reply.status, RecallStatus::KeywordOnly { .. }));
            assert_eq!(reply.hits.len(), 1);
        }
        assert!(normalize(vec![1.0, 0.0], Some(3)).is_err());
    }

    struct GatedEncoder {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        first: bool,
    }
    impl Encoder for GatedEncoder {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            if self.first {
                self.first = false;
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
        }
    }

    fn gated_worker() -> (RecallWorker, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (entered, did_enter) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut worker = RecallWorker::default();
        worker
            .start(move || GatedEncoder {
                entered,
                release: wait,
                first: true,
            })
            .unwrap();
        (worker, did_enter, release)
    }

    fn wait_reply(worker: &mut RecallWorker) -> RecallReply {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(reply) = worker.poll() {
                return reply;
            }
            assert!(Instant::now() < deadline, "worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn pending_queries_coalesce_and_in_flight_stale_reply_cannot_escape() {
        let state = state(&[("A lantern.", TranscriptKind::Story)]);
        let snapshot = RecallSnapshot::from_state(&state);
        let (mut worker, entered, release) = gated_worker();
        worker.submit(snapshot.clone(), "first").unwrap();
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        for n in 0..100 {
            worker
                .submit(snapshot.clone(), &format!("middle {n}"))
                .unwrap();
        }
        let last_id = worker.submit(snapshot, "latest").unwrap();
        assert_eq!(
            lock(worker.shared.as_ref().unwrap())
                .pending
                .as_ref()
                .unwrap()
                .query,
            "latest"
        );
        release.send(()).unwrap();
        let reply = wait_reply(&mut worker);
        assert_eq!(reply.request_id, last_id);
        assert_eq!(reply.query, "latest");
        assert!(worker.poll().is_none());
    }

    #[test]
    fn cancellation_drops_ready_reply_and_rejects_in_flight_old_story() {
        let state = state(&[("Old story lantern.", TranscriptKind::Story)]);
        let (mut worker, entered, release) = gated_worker();
        worker
            .submit(RecallSnapshot::from_state(&state), "lantern")
            .unwrap();
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        worker.cancel();
        assert!(worker.poll().is_none());
        let mut new_state = AppState::default();
        new_state.game_dir = PathBuf::from("new.save");
        new_state.push_transcript("New story key.");
        let new_id = worker
            .submit(RecallSnapshot::from_state(&new_state), "key")
            .unwrap();
        release.send(()).unwrap();
        let reply = wait_reply(&mut worker);
        assert_eq!(reply.request_id, new_id);
        assert!(reply
            .hits
            .iter()
            .all(|hit| !hit.excerpt.contains("lantern")));
        assert!(reply.matches_state(&new_state));
        // Publish this otherwise valid result into the slot, then cancel it.
        lock(worker.shared.as_ref().unwrap()).reply = Some(reply);
        worker.cancel();
        assert!(worker.poll().is_none());
    }

    #[test]
    fn construction_and_oversized_query_do_not_start_a_thread_or_load_model() {
        let mut worker = RecallWorker::default();
        assert!(worker.shared.is_none());
        worker.cancel();
        assert!(worker.poll().is_none());
        assert!(worker
            .submit(
                RecallSnapshot::from_state(&AppState::default()),
                &"x".repeat(MAX_QUERY_CHARS + 1)
            )
            .is_err());
        assert!(worker.shared.is_none());
    }

    #[test]
    fn cancelled_request_skips_model_work_and_results_are_bounded_by_source_line() {
        let state = state(&[("A lantern.", TranscriptKind::Story)]);
        let mut encoder = FakeEncoder::default();
        assert!(retrieve(
            &request(&state, "lantern"),
            &mut encoder,
            &mut HashMap::new(),
            &|| false
        )
        .is_none());
        assert!(encoder.calls.is_empty());
        let passages = (0..100)
            .map(|n| RecallPassage {
                raw_line: n / 2,
                text: "observed".into(),
                command: None,
            })
            .collect::<Vec<_>>();
        let ranked = (0..100).map(|n| (n, 1.0)).collect::<Vec<_>>();
        let hits = fuse(&passages, &ranked, &[]);
        assert_eq!(hits.len(), MAX_HITS);
        assert_eq!(
            hits.iter()
                .map(|h| h.raw_line)
                .collect::<HashSet<_>>()
                .len(),
            MAX_HITS
        );
    }

    /// Full production retrieval, not the fake encoder used by pure math and
    /// mailbox tests. Validation lane must run this explicitly before shipping.
    #[test]
    #[ignore = "downloads the pinned 91MB MiniLM model on first use"]
    fn real_model_recall_finds_observed_light_source_without_lexical_overlap() {
        use TranscriptKind::*;
        let state = state(&[
            ("> examine cabinet", Input),
            (
                "A brass lantern burns with a steady flame, casting a warm glow across the room.",
                Story,
            ),
            ("> examine table", Input),
            ("A loaf of bread rests beside a bowl of apples.", Story),
            ("> examine doorway", Input),
            (
                "An iron key hangs from a hook beside the locked door.",
                Story,
            ),
            (
                "Spoiler: a magic torch waits in an undiscovered chamber.",
                Meta,
            ),
        ]);
        let reply = retrieve(
            &request(&state, "illumination for darkness"),
            &mut RealEncoder::default(),
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.status, RecallStatus::Hybrid);
        assert!(!reply.hits.is_empty());
        assert_eq!(reply.hits[0].raw_line, 1, "{:#?}", reply.hits);
        assert!(reply
            .hits
            .iter()
            .all(|hit| !hit.excerpt.contains("undiscovered")));
    }

    fn acceptance_scenes() -> [(&'static str, &'static str); 13] {
        [
            ("examine cabinet", "A brass lantern burns with a steady flame, casting a warm glow across the room."),
            ("examine hook", "An iron key hangs from a hook beside the locked oak door."),
            ("examine coils", "A sturdy climbing rope lies coiled beside the deep pit. It could support your weight."),
            ("examine table", "A fresh loaf of bread rests beside a bowl of ripe apples."),
            ("ask ferryman about passage", "The ferryman will carry you across the river in his boat for two copper coins."),
            ("ask librarian about password", "The librarian whispers that the secret password is winter. She asks you to remember it."),
            ("lift grate", "The iron grate is too heavy. You cannot lift it with your bare hands."),
            ("unlock copper lock with silver key", "The silver key does not fit the copper lock. The chest remains locked."),
            ("examine mushroom", "The red mushroom bears a skull symbol. Your guide warns that eating it would poison you."),
            ("read tablet", "The stone tablet's inscription is written backwards. Its letters become clear when reflected in a mirror."),
            ("examine clock", "The great clock in the tower rings its bell at midnight, waking every sleeper in the village."),
            ("examine bottle", "The green bottle is filled with cool spring water, fresh and safe to drink."),
            ("examine display", "The sapphire brooch was stolen from the queen's exhibition. Its empty velvet stand remains."),
        ]
    }

    fn acceptance_cases() -> [(&'static str, usize, usize); 24] {
        // Original targets stay unchanged. The second set varies the wording
        // independently instead of calibrating to one failing light query.
        [
            ("What can I use to see in a dark place?", 0, 1),
            ("How could I get across the river?", 4, 1),
            ("Where did I find something to drink?", 11, 3),
            ("What was the secret word the librarian told me?", 5, 1),
            ("What might let me descend into a hole safely?", 2, 3),
            ("I am hungry; where was there food?", 3, 1),
            ("Which inscription needed to be read backwards?", 9, 1),
            ("sapphire brooch", 12, 1),
            ("copper lock silver key", 7, 1),
            ("lift grate", 6, 1),
            ("When does the clock ring?", 10, 3),
            ("What would be poisonous to eat?", 8, 3),
            ("How can I brighten this place?", 0, 3),
            ("What was hanging by the oak entrance?", 1, 3),
            ("Something strong enough to lower myself down a shaft", 2, 3),
            ("What provisions could satisfy my hunger?", 3, 3),
            ("Who offered transportation over the water?", 4, 3),
            (
                "Which phrase did the keeper of books ask me to remember?",
                5,
                3,
            ),
            ("What was too heavy for me to raise?", 6, 3),
            ("Why did the chest stay locked after my attempt?", 7, 3),
            ("Which fungus was unsafe to consume?", 8, 3),
            ("How could I decipher the reversed lettering?", 9, 3),
            ("What sound wakes people in the village at night?", 10, 3),
            ("Where was the container of fresh drinking water?", 11, 3),
        ]
    }

    fn unrelated_queries() -> [&'static str; 8] {
        [
            "Kubernetes pod autoscaling deployment metrics",
            "How do I renew an OAuth authentication token?",
            "Explain the derivative of a quadratic polynomial.",
            "Where can I recharge my smartphone battery?",
            "How do I repair a spacecraft fusion reactor?",
            "What causes mitosis in animal cells?",
            "How should a neural network optimizer schedule its learning rate?",
            "What are the rules for a chess en passant capture?",
        ]
    }

    #[test]
    #[ignore = "downloads the pinned 91MB MiniLM model; reports threshold calibration evidence"]
    fn real_model_semantic_floor_calibration() {
        let scenes = acceptance_scenes();
        let mut encoder = RealEncoder::default();
        let vectors = encoder
            .embed(
                &scenes
                    .iter()
                    .map(|scene| scene.1.to_owned())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let mut min_positive = f32::INFINITY;
        let mut max_negative = f32::NEG_INFINITY;
        for (query, expected, _) in acceptance_cases() {
            let q = encoder.embed(&[query.into()]).unwrap().remove(0);
            let mut scored = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, cosine(&q, v)))
                .collect::<Vec<_>>();
            sort_scores(&mut scored);
            let target_score = cosine(&q, &vectors[expected]);
            min_positive = min_positive.min(target_score);
            eprintln!(
                "positive {query:?}: expected={expected} score={target_score} rank={} top3={:?}",
                scored.iter().position(|(i, _)| *i == expected).unwrap() + 1,
                &scored[..3]
            );
        }
        for query in unrelated_queries() {
            let q = encoder.embed(&[query.into()]).unwrap().remove(0);
            let mut scored = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, cosine(&q, v)))
                .collect::<Vec<_>>();
            sort_scores(&mut scored);
            max_negative = max_negative.max(scored[0].1);
            eprintln!("negative {query:?}: top3={:?}", &scored[..3]);
        }
        eprintln!("calibration: minimum expected positive cosine={min_positive}, maximum unrelated cosine={max_negative}, current floor={MIN_COSINE}");
        assert!(
            min_positive >= MIN_COSINE,
            "floor suppresses a known paraphrase"
        );
        assert!(
            max_negative < MIN_COSINE,
            "floor admits a known unrelated query"
        );
    }

    #[test]
    #[ignore = "downloads the pinned 91MB MiniLM model; integrated ranking acceptance"]
    fn real_model_hybrid_recall_acceptance_matrix() {
        use TranscriptKind::*;
        let scenes = acceptance_scenes();
        let cases = acceptance_cases();
        let mut encoder = RealEncoder::default();
        let mut cache = HashMap::new();
        let mut failures = Vec::new();
        let diagnostic = encoder
            .embed(&[
                "What can I use to see in a dark place?".into(),
                scenes[0].1.into(),
                format!(
                    "Observed command context: {}\nStory response: {}",
                    scenes[0].0, scenes[0].1
                ),
            ])
            .unwrap();
        let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        eprintln!("dark-place paraphrase: bare prose cosine={}, decorated cosine={}, relevance floor={MIN_COSINE}",
            cosine(&diagnostic[0], &diagnostic[1]), cosine(&diagnostic[0], &diagnostic[2]));
        for inline in [false, true] {
            for reversed in [false, true] {
                let mut state = AppState::default();
                let source_scene = |position: usize| {
                    if reversed {
                        scenes.len() - 1 - position
                    } else {
                        position
                    }
                };
                for position in 0..scenes.len() {
                    let (command, response) = scenes[source_scene(position)];
                    if inline {
                        state.record_command(command);
                        state.push_transcript(">");
                        state.append_to_last_transcript_line(command);
                    } else {
                        state.push_transcript_kind(&format!("> {command}"), Input);
                    }
                    state.push_transcript_kind(response, Story);
                }
                state.push_transcript_kind(
                    "Walkthrough: use a secret teleport spell to cross the river.",
                    Meta,
                );
                state.push_transcript_kind("Perhaps you should eat the mushroom.", Assist);
                for (query, scene, limit) in cases {
                    let reply =
                        retrieve(&request(&state, query), &mut encoder, &mut cache, &|| true)
                            .unwrap();
                    assert_eq!(reply.status, RecallStatus::Hybrid);
                    let rank = reply
                        .hits
                        .iter()
                        .position(|hit| source_scene(hit.raw_line / 2) == scene)
                        .map(|rank| rank + 1);
                    eprintln!(
                "inline={inline}, reversed={reversed}, {query:?}: expected scene {scene}, rank {rank:?}; top3 {:?}",
                reply
                    .hits
                    .iter()
                    .take(3)
                    .map(|hit| (source_scene(hit.raw_line / 2), hit.score))
                    .collect::<Vec<_>>()
            );
                    if !rank.is_some_and(|rank| rank <= limit) {
                        failures.push(format!(
                    "inline={inline}, reversed={reversed}, {query:?}: expected scene {scene} in top {limit}, got {rank:?}"
                ));
                    }
                    assert!(reply
                        .hits
                        .iter()
                        .all(|hit| !hit.excerpt.contains("teleport")
                            && !hit.excerpt.contains("Perhaps you should")));
                    if scene == 7 {
                        let hit = reply
                            .hits
                            .iter()
                            .find(|hit| source_scene(hit.raw_line / 2) == 7)
                            .expect("failed command passage");
                        assert!(
                            hit.excerpt.contains("does not fit")
                                && hit.excerpt.contains("remains locked")
                        );
                    }
                    if scene == 6 {
                        let hit = reply
                            .hits
                            .iter()
                            .find(|hit| source_scene(hit.raw_line / 2) == 6)
                            .expect("failed lift passage");
                        assert!(hit.excerpt.contains("cannot lift"));
                    }
                }
                for query in unrelated_queries() {
                    let unrelated =
                        retrieve(&request(&state, query), &mut encoder, &mut cache, &|| true)
                            .unwrap();
                    assert_eq!(unrelated.status, RecallStatus::Hybrid);
                    assert!(
                        unrelated.hits.is_empty(),
                        "unrelated query {query:?} received invented relevance: {:?}",
                        unrelated.hits
                    );
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Fresh game evidence, separate from the synthetic development corpus:
    /// Minizork, `open mailbox`, `read leaflet`, captured in the parent's
    /// /tmp/lanthorn-recall-demo.raw PTY run on 2026-09-24. This is a regression
    /// fixture from an observed failure, not a held-out accuracy benchmark.
    /// Known MiniLM limitation: the vague paper query misses the printed text.
    /// Explicitly running this ignored diagnostic is expected to fail until a
    /// separately evaluated model improvement satisfies the original query.
    #[test]
    #[ignore = "known MiniLM limitation: paper query fails observed leaflet retrieval; manual reproduction"]
    fn real_model_known_limitation_minizork_leaflet() {
        let opening = "You are standing in an open field west of a white house, with a boarded front door. You could circle the house to the north or south. There is a small mailbox here.";
        let mailbox = "Opening the small mailbox reveals a leaflet.";
        let leaflet = "[Taken]\n\"WELCOME TO ZORK, a game of adventure, danger, and low cunning. No computer should be without one!\"\nNote: this \"mini-zork\" contains only a sub-set of the locations, puzzles, and descriptions found in the larger, disk-based version of Zork I.";
        let mut state = AppState::default();
        state.push_transcript(opening);
        for (command, response) in [("open mailbox", mailbox), ("read leaflet", leaflet)] {
            state.record_command(command);
            state.push_transcript(">");
            state.append_to_last_transcript_line(command);
            state.push_transcript(response);
        }
        let query = "what was written on the paper";
        let mut encoder = RealEncoder::default();
        let diagnostic = encoder
            .embed(&[
                query.into(),
                leaflet.into(),
                format!("read leaflet\n{leaflet}"),
                mailbox.into(),
            ])
            .unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        eprintln!(
            "Minizork paper query: body={}, command+body={}, mailbox={}, floor={MIN_COSINE}",
            dot(&diagnostic[0], &diagnostic[1]),
            dot(&diagnostic[0], &diagnostic[2]),
            dot(&diagnostic[0], &diagnostic[3])
        );
        let reply = retrieve(
            &request(&state, query),
            &mut encoder,
            &mut HashMap::new(),
            &|| true,
        )
        .unwrap();
        assert_eq!(reply.status, RecallStatus::Hybrid);
        eprintln!("Minizork actual recall hits: {:?}", reply.hits);
        let leaflet_rank = reply
            .hits
            .iter()
            .position(|hit| hit.excerpt.contains("WELCOME TO ZORK"))
            .map(|rank| rank + 1);
        eprintln!("Minizork observed paper-query diagnostic: leaflet rank={leaflet_rank:?}; exploratory evidence, not an accuracy gate");
        assert!(
            reply.visible_matches(&state).is_some(),
            "diagnostic results must still point into the actual observed transcript"
        );
        assert!(leaflet_rank.is_some_and(|rank| rank <= 3),
            "known MiniLM limitation: original paper query should retrieve the observed leaflet in the first three passages");
    }
}
