# v6 Phase 0 — Boot & Text-Playable — Design

**Date:** 2026-07-21
**Quest:** SQ-0186 (v6 graphical Z-machine)
**Status:** Design — approved, pre-plan
**Prior art:** `docs/design/2026-07-16-glk-crate-and-format-strategy.md` (Decision 2 — implement v6 natively in zvm, not via Glk)

## Goal

Make a real Version-6 Z-machine story **load, boot through its main routine, and take text turns** in babelmap without desync or abort. This is the first of four v6 slices; it delivers the addressing/boot plumbing plus safe stubs for the v6 opcode set, so the VM stays in sync while graphics and the window model remain unimplemented.

## Context: what v6 changes vs. the versions we already run

zvm today accepts v3/4/5/7/8 and rejects v6 at the header (`crates/zvm/src/header.rs:45` → `ZError::GraphicalV6`). v6 differs from the supported versions in exactly three ways that matter for booting:

1. **Packed-address scheme.** v6 unpacks routine and string addresses as `4·P + 8·offset` — the same formula as v7, using the `routines_offset`/`strings_offset` header words at bytes 0x28/0x2A.
2. **Boot entry.** In v3–8 the header word at 0x06 is a *direct instruction address*; execution begins there. In **v6 it is the packed address of the "main" routine**, which the interpreter *calls* (no arguments, no result). When main returns, the story ends.
3. **Opcode set.** v6 adds ~18 graphics/window opcodes, all in the **EXT** family. None are implemented. Several also introduce v6-specific operand forms on existing opcodes (`set_window`, `split_window`, `erase_window`, `set_cursor`, `set_colour`) — those are **out of scope for Phase 0** (Phase 1).

Everything else (dynamic/static memory, dictionary, objects, text decode, the file-length `×8` scale) is shared with v5/v7/v8 and already works.

## Non-goals (later phases)

- The v6 window model (up to 8 windows), cell-quantized tiling, `ScreenModel` graphics nodes — **Phase 1**.
- Real picture rendering, Blorb `Pict` resources — **Phase 2**.
- Mouse, menus, `print_form`, `scroll_window`, margins, v6 colour/cursor semantics — **Phase 1/3**.
- The **cell-text-wins** compositing policy (SQ-0186 note, 2026-07-21) is a Phase 1 rendering concern; Phase 0 renders no v6 graphics at all.

In Phase 0 the ~18 v6 EXT opcodes are **no-ops** that consume their operands correctly and satisfy their store/branch contract. `set_window` et al. run their existing v5 logic (harmless for a headless text boot).

## Design

### 1. Header admission (`crates/zvm/src/header.rs`)

- Move `6` from the reject arm into the accepted set: `3 | 4 | 5 | 6 | 7 | 8 => {}`.
- Read `routines_offset`/`strings_offset` for `6 | 7` (currently `version == 7` only). v6 and v7 both carry them at 0x28/0x2A.
- The `file_length` `×8` scale already covers v6 (`7 | 8 =>` arm — extend to `6 | 7 | 8`).
- Remove/repurpose the `rejects_v6_with_specific_error` test; add a `parses_v6_header_fields` test asserting v6 acceptance and offset reads.

`ZError::GraphicalV6` becomes unused. Leave the variant in place (removing a public error variant is churn beyond this phase's request); note it as dead in the plan for a later cleanup, per house rule on orphans.

### 2. Packed addressing (`crates/zvm/src/memory.rs`)

Add a `6 =>` arm to both `unpack_routine` and `unpack_string`, identical to the v7 arm:

```rust
6 | 7 => 4 * p + 8 * self.header.routines_offset as u32,   // unpack_routine
6 | 7 => 4 * p + 8 * self.header.strings_offset as u32,     // unpack_string
```

Unit tests: `unpack_routine`/`unpack_string` for a v6 story with a nonzero offset return the offset-adjusted address.

### 3. Boot: call the main routine (`crates/zvm/src/cpu/exec.rs`)

For v3–8 today, `with_output` sets `state.pc = mem.initial_pc()` (the raw 0x06 word) and the interpreter runs from there with no enclosing frame.

For **v6**, boot must instead behave as if the game called `main` with zero arguments and no store:

- Unpack the 0x06 word as a **routine** address.
- Synthesize the initial call frame using the **existing call machinery** (the same routine-prologue path that `call_*` opcodes use: read local count, initialise locals, set `state.pc` past the prologue). Do **not** hand-roll a second prologue implementation — reuse it, so local-init and argument semantics stay identical.
- The frame has no store target and no arguments.
- When this outermost frame returns, the story ends. Verify the existing `ret`-from-outermost path yields a clean quit for v6 (it must not read a bogus caller frame). If the current return logic assumes an initial frameless PC, add the minimal v6 branch so an outermost return quits.

The v3–8 path is unchanged: direct `initial_pc`, no synthesized frame.

Unit test: a synthetic v6 `sample_story` whose 0x06 points at a packed routine that immediately `@quit`s (or returns) boots without panic and halts cleanly; a v6 `sample_story` whose main routine prints a token and returns produces that token then ends.

### 4. v6 EXT opcode signatures + no-op bodies

**This is the correctness-critical part.** `crates/zvm/src/cpu/decode.rs:ext_op_sig` returns `(false, false, false)` for every v6 EXT opcode today. That table decides how many bytes the decoder consumes after the operands. If a v6 opcode **stores** or **branches** and the table says it does not, the decoder under-reads the instruction and **the PC desyncs into garbage** — a far worse failure than a clean abort. The executor no-op is trivial; the signature table must be exactly right.

**Signatures MUST be verified against the Z-Machine Standards Document 1.1 opcode tables (§14–§15), not from memory** (house rule: verify external opcode/constant tables against an authoritative source). The working table below is the starting point for that verification, not the final word:

| EXT | Opcode | stores | branches | Phase 0 body |
|-----|--------|--------|----------|--------------|
| 0x05 | draw_picture | no | no | no-op |
| 0x06 | picture_data | no | **yes** | no-op; **branch as "no data available"** (do not take branch) |
| 0x07 | erase_picture | no | no | no-op |
| 0x08 | set_margins | no | no | no-op |
| 0x10 | move_window | no | no | no-op |
| 0x11 | window_size | no | no | no-op |
| 0x12 | window_style | no | no | no-op |
| 0x13 | get_wind_prop | **yes** | no | store 0 |
| 0x14 | scroll_window | no | no | no-op |
| 0x15 | pop_stack | no | no | no-op (see note) |
| 0x16 | read_mouse | no | no | no-op |
| 0x17 | mouse_window | no | no | no-op |
| 0x18 | push_stack | no | **yes** | no-op; **branch as success** (take branch) |
| 0x19 | put_wind_prop | no | no | no-op |
| 0x1A | print_form | no | no | no-op |
| 0x1B | make_menu | no | **yes** | no-op; **branch as failure** (do not take branch) |
| 0x1C | picture_table | no | no | no-op |
| 0x1D | buffer_screen | **yes** | no | store 0 |

Notes:
- `EXT:0x05 draw_picture` already has a placeholder arm (`exec.rs:1474`). Fold it into the uniform v6-stub handling.
- **`pop_stack`/`push_stack` semantics:** these actually manipulate the stack (user or a named stack). A pure no-op is wrong if a v6 game relies on them during boot. Decision: implement `push_stack`/`pop_stack` for the **default (game) stack** minimally — `push_stack value` pushes and branches on success; `pop_stack items` discards `items` values. They are cheap and real stack desync is worse than a graphics stub. Treat the *user-stack* operand form as out of scope (branch = success / no-op) and note it. Confirm exact branch semantics against the ZMSD during implementation.
- The store-0 / branch-default choices must match what the standard says a *failed/absent* operation returns, so a game that checks the result takes a sane path.

The executor gets a single match arm per v6 EXT opcode (or a grouped arm) that honours the signature: `do_store(store, 0)` for stores, `do_branch(...)` with the documented default for branches, plain `Continue` otherwise. Keep the existing "unimplemented EXT" diagnostic fallthrough for anything genuinely unknown.

Decode tests: one test per v6 EXT opcode constructing a byte sequence and asserting the decoder consumes the correct number of bytes (i.e. `next_pc` and `store`/`branch` fields match the signature). These tests are the guard against the desync trap.

### 5. Test fixtures

- Extend `crates/zvm/src/header.rs::tests_support::sample_story` / `sample_header_bytes` to build a valid v6 buffer: version byte 6, a `routines_offset`/`strings_offset` pair, and a 0x06 word pointing at a tiny packed main routine.
- The synthetic v6 story is the unit-level oracle for header/addressing/boot.
- **Real-story smoke:** `stories/zork0-r393-s890714.z6` (bare v6, 300032 bytes) is the acceptance oracle. Its header, confirmed 2026-07-21: version 6, `initial_pc`/main = 0x3871 (packed), `routines_offset` = 0x1d26, `strings_offset` = 0x6c5c (**both nonzero — exercises the `+ 8·offset` term**), file-length word 0x9278 (×8 = 299968). Add a headless `zvm-cli` smoke (step-capped) asserting the story boots to its first input prompt and accepts a command without mem-fault or desync. This is the Phase 0 acceptance gate. `stories/Zork0.blb` (Blorb) and the other v6 titles (`arthur-r74-s890714.z6`, `journey-r83-s890706.z6`, `shogun-r322-s890706.z6` + `.blb` siblings) are for Phase 1+/graphics — Phase 0 uses the bare `.z6`.

### 6. Debug inspector / disassembler (mostly free, one cleanup)

A v6 story flows straight into the debug inspector's disassembly and `--debug` execution trace, so those paths must not desync or mislead. The good news: the disassembler is already largely v6-ready.

- **Shared decode path.** `disasm.rs` reuses `decode` (`cpu/decode.rs`), so the Phase 0 `ext_op_sig` fix makes debug disassembly byte-correct automatically — no separate signature work.
- **Mnemonics already complete.** `disasm.rs::mnemonic` already names all 18 v6 EXT opcodes (0x05–0x08, 0x10–0x1D). No additions needed.
- **`Unpack` already v6-aware.** `disasm.rs::Unpack` reads `routine_off`/`string_off` for `version == 6 || version == 7`. Packed-routine reachability walks unpack correctly for v6 already.
- **`operand_role` needs nothing.** v6 EXT opcodes take window/picture numbers and pixel coordinates — all plain integers, no routine/packed-address operands to annotate.
- **Cleanup:** `cpu/opcode_help.rs:~280` carries a comment *"babelmap rejects v6 stories, so these …"* that becomes false once v6 is accepted. Fix the comment in-phase (orphan created by this change).

**Secondary oracle:** the debug inspector doubles as an independent correctness check for Phase 0. The Rd (reachable-disassembly) provenance walk and the `--debug` executed-PC trace both depend on exactly the decode/unpack correctness this phase delivers. A `--debug` headless run of Zork0 that disassembles the reached region cleanly and produces sane executed-PC coverage is strong evidence the EXT signatures and v6 addressing are right — corroborating the boot smoke from a different angle.

## Testing strategy summary

| Layer | Test |
|-------|------|
| Header | v6 accepted; offsets read from 0x28/0x2A; file-length ×8 |
| Addressing | `unpack_routine`/`unpack_string` v6 arm with nonzero offset |
| Boot | synthetic v6 main-routine call boots + clean outermost return → quit |
| Decode | per-opcode byte-consumption for all 18 v6 EXT opcodes (desync guard) |
| Execute | store-0 / branch-default bodies leave the VM in sync |
| Smoke | Zork0 (`stories/zork0-r393-s890714.z6`) reaches first prompt headlessly |
| Debug (secondary oracle) | `--debug` headless Zork0 run disassembles the reached region without desync + sane executed-PC coverage |

## Risks & mitigations

- **Signature desync (highest risk).** Mitigated by authoritative ZMSD verification + per-opcode decode tests.
- **Outermost-frame return on v6.** The existing return path may assume a frameless initial PC; the synthesized boot frame must return-to-quit cleanly. Covered by a boot test.
- **A v6 game leaning on an unstubbed behavior during boot** (e.g. `get_wind_prop` reading window size to lay out text). Store-0 may route it down an odd path. Acceptable for Phase 0 (text still flows); revisited in Phase 1 when window props become real. Flag any such case surfaced by the real-story smoke.

## Cross-crate / constraints

- **zvm stays zero-dependency.** All Phase 0 work is in zvm (`header.rs`, `memory.rs`, `cpu/decode.rs`, `cpu/exec.rs`, plus a one-line comment fix in `cpu/opcode_help.rs`) — no new deps.
- No app/render changes in Phase 0. `startup.rs` already routes any accepted Z-machine version through `GameSession`; v6 acceptance flows through unchanged.
- No back-compat concerns (pre-release).

## Definition of done

1. All unit/decode/boot tests above pass; full `cargo test -p zvm` green.
2. `cargo clippy -p zvm` clean; zvm `[dependencies]` still empty.
3. Real v6 story (user-supplied) boots headlessly to first prompt and accepts a command — the Phase 0 acceptance smoke.
4. SQ-0186 noted with Phase 0 completion; Phase 1 (window model) scoped next.
