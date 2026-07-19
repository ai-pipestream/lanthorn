# Routine-Discovery Disassembly Cache — Design (SQ-0418)

## Problem

The debug inspector's disassembly is correct **forward** from the PC but unreliable **backward**.
The Z-machine is a variable-length ISA with no code/data separation, so "where does the previous
instruction start?" is fundamentally ambiguous. The current `prev_instr` uses a forward-sweep
voting heuristic that guarantees round-trip consistency (`next_instr(prev_instr(a)) == a`) but not
that it found a *true* instruction boundary. Scrolling up therefore hits either non-code bytes
(routine headers, dictionary, grammar, strings) or a misaligned boundary, and mis-decodes them —
surfacing garbage like `op:2op #00, #eb` (an unassigned opcode is the tell: 2OP:0x00 is not a real
instruction).

The root cause is that on-demand, address-local decoding can't know code from data. The fix is to
discover the program's real instruction boundaries **once**, cache them, and navigate the cache.

## Goal

Replace heuristic backward decoding with an exact, cached `address → instruction` map built by
**routine discovery** (recursive descent + a validated linear scan). Backward navigation becomes
exact; addresses outside discovered code are known to be data and rendered as such (no fake
instructions). The Z-machine `Debugger` trait surface is unchanged — this is a correctness upgrade
behind the existing engine-neutral seam, and it establishes the discovery/cache/navigation pattern
that a future gvm (Glulx) debugger reuses.

## Non-goals (v1)

- **Decoding strings.** Packed strings / `print_paddr` targets in high memory are treated as data
  (raw bytes) in v1. Discovering and decoding all string addresses is a separate follow-up.
- **Perfect indirect-call coverage.** RD + linear scan catches ~all real routines; a pathological
  routine reachable by neither a constant call nor a valid-looking header may still be classed as
  data. That is acceptable and strictly better than today.
- **gvm implementation.** Only the *pattern* is designed to generalize; the Glulx impl is its own
  quest.
- **Self-modifying code.** Z-machine code lives in read-only static/high memory; the cache is built
  once and never invalidated. (If a future engine needs invalidation, that's an engine concern.)

## Approach

### Where it lives
The cache is a Z-machine-specific structure owned by `GameSession` (or a `DisasmCache` it holds),
built lazily on first disassembly access and kept for the session. The `Debugger` methods
(`disassemble`, `next_instr`, `prev_instr`) consult it. **No trait change, no app/render/click
change** — `prev_instr`/`next_instr` just stop guessing.

### The cache: an ordered list of display units
Model the whole code+string region as an ordered `Vec<Unit>`, each either:
- `Unit::Instr { addr, next, /* pre-formatted or a decoded Instr */ }` — one decoded instruction
  from a discovered routine, or
- `Unit::RoutineHeader { addr, nlocals }` — optional, a `; routine @…, N locals` marker at each
  routine entry (reads like a real disassembly and visually anchors boundaries), or
- `Unit::Data { addr, len }` — a run of non-code bytes (rendered as raw bytes, e.g. 16/line).

Units tile `[code_region_start, region_end)` with no gaps. Navigation is a binary search on `addr`:
- `next_instr(a)` = the unit strictly after the unit containing `a`.
- `prev_instr(a)` = the unit strictly before it.
- `disassemble(a, n)` = `n` consecutive units from the unit at/after `a`, each formatted (Instr via
  the existing translated formatter; Data as a raw-byte line; RoutineHeader as its marker).
This makes backward navigation exact and unifies code+data scrolling. The existing windowing
(pre-render ~256 lines around `disasm_addr`) is unchanged — we format a window, not the whole cache.

### Discovery algorithm (RD + validated linear scan)
1. **Seeds.** The initial-PC / main routine (header field — v1–5: byte address of the first
   instruction, which is *not* a routine header; v6+: packed main-routine address). Verify the
   exact header offset + packing against the crate's `header`/`memory` module (do not hard-code
   from memory — see the project's "verify external constants" rule).
2. **Recursive descent.** Decode each known routine from its header (1 locals-count byte; in v1–4
   followed by `nlocals` 2-byte initial values; v5+ none → first instruction address). While
   decoding, every `call*` instruction whose routine operand is a **constant** (`Large`/`Small`)
   unpacks to a new routine entry. Iterate to a fixpoint. Collect entries into a sorted set.
3. **Linear-scan augmentation.** Scan the code region for routine headers not reached by any
   constant call (routines invoked indirectly — via object properties, grammar/action tables, or a
   `call` with a variable operand). A candidate at address `p` is a routine iff:
   - the locals byte is `0..=15`, and
   - decoding from its first instruction proceeds cleanly (every opcode assigned, `next_pc`
     strictly advances, no read past the region) up to the next already-known boundary, and
   - it does not overlap a higher-confidence RD routine (RD wins on conflict).
   Accepted candidates join the routine set. (This is the txd-style pass the user selected; the
   validation heuristic is what keeps false positives out — tune + test against real stories.)
4. **Extents.** Sort all routine entries. Each routine's instructions are the linear decode from
   its first-instruction address to the next routine entry (or region end). Gaps before the first
   routine, between routines, and after the last (up to the string region / file end) become
   `Data` units.
5. **Region bounds.** Constrain discovery to the plausible code region (roughly high-memory base up
   to the string region). Deriving exact bounds is itself heuristic; start permissive and let the
   validation in step 3 reject non-code. Reference the header's high-memory / static-memory marks;
   verify offsets against the crate.

### Data & string handling
Everything not in a discovered routine renders as `Data` (raw bytes). This is where the dictionary,
object table tail, grammar, abbreviations, and packed strings land — all correctly *not*
disassembled. A later enhancement can decode known string addresses; out of scope here.

### Runtime confirmation (self-healing)
The static pass is best-effort; **observed execution is ground truth.** Every PC the VM actually
executes is, by definition, a real instruction start — so confirmed PCs override the static guess
and make the cache self-correcting for regions the player reaches.

Sources of confirmed boundaries, all already available:
- `exec_pcs` — instruction start-PCs of the last turn (already tracked for the coverage gutter);
  accumulate them into a session-persistent `confirmed_pcs` set.
- The parked PC (always a real instruction start).
- Each call-stack frame's `func_addr` — a confirmed **routine entry** (the strongest signal), plus
  every executed call target.

Reconciliation rule: a confirmed PC is a definite instruction boundary that **wins over any static
classification**. If a confirmed PC falls in a region the static pass called `Data`, or lands
mid-instruction (misaligned boundary), re-anchor at that address and re-decode forward, splitting/
replacing the affected units locally (cheap — a bounded re-decode, not a full rebuild). A confirmed
`func_addr` is promoted to a routine entry, which re-aligns that whole routine going forward.

Fold this in after each turn (the panel already refreshes per turn): union the turn's `exec_pcs`
(and the current frames' `func_addr`s and parked PC) into `confirmed_pcs`, then repair any cache
units that disagree. Caveats, stated honestly: this only corrects regions **actually executed**
(incremental, on demand); addresses immediately *before* a confirmed PC stay best-effort until they
too are confirmed (though confirming a routine entry cleans that routine forward). The static pass
still provides breadth for unvisited code.

This division of labor is the robustness story: **static discovery for breadth, runtime
confirmation for correctness where you've been** — and it's why even a modest static pass is safe,
since anything it gets wrong in a reachable area is corrected the moment the PC visits it.

## Debugger trait impact

**None.** `disassemble(addr, lines)`, `next_instr(addr)`, `prev_instr(addr)` keep their signatures;
their bodies consult the cache. `GameSession` builds the cache lazily (first call) and memoizes it
(`OnceCell`/`RefCell<Option<…>>` — it's a `&self` read path, so interior mutability, consistent with
the existing `mem_fault` `Cell` pattern; drain any debug-read fault as the current methods do).

## gvm generalization (why this is the right fix, not throwaway)
Because the seam is engine-neutral (each engine formats its own lines and owns navigation), the
**structure** — seeds → RD → linear-scan validation → sorted routines → tiled unit list → binary-
search navigation — is engine-independent. Glulx supplies its own decoder and a slightly stronger
anchor (function headers begin with `0xC0`/`0xC1`), but reuses the same shape. Design the zvm cache
so the discovery/cache/nav logic is conceptually separable from the zvm-specific decode step.

## Testing

zvm (fixture-backed, `minizork.z3`, skips if absent):
- **Known routine count / boundaries.** Assert the discovered routine set matches a known-good
  reference for minizork (capture the count + a few spot entry addresses; cross-check against txd
  output if available). This is the correctness oracle.
- **No garbage above real code.** Disassembling backward from the parked PC across a routine
  boundary yields `RoutineHeader`/`Data` units, never an `op:2op`-style fake instruction.
- **Round-trip + monotonic nav.** `next_instr`/`prev_instr` over the cache are exact inverses on
  code units and never stall.
- **Linear-scan finds an indirectly-called routine** that pure RD misses (pick one in minizork
  reached only via a property/grammar route), and **rejects a non-routine** candidate (a byte that
  looks like a header but doesn't decode cleanly).
- **Build cost.** A timing sanity check that the full-story build is well under a budget (assert it
  completes; the point is it's milliseconds).
- **Runtime confirmation heals a wrong classification.** Deliberately seed the cache with a region
  mis-classified as `Data` (or a misaligned boundary), feed a confirmed PC inside it, and assert the
  affected units are re-decoded to correct instructions and the boundary now matches the confirmed
  PC. Also assert a confirmed `func_addr` promotes to a routine entry.

app:
- `disassemble`/`prev_instr` behavior through `GameSession` is unchanged in signature; a smoke test
  that the disasm window around the PC is stable and that scrolling up shows data markers, not
  garbage.

## Risks / open questions
- **False positives in the linear scan** are the main risk; the decode-cleanly-to-next-boundary
  validation is the mitigation, tuned against real stories. Err toward classifying ambiguous bytes
  as `Data` (safe) over inventing routines.
- **Exact code-region bounds** are heuristic; rely on validation rather than precise bounds.
- **v6/v7 packing** for the main seed and call targets needs the routine-offset header words (the
  same ones the existing `Unpack` reads) — reuse that.
- **Cache memory**: ~one small unit per instruction; sub-MB for any real story. Fine.

## Rollout
Internal to zvm/GameSession, behind the unchanged `Debugger` trait, so the app never sees a
half-built state. Natural phases if it proves large:
1. **RD-only cache + navigation + data units** — correct for everything reached by constant calls.
2. **Linear-scan augmentation** — the validated header scan for indirectly-called routines.
3. **Runtime confirmation** — fold `confirmed_pcs` in per turn to self-heal reachable regions.

Each phase strictly improves correctness and lands behind the same trait. Phase 3 is independently
valuable — it's the safety net that makes phases 1–2's heuristics low-risk.
