# Z-Machine Feature-Gap Inventory — 2026-06-30

**Scope:** `zvm` crate + `app`/`zvm-cli` front-ends vs. Z-Machine Standards Document v1.1 (ZMSD §3–16).  
**Code revision:** main @ `93137b3` (2026-06-30).  
**How to read:** Each entry gives: spec §, current status (MISSING / PARTIAL / ADVERTISED-UNSUPPORTED / MISLABELED), effort (S/M/L/XL), recommendation (ADD-soon / DEFER / SKIP), and whether it is already tracked in `TODO.md`.

---

## Top Recommendations

The highest-value additions, in priority order:

1. **Fix standard revision 1.2 → 1.1** (S): byte 0x33 is written as `2`, should be `1`; this is the only published standard. Tiny fix, wrong claim.
2. **Advertise bold + italic in Flags1 v4+ bits 2 & 3** (S): both styles are already rendered by the TUI and zvm-cli via SGR/span; clearing the bits tells games they have no emphasis, so games skip them. Setting them costs nothing and lights up emphasis in every game that probes the header.
3. **Advertise undo in Flags2 bit 4** (S): `save_undo`/`restore_undo` (EXT:0x09/0x0A) are fully implemented. The bit is currently cleared, so games with multi-level undo disable the feature at startup. One-line fix.
4. **Implement `throw` (2OP:0x1C)** (M): catch/throw is a v5+ non-local return pair. `catch` (0OP:0x09) correctly stores the frame depth but `throw` silently falls through the 2OP catch-all with no diagnostic, producing wrong results in any game that uses catch/throw (Infocom's `THROW` in later games). Small stack-unwind implementation.
5. **Complete `erase_window` for -2 and 0** (S): -2 (erase all without unsplit) and 0 (erase lower window) are both no-ops today; only -1 and 1 are handled. Several Infocom v4/v5 games call erase_window(0) to clear the transcript pane before redrawing.
6. **Thread actual function-key terminator through `supply_line`** (M): the terminating-characters table (`is_terminator`) is correctly parsed; `supply_line` always stores 13 (Enter) instead of the actual key code. The host needs to pass the ZSCII code of the terminating key so the store variable gets the right value. Required for cursor-key-driven menus (BeyondZork hint menu, etc.).
7. **Support Font 4 (Courier/fixed-pitch)** (S): `set_font 4` returns 0 (unavailable). Font 4 is just "a fixed-pitch Courier-like font"; a TUI renders it identically to font 1. Return `prev` (success) and keep the same grid rendering; this is a one-line change.
8. **Quetzal UMem chunk support on restore** (M): we only emit CMem and only accept CMem on restore. A save file from another interpreter that used UMem (uncompressed dynamic memory) will fail to restore with `Truncated`. Affects cross-interpreter save compatibility.
9. **`tokenise` custom dictionary operand** (S): VAR:0x1B `tokenise` ignores operand 2 (custom dict address) and always uses the standard story dictionary. Games rarely pass a non-zero dict but when they do the parse results are wrong.
10. **Clear Flags1 v3 bit 6 (variable-pitch default)** (S): bit 6 is not touched during `init_header_caps` for v3, so the story file's value (often 0, but not guaranteed) passes through. We use fixed-pitch; explicitly clearing it is a one-byte write.

---

## Detailed Gap List

### A. Header Capability Bits (ZMSD §11.1)

| Feature | Spec § | Status | What it means | Effort | Rec | TODO? |
|---------|--------|--------|---------------|--------|-----|-------|
| Standard revision written as 1.2 | §11.1.6 | BUG | `mem[0x32]=1, mem[0x33]=2` but ZMSD 1.1 is the latest; no 1.2 exists. Games may probe this. | S | ADD-soon | No |
| Flags1 v4+ bit 2 — bold available | §11.1.3 | ADVERTISED-UNSUPPORTED | CLEARED even though TUI + cli render bold via SGR / span styles. Should be SET. | S | ADD-soon | No — Tracked in item "Audit + expand header capability bits" in TODO but the fix is unresolved. |
| Flags1 v4+ bit 3 — italic available | §11.1.3 | ADVERTISED-UNSUPPORTED | Same as bold; CLEARED despite working italic rendering. | S | ADD-soon | No (same item) |
| Flags1 v4+ bit 0 — colour available | §11.1.3 | CORRECTLY CLEARED | We don't honour game-set colours (we own the palette). No change needed. | — | SKIP | No |
| Flags1 v4+ bit 5 — sound effects | §11.1.3 | CORRECTLY CLEARED | Sampled sounds not implemented; bleeps are visual-only. | — | DEFER (until audio lands) | Yes (sound_effect item) |
| Flags1 v4+ bit 7 — timed keyboard | §11.1.3 | CORRECTLY CLEARED | Timed input is ignored. | — | DEFER | Yes (timed input item) |
| Flags1 v3 bit 6 — variable-pitch default | §11.1.1 | NOT CLEARED (left from story) | We are fixed-pitch; bit 6 should be explicitly 0. | S | ADD-soon | No |
| Flags2 bit 4 — undo available | §11.1.4 | ADVERTISED-UNSUPPORTED | CLEARED despite `save_undo`/`restore_undo` being fully implemented. Games disable undo at start. | S | ADD-soon | No (tracked in audit item only) |
| Flags2 bit 3 — pictures | §11.1.4 | CORRECTLY CLEARED | No picture support. | — | SKIP | Yes (v6 item) |
| Flags2 bit 5 — mouse | §11.1.4 | CORRECTLY CLEARED | No in-game mouse input via read_mouse. | — | SKIP | No |
| Flags2 bit 7 — sound | §11.1.4 | CORRECTLY CLEARED | No audio playback. | — | DEFER | Yes |
| Interpreter number (byte 0x1E) | §11.1.5 | HARDCODED = 6 (IBM PC) | See TODO item; per-game policy deferred. | M | DEFER | Yes |

---

### B. Opcodes (ZMSD §14–15)

#### 2OP

| Opcode | Name | Status | Notes | Effort | Rec | TODO? |
|--------|------|--------|-------|--------|-----|-------|
| 2OP:0x1B | set_colour | ACCEPTED/IGNORED | Explicit no-op; we own the colour scheme. Correct given Flags1 bit 0 = 0. | — | SKIP | No |
| 2OP:0x1C | throw | MISSING — silent | Falls through 2OP catch-all with zero diagnostic. `catch` returns correct frame depth but `throw` never unwinds — any game using catch/throw silently produces wrong results. | M | ADD-soon | No |

All other 2OP opcodes (je, jl, jg, dec_chk, inc_chk, jin, test, or, and, test_attr, set_attr, clear_attr, store, insert_obj, loadw, loadb, get_prop, get_prop_addr, get_next_prop, add, sub, mul, div, mod, call_2s, call_2n) are fully implemented and tested.

#### 1OP

All 1OP opcodes (jz, get_sibling, get_child, get_parent, get_prop_len, inc, dec, print_addr, call_1s, remove_obj, print_obj, ret, jump, print_paddr, load, not/call_1n) are fully implemented.

#### 0OP

All 0OP opcodes (rtrue, rfalse, print, print_ret, nop, save, restore, restart, ret_popped, pop/catch, quit, new_line, show_status, verify, piracy) are fully implemented.

Note on `catch` (0OP:0x09 v5+): correctly stores `frames.len()` as the frame depth. But `throw` (2OP:0x1C) is broken — see above.

#### VAR

| Opcode | Name | Status | Notes | Effort | Rec | TODO? |
|--------|------|--------|-------|--------|-----|-------|
| VAR:0x04 | sread/aread | PARTIAL | Time/routine operands accepted but silently ignored. | M | DEFER | Yes |
| VAR:0x0A | split_window | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x0B | set_window | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x0D | erase_window | PARTIAL | Only -1 (all+unsplit) and 1 (upper) handled. Missing 0 (lower) and -2 (all without unsplit, v5). | S | ADD-soon | No |
| VAR:0x0E | erase_line | IMPLEMENTED | Clears upper-window row from cursor. | — | — | — |
| VAR:0x0F | set_cursor | IMPLEMENTED | Row/col form; v6 three-operand form not needed. | — | — | — |
| VAR:0x10 | get_cursor | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x11 | set_text_style | IMPLEMENTED | Style bitmask tracked; passed to output sink. | — | — | — |
| VAR:0x12 | buffer_mode | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x13 | output_stream | IMPLEMENTED | All four streams, nested stream-3. | — | — | — |
| VAR:0x14 | input_stream | MISSING (warned) | Fires unimplemented warning once. Intentionally deferred. | M | DEFER | Yes |
| VAR:0x15 | sound_effect | PARTIAL | Bleeps #1/#2 queued visually; sampled sounds (#≥3) log a one-time diagnostic. | L | DEFER | Yes |
| VAR:0x16 | read_char | IMPLEMENTED | Suspend + supply_char. Time/routine operands ignored. | — | — | — |
| VAR:0x17 | scan_table | IMPLEMENTED | Word and byte forms; store + branch. | — | — | — |
| VAR:0x18 | not (VAR form) | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x19 | call_vn | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x1A | call_vn2 | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x1B | tokenise | PARTIAL | Operand 2 (custom dict address) ignored; uses standard dictionary. | S | ADD-soon | Yes (noted in code) |
| VAR:0x1C | encode_text | IMPLEMENTED | ✓ | — | — | — |
| VAR:0x1D | copy_table | IMPLEMENTED | Forward/backward/zero forms. | — | — | — |
| VAR:0x1E | print_table | IMPLEMENTED | ZSCII rectangle rendered at cursor. | — | — | — |
| VAR:0x1F | check_arg_count | IMPLEMENTED | ✓ | — | — | — |

#### EXT (v5+, ZMSD §15 extended table)

| Opcode | Name | Status | Notes | Effort | Rec | TODO? |
|--------|------|--------|-------|--------|-----|-------|
| EXT:0x00 | save | IMPLEMENTED | 0-operand (file) + ≥3-operand (aux table) forms. | — | — | — |
| EXT:0x01 | restore | IMPLEMENTED | Same dual-form. | — | — | — |
| EXT:0x02 | log_shift | IMPLEMENTED | ✓ | — | — | — |
| EXT:0x03 | art_shift | IMPLEMENTED | ✓ | — | — | — |
| EXT:0x04 | set_font | PARTIAL | Fonts 0 (query), 1 (normal), 3 (char-graphics) work. Font 2 (picture, v6) returns 0 (unavailable — correct). Font 4 (Courier) also returns 0 — should succeed with same rendering. | S | ADD-soon | No |
| EXT:0x05 | draw_picture (v6) | MISLABELED no-op | Code comment says `set_true_colour` but spec says EXT:0x05 = `draw_picture` (v6). Both are no-ops here; the mislabeling is a comment bug, not a behavioral bug. | S | ADD-soon (fix comment only) | No |
| EXT:0x09 | save_undo | IMPLEMENTED | In-memory Quetzal snapshot; configurable depth (default 16). | — | — | — |
| EXT:0x0A | restore_undo | IMPLEMENTED | ✓ | — | — | — |
| EXT:0x0B | print_unicode | IMPLEMENTED | Valid codepoints output; invalid → U+FFFD. | — | — | — |
| EXT:0x0C | check_unicode | IMPLEMENTED | Returns 3 (print+input) for valid, 0 for invalid. | — | — | — |
| EXT:0x0D | set_true_colour (v5) | NO-OP (catch-all) | Falls through catch-all; not explicitly handled (the explicit arm at 0x05 is mislabeled). Behaviorally a no-op is correct (we own colours). Fine. | S | ADD-soon (add explicit arm + fix 0x05 label) | No |
| EXT:0x06–0x08, 0x10–0x1D | v6 opcodes | NO-OP (catch-all) | draw_picture, picture_data, erase_picture, set_margins, move_window, window_size, window_style, get_wind_prop, scroll_window, pop_stack, read_mouse, mouse_window, push_stack, put_wind_prop, print_form, make_menu, picture_table, buffer_screen. All no-op. v6 not supported. | XL | SKIP | Yes (v6 item) |

---

### C. Screen Model (ZMSD §8)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| v3 status line (score/time) | §8.2 | IMPLEMENTED | Computed from globals G0/G1/G2; time-mode flag from Flags1 bit 1. ✓ | — | — | — |
| v4+ upper window (split/set/cursor/grid) | §8.7 | IMPLEMENTED | UpperWindow grid; split_window resizes; set_window selects; cursor tracked. ✓ | — | — | — |
| erase_window(-1) | §8.7.3 | IMPLEMENTED | Clears all + unsplits. ✓ | — | — | — |
| erase_window(-2) | §8.7.3 | MISSING | Should erase all without changing window properties. Currently ignored. | S | ADD-soon | No |
| erase_window(0) — lower | §8.7.3 | MISSING | Should clear lower window and home cursor (top-left for v5+, bottom-left for v4). Currently ignored. | S | ADD-soon | No |
| erase_window(1) — upper | §8.7.3 | IMPLEMENTED | Clears upper grid. ✓ | — | — | — |
| erase_line | §8.7.3 | IMPLEMENTED | Value=1 clears upper-window row from cursor. ✓ | — | — | — |
| set_text_style (reverse/bold/italic/fixed) | §8.7.2 | IMPLEMENTED | All 4 bits tracked; passed to output sink via print_styled. ✓ | — | — | — |
| bold/italic rendering (TUI) | §8.7.2 | PARTIAL | Bits tracked at engine; zvm-cli emits SGR codes; app renders per-span. But Flags1 bits 2/3 say "unsupported", so games may never send these styles. | S | ADD-soon (see A above) | No |
| Font 1 (normal) | §16 | IMPLEMENTED | Default. ✓ | — | — | — |
| Font 2 (picture, v6) | §16 | CORRECTLY UNAVAILABLE | Returns 0. ✓ | — | — | — |
| Font 3 (char-graphics) | §16 | IMPLEMENTED | Full font3_translate table (ZMSD §16 bitmap mapping); used by BeyondZork. ✓ | — | — | — |
| Font 4 (Courier/fixed-pitch) | §16 | INCORRECTLY UNAVAILABLE | Returns 0 (unavailable); should return prev_font and succeed — TUI renders it identically to font 1. One-line fix. | S | ADD-soon | No |
| set_colour (game-driven colours) | §8.3 | ACCEPTED/IGNORED | No-op; we own the palette; Flags1 bit 0 = 0 so well-written games skip it. | — | SKIP | No |
| set_true_colour | §8.3 | ACCEPTED/IGNORED | No-op (both via mislabeled EXT:0x05 and catch-all EXT:0x0D). Fine. | — | SKIP | No |
| buffer_mode | §8.7.1 | IMPLEMENTED | Tracked in ScreenState; forwarded to output sink. ✓ | — | — | — |
| v6 multi-window (8 windows) | §8.6 | MISSING | v6 not supported at all. | XL | SKIP | Yes |
| v6 pixel positioning | §8.6 | MISSING | v6 not supported. | XL | SKIP | Yes |
| output_stream(3) — memory | §7.1.2.5 | IMPLEMENTED | Up to 16 nested stream-3 frames. ✓ | — | — | — |
| output_stream(2) — transcript | §7.1.3 | PARTIAL | Flag tracked in StreamState; the host app honours it (transcript export). The CLI wires stream2 to a separate transcript file in zvm-cli. Fine. | — | — | — |

---

### D. Input (ZMSD §10)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| `read` basic (text+parse buffers) | §10.1 | IMPLEMENTED | Both v3 (null-terminated) and v5+ (count byte) layouts. ✓ | — | — | — |
| Terminating-characters table (header 0x2E) | §10.7 | PARTIAL | `is_terminator` correctly reads the table; function keys (ch≥129), char 255 wildcard all handled. BUT `supply_line` always stores 13 (Enter) regardless of which key ended input. | M | ADD-soon | Yes |
| Timed/interrupt input (read time+routine) | §10.2 | MISSING | Operands accepted; ignored. Clock-driven input break not implemented. | L | DEFER | Yes |
| `read_char` | §10.1.4 | IMPLEMENTED | Suspend + supply_char. ✓ | — | — | — |
| `read_char` timed variant | §10.1.4 | MISSING | Time/routine operands ignored. | L | DEFER | Yes |
| `input_stream` (VAR:0x14) | §10.3 | INTENTIONALLY DEFERRED | Issues a one-time diagnostic. No game in the Infocom library uses it interactively. | M | DEFER | Yes |
| Function-key codes (129–154) to ZSCII | §3.8.2 | PARTIAL | zvm-cli decodes arrows + F1–F4; full range (other function keys) falls back to raw ESC. App maps key events to ZSCII per keymap but function-key ZSCII range is not fully exhausted. | M | ADD-soon | Yes (in zvm-cli push-off list) |
| Mouse input (`read_mouse`, v6) | §10.6 | MISSING | v6 only. Not supported. | XL | SKIP | No |

---

### E. Sound (ZMSD §9)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| sound_effect #1 (high bleep) | §9.4 | VISUAL ONLY | Queued as `Beep::High`; host shows a border pulse. No audio output. | L | DEFER | Yes |
| sound_effect #2 (low bleep) | §9.4 | VISUAL ONLY | Same. | L | DEFER | Yes |
| sound_effect #≥3 (Blorb sampled sounds) | §9.4 | MISSING | One-time diagnostic fired. No Blorb audio parser or playback backend. | L | DEFER | Yes |
| Blorb 'Snd ' resource loading | ZMSD §9 / Blorb | MISSING | No Blorb audio resource reader. Requires IFF parser + audio backend (rodio/cpal). | L | DEFER | Yes |
| sound_effect routine callback | §9.4 | MISSING | Finish-callback routine not invoked (tied to timed input machinery). | L | DEFER | Yes |

---

### F. Character Set / Unicode (ZMSD §3, §16)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| ZSCII 32–126 (ASCII) | §3.8 | IMPLEMENTED | Identity mapping. ✓ | — | — | — |
| ZSCII 13 (newline) | §3.8 | IMPLEMENTED | Maps to `\n`. ✓ | — | — | — |
| ZSCII 155–223 (default Unicode table) | §3.8.5 | IMPLEMENTED | Full 69-entry default table. ✓ | — | — | — |
| Custom Unicode translation table (header-ext §3.8.5.4) | §3.8.5.4 | IMPLEMENTED | Parsed from header-extension word 3; overrides default table for ZSCII ≥155. ✓ | — | — | — |
| Custom alphabet table (header 0x34, v5+) | §3.3 | IMPLEMENTED | Decode and encode both honour it. ✓ | — | — | — |
| print_unicode (EXT:0x0B) | §3.8 | IMPLEMENTED | ✓ | — | — | — |
| check_unicode (EXT:0x0C) | §3.8 | IMPLEMENTED | Returns 3 (print+input) for valid scalars. ✓ | — | — | — |
| Abbreviations | §3.6 | IMPLEMENTED | Full recursive decode (one level, correct per spec). ✓ | — | — | — |
| CP437 rendering (interpreter 6 path) | §3.8 (implied) | IN-FLIGHT | Deliberately interpreter 6; full Font-3 table is done; raw CP437 byte passthrough for games that send them directly is tracked in TODO as in-progress (2026-06-30). | M | DEFER | Yes |
| ZSCII 8 (delete), 27 (ESC) in read_char | §3.8.2 | NOT VERIFIED | Special input codes (backspace, escape, arrow keys) need to be correctly passed as their ZSCII values from the host. Engine accepts any u8 via supply_char. | S | ADD-soon | No |

---

### G. Save / Restore (ZMSD §7, Quetzal 1.4)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| Quetzal save (IFhd + CMem + Stks) | Quetzal 1.4 | IMPLEMENTED | Full round-trip tested. ✓ | — | — | — |
| Quetzal UMem chunk on RESTORE | Quetzal §3 | MISSING | We emit CMem only. Restoring a save with UMem (from another interpreter) returns `Truncated`. Cross-interpreter save compatibility affected. | M | ADD-soon | No |
| Quetzal optional IntD chunk | Quetzal §4.4 | NOT SUPPORTED | No interpreter-private data; not needed. Harmless omission. | — | SKIP | No |
| v5 save/restore to table (EXT:0x00/0x01 ≥3 ops) | §15 | IMPLEMENTED | Aux table; host persists via archive or global file. ✓ | — | — | — |
| save_undo (EXT:0x09) | §15 | IMPLEMENTED | In-memory Quetzal snapshots; configurable depth. ✓ | — | — | — |
| restore_undo (EXT:0x0A) | §15 | IMPLEMENTED | Pops newest snapshot. ✓ | — | — | — |
| v3 in-game save/restore (branch form) | §15 | PARTIAL | Engine handles v3 branch form for save (SaveDest::Branch); the app-side UI for v3-specific file dialog is deferred. Engine-side: correct. | S | DEFER | Yes |
| Import/export .qzl files | App feature | IMPLEMENTED | Saves manager modal supports export (e) and import (i) of .qzl files. ✓ | — | — | — |

---

### H. Interpreter Number / Version (ZMSD §11.1.5)

| Feature | Status | Notes | Effort | Rec | TODO? |
|---------|--------|-------|--------|-----|-------|
| Interpreter number = 6 (IBM PC) | SINGLE GLOBAL | Hard-coded to 6. BeyondZork branches on this for Font-3 vs CP437 path — currently the right choice. Some v6 graphical games branch on it differently. Cross-game impact is real but manageable. | M | DEFER | Yes |
| Per-game interpreter number | NOT IMPLEMENTED | Would allow optimising the tag per-game. Complex policy; see TODO note. | L | DEFER | Yes |

---

### I. Dictionary / Parser (ZMSD §13)

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| Standard dictionary load + tokenise | §13 | IMPLEMENTED | v3 (6-char/3-word) + v5 (9-char/4-word) encodings; multiple word separators. ✓ | — | — | — |
| Custom dictionary (tokenise operand 2) | §15 | PARTIAL | VAR:0x1B ignores the custom-dict address and uses the standard dict. Rare but used by some games. | S | ADD-soon | Yes (code comment) |
| encode_text | §15 | IMPLEMENTED | Custom alphabet table honoured (encode_word_mem). ✓ | — | — | — |

---

### J. Object Model (ZMSD §12)

All object operations (get/set/clear_attr, insert_obj, remove_obj, get_sibling, get_child, get_parent, get_prop, get_prop_addr, get_next_prop, get_prop_len, put_prop, print_obj) are fully implemented. v3 (31 defaults, 32 objects) and v4+ (63 defaults, 65535 objects) table layouts are both handled. No gaps.

---

### K. Miscellaneous

| Feature | Spec § | Status | Notes | Effort | Rec | TODO? |
|---------|--------|--------|-------|--------|-----|-------|
| `piracy` (0OP:0x0F) | §15 | IMPLEMENTED | Always branches (genuine). ✓ | — | — | — |
| `verify` (0OP:0x0D) | §15 | IMPLEMENTED | Checksums ORIGINAL story image. ✓ | — | — | — |
| `random` predictable/entropy modes | §15 | IMPLEMENTED | xorshift32; negative → predictable seed; 0 → entropy step. ✓ | — | — | — |
| `restart` | §15 | IMPLEMENTED | Returns `StepResult::Restart` to host. ✓ | — | — | — |
| v7/v8 packed-address scaling | §1.2.3 | IMPLEMENTED | v7 routine/string offsets parsed; v8 8× scaling. ✓ | — | — | — |
| v6 load (graphical Z-machine) | §1 | REJECTED | `header.rs` returns `ZError::GraphicalV6`. Aspirational; TUI constraint. | XL | SKIP | Yes |
| v1/v2 support | §1 | REJECTED | Returns `UnsupportedVersion`. These are museum pieces; not worth the effort. | XL | SKIP | No |
| Accessibility (ZMSD §8.8, §10.8) | §8.8 | NOT ADDRESSED | Spec mentions non-visual output considerations; we have no specific accessibility pass. Out of scope. | XL | SKIP | No |

---

## Gap Count by Recommendation

| Recommendation | Count |
|---------------|-------|
| ADD-soon | 14 |
| DEFER | 13 |
| SKIP | 11 |

**Total gaps identified: 38**  
(Excludes implemented/correct items and the many SKIP/DEFER v6 opcodes counted as one group.)

---

## Cross-Reference: Items NOT in TODO.md

These are real gaps not yet tracked anywhere:

1. Standard revision 1.2 → 1.1 bug (byte 0x33 = 2, should be 1)
2. Flags1 v4+ bit 2 bold, bit 3 italic — clear despite working rendering
3. Flags2 bit 4 undo — clear despite working undo
4. `throw` (2OP:0x1C) — silent no-op, breaks catch/throw
5. `erase_window(-2)` and `erase_window(0)` — unhandled
6. Font 4 (Courier) returning 0 — should succeed
7. EXT:0x05 comment mislabeled (draw_picture, not set_true_colour); EXT:0x0D not explicit
8. Quetzal UMem restore
9. Flags1 v3 bit 6 (variable-pitch) not explicitly cleared
10. ZSCII 8/27 special input codes — not verified end-to-end
