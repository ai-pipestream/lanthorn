# Blorb Container Parser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A zero-dep `blorb` crate that parses the IFF/Blorb container, exposes the embedded executable (`ZCode`/`Glulx` + bytes) and a generic resource accessor, and is wired into the app + zvm-cli load paths so `.zblorb` Z-machine games load (Glulx detected and rejected for now).

**Architecture:** New `crates/blorb` (std only). The app and zvm-cli depend on it and run an extraction step after their existing raw/ZIP read.

**Tech Stack:** Rust, std only. Big-endian IFF parsed by hand.

**Spec:** `docs/superpowers/specs/2026-06-27-blorb-parser-design.md`

## Global Constraints

- New `blorb` crate is zero-dependency (std only).
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green after every task.
- No regression to existing raw / ZIP / `.z5` loading; non-Blorb bytes pass through unchanged.
- Commit-only on local `main` (this branch); one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do not edit `TODO.md`.

## Blorb format reference

- File = IFF FORM: `b"FORM"` + `u32` BE length + `b"IFRS"` + chunks.
- Chunk = 4-byte type + `u32` BE length + data + 1 pad byte iff length is odd.
- First chunk `RIdx`: `u32` BE count, then `count` × { usage `[u8;4]`, number `u32` BE, start `u32` BE (file offset of the resource's chunk header) }.
- `Exec`/`0` → a chunk of type `ZCOD` (Z-code) or `GLUL` (Glulx).

---

## Task 1: `blorb` crate — `is_blorb`, `parse`, resource index

**Files:**
- Create: `crates/blorb/Cargo.toml`, `crates/blorb/src/lib.rs`
- Modify: root `Cargo.toml` (add `crates/blorb` to `[workspace] members`)

**Interfaces:**
- Produces: `BlorbError`, `ResourceEntry`, `Blorb::is_blorb(&[u8]) -> bool`, `Blorb::parse(Vec<u8>) -> Result<Blorb, BlorbError>`, `Blorb::resources(&self) -> &[ResourceEntry]`.

- [ ] **Step 1: Crate scaffold**

`crates/blorb/Cargo.toml`:
```toml
[package]
name = "blorb"
version = "0.1.0"
edition = "2021"

[dependencies]
```
Add `"crates/blorb"` to the root `Cargo.toml` workspace `members` list.

- [ ] **Step 2: Write failing tests** in `crates/blorb/src/lib.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Build an IFF chunk: type + BE len + data + pad-to-even.
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 { v.push(0); }
        v
    }

    /// Build a Blorb with the given resources. Each resource is
    /// (usage, number, chunk_type, data). Returns the file bytes.
    fn build_blorb(res: &[(&[u8; 4], u32, &[u8; 4], &[u8])]) -> Vec<u8> {
        // First lay out the resource chunks after the RIdx chunk to compute offsets.
        let count = res.len() as u32;
        let ridx_data_len = 4 + 12 * res.len();
        // RIdx chunk header (8) sits at file offset 12 (after FORM+len+IFRS).
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let mut offsets = Vec::new();
        let mut cursor = first_res_off;
        let mut body = Vec::new();
        for (_u, _n, ty, data) in res {
            offsets.push(cursor as u32);
            let c = chunk(ty, data);
            cursor += c.len();
            body.extend_from_slice(&c);
        }
        // RIdx data
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&count.to_be_bytes());
        for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
            ridx.extend_from_slice(*usage);
            ridx.extend_from_slice(&number.to_be_bytes());
            ridx.extend_from_slice(&offsets[i].to_be_bytes());
        }
        let ridx_chunk = chunk(b"RIdx", &ridx);
        // Assemble FORM
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&body);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn is_blorb_detects_magic() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        assert!(Blorb::is_blorb(&b));
        assert!(!Blorb::is_blorb(b"not a blorb at all"));
        assert!(!Blorb::is_blorb(&[]));
    }

    #[test]
    fn parse_indexes_resources() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 1, b"FORM", b"xyz")]);
        let blorb = Blorb::parse(b).unwrap();
        assert_eq!(blorb.resources().len(), 2);
        let exec = blorb.resources().iter().find(|r| &r.usage == b"Exec").unwrap();
        assert_eq!(&exec.chunk_type, b"ZCOD");
        assert_eq!(exec.len, 4);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert_eq!(Blorb::parse(b"junk".to_vec()), Err(BlorbError::NotBlorb));
        // FORM/IFRS but truncated before any chunk
        let mut t = b"FORM".to_vec();
        t.extend_from_slice(&4u32.to_be_bytes());
        t.extend_from_slice(b"IFRS");
        assert_eq!(Blorb::parse(t), Err(BlorbError::NoResourceIndex));
    }
}
```

- [ ] **Step 3: Run → fail.**

- [ ] **Step 4: Implement** in `crates/blorb/src/lib.rs`

```rust
//! Zero-dependency parser for the IFF "Blorb" interactive-fiction resource
//! container. Exposes the embedded executable and a resource accessor.

#[derive(Debug, PartialEq, Eq)]
pub enum BlorbError { NotBlorb, Truncated, NoResourceIndex, BadOffset, NoExecutable }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEntry {
    pub usage: [u8; 4],
    pub number: u32,
    pub start: usize,        // file offset of the chunk header
    pub chunk_type: [u8; 4],
    pub len: usize,          // chunk data length
}

pub struct Blorb {
    bytes: Vec<u8>,
    index: Vec<ResourceEntry>,
}

fn be_u32(b: &[u8], off: usize) -> Result<u32, BlorbError> {
    let s = b.get(off..off + 4).ok_or(BlorbError::Truncated)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

impl Blorb {
    pub fn is_blorb(b: &[u8]) -> bool {
        b.len() >= 12 && &b[0..4] == b"FORM" && &b[8..12] == b"IFRS"
    }

    pub fn parse(bytes: Vec<u8>) -> Result<Blorb, BlorbError> {
        if !Self::is_blorb(&bytes) {
            return Err(BlorbError::NotBlorb);
        }
        let end = bytes.len();
        // Walk top-level chunks (start at 12, after FORM+len+IFRS) to find RIdx.
        let mut ridx: Option<(usize, usize)> = None; // (data_start, count)
        let mut pos = 12;
        while pos + 8 <= end {
            let ctype = [bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]];
            let clen = be_u32(&bytes, pos + 4)? as usize;
            let data_start = pos + 8;
            if data_start + clen > end {
                return Err(BlorbError::Truncated);
            }
            if &ctype == b"RIdx" {
                let count = be_u32(&bytes, data_start)? as usize;
                ridx = Some((data_start + 4, count));
                break;
            }
            pos = data_start + clen + (clen & 1);
        }
        let (mut p, count) = ridx.ok_or(BlorbError::NoResourceIndex)?;
        let mut index = Vec::with_capacity(count);
        for _ in 0..count {
            let usage = *bytes.get(p..p + 4).ok_or(BlorbError::Truncated)?;
            let usage = [usage[0], usage[1], usage[2], usage[3]];
            let number = be_u32(&bytes, p + 4)?;
            let start = be_u32(&bytes, p + 8)? as usize;
            // Read the pointed-at chunk header.
            let chunk_type = {
                let s = bytes.get(start..start + 4).ok_or(BlorbError::BadOffset)?;
                [s[0], s[1], s[2], s[3]]
            };
            let len = be_u32(&bytes, start + 4)? as usize;
            if start + 8 + len > end {
                return Err(BlorbError::BadOffset);
            }
            index.push(ResourceEntry { usage, number, start, chunk_type, len });
            p += 12;
        }
        Ok(Blorb { bytes, index })
    }

    pub fn resources(&self) -> &[ResourceEntry] {
        &self.index
    }
}
```

- [ ] **Step 5: Run + commit** — `cargo test -p blorb` green; `cargo build` 0 warnings.

```bash
git add Cargo.toml crates/blorb
git commit  # feat(blorb): zero-dep IFF/Blorb container parser (FORM/IFRS + RIdx index)
```

---

## Task 2: `executable()` + `resource()` accessors

**Files:** Modify `crates/blorb/src/lib.rs`.

**Interfaces:**
- Produces: `ExecKind { ZCode, Glulx }`, `Blorb::executable(&self) -> Result<(ExecKind, &[u8]), BlorbError>`, `Blorb::resource(&self, usage: &[u8;4], number: u32) -> Option<(&[u8;4], &[u8])>`.

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn executable_returns_zcode_data() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd")]);
        let blorb = Blorb::parse(b).unwrap();
        let (kind, data) = blorb.executable().unwrap();
        assert_eq!(kind, ExecKind::ZCode);
        assert_eq!(data, b"abcd");
    }

    #[test]
    fn executable_detects_glulx() {
        let b = build_blorb(&[(b"Exec", 0, b"GLUL", b"glul")]);
        assert_eq!(Blorb::parse(b).unwrap().executable().unwrap().0, ExecKind::Glulx);
    }

    #[test]
    fn executable_missing_is_error() {
        let b = build_blorb(&[(b"Snd ", 1, b"FORM", b"x")]);
        assert_eq!(Blorb::parse(b).unwrap().executable(), Err(BlorbError::NoExecutable));
    }

    #[test]
    fn resource_fetches_by_usage_number() {
        let b = build_blorb(&[(b"Exec", 0, b"ZCOD", b"abcd"), (b"Snd ", 3, b"OGGV", b"oggdata")]);
        let blorb = Blorb::parse(b).unwrap();
        let (ty, data) = blorb.resource(b"Snd ", 3).unwrap();
        assert_eq!(ty, b"OGGV");
        assert_eq!(data, b"oggdata");
        assert!(blorb.resource(b"Snd ", 99).is_none());
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement** (append to `impl Blorb`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind { ZCode, Glulx }

impl Blorb {
    fn chunk_data(&self, e: &ResourceEntry) -> &[u8] {
        &self.bytes[e.start + 8..e.start + 8 + e.len]
    }

    pub fn executable(&self) -> Result<(ExecKind, &[u8]), BlorbError> {
        let e = self
            .index
            .iter()
            .find(|r| &r.usage == b"Exec")
            .ok_or(BlorbError::NoExecutable)?;
        let kind = match &e.chunk_type {
            b"ZCOD" => ExecKind::ZCode,
            b"GLUL" => ExecKind::Glulx,
            _ => return Err(BlorbError::NoExecutable),
        };
        Ok((kind, self.chunk_data(e)))
    }

    pub fn resource(&self, usage: &[u8; 4], number: u32) -> Option<(&[u8; 4], &[u8])> {
        let e = self.index.iter().find(|r| &r.usage == usage && r.number == number)?;
        // SAFETY of indexing: bounds were validated in parse().
        let entry = self.index.iter().find(|r| std::ptr::eq(*r, e)).unwrap_or(e);
        Some((&entry.chunk_type, self.chunk_data(e)))
    }
}
```

(Implementer: simplify `resource` — return `(&e.chunk_type, self.chunk_data(e))` directly; the spec only needs the type + data. Keep borrow-checker happy without the `ptr::eq` dance.)

- [ ] **Step 4: Run + commit** — `cargo test -p blorb` green, 0 warnings.

```bash
git add crates/blorb/src/lib.rs
git commit  # feat(blorb): executable() + resource() accessors over the index
```

---

## Task 3: Wire Blorb into the app + zvm-cli load paths

**Files:**
- Modify: `crates/app/Cargo.toml`, `crates/app/src/hints.rs`
- Modify: `crates/zvm-cli/Cargo.toml`, `crates/zvm-cli/src/main.rs`

**Interfaces:**
- Consumes: `blorb::{Blorb, ExecKind}`.

- [ ] **Step 1: Add the dependency** — `blorb = { path = "../blorb" }` to both `crates/app/Cargo.toml` and `crates/zvm-cli/Cargo.toml`.

- [ ] **Step 2: Failing test** in `crates/app/src/hints.rs` tests

```rust
    #[test]
    fn load_story_bytes_extracts_zblorb_executable() {
        use std::io::Write;
        // Minimal Blorb wrapping a tiny ZCOD payload (see blorb crate's builder
        // shape: FORM/IFRS + RIdx + Exec/0 ZCOD chunk).
        let zcode = b"ZCODE-PAYLOAD";
        let file = make_zblorb(zcode); // helper building the bytes (mirror blorb tests)
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.zblorb");
        std::fs::File::create(&path).unwrap().write_all(&file).unwrap();
        let out = load_story_bytes(&path).unwrap();
        assert_eq!(out, zcode);
    }

    #[test]
    fn load_story_bytes_rejects_glulx_blorb() {
        // A Blorb whose Exec chunk is GLUL → a clear error, not raw bytes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.gblorb");
        std::fs::write(&path, make_gblorb(b"GLULPAYLOAD")).unwrap();
        let err = load_story_bytes(&path).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("glulx"));
    }
```

(`make_zblorb`/`make_gblorb` are small test helpers mirroring the blorb crate's
`build_blorb` shape; or expose a `#[cfg(test)]`/`pub` builder from the blorb
crate and reuse it. The raw `.z5` and ZIP tests already exist and must still pass.)

- [ ] **Step 2b: Run → fail.**

- [ ] **Step 3: Add Blorb extraction to `load_story_bytes`** (`crates/app/src/hints.rs`)

After computing `bytes` from the raw/ZIP branches, route through Blorb:

```rust
pub fn load_story_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    let bytes = if raw.starts_with(ZIP_MAGIC) {
        // existing ZIP handling → the entry bytes
        match read_zip_entry(path, |name| {
            let l = name.to_ascii_lowercase();
            l.ends_with(".z3") || l.ends_with(".z5") || l.ends_with(".z8")
        })? {
            Some(b) => b,
            None => return Err(io::Error::new(io::ErrorKind::NotFound,
                format!("no .z3/.z5/.z8 entry found in zip: {}", path.display()))),
        }
    } else {
        raw
    };
    extract_story(bytes)
}

/// If `bytes` is a Blorb, return its Z-code executable; reject Glulx; otherwise
/// pass the bytes through unchanged (a raw story file).
fn extract_story(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid Blorb: {e:?}")))?;
    match b.executable() {
        Ok((blorb::ExecKind::ZCode, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::Glulx, _)) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Glulx story files are not yet supported".to_string(),
        )),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, format!("Blorb has no executable: {e:?}"))),
    }
}
```

- [ ] **Step 4: zvm-cli load path** — in `crates/zvm-cli/src/main.rs`, after `fs::read(&story_path)`, pass the bytes through the same extraction (a small local copy of `extract_story`, or call a shared helper). On the Glulx/`InvalidData` error, print the message and `process::exit(1)` (matching the existing read-error style). Raw `.z5` is unaffected.

- [ ] **Step 5: Run + commit** — `cargo test --workspace` green, 0 warnings. Confirm a raw `.z5` and a ZIP still load (existing tests).

```bash
git add crates/app/Cargo.toml crates/app/src/hints.rs crates/zvm-cli/Cargo.toml crates/zvm-cli/src/main.rs
git commit  # feat(app,zvm-cli): load .zblorb via the blorb crate (Glulx rejected cleanly)
```

---

## Self-review checklist (run before final review)

- `blorb` crate has no dependencies; `cargo tree -p blorb` shows only std.
- A raw `.z5`, a ZIP story, and a `.zblorb` all load; a Glulx Blorb errors clearly; non-Blorb bytes pass through unchanged.
- Bounds are checked everywhere (no panics on truncated/odd/garbage input — the malformed tests cover NotBlorb/Truncated/NoResourceIndex/BadOffset/NoExecutable).
- 0 warnings; `cargo test --workspace` green.
