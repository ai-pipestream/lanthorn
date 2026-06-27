# Blorb Container Parser — Design (Glulx support, sub-project 1 of 4)

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**New crate:** `crates/blorb` (zero-dependency)

## Context

This is the foundation of Glulx support (decomposition: **1. Blorb parser** →
2. Glulx VM core → 3. Glk I/O → 4. automapping integration). It is independently
useful *now*: `.zblorb`/`.blorb` are listed as picker extensions but
`load_story_bytes` only handles raw + ZIP, so Blorb-wrapped story files do not
actually load. A Blorb parser makes `.zblorb` Z-machine games playable today and
unblocks the deferred **sound** work (Blorb `Snd ` resources) and later Glulx
graphics (`Pict`).

Blorb is the IFF "Interactive Fiction Resource" container (the Blorb spec). It is
plain big-endian IFF — hand-parseable with no dependencies (unlike the ZIP path,
which uses the `zip` crate). It can wrap a Z-code (`ZCOD`) or Glulx (`GLUL`)
executable plus sound/image/metadata resources.

## Goal

A small, zero-dep `blorb` crate that parses a Blorb file, exposes the embedded
**executable** (kind + bytes) and a generic **resource accessor** over the
resource index, and is wired into the app + zvm-cli load paths so `.zblorb`
Z-games load. Glulx executables are detected and rejected with a clear "not yet
supported" message until sub-project 2.

## Blorb format (what we parse)

- File = an IFF FORM: `b"FORM"` + `u32` BE length + form type `b"IFRS"`, then
  chunks.
- Chunk = 4-byte type id + `u32` BE length + `length` bytes of data + a single
  pad byte when `length` is odd (chunks are 2-byte aligned).
- The first chunk is `RIdx` (Resource Index): `u32` BE count, then `count`
  entries of 12 bytes — `usage` (4 bytes: `b"Exec"`, `b"Pict"`, `b"Snd "`,
  `b"Data"`), `number` (`u32` BE), `start` (`u32` BE = the byte offset, from the
  start of the file, of the resource's chunk header).
- The executable is the `Exec`/`0` entry; the chunk it points at has type `ZCOD`
  (Z-code) or `GLUL` (Glulx).
- Other chunks (`Snd `, `Pict`, `IFmd` metadata, `Fspc`, `IFID`/`UUID `, …) are
  reachable through the index but not decoded here.

## Design

### Crate `blorb` (`crates/blorb`)

Zero-dep. Owns the file bytes; parses the index eagerly; returns borrowed slices.

```rust
#[derive(Debug, PartialEq)]
pub enum BlorbError { NotBlorb, Truncated, NoResourceIndex, BadOffset, NoExecutable }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind { ZCode, Glulx } // from the Exec chunk's type (ZCOD / GLUL)

/// One parsed resource-index entry.
#[derive(Debug, Clone, Copy)]
pub struct ResourceEntry {
    pub usage: [u8; 4],
    pub number: u32,
    pub start: usize,         // file offset of the chunk header
    pub chunk_type: [u8; 4],  // the 4-byte type at `start`
    pub len: usize,           // chunk data length
}

pub struct Blorb {
    bytes: Vec<u8>,
    index: Vec<ResourceEntry>,
}

impl Blorb {
    /// Returns NotBlorb when the data isn't `FORM…IFRS` (so callers can fall
    /// back to treating the bytes as a raw story file).
    pub fn parse(bytes: Vec<u8>) -> Result<Blorb, BlorbError>;

    /// Cheap magic check without allocating/parsing.
    pub fn is_blorb(bytes: &[u8]) -> bool; // bytes[0..4]==FORM && bytes[8..12]==IFRS

    /// The embedded executable: its kind plus a slice of its chunk data.
    pub fn executable(&self) -> Result<(ExecKind, &[u8]), BlorbError>;

    /// A resource by usage+number (e.g. (b"Snd ", 3)); chunk type + data slice.
    pub fn resource(&self, usage: &[u8; 4], number: u32) -> Option<(&[u8; 4], &[u8])>;

    /// The parsed index (for enumeration; used by the future sound/pict work).
    pub fn resources(&self) -> &[ResourceEntry];
}
```

Parsing:
1. Validate `FORM`/`IFRS` (else `NotBlorb`). Bounds-check the FORM length.
2. Walk chunks to find `RIdx` (else `NoResourceIndex`); parse its entries. For
   each entry, read the 4-byte chunk type + length at `start` (bounds-checked →
   `BadOffset`/`Truncated`), filling `chunk_type`/`len`.
3. `executable()` finds `usage==b"Exec"` (number 0), maps `chunk_type` `ZCOD`→
   `ZCode`, `GLUL`→`Glulx`, else `NoExecutable`, and returns the data slice.

### Load-path integration (app + zvm-cli)

A shared extraction step used after raw/zip read: given the bytes, if
`Blorb::is_blorb`, parse and take `executable()`:
- `ZCode` → return the Z-code bytes (the existing `Memory::new` path).
- `Glulx` → a clear error: "Glulx story files are not yet supported."
- non-Blorb → return the bytes unchanged (today's behavior).

In the app, `hints::load_story_bytes` gains this step after the existing
raw/ZIP handling (a `.zblorb` is a raw Blorb, not a ZIP, so it flows through the
new branch). `zvm-cli`'s `fs::read` load gains the same extraction (so the CLI
also plays `.zblorb`). The app's `picker`/`build_machine` and `compute_ifid`
operate on the extracted Z-code bytes, so IFID/known-title/everything downstream
is unchanged.

The app depends on the new `blorb` crate (add to `crates/app/Cargo.toml` and
`crates/zvm-cli/Cargo.toml`).

## Testing

In `crates/blorb` (unit, hand-built byte buffers):
- `is_blorb` true for `FORM…IFRS`, false otherwise.
- `parse` a minimal Blorb (RIdx with one `Exec`/0 → a tiny `ZCOD` chunk) →
  `executable()` returns `(ZCode, <data>)`.
- A `GLUL` exec → `executable()` returns `(Glulx, …)`.
- `resource(b"Snd ", 1)` returns the right chunk type + data; absent → `None`;
  odd-length chunk's pad byte handled (next chunk still found).
- Malformed: not-Blorb → `NotBlorb`; truncated FORM/chunk → `Truncated`; index
  offset past EOF → `BadOffset`; missing RIdx → `NoResourceIndex`; no Exec entry
  → `NoExecutable`.

In `crates/app` (integration):
- `load_story_bytes` on a `.zblorb`-style Blorb wrapping a Z-code stub returns
  the inner Z-code bytes; on a Glulx Blorb returns a clear error; a raw `.z5`
  and a ZIP still load as before (no regression).

## Out of scope (this sub-project)

- Decoding `Snd `/`Pict` payloads (the sound and Glulx-graphics sub-projects
  consume the resource accessor this provides).
- The Glulx executable itself — detected and rejected here; run by sub-project 2.
- Blorb metadata (`IFmd` IFID/title) — IFID stays computed from the extracted
  Z-code header for now.
- Sibling/standalone resource Blorbs (a `.blb` next to a bare story) — the sound
  sub-project's concern; this crate's accessor is reused there.

## Global constraints

- New `blorb` crate is **zero-dependency** (std only); IFF is parsed by hand.
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace`
  green per task.
- No regression to existing raw/ZIP/`.z5` loading; non-Blorb bytes pass through
  unchanged.
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
