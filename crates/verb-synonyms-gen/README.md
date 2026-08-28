# verb-synonyms-gen

The offline generator behind `crates/verb-synonyms/src/synonym_groups.tsv`.

It exists because of the persistence rule: nothing goes into the tree without
its regeneration inputs, or the table becomes a blob nobody can reproduce when
the corpus grows or the lexical source changes. This crate is a development
tool. It is never linked into `lanthorn`, and it takes no external
dependencies — only the three story readers, so that a checkout that builds the
workspace can rebuild the table.

## The problem it solves

A player types `illuminate lamp`; the story wants `light`. Nothing in the story
file records what `illuminate` *means*, and every mechanism that works on FORM
fails: edit distance is 8 on a 10-letter word, stemming reaches `illuminat-` and
nothing else, and the grammar's shape only narrows to "a verb taking one noun",
which is most verbs. The bridge has to come from outside the story file.

Shipping a general thesaurus and querying it at runtime would be enormous. But
the IF side is BOUNDED — a few thousand verb spellings across every game anyone
has written — so the vocabulary is harvested, WordNet is filtered down to the
synsets that vocabulary actually touches, and the result is shipped as a table
with no runtime dependency and no network.

## Sources

Neither is vendored; both are downloaded by `fetch-sources.sh`, which pins the
exact releases by SHA-256.

| source | version | what it supplies |
|---|---|---|
| [WordNet](https://wordnet.princeton.edu/) `dict/` | **3.0** (2006), `WordNet-3.0.tar.gz`, sha256 `640db279…d3a52` | synonymy (`data.verb`, `index.verb`), the hypernym pointer graph, and irregular verb inflections (`verb.exc`) |
| [12dicts](http://wordlist.aspell.net/12dicts/) `Lemmatized/2+2+3frq.txt` | **6.0.2** (June 2016), `12dicts-6.0.2.zip`, sha256 `64ac1d35…780e52` | the frequency ranking (21 bands, commonest first) and a lemmatisation map, headword at column 0 with its inflected and derived forms indented |

Licences: both are permissive and compatible with lanthorn's BSD-3-Clause, and
both require a notice to travel with derived data. That notice is
`THIRD-PARTY-NOTICES.md` at the repository root. Read it before swapping either
source.

Why 12dicts and not a web-scale frequency list: the obvious candidates are not
usable. `first20hours/google-10000-english` says outright "I do not recommend
using this data for commercial purposes without licensing it from the Linguistic
Data Consortium"; `hermitdave/FrequencyWords` is MIT for its code but CC-BY-SA
4.0 for its content, and share-alike on a derived database is not a thing to
take on casually. 12dicts' chain — Beale → AGID → Moby (public domain), ENABLE2K
(public domain), WordNet — is permissive the whole way down, and it has the
property the others lack: it is already **lemmatised**, so the frequency ranking
and the inflection map come from one file.

## Rebuilding

```sh
./crates/verb-synonyms-gen/fetch-sources.sh /tmp/verbsyn      # ~22 MB of downloads

# Step 1 — the IF vocabulary. Needs a corpus of story files; its output is
# COMMITTED (if_verbs.tsv), so step 2 is reproducible without one.
cargo run -p verb-synonyms-gen -- harvest \
    --corpus stories --corpus unit_tests \
    --wordnet /tmp/verbsyn/WordNet-3.0/dict \
    --freq /tmp/verbsyn/12dicts/Lemmatized/2+2+3frq.txt \
    -o crates/verb-synonyms-gen/if_verbs.tsv

# Step 2 — the shipped table.
cargo run -p verb-synonyms-gen -- build \
    --wordnet /tmp/verbsyn/WordNet-3.0/dict \
    --freq /tmp/verbsyn/12dicts/Lemmatized/2+2+3frq.txt \
    --if-verbs crates/verb-synonyms-gen/if_verbs.tsv \
    -o crates/verb-synonyms/src/synonym_groups.tsv

cargo nextest run -p verb-synonyms   # the canonical mappings must survive
```

Both steps print a report to stderr. The number to look at is the coverage
audit: what fraction of the commonest English verbs (12dicts bands 1–11,
lemmatised and reduced to those WordNet knows as verbs) reach a surviving group.
That is the quality metric for the whole exercise — far more meaningful than the
row count — and it is what tells you whether a change to the filters helped.

### The knobs

`build` takes `--sense-cap`, `--band-cap`, `--group-cap`, `--hyponym-cap`,
`--common-bands` and `--no-gap-fill`; the defaults are in `Params::default` and
each is documented where it is declared. They are on the command line so that a
retune can be argued with rather than recompiled.

## Why `if_verbs.tsv` is committed

It is a sorted list of ordinary English verb spellings with a count of how many
stories accept each — `take 118`, `xyzzy 9`. It carries no game text, no game
titles and no attribution of any word to any story: the per-story detail exists
only in the harvest's stderr report. `stories/` is gitignored because it holds
commercial game files; a de-duplicated vocabulary list drawn across 119 of them
is not one, and committing it is what makes step 2 — and therefore the shipped
table — reproducible by CI and by anyone without the corpus.
