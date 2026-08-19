# SQ-0348 — Story metadata: IFmd offline, IFDB on request, a per-story sidecar, and a sortable story list — Design

**Status:** design (awaiting user review)
**Date:** 2026-07-15
**Depends on:** SQ-0284 (per-game dir layout — the sidecar lives there)
**Blocks:** SQ-0276 (capability probe — second writer into the same sidecar)
**Related:** SQ-0347 (asset preview in the info panel — shares the panel, not this data)

## Problem

The story browser's info panel (`picker_ui.rs:481-681`) shows what it can derive from the
story file itself: size, format, version, release, serial, IFID, blorb chunk list, and a
cover from the `Fspc` frontispiece. For **title** it falls back to a bundled TSV keyed on
IFID (`known_title`, `session.rs:651`), then to the filename stem. There is no author, no
blurb, no genre, no publication year — and for any story not in the TSV, no real title
either.

Two sources could fix that, and the app uses neither:

1. **`IFmd`** — the Treaty of Babel iFiction metadata chunk. Blorbs routinely carry it.
   `crates/blorb` recognises exactly two top-level chunks, `RIdx` and `Fspc`
   (`blorb/src/lib.rs:91-111`); `IFmd` is skipped by length. Both the blorb-parser spec
   (`2026-06-27-blorb-parser-design.md:42`) and the info-panel spec
   (`2026-07-01-story-picker-info-panel-design.md:29`) scoped it out deliberately.
2. **IFDB** — serves the *same* iFiction XML over HTTP, keyed by IFID.

Separately, SQ-0276 wants to cache a runtime capability probe per story, and there is
nowhere to put it: no per-story metadata sidecar exists. The only JSON in the tree lives
*inside* the `.lanthorn` save zip (`archive.rs:75-86`), which describes a save, not a story.

## Goals

1. **Parse `IFmd` and use it**, automatically, at scan time. No cache, no network, no flag.
2. **Fetch from IFDB on an explicit keypress only.** The app never touches the network
   unless the user asks it to, by name, in the browser.
3. **A per-story `info.json` sidecar** that caches *only* what cannot be cheaply
   recomputed from the story file — today the IFDB fetch, tomorrow SQ-0276's probe.
4. **A library sweep that self-invalidates on upgrade**: it skips stories already fetched
   by the *current fetch version*, so improving the scanner re-fetches automatically
   without a separate rescan-all binding.
5. **Show it in the info panel**: author, blurb, genre, year, language; a fetched cover
   when the story has no `Fspc` of its own.
6. **Show it in the list**: the real title (not the filename stem), plus author and year
   as aligned columns under a header, with **click-to-sort** headers.

## Non-goals

- **Caching IFmd.** It is re-derived from the story file on every scan (see "Why the
  offline path needs no cache"). The blorb *is* the cache.
- **Writing metadata back** to IFDB, or any authenticated IFDB use.
- **Automatic/background network fetch**, ever — including "just this once on first run".
  User's explicit choice: hotkey only.
- **IFDB's non-bibliographic payload** — tags, ratings, download links, ClubFloyd
  transcripts, play-online URLs. The response carries all of it; we keep a narrow subset
  (`FetchedMeta` below is the exhaustive list). Ratings/tags are a possible follow-up,
  not this quest.
- **SQ-0276's probe itself.** This spec reserves its slot in the sidecar and nothing more.
- **Persisting the sort preference** across sessions. In-session only; add it if it turns
  out to be wanted.
- **Filtering or searching the list.** Sorting only. A filter is a natural neighbour and a
  separate quest.
- **Back-compat with any earlier sidecar.** None exists, and per project policy pre-release
  formats break freely.

## Why the offline path needs no cache

`scan_stories` (`picker.rs:452-557`) already reads every story file whole — it must, to
find the `UUID://` marker for the IFID (`ifid.rs:34`). For blorb-extension files it
already calls `blorb::Blorb::parse` and walks the chunk list (`picker.rs:502-512`).
Extracting `IFmd` from that parse is a chunk lookup plus a small XML parse over bytes
already in memory.

So IFmd metadata is populated into `StoryMeta` at scan time exactly as `version`,
`serial`, and `release` already are. This is what makes "offline on by default" cheap
enough to need no opt-out.

**The catch, stated plainly:** only a *container* can carry `IFmd`. A bare `.z5`/`.z8`/`.ulx`
has nowhere to put the chunk, so the offline path yields nothing for classic Infocom
files — including this project's own `zork1-invclues-r52-s871125.z5` test story. Offline
metadata is a win for modern blorbed IF and a no-op for a bare-z-file library. The hotkey
fetch is therefore not a nice-to-have; for such a library it is the only source.

## Design

### Crate split (blorb stays zero-dep)

`crates/blorb` has an empty `[dependencies]` by design. An XML parser must not land there.

| Crate | Gains | Dep cost |
|-------|-------|----------|
| `blorb` | `IFmd` in the top-level chunk walk; `fn metadata(&self) -> Option<&[u8]>` returning **raw chunk bytes** | none — stays zero-dep |
| `app` | `ifiction` (XML → struct), `ifdb` (HTTP), `story_info` (sidecar) | `roxmltree`, `ureq` |

`blorb` never interprets the chunk. It hands over bytes; `app` parses them. The same
`app::ifiction` parser consumes both an `IFmd` chunk and an IFDB response, because they
are the same format.

### `app::ifiction` — the shared parser

Verified against a live IFDB response (`ifdb.org/viewgame?ifiction&ifid=ZCODE-88-840726-A129`,
fetched 2026-07-15). Document element `<ifindex version="1.0">`, namespace
`http://babel.ifarchive.org/protocol/iFiction/`, containing `<story>` with:

- `<identification>` — one or more `<ifid>`, plus `<format>`, `<bafn>`
- `<bibliographic>` — `<title>`, `<author>`, `<language>`, `<firstpublished>`,
  `<genre>`, `<description>` (the blurb)
- `<ifdb>` — an **extension** element in namespace `http://ifdb.org/api/xmlns`, carrying
  `<tuid>`, `<link>`, `<coverart><url>`, `<averageRating>`, `<starRating>`, `<tags>`,
  `<downloads>`

```rust
/// Parsed iFiction. Every field optional: an IFmd chunk may carry only a subset,
/// and we must never fail a scan over a story with thin metadata.
pub struct IFiction {
    pub ifids: Vec<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    /// From the ifdb.org extension namespace; absent in an IFmd chunk.
    pub ifdb: Option<IfdbExt>,
}

pub struct IfdbExt {
    pub tuid: String,
    pub link: Option<String>,
    pub cover_url: Option<String>,
}

pub fn parse(xml: &[u8]) -> Result<IFiction, IFictionError>;
```

Namespace-aware (`roxmltree` gives this): match on local name **within** the Babel
namespace, so a `<title>` nested in `<downloads><link>` — the live response has 25 —
can never be mistaken for the bibliographic title. This is a real trap, not a hypothetical:
the Zork response contains 26 `<title>` elements, only one of which is the game's.

### `app::story_info` — the sidecar

Path: `<data_base>/<story-key>.save/info.json`, alongside `style.toml` and `config.toml`.
`story_key` and `game_dir` are SQ-0284's existing helpers (`storage.rs:14-28`) — reused
verbatim, not reopened.

```rust
#[derive(Serialize, Deserialize)]
pub struct StoryInfo {
    pub format_version: u32,        // 1
    /// The IFID the cached blocks were fetched/probed for — see "Identity check".
    pub ifid: String,
    pub fetched: Option<FetchedMeta>,   // this quest
    pub probe: Option<ProbeMeta>,       // reserved for SQ-0276; always None here
}

/// Present ONLY for a fetch that ran to completion — found or authoritatively
/// not-found. A network error writes no block at all, so `r` retries it.
#[derive(Serialize, Deserialize)]
pub struct FetchedMeta {
    pub scanned_at: String,         // RFC3339, via jiff (already a dep)
    /// The scanner that produced this block. `r` skips a story whose block
    /// carries the CURRENT value and refetches every other. See "Scanner version".
    pub fetch_version: u32,
    pub source: String,             // "ifdb"
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub ifdb_tuid: Option<String>,
    pub ifdb_link: Option<String>,
    /// Filename of the cached cover next to this file, e.g. "cover.png".
    /// Bytes are NOT base64'd into the JSON.
    pub cover: Option<String>,
    /// IFDB has no record for this IFID. Still a completed fetch — the answer is
    /// "nothing there" — so `r` skips it at the current fetch version. `f`
    /// retries it on demand.
    pub not_found: bool,
}
```

### Identity check

The sidecar is keyed by **filename** (SQ-0284), but its contents describe an **IFID**.
Those can come apart: replace `zork1.z5` with a different game under the same name and the
old sidecar is still sitting in the matching `game_dir`, ready to attribute Zork's blurb
and cover to it.

So `StoryInfo.ifid` is a checked field, not a record-keeping one. On load, compare it to
the IFID `scan_stories` computed for the file actually on disk:

- **match** → blocks are usable
- **mismatch** → every block is stale. Ignore all of them and treat the story as unfetched;
  the next `r` refetches it, `f` overwrites it.

This is cheap — the IFID is already in `StoryMeta` by the time anything reads the sidecar —
and it is the only thing standing between a filename-keyed cache and wrong metadata on a
swapped file. Note it protects the `probe` block too, which matters more for SQ-0276: wrong
*capabilities* for a story would be a subtler bug than a wrong blurb.

### Fetch version

`FETCH_VERSION: u32` — a constant in `app::story_info`, starting at `1`.

**Bump it whenever a re-fetch would produce a materially different block**: a new field
extracted, a changed endpoint, fixed parsing. Do *not* bump for refactors that cannot
change output.

It is deliberately **not** `CARGO_PKG_VERSION`. Tying invalidation to the app version
would make every unrelated release re-fetch every story in every library — pointless load
on a volunteer-run site, and slow for the user. A hand-bumped constant means invalidation
is a decision someone made on purpose. The cost is that it can be forgotten; the mitigation
is that `f` always force-fetches a single story, so a user is never stuck with a stale
record, and a missed bump is recoverable by bumping it in the next release.

Each block owns its `scanned_at`, so the two writers never need to agree on a clock or
step on each other. A malformed or unknown-`format_version` sidecar is **ignored and
overwritten**, not migrated (no back-compat pre-release).

**Storage key is the filename, not the IFID** — SQ-0284 decided that deliberately
(`storage.rs:1-8`). Consequence, accepted: two copies of the same story under different
filenames each get their own sidecar and each fetch separately. The IFID is the *lookup*
key for IFDB; the filename is the *storage* key. The sidecar records the IFID it fetched
for so the two can be checked against each other.

### Precedence

For each field independently, first non-empty wins:

```
IFmd (the file's own)  >  IFDB (fetched)  >  known_title TSV  >  filename stem
```

IFmd outranks IFDB because it is the author's own metadata, shipped inside the exact file
in hand; IFDB is a community record about a *work*, which may span editions (the live Zork
response lists nine IFIDs across Z-code, Hugo, and Glulx ports). An explicit fetch
therefore *fills gaps* and never overwrites what the file itself asserts. The TSV keeps
its current job as a fallback for bare files we have not fetched.

Cover art follows the same rule: a story's own `Fspc` frontispiece always wins; a fetched
`cover.png` is used only when there is no `Fspc`.

**Where it resolves: `scan_stories`.** Resolution happens once, at scan time, filling
`StoryEntry.title` and new `StoryMeta` fields (`author`, `year`, `genre`, `language`,
`description`). Everything downstream — list rows, sorting, the info panel — then reads
plain fields and needs no notion of where a value came from.

This forces a signature change:

```rust
// before
pub fn scan_stories(dir: &Path) -> Vec<StoryEntry>

// after — needs data_base to locate <data_base>/<story-key>.save/info.json
pub fn scan_stories(dir: &Path, data_base: &Path) -> Vec<StoryEntry>
```

Cost per story: one small JSON read from a directory the picker already stats for the save
badge (`compute_row_badges` → `game_dir_has_save`, `picker.rs:594`). Sidecar reads that
fail, parse wrong, or fail the identity check are simply absent metadata — never a scan
error, never a skipped story.

**After a fetch, re-resolve.** An `r` sweep writes sidecars while the picker holds an
already-resolved `Vec<StoryEntry>`. Each completed story re-resolves its own entry in place
from the block just written, so titles appear as the sweep progresses rather than after a
restart. This is what makes the list re-sort mid-sweep — see "Sorting".

### `app::ifdb` — the client

- **Endpoint:** `GET https://ifdb.org/viewgame?ifiction&ifid=<IFID>` (verified live).
- **Cover:** a *second* `GET` to the `<coverart><url>` from the response — live form
  `https://ifdb.org/coverart?id=<tuid>&version=<n>`. Note this is **not**
  `viewgame?coverart`; that guess was wrong and was corrected against the live API.
  Decode through the existing `image` dep; store as `cover.png`.
- **Not found:** IFDB may 404 or return an empty index. Both are *completed* fetches and
  record `not_found: true`. A transport error, timeout, or 5xx is **not** a completed
  fetch: it writes no block at all, so the next `r` retries it. This is the whole reason
  the block's presence — rather than a status enum — is the skip signal.
- **Transport:** `ureq` — blocking, rustls (no system OpenSSL, so Windows/Linux/macOS all
  work per the cross-platform requirement), no async runtime. Blocking is the *right*
  shape here: it runs on a worker thread, mirroring the established `CoverDecoder` pattern
  (`cover.rs:117-160`).
- **Etiquette.** IFDB is volunteer-run and a 50-story library is 50-100 requests:
  - `User-Agent: lanthorn/<CARGO_PKG_VERSION> (+https://github.com/sharkusk/lanthorn)`
  - a fixed inter-request delay (**500 ms**) between stories in an `r` sweep — it is
    user-initiated and visibly progressing, so throughput is not the priority
  - 10 s timeout per request; a failure is per-story, never fatal to the sweep
  - `r` skips stories settled at the current fetch version, so a second `r` on an
    unchanged library issues **zero** requests. This is the main protection against
    repeat load, and it is why `FETCH_VERSION` must not track the app version.
  - `f` is forced by design and thus unthrottled, but it is one request per keypress on
    one story — human-paced by construction.
  - Cap response body reads (**1 MiB** XML, **8 MiB** cover) so a hostile or broken
    response cannot exhaust memory.

### The scan UI

Two new picker keys. Both unshifted, deliberately: the `slide.open && shift` branch at
`picker_ui.rs:289` swallows unmatched keys, so a `Shift`-bound scan key would silently do
nothing while the info panel is open.

| Key | Scope | Cache behaviour |
|-----|-------|-----------------|
| `f` | The **selected** story only | **Forced** — always refetches and overwrites, whatever is cached |
| `r` | The **whole library** | Skips any story whose `fetched.fetch_version == FETCH_VERSION`; fetches everything else |
| `Esc` | — | Cancels a running sweep; sidecars already written stay |

The two are deliberately asymmetric. `f` is the "this one record looks wrong, go get it
again" escape hatch, so it must never consult the cache. `r` is the bulk sweep, so it must
never redo settled work — but "settled" means *settled by the scanner we're running now*,
which is what makes it double as the rescan-all: bump `FETCH_VERSION` and the next
`r` refreshes the library on its own.

What `r` re-fetches, precisely:

| Sidecar state | `r` |
|---|---|
| no `fetched` block (never tried, or last attempt errored) | fetch |
| `fetched` block, older `fetch_version` | fetch |
| `fetched` block, current `fetch_version`, found | **skip** |
| `fetched` block, current `fetch_version`, `not_found` | **skip** — a completed answer |

**One mechanism serves both.** `f` is not a special case — it is a work order of length
one that skips the cache check. That keeps a single code path: same worker, same progress
channel, same cancel, same rendering. `f` on 1 story and `r` on 200 differ only in what the
picker puts in the order.

```rust
struct FetchOrder {
    stories: Vec<(PathBuf, String /*ifid*/)>,
    /// `f` → true (ignore cached blocks entirely).
    /// `r` → false (skip stories settled at FETCH_VERSION).
    forced: bool,
}
```

Following the `CoverDecoder` pattern (`cover.rs:117-160`), which already proves out in
this exact loop:

- a worker thread receives the `FetchOrder` on an `mpsc`
- it fetches, writes each sidecar as it completes, and sends `FetchProgress { done, total,
  title, outcome }` back per story
- the picker loop drains non-blocking each iteration (as it does covers at
  `picker_ui.rs:221-226`), redraws on arrival, and keeps the 16 ms busy tick
  (`picker_ui.rs:260-268`) while work is in flight
- cancellation: an `AtomicBool` the worker checks between stories. Never mid-write, so a
  cancelled sweep leaves every written sidecar complete and valid.

The skip decision is the worker's, not the picker's — it re-reads each sidecar just before
fetching. This keeps `r` correct even if a sweep is cancelled and restarted, or if `f`
refreshed a story while a sweep was queued.

Progress renders as a one-line bar in the browser footer, from the same `FetchProgress`:

- `r` — `Fetching 7/23 — Zork I`, then `Fetched 19, skipped 3, not found 1`
- `f` — `Fetching Zork I…`, then `Fetched Zork I` / `No IFDB record for Zork I` /
  `Fetch failed: timed out`

Per project policy every new UI element is themeable: new `ColorScheme` fields +
`style.rs` selectors + render apply. No hard-coded styles.

### Story list redesign

The list is where the metadata earns its keep — the info panel shows one story at a time
and only when opened. Today a row is (`picker_ui.rs:408-461`):

```
▸ Zork I   (zork1-invclues-r52-s871125.z5)                          ZBSH
```

New layout — aligned columns under a dimmed header:

```
  TITLE                   AUTHOR                  YEAR
 ────────────────────────────────────────────────────────────────────
 ▸ Anchorhead              Michael S. Gentry       1998    ZB H
   Curses                  Graham Nelson           1993    Z B
   Zork I                  Marc Blank and Dave…    1980    ZBSH
   zork2-r63-s860811.z5    (no metadata yet)               Z S
```

The badge cluster (`type`/`blorb`/`save`/`hint`) is unchanged — same glyphs, same
right-aligned fixed columns, same reverse-on-selection treatment (`picker_ui.rs:435-459`).

**Title comes free.** The row already renders `entry.title`; it is the *source* of that
field that changes (see "Precedence"). No render change is needed for the title itself.

**Empty columns are the common case, not the edge case.** Until an `r` sweep runs, a bare-z
library has no author or year at all. A row with no metadata puts the filename in the title
column (as today, via the stem fallback) and a dimmed `(no metadata yet)` in the author
column, so a fresh library reads as "nothing fetched yet" rather than as broken.

**Narrow widths.** Columns drop right-to-left — year first, then author — leaving
title + badges at the narrowest. Each step is a coherent layout with no gap. This matters
because the info panel takes roughly half the width when open (`split_picker_area`,
`picker_ui.rs:33`), so the dropped states are normal operation, not a corner case.

### Sorting

```rust
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortKey { Title, Author, Year }

pub struct Sort { pub key: SortKey, pub desc: bool }   // default: Title, ascending
```

`sort_stories(&mut [StoryEntry], Sort)` is a pure function — the whole sort is testable
without a terminal.

- **Click a header** → sort by that column. Click the active header again → reverse.
  Hit-testing reuses the existing `row_rects` pattern (`picker_ui.rs:142`, `327`): the
  draw returns `header_rects: Vec<(SortKey, Rect)>`, the click handler finds the hit.
- **Keyboard equivalent: `s` cycles the column** (Title → Author → Year → Title), **`d`
  toggles direction.** Two unshifted keys rather than one shifted pair, because the
  `slide.open && shift` branch (`picker_ui.rs:289`) swallows unmatched keys while the info
  panel is open — a `Shift`-bound sort would silently do nothing exactly when the user can
  see the columns. Rationale for having a keyboard path at all: the browser is otherwise
  fully keyboard-navigable, so a mouse-only sort would make this the one feature
  unreachable without a pointer.
- **Blanks sort last**, in both directions — a story with no author must not outrank one
  with an author just because empty-string sorts first. Ties break on filename, as today.
- The active header shows the direction (`TITLE ▲`) and is styled distinctly from the
  inactive ones.
- **Sort state is in-session only.** Not persisted to `config.toml`. Flagged as an easy
  follow-up if it turns out to want persisting; not built on speculation.

**The footer must grow.** It currently reads (`picker_ui.rs:469`):

```
 ↑/↓ or j/k: move   PgUp/PgDn   Enter / click: open   i/Tab: info   q / Esc: quit
```

Four new bindings (`f`, `r`, `s`, `d`) do not fit at 80 columns. The footer drops hints
right-to-left as width shrinks, keeping the least guessable ones longest — `f`/`r` outrank
`PgUp/PgDn`, which nobody needs told. `draw_str_clipped` already truncates rather than
wrapping, so the failure mode today is a silently half-shown hint; that is what the drop
order replaces.

**Selection must survive a re-sort.** The selection is an index (`list.selected`,
`picker_ui.rs:412`), so anything that reorders the list silently moves the cursor to a
different game. Three things reorder it: changing the sort key, toggling direction, and
**an `r` sweep landing new titles** (titles change → the default title sort reorders under
a cursor the user is not touching). Every reorder must therefore capture the selected
`PathBuf` first and restore the index by path afterwards. This is the most likely bug in
the whole quest and it is invisible until it bites — a user watches a sweep finish and
presses Enter on what is now a different game.

### Info panel additions

Extends `draw_info_panel` (`picker_ui.rs:481-681`) below the existing title/format block:

```
Zork I
zork1-invclues-r52-s871125.z5 · 105 KB · 2026-06-14
Z-code v5 · Release 52
Serial 871125
IFID ZCODE-52-871125
Marc Blank and Dave Lebling · 1980 · Zorkian/Cave crawl        <- new
                                                                <- new
Many strange tales have been told of the fabulous treasure,     <- new
exotic creatures, and diabolical puzzles in the Great           <- new
Underground Empire. […]                                         <- new
Features: sound graphics
```

The blurb wraps to the panel width and participates in the existing panel scroll
(`panel_scroll`/`panel_max`), which already handles overflow.

## Testing

Unit, no network:

- `ifiction::parse` on a captured IFDB response fixture (the live Zork XML above) → asserts
  title/author/description/tuid/cover_url, and that the 25 download `<title>` elements do not
  confuse the bibliographic one.
- `ifiction::parse` on a minimal IFmd chunk carrying only `<title>` → all other fields
  `None`, no error.
- `ifiction::parse` on malformed XML → `Err`, never a panic.
- `blorb`: a fixture blorb with an `IFmd` chunk → `metadata()` returns the exact bytes; one
  without → `None`; the existing `RIdx`/`Fspc` tests must still pass.
- Precedence: a table test over (IFmd, fetched, TSV, stem) presence combinations asserting
  the documented ranking, including the cover `Fspc`-wins rule.
- Sidecar round-trip; unknown `format_version` → ignored, not an error; malformed JSON →
  ignored.
- **Identity check**: a sidecar whose `ifid` disagrees with the story on disk → every block
  ignored, story reads as unfetched.
- **`sort_stories`** — a pure function, so all of it is testable: each key ascending and
  descending; blanks last in *both* directions; filename tie-break; stability across equal
  keys.
- **Selection survives a reorder** — the highest-value test in the quest. Select a story,
  re-sort, assert the selected `PathBuf` is unchanged (not the index). Repeat for: key
  change, direction toggle, and a simulated sweep that renames titles underneath. This bug
  is invisible in review and obvious to a user pressing Enter on the wrong game.
- **Column drop** — the layout at each width threshold, including that a dropped column
  leaves no gap and the badge cluster stays right-aligned.
- **The skip predicate** — a pure function over (`FetchOrder.forced`, sidecar state)
  covering every row of the `r` table above, plus `forced: true` overriding all four. This
  is the quest's most breakable logic and it costs nothing to test.
- The worker against a fake client: a 404 and a 5xx must behave differently — 404 writes a
  `not_found` block, 5xx writes **nothing** — and a mid-sweep cancel must leave prior
  sidecars intact.

The HTTP client is behind a trait so the scan worker is testable against a fake that
returns canned responses, failures, and 404s — no live IFDB in the test suite.

**Not unit-testable, needs a real smoke** (per project policy these are what `confirm`
is for): pressing `f` against live IFDB with a real library, the progress line's feel,
Esc-cancel mid-sweep, a fetched cover actually rendering, whether the column widths and
header look right at real terminal sizes, and whether header clicks land where they look
like they should.

## Risks

| Risk | Handling |
|------|----------|
| Two new deps (`ureq`, `roxmltree`) — first network + TLS in the workspace | Confined to `app`; `blorb` and the VM crates stay zero-dep. rustls keeps it cross-platform. |
| The app now makes network requests at all | Never without an explicit keypress. No timer, no first-run prompt, no background retry. |
| IFDB changes its API or goes away | Every field optional, failures per-story and non-fatal; the app degrades to exactly today's behaviour. |
| Hammering a volunteer site | 500 ms spacing in a sweep; a second `r` on an unchanged library issues zero requests; identifying User-Agent; capped bodies. |
| `FETCH_VERSION` forgotten on a fetch-algorithm change | Stale blocks survive an `r` that should have refreshed them. Mitigated by `f` (always forced, per story) and recoverable by bumping in the next release. Called out in the module docs. |
| Scan slows picker startup | It doesn't — fetching is keypress-driven and threaded; `scan_stories` gains only a chunk lookup, a small XML parse for blorbs, and one small JSON read per story. |
| Selection jumps when the list reorders | Every reorder captures and restores the selected `PathBuf`. Directly tested; called out as the quest's likeliest bug. |
| The list looks empty on a fresh bare-z library | Author/year are blank until `r` runs — expected, not a defect. The author column reads `(no metadata yet)` so the state is legible, and the footer advertises `r`. |

## Decisions taken during review

- **`f` = selected story, forced; `r` = whole library, skip-if-current.** Not the batch/
  fill-gaps pair originally specced. `f` is the "this record looks wrong" escape hatch, so
  it must ignore the cache; `r` is the sweep, so it must not redo settled work.
- **A dedicated `FETCH_VERSION`, bumped only when the fetch algorithm changes** — not the
  app version. This is what lets `r` serve as the rescan-all: bump it and the next sweep
  refreshes the library by itself, with no third binding and no load on IFDB in between.
- **The story list is in scope**, not a follow-up quest. It is where the metadata is
  actually seen, and splitting it would mean two passes over the same `draw_picker` code.
- **Aligned columns with a header**, over an enriched single line or two-line rows.
- **Click-to-sort headers**, plus an `s`/`d` keyboard path so sort is not the browser's one
  mouse-only feature.
