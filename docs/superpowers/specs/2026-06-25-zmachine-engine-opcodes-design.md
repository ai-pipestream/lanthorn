# Z-Machine Engine Opcodes (Core + Graceful) — Design

**Date:** 2026-06-25
**Status:** Approved (scope confirmed) — automated wave, zvm crate only.
**TODO items:** tokenise, encode_text, print_unicode/check_unicode, custom alphabet table, terminating-characters table, set_colour, set_true_colour, set_font, verify, piracy.

## Goal

Implement a batch of standard Z-machine opcodes that are currently missing or
stubbed, raising spec coverage. All changes are in the `zvm` crate; they flow to
`zvm-cli` automatically. Deferred (separate efforts): `input_stream`, v5
save/restore to a memory table, `sound_effect` (Blorb audio), v6 graphics.

## Scope & semantics (Z-Machine Standard 1.1)

**High-value (real implementation):**

- **tokenise (VAR:0x1B)** — `tokenise text parse dictionary flag`. Lexically
  analyse the text buffer into the parse buffer, exactly like the lexing half of
  `read`. Reuse `dictionary::load` + `Dictionary::tokenise` (the same path
  `supply_line` uses, exec.rs ~1271). Operands: `text` (byte addr of text
  buffer), `parse` (byte addr of parse buffer), optional `dictionary` (addr; 0 →
  the standard dictionary), optional `flag` (if nonzero, do not overwrite parse
  entries for words not in the dictionary). Writes the parse buffer in the
  standard format: byte0 = max words; byte1 = number of words parsed; then per
  word 4 bytes: dictionary addr (word), token length (byte), token text position
  (byte). No store, no branch.

- **encode_text (VAR:0x1C)** — `encode_text zscii-text length from coded-text`.
  Take `length` ZSCII bytes starting at `zscii-text + from`, encode to the packed
  dictionary form (4 bytes v3, 6 bytes v4+) and write to `coded-text`. Reuse
  `text::encode::encode_word`. No store, no branch.

- **print_unicode (EXT:0x0B)** — `print_unicode char-number`. Output the Unicode
  codepoint to the current stream. Convert `u16`/`u32` codepoint → `char` →
  UTF-8 and call `print_text` (the same sink `print_char` uses). Invalid
  codepoints print U+FFFD. No store.

- **check_unicode (EXT:0x0C)** — `check_unicode char-number -> (result)`. STORE
  op. Store a bitmask: bit 0 set if the interpreter can PRINT the char, bit 1 set
  if it can READ it from the keyboard. We render UTF-8 and read UTF-8, so for any
  valid Unicode scalar value store `3`; for an invalid codepoint store `0`.

- **Custom alphabet table (header 0x34, v5+)** — when header word 0x34 is nonzero,
  the three 26-entry alphabet rows (A0/A1/A2) come from that table (78 ZSCII
  bytes) instead of the hard-coded defaults. `text/decode.rs` must consult it.
  A0/A1/A2 selection logic is unchanged; only the glyph source changes. v1-4 and
  a zero pointer keep the built-in tables.

- **Terminating-characters table (header 0x2E, v5+)** — when header word 0x2E is
  nonzero it points to a zero-terminated list of ZSCII terminating characters
  (255 = "any function key"). On a v5+ `read`, input also terminates on any listed
  terminator, and the terminating character actually used is stored (replacing the
  hard-coded `13`). Enter (13) always terminates. With no table (0) behaviour is
  unchanged (Enter only, store 13).

**Graceful (cheap, no-crash):**

- **set_font (EXT:0x04)** — STORE. Return the previous font on success, 0 if the
  requested font is unavailable. We have one fixed font (id 1): requesting font 1
  (or 0 = "query current") → store 1; any other font → store 0 (unavailable). No
  actual font change.

- **set_colour (2OP:0x1B)** / **set_true_colour (EXT:0x05)** — explicit no-op arms
  (accept and ignore operands). lanthorn renders with its own styling and
  advertises no game-driven colour; an explicit arm documents the intentional
  no-op (currently a silent fall-through).

- **verify (0OP:0x0D)** — BRANCH. Replace the always-true stub with a real check:
  sum (mod 0x10000) of every byte from 0x40 to `0x40 + (file_length)` exclusive,
  compared against the header checksum (word 0x1C); `file_length` = header word
  0x1A × scale (2 for v3, 4 for v4-5, 8 for v6-7). Branch on equality. If the
  header length/checksum is 0 (some dev builds), branch true.

- **piracy (0OP:0x0F)** — BRANCH. Already correct (the standard says interpreters
  "should" branch as if genuine = always take the branch). Keep always-true; just
  document the arm. No behavioural change.

## Architecture / anchors (zvm)

- Dispatch: `exec_2op` (exec.rs ~224), `exec_0op` (~540), `exec_var` (~659),
  `exec_ext` (~1020, store-only — no branch param). Unknown VAR warns once;
  unknown 2OP/EXT silently `Continue`.
- Helpers: `do_store(Option<u8>, u16)` (~1126), `do_branch(Option<Branch>, bool)`
  (~1109), `print_text(&str)` (~1165), `Memory::{read_byte,write_byte,read_word,
  write_word}`, `Memory::dictionary()` / header readers.
- Reuse: `dictionary::load` + `Dictionary::tokenise` (dictionary.rs), 
  `text::encode::encode_word` (text/encode.rs), the A0/A1/A2 tables + selection in
  `text/decode.rs` (~91), the v5 read terminator store (exec.rs ~1290) and the
  tokenise call in `supply_line` (~1271).
- Tests: encode EXT as `0xBE op typebyte ops [store]`; 2OP/VAR/0OP per existing
  templates; `sample_story(v)`, `Machine::new`, `m.state.pc=0x10`,
  `run_until_quit`, `m.global(n)`, `m.mem.read_*` for buffer assertions. The
  `log_shift` test is the EXT template; `je_branch_taken_and_not_taken` the branch
  template.

## Testing

Per opcode: a focused test using the real harness. tokenise: a known word in the
story dictionary parses into the parse buffer with the right dict addr/len/pos.
encode_text: a word encodes to the same bytes `encode_word` produces / that the
dictionary stores. print_unicode: an accented char reaches the output sink.
check_unicode: a valid scalar stores 3, an invalid one 0. set_font: font 1 → 1,
font 4 → 0. verify: a story with a correct checksum branches true; a corrupted
byte branches false. Custom alphabet: a story with a custom table decodes a glyph
from it. Terminating chars: a story with a terminator table terminates+stores the
listed char.

## Out of scope (deferred)

- `input_stream` (VAR:0x14) — command-file playback (needs host plumbing).
- v5 save/restore to a memory table.
- `sound_effect` via Blorb (audio system; separate effort).
- v6 graphical Z-machine + Blorb graphics.
- Actually changing the rendered font/colours (we keep our own styling).
