# Auxiliary Save Data (v5 `save/restore table`) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the v5 auxiliary `save table bytes name` / `restore table bytes name` opcodes (EXT:0x00 / EXT:0x01, store form, ≥3 operands) so games can persist a named region of memory. The engine owns an in-memory aux table (no filesystem, no suspend); the app persists it either inside the `.babelmap` archive or in a per-game global file, chosen once by the user via a `aux_storage` config (default "ask", with a first-use prompt). 0-operand `save`/`restore` keep the existing full game-state behavior.

**Architecture:** Six layers. (1) zvm: `Machine.aux_data` + `aux_dirty` + the table-form opcodes. (2) app `aux_store` module: the blob codec + the global-file backend. (3) app `archive.rs`: always embed/extract an `aux.dat` zip entry. (4) app `config.rs` + config screen: the `aux_storage` tri-state setting. (5) app run-loop wiring: startup load (global mode), post-turn persist, archive-load sites repopulate the table. (6) app first-use prompt dialog (resolves "ask").

**Tech Stack:** Rust (zvm + app crates), `zip`, `toml`/`toml_edit`, ratatui.

Design reference: `docs/superpowers/specs/2026-06-26-aux-save-data-design.md`.

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in commit bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Per task: `cargo test -p zvm` (Task 1) or `cargo test -p app` (Tasks 2–6) green, **0 warnings** (`cargo build` clean). The headless smoke test (`crates/app/tests/headless.rs`) must still pass.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Scope is **v5+** table form. The 0-operand `save`/`restore` paths are unchanged.
- The Z-machine performs **no filesystem I/O** and the aux opcodes **never suspend** (always `StepResult::Continue`). All persistence is app-side.
- All game-controlled addresses (`table`, `bytes`, `name`) must be **bounds-clamped** against `self.mem.len()` — never index past memory.
- New UI (the prompt dialog) must be themeable via existing style selectors; no hard-coded styles (mirror the reset dialog).
- The aux table type is `std::collections::BTreeMap<String, Vec<u8>>` everywhere (deterministic ordering → byte-stable archives).

---

### Task 1: zvm — `aux_data` table + `save/restore table` opcodes

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add two `Machine` fields + init; rewrite the `EXT:0x00`/`EXT:0x01` arms (~1063–1076) to branch on operand count; add a private `read_aux_name`; add tests in the `pub(crate) mod tests` block.

**Interfaces:**
- Consumes: `self.mem.read_byte(u32) -> u8`, `self.mem.write_byte(u32, u8)`, `self.mem.len() -> usize`, `self.do_store(Option<u8>, u16)`, `StepResult`.
- Produces: `pub aux_data: std::collections::BTreeMap<String, Vec<u8>>` and `pub aux_dirty: bool` on `Machine`; the table-form opcode behavior.

- [ ] **Step 1: Write the failing tests**

In `crates/zvm/src/cpu/exec.rs`, inside `pub(crate) mod tests`, add (calls the private `exec_ext` directly — the test module is in the same file, so private methods are reachable):

```rust
// ── v5 auxiliary save/restore table form (EXT:0x00 / EXT:0x01, ≥3 operands) ──
//
// Lays a name string at 0x300 ("AB", length-prefixed) and a 4-byte data region
// at 0x310, then drives exec_ext directly. The in-memory table round-trips and
// the game-visible store values follow the spec (save→1, restore→bytes-read).
fn aux_machine() -> Machine {
    let mem = Memory::new(sample_story(5)).unwrap();
    let mut m = Machine::new(mem);
    // name string "AB" at 0x300: [len=2]['A']['B']
    m.mem.write_byte(0x300, 2);
    m.mem.write_byte(0x301, b'A');
    m.mem.write_byte(0x302, b'B');
    // data region at 0x310: 0xDE 0xAD 0xBE 0xEF
    for (i, b) in [0xDE, 0xAD, 0xBE, 0xEF].into_iter().enumerate() {
        m.mem.write_byte(0x310 + i as u32, b);
    }
    m
}

#[test]
fn aux_save_table_stores_one_and_fills_table() {
    let mut m = aux_machine();
    // save table=0x310 bytes=4 name=0x300 -> store G0
    let r = m.exec_ext(0x00, &[0x310, 4, 0x300], Some(0x10));
    assert_eq!(r, StepResult::Continue, "aux save never suspends");
    assert_eq!(m.global(0), 1, "aux save stores 1 (success)");
    assert!(m.aux_dirty, "aux save marks the table dirty");
    assert_eq!(m.aux_data.get("AB").map(|v| v.as_slice()), Some(&[0xDE,0xAD,0xBE,0xEF][..]));
}

#[test]
fn aux_restore_table_round_trips_and_stores_count() {
    let mut m = aux_machine();
    m.exec_ext(0x00, &[0x310, 4, 0x300], Some(0x10)); // save first
    // clobber the region
    for i in 0..4 { m.mem.write_byte(0x310 + i, 0); }
    // restore table=0x310 bytes=4 name=0x300 -> store G0
    let r = m.exec_ext(0x01, &[0x310, 4, 0x300], Some(0x10));
    assert_eq!(r, StepResult::Continue);
    assert_eq!(m.global(0), 4, "restore stores the number of bytes read");
    assert_eq!(m.mem.read_byte(0x310), 0xDE);
    assert_eq!(m.mem.read_byte(0x313), 0xEF);
}

#[test]
fn aux_restore_missing_name_stores_zero() {
    let mut m = aux_machine();
    let r = m.exec_ext(0x01, &[0x310, 4, 0x300], Some(0x10));
    assert_eq!(r, StepResult::Continue);
    assert_eq!(m.global(0), 0, "restoring an unsaved name stores 0");
}

#[test]
fn aux_save_out_of_bounds_does_not_panic() {
    let mut m = aux_machine();
    let huge = (m.mem.len() as u16).wrapping_sub(2);
    // table near EOF, bytes huge, name near EOF — must clamp, not panic.
    let r = m.exec_ext(0x00, &[huge, 0xFFFF, huge], Some(0x10));
    assert_eq!(r, StepResult::Continue);
    assert_eq!(m.global(0), 1);
}

#[test]
fn ext_save_restore_zero_operands_still_suspend() {
    let mut m = aux_machine();
    assert_eq!(m.exec_ext(0x00, &[], Some(0x10)), StepResult::SaveRequest);
    assert_eq!(m.exec_ext(0x01, &[], Some(0x10)), StepResult::RestoreRequest);
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p zvm aux_`
Expected: compile errors (`aux_data`, `aux_dirty` missing).

- [ ] **Step 3: Add the `Machine` fields + init**

In `crates/zvm/src/cpu/exec.rs`, in `struct Machine` after `pub diagnostics: Vec<String>,` (~112):

```rust
    /// In-memory auxiliary save table for the v5 `save/restore table` opcodes,
    /// keyed by the game-supplied name string. The host persists/repopulates it
    /// (in the `.babelmap` archive or a per-game global file); the engine itself
    /// never touches the filesystem.
    pub aux_data: std::collections::BTreeMap<String, Vec<u8>>,
    /// Set true whenever an aux `save table` writes the table. The host clears it
    /// after persisting; correctness does not depend on the flag (every archive
    /// write embeds the latest table) — it is a "data changed" notification.
    pub aux_dirty: bool,
```

In `with_output` (~140), after `diagnostics: Vec::new(),`:

```rust
            aux_data: std::collections::BTreeMap::new(),
            aux_dirty: false,
```

- [ ] **Step 4: Rewrite the `EXT:0x00`/`EXT:0x01` arms + add `read_aux_name`**

Replace the existing `0x00` and `0x01` arms in `exec_ext` (~1063–1076) with:

```rust
            // EXT:0x00 save — 0 operands: full game-state save (suspend).
            // ≥3 operands: v5 auxiliary "save table bytes name [prompt]".
            0x00 => {
                if ops.len() >= 3 {
                    let table = ops[0] as u32;
                    let len = ops[1] as u32;
                    let name = self.read_aux_name(ops[2] as u32);
                    let mut data = Vec::with_capacity(len.min(self.mem.len() as u32) as usize);
                    for i in 0..len {
                        let a = table + i;
                        if a as usize >= self.mem.len() { break; }
                        data.push(self.mem.read_byte(a));
                    }
                    self.aux_data.insert(name, data);
                    self.aux_dirty = true;
                    self.do_store(store, 1);
                    StepResult::Continue
                } else {
                    let dest = match store {
                        Some(sv) => SaveDest::Store(sv),
                        None => SaveDest::Store(0),
                    };
                    self.pending_save = Some(PendingSave { result_dest: dest });
                    StepResult::SaveRequest
                }
            }
            // EXT:0x01 restore — 0 operands: full restore (suspend). ≥3 operands:
            // v5 auxiliary "restore table bytes name [prompt]" (stores bytes read).
            0x01 => {
                if ops.len() >= 3 {
                    let table = ops[0] as u32;
                    let len = ops[1] as u32;
                    let name = self.read_aux_name(ops[2] as u32);
                    let written = match self.aux_data.get(&name).cloned() {
                        Some(data) => {
                            let n = (data.len() as u32).min(len);
                            let mut w = 0u16;
                            for i in 0..n {
                                let a = table + i;
                                if a as usize >= self.mem.len() { break; }
                                self.mem.write_byte(a, data[i as usize]);
                                w += 1;
                            }
                            w
                        }
                        None => 0,
                    };
                    self.do_store(store, written);
                    StepResult::Continue
                } else {
                    self.pending_restore_store = store;
                    StepResult::RestoreRequest
                }
            }
```

Add a private helper in `impl Machine` (near `exec_ext`):

```rust
    /// Read the length-prefixed ASCII filename string for the v5 aux opcodes:
    /// byte 0 is the length, followed by that many ASCII bytes. Bounds-safe —
    /// returns an empty string (a valid table key) for a 0 / out-of-range addr.
    fn read_aux_name(&self, addr: u32) -> String {
        if addr == 0 || addr as usize >= self.mem.len() {
            return String::new();
        }
        let len = self.mem.read_byte(addr) as u32;
        let mut s = String::with_capacity(len as usize);
        for i in 0..len {
            let a = addr + 1 + i;
            if a as usize >= self.mem.len() { break; }
            s.push(self.mem.read_byte(a) as char);
        }
        s
    }
```

- [ ] **Step 5: Run the tests + full zvm suite**

Run: `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(zvm): v5 auxiliary save/restore table opcodes (in-memory table)

EXT:0x00/0x01 with >=3 operands now operate on an in-memory aux_data table
keyed by the game-supplied name: save copies bytes from the table and stores
1; restore copies them back and stores the byte count (0 if absent). Sets
aux_dirty for the host to persist. All addresses are bounds-clamped. The
0-operand forms keep the full game-state save/restore (suspend) behavior.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: app — `aux_store` module (blob codec + global-file backend)

**Files:**
- Create: `crates/app/src/aux_store.rs`.
- Modify: `crates/app/src/lib.rs` — add `pub mod aux_store;`.

**Interfaces:**
- Produces:
  - `pub fn encode_aux(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8>`
  - `pub fn decode_aux(bytes: &[u8]) -> BTreeMap<String, Vec<u8>>` (tolerant: malformed → empty)
  - `pub fn aux_path(save_dir: &Path, ifid: &str) -> PathBuf` (sanitized `<save_dir>/<ifid>.aux`)
  - `pub fn read_global_aux(save_dir: &Path, ifid: &str) -> BTreeMap<String, Vec<u8>>`
  - `pub fn write_global_aux(save_dir: &Path, ifid: &str, table: &BTreeMap<String, Vec<u8>>) -> std::io::Result<()>`

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/aux_store.rs` containing the tests (and `use` lines) first; the functions follow in Step 3.

```rust
//! Auxiliary save-data codec + global-file backend (v5 `save/restore table`).
//! See docs/superpowers/specs/2026-06-26-aux-save-data-design.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// (functions added in Step 3)

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        m.insert("AB".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        m.insert("".to_string(), vec![]); // empty key + empty value
        m
    }

    #[test]
    fn codec_round_trips() {
        let m = sample();
        assert_eq!(decode_aux(&encode_aux(&m)), m);
    }

    #[test]
    fn decode_tolerates_garbage() {
        assert!(decode_aux(b"\xff\xff\xffnonsense").is_empty());
        assert!(decode_aux(&[]).is_empty());
    }

    #[test]
    fn aux_path_sanitizes_and_stays_in_dir() {
        let dir = Path::new("/tmp/saves");
        let p = aux_path(dir, "../../etc/ZCODE-1-840726");
        assert_eq!(p.parent(), Some(dir), "no path escape");
        let fname = p.file_name().unwrap().to_string_lossy();
        assert!(fname.ends_with(".aux"));
        assert!(!fname.contains('/') && !fname.contains("..") && !fname.contains('\\'));
    }

    #[test]
    fn global_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("babelmap-aux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ifid = "ZCODE-1-840726-ABCD";
        assert!(read_global_aux(&dir, ifid).is_empty(), "absent file → empty");
        write_global_aux(&dir, ifid, &sample()).unwrap();
        assert_eq!(read_global_aux(&dir, ifid), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app aux_store` (after adding `pub mod aux_store;` to `lib.rs`).
Expected: compile errors (functions missing).

- [ ] **Step 3: Implement the codec + backend**

Add to `crates/app/src/aux_store.rs` (above the tests):

```rust
/// Encode the aux table as a compact length-prefixed binary blob:
/// `u32 count` then per entry `u16 name_len, name, u32 data_len, data`
/// (all big-endian). Deterministic because the input is a BTreeMap.
pub fn encode_aux(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(table.len() as u32).to_be_bytes());
    for (name, data) in table {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u16).to_be_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// Decode `encode_aux` output. Tolerant: any truncation/overflow yields whatever
/// was parsed so far (empty for non-aux bytes), never panics or errors.
pub fn decode_aux(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*p..*p + n)?;
        *p += n;
        Some(s)
    };
    let count = match take(&mut p, 4) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => return out,
    };
    for _ in 0..count {
        let nl = match take(&mut p, 2) { Some(b) => u16::from_be_bytes([b[0], b[1]]) as usize, None => break };
        let name = match take(&mut p, nl) { Some(b) => String::from_utf8_lossy(b).into_owned(), None => break };
        let dl = match take(&mut p, 4) { Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize, None => break };
        let data = match take(&mut p, dl) { Some(b) => b.to_vec(), None => break };
        out.insert(name, data);
    }
    out
}

/// `<save_dir>/<sanitized-ifid>.aux`. The IFID is interpreter-generated and
/// normally safe; sanitized defensively to `[A-Za-z0-9._-]` with no separators.
pub fn aux_path(save_dir: &Path, ifid: &str) -> PathBuf {
    let safe: String = ifid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let stem = if safe.is_empty() { "game".to_string() } else { safe };
    save_dir.join(format!("{stem}.aux"))
}

/// Read the per-game global aux file (empty map if absent or unreadable).
pub fn read_global_aux(save_dir: &Path, ifid: &str) -> BTreeMap<String, Vec<u8>> {
    match std::fs::read(aux_path(save_dir, ifid)) {
        Ok(bytes) => decode_aux(&bytes),
        Err(_) => BTreeMap::new(),
    }
}

/// Write the per-game global aux file (creating `save_dir` if needed).
pub fn write_global_aux(save_dir: &Path, ifid: &str, table: &BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
    std::fs::create_dir_all(save_dir)?;
    std::fs::write(aux_path(save_dir, ifid), encode_aux(table))
}
```

- [ ] **Step 4: Run the tests + full app suite**

Run: `cargo test -p app` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/aux_store.rs crates/app/src/lib.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): aux_store — aux-table blob codec + per-game global file

encode_aux/decode_aux serialize the BTreeMap<String,Vec<u8>> aux table as a
compact length-prefixed blob (tolerant decode). aux_path/read_global_aux/
write_global_aux back the global storage mode under a sanitized <ifid>.aux.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: app — embed/extract `aux.dat` in the `.babelmap` archive

**Files:**
- Modify: `crates/app/src/archive.rs` — add `ENTRY_AUX`; embed aux in `save_archive_meta`; extract in `load_archive`; add `aux` to `ArchiveContents`; add a round-trip test.

**Interfaces:**
- Consumes: `crate::aux_store::{encode_aux, decode_aux}` (Task 2), `machine.aux_data` (Task 1).
- Produces: `pub aux: std::collections::BTreeMap<String, Vec<u8>>` on `ArchiveContents`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/archive.rs` `mod tests`, add (uses `dummy_machine`/`small_mapper`/`temp_archive_path`):

```rust
    #[test]
    fn archive_round_trips_aux_data() {
        let mut machine = dummy_machine();
        machine.aux_data.insert("hints".to_string(), vec![1, 2, 3]);
        let path = temp_archive_path("aux");
        save_archive(&path, &small_mapper(), &machine, &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(ac.aux.get("hints").map(|v| v.as_slice()), Some(&[1, 2, 3][..]));
    }

    #[test]
    fn archive_without_aux_loads_empty_map() {
        let machine = dummy_machine(); // empty aux_data
        let path = temp_archive_path("noaux");
        save_archive(&path, &small_mapper(), &machine, &[], &[], &[]).expect("save");
        let ac = load_archive(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert!(ac.aux.is_empty());
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app archive_round_trips_aux_data archive_without_aux`
Expected: compile error (`ArchiveContents.aux` missing).

- [ ] **Step 3: Implement embed + extract**

In `crates/app/src/archive.rs`:

1. After the entry constants (~37), add: `const ENTRY_AUX: &str = "aux.dat";`
2. Add field to `ArchiveContents` (after `screen`, ~138): `pub aux: std::collections::BTreeMap<String, Vec<u8>>,`
3. In `save_archive_meta`, after the `screen.json` entry (~219) and before the history block, embed the table when non-empty:

```rust
        if !machine.aux_data.is_empty() {
            zip.start_file(ENTRY_AUX, options)?;
            zip.write_all(&crate::aux_store::encode_aux(&machine.aux_data))?;
        }
```

4. In `load_archive`, after the transcript/screen extraction and before building the return value, read the optional entry:

```rust
        let aux = match zip.by_name(ENTRY_AUX) {
            Ok(mut entry) => {
                let mut buf = Vec::new();
                let _ = entry.read_to_end(&mut buf);
                crate::aux_store::decode_aux(&buf)
            }
            Err(_) => std::collections::BTreeMap::new(),
        };
```

5. Add `aux,` to the `ArchiveContents { ... }` constructor in the `Ok(...)` return.

- [ ] **Step 4: Run the tests + full app suite**

Run: `cargo test -p app` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/archive.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): embed/extract aux.dat in the .babelmap archive

save_archive_meta embeds the machine's aux_data as an aux.dat zip entry when
non-empty; load_archive extracts it into ArchiveContents.aux (empty for older
archives without the entry). Archive storage mode reads this back; global mode
ignores it in favor of the per-game file.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: app — `aux_storage` config setting (+ config screen)

**Files:**
- Modify: `crates/app/src/config.rs` — `AuxStorage` enum; `Config.aux_storage` field; `Default`; `resolve` merge; `write_config`; tests.
- Modify: `crates/app/src/render/config_screen.rs` — add a row + value rendering.
- Modify: `crates/app/src/input.rs` — add the cycle handler + wire it into `config_toggle_or_edit`/`config_cycle`.

**Interfaces:**
- Produces: `pub enum AuxStorage { Ask, Archive, Global }` (`Default = Ask`), `pub aux_storage: AuxStorage` on `Config`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/config.rs` tests (mirror `background_tidy_parses_*`):

```rust
    #[test]
    fn aux_storage_defaults_to_ask() {
        assert_eq!(Config::default().aux_storage, AuxStorage::Ask);
    }

    #[test]
    fn aux_storage_parses_variants_from_toml() {
        let c: Config = toml::from_str("aux_storage = \"archive\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Archive);
        let c: Config = toml::from_str("aux_storage = \"global\"").unwrap();
        assert_eq!(c.aux_storage, AuxStorage::Global);
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app aux_storage` → compile error (`AuxStorage` missing).

- [ ] **Step 3: Implement the enum + field + plumbing**

In `crates/app/src/config.rs`:

1. Near `BackgroundTidy` (~186), add:

```rust
/// Where to persist v5 auxiliary save data (the `save/restore table` opcodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuxStorage {
    /// Ask the user on first use, then store the choice in config.
    #[default]
    Ask,
    /// Inside each `.babelmap` save archive.
    Archive,
    /// In one per-game file in the save directory (shared across playthroughs).
    Global,
}
```

2. In `Config` (~246) add: `#[serde(default)] pub aux_storage: AuxStorage,`
3. In `Default for Config` (~291): `aux_storage: AuxStorage::Ask,`
4. In `resolve` merge (~356–377): `cfg.aux_storage = from_file.aux_storage;`
5. In `write_config` (mirror `background_tidy`, ~413):

```rust
    let aux_str = match cfg.aux_storage {
        AuxStorage::Ask => "ask",
        AuxStorage::Archive => "archive",
        AuxStorage::Global => "global",
    };
    doc["aux_storage"] = toml_edit::value(aux_str);
```

- [ ] **Step 4: Add to the config screen**

In `crates/app/src/render/config_screen.rs`:
1. Append to `CONFIG_ROWS` (~12–22): `("aux_storage", ConfigRowKind::Enum),` (note the new row index `N` = current last index + 1).
2. In `config_row_value` (~126–144) add a match arm for index `N`:

```rust
        N => match cfg.aux_storage {
            crate::config::AuxStorage::Ask => "ask".to_string(),
            crate::config::AuxStorage::Archive => "archive".to_string(),
            crate::config::AuxStorage::Global => "global".to_string(),
        },
```

In `crates/app/src/input.rs`:
3. Add a cycle helper after `config_cycle_background_tidy` (~2429):

```rust
fn config_cycle_aux_storage(val: &mut crate::config::AuxStorage, delta: i32) {
    use crate::config::AuxStorage::*;
    let variants = [Ask, Archive, Global];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}
```

4. In `config_toggle_or_edit` (~2396) add the row-`N` arm: `N => { if let Some(cs) = &mut state.config_screen { config_cycle_aux_storage(&mut cs.working.aux_storage, 1); } }`
5. In `config_cycle` (~2433) add the row-`N` arm calling `config_cycle_aux_storage(&mut cs.working.aux_storage, delta)`.

(Confirm the exact new index `N` by counting `CONFIG_ROWS`; update both `config_row_value` and the two input arms consistently.)

- [ ] **Step 5: Run tests + full app suite**

Run: `cargo test -p app` → PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/config.rs crates/app/src/render/config_screen.rs crates/app/src/input.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): aux_storage config setting (ask/archive/global)

New tri-state config (default ask) controlling where v5 aux save data is
persisted, wired through Default/resolve/write_config and the in-app config
screen (selectable row + left/right cycle).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: app — persistence wiring (startup load, post-turn persist, load sites)

**Files:**
- Modify: `crates/app/src/main.rs` — startup global-mode load; a `persist_aux_after_turn` helper called after `post_turn_bookkeeping` at both turn sites; set `session.machine.aux_data` from the loaded archive at the archive-load sites.

**Interfaces:**
- Consumes: `session.machine.aux_data` / `aux_dirty` (Task 1), `aux_store::{read_global_aux, write_global_aux}` (Task 2), `ArchiveContents.aux` (Task 3), `state.config.aux_storage` (Task 4).

This task assumes `aux_storage` is `Archive` or `Global` (treat `Ask` as `Archive` here; Task 6 replaces that with the prompt). Archive-mode writing is automatic: `save_archive_meta` already embeds `aux_data` (Task 3), so the existing per-turn auto-save persists it. This task adds: (a) **global-mode** file writes, (b) **startup** load, (c) repopulating the table when an archive is loaded.

- [ ] **Step 1: Add `persist_aux_after_turn`**

Near `post_turn_bookkeeping` in `crates/app/src/main.rs`, add a helper that takes `&mut GameSession` (it must clear `aux_dirty`):

```rust
/// After a turn, persist the VM's aux table if it changed. Archive mode is
/// already covered by the per-turn auto-save (save_archive_meta embeds it);
/// global mode writes the per-game file here. `Ask` is treated as `Archive`
/// until the first-use prompt (Task 6) resolves it.
fn persist_aux_after_turn(
    session: &mut app::session::GameSession,
    cfg: &app::config::Config,
    save_dir: &std::path::Path,
    ifid: &str,
) {
    if !session.machine.aux_dirty {
        return;
    }
    if cfg.aux_storage == app::config::AuxStorage::Global {
        let _ = app::aux_store::write_global_aux(save_dir, ifid, &session.machine.aux_data);
    }
    session.machine.aux_dirty = false;
}
```

- [ ] **Step 2: Call it after both turn sites**

After each `post_turn_bookkeeping(...)` call (the submit path ~the auto-save site, and inside/after `finish_resumed_turn`), add:

```rust
                persist_aux_after_turn(&mut session, &state.config, &save_dir, &ifid);
```

(`post_turn_bookkeeping` takes `&GameSession` immutably, so this must be a separate call where `session` is `&mut` — i.e. at the run-loop call site, not inside `post_turn_bookkeeping`.)

- [ ] **Step 3: Startup load (global mode)**

After `save_dir` (~933) and after the session is created, before the run loop, seed the table in global mode:

```rust
    if cfg.aux_storage == app::config::AuxStorage::Global {
        session.machine.aux_data = app::aux_store::read_global_aux(&save_dir, &ifid);
    }
```

- [ ] **Step 4: Repopulate the table at archive-load sites (archive mode)**

At each site that rebuilds `mapper` from a loaded `ArchiveContents ac` — the startup load (~965), the in-game restore branch (~2322), and the non-in-game restore branch (~2352) — when **not** in global mode, also set the table:

```rust
                    if state.config.aux_storage != app::config::AuxStorage::Global {
                        session.machine.aux_data = ac.aux.clone();
                    }
```

(In global mode the startup-loaded global file is authoritative; do not overwrite it from the archive. `apply_launch_resume` needs no change — the mapper/aux were already loaded at startup.)

- [ ] **Step 5: Build, test, headless smoke**

Run: `cargo build -p app && cargo test -p app` → clean (0 warnings), suite + headless PASS.

Manual (not gating): with `aux_storage = global` in config, a v5 story that issues `save table` writes `<save_dir>/<ifid>.aux`; relaunching restores it before the game's first `restore table`.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/main.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): persist/restore aux save data (archive + global modes)

Global mode writes the per-game <ifid>.aux after any turn that changed the aux
table and pre-loads it at startup; archive mode rides the existing per-turn
auto-save and repopulates the VM table from the loaded archive. Ask is treated
as archive until the first-use prompt lands.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: app — first-use prompt dialog (resolve `Ask`)

**Files:**
- Modify: `crates/app/src/state.rs` — add `pub aux_prompt: bool` (+ init `false`).
- Create: `crates/app/src/render/aux_dialog.rs` — `draw_aux_dialog` (mirror `render/reset_dialog.rs`); register in the render module.
- Modify: `crates/app/src/main.rs` — open the dialog when `aux_dirty` && `aux_storage == Ask`; intercept its keys (mirror the reset-dialog block ~1282); on choice, set `state.config.aux_storage`, `write_config`, persist, close.

**Interfaces:**
- Consumes: `app::config::{AuxStorage, write_config}`, `aux_store::write_global_aux`, `state.dialog_focus`.

This is the intricate task. **Mirror the reset-dialog pattern exactly** — the only differences are the two button labels/outcomes and the post-choice action (write config + persist aux).

- [ ] **Step 1: Add `aux_prompt` to `AppState`**

In `crates/app/src/state.rs`, near `reset_dialog: bool` (~734): `pub aux_prompt: bool,`; init `aux_prompt: false,` (~849).

- [ ] **Step 2: Render the dialog**

Create `crates/app/src/render/aux_dialog.rs` mirroring `render/reset_dialog.rs`: return `None` unless `state.aux_prompt`; draw a themed box titled e.g. "Side-data" with the message "This story saves persistent side-data. Where should babelmap keep it?" and two focusable buttons — index 0 "With each save" (Archive), index 1 "Globally" (Global) — highlighted by `state.dialog_focus`. Return a rects struct (`area`, `archive`, `global`, optional `close`) for mouse hit-testing. Register `pub mod aux_dialog;` in `render/mod.rs` and call it from the frame draw where `reset_dialog` is drawn.

- [ ] **Step 3: Trigger the dialog instead of treating Ask as Archive**

Update `persist_aux_after_turn` (Task 5) so `Ask` opens the dialog rather than defaulting:

```rust
    match cfg.aux_storage {
        app::config::AuxStorage::Global => {
            let _ = app::aux_store::write_global_aux(save_dir, ifid, &session.machine.aux_data);
            session.machine.aux_dirty = false;
        }
        app::config::AuxStorage::Archive => {
            session.machine.aux_dirty = false; // archive auto-save already embedded it
        }
        app::config::AuxStorage::Ask => {
            state.aux_prompt = true;      // resolve in the dialog; leave aux_dirty set
            state.dialog_focus = 0;
        }
    }
```

(Signature gains `state: &mut AppState`; pass it at both call sites.)

- [ ] **Step 4: Intercept the dialog keys + resolve**

Mirror the reset-dialog intercept (~1282). Tab/BackTab cycle `dialog_focus` over 2 buttons; Enter activates the focused one; Esc / close → Archive (the conservative default, so the prompt always resolves and never loops). On a resolved choice `mode`:

```rust
            state.aux_prompt = false;
            state.config.aux_storage = mode; // AuxStorage::Archive | Global
            let _ = app::config::write_config(&cfg.user_dir, &state.config);
            if mode == app::config::AuxStorage::Global {
                let _ = app::aux_store::write_global_aux(&save_dir, &ifid, &session.machine.aux_data);
            }
            // Archive mode: the next archive auto-save embeds the table.
            session.machine.aux_dirty = false;
```

(Use `state.config.user_dir` for `write_config`; `save_dir`/`ifid`/`session` are in run-loop scope.)

- [ ] **Step 5: Build, test, headless smoke + manual check**

Run: `cargo build -p app && cargo test -p app` → 0 warnings, suite + headless PASS.

Manual (not gating): set `aux_storage = ask`; a story's first `save table` pops the dialog; choosing "Globally" writes config + `<ifid>.aux` and stops prompting; reopening config shows the new value.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/state.rs crates/app/src/render/aux_dialog.rs crates/app/src/render/mod.rs crates/app/src/main.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): first-use prompt for aux storage mode (resolves ask)

On the first aux save while aux_storage is ask, a themed dialog asks whether to
keep side-data with each save or globally; the choice is written to config and
governs all games. Esc/close defaults to archive so the prompt always resolves.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- **Dependency order:** 1 → 2 → 3 → 4 → 5 → 6. Task 1 is `cargo test -p zvm`; 2–6 are `cargo test -p app`. Each ends green, 0 warnings, before committing.
- **Engine stays FS-free and never suspends** for aux ops — all I/O is app-side (Tasks 2, 5, 6).
- **archive.rs is mode-agnostic:** it always embeds/extracts `aux.dat`. The mode decision lives in the app (Task 5/6): global mode pre-loads + writes the global file and ignores the archive copy on load; archive mode uses the archive copy.
- **Task 5/6 are the risky integration tasks** (run-loop + dialog). Line numbers (~904/931/933/965/2322/2352/2733/1282) are from a snapshot; confirm by grep before editing. `session.machine` is public; `post_turn_bookkeeping` takes `&GameSession` (immutable) so aux persistence + `aux_dirty` clearing happen at the call site where `session` is `&mut`.
- **`Ask` + no dialog host (CLI/headless):** those paths never call `persist_aux_after_turn`, so `Ask` never blocks there; aux works in-memory for the session.
- `README.md` is committed; `TODO.md` is gitignored — never stage it. No README change required (the config screen surfaces the setting); add one only if asked.
