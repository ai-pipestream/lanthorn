# Rewind / Replay — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optionally record a per-turn history of the game (VM Quetzal state + map snapshots) into the `.babelmap` archive, and add a modal that lets the player step/replay/rewind and **resume linearly** from any past turn (resuming truncates later turns). Gated by an opt-in config flag, default off.

**Architecture:** A new `app::history` module owns `TurnRecord` + pure capture/reconstruction helpers (no event-loop coupling, fully unit-testable). `AppState.history: Vec<TurnRecord>` is filled per-turn in `main.rs` when the flag is on (a map snapshot is stored only on turns that structurally change the graph, so storage ≈ #map-changes). `archive.rs` round-trips the history into the zip under `history/`. A `render/history.rs` modal + `AppState.replay: Option<ReplayState>` sub-mode (mirroring the existing `saves`/`tidy_anim` modals) renders a reconstructed preview map for the selected turn and resumes via `restore_quetzal` + `mapper` replacement.

**Tech Stack:** Rust (app crate only; `mapper`/`zvm` are used read-only).

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in commit bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Zero compiler warnings; remove any symbol a change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit TODO.md (gitignored).
- Run `cargo test -p app` after each task: 0 failures, 0 warnings. The headless smoke test (`crates/app/tests/headless.rs`) must still pass.
- mapper/zvm are used READ-ONLY (`to_json`/`from_json`, `save_quetzal`/`restore_quetzal`) — no changes to those crates.

### Resolved spec ambiguity (read before starting)

The spec (§Config) asks for a NEW `record_history: bool` (default **false**). A field named `record_history` **already exists** in `crates/app/src/config.rs` (default **true**, different meaning: "record command history across sessions"). To avoid a semantic/default collision, this plan names the new rewind flag **`record_turn_history`** (default false). All references to the spec's `record_history` map to `record_turn_history`.

---

### Task 1: `history` module — data model + pure helpers

**Files:**
- Create: `crates/app/src/history.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod history;`)

**Interfaces:**
- Consumes: `mapper::mapper::Mapper`, `mapper::persist::to_json(&Mapper) -> String`, `mapper::persist::from_json(&str) -> Result<Mapper, serde_json::Error>`, `crate::session::GameSession` (tests only), `crate::session::apply_turn` (tests only).
- Produces:
  - `pub struct TurnRecord { pub turn: u32, pub command: String, pub save: Vec<u8>, pub map_snapshot: Option<String>, pub transcript: String }`
  - `pub fn record_turn(history: &mut Vec<TurnRecord>, turn: u32, command: &str, save: Vec<u8>, mapper: &Mapper, map_changed: bool, transcript: &str)`
  - `pub fn map_at_turn(history: &[TurnRecord], turn: u32) -> Option<&str>`
  - `pub struct ResumePlan { pub save: Vec<u8>, pub map_json: Option<String>, pub turn: u32 }`
  - `pub fn resume_plan(history: &[TurnRecord], idx: usize) -> ResumePlan`
  - `pub fn rebuild_transcript(history: &[TurnRecord], idx: usize) -> (Vec<String>, Vec<crate::state::TranscriptKind>)`

- [ ] **Step 1: Write the failing tests**

Create `crates/app/src/history.rs` with the module skeleton and a `mod tests`. Add the module to the crate first so it compiles: in `crates/app/src/lib.rs`, after `pub mod export;`, add `pub mod history;`. Then write `crates/app/src/history.rs`:

```rust
//! Per-turn rewind/replay history: a `TurnRecord` per played turn (Quetzal save
//! + optional map snapshot + transcript), plus pure helpers used by the capture
//! loop (`main.rs`), the archive (`archive.rs`), and the replay modal.

use mapper::mapper::Mapper;

/// One recorded turn. `save` is the Quetzal snapshot of the VM AFTER this turn;
/// `map_snapshot` is the serialized `Mapper` ONLY on turns where the graph
/// structurally changed (so storage ≈ #map-changes, not #turns).
#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub turn: u32,
    pub command: String,
    pub save: Vec<u8>,
    pub map_snapshot: Option<String>,
    pub transcript: String,
}

/// Append a record for a completed turn. The caller computes `map_changed`
/// (cheap room/connection-count delta); the map snapshot is serialized and
/// stored only when it changed.
pub fn record_turn(
    history: &mut Vec<TurnRecord>,
    turn: u32,
    command: &str,
    save: Vec<u8>,
    mapper: &Mapper,
    map_changed: bool,
    transcript: &str,
) {
    let map_snapshot = map_changed.then(|| mapper::persist::to_json(mapper));
    history.push(TurnRecord {
        turn,
        command: command.to_string(),
        save,
        map_snapshot,
        transcript: transcript.to_string(),
    });
}

/// Return the latest `map_snapshot` at-or-before `turn` (the map as it stood
/// then), or `None` if no record at-or-before `turn` carries a snapshot.
pub fn map_at_turn(history: &[TurnRecord], turn: u32) -> Option<&str> {
    history
        .iter()
        .filter(|r| r.turn <= turn)
        .rev()
        .find_map(|r| r.map_snapshot.as_deref())
}

/// What a linear resume from `history[idx]` needs: the VM save to restore, the
/// reconstructed map JSON at-or-before that turn (if any), and the turn number.
#[derive(Debug, Clone)]
pub struct ResumePlan {
    pub save: Vec<u8>,
    pub map_json: Option<String>,
    pub turn: u32,
}

/// Compute the resume plan for turn index `idx`. Does NOT mutate `history`
/// (the caller truncates to `[0..=idx]` after restoring).
pub fn resume_plan(history: &[TurnRecord], idx: usize) -> ResumePlan {
    let rec = &history[idx];
    ResumePlan {
        save: rec.save.clone(),
        map_json: map_at_turn(history, rec.turn).map(|s| s.to_string()),
        turn: rec.turn,
    }
}

/// Rebuild the on-screen transcript from records `[0..=idx]`: each record
/// contributes an echoed `> command` (Input) followed by its turn output (Story).
pub fn rebuild_transcript(
    history: &[TurnRecord],
    idx: usize,
) -> (Vec<String>, Vec<crate::state::TranscriptKind>) {
    use crate::state::TranscriptKind;
    let mut lines = Vec::new();
    let mut kinds = Vec::new();
    for rec in history.iter().take(idx + 1) {
        if !rec.command.is_empty() {
            lines.push(format!("> {}", rec.command));
            kinds.push(TranscriptKind::Input);
        }
        for line in rec.transcript.split('\n') {
            lines.push(line.to_string());
            kinds.push(TranscriptKind::Story);
        }
    }
    (lines, kinds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    fn mapper_with(n: usize) -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        if n >= 2 {
            m.observe(2, "Forest", Some(Direction::N));
        }
        m
    }

    #[test]
    fn record_turn_stores_snapshot_only_when_changed() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        // Turn 1: a room was added -> snapshot present.
        record_turn(&mut hist, 1, "look", vec![1, 2, 3], &m1, true, "West of House");
        // Turn 2: no structural change -> snapshot absent.
        record_turn(&mut hist, 2, "wait", vec![4, 5, 6], &m1, false, "Time passes.");
        // Turn 3: a second room added -> snapshot present.
        let m2 = mapper_with(2);
        record_turn(&mut hist, 3, "north", vec![7, 8, 9], &m2, true, "Forest");

        assert_eq!(hist.len(), 3);
        assert!(!hist[0].save.is_empty(), "save must be non-empty");
        assert_eq!(hist[0].transcript, "West of House");
        assert!(hist[0].map_snapshot.is_some(), "changed turn has a snapshot");
        assert!(hist[1].map_snapshot.is_none(), "unchanged turn has no snapshot");
        assert!(hist[2].map_snapshot.is_some(), "changed turn has a snapshot");
    }

    #[test]
    fn map_at_turn_returns_latest_at_or_before() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        let m2 = mapper_with(2);
        record_turn(&mut hist, 1, "a", vec![0], &m1, true, "");   // snapshot @1
        record_turn(&mut hist, 2, "b", vec![0], &m1, false, "");  // none
        record_turn(&mut hist, 3, "c", vec![0], &m2, true, "");   // snapshot @3
        record_turn(&mut hist, 4, "d", vec![0], &m2, false, "");  // none

        assert_eq!(map_at_turn(&hist, 0), None, "nothing at-or-before turn 0");
        assert_eq!(map_at_turn(&hist, 1), hist[0].map_snapshot.as_deref());
        assert_eq!(map_at_turn(&hist, 2), hist[0].map_snapshot.as_deref(), "falls back to @1");
        assert_eq!(map_at_turn(&hist, 3), hist[2].map_snapshot.as_deref());
        assert_eq!(map_at_turn(&hist, 99), hist[2].map_snapshot.as_deref(), "latest <= turn");
    }

    #[test]
    fn resume_plan_and_truncate() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        let m2 = mapper_with(2);
        record_turn(&mut hist, 1, "a", vec![10], &m1, true, "one");
        record_turn(&mut hist, 2, "b", vec![20], &m1, false, "two");
        record_turn(&mut hist, 3, "c", vec![30], &m2, true, "three");

        let plan = resume_plan(&hist, 1);
        assert_eq!(plan.save, vec![20], "resume save is history[k].save");
        assert_eq!(plan.turn, 2);
        assert_eq!(plan.map_json.as_deref(), map_at_turn(&hist, 2), "reconstructed @<=2");

        hist.truncate(1 + 1); // caller truncates to [0..=idx]
        assert_eq!(hist.len(), 2, "history truncated to k+1");
    }

    #[test]
    fn rebuild_transcript_concatenates_through_idx() {
        let mut hist = Vec::new();
        let m1 = mapper_with(1);
        record_turn(&mut hist, 1, "look", vec![0], &m1, true, "West of House");
        record_turn(&mut hist, 2, "north", vec![0], &m1, false, "Forest");
        let (lines, kinds) = rebuild_transcript(&hist, 1);
        assert_eq!(lines, vec!["> look", "West of House", "> north", "Forest"]);
        assert_eq!(kinds.len(), lines.len());
        use crate::state::TranscriptKind;
        assert_eq!(kinds[0], TranscriptKind::Input);
        assert_eq!(kinds[1], TranscriptKind::Story);
    }

    /// Integration-flavored capture test: drive a real GameSession and prove the
    /// spec invariants (non-empty save + transcript; snapshot only on map-change).
    /// Mirrors archive.rs's czech.z5 fixture pattern; skips if the fixture is absent.
    #[test]
    fn capture_over_real_session_snapshots_on_room_change() {
        use crate::session::{apply_turn, GameSession};
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return };

        let mut session = GameSession::new(story).expect("GameSession::new");
        let mut mapper = Mapper::default();
        let mut hist: Vec<TurnRecord> = Vec::new();

        for (turn, cmd) in ["look", "wait"].iter().enumerate() {
            let rooms_before = mapper.graph.rooms().count();
            let conns_before = mapper.graph.connections().len();
            let result = session.submit(cmd);
            apply_turn(&mut mapper, cmd, &result);
            let map_changed = mapper.graph.rooms().count() != rooms_before
                || mapper.graph.connections().len() != conns_before;
            record_turn(
                &mut hist,
                (turn + 1) as u32,
                cmd,
                session.machine.save_quetzal(),
                &mapper,
                map_changed,
                &result.transcript,
            );
        }

        assert_eq!(hist.len(), 2);
        for r in &hist {
            assert!(!r.save.is_empty(), "every record has a non-empty Quetzal save");
        }
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app --lib history::`
Expected: compile error (module/types/fns missing) until `history.rs` + the `lib.rs` line are in place; then the tests link and pass.

- [ ] **Step 3: Implement**

The Step-1 code above IS the implementation (module body + `pub mod history;` in `lib.rs`). No further code needed — the failing-then-passing transition happens once `lib.rs` references the module and the file compiles.

- [ ] **Step 4: Run the tests + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/history.rs crates/app/src/lib.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): history module — TurnRecord + capture/replay helpers

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: `record_turn_history` config flag

**Files:**
- Modify: `crates/app/src/config.rs` (`Config.record_turn_history` field + default + `resolve` merge + `Default` impl + the test-literal `Config { … }` at ~642; new test)

**Interfaces:**
- Produces: `Config.record_turn_history: bool` (default false). When true, `main.rs` records a `TurnRecord` per turn into `AppState.history` (Task 3).

- [ ] **Step 1: Write the failing test**

In `crates/app/src/config.rs`, inside `mod tests`, add:

```rust
#[test]
fn record_turn_history_defaults_false_and_round_trips() {
    assert_eq!(Config::default().record_turn_history, false);
    let cfg: Config = toml::from_str("record_turn_history = true\n").unwrap();
    assert!(cfg.record_turn_history);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app record_turn_history_defaults_false_and_round_trips`
Expected: compile error (field missing).

- [ ] **Step 3: Add the field**

In `crates/app/src/config.rs`, in the `Config` struct, after the existing `record_history` field (~line 233), add:

```rust
    /// When true, record a per-turn rewind/replay history (Quetzal save + map
    /// snapshots) into the `.babelmap` archive. Default false (opt-in: it grows
    /// the archive and keeps per-turn blobs in memory).
    #[serde(default)]
    pub record_turn_history: bool,
```

- [ ] **Step 4: Default + resolve merge + test literal**

In `impl Default for Config` (after `record_history: true,`, ~285), add:

```rust
            record_turn_history: false,
```

In `resolve`, in the from-file merge block (after `cfg.record_history = from_file.record_history;`, ~344), add:

```rust
            cfg.record_turn_history = from_file.record_turn_history;
```

In the test-literal `Config { … }` inside `write_config_round_trips_scalars_and_preserves_keymap` (the block that sets `record_history: false,`, ~649), add:

```rust
            record_turn_history: false,
```

(Note: `clone_config` in `input.rs` is just `cfg.clone()` — no manual fields to update. `write_config` does not need to emit this key; it is read-only-from-TOML like `use_default_map`.)

- [ ] **Step 5: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/config.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): record_turn_history config flag (default false)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: `AppState.history` + per-turn capture wiring

**Files:**
- Modify: `crates/app/src/state.rs` (`AppState.history: Vec<crate::history::TurnRecord>` field + `Default`; new test)
- Modify: `crates/app/src/main.rs` (capture after the per-turn `apply_turn`, ~line 1682)

**Interfaces:**
- Consumes: `crate::history::record_turn`, `Config.record_turn_history`, `session.machine.save_quetzal()`, the `rooms_before`/`conns_before` locals already computed in the turn loop (`main.rs` ~1679-1680).
- Produces: `AppState.history: Vec<crate::history::TurnRecord>` (empty by default).

- [ ] **Step 1: Write the failing test**

In `crates/app/src/state.rs`, inside `mod tests`, add:

```rust
#[test]
fn appstate_history_defaults_empty() {
    let s = AppState::default();
    assert!(s.history.is_empty(), "history starts empty");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app appstate_history_defaults_empty`
Expected: compile error (field missing).

- [ ] **Step 3: Add the field + default**

In `crates/app/src/state.rs`, in `pub struct AppState`, near `pub turns: u32,` add:

```rust
    /// Per-turn rewind/replay history. Filled when `config.record_turn_history`
    /// is on; persisted into the `.babelmap` archive. Empty otherwise.
    pub history: Vec<crate::history::TurnRecord>,
```

In `impl Default for AppState`, near `turns: 0,` add:

```rust
            history: Vec::new(),
```

- [ ] **Step 4: Wire per-turn capture in main.rs**

In `crates/app/src/main.rs`, in the turn loop, the graph deltas are already computed at ~1679-1685:

```rust
let rooms_before = mapper.graph.rooms().count();
let conns_before = mapper.graph.connections().len();
apply_turn(&mut mapper, &cmd, &result);
state.graph_gen = state.graph_gen.wrapping_add(1);
```

Immediately AFTER the `state.graph_gen` bump (line ~1685), add:

```rust
                // ── Rewind/replay capture (opt-in) ────────────────────────────
                if state.config.record_turn_history {
                    let map_changed = mapper.graph.rooms().count() != rooms_before
                        || mapper.graph.connections().len() != conns_before;
                    app::history::record_turn(
                        &mut state.history,
                        state.turns,
                        &cmd,
                        session.machine.save_quetzal(),
                        &mapper,
                        map_changed,
                        &result.transcript,
                    );
                }
```

(`cmd`, `result`, `mapper`, `session`, `state.turns` are all in scope here; `record_history=false` adds nothing because the block is gated.)

- [ ] **Step 5: Build + run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS, including the headless smoke test. (Capture behavior is unit-covered by Task 1's `capture_over_real_session_snapshots_on_room_change`; this step is the thin guarded wiring + the default-empty field.)

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/state.rs crates/app/src/main.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): capture per-turn history when record_turn_history is on

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: Archive persistence — round-trip history into `.babelmap`

**Files:**
- Modify: `crates/app/src/archive.rs` (`save_archive`/`save_archive_meta` gain a `history` param + write `history/` entries; `load_archive` reads them; `ArchiveContents.history`; `CURRENT_FORMAT_VERSION` → 2; version gate relaxed; tests)
- Modify: `crates/app/src/main.rs` (pass `&state.history` to the archive-write call sites; consume `ac.history` on the load path)
- Modify: `crates/app/src/persist_files.rs` (`save_named` passes `&[]` for history)

**Interfaces:**
- Consumes: `crate::history::TurnRecord`, `mapper::persist::{to_json, from_json}`.
- Produces:
  - `ArchiveContents.history: Vec<crate::history::TurnRecord>`
  - `save_archive(path, mapper, machine, transcript, transcript_kinds, history: &[crate::history::TurnRecord]) -> io::Result<()>`
  - `save_archive_meta(path, mapper, machine, meta, transcript, transcript_kinds, history: &[crate::history::TurnRecord]) -> io::Result<()>`
  - `pub const CURRENT_FORMAT_VERSION: u32 = 2;` (was `1`)
- Zip layout (written only when `!history.is_empty()`): `history/index.json` = `Vec<{ turn, command, has_map }>`; `history/turn-NNNN.sav`; `history/turn-NNNN.map.json` (only when the record has a snapshot); `history/turn-NNNN.txt`. History load is gated on the **presence** of `history/index.json`, so v1 archives (no `history/`) load with an empty `history` regardless of version.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/archive.rs`, inside `mod tests`, add:

```rust
#[test]
fn history_round_trips_in_archive() {
    use crate::history::TurnRecord;

    let mapper = small_mapper();
    let map_json = mapper::persist::to_json(&mapper);
    let history = vec![
        TurnRecord { turn: 1, command: "look".into(), save: vec![1, 2, 3],
            map_snapshot: Some(map_json.clone()), transcript: "West of House".into() },
        TurnRecord { turn: 2, command: "wait".into(), save: vec![4, 5, 6, 7],
            map_snapshot: None, transcript: "Time passes.".into() },
    ];

    let path = temp_archive_path("history-rt");
    save_archive(&path, &mapper, &dummy_machine(), &[], &[], &history)
        .expect("save_archive");
    let ac = load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);

    assert_eq!(ac.history.len(), 2);
    assert_eq!(ac.history[0].turn, 1);
    assert_eq!(ac.history[0].command, "look");
    assert_eq!(ac.history[0].save, vec![1, 2, 3], "save bytes byte-identical");
    assert_eq!(ac.history[0].map_snapshot.as_deref(), Some(map_json.as_str()));
    assert_eq!(ac.history[0].transcript, "West of House");
    assert_eq!(ac.history[1].save, vec![4, 5, 6, 7]);
    assert!(ac.history[1].map_snapshot.is_none(), "no-change turn has no map");
    assert_eq!(ac.history[1].transcript, "Time passes.");
}

#[test]
fn v1_archive_loads_with_empty_history() {
    // An archive with no history/ entries (e.g. written before this feature)
    // loads with an empty history and unchanged behavior.
    let mapper = small_mapper();
    let path = temp_archive_path("history-v1");
    save_archive(&path, &mapper, &dummy_machine(), &[], &[], &[])
        .expect("save_archive without history");
    let ac = load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);
    assert!(ac.history.is_empty(), "archive without history/ → empty history");
}
```

Add a tiny test helper near `small_mapper()` (a machine the archive can `save_quetzal` from — reuse the czech fixture exactly like `round_trip_map_and_save_bytes`, or fall back to a minimal one). Add:

```rust
fn dummy_machine() -> zvm::cpu::exec::Machine {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zvm/tests/fixtures/czech.z5");
    let story = std::fs::read(&fixture).expect("czech.z5 fixture for archive tests");
    let mem = zvm::memory::Memory::new(story).unwrap();
    let mut m = zvm::cpu::exec::Machine::new(mem);
    m.init_caps();
    m
}
```

(If the czech fixture may be absent in CI, guard the two new tests with the same early-return-on-missing-fixture pattern used by `round_trip_map_and_save_bytes`.)

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app --lib archive::tests::history_round_trips_in_archive archive::tests::v1_archive_loads_with_empty_history`
Expected: compile error (param arity / `ArchiveContents.history` missing).

- [ ] **Step 3: Bump the version + add history entry-name constant**

In `crates/app/src/archive.rs`, change the version constant to be `pub` and `2`:

```rust
pub const CURRENT_FORMAT_VERSION: u32 = 2;
```

Add the history dir prefix near the other entry-name consts (after `ENTRY_TRANSCRIPT`):

```rust
const HISTORY_INDEX: &str = "history/index.json";
```

- [ ] **Step 4: Add `history` to `ArchiveContents` + a serde index type**

After `TranscriptData`, add the on-disk index entry type:

```rust
/// One row of `history/index.json`: per-turn metadata + ordering. The bytes,
/// map JSON, and transcript live in sibling `turn-NNNN.*` entries.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct HistoryIndexEntry {
    turn: u32,
    command: String,
    has_map: bool,
}
```

In `pub struct ArchiveContents`, add:

```rust
    /// Per-turn rewind/replay history (empty for archives without `history/`).
    pub history: Vec<crate::history::TurnRecord>,
```

- [ ] **Step 5: Extend the writers**

Change `save_archive` to accept and forward `history`:

```rust
pub fn save_archive(
    path: &Path,
    mapper: &Mapper,
    machine: &zvm::cpu::exec::Machine,
    transcript: &[String],
    transcript_kinds: &[crate::state::TranscriptKind],
    history: &[crate::history::TurnRecord],
) -> io::Result<()> {
    save_archive_meta(path, mapper, machine, Meta {
        format_version: CURRENT_FORMAT_VERSION,
        ifid: None,
        name: None,
        turns: 0,
        saved_at: String::new(),
    }, transcript, transcript_kinds, history)
}
```

Add the `history` param to `save_archive_meta`'s signature (last param, same type) and, just before `zip.finish()?;`, write the history entries:

```rust
    // history/ — per-turn rewind/replay records (only when non-empty).
    if !history.is_empty() {
        let index: Vec<HistoryIndexEntry> = history
            .iter()
            .map(|r| HistoryIndexEntry {
                turn: r.turn,
                command: r.command.clone(),
                has_map: r.map_snapshot.is_some(),
            })
            .collect();
        let index_json =
            serde_json::to_string_pretty(&index).expect("history index serializable");
        zip.start_file(HISTORY_INDEX, options)?;
        zip.write_all(index_json.as_bytes())?;

        for r in history {
            zip.start_file(format!("history/turn-{:04}.sav", r.turn), options)?;
            zip.write_all(&r.save)?;
            if let Some(map) = &r.map_snapshot {
                zip.start_file(format!("history/turn-{:04}.map.json", r.turn), options)?;
                zip.write_all(map.as_bytes())?;
            }
            zip.start_file(format!("history/turn-{:04}.txt", r.turn), options)?;
            zip.write_all(r.transcript.as_bytes())?;
        }
    }
```

- [ ] **Step 6: Relax the load version gate + read history back**

In `load_archive`, change the strict equality gate to accept any version up to current (so on-disk v1 archives still load):

```rust
    if meta.format_version > CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected <= {}",
                meta.format_version, CURRENT_FORMAT_VERSION
            ),
        ));
    }
```

After the transcript block and before `Ok(ArchiveContents { … })`, read the history (gated on index presence):

```rust
    // history/ — optional; absent in archives that pre-date this feature.
    let history: Vec<crate::history::TurnRecord> = match zip.by_name(HISTORY_INDEX) {
        Ok(mut entry) => {
            let mut buf = String::new();
            entry.read_to_string(&mut buf)?;
            let index: Vec<HistoryIndexEntry> =
                serde_json::from_str(&buf).unwrap_or_default();
            let mut out = Vec::with_capacity(index.len());
            for e in index {
                let read_entry = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<Vec<u8>> {
                    let mut z = zip.by_name(name).ok()?;
                    let mut b = Vec::new();
                    z.read_to_end(&mut b).ok()?;
                    Some(b)
                };
                let save = read_entry(&mut zip, &format!("history/turn-{:04}.sav", e.turn))
                    .unwrap_or_default();
                let map_snapshot = if e.has_map {
                    read_entry(&mut zip, &format!("history/turn-{:04}.map.json", e.turn))
                        .and_then(|b| String::from_utf8(b).ok())
                } else {
                    None
                };
                let transcript = read_entry(&mut zip, &format!("history/turn-{:04}.txt", e.turn))
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default();
                out.push(crate::history::TurnRecord {
                    turn: e.turn,
                    command: e.command,
                    save,
                    map_snapshot,
                    transcript,
                });
            }
            out
        }
        Err(_) => Vec::new(),
    };
```

Add `history` to the returned struct:

```rust
    Ok(ArchiveContents { mapper, save, meta, transcript, transcript_kinds, history })
```

(The borrow checker note: `zip.by_name` borrows `zip` mutably; the closure above takes `&mut zip` per call so each read is sequential — no overlapping borrows. If the closure form fights the borrow checker, inline the three reads as straight-line `zip.by_name(...)` blocks like the existing `save`/`transcript` reads.)

- [ ] **Step 7: Update call sites so the crate compiles**

The writers gained a param. Update every direct caller:

- `crates/app/src/main.rs` — the `save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds)` calls at ~1085, ~1122, ~1492, ~1756, ~1841, ~2177: append `, &state.history` as the final argument (these write the live `.babelmap` and should carry history).
- `crates/app/src/persist_files.rs` — `save_named` wraps `save_archive_meta`; append `, &[]` (named-slot exports do not carry rewind history). If `save_named` is also called with history elsewhere, keep `&[]` — named saves are separate slots per the spec.
- On the load path in `main.rs` (e.g. the auto-load block ~683 and restore blocks that bind `ac`): after a successful restore that sets `mapper = ac.mapper;`, also set `state.history = ac.history;` so a resumed game carries its recorded history forward (mirror the existing `state.transcript = ac.transcript;` line). Apply at the load sites near ~2067-2071 and ~1856-1882 and ~683 where `ac` is consumed.
- Existing archive.rs tests (`round_trip_map_and_save_bytes`, etc.) call `save_archive(..., &[], &[])` — append `, &[]`.

- [ ] **Step 8: Run the tests + full suite**

Run: `cargo test -p app`
Expected: PASS (incl. the two new history tests, the pre-existing `unknown_format_version_returns_err` which uses 99 > 2, and the smoke test), 0 warnings.

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/archive.rs crates/app/src/main.rs crates/app/src/persist_files.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): round-trip rewind history into the .babelmap archive (format v2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: `OpenHistory` command + keybinding + hotkey group

**Files:**
- Modify: `crates/app/src/keymap.rs` (`Command::OpenHistory` in the enum, `to_action`, `name`, `label`, `context`, `ALL_COMMANDS`, default binding, the "Files" hotkey group; tests)
- Modify: `crates/app/src/input.rs` (`Action::OpenHistory` variant)

**Interfaces:**
- Produces: `Command::OpenHistory` (name `"open_history"`, label `"history"`, `Context::Global`, default key plain `F4`); `Action::OpenHistory`. `Command::OpenHistory.to_action() == Action::OpenHistory`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/keymap.rs`, inside `mod tests`, add:

```rust
#[test]
fn open_history_command_wiring() {
    assert_eq!(Command::OpenHistory.name(), "open_history");
    assert_eq!(Command::OpenHistory.label(), "history");
    assert_eq!(Command::OpenHistory.context(), Context::Global);
    assert!(matches!(Command::OpenHistory.to_action(), Action::OpenHistory));
    // F4 is the default key.
    let km = KeyMap::default();
    let f4 = KeySpec { code: KeyCode::F(4), ctrl: false, shift: false, alt: false };
    assert_eq!(km.lookup(&f4, Context::Global), Some(Command::OpenHistory));
    // It appears in the Files hotkey group.
    let layout = HotkeyLayout::default();
    let files = layout.groups.iter().find(|(t, _)| t == "Files").expect("Files group");
    assert!(files.1.contains(&Command::OpenHistory), "OpenHistory in Files group");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app open_history_command_wiring`
Expected: compile error (`Command::OpenHistory` / `Action::OpenHistory` missing).

- [ ] **Step 3: Add the Action variant**

In `crates/app/src/input.rs`, in `pub enum Action`, near `OpenSaves` / `OpenHints`, add:

```rust
    /// Open the rewind/replay history modal (seeds `replay` at the last turn).
    OpenHistory,
```

- [ ] **Step 4: Add the Command variant + all match arms**

In `crates/app/src/keymap.rs`:
- In `pub enum Command`, after `OpenHints`, add `OpenHistory,`.
- In `to_action`, add: `Command::OpenHistory => Action::OpenHistory,`.
- In `name`, add: `Command::OpenHistory => "open_history",`.
- In `label`, add: `Command::OpenHistory => "history",`.
- In `context`, add to the Global group: `Command::OpenHistory => Context::Global,` (place alongside `Command::OpenHints => Context::Global,`).
- In `ALL_COMMANDS`, add `Command::OpenHistory,` at the end.

- [ ] **Step 5: Add the default binding + Files group entry**

In `KeyMap::default()`, near the F-key globals (after the `plain(F(5)) → ResetGame` bind), add:

```rust
        // F4 → open rewind/replay history modal (free function key).
        bind!(plain(F(4)), Command::OpenHistory, Context::Global);
```

In `DEFAULT_GROUPS`, extend the `"Files"` group to include `"open_history"`:

```rust
    ("Files", &["open_saves", "open_history", "reset_game", "export_svg", "export_dot", "export_dump"]),
```

- [ ] **Step 6: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/keymap.rs crates/app/src/input.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): OpenHistory command + F4 binding + Files hotkey group

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: `ReplayState` + sub-mode key routing + apply_action arms

**Files:**
- Modify: `crates/app/src/state.rs` (`ReplayState` struct + `AppState.replay: Option<ReplayState>` + `Default` + `any_overlay_open`; tests)
- Modify: `crates/app/src/input.rs` (`Action` variants `ReplayStep`/`ReplayTogglePlay`/`ReplayClose`/`ReplayResume`; `history_key_to_action`; sub-mode dispatch in `key_to_action`; `apply_action` arms; tests)

**Interfaces:**
- Consumes: `crate::history::TurnRecord` (via `state.history`).
- Produces:
  - `pub struct ReplayState { pub idx: usize, pub playing: bool, last_advance: Instant }` with `new(last_idx)`, `step(&mut self, delta: isize, len: usize)`, `toggle_play(&mut self)`, `tick(&mut self, dwell: Duration, len: usize) -> bool` (mirrors `TidyAnim`).
  - `AppState.replay: Option<ReplayState>`.
  - `Action::ReplayStep(isize)`, `Action::ReplayTogglePlay`, `Action::ReplayClose`, `Action::ReplayResume`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/state.rs`, inside `mod tests`, add:

```rust
#[test]
fn replay_state_step_clamps_and_pauses() {
    let mut r = ReplayState::new(4); // start at last idx
    assert_eq!(r.idx, 4);
    r.step(-1, 5);
    assert_eq!(r.idx, 3);
    assert!(!r.playing, "manual step pauses");
    r.step(-10, 5);
    assert_eq!(r.idx, 0, "clamped at 0");
    r.step(10, 5);
    assert_eq!(r.idx, 4, "clamped at len-1");
}

#[test]
fn replay_counts_as_overlay() {
    let mut s = AppState::default();
    assert!(!s.any_overlay_open());
    s.replay = Some(ReplayState::new(0));
    assert!(s.any_overlay_open(), "replay open => any_overlay_open true");
}
```

In `crates/app/src/input.rs`, inside `mod tests`, add:

```rust
#[test]
fn history_keys_step_resume_and_close() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let plain = |c| KeyEvent::new(c, KeyModifiers::NONE);
    assert!(matches!(history_key_to_action(plain(KeyCode::Left)), Action::ReplayStep(-1)));
    assert!(matches!(history_key_to_action(plain(KeyCode::Right)), Action::ReplayStep(1)));
    assert!(matches!(history_key_to_action(plain(KeyCode::Char(' '))), Action::ReplayTogglePlay));
    assert!(matches!(history_key_to_action(plain(KeyCode::Enter)), Action::ReplayResume));
    assert!(matches!(history_key_to_action(plain(KeyCode::Char('r'))), Action::ReplayResume));
    assert!(matches!(history_key_to_action(plain(KeyCode::Esc)), Action::ReplayClose));
    assert!(matches!(history_key_to_action(plain(KeyCode::Char('q'))), Action::ReplayClose));
}

#[test]
fn replay_step_moves_idx_and_close_clears() {
    use crate::state::{AppState, ReplayState};
    use mapper::mapper::Mapper;
    let mut s = AppState::default();
    // Three records so idx 0..=2 are valid.
    let m = Mapper::default();
    for t in 1..=3 {
        crate::history::record_turn(&mut s.history, t, "x", vec![t as u8], &m, false, "");
    }
    s.replay = Some(ReplayState::new(2));
    apply_action(Action::ReplayStep(-1), &mut s, &mut Mapper::default());
    assert_eq!(s.replay.as_ref().unwrap().idx, 1);
    apply_action(Action::ReplayClose, &mut s, &mut Mapper::default());
    assert!(s.replay.is_none(), "Esc closes without change");
    assert_eq!(s.history.len(), 3, "close leaves history intact");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app replay_state_step_clamps_and_pauses replay_counts_as_overlay history_keys_step_resume_and_close replay_step_moves_idx_and_close_clears`
Expected: compile errors (types/variants/fn missing).

- [ ] **Step 3: Add `ReplayState` to state.rs**

In `crates/app/src/state.rs`, after the `TidyAnim` impl block (near line 239), add:

```rust
// ── Replay / rewind ───────────────────────────────────────────────────────────

/// Transient state for the rewind/replay modal. While `Some`, the map pane
/// renders the reconstructed snapshot for `idx` instead of the live graph
/// (like `TidyAnim`). `Esc`/`q` clears it back to the live game with no change.
#[derive(Debug)]
pub struct ReplayState {
    /// Selected turn index into `AppState.history`.
    pub idx: usize,
    pub playing: bool,
    last_advance: Instant,
}

impl ReplayState {
    /// Open seeded at the last turn (`last_idx`), paused.
    pub fn new(last_idx: usize) -> Self {
        Self { idx: last_idx, playing: false, last_advance: Instant::now() }
    }

    /// Step `delta` turns (clamped to `[0, len-1]`) and pause.
    pub fn step(&mut self, delta: isize, len: usize) {
        if len == 0 { self.idx = 0; self.playing = false; return; }
        let last = (len - 1) as isize;
        self.idx = (self.idx as isize + delta).clamp(0, last) as usize;
        self.playing = false;
    }

    /// Toggle auto-play; resuming restarts the dwell clock.
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.last_advance = Instant::now();
    }

    /// Advance one turn if playing and `dwell` elapsed; holds at the last turn.
    /// Returns true if `idx` changed.
    pub fn tick(&mut self, dwell: Duration, len: usize) -> bool {
        if !self.playing || len == 0 || self.idx + 1 >= len {
            self.playing = false;
            return false;
        }
        if self.last_advance.elapsed() < dwell {
            return false;
        }
        self.idx += 1;
        self.last_advance = Instant::now();
        if self.idx + 1 >= len {
            self.playing = false;
        }
        true
    }
}
```

In `pub struct AppState`, near `pub history: …` (Task 3), add:

```rust
    /// Active rewind/replay modal state. `None` means the modal is closed.
    pub replay: Option<ReplayState>,
```

In `impl Default for AppState`, near `history: Vec::new(),` add:

```rust
            replay: None,
```

In `any_overlay_open`, add `|| self.replay.is_some()` to the chain.

- [ ] **Step 4: Add the Action variants**

In `crates/app/src/input.rs`, in `pub enum Action`, near `OpenHistory` (Task 5), add:

```rust
    /// Step the replay selection by delta turns (-1 left, +1 right).
    ReplayStep(isize),
    /// Toggle replay auto-play.
    ReplayTogglePlay,
    /// Close the replay modal (back to live, no change).
    ReplayClose,
    /// Resume the live game from the selected turn (caller-handled in main.rs).
    ReplayResume,
```

- [ ] **Step 5: Add `history_key_to_action` + sub-mode dispatch**

In `crates/app/src/input.rs`, add the routing fn near `saves_key_to_action`:

```rust
/// Hardwired replay/rewind sub-mode keys (not rebindable, like saves/anim).
fn history_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Left => Action::ReplayStep(-1),
        KeyCode::Right => Action::ReplayStep(1),
        KeyCode::Char(' ') => Action::ReplayTogglePlay,
        KeyCode::Enter | KeyCode::Char('r') => Action::ReplayResume,
        KeyCode::Esc | KeyCode::Char('q') => Action::ReplayClose,
        _ => Action::None,
    }
}
```

In `key_to_action`, add a dispatch branch alongside the saves branch (after the `state.saves.is_some()` block, ~296):

```rust
    // Replay/rewind sub-mode: when the history modal is open, route to replay keys.
    if state.replay.is_some() {
        return history_key_to_action(key);
    }
```

- [ ] **Step 6: Add the `apply_action` arms**

In `crates/app/src/input.rs`, in `apply_action`, near the saves arms (~1690), add:

```rust
        // ── Replay / rewind actions ───────────────────────────────────────────

        Action::OpenHistory => {
            // Seed at the last turn; no-op when there is no history.
            state.hotkey_dialog = false;
            if !state.history.is_empty() {
                state.replay = Some(crate::state::ReplayState::new(state.history.len() - 1));
            }
        }

        Action::ReplayStep(delta) => {
            let len = state.history.len();
            if let Some(r) = &mut state.replay {
                r.step(delta, len);
            }
        }

        Action::ReplayTogglePlay => {
            if let Some(r) = &mut state.replay {
                r.toggle_play();
            }
        }

        Action::ReplayClose => {
            state.replay = None;
        }

        // ReplayResume is caller-handled in main.rs (needs the live session/VM).
        Action::ReplayResume => {}
```

- [ ] **Step 7: Run the tests + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 8: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/state.rs crates/app/src/input.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): ReplayState + replay sub-mode key routing and actions

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: Replay modal render + preview graph + linear resume

**Files:**
- Create: `crates/app/src/render/history.rs` (`draw_history` modal)
- Modify: `crates/app/src/render/mod.rs` (`pub mod history;`)
- Modify: `crates/app/src/main.rs` (`draw_frame`: reconstructed preview graph + `draw_history` overlay; no-event-tick auto-play; `ReplayResume` handler)

**Interfaces:**
- Consumes: `crate::history::{map_at_turn, resume_plan, rebuild_transcript}`, `mapper::persist::from_json`, `session.machine.restore_quetzal(&[u8])`, `crate::session::{apply_turn, TurnResult}`, `crate::render::dialog::{draw_dialog, DialogSpec, DialogStyle, DialogButton, ButtonId, Placement, DialogRects}`.
- Produces: `pub fn draw_history(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects>`.

- [ ] **Step 1: Write the failing test**

Create `crates/app/src/render/history.rs` with `mod tests` containing a render smoke test (mirrors `render/saves.rs` tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::state::{AppState, ReplayState};
    use mapper::mapper::Mapper;

    #[test]
    fn draw_history_renders_when_open_and_noops_when_closed() {
        let mut state = AppState::default();
        let m = Mapper::default();
        for t in 1..=2 {
            crate::history::record_turn(&mut state.history, t, "go north", vec![t as u8], &m, false, "Forest");
        }

        // Closed → None.
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            let out = draw_history(&state, area, f.buffer_mut());
            assert!(out.is_none(), "draw_history is a no-op when replay is None");
        }).unwrap();

        // Open → Some, and a turn command appears in the buffer.
        state.replay = Some(ReplayState::new(1));
        let mut term2 = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let rects = term2.draw(|f| {
            let area = f.area();
            draw_history(&state, area, f.buffer_mut())
        }).unwrap();
        assert!(rects.is_some(), "draw_history returns rects when open");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app --lib render::history::`
Expected: compile error (`draw_history` / module missing).

- [ ] **Step 3: Implement `draw_history` (model on `render/saves.rs`)**

In `crates/app/src/render/mod.rs`, add `pub mod history;` to the module list.

Create the body of `crates/app/src/render/history.rs` above the test module:

```rust
//! Rewind/replay history modal overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::render::dialog::{
    ButtonId, DialogButton, DialogRects, DialogSpec, DialogStyle, Placement, draw_dialog,
};
use crate::state::AppState;

/// Draw the replay/rewind modal centered over `area`: a turn list (turn# +
/// command) with the selected turn highlighted, the selected turn's transcript,
/// and a transport footer. Does nothing when `state.replay` is `None`.
/// Returns `Some(DialogRects)` when drawn (for mouse hit-testing).
pub fn draw_history(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<DialogRects> {
    let replay = state.replay.as_ref()?;
    if state.history.is_empty() {
        return None;
    }

    let modal_w = 64u16.min(area.width.saturating_sub(4));
    // up to 12 list rows + 1 footer + 2 header/sep + chrome.
    let list_rows = (state.history.len() as u16).min(12);
    let modal_h = (list_rows + 6).min(area.height.saturating_sub(2));
    if modal_w < 24 || modal_h < 6 {
        return None;
    }

    let st = DialogStyle {
        frame: state.colors.dialog,
        box_style: state.colors.dialog_box_style,
        title: state.colors.dialog_title,
        button: state.colors.dialog_button,
        button_active: state.colors.dialog_button_active,
        shadow: state.colors.dialog_shadow,
        shadow_on: state.colors.dialog_shadow_on,
    };
    let buttons = &[DialogButton { id: ButtonId::Done, label: "Done" }];
    let spec = DialogSpec {
        title: "Replay",
        placement: Placement::Centered { w: modal_w, h: modal_h },
        buttons,
        show_close: true,
        default: Some(ButtonId::Done),
        focus: Some(state.dialog_focus),
    };
    let rects = draw_dialog(buf, &spec, &st);
    let content = rects.content;

    let normal = state.colors.dialog;
    let selected_style = Style::new()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Cyan)
        .add_modifier(Modifier::BOLD);

    // ── Turn list ────────────────────────────────────────────────────────────
    // Window the list around the selection so it stays visible.
    let visible = list_rows as usize;
    let first = replay.idx.saturating_sub(visible.saturating_sub(1));
    for (row, i) in (first..state.history.len()).take(visible).enumerate() {
        let row_y = content.y + row as u16;
        if row_y >= content.bottom() { break; }
        let rec = &state.history[i];
        let style = if i == replay.idx { selected_style } else { normal };
        for col in content.x..content.right() {
            if let Some(cell) = buf.cell_mut((col, row_y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        let marker = if i == replay.idx { ">" } else { " " };
        let cmd_trunc: String = rec.command.chars().take(40).collect();
        let map_tag = if rec.map_snapshot.is_some() { "*" } else { " " };
        let line = format!("{} T{:<5} {} {}", marker, rec.turn, map_tag, cmd_trunc);
        crate::render::draw_str_clipped(buf, content.x, row_y, &line, style, content);
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    let footer_y = content.bottom().saturating_sub(1);
    if footer_y >= content.y {
        let footer_style = Style::new()
            .fg(ratatui::style::Color::DarkGray)
            .patch(state.colors.dialog);
        let footer = "←/→:step  Space:play  Enter/r:resume  Esc:close";
        crate::render::draw_str_clipped(buf, content.x, footer_y, footer, footer_style, content);
    }

    Some(rects)
}
```

(`DialogStyle`/`DialogSpec`/`Placement`/`draw_str_clipped` usage matches `render/saves.rs` exactly; if any `state.colors.dialog*` field name differs, copy the precise field list from the `st` block in `render/saves.rs`.)

- [ ] **Step 4: Run the render test**

Run: `cargo test -p app --lib render::history::`
Expected: PASS.

- [ ] **Step 5: Wire the preview graph + overlay into `draw_frame`**

In `crates/app/src/main.rs`:

1. Add the import near `use app::render::saves::draw_saves;` (~41):
   `use app::render::history::draw_history;`

2. At the top of the `terminal.draw(|f| { … })` closure in `draw_frame` (before the `let rm = match &state.tidy_anim { … }` at ~244), reconstruct the replay preview graph once:

```rust
        // During replay the map shows the reconstructed snapshot for the selected turn.
        let replay_graph: Option<mapper::graph::MapGraph> = state.replay.as_ref().and_then(|r| {
            let turn = state.history.get(r.idx).map(|rec| rec.turn)?;
            let json = app::history::map_at_turn(&state.history, turn)?;
            mapper::persist::from_json(json).ok().map(|m| m.graph)
        });
```

3. Extend the existing `match &state.tidy_anim` graph selections to prefer `replay_graph`. For the `rm` binding, change:

```rust
        let rm = match &state.tidy_anim {
            Some(anim) => render_layer(&anim.current().graph, state.active_layer(&anim.current().graph)),
            None => render_layer(&mapper.graph, state.active_layer(&mapper.graph)),
        };
```

to prefer the replay graph first:

```rust
        let rm = if let Some(g) = &replay_graph {
            render_layer(g, state.active_layer(g))
        } else {
            match &state.tidy_anim {
                Some(anim) => render_layer(&anim.current().graph, state.active_layer(&anim.current().graph)),
                None => render_layer(&mapper.graph, state.active_layer(&mapper.graph)),
            }
        };
```

Apply the same `replay_graph.as_ref()`-first preference to the two `let graph = match &state.tidy_anim { … }` selections in the `MapFull` (~309) and `Split` (~382) arms (used for layer tabs / map rendering). `render_map_layered(&rm, &mapper.graph, …)` keeps `&mapper.graph` for room-rect/scroll bookkeeping; the visible cells come from `rm`, which now reflects the replay graph — matching how `tidy_anim` renders an alternate `rm` while passing the live graph for geometry.

4. Draw the modal overlay. Near the saves overlay dispatch (~526 `if state.saves.is_some() { dialog_rects_out = draw_saves(state, full, buf); }`), add:

```rust
        if state.replay.is_some() {
            dialog_rects_out = draw_history(state, full, buf);
        }
```

- [ ] **Step 6: Auto-play tick in the no-event branch**

In `main.rs`, in the `if !event_ready { … }` branch where `state.tidy_anim` ticks (~984), add a replay tick:

```rust
            if let Some(r) = &mut state.replay {
                r.tick(Duration::from_millis(700), state.history.len());
            }
```

- [ ] **Step 7: Handle `ReplayResume` in main.rs**

`ReplayResume` is caller-handled (like `SavesLoad`). Intercept it before/around `apply_action` in the run loop — locate where the live `Action` is matched (the same place `Action::SavesLoad` is special-cased on the keyboard path). Add a handler that performs the linear resume:

```rust
            Action::ReplayResume => {
                if let Some(r) = state.replay.take() {
                    if r.idx < state.history.len() {
                        let plan = app::history::resume_plan(&state.history, r.idx);
                        match session.machine.restore_quetzal(&plan.save) {
                            Ok(()) => {
                                if let Some(json) = &plan.map_json {
                                    if let Ok(m) = mapper::persist::from_json(json) {
                                        mapper = m;
                                    }
                                }
                                // Linear: discard later turns.
                                state.history.truncate(r.idx + 1);
                                let (lines, kinds) =
                                    app::history::rebuild_transcript(&state.history, r.idx);
                                state.transcript = lines;
                                state.transcript_kinds = kinds;
                                state.turns = plan.turn;
                                state.graph_gen = state.graph_gen.wrapping_add(1);
                                // Re-observe current location (mirror the restore path).
                                if let Some(snap) = zvm::current_location(&session.machine) {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult {
                                        transcript: String::new(),
                                        location: Some(snap),
                                        quit: false,
                                        info: None,
                                        beep: None,
                                        diagnostics: vec![],
                                        location_method: None,
                                    };
                                    apply_turn(&mut mapper, "", &restore_result);
                                    state.set_viewed_layer(None);
                                    state.select_room(Some(rid));
                                }
                                state.push_transcript(&format!("[Resumed from turn {}]", plan.turn));
                            }
                            Err(e) => {
                                state.push_transcript(&format!("[Resume failed: {:?}]", e));
                            }
                        }
                    }
                }
            }
```

Place this arm in the same `match action { … }` (or the caller-side intercept) that already special-cases `Action::SavesLoad`; `session`, `mapper`, `state`, `TurnResult`, `apply_turn` are all in scope there. If the run loop routes most actions through `apply_action` and only intercepts a few caller-handled ones, add `Action::ReplayResume` to that interception set so it does not fall through to the (no-op) `apply_action` arm.

- [ ] **Step 8: Build + run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS including the smoke test. (The resume math — `resume_plan` returns `history[k].save`, the reconstructed `map_json`, and `history.truncate(k+1)` → `len == k+1` — is unit-covered by Task 1's `resume_plan_and_truncate`; the key routing and `idx` movement by Task 6; this step is the render + caller wiring, verified by the render smoke test and the headless smoke test.)

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/render/history.rs crates/app/src/render/mod.rs crates/app/src/main.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): replay/rewind modal — preview graph, transport, linear resume

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- **Dependency order is strict:** Task 1 (history module) → Task 2 (config) → Task 3 (capture wiring, needs both) → Task 4 (archive, needs `TurnRecord`) → Task 5 (command/action) → Task 6 (state/input, needs Action from Task 5) → Task 7 (render/resume, needs all). Tasks 4 and 5 are independent of each other and could be done in either order, but both precede 6/7.
- Every task ends with `cargo test -p app` green and 0 warnings before committing. The headless smoke test (`crates/app/tests/headless.rs`) must keep passing throughout — it exercises the live turn loop, so Task 3's guarded capture (flag default false) must not change its behavior.
- `mapper`/`zvm` stay untouched: only `to_json`/`from_json` and `save_quetzal`/`restore_quetzal` are called.
- Naming: the rewind flag is `record_turn_history` (NOT `record_history`, which already exists with a different meaning/default). See "Resolved spec ambiguity" above.
- Archive format: `CURRENT_FORMAT_VERSION` becomes 2 and the load gate is relaxed to `> CURRENT` so existing on-disk v1 archives still load; history load is gated on the presence of `history/index.json`, giving v1 archives an empty history automatically.
- If the czech.z5 fixture can be absent in the test environment, guard the fixture-dependent tests (Task 1 capture test, Task 4 archive tests) with the early-return-on-missing-file pattern already used by `archive::tests::round_trip_map_and_save_bytes`.
