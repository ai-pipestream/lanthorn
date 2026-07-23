# Scott Adams + Glulx debug inspectors (SQ-0464, SQ-0465)

Two engine-specific `/debug` inspectors, built on the existing engine-generic
`Debugger` trait (`app::engine`, used by `DebugPanelState` — today only the
Z-machine implements it). Subagent lanes; controller reviews + commits per lane.

## Design anchors

- **Scott (SQ-0464)** has no bytecode: the file is a fully-parsed `Database`
  (actions/verbs/nouns/rooms/messages/items). The "disassembly" is a
  ScottKit-style decompile of the actions table with operands resolved to names;
  "executed coverage" becomes **actions that ever fired** (per-turn + cumulative,
  reusing the SQ-0449 `.pcs` sidecar with action indices as the u32s). Live
  state pane = flags (named 15/16), counters, item locations, lamp turns.
- **Glulx (SQ-0465)** is typed (C0/C1 functions, E0–E2 strings, header memmap),
  so misdetection is near-nil — but the decoder is fused into `gvm::exec` and
  must first be split into a shared `decode()` the disassembler reuses (the
  SQ-0463 never-desync lesson). Annotations: `@glk` selector names, accel-func
  badges, inline decoded-string previews. Large I7 images ⇒ cache builds
  lazily/windowed, not whole-image at boot.
- Both map onto the existing panel via `Engine::debugger()`; the Z-flavoured
  section labels (Objects/Dict) gain a per-engine label override
  (Scott: Actions/Items/Vocab/Flags; Glulx: Functions/Strings/Glk).

## Lanes

| Lane | Model | Scope (files) | Quest |
|---|---|---|---|
| **S1** | Sonnet | `scott` crate only: decompiler module (mnemonics shared/verified against `vm.rs` dispatch), fired-action tracking (per-turn + cumulative sets, trace flag) | SQ-0464 |
| **G1** | Opus | `gvm` crate only: split shared `decode()` out of `exec.rs` (behaviour-identical refactor, suite as oracle); disasm cache (RD from start func + type-byte-validated linear scan, tiers, next/prev, opcode help, glk/accel/string annotations); exec-PC tracing (per-turn + cumulative) | SQ-0465 |
| **S2** | Opus | app: `Debugger` impl for Scott over S1, per-engine section labels, ever-fired tier + sidecar, `--debug` wiring, docs | SQ-0464 |
| **G2** | Opus | app: `Debugger` impl for Glulx over G1, Functions/Strings/Glk sections, tiers + sidecar, docs | SQ-0465 |

Order: S1 ∥ G1 (disjoint crates). S2 after S1 (owns the app-side
generalization). G2 after S2 + G1 (reuses the generalization; avoids app
hot-file collisions). Controller gates `cargo test -p zvm -p gvm -p scott -p app`
per landing; scott/gvm stay zero-dep.

## Verification

- S1/G1: crate unit tests + real-image tests (`stories/adv01.dat`…, any local
  `.ulx`/`.gblorb`), skip-if-missing pattern.
- S2/G2: panel refresh/section tests against the new impls; existing Z-machine
  inspector tests must stay green (regression guard on the generalization).
- TTY smokes (user): `/debug` on Adventureland → decompiled actions readable,
  fired actions highlight after a turn; `/debug` on a Glulx story → disassembly
  at the current PC with glk names, functions/strings browsable.
