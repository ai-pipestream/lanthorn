# Glulx Save/Restore — Phase A (gvm: Glk model in the snapshot) Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `gvm`'s `save_state`/`restore_state` include the Glk window/stream `Model`, so a Glulx snapshot is self-contained and a cross-session restore reinstalls the windows. Z-machine unaffected; existing gvm round-trip stays exact.

**Spec:** `docs/superpowers/specs/2026-06-27-glulx-save-restore-design.md` (Phase A).

## Existing interfaces

- `crates/gvm/src/exec.rs`: `Machine::save_state(&self) -> Vec<u8>` (writes `FORM IFZS`: `IFhd`/`CMem`/`Stks`/`MAll`/`GReg` via `push_chunk`); `restore_state(&mut self, &[u8]) -> Result<(), GError>` (`find(b"…")` per chunk). Add a `Glk ` chunk.
- `crates/gvm/src/glk.rs`: `Model { windows: Vec<Option<Window>>, streams: Vec<Option<Stream>>, root: u32, cur_stream: u32, cur_style, display size, … }`; `Window { win_type: WinType, rock, stream, pair fields (split dir/method/size/children), grid cells + cursor }`; `Stream { kind, memory buffer addr/len/positions, rock, window assoc }`. The `Machine` holds the `Model` (find the field, e.g. `self.glk`).
- `crates/gvm/GLULX_NOTES.md`: document the `Glk ` chunk format.

## Global Constraints

- `gvm` stays zero-dep. 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green per task.
- Z-machine untouched; the existing gvm same-session save/restore round-trip stays exact; old snapshots (no `Glk ` chunk) restore with an empty model (non-fatal).
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push. Do not edit `TODO.md`.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

## Task 1: Serialize/restore the Glk `Model` (`Glk ` chunk)

**Files:** `crates/gvm/src/glk.rs` (a `Model::serialize()/deserialize()` or free fns), `crates/gvm/src/exec.rs` (`save_state`/`restore_state` wiring), `GLULX_NOTES.md`, `asm.rs` (test helpers).

- [ ] **Step 1:** Note the `Glk ` chunk layout in `GLULX_NOTES.md` (a self-describing binary: counts + per-window/per-stream records). Use little/big-endian consistently with the rest of gvm.
- [ ] **Step 2: Failing tests** in gvm:
  - **Cross-session:** build a program that `glk_window_open`s a pair (grid + buffer), opens a memory stream, sets current stream/style, and writes some grid cells; `save_state`; construct a **fresh** `Machine` from the same image; `restore_state`; assert the restored `Model` has the same window tree (ids/types/rocks/pair split), `root`/`cur_stream`/`cur_style`, the memory stream (addr/len/positions), and the grid cells/cursor — then a subsequent `glk_put`/grid op routes to the right window.
  - **Back-compat:** a snapshot with the `Glk ` chunk stripped restores with an empty `Model` and returns `Ok` (no panic).
  - **Same-session exact:** the existing round-trip test (mutate → save → mutate → restore) still passes, now including the model.
- [ ] **Step 3: Implement** `Model` serialize/deserialize: windows (slot index, `WinType` tag, rock, stream id, pair split dir/method/size + child ids), `root`/`cur_stream`/`cur_style`, streams (slot, kind, memory addr/len/read+write positions, rock, window assoc), and text-grid cells + cursor. Wire `push_chunk(&mut body, b"Glk ", &self.glk.serialize())` into `save_state`; in `restore_state`, `find(b"Glk ")` → rebuild the model (absent → empty model). Keep `IFhd`/`CMem`/`Stks`/`MAll`/`GReg` exactly as-is.
- [ ] **Step 4: Run + commit** — `cargo test --workspace` green, 0 warnings. Commit — `feat(gvm): include the Glk window/stream model in save_state/restore_state`.

---

## Self-review checklist (run before final review)

- A Glulx snapshot now round-trips the Glk window tree + streams + grid cells + current stream/style across a **fresh** machine (cross-session restore works).
- Old snapshots without `Glk ` restore (empty model) without panicking.
- `IFhd`/`CMem`/`Stks`/`MAll`/`GReg` are unchanged; the existing same-session round-trip is still exact.
- `gvm` still zero-dep; `GLULX_NOTES.md` documents the `Glk ` chunk.
- 0 warnings; `cargo test --workspace` green.
