# Glulx / Glk Feature-Gap Inventory — 2026-06-30

**Sources audited:**
- `crates/gvm/src/exec.rs` (opcode executor, gestalt dispatch ~L1178, @glk dispatch ~L2056, glk_gestalt ~L2627)
- `crates/gvm/src/glk.rs` (GlkBackend trait, Model, StreamKind, WinType, event types)
- `crates/gvm/src/lib.rs`
- `crates/gvm-cli/src/main.rs` + `glk_term.rs`
- `crates/app/src/glk_backend.rs` + `glulx_session.rs`
- `TODO.md` (SP3/SP4 tracking + "PUSHED OFF" list)
- `docs/superpowers/specs/2026-06-27-compliant-glulx-saves-design.md`
- `docs/superpowers/plans/2026-06-27-compliant-glulx-saves-A-glk-file-layer.md`
- Glulx spec 3.1.2 (fetched from eblong.com) — §2.18 gestalt, §2.10–§2.17 opcodes
- Glk spec 0.7.5 reference (§4 gestalt selectors, §3–§12 features)

**Status key:** DONE / PARTIAL / REPORTS-UNSUPPORTED / MISSING
**Rec key:** ADD-soon / DEFER / SKIP
**Effort key:** S (hours–day) / M (1–3 days) / L (1–2 weeks) / XL (2+ weeks)
**Already tracked:** YES (cite doc) or NO

---

## Top Recommendations

- **Implement `@restart` (0x0122)** — any game with a RESTART command crashes the VM today. One-liner that resets state to the initial image. (Effort: S)
- **Implement `@save` / `@restore` stream opcodes (0x0123/0x0124)** — blocked on the Glk file layer (saves plan A), but the opcode stubs need to at least fail gracefully (not crash) until then. The full compliant-Quetzal round-trip is already designed.
- **Implement Glk file layer (filerefs + file streams)** — tracked in saves plan A but not yet started. Required for `@save`/`@restore` stream opcodes, and games probe create_by_name + does_file_exist at startup to detect saved games. Currently all fileref calls return 0 silently, which is safe but means no in-game SAVE/RESTORE.
- **Implement `glk_get_char_stream` / `glk_get_line_stream` / `glk_get_buffer_stream` (0x0090–0x0092, plus uni variants)** — stream-read functions are completely absent. Needed for reading from memory and file streams. Should land alongside the file-layer work.
- **Wire resource streams (`glk_stream_open_resource` 0x0049 / `_uni` 0x014A)** — Blorb resources are already parsed; opening them as read-only Glk streams just needs the plumbing. Some Inform 7 games load data tables via resource streams. (Effort: M)
- **Make `@save`/`@restore` fail gracefully instead of crashing** — until saves plan A/B land, these opcodes hit the `other => Err("illegal opcode")` arm, which halts the machine. A soft no-op (store failure, emit diagnostic) would be far better. (Effort: S)
- **Flip `glk_gestalt` truthfully as features land** — the gestalt function currently returns 0 for ~18 selectors that may eventually be supported. The TODO.md entry already tracks this; each subsection below marks the right moment.
- **Timer events (`gestalt_Timer` 5, `glk_request_timer_events` 0x00D6)** — needed for real-time IF (countdown puzzles, interrupt-driven menus). `request_timer_events` is already a logged no-op; adding a host-driven tick would unlock a class of games. (Effort: M)
- **Line terminators (`gestalt_LineTerminators` 18/19, `glk_set_terminators_line_event` 0x0151)** — some games use function keys to navigate menus or select options during line input. Currently a silent no-op (the val2 terminator field is always 0). (Effort: M)
- **`debugtrap` opcode (0x0101)** — currently crashes the VM. Safe to make a graceful no-op (diagnostic + continue) rather than an illegal-opcode fault. (Effort: S)

---

## 1. Glulx VM `@gestalt` Selectors

Dispatch in `exec.rs` ~L1178. Selectors not in the `match` return 0 from the `_ => 0` default.

| Selector | Value | Spec § | Current | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|---|
| GlulxVersion | 0 | §2.18 | DONE | Returns 0x0003_0102 (spec 3.1.2) | — | — | — |
| TerpVersion | 1 | §2.18 | DONE | Returns 0x0000_0100 (gvm 0.1.0) | — | — | — |
| ResizeMem | 2 | §2.18 | DONE | Returns 1 | — | — | — |
| Undo | 3 | §2.18 | DONE | In-memory, up to UNDO_CAP=16 snapshots | — | — | — |
| IOSystem | 4 | §2.18 | PARTIAL | null(0)+Glk(2)=1; filter(1)=0 (deferred; diagnostic emitted); FyreVM(20)=0 | M | DEFER | YES – TODO.md SP3 |
| Unicode | 5 | §2.18 | DONE | Returns 1 | — | — | — |
| MemCopy | 6 | §2.18 | DONE | Returns 1; mzero+mcopy fully implemented | — | — | — |
| MAlloc | 7 | §2.18 | DONE | Returns 1; malloc+mfree fully implemented | — | — | — |
| MAllocHeap | 8 | §2.18 | DONE | Returns `heap_start` (0 when inactive) | — | — | — |
| Acceleration | 9 | §2.18 | REPORTS-UNSUPPORTED | 0x0180/0x0181 store assignments; no interception; reports 0 | M | DEFER | YES – TODO.md SP3 |
| AccelFunc | 10 | §2.18 | REPORTS-UNSUPPORTED | Reports 0 for all function numbers | M | DEFER | YES – TODO.md SP3 |
| Float | 11 | §2.18 | REPORTS-UNSUPPORTED | All FP opcodes unimplemented (crash if hit); reports 0 | L | DEFER | YES – TODO.md SP3 |
| ExtUndo | 12 | §2.18 | MISSING | Not in the match; falls to `_ => 0`; spec 3.1.2 extended undo (unlimited depth) | S | DEFER | NO |
| DoubleFloat | 13 | §2.18+ | MISSING | Not in spec 3.1.2; added in later spec revisions; all DF opcodes absent | L | SKIP | NO |

**Notes:**
- IOSystem filter(1): `setiosys` stores mode=1 + emits a diagnostic; the filter function is never called. A game that switches to filter mode will see no output and no crash.
- Acceleration: the `accelfunc`/`accelparam` tables are maintained in memory (correct storage behavior), but the accelerated function interception that replaces VM calls with native Rust code is not done. Games relying on acceleration for performance will run the unaccelerated VM path instead.

---

## 2. Glk `glk_gestalt` / `glk_gestalt_ext` Selectors

Dispatch in `exec.rs` ~L2627 (`glk_gestalt` fn). `glk_gestalt_ext` (0x0005) delegates to the same function and ignores the `arr`/`len` result-array arguments.

| Selector | Value | Glk spec § | Current | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|---|
| Version | 0 | §4 | DONE | Returns 0x0000_0706 (Glk 0.7.6; Wave-5) | — | — | — |
| CharInput | 1 | §4 | DONE | Returns 1 (with val=any key → 1) | — | — | — |
| LineInput | 2 | §4 | DONE | Returns 1 | — | — | — |
| CharOutput | 3 | §4 | PARTIAL | Returns 2 (ExactPrint) for any char; `glk_gestalt_ext` arr/len ignored | S | DEFER | NO |
| MouseInput | 4 | §4 | MISSING | Returns 0; `glk_request_mouse_event` is a logged no-op | S | DEFER | NO |
| Timer | 5 | §4 | MISSING | Returns 0; `glk_request_timer_events` is a logged no-op | M | ADD-soon | NO |
| Graphics | 6 | §4 | MISSING | Returns 0; wintype_Graphics (2) unsupported | L | DEFER | NO |
| DrawImage | 7 | §4 | MISSING | Returns 0; no image drawing in any backend | L | DEFER | NO |
| Sound | 8 | §4 | MISSING | Returns 0; no sound channel API | L | DEFER | NO |
| SoundVolume | 9 | §4 | MISSING | Returns 0 | M | DEFER | NO |
| SoundNotify | 10 | §4 | MISSING | Returns 0 | M | DEFER | NO |
| Hyperlinks | 11 | §4 | MISSING | Returns 0; no hyperlink API | M | DEFER | NO |
| HyperlinkInput | 12 | §4 | MISSING | Returns 0 | M | DEFER | NO |
| GraphicsTransparency | 13 | §4 | MISSING | Returns 0 | L | DEFER | NO |
| Unicode | 15 | §4 | DONE | Returns 1 | — | — | — |
| UnicodeNorm | 16 | §4 | DONE | Returns 1; `glk_buffer_canon_decompose_uni` (0x0123, NFD) and `_normalize_uni` (0x0124, NFC) implemented via checked-in generated Unicode 16.0.0 tables (SQ-0317) | — | — | — |
| LineInputEcho | 17 | §4 | PARTIAL | Returns 0; `glk_set_echo_line_event` (0x0150) is a silent no-op | S | DEFER | NO |
| LineTerminators | 18 | §4 | MISSING | Returns 0; `glk_set_terminators_line_event` (0x0151) is a silent no-op | M | ADD-soon | NO |
| LineTerminatorKey | 19 | §4 | MISSING | Returns 0; val2 of line-input events is always 0 | M | ADD-soon | NO |
| DateTime | 20 | §4 | DONE | Returns 1; full date/time selector family 0x0160–0x016F; `_local` uses real host tz offset via `GlkBackend::local_utc_offset_seconds` (Wave-5 T1; T4 SQ-0317 real local time) | — | — | — |
| Sound2 | 21 | §4 | DONE | Returns 1 (follows sound_enabled); extended suite `glk_schannel_create_ext` (0x00F4), `_play_multi` (0x00F7), `_set_volume_ext` (0x00FD), `_pause` (0x00FE), `_unpause` (0x00FF) + evtype_VolumeNotify (9) implemented (SQ-0308) | — | — | — |
| ResourceStream | 22 | §4 | DONE | Returns 1; `glk_stream_open_resource` (0x0049) and `_uni` (0x013A — this audit's 0x014A was wrong) implemented (SQ-0308) | — | — | — |
| GraphicsCharInput | 23 | §4 | MISSING | Returns 0 | M | DEFER | NO |
| GlkDisp (dispatch API) | N/A | §16 | MISSING | Glulx Glk dispatch interface; not a standard gestalt selector — tested by glulxercise `gidispa` group | M | SKIP | YES – TODO.md gidispa |

**Notes:**
- `glk_gestalt_ext` (0x0005) args 3 and 4 (result array pointer + length) are silently ignored. For `gestalt_CharOutput`, the spec says to fill the array with the output representation. Low impact in practice.
- The comment in the `glk_gestalt` fn already documents the supported/unsupported split accurately.

---

## 3. Glulx Opcodes — Unimplemented / Partial

All opcodes not in the `execute()` match arm fall to `other => Err("illegal/unimplemented opcode {:#x}")`, which records a fault in `diagnostics` and halts the machine (`StepResult::Quit`). This is a hard crash, not a graceful no-op.

### 3a. Game-State Opcodes (§2.10)

| Opcode | Hex | Spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|---|
| quit | 0x0120 | §2.10 | DONE | Sets `halted=true` | — | — | — |
| verify | 0x0121 | §2.10 | DONE | Checksums original image | — | — | — |
| restart | 0x0122 | §2.10 | MISSING | **Crashes VM** — hits `illegal opcode` error; any game with a RESTART command breaks | S | ADD-soon | YES – TODO.md SP3 |
| save | 0x0123 | §2.10 | MISSING | **Crashes VM** — hits `illegal opcode` error; in-game SAVE breaks | M | ADD-soon | YES – saves-design.md |
| restore | 0x0124 | §2.10 | MISSING | **Crashes VM** — hits `illegal opcode` error; in-game RESTORE breaks | M | ADD-soon | YES – saves-design.md |
| saveundo | 0x0125 | §2.10 | DONE | In-memory; up to UNDO_CAP snapshots | — | — | — |
| restoreundo | 0x0126 | §2.10 | DONE | Pops and restores newest snapshot | — | — | — |
| protect | 0x0127 | §2.10 | DONE | Protected-range stored; preserved on restore | — | — | — |

**Critical:** `@restart` (0x0122) should be at minimum a graceful soft-fail (clear the stack and reinitialize) rather than a hard crash. The saves design doc tracks the full save/restore work; `@restart` is independent and smaller.

### 3b. Miscellaneous Opcodes (§2.18)

| Opcode | Hex | Spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|---|
| gestalt | 0x0100 | §2.18 | DONE | Full dispatch in `gestalt()` | — | — | — |
| debugtrap | 0x0101 | §2.18 | MISSING | **Crashes VM** — should be a graceful no-op (diagnostic + continue) | S | ADD-soon | NO |
| getmemsize | 0x0102 | §2.18 | DONE | — | — | — | — |
| setmemsize | 0x0103 | §2.18 | DONE | Faults if heap active (per spec) | — | — | — |
| jumpabs | 0x0104 | §2.18 | DONE | — | — | — | — |
| getiosys | 0x0148 | §2.18 | DONE | — | — | — | — |
| setiosys | 0x0149 | §2.18 | PARTIAL | Filter mode (1) stored but deferred; diagnostic emitted | M | DEFER | YES – TODO.md |
| getstringtbl | 0x0140 | §2.11 | DONE | — | — | — | — |
| setstringtbl | 0x0141 | §2.11 | DONE | — | — | — | — |

### 3c. Output Opcodes (§2.11)

| Opcode | Hex | Spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|---|
| streamchar | 0x0070 | §2.11 | DONE | Latin-1 | — | — | — |
| streamnum | 0x0071 | §2.11 | DONE | Signed decimal | — | — | — |
| streamstr | 0x0072 | §2.11 | DONE | E0/E1/E2 dispatch | — | — | — |
| streamunichar | 0x0073 | §2.11 | DONE | Full Unicode code point | — | — | — |
| glk | 0x0130 | §2.11 | DONE | Dispatch to `glk_dispatch()` | — | — | — |

### 3d. Floating-Point Math + Comparisons (§2.12 / §2.13)

All floating-point opcodes are absent from `execute()` — they crash the machine with "illegal opcode".

| Opcode | Hex | Spec § | Status | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| numtof | 0x0190 | §2.12 | MISSING | M | DEFER | YES – TODO.md SP3 |
| ftonumz | 0x0191 | §2.12 | MISSING | M | DEFER | YES – TODO.md SP3 |
| ftonumn | 0x0192 | §2.12 | MISSING | M | DEFER | YES – TODO.md SP3 |
| ceil | 0x0198 | §2.12 | MISSING | M | DEFER | YES |
| floor | 0x0199 | §2.12 | MISSING | M | DEFER | YES |
| fadd | 0x01A0 | §2.12 | MISSING | M | DEFER | YES |
| fsub | 0x01A1 | §2.12 | MISSING | M | DEFER | YES |
| fmul | 0x01A2 | §2.12 | MISSING | M | DEFER | YES |
| fdiv | 0x01A3 | §2.12 | MISSING | M | DEFER | YES |
| fmod | 0x01A4 | §2.12 | MISSING | M | DEFER | YES |
| sqrt | 0x01A8 | §2.12 | MISSING | M | DEFER | YES |
| exp | 0x01A9 | §2.12 | MISSING | M | DEFER | YES |
| log | 0x01AA | §2.12 | MISSING | M | DEFER | YES |
| pow | 0x01AB | §2.12 | MISSING | M | DEFER | YES |
| sin | 0x01B0 | §2.12 | MISSING | M | DEFER | YES |
| cos | 0x01B1 | §2.12 | MISSING | M | DEFER | YES |
| tan | 0x01B2 | §2.12 | MISSING | M | DEFER | YES |
| asin | 0x01B3 | §2.12 | MISSING | M | DEFER | YES |
| acos | 0x01B4 | §2.12 | MISSING | M | DEFER | YES |
| atan | 0x01B5 | §2.12 | MISSING | M | DEFER | YES |
| atan2 | 0x01B6 | §2.12 | MISSING | M | DEFER | YES |
| jfeq | 0x01C0 | §2.13 | MISSING | M | DEFER | YES |
| jfne | 0x01C1 | §2.13 | MISSING | M | DEFER | YES |
| jflt | 0x01C2 | §2.13 | MISSING | M | DEFER | YES |
| jfle | 0x01C3 | §2.13 | MISSING | M | DEFER | YES |
| jfgt | 0x01C4 | §2.13 | MISSING | M | DEFER | YES |
| jfge | 0x01C5 | §2.13 | MISSING | M | DEFER | YES |
| jisnan | 0x01C8 | §2.13 | MISSING | M | DEFER | YES |
| jisinf | 0x01C9 | §2.13 | MISSING | M | DEFER | YES |

**Note on FP opcode range:** The Glulx FP opcodes are in the 0x0190–0x01C9 range — NOT 0x0130–0x013B as mentioned in the audit brief (0x0130 is the `glk` opcode). Implementing all FP ops requires `f32` bit-manipulation (IEEE 754 NaN/Inf handling, `fmod` semantics). Effort is M collectively (one Rust block using `f32` casts). The `Float` gestalt selector should flip to 1 when done.

### 3e. Fully Implemented Opcode Groups (for completeness)

The following groups are fully implemented and pass glulxercise: arithmetic (§2.1: add/sub/mul/div/mod/neg/bitand/bitor/bitxor/bitnot/shiftl/sshiftr/ushiftr), branches (§2.2: jump/jz/jnz/jeq/jne/jlt/jge/jgt/jle/jltu/jgeu/jgtu/jleu/jumpabs), data movement (§2.3: copy/copys/copyb/sexs/sexb), array data (§2.4: aload/aloads/aloadb/aloadbit/astore/astores/astoreb/astorebit), stack (§2.5: stkcount/stkpeek/stkswap/stkroll/stkcopy), functions (§2.6: call/return/tailcall/callf/callfi/callfii/callfiii), continuations (§2.7: catch/throw), memory map (§2.8: getmemsize/setmemsize), heap allocation (§2.9: malloc/mfree), random numbers (§2.14: random/setrandom — with the setrandom(0) caveat below), block operations (§2.15: mzero/mcopy), searching (§2.16: linearsearch/binarysearch/linkedsearch), acceleration storage (§2.17: accelfunc/accelparam — storage only).

### 3f. PRNG — setrandom(0)

`setrandom(0)` is spec-defined to reseed from a true-entropy source. The current implementation falls back to a fixed deterministic seed (`DEFAULT_SEED = 0x2BAD_C0DE`) and emits a diagnostic. This is a PARTIAL implementation.

| Feature | Status | Effort | Rec | Already tracked? |
|---|---|---|---|---|
| setrandom(0) true-entropy seed | PARTIAL | S | DEFER | YES – TODO.md SP3 |

---

## 4. Glk Backend Features

### 4a. Window Management (Glk spec §3)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| wintype_Blank (0) | §3.1 | MISSING | `WinType::from_arg(0)` → None; `glk_window_open` returns 0 with diagnostic | S | DEFER | NO |
| wintype_Pair (1) | §3.3 | DONE | Internal layout node; never requested directly | — | — | — |
| wintype_Graphics (2) | §3.8 | MISSING | `WinType::from_arg(2)` → None; graphics window opens return 0 | L | DEFER | NO |
| wintype_TextBuffer (3) | §3.5 | DONE | Full output, styles, stream, char+line input | — | — | — |
| wintype_TextGrid (4) | §3.6 | DONE | Cursor positioning, stream, char+line input | — | — | — |
| winmethod_NoBorder (0x100) | §3.2 | MISSING | Split method flag; silently ignored. Fine for TUI | S | SKIP | NO |
| glk_window_open / close | §3.1 | DONE | Full tree management, layout, backend notify | — | — | — |
| glk_window_iterate / get_rock | §3.1 | DONE | — | — | — | — |
| glk_window_get_root / parent / sibling | §3.1 | DONE | — | — | — | — |
| glk_window_get_type / size / stream | §3.1 | DONE | — | — | — | — |
| glk_window_get_arrangement / set_arrangement | §3.3 | DONE | Triggers relayout + queues evtype_Arrange | — | — | — |
| glk_window_clear | §3.1 | DONE | Grid resets cursor; buffer clears; backend notified | — | — | — |
| glk_window_move_cursor | §3.6 | DONE | Text-grid cursor | — | — | — |
| glk_set_window | §5.6 | DONE | Sets current stream to window's stream | — | — | — |
| Resize → notify_resize() | §3.4 | PARTIAL | gvm-cli wires it; GlulxSession::set_screen_size exists but does NOT call notify_resize; no evtype_Arrange queued on TUI resize for the app | S | ADD-soon | YES – TODO.md note |
| Graphics window API (draw, fill, erase, bg color) | §8 | MISSING | glk_image_draw (0x00E0), image_draw_scaled (0x00E1), window_flow_break (0x00E8), window_erase_rect (0x00E9), window_fill_rect (0x00EA), window_set_background_color (0x00EB) — all fall to diagnostic | XL | DEFER | NO |
| glk_image_get_info | §8 | MISSING | Falls to diagnostic | L | DEFER | NO |

### 4b. Streams (Glk spec §5)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| Window streams | §5.6 | DONE | Open via window_open; routes to backend | — | — | — |
| Memory streams (byte) | §5.4 | DONE | glk_stream_open_memory (0x0043) | — | — | — |
| Memory streams (unicode) | §5.4 | DONE | glk_stream_open_memory_uni (0x0139) | — | — | — |
| File streams (byte) | §5.5 | MISSING | glk_stream_open_file (0x0042) falls to diagnostic; StreamKind::File not in enum | M | ADD-soon | YES – saves plan A |
| File streams (unicode) | §5.5 | MISSING | glk_stream_open_file_uni (0x0138) falls to diagnostic | M | ADD-soon | YES – saves plan A |
| Resource streams (byte) | §5.8 | DONE | glk_stream_open_resource (0x0049); host GlkBackend::data_resource serves Blorb Data chunks (SQ-0308) | — | — | — |
| Resource streams (unicode) | §5.8 | DONE | glk_stream_open_resource_uni (0x013A); TEXT→UTF-8, BINA/FORM→4-byte BE (SQ-0308) | — | — | — |
| glk_stream_iterate / get_rock | §5 | DONE | — | — | — | — |
| glk_stream_close | §5 | DONE | Returns read/write counts | — | — | — |
| glk_stream_set/get_current | §5 | DONE | — | — | — | — |
| glk_stream_set/get_position | §5 | DONE | Memory streams; window streams return write_count | — | — | — |
| glk_put_char / string / buffer | §5 | DONE | Latin-1 and unicode variants | — | — | — |
| glk_put_*_stream_uni (0x012B–0x012D) | §5 | DONE | Unicode stream-put variants (SQ-0308) | — | — | — |
| Echo streams (0x002D/0x002E) | §3.6 | DONE | window_set/get_echo_stream; loop-guarded; unhooked on stream close (SQ-0308) | — | — | — |
| glk_get_char_stream (0x0090) | §5 | DONE | Reads memory / file / resource streams | — | — | — |
| glk_get_line_stream (0x0091) | §5 | DONE | — | — | — | — |
| glk_get_buffer_stream (0x0092) | §5 | DONE | — | — | — | — |
| glk_get_char_stream_uni (0x0130) | §5 | DONE | Implemented at 0x0130 (this audit's 0x012C was wrong — that slot is put_string_stream_uni; fixed in SQ-0308) | — | — | — |
| glk_get_buffer_stream_uni (0x0131) | §5 | DONE | Implemented at 0x0131 (audit's 0x012D was wrong; SQ-0308) | — | — | — |
| glk_get_line_stream_uni (0x0132) | §5 | DONE | Implemented at 0x0132 (audit's 0x012E was wrong; SQ-0308) | — | — | — |

### 4c. Filerefs (Glk spec §6)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| glk_fileref_create_temp (0x0060) | §6 | MISSING | Silently returns 0 — games probing for save data get "no file" | M | ADD-soon | YES – saves plan A |
| glk_fileref_create_by_name (0x0061) | §6 | MISSING | Returns 0 silently | M | ADD-soon | YES – saves plan A |
| glk_fileref_create_by_prompt (0x0062) | §6 | MISSING | Returns 0 silently; spec requires host dialog (NeedFile suspend) | M | ADD-soon | YES – saves plan A |
| glk_fileref_create_from_fileref (0x0068) | §6 | MISSING | Returns 0 silently | M | ADD-soon | YES – saves plan A |
| glk_fileref_destroy (0x0063) | §6 | MISSING | Returns 0 silently (no-op since no filerefs tracked) | S | ADD-soon | YES – saves plan A |
| glk_fileref_iterate (0x0064) | §6 | PARTIAL | Returns (0, 0) — correct end-of-iteration since no filerefs; rockptr written | S | ADD-soon | YES – saves plan A |
| glk_fileref_get_rock (0x0065) | §6 | MISSING | Returns 0 silently | S | ADD-soon | YES – saves plan A |
| glk_fileref_delete_file (0x0066) | §6 | MISSING | Returns 0 silently | M | ADD-soon | YES – saves plan A |
| glk_fileref_does_file_exist (0x0067) | §6 | MISSING | Returns 0 silently (games can't detect whether a save exists) | M | ADD-soon | YES – saves plan A |
| GlkBackend file methods | §6 | MISSING | Trait lacks file_token_by_name, file_read, file_write, file_exists, file_delete | M | ADD-soon | YES – saves plan A |

### 4d. Events (Glk spec §7)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| evtype_CharInput (2) | §7 | DONE | supply_char delivers it; special keycodes mapped | — | — | — |
| evtype_LineInput (3) | §7 | DONE | supply_line delivers it; val1 = char count; val2 always 0 (no terminators) | — | — | — |
| evtype_Arrange (5) | §7 | DONE | Queued on window layout changes; also on notify_resize (gvm-cli) | — | — | — |
| evtype_Redraw (6) | §7 | PARTIAL | evtype constant defined; never actually queued or delivered | S | DEFER | NO |
| evtype_Timer (1) | §7 | MISSING | glk_request_timer_events (0x00D6) is a logged no-op; no tick mechanism in host | M | ADD-soon | NO |
| evtype_MouseInput (4) | §7 | MISSING | glk_request_mouse_event (0x00D4) is a logged no-op | M | DEFER | NO |
| evtype_SoundNotify (7) | §7 | MISSING | No sound channel support | L | DEFER | NO |
| evtype_Hyperlink (8) | §7 | MISSING | No hyperlink API | M | DEFER | NO |
| glk_select_poll (0x00C1) | §7 | DONE | Drains queued non-input events; returns None immediately | — | — | — |
| Line terminators (val2) | §7 | MISSING | val2 of LineInput events always 0; set_terminators_line_event is a no-op | M | ADD-soon | NO |
| glk_cancel_line_event (0x00D1) | §7 | DONE | Drops request; fills event struct with initlen | — | — | — |
| glk_cancel_char_event (0x00D3) | §7 | DONE | Drops request | — | — | — |
| glk_set_echo_line_event (0x0150) | §7 | PARTIAL | Accepted as a no-op; returns 0; gestalt_LineInputEcho reports unsupported | S | DEFER | NO |
| glk_set_terminators_line_event (0x0151) | §7 | PARTIAL | Accepted as no-op; gestalt_LineTerminators reports unsupported | M | ADD-soon | NO |

### 4e. Styles (Glk spec §5.9)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| glk_set_style / set_style_stream | §5.9 | DONE | All 11 style classes tracked per stream | — | — | — |
| glk_stylehint_set (0x00B0) | §5.9 | PARTIAL | Accepted as no-op (best-effort) | S | SKIP | NO |
| glk_stylehint_clear (0x00B1) | §5.9 | PARTIAL | Accepted as no-op | S | SKIP | NO |
| glk_style_distinguish (0x00B2) | §5.9 | MISSING | Falls to diagnostic; should return 0 (styles not distinguished by default) | S | ADD-soon | NO |
| glk_style_measure (0x00B3) | §5.9 | MISSING | Falls to diagnostic; should return 0 (measurement unsupported) | S | ADD-soon | NO |

**Note:** `glk_style_distinguish` and `glk_style_measure` are the most likely to crash a game on startup (some games probe them). They should return 0 gracefully instead of emitting a diagnostic halt.

### 4f. Unicode Operations (Glk spec §12)

| Feature | Glk spec § | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|---|
| glk_char_to_lower / upper | §12 | DONE | Latin-1 mapping (ASCII + accented) | — | — | — |
| glk_buffer_to_lower_case_uni | §12 | DONE | Full Unicode via Rust `char::to_lowercase` | — | — | — |
| glk_buffer_to_upper_case_uni | §12 | DONE | Full Unicode via Rust `char::to_uppercase` | — | — | — |
| glk_buffer_to_title_case_uni | §12 | DONE | lower_rest variant included | — | — | — |
| glk_buffer_canon_decompose_uni (0x0123) | §12 | DONE | NFD (full recursive canonical decomposition + canonical ordering) via generated Unicode 16.0.0 tables; Hangul algorithmic (SQ-0317) | — | — | — |
| glk_buffer_canon_normalize_uni (0x0124) | §12 | DONE | NFC (NFD then canonical composition, honoring composition exclusions + Hangul) via generated Unicode 16.0.0 tables (SQ-0317) | — | — | — |

### 4g. Sound Channels (Glk spec §9)

All sound-channel API functions fall to the diagnostic arm. Games that probe for sound will see "unhandled @glk selector" in diagnostics but won't crash (returns 0). This includes: `glk_schannel_create` (0x00F0), `glk_schannel_destroy` (0x00F1), `glk_schannel_iterate` (0x00F2), `glk_schannel_get_rock` (0x00F3), `glk_schannel_play` (0x00F8), `glk_schannel_play_ext` (0x00F9), `glk_schannel_stop` (0x00FA), `glk_schannel_set_volume` (0x00FB), and Sound2 extensions (0x00F4–0x00F7, 0x00FD). (corrected 2026-07-04: was swapped)

| Feature | Glk spec § | Status | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|
| All sound channel API | §9 | MISSING | L | DEFER | NO |

### 4h. Hyperlinks (Glk spec §10)

All hyperlink API functions fall to the diagnostic arm: `glk_set_hyperlink` (0x0100), `glk_set_hyperlink_stream` (0x0101), `glk_request_hyperlink_event` (0x0102), `glk_cancel_hyperlink_event` (0x0103). (corrected 2026-07-04: was swapped)

| Feature | Glk spec § | Status | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|
| All hyperlink API | §10 | MISSING | M | DEFER | NO |

### 4i. Date and Time (Glk spec §11)

Implemented (Wave-5 T1): `glk_current_time` (0x0160), `glk_current_simple_time` (0x0161), and the date conversion functions (0x0168–0x016F), via the zero-dep `glk::datetime` module. Real local time (T4, SQ-0317): the `_local` selectors take a per-instant UTC offset from the host via `GlkBackend::local_utc_offset_seconds` (DST-correct; `date_to_time_local` uses mktime-style two-pass offset resolution), falling back to UTC when the host returns `None`. The `app` and `gvm-cli` hosts supply it via `jiff` (thread-safe, cross-platform); gvm stays zero-dep.

| Feature | Glk spec § | Status | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|
| All DateTime API | §11 | DONE | — | — | — |

### 4j. GlkBackend Trait Gaps

The `GlkBackend` trait (`glk.rs`) is missing methods required by pending features:

| Missing method | Required for | Effort | Rec | Already tracked? |
|---|---|---|---|---|
| `file_token_by_name(usage, name)` | Fileref create_by_name | S | ADD-soon | YES – saves plan A |
| `file_token_temp(usage)` | Fileref create_temp | S | ADD-soon | YES – saves plan A |
| `file_exists(token)` | glk_fileref_does_file_exist | S | ADD-soon | YES – saves plan A |
| `file_delete(token)` | glk_fileref_delete_file | S | ADD-soon | YES – saves plan A |
| `file_read(token)` | File stream open (read mode) | S | ADD-soon | YES – saves plan A |
| `file_write(token, data)` | File stream close (write mode) | S | ADD-soon | YES – saves plan A |
| `image_draw(win, image_id, x, y)` | glk_image_draw | — | L | DEFER | NO |
| `image_get_info(image_id)` | glk_image_get_info | — | L | DEFER | NO |

---

## 5. Save Format

| Sub-feature | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|
| App-initiated save (Ctrl+S at glk_select) | DONE | GReg + Glk-chunk format; cross-session restore works | — | — | — |
| saveundo/restoreundo | DONE | In-memory, up to 16 snapshots | — | — | — |
| @save / @restore stream opcodes | MISSING | 0x0123/0x0124 crash VM with "illegal opcode" | M | ADD-soon | YES – saves-design.md |
| @restart opcode | MISSING | 0x0122 crashes VM | S | ADD-soon | YES – TODO.md |
| Compliant Glulx Quetzal (no GReg, proper Stks call-stub resume) | MISSING | Current save uses custom GReg chunk; sub-project B of saves design | M | ADD-soon | YES – saves-design.md |
| UMem chunk reading | MISSING | Current code only reads CMem; foreign saves (from Glulxe etc.) may write UMem | S | ADD-soon | YES – saves-design.md §50 |
| Glk side-car (replaces Glk chunk in save bytes) | MISSING | Design specifies moving Glk model to a glk.json side-car like screen.json | M | ADD-soon | YES – saves-design.md §89 |
| Bidirectional interop (.glksave ↔ other interpreters) | MISSING | Requires compliant Quetzal + drop of GReg | M | ADD-soon | YES – saves-design.md |
| @save graceful fallback (until plan lands) | MISSING | Bare minimum: store failure (0xFFFF_FFFF) + diagnostic instead of crash | S | ADD-soon | NO |
| protect range across @restore | DONE | `protect` opcode sets range; `decompress_ram` re-applies it | — | — | — |
| MAll heap restore | DONE | MAll chunk round-trips | — | — | — |

---

## 6. IOSystem / Filter

| Feature | Status | Notes | Effort | Rec | Already tracked? |
|---|---|---|---|---|---|
| IOSystem 0 (null) | DONE | Output silently discarded | — | — | — |
| IOSystem 2 (Glk) | DONE | All stream output routed through Glk | — | — | — |
| IOSystem 1 (filter) | PARTIAL | setiosys stores mode=1 and emits a diagnostic; no output dispatched to the filter function; gestalt returns 0 for filter(1) | M | DEFER | YES – TODO.md SP3 |
| IOSystem 20 (FyreVM) | MISSING | Not planned; returns 0 from gestalt | — | SKIP | NO |
| String-decoding table (E1 Huffman) | DONE | `setstringtbl`/`getstringtbl` + full Huffman decoder | — | — | — |

---

## 7. glulxercise Conformance

From the TODO.md, ~35 groups pass as of the SP3a merge. The explicitly noted out-of-scope groups are:

| Group | Status | Rec | Notes |
|---|---|---|---|
| filter iosys | PARTIAL / out-of-scope | DEFER | setiosys(1) stored but no filter dispatch |
| gidispa (Glk dispatch API) | MISSING / out-of-scope | SKIP | Glk dispatch interface not applicable to embedded @glk model |
| acceleration interception | PARTIAL / out-of-scope | DEFER | accelfunc/accelparam tables stored; no interception |
| float | MISSING / out-of-scope | DEFER | All 29 FP opcodes absent; gestalt_Float = 0 |

Additional conformance items not explicitly tracked:
- Any glulxercise group testing `@restart` would fail (VM crash).
- Any glulxercise group testing `@save`/`@restore` stream opcodes would fail (VM crash).
- The `debugtrap` test (if present) would fail (VM crash).
- Stream-read tests (glk_get_char_stream etc.) would fail.
- Fileref tests would fail (all return 0).

---

## 8. Already-Tracked Items (Cross-Reference)

The following items from TODO.md "PUSHED OFF" / SP3 / SP4 or the saves design docs overlap with this audit. They are cross-referenced here but NOT duplicated above — the sections above flag them with "YES – [doc]".

| Item | Source | Rec |
|---|---|---|
| @save/@restore stream opcodes | TODO.md SP3 gvm-follow-ups; saves-design.md | ADD-soon |
| @restart opcode | TODO.md SP3 gvm-follow-ups | ADD-soon |
| Compliant Quetzal (no GReg) | saves-design.md sub-project B | ADD-soon |
| Glk file layer (filerefs + file streams) | saves-plan-A.md | ADD-soon |
| Filter iosys | TODO.md SP3 out-of-scope | DEFER |
| Acceleration interception | TODO.md SP3 out-of-scope | DEFER |
| Float opcodes | TODO.md SP3 out-of-scope; gestalt_Float=0 | DEFER |
| setrandom(0) true-entropy | TODO.md SP3 gvm-follow-ups | DEFER |
| gidispa | TODO.md SP3 out-of-scope | SKIP |
| SP4 automapping / Inform 7 location | TODO.md SP4 | DEFER |
| GlulxSession set_screen_size → notify_resize | TODO.md note | ADD-soon |

---

## Gap Count by Recommendation

Counting distinct items (not rows of FP opcodes individually, but as one group):

| Recommendation | Count | Key items |
|---|---|---|
| **ADD-soon** | **16** | @restart, @save/@restore (+ graceful fallback), @save graceful fallback, debugtrap, glk file layer, glk stream reads (get_char/line/buffer + uni = 6 selectors counted as 1 group), resource streams, glk_style_distinguish + measure, line terminators, timer events, GlulxSession→notify_resize, UMem read, Glk side-car, compliant Quetzal |
| **DEFER** | **24** | Float ops (group), Acceleration interception, Filter IOSystem, ExtUndo gestalt, wintype_Blank, wintype_Graphics, charOutput gestalt_ext arr, MouseInput, Graphics gestalt, DrawImage, SoundVolume, SoundNotify, HyperlinkInput, GraphicsTransparency, LineInputEcho, DateTime, Sound2, GraphicsCharInput, setrandom(0), evtype_Redraw, evtype_MouseInput, evtype_SoundNotify, evtype_Hyperlink, sound channel API, hyperlink API, datetime API |
| **SKIP** | **4** | FyreVM, DoubleFloat, winmethod_NoBorder, gidispa |
