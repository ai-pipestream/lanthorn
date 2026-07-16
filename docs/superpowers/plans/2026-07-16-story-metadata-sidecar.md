# Story Metadata Sidecar + Sortable Story List — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the story browser real metadata — title, author, year, genre, blurb, cover — from the story's own `IFmd` chunk automatically and from IFDB on an explicit keypress, cached in a per-story sidecar, and show it as a sortable column list.

**Architecture:** `blorb` gains only raw `IFmd` byte exposure (stays zero-dep). `app` gains four modules: `ifiction` (one XML parser serving both sources), `story_info` (the `info.json` sidecar), `ifdb` (HTTP client behind a trait), and a fetch worker on the established `CoverDecoder` thread+mpsc pattern. Precedence resolves once in `scan_stories`, so every consumer downstream reads plain fields.

**Tech Stack:** Rust 2021, ratatui 0.30, `roxmltree 0.21` (XML), `ureq 3.3` (HTTP, rustls).

**Spec:** `docs/superpowers/specs/2026-07-15-story-metadata-sidecar-design.md` — read it before Task 1.

## Global Constraints

- **`crates/blorb` stays zero-dep.** Its `[dependencies]` is empty and must remain empty. It exposes `IFmd` bytes; it never parses them. `zvm`/`gvm` are likewise untouched.
- **`ureq` is 3.3, NOT 2.x.** The API changed. Verified surface: `Agent::config_builder().timeout_global(Some(Duration)).build()`; `agent.get(url).header("User-Agent", ua).call()?`; `.body_mut().read_to_string()?`. **A 404 arrives as `Err(ureq::Error::StatusCode(404))`, not `Ok`** — see Task 4. Do not write ureq 2.x code from memory; check `cargo doc -p ureq --open` for anything not stated here.
- **No automatic network, ever.** No timer, no first-run prompt, no background retry, no fetch-on-idle. The only two things that may touch the network are the `f` and `r` keypresses.
- **`FETCH_VERSION` starts at `1`** and is a `u32` constant in `app::story_info`. Never `CARGO_PKG_VERSION`.
- **User-Agent is exactly** `babelmap/<CARGO_PKG_VERSION> (+https://github.com/sharkusk/babelmap)`.
- **Inter-request delay in an `r` sweep: 500 ms. Per-request timeout: 10 s. Body caps: 1 MiB XML, 8 MiB cover.**
- **Every new UI element is themeable**: a `ColorScheme` field + a `style.rs` selector + applied at render. Never a hard-coded `Style`/`Color`. This is a project standing rule.
- **Pre-release: no back-compat.** A sidecar with an unknown `format_version` is ignored and overwritten. No migration, no tolerant decoding, no old-file fixtures.
- **Cross-platform**: Windows/Linux/macOS. rustls (not native-tls) is why `ureq`'s default features are acceptable.
- **Run `cargo test --workspace` before every commit.** Baseline is 28 green test binaries, 0 failures. Clippy baseline is 25 warnings across app+mapper — do not increase it.
- Commit messages end with the `Quest: SQ-0348` trailer. Do not push.

---

### Task 1: `blorb` — expose the `IFmd` chunk

**Files:**
- Modify: `crates/blorb/src/lib.rs` (chunk walk ~`85-111`; `build_blorb` test helper ~`345-379`)

**Interfaces:**
- Consumes: nothing.
- Produces: `impl Blorb { pub fn metadata(&self) -> Option<&[u8]> }` — the raw `IFmd` chunk body, no interpretation. Task 5 calls it.

Context: the top-level walk currently recognises `RIdx` and `Fspc` only; every other chunk is skipped by length. `IFmd` is a top-level chunk (a sibling of `RIdx`), **not** an `RIdx`-indexed resource — so `build_blorb`, which only emits resources, cannot produce a test fixture as written and must be extended.

- [ ] **Step 1: Extend the `build_blorb` test helper to emit top-level chunks**

In the `#[cfg(test)] mod tests`, add a variant that appends extra top-level chunks after `RIdx` and before the resource body. Keep `build_blorb` itself working unchanged (existing tests call it):

```rust
fn build_blorb(res: &[BlorbRes]) -> Vec<u8> {
    build_blorb_with_top(res, &[])
}

/// `top` = extra top-level chunks as (type, data), emitted after RIdx.
/// Resource offsets must account for their size — hence the shared body layout.
fn build_blorb_with_top(res: &[BlorbRes], top: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let count = res.len() as u32;
    let ridx_data_len = 4 + 12 * res.len();
    let mut top_chunks = Vec::new();
    for (ty, data) in top {
        top_chunks.extend_from_slice(&chunk(ty, data));
    }
    // RIdx header (8) sits at offset 12; resources follow RIdx AND the top chunks.
    let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2) + top_chunks.len();
    let mut offsets = Vec::new();
    let mut cursor = first_res_off;
    let mut body = Vec::new();
    for (_u, _n, ty, data) in res {
        offsets.push(cursor as u32);
        let c = chunk(ty, data);
        cursor += c.len();
        body.extend_from_slice(&c);
    }
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&count.to_be_bytes());
    for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
        ridx.extend_from_slice(*usage);
        ridx.extend_from_slice(&number.to_be_bytes());
        ridx.extend_from_slice(&offsets[i].to_be_bytes());
    }
    let ridx_chunk = chunk(b"RIdx", &ridx);
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&ridx_chunk);
    inner.extend_from_slice(&top_chunks);
    inner.extend_from_slice(&body);
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    file
}
```

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn ifmd_chunk_is_exposed_verbatim() {
    let xml = br#"<ifindex version="1.0"><story><bibliographic><title>T</title></bibliographic></story></ifindex>"#;
    let b = Blorb::parse(build_blorb_with_top(
        &[(b"Exec", 0, b"ZCOD", b"abcd")],
        &[(b"IFmd", xml)],
    ))
    .unwrap();
    assert_eq!(b.metadata(), Some(&xml[..]), "IFmd bytes returned uninterpreted");
}

#[test]
fn blorb_without_ifmd_has_no_metadata() {
    let b = Blorb::parse(build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")])).unwrap();
    assert_eq!(b.metadata(), None);
}

/// An odd-length IFmd is padded to even in the container; the returned slice
/// must be the DECLARED length, not the padded one, or the XML gains a NUL
/// byte and roxmltree rejects it.
#[test]
fn odd_length_ifmd_excludes_its_pad_byte() {
    let odd = b"<ifindex></ifindex>"; // 19 bytes → forces a pad byte
    assert_eq!(odd.len() % 2, 1, "the fixture must be odd for this test to mean anything");
    let b = Blorb::parse(build_blorb_with_top(
        &[(b"Exec", 0, b"ZCOD", b"abcd")],
        &[(b"IFmd", odd)],
    ))
    .unwrap();
    assert_eq!(b.metadata(), Some(&odd[..]), "no trailing pad byte");
}

/// Adding a top-level chunk must not break resource offset resolution.
#[test]
fn resources_still_resolve_with_an_ifmd_present() {
    let b = Blorb::parse(build_blorb_with_top(
        &[(b"Pict", 1, b"PNG ", b"pngdata")],
        &[(b"IFmd", b"<ifindex/>")],
    ))
    .unwrap();
    assert_eq!(b.resource(b"Pict", 1).map(|r| r.data), Some(&b"pngdata"[..]));
}
```

Note: `resource()`'s exact return shape may differ — match the existing tests in the file (see `resolve_*` tests ~`631-715`) rather than the sketch above.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p blorb ifmd`
Expected: FAIL — `no method named 'metadata'`.

- [ ] **Step 4: Implement**

Add an `ifmd: Option<(usize, usize)>` local to `parse`, recorded in the walk beside `Fspc`, stored as a field on `Blorb` (`(start, len)` into the owned `bytes`), and read back by `metadata()`. Match the surrounding style — the walk's existing `else if` chain, and `Fspc`'s `clen >= 4` guard is the precedent for a length check:

```rust
} else if &ctype == b"IFmd" {
    ifmd = Some((data_start, clen));
}
```

`metadata()` returns `self.ifmd.map(|(s, l)| &self.bytes[s..s + l])`. `data_start + clen > end` is already checked by the loop's `break`, so the slice cannot be out of range.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p blorb`
Expected: PASS, including every pre-existing blorb test (the `build_blorb` refactor must not disturb them).

- [ ] **Step 6: Confirm blorb is still zero-dep**

Run: `grep -A3 '^\[dependencies\]' crates/blorb/Cargo.toml`
Expected: no entries under `[dependencies]`.

- [ ] **Step 7: Commit**

```bash
git add crates/blorb/src/lib.rs
git commit -m "feat(blorb): expose the IFmd metadata chunk as raw bytes

Quest: SQ-0348"
```

---

### Task 2: `app::ifiction` — the shared iFiction parser

**Files:**
- Create: `crates/app/src/ifiction.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod ifiction;`)
- Modify: `crates/app/Cargo.toml` (add `roxmltree = "0.21"`)
- Test fixture (already committed, do not refetch): `crates/app/tests/fixtures/ifdb-zork1.xml`

**Interfaces:**
- Consumes: nothing.
- Produces: `IFiction`, `IfdbExt`, `IFictionError`, `pub fn parse(xml: &[u8]) -> Result<IFiction, IFictionError>`. Tasks 4 and 5 consume it.

Context: IFDB and a blorb's `IFmd` chunk serve the **same** format, so one parser serves both. The fixture is a real IFDB response captured 2026-07-16 from `ifdb.org/viewgame?ifiction&ifid=ZCODE-88-840726-A129`.

**The trap this task exists to avoid:** the fixture contains **26** `<title>` elements — one bibliographic, the rest inside `<downloads><link>`. A naive descendant search returns the wrong one. Match on local name **within the Babel namespace** (`http://babel.ifarchive.org/protocol/iFiction/`) and **only as a direct child of `<bibliographic>`**.

- [ ] **Step 1: Add the dep**

```bash
cargo add --package app roxmltree@0.21
```

- [ ] **Step 2: Write the failing tests**

Create `crates/app/src/ifiction.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ZORK: &[u8] = include_bytes!("../tests/fixtures/ifdb-zork1.xml");

    /// The live IFDB response. Guards the 26-<title> trap: all but one are
    /// inside <downloads><link>; only <bibliographic><title> is the game's.
    #[test]
    fn parses_the_live_ifdb_response() {
        let f = parse(ZORK).expect("fixture parses");
        assert_eq!(f.title.as_deref(), Some("Zork I"), "the bibliographic title, not a download's");
        assert_eq!(f.author.as_deref(), Some("Marc Blank and Dave Lebling"));
        assert_eq!(f.first_published.as_deref(), Some("1980"));
        assert_eq!(f.genre.as_deref(), Some("Zorkian/Cave crawl"));
        assert!(
            f.description.as_deref().unwrap().starts_with("Many strange tales"),
            "the blurb: {:?}", f.description
        );
        assert!(f.ifids.contains(&"ZCODE-52-871125".to_string()), "IFDB groups editions: {:?}", f.ifids);
    }

    #[test]
    fn extracts_the_ifdb_extension_block() {
        let f = parse(ZORK).unwrap();
        let ext = f.ifdb.expect("ifdb extension present");
        assert_eq!(ext.tuid, "0dbnusxunq7fw5ro");
        assert_eq!(
            ext.cover_url.as_deref(),
            Some("https://ifdb.org/coverart?id=0dbnusxunq7fw5ro&version=45"),
            "entity-decoded: &amp; must become &"
        );
    }

    /// A thin IFmd chunk carrying only a title must parse, not error — most of
    /// the struct is legitimately absent.
    #[test]
    fn a_minimal_ifmd_chunk_parses_with_everything_else_none() {
        let xml = br#"<ifindex version="1.0" xmlns="http://babel.ifarchive.org/protocol/iFiction/">
            <story><bibliographic><title>Curses</title></bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Curses"));
        assert!(f.author.is_none() && f.description.is_none() && f.ifdb.is_none());
    }

    /// An IFmd chunk with no default namespace (some tools emit bare iFiction).
    /// Accepted: namespace-absent is not namespace-wrong.
    #[test]
    fn a_namespaceless_chunk_still_parses() {
        let xml = br#"<ifindex><story><bibliographic><title>Bare</title></bibliographic></story></ifindex>"#;
        assert_eq!(parse(xml).unwrap().title.as_deref(), Some("Bare"));
    }

    #[test]
    fn malformed_xml_errors_and_never_panics() {
        assert!(parse(b"<ifindex><story>").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"\xff\xfe\x00garbage").is_err());
    }

    /// Whitespace around element text is incidental in XML; a title of
    /// "\n  Zork I\n  " must not reach the UI.
    #[test]
    fn text_is_trimmed_and_blanks_become_none() {
        let xml = br#"<ifindex><story><bibliographic>
            <title>  Spaced  </title><author>   </author>
        </bibliographic></story></ifindex>"#;
        let f = parse(xml).unwrap();
        assert_eq!(f.title.as_deref(), Some("Spaced"));
        assert!(f.author.is_none(), "whitespace-only is absent, not Some(\"\")");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p app ifiction`
Expected: FAIL — module has no `parse`.

- [ ] **Step 4: Implement**

```rust
//! Treaty of Babel iFiction metadata.
//!
//! One parser for two sources: a blorb's `IFmd` chunk and IFDB's
//! `viewgame?ifiction` response are the same format. IFDB additionally carries an
//! `<ifdb>` extension element in its own namespace, which an IFmd chunk lacks.

const BABEL_NS: &str = "http://babel.ifarchive.org/protocol/iFiction/";
const IFDB_NS: &str = "http://ifdb.org/api/xmlns";

/// Parsed iFiction. Every field is optional: an IFmd chunk may carry only a
/// subset, and thin metadata must never fail a scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfdbExt {
    pub tuid: String,
    pub link: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug)]
pub enum IFictionError {
    Xml(roxmltree::Error),
    /// Well-formed XML that is not an iFiction document.
    NotIFiction,
}

pub fn parse(xml: &[u8]) -> Result<IFiction, IFictionError> { /* see below */ }
```

Implementation notes — follow these exactly, they encode the traps:

1. `std::str::from_utf8(xml)` first; a UTF-8 error is an `IFictionError`, not a panic.
2. `roxmltree::Document::parse(s)` → map error to `IFictionError::Xml`.
3. Find `<story>` as a descendant. No `<story>` → `NotIFiction`.
4. Helper `fn child_text<'a>(parent: roxmltree::Node<'a, 'a>, name: &str) -> Option<String>`: finds a **direct child** element whose `tag_name().name() == name` **and** whose namespace is `BABEL_NS` **or `None`** (the namespaceless case), takes its text, `trim()`s it, and returns `None` when empty. Direct-child-only is what keeps `<downloads><link><title>` out.
5. `<bibliographic>` is a direct child of `<story>`; pull title/author/language/firstpublished/genre/description from it via `child_text`.
6. `<identification>` → collect every `<ifid>` child's trimmed text into `ifids`.
7. The `<ifdb>` element is in `IFDB_NS` — match on namespace, not just name. `tuid` is required for an `IfdbExt`; without it, `ifdb` is `None`. `cover_url` is `<coverart><url>`. roxmltree decodes `&amp;` for you — do not hand-decode.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app ifiction`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/ifiction.rs crates/app/src/lib.rs crates/app/Cargo.toml Cargo.lock crates/app/tests/fixtures/ifdb-zork1.xml
git commit -m "feat(app): parse Treaty of Babel iFiction metadata

One parser for both sources: a blorb IFmd chunk and an IFDB
viewgame?ifiction response are the same format.

Quest: SQ-0348"
```

---

### Task 3: `app::story_info` — the sidecar

**Files:**
- Create: `crates/app/src/story_info.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod story_info;`)

**Interfaces:**
- Consumes: `crate::storage::{story_key, game_dir}` (existing, `storage.rs:14-28`) — reuse verbatim, do not reimplement.
- Produces: `StoryInfo`, `FetchedMeta`, `FETCH_VERSION`, `load(game_dir, expect_ifid) -> Option<StoryInfo>`, `save(game_dir, &StoryInfo) -> io::Result<()>`, `needs_fetch(Option<&StoryInfo>, forced) -> bool`, `info_path(game_dir) -> PathBuf`. Tasks 5, 6, 9 consume it.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bm_story_info_{}_{:p}", std::process::id(), &0u8 as *const u8));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fetched(v: u32) -> FetchedMeta {
        FetchedMeta {
            scanned_at: "2026-07-16T00:00:00Z".into(),
            fetch_version: v,
            source: "ifdb".into(),
            title: Some("Zork I".into()),
            author: Some("Marc Blank and Dave Lebling".into()),
            language: None, first_published: Some("1980".into()), genre: None,
            description: None, ifdb_tuid: None, ifdb_link: None, cover: None,
            not_found: false,
        }
    }

    fn info(ifid: &str, f: Option<FetchedMeta>) -> StoryInfo {
        StoryInfo { format_version: FORMAT_VERSION, ifid: ifid.into(), fetched: f, probe: None }
    }

    #[test]
    fn round_trips() {
        let d = tmp();
        let i = info("ZCODE-52-871125", Some(fetched(FETCH_VERSION)));
        save(&d, &i).unwrap();
        assert_eq!(load(&d, "ZCODE-52-871125"), Some(i));
    }

    /// SPEC "Identity check": the sidecar is keyed by FILENAME but describes an
    /// IFID. Swap a different game in under the same filename and the stale
    /// sidecar would otherwise hand it Zork's blurb and cover.
    #[test]
    fn a_sidecar_for_a_different_ifid_is_ignored_entirely() {
        let d = tmp();
        save(&d, &info("ZCODE-52-871125", Some(fetched(FETCH_VERSION)))).unwrap();
        assert_eq!(load(&d, "ZCODE-88-840726"), None, "wrong IFID → every block stale");
    }

    #[test]
    fn unknown_format_version_is_ignored_not_an_error() {
        let d = tmp();
        let mut i = info("X", None);
        i.format_version = 9999;
        save(&d, &i).unwrap();
        assert_eq!(load(&d, "X"), None);
    }

    #[test]
    fn malformed_json_is_ignored() {
        let d = tmp();
        std::fs::write(info_path(&d), b"{ not json").unwrap();
        assert_eq!(load(&d, "X"), None);
    }

    #[test]
    fn a_missing_sidecar_is_none_not_an_error() {
        assert_eq!(load(&tmp().join("nope"), "X"), None);
    }

    /// SPEC "The scan UI" — the skip table. This predicate is the quest's most
    /// breakable logic and costs nothing to test.
    #[test]
    fn needs_fetch_matches_the_spec_table() {
        // r (forced = false)
        assert!(needs_fetch(None, false), "never tried, or last attempt errored → fetch");
        assert!(needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION - 1)))), false), "older fetch_version → fetch");
        assert!(!needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION)))), false), "current, found → skip");
        let mut nf = fetched(FETCH_VERSION);
        nf.not_found = true;
        assert!(!needs_fetch(Some(&info("X", Some(nf))), false), "current, not_found → skip: a completed answer");
        // A sidecar that exists only for a probe block has never been fetched.
        assert!(needs_fetch(Some(&info("X", None)), false), "no fetched block → fetch");
        // f (forced = true) ignores all of it.
        assert!(needs_fetch(Some(&info("X", Some(fetched(FETCH_VERSION)))), true), "forced overrides current+found");
        assert!(needs_fetch(None, true));
    }

    /// A probe block must survive a fetch rewriting the fetched block — the two
    /// writers must not clobber each other (SQ-0276 depends on this).
    #[test]
    fn writing_a_fetched_block_preserves_an_existing_probe_block() {
        let d = tmp();
        let mut i = info("X", None);
        i.probe = Some(ProbeMeta::default());
        save(&d, &i).unwrap();
        let mut loaded = load(&d, "X").unwrap();
        loaded.fetched = Some(fetched(FETCH_VERSION));
        save(&d, &loaded).unwrap();
        let back = load(&d, "X").unwrap();
        assert!(back.probe.is_some() && back.fetched.is_some());
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p app story_info`
Expected: FAIL — no such module.

- [ ] **Step 3: Implement**

```rust
//! Per-story metadata cache: `<data_base>/<story-key>.save/info.json`.
//!
//! Caches ONLY what cannot be cheaply recomputed from the story file — the IFDB
//! fetch, and (SQ-0276) a runtime capability probe. A blorb's own `IFmd` is NOT
//! cached: `scan_stories` already holds the bytes, so the blorb is the cache.
//!
//! Keyed by filename (SQ-0284) but describing an IFID, so `load` checks the two
//! agree — see `load`.

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;

/// The fetch algorithm's version. **Bump when a re-fetch would produce a
/// materially different block** — a new field extracted, a changed endpoint,
/// fixed parsing. Do NOT bump for refactors that cannot change output, and do
/// NOT tie this to CARGO_PKG_VERSION: that would re-fetch every story in every
/// library on every release, for nothing.
///
/// `r` skips stories already fetched at this version, which is what makes it
/// double as the rescan-all: bump this and the next `r` refreshes the library.
pub const FETCH_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryInfo {
    pub format_version: u32,
    /// The IFID these blocks describe. Checked against the story on disk.
    pub ifid: String,
    pub fetched: Option<FetchedMeta>,
    /// Reserved for SQ-0276. Always None here, but preserved across writes.
    pub probe: Option<ProbeMeta>,
}

/// Present ONLY for a fetch that ran to completion — found, or authoritatively
/// not-found. A transport error writes no block, so `r` retries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedMeta {
    pub scanned_at: String,
    pub fetch_version: u32,
    pub source: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub first_published: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub ifdb_tuid: Option<String>,
    pub ifdb_link: Option<String>,
    /// Filename of the cached cover beside this file, e.g. "cover.png".
    pub cover: Option<String>,
    pub not_found: bool,
}

/// SQ-0276's slot. Defined here so writes preserve it; not populated by SQ-0348.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeMeta {
    pub probed_at: Option<String>,
}

pub fn info_path(game_dir: &Path) -> PathBuf { game_dir.join("info.json") }

/// Load, or None if absent/unreadable/malformed/wrong-version/wrong-IFID.
/// Never an error: absent metadata is a normal state, not a failure.
pub fn load(game_dir: &Path, expect_ifid: &str) -> Option<StoryInfo> {
    let raw = std::fs::read(info_path(game_dir)).ok()?;
    let info: StoryInfo = serde_json::from_slice(&raw).ok()?;
    if info.format_version != FORMAT_VERSION || info.ifid != expect_ifid {
        return None;
    }
    Some(info)
}

pub fn save(game_dir: &Path, info: &StoryInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(game_dir)?;
    let json = serde_json::to_string_pretty(info)?;
    std::fs::write(info_path(game_dir), json)
}

/// The `r`/`f` skip decision. `forced` (`f`) ignores the cache entirely.
pub fn needs_fetch(info: Option<&StoryInfo>, forced: bool) -> bool {
    if forced {
        return true;
    }
    match info.and_then(|i| i.fetched.as_ref()) {
        Some(f) => f.fetch_version != FETCH_VERSION,
        None => true,
    }
}
```

Note: `save` writing then a later `load` failing the identity check is intentional and load-bearing — do not "fix" it by dropping the check.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p app story_info`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/story_info.rs crates/app/src/lib.rs
git commit -m "feat(app): per-story info.json sidecar with an identity check

Caches only what cannot be recomputed from the story file. Keyed by
filename but describing an IFID, so load() rejects a sidecar whose IFID
disagrees with the file on disk.

Quest: SQ-0348"
```

---

### Task 4: `app::ifdb` — the HTTP client

**Files:**
- Create: `crates/app/src/ifdb.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod ifdb;`)
- Modify: `crates/app/Cargo.toml` (add `ureq = "3.3"`)

**Interfaces:**
- Consumes: `crate::ifiction::{self, IFiction}`.
- Produces:
  ```rust
  pub enum FetchOutcome { Found(IFiction), NotFound }
  pub enum FetchError { Transport(String) }
  pub trait MetadataSource: Send + Sync {
      fn fetch(&self, ifid: &str) -> Result<FetchOutcome, FetchError>;
      fn fetch_cover(&self, url: &str) -> Result<Vec<u8>, FetchError>;
  }
  pub struct IfdbClient;                 // impl MetadataSource
  impl IfdbClient { pub fn new() -> Self }
  ```
  Task 6 takes a `&dyn MetadataSource` so it can be tested against a fake.

**THE trap — read before implementing.** `ureq` returns **`Err(ureq::Error::StatusCode(404))`** for a 404, not `Ok`. The obvious `match { Ok => write, Err => skip }` would classify "IFDB has no record" as a transport error, so no `not_found` block would ever be written and `r` would re-request those stories on every sweep, forever. **404 → `Ok(FetchOutcome::NotFound)`. Only genuine transport/timeout/5xx → `Err`.** The whole skip design rests on this line.

- [ ] **Step 1: Add the dep**

```bash
cargo add --package app ureq@3.3
```

Verify rustls, not native-tls: `cargo tree -p app -i native-tls` must report nothing found.

- [ ] **Step 2: Write the failing tests**

Network is not available in tests. Test only what is pure:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifiction_url_is_the_verified_endpoint() {
        assert_eq!(
            ifiction_url("ZCODE-52-871125"),
            "https://ifdb.org/viewgame?ifiction&ifid=ZCODE-52-871125"
        );
    }

    /// An IFID reaches us from story bytes and is not guaranteed URL-safe.
    #[test]
    fn ifid_is_percent_encoded_into_the_query() {
        assert!(ifiction_url("A B&c=d").ends_with("ifid=A%20B%26c%3Dd"), "got {}", ifiction_url("A B&c=d"));
    }

    #[test]
    fn user_agent_identifies_babelmap_and_its_repo() {
        let ua = user_agent();
        assert!(ua.starts_with("babelmap/"));
        assert!(ua.contains("github.com/sharkusk/babelmap"));
        assert!(ua.contains(env!("CARGO_PKG_VERSION")));
    }

    /// The cover URL is IFDB's, taken from the response — never constructed by
    /// us. `viewgame?coverart` does not exist; the real form is
    /// `ifdb.org/coverart?id=<tuid>&version=<n>` and only the response knows it.
    #[test]
    fn cover_url_comes_from_the_response_not_from_us() {
        let f = crate::ifiction::parse(include_bytes!("../tests/fixtures/ifdb-zork1.xml")).unwrap();
        assert_eq!(
            f.ifdb.unwrap().cover_url.as_deref(),
            Some("https://ifdb.org/coverart?id=0dbnusxunq7fw5ro&version=45")
        );
    }
}
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test -p app ifdb`
Expected: FAIL — no such module.

- [ ] **Step 4: Implement**

```rust
//! IFDB metadata lookup. Network only — never called except from an explicit
//! `f`/`r` keypress (see the fetch worker).
//!
//! Endpoint verified live 2026-07-16:
//!   GET https://ifdb.org/viewgame?ifiction&ifid=<IFID>   → iFiction XML
//! The cover is a SECOND request to the <coverart><url> in that response
//! (`https://ifdb.org/coverart?id=<tuid>&version=<n>`). There is no
//! `viewgame?coverart` endpoint — do not construct cover URLs.

use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_XML: u64 = 1024 * 1024;          // 1 MiB
const MAX_COVER: u64 = 8 * 1024 * 1024;    // 8 MiB

fn user_agent() -> String {
    format!("babelmap/{} (+https://github.com/sharkusk/babelmap)", env!("CARGO_PKG_VERSION"))
}

fn ifiction_url(ifid: &str) -> String {
    format!("https://ifdb.org/viewgame?ifiction&ifid={}", percent_encode(ifid))
}
```

Notes:
- `percent_encode`: hand-roll a small helper encoding everything outside `[A-Za-z0-9._~-]`. Do not add a `percent-encoding` dep for one call site (YAGNI, and the workspace is dep-conscious).
- Build one `ureq::Agent` per client via `Agent::config_builder().timeout_global(Some(TIMEOUT)).build()`, reused across requests.
- Map results:
  - `Ok(resp)` → read body (capped at `MAX_XML`), `ifiction::parse` → `Ok(Found(f))`. **A parse failure is `Err(Transport)`, not `NotFound`** — a story with no record must not be conflated with a response we failed to read.
  - IFDB may also answer 200 with an `<ifindex>` containing **no** `<story>` — `ifiction::parse` returns `NotIFiction` for that. Treat *that specific* error as `Ok(NotFound)`. Confirm the real shape by hitting a junk IFID by hand once (`curl 'https://ifdb.org/viewgame?ifiction&ifid=ZCODE-1-000000-0000'`) and match the code to what it actually returns — do not assume.
  - `Err(ureq::Error::StatusCode(404))` → `Ok(NotFound)`.
  - Any other `Err` → `Err(Transport(e.to_string()))`.
- Body caps: check `cargo doc -p ureq` for the 3.x limit API on `Body` (e.g. `.body_mut().with_config().limit(n).read_to_vec()`); if unclear, cap by reading through `.as_reader().take(n)`. Do not read unbounded.
- `IfdbClient::new()` must not perform I/O.

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p app ifdb`
Expected: PASS (4 tests).

- [ ] **Step 6: Manually confirm the not-found shape once**

Run: `curl -sS 'https://ifdb.org/viewgame?ifiction&ifid=ZCODE-1-000000-0000' | head -c 300; echo`
Record what it actually returns in a code comment, and make the mapping match. **Report the observed output in your task report.**

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/ifdb.rs crates/app/src/lib.rs crates/app/Cargo.toml Cargo.lock
git commit -m "feat(app): IFDB metadata client behind a MetadataSource trait

404 maps to Ok(NotFound), not Err: a story IFDB has no record for is a
completed answer, and conflating it with a transport error would make the
library sweep re-request it forever.

Quest: SQ-0348"
```

---

### Task 5: Precedence — resolve metadata in `scan_stories`

**Files:**
- Modify: `crates/app/src/picker.rs` (`StoryMeta` ~`42`, `scan_stories` ~`452-557`)
- Modify: callers of `scan_stories` (find with `grep -rn "scan_stories" crates/`)

**Interfaces:**
- Consumes: `blorb::Blorb::metadata()` (Task 1), `ifiction::parse` (Task 2), `story_info::load` (Task 3), existing `session::known_title`.
- Produces: new `StoryMeta` fields `author`, `year`, `genre`, `language`, `description: Option<String>`; `pub fn scan_stories(dir: &Path, data_base: &Path) -> Vec<StoryEntry>`. Tasks 7-10 read these fields.

Context — the precedence, per spec, per field independently, first non-empty wins:

```
IFmd (the file's own)  >  IFDB (fetched sidecar)  >  known_title TSV  >  filename stem
```

IFmd outranks IFDB because it ships inside the exact file in hand; IFDB describes a *work* spanning editions (the fixture lists nine IFIDs across Z-code, Hugo, and Glulx ports). A fetch fills gaps; it never overwrites what the file asserts. The TSV and stem apply to `title` only.

- [ ] **Step 1: Write the failing tests**

```rust
/// SPEC "Precedence". Resolution happens ONCE, here — everything downstream
/// reads plain fields and never asks where a value came from.
#[test]
fn ifmd_outranks_a_fetched_sidecar_field_by_field() {
    let ifmd = IFiction { title: Some("From IFmd".into()), author: None, ..Default::default() };
    let fetched = FetchedMeta { title: Some("From IFDB".into()), author: Some("From IFDB".into()), ..fetched_stub() };
    let r = resolve(Some(&ifmd), Some(&fetched), None, "stem");
    assert_eq!(r.title, "From IFmd", "the file's own metadata wins");
    assert_eq!(r.author.as_deref(), Some("From IFDB"), "but IFDB fills the gap IFmd left");
}

#[test]
fn tsv_then_stem_when_nothing_else_has_a_title() {
    assert_eq!(resolve(None, None, Some("From TSV"), "stem").title, "From TSV");
    assert_eq!(resolve(None, None, None, "stem").title, "stem");
}

#[test]
fn a_not_found_block_contributes_nothing_but_is_not_an_error() {
    let nf = FetchedMeta { not_found: true, title: None, ..fetched_stub() };
    assert_eq!(resolve(None, Some(&nf), Some("From TSV"), "stem").title, "From TSV");
}
```

Then an integration-style test over `scan_stories` with a temp dir: a bare `.z5` with a sidecar → fetched title in `StoryEntry.title`; the same story with a wrong-IFID sidecar → falls back to TSV/stem.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p app precedence`
Expected: FAIL — no `resolve`.

- [ ] **Step 3: Implement**

Add a pure `fn resolve(ifmd: Option<&IFiction>, fetched: Option<&FetchedMeta>, tsv: Option<&str>, stem: &str) -> Resolved` — pure so the whole precedence table is testable without a filesystem. Then in `scan_stories`:

- extract `IFmd` from the blorb parse that already happens at `picker.rs:502-512` (do not re-read the file — the bytes are in hand);
- `story_info::load(&game_dir(data_base, &story_key(&path)), &ifid)` for the fetched block;
- call `resolve`, fill `StoryEntry.title` and the new `StoryMeta` fields.

A sidecar read that fails, parses wrong, or fails the identity check is simply absent metadata — never a scan error, never a skipped story.

- [ ] **Step 4: Update every caller**

Run: `grep -rn "scan_stories" crates/ --include="*.rs"`
Each caller must pass `data_base`. It is already in scope at the picker call site (`ensure_aux` takes it).

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, 0 failures. Titles now resolve from IFmd/sidecar.

- [ ] **Step 6: Commit**

```bash
git add -A crates/app/src
git commit -m "feat(app): resolve story metadata by precedence at scan time

IFmd > fetched > TSV > stem, per field. Resolution happens once in
scan_stories so the list, sort, and info panel read plain fields.

Quest: SQ-0348"
```

---

### Task 6: The fetch worker

**Files:**
- Create: `crates/app/src/fetch_worker.rs`
- Modify: `crates/app/src/lib.rs`

**Interfaces:**
- Consumes: `ifdb::MetadataSource` (Task 4), `story_info::{load, save, needs_fetch, FETCH_VERSION}` (Task 3).
- Produces:
  ```rust
  pub struct FetchOrder { pub stories: Vec<(PathBuf, String)>, pub forced: bool }
  pub enum Outcome { Fetched, Skipped, NotFound, Failed(String) }
  pub struct FetchProgress { pub done: usize, pub total: usize, pub path: PathBuf, pub title: String, pub outcome: Outcome }
  pub struct Fetcher { /* req_tx, res_rx, cancel: Arc<AtomicBool>, _worker */ }
  impl Fetcher {
      pub fn new(source: Box<dyn MetadataSource>, data_base: PathBuf, delay: Duration) -> Self;
      pub fn request(&self, order: FetchOrder);
      pub fn drain(&self) -> Vec<FetchProgress>;
      pub fn cancel(&self);
      pub fn busy(&self) -> bool;
  }
  ```
  Task 9 drives it.

Context: mirror `CoverDecoder` (`cover.rs:117-160`) — a long-lived `std::thread` + two `mpsc` channels, drained non-blocking from the render loop. No async runtime. `delay` is a constructor arg **so tests can pass `Duration::ZERO`**; production passes 500 ms.

`f` is **not** a special case: it is a `FetchOrder` of length one with `forced: true`. One worker, one progress channel, one cancel path.

The skip decision is the **worker's**, not the picker's: it re-reads each sidecar immediately before fetching, so `r` stays correct if a sweep is cancelled and restarted, or if `f` refreshed a story while a sweep was queued.

- [ ] **Step 1: Write the failing tests** (against a fake `MetadataSource`)

```rust
struct Fake { responses: HashMap<String, Result<FetchOutcome, FetchError>>, calls: Arc<Mutex<Vec<String>>> }
```

Tests:
- an `r` order skips a story already at `FETCH_VERSION` — assert the fake was **never called** for it (`calls` is the assertion, not the outcome);
- a `forced` order calls the fake even for a current+found story;
- `NotFound` writes a `FetchedMeta` with `not_found: true`;
- **`Err(Transport)` writes NO sidecar block at all** — reload and assert `fetched.is_none()`, so the next `r` retries it;
- cancel mid-order: sidecars written before the cancel survive, and the fake is not called after it;
- `FetchProgress.done/total` count every story in the order, skips included (the progress line must not stall).

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p app fetch_worker`
Expected: FAIL.

- [ ] **Step 3: Implement**

Worker loop per story: check `cancel` → `story_info::load` → `needs_fetch(info, order.forced)` → if not, send `Outcome::Skipped` and continue (**no delay** — a skip costs no request, so it must not sleep) → else `source.fetch(&ifid)` → build `FetchedMeta { fetch_version: FETCH_VERSION, scanned_at: now_rfc3339(), .. }` → preserve any existing `probe` block → `story_info::save` → on `Found` with a `cover_url` and no local `Fspc`, `fetch_cover` and write `cover.png` → send progress → `thread::sleep(delay)`.

Timestamp via `jiff` (already a dep). Cancel is `Arc<AtomicBool>`, checked **between** stories only — never mid-write, so a cancelled sweep leaves every written sidecar complete and valid.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p app fetch_worker`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/fetch_worker.rs crates/app/src/lib.rs
git commit -m "feat(app): background fetch worker with cancel and progress

f is an order of length one with forced=true, so one code path serves both
keys. The skip decision is the worker's: it re-reads each sidecar just
before fetching.

Quest: SQ-0348"
```

---

### Task 7: Sorting

**Files:**
- Modify: `crates/app/src/picker.rs`

**Interfaces:**
- Consumes: `StoryMeta.author`/`year` (Task 5).
- Produces: `SortKey`, `Sort`, `pub fn sort_stories(&mut [StoryEntry], Sort)`. Tasks 8 and 9 consume.

- [ ] **Step 1: Write the failing tests**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey { Title, Author, Year }
pub struct Sort { pub key: SortKey, pub desc: bool }
```

- each key ascending and descending;
- **blanks sort last in BOTH directions** — a story with no author must not outrank one with an author just because the empty string sorts first. This is the test that will fail with a naive `sort_by_key`;
- filename tie-break (as today, `picker.rs:550`);
- title sort is case-insensitive (preserve today's behaviour, `picker.rs:551`);
- `Year` sorts numerically, not lexically — `"1980"` before `"1998"`, and a non-numeric year sorts as blank.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p app sort_stories`

- [ ] **Step 3: Implement** — key extractor returning `(is_blank: bool, value)` so blanks always land last, then reverse only the `value` comparison when `desc`, never the blank flag.

- [ ] **Step 4: Run to verify they pass** — `cargo test -p app sort_stories`

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/picker.rs
git commit -m "feat(app): sortable story list by title, author, or year

Blanks sort last in both directions: an unfetched story must not outrank a
fetched one just because the empty string compares first.

Quest: SQ-0348"
```

---

### Task 8: List redesign — columns, header, theming

**Files:**
- Modify: `crates/app/src/picker_ui.rs` (`draw_picker` rows ~`408-461`, footer ~`469`)
- Modify: `crates/app/src/style.rs` (new `ColorScheme` fields + selectors + `DEFAULT_STYLE_TOML`)

**Interfaces:**
- Consumes: `SortKey`/`Sort` (Task 7), `StoryMeta` fields (Task 5).
- Produces: `draw_picker` additionally returns `header_rects: Vec<(SortKey, Rect)>`. Task 9 hit-tests them.

Target (badges unchanged — same glyphs, same right-aligned fixed columns, same reverse-on-selection at `picker_ui.rs:435-459`):

```
  TITLE ▲                 AUTHOR                  YEAR
 ────────────────────────────────────────────────────────────────────
 ▸ Anchorhead              Michael S. Gentry       1998    ZB H
   Curses                  Graham Nelson           1993    Z B
   Zork I                  Marc Blank and Dave…    1980    ZBSH
   zork2-r63-s860811.z5    (no metadata yet)               Z S
```

- [ ] **Step 1: Add themeable styles first**

New `ColorScheme` fields: `story_header`, `story_header_active`, `story_author`, `story_year`, `story_no_metadata`. Each needs a `style.rs` selector and a `DEFAULT_STYLE_TOML` entry. **No hard-coded `Style::new().fg(...)` in the render** — project standing rule. Follow `story_badge` (`picker_ui.rs:448`) as the precedent.

- [ ] **Step 2: Write the failing tests**

The file already has render tests (`picker_ui.rs:784`, `821`, `1108`) — follow their harness (render into a `Buffer`, assert on cell contents).

- header row renders the three column names; the active one carries its direction arrow;
- a story with metadata renders author and year in their columns, aligned across rows;
- **a story with no metadata renders `(no metadata yet)`** in the author column — a fresh bare-z library is the *common* case, not an edge case, and must read as "nothing fetched yet" rather than as a rendering fault;
- **column drop**: at narrowing widths, year drops, then author; assert each step leaves no gap and the badge cluster stays right-aligned. The info panel takes ~half the width when open (`split_picker_area`, `picker_ui.rs:33`), so dropped states are normal operation;
- long author truncates with `…` and never overruns its column;
- `header_rects` line up with the header text actually drawn (this is what Task 9's clicks depend on).

- [ ] **Step 3: Run to verify they fail** — `cargo test -p app picker_ui`

- [ ] **Step 4: Implement.** Reuse `draw_str_clipped`. Compute column widths once per draw from `row_w`; use `unicode-width` (already a dep) for truncation, not `chars().count()`, or CJK titles will misalign.

- [ ] **Step 5: Footer.** Four new bindings (`f`, `r`, `s`, `d`) do not fit at 80 columns. Drop hints right-to-left as width shrinks, keeping the least guessable longest — `f`/`r` outrank `PgUp/PgDn`, which nobody needs told. `draw_str_clipped` truncates silently today; the drop order replaces that.

- [ ] **Step 6: Run to verify they pass** — `cargo test -p app`

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/picker_ui.rs crates/app/src/style.rs
git commit -m "feat(app): story list as sortable columns with a header

Quest: SQ-0348"
```

---

### Task 9: Picker wiring — keys, clicks, selection preservation

**Files:**
- Modify: `crates/app/src/picker_ui.rs` (`run_story_picker` loop ~`154-360`)

**Interfaces:**
- Consumes: everything above.
- Produces: the finished feature.

| Key | Scope | Cache |
|-----|-------|-------|
| `f` | selected story | forced — always refetches |
| `r` | whole library | skips stories at current `FETCH_VERSION` |
| `s` | — | cycle sort column |
| `d` | — | toggle sort direction |
| `Esc` | — | cancel a running sweep (**before** its existing quit meaning) |

All unshifted: the `slide.open && shift` branch (`picker_ui.rs:289`) swallows unmatched keys, so a Shift-bound key would silently do nothing exactly when the panel is open.

- [ ] **Step 1: Write the failing test — selection survives a reorder**

**This is the highest-value test in the plan.** Selection is an *index* (`list.selected`, `picker_ui.rs:412`), and three separate things reorder the list: changing the sort key, toggling direction, and **an `r` sweep landing new titles** — that last one reorders under a cursor the user is not touching. The user watches a sweep finish, presses Enter, and launches a different game.

Extract the reorder as a pure helper and test it directly:

```rust
/// Reorder, keeping the SELECTION on the same story — by path, never by index.
pub fn resort_preserving_selection(stories: &mut Vec<StoryEntry>, selected: usize, sort: Sort) -> usize {
    let keep = stories.get(selected).map(|e| e.path.clone());
    sort_stories(stories, sort);
    keep.and_then(|p| stories.iter().position(|e| e.path == p)).unwrap_or(0)
}
```

Test: key change, direction toggle, and a simulated sweep that rewrites titles — the selected `PathBuf` is unchanged in every case.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p app resort_preserving_selection`

- [ ] **Step 3: Implement the reorder helper** and route every reorder through it.

- [ ] **Step 4: Wire the keys.** `f` → order of length one, `forced: true`. `r` → all stories, `forced: false`. `s`/`d` → resort via the helper. `Esc` → if a sweep is in flight, cancel it and consume the key; otherwise quit as today.

- [ ] **Step 5: Wire header clicks.** Follow the existing `row_rects` hit-test (`picker_ui.rs:327`). Click a header → sort by it; click the active header → reverse. Route through the same helper.

- [ ] **Step 6: Drain progress and re-resolve.** Each iteration, drain `FetchProgress` (as covers are drained at `picker_ui.rs:221-226`), re-resolve that story's `StoryEntry` in place from the block just written, re-sort via the helper, redraw, and keep the 16 ms busy tick (`picker_ui.rs:260-268`) while `fetcher.busy()`.

- [ ] **Step 7: Progress line** — themeable, from `FetchProgress`:
  - `r`: `Fetching 7/23 — Zork I`, then `Fetched 19, skipped 3, not found 1`
  - `f`: `Fetching Zork I…`, then `Fetched Zork I` / `No IFDB record for Zork I` / `Fetch failed: timed out`

- [ ] **Step 8: Run the full suite** — `cargo test --workspace` (0 failures) and `cargo clippy --workspace` (≤25 warnings).

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/picker_ui.rs
git commit -m "feat(app): fetch keys, click-to-sort headers, live sweep progress

Every reorder preserves the selection by path: an r sweep rewrites titles
and re-sorts under a cursor the user is not touching.

Quest: SQ-0348"
```

---

### Task 10: Info panel — author, year, genre, blurb, fetched cover

**Files:**
- Modify: `crates/app/src/picker_ui.rs` (`draw_info_panel` ~`481-681`)
- Modify: `crates/app/src/cover.rs` (fetched-cover fallback)
- Modify: `crates/app/src/style.rs` (blurb style)

- [ ] **Step 1: Write the failing tests** — the author/year/genre line renders; the blurb wraps to panel width and participates in the existing `panel_scroll`/`panel_max` overflow (do not add a second scroll mechanism); a story with no metadata renders the panel exactly as today (no empty labels, no stray separators).

- [ ] **Step 2: Run to verify they fail** — `cargo test -p app info_panel`

- [ ] **Step 3: Implement.** Insert below the `IFID` line, above `Features:`. Cover fallback: a story's own `Fspc` always wins; `cover.png` is used **only** when there is no `Fspc` (spec "Precedence"). Extend `load_cover` (`cover.rs:26-35`) to fall back to the sidecar's cover — it already runs on the decoder thread, so no new threading.

- [ ] **Step 4: Run to verify they pass** — `cargo test --workspace`

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/picker_ui.rs crates/app/src/cover.rs crates/app/src/style.rs
git commit -m "feat(app): show author, year, genre, blurb, and fetched cover

Quest: SQ-0348"
```

---

## Final verification

- [ ] `cargo test --workspace` — 0 failures
- [ ] `cargo clippy --workspace --all-targets` — ≤ 25 warnings
- [ ] `grep -A3 '^\[dependencies\]' crates/blorb/Cargo.toml` — still empty
- [ ] `cargo tree -p app -i native-tls` — not found (rustls only)
- [ ] `grep -rn "Style::new().fg\|Color::" crates/app/src/picker_ui.rs` — no new hard-coded styles
- [ ] Update `README.md` — this is a major feature (metadata fetch + redesigned browser). Per project rule, README covers major features only.
- [ ] Update `docs/` style docs with the new `style.toml` selectors.

## Smoke tests for the user (cannot be verified headless → `confirm`)

1. `f` on a story → real title/author/blurb appear; the list re-sorts and the selection stays put.
2. `r` on the library → progress counts up, titles fill in live, `Esc` cancels cleanly.
3. `r` again → **zero** network requests (everything at current `FETCH_VERSION`).
4. Click each header → sorts; click again → reverses. `s`/`d` do the same.
5. A story IFDB doesn't know → `not_found`, and `r` doesn't retry it.
6. A fetched cover renders; a story with its own `Fspc` still shows its own.
7. Column layout and header at real terminal sizes, and with the info panel open.
